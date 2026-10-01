//! Die Bausteine „Datei importieren und transkribieren“ (`meeting.import`) und „YouTube-Quelle
//! anlegen“ (`youtube.add_source`) (B3).
//!
//! # `meeting.import`: Datei -> Besprechung (im Projekt)
//!
//! ```json
//! {"id": "imp", "action": "meeting.import",
//!  "params": {"via": "eingang", "path": "{{trigger.path}}", "title": "{{trigger.name}}", "project": "<ordner-id>"}}
//! ```
//!
//! Der Baustein reiht die Datei in die VORHANDENE Import-Warteschlange der Besprechungen ein
//! (`meetings::queue`, dieselbe wie „Datei importieren“ in der Oberflaeche: Reihenfolge,
//! Fortschritt, Pause und Stopp, Aufnahme hat Vorrang, gleichzeitige Transkriptionen, RAM- und
//! GPU-Tor) und kehrt SOFORT zurueck; Untertitel (VTT/SRT) brauchen keine Transkription und sind
//! sofort fertig. Ausgabe: `steps.<id>.meeting.id` (wird zum `meeting` des Laufs),
//! `steps.<id>.status` (`queued` oder `ready`).
//!
//! **Warum der Schritt nicht auf das Ende der Transkription wartet.** Es gibt EINEN Arbeiter je
//! Engine; ein Schritt, der eine Stunde blockiert, liesse in der Zeit keinen Termin starten und
//! keine Einwilligung entgegennehmen, und die Aufnahme, auf die die Transkription Ruecksicht
//! nimmt, koennte nie beginnen. Ein „Warten per Defer“ liefe bei jedem Versuch erneut durch das
//! Tor (Audit je Versuch, bei „fragen“ jedes Mal eine neue Freigabe). Stattdessen gilt:
//! - Schritte, die ein fertiges Transkript brauchen, pruefen es selbst und warten per `Defer`
//!   (sie haben kein Recht am Tor, siehe [`meeting_state`]); das ist Sache der Bausteine
//!   „Notizen“, „Protokoll“ und „Zusammenfassung“ (B4).
//! - Alle schweren Schritte der Engine laufen NICHT, solange die Import-Warteschlange arbeitet
//!   oder eine Aufnahme laeuft (`queue_gate::QueueAwareGate`): zwei gleichzeitig ausgeloeste
//!   Dateien werden nacheinander transkribiert, Notizen entstehen erst danach.
//!
//! **Rechte.** `files.read` an der Ordner-Integration `via` (Katalog; Vorgabe fuer Ablaeufe:
//! erlaubt). Die Datei muss UNTER der Wurzel dieser Integration liegen (Sandbox
//! `integrations::folder::Sandbox`: kein `..`, keine Verknuepfung nach aussen); ein Pfad
//! ausserhalb ist ein dauerhafter Fehler, nie ein stilles Lesen. Nur Ordner-Integrationen
//! (Art `folder`) sind hier Quelle. Liegt die Datei nur in der Cloud (OneDrive-Platzhalter),
//! wird sie NICHT gelesen und der Schritt scheitert dauerhaft mit Hinweis (kein Download).
//!
//! **Einwilligung (#15).** Das Einreihen hinterlegt keine Einwilligung (`consent_confirmed_at`
//! leer): sie wurde nicht erfragt; der Import ist nach der Regel der App („nur eine
//! Oberflaechenpruefung“) kein Aufnehmen.
//!
//! **Idempotenz** (`External`): vor dem Einreihen sucht der Baustein eine Besprechung mit
//! derselben Quelldatei, die seit dem Beginn des Schritts entstand (`find_since`); gibt es sie
//! (Wiederholung, Absturz zwischen Einreihen und Journal), wird sie wiederverwendet, nie ein
//! zweites Mal eingereiht. `confirm` liefert denselben Beleg nach einem Absturz.
//!
//! # `youtube.add_source`: Video -> Quelle (A2-Weg)
//!
//! Legt aus dem Video-Link eine Besprechung mit Quelle `youtube` an (der vorhandene Weg
//! `youtube::source::add_youtube_source`: ein oEmbed-Abruf fuer Titel und Kanal, Audit,
//! Projektzuordnung, Provenienz). Gibt es zu dieser Video-Kennung schon eine Besprechung, wird
//! SIE verwendet (`reused: true`): dasselbe Video landet nie zweimal in der Wissensbasis. Recht:
//! `youtube.add` an der Integration `via`. Transkript, Untertitel und eigene Transkription sind
//! nicht Teil dieses Bausteins (A3-Weg, Baustein `youtube.transcript`, B6).
//!
//! Die App-Seite (`ImportQueue`, `MeetingStore`, `AppHandle`) steckt hinter [`ImportControl`]
//! und [`YoutubeControl`] (`import_app` setzt die echten ein, Tests Attrappen).

