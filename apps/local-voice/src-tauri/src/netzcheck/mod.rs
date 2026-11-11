//! Netzwerkprüfung mit kopierbarem Bericht für die IT (lva#496, Epic #495).
//!
//! Ein Knopf in den Einstellungen prüft, ob Local Voice die Gegenstellen
//! erreicht, die der Nutzer eingerichtet hat (Geräteabgleich, Update-Quelle,
//! freigegebene Online-Anbieter für Sprachmodelle und Stimmen, Kalender,
//! Modell-Downloads). Nur auf Knopfdruck, nie im Hintergrund.
//!
//! Aufbau (der Kern ist rein und ohne Netz testbar):
//!
//! * [`targets`]: aus den Einstellungen ([`targets::Setup`]) die Gegenstellen
//!   ableiten: nur Eingerichtetes, nie Loopback, Dubletten zusammengefasst.
//! * [`Probe`]: die EINE Stelle, die das Netz berührt. [`probe::SystemProbe`]
//!   macht Namensauflösung, TLS-Aufbau und HTTP; Tests nutzen eine Attrappe.
//! * [`judge`]: aus der Messung erreichbar / eingeschränkt / nicht erreichbar
//!   samt Grund und Auswirkung in einem Satz.
//! * [`proxy`]: System-Proxy erkennen (Umgebung, Windows-Internetoptionen).
//! * [`redact`]: Schwärzung; jede Adresse geht dadurch, bevor sie in Zeile oder
//!   Bericht steht.
//! * [`report`]: der Berichtstext (Hostname, Port, Ergebnis, App-Version,
//!   Betriebssystem; nie Token, Schlüssel, Nutzernamen, Pfade, Inhalte).
//!
//! Zeitlimit je Gegenstelle [`PROBE_TIMEOUT`] (5 s); alle Gegenstellen laufen
//! gleichzeitig, also dauert die ganze Prüfung höchstens etwa so lange.

pub mod judge;
pub mod probe;
pub mod proxy;
pub mod redact;
pub mod report;
pub mod targets;

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use specta::Type;

use redact::Endpoint;

/// Zeitlimit je Gegenstelle.
pub const PROBE_TIMEOUT: Duration = Duration::from_secs(5);

/// Ab dieser Gesamtdauer gilt eine antwortende Gegenstelle als „eingeschränkt“.
pub const SLOW_MS: u32 = 3_000;

/// Wozu die App die Gegenstelle braucht.
#[derive(
    Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Type,
)]
#[serde(rename_all = "snake_case")]
#[specta(rename = "NetzCategory")]
pub enum Category {
    SyncHub,
    Update,
    Llm,
    Tts,
    CalendarGraph,
    CalendarIcs,
    ModelDownload,
}

/// Ergebnis je Zeile.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Type)]
#[serde(rename_all = "snake_case")]
#[specta(rename = "NetzState")]
pub enum State {
    Reachable,
    Limited,
    Unreachable,
}

/// Warum eine Zeile nicht „erreichbar“ ist.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Type)]
#[serde(rename_all = "snake_case")]
#[specta(rename = "NetzReason")]
pub enum Reason {
    /// Der Name lässt sich nicht auflösen.
    DnsFailed,
    /// Keine Antwort innerhalb des Zeitlimits.
    Timeout,
    /// Verbindung abgelehnt oder Ziel nicht erreichbar.
    ConnectFailed,
    /// Das Zertifikat wird nicht als vertrauenswürdig angenommen.
    TlsCertificate,
    /// TLS-Aufbau scheitert aus anderem Grund.
    TlsFailed,
    /// Verbindung steht, HTTP scheitert.
    HttpFailed,
    /// 5xx vom Server oder vom Proxy.
    // Serde schriebe `Http5xx` sonst als `http5xx`; Oberfläche, Bindings und
    // Übersetzungen kennen `http_5xx`.
    #[serde(rename = "http_5xx")]
    Http5xx,
    /// 407: Der Proxy verlangt eine Anmeldung.
    ProxyAuth,
    /// Zertifikat stammt vermutlich von einem TLS-aufbrechenden Proxy.
    TlsInspection,
    /// Antwortet, aber langsamer als [`SLOW_MS`].
    Slow,
}

/// In welchem Schritt ein Zeitlimit ablief.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    Dns,
    Connect,
    Http,
    /// Nur das äußere Zeitlimit hat zugeschlagen; der Schritt ist unbekannt.
    Unknown,
}

/// Wie ein Versuch gescheitert ist (vor der Bewertung).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Failure {
    Dns,
    Timeout(Stage),
    Connect,
    TlsCertificate,
    Tls,
    Http,
}

