//! `PortalHub`: der Assistent-Sync-Hub im wai-portal als [`HubSpeicher`] (lva#388, ADR-0028 E3).
//!
//! Spricht die Endpunkte unter `/api/v1/assistent/sync/` (Doku: wai-portal `docs/ASSISTENT-SYNC.md`)
//! und bildet deren Fehlercodes auf [`HubFehler`] des Sync-Kerns ab. Der Hub sieht nur Steuerfelder,
//! Chiffrat und Nonce; dieser Baustein kennt keinen Schlüssel.
//!
//! Der Transport steckt hinter [`HubTransport`]: In der App ist es der Hub-Client des
//! Bibliotheks-Syncs (selbes Token, selber HTTP-Weg einschließlich curl unter Windows, siehe
//! `assistent_sync::ClientTransport`), in Tests ein Hub im Arbeitsspeicher.
//!
//! Regeln:
//! - Zähler (`nach_revision`, `erwartete_*`, `cursor`, `key_epoch`) gehen als Int64String
//!   (`"17"`) über die Leitung, nie als JSON-Zahl.
//! - Der authentifizierte Absender ist das Gerät des Tokens. Eine fremde [`AuthGeraet`] oder ein
//!   fremdes `geraet_id` im Umschlag wird schon hier abgewiesen, ohne Netzaufruf.
//! - Kein Inhalt im Log: Es wird höchstens der Fehlercode des Hubs (fester Kurztext) und der
//!   HTTP-Status protokolliert, nie Umschläge, Chiffrat, Kennungen oder Geräte-IDs.

use std::fmt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use serde_json::{json, Value};
use wai_assistent::sync::{
    AuthGeraet, Hex128, HubFehler, HubSpeicher, LeseFilter, Sammlung, Schreiben, Umschlag,
};

/// Pfadteil hinter `/api/v1/assistent/sync`.
pub const BASIS: &str = "/api/v1/assistent/sync";

/// Höchstlänge des Chiffrats (Base64-Zeichen), die der Hub annimmt (Hoster-Grenze, Hub-Doku).
pub const MAX_CIPHERTEXT: usize = 4_300_000;

/// Seitengröße beim Lesen (Höchstwert des Hubs).
const LESE_SEITE: u32 = 100;

/// Obergrenze der Lese-Seiten je Aufruf (Schutz vor einem Hub, der nie aufhört).
const MAX_SEITEN: usize = 100_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Methode {
    Get,
    Post,
}

/// Eine Anfrage an den Hub. `pfad` steht hinter [`BASIS`], z. B. `/schreiben`.
#[derive(Debug, Clone)]
pub struct Anfrage {
    pub methode: Methode,
    pub pfad: &'static str,
    pub query: Vec<(&'static str, String)>,
    pub body: Option<Value>,
}

/// HTTP-Status und JSON-Körper (bei Nicht-JSON `Null`).
#[derive(Debug, Clone)]
pub struct Antwort {
    pub status: u16,
    pub body: Value,
}

/// Der Hub war nicht zu erreichen (Netz, Zeitüberschreitung). Ohne Text, damit nie etwas Fremdes
/// in Meldungen oder Log gelangt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TransportFehler;

pub trait HubTransport: Send + Sync {
    fn senden(&self, anfrage: &Anfrage) -> Result<Antwort, TransportFehler>;
}

/// Einmalige Freigabe einer Kennung für ein weiteres Gerät (`POST /freigeben`).
#[derive(Clone, PartialEq, Eq)]
pub struct Freigabe {
    /// Einmalgeheimnis (base64url), nur in der Antwort des Hubs; geht auf einem eigenen Kanal an das neue Gerät.
    pub code: String,
    pub gueltig_bis: String,
}

impl fmt::Debug for Freigabe {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Freigabe")
            .field("code", &"***")
            .field("gueltig_bis", &self.gueltig_bis)
            .finish()
    }
}

/// Der Hub als [`HubSpeicher`].
pub struct PortalHub<T: HubTransport> {
    pub(crate) transport: T,
    geraet_id: String,
    letzter_code: Mutex<Option<String>>,
    /// Gemeinsame Schranke: Ist sie gesetzt, geht KEIN weiterer Aufruf ans Netz (Ausschalten).
    sperre: Arc<AtomicBool>,
}

