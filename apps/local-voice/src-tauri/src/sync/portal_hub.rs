//! `PortalHub`: der Assistent-Sync-Hub im wai-portal als [`HubSpeicher`] (lva#388, ADR-0028 E3, E3.8).
//!
//! Spricht die Endpunkte unter `/api/v1/assistent/sync/` (Doku: wai-portal `docs/ASSISTENT-SYNC.md`)
//! und bildet deren Fehlercodes auf [`HubFehler`] des Sync-Kerns ab. Der Hub sieht nur Steuerfelder,
//! Chiffrat, Nonce, das signierte Geräteverzeichnis und gewrappte Schlüsselpakete; dieser Baustein
//! kennt keinen Schlüssel und vertraut dem Hub nicht: Er reicht nur durch, was der Kern prüft.
//!
//! Der Transport steckt hinter [`HubTransport`]: In der App ist es der Hub-Client des
//! Bibliotheks-Syncs (selbes Token, selber HTTP-Weg einschließlich curl unter Windows, siehe
//! `assistent_sync::ClientTransport`), in Tests ein Hub im Arbeitsspeicher oder der echte PHP-Hub.
//!
//! Regeln:
//! - Zähler (`nach_revision`, `erwartete_*`, `cursor`, `key_epoch`, `ab`, `nummer`, `epoche`) gehen als
//!   Int64String (`"17"`) über die Leitung, nie als JSON-Zahl.
//! - **Transportgrenzen (E3.8, Must-fix 7):** Umschläge, Verzeichniseinträge und Schlüsselpakete werden
//!   nur über `Umschlag::liste_aus_json`/`aus_json`, `Verzeichniseintrag::aus_json` und
//!   `Schluesselpaket::aus_json` gelesen; die Byte- und Anzahlgrenzen gelten dort.
//! - **Größengrenzen:** Ein Antwortkörper hat höchstens [`MAX_ANTWORT`] Bytes (der Transport liest
//!   begrenzt und bricht sonst ab), Listen gehen als Rohtext ohne Umweg über einen JSON-Baum an den
//!   Kern, und ein Leseaufruf blättert höchstens [`MAX_GESAMT_BYTES`] bzw. [`MAX_GESAMT_OBJEKTE`].
//! - Der authentifizierte Absender ist das Gerät des Tokens. Eine fremde [`AuthGeraet`] oder ein
//!   fremdes `geraet_id` im Umschlag wird schon hier abgewiesen, ohne Netzaufruf.
//! - Kein Inhalt im Log: Es wird höchstens der Fehlercode des Hubs (fester Kurztext) und der
//!   HTTP-Status protokolliert, nie Umschläge, Chiffrat, Kennungen oder Geräte-IDs.

use std::collections::HashMap;
use std::fmt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use serde_json::value::RawValue;
use serde_json::{json, Value};
use wai_assistent::sync::{
    AuthGeraet, Hex128, HubFehler, HubSpeicher, LeseFilter, Sammlung, Schluesselpaket, Schreiben,
    Umschlag, Verzeichniseintrag,
};

/// Höchstlänge des Chiffrats (Base64-Zeichen), die der Hub annimmt (Hoster-Grenze, Hub-Doku).
pub const MAX_CIPHERTEXT: usize = 4_300_000;

/// Seitengröße beim Lesen von Objekten und Fassungen (Höchstwert des Hubs).
const LESE_SEITE: usize = 100;

/// Höchstens so viele Pakete je `POST /pakete` (Hub-Grenze).
const PAKETE_JE_ANFRAGE: usize = 100;

/// Höchstgröße eines Antwortkörpers in Bytes. Eine Seite endet beim Hub bei etwa 4 MiB Chiffrat
/// (mindestens ein Objekt bis 4,3 MB); alles darüber ist kein ehrlicher Hub. Der Transport liest
/// höchstens so viel (begrenzt gestreamt) und meldet sonst [`TransportFehler::ZuGross`].
pub const MAX_ANTWORT: usize = 16 * 1024 * 1024;

/// Gesamtgrenzen je Leseaufruf (`lesen`, `fassungen_lesen`): danach endet das Blättern mit dem bis
/// dahin Gelesenen (aufsteigend). Der Kern setzt seinen Cursor auf die höchste gelieferte Revision
/// und liest den Rest beim nächsten Lauf; ein Hub, der nie aufhört, bindet so keinen Speicher.
const MAX_SEITEN: usize = 1_000;
const MAX_GESAMT_BYTES: usize = 64 * 1024 * 1024;
const MAX_GESAMT_OBJEKTE: usize = 20_000;

