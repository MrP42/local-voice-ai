//! App-Bausteine der Engine (B4): KI-Notizen, Protokoll, Zusammenfassung, Ablegen in
//! einen Ordner, Vorlesen (Audiodatei) und lokale Mitteilung. `wait` steht in `builtin`.
//!
//! Die Bausteine kennen die App nicht selbst. Alles, was einen `AppHandle` braucht
//! (Modellaufrufe mit Fortschrittsmeldung, Sprachausgabe, Mitteilung), steht hinter dem
//! Trait [`AppServices`]; die App setzt ihre Umsetzung ein (`hub::AppServicesImpl`), Tests
//! eine mit Mock-Modell. So laeuft in Tests derselbe Baustein-Code wie in der App, nur der
//! unterste Aufruf (Fenster, Sprachserver, Windows) ist ersetzt.
//!
//! # Vertrag je Baustein
//!
//! | Baustein           | Wirkung      | Schwer  | Recht (Tor)                       |
//! |--------------------|--------------|---------|-----------------------------------|
//! | `meeting.notes`    | idempotent   | Modell  | keines (App-eigene Funktion)      |
//! | `meeting.minutes`  | idempotent   | Modell  | keines                            |
//! | `text.summarize`   | idempotent   | Modell  | keines                            |
//! | `export.document`  | idempotent   | nein    | `files.write` auf das Ziel        |
//! | `tts.render`       | idempotent   | Sprache | keines                            |
//! | `notify.local`     | idempotent   | nein    | keines                            |
//!
//! - **Besprechung noch nicht fertig**: `meeting.*`, `text.summarize` (mit Quelle aus der
//!   Besprechung) und `export.document` melden `Defer`, solange die Besprechung aufnimmt oder
//!   verarbeitet (alle 15 s), und scheitern erst nach sechs Stunden (`Permanent`). Das Tor fuer
//!   schwere Schritte wird dabei nicht dauerhaft belegt: jeder Versuch gibt es wieder frei.
//! - **Idempotenz**: `<lauf>:<schritt>` (`RunCtx::idempotency_key`) ist die Kennung. Wer Inhalt
//!   erzeugt, schreibt einen Provenienz-Eintrag mit Akteur `workflow/<lauf>/<schritt>`; vor der
//!   Erzeugung schaut der Baustein nach, ob es fuer genau diesen Schritt schon einen Eintrag
//!   gibt (und sein Ergebnis noch existiert). Nach einem Absturz zwischen Erzeugung und
//!   Journal entsteht so kein zweites Protokoll und keine zweite Datei. Offen bleibt das
//!   Fenster zwischen dem Schliessen der Datei und dem Provenienz-Eintrag (zwei Anweisungen
//!   hintereinander); dort koennte eine Kopie `Name (2).docx` entstehen.
//! - **Rechte**: `export.document` ruft `integrations::gate` NICHT selbst, die Engine fragt das
//!   Tor vor `run` (Recht, Freigabe, Audit). Geschrieben wird deshalb mit der Pfad-Sandbox
//!   (`integrations::folder::Sandbox`), der untersten Ebene von `targets::place_export`: ein
//!   zweiter Torgang haette bei „fragen“ eine zweite Freigabe verlangt und im Audit doppelt
//!   gebucht.
//! - **Fehlerklassen**: Modell nicht eingerichtet / Vorlage unbekannt / Transkript leer ->
//!   `Permanent`; Arbeitsspeicher knapp, Besprechung belegt, Aufnahme laeuft -> `Defer`
//!   (Rueckstau statt Abbruch); Modellfehler, gesperrter Ordner -> `Transient` (es wurde
//!   nichts abgelegt); Ordner-Wurzel fehlt oder verlaesst die Sandbox -> `Permanent`.
//!
//! # Fehlerfaelle (B4) und ihre Absicherung
//!
//! - **Nebenlaeufigkeit**: zwei Laeufe fuer dieselbe Besprechung (`minutes_busy`,
//!   `enhance_busy`) warten (`Defer`) statt zu scheitern; zwei Laeufe, die in denselben Ordner
//!   exportieren, bekommen `Name.docx` und `Name (2).docx`, nie ein Ueberschreiben
//!   (`Sandbox::write_new`). Schwere Schritte laufen seriell (`HeavyGate`).
//! - **Abbruch mitten im Vorgang**: siehe Idempotenz oben; ein Abbruch durch den Nutzer
//!   (`RunCtx::cancelled`) beendet den Modellaufruf (das Future faellt, nichts wird
//!   gespeichert) und meldet `Transient` (nichts geschrieben).
//! - **Voller Datentraeger / gesperrte Datei**: `write_new` entfernt eine halb geschriebene
//!   Datei; der Fehler ist `Transient` (neuer Versuch sicher). Das Audio der Sprachausgabe
//!   schreibt der Sprachserver in eine feste Datei (`vorlesen-<lauf>-<schritt>.wav`).
//! - **Fehlendes Geraet / Audio-Echtzeitpfad**: nicht beteiligt; die Bausteine fassen weder
//!   Mikrofon noch Wiedergabe an (Vorlesen schreibt eine Datei, es spielt nichts ab).
//! - **Absturz eines Kindprozesses**: Modell und Sprachserver starten ueber ihre Verwalter
//!   (`process_guard`: Job-Objekt, RAM-/CPU-Deckel, Start-Tor); stirbt der Server, ist der
//!   Fehler `Transient` und der Schritt laeuft erneut.
//! - **Voller Arbeitsspeicher**: das `HeavyGate` haelt den Schritt zurueck (30 s, kein Versuch
//!   verbraucht); meldet der Modellverwalter selbst Speichermangel (`memory_low`, „Zu wenig
//!   freier Arbeitsspeicher“), wird daraus `Defer`. PDF braucht ein verstecktes WebView2-
//!   Fenster; dessen RAM-Tor (`pdf::ram_gate`) meldet den Mangel als Fehler `Transient`.
//! - **Mitteilung**: ohne Windows-Mitteilungsdienst oder bei unbekannter App-Kennung kann der
//!   Aufruf scheitern; das ist `Transient`, der Ablauf kann den Schritt mit `on_error: continue`
//!   ueberspringen.

