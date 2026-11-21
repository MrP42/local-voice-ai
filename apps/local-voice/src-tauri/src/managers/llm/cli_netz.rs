//! Abo-CLI im Firmennetz (lva#739): Installationshinweis statt Installer,
//! Proxy fuer den Kindprozess.

use std::process::Command;

use super::cli::{self, Cli};
use crate::netz::proxy::{self, Weg};

/// Was die Oberflaeche ueber die CLI eines Anbieters wissen muss (lva#739):
/// gefunden oder nicht, und wohin die offizielle Anleitung fuehrt.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, specta::Type)]
pub struct CliInstallInfo {
    /// Vorlagenkennung der Verbindung (`claude_cli`, `codex_cli`).
    pub kind: String,
    pub found: bool,
    /// Fundort der Programmdatei (nur wenn gefunden).
    pub path: Option<String>,
    /// Offizielle Installationsanleitung des Anbieters (nur https).
    pub docs_url: String,
}

pub fn docs_url(cli: Cli) -> &'static str {
    match cli {
        Cli::Claude => "https://code.claude.com/docs/en/setup",
        Cli::Codex => "https://developers.openai.com/codex/cli",
    }
}

pub fn kind_of(cli: Cli) -> &'static str {
    match cli {
        Cli::Claude => "claude_cli",
        Cli::Codex => "codex_cli",
    }
}

/// Erkennt die CLI ohne Prozessstart oder Download.
pub fn install_info(cli: Cli) -> CliInstallInfo {
    let path = cli::locate(cli).map(|p| p.to_string_lossy().into_owned());
    CliInstallInfo {
        kind: kind_of(cli).to_string(),
        found: path.is_some(),
        path,
        docs_url: docs_url(cli).to_string(),
    }
}

pub(crate) fn ziel_url(cli: Cli) -> &'static str {
    match cli {
        Cli::Claude => "https://api.anthropic.com/",
        Cli::Codex => "https://chatgpt.com/",
    }
}

const PROXY_NAMEN: [&str; 6] = [
    "HTTPS_PROXY",
    "HTTP_PROXY",
    "ALL_PROXY",
    "https_proxy",
    "http_proxy",
    "all_proxy",
];
const KEIN_PROXY: &str = "localhost,127.0.0.1";

/// Ein Registry-Proxy ohne Umgebungsvariable und ohne PAC wird nicht uebertragen.
pub(crate) fn proxy_umgebung(
    weg: &Weg,
    vermittelt: impl Fn(&str) -> String,
    lesen: impl Fn(&str) -> Option<String>,
) -> Vec<(&'static str, Option<String>)> {
    match weg {
        Weg::Standard => {
            let Some(p) = [
                "HTTPS_PROXY",
                "https_proxy",
                "ALL_PROXY",
                "all_proxy",
                "HTTP_PROXY",
                "http_proxy",
            ]
            .into_iter()
            .filter_map(&lesen)
            .map(|p| p.trim().to_string())
            .find(|p| !p.is_empty()) else {
                return Vec::new();
            };
            let url = vermittelt(&p);
            if url == p {
                return Vec::new();
            }
            let kein_proxy = lesen("NO_PROXY")
                .filter(|p| !p.trim().is_empty())
                .or_else(|| lesen("no_proxy").filter(|p| !p.trim().is_empty()))
                .map(|p| format!("{p},{KEIN_PROXY}"))
                .unwrap_or_else(|| KEIN_PROXY.to_string());
            vec![
                ("HTTPS_PROXY", Some(url.clone())),
                ("HTTP_PROXY", Some(url.clone())),
                ("https_proxy", Some(url.clone())),
                ("http_proxy", Some(url)),
                ("NO_PROXY", Some(kein_proxy.clone())),
                ("no_proxy", Some(kein_proxy)),
                ("ALL_PROXY", None),
                ("all_proxy", None),
            ]
        }
        Weg::Direkt => PROXY_NAMEN.into_iter().map(|name| (name, None)).collect(),
        Weg::Ueber(p) => {
            let url = vermittelt(p);
            vec![
                ("HTTPS_PROXY", Some(url.clone())),
                ("HTTP_PROXY", Some(url.clone())),
                ("https_proxy", Some(url.clone())),
                ("http_proxy", Some(url)),
                ("NO_PROXY", Some(KEIN_PROXY.to_string())),
                ("no_proxy", Some(KEIN_PROXY.to_string())),
                ("ALL_PROXY", None),
                ("all_proxy", None),
            ]
        }
    }
}

