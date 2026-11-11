//! Gegenstellen aus den Einstellungen ableiten (lva#496). Rein.
//!
//! Geprüft wird NUR, was der Nutzer eingerichtet hat:
//!
//! * Geräteabgleich: nur mit Konto (`sync.json` vorhanden).
//! * Update-Quelle: nur mit eingeschalteter Update-Prüfung; die Adressen kommen
//!   aus der Updater-Einstellung der App (also auch bei einer eigenen Quelle).
//! * Sprachmodell-Anbieter: eingeschaltete Verbindungen mit Adresse `http(s)`;
//!   Loopback (lokale Server) und Nicht-Netz-Anbieter (`cli://`,
//!   `apple-intelligence://`) fallen weg.
//! * Online-Stimmen: nur Anbieter, die freigegeben sind UND bei denen der
//!   Fernzugriff erlaubt ist; Loopback fällt weg.
//! * Kalender: Microsoft 365 nur mit eingerichteter, eingeschalteter Quelle;
//!   ICS je eingeschalteter Quelle (nur der Host, die Adresse selbst ist ein
//!   Geheimnis und kommt hier nie an).
//! * Modell-Downloads: die Hosts, von denen der Modellkatalog lädt; Hugging
//!   Face nur, wenn eine Stimme (Phonon) Downloads dort erlaubt.
//!
//! Alle Adressen gehen durch [`super::redact`]: übrig bleiben Host und Port.

use super::redact::{endpoint_from_host, endpoint_from_url, Endpoint};
use super::{Category, Target};

/// Obergrenze, damit eine Prüfung nie Hunderte Verbindungen auf einmal öffnet.
pub const MAX_TARGETS: usize = 40;

/// Anmeldeadresse und Graph-Adresse von Microsoft 365 (wie in
/// `managers/calendar/graph.rs`).
pub const GRAPH_URLS: [&str; 2] = [
    "https://login.microsoftonline.com",
    "https://graph.microsoft.com",
];

/// Quelle der Modell-Downloads, die immer zum Katalog gehört.
pub const HUGGING_FACE_URL: &str = "https://huggingface.co";

/// Ein Dienst mit Adresse und Schaltern (Sprachmodell-Verbindung, Stimme).
#[derive(Debug, Clone, Default)]
pub struct Service {
    pub url: String,
    pub enabled: bool,
    /// Nur für Stimmen: darf der Anbieter außerhalb von Loopback sprechen?
    pub allow_remote: bool,
}

/// Alles, was die Ableitung braucht, schon aus den Einstellungen gelesen.
#[derive(Debug, Clone, Default)]
pub struct Setup {
    /// Adresse des Sync-Hubs, wenn ein Konto verbunden ist.
    pub sync_hub_url: Option<String>,
    pub update_checks_enabled: bool,
    pub update_endpoints: Vec<String>,
    pub llm_connections: Vec<Service>,
    pub tts_services: Vec<Service>,
    /// Es gibt eine eingeschaltete Microsoft-365-Quelle.
    pub calendar_graph: bool,
    /// Hosts der eingeschalteten ICS-Quellen.
    pub calendar_ics_hosts: Vec<String>,
    /// Adressen der Modelle im Katalog (nur Hosts zählen).
    pub model_download_urls: Vec<String>,
    /// Eine Stimme darf von Hugging Face laden.
    pub huggingface_downloads: bool,
}

/// Loopback und „kein Netz“: `localhost`, `127.0.0.0/8`, `::1`, `0.0.0.0`.
pub fn is_loopback_host(host: &str) -> bool {
    let h = host.trim_matches(['[', ']']).to_ascii_lowercase();
    if h == "localhost" || h.ends_with(".localhost") || h == "0.0.0.0" || h == "::" {
        return true;
    }
    if let Ok(ip) = h.parse::<std::net::IpAddr>() {
        return ip.is_loopback()
            || matches!(ip, std::net::IpAddr::V6(v6) if v6.to_ipv4_mapped().is_some_and(|v4| v4.is_loopback()));
    }
    false
}

