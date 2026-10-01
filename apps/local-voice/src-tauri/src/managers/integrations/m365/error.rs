//! Fehler des Microsoft-365-Kontos (A5).
//!
//! Jeder Fehler hat einen stabilen Code (`code()`), den die Oberflaeche uebersetzt,
//! und einen deutschen Klartext (`Display`). An die Oberflaeche und ins Audit geht
//! die Kurzform `code` oder `code|detail` (`wire()`): `detail` ist nie geheim
//! (Statuszahl, Microsoft-Fehlercode, Minuten), nie ein Token, nie eine Adresse.

use std::time::Duration;

use crate::managers::calendar::graph::GraphError;
use crate::managers::integrations::model::Capability;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum M365Error {
    /// Keine Client-ID eingetragen.
    NotConfigured,
    /// Keine Faehigkeit eingeschaltet (Anmelden ohne Scopes waere sinnlos).
    NoCapability,
    /// Diese Faehigkeit ist an diesem Konto nicht eingeschaltet.
    CapabilityOff(Capability),
    /// Token fehlt, ist abgelaufen oder widerrufen: neu anmelden.
    NeedsSignIn,
    /// Eine Faehigkeit wurde nach der Anmeldung eingeschaltet: Anmelden erweitert
    /// die Zustimmung um die fehlenden Scopes.
    NeedsConsent {
        missing: Vec<String>,
    },
    /// Microsoft oder der Mandant lehnt ab (403, Zustimmung verweigert).
    Denied(String),
    /// Termin, Pfad oder Konto nicht gefunden (404).
    NotFound(String),
    /// OneDrive voll (507 / `quotaLimitReached`).
    StorageFull,
    Throttled {
        retry_after: Duration,
    },
    Http {
        status: u16,
        code: String,
    },
    Network(String),
    Timeout,
    /// Die Aktion kann trotz des Fehlers ausgefuehrt worden sein (z. B. Mail: die
    /// Verbindung riss, nachdem die Anfrage gesendet war). Nie automatisch wiederholen.
    Uncertain(String),
    Parse(String),
    /// Eingabe nie gueltig (Empfaenger, Dateiname, Pfad, Groesse).
    Invalid(String),
    Cancelled,
    SignInTimeout,
    Browser(String),
    Listener(String),
    Config(String),
    /// Geheimnisspeicher oder Datei (voll, gesperrt, kaputt).
    Store(String),
    /// Zu wenig freier Arbeitsspeicher fuer einen grossen Upload.
    MemoryLow(String),
}

impl M365Error {
    /// Stabiler Code fuer die Oberflaeche (`integrations.m365.errors.<code>`).
    pub fn code(&self) -> &'static str {
        match self {
            M365Error::NotConfigured => "m365_not_configured",
            M365Error::NoCapability => "m365_no_capability",
            M365Error::CapabilityOff(_) => "m365_capability_off",
            M365Error::NeedsSignIn => "m365_needs_sign_in",
            M365Error::NeedsConsent { .. } => "m365_needs_consent",
            M365Error::Denied(_) => "m365_denied",
            M365Error::NotFound(_) => "m365_not_found",
            M365Error::StorageFull => "m365_storage_full",
            M365Error::Throttled { .. } => "m365_throttled",
            M365Error::Http { .. } => "m365_http",
            M365Error::Network(_) => "m365_network",
            M365Error::Timeout => "m365_timeout",
            M365Error::Uncertain(_) => "m365_uncertain",
            M365Error::Parse(_) => "m365_parse",
            M365Error::Invalid(_) => "m365_invalid",
            M365Error::Cancelled => "m365_cancelled",
            M365Error::SignInTimeout => "m365_sign_in_timeout",
            M365Error::Browser(_) => "m365_browser",
            M365Error::Listener(_) => "m365_listener",
            M365Error::Config(_) => "m365_config",
            M365Error::Store(_) => "m365_store",
            M365Error::MemoryLow(_) => "m365_memory_low",
        }
    }

    /// Kurzer, nie geheimer Zusatz zum Code (Statuszahl, Microsoft-Code, Minuten).
    pub fn detail(&self) -> Option<String> {
        match self {
            M365Error::CapabilityOff(c) => Some(c.as_str().to_string()),
            M365Error::NeedsConsent { missing } => Some(missing.join(" ")),
            M365Error::Throttled { retry_after } => {
                Some(retry_after.as_secs().div_ceil(60).max(1).to_string())
            }
            M365Error::Http { status, code } => Some(if code.is_empty() {
                status.to_string()
            } else {
                format!("{status} {code}")
            }),
            M365Error::Denied(d)
            | M365Error::NotFound(d)
            | M365Error::Network(d)
            | M365Error::Uncertain(d)
            | M365Error::Parse(d)
            | M365Error::Invalid(d)
            | M365Error::Browser(d)
            | M365Error::Listener(d)
            | M365Error::Config(d)
            | M365Error::Store(d)
            | M365Error::MemoryLow(d) => (!d.is_empty()).then(|| d.clone()),
            _ => None,
        }
    }

    /// `code` oder `code|detail`: so gehen Fehler an die Oberflaeche und ins Audit.
    pub fn wire(&self) -> String {
        match self.detail() {
            Some(d) => format!("{}|{}", self.code(), d.replace('|', "/")),
            None => self.code().to_string(),
        }
    }

    /// Hilft die Oberflaeche mit einem Klick auf „Anmelden“?
    pub fn needs_sign_in(&self) -> bool {
        matches!(
            self,
            M365Error::NeedsSignIn | M365Error::NeedsConsent { .. }
        )
    }
}