use std::path::{Path, PathBuf};
use std::sync::Arc;

use rusqlite::Connection;
use serde_json::{json, Value};

use crate::managers::integrations::folder::{FolderConfig, Sandbox};
use crate::managers::integrations::model::Kind;
use crate::managers::integrations::store as register;
use crate::managers::meetings::queue::title_from_path;
use crate::managers::provenance::SourceRef;
use crate::managers::youtube::{normalize_link, YoutubeError};

use super::action::{
    Action, EffectKind, HeavyNeed, Needs, NeedsError, RunCtx, StepError, StepOutput,
};
use super::catalog::{self, ActionSpec};
use super::engine::Engine;
use super::trigger::folder::{default_extensions, display_path, FileProbe, SystemProbe};

/// Laengster Titel einer Besprechung (wie `meetings::metadata`).
const TITLE_MAX_CHARS: usize = 300;
/// So viel frueher als der Schrittbeginn zaehlt eine Besprechung noch als „von diesem Schritt“.
const SINCE_SLACK_MS: i64 = 2_000;

// ---------------------------------------------------------------------------
// Schnittstelle zur App
// ---------------------------------------------------------------------------

/// Was eingereiht werden soll.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImportRequest {
    pub path: PathBuf,
    pub title: String,
    /// Kennung des Projekts (Ordner); `None`: keins.
    pub project: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImportState {
    /// Wartet in der Import-Warteschlange (oder laeuft dort).
    Queued,
    /// Fertig (Untertitel).
    Ready,
}

impl ImportState {
    pub fn as_str(self) -> &'static str {
        match self {
            ImportState::Queued => "queued",
            ImportState::Ready => "ready",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Imported {
    pub meeting_id: String,
    pub title: String,
    pub state: ImportState,
}

/// Die Import-Warteschlange und der Speicher der Besprechungen, wie die Bausteine sie sehen.
pub trait ImportControl: Send + Sync {
    /// Reiht die Datei ein (Medien) bzw. importiert sie sofort (Untertitel). `Err` ist ein Code
    /// der Warteschlange (`queue_enqueue_failed: ...`, `folder_not_found`, `subtitle_invalid`,
    /// `meetings_unavailable`).
    fn enqueue(&self, req: &ImportRequest) -> Result<Imported, String>;

    /// Gibt es eine Besprechung aus dieser Quelldatei, die seit `since_ms` entstand?
    fn find_since(&self, source_path: &str, since_ms: i64) -> Option<Imported>;
}

/// Eine vorhandene YouTube-Besprechung.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MeetingRef {
    pub id: String,
    pub title: String,
    /// Anlegezeit (ms UTC).
    pub created_at_ms: i64,
}

/// Der vorhandene YouTube-Quellweg (A2).
pub trait YoutubeControl: Send + Sync {
    /// Eine vorhandene Besprechung mit dieser Video-Kennung.
    fn find_video(&self, video_id: &str) -> Option<MeetingRef>;
    /// Legt die Quelle an (blockierend: ein oEmbed-Abruf).
    fn add(&self, url: &str, project: Option<&str>) -> Result<MeetingRef, YoutubeError>;
}

// ---------------------------------------------------------------------------
// Hilfen
// ---------------------------------------------------------------------------

fn text<'a>(params: &'a Value, key: &str) -> Option<&'a str> {
    params
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
}

fn clean_title(raw: &str) -> String {
    let joined = raw
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    joined.chars().take(TITLE_MAX_CHARS).collect()
}

/// Ob die Besprechung ein fertiges Transkript hat: fuer Bausteine, die darauf warten
/// (`Defer`, solange `Pending`). Status der Besprechung -> Antwort.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MeetingReadiness {
    /// Fertig: das Transkript ist final.
    Ready,
    /// Wartet oder laeuft noch (`queued`, `processing`, `recording`).
    Pending,
    /// Endet nie von selbst (`failed`, `cancelled`, unbekannt): ein dauerhafter Fehler.
    Never,
}

/// Siehe [`MeetingReadiness`]; `status` ist `meetings.status`.
pub fn meeting_state(status: &str) -> MeetingReadiness {
    match status {
        "ready" => MeetingReadiness::Ready,
        "queued" | "processing" | "recording" => MeetingReadiness::Pending,
        _ => MeetingReadiness::Never,
    }
}

