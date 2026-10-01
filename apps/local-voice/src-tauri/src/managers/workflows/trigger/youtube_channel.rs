//! Ausloeser „Neues Video im Kanal“ (B3, AK11 Ausloeserteil): fragt den OFFIZIELLEN RSS-Feed
//! eines YouTube-Kanals ab und startet je neuem Video einen Lauf.
//!
//! ```json
//! {"type": "youtube.channel_new_video", "channel_id": "UCxxxxxxxxxxxxxxxxxxxxxx",
//!  "poll_minutes": 60, "backfill": 0}
//! ```
//!
//! # Quelle und Netzverkehr
//!
//! Nur `https://www.youtube.com/feeds/videos.xml?channel_id=<ID>` (Atom, ohne API-Schluessel,
//! ohne Anmeldung, ohne `yt-dlp`). Kein Cookie, kein Referer, keine Umleitung, 10 s Zeitlimit,
//! Antwort hoechstens 1 MiB (schon beim Empfang abgebrochen), Texte untrusted (Steuerzeichen
//! weg, gekuerzt), DTD und Entitaetsdefinitionen werden abgelehnt (kein XML-Bombenrisiko). Der
//! Feed nennt die 15 neuesten Videos. Ohne einen eingeschalteten Ablauf mit diesem Ausloeser
//! geht nichts ins Netz (Offline-Pfad, QG5 des Goals A). Ist die Integration „YouTube“
//! ausgeschaltet, ruht der Abruf (Hauptschalter des Nutzers).
//!
//! # Genau einmal je Video, keine Flut
//!
//! - **Lauf-Schluessel** `yt:<kanal>:<video>`; die Engine macht daraus je Ablauf hoechstens einen
//!   Lauf. Das Ledger (`ledger`: `yt:<ablauf>:<kanal>:<video>`) haelt zusaetzlich fest, was
//!   gesehen wurde, auch nach der Aufbewahrungsgrenze der Laeufe.
//! - **Der erste Abruf** eines Ablaufs fuer einen Kanal (Marke `ytinit:`) markiert die
//!   vorhandenen Videos als BEKANNT und startet KEINEN Lauf (sonst liefe ein neuer Ablauf
//!   gleich 15 Mal). Wer die neuesten `n` Videos gleich verarbeiten will, setzt `backfill`
//!   (0 bis 15, Vorgabe 0): dann bekommen genau diese ihren Lauf, der Rest wird bekannt.
//! - Danach startet jedes neue Video genau einmal, aelteste zuerst. Ein Abruf, der nichts
//!   Neues bringt, ist ein No-op; ein Neustart wiederholt keinen Lauf.
//! - **Einreihen scheitert** (Platte voll, Datenbank gesperrt): das Video bleibt UNBEKANNT und
//!   wird beim naechsten Abruf (nach hoechstens 5 min) wieder angeboten; nichts geht verloren.
//!
//! # Intervall und Ausfall
//!
//! `poll_minutes` (15 bis 1440, Vorgabe 60) je Kanal, im Speicher gefuehrt: nach einem Neustart
//! wird beim ersten Takt abgerufen (Nachholen aus dem Ledger). Es gibt hoechstens
//! [`MAX_FETCH_PER_TICK`] Abrufe je Takt (der Rest folgt im naechsten).
//! **Ausfall** (R9: YouTube hat Feeds zeitweise mit 404 beantwortet): jeder Fehlschlag wird
//! gezaehlt; nach [`OUTAGE_AFTER`] Fehlschlaegen in Folge steht ein Ausfall im Bericht
//! (`TickReport::errors`, Log) und als Audit-Eintrag der Integration YouTube, EINMAL; die
//! Wiederholung rueckt dabei auf hoechstens alle 5 min mal Fehlerzahl (nie seltener als
//! `poll_minutes`). Die erste erfolgreiche Antwort danach meldet die Rueckkehr (Audit, Log).
//! [`State::status`] liefert den Stand je Kanal fuer die Oberflaeche (B7).

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::Duration;

use reqwest::redirect::Policy;
use reqwest::StatusCode;
use serde_json::{json, Map, Value};
use url::Url;

use crate::managers::integrations::audit;
use crate::managers::integrations::model::{AuditOutcome, Caller, Integration, Kind, NewAudit};
use crate::managers::integrations::store as register;
use crate::managers::meetings::store::open_connection;
use crate::managers::youtube::link::valid_video_id;
use crate::managers::youtube::INTEGRATION_ID;

