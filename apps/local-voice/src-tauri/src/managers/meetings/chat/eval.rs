//! Eval AK8 (M4 §11, P4f): der Chat ueber Besprechungen gegen synthetische
//! Fixtures, mit dem eingestellten Sprachmodell und dem echten Suchindex.
//!
//! `--eval-chat <dir>` liest `<dir>/questions.json` (Besprechungen mit
//! Ordner, Teilnehmern, Startzeit und die Fragen), legt die Besprechungen in
//! einem Sandbox-Store an, baut den Index synchron auf (Chunks, FTS, Vektoren
//! ueber den Embedding-Server; `--lexical-only` ohne Vektoren) und stellt die
//! Fragen seriell ueber denselben Ablauf wie die App (`controller::ask_guarded`).
//!
//! Bewertung je Frage (`judge`, rein):
//! - beantwortbar: jede Schluesselgruppe (`all_of`, Synonyme) steht im
//!   normalisierten Antworttext (klein, ss statt ß, Tausenderpunkte weg,
//!   Dezimalkomma als Punkt, Zahlwoerter als Ziffern) UND mindestens ein Zitat
//!   liegt auf einer erwarteten Stelle: Transkript-Zitat, dessen Index-Chunk
//!   ein erwartetes Segment enthaelt, oder Notizen-Zitat auf einen erwarteten
//!   Notizblock (`notes`, nur wo der Block die Antwort woertlich traegt);
//! - unbeantwortbar: `not_found` und kein Zitat.
//!
//! Fehlfragen bekommen eine Ursache: `retrieval` (erwartete Stelle nicht
//! gelesen), `model` (gelesen, aber falsch/nicht beantwortet) oder `citation`
//! (Antwort richtig, Beleg falsch oder fehlt).
//!
//! Kein Modul hier ruft `settings::get_settings(&AppHandle)`; die
//! Einstellungen kommen vom Aufrufer (lib.rs).

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::Instant;

use chrono::{NaiveDate, TimeZone};
use futures_util::future::BoxFuture;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::controller::{ask_guarded, CancelFlag, ChatEnv, ChatProgress};
use super::{ChatAnswer, ChatRequest, ChatScope, ChunkSource, MessageMeta, ScopeFilter};
use crate::managers::meetings::llm_call::resolve_provider_coded;
use crate::managers::meetings::notes::model::NoteBlock;
use crate::managers::meetings::search::embed::{EmbedError, EmbedKind, Embedder, LlamaEmbedder};
use crate::managers::meetings::search::index::ChunkRow;
use crate::managers::meetings::search::indexer::{reindex_all, ReindexReport};
use crate::managers::meetings::search::vectors::VectorCache;
use crate::managers::meetings::store::{
    MeetingSource, MeetingStatus, MeetingStore, StoredSegment, TranscriptDelta,
};
use crate::settings::AppSettings;

pub const EXIT_OK: i32 = 0;
pub const EXIT_ERROR: i32 = 1;
pub const EXIT_MISSED: i32 = 3;
/// AK8: mindestens 85 % richtige Antworten mit korrektem Zitat.
pub const TARGET_ACCURACY: f64 = 0.85;
pub const QUESTIONS_FILE: &str = "questions.json";

// ---------------------------------------------------------------------------
// Fragenkatalog und Fixtures
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Deserialize)]
pub struct EvalSet {
    /// `JJJJ-MM-TT`: "Heute" im Prompt (Standard: echtes Datum).
    #[serde(default)]
    pub today: Option<String>,
    pub meetings: Vec<MeetingSpec>,
    pub questions: Vec<EvalQuestion>,
}

/// Eine Besprechung des Katalogs. Ordner, Teilnehmer und Startzeit ergaenzen
/// oder ueberschreiben die Angaben der Fixture-Datei (die P1e-Fixtures haben
/// keine).
#[derive(Clone, Debug, Deserialize)]
pub struct MeetingSpec {
    pub name: String,
    /// Relativ zum Katalog-Verzeichnis.
    pub file: String,
    #[serde(default)]
    pub folder: Option<String>,
    #[serde(default)]
    pub participants: Option<Vec<String>>,
    /// RFC 3339, z. B. `2026-09-07T09:00:00+02:00`.
    #[serde(default)]
    pub started_at: Option<String>,
}

/// Besprechungs-Fixture (Format der M1-Eval plus Ordner/Teilnehmer/Start).
#[derive(Clone, Debug, Deserialize)]
pub struct ChatFixture {
    pub title: String,
    pub segments: Vec<StoredSegment>,
    #[serde(default)]
    pub notes: Vec<NoteBlock>,
    #[serde(default)]
    pub folder: Option<String>,
    #[serde(default)]
    pub participants: Vec<String>,
    #[serde(default)]
    pub started_at: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum QuestionScope {
    Meeting {
        meeting: String,
    },
    Global {
        /// Ordnername.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        folder: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        person: Option<String>,
        /// `JJJJ-MM-TT` (Ortszeit, ganzer Tag).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        from: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        to: Option<String>,
    },
}

#[derive(Clone, Debug, Deserialize)]
pub struct EvalQuestion {
    pub id: String,
    /// `meeting` | `global` | `two_meetings` | `filter` | `unanswerable`
    #[serde(default)]
    pub kind: String,
    pub scope: QuestionScope,
    pub question: String,
    pub expect: Expect,
}

/// Eine erwartete Belegstelle.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
pub struct ExpectSource {
    pub meeting: String,
    #[serde(default)]
    pub segments: Vec<u32>,
    /// NoteBlock-IDs, die die Antwort woertlich tragen.
    #[serde(default)]
    pub notes: Vec<String>,
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct Expect {
    #[serde(default)]
    pub not_found: bool,
    /// Schluesselgruppen: jede muss vorkommen, innerhalb einer Gruppe reicht ein Synonym.
    #[serde(default)]
    pub all_of: Vec<Vec<String>>,
    #[serde(default)]
    pub sources: Vec<ExpectSource>,
    /// Kurzform aus §11 (`meeting` + `segments`).
    #[serde(default)]
    pub meeting: Option<String>,
    #[serde(default)]
    pub segments: Vec<u32>,
}

impl Expect {
    pub fn all_sources(&self) -> Vec<ExpectSource> {
        let mut out = self.sources.clone();
        if let Some(meeting) = &self.meeting {
            out.push(ExpectSource {
                meeting: meeting.clone(),
                segments: self.segments.clone(),
                notes: Vec::new(),
            });
        }
        out
    }
}

fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T, String> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| format!("{} nicht lesbar: {e}", path.display()))?;
    serde_json::from_str(&text).map_err(|e| format!("{} ungueltig: {e}", path.display()))
}