use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use rusqlite::OptionalExtension;
use serde_json::{json, Map, Value};

use crate::managers::integrations::folder::{FolderConfig, FolderError, Sandbox};
use crate::managers::integrations::model::Kind;
use crate::managers::integrations::store as integrations_store;
use crate::managers::meetings::basis::DocBasis;
use crate::managers::meetings::export::{self, ExportParts};
use crate::managers::meetings::language;
use crate::managers::meetings::store::{Meeting, MeetingDocument, MeetingStore};
use crate::managers::provenance::{NewProvenance, SourceRef, SubjectKind};
use crate::summarizer::SummaryOptions;

use super::action::{
    actor_ref, Action, EffectKind, HeavyNeed, Needs, NeedsError, RunCtx, StepError, StepOutput,
};
use super::catalog::{self, ActionSpec};
use super::engine::Engine;
use super::heavy::MEMORY_RETRY_MS;

/// So lange wartet ein Schritt auf eine Besprechung, die aufnimmt oder verarbeitet wird.
pub const MAX_WAIT_FOR_MEETING_MS: i64 = 6 * 3_600_000;
/// Abstand der Nachfragen, ob die Besprechung fertig ist (oder ob sie nicht mehr belegt ist).
pub const READY_POLL_MS: u64 = 15_000;
/// Laengster Text fuer die Sprachausgabe (Zeichen).
pub const MAX_TTS_CHARS: usize = 20_000;
/// Laengste Zusammenfassung im Ergebnis des Schritts (Zeichen); mehr wird gekuerzt (`truncated`).
const MAX_SUMMARY_CHARS: usize = 16_000;
const MAX_NOTIFY_TITLE_CHARS: usize = 80;
const MAX_NOTIFY_BODY_CHARS: usize = 240;

// ---------------------------------------------------------------------------
// Die Schnittstelle zur App
// ---------------------------------------------------------------------------

/// Warum ein Dienst der App nicht lieferte, schon in der Sprache der Engine.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ServiceError {
    /// Nicht jetzt (Arbeitsspeicher, Besprechung belegt, Aufnahme laeuft): spaeter erneut.
    Busy { retry_after_ms: u64, reason: String },
    /// Wird nie gelingen (Modell nicht eingerichtet, Vorlage unbekannt, Transkript leer).
    Permanent(String),
    /// Nichts ist passiert, ein neuer Versuch ist sicher.
    Transient(String),
    /// Der Nutzer hat den Lauf abgebrochen.
    Cancelled,
    /// Den Dienst gibt es in dieser Umgebung nicht (Trockenlauf-Kommandozeile, Tests).
    NotAvailable(String),
}

impl ServiceError {
    pub fn into_step_error(self) -> StepError {
        match self {
            ServiceError::Busy {
                retry_after_ms,
                reason,
            } => StepError::Defer {
                retry_after_ms,
                reason,
            },
            ServiceError::Permanent(m) => StepError::Permanent(m),
            ServiceError::Transient(m) => StepError::Transient(m),
            ServiceError::Cancelled => {
                StepError::Transient("Der Lauf wurde abgebrochen.".to_string())
            }
            ServiceError::NotAvailable(m) => StepError::NotAvailable(m),
        }
    }
}

/// Was ein Erzeugungsaufruf braucht.
#[derive(Clone, Debug, Default)]
pub struct GenRequest {
    pub meeting_id: String,
    /// Kennung der Vorlage, `auto` oder `None` (die der Besprechung bzw. die Standardvorlage).
    pub template_id: Option<String>,
    pub basis: DocBasis,
}

/// Die Dienste der App, die die Bausteine brauchen. Jeder Aufruf blockiert und darf lange
/// dauern; `cancel` fragt die Engine, ob der Lauf abgebrochen wurde.
pub trait AppServices: Send + Sync {
    fn store(&self) -> Result<Arc<MeetingStore>, ServiceError>;

    /// Laeuft das gewaehlte Sprachmodell auf diesem Rechner? Bestimmt den RAM-Bedarf des
    /// Schritts (entfernte Anbieter brauchen keinen).
    fn llm_is_local(&self) -> bool {
        true
    }

    fn generate_notes(
        &self,
        req: &GenRequest,
        cancel: &dyn Fn() -> bool,
    ) -> Result<MeetingDocument, ServiceError>;

    fn generate_minutes(
        &self,
        req: &GenRequest,
        cancel: &dyn Fn() -> bool,
    ) -> Result<MeetingDocument, ServiceError>;

    fn summarize(
        &self,
        text: &str,
        opts: &SummaryOptions,
        cancel: &dyn Fn() -> bool,
    ) -> Result<String, ServiceError>;

    /// Ordner fuer erzeugtes Audio (bei einer Besprechung deren Ordner).
    fn audio_dir(&self, meeting_id: Option<&str>) -> Result<PathBuf, ServiceError>;

    /// Spricht `text` in die Datei `out` (mit der in den Einstellungen gewaehlten Stimme) und
    /// gibt den endgueltigen Pfad zurueck (das Format bestimmt die Einstellung).
    fn render_speech(
        &self,
        text: &str,
        out: &Path,
        cancel: &dyn Fn() -> bool,
    ) -> Result<PathBuf, ServiceError>;

    /// Zeigt eine Mitteilung des Betriebssystems.
    fn notify(&self, title: &str, body: &str) -> Result<(), ServiceError>;
}

/// Die Dienste, wo es keine App gibt: jeder Aufruf meldet „nicht eingebaut“.
pub struct UnavailableServices;

fn unavailable<T>() -> Result<T, ServiceError> {
    Err(ServiceError::NotAvailable(
        "Dieser Baustein läuft nur in der App, nicht in dieser Umgebung.".to_string(),
    ))
}

