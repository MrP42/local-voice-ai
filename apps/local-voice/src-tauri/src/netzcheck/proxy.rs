//! System-Proxy erkennen (lva#496).
//!
//! Ein Proxy ändert, was die Prüfung messen kann: Mit Proxy löst nicht dieser
//! Rechner den Namen auf und baut nicht selbst die TLS-Verbindung zur
//! Gegenstelle auf. Der Bericht nennt ihn deshalb, damit die IT weiß, durch
//! welchen Weg die App geht.
//!
//! Erkannt wird, was ohne neue Abhängigkeit lesbar ist:
//!
//! * die Umgebungsvariablen `HTTPS_PROXY`, `HTTP_PROXY`, `ALL_PROXY`;
//! * unter Windows die Internetoptionen des Benutzers (Proxyserver, und ob ein
//!   Konfigurationsskript (PAC) eingetragen ist).
//!
//! Nicht erkannt: macOS-Netzwerkeinstellungen und ein WPAD-Abruf (automatische
//! Erkennung). Der Bericht sagt das nicht an, weil er nur Befunde enthält; die
//! Oberfläche weist darauf hin, dass „nicht erkannt“ keine Gewähr ist.
//!
//! Zugangsdaten in Proxy-Adressen werden hier abgeschnitten
//! ([`super::redact::host_port_from_any`]); vom PAC-Skript bleibt nur, DASS es
//! eines gibt.

use serde::{Deserialize, Serialize};
use specta::Type;

use super::redact::{format_host_port, host_port_from_any};

/// Woher der Proxy bekannt ist.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Type)]
#[serde(rename_all = "snake_case")]
pub enum ProxySource {
    /// Umgebungsvariable (`HTTPS_PROXY` usw.).
    Env,
    /// Windows-Internetoptionen.
    System,
}

/// Ein erkannter Proxy: wofür er gilt und `host:port`.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, Type)]
pub struct ProxyEntry {
    pub source: ProxySource,
    /// `http`, `https`, `all` oder `socks`.
    pub scope: String,
    /// `host:port`, ohne Zugangsdaten.
    pub endpoint: String,
}

#[derive(Serialize, Deserialize, Debug, Clone, Default, PartialEq, Eq, Type)]
pub struct ProxyInfo {
    pub entries: Vec<ProxyEntry>,
    /// In den Internetoptionen ist ein Konfigurationsskript (PAC) eingetragen.
    pub pac: bool,
}

impl ProxyInfo {
    /// Läuft die App vermutlich über einen Proxy?
    pub fn configured(&self) -> bool {
        self.pac || !self.entries.is_empty()
    }
}

fn entry(source: ProxySource, scope: &str, raw: &str) -> Option<ProxyEntry> {
    let (host, port) = host_port_from_any(raw)?;
    Some(ProxyEntry {
        source,
        scope: scope.to_string(),
        endpoint: format_host_port(&host, port),
    })
}

/// Proxys aus Umgebungsvariablen. Leere Werte zählen nicht; Groß- und
/// Kleinschreibung der Namen ist gleich. `no_proxy` wird nicht ausgewertet.
pub fn parse_env(vars: &[(String, String)]) -> Vec<ProxyEntry> {
    let mut out = Vec::new();
    for (scope, name) in [
        ("https", "HTTPS_PROXY"),
        ("http", "HTTP_PROXY"),
        ("all", "ALL_PROXY"),
    ] {
        let value = vars
            .iter()
            .find(|(k, v)| k.eq_ignore_ascii_case(name) && !v.trim().is_empty())
            .map(|(_, v)| v.as_str());
        if let Some(e) = value.and_then(|v| entry(ProxySource::Env, scope, v)) {
            if !out.contains(&e) {
                out.push(e);
            }
        }
    }
    out
}

/// Der Wert `ProxyServer` der Windows-Internetoptionen: entweder ein Server
/// für alles (`proxy:8080`) oder `http=a:80;https=b:443;socks=c:1080`.
pub fn parse_windows_server(server: &str) -> Vec<ProxyEntry> {
    let mut out = Vec::new();
    for part in server.split(';').map(str::trim).filter(|p| !p.is_empty()) {
        let (scope, addr) = match part.split_once('=') {
            Some((s, a)) => (s.trim().to_ascii_lowercase(), a.trim()),
            None => ("all".to_string(), part),
        };
        let scope = match scope.as_str() {
            "http" | "https" | "all" | "socks" => scope,
            _ => continue,
        };
        let raw = if scope == "socks" && !addr.contains("://") {
            format!("socks5://{addr}")
        } else {
            addr.to_string()
        };
        if let Some(e) = entry(ProxySource::System, &scope, &raw) {
            if !out.contains(&e) {
                out.push(e);
            }
        }
    }
    out
}

