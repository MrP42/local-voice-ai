//! Vorbereitung vor dem Termin (G4, #499): zwei bis drei offene Punkte aus den
//! EIGENEN frueheren Besprechungsnotizen, jeder mit Quelle (Besprechung + Datum).
//!
//! Baut auf dem Brief (`brief.rs`) auf: dieselbe Zuordnung Termin-Teilnehmende ->
//! Personen (`event_people`), keine zweite Personenlogik. Der Brief ist das
//! ausfuehrliche Gespraech ueber den Chat; die Vorbereitung ist die kurze Karte
//! am Termin.
//!
//! Ablauf (alles lokal, kein Netz ausser dem gewaehlten Sprachmodell):
//! 1. Auswahl der frueheren Besprechungen: fertig, nicht geloescht, im
//!    Zeitfenster ([`WINDOW_DAYS`]), nicht zu genau diesem Termin aufgenommen,
//!    und entweder aus derselben Terminserie oder mit mindestens der Haelfte der
//!    bekannten Teilnehmenden ([`pick_meetings`]).
//! 2. Kandidaten: offene Aufgaben (`action_items`), nicht abgehakte Aufgaben im
//!    eigenen Notizblock und die Abschnitte „Offene Fragen“ der KI-Notizen.
//!    Erledigtes in einer neueren Besprechung schliesst Gleiches in einer aelteren
//!    ([`rank_open`]).
//! 3. Rangfolge ohne Modell ([`rank_open`]): zugesagte Lieferung vor Aufgabe vor
//!    Frage, Serie vor Teilnehmenden, hoechstens zwei je Besprechung.
//! 4. Mit Modell (`commands/meeting_prep.rs`): das Modell sieht NUR die
//!    Kandidatenzeilen (keine Transkripte, keine Namen der Teilnehmenden) und
//!    waehlt/kuerzt; die Quelle kommt immer vom Kandidaten, nie vom Modell
//!    ([`apply_reply`]). Ohne Modell, bei Sperre durch die Compliance-Regeln oder
//!    bei unbrauchbarer Antwort gilt Schritt 3 unveraendert (Text ohne
//!    Umformulierung).
//!
//! Datenschutz (ADR-0014): die Vorbereitung ist an den Termin gebunden. Sie wird
//! weder gespeichert noch je Person gezaehlt oder zusammengefasst; der Zwischenspeicher
//! ([`cache_get`]) liegt nur im Arbeitsspeicher, ist je Termin getrennt und hat
//! keinen Personenbezug. Logzeilen nennen nie Notiz- oder Besprechungstexte.
//!
//! Dieses Modul kennt weder Tauri noch Einstellungen noch das Netz.

use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::sync::Mutex;

use anyhow::Result;
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use specta::Type;

use crate::managers::calendar::model::CalEvent;
use crate::managers::meetings::notes::model::{
    EnhancedNotes, NoteBlockKind, SectionKind, STATUS_DONE,
};
use crate::managers::meetings::store::MeetingStore;

use super::brief::event_people;

/// Wie viele Punkte die Karte hoechstens zeigt.
pub const MAX_POINTS: usize = 3;
/// Wie weit zurueck frueherer Besprechungen gelten (Tage).
pub const WINDOW_DAYS: i64 = 180;
/// So viele fruehere Besprechungen gehen hoechstens in die Auswahl.
pub const MAX_MEETINGS: usize = 8;
/// So viele Kandidaten sieht das Modell hoechstens.
pub const MAX_CANDIDATES: usize = 12;
/// Laenge eines Punktes in Zeichen.
pub const MAX_POINT_CHARS: usize = 160;
/// Hoechstens so viele Punkte aus derselben Besprechung (Vielfalt).
const MAX_PER_MEETING: usize = 2;
/// Laenge eines Kandidatentextes beim Einsammeln.
const MAX_ITEM_CHARS: usize = 300;
/// Abschnitte der KI-Notizen mit ungeklaerten Fragen (Vorlagen-Kennungen).
const OPEN_SECTION_IDS: [&str; 2] = ["offene_fragen", "offene_punkte_bedenken"];
/// Zeichen eines Titels im Modell-Prompt.
const MAX_TITLE_CHARS: usize = 80;

// ---------------------------------------------------------------------------
// Ergebnis (geht an die Oberflaeche)
// ---------------------------------------------------------------------------

/// Art eines offenen Punktes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum PrepKind {
    /// Offene Aufgabe.
    Task,
    /// Ungeklaerte Frage.
    Question,
    /// Zugesagte Lieferung.
    Delivery,
}

impl PrepKind {
    fn weight(self) -> i32 {
        match self {
            PrepKind::Delivery => 30,
            PrepKind::Task => 20,
            PrepKind::Question => 10,
        }
    }

    fn label(self) -> &'static str {
        match self {
            PrepKind::Delivery => "promised delivery",
            PrepKind::Task => "task",
            PrepKind::Question => "open question",
        }
    }
}

/// Woher ein Punkt stammt: die fruehere Besprechung (Sprung dorthin).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct PrepSource {
    pub meeting_id: String,
    pub title: String,
    /// Zeitpunkt der Besprechung in Sekunden.
    pub at: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct PrepPoint {
    pub text: String,
    pub kind: PrepKind,
    pub source: PrepSource,
}

/// Womit die Punkte entstanden sind.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum PrepMode {
    /// Das gewaehlte Sprachmodell hat ausgewaehlt und gekuerzt.
    Model,
    /// Regelbasiert: Texte aus den Notizen ohne Umformulierung.
    Rules,
}

/// Die Vorbereitung zu einem Termin. `points` leer = nichts anzuzeigen.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct Prep {
    pub points: Vec<PrepPoint>,
    pub mode: PrepMode,
    /// Wie viele fruehere Besprechungen in die Auswahl kamen.
    pub meetings: u32,
}

impl Prep {
    pub fn empty() -> Self {
        Self {
            points: Vec::new(),
            mode: PrepMode::Rules,
            meetings: 0,
        }
    }
}

// ---------------------------------------------------------------------------
// Auswahl der frueheren Besprechungen (rein)
// ---------------------------------------------------------------------------

/// Eine fruehere Besprechung als Kandidat fuer die Vorbereitung.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PastMeeting {
    pub id: String,
    pub title: String,
    /// Zeitpunkt in Sekunden.
    pub at: i64,
    /// Zur selben Terminserie (UID) verknuepft.
    pub series: bool,
    /// Wie viele der bekannten Teilnehmenden des Termins dabei waren.
    pub shared_people: u32,
}

/// Wie viele der bekannten Teilnehmenden eine Besprechung (ohne Serie) mit dem
/// Termin teilen muss: die Haelfte, aufgerundet, mindestens eine.
pub fn people_needed(other_known: u32) -> u32 {
    other_known.div_ceil(2).max(1)
}

/// Zeitfenster, Relevanz, Reihenfolge (neueste zuerst) und Obergrenze.
pub fn pick_meetings(mut all: Vec<PastMeeting>, now_s: i64, other_known: u32) -> Vec<PastMeeting> {
    let from = now_s - WINDOW_DAYS * 86_400;
    let needed = people_needed(other_known);
    all.retain(|m| m.at >= from && m.at <= now_s && (m.series || m.shared_people >= needed));
    all.sort_by(|a, b| b.at.cmp(&a.at).then_with(|| b.id.cmp(&a.id)));
    all.dedup_by(|a, b| a.id == b.id);
    all.truncate(MAX_MEETINGS);
    all
}