impl std::fmt::Display for M365Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            M365Error::NotConfigured => write!(
                f,
                "Es ist keine Client-ID eingetragen. Lege eine App-Registrierung an und trage ihre Anwendungs-ID ein."
            ),
            M365Error::NoCapability => write!(
                f,
                "Es ist keine Fähigkeit eingeschaltet. Schalte mindestens eine ein, bevor du dich anmeldest."
            ),
            M365Error::CapabilityOff(c) => write!(
                f,
                "Die Fähigkeit „{}“ ist an diesem Konto nicht eingeschaltet.",
                c.as_str()
            ),
            M365Error::NeedsSignIn => write!(
                f,
                "Anmeldung nötig: Die Anmeldung ist abgelaufen oder wurde widerrufen. Bitte neu anmelden."
            ),
            M365Error::NeedsConsent { missing } => write!(
                f,
                "Anmeldung nötig: Für die eingeschalteten Fähigkeiten fehlen noch Berechtigungen ({}). Bitte erneut anmelden und zustimmen.",
                missing.join(", ")
            ),
            M365Error::Denied(msg) => write!(f, "Microsoft hat den Zugriff abgelehnt: {msg}"),
            M365Error::NotFound(what) => write!(f, "Nicht gefunden: {what}"),
            M365Error::StorageFull => write!(f, "Der OneDrive-Speicher ist voll."),
            M365Error::Throttled { retry_after } => {
                let minutes = retry_after.as_secs().div_ceil(60).max(1);
                write!(
                    f,
                    "Microsoft drosselt die Abfragen (HTTP 429). Nächster Versuch in {minutes} min."
                )
            }
            M365Error::Http { status, code } if code.is_empty() => {
                write!(f, "Microsoft Graph antwortet mit HTTP {status}.")
            }
            M365Error::Http { status, code } => {
                write!(f, "Microsoft Graph antwortet mit HTTP {status} ({code}).")
            }
            M365Error::Network(msg) => write!(f, "Keine Verbindung: {msg}"),
            M365Error::Timeout => write!(f, "Zeitüberschreitung bei Microsoft."),
            M365Error::Uncertain(msg) => write!(
                f,
                "Unklar, ob die Aktion ausgeführt wurde ({msg}). Bitte erst prüfen (z. B. Ordner „Gesendet“), bevor du es erneut versuchst."
            ),
            M365Error::Parse(msg) => {
                write!(f, "Die Antwort von Microsoft ist nicht lesbar: {msg}")
            }
            M365Error::Invalid(msg) => write!(f, "{msg}"),
            M365Error::Cancelled => write!(f, "Abgebrochen."),
            M365Error::SignInTimeout => write!(
                f,
                "Die Anmeldung wurde nicht innerhalb von 5 Minuten abgeschlossen."
            ),
            M365Error::Browser(msg) => {
                write!(f, "Der Browser konnte nicht geöffnet werden: {msg}")
            }
            M365Error::Listener(msg) => write!(
                f,
                "Die lokale Anmelde-Schnittstelle konnte nicht gestartet werden: {msg}"
            ),
            M365Error::Config(msg) => write!(f, "{msg}"),
            M365Error::Store(msg) => write!(f, "Speicherfehler: {msg}"),
            M365Error::MemoryLow(msg) => write!(f, "{msg}"),
        }
    }
}

impl std::error::Error for M365Error {}

impl From<GraphError> for M365Error {
    fn from(e: GraphError) -> Self {
        match e {
            GraphError::NeedsSignIn => M365Error::NeedsSignIn,
            GraphError::Denied(m) => M365Error::Denied(m),
            GraphError::Throttled { retry_after } => M365Error::Throttled { retry_after },
            GraphError::Http(status) => M365Error::Http {
                status,
                code: String::new(),
            },
            GraphError::Network(m) => M365Error::Network(m),
            GraphError::Timeout => M365Error::Timeout,
            GraphError::Parse(m) => M365Error::Parse(m),
            GraphError::SignInTimeout => M365Error::SignInTimeout,
            GraphError::Cancelled => M365Error::Cancelled,
            GraphError::Browser(m) => M365Error::Browser(m),
            GraphError::Listener(m) => M365Error::Listener(m),
            GraphError::Config(m) => M365Error::Config(m),
        }
    }
}
