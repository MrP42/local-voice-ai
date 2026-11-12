//! Schwärzung für die Netzwerkprüfung (lva#496).
//!
//! Der Bericht geht an die IT und darf nur enthalten, was sie braucht: Hostname,
//! Port, Ergebnis. Adressen der Nutzer tragen oft Geheimnisse (ICS-Adressen mit
//! Zugriffsschlüssel in Pfad oder Abfrage, `https://nutzer:passwort@host/…`,
//! Proxy-Angaben mit Zugangsdaten). Deshalb geht jede Adresse durch genau diese
//! Funktionen, bevor sie in eine Zeile, einen Bericht oder eine Meldung kommt:
//!
//! * [`endpoint_from_url`]: nur Schema, Host, Port; Userinfo, Pfad, Abfrage und
//!   Fragment fallen weg.
//! * [`host_port_from_any`]: dasselbe für Proxy-Angaben (beliebiges Schema, auch
//!   ohne Schema).
//! * [`scrub_text`]: Freitext (z. B. der Aussteller eines Zertifikats) ohne
//!   Adressen, E-Mail-Adressen, Pfade und schlüsselartige Zeichenfolgen.
//!
//! Rein und ohne Netz; alles hier ist getestet.

use std::sync::OnceLock;

use regex::Regex;
use url::Url;

/// Eine Gegenstelle, wie sie geprüft und berichtet wird: nie mehr als dies.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Endpoint {
    /// Kleingeschrieben, ohne Klammern bei IPv6, ohne Punkt am Ende.
    pub host: String,
    pub port: u16,
    pub tls: bool,
}

impl Endpoint {
    /// `host:port` für die Anzeige (IPv6 in Klammern).
    pub fn host_port(&self) -> String {
        format_host_port(&self.host, self.port)
    }
}

pub fn format_host_port(host: &str, port: u16) -> String {
    if host.contains(':') {
        format!("[{host}]:{port}")
    } else {
        format!("{host}:{port}")
    }
}

const MAX_HOST_LEN: usize = 253;

/// Ein Hostname aus Buchstaben, Ziffern, Punkt und Bindestrich oder eine
/// IPv6-Adresse. Alles andere (Leerzeichen, `@`, `/`, `?`, `%`, Steuerzeichen)
/// ist ein Zeichen, dass hier etwas anderes als ein Host steht.
fn clean_host(raw: &str) -> Option<String> {
    let host = raw
        .trim()
        .trim_start_matches('[')
        .trim_end_matches(']')
        .trim_end_matches('.')
        .to_ascii_lowercase();
    if host.is_empty() || host.len() > MAX_HOST_LEN {
        return None;
    }
    let ipv6 = host.contains(':');
    let ok = if ipv6 {
        host.chars()
            .all(|c| c.is_ascii_hexdigit() || c == ':' || c == '.')
    } else {
        host.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '_')
    };
    ok.then_some(host)
}

/// Gegenstelle aus einer Adresse mit Schema `http` oder `https`.
/// `None` bei allem anderen (`cli://…`, `file://…`, kein Host, ungültig).
pub fn endpoint_from_url(raw: &str) -> Option<Endpoint> {
    let url = Url::parse(raw.trim()).ok()?;
    let tls = match url.scheme() {
        "https" => true,
        "http" => false,
        _ => return None,
    };
    let host = clean_host(url.host_str()?)?;
    let port = url.port_or_known_default()?;
    Some(Endpoint { host, port, tls })
}

/// Gegenstelle aus einem nackten Host (etwa dem Anzeigehinweis einer
/// Kalenderquelle) mit Standardport 443 und TLS.
pub fn endpoint_from_host(raw: &str) -> Option<Endpoint> {
    // Ein Hinweis kann versehentlich mehr tragen: über die Adressfunktion
    // schicken, damit Userinfo, Pfad und Abfrage auch hier wegfallen.
    let raw = raw.trim();
    if raw.contains("://") {
        return endpoint_from_url(raw);
    }
    endpoint_from_url(&format!("https://{raw}"))
}

/// `(Host, Port)` einer Proxy-Angabe: beliebiges Schema (`http`, `socks5`, …)
/// oder gar keines (`proxy.firma.example:8080`). Zugangsdaten fallen weg.
///
/// Ein Passwort darf `/`, `?` oder `#` enthalten (`http://anna:12345/x@proxy:3128`);
/// eine reine URL-Zerlegung würde dann `anna` als Host lesen. Kommt ein `@`
/// vor, zählt deshalb nur, was nach dem LETZTEN `@` steht.
pub fn host_port_from_any(raw: &str) -> Option<(String, u16)> {
    let raw = raw.trim();
    let (scheme, rest) = raw.split_once("://").unwrap_or(("http", raw));
    let host_part = rest.rsplit_once('@').map_or(rest, |(_, host)| host);
    let url = Url::parse(&format!("{scheme}://{host_part}")).ok()?;
    let host = clean_host(url.host_str()?)?;
    let port = url.port().or_else(|| match url.scheme() {
        "http" => Some(80),
        "https" => Some(443),
        "socks5" | "socks5h" | "socks4" | "socks" => Some(1080),
        _ => None,
    })?;
    Some((host, port))
}