// ---------------------------------------------------------------------------
// Kandidaten und Rangfolge (rein)
// ---------------------------------------------------------------------------

/// Ein Punkt aus den Notizen einer Besprechung.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Candidate {
    pub kind: PrepKind,
    pub text: String,
    pub assignee: Option<String>,
    pub due: Option<String>,
    pub done: bool,
}

/// Die Besprechung samt ihren Punkten.
#[derive(Clone, Debug)]
pub struct Material {
    pub meeting: PastMeeting,
    pub items: Vec<Candidate>,
}

/// Ein offener Punkt in Rangfolge.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ranked {
    pub source: PrepSource,
    pub kind: PrepKind,
    /// Der Text, wie er ohne Modell erscheint (Zustaendige und Termin angehaengt).
    pub display: String,
    pub series: bool,
    pub score: i32,
    key: String,
}

/// Einzeilig, Leerraum zusammengefasst, Aufzaehlungszeichen vorne entfernt.
pub fn clean_text(raw: &str) -> String {
    let flat = raw.split_whitespace().collect::<Vec<_>>().join(" ");
    let trimmed = flat
        .trim_start_matches(|c: char| matches!(c, '-' | '*' | '•' | '–' | '#') || c.is_whitespace())
        .trim();
    trimmed.to_string()
}

/// Kuerzt auf hoechstens `max` Zeichen an einer Wortgrenze, mit „…“.
pub fn clip(text: &str, max: usize) -> String {
    let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() <= max {
        return flat;
    }
    let cut: String = flat.chars().take(max.saturating_sub(1)).collect();
    let base = match cut.rfind(' ') {
        Some(i) if i > max / 2 => &cut[..i],
        _ => cut.as_str(),
    };
    format!("{}…", base.trim_end_matches([',', ';', ':', '.', ' ']))
}

/// Vergleichsform: Kleinbuchstaben, nur Buchstaben und Ziffern, Leerraum eins.
fn key_of(text: &str) -> String {
    text.chars()
        .map(|c| {
            if c.is_alphanumeric() {
                c.to_lowercase().next().unwrap_or(c)
            } else {
                ' '
            }
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Eine Aufgabe, die etwas Zugesagtes zu liefern meint, zaehlt als Lieferung.
pub fn classify_task(text: &str) -> PrepKind {
    const MARKERS: [&str; 10] = [
        "zusage",
        "zugesagt",
        "liefer",
        "zusend",
        "zuschick",
        "nachreich",
        "bereitstell",
        "übermitt",
        "uebermitt",
        "schick",
    ];
    let lower = text.to_lowercase();
    if MARKERS.iter().any(|m| lower.contains(m)) {
        PrepKind::Delivery
    } else {
        PrepKind::Task
    }
}

fn contains_ci(haystack: &str, needle: &str) -> bool {
    haystack.to_lowercase().contains(&needle.to_lowercase())
}

/// Der Text eines Punktes ohne Modell: der Notiztext, dazu (nur falls er sie
/// nicht schon nennt) die Zustaendigen vorn und der Termin hinten. Kein
/// Umformulieren.
pub fn display_text(c: &Candidate) -> String {
    let mut text = c.text.clone();
    if let Some(who) = c
        .assignee
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        if !contains_ci(&text, who) {
            text = format!("{who}: {text}");
        }
    }
    if let Some(due) = c.due.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        if !contains_ci(&text, due) {
            text = format!("{text} (bis {due})");
        }
    }
    clip(&text, MAX_POINT_CHARS)
}

/// Alle offenen Punkte der Besprechungen in Rangfolge (beste zuerst). Ohne
/// Modell geht diese Liste unveraendert an die Karte.
///
/// - Neueste Besprechung zuerst durchgehen: ein erledigter Punkt schliesst den
///   gleichen Text in dieser und in aelteren Besprechungen; ein offener Punkt
///   zaehlt nur einmal (die neueste Fassung).
/// - Wertung: Art (Lieferung > Aufgabe > Frage), Serie, Termin gesetzt;
///   Gleichstand entscheidet die juengere Besprechung.
pub fn rank_open(materials: &[Material]) -> Vec<Ranked> {
    let mut order: Vec<&Material> = materials.iter().collect();
    order.sort_by(|a, b| {
        b.meeting
            .at
            .cmp(&a.meeting.at)
            .then_with(|| b.meeting.id.cmp(&a.meeting.id))
    });
    let mut closed: HashSet<String> = HashSet::new();
    let mut seen: HashSet<String> = HashSet::new();
    let mut out: Vec<Ranked> = Vec::new();
    for m in order {
        for c in m.items.iter().filter(|c| c.done) {
            closed.insert(key_of(&c.text));
        }
        for c in m.items.iter().filter(|c| !c.done) {
            let key = key_of(&c.text);
            if key.is_empty() || closed.contains(&key) || !seen.insert(key.clone()) {
                continue;
            }
            let has_due = c.due.as_deref().is_some_and(|d| !d.trim().is_empty());
            let score =
                c.kind.weight() + if m.meeting.series { 15 } else { 0 } + i32::from(has_due) * 5;
            out.push(Ranked {
                source: PrepSource {
                    meeting_id: m.meeting.id.clone(),
                    title: m.meeting.title.clone(),
                    at: m.meeting.at,
                },
                kind: c.kind,
                display: display_text(c),
                series: m.meeting.series,
                score,
                key,
            });
        }
    }
    // Stabil: bei Gleichstand bleibt die Reihenfolge (juengere Besprechung zuerst).
    out.sort_by(|a, b| b.score.cmp(&a.score));
    out
}

/// Die ersten `n` Punkte, hoechstens [`MAX_PER_MEETING`] je Besprechung.
pub fn top_points(ranked: &[Ranked], n: usize) -> Vec<&Ranked> {
    let mut per: HashMap<&str, usize> = HashMap::new();
    let mut out = Vec::new();
    for r in ranked {
        if out.len() >= n {
            break;
        }
        let used = per.entry(r.source.meeting_id.as_str()).or_insert(0);
        if *used >= MAX_PER_MEETING {
            continue;
        }
        *used += 1;
        out.push(r);
    }
    out
}

fn point_of(r: &Ranked, text: String) -> PrepPoint {
    PrepPoint {
        text,
        kind: r.kind,
        source: r.source.clone(),
    }
}

// ---------------------------------------------------------------------------
// Eingabe aus dem Store
// ---------------------------------------------------------------------------

/// Was ein Lauf braucht: Kandidaten in Rangfolge und ein Merkmal, an dem sich
/// zeigt, ob sich seit dem letzten Lauf etwas geaendert hat.
#[derive(Clone, Debug)]
pub struct PrepInput {
    pub ranked: Vec<Ranked>,
    pub meetings: u32,
    pub fingerprint: String,
}

impl PrepInput {
    /// Nichts zu zeigen: kein Modell noetig.
    pub fn is_empty(&self) -> bool {
        self.ranked.is_empty()
    }
}

fn fingerprint(event_key: &str, ranked: &[Ranked]) -> String {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    event_key.hash(&mut h);
    for r in ranked {
        r.source.meeting_id.hash(&mut h);
        r.key.hash(&mut h);
    }
    format!("{:016x}", h.finish())
}

/// Fruehere Besprechungen als Kandidaten: aus derselben Terminserie oder mit
/// Teilnehmenden des Termins. Fertig, nicht geloescht, im Zeitfenster, nicht zu
/// genau diesem Termin aufgenommen.
fn past_meetings(
    conn: &Connection,
    event: &CalEvent,
    human_ids: &[String],
    now_s: i64,
) -> Result<Vec<PastMeeting>> {
    let from = now_s - WINDOW_DAYS * 86_400;
    let mut by_id: HashMap<String, PastMeeting> = HashMap::new();

    if !event.uid.is_empty() {
        let mut stmt = conn.prepare(
            "SELECT m.id, m.title, COALESCE(m.started_at, m.created_at)
             FROM meetings m JOIN meeting_calendar_links l ON l.meeting_id = m.id
             WHERE l.uid = ?1 AND l.event_start <> ?2
               AND (l.event_key IS NULL OR l.event_key <> ?3)
               AND m.deleted_at IS NULL AND m.status = 'ready'
               AND COALESCE(m.started_at, m.created_at) BETWEEN ?4 AND ?5",
        )?;
        let rows = stmt.query_map(
            params![event.uid, event.starts_at, event.key, from, now_s],
            |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, i64>(2)?,
                ))
            },
        )?;
        for row in rows {
            let (id, title, at) = row?;
            by_id.insert(
                id.clone(),
                PastMeeting {
                    id,
                    title,
                    at,
                    series: true,
                    shared_people: 0,
                },
            );
        }
    }

    if !human_ids.is_empty() {
        let mut stmt = conn.prepare(
            "SELECT m.id, m.title, COALESCE(m.started_at, m.created_at),
                    COUNT(DISTINCT p.human_id)
             FROM meetings m JOIN meeting_participants p ON p.meeting_id = m.id
             WHERE p.human_id IN (SELECT value FROM json_each(?1))
               AND m.deleted_at IS NULL AND m.status = 'ready'
               AND m.id NOT IN (SELECT meeting_id FROM meeting_calendar_links
                                WHERE event_key = ?2)
               AND COALESCE(m.started_at, m.created_at) BETWEEN ?3 AND ?4
             GROUP BY m.id
             ORDER BY 3 DESC, m.id DESC
             LIMIT 50",
        )?;
        let rows = stmt.query_map(
            params![serde_json::to_string(human_ids)?, event.key, from, now_s],
            |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, i64>(2)?,
                    r.get::<_, i64>(3)?,
                ))
            },
        )?;
        for row in rows {
            let (id, title, at, shared) = row?;
            by_id
                .entry(id.clone())
                .and_modify(|m| m.shared_people = shared as u32)
                .or_insert(PastMeeting {
                    id,
                    title,
                    at,
                    series: false,
                    shared_people: shared as u32,
                });
        }
    }
    Ok(by_id.into_values().collect())
}

