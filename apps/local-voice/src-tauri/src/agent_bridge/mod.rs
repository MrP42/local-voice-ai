//! Agentenbruecke (Goal „Integrationen“, Issue #66, Paket A7): externe KI-Agenten
//! steuern die laufende App ueber eine Named Pipe, mit Token, Rechten je
//! Werkzeug, Freigaben und Audit. Dazu das Kommandozeilenwerkzeug
//! `local-voice-ai.exe ctl ...` (`ctl`).
//!
//! ```text
//! Agent / MCP-Proxy / ctl --(Named Pipe, nur aktueller Benutzer)--> server --> bridge --> gate (A1)
//!                                                                                 |        |-- Audit
//!                                                                                 |        `-- Freigabe (nur die App-Oberflaeche entscheidet)
//!                                                                                 `--> ToolHandler (A8: Aufnahme, Datei, Vorlesen, YouTube ...)
//! ```
//!
//! Bausteine:
//! - `schema`: Migration Index 8 (Zugaenge, Werkzeugrechte, Freigabe-Zuordnung).
//! - `clients`: Zugaenge und Token (nur als SHA-256 gespeichert), Werkzeugrechte je Zugang.
//! - `catalog`: die EINE Tabelle Werkzeug -> Faehigkeit (Recht), `ToolHandler` und Registry.
//! - `bridge`: die Entscheidungen (anmelden, Liste, Aufruf, Freigabestand) ueber dem Tor aus A1.
//! - `limits`: Aufruf-Obergrenzen je Zugang (QG5).
//! - `protocol` / `server`: JSON-Zeilen, eine Verbindung = eine Aufgabe, Wartezeit auf Freigaben.
//! - `pipe`: Windows-Pipe (DACL nur aktueller Benutzer, keine Fernzugriffe, Pruefung des Gegenuebers).
//! - `client` / `ctl`: Gegenstelle fuer `ctl` und den MCP-Proxy (A8); Exit-Codes.
//! - `view`: Ansichten fuer die Tauri-Commands (`commands/agent_bridge.rs`).
//! - `runtime`: Start mit der App, Zustand fuer die Oberflaeche, headless Sandbox-Instanz.
//! - `test_tools`: Echo-Werkzeuge nur fuer die headless Sandbox-Instanz.
//!
//! # Rechte (E3)
//! Wirksamer Modus eines Aufrufs = `grants::effective_mode` (A1) fuer die Agent-Integration
//! des Zugangs, Aufrufer `agent_external`, mit dem Recht des Zugangs fuer das Werkzeug als
//! `tool_mode`: das Strengere gewinnt. Beides steht standardmaessig auf „aus“ (externe
//! Agenten aus; ein neuer Zugang hat kein Werkzeug). „aus“ -> das Werkzeug fehlt in
//! `tools/list` und ein Aufruf wird abgelehnt (Audit); „fragen“ -> Freigabe in der App; nach
//! `Config::approval_wait` ohne Antwort kommt `pending` mit Kennung zurueck und der Stand ist
//! mit `approval/status` abfragbar; „erlaubt“ -> ausfuehren. „Aufnahme starten“ ist nie
//! dauerhaft erlaubt (A1: `never_allow`). Die Entscheidung einer Freigabe gibt es NUR in der
//! Oberflaeche (A4, Tauri-Command), nie ueber die Pipe: das Protokoll kennt keine solche Methode.
//!
//! # Fehlerfaelle und ihre Absicherung (Tests in den Modulen)
//!
//! | # | Fehlerfall | Verhalten | Absicherung |
//! |---|---|---|---|
//! | 1 | Anderer Benutzer oder Fernrechner verbindet sich | Pipe-DACL erlaubt nur die SID des aktuellen Benutzers, Netzwerk-Anmeldungen ausdruecklich verboten, `PIPE_REJECT_REMOTE_CLIENTS`, Pruefung der Benutzer-SID des Gegenuebers je Verbindung | `pipe::tests::the_pipe_dacl_*`, `remote_clients_are_rejected_*`, `a_peer_with_another_sid_is_refused` |
//! | 2 | Ein anderer Prozess belegt den Pipe-Namen vor der App (Squatting) | erste Instanz nur mit `FILE_FLAG_FIRST_PIPE_INSTANCE`; die App meldet den Fehler, die Gegenstelle prueft die Benutzer-SID des Servers | `pipe::tests::a_second_first_instance_is_refused`, `client_refuses_a_server_of_another_user` |
//! | 3 | Token falsch, fehlt oder wurde zurueckgezogen | abgelehnt (`token_invalid`/`token_revoked`), Audit `denied`, nach drei Fehlversuchen schliesst die Verbindung; Fehlversuche insgesamt begrenzt | `bridge::tests::*token*`, `server::tests::*` |
//! | 4 | Token im Klartext auf der Platte | nur SHA-256 gespeichert, Test liest die Datenbankdatei | `clients::tests::the_token_is_stored_only_as_a_hash` |
//! | 5 | Zugang wird waehrend einer Verbindung zurueckgezogen oder das Recht auf „aus“ gesetzt | jede Anfrage prueft Zugang und Recht neu; `run_approved` prueft das Recht erneut | `server::tests::a_revoked_client_is_cut_off_*`, `bridge::tests::rights_withdrawn_after_approval_*` |
//! | 6 | Zwei Zugaenge oder Verbindungen fragen gleichzeitig dasselbe | eine Freigabe je (Zugang, Werkzeug, Argumente); eine Genehmigung gilt einmal und nur fuer diese Ausfuehrung | `bridge::tests::concurrent_identical_asks_*`, `an_approval_cannot_be_used_twice`, `another_clients_approval_is_unusable` |
//! | 7 | Der Agent bricht ab, waehrend die App auf die Freigabe wartet | Verbindungsende wird erkannt, nichts wird ausgefuehrt, die Freigabe bleibt offen und verfaellt | `server::tests::a_client_that_leaves_while_waiting_never_runs_the_tool` |
//! | 8 | Die App endet oder die Verbindung reisst mitten im Aufruf | Audit-Eintrag bleibt `pending` (A1) und ist sichtbar; die Gegenstelle bekommt Ende der Verbindung -> `ctl` Exit 2 | `ctl::tests::a_connection_lost_mid_call_is_exit_2`, `gate` (A1) |
//! | 9 | Datentraeger voll / Datenbank gesperrt (Audit nicht schreibbar) | Tor schlaegt fehl geschlossen: Aktion laeuft NICHT, Antwort `store_unavailable` | `bridge::tests::a_failing_audit_write_blocks_the_tool` |
//! | 10 | Werkzeug stuerzt ab (Panik) | `catch_unwind` -> `failed`, Audit `error`, Bruecke laeuft weiter | `bridge::tests::a_panicking_tool_is_contained` |
//! | 11 | App laeuft nicht (Pipe fehlt) | `ctl` Exit 2, keine Wartezeit | `ctl::tests::no_app_is_exit_2` |
//! | 12 | Flut: viele Verbindungen, viele Aufrufe, riesige Zeilen, Verbindung ohne Anmeldung | Obergrenze Verbindungen (8), Aufrufe je Minute je Zugang, Zeilen 1 MiB, Anmeldefrist 10 s, Leerlauf, Antwortgroesse; Verweigerungen werden im Audit zusammengefasst (A1) | `limits::tests`, `server::tests::*cap*`, `*timeout*`, `*too_long*` |
//! | 13 | Fehlerhafte Eingaben (kein JSON, falscher Typ, unbekannte Methode/Werkzeug, Argumente keine Objekte) | strukturierter Fehler, nie Panik, Verbindung bleibt (ausser Zeile zu lang) | `protocol::tests`, `server::tests::malformed_*` |
//! | 14 | Agent versucht, sich selbst Rechte zu geben oder eine Freigabe zu entscheiden | es gibt weder eine Methode noch ein Werkzeug dafuer; die Entscheidung liegt in den Tauri-Commands der Oberflaeche | `server::tests::there_is_no_way_to_decide_an_approval_over_the_pipe` |
//! | 15 | Migration auf bestehender Datenbank, Abbruch mitten im Schritt | nur CREATE, eine Transaktion; Altdaten unveraendert, bei Abbruch Stand wie vorher | `schema::tests::*`, `meetings::migration_chain::tests` |
//! | 16 | Kindprozess / Audio / fehlendes Geraet | die Bruecke startet keinen Kindprozess und beruehrt weder Audio-Callback noch Geraete; Aufnahme starten gehoert A8 und bleibt hinter dem Einwilligungsdialog | entfaellt (Begruendung); `grants::never_allow` (A1) |
//! | 17 | Voller Arbeitsspeicher | je Verbindung hoechstens eine Zeile (1 MiB) und eine Antwort (1 MiB) im Speicher, hoechstens 8 Verbindungen: unter 20 MiB; keine Modellstarts, daher weder `process_guard` noch RAM-Gate noetig | `server::tests::*cap*`, Obergrenzen als Konstanten |
//!
//! Was die Bruecke NICHT schuetzt: ein Agent mit freiem Shell-Zugriff unter demselben
//! Windows-Benutzer kann die Datenbankdatei direkt aendern. Die Rechte gelten gegenueber
//! Agenten, die ueber Pipe, MCP oder `ctl` sprechen.

