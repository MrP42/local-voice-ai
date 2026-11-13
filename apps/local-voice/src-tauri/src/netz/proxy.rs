//! Proxy für Downloads so wählen, wie Windows es täte (lva#573).
//!
//! Befund: In Behördennetzen löst der interne DNS externe Namen nicht auf; ins
//! Internet kommt man nur über einen Proxy, den eine PAC-Datei oder die
//! Auto-Erkennung (WPAD) vorgibt. reqwest liest von sich aus nur
//! `HTTPS_PROXY`/`ALL_PROXY` und den fest eingetragenen Registry-Proxy, aber
//! weder PAC noch WPAD. Ohne diese Auflösung fragt die App den DNS direkt und
//! scheitert mit „Der angegebene Host ist unbekannt“ (11001).
//!
//! Reihenfolge:
//! 1. eigener Proxy aus den Einstellungen (`direkt` erzwingt ohne Proxy),
//! 2. Proxy-Umgebungsvariablen (reqwest wertet sie selbst aus),
//! 3. Windows: PAC-Datei bzw. Auto-Erkennung über WinHTTP, je Ziel-URL,
//! 4. sonst reqwests Standard (inkl. statischem Registry-Proxy mit Ausnahmeliste).
//!
//! Zugangsdaten in der Proxy-Adresse werden abgelehnt: die Einstellungen sind
//! kein Geheimnisspeicher, und Adressen landen im Log.

use std::collections::HashMap;
use std::sync::{Mutex, RwLock};
use std::time::{Duration, Instant};

/// Wie eine Anfrage ihr Ziel erreicht.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Weg {
    /// reqwest entscheidet selbst (Umgebungsvariablen, statischer Registry-Proxy).
    Standard,
    /// Ausdrücklich ohne Proxy.
    Direkt,
    /// Über diesen Proxy (`http://host:port`).
    Ueber(String),
}

impl Weg {
    /// Für das Log; enthält nie Zugangsdaten (die lehnt [`normalisiere`] ab,
    /// und Systemproxys liefern keine).
    pub fn beschreibung(&self) -> String {
        match self {
            Weg::Standard => "Standard".into(),
            Weg::Direkt => "direkt".into(),
            Weg::Ueber(p) => format!("Proxy {p}"),
        }
    }
}

/// Wörter, mit denen die Einstellung „ohne Proxy“ erzwingt.
const DIREKT_WOERTER: [&str; 2] = ["direkt", "direct"];

/// Wie lange ein aufgelöster Systemweg gilt (PAC-Auswertung kann Sekunden dauern).
const CACHE_DAUER: Duration = Duration::from_secs(300);

static EIGENER: RwLock<Option<String>> = RwLock::new(None);
/// Aufgelöster Systemweg je `schema://host` mit Zeitpunkt; `None` = kein Systemproxy.
type WegCache = HashMap<String, (Instant, Option<Weg>)>;

static CACHE: Mutex<Option<WegCache>> = Mutex::new(None);

/// Prüft und vereinheitlicht eine Proxy-Angabe aus den Einstellungen.
/// Leer → `None`; `direkt`/`direct` bleibt das Wort; `host:port` bekommt
/// `http://`. Erlaubt sind `http` und `https`, Zugangsdaten nicht.
pub fn normalisiere(eingabe: &str) -> Result<Option<String>, String> {
    let s = eingabe.trim();
    if s.is_empty() {
        return Ok(None);
    }
    if DIREKT_WOERTER.iter().any(|w| s.eq_ignore_ascii_case(w)) {
        return Ok(Some("direkt".into()));
    }
    let mit_schema = if s.contains("://") {
        s.to_string()
    } else {
        format!("http://{s}")
    };
    let url = url::Url::parse(&mit_schema).map_err(|_| "Ungültige Proxy-Adresse.".to_string())?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err("Proxy-Adresse muss mit http:// oder https:// beginnen.".into());
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(
            "Bitte keine Zugangsdaten in die Proxy-Adresse schreiben (nur Host und Port).".into(),
        );
    }
    let host = url
        .host_str()
        .filter(|h| !h.is_empty())
        .ok_or_else(|| "Proxy-Adresse ohne Host.".to_string())?;
    if url.path() != "/" || url.query().is_some() || url.fragment().is_some() {
        return Err("Proxy-Adresse nur als Host und Port angeben.".into());
    }
    Ok(Some(match url.port() {
        Some(port) => format!("{}://{}:{}", url.scheme(), host, port),
        None => format!("{}://{}", url.scheme(), host),
    }))
}

/// Setzt den eigenen Proxy aus den Einstellungen (ungültige Angaben = keiner)
/// und verwirft zwischengespeicherte Systemwege.
pub fn setze_eigenen(einstellung: Option<&str>) {
    let wert = einstellung.and_then(|s| normalisiere(s).ok().flatten());
    if let Ok(mut eigener) = EIGENER.write() {
        *eigener = wert;
    }
    if let Ok(mut cache) = CACHE.lock() {
        *cache = None;
    }
}

