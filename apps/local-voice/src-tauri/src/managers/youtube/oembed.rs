//! Metadaten ueber oEmbed (A2): Titel, Kanal und Adresse des Vorschaubilds
//! eines oeffentlichen Videos, ohne API-Schluessel.
//!
//! Das ist der EINZIGE Netzzugriff dieses Pakets und er laeuft nur, wenn der
//! Nutzer bewusst einen Link einfuegt (siehe `source`). Grenzen:
//! - Zeitlimit 10 s gesamt (Verbindungsaufbau hoechstens 5 s);
//! - Antwort hoechstens 64 KiB, schon beim Empfang abgebrochen;
//! - keine Umleitungen (eine Umleitung waere ein anderer Host, als wir fragen);
//! - kein Cookie, kein Schluessel, kein Referer; der Link im Abruf ist der
//!   bereinigte (nur die ID);
//! - Fehlermeldungen enthalten weder Adresse noch Video-ID.
//!
//! Was aus der Antwort uebernommen wird, ist untrusted: Texte verlieren Steuer-
//! zeichen und werden gekuerzt, Adressen nur uebernommen, wenn sie `https` sind
//! und auf YouTube zeigen (Kanal) bzw. `ytimg.com` (Vorschaubild).

use std::time::Duration;

use reqwest::redirect::Policy;
use reqwest::StatusCode;
use serde_json::Value;
use url::Url;

use super::link::YoutubeRef;
use super::YoutubeError;

#[cfg(test)]
mod tests;

/// Die oEmbed-Adresse von YouTube.
pub const OEMBED_URL: &str = "https://www.youtube.com/oembed";
pub const TIMEOUT: Duration = Duration::from_secs(10);
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
pub const MAX_BODY_BYTES: usize = 64 * 1024;
/// Laengster uebernommener Text (Titel, Kanalname) in Zeichen.
pub const MAX_TEXT_CHARS: usize = 300;
const MAX_URL_CHARS: usize = 500;

/// Test-Umgebungsvariable: eine andere oEmbed-Adresse (lokaler Teststand). Sie
/// gilt NUR in der Sandbox (`LVA_MEETINGS_DIR` gesetzt): in der echten App kann
/// keine Umgebung den Abruf auf einen fremden Host lenken.
pub const OEMBED_OVERRIDE_ENV: &str = "LVA_YOUTUBE_OEMBED_URL";

/// Titel, Kanal und Adressen, wie sie die Besprechung speichert.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VideoMeta {
    pub title: String,
    /// Kanalname; leer, wenn die Antwort keinen nannte.
    pub channel: String,
    pub channel_url: Option<String>,
    pub thumbnail_url: Option<String>,
}

/// Grenzen eines Abrufs; die Vorgabe sind die Werte oben. Tests setzen kleine.
#[derive(Clone, Debug)]
pub struct FetchOpts {
    pub timeout: Duration,
    pub max_bytes: usize,
    /// Proxy-Umgebungsvariablen (`HTTPS_PROXY`, ...) beachten; in Tests aus.
    pub use_env_proxy: bool,
}

impl Default for FetchOpts {
    fn default() -> Self {
        Self {
            timeout: TIMEOUT,
            max_bytes: MAX_BODY_BYTES,
            use_env_proxy: true,
        }
    }
}

/// Rein: welche Adresse gilt? Ein Vorgabewert zaehlt nur in der Sandbox und nur,
/// wenn er eine `http(s)`-Adresse ist.
pub fn oembed_base_from(override_url: Option<&str>, sandbox_dir: Option<&str>) -> String {
    let in_sandbox = sandbox_dir.is_some_and(|d| !d.trim().is_empty());
    if in_sandbox {
        if let Some(candidate) = override_url.map(str::trim) {
            let usable = Url::parse(candidate)
                .ok()
                .is_some_and(|u| matches!(u.scheme(), "http" | "https") && u.host_str().is_some());
            if usable {
                return candidate.to_string();
            }
        }
    }
    OEMBED_URL.to_string()
}

/// Die Adresse aus der Umgebung (siehe `oembed_base_from`).
pub fn oembed_base() -> String {
    oembed_base_from(
        std::env::var(OEMBED_OVERRIDE_ENV).ok().as_deref(),
        std::env::var(crate::managers::meetings::MEETINGS_DIR_ENV)
            .ok()
            .as_deref(),
    )
}

/// `base?url=<bereinigter Link>&format=json`, die Abfrage prozentkodiert.
pub fn request_url(base: &str, video: &YoutubeRef) -> Result<Url, YoutubeError> {
    let mut url = Url::parse(base)
        .ok()
        .filter(|u| matches!(u.scheme(), "http" | "https") && u.host_str().is_some())
        .ok_or_else(|| YoutubeError::Network("Ungültige Adresse des Dienstes.".to_string()))?;
    url.query_pairs_mut()
        .clear()
        .append_pair("url", &video.canonical_url())
        .append_pair("format", "json");
    Ok(url)
}

/// Steuerzeichen und Zeilenumbrueche werden Leerzeichen, Leerraum zusammengezogen,
/// auf `max` Zeichen gekuerzt (mit `…`).
fn clean_text(raw: &str, max: usize) -> String {
    let spaced: String = raw
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    let joined = spaced.split_whitespace().collect::<Vec<_>>().join(" ");
    if joined.chars().count() <= max {
        return joined;
    }
    let mut out: String = joined.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}