/// Windows-Internetoptionen des Benutzers: (Proxyserver, PAC eingetragen).
#[cfg(windows)]
fn windows_settings() -> (Option<String>, bool) {
    use winreg::enums::HKEY_CURRENT_USER;
    use winreg::RegKey;
    let Ok(key) = RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey(r"Software\Microsoft\Windows\CurrentVersion\Internet Settings")
    else {
        return (None, false);
    };
    let enabled: u32 = key.get_value("ProxyEnable").unwrap_or(0);
    let server: Option<String> = if enabled != 0 {
        key.get_value("ProxyServer").ok()
    } else {
        None
    };
    let pac = key
        .get_value::<String, _>("AutoConfigURL")
        .map(|s| !s.trim().is_empty())
        .unwrap_or(false);
    (server, pac)
}

#[cfg(not(windows))]
fn windows_settings() -> (Option<String>, bool) {
    (None, false)
}

/// Umgebungsvariablen als Textpaare. Einträge, die nicht gültiges Unicode sind,
/// fallen weg: `std::env::vars()` bricht bei ihnen mit einer Panik ab, und
/// ein fremder Eintrag darf die Prüfung nicht abschießen.
fn utf8_pairs<I>(vars: I) -> Vec<(String, String)>
where
    I: IntoIterator<Item = (std::ffi::OsString, std::ffi::OsString)>,
{
    vars.into_iter()
        .filter_map(|(k, v)| Some((k.to_str()?.to_string(), v.to_str()?.to_string())))
        .collect()
}

/// Liest Umgebung und (unter Windows) Internetoptionen.
pub fn detect() -> ProxyInfo {
    let vars = utf8_pairs(std::env::vars_os());
    let mut entries = parse_env(&vars);
    let (server, pac) = windows_settings();
    if let Some(server) = server {
        for e in parse_windows_server(&server) {
            if !entries.contains(&e) {
                entries.push(e);
            }
        }
    }
    ProxyInfo { entries, pac }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vars(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn umgebung_ohne_zugangsdaten_und_ohne_leere_werte() {
        let e = parse_env(&vars(&[
            ("https_proxy", "http://anna:geheim@proxy.firma.example:3128"),
            ("HTTP_PROXY", "  "),
            ("PATH", "/usr/bin"),
        ]));
        assert_eq!(e.len(), 1);
        assert_eq!(e[0].scope, "https");
        assert_eq!(e[0].endpoint, "proxy.firma.example:3128");
        let shown = format!("{e:?}");
        assert!(!shown.contains("anna") && !shown.contains("geheim"));
    }

    #[test]
    fn nicht_unicode_variablen_fallen_weg_ohne_absturz() {
        use std::ffi::OsString;
        let mut raw = vec![
            (
                OsString::from("HTTPS_PROXY"),
                OsString::from("proxy.example:3128"),
            ),
            (OsString::from("PATH"), OsString::from("/usr/bin")),
        ];
        #[cfg(unix)]
        {
            use std::os::unix::ffi::OsStringExt;
            raw.push((
                OsString::from("KAPUTT"),
                OsString::from_vec(vec![0x66, 0xff, 0xfe]),
            ));
            raw.push((OsString::from_vec(vec![0xc3, 0x28]), OsString::from("x")));
        }
        #[cfg(windows)]
        {
            use std::os::windows::ffi::OsStringExt;
            raw.push((
                OsString::from("KAPUTT"),
                OsString::from_wide(&[0x66, 0xd800]),
            ));
        }
        let pairs = utf8_pairs(raw);
        assert_eq!(pairs.len(), 2, "{pairs:?}");
        assert_eq!(parse_env(&pairs).len(), 1);
    }

    #[test]
    fn detect_stuerzt_mit_kaputter_umgebung_nicht_ab() {
        // `detect` liest die echte Umgebung; es darf nie eine Panik geben.
        let _ = detect();
    }

    #[test]
    fn keine_variablen_kein_proxy() {
        let info = ProxyInfo {
            entries: parse_env(&vars(&[("HOME", "/root")])),
            pac: false,
        };
        assert!(!info.configured());
    }

    #[test]
    fn windows_ein_server_fuer_alles() {
        let e = parse_windows_server("proxy.firma.example:8080");
        assert_eq!(e.len(), 1);
        assert_eq!(e[0].scope, "all");
        assert_eq!(e[0].endpoint, "proxy.firma.example:8080");
        assert_eq!(e[0].source, ProxySource::System);
    }

    #[test]
    fn windows_je_protokoll_und_socks() {
        let e = parse_windows_server(
            "http=a.example:80;https=b.example:443;socks=c.example:1080;ftp=d.example:21",
        );
        let shown: Vec<_> = e
            .iter()
            .map(|x| format!("{}={}", x.scope, x.endpoint))
            .collect();
        assert_eq!(
            shown,
            [
                "http=a.example:80",
                "https=b.example:443",
                "socks=c.example:1080"
            ]
        );
    }

    #[test]
    fn pac_allein_zaehlt_als_proxy() {
        let info = ProxyInfo {
            entries: vec![],
            pac: true,
        };
        assert!(info.configured());
    }

    #[test]
    fn muell_wird_ignoriert() {
        assert!(parse_windows_server("").is_empty());
        assert!(parse_windows_server(";;").is_empty());
    }
}
