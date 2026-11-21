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

pub(crate) fn proxy_umgebung(
    weg: &Weg,
    vermittelt: impl Fn(&str) -> String,
) -> Vec<(&'static str, Option<String>)> {
    match weg {
        Weg::Standard => Vec::new(),
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

pub(crate) fn anwenden_weg(cmd: &mut Command, weg: &Weg, vermittelt: impl Fn(&str) -> String) {
    // Nur im Kindprozess setzen, nie im eigenen Prozess (kein std::env::set_var).
    // NODE_USE_ENV_PROXY/NODE_EXTRA_CA_CERTS bleiben bewusst ungesetzt:
    // Bedarf der CLI nicht belegt; Handprobe Patrick.
    for (name, wert) in proxy_umgebung(weg, vermittelt) {
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
    anwenden_weg(
        cmd,
        &proxy::weg_fuer(ziel_url(cli)),
        crate::netz::proxy_auth::vermittelt,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ueber_proxy_setzt_https_und_http_und_kein_proxy_fuer_lokal() {
        let weg = Weg::Ueber("http://proxy.example.invalid:3128".into());
        let umgebung = proxy_umgebung(&weg, |_| "http://lv:tok@127.0.0.1:5555".into());
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
        let umgebung = proxy_umgebung(&Weg::Standard, |_| panic!("Keine Vermittlung erwartet"));
        assert!(umgebung.is_empty());
    }

    #[test]
    fn direkt_entfernt_alle_proxyvariablen() {
        let umgebung = proxy_umgebung(&Weg::Direkt, |_| panic!("Keine Vermittlung erwartet"));
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
        anwenden_weg(&mut cmd, &weg, |p| format!("{p}/vermittelt"));
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