// ---------------------------------------------------------------------------
// Datei aufloesen (Sandbox)
// ---------------------------------------------------------------------------

/// Die zu importierende Datei, geprueft.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Source {
    /// Pfad fuer die Besprechung (kanonisch, ohne `\\?\`).
    pub path: PathBuf,
    pub name: String,
}

fn permanent(m: impl Into<String>) -> StepError {
    StepError::Permanent(m.into())
}

/// Prueft `raw_path` gegen die Ordner-Integration `via` und liefert die Datei. Alle Fehler sind
/// dauerhaft (`Permanent`), ausser ein nicht lesbares Register (`Transient`).
pub fn resolve_source(
    conn: &Connection,
    via: &str,
    raw_path: &str,
    probe: &dyn FileProbe,
) -> Result<Source, StepError> {
    let integration = register::get(conn, via)
        .map_err(|e| StepError::Transient(format!("Das Register ist nicht lesbar: {e}")))?
        .ok_or_else(|| permanent(format!("Die Integration „{via}“ gibt es nicht.")))?;
    if integration.kind != Kind::Folder {
        return Err(permanent(format!(
            "„{}“ ist keine Ordner-Integration; importiert wird nur aus einem Ordner.",
            integration.label
        )));
    }
    let config = FolderConfig::from_config_json(&integration.config_json)
        .map_err(|e| permanent(e.to_string()))?;
    let sandbox = Sandbox::open(&config.path).map_err(|e| permanent(e.to_string()))?;

    let given = Path::new(raw_path);
    let canonical = if given.is_absolute() {
        std::fs::canonicalize(given)
            .map_err(|_| permanent("Die Datei gibt es nicht (mehr) oder sie ist nicht lesbar."))?
    } else {
        let resolved = sandbox
            .resolve_file(raw_path, false)
            .map_err(|e| permanent(e.to_string()))?;
        std::fs::canonicalize(&resolved)
            .map_err(|_| permanent("Die Datei gibt es nicht (mehr) oder sie ist nicht lesbar."))?
    };
    if !sandbox.contains(&canonical) {
        return Err(permanent(format!(
            "Die Datei liegt nicht im Ordner der Integration „{}“; importiert wird nur von dort.",
            integration.label
        )));
    }
    let rel = sandbox.rel_of(&canonical);
    // Die Pruefung auf Verknuepfungen auf jeder Ebene (auch bei absolutem Pfad).
    sandbox
        .resolve_file(&rel, false)
        .map_err(|e| permanent(e.to_string()))?;
    let meta = std::fs::symlink_metadata(&canonical)
        .map_err(|_| permanent("Die Datei gibt es nicht (mehr) oder sie ist nicht lesbar."))?;
    if !meta.is_file() {
        return Err(permanent("Der Pfad ist keine Datei."));
    }
    let extension = canonical
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or_default()
        .to_lowercase();
    if !default_extensions().contains(&extension) {
        return Err(permanent(format!(
            "Dateien vom Typ „.{extension}“ lassen sich nicht importieren (Audio, Video, VTT, SRT)."
        )));
    }
    if probe.cloud_only(&canonical, &meta) {
        return Err(permanent(
            "Die Datei liegt nur in der Cloud (OneDrive) und wird nicht heruntergeladen: „Immer auf diesem Gerät behalten“ wählen und den Lauf wiederholen.",
        ));
    }
    if meta.len() == 0 {
        return Err(permanent("Die Datei ist leer."));
    }
    let name = canonical
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    Ok(Source {
        path: PathBuf::from(display_path(&canonical)),
        name,
    })
}

// ---------------------------------------------------------------------------
// meeting.import
// ---------------------------------------------------------------------------

pub struct MeetingImport {
    spec: &'static ActionSpec,
    control: Arc<dyn ImportControl>,
    probe: Arc<dyn FileProbe>,
}

impl MeetingImport {
    pub fn new(control: Arc<dyn ImportControl>) -> Self {
        Self::with_probe(control, Arc::new(SystemProbe))
    }

    pub fn with_probe(control: Arc<dyn ImportControl>, probe: Arc<dyn FileProbe>) -> Self {
        Self {
            spec: catalog::action_spec("meeting.import").expect("Katalogeintrag meeting.import"),
            control,
            probe,
        }
    }

