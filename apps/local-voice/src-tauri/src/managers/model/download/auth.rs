//! Zugang (Token) fuer einzelne Downloads, z. B. das Hugging-Face-Token fuer gated Modelle.
//!
//! Regeln, jede durch Tests in `tests.rs` festgehalten:
//!
//! * Das Token geht an GENAU EINEN Ursprung (Schema, Host, Port), nie an einen anderen.
//!   Fuer Hugging Face ist das `https://huggingface.co` (Port 443); eine Unterdomain,
//!   ein Port oder `http` zaehlen als anderer Ursprung.
//! * Weiterleitungen folgt der Downloader hier SELBST (`redirect::Policy::none()`): bei jedem
//!   Schritt wird neu entschieden, ob das Token mitgeht. Hugging Face leitet grosse Dateien
//!   auf ein CDN anderer Hosts um; dort steht nie ein Token im Kopf. Der `Range`-Kopf bleibt
//!   erhalten (Fortsetzen).
//! * Kein Rueckschritt von https auf http, nur http(s), keine Zugangsdaten in der Adresse,
//!   hoechstens [`MAX_REDIRECTS`] Schritte.
//! * Das Token steht in keiner Fehlermeldung, keinem `Debug` und keinem Log: [`Secret`]
//!   zeigt nur `***`, der Kopf ist als sensibel markiert, Fehler tragen keine Adresse
//!   (`without_url`: eine signierte CDN-Adresse waere selbst ein Geheimnis auf Zeit).

use std::fmt;

use anyhow::{anyhow, Result};
use reqwest::header::{HeaderValue, AUTHORIZATION, LOCATION, RANGE};
use reqwest::{StatusCode, Url};

/// So viele Weiterleitungen werden hoechstens gefolgt.
const MAX_REDIRECTS: usize = 5;

/// Ein geheimer Wert. `Debug` zeigt ihn nie; es gibt kein `Display`.
#[derive(Clone)]
pub(crate) struct Secret(String);

impl Secret {
    /// `None` fuer leer, zu lang (ueber 512 Zeichen) oder mit anderem als sichtbarem ASCII
    /// (Leerzeichen, Zeilenumbrueche, Umlaute): so kann kein Wert einen Kopf einschleusen.
    /// Rand-Leerraum (aus einer Datei oder Umgebungsvariable) wird abgeschnitten.
    pub(crate) fn new(raw: &str) -> Option<Self> {
        let t = raw.trim();
        let ok = !t.is_empty() && t.len() <= 512 && t.bytes().all(|b| (0x21..=0x7e).contains(&b));
        ok.then(|| Self(t.to_string()))
    }

    /// Nur fuer Tests: der Wert (Produktionscode hat keinen Zugriff darauf ausser dem Kopf).
    #[cfg(test)]
    pub(crate) fn expose_for_test(&self) -> &str {
        &self.0
    }

    fn bearer(&self) -> Option<HeaderValue> {
        let mut value = HeaderValue::from_str(&format!("Bearer {}", self.0)).ok()?;
        value.set_sensitive(true);
        Some(value)
    }

    /// Der Wert als Inhalt eines eigenen Kopfes (z. B. `x-api-key` bei Gradium), als sensibel
    /// markiert: reqwest und `Debug` zeigen ihn nie. Der Aufrufer entscheidet ueber den Ursprung
    /// (`Origin::of`), an den der Kopf gehen darf.
    pub(crate) fn header_value(&self) -> Option<HeaderValue> {
        let mut value = HeaderValue::from_str(&self.0).ok()?;
        value.set_sensitive(true);
        Some(value)
    }

    /// `text` ohne den geheimen Wert (jedes Vorkommen wird zu `***`): fuer Fremdtext aus
    /// Antworten, der in eine Meldung soll. Ein Server, der den Schluessel zurueckschickt,
    /// bringt ihn so nie in eine Fehlermeldung.
    pub(crate) fn scrub(&self, text: &str) -> String {
        text.replace(&self.0, "***")
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Secret(***)")
    }
}

/// Schema, Host und Port: der Ort, an den ein Token gehen darf.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Origin {
    https: bool,
    host: String,
    port: u16,
}

impl Origin {
    pub(crate) fn huggingface() -> Self {
        Self {
            https: true,
            host: "huggingface.co".to_string(),
            port: 443,
        }
    }