/// Höchstzahl der Verzeichniseinträge und Pakete je Antwort (Kontingent des Hubs: 1000 bzw. 10 000).
const MAX_VERZEICHNIS: usize = 10_000;
const MAX_PAKETE: usize = 100_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Methode {
    Get,
    Post,
}

/// Eine Anfrage an den Hub. `pfad` steht hinter `/api/v1/assistent/sync`, z. B. `/schreiben`.
#[derive(Debug, Clone)]
pub struct Anfrage {
    pub methode: Methode,
    pub pfad: &'static str,
    pub query: Vec<(&'static str, String)>,
    pub body: Option<Value>,
}

/// HTTP-Status und roher Körper (höchstens [`MAX_ANTWORT`] Bytes). Listen gehen unverändert als
/// Rohbytes an die `aus_json`-Funktionen des Kerns.
#[derive(Debug, Clone)]
pub struct Antwort {
    pub status: u16,
    pub roh: Vec<u8>,
}

impl Antwort {
    /// Antwort mit diesem JSON-Körper (Tests).
    #[cfg(test)]
    pub fn json(status: u16, wert: &Value) -> Self {
        Self {
            status,
            roh: serde_json::to_vec(wert).unwrap_or_default(),
        }
    }

    /// Körper als JSON-Wert; kein JSON ergibt `Null`.
    pub fn wert(&self) -> Value {
        serde_json::from_slice(&self.roh).unwrap_or(Value::Null)
    }
}

/// Der Transport hat keine verwertbare Antwort. Ohne Text, damit nie etwas Fremdes in Meldungen oder
/// Log gelangt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransportFehler {
    /// Nicht erreichbar (Netz, Zeitüberschreitung): später erneut versuchen.
    Netz,
    /// Der Körper ist größer als [`MAX_ANTWORT`]; das Lesen wurde abgebrochen.
    ZuGross,
}

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

const UNGUELTIG: HubFehler = HubFehler::Vertrag("Hub-Antwort ungueltig");
const ZU_GROSS: HubFehler = HubFehler::Vertrag("Hub-Antwort zu gross");

impl<T: HubTransport> PortalHub<T> {
    /// `geraet_id` ist die Kennung dieses Geräts; der Transport trägt das Token, das der Hub an dieses
    /// Gerät gebunden hat.
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

    /// Unbrauchbare Antwort des Hubs: Code merken, Vertragsfehler.
    fn ungueltig(&self) -> HubFehler {
        self.merken("antwort_ungueltig");
        UNGUELTIG
    }

    /// Schickt die Anfrage und liefert den rohen Körper einer erfolgreichen (2xx) Antwort.
    fn aufruf_roh(&self, anfrage: Anfrage) -> Result<Vec<u8>, HubFehler> {
        if self.sperre.load(Ordering::SeqCst) {
            self.merken(CODE_ABGEBROCHEN);
            return Err(HubFehler::Nichtverfuegbar);
        }
        let antwort = match self.transport.senden(&anfrage) {
            Ok(a) if a.roh.len() <= MAX_ANTWORT => a,
            Ok(_) | Err(TransportFehler::ZuGross) => {
                self.merken("antwort_zu_gross");
                return Err(ZU_GROSS);
            }
            Err(TransportFehler::Netz) => {
                self.merken("netz");
                return Err(HubFehler::Nichtverfuegbar);
            }
        };
        if (200..300).contains(&antwort.status) {
            self.vergessen();
            return Ok(antwort.roh);
        }
        let (fehler, code) = abbilden(antwort.status, &antwort.wert());
        log::warn!(
            "Assistent-Sync: Hub lehnt ab (HTTP {}, {code})",
            antwort.status
        );
        self.merken(&code);
        Err(fehler)
    }

    /// Wie [`Self::aufruf_roh`] für kleine Antworten mit einzelnen Feldern (kein JSON: `Null`).
    fn aufruf(&self, anfrage: Anfrage) -> Result<Value, HubFehler> {
        let roh = self.aufruf_roh(anfrage)?;
        Ok(serde_json::from_slice(&roh).unwrap_or(Value::Null))
    }

