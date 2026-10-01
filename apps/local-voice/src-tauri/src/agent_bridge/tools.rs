//! Die Werkzeuge der Agentenbruecke (A8): was ein externer Agent ueber Pipe, MCP-Proxy oder
//! `ctl` in der App ausloesen kann. Jedes Werkzeug steht im Katalog (`catalog::CATALOG`) mit
//! seiner Faehigkeit; die Rechte, Freigaben und das Audit erledigt die Bruecke (A7) VOR dem
//! Aufruf, die Handler hier setzen nur um, was freigegeben wurde, und pruefen ihre Eingaben
//! selbst, denn Argumente eines Agenten sind unvertraut (Prompt-Injection ueber Besprechungs-
//! und Videoinhalte, Fehlbedienung).
//!
//! | Werkzeug | Wirkung | Hinweise |
//! |---|---|---|
//! | `transcribe_file` | reiht eine Audio-/Videodatei in die Import-Warteschlange ein, liefert `meeting_id` | Pfad nur lokal und absolut (`paths`), die Warteschlange hat das RAM-Tor |
//! | `create_session` / `create_meeting` | Ordner bzw. leere Besprechung | nur Titel/Name, Obergrenzen |
//! | `add_youtube_source` | Besprechung mit Quelle YouTube | ein oEmbed-Abruf, nie yt-dlp, nie ein Download |
//! | `tts_page_create` / `tts_render_audio` | Seite der Vorlesen-Bibliothek, Audiodatei darin | ein Lauf zugleich, Textgrenze, nur im Seitenordner |
//! | `start_recording` | Aufnahme | NUR nach der Zustimmung des Nutzers in der App (`ctx.approved`) |
//! | `stop_recording` | beendet die laufende Aufnahme | |
//!
//! # Fehlerfaelle und ihre Absicherung (Tests in `tools/tests.rs`, `paths/tests.rs`)
//!
//! | # | Fehlerfall | Verhalten | Absicherung |
//! |---|---|---|---|
//! | 1 | Zwei Verbindungen starten gleichzeitig dieselbe Aufnahme, denselben Render, dasselbe Video | genau EINE Aufnahme (`already_recording`), ein Render zugleich (`busy`), Sperre je Video (A2) | `two_recording_starts_start_one`, `a_second_render_is_refused_while_one_runs`, `a_video_is_added_once_at_a_time` |
//! | 2 | Abbruch mitten im Vorgang (Agent geht, App endet) | Einreihen ist eine Transaktion (Besprechung + Warteschlangenzeile), Seite = Ordner + atomare Dateien, Render schreibt ueber eine Temp-Datei und benennt um, das Audit bleibt `pending` (A1) | `a_failed_render_leaves_no_half_file`, `a_failed_enqueue_creates_no_meeting` |
//! | 3 | Platte oder Speicher voll | Audit nicht schreibbar -> Aktion unterbleibt (A1/A7); Schreibfehler beim Render -> Fehler, keine Datei; RAM: TTS-Engine- und Import-Tor der App; Antworten und Texte sind begrenzt | `a_failed_render_leaves_no_half_file`, Grenzen in den Argumenttests |
//! | 4 | Geraet fehlt (Mikrofon, Systemton), Engine nicht installiert | die Aufnahme startet nicht und meldet den Grund; kein halber Eintrag | `a_recorder_that_cannot_start_is_reported_and_nothing_runs` |
//! | 5 | Kindprozess (ffmpeg beim Import, Sprach-Engine) stuerzt ab | die Warteschlange markiert die Besprechung `failed`; der Render meldet den Fehler; die Bruecke laeuft weiter (A7: `catch_unwind`) | `a_failing_host_is_reported_as_a_tool_error` |
//! | 6 | Unvertraute Argumente: UNC-/Geraetepfad, `..`, Datenstrom, falscher Typ, unbekanntes Feld, riesiger Text, Steuerzeichen | strenge Pruefung, klare Meldung, nichts getan | `paths/tests.rs`, `arguments_are_checked_strictly`, `text_limits_hold` |
//! | 7 | Aufnahme ohne Einwilligung | der Handler startet nur mit `ctx.approved`; die Freigabe ist einmalig, an Zugang und Argumente gebunden und verlangt in der App das Haekchen „Alle Beteiligten haben zugestimmt“ | `start_recording_without_the_users_approval_never_starts`, `recording_through_the_bridge_needs_the_users_decision` |
//! | 8 | Aufnahme vergessen (Agent endet, stoppt nie) | die App merkt ein Ende vor (Vorgabe 8 Stunden, hoechstens 12) | `a_started_recording_gets_a_stop_time` |
//!
//! Audio-Echtzeitpfad: dieses Modul beruehrt ihn nicht. Der Start geht ueber den vorhandenen
//! Weg des Recorders (`start_into`, Arbeitsthread, nie im Audio-Callback).