use super::super::model::TriggerDef;
use super::ledger::{self, Entry};
use super::{enabled_with, fire, iso, RunSink, TickReport};

pub const KIND: &str = "youtube.channel_new_video";
/// Die Adresse des offiziellen Feeds.
pub const FEED_URL: &str = "https://www.youtube.com/feeds/videos.xml";
/// Test-Umgebungsvariable: ein lokaler Teststand statt YouTube; gilt NUR in der Sandbox
/// (`LVA_MEETINGS_DIR` gesetzt), wie bei oEmbed.
pub const FEED_OVERRIDE_ENV: &str = "LVA_YOUTUBE_FEED_URL";
pub const DEFAULT_POLL_MINUTES: i64 = 60;
pub const MIN_POLL_MINUTES: i64 = 15;
pub const MAX_POLL_MINUTES: i64 = 1440;
pub const MAX_BACKFILL: i64 = 15;
/// Nach so vielen Fehlschlaegen in Folge gilt der Kanal als ausgefallen.
pub const OUTAGE_AFTER: u32 = 3;
/// Hoechstzahl Abrufe je Takt.
pub const MAX_FETCH_PER_TICK: usize = 3;
/// Wartezeit nach einem Fehlschlag (mal Fehlerzahl, hoechstens `poll_minutes`).
pub const RETRY_MS: i64 = 5 * 60_000;
pub const TIMEOUT: Duration = Duration::from_secs(10);
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
pub const MAX_BODY_BYTES: usize = 1024 * 1024;
const MAX_TEXT_CHARS: usize = 300;
const MAX_ENTRIES: usize = 50;

// ---------------------------------------------------------------------------
// Felder des Ausloesers
// ---------------------------------------------------------------------------

/// Eine Kanal-Kennung: `UC` plus 22 Zeichen aus `A-Z a-z 0-9 _ -`.
pub fn valid_channel_id(id: &str) -> bool {
    id.len() == 24
        && id.starts_with("UC")
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Spec {
    /// Integration YouTube; `None` = die Standardkennung.
    pub integration: Option<String>,
    pub channel_id: String,
    pub poll_ms: i64,
    pub backfill: usize,
}

impl Spec {
    pub fn from_def(t: &TriggerDef) -> Option<Self> {
        let channel_id = t.params.get("channel_id")?.as_str()?.trim().to_string();
        if !valid_channel_id(&channel_id) {
            return None;
        }
        let poll = t
            .params
            .get("poll_minutes")
            .and_then(Value::as_i64)
            .unwrap_or(DEFAULT_POLL_MINUTES)
            .clamp(MIN_POLL_MINUTES, MAX_POLL_MINUTES);
        let backfill = t
            .params
            .get("backfill")
            .and_then(Value::as_i64)
            .unwrap_or(0)
            .clamp(0, MAX_BACKFILL) as usize;
        Some(Self {
            integration: t
                .params
                .get("integration")
                .and_then(Value::as_str)
                .map(str::to_string),
            channel_id,
            poll_ms: poll * 60_000,
            backfill,
        })
    }
}

/// Zusaetzliche Pruefung beim Speichern (`trigger::check_definition`): (Feld, Satz).
pub fn check(params: &Map<String, Value>) -> Vec<(&'static str, String)> {
    let mut out = Vec::new();
    match params.get("channel_id").and_then(Value::as_str) {
        Some(id) if valid_channel_id(id.trim()) => {}
        Some(_) => out.push((
            "channel_id",
            "Eine Kanal-Kennung beginnt mit „UC“ und hat 24 Zeichen (nicht der @Name und nicht der Link): sie steht in der Adresse …/channel/UC… der Kanalseite."
                .to_string(),
        )),
        None => {}
    }
    out
}

// ---------------------------------------------------------------------------
// Der Feed (rein)
// ---------------------------------------------------------------------------

/// Ein Video aus dem Feed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Video {
    pub video_id: String,
    pub title: String,
    /// RFC 3339 in UTC, falls der Feed eine gueltige Zeit nannte.
    pub published: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Feed {
    pub channel_id: Option<String>,
    pub channel_title: Option<String>,
    /// Neueste zuerst, wie im Feed.
    pub videos: Vec<Video>,
}

/// Warum kein Feed vorliegt. `code()` fuer Audit und Oberflaeche, `Display` Klartext.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FeedError {
    NotFound,
    RateLimited,
    Timeout,
    Network(String),
    Http(u16),
    TooBig,
    Bad(String),
}