/// Die Adresse in der Form, die in den Bericht darf: `https://host[:port]`
/// (Standardport weggelassen). `None`, wenn die Adresse keine Gegenstelle ergibt.
pub fn redact_url(raw: &str) -> Option<String> {
    let ep = endpoint_from_url(raw)?;
    let scheme = if ep.tls { "https" } else { "http" };
    let default = if ep.tls { 443 } else { 80 };
    let host = if ep.host.contains(':') {
        format!("[{}]", ep.host)
    } else {
        ep.host.clone()
    };
    Some(if ep.port == default {
        format!("{scheme}://{host}")
    } else {
        format!("{scheme}://{host}:{}", ep.port)
    })
}

fn re(cell: &'static OnceLock<Regex>, pattern: &str) -> &'static Regex {
    cell.get_or_init(|| Regex::new(pattern).expect("Schwärzungsmuster"))
}

/// Steht der Treffer in einem Hostnamen (`proxy-zentral-rechenzentrum-nord.firma.example`
/// oder `lang-lang-lang-lang-lang-name:3128`)? Dann ist er kein Schlüssel, sondern
/// ein lesbares Namensstück, das die IT braucht. Streng gefasst, damit Schlüssel
/// und Token nicht durchrutschen: Der ganze Namenslauf (Buchstaben, Ziffern,
/// `-`, `.`) muss wie ein Hostname gebaut sein (Teile höchstens 63 Zeichen, nicht
/// mit `-` beginnend oder endend, Endung 2 bis 24 Zeichen, mit Buchstaben
/// beginnend), oder es folgt ein Port (`:3128`).
fn is_hostname_token(s: &str, start: usize, end: usize) -> bool {
    let in_run = |c: char| c.is_ascii_alphanumeric() || c == '-' || c == '.';
    let run_start = s[..start]
        .char_indices()
        .rev()
        .find(|(_, c)| !in_run(*c))
        .map_or(0, |(i, c)| i + c.len_utf8());
    let run_end = s[end..]
        .char_indices()
        .find(|(_, c)| !in_run(*c))
        .map_or(s.len(), |(i, _)| end + i);
    let run = s[run_start..run_end].trim_matches('.');
    let token = &s[start..end];
    if !token.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
        return false;
    }
    let label_ok =
        |l: &str| !l.is_empty() && l.len() <= 63 && !l.starts_with('-') && !l.ends_with('-');
    let labels: Vec<&str> = run.split('.').collect();
    if labels.len() >= 2 && labels.iter().all(|l| label_ok(l)) {
        let tld = labels[labels.len() - 1];
        return (2..=24).contains(&tld.len()) && tld.starts_with(|c: char| c.is_ascii_alphabetic());
    }
    let after = &s[run_end..];
    labels.len() == 1
        && label_ok(run)
        && after
            .strip_prefix(':')
            .is_some_and(|p| p.starts_with(|c: char| c.is_ascii_digit()))
}

/// Schlüsselartige Zeichenfolgen durch `[…]` ersetzen, Hostnamen ausgenommen.
fn scrub_keys(key_re: &Regex, s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut last = 0;
    for m in key_re.find_iter(s) {
        out.push_str(&s[last..m.start()]);
        if is_hostname_token(s, m.start(), m.end()) {
            out.push_str(m.as_str());
        } else {
            out.push_str("[…]");
        }
        last = m.end();
    }
    out.push_str(&s[last..]);
    out
}

