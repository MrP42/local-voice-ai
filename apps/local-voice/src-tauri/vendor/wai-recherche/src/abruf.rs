//! Sicherer Abruf von Webseiten (ADR-0026 par. 4 Schritt 4).
//!
//! * nur http/https, keine Zugangsdaten in der URL
//! * DNS wird selbst aufgeloest; JEDE Adresse muss den `IpPruefer` bestehen; verbunden wird auf die
//!   geprueften Adressen (`resolve_to_addrs`), damit kein zweiter DNS-Lauf umgeleitet werden kann
//! * Weiterleitungen werden manuell gefolgt (hoechstens 5), jeder Sprung wird neu geprueft
//! * Groessenlimit mit Streaming-Abbruch, Zeitlimit, keine Cookies, kein JavaScript, kein System-Proxy
//!   (ein Proxy wuerde die Adresspruefung umgehen)
//! * nur `text/html`, `text/plain`, `application/xhtml+xml`
//! * nur Ports 80, 443, 8080, 8443 (weitere per `AbrufKonfig::erlaubte_ports`)
//! * `robots.txt` wird beachtet (Cache je Ursprung im Speicher, hoechstens 512 Hosts, 24 h). Der
//!   robots-Abruf zaehlt nicht als Fetch im Budget, liegt aber unter dem Gesamtzeitlimit.
//! * Abrufe laufen nur budgetiert (`abrufen_mit_budget`); das Sekundenbudget deckelt das Zeitlimit
//!
//! Bezahlschranken werden nicht umgangen: es gibt keine Anmelde-, Cookie- oder Umgehungslogik; eine
//! Seite hinter einer Schranke liefert nur, was ohne Anmeldung ausgeliefert wird.

use std::net::{IpAddr, SocketAddr};
use std::sync::Mutex;
use std::time::Duration;

use url::{Host, Url};

use crate::budget::BudgetZaehler;
use crate::fehler::RechercheFehler;
use crate::ip::IpPruefer;
use crate::robots::{Robots, RobotsCache};

const ROBOTS_MAX_HOSTS: usize = 512;
const ROBOTS_TTL: Duration = Duration::from_secs(24 * 3600);
const ERLAUBTE_TYPEN: [&str; 3] = ["text/html", "text/plain", "application/xhtml+xml"];

#[derive(Debug, Clone)]
pub struct AbrufKonfig {
    pub max_bytes: usize,
    pub zeitlimit: Duration,
    pub max_weiterleitungen: usize,
    pub user_agent: String,
    /// Kennung fuer robots.txt (Kleinbuchstaben).
    pub robots_agent: String,
    /// Erlaubte Zielports; weitere nur bewusst per Konfiguration.
    pub erlaubte_ports: Vec<u16>,
}