impl AppServices for UnavailableServices {
    fn store(&self) -> Result<Arc<MeetingStore>, ServiceError> {
        unavailable()
    }
    fn generate_notes(
        &self,
        _: &GenRequest,
        _: &dyn Fn() -> bool,
    ) -> Result<MeetingDocument, ServiceError> {
        unavailable()
    }
    fn generate_minutes(
        &self,
        _: &GenRequest,
        _: &dyn Fn() -> bool,
    ) -> Result<MeetingDocument, ServiceError> {
        unavailable()
    }
    fn summarize(
        &self,
        _: &str,
        _: &SummaryOptions,
        _: &dyn Fn() -> bool,
    ) -> Result<String, ServiceError> {
        unavailable()
    }
    fn audio_dir(&self, _: Option<&str>) -> Result<PathBuf, ServiceError> {
        unavailable()
    }
    fn render_speech(
        &self,
        _: &str,
        _: &Path,
        _: &dyn Fn() -> bool,
    ) -> Result<PathBuf, ServiceError> {
        unavailable()
    }
    fn notify(&self, _: &str, _: &str) -> Result<(), ServiceError> {
        unavailable()
    }
}

/// Fuehrt ein Future auf der Laufzeit der App zu Ende und bricht es ab, sobald `cancel`
/// wahr wird (das Future faellt, die Anfrage endet, nichts wird gespeichert). `None`: abgebrochen.
pub fn run_cancellable<F: Future>(cancel: &dyn Fn() -> bool, fut: F) -> Option<F::Output> {
    tauri::async_runtime::block_on(async {
        tokio::select! {
            out = fut => Some(out),
            _ = async {
                while !cancel() {
                    tokio::time::sleep(Duration::from_millis(250)).await;
                }
            } => None,
        }
    })
}

/// Fehlertext der Erzeugung (`<code>` oder `<code>: <grund>`, ein Satz des Modellverwalters)
/// in die Sprache der Engine.
pub fn classify_generation_error(err: &str) -> ServiceError {
    let code = err.split(':').next().unwrap_or(err).trim();
    match code {
        "meeting_not_finished" | "minutes_busy" | "enhance_busy" => ServiceError::Busy {
            retry_after_ms: READY_POLL_MS,
            reason: "Wartet: die Besprechung wird noch bearbeitet.".to_string(),
        },
        "memory_low" => ServiceError::Busy {
            retry_after_ms: MEMORY_RETRY_MS,
            reason: "Wartet auf Arbeitsspeicher für das Sprachmodell.".to_string(),
        },
        "recording_active" => ServiceError::Busy {
            retry_after_ms: MEMORY_RETRY_MS,
            reason: "Wartet: eine Aufnahme läuft, das lokale Sprachmodell hat so lange Pause."
                .to_string(),
        },
        "no_provider" | "no_model" => ServiceError::Permanent(
            "Es ist kein Sprachmodell eingerichtet (Einstellungen → Nachbearbeitung).".to_string(),
        ),
        "no_transcript" => ServiceError::Permanent(
            "Die Besprechung hat kein Transkript, aus dem sich etwas erzeugen ließe.".to_string(),
        ),
        "template_not_found" => {
            ServiceError::Permanent("Die gewählte Vorlage gibt es nicht.".to_string())
        }
        "meeting_not_found" => {
            ServiceError::Permanent("Die Besprechung gibt es nicht mehr.".to_string())
        }
        "minutes_cancelled" => ServiceError::Cancelled,
        // Der Nutzer hat den Auftrag im Statusbereich gestoppt: kein neuer Versuch.
        "stopped" => ServiceError::Permanent(
            "Die Erzeugung wurde vom Nutzer gestoppt.".to_string(),
        ),
        _ if crate::managers::meetings::llm_call::is_memory_error(err) => ServiceError::Busy {
            retry_after_ms: MEMORY_RETRY_MS,
            reason: "Wartet auf Arbeitsspeicher für das Sprachmodell.".to_string(),
        },
        _ if err.contains("Kein LLM-Provider") || err.contains("kein Modell eingetragen") => {
            ServiceError::Permanent(
                "Es ist kein Sprachmodell eingerichtet (Einstellungen → Nachbearbeitung)."
                    .to_string(),
            )
        }
        _ if err.starts_with("Kein Text zum Zusammenfassen") => {
            ServiceError::Permanent("Es gibt keinen Text zum Zusammenfassen.".to_string())
        }
        // Modellfehler, Speicherfehler, Unbekanntes: es wurde nichts gespeichert.
        _ => ServiceError::Transient(format!("Die Erzeugung ist gescheitert ({code}).")),
    }
}

/// Ergebnis von [`run_cancellable`] mit einem Fehlertext der Erzeugung in die Sprache der Engine.
pub fn finish_generation<T>(result: Option<Result<T, String>>) -> Result<T, ServiceError> {
    match result {
        None => Err(ServiceError::Cancelled),
        Some(Err(e)) => Err(classify_generation_error(&e)),
        Some(Ok(v)) => Ok(v),
    }
}

// ---------------------------------------------------------------------------
// Gemeinsames
// ---------------------------------------------------------------------------

/// Ein nicht leerer, getrimmter Textparameter.
fn text_param<'a>(params: &'a Value, key: &str) -> Option<&'a str> {
    params
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
}

fn has_template(s: &str) -> bool {
    s.contains("{{")
}

