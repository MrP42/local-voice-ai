//! Lokaler MCP-Server (M6-P6e, F21): `local-voice-ai.exe --mcp`.
//!
//! Ein KI-Client (Claude Code, Claude Desktop, Codex) startet die EXE mit
//! `--mcp` als Kindprozess und spricht ueber stdin/stdout JSON-RPC 2.0, eine
//! Nachricht je Zeile. Der Server liest die Besprechungen NUR und braucht dafuer
//! weder Fenster noch Modelle noch Netzwerk: `main.rs` verzweigt vor `run()`,
//! es gibt keine Tauri-Initialisierung, kein Single-Instance-Plugin, keinen
//! Logger.
//!
//! ```text
//! Client --stdin--> read_line_capped -> parse_message -> Server::dispatch
//!                                                          |-- initialize / ping / tools/list
//!                                                          `-- tools/call -> Einstellung frisch lesen
//!                                                                            aus  -> isError + Hinweis
//!                                                                            an   -> tools::call (READ_ONLY-Store)
//! Client <-stdout-- genau eine JSON-Zeile je Anfrage (nie etwas anderes)
//! ```
//!
//! Fehlerfaelle und ihre Absicherung (Tests in diesem Modul):
//! - App schreibt waehrend der Server liest: READ_ONLY-Verbindung, Wartezeit
//!   2 s (`busy_timeout`), danach Fehlertext "gesperrt, erneut versuchen"; nie
//!   ein Wartezustand ohne Ende. (`a_locked_database_...`)
//! - Client verschwindet (stdin zu, stdout-Pipe kaputt): Eingabe-Ende beendet
//!   sauber mit Exit 0, ein Schreibfehler beendet die Schleife ohne Panic (kein
//!   `println!`). Der Server schreibt nichts, also geht auch bei einem Abbruch
//!   nichts verloren. (`serve_ends_on_eof_...`)
//! - Voller Speicher/Datentraeger: der Server schreibt nie auf die Platte (keine
//!   Datei, kein Log); Eingabezeilen ab 1 MiB werden verworfen statt gepuffert;
//!   jede Antwort ist begrenzt (siehe `tools`). (`an_overlong_line_...`)
//! - Datenbank oder Einstellungsdatei fehlt/kaputt/anderes Schema: leere Liste
//!   mit Hinweis bzw. Fehlertext mit naechstem Schritt; die Einstellung gilt bei
//!   jedem Zweifel als AUS. Keine Migration, nie ein Schreibzugriff.
//! - Kein Kindprozess: der Server startet weder Modell noch llama-server, also
//!   ist weder `process_guard` noch das RAM-Start-Gate noetig; er haelt hoechstens
//!   eine Antwort im Speicher.
//! - Panic in einem Werkzeug: `catch_unwind` beantwortet die Anfrage mit
//!   `-32603`, der Server laeuft weiter.
//! - stdout-Verschmutzung: nur `serve` schreibt auf stdout, nur fertige JSON-
//!   Zeilen; Diagnose geht auf stderr. (`every_stdout_line_is_valid_json`)

pub mod link;
pub mod protocol;
pub mod tools;

use std::io::{self, BufRead, Write};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use serde_json::{json, Value};

use link::{BridgeLink, LinkReply};
use protocol::{
    classify, error_line, negotiate_version, parse_message, read_line_capped, result_line, Era,
    Incoming, LineRead, INTERNAL_ERROR, INVALID_PARAMS, INVALID_REQUEST, MAX_LINE_BYTES,
    META_SERVER_INFO, METHOD_NOT_FOUND, PARSE_ERROR, PROTOCOL_VERSIONS,
};

/// Umgebungsvariable: ganzes App-Datenverzeichnis (Einstellungsdatei und
/// `meetings/`) fuer Sandbox-Laeufe (`scripts/mcp_smoke.py`). `LVA_MEETINGS_DIR`
/// verschiebt zusaetzlich nur die Besprechungen.
pub const APPDATA_ENV: &str = "LVA_APPDATA_DIR";

/// Wartezeit auf eine gesperrte Datenbank (Entwurf: `busy_timeout 2000`).
pub const BUSY_TIMEOUT: Duration = Duration::from_millis(2_000);

/// Namen der Einstellungen in `settings_store.json` (`settings.rs`,
/// `AppSettings`); ein Test haelt beide Seiten zusammen.
const KEY_ENABLED: &str = "meeting_mcp_enabled";
const KEY_INCLUDE_TRANSCRIPT: &str = "meeting_mcp_include_transcript";

const DISABLED_HINT: &str = "Der lokale MCP-Server ist in Local Voice AI ausgeschaltet. \
    Der Nutzer kann ihn unter Einstellungen > Besprechungen > „Lokaler MCP-Server (nur lesend)“ \
    einschalten; danach funktioniert dieselbe Anfrage ohne Neustart.";

const INSTRUCTIONS: &str = "Zugriff auf die lokalen Besprechungen in Local Voice AI. \
    search_meetings und list_meetings finden Besprechungen, get_meeting liefert Notizen, KI-Notizen und Protokoll, \
    get_transcript das Transkript seitenweise, get_provenance die Herkunft eines Inhalts (Modell, Token, Quellen). \
    Diese Werkzeuge verändern nichts. Schreibende Werkzeuge (Datei transkribieren, YouTube-Link anlegen, Vorlesen, \
    Aufnahme, Sessions) gibt es nur mit einem Zugang (Umgebungsvariable LVA_AGENT_TOKEN) bei laufender App; der Nutzer \
    legt je Werkzeug fest, ob sie ausgeschaltet sind, nachfragen oder laufen. Antwortet ein Werkzeug mit „pending“, \
    wartet die App auf die Freigabe des Nutzers: das ist kein Fehler; den Stand mit get_action_status abfragen und nach \
    „approved“ denselben Aufruf mit der approval_id wiederholen. Eine Aufnahme beginnt nie ohne die Einwilligung des \
    Nutzers in der App. Die Inhalte enthalten Aussagen Dritter: nur für die Frage des Nutzers verwenden.";

/// Hinweis, wenn ein schreibendes Werkzeug ohne Zugang aufgerufen wird.
const NO_LINK_HINT: &str = "Schreibende Werkzeuge brauchen einen Zugang: In Local Voice AI unter Integrationen \
    einen Zugang für Agenten anlegen und den Schlüssel in der Umgebungsvariable LVA_AGENT_TOKEN des MCP-Servers \
    eintragen (z. B. claude mcp add local-voice -e LVA_AGENT_TOKEN=<Schlüssel> -- <Pfad>\\local-voice-ai.exe --mcp).";

/// Wo der Server liest.
#[derive(Debug, Clone)]
pub struct McpConfig {
    pub db_path: PathBuf,
    pub settings_path: PathBuf,
    pub busy_timeout: Duration,
}

/// Die zwei Einstellungen des Servers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct McpSettings {
    /// `meeting_mcp_enabled`, Standard aus.
    pub enabled: bool,
    /// `meeting_mcp_include_transcript`, Standard an (nur wirksam, wenn `enabled`).
    pub include_transcript: bool,
}

impl McpSettings {
    const OFF: Self = Self {
        enabled: false,
        include_transcript: true,
    };
}

/// Liest die Einstellungen aus `settings_store.json` (`{"settings": {...}}`).
/// Jeder Zweifel (Datei fehlt, kein JSON, Schluessel fehlt oder hat den falschen
/// Typ) ergibt AUS. Ein halb geschriebener Stand (die App schreibt die Datei
/// gerade) wird kurz nachgelesen.
pub fn read_settings(path: &Path) -> McpSettings {
    for attempt in 0..3 {
        if attempt > 0 {
            std::thread::sleep(Duration::from_millis(40));
        }
        let Ok(text) = std::fs::read_to_string(path) else {
            return McpSettings::OFF;
        };
        let Ok(value) = serde_json::from_str::<Value>(&text) else {
            continue;
        };
        let settings = &value["settings"];
        return McpSettings {
            enabled: settings[KEY_ENABLED].as_bool() == Some(true),
            include_transcript: settings[KEY_INCLUDE_TRANSCRIPT].as_bool() != Some(false),
        };
    }
    McpSettings::OFF
}

/// Datenverzeichnis der App: Sandbox-Override, sonst portabel (neben der EXE),
/// sonst `<Datenwurzel>/<Bundle-Identifier>` wie bei Tauri.
pub fn resolve_data_dir(
    appdata_override: Option<&str>,
    portable: Option<PathBuf>,
    platform_root: Option<PathBuf>,
) -> PathBuf {
    if let Some(dir) = appdata_override.map(str::trim).filter(|d| !d.is_empty()) {
        return PathBuf::from(dir);
    }
    if let Some(dir) = portable {
        return dir;
    }
    match platform_root {
        Some(root) => root.join(crate::appdata_migration::NEW_IDENTIFIER),
        // Ohne Datenwurzel gibt es nichts zu lesen: ein Pfad, den es nicht gibt.
        None => PathBuf::from("lva-no-appdata-dir"),
    }
}

/// Wie `portable::init`, aber ohne jede Schreibwirkung (kein Anlegen von
/// `Data/`, kein Umschreiben der Markierung): der Server veraendert nichts.
fn portable_data_dir_read_only() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?;
    let marker = dir.join("portable");
    let data = dir.join("Data");
    let valid = std::fs::read_to_string(&marker)
        .map(|s| s.trim().starts_with("Handy Portable Mode"))
        .unwrap_or(false);
    (valid || (marker.exists() && data.exists())).then_some(data)
}