    /// Die API von Gradium (`https://api.gradium.ai`, Port 443): der einzige Ort, an den der
    /// Gradium-Schluessel geht.
    pub(crate) fn gradium() -> Self {
        Self {
            https: true,
            host: "api.gradium.ai".to_string(),
            port: 443,
        }
    }

    /// Nur fuer Tests (Fake-Server auf Loopback).
    #[cfg(test)]
    pub(crate) fn plain_http(host: &str, port: u16) -> Self {
        Self {
            https: false,
            host: host.to_ascii_lowercase(),
            port,
        }
    }

    pub(crate) fn of(url: &Url) -> Option<Self> {
        let https = match url.scheme() {
            "https" => true,
            "http" => false,
            _ => return None,
        };
        Some(Self {
            https,
            host: url.host_str()?.to_ascii_lowercase(),
            port: url.port_or_known_default()?,
        })
    }
}

/// Token und der EINE Ursprung, an den es gehen darf.
#[derive(Debug, Clone)]
pub(crate) struct DownloadAuth {
    token: Secret,
    origin: Origin,
}

impl DownloadAuth {
    pub(crate) fn new(token: Secret, origin: Origin) -> Self {
        Self { token, origin }
    }

    pub(crate) fn huggingface(token: Secret) -> Self {
        Self::new(token, Origin::huggingface())
    }

    /// Darf das Token an diese Adresse? Nur an genau den freigegebenen Ursprung.
    pub(crate) fn applies_to(&self, url: &Url) -> bool {
        Origin::of(url).as_ref() == Some(&self.origin)
    }
}

/// Der Server hat mit einem Fehlerstatus geantwortet. Eigener Typ, damit Aufrufer 401/403
/// (gesperrt, Anmeldung fehlt) von Netzfehlern trennen koennen; der Text bleibt wie zuvor.
#[derive(Debug)]
pub(crate) struct HttpStatusError(pub(crate) StatusCode);

impl fmt::Display for HttpStatusError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "server returned HTTP {}", self.0)
    }
}

impl std::error::Error for HttpStatusError {}

/// Naechstes Ziel einer Weiterleitung. Nur http(s), kein Rueckschritt von https auf http, keine
/// Zugangsdaten in der Adresse. `location` darf relativ sein.
pub(crate) fn redirect_target(current: &Url, location: &str) -> Result<Url> {
    let next = current
        .join(location)
        .map_err(|_| anyhow!("redirect to an invalid address"))?;
    match next.scheme() {
        "https" => {}
        "http" if current.scheme() == "http" => {}
        "http" => return Err(anyhow!("redirect from https to http refused")),
        other => return Err(anyhow!("redirect to a forbidden scheme: {other}")),
    }
    if !next.username().is_empty() || next.password().is_some() {
        return Err(anyhow!("redirect to an address with credentials refused"));
    }
    Ok(next)
}

fn is_followed_redirect(status: StatusCode) -> bool {
    matches!(
        status,
        StatusCode::MOVED_PERMANENTLY
            | StatusCode::FOUND
            | StatusCode::SEE_OTHER
            | StatusCode::TEMPORARY_REDIRECT
            | StatusCode::PERMANENT_REDIRECT
    )
}

/// GET mit `Range` ab `range_from` (0 = ganz). Folgt Weiterleitungen selbst; das Token geht nur
/// an Adressen des freigegebenen Ursprungs. `client` muss mit `redirect::Policy::none()`
/// gebaut sein, sonst folgte er selbst.
pub(crate) async fn send_following(
    client: &reqwest::Client,
    url: &str,
    range_from: u64,
    auth: &DownloadAuth,
) -> Result<reqwest::Response> {
    let mut current = Url::parse(url).map_err(|_| anyhow!("invalid download address"))?;
    for _ in 0..=MAX_REDIRECTS {
        let mut request = client.get(current.clone());
        if range_from > 0 {
            request = request.header(RANGE, format!("bytes={range_from}-"));
        }
        if auth.applies_to(&current) {
            if let Some(value) = auth.token.bearer() {
                request = request.header(AUTHORIZATION, value);
            }
        }
        let response = request.send().await.map_err(|e| e.without_url())?;
        if !is_followed_redirect(response.status()) {
            return Ok(response);
        }
        let Some(location) = response
            .headers()
            .get(LOCATION)
            .and_then(|v| v.to_str().ok())
        else {
            return Ok(response); // Weiterleitung ohne Ziel: der Aufrufer meldet den Status
        };
        current = redirect_target(&current, location)?;
    }
    Err(anyhow!("too many redirects"))
}
