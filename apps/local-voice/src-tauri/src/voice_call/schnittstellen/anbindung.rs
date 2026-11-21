//! Ablaufhuelle fuer DNS, TCP/TLS und atomaren Versand mit Zeitlimit, Abbruch und
//! Antwortgroessengrenze. Netzadapter beachten diese Grenzen ueber die Traits.
//! Adapter duerfen weder Proxys noch Alt-Svc/ALTSVC, Upgrade, Weiterleitungen oder
//! zusaetzliche Namensaufloesungen verwenden. TLS prueft den urspruenglichen Host.
use std::net::IpAddr;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::time::{Duration, Instant};

use super::adressen::pruefe_netz;
use super::anfrage::{aus_geprueft, baue_adresse, pruefe_argumente};
use super::bestaetigung::Freigabe;
use super::bindung::aus_teilen;
use super::kodierung::{kanonisch, Wert};
use super::objekt;
use super::rahmung::{status_ergebnis, MAX_TRANSPORT};

pub const MAX_AUFRUFE_JE_ANTWORT: usize = 5;
pub const MAX_ANTWORT_BYTES: usize = MAX_TRANSPORT;

#[derive(Clone, Debug, Default)]
pub struct Abbruch(Arc<AtomicBool>);
impl Abbruch {
    pub fn neu() -> Self {
        Self::default()
    }
    pub fn ausloesen(&self) {
        self.0.store(true, Ordering::SeqCst);
    }
    pub fn ausgeloest(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
}

pub struct Lauf {
    ende: Instant,
    abbruch: Abbruch,
    verbraucht: AtomicBool,
}
impl Lauf {
    fn verbrauchen(&self) -> bool {
        self.verbraucht.swap(true, Ordering::SeqCst)
    }
    pub fn ende(&self) -> Instant {
        self.ende
    }
    pub fn rest(&self) -> Option<Duration> {
        let jetzt = Instant::now();
        (jetzt < self.ende).then(|| self.ende.duration_since(jetzt))
    }
    pub fn pruefen(&self) -> Result<(), &'static str> {
        if self.abbruch.ausgeloest() {
            Err("abgebrochen")
        } else if self.rest().is_none() {
            Err("zeitlimit")
        } else {
            Ok(())
        }
    }
    pub fn abbruch(&self) -> &Abbruch {
        &self.abbruch
    }
    fn ohne_abbruch(&self) -> Lauf {
        Lauf {
            ende: self.ende,
            abbruch: Abbruch::neu(),
            verbraucht: AtomicBool::new(false),
        }
    }
    #[cfg(test)]
    pub(crate) fn fuer_test(frist: Duration, abbruch: Abbruch) -> Lauf {
        Lauf {
            ende: Instant::now() + frist,
            abbruch,
            verbraucht: AtomicBool::new(false),
        }
    }
}

#[derive(Default)]
pub struct Antwortrunde {
    aufrufe: usize,
}
impl Antwortrunde {
    pub fn neu() -> Self {
        Self::default()
    }
    /// Einziger produktiver Zugang zu einem Lauf: die privaten Felder sichern
    /// die Aufrufgrenze fuer eine Antwort gegen Umgehung ab.
    pub fn naechster_aufruf(
        &mut self,
        s: &Schnappschuss,
        abbruch: &Abbruch,
    ) -> Result<Lauf, &'static str> {
        if self.aufrufe >= MAX_AUFRUFE_JE_ANTWORT {
            return Err("abgelehnt");
        }
        self.aufrufe += 1;
        Ok(Lauf {
            ende: Instant::now() + Duration::from_millis(s.zeitlimit_ms()),
            abbruch: abbruch.clone(),
            verbraucht: AtomicBool::new(false),
        })
    }
}

