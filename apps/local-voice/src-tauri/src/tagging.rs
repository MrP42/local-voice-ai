//! Auto-Tagging des Vorlesetexts (Paket C-T4): ein LLM schlägt Emotions-/
//! Vortrags-Tags (`[…]`) vor — aber der Vorschlag erreicht die Oberfläche
//! NIE unvalidiert. `validate_tag_only_edit` lehnt JEDE Textänderung ab, die
//! nicht ausschließlich neu eingefügte `[…]`-Tags ist; `diff_insertions`
//! rechnet die validierten Zusätze in exakte Einfüge-Positionen im Original
//! um (Byte- UND Zeichen-Offset, siehe [`TagInsertion`]).
//!
//! Struktur bewusst wie `translator.rs`: die reine Prüf-/Diff-Logik und der
//! LLM-Aufruf brauchen keinen `AppHandle` — das `llm-activity`-Event emittiert
//! der Tauri-Command (`commands::tts::tts_auto_tag`), nicht dieses Modul.

use crate::llm_client;
use crate::managers::tts::protocol::tag_spans;
use crate::settings::{AppSettings, PostProcessProvider};

// ---------------------------------------------------------------------------
// Nur-Einfüge-Invariante
// ---------------------------------------------------------------------------

/// Text ohne jeden Tag-Span (`[…]`, wiederverwendet aus `protocol::tag_spans`).
fn strip_tag_spans(text: &str) -> String {
    let spans = tag_spans(text);
    let mut out = String::with_capacity(text.len());
    let mut last = 0usize;
    for span in &spans {
        out.push_str(&text[last..span.start]);
        last = span.end;
    }
    out.push_str(&text[last..]);
    out
}

/// Whitespace-Normalisierung für den Gleichheitsvergleich: ein Lauf aus
/// Whitespace-Zeichen wird zu EINEM Zeichen zusammengefasst — enthält der
/// Lauf einen Zeilenumbruch, wird er zu `\n` (ein Absatzumbruch bleibt ein
/// Absatzumbruch, egal ob \n oder \n\n), sonst zu einem einzelnen Leerzeichen
/// (mehrere Leerzeichen, z. B. Polster um ein neu eingefügtes Tag, zählen
/// nicht als Änderung). Führendes/nachgestelltes Whitespace fällt weg
/// (`trim`). Ein `\n`, das im Original ein Leerzeichen war (oder umgekehrt),
/// bleibt dadurch weiterhin eine erkannte Abweichung — echte
/// Zeilenumbruch-Änderungen werden NICHT toleriert.
fn collapse_whitespace(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.trim().chars().peekable();
    while let Some(c) = chars.next() {
        if c.is_whitespace() {
            let mut has_newline = c == '\n';
            while let Some(&next) = chars.peek() {
                if !next.is_whitespace() {
                    break;
                }
                if next == '\n' {
                    has_newline = true;
                }
                chars.next();
            }
            out.push(if has_newline { '\n' } else { ' ' });
        } else {
            out.push(c);
        }
    }
    out
}

/// Kurze, für Menschen lesbare Diagnose der ersten Abweichung — Kontext
/// davor/danach, keine vollständigen Texte (die können lang sein).
fn short_diagnosis(orig_norm: &str, tagged_norm: &str) -> String {
    let orig_chars: Vec<char> = orig_norm.chars().collect();
    let tagged_chars: Vec<char> = tagged_norm.chars().collect();
    let mut i = 0usize;
    while i < orig_chars.len() && i < tagged_chars.len() && orig_chars[i] == tagged_chars[i] {
        i += 1;
    }
    let ctx = |chars: &[char], at: usize| -> String {
        let start = at.saturating_sub(15);
        let end = (at + 15).min(chars.len());
        chars[start..end].iter().collect()
    };
    format!(
        "Text weicht ab Zeichenposition {i} ab: Original '…{}…' vs. Antwort '…{}…'",
        ctx(&orig_chars, i),
        ctx(&tagged_chars, i),
    )
}

/// Der LLM-Output darf sich vom Original NUR durch eingefügte `[…]`-Tags
/// unterscheiden. Beide Seiten werden ohne Tag-Spans und mit normalisiertem
/// Whitespace verglichen; weichen sie ab, kommt eine kurze Diagnose zurück.
pub fn validate_tag_only_edit(original: &str, tagged: &str) -> Result<(), String> {
    let orig_norm = collapse_whitespace(&strip_tag_spans(original));
    let tagged_norm = collapse_whitespace(&strip_tag_spans(tagged));
    if orig_norm == tagged_norm {
        Ok(())
    } else {
        Err(short_diagnosis(&orig_norm, &tagged_norm))
    }
}

// ---------------------------------------------------------------------------
// Diff: validierte Zusätze -> exakte Einfüge-Positionen im Original
// ---------------------------------------------------------------------------

/// Eine vom LLM vorgeschlagene Tag-Einfügung. `offset_in_original` ist ein
/// BYTE-Offset in `original` (Rust-Konvention), `offset_chars` der
/// Unicode-Skalarwert-Offset (was `chars().count()` bis dahin liefert). Das
/// Frontend arbeitet mit UTF-16-Offsets (JS-String-Indizes) — es rechnet
/// `offset_chars` selbst um (Iteration über die Codepoints, Surrogatpaare bei
/// Zeichen jenseits der Basisebene wie Emoji zählen dort doppelt).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, specta::Type)]
pub struct TagInsertion {
    pub offset_in_original: usize,
    pub offset_chars: usize,
    pub tag: String,
}

