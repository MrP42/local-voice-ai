//! B13 (P6f): Follow-up-Entwurf unabhaengig von der Chat-Belegpflicht.
//!
//! Der Chat antwortet bei strenger Belegpflicht "nicht gefunden", wenn die
//! Frage keinen Beleg im Transkript hat -- fuer eine Mail wie "schreibe ein
//! Follow-up" trifft das oft zu (Maerchen-Hoerspiel, Notizen ohne Beschluss),
//! und "erneut versuchen" half nie. Dieses Modul baut deshalb einen eigenen
//! Prompt: Grundlage sind KI-Notizen, eigene Notizen und Protokoll (soweit
//! vorhanden), sonst das Transkript; der Recipe-Text "Follow-up-E-Mail an ..."
//! bleibt die Vorlage. Der Aufruf laeuft lokal deterministisch
//! (`Purpose::Followup`, Temperatur 0 und fester Startwert wie P1g).
//!
//! Ergebnisse und Fehler sind Codes fuer die Oberflaeche:
//! - `followup_no_content`: keine Grundlage (nichts gesprochen/notiert) oder
//!   das Modell meldet `KEIN_INHALT` -- ein neuer Versuch aendert nichts.
//! - `followup_empty`: das Modell lieferte auch beim zweiten Versuch keinen
//!   Text.
//! - sonst die Codes des Chats (`no_provider`, `no_model`, `memory_low`,
//!   `recording_active_cpu`, `chat_busy`, `llm_failed`, `meeting_not_found`,
//!   `store_failed`).
//!
//! Kein Modul hier darf `settings::get_settings(&AppHandle)` rufen: die
//! Einstellungen kommen als Parameter. Datenschutz (D9): Logzeilen nennen nur
//! Laengen, Zaehler und Codes, nie Notiz-, Transkript- oder Antworttext.

use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use super::chat::controller::{check_gates, map_llm_error, ChatEnv, ChatGuard, RUN_TIMEOUT};
use super::chat::recipes::{load_recipe, render_recipe, StoreLookup};
use super::chat::{ChatError, CODE_LLM_FAILED, CODE_MEETING_NOT_FOUND, CODE_STORE_FAILED};
use super::export::build_bundle;
use super::llm_call::{
    build_head_with, head_facts_block, is_truncation_error, mm_ss, resolve_provider_coded,
    sorted_segments, strip_code_fence, MeetingHead, CODE_NO_MODEL as LLM_NO_MODEL,
};
use super::mail::{draft_from_answer, MailDraft};
use super::notes::enhance::{enhanced_to_markdown, single_pass_budget_chars};
use super::notes::model::{NoteBlock, NoteBlockKind};
use super::store::MeetingStore;
use crate::managers::usage::Purpose;
use crate::settings::AppSettings;

/// Keine Grundlage fuer eine Nachbereitung (siehe Moduldoku).
pub const CODE_NO_CONTENT: &str = "followup_no_content";
/// Das Modell lieferte keinen Text (siehe Moduldoku).
pub const CODE_EMPTY: &str = "followup_empty";

/// Antwort des Modells, wenn es nichts Brauchbares gibt (Vorgabe im System-Prompt).
pub const NO_CONTENT_MARKER: &str = "KEIN_INHALT";

/// Weniger Zeichen Notizen/Protokoll zaehlen als "duenn": das Transkript
/// kommt dann dazu (eine Notiz "Budget klaeren" traegt keine Mail).
pub const THIN_DIGEST_CHARS: usize = 400;
/// Weniger Zeichen Material insgesamt: nichts, worueber sich eine Mail
/// schreiben liesse (Stille, einzelne Laute).
pub const MIN_CONTENT_CHARS: usize = 20;
/// Bei Kuerzung bleibt so viel vom Anfang (Rest: Ende, Mitte faellt weg).
const CLIP_HEAD_PERCENT: usize = 65;
const CLIP_MARKER: &str = "[... gekürzt ...]";
/// Vorlage, wenn das Recipe nicht ladbar ist (kann bei `builtin:` nicht
/// passieren, haelt aber den Entwurf am Leben).
const FALLBACK_INSTRUCTION: &str = "Schreibe eine kurze Follow-up-E-Mail an {addressee} zu dieser \
     Besprechung: Dank für das Gespräch, die wichtigsten Ergebnisse und die vereinbarten \
     nächsten Schritte mit Verantwortlichen und Terminen.\nDie erste Zeile lautet genau \
     „Betreff: <Betreff der E-Mail>“. Danach folgt der Text der E-Mail ohne Überschriften.";
/// Zweiter Versuch nach einer leeren Antwort (derselbe Prompt, lokal
/// deterministisch, gaebe dieselbe leere Antwort).
const EMPTY_RETRY_HINT: &str = "Your previous reply was empty. Write the e-mail now: the first \
     line is \"Betreff: <subject>\", then the body. Only if the material has no usable content \
     at all, reply with KEIN_INHALT.";

// ---------------------------------------------------------------------------
// Material
// ---------------------------------------------------------------------------

/// Woraus der Entwurf entsteht.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    /// KI-Notizen, eigene Notizen und/oder Protokoll.
    Notes,
    /// Wie `Notes`, dazu das Transkript (das Material war duenn).
    NotesAndTranscript,
    /// Nur das Transkript.
    Transcript,
}

