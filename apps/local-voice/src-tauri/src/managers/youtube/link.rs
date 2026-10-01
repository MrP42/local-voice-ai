//! Link-Normalisierung (A2): aus dem, was jemand einfuegt, wird eine
//! Video-ID oder eine verstaendliche Ablehnung.
//!
//! Verstanden werden die Formen, die YouTube selbst erzeugt: `watch?v=`,
//! `youtu.be/`, `shorts/`, `embed/`, `live/`, `v/`, `m.`/`music.`-Hosts und
//! `youtube-nocookie.com`, mit beliebigen Zusatzparametern (`t=`, `start=`, `si=`,
//! `list=` ...). Verworfen werden Verfolgungsparameter (`si`, `feature`, `pp`)
//! und der Wiedergabelisten-Bezug; uebernommen wird nur die Startzeit.
//!
//! Sicherheit: der Host wird aus der geparsten Adresse gelesen und muss GENAU
//! einer der bekannten sein (kein `endswith`, keine Unterdomaene), ohne
//! Zugangsdaten und ohne fremden Port. Die Video-ID ist genau elf Zeichen aus
//! `[A-Za-z0-9_-]`; erst danach darf sie in eine Adresse oder eine Datei gelangen.

use url::Url;

#[cfg(test)]
mod tests;

/// Laengster akzeptierter Link (Zeichen). Mehr ist nie ein Link, sondern ein
/// eingefuegter Absatz.
pub const MAX_LINK_CHARS: usize = 2048;
/// Laengste Startzeit: 99 Stunden. Alles darueber ist ein Tippfehler.
const MAX_START_S: u64 = 99 * 3600;

/// Hosts, deren Links wir verstehen (genau diese, kleingeschrieben).
const VIDEO_HOSTS: [&str; 6] = [
    "youtube.com",
    "www.youtube.com",
    "m.youtube.com",
    "music.youtube.com",
    "youtube-nocookie.com",
    "www.youtube-nocookie.com",
];
const SHORT_HOST: &str = "youtu.be";

/// Ein Video, wie es ein Link bezeichnet.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct YoutubeRef {
    pub video_id: String,
    /// Startzeit aus `t=`/`start=` in Sekunden, wenn der Link eine trug.
    pub start_s: Option<u32>,
}

impl YoutubeRef {
    /// Die bereinigte Adresse: nur die ID, ohne Zeit, Verfolgung und Liste.
    pub fn canonical_url(&self) -> String {
        format!("https://www.youtube.com/watch?v={}", self.video_id)
    }
}

/// Warum ein Text kein Video-Link ist.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LinkError {
    Empty,
    TooLong,
    /// Kein Text, der sich als Internetadresse lesen laesst.
    NotAnAddress,
    NotYoutube,
    /// Wiedergabeliste (folgt spaeter, Paket A9).
    Playlist,
    /// Kanal oder Nutzerseite.
    Channel,
    /// YouTube-Adresse, aber ohne Video (Startseite, Suche, Feed).
    NoVideo,
    BadVideoId,
}

impl LinkError {
    pub fn code(self) -> &'static str {
        match self {
            LinkError::Empty => "youtube_empty",
            LinkError::TooLong => "youtube_too_long",
            LinkError::NotAnAddress | LinkError::NoVideo | LinkError::BadVideoId => {
                "youtube_invalid_link"
            }
            LinkError::NotYoutube => "youtube_not_youtube",
            LinkError::Playlist => "youtube_playlist",
            LinkError::Channel => "youtube_channel",
        }
    }
}

impl std::fmt::Display for LinkError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let text = match self {
            LinkError::Empty => "Bitte einen YouTube-Link einfügen.",
            LinkError::TooLong => "Der Text ist zu lang für einen Link.",
            LinkError::NotAnAddress | LinkError::BadVideoId => {
                "Das ist kein gültiger YouTube-Link zu einem einzelnen Video."
            }
            LinkError::NoVideo => "Dieser YouTube-Link führt zu keinem einzelnen Video.",
            LinkError::NotYoutube => "Das ist kein YouTube-Link.",
            LinkError::Playlist => {
                "Playlists folgen später. Bitte den Link eines einzelnen Videos einfügen."
            }
            LinkError::Channel => {
                "Kanäle werden nicht unterstützt. Bitte den Link eines einzelnen Videos einfügen."
            }
        };
        f.write_str(text)
    }
}

impl std::error::Error for LinkError {}