fn is_open_section(id: &str, title: &str) -> bool {
    if OPEN_SECTION_IDS.contains(&id) {
        return true;
    }
    let t = title.to_lowercase();
    t.contains("offene") || t.contains("ungeklärt") || t.contains("ungeklaert")
}

fn push(
    items: &mut Vec<Candidate>,
    kind: PrepKind,
    raw: &str,
    assignee: Option<&str>,
    due: Option<&str>,
    done: bool,
) {
    let text = clip(&clean_text(raw), MAX_ITEM_CHARS);
    if text.chars().count() < 3 {
        return;
    }
    let keep = |s: Option<&str>| {
        s.map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
    };
    items.push(Candidate {
        kind,
        text,
        assignee: keep(assignee),
        due: keep(due),
        done,
    });
}

/// Die Punkte einer Besprechung. Eine unlesbare Quelle (beschaedigte Notizen)
/// kostet nur ihre Punkte, nie den ganzen Lauf.
fn collect_items(store: &MeetingStore, meeting_id: &str) -> Vec<Candidate> {
    let mut items = Vec::new();
    if let Ok(tasks) = store.list_action_items(meeting_id) {
        for t in tasks {
            push(
                &mut items,
                classify_task(&t.text),
                &t.text,
                t.assignee_label.as_deref(),
                None,
                t.status == STATUS_DONE,
            );
        }
    }
    if let Ok(notes) = store.get_notes(meeting_id) {
        for b in notes
            .blocks
            .iter()
            .filter(|b| b.kind == NoteBlockKind::Todo)
        {
            push(
                &mut items,
                classify_task(&b.text),
                &b.text,
                None,
                None,
                b.checked,
            );
        }
    }
    if let Ok(docs) = store.get_documents(meeting_id) {
        let latest = docs
            .iter()
            .filter(|d| d.kind == "enhanced_notes")
            .max_by_key(|d| d.version);
        let parsed = latest.and_then(|d| serde_json::from_str::<EnhancedNotes>(&d.body).ok());
        if let Some(notes) = parsed {
            for s in notes
                .sections
                .iter()
                .filter(|s| s.kind == SectionKind::Text && is_open_section(&s.id, &s.title))
            {
                for e in s.entries.iter().filter(|e| !e.flags.unsupported) {
                    push(&mut items, PrepKind::Question, &e.text, None, None, false);
                }
            }
        }
    }
    items
}

/// Alles, was ein Lauf zu diesem Termin braucht. `own`: die eigenen Adressen
/// (Einstellung „Meine E-Mail-Adressen“), `now_s`: jetzt in Sekunden.
pub fn prepare_input(
    store: &MeetingStore,
    event: &CalEvent,
    own: &[String],
    now_s: i64,
) -> Result<PrepInput> {
    let people = event_people(store, event, own)?;
    let past = {
        let conn = store.get_connection()?;
        past_meetings(&conn, event, &people.ids, now_s)?
    };
    let picked = pick_meetings(past, now_s, people.ids.len() as u32);
    let meetings = picked.len() as u32;
    let materials: Vec<Material> = picked
        .into_iter()
        .map(|meeting| {
            let items = collect_items(store, &meeting.id);
            Material { meeting, items }
        })
        .collect();
    let ranked = rank_open(&materials);
    let fingerprint = fingerprint(&event.key, &ranked);
    Ok(PrepInput {
        ranked,
        meetings,
        fingerprint,
    })
}

/// Die Vorbereitung ohne Modell: die ersten Punkte, Texte unveraendert.
pub fn rules_prep(input: &PrepInput) -> Prep {
    Prep {
        points: top_points(&input.ranked, MAX_POINTS)
            .into_iter()
            .map(|r| point_of(r, r.display.clone()))
            .collect(),
        mode: PrepMode::Rules,
        meetings: input.meetings,
    }
}

// ---------------------------------------------------------------------------
// Modell: Prompt und Antwort (rein)
// ---------------------------------------------------------------------------

/// Wer die Verarbeitung angestossen hat.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Trigger {
    /// Der Nutzer oeffnet den Termin oder die Aufnahme.
    Open,
    /// Vorab, ohne dass jemand hinsieht (App-Start, Termine des Tages).
    Prefetch,
}

/// Was ueber das gewaehlte Modell feststeht (vom Aufrufer ermittelt).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ModelChoice {
    /// Anbieter und Modell sind eingetragen.
    pub configured: bool,
    /// Das Modell laeuft auf diesem Rechner.
    pub local: bool,
    /// Die Compliance-Regeln erlauben diesen Anbieter (nicht „gesperrt“); bei
    /// „nur lokal“ ist ein Cloud-Anbieter nie erlaubt.
    pub allowed: bool,
    /// Gerade ungeeignet: Aufnahme auf der CPU, KI-Notizen laufen, RAM knapp.
    pub busy: bool,
}