impl Default for AbrufKonfig {
    fn default() -> Self {
        Self {
            max_bytes: 2 * 1024 * 1024,
            zeitlimit: Duration::from_secs(15),
            max_weiterleitungen: 5,
            user_agent: format!("WAI-Recherche/{}", crate::VERSION),
            robots_agent: "wai-recherche".into(),
            erlaubte_ports: vec![80, 443, 8080, 8443],
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Abruf {
    /// Endgueltige URL nach Weiterleitungen.
    pub url: String,
    /// Medientyp ohne Parameter, kleingeschrieben.
    pub inhaltstyp: String,
    /// Inhalt als UTF-8 (verlustbehaftet); Zeichensatzangaben der Seite werden nicht ausgewertet.
    pub text: String,
}

struct Roh {
    url: Url,
    inhaltstyp: String,
    body: Vec<u8>,
}

enum Einzel {
    Fertig(Roh),
    Weiter(Url),
}

pub struct SichererAbruf {
    pruefer: IpPruefer,
    konfig: AbrufKonfig,
    robots: Mutex<RobotsCache>,
}

impl Default for SichererAbruf {
    fn default() -> Self {
        Self::mit(IpPruefer::streng(), AbrufKonfig::default())
    }
}

impl SichererAbruf {
    pub fn neu() -> Self {
        Self::default()
    }

    pub fn mit(pruefer: IpPruefer, konfig: AbrufKonfig) -> Self {
        Self {
            pruefer,
            konfig,
            robots: Mutex::new(RobotsCache::neu(ROBOTS_MAX_HOSTS, ROBOTS_TTL)),
        }
    }

    /// Einziger oeffentlicher Abrufpfad: verbucht einen Abruf (Ueberschreitung = Fehler) und begrenzt das
    /// Zeitlimit auf `min(zeitlimit, Restzeit des Sekundenbudgets)`.
    pub async fn abrufen_mit_budget(
        &self,
        url: &str,
        budget: &mut BudgetZaehler,
    ) -> Result<Abruf, RechercheFehler> {
        budget.abruf()?;
        let limit = self.konfig.zeitlimit.min(budget.verbleibend());
        self.abrufen(url, limit).await
    }

    pub(crate) async fn abrufen(
        &self,
        url: &str,
        zeitlimit: Duration,
    ) -> Result<Abruf, RechercheFehler> {
        let start = Url::parse(url).map_err(|_| RechercheFehler::UngueltigeUrl)?;
        let lauf = self.folge(start);
        let roh = tokio::time::timeout(zeitlimit, lauf)
            .await
            .map_err(|_| RechercheFehler::Zeitueberschreitung)??;
        Ok(Abruf {
            url: roh.url.to_string(),
            inhaltstyp: roh.inhaltstyp,
            text: String::from_utf8_lossy(&roh.body).into_owned(),
        })
    }

    async fn folge(&self, start: Url) -> Result<Roh, RechercheFehler> {
        let mut aktuell = start;
        for _ in 0..=self.konfig.max_weiterleitungen {
            self.robots_pruefen(&aktuell).await?;
            match self.einzelabruf(&aktuell, true).await? {
                Einzel::Fertig(r) => return Ok(r),
                Einzel::Weiter(ziel) => aktuell = ziel,
            }
        }
        Err(RechercheFehler::ZuVieleWeiterleitungen)
    }

    /// Wie `folge`, aber ohne robots-Pruefung und ohne Typfilter (fuer robots.txt selbst).
    async fn folge_roh(&self, start: Url) -> Result<Roh, RechercheFehler> {
        let mut aktuell = start;
        for _ in 0..=self.konfig.max_weiterleitungen {
            match self.einzelabruf(&aktuell, false).await? {
                Einzel::Fertig(r) => return Ok(r),
                Einzel::Weiter(ziel) => aktuell = ziel,
            }
        }
        Err(RechercheFehler::ZuVieleWeiterleitungen)
    }

    fn schema_pruefen(&self, url: &Url) -> Result<(), RechercheFehler> {
        if !matches!(url.scheme(), "http" | "https") {
            return Err(RechercheFehler::SchemaNichtErlaubt);
        }
        if !url.username().is_empty() || url.password().is_some() {
            return Err(RechercheFehler::UngueltigeUrl);
        }
        let port = url
            .port_or_known_default()
            .ok_or(RechercheFehler::UngueltigeUrl)?;
        if !self.konfig.erlaubte_ports.contains(&port) {
            return Err(RechercheFehler::PortNichtErlaubt);
        }
        Ok(())
    }

    /// Loest den Host auf und prueft ALLE Adressen; gibt Domain (falls Name) und Adressen zurueck.
    async fn aufloesen(
        &self,
        url: &Url,
    ) -> Result<(Option<String>, Vec<SocketAddr>), RechercheFehler> {
        let port = url
            .port_or_known_default()
            .ok_or(RechercheFehler::UngueltigeUrl)?;
        let (domain, ips): (Option<String>, Vec<IpAddr>) = match url.host() {
            None => return Err(RechercheFehler::UngueltigeUrl),
            Some(Host::Ipv4(a)) => (None, vec![IpAddr::V4(a)]),
            Some(Host::Ipv6(a)) => (None, vec![IpAddr::V6(a)]),
            Some(Host::Domain(d)) => {
                let adressen = tokio::net::lookup_host((d, port))
                    .await
                    .map_err(|_| RechercheFehler::DnsFehler)?;
                (Some(d.to_string()), adressen.map(|s| s.ip()).collect())
            }
        };
        if ips.is_empty() {
            return Err(RechercheFehler::DnsFehler);
        }
        for ip in &ips {
            self.pruefer.pruefe(*ip)?;
        }
        Ok((
            domain,
            ips.into_iter()
                .map(|ip| SocketAddr::new(ip, port))
                .collect(),
        ))
    }

    async fn einzelabruf(&self, url: &Url, typfilter: bool) -> Result<Einzel, RechercheFehler> {
        self.schema_pruefen(url)?;
        let (domain, adressen) = self.aufloesen(url).await?;
        let mut b = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(self.konfig.zeitlimit)
            .user_agent(self.konfig.user_agent.clone())
            .no_gzip()
            .no_brotli()
            .no_deflate()
            .no_proxy();
        if let Some(d) = &domain {
            b = b.resolve_to_addrs(d, &adressen);
        }
        let client = b.build().map_err(|_| RechercheFehler::Netzwerk)?;
        let mut antwort = client
            .get(url.clone())
            .header(
                "Accept",
                "text/html, application/xhtml+xml, text/plain;q=0.8",
            )
            .send()
            .await
            .map_err(|e| {
                if e.is_timeout() {
                    RechercheFehler::Zeitueberschreitung
                } else {
                    RechercheFehler::Netzwerk
                }
            })?;
        let status = antwort.status();
        if status.is_redirection() {
            let ort = antwort
                .headers()
                .get("location")
                .and_then(|v| v.to_str().ok())
                .ok_or(RechercheFehler::HttpStatus(status.as_u16()))?;
            let ziel = url.join(ort).map_err(|_| RechercheFehler::UngueltigeUrl)?;
            return Ok(Einzel::Weiter(ziel));
        }
        if !status.is_success() {
            return Err(RechercheFehler::HttpStatus(status.as_u16()));
        }
        let inhaltstyp = antwort
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .map(|v| {
                v.split(';')
                    .next()
                    .unwrap_or("")
                    .trim()
                    .to_ascii_lowercase()
            })
            .unwrap_or_default();
        if typfilter && !ERLAUBTE_TYPEN.contains(&inhaltstyp.as_str()) {
            return Err(RechercheFehler::InhaltstypNichtErlaubt);
        }
        if antwort
            .content_length()
            .is_some_and(|l| l > self.konfig.max_bytes as u64)
        {
            return Err(RechercheFehler::ZuGross);
        }
        let mut body = Vec::new();
        while let Some(teil) = antwort.chunk().await.map_err(|e| {
            if e.is_timeout() {
                RechercheFehler::Zeitueberschreitung
            } else {
                RechercheFehler::Netzwerk
            }
        })? {
            if body.len() + teil.len() > self.konfig.max_bytes {
                return Err(RechercheFehler::ZuGross);
            }
            body.extend_from_slice(&teil);
        }
        Ok(Einzel::Fertig(Roh {
            url: url.clone(),
            inhaltstyp,
            body,
        }))
    }

    async fn robots_pruefen(&self, url: &Url) -> Result<(), RechercheFehler> {
        self.schema_pruefen(url)?;
        let ursprung = url.origin().ascii_serialization();
        let gespeichert = self
            .robots
            .lock()
            .map_err(|_| RechercheFehler::Netzwerk)?
            .holen(&ursprung);
        let regeln = match gespeichert {
            Some(r) => r,
            None => {
                let r = self.robots_laden(url).await?;
                self.robots
                    .lock()
                    .map_err(|_| RechercheFehler::Netzwerk)?
                    .setzen(ursprung, r.clone());
                r
            }
        };
        let mut pfad = url.path().to_string();
        if let Some(q) = url.query() {
            pfad.push('?');
            pfad.push_str(q);
        }
        if regeln.erlaubt(&pfad) {
            Ok(())
        } else {
            Err(RechercheFehler::RobotsVerbietet)
        }
    }

    /// 2xx: auswerten; 4xx: keine Datei, alles erlaubt (RFC 9309); 5xx: nicht lesbar, Abruf unterbleiben.
    async fn robots_laden(&self, url: &Url) -> Result<Robots, RechercheFehler> {
        let ziel = url
            .join("/robots.txt")
            .map_err(|_| RechercheFehler::UngueltigeUrl)?;
        match self.folge_roh(ziel).await {
            Ok(roh) => Ok(Robots::parse(
                &String::from_utf8_lossy(&roh.body),
                &self.konfig.robots_agent,
            )),
            Err(RechercheFehler::HttpStatus(s)) if (400..500).contains(&s) => {
                Ok(Robots::alles_erlaubt())
            }
            Err(RechercheFehler::HttpStatus(_)) => Err(RechercheFehler::RobotsVerbietet),
            Err(e) => Err(e),
        }
    }
}
