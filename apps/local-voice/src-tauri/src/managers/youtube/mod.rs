//! YouTube als Quelle (Goal „Integrationen“, Issue #66 / Feature #65, Paket A2).
//!
//! Ein Link wird zu einer Besprechung mit Quelle `youtube`: Titel und Kanal
//! kommen einmalig ueber oEmbed (kein API-Schluessel), angesehen wird das Video
//! im OFFIZIELLEN Einbett-Player der Oberflaeche (E1; Werbung bleibt unveraendert,
//! kein Werbeblocker). Audio und Untertitel holen erst A3, und nur ueber ein vom
//! Nutzer selbst installiertes `yt-dlp` hinter dem Schalter „privat“.
//!
//! Bausteine:
//! - `link`: Link-Normalisierung (watch, youtu.be, shorts, embed, live, m., music.,
//!   nocookie; `t=`/`si=` werden verstanden bzw. verworfen), Playlist- und
//!   Kanal-Links werden mit eigener Meldung abgewiesen.
//! - `oembed`: der EINE Netzzugriff dieses Pakets (Titel, Kanal, Adresse des
//!   Vorschaubilds), mit Zeitlimit, Groessenlimit und ohne Umleitungen.
//! - `source`: Register-Eintrag, Tor, Audit, Anlegen der Besprechung (eine
//!   Transaktion) samt Provenienz „Quelle YouTube“.
//! - `tool`: Erkennung eines selbst installierten `yt-dlp` (Version), Kindprozess
//!   ueber `process_guard`, mit Zeitlimit. Nichts wird gebuendelt oder geladen.
//!
//! Datenschutz (QG5, Offline-Pfad): Netzverkehr gibt es nur fuer den bewussten
//! Schritt „Link einfuegen“ (ein oEmbed-Abruf, im Audit) und fuer das Abspielen,
//! das der Nutzer in der Oberflaeche ausdruecklich startet. Beim Oeffnen einer
//! Besprechung, in der Liste und beim Start der App verbindet sich nichts.
//!
//! Fehlerfaelle (A2) und ihre Absicherung:
//! - **Zwei gleiche Anfragen gleichzeitig** (Doppelklick, Einfuegen plus Knopf):
//!   je Video-ID laeuft hoechstens ein Anlegen; das zweite meldet `youtube_busy`,
//!   es entsteht genau eine Besprechung (`source::tests::a_second_add_of_the_same_video_*`).
//! - **Abbruch mitten im Vorgang**: Netz weg, Zeitlimit oder Fehlerantwort lassen
//!   NICHTS zurueck ausser dem Audit-Eintrag `error` (`source::tests::a_failed_fetch_*`).
//!   Besprechung, Projektzuordnung und Provenienz entstehen in EINER Transaktion
//!   (`source::tests::an_unknown_project_leaves_no_meeting_behind`, `…_rolls_back`).
//!   Bricht die App zwischen Audit `pending` und Ergebnis ab, bleibt der Eintrag
//!   `pending` (Semantik des Tors aus A1: sichtbar, die Aktion kann gelaufen sein).
//! - **Voller Datentraeger / gesperrte Datenbank**: schlaegt das Audit VOR dem
//!   Abruf fehl, geht kein Byte ins Netz (fail closed,
//!   `source::tests::without_a_writable_audit_nothing_goes_to_the_network`);
//!   schlaegt das Anlegen fehl, ist nichts Halbes da (Transaktion).
//! - **Fehlendes Geraet / Audio-Callback**: nicht beteiligt (keine Aufnahme,
//!   kein Audiopfad); entfaellt.
//! - **Kindprozess haengt oder stirbt** (`yt-dlp --version`): Zeitlimit, dann
//!   Beenden ueber das Prozess-Handle samt Job-Objekt (KILL_ON_JOB_CLOSE); Abbruch
//!   mit Fehlercode, nie ein Haenger (`tool::tests::a_hanging_tool_is_killed_*`).
//! - **Feindliche Eingaben**: Lookalike-Hosts, Zugangsdaten und Ports im Link,
//!   Steuerzeichen, Laenge (`link::tests`); riesige oder falsch typisierte
//!   oEmbed-Antworten, fremde Hosts in Adressen der Antwort (`oembed::tests`).
//! - **Migrationen**: keine. Die Quelle steht in `meetings.source` (TEXT, ohne
//!   CHECK) und `meetings.metadata_json` (vorhanden); Altdaten bleiben unberuehrt.
//! - **Speicher/RAM**: der einzige Kindprozess ist `yt-dlp --version` (Sekundenbruchteile,
//!   Job-Objekt mit RAM-Deckel 1 GB und CPU-Deckel). Bei vollem RAM scheitert der
//!   Start dieses Prozesses mit Fehlercode; die App bleibt bedienbar.