impl FeedError {
    pub fn code(&self) -> &'static str {
        match self {
            FeedError::NotFound => "feed_not_found",
            FeedError::RateLimited => "feed_rate_limited",
            FeedError::Timeout => "feed_timeout",
            FeedError::Network(_) => "feed_network",
            FeedError::Http(_) => "feed_http",
            FeedError::TooBig => "feed_too_big",
            FeedError::Bad(_) => "feed_bad",
        }
    }
}

impl std::fmt::Display for FeedError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FeedError::NotFound => write!(
                f,
                "Der Feed des Kanals wurde nicht gefunden (404). YouTube beantwortet Feeds zeitweise nicht; sonst die Kanal-Kennung prüfen."
            ),
            FeedError::RateLimited => write!(f, "YouTube bremst die Abrufe (429); später erneut."),
            FeedError::Timeout => write!(f, "YouTube antwortet nicht rechtzeitig."),
            FeedError::Network(m) => write!(f, "Keine Verbindung zu YouTube: {m}"),
            FeedError::Http(c) => write!(f, "YouTube antwortet mit dem Status {c}."),
            FeedError::TooBig => write!(f, "Die Antwort von YouTube ist zu groß."),
            FeedError::Bad(m) => write!(f, "Der Feed ist nicht lesbar ({m})."),
        }
    }
}

impl std::error::Error for FeedError {}

fn bad(m: &str) -> FeedError {
    FeedError::Bad(m.to_string())
}

/// Der Text zwischen `<name>` und `</name>` (auch `<name attr=...>`), roh.
fn tag_inner<'a>(block: &'a str, name: &str) -> Option<&'a str> {
    let plain = format!("<{name}>");
    let attr = format!("<{name} ");
    let start = match block.find(&plain) {
        Some(i) => i + plain.len(),
        None => {
            let i = block.find(&attr)?;
            i + block[i..].find('>')? + 1
        }
    };
    let close = format!("</{name}>");
    let end = start + block[start..].find(&close)?;
    Some(&block[start..end])
}