/// Zwei-Zeiger-Lauf: weil die Nur-Einfüge-Invariante gilt (aufrufseitig durch
/// `validate_tag_only_edit` gesichert), sind alle Zusätze in `tagged`
/// `[…]`-Spans mit einer exakten Zielposition im Original. Bei einem
/// Zeichen-Unterschied, der KEIN Tag-Anfang ist, wird zunächst versucht,
/// überzähliges Whitespace (z. B. ein Leerzeichen, das das Modell zusätzlich
/// um ein neues Tag herum gesetzt hat) auf beiden Seiten unabhängig zu
/// überspringen; hilft das nicht, ist das Ergebnis ein Fehler statt eines
/// falschen Offsets.
pub fn diff_insertions(original: &str, tagged: &str) -> Result<Vec<TagInsertion>, String> {
    let orig: Vec<(usize, char)> = original.char_indices().collect();
    let tagged_chars: Vec<(usize, char)> = tagged.char_indices().collect();
    let tagged_spans = tag_spans(tagged);

    let mut insertions = Vec::new();
    let mut i = 0usize;
    let mut j = 0usize;

    while j < tagged_chars.len() {
        let (t_byte, t_char) = tagged_chars[j];
        if i < orig.len() && orig[i].1 == t_char {
            i += 1;
            j += 1;
            continue;
        }
        if t_char == '[' {
            let span = tagged_spans.iter().find(|s| s.start == t_byte).ok_or_else(|| {
                format!("diff_insertions: '[' bei Byte {t_byte} in der Antwort ist kein gültiger Tag")
            })?;
            let inner = &tagged[span.start + 1..span.end - 1];
            let offset_in_original = orig.get(i).map(|(b, _)| *b).unwrap_or(original.len());
            insertions.push(TagInsertion {
                offset_in_original,
                offset_chars: i,
                tag: inner.to_string(),
            });
            let end_byte = span.end;
            while j < tagged_chars.len() && tagged_chars[j].0 < end_byte {
                j += 1;
            }
            continue;
        }
        // Kein Tag-Anfang: überzähliges Whitespace-Polster unabhängig auf
        // beiden Seiten überspringen und erneut vergleichen.
        let mut skipped = false;
        while i < orig.len() && orig[i].1.is_whitespace() {
            i += 1;
            skipped = true;
        }
        while j < tagged_chars.len() && tagged_chars[j].1.is_whitespace() {
            j += 1;
            skipped = true;
        }
        if skipped {
            continue;
        }
        return Err(format!(
            "diff_insertions: unerwartete Abweichung bei Original-Zeichen {i} / Antwort-Zeichen {j} (weder Tag noch Whitespace) — die Nur-Einfüge-Prüfung hätte das abfangen müssen"
        ));
    }
    if i != orig.len() {
        return Err(format!(
            "diff_insertions: {} Zeichen des Originals wurden in der Antwort nicht wiedergefunden",
            orig.len() - i
        ));
    }
    Ok(insertions)
}

// ---------------------------------------------------------------------------
// Codeblock-Strip
// ---------------------------------------------------------------------------

/// Entfernt einen umschließenden Markdown-Codeblock (```` ``` ```` oder
/// ```` ```lang ````), falls die Antwort einen enthält — Modelle antworten
/// manchmal so, obwohl der System-Prompt es verbietet. Ohne Fence bleibt der
/// Text (getrimmt) unverändert.
pub fn strip_code_fence(text: &str) -> &str {
    let trimmed = text.trim();
    let Some(after_open) = trimmed.strip_prefix("```") else {
        return trimmed;
    };
    // Die erste Zeile kann eine Sprachangabe sein (z. B. "```text") — bis zum
    // ersten Zeilenumbruch überspringen, falls einer folgt.
    let after_lang = match after_open.find('\n') {
        Some(idx) => &after_open[idx + 1..],
        None => after_open,
    };
    match after_lang.rfind("```") {
        Some(idx) => after_lang[..idx].trim(),
        // Öffnende Fence ohne schließende: kein vollständiger Codeblock,
        // lieber den Originaltext (samt Fence) unverändert lassen als raten.
        None => trimmed,
    }
}

// ---------------------------------------------------------------------------
// Provider-Wahl (pure)
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub struct ResolvedProvider {
    pub provider: PostProcessProvider,
    pub model: String,
}

/// `provider_override`: `None`/`Some("")` → aktiver Post-Processing-Provider
/// (Modell aus `post_process_models`); `Some("anthropic")` → fest Claude
/// (Modell aus `tts_tag_model`). Andere Werte sind ein Fehler.
pub fn resolve_provider(
    settings: &AppSettings,
    provider_override: Option<&str>,
) -> Result<ResolvedProvider, String> {
    match provider_override {
        None | Some("") => {
            let provider = settings
                .active_post_process_provider()
                .cloned()
                .ok_or_else(|| {
                    "Kein Post-Processing-Provider konfiguriert (Einstellungen → Post Process)"
                        .to_string()
                })?;
            let model = settings
                .post_process_models
                .get(&provider.id)
                .cloned()
                .unwrap_or_default();
            if model.trim().is_empty() {
                return Err(format!(
                    "Für '{}' ist kein Modell eingetragen (Einstellungen → Nachbearbeitung → Modell).",
                    provider.label
                ));
            }
            Ok(ResolvedProvider { provider, model })
        }
        Some("anthropic") => {
            let provider = settings
                .post_process_provider("anthropic")
                .cloned()
                .ok_or_else(|| "Anthropic ist nicht als Provider registriert".to_string())?;
            let model = settings.tts_tag_model.clone();
            if model.trim().is_empty() {
                return Err("Kein Claude-Modell eingetragen (Einstellungen → Vorlesen)".to_string());
            }
            Ok(ResolvedProvider { provider, model })
        }
        Some(other) => Err(format!("Unbekannter Auto-Tagging-Provider '{other}'")),
    }
}