/// Die Besprechung dieses Laufs: `meeting.id` im Laufkontext (setzt jeder Baustein, der eine
/// Besprechung anlegt oder liefert), sonst `trigger.meeting_id`.
fn meeting_id_of(ctx: &RunCtx<'_>) -> Option<String> {
    let c = ctx.context;
    c.pointer("/meeting/id")
        .and_then(Value::as_str)
        .or_else(|| c.pointer("/trigger/meeting_id").and_then(Value::as_str))
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

fn no_meeting() -> StepError {
    StepError::Permanent(
        "Dieser Schritt braucht eine Besprechung: davor muss ein Schritt stehen, der eine anlegt \
         (Import, Aufnahme), oder der Auslöser nennt eine."
            .to_string(),
    )
}

fn svc(e: ServiceError) -> StepError {
    e.into_step_error()
}

/// Die Besprechung, sobald sie fertig ist; vorher `Defer`.
fn ready_meeting(
    ctx: &RunCtx<'_>,
    services: &dyn AppServices,
    meeting_id: &str,
) -> Result<(Arc<MeetingStore>, Meeting), StepError> {
    let store = services.store().map_err(svc)?;
    let meeting = store
        .get_meeting(meeting_id)
        .map_err(|e| {
            StepError::Transient(format!("Die Besprechung ließ sich nicht lesen ({e})."))
        })?
        .filter(|m| m.deleted_at.is_none())
        .ok_or_else(|| StepError::Permanent("Die Besprechung gibt es nicht mehr.".to_string()))?;
    match meeting.status.as_str() {
        "ready" => Ok((store, meeting)),
        "recording" | "processing" => {
            let waited = ctx.now_ms().saturating_sub(ctx.step_started_at);
            if waited >= MAX_WAIT_FOR_MEETING_MS {
                return Err(StepError::Permanent(
                    "Die Besprechung wurde in sechs Stunden nicht fertig.".to_string(),
                ));
            }
            Err(StepError::Defer {
                retry_after_ms: READY_POLL_MS,
                reason: if meeting.status == "recording" {
                    "Wartet: die Besprechung wird noch aufgenommen.".to_string()
                } else {
                    "Wartet, bis die Besprechung fertig verarbeitet ist.".to_string()
                },
            })
        }
        "failed" => Err(StepError::Permanent(
            "Die Verarbeitung der Besprechung ist fehlgeschlagen.".to_string(),
        )),
        "cancelled" => Err(StepError::Permanent(
            "Die Verarbeitung der Besprechung wurde gestoppt.".to_string(),
        )),
        other => Err(StepError::Permanent(format!(
            "Die Besprechung hat einen unbekannten Zustand ({other})."
        ))),
    }
}

fn meeting_value(m: &Meeting) -> Value {
    json!({"id": m.id, "title": m.title})
}

/// Ergebnis, das ein frueherer Versuch DIESES Schritts hinterlassen hat (Provenienz mit dem
/// Akteur `workflow/<lauf>/<schritt>`): Inhalts-ID und Parameter.
fn previous_result(
    ctx: &RunCtx<'_>,
    kind: SubjectKind,
    operation: &str,
) -> Option<(String, Option<Value>)> {
    let conn = ctx.conn().ok()?;
    let actor = actor_ref(ctx.workflow_id, ctx.run_id, ctx.step_id);
    let row: Option<(String, Option<String>)> = conn
        .query_row(
            "SELECT subject_id, params_json FROM provenance
             WHERE actor_ref = ?1 AND subject_kind = ?2 AND operation = ?3
             ORDER BY created_at DESC, id DESC LIMIT 1",
            rusqlite::params![actor, kind.as_str(), operation],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .ok()
        .flatten();
    row.map(|(id, params)| {
        (
            id,
            params.and_then(|p| serde_json::from_str::<Value>(&p).ok()),
        )
    })
}

/// Schreibt den Provenienz-Eintrag eines erzeugten Inhalts; ein Fehler wird nur geloggt.
fn record(
    ctx: &RunCtx<'_>,
    kind: SubjectKind,
    subject_id: &str,
    operation: &str,
    sources: Vec<SourceRef>,
    params: Value,
) {
    let mut entry = NewProvenance::new(
        kind,
        subject_id,
        operation,
        crate::managers::provenance::ActorKind::Workflow,
    );
    entry.sources = sources;
    entry.params = Some(params);
    if let Err(e) = ctx.record_provenance(entry) {
        log::warn!(
            "workflows: Provenienz fuer {}/{} nicht geschrieben: {e}",
            ctx.run_id,
            ctx.step_id
        );
    }
}

fn meeting_source(m: &Meeting) -> SourceRef {
    SourceRef::new("meeting", &m.id, Some(&m.title))
}

/// `\\?\C:\x` -> `C:\x` (die Sandbox kanonisiert; so steht der Pfad im Ergebnis und in Mails).
fn display_path(p: &Path) -> String {
    let s = p.to_string_lossy().to_string();
    if let Some(rest) = s.strip_prefix(r"\\?\UNC\") {
        return format!(r"\\{rest}");
    }
    match s.strip_prefix(r"\\?\") {
        Some(rest) if rest.chars().nth(1) == Some(':') => rest.to_string(),
        _ => s,
    }
}

fn llm_need(services: &dyn AppServices) -> HeavyNeed {
    if services.llm_is_local() {
        HeavyNeed {
            ram_mb: 6_144,
            label: "Sprachmodell",
        }
    } else {
        HeavyNeed {
            ram_mb: 0,
            label: "Sprachmodell (Anbieter)",
        }
    }
}

fn spec_of(id: &str) -> &'static ActionSpec {
    catalog::action_spec(id).unwrap_or_else(|| panic!("Katalogeintrag {id} fehlt"))
}

// ---------------------------------------------------------------------------
// meeting.notes und meeting.minutes
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DocKind {
    Notes,
    Minutes,
}

impl DocKind {
    fn action_id(self) -> &'static str {
        match self {
            DocKind::Notes => "meeting.notes",
            DocKind::Minutes => "meeting.minutes",
        }
    }
    /// Operation im Provenienz-Eintrag (dieselbe wie bei der Erzeugung von Hand).
    fn operation(self) -> &'static str {
        match self {
            DocKind::Notes => "notes",
            DocKind::Minutes => "minutes",
        }
    }
    fn noun(self) -> &'static str {
        match self {
            DocKind::Notes => "KI-Notizen",
            DocKind::Minutes => "Protokoll",
        }
    }
}

/// Die Ausgabesprache aus dem Parameter: `None` (leer, `auto`) oder ein Sprachcode.
fn output_language(params: &Value) -> Result<Option<String>, String> {
    match text_param(params, "output_language") {
        None => Ok(None),
        Some(s) if s.eq_ignore_ascii_case("auto") => Ok(None),
        Some(s) => language::normalize_code(s).map(Some).ok_or_else(|| {
            format!("„{s}“ ist kein Sprachcode (zum Beispiel de, en, fr) und nicht „auto“.")
        }),
    }
}