    fn source(&self, ctx: &RunCtx<'_>, params: &Value) -> Result<Source, StepError> {
        let via = text(params, "via")
            .ok_or_else(|| permanent("Der Parameter „via“ (Ordner-Integration) fehlt."))?;
        let path = text(params, "path")
            .ok_or_else(|| permanent("Der Parameter „path“ (Datei) fehlt oder ist leer."))?;
        let conn = ctx
            .conn()
            .map_err(|e| StepError::Transient(format!("Datenbank nicht geöffnet: {e}")))?;
        resolve_source(&conn, via, path, &*self.probe)
    }

    fn output(&self, source: &Source, imported: &Imported, reused: bool) -> StepOutput {
        let mut out = StepOutput::with_data(json!({
            "meeting_id": imported.meeting_id,
            "meeting": {"id": imported.meeting_id, "title": imported.title},
            "file": source.name,
            "status": imported.state.as_str(),
            "reused": reused,
        }));
        out.summary = Some(match (imported.state, reused) {
            (ImportState::Ready, _) => format!("„{}“ importiert.", source.name),
            (ImportState::Queued, false) => format!(
                "„{}“ in die Import-Warteschlange gestellt; die Transkription läuft dort.",
                source.name
            ),
            (ImportState::Queued, true) => {
                format!("„{}“ war schon eingereiht; dieselbe Besprechung.", source.name)
            }
        });
        out.sources = vec![SourceRef::new("file", &source.name, Some(&source.name))];
        out
    }
}

/// Fehler der Warteschlange -> Fehlerklasse des Schritts.
fn map_import_error(code: &str) -> StepError {
    let head = code.split(':').next().unwrap_or(code).trim();
    match head {
        // Eine Transaktion: nichts ist entstanden, ein neuer Versuch ist sicher.
        "queue_enqueue_failed" | "meeting_create_failed" => {
            StepError::Transient(format!("Das Einreihen scheiterte ({code})."))
        }
        "meetings_unavailable" => StepError::Transient(
            "Die Besprechungen sind noch nicht verfügbar (Speicher nicht geöffnet).".to_string(),
        ),
        "folder_not_found" | "youtube_project_not_found" => {
            permanent("Das Projekt gibt es nicht (mehr); der Parameter „project“ nennt es.")
        }
        "subtitle_invalid" | "subtitle_unreadable" | "import_path_invalid" => {
            permanent(format!("Die Datei lässt sich nicht importieren ({head})."))
        }
        // Nach dem Anlegen der Besprechung gescheitert: unklar, ob etwas zurueckblieb.
        "segments_store_failed" | "status_ready_failed" => {
            StepError::Unknown(format!("Der Import blieb unvollständig ({code})."))
        }
        _ => StepError::Unknown(format!("Das Einreihen ist unklar ({code}).")),
    }
}

impl Action for MeetingImport {
    fn id(&self) -> &str {
        "meeting.import"
    }

    fn effect(&self) -> EffectKind {
        EffectKind::External
    }

    fn heavy(&self, _params: &Value) -> Option<HeavyNeed> {
        self.spec.heavy
    }

    fn needs(&self, params: &Value) -> Result<Option<Needs>, NeedsError> {
        catalog::needs_from_spec(self.spec, params)
    }

    fn describe(&self, params: &Value) -> String {
        catalog::describe_from_spec(self.spec, params)
    }

    fn run(&self, ctx: &RunCtx<'_>, params: &Value) -> Result<StepOutput, StepError> {
        let source = self.source(ctx, params)?;
        let source_text = source.path.to_string_lossy().into_owned();
        // Schon eingereiht (Wiederholung, Absturz vor dem Journal)? Dann dieselbe Besprechung.
        let since = ctx.step_started_at.saturating_sub(SINCE_SLACK_MS);
        if let Some(found) = self.control.find_since(&source_text, since) {
            return Ok(self.output(&source, &found, true));
        }
        let title = text(params, "title")
            .map(clean_title)
            .filter(|t| !t.is_empty())
            .unwrap_or_else(|| clean_title(&title_from_path(&source.path)));
        let project = text(params, "project").map(str::to_string);
        let imported = self
            .control
            .enqueue(&ImportRequest {
                path: source.path.clone(),
                title,
                project,
            })
            .map_err(|code| map_import_error(&code))?;
        Ok(self.output(&source, &imported, false))
    }

