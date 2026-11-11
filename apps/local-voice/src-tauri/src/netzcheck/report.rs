//! Berichtstext der Netzwerkprüfung zum Kopieren für die IT (lva#496). Rein.
//!
//! Inhalt: Hostname, Port, Ergebnis, App-Version, Betriebssystem. Nie Token,
//! Schlüssel, Nutzernamen, Pfade oder Inhalte: Hosts und Ports stammen aus
//! [`super::redact`], jeder Freitext (Betriebssystem, Aussteller) geht durch
//! [`super::redact::scrub_text`]. Nutzerbeschriftungen von Verbindungen stehen
//! bewusst nicht im Bericht, nur der Zweck.

use super::judge::{category_name, impact_text, reason_text, state_text};
use super::proxy::{ProxyInfo, ProxySource};
use super::redact::{format_host_port, scrub_text};
use super::{Row, State, PROBE_TIMEOUT};

/// Angaben zum Rechner, vom Aufrufer eingesetzt.
#[derive(Debug, Clone)]
pub struct ReportMeta {
    pub app_version: String,
    /// z. B. `windows 10.0.26100 (x86_64)`.
    pub os: String,
    /// UTC, z. B. `2026-11-11 13:05`.
    pub generated_at: String,
}

fn ms(v: Option<u32>) -> String {
    v.map_or_else(|| "–".to_string(), |v| format!("{v} ms"))
}

fn proxy_line(proxy: &ProxyInfo) -> String {
    if !proxy.configured() {
        return "nicht erkannt (Umgebungsvariablen und Windows-Internetoptionen geprüft)"
            .to_string();
    }
    let mut parts: Vec<String> = proxy
        .entries
        .iter()
        .map(|e| {
            let source = match e.source {
                ProxySource::Env => "Umgebungsvariable",
                ProxySource::System => "Windows-Internetoptionen",
            };
            let scope = match e.scope.as_str() {
                "http" => "HTTP",
                "https" => "HTTPS",
                "socks" => "SOCKS",
                _ => "alle Protokolle",
            };
            format!("{scope} über {} ({source})", scrub_text(&e.endpoint))
        })
        .collect();
    if proxy.pac {
        parts.push("Konfigurationsskript (PAC) eingetragen".to_string());
    }
    format!("erkannt: {}", parts.join("; "))
}

/// Der Bericht als Text.
pub fn build(rows: &[Row], proxy: &ProxyInfo, meta: &ReportMeta) -> String {
    let mut out = String::new();
    out.push_str("Local Voice – Netzwerkprüfung\n");
    out.push_str("=============================\n");
    out.push_str(&format!("App-Version: {}\n", scrub_text(&meta.app_version)));
    out.push_str(&format!("Betriebssystem: {}\n", scrub_text(&meta.os)));
    out.push_str(&format!(
        "Zeitpunkt (UTC): {}\n",
        scrub_text(&meta.generated_at)
    ));
    out.push_str(&format!(
        "Zeitlimit je Gegenstelle: {} s, alle gleichzeitig\n",
        PROBE_TIMEOUT.as_secs()
    ));
    out.push_str(&format!("System-Proxy: {}\n", proxy_line(proxy)));
    out.push('\n');

    if rows.is_empty() {
        out.push_str("Es sind keine Gegenstellen eingerichtet, die geprüft werden müssten.\n");
    } else {
        out.push_str("Ergebnis je Gegenstelle\n");
        out.push_str("-----------------------\n");
        for (i, r) in rows.iter().enumerate() {
            let scheme = if r.tls { "TLS" } else { "ohne TLS" };
            out.push_str(&format!(
                "{}. [{}] {} – {} ({scheme})\n",
                i + 1,
                state_text(r.state),
                category_name(r.category),
                format_host_port(&r.host, r.port),
            ));
            if let Some(reason) = r.reason {
                out.push_str(&format!("   Grund: {}\n", reason_text(reason)));
            }
            if let Some(issuer) = &r.issuer {
                out.push_str(&format!(
                    "   Aussteller des Zertifikats: {}\n",
                    scrub_text(issuer)
                ));
            }
            let status = r
                .http_status
                .map_or_else(|| "–".to_string(), |s| format!("HTTP {s}"));
            out.push_str(&format!(
                "   Messung: Namensauflösung {}, TLS-Aufbau {}, {status}, gesamt {}\n",
                ms(r.dns_ms),
                ms(r.tls_ms),
                r.total_ms.to_string() + " ms",
            ));
            out.push_str(&format!(
                "   Auswirkung: {}\n",
                impact_text(r.category, r.state)
            ));
        }
        let blocked: Vec<String> = rows
            .iter()
            .filter(|r| r.state != State::Reachable)
            .map(|r| format_host_port(&r.host, r.port))
            .fold(Vec::new(), |mut acc, h| {
                if !acc.contains(&h) {
                    acc.push(h);
                }
                acc
            });
        out.push('\n');
        if blocked.is_empty() {
            out.push_str("Alle geprüften Gegenstellen sind erreichbar.\n");
        } else {
            out.push_str("Für die IT: Diese Ziele bitte ausgehend (TCP) freigeben bzw. prüfen:\n");
            for h in blocked {
                out.push_str(&format!("  - {h}\n"));
            }
        }
    }
    out.push('\n');
    out.push_str(
        "Der Bericht enthält nur Hostnamen, Ports und Messwerte. Zugangsdaten, Schlüssel, \
         Nutzernamen, Pfade und Inhalte sind nicht enthalten; Adressen sind auf Host und Port gekürzt.\n",
    );
    out
}