pub(crate) fn anwenden_weg(
    cmd: &mut Command,
    weg: &Weg,
    vermittelt: impl Fn(&str) -> String,
    lesen: impl Fn(&str) -> Option<String>,
) {
    // Nur im Kindprozess setzen, nie im eigenen Prozess (kein std::env::set_var).
    // NODE_USE_ENV_PROXY/NODE_EXTRA_CA_CERTS bleiben bewusst ungesetzt:
    // Bedarf der CLI nicht belegt; Handprobe Patrick.
    for (name, wert) in proxy_umgebung(weg, vermittelt, lesen) {
        match wert {
            Some(wert) => cmd.env(name, wert),
            None => cmd.env_remove(name),
        };
    }
}

/// Blockiert ggf. kurz (PAC-Auswertung, gecacht); nur aus synchronem Code
/// ausserhalb des async-Laufzeitfadens aufrufen.
/// Proxywerte nie loggen: Sie koennen den Zufalls-Token des lokalen Weiterleiters enthalten.
pub(crate) fn anwenden(cmd: &mut Command, cli: Cli) {
    anwenden_fuer(cmd, ziel_url(cli));
}

/// Wie lange der Start der CLI hoechstens auf die PAC-/WPAD-Auswertung wartet; danach gilt der
/// Standardweg (die CLI erbt ihre Umgebung). Die Auswertung laeuft weiter und fuellt den Cache.
#[cfg(not(test))]
const WEG_FRIST: std::time::Duration = std::time::Duration::from_secs(10);

/// Proxyweg fuer `ziel`, nie laenger als [`WEG_FRIST`]. In Unit-Tests gilt der Standardweg: sie
/// duerfen weder WinHTTP noch das Netz des Entwicklerrechners befragen.
fn weg_begrenzt(ziel: &str) -> Weg {
    #[cfg(test)]
    {
        let _ = ziel;
        Weg::Standard
    }
    #[cfg(not(test))]
    {
        let (tx, rx) = std::sync::mpsc::channel();
        let ziel = ziel.to_string();
        std::thread::spawn(move || {
            let _ = tx.send(proxy::weg_fuer(&ziel));
        });
        rx.recv_timeout(WEG_FRIST).unwrap_or(Weg::Standard)
    }
}