pub mod link;
pub mod oembed;
pub mod source;
pub mod tool;

#[cfg(test)]
pub(crate) mod test_support;

pub use link::{normalize_link, LinkError};

/// Der Wert von `meetings.source` fuer Besprechungen aus einem YouTube-Link.
pub const SOURCE_KIND: &str = "youtube";
/// Schluessel in `meetings.metadata_json` mit den Angaben zur Quelle.
pub const METADATA_KEY: &str = "youtube";
/// Feste Kennung der YouTube-Integration im Register (eine je App).
pub const INTEGRATION_ID: &str = "youtube";

/// Fehler rund um die YouTube-Quelle. `code()` ist der maschinenlesbare Teil
/// (die Oberflaeche uebersetzt ihn), `Display` Klartext auf Deutsch.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum YoutubeError {
    Link(LinkError),
    /// Die Integration ist ausgeschaltet oder das Tor verweigert (Text des Tores).
    Disabled(String),
    /// Zu diesem Video laeuft gerade schon ein Anlegen.
    Busy,
    /// Das Projekt gibt es nicht (mehr).
    Project,
    /// oEmbed: nicht gefunden, privat, geloescht oder nicht einbettbar.
    Unavailable,
    RateLimited,
    Timeout,
    /// Keine Verbindung (DNS, TLS, Proxy ...), Text ohne Adresse.
    Network(String),
    /// Unerwarteter HTTP-Status.
    Http(u16),
    /// Antwort nicht auswertbar (kein JSON, zu gross, falsche Art).
    BadResponse(String),
    /// Datenbank (gesperrt, voll, defekt).
    Store(String),
}

impl YoutubeError {
    pub fn code(&self) -> &'static str {
        match self {
            YoutubeError::Link(e) => e.code(),
            YoutubeError::Disabled(_) => "youtube_disabled",
            YoutubeError::Busy => "youtube_busy",
            YoutubeError::Project => "youtube_project_not_found",
            YoutubeError::Unavailable => "youtube_unavailable",
            YoutubeError::RateLimited => "youtube_rate_limited",
            YoutubeError::Timeout => "youtube_timeout",
            YoutubeError::Network(_) => "youtube_network",
            YoutubeError::Http(_) => "youtube_http",
            YoutubeError::BadResponse(_) => "youtube_bad_response",
            YoutubeError::Store(_) => "youtube_store_failed",
        }
    }

    /// Die Zeichenkette, die ein Command an die Oberflaeche gibt: der Code,
    /// bei Fehlern mit Zusatz `code: Zusatz` (die Oberflaeche nimmt den Teil vor
    /// dem ersten Doppelpunkt).
    pub fn to_command_error(&self) -> String {
        match self {
            YoutubeError::Http(status) => format!("{}: {status}", self.code()),
            YoutubeError::Network(m) | YoutubeError::Store(m) | YoutubeError::BadResponse(m) => {
                format!("{}: {m}", self.code())
            }
            YoutubeError::Disabled(m) => format!("{}: {m}", self.code()),
            _ => self.code().to_string(),
        }
    }
}

impl std::fmt::Display for YoutubeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            YoutubeError::Link(e) => write!(f, "{e}"),
            YoutubeError::Disabled(m) => write!(f, "{m}"),
            YoutubeError::Busy => write!(f, "Dieses Video wird gerade angelegt."),
            YoutubeError::Project => write!(f, "Das gewählte Projekt gibt es nicht mehr."),
            YoutubeError::Unavailable => write!(
                f,
                "Das Video ist nicht verfügbar: privat, gelöscht oder nicht zum Einbetten freigegeben."
            ),
            YoutubeError::RateLimited => write!(
                f,
                "YouTube antwortet gerade nicht auf weitere Anfragen. Bitte später erneut versuchen."
            ),
            YoutubeError::Timeout => write!(f, "YouTube hat nicht rechtzeitig geantwortet."),
            YoutubeError::Network(m) => write!(f, "Keine Verbindung zu YouTube: {m}"),
            YoutubeError::Http(s) => write!(f, "YouTube antwortete mit Status {s}."),
            YoutubeError::BadResponse(m) => {
                write!(f, "Die Antwort von YouTube war nicht auswertbar: {m}")
            }
            YoutubeError::Store(m) => write!(f, "Speicherfehler: {m}"),
        }
    }
}