/// Die Vorlage aus dem Parameter: Kennung (`builtin:kunde`), Titel („Kundengespräch“),
/// `auto` oder leer (Vorgabe der Besprechung).
fn resolve_template(store: &MeetingStore, wanted: Option<&str>) -> Result<Option<String>, String> {
    let Some(wanted) = wanted else {
        return Ok(None);
    };
    if crate::managers::meetings::notes::templates::is_auto_id(wanted) {
        return Ok(Some("auto".to_string()));
    }
    let infos = store
        .list_template_infos()
        .map_err(|e| format!("Die Vorlagen ließen sich nicht lesen ({e})."))?;
    let lower = wanted.to_lowercase();
    let found = infos.iter().find(|i| i.id == wanted).or_else(|| {
        infos.iter().find(|i| {
            i.title.to_lowercase() == lower
                || i.id.to_lowercase() == format!("builtin:{lower}")
        })
    });
    found
        .map(|i| Some(i.id.clone()))
        .ok_or_else(|| format!("Die Vorlage „{wanted}“ gibt es nicht."))
}

pub struct MeetingDocAction {
    kind: DocKind,
    spec: &'static ActionSpec,
    services: Arc<dyn AppServices>,
}

impl MeetingDocAction {
    pub fn new(kind: DocKind, services: Arc<dyn AppServices>) -> Self {
        Self {
            kind,
            spec: spec_of(kind.action_id()),
            services,
        }
    }
}

impl Action for MeetingDocAction {
    fn id(&self) -> &str {
        self.kind.action_id()
    }

    fn effect(&self) -> EffectKind {
        EffectKind::Idempotent
    }

    fn heavy(&self, _params: &Value) -> Option<HeavyNeed> {
        Some(llm_need(&*self.services))
    }

    fn needs(&self, _params: &Value) -> Result<Option<Needs>, NeedsError> {
        Ok(None)
    }

    fn validate(&self, params: &Map<String, Value>) -> Result<(), String> {
        if let Some(Value::String(s)) = params.get("output_language") {
            if !has_template(s) {
                output_language(&Value::Object(params.clone()))
                    .map_err(|m| format!("output_language: {m}"))?;
            }
        }
        Ok(())
    }

    fn describe(&self, params: &Value) -> String {
        let base = catalog::describe_from_spec(self.spec, params);
        let mut extra = Vec::new();
        if let Some(t) = text_param(params, "template") {
            extra.push(format!("Vorlage {t}"));
        }
        if let Some(l) = text_param(params, "output_language") {
            extra.push(format!("Sprache {l}"));
        }
        if extra.is_empty() {
            base
        } else {
            format!("{base} ({})", extra.join(", "))
        }
    }

    fn run(&self, ctx: &RunCtx<'_>, params: &Value) -> Result<StepOutput, StepError> {
        let meeting_id = meeting_id_of(ctx).ok_or_else(no_meeting)?;
        let (store, meeting) = ready_meeting(ctx, &*self.services, &meeting_id)?;

        // Ein frueherer Versuch dieses Schritts hat das Dokument schon erzeugt.
        if let Some((doc_id, _)) = previous_result(ctx, SubjectKind::Document, self.kind.operation())
        {
            if let Ok(Some(doc)) = store.get_document(&doc_id) {
                return Ok(doc_output(self.kind, &meeting, &doc, true));
            }
        }

        let basis = DocBasis {
            variant_id: text_param(params, "variant_id").map(str::to_string),
            output_language: output_language(params).map_err(StepError::Permanent)?,
        };
        let template_id =
            resolve_template(&store, text_param(params, "template")).map_err(StepError::Permanent)?;
        let req = GenRequest {
            meeting_id: meeting_id.clone(),
            template_id,
            basis: basis.clone(),
        };
        let cancel = || ctx.cancelled();
        let doc = match self.kind {
            DocKind::Notes => self.services.generate_notes(&req, &cancel),
            DocKind::Minutes => self.services.generate_minutes(&req, &cancel),
        }
        .map_err(svc)?;

        record(
            ctx,
            SubjectKind::Document,
            &doc.id,
            self.kind.operation(),
            vec![meeting_source(&meeting)],
            json!({
                "meeting": meeting.id,
                "template_id": doc.template_id,
                "version": doc.version,
                "basis_variant": basis.variant_id,
                "output_language": basis.output_language,
                "idempotency_key": ctx.idempotency_key,
            }),
        );
        Ok(doc_output(self.kind, &meeting, &doc, false))
    }
}

fn doc_output(kind: DocKind, meeting: &Meeting, doc: &MeetingDocument, reused: bool) -> StepOutput {
    let mut out = StepOutput::with_data(json!({
        "meeting": meeting_value(meeting),
        "meeting_id": meeting.id,
        "document_id": doc.id,
        "version": doc.version,
        "template_id": doc.template_id,
        "chars": doc.body.chars().count(),
    }));
    out.sources = vec![meeting_source(meeting)];
    out.summary = Some(if reused {
        format!("{} lag schon vor (Wiederaufnahme).", kind.noun())
    } else {
        format!("{} „{}“ erzeugt.", kind.noun(), meeting.title)
    });
    out
}

// ---------------------------------------------------------------------------
// text.summarize
// ---------------------------------------------------------------------------

const STYLES: [&str; 4] = ["kurz", "mittel", "lang", "management"];

fn summary_options(style: Option<&str>) -> Result<SummaryOptions, String> {
    let (length, detail, audience) = match style.map(str::to_lowercase).as_deref() {
        None | Some("mittel") => ("mittel", "ausgewogen", "allgemein"),
        Some("kurz") => ("kurz", "ueberblick", "allgemein"),
        Some("lang") => ("lang", "detailliert", "allgemein"),
        Some("management") => ("kurz", "ueberblick", "management"),
        Some(other) => {
            return Err(format!(
                "Die Art „{other}“ gibt es nicht; erlaubt sind {}.",
                STYLES.join(", ")
            ))
        }
    };
    Ok(SummaryOptions {
        length: length.to_string(),
        detail: detail.to_string(),
        audience: audience.to_string(),
    })
}

pub struct Summarize {
    spec: &'static ActionSpec,
    services: Arc<dyn AppServices>,
}

impl Summarize {
    pub fn new(services: Arc<dyn AppServices>) -> Self {
        Self {
            spec: spec_of("text.summarize"),
            services,
        }
    }
}

impl Action for Summarize {
    fn id(&self) -> &str {
        "text.summarize"
    }

    fn effect(&self) -> EffectKind {
        EffectKind::Idempotent
    }