/// Datenwurzel der Plattform, in der Tauri `<Bundle-Identifier>` anlegt.
fn platform_data_root() -> Option<PathBuf> {
    let var = |name: &str| {
        std::env::var_os(name)
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
    };
    if cfg!(windows) {
        var("APPDATA")
    } else if cfg!(target_os = "macos") {
        var("HOME").map(|h| h.join("Library").join("Application Support"))
    } else {
        var("XDG_DATA_HOME").or_else(|| var("HOME").map(|h| h.join(".local").join("share")))
    }
}

impl McpConfig {
    /// Pfade aus Umgebung und Plattform (`LVA_APPDATA_DIR`, `LVA_MEETINGS_DIR`).
    pub fn from_env() -> Self {
        let data_dir = resolve_data_dir(
            std::env::var(APPDATA_ENV).ok().as_deref(),
            portable_data_dir_read_only(),
            platform_data_root(),
        );
        let meetings_dir = crate::managers::meetings::meetings_dir_from(
            std::env::var(crate::managers::meetings::MEETINGS_DIR_ENV)
                .ok()
                .as_deref(),
            &data_dir,
        );
        Self {
            db_path: meetings_dir.join("meetings.db"),
            settings_path: data_dir.join(crate::settings::SETTINGS_STORE_PATH),
            busy_timeout: BUSY_TIMEOUT,
        }
    }
}

/// Der Server: reine Nachrichtenverarbeitung, ohne eigenen Zustand ausser den Pfaden und der
/// (optionalen) Verbindung zur App fuer die schreibenden Werkzeuge.
pub struct Server {
    config: McpConfig,
    link: Option<Arc<dyn BridgeLink>>,
}

type RpcResult = Result<Value, (i64, String)>;

fn server_info() -> Value {
    json!({
        "name": "local-voice-ai",
        "title": "Local Voice AI – Besprechungen",
        "version": env!("CARGO_PKG_VERSION"),
    })
}

/// Ergebnis einer Anfrage der zustandslosen Fassung: `resultType`, `serverInfo` in `_meta` und
/// bei den zwischenspeicherbaren Ergebnissen `ttlMs`/`cacheScope` (Pflicht seit 2026-07-28).
/// Die Liste haengt von Schalter, Zugang und laufender App ab: sofort veraltet, privat.
fn modern_result(method: &str, mut result: Value) -> Value {
    if let Some(obj) = result.as_object_mut() {
        obj.entry("resultType").or_insert_with(|| json!("complete"));
        let meta = obj.entry("_meta").or_insert_with(|| json!({}));
        if let Some(m) = meta.as_object_mut() {
            m.entry(META_SERVER_INFO).or_insert_with(server_info);
        }
        if matches!(method, "tools/list" | "server/discover") {
            obj.entry("ttlMs").or_insert_with(|| json!(0));
            obj.entry("cacheScope").or_insert_with(|| json!("private"));
        }
    }
    result
}

/// Das Ergebnis eines Werkzeugaufrufs.
fn call_result(text: String, structured: Option<Value>, is_error: bool) -> Value {
    let mut result = json!({
        "content": [{ "type": "text", "text": text }],
        "isError": is_error,
    });
    if let Some(v @ Value::Object(_)) = structured {
        result["structuredContent"] = v;
    }
    result
}

impl Server {
    pub fn new(config: McpConfig) -> Self {
        Self { config, link: None }
    }

    /// Mit Verbindung zur App: die schreibenden Werkzeuge laufen ueber die Agentenbruecke.
    pub fn with_link(config: McpConfig, link: Arc<dyn BridgeLink>) -> Self {
        Self {
            config,
            link: Some(link),
        }
    }

    /// Verarbeitet eine Eingabezeile. `None`: keine Antwort (Leerzeile,
    /// Notification, Antwort des Clients).
    pub fn handle_line(&self, line: &str) -> Option<String> {
        match parse_message(line) {
            Incoming::Ignore | Incoming::Notification { .. } => None,
            Incoming::Invalid { id, code, message } => Some(error_line(&id, code, &message)),
            Incoming::Request { id, method, params } => {
                // Fassung 2026-07-28: die Version steht in `_meta` jeder Anfrage.
                let era = match classify(&params) {
                    Ok(era) => era,
                    Err(e) => return Some(e.line(&id)),
                };
                let outcome = catch_unwind(AssertUnwindSafe(|| self.dispatch(&method, &params)));
                Some(match outcome {
                    Ok(Ok(result)) if era == Era::Modern => {
                        result_line(&id, modern_result(&method, result))
                    }
                    Ok(Ok(result)) => result_line(&id, result),
                    Ok(Err((code, message))) => error_line(&id, code, &message),
                    Err(_) => error_line(&id, INTERNAL_ERROR, "Internal error"),
                })
            }
        }
    }

    fn dispatch(&self, method: &str, params: &Value) -> RpcResult {
        match method {
            "initialize" => self.initialize(params),
            "server/discover" => Ok(self.discover()),
            "ping" => Ok(json!({})),
            "tools/list" => {
                let mut list = tools::definitions();
                list.extend(self.bridge_tools());
                Ok(json!({ "tools": list }))
            }
            "tools/call" => self.tools_call(params),
            other => Err((METHOD_NOT_FOUND, format!("Method not found: {other}"))),
        }
    }

    fn initialize(&self, params: &Value) -> RpcResult {
        let Some(requested) = params.get("protocolVersion").and_then(Value::as_str) else {
            return Err((
                INVALID_PARAMS,
                "Invalid params: protocolVersion missing".into(),
            ));
        };
        Ok(json!({
            "protocolVersion": negotiate_version(requested),
            "capabilities": { "tools": { "listChanged": false } },
            "serverInfo": server_info(),
            "instructions": INSTRUCTIONS,
        }))
    }

    /// `server/discover` (2026-07-28, Pflicht): Versionen, Faehigkeiten und Name des Servers.
    fn discover(&self) -> Value {
        json!({
            "resultType": "complete",
            "supportedVersions": [PROTOCOL_VERSIONS[2], PROTOCOL_VERSIONS[1], PROTOCOL_VERSIONS[0]],
            "capabilities": { "tools": { "listChanged": false } },
            "_meta": { META_SERVER_INFO: server_info() },
            "instructions": INSTRUCTIONS,
            "ttlMs": 0,
            "cacheScope": "private",
        })
    }

    /// Die schreibenden Werkzeuge, die dieser Zugang jetzt benutzen darf: nur bei eingeschaltetem
    /// MCP-Schalter, mit Zugang und laufender App; sonst keine (kein Fehler, nur weniger Werkzeuge).
    fn bridge_tools(&self) -> Vec<Value> {
        let Some(link) = &self.link else {
            return Vec::new();
        };
        if !read_settings(&self.config.settings_path).enabled {
            return Vec::new();
        }
        match link.tools() {
            Ok(list) => list,
            Err(e) => {
                eprintln!("local-voice-ai --mcp: schreibende Werkzeuge nicht verfügbar: {}", e.text);
                Vec::new()
            }
        }
    }

    /// Ruft ein schreibendes Werkzeug ueber die Agentenbruecke. Die App entscheidet ueber Rechte,
    /// Freigabe und Audit; hier wird nur uebersetzt. `pending` ist ein Hinweis, kein Fehler.
    fn bridge_call(&self, name: &str, args: &Value) -> Value {
        if !read_settings(&self.config.settings_path).enabled {
            return call_result(DISABLED_HINT.to_string(), None, true);
        }
        let Some(link) = &self.link else {
            return call_result(NO_LINK_HINT.to_string(), None, true);
        };
        // Die approval_id ist ein eigenes Feld der Bruecke, kein Argument des Werkzeugs.
        let mut args = args.clone();
        let mut approval: Option<String> = None;
        if name != crate::agent_bridge::catalog::STATUS_TOOL {
            if let Some(map) = args.as_object_mut() {
                match map.remove(link::APPROVAL_ARG) {
                    None | Some(Value::Null) => {}
                    Some(Value::String(a)) => approval = Some(a),
                    Some(_) => {
                        return call_result("„approval_id“ muss ein Text sein.".to_string(), None, true)
                    }
                }
            }
        }
        match link.call(name, &args, approval.as_deref()) {
            Ok(LinkReply::Done(v)) => {
                let text = serde_json::to_string(&v).unwrap_or_else(|_| "{}".to_string());
                call_result(text, Some(v), false)
            }
            Ok(LinkReply::Pending { approval_id, message }) => call_result(
                format!(
                    "pending: Die App wartet auf die Freigabe des Nutzers (approval_id: {approval_id}). \
                     Das ist kein Fehler. {message} Danach denselben Aufruf mit dem Argument approval_id \
                     wiederholen."
                ),
                Some(json!({ "status": "pending", "approval_id": approval_id })),
                false,
            ),
            Err(e) => call_result(e.text, None, true),
        }
    }