/// Felder bleiben privat: nach der Frage wird keine Anfrage neu gebaut oder veraendert.
pub struct Schnappschuss {
    anfrage: Wert,
    bindung: String,
    netz: String,
    schreibend: bool,
    ziel: Ziel,
    zeitlimit_ms: u64,
    antwort_max_bytes: usize,
    kennung: String,
    art: String,
    abfluss: String,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ziel {
    pub host: String,
    pub port: u16,
    pub tls: bool,
}
impl Schnappschuss {
    /// Eintrag muss zuvor vom Verzeichnispruefer akzeptiert worden sein.
    /// Nur HTTP/Web; andere Arten liefern `art_nicht_unterstuetzt`.
    pub fn neu(e: &Wert, a: &Wert, geheimnis: Option<&Wert>) -> Result<Self, Wert> {
        let werte = pruefe_argumente(e, a)?;
        let adresse = baue_adresse(e, &werte)?;
        let art = e["art"].s();
        let methode = if art == "web" {
            "GET"
        } else {
            e["http"]["methode"].s()
        };
        let b = aus_teilen(
            e["id"].s(),
            &format!("schnittstelle.{art}"),
            &format!("{methode} {adresse}"),
            &werte,
        )?;
        let (schema, rest) = adresse.split_once("://").expect("gepruefte Adresse");
        let autoritaet = rest.split('/').next().expect("Autoritaet");
        let (host, port) = if let Some((h, p)) = autoritaet.split_once(':') {
            (h, p.parse().expect("gepruefter Port"))
        } else {
            (autoritaet, if schema == "https" { 443 } else { 80 })
        };
        let ziel = Ziel {
            host: host.into(),
            port,
            tls: schema == "https",
        };
        let anfrage = aus_geprueft(e, &werte, adresse, geheimnis)?;
        super::http1::baue_anfragebytes(&ziel, &anfrage)
            .map_err(|_| super::fehler("anfrage_ungueltig", None))?;
        Ok(Self {
            anfrage,
            bindung: b["bindung"].s().into(),
            netz: e[art]["netz"].s().into(),
            schreibend: e["zugriff"].s() == "schreibend",
            ziel,
            zeitlimit_ms: e["zeitlimit_ms"].ganz().unwrap_or(15_000).max(0) as u64,
            antwort_max_bytes: usize::try_from(
                e[art]["antwort_max_bytes"].ganz().unwrap_or(20_480).max(0),
            )
            .unwrap_or(usize::MAX),
            kennung: e["id"].s().into(),
            art: art.into(),
            abfluss: e["abfluss"].text().unwrap_or("netz").into(),
        })
    }
    pub fn zeitlimit_ms(&self) -> u64 {
        self.zeitlimit_ms
    }
    pub fn antwort_max_bytes(&self) -> usize {
        self.antwort_max_bytes
    }
    pub fn kennung(&self) -> &str {
        &self.kennung
    }
    pub fn art(&self) -> &str {
        &self.art
    }
    pub fn abfluss(&self) -> &str {
        &self.abfluss
    }
    pub fn schreibend(&self) -> bool {
        self.schreibend
    }
    pub fn ziel(&self) -> &Ziel {
        &self.ziel
    }
    pub fn anfrage(&self) -> &Wert {
        &self.anfrage
    }
    pub fn bindung(&self) -> &str {
        &self.bindung
    }
    /// Vorschau/Fragetext bezieht sich auf genau diese geprueften Bytes.
    /// Kopfzeilen koennen Geheimnisse enthalten und duerfen nicht angezeigt werden.
    pub fn methode_adresse_koerper(&self) -> (&str, &str, &Wert) {
        (
            self.anfrage["methode"].s(),
            self.anfrage["adresse"].s(),
            &self.anfrage["koerper"],
        )
    }
}
pub struct Antwort {
    pub status: u16,
    pub koerper: Vec<u8>,
    pub empfangen_bytes: usize,
}
impl Antwort {
    pub fn ergebnis(&self) -> &'static str {
        status_ergebnis(self.status)
    }
}

#[derive(Debug, PartialEq)]
pub struct Ausgang {
    pub ergebnis: &'static str,
    pub status: Option<u16>,
    pub koerper: Vec<u8>,
    pub dauer_ms: u64,
    pub gesendet_bytes: usize,
    pub empfangen_bytes: usize,
}