/// Modell oder Regeln? Regeln, sobald eine Bedingung fehlt. Vorab (ohne dass
/// jemand hinsieht) geht nie etwas an einen Cloud-Anbieter: nur ein lokales
/// Modell, sonst Regeln.
pub fn decide_mode(trigger: Trigger, model: ModelChoice) -> PrepMode {
    let usable = model.configured && model.allowed && !model.busy;
    let by_trigger = match trigger {
        Trigger::Open => true,
        Trigger::Prefetch => model.local,
    };
    if usable && by_trigger {
        PrepMode::Model
    } else {
        PrepMode::Rules
    }
}

pub fn system_prompt() -> String {
    format!(
        "You help the user prepare for an upcoming appointment. You get a numbered list of open \
         points taken from the user's own notes of earlier meetings.\n\
         Rules:\n\
         - Pick the {min} to {max} points that matter most for the appointment. Prefer promised \
         deliveries and open tasks over questions.\n\
         - Reply with JSON only, exactly: {{\"points\":[{{\"ref\":\"P1\",\"text\":\"...\"}}]}}\n\
         - \"ref\" is the id of the point from the list. \"text\" is that point in German, one \
         short sentence, at most {chars} characters.\n\
         - Use only what the point says. Never add names, dates, amounts or facts that are not \
         in it, and never combine points.\n\
         - If no point is useful for the appointment, reply {{\"points\":[]}}.\n\
         - No explanations, no markdown.",
        min = MAX_POINTS - 1,
        max = MAX_POINTS,
        chars = MAX_POINT_CHARS - 20,
    )
}

fn date_label(at: i64) -> String {
    chrono::DateTime::from_timestamp(at, 0)
        .map(|d| {
            d.with_timezone(&chrono::Local)
                .format("%d.%m.%Y")
                .to_string()
        })
        .unwrap_or_default()
}

fn one_line(text: &str, max: usize) -> String {
    clip(&text.replace(['|', '\n', '\r'], " "), max)
}

/// Id eines Kandidaten im Prompt: „P1“, „P2“, ...
fn ref_of(index: usize) -> String {
    format!("P{}", index + 1)
}

/// Die Kandidaten, die das Modell sieht: die ersten [`MAX_CANDIDATES`] der
/// Rangfolge (mit der Vielfalt je Besprechung).
pub fn model_candidates(ranked: &[Ranked]) -> Vec<&Ranked> {
    top_points(ranked, MAX_CANDIDATES)
}

/// Der Prompt: nur Termintitel und Kandidatenzeilen. Keine Namen der
/// Teilnehmenden, keine Beschreibung des Termins, kein Transkript.
pub fn user_prompt(event_title: &str, cands: &[&Ranked]) -> String {
    let mut out = format!(
        "Appointment: {}\n\nOpen points from earlier meetings (id | type | meeting, date | text):\n",
        one_line(event_title, MAX_TITLE_CHARS)
    );
    for (i, r) in cands.iter().enumerate() {
        out.push_str(&format!(
            "{} | {} | {}, {} | {}\n",
            ref_of(i),
            r.kind.label(),
            one_line(&r.source.title, MAX_TITLE_CHARS),
            date_label(r.source.at),
            one_line(&r.display, MAX_POINT_CHARS),
        ));
    }
    out
}

#[derive(Deserialize)]
struct ModelReply {
    #[serde(default)]
    points: Vec<ModelPoint>,
}

#[derive(Deserialize)]
struct ModelPoint {
    #[serde(rename = "ref", default)]
    reference: String,
    #[serde(default)]
    text: String,
}

/// Der JSON-Block einer Antwort (auch aus einem Codezaun oder Begleittext).
fn json_block(reply: &str) -> Option<&str> {
    let start = reply.find('{')?;
    let end = reply.rfind('}')?;
    (end > start).then(|| &reply[start..=end])
}

/// Grobe Wortstaemme (erste fuenf Buchstaben von Woertern ab vier Zeichen).
fn stems(text: &str) -> HashSet<String> {
    key_of(text)
        .split(' ')
        .filter(|w| w.chars().count() >= 4)
        .map(|w| w.chars().take(5).collect())
        .collect()
}

/// Traegt der Modelltext genug vom Kandidaten (zwei Wortstaemme, bei kurzen
/// Kandidaten einen)? Sonst hat das Modell etwas erfunden oder die falsche
/// Kennung genannt.
fn grounded(text: &str, cand: &Ranked) -> bool {
    let base = stems(&cand.display);
    let need = base.len().min(2);
    base.intersection(&stems(text)).count() >= need
}

/// Was aus der Antwort des Modells wird.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Reply {
    Points(Vec<PrepPoint>),
    /// Kein JSON, keine gueltige Kennung oder nichts ausgewaehlt: es gelten die
    /// Regeln.
    Invalid,
}

/// Uebersetzt die Antwort in Punkte. Die Quelle kommt IMMER vom Kandidaten mit
/// der genannten Kennung. Unbekannte und doppelte Kennungen entfallen; ein Text,
/// der nichts vom Kandidaten traegt, wird durch dessen eigenen Text ersetzt; zu
/// lange Texte werden gekuerzt. Hoechstens [`MAX_POINTS`].
pub fn apply_reply(reply: &str, cands: &[&Ranked]) -> Reply {
    let Some(block) = json_block(reply) else {
        return Reply::Invalid;
    };
    let Ok(parsed) = serde_json::from_str::<ModelReply>(block) else {
        return Reply::Invalid;
    };
    let mut used: HashSet<usize> = HashSet::new();
    let mut points = Vec::new();
    for p in parsed.points {
        if points.len() >= MAX_POINTS {
            break;
        }
        let Some(index) = cands
            .iter()
            .enumerate()
            .find(|(i, _)| ref_of(*i) == p.reference.trim())
            .map(|(i, _)| i)
        else {
            continue;
        };
        if !used.insert(index) {
            continue;
        }
        let cand = cands[index];
        let text = clip(&clean_text(&p.text), MAX_POINT_CHARS);
        let text = if text.is_empty() || !grounded(&text, cand) {
            cand.display.clone()
        } else {
            text
        };
        points.push(point_of(cand, text));
    }
    if points.is_empty() {
        Reply::Invalid
    } else {
        Reply::Points(points)
    }
}

/// Die Vorbereitung aus einer brauchbaren Modellantwort; sonst die Regeln.
pub fn model_prep(input: &PrepInput, reply: Option<&str>) -> Prep {
    let cands = model_candidates(&input.ranked);
    match reply.map(|r| apply_reply(r, &cands)) {
        Some(Reply::Points(points)) => Prep {
            points,
            mode: PrepMode::Model,
            meetings: input.meetings,
        },
        _ => rules_prep(input),
    }
}

// ---------------------------------------------------------------------------
// Zwischenspeicher (nur Arbeitsspeicher, je Termin)
// ---------------------------------------------------------------------------

const CACHE_MAX: usize = 32;

type CacheEntry = (String, String, Prep);

static CACHE: Mutex<Vec<CacheEntry>> = Mutex::new(Vec::new());

/// Eine fruehere Vorbereitung zu diesem Termin, wenn sich weder die Notizen noch
/// die Verarbeitungsart seitdem geaendert haben.
pub fn cache_get(event_key: &str, fingerprint: &str, mode: PrepMode) -> Option<Prep> {
    let cache = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    cache
        .iter()
        .find(|(k, f, p)| k == event_key && f == fingerprint && p.mode == mode)
        .map(|(_, _, p)| p.clone())
}

