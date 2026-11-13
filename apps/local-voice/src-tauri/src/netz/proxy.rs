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
use std::net::{Ipv4Addr, Ipv6Addr};
use std::sync::atomic::{AtomicU64, Ordering};
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

/// Gesamtfrist für eine PAC-/WPAD-Auswertung; danach gilt der Standardweg.
const SYSTEM_FRIST: Duration = Duration::from_secs(20);

static EIGENER: RwLock<Option<String>> = RwLock::new(None);
/// Erhöht bei jedem Zurücksetzen; eine Auflösung, die vor dem Zurücksetzen
/// begann, trägt ihr Ergebnis danach nicht mehr ein.
static GENERATION: AtomicU64 = AtomicU64::new(0);
/// Aufgelöster Systemweg je Ziel-URL (ohne Query) mit Zeitpunkt; `None` = kein Systemproxy.
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
        GENERATION.fetch_add(1, Ordering::SeqCst);
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

/// Ziele auf diesem Rechner oder im lokalen Segment (Loopback, Link-Local,
/// `localhost`): nie über einen Proxy, nie PAC/WPAD.
pub fn ist_lokal(url: &str) -> bool {
    let Ok(u) = url::Url::parse(url) else {
        return false;
    };
    match u.host() {
        Some(url::Host::Domain(d)) => {
            let d = d.trim_end_matches('.').to_ascii_lowercase();
            d == "localhost" || d.ends_with(".localhost")
        }
        Some(url::Host::Ipv4(ip)) => v4_lokal(ip),
        Some(url::Host::Ipv6(ip)) => v6_lokal(ip),
        None => false,
    }
}

fn v4_lokal(ip: Ipv4Addr) -> bool {
    ip.is_loopback() || ip.is_link_local() || ip.is_unspecified()
}

fn v6_lokal(ip: Ipv6Addr) -> bool {
    if let Some(v4) = ip.to_ipv4_mapped() {
        return v4_lokal(v4);
    }
    ip.is_loopback() || ip.is_unspecified() || (ip.segments()[0] & 0xffc0) == 0xfe80
}

/// Darf eine Weiterleitung von `bisher` (alle vorherigen Ziele) nach `ziel`
/// gehen? Nie von einem entfernten auf ein lokales Ziel (sonst könnte ein
/// fremder Server die App über ihren Proxy oder direkt auf lokale Dienste lenken).
pub fn weiterleitung_erlaubt<'a>(bisher: impl IntoIterator<Item = &'a str>, ziel: &str) -> bool {
    !ist_lokal(ziel) || bisher.into_iter().all(ist_lokal)
}

/// PAC-Regeln dürfen nach Pfad und Port unterscheiden (WinHTTP wertet je URL
/// aus): Schlüssel ist die URL ohne Query und Fragment.
fn cache_schluessel(url: &str) -> String {
    match url::Url::parse(url) {
        Ok(mut u) => {
            u.set_query(None);
            u.set_fragment(None);
            u.to_string()
        }
        Err(_) => url.to_string(),
    }
}

/// Der Weg für `url`. Blockiert ggf. (PAC-Auswertung); aus async-Code
/// [`weg_fuer_async`] nehmen.
pub fn weg_fuer(url: &str) -> Weg {
    vorentscheidung(url).unwrap_or_else(|| systemweg_gecacht(url))
}

/// Was ohne Systemauflösung feststeht, und zwar verbindlich: lokale Ziele nie
/// über einen Proxy (auch nicht aus der Umgebung), dann die eigene Einstellung,
/// dann die Umgebungsvariablen. `None` = der Systemresolver entscheidet.
fn vorentscheidung(url: &str) -> Option<Weg> {
    if ist_lokal(url) {
        return Some(Weg::Direkt);
    }
    if let Some(p) = eigener() {
        return Some(if p == "direkt" {
            Weg::Direkt
        } else {
            Weg::Ueber(p)
        });
    }
    if umgebung_gesetzt(|k| std::env::var(k).ok()) {
        return Some(Weg::Standard);
    }
    None
}