/// Adapter melden netzfehler, zeitlimit, abgebrochen, zu_gross oder antwort_ungueltig.
/// Sie beobachten `lauf` waehrend jedes Wartens; unbekannte Fehler gelten als netzfehler.
pub trait Namensaufloesung {
    /// Genau ein lokaler Aufloesungsvorgang; Fehler einschliesslich Proxy-Zwang.
    fn aufloesen(&mut self, host: &str, lauf: &Lauf) -> Result<Vec<String>, &'static str>;
}
/// Adapter beobachten `lauf` waehrend jedes Wartens und melden die Fehlerwoerter
/// wie bei der Namensaufloesung.
pub trait Verbindung {
    /// Beginnt unmittelbar den Versand der unveraenderten Anfrage. Kein DNS/TLS.
    /// Muss nicht blockierend sein: kein Warten auf Schreibbereitschaft oder Antwort.
    /// Der Aufruf erfolgt bei schreibenden Anfragen unter dem Freigabe-Mutex.
    /// Keine automatische Wiederholung, auch nicht nach einem Schreibfehler.
    fn senden(&mut self, anfrage: &Wert) -> Result<(), &'static str>;
    /// Liest Status und Koerper und achtet waehrend jedes Wartens auf `lauf.pruefen()`.
    /// Mehr als `max_bytes` Koerperbytes ergeben zu_gross, ohne den Rest zu lesen.
    fn antwort(&mut self, lauf: &Lauf, max_bytes: usize) -> Result<Antwort, &'static str>;
    /// Tatsaechlich gesendete Bytes, auch nach einem fehlgeschlagenen Versand.
    fn gesendet_bytes(&self) -> usize;
}
/// Adapter beobachten `lauf` waehrend jedes Wartens und melden die Fehlerwoerter
/// wie bei der Namensaufloesung.
pub trait Verbindungsaufbau {
    type Verbunden: Verbindung;
    /// Nur `ip:ziel.port`, mit SNI und Host-Kopf `ziel.host`; keine Nebenanfragen.
    /// Baut TCP und gegebenenfalls TLS vollstaendig auf, sendet noch keine Anfrage.
    fn verbinden(
        &mut self,
        ziel: &Ziel,
        ip: IpAddr,
        lauf: &Lauf,
    ) -> Result<Self::Verbunden, &'static str>;
}
/// Reine Ablaufhuelle fuer die Abnahme der Adapter. Keine globale Verdrahtung.
pub fn versende(
    s: &Schnappschuss,
    frage: Option<(&Freigabe, u64)>,
    dns: &mut impl Namensaufloesung,
    verbindung: &mut impl Verbindungsaufbau,
    lauf: &Lauf,
) -> Ausgang {
    if lauf.verbrauchen() {
        return Ausgang {
            ergebnis: "abgelehnt",
            status: None,
            koerper: vec![],
            dauer_ms: 0,
            gesendet_bytes: 0,
            empfangen_bytes: 0,
        };
    }
    let start = Instant::now();
    let mut gesendet = 0;
    let ergebnis = lauf_ab(s, frage, dns, verbindung, lauf, &mut gesendet);
    let dauer_ms = u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX);
    match ergebnis {
        Ok(a) => {
            let ergebnis = a.ergebnis();
            Ausgang {
                ergebnis,
                status: Some(a.status),
                koerper: if matches!(ergebnis, "ok" | "http_fehler") {
                    a.koerper
                } else {
                    vec![]
                },
                dauer_ms,
                gesendet_bytes: gesendet,
                empfangen_bytes: a.empfangen_bytes,
            }
        }
        Err(ergebnis) => Ausgang {
            ergebnis,
            status: None,
            koerper: vec![],
            dauer_ms,
            gesendet_bytes: gesendet,
            empfangen_bytes: 0,
        },
    }
}