/// Katalog und Fixtures laden und pruefen (Namen, Segmente, Notizen,
/// Erwartungen). Fehler vor jedem Modellaufruf.
pub fn load_set(dir: &Path) -> Result<(EvalSet, Vec<(MeetingSpec, ChatFixture)>), String> {
    let set: EvalSet = read_json(&dir.join(QUESTIONS_FILE))?;
    let mut fixtures = Vec::new();
    let mut names = HashSet::new();
    for spec in &set.meetings {
        if !names.insert(spec.name.clone()) {
            return Err(format!("Besprechung {} doppelt", spec.name));
        }
        let mut fixture: ChatFixture = read_json(&dir.join(&spec.file))?;
        if spec.folder.is_some() {
            fixture.folder = spec.folder.clone();
        }
        if let Some(p) = &spec.participants {
            fixture.participants = p.clone();
        }
        if spec.started_at.is_some() {
            fixture.started_at = spec.started_at.clone();
        }
        if let Some(raw) = &fixture.started_at {
            parse_start(raw).ok_or_else(|| format!("{}: started_at {raw} ungueltig", spec.name))?;
        }
        fixtures.push((spec.clone(), fixture));
    }
    let by_name: HashMap<&str, &ChatFixture> =
        fixtures.iter().map(|(s, f)| (s.name.as_str(), f)).collect();
    let mut ids = HashSet::new();
    for q in &set.questions {
        if !ids.insert(q.id.clone()) {
            return Err(format!("Frage {} doppelt", q.id));
        }
        match &q.scope {
            QuestionScope::Meeting { meeting } if !by_name.contains_key(meeting.as_str()) => {
                return Err(format!("{}: unbekannte Besprechung {meeting}", q.id));
            }
            QuestionScope::Global { from, to, .. } => {
                for d in [from, to].into_iter().flatten() {
                    parse_day(d).ok_or_else(|| format!("{}: Datum {d} ungueltig", q.id))?;
                }
            }
            _ => {}
        }
        if q.expect.not_found {
            if !q.expect.all_of.is_empty() || !q.expect.all_sources().is_empty() {
                return Err(format!("{}: unbeantwortbar, aber mit Erwartung", q.id));
            }
            continue;
        }
        let sources = q.expect.all_sources();
        if q.expect.all_of.is_empty() || q.expect.all_of.iter().any(Vec::is_empty) {
            return Err(format!("{}: Schluesselgruppen fehlen", q.id));
        }
        if sources.is_empty() {
            return Err(format!("{}: erwartete Stelle fehlt", q.id));
        }
        for src in &sources {
            let fixture = by_name
                .get(src.meeting.as_str())
                .ok_or_else(|| format!("{}: unbekannte Besprechung {}", q.id, src.meeting))?;
            for seg in &src.segments {
                if !fixture.segments.iter().any(|s| s.segment_index == *seg) {
                    return Err(format!("{}: Segment {seg} fehlt in {}", q.id, src.meeting));
                }
            }
            for note in &src.notes {
                if !fixture.notes.iter().any(|n| &n.id == note) {
                    return Err(format!("{}: Notiz {note} fehlt in {}", q.id, src.meeting));
                }
            }
        }
    }
    Ok((set, fixtures))
}

fn parse_start(raw: &str) -> Option<i64> {
    chrono::DateTime::parse_from_rfc3339(raw.trim())
        .ok()
        .map(|d| d.timestamp())
}

fn parse_day(raw: &str) -> Option<NaiveDate> {
    NaiveDate::parse_from_str(raw.trim(), "%Y-%m-%d").ok()
}

fn local_ts(date: NaiveDate, h: u32, m: u32, s: u32) -> Option<i64> {
    let naive = date.and_hms_opt(h, m, s)?;
    chrono::Local
        .from_local_datetime(&naive)
        .earliest()
        .map(|d| d.timestamp())
}

// ---------------------------------------------------------------------------
// Normalisierung und Bewertung (rein)
// ---------------------------------------------------------------------------

fn unit(w: &str) -> Option<u64> {
    Some(match w {
        "ein" | "eins" => 1,
        "zwei" => 2,
        "drei" => 3,
        "vier" => 4,
        "fünf" => 5,
        "sechs" => 6,
        "sieben" => 7,
        "acht" => 8,
        "neun" => 9,
        _ => return None,
    })
}

fn teens_or_tens(w: &str) -> Option<u64> {
    Some(match w {
        "zehn" => 10,
        "elf" => 11,
        "zwölf" => 12,
        "dreizehn" => 13,
        "vierzehn" => 14,
        "fünfzehn" => 15,
        "sechzehn" => 16,
        "siebzehn" => 17,
        "achtzehn" => 18,
        "neunzehn" => 19,
        "zwanzig" => 20,
        "dreissig" => 30,
        "vierzig" => 40,
        "fünfzig" => 50,
        "sechzig" => 60,
        "siebzig" => 70,
        "achtzig" => 80,
        "neunzig" => 90,
        _ => return None,
    })
}

fn below_hundred(w: &str) -> Option<u64> {
    if let Some(n) = unit(w).or_else(|| teens_or_tens(w)) {
        return Some(n);
    }
    for (i, _) in w.match_indices("und") {
        if let (Some(u), Some(t)) = (unit(&w[..i]), teens_or_tens(&w[i + 3..])) {
            if t >= 20 && t % 10 == 0 {
                return Some(t + u);
            }
        }
    }
    None
}

fn below_thousand(w: &str) -> Option<u64> {
    if let Some(i) = w.find("hundert") {
        let (left, right) = (&w[..i], &w[i + "hundert".len()..]);
        let hundreds = if left.is_empty() { 1 } else { unit(left)? };
        let rest = match right.strip_prefix("und").unwrap_or(right) {
            "" => 0,
            r => below_hundred(r)?,
        };
        return Some(hundreds * 100 + rest);
    }
    below_hundred(w)
}

fn cardinal(w: &str) -> Option<u64> {
    if let Some(i) = w.find("tausend") {
        let (left, right) = (&w[..i], &w[i + "tausend".len()..]);
        let thousands = if left.is_empty() {
            1
        } else {
            below_thousand(left)?
        };
        let rest = match right.strip_prefix("und").unwrap_or(right) {
            "" => 0,
            r => below_thousand(r)?,
        };
        return Some(thousands * 1000 + rest);
    }
    below_thousand(w)
}

/// Ein deutsches Zahlwort (auch Ordinalzahl: "fünfzehnten", "ersten") als
/// Zahl. Der Artikel "ein" bleibt ein Wort.
pub fn number_word(token: &str) -> Option<u64> {
    if token == "ein" {
        return None;
    }
    if let Some(n) = cardinal(token) {
        return Some(n);
    }
    for suffix in ["sten", "ste", "ster", "stes", "stem"] {
        if let Some(n) = token.strip_suffix(suffix).and_then(cardinal) {
            if n >= 20 {
                return Some(n);
            }
        }
    }
    for suffix in ["ten", "te", "ter", "tes", "tem"] {
        if let Some(base) = token.strip_suffix(suffix) {
            let n = match base {
                "ers" => Some(1),
                "drit" => Some(3),
                "sieb" => Some(7),
                "ach" => Some(8),
                _ => cardinal(base).filter(|n| *n < 20),
            };
            if n.is_some() {
                return n;
            }
        }
    }
    None
}

/// Text fuer den Schluesselvergleich: klein, `ß` -> `ss`, Tausenderpunkte
/// weg (`55.000` -> `55000`), Dezimalkomma als Punkt (`1,8` -> `1.8`),
/// Zahlwoerter als Ziffern (`achtundzwanzig` -> `28`, `eins komma acht` ->
/// `1.8`), Satzzeichen als Leerraum. Ergebnis: Tokens mit je einem Leerzeichen.
pub fn normalize(text: &str) -> String {
    use std::sync::OnceLock;
    static THOUSANDS: OnceLock<regex::Regex> = OnceLock::new();
    static DECIMAL: OnceLock<regex::Regex> = OnceLock::new();
    let thousands = THOUSANDS
        .get_or_init(|| regex::Regex::new(r"(\d)[.\u{202f}\u{a0}](\d{3})\b").expect("Regex"));
    let decimal = DECIMAL.get_or_init(|| regex::Regex::new(r"(\d),(\d)").expect("Regex"));

    let mut s = text.to_lowercase().replace('ß', "ss");
    loop {
        let next = thousands.replace_all(&s, "$1$2").into_owned();
        if next == s {
            break;
        }
        s = next;
    }
    let s = decimal.replace_all(&s, "$1.$2");
    let tokens: Vec<String> = s
        .split(|c: char| !(c.is_alphanumeric() || c == '.'))
        .map(|t| t.trim_matches('.'))
        .filter(|t| !t.is_empty())
        .map(|t| match number_word(t) {
            Some(n) => n.to_string(),
            None => t.to_string(),
        })
        .collect();
    let is_num = |t: &str| t.chars().all(|c| c.is_ascii_digit());
    let mut out: Vec<String> = Vec::with_capacity(tokens.len());
    let mut i = 0;
    while i < tokens.len() {
        if i + 2 < tokens.len()
            && tokens[i + 1] == "komma"
            && is_num(&tokens[i])
            && is_num(&tokens[i + 2])
        {
            out.push(format!("{}.{}", tokens[i], tokens[i + 2]));
            i += 3;
        } else {
            out.push(tokens[i].clone());
            i += 1;
        }
    }
    out.join(" ")
}

