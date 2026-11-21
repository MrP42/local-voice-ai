//! Erreichbarkeit der offiziellen CLI-Installer vor dem Oeffnen eines Fensters.

use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Erreichbarkeit {
    Ok,
    Gesperrt,
    ProxyAnmeldung,
    ProxyFehler,
    Unklar,
}

impl Erreichbarkeit {
    pub fn code(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Gesperrt => "blocked",
            Self::ProxyAnmeldung => "proxyAuth",
            Self::ProxyFehler => "proxy",
            Self::Unklar => "unknown",
        }
    }
}

pub fn aus_antwort(
    status: u16,
    server: Option<&str>,
    squid_kopf: bool,
    koerper_anfang: &str,
) -> Erreichbarkeit {
    if status == 407 {
        return Erreichbarkeit::ProxyAnmeldung;
    }
    let bytes = koerper_anfang.as_bytes();
    let anfang = String::from_utf8_lossy(&bytes[..bytes.len().min(4096)]).to_ascii_lowercase();
    if matches!(status, 403 | 451)
        || (server.is_some_and(|s| s.to_ascii_lowercase().contains("squid"))
            && !(200..300).contains(&status))
        || squid_kopf
        || anfang.contains("cache access denied")
        || anfang.contains("err_access_denied")
    {
        Erreichbarkeit::Gesperrt
    } else if (200..400).contains(&status) {
        Erreichbarkeit::Ok
    } else {
        Erreichbarkeit::Unklar
    }
}

pub fn aus_fehler(text: &str) -> Erreichbarkeit {
    use super::fehler::Netzfehler;
    match super::fehler::einordnen(text) {
        Netzfehler::ProxyAnmeldung => Erreichbarkeit::ProxyAnmeldung,
        Netzfehler::Gesperrt => Erreichbarkeit::Gesperrt,
        Netzfehler::ProxyFehler => Erreichbarkeit::ProxyFehler,
        _ => Erreichbarkeit::Unklar,
    }
}

pub async fn pruefe(url: &str) -> (Erreichbarkeit, super::proxy::Weg) {
    let weg = super::proxy::weg_fuer_async(url).await;
    let ergebnis = pruefe_mit_weg(url, &weg).await;
    (ergebnis, weg)
}