fn eigener() -> Option<String> {
    EIGENER.read().ok().and_then(|e| e.clone())
}

/// Hat der Nutzer bzw. die Umgebung reqwest schon einen Proxy vorgegeben?
fn umgebung_gesetzt(lesen: impl Fn(&str) -> Option<String>) -> bool {
    [
        "HTTPS_PROXY",
        "https_proxy",
        "ALL_PROXY",
        "all_proxy",
        "HTTP_PROXY",
        "http_proxy",
    ]
    .iter()
    .any(|k| lesen(k).is_some_and(|v| !v.trim().is_empty()))
}

/// Liest eine Proxy-Liste, wie WinHTTP oder eine PAC-Datei sie liefert:
/// Einträge mit `;` oder Leerzeichen getrennt, je Eintrag `host:port`,
/// `https=host:port`, `PROXY host:port` oder `DIRECT`. Bevorzugt einen
/// `https=`-Eintrag, sonst den ersten allgemeinen.
pub fn lies_proxy_liste(liste: &str) -> Option<Weg> {
    let mut allgemein: Option<Weg> = None;
    let mut eintraege = liste
        .split(';')
        .map(str::trim)
        .filter(|e| !e.is_empty())
        .peekable();
    eintraege.peek()?;
    for eintrag in eintraege {
        let (art, rest) = match eintrag.split_once('=') {
            Some((a, r)) => (Some(a.trim().to_ascii_lowercase()), r.trim()),
            None => (None, eintrag),
        };
        let rest = rest
            .strip_prefix("PROXY ")
            .or_else(|| rest.strip_prefix("proxy "))
            .or_else(|| rest.strip_prefix("HTTPS "))
            .unwrap_or(rest)
            .trim();
        let weg = if rest.eq_ignore_ascii_case("DIRECT") {
            Weg::Direkt
        } else {
            match normalisiere(rest) {
                Ok(Some(p)) if p != "direkt" => Weg::Ueber(p),
                _ => continue,
            }
        };
        match art.as_deref() {
            Some("https") => return Some(weg),
            Some(_) => {}
            None => {
                if allgemein.is_none() {
                    allgemein = Some(weg);
                }
            }
        }
    }
    allgemein
}

/// Ziele auf diesem Rechner (Tests, lokale Server): nie Proxy, nie PAC/WPAD.
fn ist_lokal(url: &str) -> bool {
    let Ok(u) = url::Url::parse(url) else {
        return false;
    };
    match u.host() {
        Some(url::Host::Domain(d)) => d.eq_ignore_ascii_case("localhost"),
        Some(url::Host::Ipv4(ip)) => ip.is_loopback(),
        Some(url::Host::Ipv6(ip)) => ip.is_loopback(),
        None => false,
    }
}

fn cache_schluessel(url: &str) -> String {
    url::Url::parse(url)
        .ok()
        .map(|u| format!("{}://{}", u.scheme(), u.host_str().unwrap_or("")))
        .unwrap_or_else(|| url.to_string())
}

/// Der Weg für `url`. Blockiert ggf. (PAC-Auswertung); aus async-Code
/// [`weg_fuer_async`] nehmen.
pub fn weg_fuer(url: &str) -> Weg {
    // Lokale Ziele (Tests, lokale Dienste) gehen nie über einen Proxy.
    if ist_lokal(url) {
        return Weg::Standard;
    }
    if let Some(p) = eigener() {
        return if p == "direkt" {
            Weg::Direkt
        } else {
            Weg::Ueber(p)
        };
    }
    if umgebung_gesetzt(|k| std::env::var(k).ok()) {
        return Weg::Standard;
    }
    let schluessel = cache_schluessel(url);
    if let Ok(cache) = CACHE.lock() {
        if let Some((zeit, weg)) = cache.as_ref().and_then(|c| c.get(&schluessel)) {
            if zeit.elapsed() < CACHE_DAUER {
                return weg.clone().unwrap_or(Weg::Standard);
            }
        }
    }
    let weg = system_weg(url);
    if let Some(w) = &weg {
        log::info!("Systemproxy für {}: {}", schluessel, w.beschreibung());
    }
    if let Ok(mut cache) = CACHE.lock() {
        cache
            .get_or_insert_with(HashMap::new)
            .insert(schluessel, (Instant::now(), weg.clone()));
    }
    weg.unwrap_or(Weg::Standard)
}

/// [`weg_fuer`] ohne den async-Laufzeitfaden zu blockieren.
pub async fn weg_fuer_async(url: &str) -> Weg {
    let url = url.to_string();
    tokio::task::spawn_blocking(move || weg_fuer(&url))
        .await
        .unwrap_or(Weg::Standard)
}