/// Steht der Schluessel im normalisierten Text? Linke Grenze: Tokenanfang
/// (`archiv` trifft auch `archivierung`); endet der Schluessel auf eine
/// Ziffer, auch rechts eine Grenze (`15` trifft nicht `150` und nicht `15.5`).
pub fn contains_key(normalized_text: &str, key: &str) -> bool {
    let key = normalize(key);
    if key.is_empty() {
        return false;
    }
    let ends_digit = key.chars().last().is_some_and(|c| c.is_ascii_digit());
    let hay = format!(" {normalized_text} ");
    let needle = format!(" {key}");
    hay.match_indices(&needle).any(|(pos, _)| {
        if !ends_digit {
            return true;
        }
        let mut after = hay[pos + needle.len()..].chars();
        match after.next() {
            Some(c) if c.is_ascii_digit() => false,
            Some('.') => !after.next().is_some_and(|c| c.is_ascii_digit()),
            _ => true,
        }
    })
}

/// Ein Zitat der Antwort, aufgeloest fuer die Bewertung.
#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct CitedRef {
    /// Name der Besprechung im Katalog (`None`: unbekannte Besprechung).
    pub meeting: Option<String>,
    pub source: ChunkSource,
    pub segment_index: Option<u32>,
    pub ref_key: Option<String>,
    /// Segmente der Index-Chunks (Transkript), die das zitierte Segment enthalten.
    pub chunk_segments: Vec<u32>,
}

#[derive(Clone, Debug, Default)]
pub struct AnswerView {
    pub text: String,
    pub not_found: bool,
    pub citations: Vec<CitedRef>,
}

#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct Verdict {
    pub correct: bool,
    pub keys_ok: bool,
    /// Schluesselgruppen, von denen kein Synonym vorkam.
    pub missing_groups: Vec<Vec<String>>,
    pub cite_ok: bool,
    /// Zitate auf eine erwartete Stelle.
    pub matched_citations: u32,
    /// `ok` | `keys_missing` | `no_citation` | `citation_wrong` | `not_found`
    /// | `answered_unanswerable` | `cited_not_found` | `error`
    pub reason: &'static str,
}

impl Verdict {
    pub fn error() -> Self {
        Self {
            correct: false,
            keys_ok: false,
            missing_groups: Vec::new(),
            cite_ok: false,
            matched_citations: 0,
            reason: "error",
        }
    }
}

pub fn citation_matches(c: &CitedRef, src: &ExpectSource) -> bool {
    if c.meeting.as_deref() != Some(src.meeting.as_str()) {
        return false;
    }
    match c.source {
        ChunkSource::Transcript => {
            let mut segs: HashSet<u32> = c.chunk_segments.iter().copied().collect();
            segs.extend(c.segment_index);
            src.segments.iter().any(|s| segs.contains(s))
        }
        ChunkSource::UserNotes => c.ref_key.as_ref().is_some_and(|k| src.notes.contains(k)),
        ChunkSource::AiNotes | ChunkSource::Title | ChunkSource::Slide => false,
    }
}

/// Die Bewertung einer Antwort (siehe Modulkopf).
pub fn judge(expect: &Expect, answer: &AnswerView) -> Verdict {
    if expect.not_found {
        let ok = answer.not_found && answer.citations.is_empty();
        return Verdict {
            correct: ok,
            keys_ok: ok,
            missing_groups: Vec::new(),
            cite_ok: answer.citations.is_empty(),
            matched_citations: 0,
            reason: if ok {
                "ok"
            } else if !answer.not_found {
                "answered_unanswerable"
            } else {
                "cited_not_found"
            },
        };
    }
    let text = normalize(&answer.text);
    let missing_groups: Vec<Vec<String>> = if answer.not_found {
        expect.all_of.clone()
    } else {
        expect
            .all_of
            .iter()
            .filter(|group| !group.iter().any(|k| contains_key(&text, k)))
            .cloned()
            .collect()
    };
    let sources = expect.all_sources();
    let matched = answer
        .citations
        .iter()
        .filter(|c| sources.iter().any(|s| citation_matches(c, s)))
        .count() as u32;
    let keys_ok = missing_groups.is_empty();
    let cite_ok = matched >= 1;
    let reason = if answer.not_found {
        "not_found"
    } else if !keys_ok {
        "keys_missing"
    } else if answer.citations.is_empty() {
        "no_citation"
    } else if !cite_ok {
        "citation_wrong"
    } else {
        "ok"
    };
    Verdict {
        correct: keys_ok && cite_ok,
        keys_ok,
        missing_groups,
        cite_ok,
        matched_citations: matched,
        reason,
    }
}

/// Ursache einer Fehlfrage. `retrieved`: lag eine erwartete Stelle in den
/// gelesenen Auszuegen (`None` = unbekannt).
pub fn failure_cause(verdict: &Verdict, retrieved: Option<bool>) -> Option<&'static str> {
    if verdict.correct {
        return None;
    }
    Some(match verdict.reason {
        "keys_missing" | "not_found" if retrieved == Some(false) => "retrieval",
        "keys_missing" | "not_found" | "answered_unanswerable" | "cited_not_found" => "model",
        "no_citation" | "citation_wrong" => "citation",
        _ => "error",
    })
}

// ---------------------------------------------------------------------------
// Sandbox, Index, Antworten
// ---------------------------------------------------------------------------

/// Embedder ohne Modell (`--lexical-only`, Tests): meldet sich als nicht
/// vorhanden, damit der Index-Aufbau keinen Embedding-Server startet.
pub struct NoVectors;

impl Embedder for NoVectors {
    fn embed<'a>(
        &'a self,
        _texts: &'a [String],
        _kind: EmbedKind,
    ) -> BoxFuture<'a, Result<Vec<Vec<f32>>, EmbedError>> {
        Box::pin(async { Err(EmbedError::NoModel) })
    }

    fn model_id(&self) -> &str {
        ""
    }

    fn available(&self) -> bool {
        false
    }
}

/// Die angelegte Sandbox: Namen -> IDs, Ordner, Index-Bericht, Chunks.
pub struct Prepared {
    pub set: EvalSet,
    /// Name im Katalog -> Besprechungs-ID.
    pub ids: BTreeMap<String, String>,
    /// Ordnername -> Ordner-ID.
    pub folders: HashMap<String, String>,
    pub index: ReindexReport,
    /// Index-Chunks je Besprechungs-ID.
    pub chunks: HashMap<String, Vec<ChunkRow>>,
}

impl Prepared {
    fn name_of(&self, meeting_id: &str) -> Option<String> {
        self.ids
            .iter()
            .find(|(_, id)| id.as_str() == meeting_id)
            .map(|(name, _)| name.clone())
    }

    fn chunk_by_id(&self, id: i64) -> Option<&ChunkRow> {
        self.chunks.values().flatten().find(|c| c.id == id)
    }
}