pub mod app_host;
pub mod paths;
pub mod sandbox;

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use serde_json::{json, Value};

use super::catalog::{self, CallContext, ToolHandler, ToolSpec};
use crate::managers::meetings::store::MeetingStore;
use crate::managers::provenance::{self, ActorKind, NewProvenance, SubjectKind};
use crate::managers::workflows::recording::{self, RecordingControl, StartRequest};
use crate::managers::youtube::source::{self, AddOptions, Origin};

/// Laengster Titel/Name (Zeichen).
pub const MAX_TITLE_CHARS: usize = 200;
/// Laengster Link.
pub const MAX_URL_CHARS: usize = 2048;
/// Laengster Text einer Vorlesen-Seite (Zeichen).
pub const MAX_TTS_TEXT_CHARS: usize = 20_000;
/// Laengster Dateiname einer erzeugten Audiodatei (ohne Endung).
pub const MAX_FILE_NAME_CHARS: usize = 100;
/// Laengster Pfad-Text, der ueberhaupt geprueft wird.
const MAX_PATH_ARG_CHARS: usize = 4096;
/// Aufnahme: laengste vorgemerkte Dauer in Minuten (wie die Bausteine der Abläufe).
pub const MAX_RECORDING_MINUTES: i64 = 720;

/// Eine eingereihte Datei.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QueuedImport {
    pub meeting_id: String,
    pub title: String,
    pub status: String,
}

/// Eine Seite der Vorlesen-Bibliothek.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PageRef {
    pub id: String,
    pub title: String,
}

/// Eine erzeugte Audiodatei.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rendered {
    pub file_name: String,
    pub path: String,
    pub bytes: u64,
    /// `wav`, `mp3`, ... (so steht es in der Datei; die Einstellung der App bestimmt das Format).
    pub format: String,
}

/// Was die Handler von der App brauchen und nicht selbst tun: Warteschlange, Seiten, Sprach-
/// Engine, Recorder. Die App liefert den echten Zugriff (`app_host`), die headless Sandbox-
/// Instanz eine Attrappe (`sandbox`), die Tests eine mitschreibende.
pub trait Host: Send + Sync {
    /// Reiht die Datei ein (die Datei wird noch nicht gelesen).
    fn enqueue_import(&self, title: &str, source_path: &str) -> Result<QueuedImport, String>;
    fn create_page(&self, title: &str, text: &str) -> Result<PageRef, String>;
    /// Der Text der Seite (leer, wenn sie keinen hat). `Err`: die Seite gibt es nicht.
    fn page_text(&self, page_id: &str) -> Result<String, String>;
    /// Erzeugt die Audiodatei `<file_name>.<format>` im Ordner der Seite.
    fn render_audio(&self, page_id: &str, text: &str, file_name: &str) -> Result<Rendered, String>;
    /// Der Recorder; `None`, wenn es hier keinen gibt (headless).
    fn recording(&self) -> Option<Arc<dyn RecordingControl>>;
    /// Merkt das Ende einer gestarteten Aufnahme vor.
    fn schedule_stop(&self, _meeting_id: &str, _at_ms: i64) {}
    /// Nach dem Anlegen einer Besprechung (Such-Index anstossen o. ae.).
    fn meeting_created(&self, _meeting_id: &str) {}
    fn youtube_options(&self) -> AddOptions;
}

/// Die Handler. Haelt den Store (Besprechungen, Ordner, Provenienz) und den `Host`.
pub struct AppTools {
    store: Arc<MeetingStore>,
    host: Arc<dyn Host>,
    rendering: AtomicBool,
}