pub(crate) async fn pruefe_mit_weg(url: &str, weg: &super::proxy::Weg) -> Erreichbarkeit {
    let Ok(client) = super::proxy::anwenden(reqwest::Client::builder(), weg)
        .timeout(Duration::from_secs(10))
        // Keine Weiterleitungen: geprueft wird nur der feste Installer-Host; eine
        // 3xx-Antwort zaehlt als erreichbar, fremden Hosts wird nie gefolgt.
        .redirect(reqwest::redirect::Policy::none())
        .build()
    else {
        return Erreichbarkeit::Unklar;
    };
    let mut response = match client.get(url).send().await {
        Ok(response) => response,
        Err(e) => return aus_fehler(&format!("{e:?}")),
    };
    let status = response.status().as_u16();
    let server = response
        .headers()
        .get(reqwest::header::SERVER)
        .and_then(|h| h.to_str().ok())
        .map(str::to_string);
    let squid_kopf = response.headers().contains_key("x-squid-error");
    let aus_koepfen = aus_antwort(status, server.as_deref(), squid_kopf, "");
    if matches!(
        aus_koepfen,
        Erreichbarkeit::Gesperrt | Erreichbarkeit::ProxyAnmeldung
    ) {
        return aus_koepfen;
    }
    let mut anfang = Vec::new();
    while anfang.len() < 4096 {
        match response.chunk().await {
            Ok(Some(stueck)) => {
                let n = stueck.len().min(4096 - anfang.len());
                anfang.extend_from_slice(&stueck[..n]);
            }
            // Status, Koepfe und bereits gelesener Anfang bleiben auch bei Lesefehlern erhalten.
            Ok(None) | Err(_) => break,
        }
    }
    aus_antwort(
        status,
        server.as_deref(),
        squid_kopf,
        &String::from_utf8_lossy(&anfang),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::netz::proxy::tests::{antwort, gesehen, testserver};
    use crate::netz::proxy::Weg;

    #[test]
    fn antworten_erkennen_sperren_auch_bei_status_200() {
        use Erreichbarkeit::*;
        for (status, server, kopf, koerper, erwartet) in [
            (407, None, false, "", ProxyAnmeldung),
            (403, None, false, "", Gesperrt),
            (451, None, false, "", Gesperrt),
            (503, Some("squid/4.10"), false, "", Gesperrt),
            (503, Some("SQUID/4.10"), false, "", Gesperrt),
            (200, None, true, "", Gesperrt),
            (
                200,
                None,
                false,
                "<title>ERROR: Cache Access Denied</title>",
                Gesperrt,
            ),
            (200, None, false, "ERR_ACCESS_DENIED", Gesperrt),
            (200, None, false, "Installer", Ok),
            (200, Some("squid/4.10"), false, "", Ok),
            (302, None, false, "", Ok),
            (500, None, false, "", Unklar),
        ] {
            assert_eq!(aus_antwort(status, server, kopf, koerper), erwartet);
        }
    }

    #[test]
    fn fehler_erkennen_tunnel_und_proxyanmeldung() {
        assert_eq!(
            aus_fehler("unsuccessful tunnel"),
            Erreichbarkeit::ProxyFehler
        );
        assert_eq!(
            aus_fehler("server returned HTTP 407 Proxy Authentication Required"),
            Erreichbarkeit::ProxyAnmeldung
        );
        assert_eq!(aus_fehler("HTTP 403 Forbidden"), Erreichbarkeit::Gesperrt);
        assert_eq!(aus_fehler("operation timed out"), Erreichbarkeit::Unklar);
    }

    #[test]
    fn codes_bleiben_stabil() {
        for (art, code) in [
            (Erreichbarkeit::Ok, "ok"),
            (Erreichbarkeit::Gesperrt, "blocked"),
            (Erreichbarkeit::ProxyAnmeldung, "proxyAuth"),
            (Erreichbarkeit::ProxyFehler, "proxy"),
            (Erreichbarkeit::Unklar, "unknown"),
        ] {
            assert_eq!(art.code(), code);
        }
    }

    #[tokio::test]
    async fn proxy_sperrseite_verhindert_die_einrichtung() {
        let (proxy, _) = testserver(antwort(
            "403 Forbidden",
            "Server: squid/4.10\r\nX-Squid-Error: ERR_ACCESS_DENIED 0\r\n",
        ))
        .await;
        assert_eq!(
            pruefe_mit_weg("http://example.invalid/install.ps1", &Weg::Ueber(proxy)).await,
            Erreichbarkeit::Gesperrt
        );
    }

    #[tokio::test]
    async fn sperrstatus_bleibt_bei_abgebrochenem_koerper_erhalten() {
        let (proxy, _) = testserver(
            "HTTP/1.1 403 Forbidden\r\ncontent-length: 100000\r\nconnection: close\r\n\r\n"
                .to_string(),
        )
        .await;
        assert_eq!(
            pruefe_mit_weg("http://example.invalid/install.ps1", &Weg::Ueber(proxy)).await,
            Erreichbarkeit::Gesperrt
        );
    }

    #[tokio::test]
    async fn erreichbarer_status_bleibt_bei_abgebrochenem_koerper_erhalten() {
        for (anfang, erwartet) in [
            ("", Erreichbarkeit::Ok),
            ("ERR_ACCESS_DENIED", Erreichbarkeit::Gesperrt),
        ] {
            let roh = format!(
                "HTTP/1.1 200 OK\r\ncontent-length: 100000\r\nconnection: close\r\n\r\n{anfang}"
            );
            let (proxy, _) = testserver(roh).await;
            assert_eq!(
                pruefe_mit_weg("http://example.invalid/install.ps1", &Weg::Ueber(proxy)).await,
                erwartet
            );
        }
    }

    #[tokio::test]
    async fn weiterleitung_wird_nicht_verfolgt() {
        let (proxy, log) =
            testserver(antwort("302 Found", "Location: http://127.0.0.1:1/x\r\n")).await;
        assert_eq!(
            pruefe_mit_weg("http://example.invalid/install.ps1", &Weg::Ueber(proxy)).await,
            Erreichbarkeit::Ok
        );
        assert_eq!(gesehen(&log).len(), 1);
    }

    #[tokio::test]
    async fn proxy_laesst_den_installer_durch() {
        let (proxy, _) = testserver(antwort("200 OK", "")).await;
        assert_eq!(
            pruefe_mit_weg("http://example.invalid/install.ps1", &Weg::Ueber(proxy)).await,
            Erreichbarkeit::Ok
        );
    }
}