fn vor_versand_fehler(art: &'static str) -> &'static str {
    match art {
        "zeitlimit" | "abgebrochen" => art,
        _ => "netzfehler",
    }
}

fn lauf_ab(
    s: &Schnappschuss,
    frage: Option<(&Freigabe, u64)>,
    dns: &mut impl Namensaufloesung,
    verbindung: &mut impl Verbindungsaufbau,
    lauf: &Lauf,
    gesendet: &mut usize,
) -> Result<Antwort, &'static str> {
    lauf.pruefen()?;
    if s.schreibend {
        let (freigabe, generation) = frage.ok_or("bestaetigung_abgelaufen")?;
        freigabe.bereit(generation, &s.bindung)?;
    }
    let adressen = if let Ok(ip) = s.ziel.host.parse::<IpAddr>() {
        vec![ip.to_string()]
    } else {
        dns.aufloesen(&s.ziel.host, lauf)
            .map_err(vor_versand_fehler)?
    };
    lauf.pruefen()?;
    match pruefe_netz(&s.netz, &adressen) {
        "ok" => {}
        "adresse_abgewiesen" => return Err("adresse_abgewiesen"),
        _ => return Err("netzfehler"),
    }
    let ip = adressen[0]
        .parse::<IpAddr>()
        .map_err(|_| "adresse_abgewiesen")?;
    let mut v = verbindung
        .verbinden(&s.ziel, ip, lauf)
        .map_err(vor_versand_fehler)?;
    lauf.pruefen()?;
    if s.schreibend {
        let (freigabe, generation) = frage.ok_or("bestaetigung_abgelaufen")?;
        let versand = freigabe.absenden(generation, &s.bindung, || v.senden(&s.anfrage))?;
        *gesendet = v.gesendet_bytes();
        versand.map_err(|_| "unklar")?;
        let antwort = v.antwort(&lauf.ohne_abbruch(), MAX_ANTWORT_BYTES);
        // Der Versand laeuft im Hintergrund: erst nach der Antwort steht die Zahl fest.
        *gesendet = v.gesendet_bytes();
        antwort.map_err(|art| {
            if art == "zu_gross" {
                "zu_gross"
            } else {
                "unklar"
            }
        })
    } else {
        let versand = v.senden(&s.anfrage);
        *gesendet = v.gesendet_bytes();
        versand.map_err(|_| "netzfehler")?;
        let antwort = v.antwort(lauf, MAX_ANTWORT_BYTES);
        *gesendet = v.gesendet_bytes();
        antwort.map_err(|art| match art {
            "zeitlimit" | "abgebrochen" | "zu_gross" | "antwort_ungueltig" => art,
            _ => "netzfehler",
        })
    }
}

pub fn vertragsprotokoll(
    s: &Schnappschuss,
    a: &Ausgang,
    assistent: &str,
    zeit: &str,
    bestaetigt: bool,
) -> Result<String, &'static str> {
    let null = super::kodierung::lies_json("null").expect("Festes JSON-Literal");
    let bestaetigt = super::kodierung::lies_json(if bestaetigt { "true" } else { "false" })
        .expect("Festes JSON-Literal");
    super::rahmung::protokoll(&objekt([
        ("zeit", zeit.into()),
        ("assistent", assistent.into()),
        ("schnittstelle", s.kennung().into()),
        ("art", s.art().into()),
        (
            "zugriff",
            if s.schreibend() {
                "schreibend"
            } else {
                "lesend"
            }
            .into(),
        ),
        ("abfluss", s.abfluss().into()),
        ("ergebnis", a.ergebnis.into()),
        (
            "dauer_ms",
            Wert::from(i64::try_from(a.dauer_ms).unwrap_or(i64::MAX)),
        ),
        (
            "status",
            a.status
                .map_or(null, |status| Wert::from(i64::from(status))),
        ),
        (
            "gesendet_bytes",
            Wert::from(i64::try_from(a.gesendet_bytes).unwrap_or(i64::MAX)),
        ),
        (
            "empfangen_bytes",
            Wert::from(i64::try_from(a.empfangen_bytes).unwrap_or(i64::MAX)),
        ),
        ("server", s.ziel().host.as_str().into()),
        ("bestaetigt", bestaetigt),
    ]))
}

pub fn protokollzeile(s: &Schnappschuss, a: &Ausgang) -> String {
    kanonisch(&objekt([
        ("kennung", s.kennung().into()),
        ("art", s.art().into()),
        (
            "zugriff",
            if s.schreibend() {
                "schreibend"
            } else {
                "lesend"
            }
            .into(),
        ),
        ("ergebnis", a.ergebnis.into()),
        (
            "dauer_ms",
            Wert::from(i64::try_from(a.dauer_ms).unwrap_or(i64::MAX)),
        ),
        (
            "gesendet_bytes",
            Wert::from(i64::try_from(a.gesendet_bytes).unwrap_or(i64::MAX)),
        ),
        (
            "empfangen_bytes",
            Wert::from(i64::try_from(a.empfangen_bytes).unwrap_or(i64::MAX)),
        ),
    ]))
    .expect("Protokoll aus Texten und Ganzzahlen")
}

pub fn protokolliere(s: &Schnappschuss, a: &Ausgang) {
    let zeile = protokollzeile(s, a);
    log::info!("lva.schnittstelle {zeile}");
}
