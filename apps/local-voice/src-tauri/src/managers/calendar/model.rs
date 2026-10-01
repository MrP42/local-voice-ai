//! Datentypen des lokalen Kalenders (M5, `entwurf/m5-m6-kalender-export.md` §5).
//!
//! Alle Zeiten sind Millisekunden UTC (`i64`). Die Typen sind reine Daten ohne
//! Verhalten; Parser, Abruf und Speicher liegen in den Nachbardateien.

use serde::{Deserialize, Serialize};
use specta::Type;

/// Art einer Kalenderquelle. `Graph` ist fuer P5f reserviert.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "lowercase")]
pub enum CalendarKind {
    Ics,
    Graph,
}

impl CalendarKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            CalendarKind::Ics => "ics",
            CalendarKind::Graph => "graph",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "ics" => Some(CalendarKind::Ics),
            "graph" => Some(CalendarKind::Graph),
            _ => None,
        }
    }
}

/// Eine Kalenderquelle, wie die Oberflaeche sie zeigt. Die ICS-Adresse ist
/// ein Geheimnis (Lesezugriff auf den ganzen Kalender) und steht NIE hier:
/// `account_hint` traegt nur den Host bzw. das Benutzerkonto.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Type)]
pub struct CalendarSource {
    pub id: String,
    pub kind: CalendarKind,
    pub label: String,
    pub account_hint: Option<String>,
    pub enabled: bool,
    pub has_attendee_data: bool,
    pub last_sync_at: Option<i64>,
    pub last_ok_at: Option<i64>,
    pub last_error: Option<String>,
    pub event_count: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct Attendee {
    /// Klein geschrieben, ohne `mailto:`; `None`, wenn die Quelle keine
    /// Adresse liefert (nur ein Name).
    pub email: Option<String>,
    pub name: Option<String>,
    pub organizer: bool,
    /// Wird erst von der Personen-/Einstellungsschicht gesetzt (P5b/P5d).
    pub is_self: bool,
    pub partstat: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Type)]
pub struct CalEvent {
    /// `source:uid:start_ms` - stabil ueber Abrufe, damit `reminded_at` und
    /// `dismissed_at` einen erneuten Abruf ueberleben.
    pub key: String,
    pub source_id: String,
    pub uid: String,
    pub title: String,
    pub starts_at: i64,
    pub ends_at: i64,
    pub all_day: bool,
    pub cancelled: bool,
    pub location: Option<String>,
    pub join_url: Option<String>,
    pub description: Option<String>,
    pub attendees: Vec<Attendee>,
}

/// Schluessel eines Termins: Quelle, Serien-/Termin-UID und Beginn in ms UTC.
pub fn event_key(source_id: &str, uid: &str, start_ms: i64) -> String {
    format!("{source_id}:{uid}:{start_ms}")
}

/// Verknuepfung Besprechung <-> Termin. Der Schnappschuss (`uid`, `event_start`,
/// `event_title`) bleibt erhalten, auch wenn Termin oder Quelle aus dem Cache
/// verschwinden (`event_key`/`source_id` werden dann `None`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct MeetingCalendarLink {
    pub meeting_id: String,
    pub event_key: Option<String>,
    pub source_id: Option<String>,
    pub uid: String,
    pub event_start: i64,
    pub event_title: String,
    /// `prompt`, `auto` oder `manual`.
    pub linked_by: String,
    pub created_at: i64,
}

/// Fehler beim Abruf und beim Lesen einer Kalenderquelle. `Display` liefert
/// den Klartext fuer die rote Quellenzeile (Deutsch). Keine Variante traegt die
/// Adresse: sie ist ein Geheimnis und darf in keinem Log und keiner Meldung
/// stehen.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CalendarError {
    Http(u16),
    NotCalendar,
    TooLarge,
    Timeout,
    Network(String),
    /// Anmeldung noetig (Graph) bzw. Geheimnis nicht lesbar.
    Auth,
    Parse(String),
}

impl std::fmt::Display for CalendarError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CalendarError::Http(code @ (401 | 403)) => write!(
                f,
                "Zugriff verweigert (HTTP {code}): Die Adresse wurde widerrufen oder das Veröffentlichen ist gesperrt."
            ),
            CalendarError::Http(code @ (404 | 410)) => write!(
                f,
                "Adresse nicht gefunden (HTTP {code}): Der Kalender ist nicht mehr veröffentlicht."
            ),
            CalendarError::Http(code) => write!(f, "Der Server antwortet mit HTTP {code}."),
            CalendarError::NotCalendar => write!(
                f,
                "Die Adresse liefert keinen Kalender (ICS), vermutlich eine Anmelde- oder Fehlerseite."
            ),
            CalendarError::TooLarge => write!(f, "Die Kalenderdatei ist größer als 20 MB."),
            CalendarError::Timeout => write!(f, "Zeitüberschreitung beim Abruf (30 s)."),
            CalendarError::Network(msg) => write!(f, "Keine Verbindung: {msg}"),
            CalendarError::Auth => write!(f, "Anmeldung nötig: Adresse neu eingeben."),
            CalendarError::Parse(msg) => write!(f, "Der Kalender ist nicht lesbar: {msg}"),
        }
    }
}

impl std::error::Error for CalendarError {}