/// Entitaeten (`&amp;` ...) und Zeichenverweise (`&#228;`, `&#xE4;`) aufloesen, CDATA
/// uebernehmen. Unbekannte Entitaeten bleiben wie sie sind (es gibt keine eigenen).
fn xml_text(raw: &str) -> String {
    let t = raw.trim();
    if let Some(inner) = t
        .strip_prefix("<![CDATA[")
        .and_then(|s| s.strip_suffix("]]>"))
    {
        return inner.to_string();
    }
    let mut out = String::with_capacity(t.len());
    let mut rest = t;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        let after = &rest[i + 1..];
        let Some(semi) = after.find(';').filter(|s| *s <= 10) else {
            out.push('&');
            rest = after;
            continue;
        };
        let name = &after[..semi];
        let decoded = match name {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            n if n.starts_with("#x") || n.starts_with("#X") => {
                u32::from_str_radix(&n[2..], 16).ok().and_then(char::from_u32)
            }
            n if n.starts_with('#') => n[1..].parse::<u32>().ok().and_then(char::from_u32),
            _ => None,
        };
        match decoded {
            Some(c) => {
                out.push(c);
                rest = &after[semi + 1..];
            }
            None => {
                out.push('&');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

/// Steuerzeichen werden Leerzeichen, Leerraum zusammengezogen, auf `max` Zeichen gekuerzt.
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

/// Beginn eines `<entry`-Elements ab `from` (nicht `<entryx`).
fn entry_start(xml: &str, from: usize) -> Option<usize> {
    let mut at = from;
    while let Some(i) = xml[at..].find("<entry") {
        let pos = at + i;
        match xml.as_bytes().get(pos + "<entry".len()) {
            Some(b'>') | Some(b' ') | Some(b'\n') | Some(b'\r') | Some(b'\t') => return Some(pos),
            _ => at = pos + "<entry".len(),
        }
    }
    None
}

/// Wertet den Atom-Feed aus. Ein Video mit unbrauchbarer Kennung wird uebergangen.
pub fn parse_feed(xml: &str) -> Result<Feed, FeedError> {
    let lower_head: String = xml.chars().take(4096).collect::<String>().to_lowercase();
    if lower_head.contains("<!doctype") || lower_head.contains("<!entity") || xml.contains("<!ENTITY")
    {
        return Err(bad("Dokumenttyp-Definitionen sind nicht erlaubt"));
    }
    if !xml.contains("<feed") {
        return Err(bad("kein Atom-Feed"));
    }
    let first = entry_start(xml, 0);
    let header = &xml[..first.unwrap_or(xml.len())];
    let mut feed = Feed {
        channel_id: tag_inner(header, "yt:channelId")
            .map(|t| xml_text(t))
            .filter(|t| !t.is_empty()),
        channel_title: tag_inner(header, "title")
            .map(|t| clean_text(&xml_text(t), MAX_TEXT_CHARS))
            .filter(|t| !t.is_empty()),
        videos: Vec::new(),
    };
    let mut at = first;
    while let Some(start) = at {
        let Some(len) = xml[start..].find("</entry>") else {
            break;
        };
        let block = &xml[start..start + len];
        at = entry_start(xml, start + len);
        if feed.videos.len() >= MAX_ENTRIES {
            break;
        }
        let Some(video_id) = tag_inner(block, "yt:videoId").map(|t| xml_text(t)) else {
            continue;
        };
        if !valid_video_id(&video_id) {
            continue;
        }
        let title = tag_inner(block, "title")
            .map(|t| clean_text(&xml_text(t), MAX_TEXT_CHARS))
            .filter(|t| !t.is_empty())
            .unwrap_or_else(|| video_id.clone());
        let published = tag_inner(block, "published")
            .map(|t| xml_text(t))
            .and_then(|t| chrono::DateTime::parse_from_rfc3339(t.trim()).ok())
            .map(|d| iso(d.timestamp_millis()));
        feed.videos.push(Video {
            video_id,
            title,
            published,
        });
    }
    if feed.channel_title.is_none() {
        // Feed ohne eigenen Titel: der Name des Autors des ersten Videos.
        if let Some(s) = first {
            feed.channel_title = tag_inner(&xml[s..], "author")
                .and_then(|a| tag_inner(a, "name"))
                .map(|t| clean_text(&xml_text(t), MAX_TEXT_CHARS))
                .filter(|t| !t.is_empty());
        }
    }
    Ok(feed)
}

pub fn watch_url(video_id: &str) -> String {
    format!("https://www.youtube.com/watch?v={video_id}")
}

// ---------------------------------------------------------------------------
// Abruf
// ---------------------------------------------------------------------------

/// Holt den Feed eines Kanals (der Text der Antwort). Tests setzen eine Attrappe ein.
pub trait FeedFetcher: Send + Sync {
    fn fetch(&self, channel_id: &str) -> Result<String, FeedError>;
}

/// Rein: welche Adresse gilt? Ein Vorgabewert zaehlt nur in der Sandbox und nur, wenn er eine
/// `http(s)`-Adresse ist.
pub fn feed_base_from(override_url: Option<&str>, sandbox_dir: Option<&str>) -> String {
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
    FEED_URL.to_string()
}

pub fn feed_base() -> String {
    feed_base_from(
        std::env::var(FEED_OVERRIDE_ENV).ok().as_deref(),
        std::env::var(crate::managers::meetings::MEETINGS_DIR_ENV)
            .ok()
            .as_deref(),
    )
}

/// Der echte Abruf ueber HTTPS. `fetch` blockiert (der Scan laeuft auf einem eigenen Thread);
/// `fetch_async` ist derselbe Abruf fuer Tests gegen einen lokalen Server.
pub struct HttpFeedFetcher {
    pub base: String,
    pub timeout: Duration,
    pub max_bytes: usize,
    pub use_env_proxy: bool,
}

impl HttpFeedFetcher {
    pub fn production() -> Self {
        Self {
            base: feed_base(),
            timeout: TIMEOUT,
            max_bytes: MAX_BODY_BYTES,
            use_env_proxy: true,
        }
    }

    fn request_url(&self, channel_id: &str) -> Result<Url, FeedError> {
        if !valid_channel_id(channel_id) {
            return Err(bad("ungültige Kanal-Kennung"));
        }
        let mut url = Url::parse(&self.base)
            .ok()
            .filter(|u| matches!(u.scheme(), "http" | "https") && u.host_str().is_some())
            .ok_or_else(|| FeedError::Network("ungültige Adresse des Dienstes".to_string()))?;
        url.query_pairs_mut()
            .clear()
            .append_pair("channel_id", channel_id);
        Ok(url)
    }

    fn map_error(&self, e: reqwest::Error) -> FeedError {
        let e = e.without_url();
        if e.is_timeout() {
            return FeedError::Timeout;
        }
        if e.is_connect() {
            return FeedError::Network(
                "Die Verbindung kam nicht zustande (Internet, Proxy oder Firewall prüfen)."
                    .to_string(),
            );
        }
        FeedError::Network("Die Anfrage ist fehlgeschlagen.".to_string())
    }

    pub async fn fetch_async(&self, channel_id: &str) -> Result<String, FeedError> {
        let url = self.request_url(channel_id)?;
        let mut builder = reqwest::Client::builder()
            .timeout(self.timeout)
            .connect_timeout(CONNECT_TIMEOUT.min(self.timeout))
            .redirect(Policy::none())
            .user_agent(concat!(
                "LocalVoiceAI/",
                env!("CARGO_PKG_VERSION"),
                " (youtube-feed)"
            ));
        if !self.use_env_proxy {
            builder = builder.no_proxy();
        }
        let client = builder.build().map_err(|e| self.map_error(e))?;
        let mut response = client
            .get(url)
            .header(
                reqwest::header::ACCEPT,
                "application/atom+xml, application/xml;q=0.9, text/xml;q=0.8",
            )
            .send()
            .await
            .map_err(|e| self.map_error(e))?;
        let status = response.status();
        if status.is_redirection() {
            return Err(bad("unerwartete Umleitung"));
        }
        match status {
            s if s.is_success() => {}
            StatusCode::NOT_FOUND | StatusCode::GONE => return Err(FeedError::NotFound),
            StatusCode::TOO_MANY_REQUESTS => return Err(FeedError::RateLimited),
            other => return Err(FeedError::Http(other.as_u16())),
        }
        if response
            .content_length()
            .is_some_and(|len| len > self.max_bytes as u64)
        {
            return Err(FeedError::TooBig);
        }
        let mut body: Vec<u8> = Vec::new();
        loop {
            match response.chunk().await {
                Ok(Some(chunk)) => {
                    if body.len() + chunk.len() > self.max_bytes {
                        return Err(FeedError::TooBig);
                    }
                    body.extend_from_slice(&chunk);
                }
                Ok(None) => break,
                Err(e) => return Err(self.map_error(e)),
            }
        }
        Ok(String::from_utf8_lossy(&body).into_owned())
    }
}

impl FeedFetcher for HttpFeedFetcher {
    fn fetch(&self, channel_id: &str) -> Result<String, FeedError> {
        tauri::async_runtime::block_on(self.fetch_async(channel_id))
    }
}

// ---------------------------------------------------------------------------
// Zustand
// ---------------------------------------------------------------------------

/// Stand eines Kanals fuer die Oberflaeche (B7) und Tests.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChannelStatus {
    pub workflow_id: String,
    pub channel_id: String,
    pub last_ok_ms: Option<i64>,
    /// Fehlschlaege in Folge.
    pub failures: u32,
    /// `true` ab [`OUTAGE_AFTER`] Fehlschlaegen in Folge bis zur naechsten Antwort.
    pub outage: bool,
    pub last_error: Option<String>,
    pub next_fetch_ms: i64,
}

#[derive(Default)]
struct Inner {
    channels: HashMap<String, ChannelStatus>,
    /// Zuletzt gemeldetes Problem je Ablauf (ausgeschaltete Integration, ...).
    problems: HashMap<String, String>,
}

#[derive(Default)]
pub struct State {
    scanning: AtomicBool,
    inner: Mutex<Inner>,
}

/// Haelt den Platz des Abrufs; der Drop gibt ihn frei.
pub struct FetchGuard<'a>(&'a AtomicBool);

impl Drop for FetchGuard<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

fn key_of(workflow_id: &str, channel_id: &str) -> String {
    format!("{workflow_id}|{channel_id}")
}

impl State {
    /// Es laeuft nie zwei Abrufrunden gleichzeitig (ein Abruf kann bis 10 s dauern).
    pub fn try_begin(&self) -> Option<FetchGuard<'_>> {
        self.scanning
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .ok()
            .map(|_| FetchGuard(&self.scanning))
    }

    /// Laeuft gerade eine Abrufrunde? (billige Frage fuer den Takt)
    pub fn is_busy(&self) -> bool {
        self.scanning.load(Ordering::Acquire)
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn is_due(&self, key: &str, now_ms: i64) -> bool {
        self.lock()
            .channels
            .get(key)
            .is_none_or(|c| c.next_fetch_ms <= now_ms)
    }

    fn entry<'a>(
        g: &'a mut Inner,
        key: &str,
        workflow_id: &str,
        channel_id: &str,
    ) -> &'a mut ChannelStatus {
        g.channels
            .entry(key.to_string())
            .or_insert_with(|| ChannelStatus {
                workflow_id: workflow_id.to_string(),
                channel_id: channel_id.to_string(),
                last_ok_ms: None,
                failures: 0,
                outage: false,
                last_error: None,
                next_fetch_ms: 0,
            })
    }

    /// Verbucht eine Antwort. Gibt zurueck, ob der Kanal gerade AUS einem Ausfall zurueckkehrt.
    fn record_ok(&self, wf: &str, channel: &str, now_ms: i64, poll_ms: i64) -> bool {
        let mut g = self.lock();
        let c = Self::entry(&mut g, &key_of(wf, channel), wf, channel);
        let recovered = c.outage;
        c.failures = 0;
        c.outage = false;
        c.last_error = None;
        c.last_ok_ms = Some(now_ms);
        c.next_fetch_ms = now_ms.saturating_add(poll_ms);
        recovered
    }

    /// Verbucht einen Fehlschlag. Gibt zurueck, ob DIES der Fehlschlag ist, der den Ausfall
    /// ausruft (genau einmal je Ausfall).
    fn record_failure(
        &self,
        wf: &str,
        channel: &str,
        err: &FeedError,
        now_ms: i64,
        poll_ms: i64,
    ) -> bool {
        let mut g = self.lock();
        let c = Self::entry(&mut g, &key_of(wf, channel), wf, channel);
        c.failures = c.failures.saturating_add(1);
        c.last_error = Some(err.to_string());
        let wait = (RETRY_MS * i64::from(c.failures)).min(poll_ms);
        c.next_fetch_ms = now_ms.saturating_add(wait);
        if c.failures >= OUTAGE_AFTER && !c.outage {
            c.outage = true;
            return true;
        }
        false
    }

    /// Das Einreihen eines Videos scheiterte: bald wieder versuchen.
    fn retry_soon(&self, wf: &str, channel: &str, now_ms: i64) {
        let mut g = self.lock();
        if let Some(c) = g.channels.get_mut(&key_of(wf, channel)) {
            c.next_fetch_ms = c.next_fetch_ms.min(now_ms.saturating_add(RETRY_MS));
        }
    }

    fn note_problem(&self, workflow_id: &str, message: &str) -> bool {
        let mut g = self.lock();
        match g.problems.get(workflow_id) {
            Some(old) if old == message => false,
            _ => {
                g.problems
                    .insert(workflow_id.to_string(), message.to_string());
                true
            }
        }
    }

    fn clear_problem(&self, workflow_id: &str) {
        self.lock().problems.remove(workflow_id);
    }

    fn retain_workflows(&self, live: &HashSet<String>) {
        let mut g = self.lock();
        g.channels.retain(|_, c| live.contains(&c.workflow_id));
        g.problems.retain(|w, _| live.contains(w));
    }

    /// Stand aller bekannten Kanaele.
    pub fn status(&self) -> Vec<ChannelStatus> {
        let mut v: Vec<ChannelStatus> = self.lock().channels.values().cloned().collect();
        v.sort_by(|a, b| (&a.workflow_id, &a.channel_id).cmp(&(&b.workflow_id, &b.channel_id)));
        v
    }
}