/// Messung einer Gegenstelle. Alle Zeiten in Millisekunden.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProbeOutcome {
    pub dns_ms: Option<u32>,
    /// TCP-Verbindung samt TLS-Handshake; nur, wenn die App die Verbindung
    /// selbst aufbauen konnte (ohne Proxy).
    pub tls_ms: Option<u32>,
    pub http_status: Option<u16>,
    pub http_ms: Option<u32>,
    pub total_ms: u32,
    pub failure: Option<Failure>,
    /// Aussteller des Zertifikats der Gegenstelle (bereits geschwärzt, gekürzt).
    pub issuer: Option<String>,
}

/// Die eine Stelle, die das Netz berührt.
#[async_trait]
pub trait Probe: Send + Sync {
    async fn probe(&self, endpoint: &Endpoint) -> ProbeOutcome;
}

/// Eine zu prüfende Gegenstelle samt Zweck.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target {
    pub category: Category,
    pub endpoint: Endpoint,
}

/// Eine Ergebniszeile für Oberfläche und Bericht.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, Type)]
#[specta(rename = "NetzRow")]
pub struct Row {
    pub category: Category,
    pub host: String,
    pub port: u16,
    pub tls: bool,
    pub state: State,
    pub reason: Option<Reason>,
    pub dns_ms: Option<u32>,
    pub tls_ms: Option<u32>,
    pub http_status: Option<u16>,
    pub total_ms: u32,
    /// Aussteller des Zertifikats, nur gesetzt bei Verdacht auf einen
    /// TLS-aufbrechenden Proxy oder bei nicht vertrautem Zertifikat. Fehlt er
    /// bei Grund `tls_certificate`, war er nicht ermittelbar.
    pub issuer: Option<String>,
}

/// Sperre gegen einen zweiten gleichzeitigen Lauf. [`RunGuard::acquire`]
/// liefert den Wächter nur, wenn die Sperre frei war; sein `Drop` gibt sie
/// wieder frei. Ein abgewiesener Aufruf darf die Sperre des laufenden Laufs
/// nie berühren (der Wächter wird dann gar nicht erst gebaut).
#[derive(Debug)]
pub struct RunGuard(&'static AtomicBool);

impl RunGuard {
    pub fn acquire(flag: &'static AtomicBool) -> Option<Self> {
        if flag
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return None;
        }
        Some(Self(flag))
    }
}