pub mod bridge;
pub mod catalog;
pub mod client;
pub mod clients;
pub mod ctl;
pub mod limits;
pub mod pipe;
pub mod protocol;
pub mod runtime;
pub mod schema;
pub mod server;
pub mod test_tools;
#[cfg(test)]
pub(crate) mod testkit;
pub mod view;

use std::time::Duration;

/// Version des Pipe-Protokolls (`hello`/`status` melden sie).
pub const PROTOCOL_VERSION: u32 = 1;
/// Name der App im Protokoll.
pub const APP_NAME: &str = "local-voice-ai";

/// Einstellbare Grenzen. Die Vorgaben sind die Werte fuer den Betrieb; Tests setzen
/// kurze Zeiten.
#[derive(Clone, Debug)]
pub struct Config {
    /// So lange wartet ein Aufruf mit „fragen“ auf die Entscheidung (30 s), bevor `pending` kommt.
    pub approval_wait: Duration,
    /// Takt, in dem der Stand der Freigabe nachgesehen wird.
    pub poll_interval: Duration,
    /// Gleichzeitige Verbindungen.
    pub max_connections: usize,
    /// `tools/call` je Minute und Zugang.
    pub calls_per_minute: u32,
    /// Alle Anfragen je Minute und Zugang.
    pub requests_per_minute: u32,
    /// Fehlgeschlagene Anmeldungen je Minute insgesamt.
    pub auth_failures_per_minute: u32,
    /// Bis zur ersten Anfrage (Anmeldung) einer neuen Verbindung.
    pub handshake_timeout: Duration,
    /// Leerlauf einer nicht angemeldeten Verbindung.
    pub anonymous_idle_timeout: Duration,
    /// Leerlauf einer angemeldeten Verbindung.
    pub idle_timeout: Duration,
    /// Fehlversuche bei der Anmeldung, nach denen die Verbindung geschlossen wird.
    pub max_auth_failures_per_connection: u32,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            approval_wait: Duration::from_secs(30),
            poll_interval: Duration::from_millis(250),
            max_connections: 8,
            calls_per_minute: 60,
            requests_per_minute: 300,
            auth_failures_per_minute: 20,
            handshake_timeout: Duration::from_secs(10),
            anonymous_idle_timeout: Duration::from_secs(30),
            idle_timeout: Duration::from_secs(600),
            max_auth_failures_per_connection: 3,
        }
    }
}