// ---------------------------------------------------------------------------
// Takt
// ---------------------------------------------------------------------------

/// Die YouTube-Integration, die den Abruf freigibt: `None` = ruht (Grund im `Err`).
fn integration_gate(
    conn: &rusqlite::Connection,
    spec: &Spec,
) -> Result<Option<Integration>, String> {
    let wanted = spec.integration.as_deref().unwrap_or(INTEGRATION_ID);
    match register::get(conn, wanted) {
        Ok(Some(i)) if i.kind != Kind::Youtube => Err(format!(
            "„{}“ ist keine YouTube-Integration.",
            i.label
        )),
        Ok(Some(i)) if !i.enabled => Err(format!(
            "Die Integration „{}“ ist ausgeschaltet; der Kanal wird nicht abgefragt.",
            i.label
        )),
        Ok(Some(i)) => Ok(Some(i)),
        // Die Standard-Integration legt A2 beim ersten Link an; fehlt sie noch, ruht nichts.
        Ok(None) if spec.integration.is_none() => Ok(None),
        Ok(None) => Err(format!(
            "Die YouTube-Integration „{wanted}“ gibt es nicht (mehr)."
        )),
        Err(e) => Err(format!("Register: {e}")),
    }
}

fn audit_channel(
    conn: &rusqlite::Connection,
    integration: Option<&Integration>,
    channel: &str,
    outcome: AuditOutcome,
    detail: Value,
    now_ms: i64,
) {
    let entry = NewAudit {
        caller: Caller::Workflow.as_str().to_string(),
        integration_id: integration.map(|i| i.id.clone()),
        capability: None,
        target: Some(format!("youtube:channel:{channel}")),
        outcome,
        detail: Some(detail),
    };
    if let Err(e) = audit::record_at(conn, &entry, now_ms) {
        log::warn!("workflows: Audit des Kanals nicht geschrieben: {e}");
    }
}