impl Drop for RunGuard {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

fn ms(d: Duration) -> u32 {
    u32::try_from(d.as_millis()).unwrap_or(u32::MAX)
}

/// Prüft alle Gegenstellen gleichzeitig, jede mit dem Zeitlimit `timeout`.
/// Gleiche Gegenstellen (Host, Port, TLS) werden nur einmal gemessen und
/// stehen dann für jeden Zweck als Zeile da. Die Reihenfolge der Zeilen
/// ist die der `targets`.
pub async fn run_check<P: Probe + ?Sized>(
    targets: &[Target],
    probe: &P,
    timeout: Duration,
) -> Vec<Row> {
    let mut unique: Vec<&Endpoint> = Vec::new();
    for t in targets {
        if !unique.contains(&&t.endpoint) {
            unique.push(&t.endpoint);
        }
    }
    let outcomes = futures_util::future::join_all(unique.iter().map(|ep| async move {
        match tokio::time::timeout(timeout, probe.probe(ep)).await {
            Ok(outcome) => outcome,
            Err(_) => ProbeOutcome {
                total_ms: ms(timeout),
                failure: Some(Failure::Timeout(Stage::Unknown)),
                ..ProbeOutcome::default()
            },
        }
    }))
    .await;
    let by_endpoint: BTreeMap<&Endpoint, &ProbeOutcome> =
        unique.iter().copied().zip(outcomes.iter()).collect();
    targets
        .iter()
        .map(|t| {
            let outcome = by_endpoint[&t.endpoint];
            judge::row(t, outcome)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::atomic::AtomicUsize;

    struct Fake {
        by_host: HashMap<String, ProbeOutcome>,
        calls: AtomicUsize,
        hang_for: Option<String>,
    }

    #[async_trait]
    impl Probe for Fake {
        async fn probe(&self, ep: &Endpoint) -> ProbeOutcome {
            self.calls.fetch_add(1, Ordering::SeqCst);
            if self.hang_for.as_deref() == Some(ep.host.as_str()) {
                tokio::time::sleep(Duration::from_secs(30)).await;
            }
            self.by_host.get(&ep.host).cloned().unwrap_or_default()
        }
    }

    fn ep(host: &str) -> Endpoint {
        Endpoint {
            host: host.to_string(),
            port: 443,
            tls: true,
        }
    }

    fn ok(status: u16, total: u32) -> ProbeOutcome {
        ProbeOutcome {
            dns_ms: Some(5),
            tls_ms: Some(40),
            http_status: Some(status),
            http_ms: Some(20),
            total_ms: total,
            ..ProbeOutcome::default()
        }
    }

    #[tokio::test]
    async fn dubletten_werden_nur_einmal_gemessen_und_je_zweck_gezeigt() {
        let mut by_host = HashMap::new();
        by_host.insert("api.example.com".to_string(), ok(404, 90));
        let fake = Fake {
            by_host,
            calls: AtomicUsize::new(0),
            hang_for: None,
        };
        let targets = vec![
            Target {
                category: Category::Llm,
                endpoint: ep("api.example.com"),
            },
            Target {
                category: Category::Tts,
                endpoint: ep("api.example.com"),
            },
        ];
        let rows = run_check(&targets, &fake, PROBE_TIMEOUT).await;
        assert_eq!(fake.calls.load(Ordering::SeqCst), 1);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].category, Category::Llm);
        assert_eq!(rows[1].category, Category::Tts);
        assert!(rows.iter().all(|r| r.state == State::Reachable));
    }

    #[tokio::test]
    async fn haengende_gegenstelle_haelt_die_anderen_nicht_auf() {
        let mut by_host = HashMap::new();
        by_host.insert("schnell.example".to_string(), ok(200, 50));
        let fake = Fake {
            by_host,
            calls: AtomicUsize::new(0),
            hang_for: Some("haengt.example".to_string()),
        };
        let targets = vec![
            Target {
                category: Category::SyncHub,
                endpoint: ep("haengt.example"),
            },
            Target {
                category: Category::Update,
                endpoint: ep("schnell.example"),
            },
        ];
        let start = std::time::Instant::now();
        let rows = run_check(&targets, &fake, Duration::from_millis(150)).await;
        assert!(
            start.elapsed() < Duration::from_secs(5),
            "parallel, nicht seriell"
        );
        assert_eq!(rows[0].state, State::Unreachable);
        assert_eq!(rows[0].reason, Some(Reason::Timeout));
        assert_eq!(rows[0].total_ms, 150);
        assert_eq!(rows[1].state, State::Reachable);
    }

    #[tokio::test]
    async fn keine_gegenstellen_keine_zeilen() {
        let fake = Fake {
            by_host: HashMap::new(),
            calls: AtomicUsize::new(0),
            hang_for: None,
        };
        assert!(run_check(&[], &fake, PROBE_TIMEOUT).await.is_empty());
        assert_eq!(fake.calls.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn zweiter_lauf_wird_abgewiesen_und_gibt_die_sperre_nicht_frei() {
        static FLAG: AtomicBool = AtomicBool::new(false);
        let first = RunGuard::acquire(&FLAG).expect("erste Sperre frei");
        assert!(RunGuard::acquire(&FLAG).is_none(), "zweiter Aufruf");
        // Der abgewiesene Aufruf darf die Sperre nicht gelöst haben.
        assert!(RunGuard::acquire(&FLAG).is_none(), "dritter Aufruf");
        assert!(FLAG.load(Ordering::SeqCst));
        drop(first);
        assert!(!FLAG.load(Ordering::SeqCst));
        let again = RunGuard::acquire(&FLAG);
        assert!(again.is_some(), "nach dem Ende wieder frei");
    }

    /// Die Oberfläche (und `bindings.ts`) kennt diese Schlüssel; serde muss
    /// sie genau so schreiben, sonst fehlt die Übersetzung im Browser.
    #[test]
    fn grund_schluessel_stimmen_mit_den_uebersetzungen_ueberein() {
        const ALL: [Reason; 10] = [
            Reason::DnsFailed,
            Reason::Timeout,
            Reason::ConnectFailed,
            Reason::TlsCertificate,
            Reason::TlsFailed,
            Reason::HttpFailed,
            Reason::Http5xx,
            Reason::ProxyAuth,
            Reason::TlsInspection,
            Reason::Slow,
        ];
        let de: serde_json::Value = serde_json::from_str(include_str!(
            "../../../src/i18n/locales/de/translation.json"
        ))
        .unwrap();
        let en: serde_json::Value = serde_json::from_str(include_str!(
            "../../../src/i18n/locales/en/translation.json"
        ))
        .unwrap();
        for (lang, root) in [("de", &de), ("en", &en)] {
            let reasons = root["settings"]["app"]["networkCheck"]["reason"]
                .as_object()
                .unwrap_or_else(|| panic!("{lang}: reason fehlt"));
            for r in ALL {
                let key = serde_json::to_value(r).unwrap();
                let key = key.as_str().unwrap();
                assert!(reasons.contains_key(key), "{lang}: {key} fehlt");
            }
            assert_eq!(reasons.len(), ALL.len(), "{lang}: überzählige Schlüssel");
        }
        assert_eq!(
            serde_json::to_value(Reason::Http5xx).unwrap(),
            serde_json::json!("http_5xx")
        );
    }
}