/// Wendet `weg` auf einen Client-Bauplan an. Eine Adresse, die reqwest nicht
/// annimmt, fällt auf den Standard zurück (mit Warnung), statt den Download
/// zu verhindern.
pub fn anwenden(builder: reqwest::ClientBuilder, weg: &Weg) -> reqwest::ClientBuilder {
    match weg {
        Weg::Standard => builder,
        Weg::Direkt => builder.no_proxy(),
        Weg::Ueber(p) => match reqwest::Proxy::all(p.as_str()) {
            Ok(proxy) => builder.proxy(proxy),
            Err(e) => {
                log::warn!("Proxy {} nicht verwendbar: {}", p, e);
                builder
            }
        },
    }
}

/// Client-Bauplan für einen Download von `url` mit dem passenden Proxy.
pub async fn client_fuer(url: &str) -> reqwest::ClientBuilder {
    let weg = weg_fuer_async(url).await;
    anwenden(reqwest::Client::builder(), &weg)
}

#[cfg(not(windows))]
fn system_weg(_url: &str) -> Option<Weg> {
    None
}

/// PAC-Datei bzw. Auto-Erkennung über WinHTTP auswerten. Ein statischer
/// Registry-Proxy bleibt reqwest überlassen (das liest ihn samt Ausnahmeliste).
#[cfg(windows)]
fn system_weg(url: &str) -> Option<Weg> {
    use windows::core::{HSTRING, PCWSTR, PWSTR};
    use windows::Win32::Foundation::{GlobalFree, HGLOBAL};
    use windows::Win32::Networking::WinHttp::{
        WinHttpCloseHandle, WinHttpGetIEProxyConfigForCurrentUser, WinHttpGetProxyForUrl,
        WinHttpOpen, WinHttpSetTimeouts, WINHTTP_ACCESS_TYPE_NAMED_PROXY,
        WINHTTP_ACCESS_TYPE_NO_PROXY, WINHTTP_AUTOPROXY_AUTO_DETECT, WINHTTP_AUTOPROXY_CONFIG_URL,
        WINHTTP_AUTOPROXY_OPTIONS, WINHTTP_AUTO_DETECT_TYPE_DHCP, WINHTTP_AUTO_DETECT_TYPE_DNS_A,
        WINHTTP_CURRENT_USER_IE_PROXY_CONFIG, WINHTTP_PROXY_INFO,
    };

    /// Übernimmt einen von WinHTTP angelegten String und gibt ihn frei.
    fn nimm(p: PWSTR) -> Option<String> {
        if p.is_null() {
            return None;
        }
        // SAFETY: WinHTTP liefert einen nullterminierten, mit GlobalAlloc
        // angelegten String; er wird genau einmal gelesen und freigegeben.
        let s = unsafe { p.to_string() }.ok();
        let _ = unsafe { GlobalFree(Some(HGLOBAL(p.0 as *mut core::ffi::c_void))) };
        s.filter(|s| !s.trim().is_empty())
    }

    let mut ie = WINHTTP_CURRENT_USER_IE_PROXY_CONFIG::default();
    // SAFETY: gültiger Zeiger auf eine nullinitialisierte Struktur.
    if unsafe { WinHttpGetIEProxyConfigForCurrentUser(&mut ie) }.is_err() {
        return None;
    }
    let auto_erkennung = ie.fAutoDetect.as_bool();
    let pac = nimm(ie.lpszAutoConfigUrl);
    let _ = nimm(ie.lpszProxy);
    let _ = nimm(ie.lpszProxyBypass);
    if pac.is_none() && !auto_erkennung {
        return None;
    }

    // SAFETY: Konstanten-Strings bzw. Nullzeiger; das Handle wird unten geschlossen.
    let sitzung = unsafe {
        WinHttpOpen(
            &HSTRING::from("LocalVoice"),
            WINHTTP_ACCESS_TYPE_NO_PROXY,
            PCWSTR::null(),
            PCWSTR::null(),
            0,
        )
    };
    if sitzung.is_null() {
        return None;
    }
    // Auflösung, Verbindung, Senden, Empfangen je 10 s: die PAC-Datei liegt im
    // Intranet; hängt sie, soll der Download nicht ewig warten.
    let _ = unsafe { WinHttpSetTimeouts(sitzung, 10_000, 10_000, 10_000, 10_000) };

    let pac_w = pac.as_deref().map(HSTRING::from);
    let mut optionen = WINHTTP_AUTOPROXY_OPTIONS {
        fAutoLogonIfChallenged: true.into(),
        ..Default::default()
    };
    if let Some(p) = &pac_w {
        optionen.dwFlags |= WINHTTP_AUTOPROXY_CONFIG_URL;
        optionen.lpszAutoConfigUrl = PCWSTR(p.as_ptr());
    }
    if auto_erkennung {
        optionen.dwFlags |= WINHTTP_AUTOPROXY_AUTO_DETECT;
        optionen.dwAutoDetectFlags = WINHTTP_AUTO_DETECT_TYPE_DHCP | WINHTTP_AUTO_DETECT_TYPE_DNS_A;
    }
    let mut info = WINHTTP_PROXY_INFO::default();
    let ziel = HSTRING::from(url);
    // SAFETY: Sitzung gültig, Strukturen leben bis nach dem Aufruf.
    let ergebnis = unsafe { WinHttpGetProxyForUrl(sitzung, &ziel, &mut optionen, &mut info) };
    let _ = unsafe { WinHttpCloseHandle(sitzung) };
    if let Err(e) = ergebnis {
        log::warn!("PAC/Auto-Erkennung nicht auswertbar: {}", e);
        return None;
    }
    let liste = nimm(info.lpszProxy);
    let _ = nimm(info.lpszProxyBypass);
    if info.dwAccessType == WINHTTP_ACCESS_TYPE_NAMED_PROXY {
        liste.as_deref().and_then(lies_proxy_liste)
    } else {
        Some(Weg::Direkt)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalisiere_ergaenzt_schema_und_lehnt_zugangsdaten_ab() {
        assert_eq!(normalisiere("  ").unwrap(), None);
        assert_eq!(
            normalisiere("proxy.example.org:8080").unwrap().as_deref(),
            Some("http://proxy.example.org:8080")
        );
        assert_eq!(
            normalisiere("http://10.0.0.1:3128/").unwrap().as_deref(),
            Some("http://10.0.0.1:3128")
        );
        assert_eq!(normalisiere("DIREKT").unwrap().as_deref(), Some("direkt"));
        assert_eq!(normalisiere("direct").unwrap().as_deref(), Some("direkt"));
        assert!(normalisiere("http://nutzer:geheim@proxy.example.org:8080").is_err());
        assert!(normalisiere("socks5://proxy.example.org:1080").is_err());
        assert!(normalisiere("http://proxy.example.org:8080/pfad").is_err());
        assert!(normalisiere("http://:8080").is_err());
    }

    #[test]
    fn proxy_liste_wie_winhttp_und_pac() {
        assert_eq!(
            lies_proxy_liste("proxy.example.org:8080"),
            Some(Weg::Ueber("http://proxy.example.org:8080".into()))
        );
        assert_eq!(
            lies_proxy_liste("PROXY a.example.org:3128; PROXY b.example.org:3128"),
            Some(Weg::Ueber("http://a.example.org:3128".into()))
        );
        assert_eq!(
            lies_proxy_liste("http=h.example.org:80;https=s.example.org:443"),
            Some(Weg::Ueber("http://s.example.org:443".into()))
        );
        assert_eq!(lies_proxy_liste("DIRECT"), Some(Weg::Direkt));
        assert_eq!(lies_proxy_liste(""), None);
        assert_eq!(lies_proxy_liste(" ; "), None);
        assert_eq!(lies_proxy_liste("ftp=f.example.org:21"), None);
    }

    #[test]
    fn umgebung_zaehlt_nur_wenn_gesetzt() {
        assert!(!umgebung_gesetzt(|_| None));
        assert!(!umgebung_gesetzt(|_| Some("  ".into())));
        assert!(umgebung_gesetzt(
            |k| (k == "https_proxy").then(|| "http://p:1".into())
        ));
    }

    #[test]
    fn eigener_proxy_hat_vorrang_und_direkt_erzwingt_ohne() {
        setze_eigenen(Some("proxy.example.org:8080"));
        assert_eq!(
            weg_fuer("https://huggingface.co/x"),
            Weg::Ueber("http://proxy.example.org:8080".into())
        );
        setze_eigenen(Some("direkt"));
        assert_eq!(weg_fuer("https://huggingface.co/x"), Weg::Direkt);
        setze_eigenen(Some("nutzer:pw@kaputt"));
        assert_ne!(weg_fuer("https://huggingface.co/x"), Weg::Direkt);
        setze_eigenen(None);
    }

    #[test]
    fn lokale_ziele_ohne_systemproxy() {
        assert!(ist_lokal("http://127.0.0.1:4000/x"));
        assert!(ist_lokal("http://localhost/x"));
        assert!(ist_lokal("http://[::1]:80/"));
        assert!(!ist_lokal("https://huggingface.co/x"));
    }

    #[test]
    fn beschreibung_ohne_geheimnisse() {
        assert_eq!(Weg::Direkt.beschreibung(), "direkt");
        assert_eq!(
            Weg::Ueber("http://p.example.org:1".into()).beschreibung(),
            "Proxy http://p.example.org:1"
        );
    }
}
