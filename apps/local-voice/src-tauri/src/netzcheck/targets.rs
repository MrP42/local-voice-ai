//! Gegenstellen aus den Einstellungen ableiten (lva#496). Rein.
//!
//! Geprüft wird NUR, was der Nutzer eingerichtet hat:
//!
//! * Geräteabgleich: nur mit Konto (`sync.json` vorhanden).
//! * Update-Quelle: nur mit eingeschalteter Update-Prüfung; die Adressen kommen
//!   aus der Updater-Einstellung der App (also auch bei einer eigenen Quelle).
//! * Sprachmodell-Anbieter: eingeschaltete Verbindungen mit Adresse `http(s)`
//!   UND mindestens einem freigegebenen Modell (eine Verbindung ohne
//!   freigegebenes Modell nutzt die App nie); Loopback (lokale Server),
//!   Link-Local und Nicht-Netz-Anbieter (`cli://`, `apple-intelligence://`)
//!   fallen weg.
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

/// Eine Sprachmodell-Verbindung mit der Zahl ihrer freigegebenen Modelle.
#[derive(Debug, Clone, Default)]
pub struct LlmService {
    pub url: String,
    pub enabled: bool,
    /// Freigegebene Modelle (`LlmModelConfig.enabled`) dieser Verbindung.
    pub enabled_models: usize,
}

/// Alles, was die Ableitung braucht, schon aus den Einstellungen gelesen.
#[derive(Debug, Clone, Default)]
pub struct Setup {
    /// Adresse des Sync-Hubs, wenn ein Konto verbunden ist.
    pub sync_hub_url: Option<String>,
    pub update_checks_enabled: bool,
    pub update_endpoints: Vec<String>,
    pub llm_connections: Vec<LlmService>,
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

/// Link-Local: `169.254.0.0/16` (auch die Metadaten-Adresse von Cloud-Rechnern)
/// und `fe80::/10`. Dort steht kein Dienst, den ein Nutzer einrichtet; ein
/// Treffer wäre nur Rauschen im Bericht. Private LAN-Bereiche (`10/8`,
/// `172.16/12`, `192.168/16`, `fc00::/7`) bleiben ausdrücklich erlaubt.
pub fn is_link_local_host(host: &str) -> bool {
    let h = host.trim_matches(['[', ']']);
    match h.parse::<std::net::IpAddr>() {
        Ok(std::net::IpAddr::V4(v4)) => v4.is_link_local(),
        Ok(std::net::IpAddr::V6(v6)) => {
            (v6.segments()[0] & 0xffc0) == 0xfe80
                || v6.to_ipv4_mapped().is_some_and(|v4| v4.is_link_local())
        }
        Err(_) => false,
    }
}

fn push(out: &mut Vec<Target>, category: Category, endpoint: Option<Endpoint>) {
    let Some(endpoint) = endpoint else { return };
    if is_loopback_host(&endpoint.host) || is_link_local_host(&endpoint.host) {
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
    for conn in setup
        .llm_connections
        .iter()
        .filter(|c| c.enabled && c.enabled_models > 0)
    {
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

    fn llm(url: &str, enabled: bool, enabled_models: usize) -> LlmService {
        LlmService {
            url: url.to_string(),
            enabled,
            enabled_models,
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
            llm_connections: vec![llm("https://api.openai.com/v1", true, 2)],
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
            llm_connections: vec![llm("https://api.mistral.ai/v1", false, 3)],
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
    fn verbindung_ohne_freigegebenes_modell_wird_nicht_geprueft() {
        let setup = Setup {
            llm_connections: vec![
                llm("https://ohne-modell.example.com/v1", true, 0),
                llm("https://mit-modell.example.com/v1", true, 1),
                llm("https://aus.example.com/v1", false, 4),
            ],
            ..Setup::default()
        };
        let t = derive_targets(&setup);
        assert_eq!(hosts(&t, Category::Llm), ["mit-modell.example.com:443"]);
    }

    #[test]
    fn link_local_faellt_weg_private_netze_bleiben() {
        let setup = Setup {
            llm_connections: vec![
                llm("http://169.254.169.254/v1", true, 1),
                llm("http://169.254.0.7:8000/v1", true, 1),
                llm("http://[fe80::1]:8000/v1", true, 1),
                llm("http://[febf::1]:8000/v1", true, 1),
                llm("http://10.0.0.5:8000/v1", true, 1),
                llm("http://172.16.4.1:8000/v1", true, 1),
                llm("http://192.168.178.20:8000/v1", true, 1),
                llm("http://[fd00::1]:8000/v1", true, 1),
                llm("http://169.255.0.1/v1", true, 1),
            ],
            ..Setup::default()
        };
        let t = derive_targets(&setup);
        assert_eq!(
            hosts(&t, Category::Llm),
            [
                "10.0.0.5:8000",
                "172.16.4.1:8000",
                "192.168.178.20:8000",
                "[fd00::1]:8000",
                "169.255.0.1:80"
            ]
        );
        for h in [
            "169.254.1.1",
            "169.254.169.254",
            "fe80::1",
            "[fe80::abcd]",
            "febf::1",
            "::ffff:169.254.1.1",
        ] {
            assert!(is_link_local_host(h), "{h}");
        }
        for h in [
            "169.253.1.1",
            "169.255.1.1",
            "fec0::1",
            "fd00::1",
            "example.com",
            "192.168.1.1",
        ] {
            assert!(!is_link_local_host(h), "{h}");
        }
    }

    #[test]
    fn lokale_server_und_cli_anbieter_fallen_weg() {
        let setup = Setup {
            llm_connections: vec![
                llm("http://localhost:11434/v1", true, 1),
                llm("http://127.0.0.1:8000/v1", true, 1),
                llm("http://[::1]:8000/v1", true, 1),
                llm("cli://claude", true, 1),
                llm("apple-intelligence://local", true, 1),
                llm("http://192.168.1.20:11434/v1", true, 1),
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
                llm("https://api.openai.com/v1", true, 1),
                llm("https://api.openai.com/v1/chat", true, 1),
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
            llm_connections: vec![llm(
                "https://u:p@llm.example.com/v1?api_key=GEHEIM",
                true,
                1,
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
                .map(|i| llm(&format!("https://h{i}.example.com"), true, 1))
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