/// Laengste Zeile in beide Richtungen (wie der MCP-Server).
pub const MAX_LINE_BYTES: usize = 1 << 20;
/// Laengste Antwort eines Werkzeugs (serialisiert).
pub const MAX_RESULT_BYTES: usize = 1 << 20;

#[cfg(test)]
mod tests {
    /// Verbindungen zur Meetings-Datenbank laufen nur ueber `meetings::store::open_connection`
    /// (G8: WAL und 30 s Wartezeit). Eine blanke `Connection::open` wartet nur 0 bis 5 s und
    /// verliert unter Last ein Schreiben (Audit, Freigabe, Zugang).
    #[test]
    fn production_code_opens_connections_only_through_open_connection() {
        let files: [(&str, &str); 14] = [
            ("bridge.rs", include_str!("bridge.rs")),
            ("catalog.rs", include_str!("catalog.rs")),
            ("client.rs", include_str!("client.rs")),
            ("clients.rs", include_str!("clients.rs")),
            ("ctl.rs", include_str!("ctl.rs")),
            ("limits.rs", include_str!("limits.rs")),
            ("pipe.rs", include_str!("pipe.rs")),
            ("protocol.rs", include_str!("protocol.rs")),
            ("runtime.rs", include_str!("runtime.rs")),
            ("schema.rs", include_str!("schema.rs")),
            ("server.rs", include_str!("server.rs")),
            ("view.rs", include_str!("view.rs")),
            ("test_tools.rs", include_str!("test_tools.rs")),
            (
                "commands/agent_bridge.rs",
                include_str!("../commands/agent_bridge.rs"),
            ),
        ];
        for (name, src) in files {
            let production = src.split("#[cfg(test)]").next().unwrap();
            assert!(
                !production.contains("Connection::open("),
                "{name}: Verbindungen nur ueber meetings::store::open_connection"
            );
        }
        // Der Weg der Bruecke (MeetingStore::get_connection) haengt das Zeitlimit an.
        let fx = crate::managers::integrations::test_support::Fx::new();
        let conn = fx.store.get_connection().unwrap();
        let ms: i64 = conn
            .query_row("PRAGMA busy_timeout", [], |r| r.get(0))
            .unwrap();
        assert_eq!(
            ms,
            crate::managers::meetings::store::BUSY_TIMEOUT.as_millis() as i64
        );
    }
}