    fn confirm(&self, ctx: &RunCtx<'_>, params: &Value) -> Option<StepOutput> {
        let source = self.source(ctx, params).ok()?;
        let source_text = source.path.to_string_lossy().into_owned();
        let since = ctx.step_started_at.saturating_sub(SINCE_SLACK_MS);
        let found = self.control.find_since(&source_text, since)?;
        Some(self.output(&source, &found, true))
    }
}

// ---------------------------------------------------------------------------
// youtube.add_source
// ---------------------------------------------------------------------------

pub struct YoutubeAddSource {
    spec: &'static ActionSpec,
    control: Arc<dyn YoutubeControl>,
}

impl YoutubeAddSource {
    pub fn new(control: Arc<dyn YoutubeControl>) -> Self {
        Self {
            spec: catalog::action_spec("youtube.add_source")
                .expect("Katalogeintrag youtube.add_source"),
            control,
        }
    }

    fn output(video_id: &str, meeting: &MeetingRef, reused: bool) -> StepOutput {
        let url = format!("https://www.youtube.com/watch?v={video_id}");
        let mut out = StepOutput::with_data(json!({
            "meeting_id": meeting.id,
            "meeting": {"id": meeting.id, "title": meeting.title},
            "video_id": video_id,
            "url": url,
            "reused": reused,
        }));
        out.summary = Some(if reused {
            format!("Das Video „{}“ ist schon als Quelle angelegt.", meeting.title)
        } else {
            format!("Quelle „{}“ angelegt.", meeting.title)
        });
        let mut source = SourceRef::new("youtube", video_id, Some(&meeting.title));
        source.url = Some(url);
        out.sources = vec![source];
        out
    }
}

/// Fehler des Quellwegs -> Fehlerklasse. Vor dem Anlegen gescheitert heisst: nichts entstanden
/// (oEmbed vor der Transaktion, Besprechung in einer Transaktion), also ist ein neuer Versuch sicher.
fn map_youtube_error(e: &YoutubeError) -> StepError {
    let message = e.to_string();
    match e {
        YoutubeError::Busy
        | YoutubeError::RateLimited
        | YoutubeError::Timeout
        | YoutubeError::Network(_)
        | YoutubeError::Http(_)
        | YoutubeError::BadResponse(_)
        | YoutubeError::Store(_) => StepError::Transient(message),
        YoutubeError::Disabled(_) => StepError::Denied(message),
        _ => StepError::Permanent(message),
    }
}

impl Action for YoutubeAddSource {
    fn id(&self) -> &str {
        "youtube.add_source"
    }

    fn effect(&self) -> EffectKind {
        EffectKind::External
    }

    fn needs(&self, params: &Value) -> Result<Option<Needs>, NeedsError> {
        catalog::needs_from_spec(self.spec, params)
    }

    fn describe(&self, params: &Value) -> String {
        catalog::describe_from_spec(self.spec, params)
    }

    fn run(&self, _ctx: &RunCtx<'_>, params: &Value) -> Result<StepOutput, StepError> {
        let url = text(params, "url")
            .ok_or_else(|| permanent("Der Parameter „url“ (Video-Link) fehlt oder ist leer."))?;
        let video = normalize_link(url).map_err(|e| permanent(YoutubeError::Link(e).to_string()))?;
        // Dasselbe Video nie zweimal als Quelle.
        if let Some(found) = self.control.find_video(&video.video_id) {
            return Ok(Self::output(&video.video_id, &found, true));
        }
        let project = text(params, "project");
        let added = self
            .control
            .add(url, project)
            .map_err(|e| map_youtube_error(&e))?;
        Ok(Self::output(&video.video_id, &added, false))
    }

    fn confirm(&self, ctx: &RunCtx<'_>, params: &Value) -> Option<StepOutput> {
        let video = normalize_link(text(params, "url")?).ok()?;
        let found = self.control.find_video(&video.video_id)?;
        if found.created_at_ms < ctx.step_started_at.saturating_sub(SINCE_SLACK_MS) {
            return None; // eine aeltere Besprechung: dieser Schritt hat nichts angelegt
        }
        Some(Self::output(&video.video_id, &found, true))
    }
}

// ---------------------------------------------------------------------------
// Einhaengen
// ---------------------------------------------------------------------------

/// Haengt beide Bausteine in die Engine (ersetzt die Katalogbausteine).
pub fn install(
    engine: &Engine,
    import: Arc<dyn ImportControl>,
    youtube: Arc<dyn YoutubeControl>,
) {
    engine.register_action(Arc::new(MeetingImport::new(import)));
    engine.register_action(Arc::new(YoutubeAddSource::new(youtube)));
}

#[cfg(test)]
mod tests;