// ---------------------------------------------------------------------------
// Chunking: der Text wird abschnittsweise getaggt (Fortschritt + Abbruch)
// ---------------------------------------------------------------------------

/// Ein zusammenhängender Abschnitt des Originaltexts. Die Chunks überdecken
/// den Text LÜCKENLOS und in Reihenfolge — `byte_start`/`char_start` sind die
/// Offsets des Abschnittsanfangs im Gesamttext, damit die je Abschnitt
/// berechneten [`TagInsertion`]-Offsets zurück auf den Gesamttext gerechnet
/// werden können.
#[derive(Debug, PartialEq, Eq)]
pub struct Chunk<'a> {
    pub byte_start: usize,
    pub char_start: usize,
    pub text: &'a str,
}

/// Zielgröße eines Abschnitts. Klein genug für sichtbaren Fortschritt und
/// schnelle erste Tags, groß genug, dass das Modell Satzkontext hat.
pub const CHUNK_TARGET_CHARS: usize = 800;

/// Satz-/Absatzgrenzen: nach `.`/`!`/`?`/`\n` (samt direkt folgendem
/// Whitespace) darf ein Abschnitt enden. Segmente werden gierig bis
/// `max_chars` zusammengelegt; ein einzelnes überlanges Segment (Satz ohne
/// Ende) bleibt EIN Abschnitt — mitten im Satz zu schneiden würde dem Modell
/// halbe Sätze zeigen und die Tag-Qualität kosten.
pub fn split_chunks(text: &str, max_chars: usize) -> Vec<Chunk<'_>> {
    // 1. Segmentgrenzen (Byte-Offsets NACH einem Satzende samt Whitespace).
    let mut segment_ends: Vec<usize> = Vec::new();
    let mut chars = text.char_indices().peekable();
    while let Some((i, c)) = chars.next() {
        if matches!(c, '.' | '!' | '?' | '\n') {
            let mut end = i + c.len_utf8();
            while let Some(&(j, next)) = chars.peek() {
                if next.is_whitespace() {
                    chars.next();
                    end = j + next.len_utf8();
                } else {
                    break;
                }
            }
            segment_ends.push(end);
        }
    }
    if segment_ends.last() != Some(&text.len()) {
        segment_ends.push(text.len());
    }

    // 2. Segmente gierig zu Chunks zusammenlegen.
    let mut chunks = Vec::new();
    let mut start = 0usize;
    let mut char_start = 0usize;
    let mut chars_in_chunk = 0usize;
    let mut seg_start = 0usize;
    for &end in &segment_ends {
        let seg_chars = text[seg_start..end].chars().count();
        if chars_in_chunk > 0 && chars_in_chunk + seg_chars > max_chars {
            chunks.push(Chunk {
                byte_start: start,
                char_start,
                text: &text[start..seg_start],
            });
            char_start += chars_in_chunk;
            start = seg_start;
            chars_in_chunk = 0;
        }
        chars_in_chunk += seg_chars;
        seg_start = end;
    }
    if start < text.len() {
        chunks.push(Chunk {
            byte_start: start,
            char_start,
            text: &text[start..],
        });
    }
    chunks
}

// ---------------------------------------------------------------------------
// System-Prompt
// ---------------------------------------------------------------------------

/// System-Prompt: „Regisseur für Sprachsynthese", Nur-Einfüge-Regel VORAB in
/// Worten — `validate_tag_only_edit` prüft sie danach hart nach, der Prompt
/// ist die erste (nicht die einzige) Verteidigungslinie. Deutsch/Englisch
/// gemischt, damit sowohl deutsch- als auch englisch-trainierte Modelle die
/// Regeln zuverlässig befolgen.
pub fn auto_tag_system_prompt(allowed_tags: &[String]) -> String {
    let tags_list = allowed_tags.join(", ");
    format!(
        "Du bist Regisseur für Sprachsynthese. Füge in den Text Emotions-/Vortrags-Tags in \
         eckigen Klammern ein (z.B. [whisper], [excited], [sighing]). Bevorzugt diese Tags: \
         {tags_list}; kurze freie Beschreibungen sind erlaubt. Regeln: (1) AUSSCHLIESSLICH Tags \
         einfügen — kein Wort, kein Satzzeichen, keinen Zeilenumbruch ändern, nichts löschen. (2) \
         Höchstens 3 Tags pro Satz, nur wo der Inhalt es trägt; weniger ist mehr. (3) Zeilen der \
         Form Name: und Marker in Spitzklammern sind Sprecherwechsel — unverändert lassen. (4) \
         Antworte NUR mit dem Text, ohne Einleitung, ohne Codeblock."
    )
}

fn retry_system_prompt(base_prompt: &str, diagnosis: &str) -> String {
    format!(
        "{base_prompt}\n\nDeine letzte Antwort hat den Text verändert: {diagnosis}. Gib den Text \
         ERNEUT aus und füge ausschließlich Tags ein."
    )
}

// ---------------------------------------------------------------------------
// LLM-Aufruf (analog translator.rs: pure Anfrage, kein AppHandle nötig)
// ---------------------------------------------------------------------------