impl std::error::Error for YoutubeError {}

impl From<LinkError> for YoutubeError {
    fn from(e: LinkError) -> Self {
        YoutubeError::Link(e)
    }
}

impl From<rusqlite::Error> for YoutubeError {
    fn from(e: rusqlite::Error) -> Self {
        YoutubeError::Store(e.to_string())
    }
}

impl From<crate::managers::integrations::model::IntegrationError> for YoutubeError {
    fn from(e: crate::managers::integrations::model::IntegrationError) -> Self {
        YoutubeError::Store(e.to_string())
    }
}

#[cfg(test)]
mod csp_tests {
    /// Die Hosts, die der Einbett-Player braucht: genau diese, sonst keine.
    /// `frame-src`: der Player-Rahmen; `script-src`: die IFrame Player API samt
    /// Widget-Skript. Gemessen mit dem echten Player: nur diese beiden Hosts fragt
    /// die Seite selbst an; alles Weitere (Bilder, Stil, Daten) gehoert dem Rahmen
    /// und seiner eigenen Richtlinie.
    const CSP_FRAME_HOSTS: [&str; 1] = ["https://www.youtube-nocookie.com"];
    const CSP_SCRIPT_HOSTS: [&str; 1] = ["https://www.youtube.com"];

    /// Stand beim Schreiben von A2: `tauri.conf.json` setzt `csp: null` -- die App
    /// hat KEINE Content-Security-Policy, der Player braucht also keine Erweiterung.
    /// Dieser Test haelt die Zusage fuer den Tag, an dem eine CSP eingefuehrt wird:
    /// dann muss sie den Player zulassen, und zwar NUR ueber diese Hosts (kein
    /// Platzhalter, kein pauschales `https:`).
    #[test]
    fn a_future_csp_must_allow_the_player_hosts_and_nothing_wider() {
        let conf: serde_json::Value =
            serde_json::from_str(include_str!("../../../tauri.conf.json")).unwrap();
        let csp = &conf["app"]["security"]["csp"];
        if csp.is_null() {
            return;
        }
        let directives: std::collections::HashMap<String, Vec<String>> = match csp {
            serde_json::Value::String(text) => text
                .split(';')
                .filter_map(|d| {
                    let mut parts = d.split_whitespace();
                    let name = parts.next()?.to_string();
                    Some((name, parts.map(str::to_string).collect()))
                })
                .collect(),
            serde_json::Value::Object(map) => map
                .iter()
                .map(|(k, v)| {
                    let values = match v {
                        serde_json::Value::String(s) => {
                            s.split_whitespace().map(str::to_string).collect()
                        }
                        serde_json::Value::Array(a) => a
                            .iter()
                            .filter_map(|x| x.as_str().map(str::to_string))
                            .collect(),
                        _ => Vec::new(),
                    };
                    (k.clone(), values)
                })
                .collect(),
            other => panic!("unerwartete CSP-Form: {other}"),
        };
        let effective = |name: &str| -> Vec<String> {
            directives
                .get(name)
                .or_else(|| directives.get("default-src"))
                .cloned()
                .unwrap_or_default()
        };
        for (directive, hosts) in [
            ("frame-src", CSP_FRAME_HOSTS),
            ("script-src", CSP_SCRIPT_HOSTS),
        ] {
            let sources = effective(directive);
            for host in hosts {
                assert!(
                    sources.iter().any(|s| s == host),
                    "{directive} muss {host} erlauben: {sources:?}"
                );
            }
            for wide in ["*", "https:", "http:", "https://*", "data:"] {
                assert!(
                    !sources.iter().any(|s| s == wide),
                    "{directive} darf nicht pauschal {wide} erlauben: {sources:?}"
                );
            }
        }
    }
}