    /// Oberste Felder einer Antwort, unverändert als Rohtext (nichts wird umgeformt).
    fn felder<'a>(&self, roh: &'a [u8]) -> Result<HashMap<String, &'a RawValue>, HubFehler> {
        serde_json::from_slice(roh).map_err(|_| self.ungueltig())
    }

    /// Liste `feld` als Elemente im Rohtext; mehr als `max` Elemente sind ein Vertragsfehler.
    fn elemente<'a>(
        &self,
        roh: &'a [u8],
        feld: &str,
        max: usize,
    ) -> Result<Vec<&'a RawValue>, HubFehler> {
        let felder = self.felder(roh)?;
        let liste = felder.get(feld).ok_or_else(|| self.ungueltig())?;
        let v: Vec<&'a RawValue> =
            serde_json::from_str(liste.get()).map_err(|_| self.ungueltig())?;
        if v.len() > max {
            return Err(self.ungueltig());
        }
        Ok(v)
    }

    /// Lässt dieses Gerät am Hub zu (`POST /geraet`): als Erstgerät, per Kopplung oder mit
    /// `freigabe_code` einer Freigabe eines zugelassenen Geräts. Das ist nur die Transportzulassung;
    /// die kryptografische Aufnahme läuft über das Geräteverzeichnis. Gibt die Epoche des Hubs zurück
    /// (nur zur Anzeige, der Kern leitet die Epoche aus dem geprüften Verzeichnis ab).
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
        zaehler(antwort.get("epoche")).ok_or_else(|| self.ungueltig())
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
            _ => Err(self.ungueltig()),
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

    /// Blättert über `/lesen` bzw. `/fassungen`. Die Seite geht als Rohtext an
    /// `Umschlag::liste_aus_json`. Weiter geht es nur nach der höchsten gelieferten Revision (der
    /// `cursor` des Hubs wird nicht gebraucht) und nur mit Fortschritt. Eine Anomalie (Revision fehlt
    /// oder steigt nicht) wird nicht hier entschieden, sondern unverändert an den Kern gereicht; er
    /// liest dann vollständig neu (Selbstheilung, E3.8). Gesamtgrenzen: siehe [`MAX_GESAMT_BYTES`].
    fn blaettern(
        &self,
        pfad: &'static str,
        feld: &str,
        nach_revision: u64,
        query: &[(&'static str, String)],
    ) -> Result<Vec<Umschlag>, HubFehler> {
        let mut alle: Vec<Umschlag> = Vec::new();
        let mut bytes = 0usize;
        let mut nach = nach_revision;
        for _ in 0..MAX_SEITEN {
            let mut q = vec![
                ("nach_revision", als_text(nach)),
                ("limit", LESE_SEITE.to_string()),
            ];
            q.extend(query.iter().cloned());
            let roh = self.aufruf_roh(Anfrage {
                methode: Methode::Get,
                pfad,
                query: q,
                body: None,
            })?;
            bytes = bytes.saturating_add(roh.len());
            let felder = self.felder(&roh)?;
            let liste = felder.get(feld).ok_or_else(|| self.ungueltig())?;
            let seite = Umschlag::liste_aus_json(liste.get().as_bytes(), LESE_SEITE)
                .map_err(|_| self.ungueltig())?;
            let mehr = felder.get("mehr").is_some_and(|m| m.get() == "true");
            let anomal = seite.iter().any(|u| u.revision.is_none_or(|r| r <= nach));
            let hoechste = seite.iter().filter_map(|u| u.revision).max();
            alle.extend(seite);
            if !mehr || anomal || bytes >= MAX_GESAMT_BYTES || alle.len() >= MAX_GESAMT_OBJEKTE {
                return Ok(alle);
            }
            match hoechste {
                Some(h) if h > nach => nach = h,
                // `mehr` ohne Fortschritt: sonst liest der Hub dieselbe Seite endlos.
                _ => return Ok(alle),
            }
        }
        Ok(alle)
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
            None => UNGUELTIG,
        },
        (Some("epoche_voraus"), _) => match aktuell {
            Some(aktuell) => HubFehler::EpocheVoraus { aktuell },
            None => UNGUELTIG,
        },
        (Some("verzeichnis_konflikt"), _) => match zaehler(feld("laenge")) {
            Some(laenge) => HubFehler::VerzeichnisKonflikt { laenge },
            None => UNGUELTIG,
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
        "verzeichniseintrag" => "Verzeichniseintrag abgewiesen",
        _ => "vom Hub abgewiesen",
    }
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
            _ => Err(self.ungueltig()),
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
        zaehler(antwort.get("revision")).ok_or_else(|| self.ungueltig())
    }

    fn holen(
        &self,
        auth: &AuthGeraet,
        sammlung: Sammlung,
        objekt_id: &Hex128,
    ) -> Result<Option<Umschlag>, HubFehler> {
        self.pruefen_auth(auth)?;
        let roh = self.aufruf_roh(Anfrage {
            methode: Methode::Get,
            pfad: "/objekt",
            query: vec![
                ("sammlung", sammlung.name().to_owned()),
                ("objekt_id", objekt_id.as_str().to_owned()),
            ],
            body: None,
        })?;
        let felder = self.felder(&roh)?;
        match felder.get("objekt") {
            Some(u) if u.get() == "null" => Ok(None),
            Some(u) => Umschlag::aus_json(u.get().as_bytes())
                .map(Some)
                .map_err(|_| self.ungueltig()),
            None => Err(self.ungueltig()),
        }
    }

    fn fassungen_lesen(
        &self,
        auth: &AuthGeraet,
        sammlung: Sammlung,
        objekt_id: &Hex128,
    ) -> Result<Vec<Umschlag>, HubFehler> {
        self.pruefen_auth(auth)?;
        self.blaettern(
            "/fassungen",
            "fassungen",
            0,
            &[
                ("sammlung", sammlung.name().to_owned()),
                ("objekt_id", objekt_id.as_str().to_owned()),
            ],
        )
    }

    fn lesen(&self, auth: &AuthGeraet, filter: &LeseFilter) -> Result<Vec<Umschlag>, HubFehler> {
        self.pruefen_auth(auth)?;
        let mut query = Vec::new();
        if let Some(r) = &filter.unterhaltung_ref {
            query.push(("unterhaltung_ref", r.as_str().to_owned()));
        }
        if let Some(s) = filter.sammlung {
            query.push(("sammlung", s.name().to_owned()));
        }
        self.blaettern("/lesen", "objekte", filter.nach_revision, &query)
    }

    fn verzeichnis_anhaengen(
        &self,
        auth: &AuthGeraet,
        eintrag: &Verzeichniseintrag,
    ) -> Result<(), HubFehler> {
        self.pruefen_auth(auth)?;
        let body =
            serde_json::to_value(eintrag).map_err(|_| HubFehler::Vertrag("Verzeichniseintrag"))?;
        let antwort = self.aufruf(Anfrage {
            methode: Methode::Post,
            pfad: "/verzeichnis",
            query: Vec::new(),
            body: Some(json!({ "eintrag": body })),
        })?;
        // Die Länge danach ist nur ein Lebenszeichen; der Kern glaubt ihr nicht.
        zaehler(antwort.get("laenge"))
            .map(|_| ())
            .ok_or_else(|| self.ungueltig())
    }

    fn verzeichnis_lesen(
        &self,
        auth: &AuthGeraet,
        ab: u64,
    ) -> Result<Vec<Verzeichniseintrag>, HubFehler> {
        self.pruefen_auth(auth)?;
        let roh = self.aufruf_roh(Anfrage {
            methode: Methode::Get,
            pfad: "/verzeichnis",
            query: vec![("ab", als_text(ab))],
            body: None,
        })?;
        self.elemente(&roh, "eintraege", MAX_VERZEICHNIS)?
            .iter()
            .map(|e| Verzeichniseintrag::aus_json(e.get().as_bytes()).map_err(|_| self.ungueltig()))
            .collect()
    }

    fn pakete_ablegen(
        &self,
        auth: &AuthGeraet,
        pakete: &[Schluesselpaket],
    ) -> Result<(), HubFehler> {
        self.pruefen_auth(auth)?;
        if pakete.iter().any(|p| p.absender != self.geraet_id) {
            self.merken("geraet_stimmt_nicht_ueberein");
            return Err(HubFehler::GeraetStimmtNichtUeberein);
        }
        // Je Anfrage höchstens 100; jeder Stapel ist am Hub atomar und idempotent.
        for stapel in pakete.chunks(PAKETE_JE_ANFRAGE) {
            let body =
                serde_json::to_value(stapel).map_err(|_| HubFehler::Vertrag("Schluesselpaket"))?;
            let antwort = self.aufruf(Anfrage {
                methode: Methode::Post,
                pfad: "/pakete",
                query: Vec::new(),
                body: Some(json!({ "pakete": body })),
            })?;
            if zaehler(antwort.get("neu")).is_none() {
                return Err(self.ungueltig());
            }
        }
        Ok(())
    }

    fn pakete_lesen(&self, auth: &AuthGeraet) -> Result<Vec<Schluesselpaket>, HubFehler> {
        self.pruefen_auth(auth)?;
        let roh = self.aufruf_roh(Anfrage {
            methode: Methode::Get,
            pfad: "/pakete",
            query: Vec::new(),
            body: None,
        })?;
        self.elemente(&roh, "pakete", MAX_PAKETE)?
            .iter()
            .map(|p| Schluesselpaket::aus_json(p.get().as_bytes()).map_err(|_| self.ungueltig()))
            .collect()
    }
}

#[cfg(test)]
#[path = "portal_hub_tests.rs"]
pub(crate) mod tests;

#[cfg(test)]
#[path = "portal_hub_interop.rs"]
mod interop;