impl AppTools {
    pub fn new(store: Arc<MeetingStore>, host: Arc<dyn Host>) -> Self {
        Self {
            store,
            host,
            rendering: AtomicBool::new(false),
        }
    }
}

// ---------------------------------------------------------------------------
// Argumente
// ---------------------------------------------------------------------------

/// Entfernt Steuerzeichen und unsichtbare Umschaltzeichen, zieht Leerraum zusammen.
pub fn clean_line(raw: &str) -> String {
    let spaced: String = raw
        .chars()
        .map(|c| if c.is_whitespace() { ' ' } else { c })
        .filter(|c| {
            !c.is_control()
                && !matches!(c,
                    '\u{200B}'..='\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}' | '\u{FEFF}')
        })
        .collect();
    spaced.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Text mit Zeilenumbruechen: Steuerzeichen (ausser Umbruch und Tab) und Umschaltzeichen fallen weg.
fn clean_text(raw: &str) -> String {
    raw.replace("\r\n", "\n")
        .chars()
        .filter(|c| {
            (*c == '\n' || *c == '\t' || !c.is_control())
                && !matches!(c,
                    '\u{200B}'..='\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}' | '\u{FEFF}')
        })
        .collect()
}

/// Die Argumente eines Aufrufs: jedes Feld wird einmal entnommen, am Ende darf nichts uebrig sein.
struct Args {
    map: BTreeMap<String, Value>,
}

impl Args {
    fn new(args: &Value) -> Result<Self, String> {
        match args {
            Value::Object(m) => Ok(Self {
                map: m.iter().map(|(k, v)| (k.clone(), v.clone())).collect(),
            }),
            Value::Null => Ok(Self {
                map: BTreeMap::new(),
            }),
            _ => Err("Die Argumente müssen ein Objekt sein.".to_string()),
        }
    }

    fn take(&mut self, key: &str) -> Option<Value> {
        match self.map.remove(key) {
            None | Some(Value::Null) => None,
            Some(v) => Some(v),
        }
    }

    /// Ein Text; `max` Zeichen vor dem Bereinigen. Leer zaehlt als fehlend.
    fn text(&mut self, key: &str, max: usize, multiline: bool) -> Result<Option<String>, String> {
        match self.take(key) {
            None => Ok(None),
            Some(Value::String(s)) => {
                if s.chars().count() > max {
                    return Err(format!("„{key}“ ist zu lang (höchstens {max} Zeichen)."));
                }
                let cleaned = if multiline {
                    clean_text(&s).trim().to_string()
                } else {
                    clean_line(&s)
                };
                Ok((!cleaned.is_empty()).then_some(cleaned))
            }
            Some(_) => Err(format!("„{key}“ muss ein Text sein.")),
        }
    }

    fn required(&mut self, key: &str, max: usize, multiline: bool) -> Result<String, String> {
        self.text(key, max, multiline)?
            .ok_or_else(|| format!("„{key}“ fehlt."))
    }

    /// Eine Kennung: 1 bis 64 Zeichen, nur Buchstaben, Ziffern, `_`, `-`.
    fn id(&mut self, key: &str) -> Result<Option<String>, String> {
        match self.take(key) {
            None => Ok(None),
            Some(Value::String(s)) => {
                let ok = !s.is_empty()
                    && s.len() <= 64
                    && s.bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-');
                if ok {
                    Ok(Some(s))
                } else {
                    Err(format!("„{key}“ ist keine gültige Kennung."))
                }
            }
            Some(_) => Err(format!("„{key}“ muss ein Text sein.")),
        }
    }

    fn integer(&mut self, key: &str, min: i64, max: i64) -> Result<Option<i64>, String> {
        match self.take(key) {
            None => Ok(None),
            Some(Value::Number(n)) => match n.as_i64() {
                Some(v) if (min..=max).contains(&v) => Ok(Some(v)),
                _ => Err(format!(
                    "„{key}“ muss eine ganze Zahl von {min} bis {max} sein."
                )),
            },
            Some(_) => Err(format!("„{key}“ muss eine Zahl sein.")),
        }
    }

    /// Jedes Feld, das niemand entnommen hat, ist ein Fehler (strenge Schemata).
    fn done(self) -> Result<(), String> {
        match self.map.keys().next() {
            None => Ok(()),
            Some(k) => Err(format!(
                "Das Argument „{}“ gibt es bei diesem Werkzeug nicht.",
                clean_line(k).chars().take(40).collect::<String>()
            )),
        }
    }
}

/// Zulaessiger Dateiname fuer eine erzeugte Audiodatei (ohne Endung).
fn file_stem(raw: &str) -> Result<String, String> {
    let name = clean_line(raw);
    let ok = !name.is_empty()
        && name.chars().count() <= MAX_FILE_NAME_CHARS
        && paths::reserved_device_name(&name).is_none()
        && !name.contains("..")
        && !name.starts_with('.')
        && !name.ends_with('.')
        && name
            .chars()
            .all(|c| c.is_alphanumeric() || matches!(c, ' ' | '_' | '-' | '.' | '(' | ')'));
    if ok {
        Ok(name)
    } else {
        Err("„file_name“ darf nur Buchstaben, Ziffern, Leerzeichen und „_ - . ( )“ enthalten (höchstens 100 Zeichen, ohne Endung und ohne Pfad).".to_string())
    }
}

// ---------------------------------------------------------------------------
// Beschreibungen
// ---------------------------------------------------------------------------

fn spec(name: &str, description: &str, input_schema: Value) -> ToolSpec {
    let entry = catalog::find(name).expect("Katalogwerkzeug");
    ToolSpec {
        name: name.to_string(),
        title: entry.title.to_string(),
        description: description.to_string(),
        input_schema,
    }
}

fn string_prop(description: &str, max: usize) -> Value {
    json!({ "type": "string", "maxLength": max, "description": description })
}

/// Die Werkzeuge, die dieser Handler anbietet (alle des Katalogs).
pub fn all_specs() -> Vec<ToolSpec> {
    vec![
        spec(
            "transcribe_file",
            "Reiht eine lokale Audio- oder Videodatei zur Transkription in die Warteschlange von Local Voice AI ein \
             und liefert sofort die meeting_id (Status queued). Die Transkription läuft im Hintergrund; das Ergebnis \
             steht danach in der Besprechung (lesen mit get_transcript, falls der Nutzer das Transkript freigegeben hat). \
             Nur absolute lokale Pfade mit Laufwerk (C:\\...), keine Netzwerkpfade; Formate: Audio und Video.",
            json!({
                "type": "object",
                "properties": {
                    "path": string_prop("Absoluter Pfad der Datei, z. B. C:\\Aufnahmen\\jour-fixe.m4a.", MAX_PATH_ARG_CHARS),
                    "title": string_prop("Titel der Besprechung (Standard: der Dateiname).", MAX_TITLE_CHARS)
                },
                "required": ["path"],
                "additionalProperties": false
            }),
        ),
        spec(
            "create_session",
            "Legt eine Session an (ein Ordner für Aufnahmen). Liefert die session_id.",
            json!({
                "type": "object",
                "properties": { "name": string_prop("Name der Session.", MAX_TITLE_CHARS) },
                "required": ["name"],
                "additionalProperties": false
            }),
        ),
        spec(
            "create_meeting",
            "Legt eine leere Besprechung an (ohne Audio), auf Wunsch gleich in einer Session. Liefert die meeting_id.",
            json!({
                "type": "object",
                "properties": {
                    "title": string_prop("Titel (Standard: „Neue Besprechung“).", MAX_TITLE_CHARS),
                    "session_id": string_prop("Kennung einer vorhandenen Session.", 64)
                },
                "additionalProperties": false
            }),
        ),
        spec(
            "add_youtube_source",
            "Legt zu einem YouTube-Link (ein einzelnes Video) eine Besprechung mit der Quelle YouTube an. Es wird nur \
             Titel und Kanal abgerufen (oEmbed); es wird nichts heruntergeladen. Liefert die meeting_id.",
            json!({
                "type": "object",
                "properties": {
                    "url": string_prop("Link zu einem YouTube-Video (watch, youtu.be, shorts).", MAX_URL_CHARS),
                    "session_id": string_prop("Kennung einer vorhandenen Session.", 64)
                },
                "required": ["url"],
                "additionalProperties": false
            }),
        ),
        spec(
            "tts_page_create",
            "Legt eine Seite in der Vorlesen-Bibliothek an und setzt den Text. Liefert die page_id für tts_render_audio.",
            json!({
                "type": "object",
                "properties": {
                    "title": string_prop("Titel der Seite.", MAX_TITLE_CHARS),
                    "text": string_prop("Der vorzulesende Text.", MAX_TTS_TEXT_CHARS)
                },
                "required": ["title", "text"],
                "additionalProperties": false
            }),
        ),
        spec(
            "tts_render_audio",
            "Erzeugt aus dem Text einer Vorlesen-Seite eine Audiodatei im Ordner der Seite (Stimme und Format wie in \
             der App eingestellt, Standard WAV). Das kann je nach Länge und Engine dauern. Liefert Dateiname, Pfad und Format.",
            json!({
                "type": "object",
                "properties": {
                    "page_id": string_prop("Kennung der Seite aus tts_page_create.", 64),
                    "file_name": string_prop("Dateiname ohne Endung und ohne Pfad (Standard: agent-<Zeitstempel>).", MAX_FILE_NAME_CHARS)
                },
                "required": ["page_id"],
                "additionalProperties": false
            }),
        ),
        spec(
            "start_recording",
            "Bittet die App um eine Aufnahme. Local Voice AI zeigt dem Nutzer IMMER den Einwilligungsdialog (§ 201 StGB: \
             alle Beteiligten müssen zustimmen); ohne seine Bestätigung beginnt nichts. Der Aufruf wartet kurz auf \
             die Entscheidung und meldet sonst pending mit einer approval_id (Stand mit get_action_status).",
            json!({
                "type": "object",
                "properties": {
                    "title": string_prop("Titel der Aufnahme (Standard: „Aufnahme (Agent)“).", MAX_TITLE_CHARS),
                    "max_minutes": { "type": "integer", "minimum": 1, "maximum": MAX_RECORDING_MINUTES,
                                     "description": "Die App beendet die Aufnahme nach so vielen Minuten (Standard 480)." }
                },
                "additionalProperties": false
            }),
        ),
        spec(
            "stop_recording",
            "Beendet die laufende Aufnahme (gleiches Recht wie start_recording).",
            json!({ "type": "object", "additionalProperties": false }),
        ),
    ]
}

// ---------------------------------------------------------------------------
// Ausfuehrung
// ---------------------------------------------------------------------------

/// Der Aufruf einer Aufnahme-Start-Meldung als Klartext (ohne Codes).
fn start_failure_text(code: &str) -> String {
    recording::start_failure(code)
}

impl AppTools {
    fn transcribe_file(&self, mut a: Args) -> Result<Value, String> {
        let path = a.required("path", MAX_PATH_ARG_CHARS, false)?;
        let title = a.text("title", MAX_TITLE_CHARS, false)?;
        a.done()?;
        let resolved = paths::check_media_path(&path).map_err(|e| e.to_string())?;
        let title =
            title.unwrap_or_else(|| crate::managers::meetings::queue::title_from_path(&resolved));
        let queued = self
            .host
            .enqueue_import(&title, &resolved.to_string_lossy())
            .map_err(|e| format!("Die Datei ließ sich nicht einreihen ({e})."))?;
        self.host.meeting_created(&queued.meeting_id);
        Ok(json!({
            "meeting_id": queued.meeting_id,
            "title": queued.title,
            "status": queued.status,
            "hint": "Die Transkription läuft in der Warteschlange der App; der Fortschritt steht in der App unter Aufnahmen.",
        }))
    }

    fn create_session(&self, mut a: Args) -> Result<Value, String> {
        let name = a.required("name", MAX_TITLE_CHARS, false)?;
        a.done()?;
        let folder = self
            .store
            .folder_save(None, &name, None)
            .map_err(|e| format!("Die Session ließ sich nicht anlegen ({e})."))?;
        Ok(json!({ "session_id": folder.id, "name": folder.name }))
    }

    fn create_meeting(&self, mut a: Args) -> Result<Value, String> {
        let title = a.text("title", MAX_TITLE_CHARS, false)?;
        let session = a.id("session_id")?;
        a.done()?;
        let title = title.unwrap_or_else(|| "Neue Besprechung".to_string());
        let meeting = self
            .store
            .create_empty_meeting(&title, session.as_deref())
            .map_err(|e| match e.to_string().as_str() {
                "folder_not_found" => "Die Session gibt es nicht.".to_string(),
                other => format!("Die Besprechung ließ sich nicht anlegen ({other})."),
            })?;
        self.host.meeting_created(&meeting.id);
        Ok(json!({ "meeting_id": meeting.id, "title": meeting.title }))
    }

    fn add_youtube_source(&self, ctx: &CallContext, mut a: Args) -> Result<Value, String> {
        let url = a.required("url", MAX_URL_CHARS, false)?;
        let session = a.id("session_id")?;
        a.done()?;
        let opts = self.host.youtube_options();
        let origin = Origin::agent_external(&ctx.client_id);
        let added = tauri::async_runtime::block_on(source::add_youtube_source_for(
            &self.store,
            &url,
            session.as_deref(),
            None,
            &opts,
            &origin,
        ))
        .map_err(|e| e.to_string())?;
        self.host.meeting_created(&added.meeting.id);
        Ok(json!({
            "meeting_id": added.meeting.id,
            "source": "youtube",
            "title": added.meta.title,
            "channel": added.meta.channel,
            "video_id": added.video.video_id,
            "url": added.video.canonical_url(),
        }))
    }

    fn tts_page_create(&self, mut a: Args) -> Result<Value, String> {
        let title = a.required("title", MAX_TITLE_CHARS, false)?;
        let text = a.required("text", MAX_TTS_TEXT_CHARS, true)?;
        a.done()?;
        let page = self
            .host
            .create_page(&title, &text)
            .map_err(|e| format!("Die Seite ließ sich nicht anlegen ({e})."))?;
        Ok(json!({ "page_id": page.id, "title": page.title, "chars": text.chars().count() }))
    }

    fn tts_render_audio(&self, ctx: &CallContext, mut a: Args) -> Result<Value, String> {
        let page_id = a
            .id("page_id")?
            .ok_or_else(|| "„page_id“ fehlt.".to_string())?;
        let file_name = match a.text("file_name", MAX_FILE_NAME_CHARS, false)? {
            Some(n) => file_stem(&n)?,
            None => format!("agent-{}", chrono::Local::now().format("%Y%m%d-%H%M%S")),
        };
        a.done()?;
        let text = self
            .host
            .page_text(&page_id)
            .map_err(|_| "Die Seite gibt es nicht.".to_string())?;
        let text = clean_text(&text);
        if text.trim().is_empty() {
            return Err("Die Seite hat keinen Text zum Vorlesen.".to_string());
        }
        if text.chars().count() > MAX_TTS_TEXT_CHARS {
            return Err(format!(
                "Der Text der Seite ist zu lang für diesen Weg (höchstens {MAX_TTS_TEXT_CHARS} Zeichen)."
            ));
        }
        // Ein Lauf zugleich: die Sprach-Engine ist teuer und ein neuer Export storniert den alten.
        if self
            .rendering
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return Err(
                "Es läuft gerade schon eine Audio-Erzeugung. Bitte später erneut versuchen."
                    .to_string(),
            );
        }
        let _guard = Releasing(&self.rendering);
        let rendered = self
            .host
            .render_audio(&page_id, &text, &file_name)
            .map_err(|e| format!("Die Audiodatei ließ sich nicht erzeugen ({e})."))?;
        self.record_tts_provenance(ctx, &page_id, &rendered, text.chars().count());
        Ok(json!({
            "page_id": page_id,
            "file": rendered.file_name,
            "path": rendered.path,
            "bytes": rendered.bytes,
            "format": rendered.format,
        }))
    }

    /// Herkunft der erzeugten Audiodatei (Best Effort: ein Fehler hier kippt die fertige Datei nie).
    fn record_tts_provenance(&self, ctx: &CallContext, page_id: &str, r: &Rendered, chars: usize) {
        let result = self
            .store
            .get_connection()
            .map_err(|e| e.to_string())
            .and_then(|conn| {
                let mut entry = NewProvenance::new(
                    SubjectKind::TtsAudio,
                    &format!("{page_id}/{}", r.file_name),
                    "tts_render",
                    ActorKind::AgentExternal,
                );
                entry.actor_ref = Some(ctx.client_id.clone());
                entry.params = Some(json!({ "chars": chars, "format": r.format }));
                provenance::record_at(&conn, &entry, ctx.now_ms)
                    .map(|_| ())
                    .map_err(|e| e.to_string())
            });
        if let Err(e) = result {
            log::warn!("agent_bridge: Herkunft der Audiodatei nicht gespeichert: {e}");
        }
    }

    fn start_recording(&self, ctx: &CallContext, mut a: Args) -> Result<Value, String> {
        let title = a.text("title", MAX_TITLE_CHARS, false)?;
        let max_minutes = a.integer("max_minutes", 1, MAX_RECORDING_MINUTES)?;
        a.done()?;
        // Sperre 3 (wie bei den Ablaeufen): nie ohne die eingeloeste Freigabe des Nutzers. Die
        // Bruecke ruft den Handler fuer dieses Recht nur nach einer Freigabe (`never_allow`);
        // das hier gilt auch, falls je jemand das Register umgeht.
        if !ctx.approved {
            return Err("Ohne Ihre Bestätigung in der App startet keine Aufnahme.".to_string());
        }
        let control = self
            .host
            .recording()
            .ok_or_else(|| "Aufnahmen sind in dieser Umgebung nicht verfügbar.".to_string())?;
        if control.current().is_some() {
            return Err(start_failure_text("already_recording"));
        }
        let title = title.unwrap_or_else(|| "Aufnahme (Agent)".to_string());
        let started = control
            .start(&StartRequest {
                title: title.clone(),
                event_key: None,
            })
            .map_err(|code| start_failure_text(&code))?;
        let now = chrono::Utc::now().timestamp_millis();
        let params = match max_minutes {
            Some(m) => json!({ "max_minutes": m }),
            None => json!({}),
        };
        let stop_at = recording::plan_stop(&params, None, now);
        self.host.schedule_stop(&started.meeting_id, stop_at);
        Ok(json!({
            "meeting_id": started.meeting_id,
            "title": started.title,
            "status": "recording",
            "auto_stop_after_minutes": (stop_at - now) / 60_000,
        }))
    }

    fn stop_recording(&self, a: Args) -> Result<Value, String> {
        a.done()?;
        let control = self
            .host
            .recording()
            .ok_or_else(|| "Aufnahmen sind in dieser Umgebung nicht verfügbar.".to_string())?;
        if control.current().is_none() {
            return Ok(json!({ "stopped": false, "message": "Es lief keine Aufnahme." }));
        }
        match control.stop() {
            Ok(meeting_id) => Ok(json!({ "stopped": true, "meeting_id": meeting_id })),
            Err(e) if e.starts_with("not_recording") => {
                Ok(json!({ "stopped": false, "message": "Es lief keine Aufnahme." }))
            }
            Err(e) => Err(format!("Die Aufnahme ließ sich nicht beenden ({e}).")),
        }
    }
}

/// Gibt die Sperre eines Render-Laufs wieder frei, auch bei Fehler und Panik.
struct Releasing<'a>(&'a AtomicBool);

impl Drop for Releasing<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

impl ToolHandler for AppTools {
    fn specs(&self) -> Vec<ToolSpec> {
        all_specs()
    }

    fn call(&self, ctx: &CallContext, tool: &str, args: &Value) -> Result<Value, String> {
        let a = Args::new(args)?;
        match tool {
            "transcribe_file" => self.transcribe_file(a),
            "create_session" => self.create_session(a),
            "create_meeting" => self.create_meeting(a),
            "add_youtube_source" => self.add_youtube_source(ctx, a),
            "tts_page_create" => self.tts_page_create(a),
            "tts_render_audio" => self.tts_render_audio(ctx, a),
            "start_recording" => self.start_recording(ctx, a),
            "stop_recording" => self.stop_recording(a),
            other => Err(format!("Das Werkzeug „{other}“ gibt es nicht.")),
        }
    }
}

#[cfg(test)]
mod tests;