fn import_meeting(
    store: &MeetingStore,
    fixture: &ChatFixture,
    folders: &mut HashMap<String, String>,
) -> anyhow::Result<String> {
    let now = chrono::Utc::now().timestamp();
    let meeting = store.create_meeting(&fixture.title, MeetingSource::Import, Some(now))?;
    store.append_delta(
        &meeting.id,
        &TranscriptDelta {
            new_segments: fixture.segments.clone(),
        },
    )?;
    if !fixture.notes.is_empty() {
        store.save_notes(&meeting.id, &fixture.notes, 0)?;
    }
    let conn = store.get_connection()?;
    if let Some(start) = fixture.started_at.as_deref().and_then(parse_start) {
        let end_ms = fixture.segments.iter().map(|s| s.end_ms).max().unwrap_or(0);
        conn.execute(
            "UPDATE meetings SET started_at = ?1, ended_at = ?2 WHERE id = ?3",
            rusqlite::params![start, start + (end_ms / 1000) as i64, meeting.id],
        )?;
    }
    // Teilnehmer als benannte Sprecher: der Personenfilter findet sie ueber
    // `speakers.display_name` (M4 §6.1, v1).
    for (i, name) in fixture.participants.iter().enumerate() {
        conn.execute(
            "INSERT INTO speakers (id, meeting_id, channel, speaker_index, human_id,
                display_name, consent_state, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, NULL, ?5, 'confirmed', ?6, ?6)",
            rusqlite::params![
                ulid::Ulid::new().to_string(),
                meeting.id,
                u8::from(i > 0),
                i as u32,
                name,
                now
            ],
        )?;
    }
    if let Some(folder) = fixture.folder.as_deref().filter(|f| !f.trim().is_empty()) {
        let folder_id = match folders.get(folder) {
            Some(id) => id.clone(),
            None => {
                let created = store.folder_save(None, folder, None)?;
                folders.insert(folder.to_string(), created.id.clone());
                created.id
            }
        };
        store.set_meeting_folders(&meeting.id, &[folder_id])?;
    }
    store.set_status(&meeting.id, MeetingStatus::Ready)?;
    Ok(meeting.id)
}

fn meeting_chunks(store: &MeetingStore, meeting_id: &str) -> anyhow::Result<Vec<ChunkRow>> {
    let ids: Vec<i64> = {
        let conn = store.get_connection()?;
        let mut stmt = conn.prepare("SELECT id FROM meeting_chunks WHERE meeting_id = ?1")?;
        let rows = stmt.query_map([meeting_id], |r| r.get::<_, i64>(0))?;
        rows.collect::<Result<_, _>>()?
    };
    store.get_chunks(&ids)
}

/// Katalog laden, Besprechungen in den (Sandbox-)Store legen und den Index
/// synchron aufbauen (`reindex_all`: Chunks, FTS, Vektoren; der
/// Embedding-Server wird am Ende beendet). Synchron: `reindex_all` blockiert
/// selbst auf dem Tauri-Runtime und darf nicht in einem `async`-Kontext laufen.
pub fn prepare(
    store: &Arc<MeetingStore>,
    dir: &Path,
    embed: Arc<dyn Embedder>,
    cache: &'static VectorCache,
    server_pid: &dyn Fn() -> Option<u32>,
) -> Result<Prepared, String> {
    let (set, fixtures) = load_set(dir)?;
    let mut ids = BTreeMap::new();
    let mut folders = HashMap::new();
    for (spec, fixture) in &fixtures {
        let id = import_meeting(store, fixture, &mut folders)
            .map_err(|e| format!("{}: Import fehlgeschlagen: {e}", spec.name))?;
        ids.insert(spec.name.clone(), id);
    }
    let index = reindex_all(store.clone(), embed, cache, true, server_pid)
        .map_err(|e| format!("Index-Aufbau fehlgeschlagen: {e}"))?;
    let mut chunks = HashMap::new();
    for id in ids.values() {
        let rows = meeting_chunks(store, id).map_err(|e| format!("Chunks nicht lesbar: {e}"))?;
        chunks.insert(id.clone(), rows);
    }
    Ok(Prepared {
        set,
        ids,
        folders,
        index,
        chunks,
    })
}

/// Die Chat-Anfrage einer Frage (neuer Verlauf je Frage).
pub fn build_request(q: &EvalQuestion, prepared: &Prepared) -> Result<ChatRequest, String> {
    let scope = match &q.scope {
        QuestionScope::Meeting { meeting } => ChatScope::Meeting {
            meeting_id: prepared
                .ids
                .get(meeting)
                .cloned()
                .ok_or_else(|| format!("unbekannte Besprechung {meeting}"))?,
        },
        QuestionScope::Global {
            folder,
            person,
            from,
            to,
        } => {
            let folder_id = match folder {
                Some(name) => Some(
                    prepared
                        .folders
                        .get(name)
                        .cloned()
                        .ok_or_else(|| format!("unbekannter Ordner {name}"))?,
                ),
                None => None,
            };
            let day = |raw: &Option<String>, end: bool| -> Result<Option<i64>, String> {
                match raw {
                    None => Ok(None),
                    Some(d) => {
                        let date = parse_day(d).ok_or_else(|| format!("Datum {d} ungueltig"))?;
                        let ts = if end {
                            local_ts(date, 23, 59, 59)
                        } else {
                            local_ts(date, 0, 0, 0)
                        };
                        ts.map(Some).ok_or_else(|| format!("Datum {d} ungueltig"))
                    }
                }
            };
            ChatScope::Global {
                filter: ScopeFilter {
                    meeting_ids: None,
                    folder_id,
                    person: person.clone(),
                    from: day(from, false)?,
                    to: day(to, true)?,
                    ..ScopeFilter::default()
                },
            }
        }
    };
    Ok(ChatRequest {
        request_id: format!("eval-{}", q.id),
        thread_id: None,
        scope,
        question: q.question.clone(),
        recipe: None,
    })
}

/// Zitate der Antwort mit Katalogname und Chunk-Segmenten.
pub fn view_of(prepared: &Prepared, answer: &ChatAnswer) -> AnswerView {
    let citations = answer
        .citations
        .iter()
        .map(|c| {
            let chunk_segments: Vec<u32> = match (c.source, c.segment_index) {
                (ChunkSource::Transcript, Some(seg)) => {
                    let mut all: Vec<u32> = prepared
                        .chunks
                        .get(&c.meeting_id)
                        .into_iter()
                        .flatten()
                        .filter(|row| {
                            row.source == ChunkSource::Transcript && row.segment_ids.contains(&seg)
                        })
                        .flat_map(|row| row.segment_ids.iter().copied())
                        .collect();
                    all.sort_unstable();
                    all.dedup();
                    all
                }
                _ => Vec::new(),
            };
            CitedRef {
                meeting: prepared.name_of(&c.meeting_id),
                source: c.source,
                segment_index: c.segment_index,
                ref_key: c.ref_key.clone(),
                chunk_segments,
            }
        })
        .collect();
    AnswerView {
        text: answer.text.clone(),
        not_found: answer.not_found,
        citations,
    }
}

/// Lag je erwarteter Besprechung eine erwartete Stelle in den gelesenen
/// Auszuegen? Aus dem gespeicherten Verlauf (`MessageMeta::excerpt_chunk_ids`).
/// Eine Besprechung, die ganz gelesen wurde (Frage an eine Besprechung,
/// Auszuege ohne Chunk), zaehlt als gelesen. `None`: unbekannt.
pub fn retrieved(
    store: &MeetingStore,
    prepared: &Prepared,
    q: &EvalQuestion,
    answer: &ChatAnswer,
) -> Option<BTreeMap<String, bool>> {
    let sources = q.expect.all_sources();
    if sources.is_empty() {
        return None;
    }
    let (_, rows) = store.thread_get(&answer.thread_id).ok().flatten()?;
    let meta: MessageMeta = rows
        .iter()
        .rev()
        .find(|m| m.role == "assistant")
        .and_then(|m| m.coverage_json.as_deref())
        .and_then(|j| serde_json::from_str(j).ok())?;
    let whole = meta.excerpt_chunk_ids.is_empty()
        && matches!(q.scope, QuestionScope::Meeting { .. })
        && meta.coverage.excerpts_read > 0;
    let mut out: BTreeMap<String, bool> = BTreeMap::new();
    for s in &sources {
        let hit = whole
            || meta.excerpt_chunk_ids.iter().any(|id| {
                prepared.chunk_by_id(*id).is_some_and(|row| {
                    prepared.name_of(&row.meeting_id).as_deref() == Some(s.meeting.as_str())
                        && match row.source {
                            ChunkSource::Transcript => {
                                s.segments.iter().any(|seg| row.segment_ids.contains(seg))
                            }
                            ChunkSource::UserNotes => {
                                s.notes.iter().any(|n| row.ref_keys.contains(n))
                            }
                            _ => false,
                        }
                })
            });
        *out.entry(s.meeting.clone()).or_default() |= hit;
    }
    Some(out)
}