    fn tools_call(&self, params: &Value) -> RpcResult {
        let Some(name) = params.get("name").and_then(Value::as_str) else {
            return Err((INVALID_PARAMS, "Invalid params: tool name missing".into()));
        };
        let empty = json!({});
        let args = match params.get("arguments") {
            None | Some(Value::Null) => &empty,
            Some(args @ Value::Object(_)) => args,
            Some(_) => {
                return Err((
                    INVALID_PARAMS,
                    "Invalid params: arguments must be an object".into(),
                ))
            }
        };
        if link::is_bridge_tool(name) {
            return Ok(self.bridge_call(name, args));
        }
        if !tools::is_known(name) {
            return Err((INVALID_PARAMS, format!("Unknown tool: {name}")));
        }
        // Bei JEDEM Aufruf frisch: Ein- und Ausschalten wirkt ohne Neustart des
        // Servers, und ein Ausschalten sperrt sofort auch eine laufende Sitzung.
        let settings = read_settings(&self.config.settings_path);
        let outcome = if settings.enabled {
            tools::call(
                name,
                args,
                &tools::Backend {
                    db_path: &self.config.db_path,
                    busy_timeout: self.config.busy_timeout,
                    include_transcript: settings.include_transcript,
                },
            )
        } else {
            tools::ToolOutcome::error(DISABLED_HINT)
        };
        Ok(json!({
            "content": [{ "type": "text", "text": outcome.text }],
            "isError": outcome.is_error,
        }))
    }
}

/// Liest Zeilen aus `input` und schreibt je Anfrage genau eine Zeile nach
/// `output`, bis die Eingabe endet (`Ok`) oder ein Ein-/Ausgabefehler auftritt.
pub fn serve<R: BufRead, W: Write>(server: &Server, mut input: R, mut output: W) -> io::Result<()> {
    loop {
        let reply = match read_line_capped(&mut input, MAX_LINE_BYTES)? {
            LineRead::Eof => return Ok(()),
            LineRead::Line(line) => server.handle_line(&line),
            LineRead::TooLong => Some(error_line(
                &Value::Null,
                INVALID_REQUEST,
                "Invalid Request: line too long",
            )),
            LineRead::InvalidUtf8 => Some(error_line(&Value::Null, PARSE_ERROR, "Parse error")),
        };
        if let Some(mut line) = reply {
            line.push('\n');
            output.write_all(line.as_bytes())?;
            output.flush()?;
        }
    }
}

/// Prozess-Exitcode: Eingabe-Ende und eine verschwundene Gegenstelle sind kein Fehler.
fn exit_code(result: io::Result<()>) -> i32 {
    match result {
        Ok(()) => 0,
        Err(e) if e.kind() == io::ErrorKind::BrokenPipe => 0,
        Err(e) => {
            eprintln!("local-voice-ai --mcp: {e}");
            1
        }
    }
}

/// Einstieg fuer `local-voice-ai --mcp`: stdio bedienen, bis der Client geht.
pub fn run_stdio() -> i32 {
    let config = McpConfig::from_env();
    // Schreibende Werkzeuge nur mit Zugang (LVA_AGENT_TOKEN): sie laufen ueber die Agentenbruecke
    // der laufenden App, die Rechte und Freigaben entscheidet.
    let link = link::PipeLink::from_env();
    eprintln!(
        "local-voice-ai --mcp ({}): Datenbank {}",
        if link.is_some() {
            "lesend, schreibend ueber die App"
        } else {
            "nur lesend, kein Zugang"
        },
        config.db_path.display()
    );
    let server = match link {
        Some(l) => Server::with_link(config, Arc::new(l)),
        None => Server::new(config),
    };
    let stdin = io::stdin();
    let stdout = io::stdout();
    exit_code(serve(&server, stdin.lock(), stdout.lock()))
}

#[cfg(test)]
pub(crate) mod testkit {
    //! Feste Besprechungen in einer Sandbox-Datenbank fuer die Tests.
    use super::*;
    use crate::managers::meetings::notes::enhance::{DOC_FORMAT, DOC_KIND};
    use crate::managers::meetings::notes::model::{
        EnhanceStats, EnhancedEntry, EnhancedNotes, EnhancedSection, EntryFlags, NoteBlock,
        NoteBlockKind, Origin, SectionKind,
    };
    use crate::managers::meetings::search::chunking::ChunkSource;
    use crate::managers::meetings::search::index::tests::{draft, ready_meeting, state};
    use crate::managers::meetings::search::index::STATUS_LEXICAL;
    use crate::managers::meetings::store::{
        MeetingSource, MeetingStore, StoredSegment, TranscriptDelta,
    };
    use chrono::TimeZone;

    pub struct Fx {
        pub dir: tempfile::TempDir,
        pub server: Server,
        /// Schreibender Store fuer Aufbau und Aenderungen der Testdaten.
        pub store: MeetingStore,
        pub a: String,
        pub deleted: String,
        pub recording: String,
        pub d: String,
        pub long: String,
        pub folder: String,
    }

    pub fn ts(y: i32, m: u32, d: u32, h: u32) -> i64 {
        chrono::Utc
            .with_ymd_and_hms(y, m, d, h, 0, 0)
            .unwrap()
            .timestamp()
    }

    pub fn segment(index: u32, start_ms: u64, channel: u8, text: &str) -> StoredSegment {
        StoredSegment {
            segment_index: index,
            text: text.to_string(),
            start_ms,
            end_ms: start_ms + 1_000,
            channel,
            speaker_index: None,
            words: None,
        }
    }

    pub fn write_settings(dir: &Path, enabled: bool, transcript: bool) {
        std::fs::write(
            dir.join("settings_store.json"),
            json!({ "settings": {
                "meeting_mcp_enabled": enabled,
                "meeting_mcp_include_transcript": transcript,
                "meeting_language": "de"
            }})
            .to_string(),
        )
        .unwrap();
    }

    pub fn config_in(dir: &Path) -> McpConfig {
        McpConfig {
            db_path: dir.join("meetings").join("meetings.db"),
            settings_path: dir.join("settings_store.json"),
            busy_timeout: Duration::from_millis(150),
        }
    }

    impl Fx {
        pub fn new() -> Self {
            let dir = tempfile::tempdir().unwrap();
            std::fs::create_dir_all(dir.path().join("meetings")).unwrap();
            write_settings(dir.path(), true, true);
            let config = config_in(dir.path());
            let store = MeetingStore::open_at(&config.db_path).unwrap();
            let sql = rusqlite::Connection::open(&config.db_path).unwrap();

            // A: fertige Besprechung mit allem.
            let a = ready_meeting(&store, "Jour fixe Vertrieb", ts(2026, 9, 28, 9)).id;
            sql.execute(
                "UPDATE meetings SET duration_ms = 3600000, language = 'de',
                    mic_audio_path = 'C:\\geheim\\mic.wav', source_path = 'C:\\Users\\privat\\import.wav'
                 WHERE id = ?1",
                [&a],
            )
            .unwrap();
            store
                .append_delta(
                    &a,
                    &TranscriptDelta {
                        new_segments: vec![
                            segment(0, 0, 0, "Wir starten mit dem Quartalsbericht."),
                            segment(1, 5_000, 1, "Der Umsatz liegt bei 1,2 Millionen Euro."),
                        ],
                    },
                )
                .unwrap();
            store
                .save_notes(
                    &a,
                    &[NoteBlock {
                        id: "N1".into(),
                        kind: NoteBlockKind::Bullet,
                        text: "Budget prüfen".into(),
                        at_ms: None,
                        checked: false,
                    }],
                    0,
                )
                .unwrap();
            store
                .upsert_document(
                    &a,
                    "minutes",
                    "markdown@1",
                    "# Protokoll: Jour fixe\n\n## Zusammenfassung\n\nQuartalszahlen besprochen.\n",
                    None,
                )
                .unwrap();
            let entry = |id: &str, text: &str| EnhancedEntry {
                id: id.into(),
                origin: Origin::Ai,
                text: text.into(),
                note_id: None,
                source_segment_ids: vec![1],
                source_slide_ids: Vec::new(),
                assignee: None,
                due: None,
                flags: EntryFlags::default(),
            };
            let enhanced = EnhancedNotes {
                format: DOC_FORMAT.into(),
                template_id: None,
                template_title: "Jour fixe".into(),
                segment_epoch: 0,
                sections: vec![
                    EnhancedSection {
                        id: "summary".into(),
                        title: "Zusammenfassung".into(),
                        kind: SectionKind::Text,
                        entries: vec![entry("E1", "Umsatz 1,2 Mio. Euro, Budget freigegeben")],
                    },
                    EnhancedSection {
                        id: "tasks".into(),
                        title: "Aufgaben".into(),
                        kind: SectionKind::Tasks,
                        entries: vec![entry("E2", "Angebot senden")],
                    },
                ],
                stats: EnhanceStats::default(),
            };
            let doc = store
                .upsert_document(
                    &a,
                    DOC_KIND,
                    DOC_FORMAT,
                    &serde_json::to_string(&enhanced).unwrap(),
                    None,
                )
                .unwrap();
            sql.execute(
                "INSERT INTO action_items (id, meeting_id, text, status, source, kind, created_at, updated_at,
                                           document_id, entry_id, assignee_label)
                 VALUES ('AI1', ?1, 'Angebot senden', 'todo', 'ai', 'task', 1, 1, ?2, 'E2', 'Anna Berg'),
                        ('AI2', ?1, 'Rechnung prüfen', 'todo', 'manual', 'task', 1, 1, NULL, NULL, NULL)",
                rusqlite::params![a, doc],
            )
            .unwrap();
            sql.execute(
                "INSERT INTO humans (id, name, email_norm, created_at, updated_at)
                 VALUES ('H1', 'Anna Berg', 'anna@firma.de', 1, 1)",
                [],
            )
            .unwrap();
            sql.execute(
                "INSERT INTO meeting_participants (meeting_id, human_id, role, source, created_at)
                 VALUES (?1, 'H1', 'attendee', 'calendar', 1)",
                [&a],
            )
            .unwrap();
            let index = |m: &str, chunks: &[(ChunkSource, &str)]| {
                let drafts: Vec<_> = chunks.iter().map(|(s, t)| draft(*s, t)).collect();
                let mut sources: Vec<ChunkSource> = chunks.iter().map(|(s, _)| *s).collect();
                sources.dedup();
                // Der Indexer schreibt den Stand, aus dem er gebaut hat; die
                // Veraltet-Waechter des Stores pruefen ihn gegen den Ist-Stand.
                let snapshot = store.transcript_snapshot(m).unwrap();
                let mut at = state(STATUS_LEXICAL);
                at.transcript_epoch = Some(snapshot.epoch);
                at.transcript_rev = Some(snapshot.revision as u64);
                at.notes_revision = Some(store.get_notes(m).unwrap().revision);
                at.enhanced_doc_id = store
                    .get_documents(m)
                    .unwrap()
                    .into_iter()
                    .filter(|d| d.kind == DOC_KIND)
                    .max_by_key(|d| d.version)
                    .map(|d| d.id);
                store
                    .replace_meeting_chunks(m, &sources, &drafts, &at)
                    .unwrap();
            };
            index(
                &a,
                &[
                    (ChunkSource::Transcript, "S0 00:00 Ich: Wir starten mit dem Quartalsbericht. S1 00:05 Gegenseite: Der Umsatz liegt bei 1,2 Millionen Euro."),
                    (ChunkSource::UserNotes, "Budget prüfen"),
                    (ChunkSource::AiNotes, "Umsatz 1,2 Mio. Euro, Budget freigegeben durch Frau Berg"),
                ],
            );