/// Eine Anfrage an den gewählten Provider. `cpu_only` greift nur bei einem
/// lokalen Ollama (dessen nativer Endpunkt nimmt `num_gpu: 0` entgegen) —
/// den Wert entscheidet der Aufrufer aus `tts_tag_device` ("cpu"/"gpu") bzw.
/// im Auto-Modus aus dem Live-Phasenstatus des TTS-Managers (wie bei der
/// Übersetzung: GPU nur, wenn der Fish-Speech-Server sie gerade nicht
/// braucht). Bei entfernten Anbietern (Claude, OpenAI, …) greift die
/// Bedingung ohnehin nicht (`ollama_native_url` erkennt nur lokale
/// Basis-URLs).
async fn ask_llm_for_tags(
    provider: &PostProcessProvider,
    api_key: String,
    model: &str,
    system_prompt: &str,
    user_text: &str,
    cpu_only: bool,
) -> Result<String, String> {
    if let Some(url) = llm_client::ollama_native_url(&provider.base_url) {
        let combined = format!("{system_prompt}\n\n{user_text}");
        match llm_client::send_ollama_native(&url, model, combined, cpu_only).await {
            Ok(Some(content)) if !content.trim().is_empty() => {
                return Ok(content.trim().to_string());
            }
            Ok(_) => {
                log::warn!("Ollama (nativ) ohne verwertbaren Inhalt fürs Auto-Tagging — versuche den üblichen Weg")
            }
            Err(e) => {
                log::warn!("Ollama (nativ) nicht erreichbar ({e}) — versuche den üblichen Weg")
            }
        }
    }

    match llm_client::send_chat_completion_with_schema(
        provider,
        api_key,
        model,
        user_text.to_string(),
        Some(system_prompt.to_string()),
        None,
        None,
        None,
    )
    .await
    {
        Ok(Some(content)) => {
            let trimmed = content.trim().to_string();
            if trimmed.is_empty() {
                Err("Auto-Tagging-Antwort kam leer zurück".into())
            } else {
                Ok(trimmed)
            }
        }
        Ok(None) => Err("Auto-Tagging-Antwort ohne Inhalt".into()),
        Err(e) => Err(format!("Auto-Tagging fehlgeschlagen: {e}")),
    }
}

/// Ein Versuch plus GENAU EIN Retry mit verschärftem Prompt, falls die erste
/// Antwort die Nur-Einfüge-Invariante verletzt. Scheitert auch der zweite
/// Versuch, ist das ein harter Fehler — der Text erreicht die UI nie
/// unvalidiert.
async fn tag_text_with_retry(
    provider: &PostProcessProvider,
    api_key: String,
    model: &str,
    allowed_tags: &[String],
    original: &str,
    cpu_only: bool,
) -> Result<String, String> {
    let system_prompt = auto_tag_system_prompt(allowed_tags);
    let first_raw = ask_llm_for_tags(
        provider,
        api_key.clone(),
        model,
        &system_prompt,
        original,
        cpu_only,
    )
    .await?;
    let first = strip_code_fence(&first_raw).to_string();
    if let Err(diagnosis) = validate_tag_only_edit(original, &first) {
        log::warn!("Auto-Tagging: erster Versuch verändert den Text ({diagnosis}) — ein Retry");
        let retry_prompt = retry_system_prompt(&system_prompt, &diagnosis);
        let second_raw =
            ask_llm_for_tags(provider, api_key, model, &retry_prompt, original, cpu_only).await?;
        let second = strip_code_fence(&second_raw).to_string();
        validate_tag_only_edit(original, &second).map_err(|diag2| {
            format!("Auto-Tagging hat den Text auch im zweiten Versuch verändert: {diag2}")
        })?;
        return Ok(second);
    }
    Ok(first)
}

/// Die Insertions eines Abschnitts auf den Gesamttext umrechnen.
fn shift_insertions(insertions: Vec<TagInsertion>, chunk: &Chunk<'_>) -> Vec<TagInsertion> {
    insertions
        .into_iter()
        .map(|ins| TagInsertion {
            offset_in_original: ins.offset_in_original + chunk.byte_start,
            offset_chars: ins.offset_chars + chunk.char_start,
            tag: ins.tag,
        })
        .collect()
}