/// Wer die Fragen beantwortet: der echte Chat-Ablauf oder ein Stub (Tests).
pub trait Answerer: Send + Sync {
    fn answer<'a>(&'a self, req: ChatRequest) -> BoxFuture<'a, Result<ChatAnswer, String>>;
}

/// Der Chat der App (`controller::ask_guarded`) mit eigenem Guard.
pub struct ControllerAnswerer {
    pub settings: AppSettings,
    pub store: Arc<MeetingStore>,
    pub embed: Arc<dyn Embedder>,
    pub env: ChatEnv,
}

impl Answerer for ControllerAnswerer {
    fn answer<'a>(&'a self, req: ChatRequest) -> BoxFuture<'a, Result<ChatAnswer, String>> {
        Box::pin(async move {
            let flag = AtomicBool::new(false);
            let progress = |_: ChatProgress<'_>| {};
            ask_guarded(
                &flag,
                &self.settings,
                self.store.clone(),
                self.embed.clone(),
                None,
                req,
                &self.env,
                &progress,
            )
            .await
            .map_err(|e| e.to_string())
        })
    }
}

/// Ergebnis einer Frage.
pub struct QuestionOutcome {
    pub kind: String,
    pub verdict: Verdict,
    pub cause: Option<&'static str>,
    pub elapsed_ms: u64,
    pub lexical_only: Option<bool>,
    pub report: Value,
}

/// Alle Fragen seriell. Fortschritt auf stderr (nur ID und Ergebnis).
pub async fn run_questions(
    store: &MeetingStore,
    prepared: &Prepared,
    answerer: &dyn Answerer,
) -> Vec<QuestionOutcome> {
    let mut out = Vec::new();
    for q in &prepared.set.questions {
        let started = Instant::now();
        let request = build_request(q, prepared);
        let result = match request {
            Ok(req) => answerer.answer(req).await,
            Err(e) => Err(e),
        };
        let elapsed_ms = started.elapsed().as_millis() as u64;
        let mut report = json!({
            "id": q.id,
            "kind": q.kind,
            "scope": q.scope,
            "question": q.question,
            "elapsed_ms": elapsed_ms,
        });
        let obj = report.as_object_mut().expect("json object");
        let (verdict, cause, lexical_only) = match result {
            Ok(answer) => {
                let view = view_of(prepared, &answer);
                let verdict = judge(&q.expect, &view);
                let got = retrieved(store, prepared, q, &answer);
                // Fehlt eine Schluesselgruppe, braucht es alle erwarteten Besprechungen.
                let all_read = got.as_ref().map(|m| m.values().all(|v| *v));
                let cause = failure_cause(&verdict, all_read);
                obj.insert("answer".into(), json!(answer.text));
                obj.insert("not_found".into(), json!(answer.not_found));
                obj.insert("citations".into(), json!(view.citations));
                obj.insert("expected_retrieved".into(), json!(got));
                obj.insert("coverage".into(), json!(answer.coverage));
                (verdict, cause, Some(answer.coverage.lexical_only))
            }
            Err(error) => {
                obj.insert("error".into(), json!(error));
                (Verdict::error(), Some("error"), None)
            }
        };
        obj.insert("correct".into(), json!(verdict.correct));
        obj.insert("verdict".into(), json!(verdict));
        obj.insert("cause".into(), json!(cause));
        eprintln!(
            "eval-chat: {} {} ({} ms){}",
            q.id,
            if verdict.correct { "richtig" } else { "FALSCH" },
            elapsed_ms,
            cause.map(|c| format!(" Ursache {c}")).unwrap_or_default()
        );
        out.push(QuestionOutcome {
            kind: q.kind.clone(),
            verdict,
            cause,
            elapsed_ms,
            lexical_only,
            report,
        });
    }
    out
}

fn round3(value: f64) -> f64 {
    (value * 1000.0).round() / 1000.0
}

fn percentile(sorted: &[u64], p: f64) -> u64 {
    if sorted.is_empty() {
        return 0;
    }
    let rank = ((p * sorted.len() as f64).ceil() as usize).clamp(1, sorted.len());
    sorted[rank - 1]
}

/// Kennzahlen ueber alle Fragen.
pub fn aggregate(outcomes: &[QuestionOutcome], meetings: usize) -> Value {
    let total = outcomes.len();
    let correct = outcomes.iter().filter(|o| o.verdict.correct).count();
    let accuracy = if total == 0 {
        0.0
    } else {
        round3(correct as f64 / total as f64)
    };
    let mut by_kind: BTreeMap<String, (u32, u32)> = BTreeMap::new();
    let mut causes: BTreeMap<&'static str, u32> = BTreeMap::new();
    for o in outcomes {
        let e = by_kind.entry(o.kind.clone()).or_default();
        e.0 += 1;
        e.1 += u32::from(o.verdict.correct);
        if let Some(c) = o.cause {
            *causes.entry(c).or_default() += 1;
        }
    }
    let mut ms: Vec<u64> = outcomes.iter().map(|o| o.elapsed_ms).collect();
    ms.sort_unstable();
    let mean = if ms.is_empty() {
        0
    } else {
        ms.iter().sum::<u64>() / ms.len() as u64
    };
    json!({
        "questions": total,
        "correct": correct,
        "accuracy": accuracy,
        "meetings": meetings,
        "errors": outcomes.iter().filter(|o| o.verdict.reason == "error").count(),
        "lexical_only_answers": outcomes.iter().filter(|o| o.lexical_only == Some(true)).count(),
        "by_kind": by_kind.iter().map(|(k, (n, c))| (k.clone(), json!({"questions": n, "correct": c}))).collect::<serde_json::Map<_, _>>(),
        "causes": causes,
        "question_ms": {
            "mean": mean,
            "p50": percentile(&ms, 0.5),
            "p95": percentile(&ms, 0.95),
            "max": ms.last().copied().unwrap_or(0),
        },
    })
}

/// Aufrufoptionen des CLI-Laufs.
pub struct CliOptions {
    /// Ohne Embedding-Modell (Vergleichslauf).
    pub lexical_only: bool,
    /// Das lokale Backend ist die CPU (kleineres Budget, wie in der App).
    pub backend_cpu: bool,
}