/// Freitext bereinigen. Entfernt, in dieser Reihenfolge: Adressen mit Schema
/// (werden auf `https://host` gekürzt, sonst `[Adresse]`), E-Mail-Adressen,
/// Windows-, UNC- und typische Unix-Pfade, schlüsselartige Zeichenfolgen
/// (24 oder mehr Zeichen aus `A–Z a–z 0–9 _ - + / =` ohne Leerzeichen; Teile von
/// Hostnamen sind ausgenommen, siehe [`is_hostname_token`]) sowie Steuerzeichen. Mehrfache Leerzeichen werden zu einem; Länge höchstens 120.
pub fn scrub_text(input: &str) -> String {
    static URL_RE: OnceLock<Regex> = OnceLock::new();
    static MAIL_RE: OnceLock<Regex> = OnceLock::new();
    static WIN_RE: OnceLock<Regex> = OnceLock::new();
    static UNC_RE: OnceLock<Regex> = OnceLock::new();
    static UNIX_RE: OnceLock<Regex> = OnceLock::new();
    static KEY_RE: OnceLock<Regex> = OnceLock::new();

    let cleaned: String = input
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    let s = re(&URL_RE, r"[A-Za-z][A-Za-z0-9+.\-]*://[^\s]+")
        .replace_all(&cleaned, |c: &regex::Captures| {
            redact_url(&c[0]).unwrap_or_else(|| "[Adresse]".to_string())
        });
    let s = re(&MAIL_RE, r"[^\s@]+@[^\s@]+\.[^\s@]+").replace_all(&s, "[E-Mail]");
    let s = re(&WIN_RE, r"\b[A-Za-z]:[\\/][^\s]*").replace_all(&s, "[Pfad]");
    let s = re(&UNC_RE, r"\\\\[^\s]+").replace_all(&s, "[Pfad]");
    let s = re(
        &UNIX_RE,
        r"/(?:home|Users|root|tmp|var|etc|opt|mnt|usr)(?:/[^\s]*)?",
    )
    .replace_all(&s, "[Pfad]");
    let s = scrub_keys(re(&KEY_RE, r"[A-Za-z0-9_\-+/=]{24,}"), &s);
    let collapsed = s.split_whitespace().collect::<Vec<_>>().join(" ");
    collapsed.chars().take(120).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn url_wird_auf_host_und_port_gekuerzt() {
        let ep = endpoint_from_url(
            "https://nutzer:geheim@Kalender.Example.COM:8443/pfad/zum/kalender.ics?token=abc123&x=1#frag",
        )
        .unwrap();
        assert_eq!(ep.host, "kalender.example.com");
        assert_eq!(ep.port, 8443);
        assert!(ep.tls);
        let shown = format!("{ep:?} {}", ep.host_port());
        for secret in [
            "nutzer",
            "geheim",
            "pfad",
            "kalender.ics",
            "token",
            "abc123",
            "frag",
        ] {
            assert!(!shown.contains(secret), "{secret} darf nicht durchkommen");
        }
    }

    #[test]
    fn standardports_und_schema() {
        let https = endpoint_from_url("https://api.example.com/v1").unwrap();
        assert_eq!((https.port, https.tls), (443, true));
        let http = endpoint_from_url("http://lan-server.example/v1").unwrap();
        assert_eq!((http.port, http.tls), (80, false));
    }

    #[test]
    fn fremde_schemata_und_muell_sind_keine_gegenstelle() {
        for raw in [
            "cli://claude",
            "apple-intelligence://local",
            "file:///C:/geheim.txt",
            "ftp://host.example",
            "",
            "   ",
            "kein url",
            "https://",
        ] {
            assert_eq!(endpoint_from_url(raw), None, "{raw}");
        }
    }

    #[test]
    fn ipv6_und_ip_werden_geklammert_angezeigt() {
        let ep = endpoint_from_url("http://[fd00::1]:11434/v1").unwrap();
        assert_eq!(ep.host, "fd00::1");
        assert_eq!(ep.host_port(), "[fd00::1]:11434");
        let ip = endpoint_from_url("http://192.168.1.20:8000/v1").unwrap();
        assert_eq!(ip.host_port(), "192.168.1.20:8000");
    }

    #[test]
    fn host_mit_punkt_am_ende_und_grossschreibung() {
        let ep = endpoint_from_host("Outlook.Office365.com.").unwrap();
        assert_eq!(ep.host, "outlook.office365.com");
        assert_eq!(ep.port, 443);
        assert!(ep.tls);
    }

    #[test]
    fn nackter_host_mit_anhang_wird_trotzdem_gekuerzt() {
        let ep = endpoint_from_host("user:pw@kalender.example/geheim?k=1").unwrap();
        assert_eq!(ep.host, "kalender.example");
        assert!(!format!("{ep:?}").contains("geheim"));
        assert!(!format!("{ep:?}").contains("pw"));
    }

    #[test]
    fn redact_url_zeigt_nur_ursprung() {
        assert_eq!(
            redact_url("https://u:p@host.example/a/b?token=1").as_deref(),
            Some("https://host.example")
        );
        assert_eq!(
            redact_url("http://host.example:8080/x").as_deref(),
            Some("http://host.example:8080")
        );
        assert_eq!(redact_url("keine adresse"), None);
    }

    #[test]
    fn proxy_angaben_ohne_zugangsdaten() {
        assert_eq!(
            host_port_from_any("http://admin:pw123@Proxy.Firma.example:3128/"),
            Some(("proxy.firma.example".to_string(), 3128))
        );
        assert_eq!(
            host_port_from_any("proxy.firma.example:8080"),
            Some(("proxy.firma.example".to_string(), 8080))
        );
        assert_eq!(
            host_port_from_any("socks5://10.0.0.5"),
            Some(("10.0.0.5".to_string(), 1080))
        );
        assert_eq!(host_port_from_any(""), None);
    }

    #[test]
    fn freitext_ohne_adressen_mails_pfade_schluessel() {
        let raw = "Zugriff https://x:y@intern.example/pfad?token=abc von a.b@firma.example \
                   C:\\Users\\Anna\\Dokumente\\a.txt und /home/anna/.ssh/id \
                   Schluessel sk-ABCDEFGHIJKLMNOPQRSTUVWXYZ123456 Ende";
        let out = scrub_text(raw);
        for secret in [
            "x:y",
            "pfad",
            "token",
            "a.b@",
            "Anna",
            "anna",
            "Dokumente",
            ".ssh",
            "ABCDEFGH",
        ] {
            assert!(!out.contains(secret), "{secret} in {out}");
        }
        assert!(out.contains("https://intern.example"), "{out}");
        assert!(out.contains("[E-Mail]"));
        assert!(out.contains("[Pfad]"));
        assert!(out.contains("[…]"));
        assert!(out.ends_with("Ende"));
    }

    #[test]
    fn proxy_mit_schraegstrich_im_passwort_liefert_den_echten_host() {
        for raw in [
            "http://anna:12345/x@proxy:3128",
            "http://anna:12345/x@proxy:3128/",
            "anna:12345/x@proxy:3128",
            "http://anna:p@ss?w#rt/@proxy:3128",
            "http://anna:pw@proxy:3128",
        ] {
            assert_eq!(
                host_port_from_any(raw),
                Some(("proxy".to_string(), 3128)),
                "{raw}"
            );
        }
        // Ohne `@` bleibt alles wie gehabt.
        assert_eq!(
            host_port_from_any("proxy.firma.example:8080/pac"),
            Some(("proxy.firma.example".to_string(), 8080))
        );
        let shown = format!("{:?}", host_port_from_any("http://anna:12345/x@proxy:3128"));
        assert!(!shown.contains("anna") && !shown.contains("12345"));
    }

    #[test]
    fn hostnamen_werden_von_der_schluesselregel_nicht_zerstoert() {
        for host in [
            "proxy-zentral-rechenzentrum-nord.firma.example:3128",
            "proxy-zentral-rechenzentrum-nord.firma.example",
            "lang-lang-lang-lang-lang-name.example",
            "proxy.lang-lang-lang-lang-lang-name.example:8080",
            "proxy-zentral-rechenzentrum-nord:3128",
        ] {
            assert_eq!(scrub_text(host), host, "{host}");
            assert_eq!(
                scrub_text(&format!("Proxy {host} erkannt")),
                format!("Proxy {host} erkannt")
            );
        }
        assert_eq!(scrub_text("[fd00:abcd::1]:3128"), "[fd00:abcd::1]:3128");
    }

    #[test]
    fn schluessel_bleiben_geschwaerzt_auch_mit_punkten_und_satzende() {
        for (raw, gone) in [
            ("Schluessel ABCDEFGHIJKLMNOPQRSTUVWXYZ123456.", "ABCDEFGH"),
            ("Token eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.dBjftJeZ4CVPmB92K27uhbUJU1p1r_wW1gFWFOEjXk", "eyJhbGci"),
            ("Key sk-ABCDEFGHIJKLMNOPQRSTUVWXYZ123456 Ende", "ABCDEFGH"),
            ("lang-lang-lang-lang-lang-name allein", "lang-lang"),
            ("123e4567-e89b-12d3-a456-426614174000", "123e4567"),
            ("ABCDEFGHIJKLMNOPQRSTUVWXYZ123456:abc", "ABCDEFGH"),
        ] {
            let out = scrub_text(raw);
            assert!(!out.contains(gone), "{raw} -> {out}");
            assert!(out.contains("[…]"), "{raw} -> {out}");
        }
    }

    #[test]
    fn freitext_ist_begrenzt_und_ohne_steuerzeichen() {
        let out = scrub_text(&format!("a\nb\t{}", "wort ".repeat(100)));
        assert!(out.chars().count() <= 120);
        assert!(!out.contains('\n') && !out.contains('\t'));
        assert!(out.starts_with("a b wort"));
    }

    #[test]
    fn harmloser_aussteller_bleibt_lesbar() {
        assert_eq!(
            scrub_text("Zscaler Intermediate Root CA (zscloud.net)"),
            "Zscaler Intermediate Root CA (zscloud.net)"
        );
    }
}