fn push(out: &mut Vec<Target>, category: Category, endpoint: Option<Endpoint>) {
    let Some(endpoint) = endpoint else { return };
    if is_loopback_host(&endpoint.host) {
        return;
    }
    let target = Target { category, endpoint };
    if !out.contains(&target) {
        out.push(target);
    }
}

/// Die Gegenstellen für diese Einrichtung, nach Zweck und Host geordnet.
pub fn derive_targets(setup: &Setup) -> Vec<Target> {
    let mut out: Vec<Target> = Vec::new();

    if let Some(url) = &setup.sync_hub_url {
        push(&mut out, Category::SyncHub, endpoint_from_url(url));
    }
    if setup.update_checks_enabled {
        for url in &setup.update_endpoints {
            push(&mut out, Category::Update, endpoint_from_url(url));
        }
    }
    for conn in setup.llm_connections.iter().filter(|c| c.enabled) {
        push(&mut out, Category::Llm, endpoint_from_url(&conn.url));
    }
    for tts in setup
        .tts_services
        .iter()
        .filter(|s| s.enabled && s.allow_remote)
    {
        push(&mut out, Category::Tts, endpoint_from_url(&tts.url));
    }
    if setup.calendar_graph {
        for url in GRAPH_URLS {
            push(&mut out, Category::CalendarGraph, endpoint_from_url(url));
        }
    }
    for host in &setup.calendar_ics_hosts {
        push(&mut out, Category::CalendarIcs, endpoint_from_host(host));
    }
    for url in &setup.model_download_urls {
        push(&mut out, Category::ModelDownload, endpoint_from_url(url));
    }
    if setup.huggingface_downloads {
        push(
            &mut out,
            Category::ModelDownload,
            endpoint_from_url(HUGGING_FACE_URL),
        );
    }

    // Stabil nach Zweck sortieren (Eingabereihenfolge bleibt innerhalb eines Zwecks).
    out.sort_by_key(|t| t.category);
    out.truncate(MAX_TARGETS);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn svc(url: &str, enabled: bool, allow_remote: bool) -> Service {
        Service {
            url: url.to_string(),
            enabled,
            allow_remote,
        }
    }

    fn hosts(targets: &[Target], category: Category) -> Vec<String> {
        targets
            .iter()
            .filter(|t| t.category == category)
            .map(|t| t.endpoint.host_port())
            .collect()
    }

    #[test]
    fn leere_einrichtung_prueft_nichts() {
        assert!(derive_targets(&Setup::default()).is_empty());
    }

    #[test]
    fn alles_eingerichtet_ergibt_je_zweck_eine_gegenstelle() {
        let setup = Setup {
            sync_hub_url: Some("https://portal.example.com".into()),
            update_checks_enabled: true,
            update_endpoints: vec![
                "https://github.com/x/y/releases/latest/download/latest.json".into(),
            ],
            llm_connections: vec![svc("https://api.openai.com/v1", true, false)],
            tts_services: vec![svc("https://tts.example.com/v1", true, true)],
            calendar_graph: true,
            calendar_ics_hosts: vec!["kalender.example.org".into()],
            model_download_urls: vec![
                "https://blob.handy.computer/ggml-small.bin".into(),
                "https://blob.handy.computer/parakeet-v3-int8.tar.gz".into(),
            ],
            huggingface_downloads: true,
        };
        let t = derive_targets(&setup);
        assert_eq!(hosts(&t, Category::SyncHub), ["portal.example.com:443"]);
        assert_eq!(hosts(&t, Category::Update), ["github.com:443"]);
        assert_eq!(hosts(&t, Category::Llm), ["api.openai.com:443"]);
        assert_eq!(hosts(&t, Category::Tts), ["tts.example.com:443"]);
        assert_eq!(
            hosts(&t, Category::CalendarGraph),
            ["login.microsoftonline.com:443", "graph.microsoft.com:443"]
        );
        assert_eq!(
            hosts(&t, Category::CalendarIcs),
            ["kalender.example.org:443"]
        );
        assert_eq!(
            hosts(&t, Category::ModelDownload),
            ["blob.handy.computer:443", "huggingface.co:443"]
        );
    }

    #[test]
    fn nur_was_eingeschaltet_ist() {
        let setup = Setup {
            sync_hub_url: None,
            update_checks_enabled: false,
            update_endpoints: vec!["https://github.com/a".into()],
            llm_connections: vec![svc("https://api.mistral.ai/v1", false, false)],
            tts_services: vec![
                svc("https://frei.example.com", true, false),
                svc("https://aus.example.com", false, true),
            ],
            calendar_graph: false,
            calendar_ics_hosts: vec![],
            model_download_urls: vec![],
            huggingface_downloads: false,
        };
        assert!(derive_targets(&setup).is_empty());
    }

    #[test]
    fn lokale_server_und_cli_anbieter_fallen_weg() {
        let setup = Setup {
            llm_connections: vec![
                svc("http://localhost:11434/v1", true, false),
                svc("http://127.0.0.1:8000/v1", true, false),
                svc("http://[::1]:8000/v1", true, false),
                svc("cli://claude", true, false),
                svc("apple-intelligence://local", true, false),
                svc("http://192.168.1.20:11434/v1", true, false),
            ],
            tts_services: vec![svc("http://127.0.0.1:8880/v1", true, true)],
            ..Setup::default()
        };
        let t = derive_targets(&setup);
        assert_eq!(hosts(&t, Category::Llm), ["192.168.1.20:11434"]);
        assert!(hosts(&t, Category::Tts).is_empty());
    }

    #[test]
    fn dubletten_innerhalb_eines_zwecks_entfallen_gleiche_hosts_anderer_zwecke_bleiben() {
        let setup = Setup {
            llm_connections: vec![
                svc("https://api.openai.com/v1", true, false),
                svc("https://api.openai.com/v1/chat", true, false),
            ],
            tts_services: vec![svc("https://api.openai.com/v1", true, true)],
            ..Setup::default()
        };
        let t = derive_targets(&setup);
        assert_eq!(hosts(&t, Category::Llm), ["api.openai.com:443"]);
        assert_eq!(hosts(&t, Category::Tts), ["api.openai.com:443"]);
    }

    #[test]
    fn geheimnisse_in_adressen_erreichen_die_gegenstelle_nie() {
        let setup = Setup {
            sync_hub_url: Some(
                "https://anna:passwort@hub.example.com:8443/api?token=GEHEIM".into(),
            ),
            calendar_ics_hosts: vec!["kal.example.org/privat/GEHEIM.ics?key=1".into()],
            llm_connections: vec![svc(
                "https://u:p@llm.example.com/v1?api_key=GEHEIM",
                true,
                false,
            )],
            ..Setup::default()
        };
        let t = derive_targets(&setup);
        let shown = format!("{t:?}");
        for secret in [
            "anna", "passwort", "GEHEIM", "token", "api_key", "privat", "key=1",
        ] {
            assert!(!shown.contains(secret), "{secret}");
        }
        assert_eq!(hosts(&t, Category::SyncHub), ["hub.example.com:8443"]);
    }

    #[test]
    fn nach_zweck_sortiert_und_begrenzt() {
        let setup = Setup {
            llm_connections: (0..100)
                .map(|i| svc(&format!("https://h{i}.example.com"), true, false))
                .collect(),
            sync_hub_url: Some("https://hub.example.com".into()),
            ..Setup::default()
        };
        let t = derive_targets(&setup);
        assert_eq!(t.len(), MAX_TARGETS);
        assert_eq!(t[0].category, Category::SyncHub);
    }

    #[test]
    fn loopback_erkennung() {
        for h in [
            "localhost",
            "LOCALHOST",
            "foo.localhost",
            "127.0.0.1",
            "127.5.5.5",
            "::1",
            "[::1]",
            "0.0.0.0",
            "::ffff:127.0.0.1",
        ] {
            assert!(is_loopback_host(h), "{h}");
        }
        for h in [
            "example.com",
            "192.168.0.1",
            "10.0.0.5",
            "localhost.example.com",
        ] {
            assert!(!is_loopback_host(h), "{h}");
        }
    }
}