/// Fester Code, wenn die Schranke einen Aufruf verhindert hat.
pub const CODE_ABGEBROCHEN: &str = "abgebrochen";

impl<T: HubTransport> PortalHub<T> {
    /// `geraet_id` ist die Kennung dieses Geräts (klein geschrieben, `[A-Za-z0-9._-]{1,64}`); der
    /// Transport trägt das Token, das der Hub an dieses Gerät gebunden hat.
    pub fn new(transport: T, geraet_id: &str) -> Self {
        Self {
            transport,
            geraet_id: geraet_id.to_owned(),
            letzter_code: Mutex::new(None),
            sperre: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Hängt die gemeinsame Abbruchschranke an. Sie wird vor JEDEM Netzaufruf geprüft.
    pub fn mit_sperre(mut self, sperre: Arc<AtomicBool>) -> Self {
        self.sperre = sperre;
        self
    }

    /// Fehlercode (`error.code`, fester Kurztext) der letzten Ablehnung durch den Hub oder
    /// `netz`, wenn er nicht zu erreichen war. Für Statusanzeige; enthält nie Inhalte.
    pub fn letzter_code(&self) -> Option<String> {
        self.letzter_code
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    fn merken(&self, code: &str) {
        *self.letzter_code.lock().unwrap_or_else(|e| e.into_inner()) = Some(code.to_owned());
    }

    fn vergessen(&self) {
        *self.letzter_code.lock().unwrap_or_else(|e| e.into_inner()) = None;
    }

    fn pruefen_auth(&self, auth: &AuthGeraet) -> Result<(), HubFehler> {
        if auth.id() == self.geraet_id {
            Ok(())
        } else {
            self.merken("geraet_stimmt_nicht_ueberein");
            Err(HubFehler::GeraetStimmtNichtUeberein)
        }
    }

    /// Schickt die Anfrage und liefert den Körper einer erfolgreichen (2xx) Antwort.
    fn aufruf(&self, anfrage: Anfrage) -> Result<Value, HubFehler> {
        if self.sperre.load(Ordering::SeqCst) {
            self.merken(CODE_ABGEBROCHEN);
            return Err(HubFehler::Nichtverfuegbar);
        }
        let antwort = match self.transport.senden(&anfrage) {
            Ok(a) => a,
            Err(TransportFehler) => {
                self.merken("netz");
                return Err(HubFehler::Nichtverfuegbar);
            }
        };
        if (200..300).contains(&antwort.status) {
            self.vergessen();
            return Ok(antwort.body);
        }
        let (fehler, code) = abbilden(antwort.status, &antwort.body);
        log::warn!(
            "Assistent-Sync: Hub lehnt ab (HTTP {}, {code})",
            antwort.status
        );
        self.merken(&code);
        Err(fehler)
    }

    /// Lässt dieses Gerät am Hub zu (`POST /geraet`): als Erstgerät, per Kopplung oder mit
    /// `freigabe_code` einer Freigabe eines zugelassenen Geräts. Gibt die aktuelle Epoche zurück.
    pub fn zulassen(&self, freigabe_code: Option<&str>) -> Result<u64, HubFehler> {
        let mut body = json!({ "geraet_id": self.geraet_id });
        if let Some(code) = freigabe_code {
            body["freigabe_code"] = Value::String(code.to_owned());
        }
        let antwort = self.aufruf(Anfrage {
            methode: Methode::Post,
            pfad: "/geraet",
            query: Vec::new(),
            body: Some(body),
        })?;
        zaehler(antwort.get("epoche")).ok_or(HubFehler::Vertrag("Hub-Antwort ungueltig"))
    }

    /// Gibt eine Kennung für ein weiteres Gerät frei (`POST /freigeben`). Nur durch ein
    /// zugelassenes Gerät; der Code steht nur in dieser Antwort.
    pub fn freigeben(&self, neues_geraet: &str) -> Result<Freigabe, HubFehler> {
        let antwort = self.aufruf(Anfrage {
            methode: Methode::Post,
            pfad: "/freigeben",
            query: Vec::new(),
            body: Some(json!({ "geraet_id": neues_geraet })),
        })?;
        let text = |feld: &str| antwort.get(feld).and_then(Value::as_str).map(str::to_owned);
        match (text("code"), text("gueltig_bis")) {
            (Some(code), Some(gueltig_bis)) if !code.is_empty() => {
                Ok(Freigabe { code, gueltig_bis })
            }
            _ => Err(HubFehler::Vertrag("Hub-Antwort ungueltig")),
        }
    }

    fn umschlag_pruefen(&self, u: &Umschlag) -> Result<(), HubFehler> {
        if u.geraet_id != self.geraet_id {
            self.merken("geraet_stimmt_nicht_ueberein");
            return Err(HubFehler::GeraetStimmtNichtUeberein);
        }
        if u.ciphertext.len() > MAX_CIPHERTEXT {
            self.merken("ciphertext_zu_gross");
            return Err(HubFehler::Vertrag("ciphertext zu gross fuer den Hub"));
        }
        Ok(())
    }
}

/// Zähler aus einer Antwort: kanonischer Dezimalstring (Int64String); eine JSON-Zahl wird
/// toleriert, weil nur der Hub antwortet.
fn zaehler(wert: Option<&Value>) -> Option<u64> {
    match wert? {
        Value::String(s) => s.parse().ok(),
        Value::Number(n) => n.as_u64(),
        _ => None,
    }
}

fn als_text(n: u64) -> String {
    n.to_string()
}

/// Bildet eine Ablehnung des Hubs (HTTP-Status und Körper) auf [`HubFehler`] ab und liefert den
/// Fehlercode für die Anzeige. Zuerst entscheidet `error.code`, dann der Status. Unbekanntes
/// und alles ab 500 gilt als vorübergehend (`Nichtverfuegbar`), unbekannte 4xx als Vertragsverletzung.
pub fn abbilden(status: u16, body: &Value) -> (HubFehler, String) {
    let fehler = body.get("error");
    let code = fehler
        .and_then(|e| e.get("code"))
        .and_then(Value::as_str)
        .filter(|c| c.len() <= 64 && c.bytes().all(|b| b.is_ascii_lowercase() || b == b'_'))
        .map(str::to_owned);
    let feld = |name: &str| fehler.and_then(|e| e.get(name));
    let aktuell = zaehler(feld("aktuell"));
    let f = match (code.as_deref(), status) {
        (Some("revision_konflikt"), _) => HubFehler::RevisionKonflikt { aktuell },
        (Some("epoche_veraltet"), _) => match zaehler(feld("mindest")) {
            Some(mindest) => HubFehler::EpocheVeraltet { mindest },
            None => HubFehler::Vertrag("Hub-Antwort ungueltig"),
        },
        (Some("epoche_voraus"), _) => match aktuell {
            Some(aktuell) => HubFehler::EpocheVoraus { aktuell },
            None => HubFehler::Vertrag("Hub-Antwort ungueltig"),
        },
        (Some("epoche_konflikt"), _) => match aktuell {
            Some(aktuell) => HubFehler::EpocheKonflikt { aktuell },
            None => HubFehler::Vertrag("Hub-Antwort ungueltig"),
        },
        (Some("nicht_zugelassen"), _) => HubFehler::NichtZugelassen,
        (Some("widerrufen"), _) => HubFehler::Widerrufen,
        (Some("geraet_stimmt_nicht_ueberein"), _) => HubFehler::GeraetStimmtNichtUeberein,
        (Some("grabstein"), _) => HubFehler::GrabsteinAbgewiesen,
        (Some("nicht_veraenderlich"), _) => HubFehler::NichtVeraenderlich,
        (Some("vertrag"), _) => HubFehler::Vertrag(vertrag_grund(
            feld("grund").and_then(Value::as_str).unwrap_or(""),
        )),
        (Some("kontingent"), _) => HubFehler::Vertrag("Kontingent des Kontos ausgeschoepft"),
        (Some("geraet_vorhanden"), _) => HubFehler::Vertrag("es gibt noch ein zugelassenes Geraet"),
        (Some("payload_too_large"), _) | (None, 413) => {
            HubFehler::Vertrag("Objekt zu gross fuer den Hub")
        }
        // Scope `voice.sync` fehlt (oder nur über `*`): dieses Token darf nicht an den Hub.
        (Some("forbidden"), _) => HubFehler::NichtZugelassen,
        (Some("hub_busy" | "rate_limited" | "unauthorized"), _) => HubFehler::Nichtverfuegbar,
        (_, 401 | 408 | 429 | 500..=599) => HubFehler::Nichtverfuegbar,
        _ => HubFehler::Vertrag("unerwartete Antwort des Hubs"),
    };
    let anzeige = code.unwrap_or_else(|| match status {
        401 => "unauthorized".to_owned(),
        s => format!("http_{s}"),
    });
    (f, anzeige)
}

/// Feste Kurztexte für `grund` einer `422 vertrag` (nie Inhalt weiterreichen).
fn vertrag_grund(grund: &str) -> &'static str {
    match grund {
        "eigenes_geraet_im_widerruf" => "eigenes Geraet im Widerruf",
        "letztes_geraet" => "letztes Geraet kann nicht widerrufen werden",
        "loeschmarke_zu_gross" => "Loeschmarke zu gross",
        _ => "vom Hub abgewiesen",
    }
}

fn umschlag_aus(wert: &Value) -> Result<Umschlag, HubFehler> {
    serde_json::from_value(wert.clone()).map_err(|_| HubFehler::Vertrag("Hub-Antwort ungueltig"))
}

impl<T: HubTransport> HubSpeicher for PortalHub<T> {
    fn schreiben(&self, auth: &AuthGeraet, umschlag: &Umschlag) -> Result<Schreiben, HubFehler> {
        self.pruefen_auth(auth)?;
        self.umschlag_pruefen(umschlag)?;
        let body = serde_json::to_value(umschlag).map_err(|_| HubFehler::Vertrag("Umschlag"))?;
        let antwort = self.aufruf(Anfrage {
            methode: Methode::Post,
            pfad: "/schreiben",
            query: Vec::new(),
            body: Some(json!({ "umschlag": body })),
        })?;
        let revision = zaehler(antwort.get("revision"));
        match (antwort.get("ergebnis").and_then(Value::as_str), revision) {
            (Some("neu"), Some(r)) => Ok(Schreiben::Neu(r)),
            (Some("vorhanden"), Some(r)) => Ok(Schreiben::Vorhanden(r)),
            _ => Err(HubFehler::Vertrag("Hub-Antwort ungueltig")),
        }
    }

    fn cas(
        &self,
        auth: &AuthGeraet,
        sammlung: Sammlung,
        objekt_id: &Hex128,
        erwartete_revision: Option<u64>,
        neuer_umschlag: &Umschlag,
    ) -> Result<u64, HubFehler> {
        self.pruefen_auth(auth)?;
        self.umschlag_pruefen(neuer_umschlag)?;
        // Der Hub nimmt Sammlung und Objekt-ID aus dem Umschlag; ein Widerspruch dazwischen wird hier
        // abgefangen (der Kern übergibt beides doppelt).
        if neuer_umschlag.sammlung != sammlung || &neuer_umschlag.objekt_id != objekt_id {
            return Err(HubFehler::Vertrag(
                "sammlung oder objekt_id passen nicht zum Umschlag",
            ));
        }
        let body =
            serde_json::to_value(neuer_umschlag).map_err(|_| HubFehler::Vertrag("Umschlag"))?;
        let antwort = self.aufruf(Anfrage {
            methode: Methode::Post,
            pfad: "/cas",
            query: Vec::new(),
            body: Some(json!({
                "erwartete_revision": erwartete_revision.map(als_text),
                "umschlag": body,
            })),
        })?;
        zaehler(antwort.get("revision")).ok_or(HubFehler::Vertrag("Hub-Antwort ungueltig"))
    }

    fn holen(
        &self,
        auth: &AuthGeraet,
        sammlung: Sammlung,
        objekt_id: &Hex128,
    ) -> Result<Option<Umschlag>, HubFehler> {
        self.pruefen_auth(auth)?;
        let antwort = self.aufruf(Anfrage {
            methode: Methode::Get,
            pfad: "/objekt",
            query: vec![
                ("sammlung", sammlung.name().to_owned()),
                ("objekt_id", objekt_id.as_str().to_owned()),
            ],
            body: None,
        })?;
        match antwort.get("objekt") {
            Some(Value::Null) => Ok(None),
            Some(u) => umschlag_aus(u).map(Some),
            None => Err(HubFehler::Vertrag("Hub-Antwort ungueltig")),
        }
    }

    fn lesen(&self, auth: &AuthGeraet, filter: &LeseFilter) -> Result<Vec<Umschlag>, HubFehler> {
        self.pruefen_auth(auth)?;
        let mut alle: Vec<Umschlag> = Vec::new();
        let mut nach = filter.nach_revision;
        for _ in 0..MAX_SEITEN {
            let mut query = vec![
                ("nach_revision", als_text(nach)),
                ("limit", LESE_SEITE.to_string()),
            ];
            if let Some(r) = &filter.unterhaltung_ref {
                query.push(("unterhaltung_ref", r.as_str().to_owned()));
            }
            if let Some(s) = filter.sammlung {
                query.push(("sammlung", s.name().to_owned()));
            }
            let antwort = self.aufruf(Anfrage {
                methode: Methode::Get,
                pfad: "/lesen",
                query,
                body: None,
            })?;
            let objekte = antwort
                .get("objekte")
                .and_then(Value::as_array)
                .ok_or(HubFehler::Vertrag("Hub-Antwort ungueltig"))?;
            let mehr = antwort
                .get("mehr")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let cursor = zaehler(antwort.get("cursor"));
            for o in objekte {
                let u = umschlag_aus(o)?;
                // Aufsteigend und nur neuer als der Stand davor: sonst ist die Antwort unbrauchbar.
                let rev = u
                    .revision
                    .ok_or(HubFehler::Vertrag("Hub-Antwort ungueltig"))?;
                if rev <= nach {
                    return Err(HubFehler::Vertrag("Hub-Antwort ungueltig"));
                }
                nach = rev;
                alle.push(u);
            }
            if !mehr {
                return Ok(alle);
            }
            // Mit `mehr` muss der Cursor vorwärts gehen, sonst liest der Hub dieselbe Seite endlos.
            match cursor {
                Some(c) if c >= nach && !objekte.is_empty() => nach = c,
                _ => return Err(HubFehler::Vertrag("Hub-Antwort ungueltig")),
            }
        }
        Err(HubFehler::Vertrag("Hub liefert kein Ende"))
    }

    fn widerrufen(
        &self,
        auth: &AuthGeraet,
        widerrufenes_geraet: &str,
        erwartete_epoche: u64,
        vorgang: &Hex128,
    ) -> Result<u64, HubFehler> {
        self.pruefen_auth(auth)?;
        let antwort = self.aufruf(Anfrage {
            methode: Methode::Post,
            pfad: "/widerruf",
            query: Vec::new(),
            body: Some(json!({
                "widerrufenes_geraet": widerrufenes_geraet,
                "erwartete_epoche": als_text(erwartete_epoche),
                "vorgang": vorgang.as_str(),
            })),
        })?;
        zaehler(antwort.get("epoche")).ok_or(HubFehler::Vertrag("Hub-Antwort ungueltig"))
    }

    fn epoche(&self, auth: &AuthGeraet) -> Result<u64, HubFehler> {
        self.pruefen_auth(auth)?;
        let antwort = self.aufruf(Anfrage {
            methode: Methode::Get,
            pfad: "/epoche",
            query: Vec::new(),
            body: None,
        })?;
        zaehler(antwort.get("epoche")).ok_or(HubFehler::Vertrag("Hub-Antwort ungueltig"))
    }

    fn zugelassene(&self, auth: &AuthGeraet) -> Result<Vec<String>, HubFehler> {
        self.pruefen_auth(auth)?;
        let antwort = self.aufruf(Anfrage {
            methode: Methode::Get,
            pfad: "/geraete",
            query: Vec::new(),
            body: None,
        })?;
        let liste = antwort
            .get("geraete")
            .and_then(Value::as_array)
            .ok_or(HubFehler::Vertrag("Hub-Antwort ungueltig"))?;
        liste
            .iter()
            .map(|g| {
                g.as_str()
                    .map(str::to_owned)
                    .ok_or(HubFehler::Vertrag("Hub-Antwort ungueltig"))
            })
            .collect()
    }
}

#[cfg(test)]
#[path = "portal_hub_tests.rs"]
pub(crate) mod tests;