/// Eine Adresse aus der Antwort: nur `https`, ohne Zugangsdaten und Port, Host
/// muss `host_ok` bestehen.
fn safe_url(raw: Option<&str>, host_ok: impl Fn(&str) -> bool) -> Option<String> {
    let raw = raw?.trim();
    if raw.is_empty() || raw.chars().count() > MAX_URL_CHARS {
        return None;
    }
    let url = Url::parse(raw).ok()?;
    let host = url.host_str()?.to_ascii_lowercase();
    let clean = url.scheme() == "https"
        && url.username().is_empty()
        && url.password().is_none()
        && url.port().is_none()
        && host_ok(&host);
    clean.then(|| url.to_string())
}

fn is_youtube_host(host: &str) -> bool {
    matches!(host, "youtube.com" | "www.youtube.com" | "m.youtube.com")
}

fn is_thumbnail_host(host: &str) -> bool {
    host == "ytimg.com" || host.ends_with(".ytimg.com")
}

/// Die Antwort auswerten (rein): JSON-Objekt, Art `video` (falls angegeben),
/// Titel Pflicht, Kanal freiwillig.
pub fn parse_response(body: &str) -> Result<VideoMeta, YoutubeError> {
    let bad = |m: &str| YoutubeError::BadResponse(m.to_string());
    let value: Value = serde_json::from_str(body).map_err(|_| bad("kein JSON"))?;
    let Some(object) = value.as_object() else {
        return Err(bad("kein JSON-Objekt"));
    };
    if let Some(kind) = object.get("type").and_then(Value::as_str) {
        if kind != "video" {
            return Err(bad("kein Video"));
        }
    }
    let text = |key: &str| {
        object
            .get(key)
            .and_then(Value::as_str)
            .map(|s| clean_text(s, MAX_TEXT_CHARS))
    };
    let title = text("title")
        .filter(|t| !t.is_empty())
        .ok_or_else(|| bad("ohne Titel"))?;
    let str_of = |key: &str| object.get(key).and_then(Value::as_str);
    Ok(VideoMeta {
        title,
        channel: text("author_name").unwrap_or_default(),
        channel_url: safe_url(str_of("author_url"), is_youtube_host),
        thumbnail_url: safe_url(str_of("thumbnail_url"), is_thumbnail_host),
    })
}

fn map_send_error(e: reqwest::Error, use_env_proxy: bool) -> YoutubeError {
    let e = e.without_url();
    if e.is_timeout() {
        return YoutubeError::Timeout;
    }
    // Keine rohe Fehlerkette weitergeben: sie kann Adressen enthalten.
    if e.is_connect() {
        let hint = if use_env_proxy {
            " Der Windows-Systemproxy wird nicht verwendet, nur HTTPS_PROXY und HTTP_PROXY."
        } else {
            ""
        };
        return YoutubeError::Network(format!(
            "Die Verbindung kam nicht zustande (Internet, Proxy oder Firewall prüfen).{hint}"
        ));
    }
    YoutubeError::Network("Die Anfrage ist fehlgeschlagen.".to_string())
}

/// Ruft Titel, Kanal und Vorschaubild ab (siehe Moduldoku).
pub async fn fetch(
    base: &str,
    video: &YoutubeRef,
    opts: &FetchOpts,
) -> Result<VideoMeta, YoutubeError> {
    let url = request_url(base, video)?;
    let mut builder = reqwest::Client::builder()
        .timeout(opts.timeout)
        .connect_timeout(CONNECT_TIMEOUT.min(opts.timeout))
        .redirect(Policy::none())
        .user_agent(concat!(
            "LocalVoiceAI/",
            env!("CARGO_PKG_VERSION"),
            " (youtube-source)"
        ));
    if !opts.use_env_proxy {
        builder = builder.no_proxy();
    }
    let client = builder
        .build()
        .map_err(|e| map_send_error(e, opts.use_env_proxy))?;
    let mut response = client
        .get(url)
        .header(reqwest::header::ACCEPT, "application/json")
        .send()
        .await
        .map_err(|e| map_send_error(e, opts.use_env_proxy))?;

    let status = response.status();
    if status.is_redirection() {
        return Err(YoutubeError::BadResponse(
            "unerwartete Umleitung".to_string(),
        ));
    }
    match status {
        s if s.is_success() => {}
        StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN | StatusCode::NOT_FOUND => {
            return Err(YoutubeError::Unavailable)
        }
        StatusCode::TOO_MANY_REQUESTS => return Err(YoutubeError::RateLimited),
        other => return Err(YoutubeError::Http(other.as_u16())),
    }
    let too_big = || YoutubeError::BadResponse("Antwort zu groß".to_string());
    if response
        .content_length()
        .is_some_and(|len| len > opts.max_bytes as u64)
    {
        return Err(too_big());
    }
    // Stueckweise lesen und an der Grenze abbrechen: ein Server ohne
    // Content-Length darf den Speicher nicht fuellen.
    let mut body: Vec<u8> = Vec::new();
    loop {
        match response.chunk().await {
            Ok(Some(chunk)) => {
                if body.len() + chunk.len() > opts.max_bytes {
                    return Err(too_big());
                }
                body.extend_from_slice(&chunk);
            }
            Ok(None) => break,
            Err(e) => return Err(map_send_error(e, opts.use_env_proxy)),
        }
    }
    let text = String::from_utf8_lossy(&body);
    parse_response(&text)
}