/// Blockiert hoechstens [`WEG_FRIST`]; nur ausserhalb des async-Laufzeitfadens aufrufen.
/// Proxywerte nie loggen: Sie koennen den Zufalls-Token des lokalen Weiterleiters enthalten.
pub(crate) fn anwenden_fuer(cmd: &mut Command, ziel: &str) {
    anwenden_weg(
        cmd,
        &weg_begrenzt(ziel),
        crate::netz::proxy_auth::vermittelt,
        |k| std::env::var(k).ok(),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ueber_proxy_setzt_https_und_http_und_kein_proxy_fuer_lokal() {
        let weg = Weg::Ueber("http://proxy.example.invalid:3128".into());
        let umgebung = proxy_umgebung(&weg, |_| "http://lv:tok@127.0.0.1:5555".into(), |_| None);
        assert_eq!(umgebung.len(), 8);
        for name in ["HTTPS_PROXY", "HTTP_PROXY", "https_proxy", "http_proxy"] {
            let eintrag = umgebung.iter().find(|(k, _)| *k == name).unwrap();
            assert_eq!(eintrag.1.as_deref(), Some("http://lv:tok@127.0.0.1:5555"));
        }
        for name in ["NO_PROXY", "no_proxy"] {
            let eintrag = umgebung.iter().find(|(k, _)| *k == name).unwrap();
            assert_eq!(eintrag.1.as_deref(), Some("localhost,127.0.0.1"));
        }
        for name in ["ALL_PROXY", "all_proxy"] {
            let eintrag = umgebung.iter().find(|(k, _)| *k == name).unwrap();
            assert_eq!(eintrag.1, None);
        }
    }

    #[test]
    fn standard_aendert_nichts() {
        let umgebung = proxy_umgebung(
            &Weg::Standard,
            |_| panic!("Keine Vermittlung erwartet"),
            |_| None,
        );
        assert!(umgebung.is_empty());
    }

    #[test]
    fn standard_mit_nicht_vermittelter_umgebung_aendert_nichts() {
        let umgebung = proxy_umgebung(
            &Weg::Standard,
            |p| p.to_string(),
            |k| (k == "HTTPS_PROXY").then(|| "http://f:3128".into()),
        );
        assert!(umgebung.is_empty());
    }

    #[test]
    fn standard_mit_vermitteltem_proxy_setzt_die_lokale_adresse_und_haengt_no_proxy_an() {
        let umgebung = proxy_umgebung(
            &Weg::Standard,
            |_| "http://lv:t@127.0.0.1:1".into(),
            |k| match k {
                "HTTPS_PROXY" => Some("http://f:3128".into()),
                "NO_PROXY" => Some("intern.example".into()),
                _ => None,
            },
        );
        for name in ["HTTPS_PROXY", "HTTP_PROXY", "https_proxy", "http_proxy"] {
            let eintrag = umgebung.iter().find(|(k, _)| *k == name).unwrap();
            assert_eq!(eintrag.1.as_deref(), Some("http://lv:t@127.0.0.1:1"));
        }
        for name in ["NO_PROXY", "no_proxy"] {
            let eintrag = umgebung.iter().find(|(k, _)| *k == name).unwrap();
            assert_eq!(
                eintrag.1.as_deref(),
                Some("intern.example,localhost,127.0.0.1")
            );
        }
        for name in ["ALL_PROXY", "all_proxy"] {
            let eintrag = umgebung.iter().find(|(k, _)| *k == name).unwrap();
            assert_eq!(eintrag.1, None);
        }
    }

    #[test]
    fn standard_ohne_umgebung_bleibt_leer() {
        let umgebung = proxy_umgebung(
            &Weg::Standard,
            |_| panic!("Keine Vermittlung erwartet"),
            |_| None,
        );
        assert!(umgebung.is_empty());
    }

    #[test]
    fn direkt_entfernt_alle_proxyvariablen() {
        let umgebung = proxy_umgebung(
            &Weg::Direkt,
            |_| panic!("Keine Vermittlung erwartet"),
            |_| None,
        );
        assert_eq!(
            umgebung,
            vec![
                ("HTTPS_PROXY", None),
                ("HTTP_PROXY", None),
                ("ALL_PROXY", None),
                ("https_proxy", None),
                ("http_proxy", None),
                ("all_proxy", None),
            ]
        );
    }

    #[test]
    fn anwenden_weg_schreibt_in_das_command() {
        use std::ffi::OsStr;

        let mut cmd = Command::new("x");
        let weg = Weg::Ueber("http://proxy.example.invalid:3128".into());
        anwenden_weg(&mut cmd, &weg, |p| format!("{p}/vermittelt"), |_| None);
        assert_eq!(
            cmd.get_envs()
                .find(|(k, _)| *k == OsStr::new("HTTPS_PROXY")),
            Some((
                OsStr::new("HTTPS_PROXY"),
                Some(OsStr::new("http://proxy.example.invalid:3128/vermittelt"))
            ))
        );
        assert_eq!(
            cmd.get_envs().find(|(k, _)| *k == OsStr::new("ALL_PROXY")),
            Some((OsStr::new("ALL_PROXY"), None))
        );
    }

    #[test]
    fn docs_url_ist_https_und_offiziell() {
        for (cli, host) in [
            (Cli::Claude, "code.claude.com"),
            (Cli::Codex, "developers.openai.com"),
        ] {
            let url = docs_url(cli);
            assert!(url.starts_with("https://"));
            assert_eq!(
                url.strip_prefix("https://").unwrap().split('/').next(),
                Some(host)
            );
        }
    }

    #[test]
    fn install_info_traegt_die_kennung() {
        let info = install_info(Cli::Claude);
        assert_eq!(info.kind, "claude_cli");
        assert_eq!(info.docs_url, docs_url(Cli::Claude));
    }

    #[test]
    fn kind_of_passt_zu_from_kind() {
        for cli in [Cli::Claude, Cli::Codex] {
            assert_eq!(Cli::from_kind(kind_of(cli)), Some(cli));
        }
    }
}