    fn heavy(&self, _params: &Value) -> Option<HeavyNeed> {
        Some(llm_need(&*self.services))
    }

    fn needs(&self, _params: &Value) -> Result<Option<Needs>, NeedsError> {
        Ok(None)
    }

    fn validate(&self, params: &Map<String, Value>) -> Result<(), String> {
        if let Some(Value::String(s)) = params.get("style") {
            if !has_template(s) {
                summary_options(Some(s.trim())).map_err(|m| format!("style: {m}"))?;
            }
        }
        Ok(())
    }

    fn describe(&self, params: &Value) -> String {
        let source = match text_param(params, "source") {
            Some("minutes") => "dem Protokoll",
            Some("notes") => "den KI-Notizen",
            Some("transcript") => "dem Transkript",
            _ => "dem angegebenen Text",
        };
        let style = text_param(params, "style").unwrap_or("mittel");
        format!("Zusammenfassung aus {source} erzeugen (Art: {style})")
    }

    fn run(&self, ctx: &RunCtx<'_>, params: &Value) -> Result<StepOutput, StepError> {
        let source = text_param(params, "source").ok_or_else(|| {
            StepError::Permanent("Es ist keine Quelle angegeben (minutes, notes, transcript oder ein Text).".to_string())
        })?;
        let opts = summary_options(text_param(params, "style")).map_err(StepError::Permanent)?;

        let mut meeting_used: Option<Meeting> = None;
        let text = match source {
            "minutes" | "notes" | "transcript" => {
                let id = meeting_id_of(ctx).ok_or_else(no_meeting)?;
                let (store, meeting) = ready_meeting(ctx, &*self.services, &id)?;
                let text = match source {
                    "transcript" => {
                        let segments = store.get_segments(&id).map_err(|e| {
                            StepError::Transient(format!("Das Transkript ließ sich nicht lesen ({e})."))
                        })?;
                        crate::managers::meetings::minutes::render_transcript_for_prompt(&segments)
                    }
                    _ => {
                        let bundle = export::build_bundle(&store, &id).map_err(|e| {
                            StepError::Transient(format!("Die Besprechung ließ sich nicht lesen ({e})."))
                        })?;
                        let have = if source == "minutes" {
                            bundle.minutes_md.is_some()
                        } else {
                            bundle.enhanced.is_some()
                        };
                        if !have {
                            return Err(StepError::Permanent(format!(
                                "Zu dieser Besprechung gibt es noch kein{} {}; davor muss der Schritt stehen, der es erzeugt.",
                                if source == "minutes" { "" } else { "e" },
                                if source == "minutes" { "Protokoll" } else { "KI-Notizen" }
                            )));
                        }
                        let parts = ExportParts {
                            ai_notes: source == "notes",
                            notes: false,
                            minutes: source == "minutes",
                            transcript: false,
                            participants: false,
                        };
                        export::bundle_to_markdown(&bundle, &parts)
                    }
                };
                meeting_used = Some(meeting);
                text
            }
            literal => literal.to_string(),
        };
        if text.trim().is_empty() {
            return Err(StepError::Permanent(
                "Es gibt keinen Text zum Zusammenfassen.".to_string(),
            ));
        }

        let cancel = || ctx.cancelled();
        let summary = self.services.summarize(&text, &opts, &cancel).map_err(svc)?;

        let truncated = summary.chars().count() > MAX_SUMMARY_CHARS;
        let shown: String = summary.chars().take(MAX_SUMMARY_CHARS).collect();
        let mut sources = Vec::new();
        if let Some(m) = &meeting_used {
            sources.push(meeting_source(m));
        }
        record(
            ctx,
            SubjectKind::Summary,
            &ctx.idempotency_key,
            "summary",
            sources.clone(),
            json!({
                "source": if meeting_used.is_some() { source } else { "text" },
                "style": text_param(params, "style").unwrap_or("mittel"),
                "chars": summary.chars().count(),
            }),
        );
        let mut data = json!({
            "text": shown,
            "chars": summary.chars().count(),
            "truncated": truncated,
        });
        if let Some(m) = &meeting_used {
            data["meeting_id"] = json!(m.id);
        }
        let mut out = StepOutput::with_data(data).summary("Zusammenfassung erzeugt.");
        out.sources = sources;
        Ok(out)
    }
}

// ---------------------------------------------------------------------------
// export.document
// ---------------------------------------------------------------------------

fn folder_error(e: FolderError) -> StepError {
    match e {
        // Der Ordner kann zurueckkommen (Netzlaufwerk, OneDrive noch nicht gestartet).
        FolderError::RootNotFound | FolderError::Io(_) => StepError::Transient(e.to_string()),
        other => StepError::Permanent(other.to_string()),
    }
}

fn export_parts(content: &str) -> ExportParts {
    match content {
        "notes" => ExportParts {
            ai_notes: true,
            notes: false,
            minutes: false,
            transcript: false,
            participants: false,
        },
        "all" => ExportParts::all(),
        _ => ExportParts {
            ai_notes: false,
            notes: false,
            minutes: true,
            transcript: false,
            participants: false,
        },
    }
}

fn join_rel(parts: &[&str]) -> String {
    parts
        .iter()
        .map(|p| p.trim().trim_matches(['/', '\\']))
        .filter(|p| !p.is_empty())
        .collect::<Vec<_>>()
        .join("/")
}

pub struct ExportDocument {
    spec: &'static ActionSpec,
    services: Arc<dyn AppServices>,
}

impl ExportDocument {
    pub fn new(services: Arc<dyn AppServices>) -> Self {
        Self {
            spec: spec_of("export.document"),
            services,
        }
    }
}

impl Action for ExportDocument {
    fn id(&self) -> &str {
        "export.document"
    }

    fn effect(&self) -> EffectKind {
        EffectKind::Idempotent
    }

    fn needs(&self, params: &Value) -> Result<Option<Needs>, NeedsError> {
        catalog::needs_from_spec(self.spec, params)
    }

    fn describe(&self, params: &Value) -> String {
        let what = match text_param(params, "content") {
            Some("notes") => "KI-Notizen",
            Some("all") => "Besprechung (alles)",
            _ => "Protokoll",
        };
        let format = text_param(params, "format").unwrap_or("docx");
        let target = text_param(params, "target").unwrap_or("…");
        format!("{what} als {format} in {target} ablegen")
    }