/// Der headless Lauf `--eval-chat <dir>`. `sandbox` ist ein frisches
/// Temp-Verzeichnis (der Aufrufer raeumt es weg und stoppt beide Server).
/// Synchron: der Index-Aufbau blockiert selbst (siehe `prepare`), die Fragen
/// laufen danach auf dem Tauri-Runtime.
pub fn run_cli(
    settings: AppSettings,
    dir: &Path,
    sandbox: &Path,
    opts: CliOptions,
) -> (i32, Value) {
    let fail = |message: String| (EXIT_ERROR, json!({ "mode": "eval-chat", "error": message }));
    let (provider, model, _) = match resolve_provider_coded(&settings) {
        Ok(p) => p,
        Err(e) => return fail(format!("{}: {}", e.code, e.message)),
    };
    let local = crate::managers::llm::is_local(&provider);
    let store = match MeetingStore::open_at(&sandbox.join("meetings.db")) {
        Ok(s) => Arc::new(s),
        Err(e) => return fail(format!("Sandbox-Store nicht anlegbar: {e}")),
    };
    let embed: Arc<dyn Embedder> = if opts.lexical_only {
        Arc::new(NoVectors)
    } else {
        Arc::new(LlamaEmbedder::new(crate::managers::llm::EMBED_MODEL_ID))
    };
    let started = Instant::now();
    eprintln!("eval-chat: Besprechungen anlegen, Index aufbauen ...");
    let prepared = match prepare(
        &store,
        dir,
        embed.clone(),
        crate::managers::meetings::search::vectors::global_cache(),
        &crate::managers::llm::embedding_pid,
    ) {
        Ok(p) => p,
        Err(e) => return fail(e),
    };
    let index_ms = started.elapsed().as_millis() as u64;
    let today = prepared
        .set
        .today
        .as_deref()
        .and_then(parse_day)
        .and_then(|d| local_ts(d, 12, 0, 0))
        .unwrap_or_else(|| chrono::Utc::now().timestamp());
    let answerer = ControllerAnswerer {
        settings,
        store: store.clone(),
        embed: embed.clone(),
        env: ChatEnv {
            ctx_tokens: crate::managers::llm::DEFAULT_CONTEXT_TOKENS,
            backend_cpu: local && opts.backend_cpu,
            recording_active: false,
            now: today,
            cancel: CancelFlag::default(),
            enhance_running: Arc::new(|| false),
        },
    };
    let outcomes = tauri::async_runtime::block_on(run_questions(&store, &prepared, &answerer));
    let meetings = prepared.ids.len();
    let mut aggregate = aggregate(&outcomes, meetings);
    let accuracy = aggregate["accuracy"].as_f64().unwrap_or(0.0);
    aggregate["elapsed_ms"] = json!(started.elapsed().as_millis() as u64);
    aggregate["index_ms"] = json!(index_ms);
    let all_failed = !outcomes.is_empty() && outcomes.iter().all(|o| o.verdict.reason == "error");
    let passed = !all_failed && accuracy >= TARGET_ACCURACY;
    let code = if all_failed {
        EXIT_ERROR
    } else if passed {
        EXIT_OK
    } else {
        EXIT_MISSED
    };
    let payload = json!({
        "mode": "eval-chat",
        "provider": provider.id,
        "model": model,
        "local": local,
        "backend_cpu": local && opts.backend_cpu,
        "context_tokens": local.then_some(crate::managers::llm::DEFAULT_CONTEXT_TOKENS),
        "lexical_only_requested": opts.lexical_only,
        "embedding_model": embed.model_id(),
        "index": prepared.index,
        "questions": outcomes.len(),
        "meetings": meetings,
        "target_accuracy": TARGET_ACCURACY,
        "aggregate": aggregate,
        "items": outcomes.iter().map(|o| o.report.clone()).collect::<Vec<_>>(),
        "passed": passed,
        "exit_code": code,
    });
    drop(answerer);
    drop(store);
    (code, payload)
}