/// Das Material einer Besprechung, bereits als Text.
#[derive(Clone, Debug)]
pub struct Sources {
    pub title: String,
    pub head: MeetingHead,
    pub ai_notes: Option<String>,
    pub own_notes: String,
    pub minutes: Option<String>,
    pub transcript: String,
}

fn render_own_notes(blocks: &[NoteBlock]) -> String {
    blocks
        .iter()
        .filter(|b| !b.text.trim().is_empty())
        .map(|b| {
            let text = b.text.split_whitespace().collect::<Vec<_>>().join(" ");
            match b.kind {
                NoteBlockKind::Heading => format!("# {text}"),
                NoteBlockKind::Bullet => format!("- {text}"),
                NoteBlockKind::Paragraph => text,
                NoteBlockKind::Todo => {
                    format!("[{}] {text}", if b.checked { "x" } else { " " })
                }
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Material aus dem Store. Nur lesend; eine fehlende Quelle bleibt leer.
pub fn gather_sources(store: &MeetingStore, meeting_id: &str) -> Result<Sources, ChatError> {
    let bundle = build_bundle(store, meeting_id).map_err(|e| {
        if e.starts_with("meeting_not_found") {
            ChatError::code_only(CODE_MEETING_NOT_FOUND)
        } else {
            ChatError::new(CODE_STORE_FAILED, "bundle")
        }
    })?;
    let head = build_head_with(&bundle.meeting, &bundle.segments, &bundle.speakers);
    let ai_notes = bundle
        .enhanced
        .as_ref()
        .filter(|n| n.sections.iter().any(|s| !s.entries.is_empty()))
        .map(|n| enhanced_to_markdown(&bundle.meeting.title, n));
    let minutes = bundle.minutes_md.clone().filter(|m| !m.trim().is_empty());
    let segments = sorted_segments(&bundle.segments);
    let transcript = segments
        .iter()
        .filter(|s| !s.text.trim().is_empty())
        .map(|s| {
            let text = s.text.split_whitespace().collect::<Vec<_>>().join(" ");
            if head.mixed_channel {
                format!("[{}] {text}", mm_ss(s.start_ms))
            } else {
                format!(
                    "[{}] {}: {text}",
                    mm_ss(s.start_ms),
                    bundle.speakers.label(s)
                )
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    Ok(Sources {
        title: bundle.meeting.title.clone(),
        head,
        ai_notes,
        own_notes: render_own_notes(&bundle.notes),
        minutes,
        transcript,
    })
}

// ---------------------------------------------------------------------------
// Budget
// ---------------------------------------------------------------------------

/// Verteilt `budget` Zeichen fair auf Teile der Laengen `lens`: kurze Teile
/// bekommen alles, was sie brauchen, lange teilen den Rest gleichmaessig.
pub fn fair_shares(lens: &[usize], budget: usize) -> Vec<usize> {
    let mut order: Vec<usize> = (0..lens.len()).collect();
    order.sort_by_key(|&i| lens[i]);
    let mut shares = vec![0; lens.len()];
    let mut left = budget;
    let mut remaining = lens.len();
    for i in order {
        let share = lens[i].min(left / remaining.max(1));
        shares[i] = share;
        left -= share;
        remaining -= 1;
    }
    shares
}

/// Kuerzt auf hoechstens `max_chars`: Anfang (`head_percent`) und Ende bleiben,
/// die Mitte faellt an Zeilengrenzen weg und wird markiert. Zweiter Wert:
/// wurde gekuerzt?
pub fn clip_text(text: &str, max_chars: usize, head_percent: usize) -> (String, bool) {
    if text.chars().count() <= max_chars {
        return (text.to_string(), false);
    }
    let marker = format!("\n{CLIP_MARKER}\n");
    let room = max_chars.saturating_sub(marker.chars().count());
    let head_room = room * head_percent / 100;
    let tail_room = room - head_room;
    let lines: Vec<&str> = text.lines().collect();

    let mut head: Vec<&str> = Vec::new();
    let mut used = 0usize;
    for line in &lines {
        let cost = line.chars().count() + 1;
        if used + cost > head_room {
            break;
        }
        used += cost;
        head.push(line);
    }
    let mut head_text = head.join("\n");
    if head.is_empty() {
        // Eine einzige riesige Zeile: hart schneiden.
        head_text = text.chars().take(head_room).collect();
    }

    let mut tail: Vec<&str> = Vec::new();
    let mut used = 0usize;
    for line in lines.iter().skip(head.len()).rev() {
        let cost = line.chars().count() + 1;
        if used + cost > tail_room {
            break;
        }
        used += cost;
        tail.push(line);
    }
    tail.reverse();
    (format!("{head_text}{marker}{}", tail.join("\n")), true)
}

// ---------------------------------------------------------------------------
// Prompt
// ---------------------------------------------------------------------------

pub fn system_prompt() -> String {
    format!(
        "You write a follow-up e-mail after a meeting or recording from the material the user \
         provides (notes, minutes or a transcript).\n\
         Rules:\n\
         - Use only what the material says. Never invent names, dates, amounts, decisions or \
         tasks; name a person, date or deadline only if it appears in the material.\n\
         - Write in German, in the tone the task asks for.\n\
         - Output format: the first line is exactly \"Betreff: <subject>\", then a blank line, \
         then the e-mail body as plain text. No headings, no markdown emphasis, no source \
         markers such as [1] or S12, no placeholders in brackets. Lines starting with \"- \" \
         are fine for lists.\n\
         - A recording without decisions or tasks still gets an e-mail: say briefly what it was \
         about and that no next steps were agreed. Never invent agreements.\n\
         - Only if the material has no readable content at all (blank, noise, single words), \
         reply with exactly {NO_CONTENT_MARKER} and nothing else."
    )
}

/// Der fertige Prompt und woraus er entstand.
#[derive(Clone, Debug)]
pub struct FollowupPrompt {
    pub system: String,
    pub user: String,
    pub source: Source,
    /// Wurde Material gekuerzt (Budget)?
    pub clipped: bool,
    /// Zeichen Material im Prompt (ohne Rahmen).
    pub payload_chars: usize,
}

/// Prompt aus dem Material: Notizen/Protokoll vor Transkript, alles im
/// Zeichenbudget (`budget_chars`, Material ohne Rahmen). Ohne brauchbares
/// Material: `followup_no_content`.
pub fn build_prompt(
    sources: &Sources,
    addressee: &str,
    instruction: &str,
    budget_chars: usize,
) -> Result<FollowupPrompt, ChatError> {
    let mut parts: Vec<(&str, &str)> = Vec::new();
    if let Some(text) = sources.ai_notes.as_deref().filter(|t| !t.trim().is_empty()) {
        parts.push(("AI meeting notes", text));
    }
    if !sources.own_notes.trim().is_empty() {
        parts.push(("The user's own notes", sources.own_notes.as_str()));
    }
    if let Some(text) = sources.minutes.as_deref().filter(|t| !t.trim().is_empty()) {
        parts.push(("Minutes", text));
    }
    let transcript = sources.transcript.trim();
    let digest_chars: usize = parts.iter().map(|(_, t)| t.chars().count()).sum();
    let transcript_chars = transcript.chars().count();

    let source = if parts.is_empty() {
        Source::Transcript
    } else if digest_chars < THIN_DIGEST_CHARS && transcript_chars > 0 {
        Source::NotesAndTranscript
    } else {
        Source::Notes
    };
    if matches!(source, Source::Transcript | Source::NotesAndTranscript) && transcript_chars > 0 {
        parts.push(("Transcript", transcript));
    }
    let total: usize = parts.iter().map(|(_, t)| t.chars().count()).sum();
    if total < MIN_CONTENT_CHARS {
        return Err(ChatError::code_only(CODE_NO_CONTENT));
    }

    let lens: Vec<usize> = parts.iter().map(|(_, t)| t.chars().count()).collect();
    let shares = fair_shares(&lens, budget_chars);
    let mut clipped = false;
    let mut payload_chars = 0usize;
    let mut material = String::new();
    for ((label, text), share) in parts.iter().zip(shares) {
        let (text, was_clipped) = clip_text(text, share, CLIP_HEAD_PERCENT);
        clipped |= was_clipped;
        payload_chars += text.chars().count();
        material.push_str(&format!("\n# Material: {label}\n{text}\n"));
    }

    let user = format!(
        "{}\n# Recipient\n{}\n\n# Task\n{}\n{material}",
        head_facts_block(&sources.head),
        addressee.trim(),
        instruction.trim(),
    );
    Ok(FollowupPrompt {
        system: system_prompt(),
        user,
        source,
        clipped,
        payload_chars,
    })
}

// ---------------------------------------------------------------------------
// Antwort
// ---------------------------------------------------------------------------

/// Was aus der Antwort des Modells wird.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Reply {
    Draft(MailDraft),
    /// Leer, nur Leerraum oder nur eine Betreffzeile.
    Empty,
    /// Das Modell meldet `KEIN_INHALT`.
    NoContent,
}

fn is_no_content_line(line: &str) -> bool {
    let cleaned: String = line
        .chars()
        .filter(|c| !matches!(c, '*' | '"' | '\'' | '.' | '`' | '!'))
        .collect::<String>()
        .replace('_', " ");
    cleaned.trim().eq_ignore_ascii_case("KEIN INHALT")
}

pub fn interpret_reply(reply: Option<&str>, title: &str, to: Vec<String>) -> Reply {
    let text = strip_code_fence(reply.unwrap_or_default());
    let Some(first) = text.lines().find(|l| !l.trim().is_empty()) else {
        return Reply::Empty;
    };
    if is_no_content_line(first) {
        return Reply::NoContent;
    }
    let draft = draft_from_answer(text, title, to);
    if draft.body_text.trim().is_empty() {
        Reply::Empty
    } else {
        Reply::Draft(draft)
    }
}

// ---------------------------------------------------------------------------
// Ablauf
// ---------------------------------------------------------------------------

/// Die Vorlage: Recipe "Follow-up-E-Mail an ..." mit dem Empfaenger eingesetzt.
fn instruction_for(store: &MeetingStore, addressee: &str) -> String {
    let recipe_id = format!("{}follow-up-mail", super::search::index::BUILTIN_PREFIX);
    let rendered = load_recipe(store, &recipe_id).ok().and_then(|recipe| {
        let mut values = std::collections::HashMap::new();
        values.insert("empfaenger".to_string(), addressee.to_string());
        render_recipe(&recipe.spec, &values, &StoreLookup(store)).ok()
    });
    rendered
        .map(|r| r.prompt)
        .unwrap_or_else(|| FALLBACK_INSTRUCTION.replace("{addressee}", addressee))
}

/// Stellschrauben, die Tests ersetzen.
#[derive(Clone, Copy, Default)]
pub struct RunLimits {
    /// `None` = `single_pass_budget_chars(model, local)`.
    pub budget_chars: Option<usize>,
}

/// Erzeugt den Entwurf (ohne Guard und Zeitlimit, siehe `draft_guarded`).
pub async fn draft(
    settings: &AppSettings,
    store: Arc<MeetingStore>,
    meeting_id: &str,
    addressee: &str,
    to: Vec<String>,
    limits: RunLimits,
) -> Result<MailDraft, ChatError> {
    let sources = {
        let store = Arc::clone(&store);
        let id = meeting_id.to_string();
        tokio::task::spawn_blocking(move || gather_sources(&store, &id))
            .await
            .map_err(|_| ChatError::new(CODE_STORE_FAILED, "join"))??
    };
    let (provider, model, api_key) = resolve_provider_coded(settings).map_err(|e| {
        ChatError::code_only(if e.code == LLM_NO_MODEL {
            super::chat::CODE_NO_MODEL
        } else {
            super::chat::CODE_NO_PROVIDER
        })
    })?;
    let local = crate::managers::llm::is_local(&provider);
    let instruction = instruction_for(&store, addressee);
    let mut budget = match limits.budget_chars {
        Some(chars) => chars,
        None => single_pass_budget_chars(&model, local).await,
    };
    let mut prompt = build_prompt(&sources, addressee, &instruction, budget)?;
    log::info!(
        "Follow-up: Quelle {:?}, {} Zeichen Material (Budget {}, gekürzt: {})",
        prompt.source,
        prompt.payload_chars,
        budget,
        prompt.clipped
    );

    let mut shrunk = false;
    let mut empty_retried = false;
    let mut user = prompt.user.clone();
    loop {
        let reply = crate::llm_client::send_chat_completion_checked(
            Purpose::Followup,
            &provider,
            api_key.clone(),
            &model,
            user.clone(),
            Some(prompt.system.clone()),
            None,
            None,
            None,
        )
        .await;
        let reply = match reply {
            Ok(reply) => reply,
            // Prompt zu gross fuer den Kontext: einmal mit halbem Budget.
            Err(e) if is_truncation_error(&e) && !shrunk => {
                shrunk = true;
                budget /= 2;
                log::warn!("Follow-up: Kontext zu klein, wiederhole mit Budget {budget}");
                prompt = build_prompt(&sources, addressee, &instruction, budget)?;
                user = prompt.user.clone();
                continue;
            }
            Err(e) => {
                let err = map_llm_error(&e);
                log::warn!("Follow-up fehlgeschlagen: {}", err.code);
                return Err(err);
            }
        };
        match interpret_reply(reply.content.as_deref(), &sources.title, to.clone()) {
            Reply::Draft(draft) => return Ok(draft),
            Reply::NoContent => return Err(ChatError::code_only(CODE_NO_CONTENT)),
            Reply::Empty if !empty_retried => {
                empty_retried = true;
                log::warn!("Follow-up: leere Antwort, zweiter Versuch mit Hinweis");
                user = format!("{}\n\n{EMPTY_RETRY_HINT}", prompt.user);
            }
            Reply::Empty => return Err(ChatError::code_only(CODE_EMPTY)),
        }
    }
}

/// Ein Lauf mit Guard (ein Chat-/Follow-up-Lauf gleichzeitig), Pforten
/// (Aufnahme auf CPU, KI-Notizen-Lauf) und Zeitlimit. Die Command-Schicht
/// ruft das mit dem globalen Flag (`chat::controller::running_flag`).
pub async fn draft_guarded(
    flag: &AtomicBool,
    settings: &AppSettings,
    store: Arc<MeetingStore>,
    meeting_id: &str,
    addressee: &str,
    to: Vec<String>,
    env: &ChatEnv,
    limits: RunLimits,
) -> Result<MailDraft, ChatError> {
    let _guard = ChatGuard::try_acquire(flag)?;
    let local = resolve_provider_coded(settings)
        .map(|(p, _, _)| crate::managers::llm::is_local(&p))
        .unwrap_or(false);
    check_gates(local, env)?;
    match tokio::time::timeout(
        RUN_TIMEOUT,
        draft(settings, store, meeting_id, addressee, to, limits),
    )
    .await
    {
        Ok(result) => result,
        Err(_) => Err(ChatError::new(CODE_LLM_FAILED, "timeout")),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use std::sync::atomic::Ordering;
    use std::sync::Mutex;

    use super::*;
    use crate::managers::meetings::chat::controller::CancelFlag;
    use crate::managers::meetings::llm_call::test_support::{
        chat_body, settings_with_mock_provider, spawn_llm_mock_with, MockReply,
    };
    use crate::managers::meetings::store::{
        MeetingSource, MeetingStatus, StoredSegment, TranscriptDelta,
    };

    fn head() -> MeetingHead {
        MeetingHead {
            title: "Die drei Schwestern".into(),
            date_iso: "2026-09-30".into(),
            duration_ms: 600_000,
            shares: vec![],
            single_speaker: true,
            mixed_channel: true,
        }
    }

    fn sources(ai: Option<&str>, own: &str, minutes: Option<&str>, transcript: &str) -> Sources {
        Sources {
            title: "Die drei Schwestern".into(),
            head: head(),
            ai_notes: ai.map(str::to_string),
            own_notes: own.into(),
            minutes: minutes.map(str::to_string),
            transcript: transcript.into(),
        }
    }

    const LONG_NOTES: &str = "## Beschlüsse\n- Angebot bis Freitag senden\n- Preisstaffel prüfen\n\
        ## Aufgaben\n- Anna: Angebot schreiben, bis 3.10.\n- Ben: Preisliste aktualisieren, bis 5.10.\n\
        ## Themen\n- Lieferzeiten im vierten Quartal\n- Rahmenvertrag mit der Meyer GmbH\n\
        Zusätzlich wurde vereinbart, dass das Team die offenen Punkte im nächsten Jour fixe erneut ansieht.
        ## Risiken
- Die Lieferkette für die Steuerungen ist noch nicht bestätigt.
        - Der Kunde möchte die Schulung vor dem Go-live abschließen, der Termin steht noch nicht.";

    fn transcript_lines(n: usize) -> String {
        (0..n)
            .map(|i| {
                format!(
                    "[{}] Es war einmal ein Müller mit drei Töchtern, Nummer {i}.",
                    mm_ss(i as u64 * 5_000)
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    // -- Prompt-Aufbau ----------------------------------------------------------

    #[test]
    fn notes_come_before_the_transcript() {
        let s = sources(Some(LONG_NOTES), "", None, &transcript_lines(20));
        assert!(LONG_NOTES.chars().count() >= THIN_DIGEST_CHARS);
        let p = build_prompt(&s, "Anna Berg", "Schreibe eine Mail.", 100_000).unwrap();
        assert_eq!(p.source, Source::Notes);
        assert!(p.user.contains("# Material: AI meeting notes"));
        assert!(p.user.contains("Angebot bis Freitag senden"));
        assert!(
            !p.user.contains("# Material: Transcript"),
            "Transkript bleibt draußen"
        );
        assert!(!p.clipped);
    }

    #[test]
    fn all_available_digests_are_used_in_a_fixed_order() {
        let s = sources(
            Some(LONG_NOTES),
            "- Rückruf bei Meyer\n[ ] Vertrag prüfen",
            Some("# Protokoll\n\nEntscheidung: Rahmenvertrag verlängern."),
            "",
        );
        let p = build_prompt(&s, "Anna", "Mail.", 100_000).unwrap();
        let ai = p.user.find("AI meeting notes").unwrap();
        let own = p.user.find("The user's own notes").unwrap();
        let minutes = p.user.find("# Material: Minutes").unwrap();
        assert!(ai < own && own < minutes);
        assert!(p.user.contains("[ ] Vertrag prüfen"));
    }

    #[test]
    fn thin_notes_get_the_transcript_added() {
        let s = sources(None, "Budget klären", None, &transcript_lines(20));
        let p = build_prompt(&s, "Anna", "Mail.", 100_000).unwrap();
        assert_eq!(p.source, Source::NotesAndTranscript);
        let notes = p.user.find("The user's own notes").unwrap();
        let transcript = p.user.find("# Material: Transcript").unwrap();
        assert!(notes < transcript);
    }

    #[test]
    fn without_notes_the_transcript_is_the_source() {
        let s = sources(None, "  \n ", None, &transcript_lines(20));
        let p = build_prompt(&s, "Anna", "Mail.", 100_000).unwrap();
        assert_eq!(p.source, Source::Transcript);
        assert!(!p.user.contains("AI meeting notes"));
        assert!(p.user.contains("Nummer 19"));
    }

    #[test]
    fn head_recipient_and_recipe_text_are_part_of_the_prompt() {
        let s = sources(None, "", None, &transcript_lines(5));
        let p = build_prompt(
            &s,
            "Frau Weber",
            "Schreibe eine kurze Follow-up-E-Mail an Frau Weber.",
            10_000,
        )
        .unwrap();
        assert!(p.user.contains("Title: Die drei Schwestern"));
        assert!(p.user.contains("# Recipient\nFrau Weber"));
        assert!(p
            .user
            .contains("# Task\nSchreibe eine kurze Follow-up-E-Mail an Frau Weber."));
        assert!(p.system.contains("Betreff: <subject>"));
        assert!(p.system.contains(NO_CONTENT_MARKER));
    }

    #[test]
    fn nothing_to_write_about_is_no_content() {
        for s in [
            sources(None, "", None, ""),
            sources(None, "   ", Some("  "), "  \n "),
            sources(None, "", None, "[00:00] hm"),
        ] {
            let err = build_prompt(&s, "Anna", "Mail.", 10_000).unwrap_err();
            assert_eq!(err.code, CODE_NO_CONTENT);
        }
    }

    #[test]
    fn a_long_transcript_is_clipped_to_the_budget_keeping_start_and_end() {
        let transcript = transcript_lines(400);
        let s = sources(None, "", None, &transcript);
        let p = build_prompt(&s, "Anna", "Mail.", 3_000).unwrap();
        assert!(p.clipped);
        assert!(p.payload_chars <= 3_000, "{}", p.payload_chars);
        assert!(p.user.contains("Nummer 0."), "Anfang bleibt");
        assert!(p.user.contains("Nummer 399."), "Ende bleibt");
        assert!(p.user.contains(CLIP_MARKER));
        assert!(!p.user.contains("Nummer 200."), "Mitte fällt weg");
    }

    #[test]
    fn the_budget_is_shared_fairly_between_sources() {
        assert_eq!(
            fair_shares(&[100, 5_000, 5_000], 3_000),
            vec![100, 1_450, 1_450]
        );
        assert_eq!(fair_shares(&[10, 20], 1_000), vec![10, 20]);
        assert_eq!(fair_shares(&[], 100), Vec::<usize>::new());
        let long_notes = "x".repeat(4_000);
        let s = sources(
            Some(&long_notes),
            "- kurz und knapp, aber lang genug für den Test",
            None,
            "",
        );
        let p = build_prompt(&s, "A", "M.", 2_000).unwrap();
        assert!(p.clipped);
        assert!(
            p.user.contains("kurz und knapp"),
            "kurze Quelle bleibt ganz"
        );
        assert!(p.payload_chars <= 2_000);
    }

    #[test]
    fn clip_text_is_char_safe_and_within_the_limit() {
        let text = (0..50)
            .map(|i| format!("Zeile {i} mit Umlauten äöü"))
            .collect::<Vec<_>>()
            .join("\n");
        for max in [30, 100, 400] {
            let (out, clipped) = clip_text(&text, max, 65);
            assert!(clipped);
            assert!(out.chars().count() <= max, "{max}: {}", out.chars().count());
        }
        let (same, clipped) = clip_text("kurz", 100, 65);
        assert_eq!((same.as_str(), clipped), ("kurz", false));
        let (cut, clipped) = clip_text(&"ä".repeat(1_000), 200, 65);
        assert!(clipped && cut.chars().count() <= 200);
    }

    // -- Antwort und Fehlerabbildung --------------------------------------------

    #[test]
    fn a_normal_reply_becomes_a_draft_without_citation_marks() {
        let reply = "Betreff: Nächste Schritte\n\nHallo Anna,\n\nwir senden das Angebot bis Freitag [1].\n\nViele Grüße";
        let Reply::Draft(d) = interpret_reply(Some(reply), "Titel", vec!["a@b.de".into()]) else {
            panic!("Entwurf erwartet");
        };
        assert_eq!(d.subject, "Nächste Schritte");
        assert!(d.body_text.contains("bis Freitag."));
        assert!(!d.body_text.contains("[1]"));
        assert_eq!(d.to, vec!["a@b.de".to_string()]);
    }

    #[test]
    fn a_reply_without_subject_line_gets_the_fallback_subject() {
        let Reply::Draft(d) =
            interpret_reply(Some("Hallo zusammen,\n\ndanke."), "Jour fixe", vec![])
        else {
            panic!("Entwurf erwartet");
        };
        assert_eq!(d.subject, "Nachbereitung: Jour fixe");
    }

    #[test]
    fn empty_replies_are_empty() {
        assert_eq!(interpret_reply(None, "T", vec![]), Reply::Empty);
        assert_eq!(interpret_reply(Some(" \n\t"), "T", vec![]), Reply::Empty);
        assert_eq!(interpret_reply(Some("```\n```"), "T", vec![]), Reply::Empty);
        assert_eq!(
            interpret_reply(Some("Betreff: Nur ein Betreff"), "T", vec![]),
            Reply::Empty
        );
    }

    #[test]
    fn the_marker_means_no_content_in_any_common_spelling() {
        for reply in [
            "KEIN_INHALT",
            " kein_inhalt. ",
            "**KEIN_INHALT**",
            "\"KEIN INHALT\"",
            "KEIN_INHALT\nDie Aufnahme ist leer.",
        ] {
            assert_eq!(
                interpret_reply(Some(reply), "T", vec![]),
                Reply::NoContent,
                "{reply}"
            );
        }
        // Ein Satz, der das Wort enthält, ist kein Marker.
        assert!(matches!(
            interpret_reply(
                Some("Betreff: Kein Inhalt vereinbart\n\nHallo,\nalles klar."),
                "T",
                vec![]
            ),
            Reply::Draft(_)
        ));
    }

    // -- Ablauf mit Mock-Anbieter -----------------------------------------------

    struct Fx {
        store: Arc<MeetingStore>,
        meeting_id: String,
    }

    fn seg(index: u32, text: &str) -> StoredSegment {
        StoredSegment {
            segment_index: index,
            text: text.into(),
            start_ms: u64::from(index) * 5_000,
            end_ms: u64::from(index) * 5_000 + 4_000,
            channel: 2,
            speaker_index: None,
            words: None,
        }
    }

    fn fixture(texts: &[&str]) -> Fx {
        let dir = tempfile::tempdir().unwrap();
        let store = MeetingStore::open_at(&dir.path().join("meetings.db")).unwrap();
        std::mem::forget(dir);
        let m = store
            .create_meeting(
                "Die drei Schwestern",
                MeetingSource::Import,
                Some(1_790_000_000),
            )
            .unwrap();
        store
            .append_delta(
                &m.id,
                &TranscriptDelta {
                    new_segments: texts
                        .iter()
                        .enumerate()
                        .map(|(i, t)| seg(i as u32, t))
                        .collect(),
                },
            )
            .unwrap();
        store.set_status(&m.id, MeetingStatus::Ready).unwrap();
        Fx {
            store: Arc::new(store),
            meeting_id: m.id,
        }
    }

    const STORY: [&str; 3] = [
        "Es war einmal ein Müller, der hatte drei Töchter.",
        "Die älteste spann Gold aus Stroh, die zweite webte Silber.",
        "Die jüngste aber sang so schön, dass der König sie zur Frau nahm.",
    ];

    async fn run(fx: &Fx, port: u16, limits: RunLimits) -> Result<MailDraft, ChatError> {
        draft(
            &settings_with_mock_provider(port),
            fx.store.clone(),
            &fx.meeting_id,
            "Frau Weber",
            vec!["weber@firma.de".into()],
            limits,
        )
        .await
    }

    fn ok_mail() -> MockReply {
        MockReply::Body(chat_body("Betreff: Zum Märchen\n\nHallo Frau Weber,\n\nwir haben die Geschichte der drei Töchter besprochen.\n\nViele Grüße"))
    }

    #[tokio::test]
    async fn a_fairy_tale_transcript_yields_a_draft_instead_of_not_found() {
        let fx = fixture(&STORY);
        let requests = Arc::new(Mutex::new(Vec::<String>::new()));
        let seen = requests.clone();
        let port = spawn_llm_mock_with(move |body| {
            seen.lock().unwrap().push(body.to_string());
            ok_mail()
        })
        .await;
        let d = run(
            &fx,
            port,
            RunLimits {
                budget_chars: Some(50_000),
            },
        )
        .await
        .unwrap();
        assert_eq!(d.subject, "Zum Märchen");
        assert!(d.body_text.contains("drei Töchter"));
        assert_eq!(d.to, vec!["weber@firma.de".to_string()]);
        let requests = requests.lock().unwrap();
        assert_eq!(requests.len(), 1);
        let body: serde_json::Value = serde_json::from_str(&requests[0]).unwrap();
        let messages = body["messages"].as_array().unwrap();
        assert_eq!(messages[0]["role"], "system");
        let user = messages[1]["content"].as_str().unwrap();
        assert!(
            user.contains("Es war einmal ein Müller"),
            "Transkript ist Grundlage"
        );
        assert!(user.contains("Frau Weber"), "Empfänger in der Vorlage");
        assert!(
            user.contains("Die erste Zeile lautet genau"),
            "Recipe-Text als Vorlage"
        );
        assert!(
            body.get("response_format").is_none(),
            "Freitext, kein JSON-Schema"
        );
    }

    #[tokio::test]
    async fn own_notes_win_over_the_transcript_when_they_are_substantial() {
        let fx = fixture(&STORY);
        let notes: Vec<NoteBlock> = LONG_NOTES
            .lines()
            .enumerate()
            .map(|(i, l)| NoteBlock {
                id: format!("B{i}"),
                kind: NoteBlockKind::Paragraph,
                text: l.into(),
                at_ms: None,
                checked: false,
            })
            .collect();
        fx.store.save_notes(&fx.meeting_id, &notes, 0).unwrap();
        let requests = Arc::new(Mutex::new(Vec::<String>::new()));
        let seen = requests.clone();
        let port = spawn_llm_mock_with(move |body| {
            seen.lock().unwrap().push(body.to_string());
            ok_mail()
        })
        .await;
        run(
            &fx,
            port,
            RunLimits {
                budget_chars: Some(50_000),
            },
        )
        .await
        .unwrap();
        let body = requests.lock().unwrap()[0].clone();
        assert!(body.contains("Angebot bis Freitag senden"));
        assert!(
            !body.contains("Es war einmal ein Müller"),
            "Transkript bleibt draußen"
        );
    }

    #[tokio::test]
    async fn a_marker_reply_is_no_content_and_is_not_retried() {
        let fx = fixture(&STORY);
        let calls = Arc::new(Mutex::new(0usize));
        let counter = calls.clone();
        let port = spawn_llm_mock_with(move |_| {
            *counter.lock().unwrap() += 1;
            MockReply::Body(chat_body("KEIN_INHALT"))
        })
        .await;
        let err = run(&fx, port, RunLimits::default()).await.unwrap_err();
        assert_eq!(err.code, CODE_NO_CONTENT);
        assert_eq!(*calls.lock().unwrap(), 1);
    }

    #[tokio::test]
    async fn an_empty_meeting_never_reaches_the_model() {
        let fx = fixture(&[]);
        let calls = Arc::new(Mutex::new(0usize));
        let counter = calls.clone();
        let port = spawn_llm_mock_with(move |_| {
            *counter.lock().unwrap() += 1;
            ok_mail()
        })
        .await;
        let err = run(&fx, port, RunLimits::default()).await.unwrap_err();
        assert_eq!(err.code, CODE_NO_CONTENT);
        assert_eq!(*calls.lock().unwrap(), 0);
    }

    #[tokio::test]
    async fn an_empty_reply_gets_one_second_try_then_followup_empty() {
        let fx = fixture(&STORY);
        let calls = Arc::new(Mutex::new(Vec::<String>::new()));
        let seen = calls.clone();
        let port = spawn_llm_mock_with(move |body| {
            seen.lock().unwrap().push(body.to_string());
            MockReply::Body(chat_body("  "))
        })
        .await;
        let err = run(&fx, port, RunLimits::default()).await.unwrap_err();
        assert_eq!(err.code, CODE_EMPTY);
        let calls = calls.lock().unwrap();
        assert_eq!(calls.len(), 2, "genau ein zweiter Versuch");
        assert!(calls[1].contains("Your previous reply was empty"));
        assert!(!calls[0].contains("Your previous reply was empty"));
    }

    #[tokio::test]
    async fn the_second_try_can_succeed() {
        let fx = fixture(&STORY);
        let count = Arc::new(Mutex::new(0usize));
        let counter = count.clone();
        let port = spawn_llm_mock_with(move |_| {
            let mut n = counter.lock().unwrap();
            *n += 1;
            if *n == 1 {
                MockReply::Body(chat_body(""))
            } else {
                ok_mail()
            }
        })
        .await;
        assert_eq!(
            run(&fx, port, RunLimits::default()).await.unwrap().subject,
            "Zum Märchen"
        );
    }

    #[tokio::test]
    async fn a_context_overflow_is_retried_once_with_half_the_budget() {
        let fx = fixture(&STORY);
        let bodies = Arc::new(Mutex::new(Vec::<usize>::new()));
        let seen = bodies.clone();
        let port = spawn_llm_mock_with(move |body| {
            let mut sizes = seen.lock().unwrap();
            sizes.push(body.len());
            if sizes.len() == 1 {
                MockReply::StatusBody(400, "exceeds the available context size".into())
            } else {
                ok_mail()
            }
        })
        .await;
        run(
            &fx,
            port,
            RunLimits {
                budget_chars: Some(120),
            },
        )
        .await
        .unwrap();
        let sizes = bodies.lock().unwrap();
        assert_eq!(sizes.len(), 2);
        assert!(sizes[1] < sizes[0], "zweiter Prompt kleiner");
    }

    #[tokio::test]
    async fn technical_errors_keep_their_own_codes() {
        let fx = fixture(&STORY);
        // Server-Fehler: llm_failed, kein Wiederholen.
        let calls = Arc::new(Mutex::new(0usize));
        let counter = calls.clone();
        let port = spawn_llm_mock_with(move |_| {
            *counter.lock().unwrap() += 1;
            MockReply::Status(500)
        })
        .await;
        let err = run(&fx, port, RunLimits::default()).await.unwrap_err();
        assert_eq!(err.code, CODE_LLM_FAILED);
        assert_eq!(*calls.lock().unwrap(), 1);
        // Unbekannte Besprechung.
        let err = draft(
            &settings_with_mock_provider(port),
            fx.store.clone(),
            "gibt-es-nicht",
            "A",
            vec![],
            RunLimits::default(),
        )
        .await
        .unwrap_err();
        assert_eq!(err.code, CODE_MEETING_NOT_FOUND);
        // Kein Anbieter.
        let mut settings = settings_with_mock_provider(port);
        settings.post_process_provider_id = "gibt-es-nicht".into();
        let err = draft(
            &settings,
            fx.store.clone(),
            &fx.meeting_id,
            "A",
            vec![],
            RunLimits::default(),
        )
        .await
        .unwrap_err();
        assert_eq!(err.code, super::super::chat::CODE_NO_PROVIDER);
    }

    fn env(recording: bool, cpu: bool, enhance: bool) -> ChatEnv {
        ChatEnv {
            ctx_tokens: 8_192,
            backend_cpu: cpu,
            recording_active: recording,
            now: 0,
            cancel: CancelFlag::default(),
            enhance_running: Arc::new(move || enhance),
        }
    }

    #[tokio::test]
    async fn guard_and_gates_protect_the_run() {
        let fx = fixture(&STORY);
        let port = spawn_llm_mock_with(|_| ok_mail()).await;
        let settings = settings_with_mock_provider(port);
        let run_with = |flag: &'static AtomicBool, env: ChatEnv| {
            let (store, id, settings) = (fx.store.clone(), fx.meeting_id.clone(), settings.clone());
            async move {
                draft_guarded(
                    flag,
                    &settings,
                    store,
                    &id,
                    "A",
                    vec![],
                    &env,
                    RunLimits::default(),
                )
                .await
            }
        };
        // Belegt: chat_busy.
        let busy: &'static AtomicBool = Box::leak(Box::new(AtomicBool::new(true)));
        assert_eq!(
            run_with(busy, env(false, false, false))
                .await
                .unwrap_err()
                .code,
            "chat_busy"
        );
        // Ein laufender KI-Notizen-Lauf hat Vorrang.
        let flag: &'static AtomicBool = Box::leak(Box::new(AtomicBool::new(false)));
        assert_eq!(
            run_with(flag, env(false, false, true))
                .await
                .unwrap_err()
                .code,
            "chat_busy"
        );
        assert!(!flag.load(Ordering::Acquire), "Guard gibt frei");
        // Freier Lauf; das Flag ist danach wieder frei.
        assert!(run_with(flag, env(false, false, false)).await.is_ok());
        assert!(!flag.load(Ordering::Acquire));
    }
}