/// Ein Takt: ruft jeden faelligen Kanal ab und reiht fuer neue Videos Laeufe ein.
pub fn on_tick(
    sink: &dyn RunSink,
    state: &State,
    fetcher: &dyn FeedFetcher,
    db_path: &Path,
    now_ms: i64,
) -> TickReport {
    let mut report = TickReport::default();
    let Some(_guard) = state.try_begin() else {
        report
            .skipped
            .push("Der vorige Abruf der Kanäle läuft noch.".to_string());
        return report;
    };
    let armed = match enabled_with(sink, &[KIND]) {
        Ok(a) => a,
        Err(e) => {
            report.errors.push(format!("Ablaeufe: {e}"));
            return report;
        }
    };
    let live: HashSet<String> = armed.iter().map(|a| a.row.id.clone()).collect();
    state.retain_workflows(&live);
    if armed.is_empty() {
        return report;
    }
    let conn = match open_connection(db_path) {
        Ok(c) => c,
        Err(e) => {
            report.errors.push(format!("Datenbank: {e}"));
            return report;
        }
    };
    let mut fetched = 0usize;
    for a in &armed {
        let Some(spec) = Spec::from_def(&a.def.trigger) else {
            continue;
        };
        let wf = a.row.id.as_str();
        let key = key_of(wf, &spec.channel_id);
        if !state.is_due(&key, now_ms) {
            continue;
        }
        let integration = match integration_gate(&conn, &spec) {
            Ok(i) => i,
            Err(reason) => {
                if state.note_problem(wf, &reason) {
                    log::warn!("workflows: Kanal-Ausloeser „{}“: {reason}", a.row.name);
                    report.skipped.push(format!("{}: {reason}", a.row.name));
                }
                continue;
            }
        };
        state.clear_problem(wf);
        if fetched >= MAX_FETCH_PER_TICK {
            continue; // der naechste Takt
        }
        fetched += 1;
        let feed = fetcher
            .fetch(&spec.channel_id)
            .and_then(|xml| parse_feed(&xml))
            .and_then(|feed| match feed.channel_id.as_deref() {
                Some(id) if id != spec.channel_id => Err(bad("der Feed gehört zu einem anderen Kanal")),
                _ => Ok(feed),
            });
        match feed {
            Ok(feed) => {
                let recovered = state.record_ok(wf, &spec.channel_id, now_ms, spec.poll_ms);
                if recovered {
                    log::info!(
                        "workflows: Kanal {} ist wieder erreichbar ({})",
                        spec.channel_id,
                        a.row.name
                    );
                    report.skipped.push(format!(
                        "{}: Der Kanal ist wieder erreichbar.",
                        a.row.name
                    ));
                    audit_channel(
                        &conn,
                        integration.as_ref(),
                        &spec.channel_id,
                        AuditOutcome::Ok,
                        json!({"phase": "rss", "recovered": true}),
                        now_ms,
                    );
                }
                process_feed(
                    sink,
                    state,
                    &conn,
                    integration.as_ref(),
                    a.row.id.as_str(),
                    &a.row.name,
                    &spec,
                    &feed,
                    now_ms,
                    &mut report,
                );
            }
            Err(err) => {
                let outage = state.record_failure(wf, &spec.channel_id, &err, now_ms, spec.poll_ms);
                log::warn!(
                    "workflows: Kanal {} nicht abgerufen ({}): {err}",
                    spec.channel_id,
                    a.row.name
                );
                if outage {
                    report.errors.push(format!(
                        "{}: Der Kanal ist seit {OUTAGE_AFTER} Abrufen nicht erreichbar: {err}",
                        a.row.name
                    ));
                    audit_channel(
                        &conn,
                        integration.as_ref(),
                        &spec.channel_id,
                        AuditOutcome::Error,
                        json!({"phase": "rss", "error": err.code(), "failures": OUTAGE_AFTER}),
                        now_ms,
                    );
                }
            }
        }
    }
    report
}