/// Ist das eine gueltige Video-ID: genau elf Zeichen aus `[A-Za-z0-9_-]`?
pub fn valid_video_id(id: &str) -> bool {
    id.len() == 11
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// Text aus der Zwischenablage -> Adresse: Leerraum und umschliessende
/// Klammern/Anfuehrungszeichen weg, Steuerzeichen und Leerraum im Inneren
/// abweisen (zwei Links in einem Text sind kein Link).
fn clean_input(raw: &str) -> Result<String, LinkError> {
    let mut text = raw.trim();
    if text.is_empty() {
        return Err(LinkError::Empty);
    }
    if text.chars().count() > MAX_LINK_CHARS {
        return Err(LinkError::TooLong);
    }
    loop {
        let stripped = text
            .strip_prefix('<')
            .and_then(|t| t.strip_suffix('>'))
            .or_else(|| text.strip_prefix('"').and_then(|t| t.strip_suffix('"')))
            .or_else(|| text.strip_prefix('\'').and_then(|t| t.strip_suffix('\'')))
            .map(str::trim);
        match stripped {
            Some(inner) => text = inner,
            None => break,
        }
    }
    if text.is_empty() {
        return Err(LinkError::Empty);
    }
    if text.chars().any(|c| c.is_control() || c.is_whitespace()) {
        return Err(LinkError::NotAnAddress);
    }
    Ok(text.to_string())
}

/// Ohne Schema (`youtu.be/ID`, `www.youtube.com/...`) wird `https://`
/// vorangestellt, aber nur, wenn der Text mit einem Host beginnt.
fn parse(text: &str) -> Result<Url, LinkError> {
    let candidate = if text.contains("://") {
        text.to_string()
    } else if looks_like_host_start(text) {
        format!("https://{text}")
    } else {
        // `javascript:alert(1)`, `mailto:x` ... haben ein Schema ohne `//`.
        return match Url::parse(text) {
            Ok(_) => Err(LinkError::NotYoutube),
            Err(_) => Err(LinkError::NotAnAddress),
        };
    };
    Url::parse(&candidate).map_err(|_| LinkError::NotAnAddress)
}

/// `host.tld/...`: vor dem ersten `/` (oder `?`, `#`) steht ein Punkt und nur
/// Zeichen, die in einem Host vorkommen duerfen.
fn looks_like_host_start(text: &str) -> bool {
    let head = text.split(['/', '?', '#']).next().unwrap_or("");
    head.contains('.')
        && !head.starts_with('.')
        && head
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == ':')
}

/// Startzeit aus `90`, `90s`, `1m30s`, `1h2m3s` (Gross-/Kleinschreibung egal).
/// Reihenfolge Stunden, Minuten, Sekunden; alles andere ist `None`.
fn parse_start(value: &str) -> Option<u32> {
    let v = value.trim().to_ascii_lowercase();
    if v.is_empty() {
        return None;
    }
    if v.bytes().all(|b| b.is_ascii_digit()) {
        return bounded(v.parse::<u64>().ok()?);
    }
    let mut total: u64 = 0;
    let mut number = String::new();
    let mut last_rank = 0u8;
    let mut seen_any = false;
    for c in v.chars() {
        if c.is_ascii_digit() {
            number.push(c);
            continue;
        }
        let (rank, factor) = match c {
            'h' => (1, 3600),
            'm' => (2, 60),
            's' => (3, 1),
            _ => return None,
        };
        if number.is_empty() || rank <= last_rank {
            return None;
        }
        total = total.checked_add(number.parse::<u64>().ok()?.checked_mul(factor)?)?;
        number.clear();
        last_rank = rank;
        seen_any = true;
    }
    // Reste ohne Einheit ("1m30") sind mehrdeutig.
    if !number.is_empty() || !seen_any {
        return None;
    }
    bounded(total)
}

fn bounded(seconds: u64) -> Option<u32> {
    (seconds <= MAX_START_S).then_some(seconds as u32)
}

fn query_value(url: &Url, key: &str) -> Option<String> {
    url.query_pairs()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v.into_owned())
}

/// Startzeit aus `t`/`start` der Abfrage, sonst aus `#t=` des Fragments.
fn start_of(url: &Url) -> Option<u32> {
    ["t", "start"]
        .iter()
        .find_map(|k| query_value(url, k))
        .or_else(|| {
            url.fragment()
                .and_then(|f| f.strip_prefix("t="))
                .map(str::to_string)
        })
        .and_then(|v| parse_start(&v))
}

fn video(id: &str, url: &Url) -> Result<YoutubeRef, LinkError> {
    if id.is_empty() {
        return Err(LinkError::NoVideo);
    }
    if !valid_video_id(id) {
        return Err(LinkError::BadVideoId);
    }
    Ok(YoutubeRef {
        video_id: id.to_string(),
        start_s: start_of(url),
    })
}

/// Aus einem eingefuegten Text die Video-Referenz machen.
pub fn normalize_link(raw: &str) -> Result<YoutubeRef, LinkError> {
    let text = clean_input(raw)?;
    let url = parse(&text)?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err(LinkError::NotYoutube);
    }
    // Zugangsdaten oder ein fremder Port in einer YouTube-Adresse sind nie echt.
    if !url.username().is_empty() || url.password().is_some() || url.port().is_some() {
        return Err(LinkError::NotYoutube);
    }
    let host = url
        .host_str()
        .map(str::to_ascii_lowercase)
        .ok_or(LinkError::NotAnAddress)?;
    let segments: Vec<&str> = url.path_segments().map(|s| s.collect()).unwrap_or_default();
    let first = segments.first().copied().unwrap_or("");
    let second = segments.get(1).copied().unwrap_or("");

    if host == SHORT_HOST {
        return video(first, &url);
    }
    if !VIDEO_HOSTS.contains(&host.as_str()) {
        return Err(LinkError::NotYoutube);
    }
    match first {
        "watch" => match query_value(&url, "v") {
            // `v=` ohne Wert ist ein kaputter Link, kein Link ohne Video.
            Some(id) if id.is_empty() => Err(LinkError::BadVideoId),
            Some(id) => video(&id, &url),
            None if query_value(&url, "list").is_some() => Err(LinkError::Playlist),
            None => Err(LinkError::NoVideo),
        },
        "playlist" => Err(LinkError::Playlist),
        "embed" if second == "videoseries" => Err(LinkError::Playlist),
        "shorts" | "embed" | "live" | "v" => video(second, &url),
        "channel" | "c" | "user" => Err(LinkError::Channel),
        s if s.starts_with('@') => Err(LinkError::Channel),
        _ => Err(LinkError::NoVideo),
    }
}