    fn run(&self, ctx: &RunCtx<'_>, params: &Value) -> Result<StepOutput, StepError> {
        let target_id = text_param(params, "target")
            .ok_or_else(|| StepError::Permanent("Es ist kein Ziel angegeben.".to_string()))?;
        let format = text_param(params, "format").unwrap_or("docx");
        if !["docx", "pdf", "md"].contains(&format) {
            return Err(StepError::Permanent(format!(
                "Das Format „{format}“ gibt es nicht (docx, pdf, md)."
            )));
        }
        let content = text_param(params, "content").unwrap_or("minutes");
        if !["minutes", "notes", "all"].contains(&content) {
            return Err(StepError::Permanent(format!(
                "Der Inhalt „{content}“ gibt es nicht (minutes, notes, all)."
            )));
        }
        let meeting_id = meeting_id_of(ctx).ok_or_else(no_meeting)?;
        let (store, meeting) = ready_meeting(ctx, &*self.services, &meeting_id)?;

        // Ziel: eine Ordner-Integration. Das Recht hat die Engine vor `run` geprueft.
        let conn = ctx
            .conn()
            .map_err(|e| StepError::Transient(format!("Das Register ist nicht erreichbar ({e}).")))?;
        let integration = integrations_store::get(&conn, target_id)
            .map_err(|e| StepError::Transient(format!("Das Register ist nicht erreichbar ({e}).")))?
            .ok_or_else(|| StepError::Permanent(format!("Das Ziel „{target_id}“ gibt es nicht.")))?;
        if integration.kind != Kind::Folder {
            return Err(StepError::Permanent(format!(
                "Das Ziel „{target_id}“ ist kein Ordner."
            )));
        }
        let cfg = FolderConfig::from_config_json(&integration.config_json)
            .map_err(folder_error)?;
        let sandbox = Sandbox::open(&cfg.path).map_err(folder_error)?;
        let rel_dir = join_rel(&[&cfg.subfolder, text_param(params, "subfolder").unwrap_or("")]);

        // Ein frueherer Versuch dieses Schritts hat die Datei schon abgelegt.
        if let Some((_, Some(prev))) = previous_result(ctx, SubjectKind::Export, "export") {
            if let Some(rel) = prev.get("rel").and_then(Value::as_str) {
                if let Ok(path) = sandbox.resolve_file(rel, false) {
                    if std::fs::metadata(&path).is_ok_and(|m| m.is_file() && m.len() > 0) {
                        return Ok(export_output(
                            &sandbox,
                            &path,
                            format,
                            std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0),
                            true,
                        ));
                    }
                }
            }
        }

        let bundle = export::build_bundle(&store, &meeting_id).map_err(|e| {
            if e.starts_with("meeting_not_found") {
                StepError::Permanent("Die Besprechung gibt es nicht mehr.".to_string())
            } else {
                StepError::Transient(format!("Die Besprechung ließ sich nicht lesen ({e})."))
            }
        })?;
        let missing = match content {
            "minutes" => bundle.minutes_md.is_none().then_some("Protokoll"),
            "notes" => bundle.enhanced.is_none().then_some("KI-Notizen"),
            _ => None,
        };
        if let Some(what) = missing {
            return Err(StepError::Permanent(format!(
                "Zu dieser Besprechung gibt es noch keine {what}; davor muss der Schritt stehen, der sie erzeugt."
            )));
        }
        let markdown = export::bundle_to_markdown(&bundle, &export_parts(content));

        let base = text_param(params, "name").unwrap_or(meeting.title.as_str());
        let ext = format!(".{format}");
        let file_name = if base.to_lowercase().ends_with(&ext) {
            base.to_string()
        } else {
            format!("{base}{ext}")
        };
        let placed = sandbox
            .write_new(&rel_dir, &file_name, |path| {
                export::write_document(path, &markdown)
            })
            .map_err(folder_error)?;

        record(
            ctx,
            SubjectKind::Export,
            &ctx.idempotency_key,
            "export",
            vec![meeting_source(&meeting)],
            json!({
                "rel": placed.rel,
                "target": target_id,
                "format": format,
                "content": content,
                "meeting": meeting.id,
                "bytes": placed.bytes,
            }),
        );
        Ok(export_output(&sandbox, &placed.path, format, placed.bytes, false))
    }
}

fn export_output(
    sandbox: &Sandbox,
    path: &Path,
    format: &str,
    bytes: u64,
    reused: bool,
) -> StepOutput {
    let file_name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    let mut out = StepOutput::with_data(json!({
        "path": display_path(path),
        "rel": sandbox.rel_of(path),
        "file_name": file_name,
        "format": format,
        "bytes": bytes,
    }));
    out.summary = Some(if reused {
        format!("„{file_name}“ lag schon im Ziel (Wiederaufnahme).")
    } else {
        format!("„{file_name}“ im Ziel abgelegt.")
    });
    out
}

// ---------------------------------------------------------------------------
// tts.render
// ---------------------------------------------------------------------------

pub struct TtsRender {
    spec: &'static ActionSpec,
    services: Arc<dyn AppServices>,
}

impl TtsRender {
    pub fn new(services: Arc<dyn AppServices>) -> Self {
        Self {
            spec: spec_of("tts.render"),
            services,
        }
    }
}

/// Dateiname aus `<lauf>:<schritt>`.
fn file_stem_of(key: &str) -> String {
    key.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect()
}

const VOICE_NOT_SUPPORTED: &str = "Die Stimme richtet sich nach den Einstellungen (Vorlesen); \
     eine eigene Stimme je Schritt wird noch nicht unterstützt.";

impl Action for TtsRender {
    fn id(&self) -> &str {
        "tts.render"
    }

    fn effect(&self) -> EffectKind {
        EffectKind::Idempotent
    }

    fn heavy(&self, _params: &Value) -> Option<HeavyNeed> {
        self.spec.heavy
    }

    fn needs(&self, _params: &Value) -> Result<Option<Needs>, NeedsError> {
        Ok(None)
    }