/// PAC/WPAD über den Cache; nur aufrufen, wenn [`vorentscheidung`] `None` ist.
fn systemweg_gecacht(url: &str) -> Weg {
    let schluessel = cache_schluessel(url);
    let generation = GENERATION.load(Ordering::SeqCst);
    if let Ok(cache) = CACHE.lock() {
        if let Some((zeit, weg)) = cache.as_ref().and_then(|c| c.get(&schluessel)) {
            if zeit.elapsed() < CACHE_DAUER {
                return weg.clone().unwrap_or(Weg::Standard);
            }
        }
    }
    let weg = system_weg(url);
    if let Some(w) = &weg {
        log::info!("Systemproxy: {}", w.beschreibung());
    }
    if let Ok(mut cache) = CACHE.lock() {
        // Nach einem Zurücksetzen während der Auflösung nichts Altes eintragen.
        if GENERATION.load(Ordering::SeqCst) == generation {
            let c = cache.get_or_insert_with(HashMap::new);
            c.retain(|_, (zeit, _)| zeit.elapsed() < CACHE_DAUER);
            c.insert(schluessel, (Instant::now(), weg.clone()));
        }
    }
    weg.unwrap_or(Weg::Standard)
}

/// [`weg_fuer`] ohne den async-Laufzeitfaden zu blockieren. Die
/// verbindlichen Entscheidungen (lokal, eigene Einstellung, Umgebung) fallen
/// vorher und ohne Frist; nur die PAC-/WPAD-Auswertung läuft im
/// Blocking-Pool mit Gesamtfrist [`SYSTEM_FRIST`] (danach Standardweg).
pub async fn weg_fuer_async(url: &str) -> Weg {
    weg_mit(url, SYSTEM_FRIST, systemweg_gecacht).await
}

async fn weg_mit(url: &str, frist: Duration, system: fn(&str) -> Weg) -> Weg {
    if let Some(weg) = vorentscheidung(url) {
        return weg;
    }
    let url = url.to_string();
    match tokio::time::timeout(frist, tokio::task::spawn_blocking(move || system(&url))).await {
        Ok(Ok(weg)) => weg,
        Ok(Err(_)) => Weg::Standard,
        Err(_) => {
            // Der Auftrag läuft weiter und trägt sein Ergebnis in den Cache ein;
            // WinHTTP bietet keinen Abbruch einer laufenden PAC-Auswertung.
            log::warn!(
                "Proxy-Auswertung dauert über {}s; Standardweg",
                frist.as_secs()
            );
            Weg::Standard
        }
    }
}

/// Zukünftiger Weg einer Adresse (für injizierte Resolver in Tests).
pub(crate) type WegFuture = std::pin::Pin<Box<dyn std::future::Future<Output = Weg> + Send>>;

/// [`weg_fuer_async`] als [`WegFuture`] für ein eigenes `String`.
pub(crate) fn weg_future(url: String) -> WegFuture {
    Box::pin(async move { weg_fuer_async(&url).await })
}

