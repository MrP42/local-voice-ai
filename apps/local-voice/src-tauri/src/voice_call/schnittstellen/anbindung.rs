//! Vertragsgrenze fuer DNS, TCP/TLS und atomaren Versand, noch ohne Netzadapter.
//! Adapter duerfen weder Proxys noch Alt-Svc/ALTSVC, Upgrade, Weiterleitungen oder
//! zusaetzliche Namensaufloesungen verwenden. TLS prueft den urspruenglichen Host.
use std::net::IpAddr;

use super::adressen::pruefe_netz;
use super::anfrage::{aus_geprueft, baue_adresse, pruefe_argumente};
use super::bestaetigung::{Freigabe, Uhr};
use super::bindung::aus_teilen;
use super::kodierung::Wert;

pub const MAX_AUFRUFE_JE_ANTWORT: usize = 5;

/// Felder bleiben privat: nach der Frage wird keine Anfrage neu gebaut oder veraendert.
pub struct Schnappschuss {
    anfrage: Wert,
    bindung: String,
    netz: String,
    schreibend: bool,
    ziel: Ziel,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ziel {
    pub host: String,
    pub port: u16,
    pub tls: bool,
}
impl Schnappschuss {
    /// Eintrag muss zuvor vom Verzeichnispruefer akzeptiert worden sein.
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
        Ok(Self {
            anfrage: aus_geprueft(e, &werte, adresse, geheimnis)?,
            bindung: b["bindung"].s().into(),
            netz: e[art]["netz"].s().into(),
            schreibend: e["zugriff"].s() == "schreibend",
            ziel,
        })
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
pub trait Namensaufloesung {
    /// Genau ein lokaler Aufloesungsvorgang; Fehler einschliesslich Proxy-Zwang.
    fn aufloesen(&mut self, host: &str) -> Result<Vec<String>, &'static str>;
}
pub trait Verbindung {
    /// Beginnt unmittelbar den Versand der unveraenderten Anfrage. Kein DNS/TLS.
    /// Keine automatische Wiederholung, auch nicht nach einem Schreibfehler.
    fn senden(&mut self, anfrage: &Wert) -> Result<(), &'static str>;
}
pub trait Verbindungsaufbau {
    type Verbunden: Verbindung;
    /// Nur `ip:ziel.port`, mit SNI und Host-Kopf `ziel.host`; keine Nebenanfragen.
    /// Baut TCP und gegebenenfalls TLS vollstaendig auf, sendet noch keine Anfrage.
    fn verbinden(&mut self, ziel: &Ziel, ip: IpAddr) -> Result<Self::Verbunden, &'static str>;
}
/// Reine Ablaufhuelle fuer die Abnahme der Adapter. Keine globale Verdrahtung.
pub fn versende(
    s: &Schnappschuss,
    frage: Option<(&Freigabe, u64)>,
    dns: &mut impl Namensaufloesung,
    verbindung: &mut impl Verbindungsaufbau,
    uhr: &impl Uhr,
) -> Result<(), &'static str> {
    if s.schreibend {
        let (freigabe, generation) = frage.ok_or("bestaetigung_abgelaufen")?;
        freigabe.bereit(generation, &s.bindung, uhr)?;
    }
    let adressen = if let Ok(ip) = s.ziel.host.parse::<IpAddr>() {
        vec![ip.to_string()]
    } else {
        dns.aufloesen(&s.ziel.host).map_err(|_| "netzfehler")?
    };
    match pruefe_netz(&s.netz, &adressen) {
        "ok" => {}
        "adresse_abgewiesen" => return Err("adresse_abgewiesen"),
        _ => return Err("netzfehler"),
    }
    let ip = adressen[0]
        .parse::<IpAddr>()
        .map_err(|_| "adresse_abgewiesen")?;
    let mut v = verbindung
        .verbinden(&s.ziel, ip)
        .map_err(|_| "netzfehler")?;
    if s.schreibend {
        let (freigabe, generation) = frage.ok_or("bestaetigung_abgelaufen")?;
        freigabe
            .absenden(generation, &s.bindung, uhr, || v.senden(&s.anfrage))?
            .map_err(|_| "unklar")
    } else {
        v.senden(&s.anfrage).map_err(|_| "netzfehler")
    }
}