    fn validate(&self, params: &Map<String, Value>) -> Result<(), String> {
        match params.get("voice") {
            Some(Value::String(s)) if s.trim().is_empty() => Ok(()),
            Some(Value::Null) | None => Ok(()),
            Some(_) => Err(format!("voice: {VOICE_NOT_SUPPORTED}")),
        }
    }

    fn describe(&self, params: &Value) -> String {
        match text_param(params, "text") {
            Some(t) => format!(
                "Text ({} Zeichen) mit der eingestellten Stimme als Audiodatei sprechen lassen",
                t.chars().count()
            ),
            None => catalog::describe_from_spec(self.spec, params),
        }
    }

    fn run(&self, ctx: &RunCtx<'_>, params: &Value) -> Result<StepOutput, StepError> {
        if text_param(params, "voice").is_some() {
            return Err(StepError::Permanent(VOICE_NOT_SUPPORTED.to_string()));
        }
        let text = text_param(params, "text")
            .ok_or_else(|| StepError::Permanent("Es ist kein Text zum Sprechen angegeben.".to_string()))?;
        if text.chars().count() > MAX_TTS_CHARS {
            return Err(StepError::Permanent(format!(
                "Der Text ist zu lang für die Sprachausgabe ({} Zeichen, höchstens {MAX_TTS_CHARS}).",
                text.chars().count()
            )));
        }
        // Ein frueherer Versuch dieses Schritts hat die Datei schon erzeugt.
        if let Some((_, Some(prev))) = previous_result(ctx, SubjectKind::TtsAudio, "tts") {
            if let Some(path) = prev.get("path").and_then(Value::as_str) {
                if std::fs::metadata(path).is_ok_and(|m| m.is_file() && m.len() > 0) {
                    return Ok(tts_output(Path::new(path), true));
                }
            }
        }
        let meeting_id = meeting_id_of(ctx);
        let dir = self.services.audio_dir(meeting_id.as_deref()).map_err(svc)?;
        std::fs::create_dir_all(&dir).map_err(|e| {
            StepError::Transient(format!("Der Ordner für die Audiodatei ließ sich nicht anlegen ({e})."))
        })?;
        let out = dir.join(format!("vorlesen-{}.wav", file_stem_of(&ctx.idempotency_key)));
        let cancel = || ctx.cancelled();
        let final_path = self
            .services
            .render_speech(text, &out, &cancel)
            .map_err(svc)?;

        let mut sources = Vec::new();
        if let Some(id) = &meeting_id {
            sources.push(SourceRef::new("meeting", id, None));
        }
        record(
            ctx,
            SubjectKind::TtsAudio,
            &ctx.idempotency_key,
            "tts",
            sources.clone(),
            json!({
                "path": display_path(&final_path),
                "chars": text.chars().count(),
            }),
        );
        let mut o = tts_output(&final_path, false);
        o.sources = sources;
        Ok(o)
    }
}

fn tts_output(path: &Path, reused: bool) -> StepOutput {
    let bytes = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    StepOutput::with_data(json!({
        "path": display_path(path),
        "file_name": name,
        "bytes": bytes,
        "format": path.extension().map(|e| e.to_string_lossy().to_string()),
    }))
    .summary(&if reused {
        format!("„{name}“ lag schon vor (Wiederaufnahme).")
    } else {
        format!("Audiodatei „{name}“ erzeugt.")
    })
}

// ---------------------------------------------------------------------------
// notify.local
// ---------------------------------------------------------------------------

pub struct NotifyLocal {
    spec: &'static ActionSpec,
    services: Arc<dyn AppServices>,
}

impl NotifyLocal {
    pub fn new(services: Arc<dyn AppServices>) -> Self {
        Self {
            spec: spec_of("notify.local"),
            services,
        }
    }
}

/// Eine Zeile Text: Steuerzeichen weg, gekuerzt.
fn clean_line(s: &str, max: usize) -> String {
    let flat: String = s
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    let flat = flat.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() > max {
        let mut cut: String = flat.chars().take(max.saturating_sub(1)).collect();
        cut.push('…');
        cut
    } else {
        flat
    }
}

impl Action for NotifyLocal {
    fn id(&self) -> &str {
        "notify.local"
    }

    fn effect(&self) -> EffectKind {
        EffectKind::Idempotent
    }

    fn needs(&self, _params: &Value) -> Result<Option<Needs>, NeedsError> {
        Ok(None)
    }

    fn describe(&self, params: &Value) -> String {
        catalog::describe_from_spec(self.spec, params)
    }

    fn run(&self, _ctx: &RunCtx<'_>, params: &Value) -> Result<StepOutput, StepError> {
        let title = clean_line(
            text_param(params, "title").unwrap_or_default(),
            MAX_NOTIFY_TITLE_CHARS,
        );
        if title.is_empty() {
            return Err(StepError::Permanent(
                "Die Mitteilung braucht einen Titel.".to_string(),
            ));
        }
        let body = clean_line(
            text_param(params, "body").unwrap_or_default(),
            MAX_NOTIFY_BODY_CHARS,
        );
        self.services.notify(&title, &body).map_err(svc)?;
        Ok(StepOutput::with_data(json!({"shown": true})).summary("Mitteilung angezeigt."))
    }
}

// ---------------------------------------------------------------------------
// Einhaengen
// ---------------------------------------------------------------------------

/// Alle App-Bausteine dieses Pakets.
pub fn actions(services: Arc<dyn AppServices>) -> Vec<Arc<dyn Action>> {
    vec![
        Arc::new(MeetingDocAction::new(DocKind::Notes, services.clone())),
        Arc::new(MeetingDocAction::new(DocKind::Minutes, services.clone())),
        Arc::new(Summarize::new(services.clone())),
        Arc::new(ExportDocument::new(services.clone())),
        Arc::new(TtsRender::new(services.clone())),
        Arc::new(NotifyLocal::new(services)),
    ]
}

/// Haengt die Bausteine in die Engine (ersetzt die Katalogbausteine).
pub fn install(engine: &Engine, services: Arc<dyn AppServices>) {
    for action in actions(services) {
        engine.register_action(action);
    }
}

#[cfg(test)]
mod tests;