/// Wendet `weg` auf einen Client-Bauplan an. Eine Adresse, die reqwest nicht
/// annimmt, fällt auf den Standard zurück (mit Warnung), statt den Download
/// zu verhindern.
pub fn anwenden(builder: reqwest::ClientBuilder, weg: &Weg) -> reqwest::ClientBuilder {
    match weg {
        Weg::Standard => builder,
        Weg::Direkt => builder.no_proxy(),
        // Je Anfrage (auch nach Weiterleitungen) prüfen: lokale Ziele nie über den Proxy.
        Weg::Ueber(p) => match url::Url::parse(p) {
            Ok(proxy_url) if reqwest::Proxy::all(proxy_url.as_str()).is_ok() => {
                builder.proxy(reqwest::Proxy::custom(move |ziel| {
                    (!ist_lokal(ziel.as_str())).then(|| proxy_url.clone())
                }))
            }
            _ => {
                log::warn!("Proxy {} nicht verwendbar", p);
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
    /// Schließt das WinHTTP-Handle auf jedem Weg (auch beim Abwickeln).
    struct Sitzung(*mut core::ffi::c_void);
    impl Drop for Sitzung {
        fn drop(&mut self) {
            // SAFETY: gültiges, genau einmal geschlossenes Handle aus WinHttpOpen.
            let _ = unsafe { WinHttpCloseHandle(self.0) };
        }
    }
    let sitzung = Sitzung(sitzung);
    // Auflösung, Verbindung, Senden, Empfangen je 10 s: die PAC-Datei liegt im
    // Intranet; hängt sie, soll der Download nicht ewig warten.
    let _ = unsafe { WinHttpSetTimeouts(sitzung.0, 10_000, 10_000, 10_000, 10_000) };

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
    let ergebnis = unsafe { WinHttpGetProxyForUrl(sitzung.0, &ziel, &mut optionen, &mut info) };
    drop(sitzung);
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
pub(crate) mod tests {
    use super::*;

    /// Tests, die den eigenen Proxy (globaler Zustand) setzen, laufen nacheinander.
    static EIGENER_SPERRE: Mutex<()> = Mutex::new(());

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
        let _sperre = EIGENER_SPERRE.lock().unwrap_or_else(|e| e.into_inner());
        setze_eigenen(Some("proxy.example.org:8080"));
        assert_eq!(
            weg_fuer("https://huggingface.co/x"),
            Weg::Ueber("http://proxy.example.org:8080".into())
        );
        setze_eigenen(Some("direkt"));
        assert_eq!(weg_fuer("https://huggingface.co/x"), Weg::Direkt);
        setze_eigenen(Some("http://nutzer:pw@kaputt.example.org:1"));
        assert_eq!(eigener(), None, "Zugangsdaten werden nie übernommen");
        setze_eigenen(None);
    }

    #[test]
    fn lokale_ziele_ohne_systemproxy() {
        for lokal in [
            "http://127.0.0.1:4000/x",
            "http://127.8.0.1/",
            "http://localhost/x",
            "http://LOCALHOST./x",
            "http://dienst.localhost/",
            "http://[::1]:80/",
            "http://[::ffff:127.0.0.1]/",
            "http://169.254.169.254/latest",
            "http://[fe80::1]/",
            "http://0.0.0.0/",
        ] {
            assert!(ist_lokal(lokal), "{lokal}");
            assert_eq!(weg_fuer(lokal), Weg::Direkt, "{lokal}");
        }
        for entfernt in [
            "https://huggingface.co/x",
            "https://localhost.example.org/",
            "http://10.0.0.1/",
            "http://[2001:db8::1]/",
        ] {
            assert!(!ist_lokal(entfernt), "{entfernt}");
        }
    }

    #[test]
    fn eigener_proxy_gilt_nie_fuer_lokale_ziele() {
        let _sperre = EIGENER_SPERRE.lock().unwrap_or_else(|e| e.into_inner());
        setze_eigenen(Some("proxy.example.org:8080"));
        assert_eq!(weg_fuer("http://127.0.0.1:9/"), Weg::Direkt);
        setze_eigenen(None);
    }

    #[test]
    fn keine_weiterleitung_von_entfernt_auf_lokal() {
        assert!(!weiterleitung_erlaubt(
            ["https://huggingface.co/a"],
            "http://127.0.0.1:8080/"
        ));
        assert!(!weiterleitung_erlaubt(
            ["https://huggingface.co/a"],
            "http://169.254.169.254/"
        ));
        assert!(weiterleitung_erlaubt(
            ["https://huggingface.co/a"],
            "https://cdn-lfs.hf.co/b"
        ));
        assert!(weiterleitung_erlaubt(
            ["http://127.0.0.1:1/a"],
            "http://127.0.0.1:1/b"
        ));
    }

    #[test]
    fn cache_unterscheidet_pfad_und_port_nicht_query() {
        assert_ne!(
            cache_schluessel("https://h.example.org/a"),
            cache_schluessel("https://h.example.org/b")
        );
        assert_ne!(
            cache_schluessel("https://h.example.org:444/a"),
            cache_schluessel("https://h.example.org/a")
        );
        assert_eq!(
            cache_schluessel("https://h.example.org/a?token=x"),
            cache_schluessel("https://h.example.org/a")
        );
    }

    /// Testserver (auch als Proxy nutzbar): merkt sich die erste Zeile jeder
    /// Anfrage und antwortet immer mit `antwort` (vollständige HTTP-Antwort).
    pub(crate) async fn testserver(
        antwort: String,
    ) -> (String, std::sync::Arc<Mutex<Vec<String>>>) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let adresse = format!("http://{}", listener.local_addr().unwrap());
        let gesehen = std::sync::Arc::new(Mutex::new(Vec::new()));
        let log = gesehen.clone();
        tokio::spawn(async move {
            while let Ok((mut sock, _)) = listener.accept().await {
                let mut buf = vec![0u8; 4096];
                let n = sock.read(&mut buf).await.unwrap_or(0);
                let zeile = String::from_utf8_lossy(&buf[..n])
                    .lines()
                    .next()
                    .unwrap_or("")
                    .to_string();
                log.lock().unwrap().push(zeile);
                let _ = sock.write_all(antwort.as_bytes()).await;
            }
        });
        (adresse, gesehen)
    }

    pub(crate) fn antwort(status: &str, kopf: &str) -> String {
        format!("HTTP/1.1 {status}\r\n{kopf}content-length: 0\r\nconnection: close\r\n\r\n")
    }

    pub(crate) fn gesehen(log: &std::sync::Arc<Mutex<Vec<String>>>) -> Vec<String> {
        log.lock().unwrap().clone()
    }

    const FRIST: Duration = Duration::from_secs(10);

    #[tokio::test]
    async fn ueber_proxy_geht_entfernt_ueber_proxy() {
        let (proxy, log) = testserver(antwort("502 Bad Gateway", "")).await;
        let client = anwenden(reqwest::Client::builder(), &Weg::Ueber(proxy))
            .build()
            .unwrap();
        let r = tokio::time::timeout(FRIST, client.get("http://modell.example.org/datei").send())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(r.status().as_u16(), 502);
        assert_eq!(
            gesehen(&log),
            ["GET http://modell.example.org/datei HTTP/1.1"]
        );
    }

    #[tokio::test]
    async fn ueber_proxy_laesst_lokale_ziele_direkt() {
        let (proxy, proxy_log) = testserver(antwort("502 Bad Gateway", "")).await;
        let (ziel, ziel_log) = testserver(antwort("204 No Content", "")).await;
        let client = anwenden(reqwest::Client::builder(), &Weg::Ueber(proxy))
            .build()
            .unwrap();
        let r = tokio::time::timeout(FRIST, client.get(format!("{ziel}/lokal")).send())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(r.status().as_u16(), 204);
        assert_eq!(gesehen(&ziel_log), ["GET /lokal HTTP/1.1"]);
        assert!(
            gesehen(&proxy_log).is_empty(),
            "der Proxy darf nichts sehen"
        );
    }

    fn langsamer_resolver(_: &str) -> Weg {
        std::thread::sleep(Duration::from_millis(500));
        Weg::Ueber("http://falsch.example.org:1".into())
    }

    /// R2-B1: Hängt der Systemresolver, bleiben lokal und „direkt“ verbindlich.
    #[tokio::test]
    // Die Sperre ordnet nur Tests nacheinander; `weg_mit` kehrt bei „direkt“
    // ohne Warten zurück.
    #[allow(clippy::await_holding_lock)]
    async fn resolverfrist_hebt_lokal_und_direkt_nicht_auf() {
        let kurz = Duration::from_millis(20);
        for lokal in [
            "http://127.0.0.1:9/",
            "http://[::1]/",
            "http://169.254.1.1/",
        ] {
            assert_eq!(weg_mit(lokal, kurz, langsamer_resolver).await, Weg::Direkt);
        }
        {
            let _sperre = EIGENER_SPERRE.lock().unwrap_or_else(|e| e.into_inner());
            setze_eigenen(Some("direkt"));
            let weg = weg_mit("https://huggingface.co/x", kurz, langsamer_resolver).await;
            setze_eigenen(None);
            assert_eq!(weg, Weg::Direkt);
        }
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