/// Kurzfassung fuer die Konsole (ohne `--json`).
pub fn summary_lines(payload: &Value) -> Vec<String> {
    if let Some(error) = payload.get("error").and_then(Value::as_str) {
        return vec![format!("eval-chat: Fehler: {error}")];
    }
    let mut lines = vec![format!(
        "eval-chat: Modell {} ({}), Einbettung {}, {} Fragen ueber {} Besprechungen",
        payload["model"].as_str().unwrap_or("?"),
        payload["provider"].as_str().unwrap_or("?"),
        payload["embedding_model"]
            .as_str()
            .filter(|m| !m.is_empty())
            .unwrap_or("keine (nur Stichwortsuche)"),
        payload["questions"],
        payload["meetings"],
    )];
    for item in payload["items"].as_array().into_iter().flatten() {
        lines.push(format!(
            "  {:<4} {:<13} {:<7} {:>7} ms  {}",
            item["id"].as_str().unwrap_or("?"),
            item["kind"].as_str().unwrap_or(""),
            if item["correct"].as_bool() == Some(true) {
                "richtig"
            } else {
                "FALSCH"
            },
            item["elapsed_ms"],
            item["cause"].as_str().unwrap_or(""),
        ));
    }
    let a = &payload["aggregate"];
    lines.push(format!(
        "eval-chat: {}/{} richtig, Genauigkeit {:.3} (Ziel {:.2}), Frage p50 {} ms, p95 {} ms",
        a["correct"],
        a["questions"],
        a["accuracy"].as_f64().unwrap_or(0.0),
        TARGET_ACCURACY,
        a["question_ms"]["p50"],
        a["question_ms"]["p95"],
    ));
    lines
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::managers::meetings::chat::{Citation, Coverage};
    use crate::managers::meetings::llm_call::test_support::{
        chat_body, settings_with_mock_provider, spawn_llm_mock_with, MockReply,
    };

    // ---- Normalisierung ---------------------------------------------------

    #[test]
    fn number_words_become_digits() {
        assert_eq!(normalize("achtundzwanzig Monteure"), "28 monteure");
        assert_eq!(normalize("fünfundfünfzigtausend Euro"), "55000 euro");
        assert_eq!(normalize("viertausendfünfhundert"), "4500");
        assert_eq!(normalize("hundertfünfzigtausend"), "150000");
        assert_eq!(normalize("dreihundertzwölf Fehler"), "312 fehler");
        assert_eq!(normalize("am fünfzehnten"), "am 15");
        assert_eq!(normalize("zum ersten Februar"), "zum 1 februar");
        assert_eq!(
            normalize("den zweiundzwanzigsten Oktober"),
            "den 22 oktober"
        );
        assert_eq!(normalize("eins Komma acht Sekunden"), "1.8 sekunden");
        assert_eq!(normalize("ein Termin"), "ein termin", "Artikel bleibt Wort");
        assert_eq!(normalize("die Kunden kosten"), "die kunden kosten");
    }

    #[test]
    fn digits_are_unified() {
        assert_eq!(normalize("55.000 €"), "55000");
        assert_eq!(normalize("1.800.000"), "1800000");
        assert_eq!(normalize("1,8 s"), "1.8 s");
        assert_eq!(normalize("am 22.10.2026"), "am 22.10.2026");
        assert_eq!(normalize("Straße, 3. November!"), "strasse 3 november");
    }

    #[test]
    fn keys_match_at_token_boundaries() {
        let t = normalize("Es sind 150 Tickets, davon 3 kritisch; Start am 1.8. im Archivraum.");
        assert!(contains_key(&t, "150"));
        assert!(!contains_key(&t, "15"), "15 steckt nicht in 150");
        assert!(contains_key(&t, "3"));
        assert!(contains_key(&t, "archiv"), "Wortanfang reicht");
        assert!(!contains_key(&t, "raum"), "kein Treffer mitten im Wort");
        assert!(!contains_key(&t, "1"), "1 steckt nicht in 1.8");
        assert!(contains_key(&t, "1.8"));
        assert!(contains_key(&t, "Tickets"));
        assert!(contains_key(
            &normalize("Das sind 55.000 Euro."),
            "fünfundfünfzigtausend"
        ));
    }

    // ---- Bewertung ----------------------------------------------------------

    fn expect(all_of: &[&[&str]], meeting: &str, segments: &[u32]) -> Expect {
        Expect {
            all_of: all_of
                .iter()
                .map(|g| g.iter().map(|s| s.to_string()).collect())
                .collect(),
            sources: vec![ExpectSource {
                meeting: meeting.into(),
                segments: segments.to_vec(),
                notes: vec!["N1".into()],
            }],
            ..Expect::default()
        }
    }

    fn cite(meeting: &str, seg: u32, chunk: &[u32]) -> CitedRef {
        CitedRef {
            meeting: Some(meeting.into()),
            source: ChunkSource::Transcript,
            segment_index: Some(seg),
            ref_key: None,
            chunk_segments: chunk.to_vec(),
        }
    }

    fn view(text: &str, citations: Vec<CitedRef>) -> AnswerView {
        AnswerView {
            text: text.into(),
            not_found: false,
            citations,
        }
    }

    #[test]
    fn a_right_answer_needs_all_keys_and_a_citation_on_an_expected_chunk() {
        let e = expect(&[&["dienstag"], &["10", "zehn"]], "jf", &[54]);
        let ok = judge(
            &e,
            &view(
                "Am Dienstag um zehn Uhr [1].",
                vec![cite("jf", 53, &[50, 53, 54])],
            ),
        );
        assert!(ok.correct, "{ok:?}");
        assert_eq!(ok.reason, "ok");

        let keys = judge(&e, &view("Am Dienstag [1].", vec![cite("jf", 54, &[54])]));
        assert!(!keys.correct);
        assert_eq!(
            keys.missing_groups,
            vec![vec!["10".to_string(), "zehn".to_string()]]
        );
        assert_eq!(keys.reason, "keys_missing");

        let far = judge(
            &e,
            &view("Dienstag, 10 Uhr [1].", vec![cite("jf", 3, &[0, 1, 2, 3])]),
        );
        assert!(!far.correct);
        assert_eq!(far.reason, "citation_wrong");

        let other = judge(
            &e,
            &view("Dienstag, 10 Uhr [1].", vec![cite("kg", 54, &[54])]),
        );
        assert_eq!(other.reason, "citation_wrong", "andere Besprechung");

        let none = judge(&e, &view("Dienstag, 10 Uhr.", vec![]));
        assert_eq!(none.reason, "no_citation");

        let note = CitedRef {
            meeting: Some("jf".into()),
            source: ChunkSource::UserNotes,
            segment_index: None,
            ref_key: Some("N1".into()),
            chunk_segments: vec![],
        };
        assert!(judge(&e, &view("Dienstag 10 Uhr [1]", vec![note])).correct);

        let mut nf = view("", vec![]);
        nf.not_found = true;
        assert_eq!(judge(&e, &nf).reason, "not_found");
    }

    #[test]
    fn an_unanswerable_question_needs_not_found_without_citations() {
        let e = Expect {
            not_found: true,
            ..Expect::default()
        };
        let mut nf = view("", vec![]);
        nf.not_found = true;
        assert!(judge(&e, &nf).correct);
        let answered = judge(
            &e,
            &view("Es sind 5000 Euro [1].", vec![cite("jf", 1, &[1])]),
        );
        assert_eq!(answered.reason, "answered_unanswerable");
        assert_eq!(failure_cause(&answered, None), Some("model"));
    }

    #[test]
    fn causes_separate_retrieval_model_and_citation() {
        let e = expect(&[&["dienstag"]], "jf", &[54]);
        let missing = judge(&e, &view("Montag [1]", vec![cite("jf", 54, &[54])]));
        assert_eq!(failure_cause(&missing, Some(false)), Some("retrieval"));
        assert_eq!(failure_cause(&missing, Some(true)), Some("model"));
        let wrong = judge(&e, &view("Dienstag [1]", vec![cite("jf", 1, &[1])]));
        assert_eq!(failure_cause(&wrong, Some(true)), Some("citation"));
        let ok = judge(&e, &view("Dienstag [1]", vec![cite("jf", 54, &[54])]));
        assert_eq!(failure_cause(&ok, Some(true)), None);
    }

    // ---- Lauf mit Stub-LLM ---------------------------------------------------

    const FACT_BUDGET: &str = "Das Budget beträgt fünfundfünfzigtausend Euro.";
    const FACT_TERMIN: &str = "Der Termin ist am Dienstag um zehn Uhr.";
    const FACT_WAWI: &str = "Die Warenwirtschaft heißt Sanitas Office.";

    fn segments(facts: &[(u32, &str)]) -> Vec<Value> {
        (0..40u32)
            .map(|i| {
                let text = facts
                    .iter()
                    .find(|(at, _)| *at == i)
                    .map(|(_, t)| t.to_string())
                    .unwrap_or_else(|| {
                        format!(
                            "Punkt {i}: Wir sprechen kurz über Nebensächliches wie Wetter, Kaffee und die Parkplätze vor dem Haus."
                        )
                    });
                json!({"segment_index": i, "text": text, "start_ms": i * 10_000,
                       "end_ms": i * 10_000 + 8_000, "channel": 0, "speaker_index": null})
            })
            .collect()
    }

    /// Zwei Besprechungen, vier Fragen (zwei in einer Besprechung, eine global,
    /// eine unbeantwortbar), Ordner und Teilnehmer.
    fn catalog() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let alpha = json!({"title": "Alpha", "folder": "Team", "participants": ["Anna"],
            "started_at": "2026-09-01T10:00:00+02:00",
            "segments": segments(&[(3, FACT_BUDGET), (35, FACT_TERMIN)]), "notes": []});
        let beta = json!({"title": "Beta", "segments": segments(&[(5, FACT_WAWI)])});
        std::fs::write(dir.path().join("alpha.json"), alpha.to_string()).unwrap();
        std::fs::write(dir.path().join("beta.json"), beta.to_string()).unwrap();
        let questions = json!({
            "today": "2026-09-29",
            "meetings": [
                {"name": "alpha", "file": "alpha.json"},
                {"name": "beta", "file": "beta.json", "folder": "Vertrieb",
                 "started_at": "2026-08-01T10:00:00+02:00"}
            ],
            "questions": [
                {"id": "q1", "kind": "meeting", "scope": {"kind": "meeting", "meeting": "alpha"},
                 "question": "Wie hoch ist das Budget?",
                 "expect": {"all_of": [["55000"]], "meeting": "alpha", "segments": [3]}},
                {"id": "q2", "kind": "meeting", "scope": {"kind": "meeting", "meeting": "alpha"},
                 "question": "Wann ist der Termin?",
                 "expect": {"all_of": [["dienstag"], ["10"]], "sources": [{"meeting": "alpha", "segments": [35]}]}},
                {"id": "q3", "kind": "filter", "scope": {"kind": "global", "folder": "Vertrieb"},
                 "question": "Welche Warenwirtschaft wird genutzt?",
                 "expect": {"all_of": [["sanitas"]], "sources": [{"meeting": "beta", "segments": [5]}]}},
                {"id": "q4", "kind": "unanswerable", "scope": {"kind": "global"},
                 "question": "Wie hoch ist die Miete für das Büro?",
                 "expect": {"not_found": true}}
            ]
        });
        std::fs::write(dir.path().join(QUESTIONS_FILE), questions.to_string()).unwrap();
        dir
    }

    static TEST_CACHE: VectorCache = VectorCache::new();

    fn sandbox(catalog: &Path) -> (tempfile::TempDir, Arc<MeetingStore>, Prepared) {
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(MeetingStore::open_at(&dir.path().join("meetings.db")).unwrap());
        let prepared =
            prepare(&store, catalog, Arc::new(NoVectors), &TEST_CACHE, &|| None).unwrap();
        (dir, store, prepared)
    }

    #[derive(Clone, Copy)]
    enum Mode {
        Perfect,
        WrongCitation,
    }

    /// Stub: antwortet aus den Erwartungen selbst, Zitat auf die erwartete
    /// Stelle (perfekt) oder 20 Segmente daneben (falsches Zitat).
    struct Stub {
        mode: Mode,
        questions: HashMap<String, EvalQuestion>,
        ids: BTreeMap<String, String>,
    }

    impl Answerer for Stub {
        fn answer<'a>(&'a self, req: ChatRequest) -> BoxFuture<'a, Result<ChatAnswer, String>> {
            Box::pin(async move {
                let q = &self.questions[&req.question];
                let base = ChatAnswer {
                    thread_id: String::new(),
                    message_id: String::new(),
                    text: String::new(),
                    citations: vec![],
                    coverage: Coverage::default(),
                    not_found: false,
                    uncited: false,
                    provider_local: true,
                };
                let first = q.expect.all_sources().into_iter().next();
                let (src_name, seg) = match (&first, self.mode) {
                    (None, Mode::Perfect) => {
                        return Ok(ChatAnswer {
                            not_found: true,
                            ..base
                        })
                    }
                    (None, Mode::WrongCitation) => ("alpha".to_string(), 20),
                    (Some(s), Mode::Perfect) => (s.meeting.clone(), s.segments[0]),
                    (Some(s), Mode::WrongCitation) => {
                        (s.meeting.clone(), (s.segments[0] + 20) % 40)
                    }
                };
                let text = if q.expect.all_of.is_empty() {
                    "Irgendetwas [1].".to_string()
                } else {
                    let keys: Vec<&str> = q.expect.all_of.iter().map(|g| g[0].as_str()).collect();
                    format!("{} [1].", keys.join(", "))
                };
                Ok(ChatAnswer {
                    text,
                    citations: vec![Citation {
                        n: 1,
                        meeting_id: self.ids[&src_name].clone(),
                        meeting_title: src_name,
                        started_at: None,
                        source: ChunkSource::Transcript,
                        epoch: 0,
                        segment_index: Some(seg),
                        start_ms: None,
                        ref_key: None,
                        quote: String::new(),
                    }],
                    ..base
                })
            })
        }
    }

    fn stub(prepared: &Prepared, mode: Mode) -> Stub {
        Stub {
            mode,
            questions: prepared
                .set
                .questions
                .iter()
                .map(|q| (q.question.clone(), q.clone()))
                .collect(),
            ids: prepared.ids.clone(),
        }
    }

    #[test]
    fn the_catalog_is_imported_with_folders_participants_and_chunks() {
        let cat = catalog();
        let (_dir, store, prepared) = sandbox(cat.path());
        assert_eq!(prepared.ids.len(), 2);
        assert!(
            prepared.index.chunks >= 6,
            "je Besprechung mehrere Chunks: {:?}",
            prepared.index
        );
        assert_eq!(prepared.index.vectors, 0);
        assert!(prepared.chunks.values().all(|rows| rows.len() >= 3));
        let alpha = &prepared.ids["alpha"];
        let by_person = store
            .resolve_scope(&ScopeFilter {
                person: Some("anna".into()),
                ..ScopeFilter::default()
            })
            .unwrap();
        assert_eq!(&by_person, &vec![alpha.clone()], "Teilnehmer als Sprecher");
        let beta_only = store
            .resolve_scope(&ScopeFilter {
                folder_id: Some(prepared.folders["Vertrieb"].clone()),
                ..ScopeFilter::default()
            })
            .unwrap();
        assert_eq!(beta_only, vec![prepared.ids["beta"].clone()]);
        let september = build_request(
            &EvalQuestion {
                id: "x".into(),
                kind: String::new(),
                scope: QuestionScope::Global {
                    folder: None,
                    person: None,
                    from: Some("2026-09-01".into()),
                    to: Some("2026-09-30".into()),
                },
                question: "?".into(),
                expect: Expect::default(),
            },
            &prepared,
        )
        .unwrap();
        let ChatScope::Global { filter } = september.scope else {
            panic!("global erwartet")
        };
        assert_eq!(store.resolve_scope(&filter).unwrap(), vec![alpha.clone()]);
    }

    #[test]
    fn a_catalog_with_a_missing_segment_is_rejected() {
        let cat = catalog();
        let path = cat.path().join(QUESTIONS_FILE);
        let text = std::fs::read_to_string(&path)
            .unwrap()
            .replace("[35]", "[99]");
        std::fs::write(&path, text).unwrap();
        let err = load_set(cat.path()).unwrap_err();
        assert!(err.contains("Segment 99"), "{err}");
    }

    #[tokio::test]
    async fn perfect_stub_answers_score_one_and_wrong_citations_score_zero() {
        let cat = catalog();
        let (_dir, store, prepared) = sandbox(cat.path());

        let perfect = run_questions(&store, &prepared, &stub(&prepared, Mode::Perfect)).await;
        let agg = aggregate(&perfect, prepared.ids.len());
        assert_eq!(agg["questions"], 4);
        assert_eq!(agg["accuracy"].as_f64(), Some(1.0), "{agg}");

        let wrong = run_questions(&store, &prepared, &stub(&prepared, Mode::WrongCitation)).await;
        let agg = aggregate(&wrong, prepared.ids.len());
        assert_eq!(agg["accuracy"].as_f64(), Some(0.0), "{agg}");
        for o in &wrong {
            assert!(!o.verdict.correct);
            let id = o.report["id"].as_str().unwrap();
            let expected = if id == "q4" {
                "answered_unanswerable"
            } else {
                "citation_wrong"
            };
            assert_eq!(o.verdict.reason, expected, "{id}");
        }
    }

    /// Nimmt aus dem Nutzerprompt die Frage und das Auszugs-ID, dessen Text
    /// die Stelle enthaelt; antwortet mit den Schluesseln und `[Qk]`.
    fn prompt_stub(body: &str, facts: &HashMap<String, (String, String)>) -> MockReply {
        let v: Value = serde_json::from_str(body).unwrap();
        let prompt = v["messages"][1]["content"]
            .as_str()
            .unwrap_or("")
            .to_string();
        let question = prompt
            .lines()
            .rev()
            .find_map(|l| l.strip_prefix("Frage: "))
            .unwrap_or("")
            .trim()
            .to_string();
        let Some((snippet, reply)) = facts.get(&question) else {
            return MockReply::Body(chat_body("KEIN_BELEG"));
        };
        let mut current: Option<String> = None;
        for line in prompt.lines() {
            if let Some(rest) = line.strip_prefix("[Q") {
                current = rest.split(']').next().map(str::to_string);
            }
            if line.contains(snippet.as_str()) {
                if let Some(k) = &current {
                    return MockReply::Body(chat_body(&format!("{reply} [Q{k}].")));
                }
            }
        }
        MockReply::Body(chat_body("KEIN_BELEG"))
    }

    #[tokio::test]
    async fn the_real_chat_with_a_stub_llm_scores_one() {
        let cat = catalog();
        let (_dir, store, prepared) = sandbox(cat.path());
        let facts: HashMap<String, (String, String)> = [
            (
                "Wie hoch ist das Budget?",
                FACT_BUDGET,
                "Das Budget beträgt 55000 Euro",
            ),
            (
                "Wann ist der Termin?",
                FACT_TERMIN,
                "Der Termin ist am Dienstag um 10 Uhr",
            ),
            (
                "Welche Warenwirtschaft wird genutzt?",
                FACT_WAWI,
                "Die Warenwirtschaft heißt Sanitas Office",
            ),
        ]
        .into_iter()
        .map(|(q, s, r)| (q.to_string(), (s.to_string(), r.to_string())))
        .collect();
        let port = spawn_llm_mock_with(move |body| prompt_stub(body, &facts)).await;
        let answerer = ControllerAnswerer {
            settings: settings_with_mock_provider(port),
            store: store.clone(),
            embed: Arc::new(NoVectors),
            env: ChatEnv {
                ctx_tokens: 8_192,
                backend_cpu: false,
                recording_active: false,
                now: 1_790_000_000,
                cancel: CancelFlag::default(),
                enhance_running: Arc::new(|| false),
            },
        };
        let outcomes = run_questions(&store, &prepared, &answerer).await;
        for o in &outcomes {
            assert!(o.verdict.correct, "{}", o.report);
        }
        let agg = aggregate(&outcomes, prepared.ids.len());
        assert_eq!(agg["accuracy"].as_f64(), Some(1.0));
        let q1 = &outcomes[0].report;
        assert_eq!(
            q1["expected_retrieved"],
            json!({"alpha": true}),
            "ganz gelesen: {q1}"
        );
        assert_eq!(
            outcomes[2].report["expected_retrieved"],
            json!({"beta": true}),
            "per Suche gelesen"
        );
    }
}