#[cfg(test)]
mod tests {
    use super::super::proxy::ProxyEntry;
    use super::super::redact::endpoint_from_url;
    use super::super::targets::{derive_targets, Service, Setup};
    use super::super::{judge, Category, Reason, State};
    use super::*;

    fn meta() -> ReportMeta {
        ReportMeta {
            app_version: "0.24.1".into(),
            os: "windows 10.0.26100 (x86_64)".into(),
            generated_at: "2026-11-11 13:05".into(),
        }
    }

    fn row(category: Category, state: State, reason: Option<Reason>, host: &str) -> Row {
        Row {
            category,
            host: host.into(),
            port: 443,
            tls: true,
            state,
            reason,
            dns_ms: Some(12),
            tls_ms: None,
            http_status: None,
            total_ms: 5000,
            issuer: None,
        }
    }

    #[test]
    fn bericht_enthaelt_host_port_ergebnis_version_und_system() {
        let rows = vec![
            row(
                Category::SyncHub,
                State::Reachable,
                None,
                "portal.example.com",
            ),
            row(
                Category::Llm,
                State::Unreachable,
                Some(Reason::DnsFailed),
                "api.example.com",
            ),
        ];
        let text = build(&rows, &ProxyInfo::default(), &meta());
        assert!(text.contains("App-Version: 0.24.1"));
        assert!(text.contains("Betriebssystem: windows 10.0.26100 (x86_64)"));
        assert!(text.contains("[erreichbar] Geräteabgleich – portal.example.com:443"));
        assert!(text.contains("[nicht erreichbar] Sprachmodell-Anbieter – api.example.com:443"));
        assert!(text.contains("Grund: Namensauflösung fehlgeschlagen"));
        assert!(text.contains("Auswirkung: Zusammenfassungen und Nachbearbeitung"));
        assert!(text.contains("  - api.example.com:443"));
        assert!(!text.contains("  - portal.example.com:443"));
        assert!(text.contains("System-Proxy: nicht erkannt"));
    }

    #[test]
    fn leerer_bericht_sagt_es() {
        let text = build(&[], &ProxyInfo::default(), &meta());
        assert!(text.contains("keine Gegenstellen eingerichtet"));
        assert!(!text.contains("Für die IT"));
    }

    #[test]
    fn proxy_wird_ohne_zugangsdaten_genannt() {
        let proxy = ProxyInfo {
            entries: vec![ProxyEntry {
                source: ProxySource::System,
                scope: "https".into(),
                endpoint: "proxy.firma.example:3128".into(),
            }],
            pac: true,
        };
        let text = build(&[], &proxy, &meta());
        assert!(text.contains("HTTPS über proxy.firma.example:3128 (Windows-Internetoptionen)"));
        assert!(text.contains("Konfigurationsskript (PAC)"));
    }

    #[test]
    fn aussteller_nur_geschwaerzt() {
        let mut r = row(
            Category::Update,
            State::Limited,
            Some(Reason::TlsInspection),
            "github.com",
        );
        r.http_status = Some(200);
        r.issuer = Some("Firma Proxy CA admin@firma.example C:\\Users\\Anna\\ca.pem".into());
        let text = build(&[r], &ProxyInfo::default(), &meta());
        assert!(text.contains("Aussteller des Zertifikats: Firma Proxy CA [E-Mail] [Pfad]"));
        assert!(!text.contains("Anna") && !text.contains("admin@"));
    }

    /// Ende-zu-Ende: Einstellungen mit Geheimnissen -> Gegenstellen -> Zeilen -> Bericht.
    #[test]
    fn bericht_aus_einrichtung_mit_geheimnissen_bleibt_sauber() {
        let setup = Setup {
            sync_hub_url: Some(
                "https://anna.mueller:Pa55wort@hub.firma.example:8443/api/v1?token=SUPERGEHEIM"
                    .into(),
            ),
            calendar_ics_hosts: vec![
                "kal.firma.example/privat/GEHEIMER-SCHLUESSEL.ics?key=ZUGRIFF".into(),
            ],
            llm_connections: vec![Service {
                url: "https://llm.firma.example/v1?api_key=sk-GEHEIMGEHEIMGEHEIMGEHEIM".into(),
                enabled: true,
                allow_remote: false,
            }],
            ..Setup::default()
        };
        let targets = derive_targets(&setup);
        let rows: Vec<Row> = targets
            .iter()
            .map(|t| {
                judge::row(
                    t,
                    &crate::netzcheck::ProbeOutcome {
                        failure: Some(crate::netzcheck::Failure::Dns),
                        total_ms: 40,
                        ..Default::default()
                    },
                )
            })
            .collect();
        let text = build(&rows, &ProxyInfo::default(), &meta());
        for secret in [
            "anna",
            "mueller",
            "Pa55wort",
            "SUPERGEHEIM",
            "token",
            "privat",
            "GEHEIMER",
            "ZUGRIFF",
            "key=",
            "api_key",
            "sk-",
            "/api/v1",
            ".ics",
        ] {
            assert!(!text.contains(secret), "{secret} im Bericht");
        }
        assert!(text.contains("hub.firma.example:8443"));
        assert!(text.contains("kal.firma.example:443"));
        assert!(text.contains("llm.firma.example:443"));
        // Der Endpunkt selbst kennt nur Host und Port.
        assert_eq!(
            endpoint_from_url("https://x.example/y?z=1")
                .unwrap()
                .host_port(),
            "x.example:443"
        );
    }
}