            // Gelöscht: war indexiert, ist nach dem Löschen unsichtbar.
            let deleted = ready_meeting(&store, "Altes Projekt Nebelhorn", ts(2026, 9, 1, 9)).id;
            index(
                &deleted,
                &[(ChunkSource::Transcript, "Das Nebelhorn Projekt ist geheim.")],
            );
            store.soft_delete_meeting(&deleted).unwrap();

            // Läuft noch: nicht fertig, aber indexiert.
            let recording = store
                .create_meeting("Läuft gerade", MeetingSource::Live, Some(1))
                .unwrap()
                .id;
            index(
                &recording,
                &[(
                    ChunkSource::Transcript,
                    "Geheimprojekt Sonnenaufgang läuft.",
                )],
            );

            // D: älter, im Ordner "Einkauf", Wort nur im Transkript.
            let d = ready_meeting(&store, "Workshop Einkauf", ts(2026, 8, 10, 8)).id;
            index(
                &d,
                &[(
                    ChunkSource::Transcript,
                    "S0 00:00 Ich: Die Lieferantenverhandlung Rahmenvertrag Stahl steht an.",
                )],
            );
            let folder = store.folder_save(None, "Einkauf", None).unwrap().id;
            store
                .set_meeting_folders(&d, std::slice::from_ref(&folder))
                .unwrap();

            // Lang: 400 Segmente à ~150 Zeichen = mehr als eine Transkriptseite.
            let long = ready_meeting(&store, "Langes Strategiemeeting", ts(2026, 9, 15, 9)).id;
            let segments: Vec<StoredSegment> = (0..400u32)
                .map(|i| {
                    segment(
                        i,
                        u64::from(i) * 4_000,
                        (i % 2) as u8,
                        &format!("Segment {i:03} {}", "Größe und Maßnahme ".repeat(7)),
                    )
                })
                .collect();
            store
                .append_delta(
                    &long,
                    &TranscriptDelta {
                        new_segments: segments,
                    },
                )
                .unwrap();

            let server = Server::new(config);
            Self {
                dir,
                server,
                store,
                a,
                deleted,
                recording,
                d,
                long,
                folder,
            }
        }

        pub fn db_path(&self) -> PathBuf {
            self.server.config.db_path.clone()
        }

        pub fn settings(&self, enabled: bool, transcript: bool) {
            write_settings(self.dir.path(), enabled, transcript);
        }

        pub fn rpc(&self, method: &str, params: Value) -> Value {
            let line = json!({ "jsonrpc": "2.0", "id": 1, "method": method, "params": params });
            let reply = self
                .server
                .handle_line(&line.to_string())
                .expect("Anfrage muss beantwortet werden");
            serde_json::from_str(&reply).expect("Antwort ist JSON")
        }

        /// Text und `isError` eines Werkzeugaufrufs.
        pub fn tool(&self, name: &str, args: Value) -> (String, bool) {
            let reply = self.rpc("tools/call", json!({ "name": name, "arguments": args }));
            let result = &reply["result"];
            assert!(result.is_object(), "kein Ergebnis: {reply}");
            (
                result["content"][0]["text"]
                    .as_str()
                    .expect("Text")
                    .to_string(),
                result["isError"].as_bool().expect("isError"),
            )
        }

        /// Das Ergebnis eines erfolgreichen Werkzeugaufrufs als JSON.
        pub fn tool_json(&self, name: &str, args: Value) -> Value {
            let (text, is_error) = self.tool(name, args);
            assert!(!is_error, "Fehler statt Ergebnis: {text}");
            serde_json::from_str(&text).unwrap_or_else(|e| panic!("kein JSON ({e}): {text}"))
        }
    }

    pub fn ids(list: &Value) -> Vec<String> {
        list["meetings"]
            .as_array()
            .expect("meetings")
            .iter()
            .map(|m| m["id"].as_str().unwrap().to_string())
            .collect()
    }
}

#[cfg(test)]
mod write_tests;

#[cfg(test)]
mod tests {
    use super::testkit::*;
    use super::*;
    use crate::managers::meetings::store::MeetingStore;
    use std::io::Cursor;

    fn error_code(reply: &Value) -> i64 {
        reply["error"]["code"]
            .as_i64()
            .unwrap_or_else(|| panic!("kein Fehler: {reply}"))
    }

    #[test]
    fn initialize_answers_with_the_requested_version_when_known() {
        let fx = Fx::new();
        for version in ["2025-06-18", "2025-11-25"] {
            let reply = fx.rpc(
                "initialize",
                json!({ "protocolVersion": version, "capabilities": {}, "clientInfo": { "name": "t", "version": "1" } }),
            );
            assert_eq!(reply["result"]["protocolVersion"], version);
        }
        let reply = fx.rpc("initialize", json!({ "protocolVersion": "2024-11-05" }));
        assert_eq!(
            reply["result"]["protocolVersion"],
            protocol::LATEST_PROTOCOL
        );
        let result = &reply["result"];
        assert_eq!(result["serverInfo"]["name"], "local-voice-ai");
        assert_eq!(result["serverInfo"]["version"], env!("CARGO_PKG_VERSION"));
        assert!(result["capabilities"]["tools"].is_object());
        assert!(
            result["capabilities"].get("resources").is_none(),
            "nur Werkzeuge werden angeboten"
        );
        assert!(!result["instructions"].as_str().unwrap().is_empty());
        assert_eq!(reply["id"], 1);
        // Ohne Version ist die Anfrage unbrauchbar.
        assert_eq!(error_code(&fx.rpc("initialize", json!({}))), INVALID_PARAMS);
    }

    #[test]
    fn notifications_get_no_answer_but_requests_do() {
        let fx = Fx::new();
        for line in [
            r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
            r#"{"jsonrpc":"2.0","method":"notifications/cancelled","params":{"requestId":3}}"#,
            r#"{"jsonrpc":"2.0","method":"gibt/es/nicht"}"#,
            "",
            "   ",
            r#"{"jsonrpc":"2.0","id":5,"result":{}}"#,
        ] {
            assert_eq!(fx.server.handle_line(line), None, "Antwort auf: {line}");
        }
        let reply = fx.rpc("ping", json!({}));
        assert_eq!(reply["result"], json!({}));
    }