/// Vollständiger Ablauf, ABSCHNITTSWEISE: Provider wählen, den Text in
/// Satz-/Absatz-Chunks teilen, je Chunk das LLM fragen (mit einem Retry),
/// validieren, die Einfüge-Positionen auf den Gesamttext umrechnen und über
/// `on_chunk` sofort melden — die UI zeigt Tags damit Abschnitt für
/// Abschnitt statt alles am Ende.
///
/// `cancel` (watch-Channel, `true` = abbrechen) wird vor jedem Abschnitt
/// geprüft UND unterbricht per `select!` eine laufende Anfrage — der Rückweg
/// ist dann `Ok` mit den bis dahin gesammelten Insertions, kein Fehler.
/// Scheitert ein einzelner Abschnitt (auch nach Retry), läuft der Rest
/// weiter; nur wenn GAR NICHTS gelang, kommt der erste Fehler zurück.
/// Entlädt ein lokales Ollama danach (best effort) — dieselbe Rücksicht auf
/// den Fish-Speech-Server wie bei der Übersetzung.
pub async fn auto_tag(
    settings: &AppSettings,
    text: &str,
    allowed_tags: &[String],
    provider_override: Option<&str>,
    cpu_only: bool,
    mut cancel: tokio::sync::watch::Receiver<bool>,
    mut on_chunk: impl FnMut(usize, usize, &[TagInsertion]),
) -> Result<Vec<TagInsertion>, String> {
    if text.trim().is_empty() {
        return Err("Kein Text zum Auto-Tagging".to_string());
    }
    let resolved = resolve_provider(settings, provider_override)?;
    let api_key = settings
        .post_process_api_keys
        .get(&resolved.provider.id)
        .cloned()
        .unwrap_or_default();

    let chunks = split_chunks(text, CHUNK_TARGET_CHARS);
    let total = chunks.len();
    let mut collected: Vec<TagInsertion> = Vec::new();
    let mut first_error: Option<String> = None;
    let mut any_ok = false;

    for (idx, chunk) in chunks.iter().enumerate() {
        if *cancel.borrow() {
            break;
        }
        if chunk.text.trim().is_empty() {
            on_chunk(idx + 1, total, &[]);
            continue;
        }
        let request = tag_text_with_retry(
            &resolved.provider,
            api_key.clone(),
            &resolved.model,
            allowed_tags,
            chunk.text,
            cpu_only,
        );
        let outcome = tokio::select! {
            r = request => Some(r),
            _ = cancel.changed() => None,
        };
        let Some(outcome) = outcome else {
            break; // abgebrochen — das Gesammelte ist das Ergebnis
        };
        match outcome.and_then(|tagged| diff_insertions(chunk.text, &tagged)) {
            Ok(insertions) => {
                any_ok = true;
                let shifted = shift_insertions(insertions, chunk);
                on_chunk(idx + 1, total, &shifted);
                collected.extend(shifted);
            }
            Err(e) => {
                log::warn!(
                    "Auto-Tagging: Abschnitt {}/{} übersprungen: {e}",
                    idx + 1,
                    total
                );
                first_error.get_or_insert(e);
                on_chunk(idx + 1, total, &[]);
            }
        }
    }

    // Best effort, auch nach Fehler/Abbruch — ein abgebrochener Lauf ließe
    // das Modell sonst geladen stehen (siehe translator::translate_on).
    llm_client::ollama_unload(&resolved.provider.base_url, &resolved.model).await;

    match first_error {
        Some(e) if !any_ok => Err(e),
        _ => Ok(collected),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::get_default_settings;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    // ---- validate_tag_only_edit: akzeptiert reine Tag-Einfügungen --------

    #[test]
    fn ein_einzelnes_eingefuegtes_tag_wird_akzeptiert() {
        let original = "Hallo Welt.";
        let tagged = "Hallo [whisper] Welt.";
        assert!(validate_tag_only_edit(original, tagged).is_ok());
    }

    #[test]
    fn mehrere_eingefuegte_tags_werden_akzeptiert() {
        let original = "Erster Satz. Zweiter Satz. Dritter Satz.";
        let tagged = "[excited] Erster Satz. [whisper] Zweiter Satz. Dritter Satz. [sighing]";
        assert!(validate_tag_only_edit(original, tagged).is_ok());
    }

    #[test]
    fn tag_am_zeilenanfang_wird_akzeptiert() {
        let original = "Guten Morgen.";
        let tagged = "[cheerful] Guten Morgen.";
        assert!(validate_tag_only_edit(original, tagged).is_ok());
    }

    #[test]
    fn tag_am_zeilenende_wird_akzeptiert() {
        let original = "Bis später.";
        let tagged = "Bis später. [waving]";
        assert!(validate_tag_only_edit(original, tagged).is_ok());
    }

    #[test]
    fn umlaute_bleiben_beim_validieren_unangetastet() {
        let original = "Schöne Grüße nach München, äöüÄÖÜß.";
        let tagged = "[cheerful] Schöne Grüße nach München, äöüÄÖÜß.";
        assert!(validate_tag_only_edit(original, tagged).is_ok());
    }

    #[test]
    fn emoji_im_text_bleiben_beim_validieren_unangetastet() {
        let original = "Das war lustig 😀 wirklich.";
        let tagged = "Das war lustig 😀 [laughing] wirklich.";
        assert!(validate_tag_only_edit(original, tagged).is_ok());
    }

    // ---- validate_tag_only_edit: lehnt echte Textänderungen ab -----------

    #[test]
    fn wortaenderung_wird_abgelehnt() {
        let original = "Hallo Welt.";
        let tagged = "Hallo Erde.";
        assert!(validate_tag_only_edit(original, tagged).is_err());
    }

    #[test]
    fn woertloeschung_wird_abgelehnt() {
        let original = "Hallo schöne Welt.";
        let tagged = "Hallo Welt.";
        assert!(validate_tag_only_edit(original, tagged).is_err());
    }

    #[test]
    fn wortumstellung_wird_abgelehnt() {
        let original = "Ich gehe heute spazieren.";
        let tagged = "Heute gehe ich spazieren.";
        assert!(validate_tag_only_edit(original, tagged).is_err());
    }

    #[test]
    fn zeilenumbruch_aenderung_wird_abgelehnt() {
        let original = "Zeile eins.\nZeile zwei.";
        let tagged = "Zeile eins. Zeile zwei."; // \n durch Leerzeichen ersetzt
        assert!(validate_tag_only_edit(original, tagged).is_err());
    }

    #[test]
    fn fehlermeldung_ist_kurz_und_nennt_die_stelle() {
        let err = validate_tag_only_edit("Hallo Welt.", "Hallo Erde.").unwrap_err();
        assert!(err.len() < 200, "Diagnose soll kurz sein: {err}");
        assert!(err.contains("Zeichenposition"));
    }

    // ---- diff_insertions: Positionstreue (Byte + char) --------------------

    #[test]
    fn diff_insertions_findet_position_mitten_im_text() {
        let original = "Hallo Welt.";
        let tagged = "Hallo [whisper] Welt.";
        let insertions = diff_insertions(original, tagged).unwrap();
        assert_eq!(insertions.len(), 1);
        assert_eq!(insertions[0].tag, "whisper");
        assert_eq!(insertions[0].offset_in_original, 6, "Byte-Offset vor 'W'");
        assert_eq!(insertions[0].offset_chars, 6, "Char-Offset vor 'W'");
    }

    #[test]
    fn diff_insertions_findet_position_am_zeilenanfang_und_ende() {
        let original = "Text.";
        let tagged = "[cheerful] Text. [waving]";
        let insertions = diff_insertions(original, tagged).unwrap();
        assert_eq!(insertions.len(), 2);
        assert_eq!(insertions[0].tag, "cheerful");
        assert_eq!(insertions[0].offset_in_original, 0);
        assert_eq!(insertions[0].offset_chars, 0);
        assert_eq!(insertions[1].tag, "waving");
        assert_eq!(insertions[1].offset_in_original, original.len());
        assert_eq!(insertions[1].offset_chars, original.chars().count());
    }

    #[test]
    fn diff_insertions_rechnet_byte_offset_ueber_umlaute_richtig() {
        // "Schöne " hat 7 Zeichen, aber 8 Bytes ('ö' ist 2 Bytes in UTF-8).
        let original = "Schöne Grüße.";
        let tagged = "Schöne [warmly] Grüße.";
        let insertions = diff_insertions(original, tagged).unwrap();
        assert_eq!(insertions.len(), 1);
        assert_eq!(
            insertions[0].offset_chars, 7,
            "7 Zeichen vor der Einfuegestelle"
        );
        assert_eq!(
            insertions[0].offset_in_original, 8,
            "8 Bytes, weil 'ö' 2 Bytes belegt"
        );
        assert_eq!(&original[..insertions[0].offset_in_original], "Schöne ");
    }

    #[test]
    fn diff_insertions_zaehlt_emoji_als_ein_zeichen_aber_vier_bytes() {
        // "😀" (U+1F600) ist im Original EIN char (Unicode-Skalarwert), aber
        // 4 Bytes in UTF-8 und — fuers Frontend relevant — 2 UTF-16-Einheiten
        // (Surrogatpaar). diff_insertions liefert den ROH-char-Offset; die
        // UTF-16-Umrechnung ist Aufgabe des Frontends (dokumentiert am Typ).
        let original = "Hallo 😀 Welt.";
        let tagged = "Hallo 😀 [laughing] Welt.";
        let insertions = diff_insertions(original, tagged).unwrap();
        assert_eq!(insertions.len(), 1);
        // "Hallo " (6) + "😀" (1 char) + " " (1) = 8 chars vor der Stelle.
        assert_eq!(insertions[0].offset_chars, 8);
        // "Hallo " (6 Bytes) + "😀" (4 Bytes) + " " (1 Byte) = 11 Bytes.
        assert_eq!(insertions[0].offset_in_original, 11);
        assert_eq!(&original[..insertions[0].offset_in_original], "Hallo 😀 ");
    }

    #[test]
    fn diff_insertions_toleriert_leerzeichen_polster_um_ein_neues_tag() {
        // Realistischer Modell-Output: das Leerzeichen zwischen "Hallo" und
        // "Welt" bleibt bestehen, plus ein zusaetzliches nach dem Tag.
        let original = "Hallo Welt.";
        let tagged = "Hallo [whisper]  Welt."; // zwei Leerzeichen nach dem Tag
        let insertions = diff_insertions(original, tagged).unwrap();
        assert_eq!(insertions.len(), 1);
        assert_eq!(insertions[0].tag, "whisper");
        assert_eq!(insertions[0].offset_in_original, 6);
    }

    #[test]
    fn diff_insertions_meldet_einen_fehler_statt_falscher_offsets_bei_echter_abweichung() {
        // Ohne vorherige Validierung kann diff_insertions auf eine echte
        // Textabweichung treffen — dann lieber Err als ein falscher Offset.
        let original = "Hallo Welt.";
        let tagged = "Hallo Erde.";
        assert!(diff_insertions(original, tagged).is_err());
    }

    // ---- Codeblock-Strip ---------------------------------------------------

    #[test]
    fn codeblock_ohne_sprachangabe_wird_entfernt() {
        let raw = "```\nHallo [whisper] Welt.\n```";
        assert_eq!(strip_code_fence(raw), "Hallo [whisper] Welt.");
    }

    #[test]
    fn codeblock_mit_sprachangabe_wird_entfernt() {
        let raw = "```text\nHallo [whisper] Welt.\n```";
        assert_eq!(strip_code_fence(raw), "Hallo [whisper] Welt.");
    }

    #[test]
    fn text_ohne_codeblock_bleibt_unveraendert() {
        let raw = "Hallo [whisper] Welt.";
        assert_eq!(strip_code_fence(raw), "Hallo [whisper] Welt.");
    }

    // ---- Provider-Wahl (pure) ----------------------------------------------

    #[test]
    fn ohne_override_wird_der_aktive_post_process_provider_gewaehlt() {
        let mut settings = get_default_settings();
        settings
            .post_process_models
            .insert(settings.post_process_provider_id.clone(), "gpt-test".into());
        let resolved = resolve_provider(&settings, None).unwrap();
        assert_eq!(resolved.provider.id, settings.post_process_provider_id);
        assert_eq!(resolved.model, "gpt-test");
    }

    #[test]
    fn ohne_konfiguriertes_modell_ist_das_ein_verstaendlicher_fehler() {
        // Frischer Default-Zustand: ein Provider ist aktiv, aber ohne
        // Modell eingetragen — derselbe Fehlerpfad wie translator::translate_on.
        let settings = get_default_settings();
        let err = resolve_provider(&settings, None).unwrap_err();
        assert!(err.contains("kein Modell eingetragen"), "war: {err}");
        assert!(err.contains("'OpenAI'"), "war: {err}");
    }

    #[test]
    fn override_anthropic_waehlt_claude_mit_tts_tag_model() {
        let settings = get_default_settings();
        let resolved = resolve_provider(&settings, Some("anthropic")).unwrap();
        assert_eq!(resolved.provider.id, "anthropic");
        assert_eq!(resolved.model, settings.tts_tag_model);
    }

    #[test]
    fn unbekannter_override_ist_ein_fehler() {
        let settings = get_default_settings();
        let err = resolve_provider(&settings, Some("does-not-exist")).unwrap_err();
        assert!(err.contains("does-not-exist"));
    }

    #[test]
    fn leerer_override_verhaelt_sich_wie_kein_override() {
        let mut settings = get_default_settings();
        settings
            .post_process_models
            .insert(settings.post_process_provider_id.clone(), "gpt-test".into());
        let none = resolve_provider(&settings, None).unwrap();
        let empty = resolve_provider(&settings, Some("")).unwrap();
        assert_eq!(none.provider.id, empty.provider.id);
        assert_eq!(none.model, empty.model);
    }

    // ---- LLM-Pfad mit Mock-Server (translator.rs-Testmuster) --------------

    /// Mock eines OpenAI-kompatiblen `/chat/completions`-Endpunkts, der
    /// IMMER dieselbe (hier: kaputte) Antwort liefert — unabhängig vom
    /// angefragten Pfad, damit auch der native Ollama-Versuch (der zuerst
    /// `/api/chat` anfragt und bei unpassendem JSON klaglos auf den üblichen
    /// Weg zurückfällt) keine eigene Route braucht.
    async fn spawn_llm_mock(reply: &'static str) -> u16 {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            loop {
                let Ok((mut sock, _)) = listener.accept().await else {
                    break;
                };
                tokio::spawn(async move {
                    let mut buf = vec![0u8; 65536];
                    let mut read = 0usize;
                    loop {
                        let n = sock.read(&mut buf[read..]).await.unwrap_or(0);
                        if n == 0 {
                            break;
                        }
                        read += n;
                        let text = String::from_utf8_lossy(&buf[..read]).to_lowercase();
                        if let Some(header_end) = text.find("\r\n\r\n") {
                            let content_length = text
                                .lines()
                                .find_map(|l| l.strip_prefix("content-length: "))
                                .and_then(|v| v.trim().parse::<usize>().ok())
                                .unwrap_or(0);
                            if read >= header_end + 4 + content_length {
                                let escaped = reply.replace('"', "\\\"");
                                let body = format!(
                                    r#"{{"choices":[{{"message":{{"role":"assistant","content":"{escaped}"}}}}]}}"#
                                );
                                let head = format!(
                                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n",
                                    body.len()
                                );
                                let _ = sock.write_all(head.as_bytes()).await;
                                let _ = sock.write_all(body.as_bytes()).await;
                                let _ = sock.shutdown().await;
                                break;
                            }
                        }
                    }
                });
            }
        });
        port
    }

    fn settings_with_mock_provider(port: u16) -> AppSettings {
        let mut settings = get_default_settings();
        settings.post_process_provider_id = "custom".into();
        if let Some(custom) = settings.post_process_provider_mut("custom") {
            custom.base_url = format!("http://127.0.0.1:{port}/v1");
        }
        settings
            .post_process_models
            .insert("custom".into(), "test-model".into());
        settings
    }

    /// Ein `auto_tag`-Aufruf mit den Standard-Nebenparametern der Tests:
    /// CPU-only, nie abgebrochen, Fortschritt verworfen. Der Sender bleibt
    /// bis nach dem Await am Leben — ein gedroppter watch-Sender gälte als
    /// Abbruch.
    async fn run_auto_tag(
        settings: &AppSettings,
        text: &str,
        tags: &[String],
    ) -> Result<Vec<TagInsertion>, String> {
        let (tx, rx) = tokio::sync::watch::channel(false);
        let result = auto_tag(settings, text, tags, None, true, rx, |_, _, _| {}).await;
        drop(tx);
        result
    }

    #[tokio::test]
    async fn ein_guter_llm_output_wird_zu_insertions() {
        let port = spawn_llm_mock("Hallo [whisper] Welt.").await;
        let settings = settings_with_mock_provider(port);
        let insertions = run_auto_tag(&settings, "Hallo Welt.", &["whisper".to_string()])
            .await
            .unwrap();
        assert_eq!(insertions.len(), 1);
        assert_eq!(insertions[0].tag, "whisper");
    }

    #[tokio::test]
    async fn ein_boeser_output_bekommt_genau_einen_retry_und_scheitert_dann() {
        // Der Mock antwortet bei JEDER Anfrage (erster Versuch UND Retry)
        // mit derselben textverändernden Antwort — die Nur-Einfüge-Prüfung
        // muss beide Male ablehnen und danach hart fehlschlagen, statt einen
        // dritten Versuch zu starten.
        let port = spawn_llm_mock("Hallo Erde.").await;
        let settings = settings_with_mock_provider(port);
        let err = run_auto_tag(&settings, "Hallo Welt.", &["whisper".to_string()])
            .await
            .unwrap_err();
        assert!(
            err.contains("zweiten Versuch"),
            "Fehlermeldung soll den zweiten Versuch nennen (Beleg fuer genau einen Retry): {err}"
        );
    }

    #[tokio::test]
    async fn ein_codeblock_um_die_antwort_wird_vor_dem_validieren_entfernt() {
        let port = spawn_llm_mock("```\\nHallo [whisper] Welt.\\n```").await;
        let settings = settings_with_mock_provider(port);
        let insertions = run_auto_tag(&settings, "Hallo Welt.", &["whisper".to_string()])
            .await
            .unwrap();
        assert_eq!(insertions.len(), 1);
        assert_eq!(insertions[0].tag, "whisper");
    }

    // ---- Chunking ----------------------------------------------------------

    /// Grundinvariante: Chunks überdecken den Text lückenlos, in Reihenfolge,
    /// und ihre byte-/char-Offsets stimmen mit der Lage im Gesamttext überein.
    fn assert_chunks_cover(text: &str, chunks: &[Chunk<'_>]) {
        let mut byte = 0usize;
        let mut chars = 0usize;
        for chunk in chunks {
            assert_eq!(chunk.byte_start, byte, "Byte-Offset lückenlos");
            assert_eq!(chunk.char_start, chars, "Char-Offset lückenlos");
            assert_eq!(&text[byte..byte + chunk.text.len()], chunk.text);
            byte += chunk.text.len();
            chars += chunk.text.chars().count();
        }
        assert_eq!(byte, text.len(), "alles überdeckt");
    }

    #[test]
    fn kurzer_text_ist_ein_einziger_chunk() {
        let text = "Hallo Welt. Wie geht es dir?";
        let chunks = split_chunks(text, 800);
        assert_eq!(chunks.len(), 1);
        assert_chunks_cover(text, &chunks);
    }

    #[test]
    fn langer_text_wird_an_satzgrenzen_geteilt() {
        let text = "Erster Satz. Zweiter Satz! Dritter Satz? Vierter Satz.";
        let chunks = split_chunks(text, 20);
        assert!(chunks.len() >= 2, "bei max 20 Zeichen muss geteilt werden");
        assert_chunks_cover(text, &chunks);
        for chunk in &chunks[..chunks.len() - 1] {
            let trimmed = chunk.text.trim_end();
            assert!(
                trimmed.ends_with('.') || trimmed.ends_with('!') || trimmed.ends_with('?'),
                "Chunk endet an einer Satzgrenze: {:?}",
                chunk.text
            );
        }
    }

    #[test]
    fn ein_ueberlanger_satz_bleibt_ein_chunk() {
        // Kein Satzende → keine Schnittstelle → ein Chunk, egal wie lang.
        let text = "wort ".repeat(50);
        let chunks = split_chunks(&text, 20);
        assert_eq!(chunks.len(), 1);
        assert_chunks_cover(&text, &chunks);
    }

    #[test]
    fn umlaute_verschieben_die_chunk_offsets_nicht() {
        let text = "Schöne Grüße. Ähnliche Öfen! Übrige Worte.";
        let chunks = split_chunks(text, 15);
        assert!(chunks.len() >= 2);
        assert_chunks_cover(text, &chunks);
    }

    #[test]
    fn shift_insertions_rechnet_auf_den_gesamttext_um() {
        let text = "Erster Satz. Zweiter Satz.";
        let chunks = split_chunks(text, 13);
        assert_eq!(chunks.len(), 2);
        let second = &chunks[1];
        // Im Chunk "Zweiter Satz." eine Einfügung an Position 0.
        let local = vec![TagInsertion {
            offset_in_original: 0,
            offset_chars: 0,
            tag: "whisper".into(),
        }];
        let shifted = shift_insertions(local, second);
        assert_eq!(shifted[0].offset_in_original, second.byte_start);
        assert_eq!(shifted[0].offset_chars, second.char_start);
        assert_eq!(&text[shifted[0].offset_in_original..], "Zweiter Satz.");
    }

    // ---- Abbruch -----------------------------------------------------------

    #[tokio::test]
    async fn ein_vorab_gesetzter_abbruch_liefert_ok_und_leer_ohne_anfrage() {
        // Kein Mock-Server: wäre der Abbruch wirkungslos, liefe die Anfrage
        // gegen eine tote Adresse und der Test käme mit einem FEHLER zurück.
        let settings = settings_with_mock_provider(1); // Port 1: nichts lauscht
        let (tx, rx) = tokio::sync::watch::channel(true);
        let result = auto_tag(
            &settings,
            "Hallo Welt.",
            &["whisper".to_string()],
            None,
            true,
            rx,
            |_, _, _| {},
        )
        .await;
        drop(tx);
        assert_eq!(result.unwrap(), Vec::new());
    }

    #[tokio::test]
    async fn der_fortschritt_meldet_jeden_abschnitt_mit_seinen_insertions() {
        let port = spawn_llm_mock("Hallo [whisper] Welt.").await;
        let settings = settings_with_mock_provider(port);
        let mut reported: Vec<(usize, usize, usize)> = Vec::new();
        let (tx, rx) = tokio::sync::watch::channel(false);
        let result = auto_tag(
            &settings,
            "Hallo Welt.",
            &["whisper".to_string()],
            None,
            true,
            rx,
            |done, total, ins| reported.push((done, total, ins.len())),
        )
        .await
        .unwrap();
        drop(tx);
        assert_eq!(reported, vec![(1, 1, 1)]);
        assert_eq!(result.len(), 1);
    }
}