#[allow(clippy::too_many_arguments)]
fn process_feed(
    sink: &dyn RunSink,
    state: &State,
    conn: &rusqlite::Connection,
    integration: Option<&Integration>,
    wf: &str,
    wf_name: &str,
    spec: &Spec,
    feed: &Feed,
    now_ms: i64,
    report: &mut TickReport,
) {
    let channel = spec.channel_id.as_str();
    let initialized = match ledger::exists(conn, &ledger::channel_init_key(wf, channel)) {
        Ok(v) => v,
        Err(e) => {
            report.errors.push(format!("{wf_name}: Ledger: {e}"));
            state.retry_soon(wf, channel, now_ms);
            return;
        }
    };
    // Unbekannte Videos, neueste zuerst (Feed-Reihenfolge).
    let mut fresh: Vec<&Video> = Vec::new();
    for v in &feed.videos {
        match ledger::exists(conn, &ledger::video_key(wf, channel, &v.video_id)) {
            Ok(false) => fresh.push(v),
            Ok(true) => {}
            Err(e) => {
                report.errors.push(format!("{wf_name}: Ledger: {e}"));
                state.retry_soon(wf, channel, now_ms);
                return;
            }
        }
    }
    // Wer bekommt einen Lauf? Beim ersten Abruf nur die neuesten `backfill`.
    let run_count = if initialized {
        fresh.len()
    } else {
        spec.backfill.min(fresh.len())
    };
    let (to_run, to_know) = fresh.split_at(run_count);
    let mut ledger_failed = false;
    if !initialized {
        for v in to_know {
            let e = Entry::new(ledger::video_key(wf, channel, &v.video_id), now_ms);
            if let Err(err) = ledger::put(conn, &e) {
                report.errors.push(format!("{wf_name}: Ledger: {err}"));
                ledger_failed = true;
                break;
            }
        }
    }
    let mut enqueue_failed = false;
    // Aelteste zuerst: die Laeufe entstehen in der Reihenfolge der Veroeffentlichung.
    for v in to_run.iter().rev() {
        let trigger = json!({
            "channel_id": channel,
            "channel_title": feed.channel_title.clone().unwrap_or_default(),
            "video_id": v.video_id,
            "title": v.title,
            "url": watch_url(&v.video_id),
            "published": v.published,
        });
        match fire(
            sink,
            report,
            wf,
            format!("yt:{channel}:{}", v.video_id),
            trigger,
        ) {
            Some(run_id) => {
                let mut e = Entry::new(ledger::video_key(wf, channel, &v.video_id), now_ms);
                e.run_id = Some(run_id);
                if let Err(err) = ledger::put(conn, &e) {
                    // Der Lauf steht; beim naechsten Abruf ist es ein Duplikat der Engine.
                    log::warn!("workflows: Ledger nicht geschrieben: {err}");
                    report.errors.push(format!("{wf_name}: Ledger: {err}"));
                }
            }
            // Im Bericht steht der Fehler (`fire`); das Video bleibt unbekannt.
            None => enqueue_failed = true,
        }
    }
    if !initialized && !ledger_failed {
        let mut marker = Entry::new(ledger::channel_init_key(wf, channel), now_ms);
        marker.size = Some(feed.videos.len() as i64);
        match ledger::put(conn, &marker) {
            Ok(()) => {
                audit_channel(
                    conn,
                    integration,
                    channel,
                    AuditOutcome::Ok,
                    json!({
                        "phase": "rss_init",
                        "known": to_know.len(),
                        "runs": to_run.len()
                    }),
                    now_ms,
                );
            }
            Err(err) => {
                report.errors.push(format!("{wf_name}: Ledger: {err}"));
                ledger_failed = true;
            }
        }
    }
    if enqueue_failed || ledger_failed {
        state.retry_soon(wf, channel, now_ms);
    }
    if let Err(e) = ledger::prune_family(conn, "yt:", ledger::VIDEO_CAP) {
        log::warn!("workflows: Ledger nicht bereinigt: {e}");
    }
}

#[cfg(test)]
mod tests;