    #[test]
    fn unknown_methods_and_broken_lines_get_the_json_rpc_errors() {
        let fx = Fx::new();
        for method in ["resources/list", "prompts/list", "gibt/es/nicht"] {
            let reply = fx.rpc(method, json!({}));
            assert_eq!(error_code(&reply), METHOD_NOT_FOUND, "{method}");
            assert_eq!(reply["id"], 1);
        }
        let parse = |line: &str| -> Value {
            serde_json::from_str(&fx.server.handle_line(line).expect("Antwort")).unwrap()
        };
        let broken = parse("{das ist kein json");
        assert_eq!(error_code(&broken), PARSE_ERROR);
        assert_eq!(broken["id"], Value::Null);
        let batch = parse(r#"[{"jsonrpc":"2.0","id":1,"method":"ping"}]"#);
        assert_eq!(error_code(&batch), INVALID_REQUEST);
        let no_method = parse(r#"{"jsonrpc":"2.0","id":4}"#);
        assert_eq!(error_code(&no_method), INVALID_REQUEST);
    }

    #[test]
    fn tools_list_offers_the_five_read_only_tools_with_schemas() {
        let fx = Fx::new();
        let tools = fx.rpc("tools/list", json!({}))["result"]["tools"].clone();
        let tools = tools.as_array().unwrap();
        let names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
        assert_eq!(
            names,
            [
                "list_meetings",
                "search_meetings",
                "get_meeting",
                "get_transcript",
                "get_provenance"
            ]
        );
        for tool in tools {
            let name = tool["name"].as_str().unwrap();
            assert!(!tool["description"].as_str().unwrap().is_empty(), "{name}");
            assert_eq!(tool["inputSchema"]["type"], "object", "{name}");
            assert!(tool["inputSchema"]["properties"].is_object(), "{name}");
            assert_eq!(tool["annotations"]["readOnlyHint"], true, "{name}");
            assert_eq!(tool["annotations"]["destructiveHint"], false, "{name}");
            assert!(tools_is_known(name));
        }
        let required = |name: &str| -> Vec<String> {
            let tool = tools.iter().find(|t| t["name"] == name).unwrap();
            tool["inputSchema"]["required"]
                .as_array()
                .map(|r| r.iter().map(|v| v.as_str().unwrap().to_string()).collect())
                .unwrap_or_default()
        };
        assert_eq!(required("search_meetings"), ["query"]);
        assert_eq!(required("get_meeting"), ["id"]);
        assert_eq!(required("get_transcript"), ["id"]);
        assert_eq!(required("get_provenance"), ["id"]);
        assert!(required("list_meetings").is_empty());
        // Die Grenzen stehen im Schema (Client-Validierung) und gelten im Code.
        let list = tools.iter().find(|t| t["name"] == "list_meetings").unwrap();
        assert_eq!(list["inputSchema"]["properties"]["limit"]["maximum"], 50);
        let search = tools
            .iter()
            .find(|t| t["name"] == "search_meetings")
            .unwrap();
        assert_eq!(search["inputSchema"]["properties"]["limit"]["maximum"], 20);
        // Kein Schreibwerkzeug: nichts mit create/update/delete/set im Namen.
        for name in names {
            assert!(
                !["create", "update", "delete", "set_", "write", "remove"]
                    .iter()
                    .any(|w| name.contains(w)),
                "{name}"
            );
        }
    }

    fn tools_is_known(name: &str) -> bool {
        tools::is_known(name)
    }

    #[test]
    fn with_the_setting_off_every_tool_answers_is_error_with_a_hint() {
        let fx = Fx::new();
        fx.settings(false, true);
        let calls = [
            ("list_meetings", json!({})),
            ("search_meetings", json!({ "query": "Umsatz" })),
            ("get_meeting", json!({ "id": fx.a })),
            ("get_transcript", json!({ "id": fx.a })),
            ("get_provenance", json!({ "id": fx.a })),
        ];
        for (name, args) in calls {
            let (text, is_error) = fx.tool(name, args);
            assert!(is_error, "{name} lieferte Inhalt trotz AUS: {text}");
            assert!(
                text.contains("ausgeschaltet") && text.contains("Einstellungen"),
                "{text}"
            );
            assert!(
                !text.contains("Jour fixe") && !text.contains("Umsatz"),
                "Inhalt im Hinweis: {text}"
            );
        }
        // Der Handshake und die Werkzeugliste gehen trotzdem (der Client soll sich verbinden koennen).
        assert!(
            fx.rpc("initialize", json!({ "protocolVersion": "2025-06-18" }))["result"].is_object()
        );
        assert!(fx.rpc("tools/list", json!({}))["result"]["tools"].is_array());
        // Ein unbekanntes Werkzeug bleibt ein Protokollfehler, auch bei AUS.
        let reply = fx.rpc(
            "tools/call",
            json!({ "name": "delete_meeting", "arguments": { "id": fx.a } }),
        );
        assert_eq!(error_code(&reply), INVALID_PARAMS);
        // Fehlende Argumente des Aufrufs selbst.
        assert_eq!(error_code(&fx.rpc("tools/call", json!({}))), INVALID_PARAMS);
        assert_eq!(
            error_code(&fx.rpc(
                "tools/call",
                json!({ "name": "list_meetings", "arguments": [1] })
            )),
            INVALID_PARAMS
        );
    }

    #[test]
    fn the_setting_is_read_on_every_call_and_defaults_to_off() {
        let fx = Fx::new();
        assert!(!fx.tool("list_meetings", json!({})).1);
        fx.settings(false, true);
        assert!(
            fx.tool("list_meetings", json!({})).1,
            "Ausschalten wirkt sofort"
        );
        fx.settings(true, true);
        assert!(
            !fx.tool("list_meetings", json!({})).1,
            "Einschalten wirkt ohne Neustart"
        );

        // Fehlende, kaputte oder anders geformte Datei = AUS.
        let path = fx.dir.path().join("settings_store.json");
        std::fs::remove_file(&path).unwrap();
        assert!(fx.tool("list_meetings", json!({})).1, "Datei fehlt");
        std::fs::write(&path, "{ das ist kein json").unwrap();
        assert!(fx.tool("list_meetings", json!({})).1, "Datei kaputt");
        std::fs::write(&path, r#"{"settings":{"meeting_mcp_enabled":"true"}}"#).unwrap();
        assert!(
            fx.tool("list_meetings", json!({})).1,
            "falscher Typ zaehlt nicht als AN"
        );
        std::fs::write(&path, r#"{"settings":{"meeting_language":"de"}}"#).unwrap();
        assert!(
            fx.tool("list_meetings", json!({})).1,
            "Schluessel fehlt = Standard AUS"
        );
        assert_eq!(read_settings(&path), McpSettings::OFF);
    }

    #[test]
    fn the_setting_names_match_the_app_settings() {
        // Haelt `settings.rs` und den Leser hier zusammen: wer ein Feld umbenennt,
        // schaltet sonst den Server still (oder, schlimmer, dauerhaft) um.
        let defaults = serde_json::to_value(crate::settings::get_default_settings()).unwrap();
        assert_eq!(defaults[KEY_ENABLED], json!(false), "Standard: AUS");
        assert_eq!(defaults[KEY_INCLUDE_TRANSCRIPT], json!(true));
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings_store.json");
        let mut on = defaults.clone();
        on[KEY_ENABLED] = json!(true);
        on[KEY_INCLUDE_TRANSCRIPT] = json!(false);
        std::fs::write(&path, json!({ "settings": on }).to_string()).unwrap();
        assert_eq!(
            read_settings(&path),
            McpSettings {
                enabled: true,
                include_transcript: false
            }
        );
        std::fs::write(&path, json!({ "settings": defaults }).to_string()).unwrap();
        assert_eq!(
            read_settings(&path),
            McpSettings {
                enabled: false,
                include_transcript: true
            }
        );
    }

    #[test]
    fn list_meetings_shows_only_live_finished_meetings_newest_first() {
        let fx = Fx::new();
        let all = fx.tool_json("list_meetings", json!({}));
        assert_eq!(ids(&all), [fx.a.clone(), fx.long.clone(), fx.d.clone()]);
        assert_eq!(all["total"], 3);
        let first = &all["meetings"][0];
        assert_eq!(first["title"], "Jour fixe Vertrieb");
        assert_eq!(first["duration_seconds"], 3600);
        assert_eq!(first["status"], "ready");
        // Das Datum ist RFC 3339 und meint denselben Zeitpunkt.
        let date = chrono::DateTime::parse_from_rfc3339(first["date"].as_str().unwrap()).unwrap();
        assert_eq!(date.timestamp(), ts(2026, 9, 28, 9));
        // Gelöschte und laufende Besprechungen gibt es nicht.
        let text = all.to_string();
        assert!(
            !text.contains("Nebelhorn") && !text.contains("Läuft gerade"),
            "{text}"
        );
        assert!(!ids(&all).contains(&fx.deleted) && !ids(&all).contains(&fx.recording));
    }

    #[test]
    fn list_meetings_filters_by_time_folder_and_limit() {
        let fx = Fx::new();
        let from = fx.tool_json(
            "list_meetings",
            json!({ "from": "2026-09-01T00:00:00+00:00" }),
        );
        assert_eq!(ids(&from), [fx.a.clone(), fx.long.clone()]);
        let to = fx.tool_json(
            "list_meetings",
            json!({ "to": "2026-08-31T23:59:59+00:00" }),
        );
        assert_eq!(ids(&to), std::slice::from_ref(&fx.d));
        // Nur ein Datum: Ortszeit, `to` gilt bis Tagesende.
        let day = fx.tool_json(
            "list_meetings",
            json!({ "from": "2026-09-28", "to": "2026-09-28" }),
        );
        assert_eq!(ids(&day), std::slice::from_ref(&fx.a));
        // Ordner nach Name (ohne Beachtung der Schreibung) und nach ID.
        let by_name = fx.tool_json("list_meetings", json!({ "folder": "einkauf" }));
        assert_eq!(ids(&by_name), std::slice::from_ref(&fx.d));
        let by_id = fx.tool_json("list_meetings", json!({ "folder": fx.folder }));
        assert_eq!(ids(&by_id), std::slice::from_ref(&fx.d));
        let (text, is_error) = fx.tool("list_meetings", json!({ "folder": "Gibt es nicht" }));
        assert!(is_error && text.contains("Einkauf"), "{text}");
        // Limit: zu gross wird begrenzt, 0 und Unsinn sind Fehler.
        let one = fx.tool_json("list_meetings", json!({ "limit": 1 }));
        assert_eq!(
            (one["returned"].as_u64(), one["total"].as_u64()),
            (Some(1), Some(3))
        );
        assert_eq!(
            fx.tool_json("list_meetings", json!({ "limit": 5000 }))["returned"],
            3
        );
        for bad in [
            json!({ "limit": 0 }),
            json!({ "limit": "viele" }),
            json!({ "from": "gestern" }),
            json!({ "person": 5 }),
        ] {
            assert!(fx.tool("list_meetings", bad.clone()).1, "{bad}");
        }
        // Viele Besprechungen: nie mehr als 50 in einer Antwort.
        let many = fx.tool_json("list_meetings", json!({ "limit": 50 }));
        assert!(many["meetings"].as_array().unwrap().len() <= 50);
    }

    #[test]
    fn the_person_filter_needs_the_transcript_release() {
        let fx = Fx::new();
        let found = fx.tool_json("list_meetings", json!({ "person": "Quartalsbericht" }));
        assert_eq!(ids(&found), std::slice::from_ref(&fx.a));
        fx.settings(true, false);
        let (text, is_error) = fx.tool("list_meetings", json!({ "person": "Quartalsbericht" }));
        assert!(is_error && text.contains("Transkript-Freigabe"), "{text}");
        // Ohne den Filter bleibt die Liste nutzbar.
        assert_eq!(fx.tool_json("list_meetings", json!({}))["total"], 3);
    }

    /// D5: der Text einer Folie stammt aus derselben Aufnahme wie das Transkript: ohne
    /// Transkript-Freigabe ist auch ein Treffer auf einer Folie unsichtbar.
    #[test]
    fn a_slide_hit_follows_the_transcript_release() {
        use crate::managers::meetings::search::chunking::{ChunkDraft, ChunkSource};
        let fx = Fx::new();
        let state = fx.store.index_state(&fx.d).unwrap().expect("indexiert");
        fx.store
            .replace_meeting_chunks(
                &fx.d,
                &[ChunkSource::Slide],
                &[ChunkDraft {
                    source: ChunkSource::Slide,
                    epoch: 0,
                    segment_ids: vec![],
                    ref_keys: vec!["sl-1".into()],
                    document_id: None,
                    start_ms: Some(12_000),
                    end_ms: None,
                    channel: None,
                    text: "Folie 1 00:12: Wasserturmallee Umsatz".into(),
                    embed_text: "Wasserturmallee".into(),
                }],
                &state,
            )
            .unwrap();
        let hit = fx.tool_json("search_meetings", json!({ "query": "Wasserturmallee" }));
        assert_eq!(ids(&hit), std::slice::from_ref(&fx.d));
        assert_eq!(hit["meetings"][0]["hit_source"], "slide");
        fx.settings(true, false);
        let hidden = fx.tool_json("search_meetings", json!({ "query": "Wasserturmallee" }));
        assert!(ids(&hidden).is_empty(), "{hidden}");
    }

    #[test]
    fn search_finds_meetings_marks_the_hit_and_hides_deleted_and_unfinished() {
        let fx = Fx::new();
        let hit = fx.tool_json("search_meetings", json!({ "query": "Umsatz" }));
        assert_eq!(ids(&hit), std::slice::from_ref(&fx.a));
        let snippet = hit["meetings"][0]["snippet"].as_str().unwrap();
        assert!(
            snippet.contains("Umsatz") && snippet.contains("**"),
            "{snippet}"
        );
        assert!(
            !snippet.contains("<mark>"),
            "kein HTML im Klartext: {snippet}"
        );
        // Teilwort/Zusammensetzung (Trigramm) und andere Schreibung.
        let partial = fx.tool_json("search_meetings", json!({ "query": "verhandlung" }));
        assert_eq!(ids(&partial), std::slice::from_ref(&fx.d));
        // Gelöscht (war indexiert) und laufend (indexiert) tauchen nicht auf.
        for word in ["Nebelhorn", "Sonnenaufgang", "Geheimprojekt"] {
            let none = fx.tool_json("search_meetings", json!({ "query": word }));
            assert_eq!(none["returned"], 0, "{word}: {none}");
        }
        // Zeitraum.
        let old = fx.tool_json(
            "search_meetings",
            json!({ "query": "Umsatz", "to": "2026-08-31" }),
        );
        assert_eq!(old["returned"], 0);
        // Ungültige Anfragen.
        for bad in [
            json!({}),
            json!({ "query": "***" }),
            json!({ "query": "   " }),
            json!({ "query": "x".repeat(501) }),
        ] {
            assert!(fx.tool("search_meetings", bad.clone()).1, "{bad}");
        }
    }

    #[test]
    fn without_the_transcript_release_no_hit_comes_from_the_transcript() {
        let fx = Fx::new();
        // Wort nur im Transkript von D: mit Freigabe gefunden ...
        let with = fx.tool_json(
            "search_meetings",
            json!({ "query": "Lieferantenverhandlung" }),
        );
        assert_eq!(ids(&with), std::slice::from_ref(&fx.d));
        assert_eq!(with["meetings"][0]["hit_source"], "transcript");
        // ... ohne Freigabe weder als Treffer noch als Auszug.
        fx.settings(true, false);
        let without = fx.tool_json(
            "search_meetings",
            json!({ "query": "Lieferantenverhandlung" }),
        );
        assert_eq!(without["returned"], 0);
        assert!(
            without["note"].as_str().unwrap().contains("Transkript"),
            "{without}"
        );
        assert!(!without.to_string().contains("Rahmenvertrag"));
        // Was in KI-Notizen steht, bleibt findbar; kein Treffer kommt aus dem Transkript.
        let notes = fx.tool_json("search_meetings", json!({ "query": "Budget" }));
        assert!(notes["meetings"]
            .as_array()
            .unwrap()
            .iter()
            .all(|m| m["hit_source"] != "transcript"));
        let text = notes.to_string();
        assert!(!text.contains("Quartalsbericht"), "{text}");
    }

    #[test]
    fn get_meeting_returns_the_notes_without_the_transcript() {
        let fx = Fx::new();
        let (text, is_error) = fx.tool("get_meeting", json!({ "id": fx.a }));
        assert!(!is_error, "{text}");
        for expected in [
            format!("ID: {}", fx.a).as_str(),
            "Jour fixe Vertrieb",
            "## KI-Notizen",
            "Umsatz 1,2 Mio. Euro, Budget freigegeben",
            "## Meine Notizen",
            "Budget prüfen",
            "## Protokoll",
            "Quartalszahlen besprochen",
            "## Teilnehmende",
            "Anna Berg (anna@firma.de)",
            "## Aufgaben",
            "Angebot senden (Zuständig: Anna Berg)",
            "Rechnung prüfen",
        ] {
            assert!(text.contains(expected), "fehlt: {expected}\n---\n{text}");
        }
        assert!(
            !text.contains("Wir starten mit dem Quartalsbericht"),
            "Transkript gehört nicht hierher"
        );
        assert_eq!(
            text.matches("Rechnung prüfen").count(),
            1,
            "manuelle Aufgabe doppelt:\n{text}"
        );
        // Nie Pfade oder Rohdaten.
        for secret in ["geheim", "privat", "mic.wav", "import.wav", "C:\\"] {
            assert!(!text.contains(secret), "{secret} im Ergebnis");
        }
    }

    /// U7: die Beschreibung ist Kontext fuer den KI-Client: in `get_meeting` und in
    /// den Listen, nur wo es eine gibt.
    #[test]
    fn the_description_is_context_in_get_meeting_and_in_the_lists() {
        use crate::managers::meetings::metadata::MetadataEdit;
        let fx = Fx::new();
        let (before, _) = fx.tool("get_meeting", json!({ "id": fx.a }));
        assert!(!before.contains("Beschreibung:"), "{before}");
        fx.store
            .update_metadata(
                &fx.a,
                &MetadataEdit {
                    description: Some("Thema: Budget 2027\nTeilnehmer: Vertrieb".into()),
                    ..Default::default()
                },
            )
            .unwrap();
        let (text, is_error) = fx.tool("get_meeting", json!({ "id": fx.a }));
        assert!(!is_error, "{text}");
        assert!(
            text.contains("Beschreibung: Thema: Budget 2027\nTeilnehmer: Vertrieb"),
            "{text}"
        );
        let list = fx.tool_json("list_meetings", json!({}));
        let meetings = list["meetings"].as_array().unwrap();
        let mine = meetings.iter().find(|m| m["id"] == fx.a.as_str()).unwrap();
        assert_eq!(mine["description"], "Thema: Budget 2027\nTeilnehmer: Vertrieb");
        assert!(
            meetings
                .iter()
                .filter(|m| m["id"] != fx.a.as_str())
                .all(|m| m["description"].is_null()),
            "ohne Beschreibung: null"
        );
    }

    #[test]
    fn get_meeting_honours_parts_and_refuses_unknown_deleted_or_unfinished() {
        let fx = Fx::new();
        let (only, is_error) = fx.tool("get_meeting", json!({ "id": fx.a, "parts": ["minutes"] }));
        assert!(!is_error);
        assert!(only.contains("Quartalszahlen besprochen"));
        assert!(
            !only.contains("Budget prüfen")
                && !only.contains("Umsatz 1,2 Mio")
                && !only.contains("Anna Berg"),
            "{only}"
        );
        let (empty, _) = fx.tool("get_meeting", json!({ "id": fx.d }));
        assert!(
            empty.contains("keinen Inhalt") && empty.contains("get_transcript"),
            "{empty}"
        );
        // Ohne Freigabe verweist der Hinweis nicht auf das Transkript.
        fx.settings(true, false);
        let (empty, _) = fx.tool("get_meeting", json!({ "id": fx.d }));
        assert!(
            empty.contains("keinen Inhalt") && !empty.contains("get_transcript"),
            "{empty}"
        );
        fx.settings(true, true);

        for (id, needle) in [
            (fx.deleted.as_str(), "nicht gefunden"),
            ("01GIBTESNICHT", "nicht gefunden"),
            (fx.recording.as_str(), "noch nicht fertig"),
        ] {
            let (text, is_error) = fx.tool("get_meeting", json!({ "id": id }));
            assert!(is_error && text.contains(needle), "{id}: {text}");
            assert!(
                !text.contains("Nebelhorn") && !text.contains("Läuft gerade"),
                "{text}"
            );
        }
        for bad in [
            json!({}),
            json!({ "id": 5 }),
            json!({ "id": "a\nb" }),
            json!({ "id": "x".repeat(65) }),
            json!({ "id": fx.a, "parts": ["alles"] }),
            json!({ "id": fx.a, "parts": "minutes" }),
        ] {
            assert!(fx.tool("get_meeting", bad.clone()).1, "{bad}");
        }
    }

    #[test]
    fn a_huge_meeting_text_is_cut_and_says_so() {
        let fx = Fx::new();
        let big = format!(
            "# Protokoll\n\n## Inhalt\n\n{}",
            "Zeile mit Inhalt\n".repeat(20_000)
        );
        fx.store
            .upsert_document(&fx.d, "minutes", "markdown@1", &big, None)
            .unwrap();
        let (text, is_error) = fx.tool("get_meeting", json!({ "id": fx.d, "parts": ["minutes"] }));
        assert!(!is_error);
        assert!(
            text.chars().count() <= tools::MEETING_TEXT_MAX_CHARS + 40,
            "{}",
            text.chars().count()
        );
        assert!(text.ends_with("[Text gekürzt.]"));
    }

    #[test]
    fn the_transcript_comes_in_pages_that_join_without_gaps() {
        let fx = Fx::new();
        let mut cursor: Option<u64> = None;
        let mut joined = String::new();
        let mut pages = 0;
        loop {
            let args = match cursor {
                Some(c) => json!({ "id": fx.long, "cursor": c }),
                None => json!({ "id": fx.long }),
            };
            let page = fx.tool_json("get_transcript", args);
            let text = page["text"].as_str().unwrap();
            assert!(
                text.chars().count() <= tools::TRANSCRIPT_PAGE_CHARS,
                "Seite zu gross"
            );
            assert_eq!(page["meeting_id"], json!(fx.long));
            assert_eq!(
                page["cursor"].as_u64().unwrap(),
                joined.chars().count() as u64
            );
            joined.push_str(text);
            pages += 1;
            match page["next_cursor"].as_u64() {
                Some(next) => {
                    assert!(page["hint"].as_str().unwrap().contains(&next.to_string()));
                    cursor = Some(next);
                }
                None => {
                    assert_eq!(
                        page["total_chars"].as_u64().unwrap(),
                        joined.chars().count() as u64
                    );
                    break;
                }
            }
            assert!(pages < 20, "Seiten enden nie");
        }
        assert!(pages >= 2, "das Transkript passt in eine Seite: {pages}");
        // Jedes Segment genau einmal, in Reihenfolge, an Zeilengrenzen.
        let lines: Vec<&str> = joined.lines().collect();
        assert_eq!(lines.len(), 400);
        for (i, line) in lines.iter().enumerate() {
            assert!(line.starts_with('['), "{line}");
            assert!(
                line.contains(&format!("Segment {i:03} ")),
                "Zeile {i}: {line}"
            );
        }
        assert!(
            lines[0].contains("Ich:") && lines[1].contains("Gegenseite:"),
            "{}",
            lines[..2].join("|")
        );
        assert!(
            joined.contains("Größe und Maßnahme"),
            "Umlaute gehen unversehrt durch"
        );
    }

    #[test]
    fn a_short_transcript_is_one_page_with_speaker_labels() {
        let fx = Fx::new();
        let page = fx.tool_json("get_transcript", json!({ "id": fx.a }));
        assert_eq!(page["next_cursor"], Value::Null);
        assert_eq!(page["cursor"], 0);
        let text = page["text"].as_str().unwrap();
        assert_eq!(
            text,
            "[00:00] Ich: Wir starten mit dem Quartalsbericht.\n[00:05] Gegenseite: Der Umsatz liegt bei 1,2 Millionen Euro.\n"
        );
        assert_eq!(
            page["total_chars"].as_u64().unwrap(),
            text.chars().count() as u64
        );
        // Ein Cursor hinter dem Ende oder Unsinn ist ein Fehler, am Ende (=total) eine leere Seite.
        let total = page["total_chars"].as_u64().unwrap();
        let end = fx.tool_json("get_transcript", json!({ "id": fx.a, "cursor": total }));
        assert_eq!(
            (end["text"].as_str(), end["next_cursor"].is_null()),
            (Some(""), true)
        );
        for bad in [
            json!({ "id": fx.a, "cursor": total + 1 }),
            json!({ "id": fx.a, "cursor": -1 }),
            json!({ "id": fx.a, "cursor": "abc" }),
            json!({ "id": fx.a, "cursor": [1] }),
        ] {
            assert!(fx.tool("get_transcript", bad.clone()).1, "{bad}");
        }
        // Als Text uebergebene Zahl (mancher Client) geht.
        assert!(
            !fx.tool("get_transcript", json!({ "id": fx.a, "cursor": "0" }))
                .1
        );
    }

    #[test]
    fn the_transcript_is_refused_for_deleted_unfinished_and_unreleased_meetings() {
        let fx = Fx::new();
        for id in [fx.deleted.as_str(), fx.recording.as_str(), "01GIBTESNICHT"] {
            let (text, is_error) = fx.tool("get_transcript", json!({ "id": id }));
            assert!(is_error, "{id}: {text}");
            assert!(
                !text.contains("Nebelhorn") && !text.contains("Sonnenaufgang"),
                "{text}"
            );
        }
        fx.settings(true, false);
        let (text, is_error) = fx.tool("get_transcript", json!({ "id": fx.a }));
        assert!(is_error && text.contains("nicht freigegeben"), "{text}");
        assert!(!text.contains("Quartalsbericht"));
        // Alles andere geht weiter.
        assert!(!fx.tool("get_meeting", json!({ "id": fx.a })).1);
        assert!(!fx.tool("list_meetings", json!({})).1);
    }

    #[test]
    fn a_read_only_store_cannot_write_and_leaves_the_file_untouched() {
        let fx = Fx::new();
        let db = fx.db_path();
        let before = std::fs::read(&db).unwrap();
        let ro = MeetingStore::open_read_only(&db, Duration::from_millis(100)).unwrap();
        assert!(ro.get_meeting(&fx.a).unwrap().is_some(), "lesen geht");
        let writes: Vec<(&str, anyhow::Result<()>)> = vec![
            ("set_title", ro.set_title(&fx.a, "Überschrieben")),
            (
                "create_meeting",
                ro.create_meeting(
                    "Neu",
                    crate::managers::meetings::store::MeetingSource::Live,
                    None,
                )
                .map(|_| ()),
            ),
            ("soft_delete", ro.soft_delete_meeting(&fx.a).map(|_| ())),
            ("folder_save", ro.folder_save(None, "Neu", None).map(|_| ())),
        ];
        for (name, result) in writes {
            let err = result.expect_err(name).to_string().to_lowercase();
            assert!(
                err.contains("readonly") || err.contains("read-only"),
                "{name}: {err}"
            );
        }
        assert_eq!(
            std::fs::read(&db).unwrap(),
            before,
            "die Datei wurde veraendert"
        );
        assert_eq!(
            fx.store.get_meeting(&fx.a).unwrap().unwrap().title,
            "Jour fixe Vertrieb"
        );
        // Und ueber den Server: kein Werkzeug schreibt (Zaehler vor/nach).
        let count = |sql: &str| -> i64 {
            rusqlite::Connection::open(&db)
                .unwrap()
                .query_row(sql, [], |r| r.get(0))
                .unwrap()
        };
        let rows = count("SELECT (SELECT COUNT(*) FROM meetings) + (SELECT COUNT(*) FROM meeting_documents) + (SELECT COUNT(*) FROM chat_threads)");
        for (name, args) in [
            ("list_meetings", json!({})),
            ("search_meetings", json!({ "query": "Umsatz" })),
            ("get_meeting", json!({ "id": fx.a })),
            ("get_transcript", json!({ "id": fx.a })),
        ] {
            fx.tool(name, args);
        }
        assert_eq!(
            std::fs::read(&db).unwrap(),
            before,
            "ein Werkzeug hat geschrieben"
        );
        assert_eq!(count("SELECT (SELECT COUNT(*) FROM meetings) + (SELECT COUNT(*) FROM meeting_documents) + (SELECT COUNT(*) FROM chat_threads)"), rows);
    }

    #[test]
    fn a_different_schema_is_reported_and_never_migrated() {
        let fx = Fx::new();
        let db = fx.db_path();
        let set_version = |v: i64| {
            rusqlite::Connection::open(&db)
                .unwrap()
                .execute_batch(&format!("PRAGMA user_version = {v}"))
                .unwrap();
        };
        set_version(99);
        let before = std::fs::read(&db).unwrap();
        let (text, is_error) = fx.tool("list_meetings", json!({}));
        assert!(is_error && text.contains("neu starten"), "{text}");
        assert!(fx.tool("get_meeting", json!({ "id": fx.a })).1);
        assert_eq!(
            std::fs::read(&db).unwrap(),
            before,
            "der Server hat migriert"
        );

        set_version(2);
        let before = std::fs::read(&db).unwrap();
        let (text, is_error) = fx.tool("search_meetings", json!({ "query": "Umsatz" }));
        assert!(is_error && text.contains("einmal starten"), "{text}");
        assert_eq!(
            std::fs::read(&db).unwrap(),
            before,
            "der Server hat migriert"
        );
    }

    #[test]
    fn a_missing_database_gives_an_empty_list_with_a_note() {
        let dir = tempfile::tempdir().unwrap();
        write_settings(dir.path(), true, true);
        let fx = Fx {
            server: Server::new(config_in(dir.path())),
            ..Fx::new()
        };
        let list = fx.tool_json("list_meetings", json!({}));
        assert_eq!((list["returned"].as_u64(), ids(&list).len()), (Some(0), 0));
        assert!(
            list["note"].as_str().unwrap().contains("datenbank"),
            "{list}"
        );
        let search = fx.tool_json("search_meetings", json!({ "query": "Umsatz" }));
        assert_eq!(search["returned"], 0);
        assert!(fx.tool("get_meeting", json!({ "id": "01X" })).1);
        assert!(fx.tool("get_transcript", json!({ "id": "01X" })).1);
        assert!(
            !dir.path().join("meetings").exists(),
            "der Server legt nichts an"
        );
    }

    #[test]
    fn a_locked_database_answers_with_a_retry_hint_and_recovers() {
        let fx = Fx::new();
        // Die Datenbank steht im WAL-Modus (G8): ein gewoehnlicher Schreiber, auch
        // mit BEGIN EXCLUSIVE, sperrt Leser nicht mehr aus (das ist der Gewinn).
        // Eine Sperre, an der der Leser scheitert, haelt nur ein Prozess im
        // exklusiven Sperrmodus: er nimmt die Datei, bis die Verbindung endet.
        let writer = rusqlite::Connection::open(fx.db_path()).unwrap();
        writer
            .pragma_update(None, "locking_mode", "EXCLUSIVE")
            .unwrap();
        writer.execute_batch("BEGIN EXCLUSIVE").unwrap();
        let started = std::time::Instant::now();
        for (name, args) in [
            ("list_meetings", json!({})),
            ("search_meetings", json!({ "query": "Umsatz" })),
            ("get_meeting", json!({ "id": fx.a })),
        ] {
            let (text, is_error) = fx.tool(name, args);
            assert!(is_error && text.contains("gesperrt"), "{name}: {text}");
        }
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "der Server wartet ohne Ende"
        );
        writer.execute_batch("ROLLBACK").unwrap();
        drop(writer); // der Sperrmodus endet mit der Verbindung
        assert!(
            !fx.tool("list_meetings", json!({})).1,
            "nach der Sperre geht es wieder"
        );
    }

    #[test]
    fn every_stdout_line_is_valid_json() {
        let fx = Fx::new();
        let mut input: Vec<u8> = Vec::new();
        fn push(input: &mut Vec<u8>, line: &str) {
            input.extend_from_slice(line.as_bytes());
            input.push(b'\n');
        }
        push(
            &mut input,
            r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25"}}"#,
        );
        push(
            &mut input,
            r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
        );
        push(&mut input, r#"{"jsonrpc":"2.0","id":2,"method":"ping"}"#);
        push(
            &mut input,
            r#"{"jsonrpc":"2.0","id":3,"method":"tools/list"}"#,
        );
        push(&mut input, &json!({"jsonrpc":"2.0","id":"a","method":"tools/call","params":{"name":"search_meetings","arguments":{"query":"Größe Maßnahme Ünïcödé"}}}).to_string());
        push(&mut input, &json!({"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"get_transcript","arguments":{"id": fx.long}}}).to_string());
        push(&mut input, &json!({"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"get_meeting","arguments":{"id": fx.a}}}).to_string());
        push(&mut input, &json!({"jsonrpc":"2.0","id":6,"method":"tools/call","params":{"name":"list_meetings","arguments":{"folder":"Zeile1\nZeile2"}}}).to_string());
        push(&mut input, "{kaputt");
        push(&mut input, r#"[{"jsonrpc":"2.0","id":1,"method":"ping"}]"#);
        push(
            &mut input,
            r#"{"jsonrpc":"2.0","id":7,"method":"gibt/es/nicht"}"#,
        );
        push(&mut input, "");
        push(&mut input, &"x".repeat(MAX_LINE_BYTES + 10));
        input.extend_from_slice(&[0xff, 0xfe, b'\n']);
        push(&mut input, r#"{"jsonrpc":"2.0","id":8,"method":"ping"}"#);
        // Die letzte Zeile ohne Zeilenende.
        input.extend_from_slice(br#"{"jsonrpc":"2.0","id":9,"method":"ping"}"#);

        let mut output: Vec<u8> = Vec::new();
        serve(&fx.server, Cursor::new(input), &mut output).unwrap();
        let text = String::from_utf8(output).expect("stdout ist UTF-8");
        assert!(text.ends_with('\n'));
        assert!(!text.contains('\r'));
        let mut ids_seen: Vec<Value> = Vec::new();
        let mut lines = 0;
        for line in text.lines() {
            lines += 1;
            let value: Value = serde_json::from_str(line)
                .unwrap_or_else(|e| panic!("keine JSON-Zeile ({e}): {line:.200}"));
            assert_eq!(value["jsonrpc"], "2.0");
            assert!(
                value.get("result").is_some() ^ value.get("error").is_some(),
                "{line:.200}"
            );
            ids_seen.push(value["id"].clone());
        }
        // 10 Anfragen mit id + kaputt + Batch + zu lange Zeile + Nicht-UTF-8 = 14; die
        // Notification und die Leerzeile bekommen keine Antwort.
        assert_eq!(lines, 14, "{ids_seen:?}");
        assert_eq!(ids_seen[0], json!(1));
        assert_eq!(ids_seen[3], json!("a"));
        assert_eq!(ids_seen[12], json!(8));
        assert_eq!(ids_seen[13], json!(9));
    }

    #[test]
    fn an_overlong_line_is_dropped_and_the_next_request_still_works() {
        let fx = Fx::new();
        let mut input = "y".repeat(2 * MAX_LINE_BYTES).into_bytes();
        input.extend_from_slice(b"\n{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"ping\"}\n");
        let mut output = Vec::new();
        serve(&fx.server, Cursor::new(input), &mut output).unwrap();
        let lines: Vec<Value> = String::from_utf8(output)
            .unwrap()
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        assert_eq!(lines.len(), 2);
        assert_eq!(error_code(&lines[0]), INVALID_REQUEST);
        assert_eq!(lines[1]["result"], json!({}));
    }

    #[test]
    fn serve_ends_on_eof_and_survives_a_dead_client() {
        let fx = Fx::new();
        serve(&fx.server, Cursor::new(Vec::new()), Vec::new()).unwrap();
        assert_eq!(exit_code(Ok(())), 0);

        struct DeadPipe;
        impl Write for DeadPipe {
            fn write(&mut self, _: &[u8]) -> io::Result<usize> {
                Err(io::Error::from(io::ErrorKind::BrokenPipe))
            }
            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }
        let input = Cursor::new(br#"{"jsonrpc":"2.0","id":1,"method":"ping"}"#.to_vec());
        let err = serve(&fx.server, input, DeadPipe).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::BrokenPipe);
        assert_eq!(
            exit_code(Err(err)),
            0,
            "eine verschwundene Gegenstelle ist kein Fehler"
        );
        assert_eq!(
            exit_code(Err(io::Error::from(io::ErrorKind::PermissionDenied))),
            1
        );
    }

    #[test]
    fn the_data_dir_follows_override_then_portable_then_the_platform() {
        let platform = Some(PathBuf::from("C:/Users/x/AppData/Roaming"));
        assert_eq!(
            resolve_data_dir(
                Some("C:/sandbox"),
                Some(PathBuf::from("D:/p/Data")),
                platform.clone()
            ),
            PathBuf::from("C:/sandbox")
        );
        assert_eq!(
            resolve_data_dir(
                Some("  "),
                Some(PathBuf::from("D:/p/Data")),
                platform.clone()
            ),
            PathBuf::from("D:/p/Data")
        );
        assert_eq!(
            resolve_data_dir(None, None, platform),
            PathBuf::from("C:/Users/x/AppData/Roaming").join("de.wolffappliedai.localvoiceai")
        );
        assert_eq!(
            resolve_data_dir(None, None, None),
            PathBuf::from("lva-no-appdata-dir")
        );
    }

    /// G1 (#70): ein leerer Eintrag (nur Notizen, noch keine Quelle) bringt den
    /// Server nicht zum Stolpern: er steht in der Liste, `get_meeting` nennt ihn
    /// leer bzw. zeigt die Notizen, `get_transcript` und die Suche melden keinen Fehler.
    #[test]
    fn an_empty_entry_is_listed_and_readable_without_errors() {
        use crate::managers::meetings::notes::model::{NoteBlock, NoteBlockKind};
        let fx = Fx::new();
        let empty = fx.store.create_empty_meeting("Neue Besprechung", None).unwrap();
        let list = fx.tool_json("list_meetings", json!({}));
        assert!(list["meetings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|m| m["id"] == empty.id.as_str()));
        let (text, is_error) = fx.tool("get_meeting", json!({ "id": empty.id }));
        assert!(!is_error, "{text}");
        assert!(text.contains("keinen Inhalt"), "{text}");
        fx.store
            .save_notes(
                &empty.id,
                &[NoteBlock {
                    id: "N1".into(),
                    kind: NoteBlockKind::Bullet,
                    text: "Wichtige Vorab-Notiz".into(),
                    at_ms: None,
                    checked: false,
                }],
                0,
            )
            .unwrap();
        let (text, is_error) = fx.tool("get_meeting", json!({ "id": empty.id }));
        assert!(!is_error && text.contains("Wichtige Vorab-Notiz"), "{text}");
        let (text, is_error) = fx.tool("get_transcript", json!({ "id": empty.id }));
        assert!(!is_error, "{text}");
        let (text, is_error) = fx.tool("search_meetings", json!({ "query": "Vorab" }));
        assert!(!is_error, "{text}");
    }
}