pub fn cache_put(event_key: &str, fingerprint: &str, prep: &Prep) {
    let mut cache = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    cache.retain(|(k, _, _)| k != event_key);
    if cache.len() >= CACHE_MAX {
        cache.remove(0);
    }
    cache.push((event_key.to_string(), fingerprint.to_string(), prep.clone()));
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::managers::calendar::model::Attendee;
    use crate::managers::meetings::notes::model::{
        ActionItem, EnhanceStats, EnhancedEntry, EnhancedSection, EntryFlags, NoteBlock, Origin,
        SOURCE_AI,
    };
    use crate::managers::meetings::store::{MeetingSource, MeetingStatus};
    use crate::managers::people::participants_from_event;

    const NOW: i64 = 1_800_000_000;
    const DAY: i64 = 86_400;

    // -- reine Teile ------------------------------------------------------------

    fn past(id: &str, at: i64, series: bool, shared: u32) -> PastMeeting {
        PastMeeting {
            id: id.into(),
            title: format!("Besprechung {id}"),
            at,
            series,
            shared_people: shared,
        }
    }

    fn cand(kind: PrepKind, text: &str, done: bool) -> Candidate {
        Candidate {
            kind,
            text: text.into(),
            assignee: None,
            due: None,
            done,
        }
    }

    fn mat(m: PastMeeting, items: Vec<Candidate>) -> Material {
        Material { meeting: m, items }
    }

    #[test]
    fn pick_keeps_window_series_and_enough_shared_people_newest_first() {
        let all = vec![
            past("alt", NOW - (WINDOW_DAYS + 1) * DAY, true, 5), // ausserhalb des Fensters
            past("zukunft", NOW + DAY, true, 5),                 // nicht "frueher"
            past("wenig", NOW - 2 * DAY, false, 1),              // 1 von 4 bekannten: zu wenig
            past("serie", NOW - 10 * DAY, true, 0),              // Serie genuegt allein
            past("viele", NOW - 3 * DAY, false, 2),              // 2 von 4: genug
            past("serie", NOW - 10 * DAY, true, 0),              // doppelt
        ];
        let got: Vec<String> = pick_meetings(all, NOW, 4)
            .into_iter()
            .map(|m| m.id)
            .collect();
        assert_eq!(got, vec!["viele".to_string(), "serie".to_string()]);
    }

    #[test]
    fn people_needed_is_half_rounded_up_and_at_least_one() {
        assert_eq!(people_needed(0), 1);
        assert_eq!(people_needed(1), 1);
        assert_eq!(people_needed(2), 1);
        assert_eq!(people_needed(3), 2);
        assert_eq!(people_needed(4), 2);
    }

    #[test]
    fn pick_is_capped() {
        let all: Vec<PastMeeting> = (0..20)
            .map(|i| past(&format!("m{i:02}"), NOW - (i + 1) * DAY, true, 0))
            .collect();
        let got = pick_meetings(all, NOW, 1);
        assert_eq!(got.len(), MAX_MEETINGS);
        assert_eq!(got[0].id, "m00");
    }

    #[test]
    fn rank_orders_by_kind_then_series_and_prefers_the_newer_duplicate() {
        let newer = past("neu", NOW - DAY, false, 1);
        let older = past("alt", NOW - 20 * DAY, true, 1);
        let ranked = rank_open(&[
            mat(
                older,
                vec![
                    cand(PrepKind::Question, "Wer übernimmt den Support?", false),
                    cand(PrepKind::Task, "Angebot schreiben", false),
                ],
            ),
            mat(
                newer,
                vec![
                    cand(PrepKind::Delivery, "Preisliste zusenden", false),
                    cand(PrepKind::Task, "angebot  schreiben!", false), // gleicher Text wie oben
                ],
            ),
        ]);
        let texts: Vec<&str> = ranked.iter().map(|r| r.display.as_str()).collect();
        // Serie gibt +15: die Aufgabe der Serie (35) schlaegt die Lieferung (30).
        // Das Duplikat zaehlt einmal, in der neueren Besprechung (Nicht-Serie: 20).
        assert_eq!(
            texts,
            vec![
                "Preisliste zusenden",
                "Wer übernimmt den Support?",
                "angebot  schreiben!"
            ]
            .into_iter()
            .map(|t| if t.starts_with("angebot") {
                "angebot schreiben!"
            } else {
                t
            })
            .collect::<Vec<_>>()
        );
    }

    #[test]
    fn a_done_item_in_a_newer_meeting_closes_the_same_open_item_in_an_older_one() {
        let older = past("alt", NOW - 20 * DAY, false, 1);
        let newer = past("neu", NOW - DAY, false, 1);
        let open_old = cand(PrepKind::Task, "Vertrag prüfen", false);
        let done_new = cand(PrepKind::Task, "Vertrag prüfen", true);
        assert!(rank_open(&[
            mat(older.clone(), vec![open_old.clone()]),
            mat(newer.clone(), vec![done_new.clone()])
        ])
        .is_empty());
        // Umgekehrt (erledigt in der aelteren, offen in der neueren): bleibt offen.
        let reopened = rank_open(&[mat(older, vec![done_new]), mat(newer, vec![open_old])]);
        assert_eq!(reopened.len(), 1);
        assert_eq!(reopened[0].source.meeting_id, "neu");
    }

    #[test]
    fn top_points_takes_at_most_two_per_meeting() {
        let a = past("a", NOW - DAY, true, 1);
        let b = past("b", NOW - 2 * DAY, false, 1);
        let ranked = rank_open(&[
            mat(
                a,
                vec![
                    cand(PrepKind::Task, "Eins A", false),
                    cand(PrepKind::Task, "Zwei A", false),
                    cand(PrepKind::Task, "Drei A", false),
                ],
            ),
            mat(b, vec![cand(PrepKind::Question, "Frage B", false)]),
        ]);
        let top = top_points(&ranked, 3);
        let ids: Vec<&str> = top.iter().map(|r| r.source.meeting_id.as_str()).collect();
        assert_eq!(ids, vec!["a", "a", "b"]);
    }

    #[test]
    fn classify_marks_promises_as_deliveries() {
        assert_eq!(
            classify_task("Angebot bis Freitag zusenden"),
            PrepKind::Delivery
        );
        assert_eq!(classify_task("Muster nachreichen"), PrepKind::Delivery);
        assert_eq!(
            classify_task("Termin mit dem Vorstand abstimmen"),
            PrepKind::Task
        );
    }

    #[test]
    fn display_adds_assignee_and_due_only_when_missing_and_never_rewrites() {
        let mut c = cand(PrepKind::Task, "Angebot schreiben", false);
        c.assignee = Some("Anna".into());
        c.due = Some("3.10.".into());
        assert_eq!(display_text(&c), "Anna: Angebot schreiben (bis 3.10.)");
        c.text = "Anna schreibt das Angebot bis 3.10.".into();
        assert_eq!(display_text(&c), "Anna schreibt das Angebot bis 3.10.");
    }

    #[test]
    fn clip_cuts_at_a_word_boundary() {
        let long = "Lieferzeiten im vierten Quartal mit dem Kunden klären und anschließend die Planung anpassen";
        let c = clip(long, 40);
        assert!(c.chars().count() <= 40, "{c}");
        assert!(c.ends_with('…'));
        assert!(long.starts_with(c.trim_end_matches('…')));
        assert_eq!(clip("kurz", 40), "kurz");
    }

    #[test]
    fn clean_text_drops_bullets_and_line_breaks() {
        assert_eq!(clean_text("- Angebot\n  schreiben "), "Angebot schreiben");
        assert_eq!(clean_text("   "), "");
    }

    fn ranked_fixture() -> Vec<Ranked> {
        rank_open(&[
            mat(
                past("m1", NOW - 3 * DAY, true, 1),
                vec![
                    cand(PrepKind::Delivery, "Preisliste an Meyer zusenden", false),
                    cand(PrepKind::Task, "Rahmenvertrag prüfen", false),
                ],
            ),
            mat(
                past("m2", NOW - 9 * DAY, false, 1),
                vec![cand(PrepKind::Question, "Wer zahlt die Schulung?", false)],
            ),
        ])
    }

    #[test]
    fn the_prompt_carries_only_candidates_and_the_title() {
        let ranked = ranked_fixture();
        let cands = model_candidates(&ranked);
        let user = user_prompt("Jour fixe | Vertrieb\nGeheim", &cands);
        assert!(
            user.contains("Appointment: Jour fixe Vertrieb Geheim"),
            "{user}"
        );
        assert!(
            user.contains("P1 | promised delivery | Besprechung m1, "),
            "{user}"
        );
        assert!(user.contains("P3 | open question |"), "{user}");
        assert!(user.contains("Preisliste an Meyer zusenden"));
        assert!(system_prompt().contains("JSON only"));
        assert!(system_prompt().contains("2 to 3"));
    }

    #[test]
    fn a_good_reply_keeps_the_source_of_the_candidate() {
        let ranked = ranked_fixture();
        let cands = model_candidates(&ranked);
        let reply =
            "```json\n{\"points\":[{\"ref\":\"P1\",\"text\":\"Preisliste an Meyer senden\"},\
                     {\"ref\":\"P3\",\"text\":\"Klären, wer die Schulung zahlt\"}]}\n```";
        let Reply::Points(points) = apply_reply(reply, &cands) else {
            panic!("erwartet: Punkte");
        };
        assert_eq!(points.len(), 2);
        assert_eq!(points[0].text, "Preisliste an Meyer senden");
        assert_eq!(points[0].source.meeting_id, "m1");
        assert_eq!(points[0].kind, PrepKind::Delivery);
        assert_eq!(points[1].source.meeting_id, "m2");
        assert_eq!(points[1].kind, PrepKind::Question);
    }

    #[test]
    fn unknown_duplicate_and_ungrounded_replies_are_corrected_not_trusted() {
        let ranked = ranked_fixture();
        let cands = model_candidates(&ranked);
        let reply = r#"{"points":[
            {"ref":"P9","text":"Erfundene Quelle"},
            {"ref":"P2","text":"Urlaub in Spanien buchen und den Flug prüfen"},
            {"ref":"P2","text":"Rahmenvertrag prüfen"},
            {"ref":" P1 ","text":""}
        ]}"#;
        let Reply::Points(points) = apply_reply(reply, &cands) else {
            panic!("erwartet: Punkte");
        };
        // P9 entfaellt, P2 steht einmal und mit dem eigenen Text (der Modelltext
        // hat nichts mit dem Kandidaten zu tun), P1 mit leerem Text ebenso.
        assert_eq!(points.len(), 2);
        assert_eq!(points[0].text, "Rahmenvertrag prüfen");
        assert_eq!(points[1].text, "Preisliste an Meyer zusenden");
    }

    #[test]
    fn at_most_three_points_come_back() {
        let many: Vec<Ranked> = rank_open(
            &(0..5)
                .map(|i| {
                    mat(
                        past(&format!("m{i}"), NOW - (i + 1) * DAY, false, 1),
                        vec![cand(PrepKind::Task, &format!("Aufgabe Nummer{i}"), false)],
                    )
                })
                .collect::<Vec<_>>(),
        );
        let cands = model_candidates(&many);
        let reply = format!(
            "{{\"points\":[{}]}}",
            (1..=5)
                .map(|i| format!("{{\"ref\":\"P{i}\",\"text\":\"Aufgabe Nummer{}\"}}", i - 1))
                .collect::<Vec<_>>()
                .join(",")
        );
        let Reply::Points(points) = apply_reply(&reply, &cands) else {
            panic!("erwartet: Punkte");
        };
        assert_eq!(points.len(), MAX_POINTS);
    }

    #[test]
    fn garbage_empty_and_unknown_only_replies_fall_back_to_the_rules() {
        let ranked = ranked_fixture();
        let cands = model_candidates(&ranked);
        for reply in [
            "",
            "Ich kann dabei nicht helfen.",
            "{\"points\":[]}",
            "{\"points\":[{\"ref\":\"P7\",\"text\":\"x\"}]}",
            "{not json}",
            "[]",
        ] {
            assert_eq!(apply_reply(reply, &cands), Reply::Invalid, "{reply}");
        }
        let input = PrepInput {
            ranked: ranked.clone(),
            meetings: 2,
            fingerprint: "f".into(),
        };
        let prep = model_prep(&input, Some("kein json"));
        assert_eq!(prep.mode, PrepMode::Rules);
        assert_eq!(prep, rules_prep(&input));
        assert_eq!(model_prep(&input, None), rules_prep(&input));
        let good = model_prep(
            &input,
            Some("{\"points\":[{\"ref\":\"P2\",\"text\":\"Rahmenvertrag prüfen\"}]}"),
        );
        assert_eq!(good.mode, PrepMode::Model);
        assert_eq!(good.points.len(), 1);
    }

    #[test]
    fn rules_keep_the_note_text_unchanged_and_at_most_three() {
        let input = PrepInput {
            ranked: ranked_fixture(),
            meetings: 2,
            fingerprint: "f".into(),
        };
        let prep = rules_prep(&input);
        assert_eq!(prep.mode, PrepMode::Rules);
        let texts: Vec<&str> = prep.points.iter().map(|p| p.text.as_str()).collect();
        assert_eq!(
            texts,
            vec![
                "Preisliste an Meyer zusenden",
                "Rahmenvertrag prüfen",
                "Wer zahlt die Schulung?"
            ]
        );
        assert_eq!(prep.meetings, 2);
    }

    fn choice(configured: bool, local: bool, allowed: bool, busy: bool) -> ModelChoice {
        ModelChoice {
            configured,
            local,
            allowed,
            busy,
        }
    }

    #[test]
    fn the_model_is_used_only_when_everything_allows_it() {
        use PrepMode::{Model, Rules};
        use Trigger::{Open, Prefetch};
        // lokal, erlaubt: auf Wunsch und vorab
        assert_eq!(decide_mode(Open, choice(true, true, true, false)), Model);
        assert_eq!(
            decide_mode(Prefetch, choice(true, true, true, false)),
            Model
        );
        // Cloud, vom Regelwerk erlaubt: nur auf Wunsch, nie vorab
        assert_eq!(decide_mode(Open, choice(true, false, true, false)), Model);
        assert_eq!(
            decide_mode(Prefetch, choice(true, false, true, false)),
            Rules
        );
        // gesperrt (z. B. „nur lokal“ mit Cloud-Anbieter): nie Modell
        assert_eq!(decide_mode(Open, choice(true, false, false, false)), Rules);
        assert_eq!(
            decide_mode(Prefetch, choice(true, false, false, false)),
            Rules
        );
        // kein Modell eingetragen / Rechner beschaeftigt
        assert_eq!(decide_mode(Open, choice(false, true, true, false)), Rules);
        assert_eq!(decide_mode(Open, choice(true, true, true, true)), Rules);
    }

    #[test]
    fn the_cache_is_per_event_and_checks_fingerprint_and_mode() {
        let prep = Prep {
            points: vec![],
            mode: PrepMode::Rules,
            meetings: 1,
        };
        cache_put("evt:a", "f1", &prep);
        assert_eq!(
            cache_get("evt:a", "f1", PrepMode::Rules),
            Some(prep.clone())
        );
        assert_eq!(
            cache_get("evt:a", "f2", PrepMode::Rules),
            None,
            "Notizen geaendert"
        );
        assert_eq!(
            cache_get("evt:a", "f1", PrepMode::Model),
            None,
            "andere Art"
        );
        assert_eq!(cache_get("evt:b", "f1", PrepMode::Rules), None);
        for i in 0..40 {
            cache_put(&format!("evt:x{i}"), "f", &prep);
        }
        assert!(CACHE.lock().unwrap().len() <= CACHE_MAX);
        assert_eq!(
            cache_get("evt:a", "f1", PrepMode::Rules),
            None,
            "aelteste verdraengt"
        );
    }

    // -- mit echtem Store ---------------------------------------------------------

    fn tmp_store() -> (tempfile::TempDir, MeetingStore) {
        let dir = tempfile::tempdir().unwrap();
        let s = MeetingStore::open_at(&dir.path().join("meetings.db")).unwrap();
        (dir, s)
    }

    fn person(email: &str, name: &str) -> Attendee {
        Attendee {
            email: Some(email.into()),
            name: Some(name.into()),
            organizer: false,
            is_self: false,
            partstat: None,
        }
    }

    fn event(uid: &str, start_ms: i64, attendees: Vec<Attendee>) -> CalEvent {
        CalEvent {
            key: format!("s:{uid}:{start_ms}"),
            source_id: "s".into(),
            uid: uid.into(),
            title: "Jour fixe Vertrieb".into(),
            starts_at: start_ms,
            ends_at: start_ms + 1_800_000,
            all_day: false,
            cancelled: false,
            location: None,
            join_url: None,
            description: None,
            attendees,
        }
    }

    /// Eine fertige Besprechung `days_ago` Tage vor `NOW` mit diesen Teilnehmenden.
    fn meeting(s: &MeetingStore, title: &str, days_ago: i64, who: Vec<Attendee>) -> String {
        let m = s
            .create_meeting(title, MeetingSource::Live, Some(1))
            .unwrap();
        s.get_connection()
            .unwrap()
            .execute(
                "UPDATE meetings SET started_at = ?1 WHERE id = ?2",
                params![NOW - days_ago * DAY, m.id],
            )
            .unwrap();
        s.set_status(&m.id, MeetingStatus::Ready).unwrap();
        participants_from_event(s, &m.id, &event("alt", 1, who)).unwrap();
        m.id
    }

    fn anna() -> Vec<Attendee> {
        vec![person("anna@firma.example", "Anna Berg")]
    }

    fn upcoming(who: Vec<Attendee>) -> CalEvent {
        event("serie1", (NOW + DAY) * 1000, who)
    }

    fn enhanced_doc(s: &MeetingStore, meeting_id: &str, open: &[&str], tasks: &[&str]) {
        let entry = |i: usize, text: &str| EnhancedEntry {
            id: format!("E{i}"),
            origin: Origin::Ai,
            text: text.into(),
            note_id: None,
            source_segment_ids: vec![],
            source_slide_ids: vec![],
            assignee: None,
            due: None,
            flags: EntryFlags::default(),
        };
        let notes = EnhancedNotes {
            format: "enhanced@1".into(),
            template_id: None,
            template_title: "Test".into(),
            segment_epoch: 0,
            sections: vec![
                EnhancedSection {
                    id: "offene_fragen".into(),
                    title: "Offene Fragen".into(),
                    kind: SectionKind::Text,
                    entries: open.iter().enumerate().map(|(i, t)| entry(i, t)).collect(),
                },
                EnhancedSection {
                    id: "aufgaben".into(),
                    title: "Aufgaben".into(),
                    kind: SectionKind::Tasks,
                    entries: tasks
                        .iter()
                        .enumerate()
                        .map(|(i, t)| entry(10 + i, t))
                        .collect(),
                },
            ],
            stats: EnhanceStats::default(),
        };
        s.insert_document(
            meeting_id,
            "enhanced_notes",
            "enhanced@1",
            &serde_json::to_string(&notes).unwrap(),
            None,
            None,
        )
        .unwrap();
    }

    fn action(
        meeting_id: &str,
        doc: &str,
        text: &str,
        status: &str,
        who: Option<&str>,
    ) -> ActionItem {
        ActionItem {
            id: String::new(),
            meeting_id: meeting_id.into(),
            text: text.into(),
            status: status.into(),
            assignee_label: who.map(str::to_string),
            document_id: Some(doc.into()),
            entry_id: None,
            source_segment_ids: vec![],
            source: SOURCE_AI.into(),
        }
    }

    fn doc_id(s: &MeetingStore, meeting_id: &str) -> String {
        s.get_documents(meeting_id)
            .unwrap()
            .last()
            .unwrap()
            .id
            .clone()
    }

    #[test]
    fn no_earlier_notes_means_nothing_to_show() {
        let (_d, s) = tmp_store();
        let ev = upcoming(anna());
        // Gar keine Besprechung.
        let input = prepare_input(&s, &ev, &[], NOW).unwrap();
        assert!(input.is_empty());
        assert!(rules_prep(&input).points.is_empty());
        // Besprechung mit Anna, aber ohne offene Punkte: ebenfalls leer.
        let m = meeting(&s, "Kennenlernen", 5, anna());
        enhanced_doc(&s, &m, &[], &[]);
        let input = prepare_input(&s, &ev, &[], NOW).unwrap();
        assert_eq!(input.meetings, 1);
        assert!(
            input.is_empty(),
            "Besprechung ja, offener Punkt nein: nichts anzeigen"
        );
        // Besprechung mit anderer Person: zaehlt nicht.
        let other = meeting(&s, "Anderes", 2, vec![person("zed@x.example", "Zed")]);
        enhanced_doc(&s, &other, &["Frage von Zed?"], &[]);
        assert!(prepare_input(&s, &ev, &[], NOW).unwrap().is_empty());
    }

    #[test]
    fn open_points_from_tasks_own_todos_and_open_questions_with_their_sources() {
        let (_d, s) = tmp_store();
        let m1 = meeting(&s, "Vertrieb Oktober", 14, anna());
        enhanced_doc(
            &s,
            &m1,
            &["Wer zahlt die Schulung?"],
            &["Preisliste an Meyer zusenden"],
        );
        let doc1 = doc_id(&s, &m1);
        s.replace_action_items(
            &m1,
            &doc1,
            &[
                action(
                    &m1,
                    &doc1,
                    "Preisliste an Meyer zusenden",
                    "todo",
                    Some("Anna"),
                ),
                action(&m1, &doc1, "Raum buchen", "done", None),
            ],
        )
        .unwrap();
        let m2 = meeting(&s, "Vertrieb November", 3, anna());
        s.save_notes(
            &m2,
            &[
                NoteBlock {
                    id: "b1".into(),
                    kind: NoteBlockKind::Todo,
                    text: "Rahmenvertrag prüfen".into(),
                    at_ms: None,
                    checked: false,
                },
                NoteBlock {
                    id: "b2".into(),
                    kind: NoteBlockKind::Todo,
                    text: "Raum buchen".into(),
                    at_ms: None,
                    checked: false,
                },
                NoteBlock {
                    id: "b3".into(),
                    kind: NoteBlockKind::Paragraph,
                    text: "Nur ein Absatz, kein Punkt".into(),
                    at_ms: None,
                    checked: false,
                },
            ],
            0,
        )
        .unwrap();
        let ev = upcoming(anna());
        let input = prepare_input(&s, &ev, &[], NOW).unwrap();
        assert_eq!(input.meetings, 2);
        let prep = rules_prep(&input);
        let got: Vec<(String, String, String)> = prep
            .points
            .iter()
            .map(|p| {
                (
                    p.text.clone(),
                    p.source.title.clone(),
                    format!("{:?}", p.kind),
                )
            })
            .collect();
        // "Raum buchen" ist in der aelteren Besprechung erledigt, in der neueren
        // aber offen: bleibt offen (Quelle: die neuere).
        assert_eq!(
            got,
            vec![
                (
                    "Anna: Preisliste an Meyer zusenden".to_string(),
                    "Vertrieb Oktober".to_string(),
                    "Delivery".to_string()
                ),
                (
                    "Rahmenvertrag prüfen".to_string(),
                    "Vertrieb November".to_string(),
                    "Task".to_string()
                ),
                (
                    "Raum buchen".to_string(),
                    "Vertrieb November".to_string(),
                    "Task".to_string()
                ),
            ]
        );
        assert_eq!(prep.points[0].source.meeting_id, m1);
        assert_eq!(prep.points[0].source.at, NOW - 14 * DAY);
        // Die offene Frage steht in der Rangfolge dahinter (Aufgaben vor Fragen).
        assert!(input.ranked.iter().any(|r| r.kind == PrepKind::Question));
        assert_eq!(input.ranked.len(), 4);
    }

    #[test]
    fn a_task_done_in_a_newer_meeting_does_not_come_back() {
        let (_d, s) = tmp_store();
        let m1 = meeting(&s, "Alt", 20, anna());
        let d1 = {
            enhanced_doc(&s, &m1, &[], &[]);
            doc_id(&s, &m1)
        };
        s.replace_action_items(
            &m1,
            &d1,
            &[action(&m1, &d1, "Vertrag unterschreiben", "todo", None)],
        )
        .unwrap();
        let m2 = meeting(&s, "Neu", 2, anna());
        enhanced_doc(&s, &m2, &[], &[]);
        let d2 = doc_id(&s, &m2);
        s.replace_action_items(
            &m2,
            &d2,
            &[action(&m2, &d2, "Vertrag unterschreiben", "done", None)],
        )
        .unwrap();
        assert!(prepare_input(&s, &upcoming(anna()), &[], NOW)
            .unwrap()
            .is_empty());
    }

    #[test]
    fn selection_respects_window_status_deletion_own_meeting_and_series() {
        let (_d, s) = tmp_store();
        let add = |s: &MeetingStore, id: &str, text: &str| {
            enhanced_doc(s, id, &[text], &[]);
        };
        let inside = meeting(&s, "Im Fenster", 30, anna());
        add(&s, &inside, "Frage im Fenster");
        let outside = meeting(&s, "Zu alt", WINDOW_DAYS + 5, anna());
        add(&s, &outside, "Frage zu alt");
        let running = s
            .create_meeting("Läuft", MeetingSource::Live, Some(1))
            .unwrap();
        participants_from_event(&s, &running.id, &event("alt", 1, anna())).unwrap();
        add(&s, &running.id, "Frage laufend");
        let deleted = meeting(&s, "Gelöscht", 4, anna());
        add(&s, &deleted, "Frage gelöscht");
        s.soft_delete_meeting(&deleted).unwrap();
        let ev = upcoming(anna());
        let own = meeting(&s, "Dieser Termin", 1, anna());
        add(&s, &own, "Frage eigener Termin");
        s.link_meeting_event(&own, &ev, "prompt", 1).unwrap();
        // Frueherer Termin derselben Serie, ohne gemeinsame Teilnehmende.
        let series = meeting(&s, "Serie früher", 7, vec![person("zed@x.example", "Zed")]);
        add(&s, &series, "Frage Serie");
        let mut earlier = event("serie1", (NOW - 7 * DAY) * 1000, vec![]);
        earlier.key = "s:serie1:earlier".into();
        s.link_meeting_event(&series, &earlier, "prompt", 1)
            .unwrap();

        let input = prepare_input(&s, &ev, &[], NOW).unwrap();
        let mut titles: Vec<String> = input
            .ranked
            .iter()
            .map(|r| r.source.title.clone())
            .collect();
        titles.sort();
        assert_eq!(
            titles,
            vec!["Im Fenster".to_string(), "Serie früher".to_string()]
        );
        assert_eq!(input.meetings, 2);
        // Die Serie zaehlt vor den Teilnehmenden (gleiche Art, +15).
        assert_eq!(input.ranked[0].source.title, "Serie früher");
    }

    #[test]
    fn myself_does_not_connect_meetings() {
        let (_d, s) = tmp_store();
        let me = || person("ich@wolff.example", "Ich");
        let m = meeting(&s, "Nur ich", 3, vec![me(), person("zed@x.example", "Zed")]);
        enhanced_doc(&s, &m, &["Frage"], &[]);
        let ev = upcoming(vec![me(), person("neu@y.example", "Neu")]);
        let own = vec!["ich@wolff.example".to_string()];
        assert!(prepare_input(&s, &ev, &own, NOW).unwrap().is_empty());
    }

    #[test]
    fn a_damaged_note_source_costs_only_its_own_points() {
        let (_d, s) = tmp_store();
        let m = meeting(&s, "Beschädigt", 3, anna());
        // Kein gueltiges JSON als KI-Notiz: wird uebersprungen.
        s.insert_document(&m, "enhanced_notes", "enhanced@1", "{kaputt", None, None)
            .unwrap();
        s.get_connection()
            .unwrap()
            .execute(
                "INSERT INTO meeting_notes (meeting_id, blocks_json, revision, created_at, updated_at) VALUES (?1, '[kaputt', 1, 1, 1)",
                params![m],
            )
            .unwrap();
        let ok = meeting(&s, "Heil", 2, anna());
        enhanced_doc(&s, &ok, &["Heile Frage"], &[]);
        let input = prepare_input(&s, &upcoming(anna()), &[], NOW).unwrap();
        let texts: Vec<&str> = input.ranked.iter().map(|r| r.display.as_str()).collect();
        assert_eq!(texts, vec!["Heile Frage"]);
    }

    #[test]
    fn the_fingerprint_changes_with_the_notes_and_the_event() {
        let (_d, s) = tmp_store();
        let m = meeting(&s, "Eins", 3, anna());
        enhanced_doc(&s, &m, &["Frage A"], &[]);
        let ev = upcoming(anna());
        let a = prepare_input(&s, &ev, &[], NOW).unwrap().fingerprint;
        assert_eq!(a, prepare_input(&s, &ev, &[], NOW).unwrap().fingerprint);
        let m2 = meeting(&s, "Zwei", 2, anna());
        enhanced_doc(&s, &m2, &["Frage B"], &[]);
        assert_ne!(a, prepare_input(&s, &ev, &[], NOW).unwrap().fingerprint);
        let other = event("serie2", (NOW + 2 * DAY) * 1000, anna());
        assert_ne!(a, prepare_input(&s, &other, &[], NOW).unwrap().fingerprint);
    }

    #[test]
    fn the_prep_type_exports_to_typescript() {
        let mut types = specta::TypeCollection::default();
        types.register::<Prep>();
        let ts = specta_typescript::Typescript::default()
            .bigint(specta_typescript::BigIntExportBehavior::Number)
            .export(&types)
            .expect("Typen muessen exportierbar sein");
        for expected in [
            "export type Prep =",
            "export type PrepPoint =",
            "export type PrepSource =",
            "export type PrepKind =",
            "\"delivery\"",
            "export type PrepMode =",
            "\"rules\"",
        ] {
            assert!(ts.contains(expected), "{expected} fehlt in:\n{ts}");
        }
    }
}
