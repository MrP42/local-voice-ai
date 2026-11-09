//! Tests des `PortalHub` (lva#388, ADR-0028 E3.8): Fehlerabbildung gegen die Doku des Hubs, Drahtformat,
//! Transportgrenzen und die Abnahmefälle (E3.0 Bindung der Steuerfelder, E3.3 (a) bis (c) Lease, Widerruf
//! über das Verzeichnis, Kopplung mit Schlüsselpaketen, manipulierte Antworten eines bösartigen Hubs)
//! gegen den Referenz-Hub `MemHub` und gegen einen Fake-HTTP-Hub, der die Endpunkte der Hub-Doku nachbildet.
//! Alle Daten sind synthetisch.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Barrier, Mutex};
use std::thread;

use serde_json::{json, Value};
use wai_assistent::sync::{
    Abgleich, AuthGeraet, Beitrag, Einschraenkung, Hex128, HubFehler, HubSpeicher, Kopf, Kopplung,
    Kopplungsangebot, Kopplungsantwort, LeaseFehler, LeseFilter, MemHub, MemSpeicher, Modus,
    Sammlung, Schreiben, SyncFehler, Testkonto, Umschlag, ZweigGrund,
};

use super::*;

const T0: u64 = 1_794_081_600;

// ---------------------------------------------------------------------------------------------
// Fake-HTTP-Hub: bildet die Endpunkte der Hub-Doku auf den MemHub ab (Drahtformat und Fehlercodes).
// ---------------------------------------------------------------------------------------------

/// Konto am Fake-Hub: Zulassungsregeln von `POST /geraet` (Erstgerät oder Freigabe mit Code).
#[derive(Default)]
pub(crate) struct Konto {
    je_zugelassen: bool,
    freigaben: BTreeMap<String, String>,
}

/// Verändert den Körper einer erfolgreichen Antwort (Pfad, Körper): ein Angreifer mit Hubzugang.
type Manipulation = Box<dyn Fn(&str, &mut Value) + Send + Sync>;

pub(crate) struct FakeHttp {
    hub: Arc<MemHub>,
    konto: Arc<Mutex<Konto>>,
    /// Gerät, an das das Token gebunden ist.
    geraet: String,
    pub(crate) log: Mutex<Vec<Anfrage>>,
    manipulation: Option<Manipulation>,
    /// Netz weg.
    offline: AtomicBool,
}

impl FakeHttp {
    pub(crate) fn neu(hub: &Arc<MemHub>, konto: &Arc<Mutex<Konto>>, geraet: &str) -> Self {
        Self {
            hub: hub.clone(),
            konto: konto.clone(),
            geraet: geraet.to_owned(),
            log: Mutex::new(Vec::new()),
            manipulation: None,
            offline: AtomicBool::new(false),
        }
    }

    fn auth(&self) -> AuthGeraet {
        AuthGeraet::new(&self.geraet)
    }
}

/// Der Hub antwortet mit `{"error": {...}}` und diesem Status (Tabelle „Fehler“ der Hub-Doku).
fn fehler_antwort(f: &HubFehler) -> Antwort {
    let (status, code, zusatz): (u16, &str, Value) = match f {
        HubFehler::RevisionKonflikt { aktuell } => (
            409,
            "revision_konflikt",
            json!({ "aktuell": aktuell.map(|a| a.to_string()) }),
        ),
        HubFehler::EpocheVeraltet { mindest } => (
            409,
            "epoche_veraltet",
            json!({ "mindest": mindest.to_string() }),
        ),
        HubFehler::EpocheVoraus { aktuell } => (
            409,
            "epoche_voraus",
            json!({ "aktuell": aktuell.to_string() }),
        ),
        HubFehler::VerzeichnisKonflikt { laenge } => (
            409,
            "verzeichnis_konflikt",
            json!({ "laenge": laenge.to_string() }),
        ),
        HubFehler::NichtZugelassen => (403, "nicht_zugelassen", json!({})),
        HubFehler::Widerrufen => (403, "widerrufen", json!({})),
        HubFehler::GeraetStimmtNichtUeberein => (403, "geraet_stimmt_nicht_ueberein", json!({})),
        HubFehler::GrabsteinAbgewiesen => (410, "grabstein", json!({})),
        HubFehler::NichtVeraenderlich => (422, "nicht_veraenderlich", json!({})),
        HubFehler::Vertrag("verzeichniseintrag") => {
            (422, "vertrag", json!({ "grund": "verzeichniseintrag" }))
        }
        HubFehler::Vertrag(_) => (422, "vertrag", json!({ "grund": "schema" })),
        HubFehler::Nichtverfuegbar => (503, "hub_busy", json!({})),
    };
    let mut e = json!({ "code": code, "message": "x" });
    if let (Some(e), Some(z)) = (e.as_object_mut(), zusatz.as_object()) {
        e.extend(z.clone());
    }
    Antwort {
        status,
        body: json!({ "error": e }),
    }
}

fn ok(body: Value) -> Antwort {
    Antwort { status: 200, body }
}

fn q<'a>(a: &'a Anfrage, name: &str) -> Option<&'a str> {
    a.query
        .iter()
        .find(|(k, _)| *k == name)
        .map(|(_, v)| v.as_str())
}

/// Zähler im Körper müssen Strings sein (Int64String); JSON-Zahlen weist der Hub mit 422 ab.
fn zaehler_string(v: &Value) -> Result<u64, Antwort> {
    v.as_str()
        .filter(|s| *s == "0" || (!s.starts_with('0') && s.bytes().all(|b| b.is_ascii_digit())))
        .and_then(|s| s.parse().ok())
        .ok_or_else(|| fehler_antwort(&HubFehler::Vertrag("zaehler")))
}

impl HubTransport for FakeHttp {
    fn senden(&self, a: &Anfrage) -> Result<Antwort, TransportFehler> {
        self.log.lock().unwrap().push(a.clone());
        if self.offline.load(Ordering::SeqCst) {
            return Err(TransportFehler);
        }
        let mut antwort = self.verarbeiten(a).unwrap_or_else(|e| e);
        if let (Some(m), 200) = (&self.manipulation, antwort.status) {
            m(a.pfad, &mut antwort.body);
        }
        Ok(antwort)
    }
}

/// Eine Seite aufsteigend nach Revision (wie `/lesen` und `/fassungen`).
fn seite(alle: &[Umschlag], nach: u64, limit: usize) -> (Vec<Value>, bool, u64) {
    let rest: Vec<&Umschlag> = alle
        .iter()
        .filter(|u| u.revision.is_some_and(|r| r > nach))
        .collect();
    let teil: Vec<&Umschlag> = rest.iter().take(limit.min(100)).copied().collect();
    let mehr = rest.len() > teil.len();
    let cursor = teil.last().and_then(|u| u.revision).unwrap_or(nach);
    let werte = teil
        .iter()
        .map(|u| serde_json::to_value(u).unwrap())
        .collect();
    (werte, mehr, cursor)
}

impl FakeHttp {
    fn verarbeiten(&self, a: &Anfrage) -> Result<Antwort, Antwort> {
        let auth = self.auth();
        let fehler = |f: HubFehler| fehler_antwort(&f);
        let body = a.body.clone().unwrap_or(Value::Null);
        let umschlag = |v: &Value| -> Result<Umschlag, Antwort> {
            serde_json::from_value(v.clone()).map_err(|_| fehler_antwort(&HubFehler::Vertrag("u")))
        };
        let sammlung_id = || -> Result<(Sammlung, Hex128), Antwort> {
            let s: Sammlung = serde_json::from_value(json!(q(a, "sammlung")))
                .map_err(|_| fehler_antwort(&HubFehler::Vertrag("sammlung")))?;
            let id = Hex128::parse(q(a, "objekt_id").unwrap_or_default())
                .map_err(|_| fehler_antwort(&HubFehler::Vertrag("objekt_id")))?;
            Ok((s, id))
        };
        let limit: usize = q(a, "limit").and_then(|l| l.parse().ok()).unwrap_or(100);
        match (a.methode, a.pfad) {
            (Methode::Post, "/geraet") => {
                let id = body["geraet_id"].as_str().unwrap_or_default().to_owned();
                if id != self.geraet {
                    return Err(fehler(HubFehler::GeraetStimmtNichtUeberein));
                }
                // Ein widerrufenes Gerät kommt nie wieder hinein.
                if let Err(HubFehler::Widerrufen) = self.hub.verzeichnis_lesen(&auth, 0) {
                    return Err(fehler(HubFehler::Widerrufen));
                }
                let mut k = self.konto.lock().unwrap();
                let code = body.get("freigabe_code").and_then(Value::as_str);
                let erlaubt = !k.je_zugelassen
                    || matches!((code, k.freigaben.get(&id)), (Some(c), Some(e)) if c == e);
                if !erlaubt {
                    return Err(fehler(HubFehler::NichtZugelassen));
                }
                k.je_zugelassen = true;
                k.freigaben.remove(&id);
                drop(k);
                self.hub.zulassen(&id);
                Ok(ok(
                    json!({ "geraet_id": id, "epoche": self.hub.epoche().to_string() }),
                ))
            }
            (Methode::Post, "/freigeben") => {
                self.hub.verzeichnis_lesen(&auth, 0).map_err(fehler)?;
                let id = body["geraet_id"].as_str().unwrap_or_default().to_owned();
                let code = format!("code-{id}-einmal");
                self.konto
                    .lock()
                    .unwrap()
                    .freigaben
                    .insert(id.clone(), code.clone());
                Ok(ok(json!({
                    "geraet_id": id,
                    "code": code,
                    "gueltig_bis": "2026-11-09T12:10:00Z"
                })))
            }
            (Methode::Post, "/schreiben") => {
                let u = umschlag(&body["umschlag"])?;
                Ok(ok(match self.hub.schreiben(&auth, &u).map_err(fehler)? {
                    Schreiben::Neu(r) => json!({ "ergebnis": "neu", "revision": r.to_string() }),
                    Schreiben::Vorhanden(r) => {
                        json!({ "ergebnis": "vorhanden", "revision": r.to_string() })
                    }
                }))
            }
            (Methode::Post, "/cas") => {
                let erwartet = match body.get("erwartete_revision") {
                    Some(Value::Null) => None,
                    Some(v) => Some(zaehler_string(v)?),
                    None => return Err(fehler(HubFehler::Vertrag("erwartete_revision fehlt"))),
                };
                let u = umschlag(&body["umschlag"])?;
                let r = self
                    .hub
                    .cas(&auth, u.sammlung, &u.objekt_id, erwartet, &u)
                    .map_err(fehler)?;
                Ok(ok(json!({ "revision": r.to_string() })))
            }
            (Methode::Get, "/objekt") => {
                let (s, id) = sammlung_id()?;
                let o = self.hub.holen(&auth, s, &id).map_err(fehler)?;
                Ok(ok(json!({ "objekt": o })))
            }
            (Methode::Get, "/fassungen") => {
                let (s, id) = sammlung_id()?;
                let nach = zaehler_string(&json!(q(a, "nach_revision").unwrap_or("0")))?;
                let alle = self.hub.fassungen_lesen(&auth, s, &id).map_err(fehler)?;
                let (fassungen, mehr, _) = seite(&alle, nach, limit);
                Ok(ok(json!({ "fassungen": fassungen, "mehr": mehr })))
            }
            (Methode::Get, "/lesen") => {
                let nach = zaehler_string(&json!(q(a, "nach_revision").unwrap_or("0")))?;
                let filter = LeseFilter {
                    nach_revision: nach,
                    unterhaltung_ref: q(a, "unterhaltung_ref").and_then(|r| Hex128::parse(r).ok()),
                    sammlung: q(a, "sammlung").and_then(|s| serde_json::from_value(json!(s)).ok()),
                };
                let alle = self.hub.lesen(&auth, &filter).map_err(fehler)?;
                let (objekte, mehr, cursor) = seite(&alle, nach, limit);
                Ok(ok(json!({
                    "objekte": objekte,
                    "cursor": cursor.to_string(),
                    "mehr": mehr
                })))
            }
            (Methode::Post, "/verzeichnis") => {
                let e = serde_json::from_value(body["eintrag"].clone())
                    .map_err(|_| fehler(HubFehler::Vertrag("verzeichniseintrag")))?;
                self.hub.verzeichnis_anhaengen(&auth, &e).map_err(fehler)?;
                Ok(ok(
                    json!({ "laenge": self.hub.verzeichnis().len().to_string() }),
                ))
            }
            (Methode::Get, "/verzeichnis") => {
                let ab = zaehler_string(&json!(q(a, "ab").unwrap_or("0")))?;
                let e = self.hub.verzeichnis_lesen(&auth, ab).map_err(fehler)?;
                Ok(ok(json!({ "eintraege": e })))
            }
            (Methode::Post, "/pakete") => {
                let p: Vec<wai_assistent::sync::Schluesselpaket> =
                    serde_json::from_value(body["pakete"].clone())
                        .map_err(|_| fehler(HubFehler::Vertrag("paket")))?;
                let vorher = self.hub.pakete().len();
                self.hub.pakete_ablegen(&auth, &p).map_err(fehler)?;
                // Die Doku gibt `neu` als JSON-Zahl zurück.
                Ok(ok(json!({ "neu": self.hub.pakete().len() - vorher })))
            }
            (Methode::Get, "/pakete") => {
                let p = self.hub.pakete_lesen(&auth).map_err(fehler)?;
                Ok(ok(json!({ "pakete": p })))
            }
            _ => Err(Antwort {
                status: 404,
                body: Value::Null,
            }),
        }
    }
}

pub(crate) type Fake = PortalHub<FakeHttp>;

/// Schaltet das Netz des Fake-Hubs ab (`false`) oder an (`true`).
pub(crate) fn netz(h: &Fake, an: bool) {
    h.transport.offline.store(!an, Ordering::SeqCst);
}

pub(crate) fn konto_hub() -> (Arc<MemHub>, Arc<Mutex<Konto>>) {
    (
        Arc::new(MemHub::new()),
        Arc::new(Mutex::new(Konto::default())),
    )
}

/// Gerät am Fake-Hub, zugelassen als Erstgerät beziehungsweise per Freigabe.
pub(crate) fn portal(hub: &Arc<MemHub>, konto: &Arc<Mutex<Konto>>, name: &str) -> Fake {
    let h = PortalHub::new(FakeHttp::neu(hub, konto, name), name);
    if konto.lock().unwrap().je_zugelassen {
        // Weitere Geräte brauchen eine Freigabe eines zugelassenen Geräts: der Test lässt sie
        // direkt zu, wo es nicht um die Zulassung selbst geht.
        hub.zulassen(name);
    } else {
        h.zulassen(None).unwrap();
    }
    h
}

fn beitrag(id: &str, eltern: Option<&str>, generation: u64, text: &str) -> Beitrag {
    Beitrag::neu(id, eltern, generation, "user", Modus::Typed, text)
}

fn kopf(titel: &str) -> Kopf {
    Kopf {
        titel: titel.into(),
        bereich: "Privat".into(),
        modellwunsch: "standard".into(),
        angeheftet: false,
    }
}

/// Kopie eines Angebots wie nach dem Lesen des QR-Codes (das Angebot ist absichtlich nicht `Clone`).
fn qr(a: &Abgleich) -> (Kopplungsangebot, Kopplungsangebot) {
    let angebot = a.kopplung_anbieten().unwrap();
    let kopie = Kopplungsangebot::aus_json(&serde_json::to_vec(&angebot).unwrap()).unwrap();
    (angebot, kopie)
}

// ---------------------------------------------------------------------------------------------
// Fehlerabbildung (Tabelle „Fehler“ der Hub-Doku)
// ---------------------------------------------------------------------------------------------

fn fehlerkoerper(code: &str, zusatz: Value) -> Value {
    let mut e = json!({ "code": code, "message": "Text" });
    e.as_object_mut()
        .unwrap()
        .extend(zusatz.as_object().unwrap().clone());
    json!({ "error": e })
}

#[test]
fn fehlercodes_der_doku_werden_auf_hubfehler_abgebildet() {
    use HubFehler::*;
    let faelle: Vec<(u16, &str, Value, HubFehler)> = vec![
        (
            409,
            "revision_konflikt",
            json!({ "aktuell": "7" }),
            RevisionKonflikt { aktuell: Some(7) },
        ),
        (
            409,
            "revision_konflikt",
            json!({ "aktuell": null }),
            RevisionKonflikt { aktuell: None },
        ),
        (
            409,
            "epoche_veraltet",
            json!({ "mindest": "3" }),
            EpocheVeraltet { mindest: 3 },
        ),
        (
            409,
            "epoche_voraus",
            json!({ "aktuell": "2" }),
            EpocheVoraus { aktuell: 2 },
        ),
        (
            409,
            "verzeichnis_konflikt",
            json!({ "laenge": "4" }),
            VerzeichnisKonflikt { laenge: 4 },
        ),
        (403, "nicht_zugelassen", json!({}), NichtZugelassen),
        (403, "widerrufen", json!({}), Widerrufen),
        (
            403,
            "geraet_stimmt_nicht_ueberein",
            json!({}),
            GeraetStimmtNichtUeberein,
        ),
        (410, "grabstein", json!({}), GrabsteinAbgewiesen),
        (422, "nicht_veraenderlich", json!({}), NichtVeraenderlich),
        (
            422,
            "vertrag",
            json!({ "grund": "letztes_geraet" }),
            Vertrag("letztes Geraet kann nicht widerrufen werden"),
        ),
        (
            422,
            "vertrag",
            json!({ "grund": "eigenes_geraet_im_widerruf" }),
            Vertrag("eigenes Geraet im Widerruf"),
        ),
        (
            422,
            "vertrag",
            json!({ "grund": "loeschmarke_zu_gross" }),
            Vertrag("Loeschmarke zu gross"),
        ),
        (
            422,
            "vertrag",
            json!({ "grund": "verzeichniseintrag", "detail": "kette" }),
            Vertrag("Verzeichniseintrag abgewiesen"),
        ),
        (
            422,
            "vertrag",
            json!({ "grund": "irgendwas mit Inhalt" }),
            Vertrag("vom Hub abgewiesen"),
        ),
        (
            507,
            "kontingent",
            json!({}),
            Vertrag("Kontingent des Kontos ausgeschoepft"),
        ),
        (
            409,
            "geraet_vorhanden",
            json!({}),
            Vertrag("es gibt noch ein zugelassenes Geraet"),
        ),
        (503, "hub_busy", json!({}), Nichtverfuegbar),
        (429, "rate_limited", json!({}), Nichtverfuegbar),
        (401, "unauthorized", json!({}), Nichtverfuegbar),
        (403, "forbidden", json!({}), NichtZugelassen),
        (
            413,
            "payload_too_large",
            json!({}),
            Vertrag("Objekt zu gross fuer den Hub"),
        ),
    ];
    for (status, code, zusatz, erwartet) in faelle {
        let (f, anzeige) = abbilden(status, &fehlerkoerper(code, zusatz));
        assert_eq!(f, erwartet, "{status} {code}");
        assert_eq!(anzeige, code);
    }
}

#[test]
fn unbekanntes_und_serverfehler_ohne_app_code() {
    // Hoster-500 oder PHP-Abbruch: kein JSON der App (Körper Null).
    assert_eq!(
        abbilden(500, &Value::Null),
        (HubFehler::Nichtverfuegbar, "http_500".into())
    );
    assert_eq!(
        abbilden(502, &json!("<html>")).0,
        HubFehler::Nichtverfuegbar
    );
    assert_eq!(
        abbilden(401, &Value::Null),
        (HubFehler::Nichtverfuegbar, "unauthorized".into())
    );
    assert_eq!(
        abbilden(413, &Value::Null).0,
        HubFehler::Vertrag("Objekt zu gross fuer den Hub")
    );
    // Unbekannter Code mit 4xx: Vertragsverletzung, mit 5xx: später wieder versuchen.
    assert_eq!(
        abbilden(418, &fehlerkoerper("teekanne", json!({}))).0,
        HubFehler::Vertrag("unerwartete Antwort des Hubs")
    );
    assert_eq!(
        abbilden(503, &fehlerkoerper("teekanne", json!({}))).0,
        HubFehler::Nichtverfuegbar
    );
    // Der alte Code `epoche_konflikt` gibt es nicht mehr: unbekannt, also Vertragsverletzung.
    assert_eq!(
        abbilden(
            409,
            &fehlerkoerper("epoche_konflikt", json!({ "aktuell": "4" }))
        )
        .0,
        HubFehler::Vertrag("unerwartete Antwort des Hubs")
    );
    // Fehlende Pflichtzusatzfelder: nie raten.
    for code in ["verzeichnis_konflikt", "epoche_voraus", "epoche_veraltet"] {
        assert_eq!(
            abbilden(409, &fehlerkoerper(code, json!({}))).0,
            HubFehler::Vertrag("Hub-Antwort ungueltig"),
            "{code}"
        );
    }
    // Ein Code mit Fremdzeichen wird nicht angezeigt oder weitergereicht.
    let (_, anzeige) = abbilden(422, &fehlerkoerper("Ü<script>", json!({})));
    assert_eq!(anzeige, "http_422");
}

/// Transport, der eine feste Antwort liefert.
struct Fest(Result<Antwort, TransportFehler>);

impl HubTransport for Fest {
    fn senden(&self, _: &Anfrage) -> Result<Antwort, TransportFehler> {
        self.0.clone()
    }
}

fn fest(status: u16, body: Value) -> PortalHub<Fest> {
    PortalHub::new(Fest(Ok(Antwort { status, body })), "dev-pc")
}

#[test]
fn fehler_kommen_ueber_alle_methoden_an_und_der_code_wird_gemerkt() {
    let auth = AuthGeraet::new("dev-pc");
    let id = Hex128::zufall().unwrap();
    let h = fest(403, fehlerkoerper("widerrufen", json!({})));
    assert_eq!(h.verzeichnis_lesen(&auth, 0), Err(HubFehler::Widerrufen));
    assert_eq!(h.pakete_lesen(&auth), Err(HubFehler::Widerrufen));
    assert_eq!(
        h.fassungen_lesen(&auth, Sammlung::Unterhaltung, &id),
        Err(HubFehler::Widerrufen)
    );
    assert_eq!(
        h.holen(&auth, Sammlung::Sitzung, &id),
        Err(HubFehler::Widerrufen)
    );
    assert_eq!(
        h.lesen(&auth, &LeseFilter::default()),
        Err(HubFehler::Widerrufen)
    );
    assert_eq!(h.letzter_code().as_deref(), Some("widerrufen"));
    assert_eq!(h.zulassen(None), Err(HubFehler::Widerrufen));

    let k = fest(
        409,
        fehlerkoerper("verzeichnis_konflikt", json!({ "laenge": "3" })),
    );
    let konto = Testkonto::neu(&["dev-pc"]);
    let e = konto.verzeichnis().eintraege()[0].clone();
    assert_eq!(
        k.verzeichnis_anhaengen(&auth, &e),
        Err(HubFehler::VerzeichnisKonflikt { laenge: 3 })
    );
    assert_eq!(k.letzter_code().as_deref(), Some("verzeichnis_konflikt"));

    let offline = PortalHub::new(Fest(Err(TransportFehler)), "dev-pc");
    assert_eq!(
        offline.verzeichnis_lesen(&auth, 0),
        Err(HubFehler::Nichtverfuegbar)
    );
    assert_eq!(offline.letzter_code().as_deref(), Some("netz"));
}

#[test]
fn erfolg_loescht_den_gemerkten_fehlercode() {
    let (hub, konto) = konto_hub();
    let p = portal(&hub, &konto, "dev-pc");
    let auth = AuthGeraet::new("dev-pc");
    assert!(p.verzeichnis_lesen(&auth, 0).unwrap().is_empty());
    assert_eq!(p.letzter_code(), None);
}

#[test]
fn fremde_identitaet_und_fremdes_geraet_im_umschlag_gehen_nie_ans_netz() {
    let (hub, konto) = konto_hub();
    let p = portal(&hub, &konto, "dev-pc");
    let vorher = p.transport.log.lock().unwrap().len();
    assert_eq!(
        p.verzeichnis_lesen(&AuthGeraet::new("dev-anders"), 0),
        Err(HubFehler::GeraetStimmtNichtUeberein)
    );

    let fremd_konto = Testkonto::neu(&["dev-anders"]);
    let mut fremd = fremd_konto.geraet("dev-anders");
    let r = fremd
        .unterhaltung_anlegen(kopf("T"), Einschraenkung::default())
        .unwrap();
    fremd
        .beitrag_anlegen(&r, beitrag("a", None, 1, "x"))
        .unwrap();
    // Ein Abgleich mit fremder Kennung über den Hub dieses Geräts: abgewiesen, kein Schreibversuch.
    assert!(fremd.hochladen(&p).is_err());
    assert_eq!(p.transport.log.lock().unwrap().len(), vorher);
    assert_eq!(hub.schreibversuche(), 0);
}

#[test]
fn fremder_absender_im_schluesselpaket_geht_nie_ans_netz() {
    let hub = Arc::new(MemHub::new());
    let k = Testkonto::mit_hub(&["dev-pc", "dev-iphone", "dev-tablet"], &hub);
    let mut pc = k.geraet("dev-pc");
    pc.widerrufen(&*hub, "dev-tablet").unwrap();
    let pakete = hub.pakete();
    assert!(!pakete.is_empty());
    let konto = Arc::new(Mutex::new(Konto {
        je_zugelassen: true,
        ..Konto::default()
    }));
    let ip = PortalHub::new(FakeHttp::neu(&hub, &konto, "dev-iphone"), "dev-iphone");
    assert_eq!(
        ip.pakete_ablegen(&AuthGeraet::new("dev-iphone"), &pakete),
        Err(HubFehler::GeraetStimmtNichtUeberein)
    );
    assert!(ip.transport.log.lock().unwrap().is_empty());
}

#[test]
fn zu_grosses_chiffrat_wird_vor_dem_netz_abgewiesen() {
    let (hub, konto) = konto_hub();
    let k = Testkonto::mit_hub(&["dev-pc"], &hub);
    let p = portal(&hub, &konto, "dev-pc");
    let mut pc = k.geraet("dev-pc");
    let r = pc
        .unterhaltung_anlegen(kopf("T"), Einschraenkung::default())
        .unwrap();
    pc.beitrag_anlegen(&r, beitrag("a", None, 1, &"x".repeat(3_300_000)))
        .unwrap();
    let f = pc.hochladen(&p).unwrap_err();
    assert!(matches!(f, SyncFehler::Hub(HubFehler::Vertrag(_))));
    assert!(
        p.transport
            .log
            .lock()
            .unwrap()
            .iter()
            .all(|a| a.pfad != "/schreiben" && a.pfad != "/cas"),
        "kein Schreibaufruf"
    );
    assert_eq!(p.letzter_code().as_deref(), Some("ciphertext_zu_gross"));
}

// ---------------------------------------------------------------------------------------------
// Drahtformat und Transportgrenzen
// ---------------------------------------------------------------------------------------------

/// Alle Zähler gehen als String, nie als JSON-Zahl, und jede Anfrage trägt nur die
/// dokumentierten Felder.
#[test]
fn zaehler_gehen_als_int64string_ueber_die_leitung() {
    let (hub, konto) = konto_hub();
    let k = Testkonto::mit_hub(&["dev-pc", "dev-zweit", "dev-drei"], &hub);
    let p = portal(&hub, &konto, "dev-pc");
    let mut pc = k.geraet("dev-pc");
    let r = pc
        .unterhaltung_anlegen(kopf("T"), Einschraenkung::default())
        .unwrap();
    pc.beitrag_anlegen(&r, beitrag("a", None, 1, "Text"))
        .unwrap();
    pc.hochladen(&p).unwrap();
    pc.kopf_aendern(&r, kopf("T2")).unwrap();
    pc.hochladen(&p).unwrap(); // cas mit erwarteter Revision
    pc.herunterladen(&p).unwrap();
    assert_eq!(pc.widerrufen(&p, "dev-zweit"), Ok(2));

    let log = p.transport.log.lock().unwrap().clone();
    fn zahl(w: &Value) -> bool {
        match w {
            Value::Number(_) => true,
            Value::Array(a) => a.iter().any(zahl),
            Value::Object(o) => o.values().any(zahl),
            _ => false,
        }
    }
    let cas: Vec<_> = log.iter().filter(|a| a.pfad == "/cas").collect();
    assert!(!cas.is_empty());
    assert!(log
        .iter()
        .any(|a| a.pfad == "/verzeichnis" && a.methode == Methode::Post));
    assert!(log
        .iter()
        .any(|a| a.pfad == "/pakete" && a.methode == Methode::Post));
    for a in &log {
        if let Some(b) = &a.body {
            assert!(!zahl(b), "{}: JSON-Zahl im Körper", a.pfad);
        }
    }
    let kopf_cas = cas.last().unwrap().body.as_ref().unwrap();
    assert!(kopf_cas["erwartete_revision"].is_string());
    assert!(
        cas[0].body.as_ref().unwrap()["erwartete_revision"].is_null(),
        "Erstschreiben: null"
    );
    assert!(log
        .iter()
        .filter(|a| a.pfad == "/lesen" || a.pfad == "/fassungen")
        .all(|a| q(a, "nach_revision").is_some()));
    assert!(log
        .iter()
        .filter(|a| a.pfad == "/verzeichnis" && a.methode == Methode::Get)
        .all(|a| q(a, "ab").is_some()));
    let umschlag = &cas[0].body.as_ref().unwrap()["umschlag"];
    assert!(umschlag["key_epoch"].is_string());
    assert!(
        umschlag.get("revision").is_none(),
        "Revision vergibt der Hub"
    );
    let felder: Vec<&str> = umschlag
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    for f in [
        "schema_version",
        "sammlung",
        "objekt_id",
        "unterhaltung_ref",
        "art",
        "geraet_id",
        "key_epoch",
        "ciphertext",
        "nonce",
    ] {
        assert!(felder.contains(&f), "{f} fehlt");
    }
}

#[test]
fn lesen_blaettert_ueber_alle_seiten() {
    let (hub, konto) = konto_hub();
    let k = Testkonto::mit_hub(&["dev-pc"], &hub);
    let p = portal(&hub, &konto, "dev-pc");
    let mut pc = k.geraet("dev-pc");
    let r = pc
        .unterhaltung_anlegen(kopf("T"), Einschraenkung::default())
        .unwrap();
    let mut eltern: Option<String> = None;
    for i in 0..250 {
        let id = format!("b{i:04}");
        pc.beitrag_anlegen(&r, beitrag(&id, eltern.as_deref(), 1, "t"))
            .unwrap();
        eltern = Some(id);
    }
    pc.hochladen(&p).unwrap();
    let vorher = p
        .transport
        .log
        .lock()
        .unwrap()
        .iter()
        .filter(|a| a.pfad == "/lesen")
        .count();
    let alle = p
        .lesen(&AuthGeraet::new("dev-pc"), &LeseFilter::default())
        .unwrap();
    assert_eq!(alle.len(), 251, "250 Beiträge und der Kopf");
    let revisionen: Vec<u64> = alle.iter().map(|u| u.revision.unwrap()).collect();
    assert!(revisionen.windows(2).all(|w| w[0] < w[1]));
    let seiten = p
        .transport
        .log
        .lock()
        .unwrap()
        .iter()
        .filter(|a| a.pfad == "/lesen")
        .count();
    assert_eq!(seiten - vorher, 3);
    // Filter gehen mit.
    let nur_kopf = p
        .lesen(
            &AuthGeraet::new("dev-pc"),
            &LeseFilter {
                nach_revision: 0,
                unterhaltung_ref: Some(r.clone()),
                sammlung: Some(Sammlung::Unterhaltung),
            },
        )
        .unwrap();
    assert_eq!(nur_kopf.len(), 1);
}

#[test]
fn fassungen_kommen_vollstaendig_und_aufsteigend() {
    let (hub, konto) = konto_hub();
    let k = Testkonto::mit_hub(&["dev-pc"], &hub);
    let p = portal(&hub, &konto, "dev-pc");
    let mut pc = k.geraet("dev-pc");
    let r = pc
        .unterhaltung_anlegen(kopf("T0"), Einschraenkung::default())
        .unwrap();
    pc.hochladen(&p).unwrap();
    for i in 1..=3 {
        pc.kopf_aendern(&r, kopf(&format!("T{i}"))).unwrap();
        pc.hochladen(&p).unwrap();
    }
    let kopf_id = hub
        .alle_umschlaege()
        .into_iter()
        .find(|u| u.sammlung == Sammlung::Unterhaltung)
        .unwrap()
        .objekt_id;
    let f = p
        .fassungen_lesen(&AuthGeraet::new("dev-pc"), Sammlung::Unterhaltung, &kopf_id)
        .unwrap();
    assert_eq!(f.len(), 4, "jede Fassung bleibt beim Hub");
    assert!(f
        .windows(2)
        .all(|w| w[0].revision.unwrap() < w[1].revision.unwrap()));
    let anfrage = p
        .transport
        .log
        .lock()
        .unwrap()
        .iter()
        .rev()
        .find(|a| a.pfad == "/fassungen")
        .cloned()
        .unwrap();
    assert_eq!(q(&anfrage, "sammlung"), Some("unterhaltung"));
    assert_eq!(q(&anfrage, "objekt_id"), Some(kopf_id.as_str()));
}

#[test]
fn lesen_traut_einem_kaputten_hub_nicht() {
    let auth = AuthGeraet::new("dev-pc");
    let vertrag = |r: Result<Vec<Umschlag>, HubFehler>| matches!(r, Err(HubFehler::Vertrag(_)));
    // `mehr` ohne Fortschritt: sonst endlose Schleife.
    let h = fest(200, json!({ "objekte": [], "cursor": "0", "mehr": true }));
    assert!(vertrag(h.lesen(&auth, &LeseFilter::default())));
    // Unbrauchbare Objekte.
    let h = fest(
        200,
        json!({ "objekte": [{ "x": 1 }], "cursor": "1", "mehr": false }),
    );
    assert!(vertrag(h.lesen(&auth, &LeseFilter::default())));

    // Echte Umschläge in falscher Reihenfolge oder doppelt: abgewiesen.
    let hub = Arc::new(MemHub::new());
    let k = Testkonto::mit_hub(&["dev-pc"], &hub);
    let mut pc = k.geraet("dev-pc");
    let r = pc
        .unterhaltung_anlegen(kopf("T"), Einschraenkung::default())
        .unwrap();
    pc.beitrag_anlegen(&r, beitrag("a", None, 1, "t")).unwrap();
    pc.hochladen(&*hub).unwrap();
    let u: Vec<Value> = hub
        .alle_umschlaege()
        .iter()
        .map(|u| serde_json::to_value(u).unwrap())
        .collect();
    assert_eq!(u.len(), 2);
    let mut u = u;
    u.sort_by_key(|o| o["revision"].as_str().unwrap().parse::<u64>().unwrap());
    let rueckwaerts = json!([u[1].clone(), u[0].clone()]);
    let h = fest(
        200,
        json!({ "objekte": rueckwaerts, "cursor": "2", "mehr": false }),
    );
    assert!(vertrag(h.lesen(&auth, &LeseFilter::default())));
    let doppelt = json!([u[0].clone(), u[0].clone()]);
    let h = fest(200, json!({ "fassungen": doppelt, "mehr": false }));
    assert!(vertrag(h.fassungen_lesen(
        &auth,
        Sammlung::Unterhaltung,
        &Hex128::zufall().unwrap()
    )));
    // Cursor hinter die letzte Revision zurück.
    let h = fest(
        200,
        json!({ "objekte": [u[1].clone()], "cursor": "0", "mehr": true }),
    );
    assert!(vertrag(h.lesen(&auth, &LeseFilter::default())));

    // Fehlende oder unbrauchbare Felder in Verzeichnis und Paketen.
    assert!(matches!(
        fest(200, json!({})).verzeichnis_lesen(&auth, 0),
        Err(HubFehler::Vertrag(_))
    ));
    assert!(matches!(
        fest(200, json!({ "eintraege": [1] })).verzeichnis_lesen(&auth, 0),
        Err(HubFehler::Vertrag(_))
    ));
    assert!(matches!(
        fest(200, json!({ "pakete": [{ "x": 1 }] })).pakete_lesen(&auth),
        Err(HubFehler::Vertrag(_))
    ));
    let e = k.verzeichnis().eintraege()[0].clone();
    assert!(matches!(
        fest(200, json!({})).verzeichnis_anhaengen(&auth, &e),
        Err(HubFehler::Vertrag(_))
    ));
    assert_eq!(
        fest(200, json!({ "laenge": "1" })).verzeichnis_anhaengen(&auth, &e),
        Ok(())
    );
}

/// Transportgrenzen (Must-fix 7): unbekannte Felder und übergroße Elemente aus der Hub-Antwort werden
/// vor dem Kern abgewiesen.
#[test]
fn transportgrenzen_weisen_fremde_felder_und_uebergroesse_ab() {
    let auth = AuthGeraet::new("dev-pc");
    let hub = Arc::new(MemHub::new());
    let k = Testkonto::mit_hub(&["dev-pc", "dev-iphone", "dev-tablet"], &hub);
    let mut pc = k.geraet("dev-pc");
    pc.widerrufen(&*hub, "dev-tablet").unwrap();
    let eintrag = serde_json::to_value(&hub.verzeichnis()[0]).unwrap();
    let paket = serde_json::to_value(&hub.pakete()[0]).unwrap();

    let mut zusatz = eintrag.clone();
    zusatz["fremd"] = json!("x");
    assert!(matches!(
        fest(200, json!({ "eintraege": [zusatz] })).verzeichnis_lesen(&auth, 0),
        Err(HubFehler::Vertrag(_))
    ));
    let mut gross = eintrag.clone();
    gross["unterzeichner"] = json!("d".repeat(5000));
    assert!(matches!(
        fest(200, json!({ "eintraege": [gross] })).verzeichnis_lesen(&auth, 0),
        Err(HubFehler::Vertrag(_))
    ));
    let mut p_gross = paket.clone();
    p_gross["chiffrat"] = json!("A".repeat(4000));
    assert!(matches!(
        fest(200, json!({ "pakete": [p_gross] })).pakete_lesen(&auth),
        Err(HubFehler::Vertrag(_))
    ));
    // Unverändert geht es durch.
    assert_eq!(
        fest(200, json!({ "eintraege": [eintrag] }))
            .verzeichnis_lesen(&auth, 0)
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        fest(200, json!({ "pakete": [paket] }))
            .pakete_lesen(&auth)
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn rundlauf_ueber_zwei_geraete_per_http_ohne_verlust_und_dopplung() {
    let (hub, konto) = konto_hub();
    let k = Testkonto::mit_hub(&["dev-pc", "dev-iphone"], &hub);
    let pc_hub = portal(&hub, &konto, "dev-pc");
    let ip_hub = portal(&hub, &konto, "dev-iphone");
    let mut pc = k.geraet("dev-pc");
    let r = pc
        .unterhaltung_anlegen(kopf("Rundlauf"), Einschraenkung::default())
        .unwrap();
    let mut eltern: Option<String> = None;
    for i in 0..20 {
        let id = format!("b{i:02}");
        pc.beitrag_anlegen(&r, beitrag(&id, eltern.as_deref(), 0, &format!("Text {i}")))
            .unwrap();
        eltern = Some(id);
    }
    let b1 = pc.hochladen(&pc_hub).unwrap();
    assert_eq!((b1.beitraege, b1.koepfe), (20, 1));
    // Wiederholung: nichts doppelt am Hub.
    let b2 = pc.hochladen(&pc_hub).unwrap();
    assert_eq!((b2.beitraege, b2.koepfe, b2.bereits_beim_hub), (0, 0, 0));
    let mut ip = k.geraet("dev-iphone");
    let down = ip.herunterladen(&ip_hub).unwrap();
    assert_eq!(down.beitraege_neu, 20);
    assert_eq!(ip.unterhaltung(&r).unwrap().kopf().titel, "Rundlauf");
    assert_eq!(hub.anzahl(Sammlung::Beitrag), 20);
    assert_eq!(ip.unterhaltung(&r).unwrap().verlauf().len(), 20);
}

#[test]
fn zulassung_als_erstgeraet_und_per_freigabe_mit_code() {
    let (hub, konto) = konto_hub();
    let pc = PortalHub::new(FakeHttp::neu(&hub, &konto, "dev-pc"), "dev-pc");
    assert_eq!(pc.zulassen(None), Ok(1), "Erstgerät");
    let tablet = PortalHub::new(FakeHttp::neu(&hub, &konto, "dev-tablet"), "dev-tablet");
    assert_eq!(
        tablet.zulassen(None),
        Err(HubFehler::NichtZugelassen),
        "ohne Freigabe nicht"
    );
    assert_eq!(tablet.letzter_code().as_deref(), Some("nicht_zugelassen"));
    let freigabe = pc.freigeben("dev-tablet").unwrap();
    assert!(!freigabe.code.is_empty());
    assert!(
        !format!("{freigabe:?}").contains(&freigabe.code),
        "Debug zeigt den Code nicht"
    );
    assert_eq!(
        tablet.zulassen(Some("falsch")),
        Err(HubFehler::NichtZugelassen)
    );
    assert_eq!(tablet.zulassen(Some(&freigabe.code)), Ok(1));
    // Danach darf das Tablet lesen.
    assert!(tablet
        .verzeichnis_lesen(&AuthGeraet::new("dev-tablet"), 0)
        .is_ok());
    let body = tablet
        .transport
        .log
        .lock()
        .unwrap()
        .iter()
        .rev()
        .find(|a| a.pfad == "/geraet")
        .and_then(|a| a.body.clone())
        .unwrap();
    assert_eq!(body["freigabe_code"], json!(freigabe.code));
    let erst = pc.transport.log.lock().unwrap()[0].body.clone().unwrap();
    assert!(erst.get("freigabe_code").is_none(), "ohne Code kein Feld");
}

#[test]
fn gruenden_und_kopplung_mit_schluesselpaket_ueber_http() {
    let (hub, konto) = konto_hub();
    // Erstgerät gründet das Konto: der Anker geht mit dem ersten Hochladen an den Hub.
    let pc_hub = portal(&hub, &konto, "dev-pc");
    let mut pc = Abgleich::gruenden("dev-pc", Box::new(MemSpeicher::new())).unwrap();
    pc.freigabe_einschalten().unwrap();
    let r = pc
        .unterhaltung_anlegen(kopf("Gegründet"), Einschraenkung::default())
        .unwrap();
    pc.beitrag_anlegen(&r, beitrag("a", None, 1, "vorher"))
        .unwrap();
    pc.hochladen(&pc_hub).unwrap();
    assert_eq!(hub.verzeichnis().len(), 1, "Anker beim Hub");

    // Neues Gerät: Transportzulassung per Freigabe, kryptografische Aufnahme per QR und Rückkanal.
    let ip_hub = PortalHub::new(FakeHttp::neu(&hub, &konto, "dev-iphone"), "dev-iphone");
    let freigabe = pc_hub.freigeben("dev-iphone").unwrap();
    ip_hub.zulassen(Some(&freigabe.code)).unwrap();
    let (angebot, kopie) = qr(&pc);
    let kopplung = Kopplung::neu("dev-iphone", kopie).unwrap();
    assert!(
        kopplung
            .abschliessen(&ip_hub, Box::new(MemSpeicher::new()))
            .unwrap()
            .is_none(),
        "vor der Aufnahme kein Abschluss"
    );
    let antwort = antwort_ueber_kanal(&kopplung);
    pc.geraet_aufnehmen(&pc_hub, &angebot, &antwort).unwrap();
    let mut ip = kopplung
        .abschliessen(&ip_hub, Box::new(MemSpeicher::new()))
        .unwrap()
        .expect("aufgenommen");
    assert_eq!(ip.konto(), pc.konto());
    let b = ip.herunterladen(&ip_hub).unwrap();
    assert_eq!(b.beitraege_neu, 1);
    assert_eq!(ip.unterhaltung(&r).unwrap().kopf().titel, "Gegründet");
    // Ein Paket lag beim Hub; der Hub hat den Schlüssel nie im Klartext gesehen.
    assert!(hub.pakete().iter().any(|p| p.empfaenger == "dev-iphone"));
    let draht: String = pc_hub
        .transport
        .log
        .lock()
        .unwrap()
        .iter()
        .filter_map(|a| a.body.as_ref().map(Value::to_string))
        .collect();
    assert!(!draht.contains("vorher"));
}

/// Die Antwort geht über einen beliebigen Kanal (auch den Hub): als JSON hin und zurück.
fn antwort_ueber_kanal(k: &Kopplung) -> Kopplungsantwort {
    let a = k.antwort().unwrap();
    Kopplungsantwort::aus_json(&serde_json::to_vec(&a).unwrap()).unwrap()
}

#[test]
fn widerruf_ueber_das_verzeichnis_per_http() {
    let (hub, konto) = konto_hub();
    let k = Testkonto::mit_hub(&["dev-pc", "dev-tablet", "dev-iphone"], &hub);
    let pc_hub = portal(&hub, &konto, "dev-pc");
    let ip_hub = portal(&hub, &konto, "dev-iphone");
    let tablet_hub = portal(&hub, &konto, "dev-tablet");
    let mut pc = k.geraet("dev-pc");
    let mut pc_empf = pc.empfaenger();
    pc_empf.sort();
    assert_eq!(
        pc_empf,
        vec!["dev-iphone".to_owned(), "dev-tablet".to_owned()]
    );
    let epoche = pc.widerrufen(&pc_hub, "dev-tablet").unwrap();
    assert_eq!(epoche, 2);
    assert_eq!(pc.bund().aktuelle_epoche(), 2);
    assert_eq!(pc.empfaenger(), vec!["dev-iphone".to_owned()]);
    assert_eq!(
        hub.epoche(),
        2,
        "Hub leitet die Epoche aus dem Verzeichnis ab"
    );
    // Das widerrufene Gerät kommt nicht mehr hinein, auch nicht mit Zulassung.
    assert_eq!(
        tablet_hub.verzeichnis_lesen(&AuthGeraet::new("dev-tablet"), 0),
        Err(HubFehler::Widerrufen)
    );
    assert_eq!(tablet_hub.zulassen(None), Err(HubFehler::Widerrufen));
    // Das iPhone bekommt den neuen Schlüssel als Paket über HTTP.
    let mut ip = k.geraet("dev-iphone");
    let b = ip.herunterladen(&ip_hub).unwrap();
    assert_eq!(b.schluessel_neu, 1);
    assert_eq!(
        ip.bund().schluessel(2).unwrap().bytes(),
        pc.bund().schluessel(2).unwrap().bytes()
    );
}

#[test]
fn hubausfall_bricht_den_lauf_mit_nichtverfuegbar_ab_und_verliert_nichts() {
    let (hub, konto) = konto_hub();
    let k = Testkonto::mit_hub(&["dev-pc"], &hub);
    let aus = portal(&hub, &konto, "dev-pc");
    netz(&aus, false);
    let mut pc = k.geraet("dev-pc");
    let r = pc
        .unterhaltung_anlegen(kopf("T"), Einschraenkung::default())
        .unwrap();
    pc.beitrag_anlegen(&r, beitrag("a", None, 0, "t")).unwrap();
    assert!(matches!(
        pc.hochladen(&aus),
        Err(SyncFehler::Hub(HubFehler::Nichtverfuegbar))
    ));
    // Später wieder da: alles geht nach.
    let wieder = PortalHub::new(FakeHttp::neu(&hub, &konto, "dev-pc"), "dev-pc");
    assert_eq!(pc.hochladen(&wieder).unwrap().beitraege, 1);
}

// ---------------------------------------------------------------------------------------------
// E3.0 und E3.8: ein bösartiger Hub verändert Objekte, Verzeichnis und Pakete auf der Leitung
// ---------------------------------------------------------------------------------------------

/// Konto mit Widerruf (Epoche 2 am Hub): ein Beitrag in Epoche 1 vom PC, Leser haben beide Schlüssel.
fn konto_mit_epoche_2() -> (Arc<MemHub>, Arc<Mutex<Konto>>, Testkonto, Hex128) {
    let (hub, konto) = konto_hub();
    let k = Testkonto::mit_hub(&["dev-pc", "dev-iphone", "dev-tablet", "dev-weg"], &hub);
    let pc_hub = portal(&hub, &konto, "dev-pc");
    let mut pc = k.geraet("dev-pc");
    let r = pc
        .unterhaltung_anlegen(kopf("T"), Einschraenkung::default())
        .unwrap();
    pc.beitrag_anlegen(&r, beitrag("a", None, 0, "geheim"))
        .unwrap();
    pc.hochladen(&pc_hub).unwrap();
    assert_eq!(pc.widerrufen(&pc_hub, "dev-weg"), Ok(2));
    (hub, konto, k, r)
}

fn mit_manipulation(
    hub: &Arc<MemHub>,
    konto: &Arc<Mutex<Konto>>,
    name: &str,
    m: impl Fn(&str, &mut Value) + Send + Sync + 'static,
) -> Fake {
    let mut p = portal(hub, konto, name);
    p.transport.manipulation = Some(Box::new(m));
    p
}

#[test]
fn e3_0_vertauschte_steuerfelder_auf_der_leitung_werden_je_feld_verworfen() {
    let andere_id = Hex128::zufall().unwrap();
    let andere_ref = Hex128::zufall().unwrap();

    type Aendern = Box<dyn Fn(&mut Value) + Send + Sync>;
    let faelle: Vec<(&str, Aendern)> = vec![
        ("objekt_id", {
            let i = andere_id.clone();
            Box::new(move |o| o["objekt_id"] = json!(i.as_str()))
        }),
        ("unterhaltung_ref", {
            let i = andere_ref.clone();
            Box::new(move |o| o["unterhaltung_ref"] = json!(i.as_str()))
        }),
        ("art", Box::new(|o| o["art"] = json!("hinweis"))),
        ("sammlung", Box::new(|o| o["sammlung"] = json!("hinweis"))),
        (
            "geraet_id",
            Box::new(|o| o["geraet_id"] = json!("dev-tablet")),
        ),
        // Epoche 2 ist gültig und dem Leser bekannt: nur die AAD verwirft das Objekt.
        ("key_epoch", Box::new(|o| o["key_epoch"] = json!("2"))),
        (
            "ciphertext",
            Box::new(|o| {
                let c = o["ciphertext"].as_str().unwrap().to_owned();
                let erstes = if c.starts_with('A') { "B" } else { "A" };
                o["ciphertext"] = json!(format!("{erstes}{}", &c[1..]));
            }),
        ),
    ];
    let transport: Mutex<Vec<(&str, bool)>> = Mutex::new(Vec::new());
    for (feld, aendern) in faelle {
        let (hub, konto, k, r) = konto_mit_epoche_2();
        // Erst die Schlüssel der Epoche 2 holen (ohne Manipulation), dann manipuliert lesen.
        let mut ip = k.geraet("dev-iphone");
        let sauber_ip = portal(&hub, &konto, "dev-iphone");
        ip.verzeichnis_und_pakete_holen(&sauber_ip);
        let angriff = mit_manipulation(&hub, &konto, "dev-iphone", move |pfad, b| {
            if pfad != "/lesen" {
                return;
            }
            for o in b["objekte"].as_array_mut().unwrap() {
                if o["sammlung"] == "beitrag" {
                    aendern(o);
                }
            }
        });
        // Entweder weist schon die Transportgrenze die Seite ab (Form, z. B. `art` passt nicht zur
        // Sammlung), oder der Kern verwirft das Objekt (Bindung der Steuerfelder in der AAD).
        match ip.herunterladen(&angriff) {
            Ok(bericht) => {
                assert_eq!(
                    bericht.beitraege_neu, 0,
                    "{feld}: manipuliertes Objekt darf nicht ankommen"
                );
                assert!(
                    bericht.verworfen + bericht.zurueckgestellt + bericht.nach_widerruf >= 1,
                    "{feld}: das manipulierte Objekt wird nicht übernommen: {bericht:?}"
                );
                transport.lock().unwrap().push((feld, false));
            }
            Err(f) => {
                assert_eq!(
                    f,
                    SyncFehler::Hub(HubFehler::Vertrag("Hub-Antwort ungueltig")),
                    "{feld}"
                );
                transport.lock().unwrap().push((feld, true));
            }
        }
        assert!(
            ip.unterhaltung(&r).is_none_or(|u| u.verlauf().is_empty()),
            "{feld}: nichts landet im Verlauf"
        );
        // Und ohne Manipulation kommt es an: der Test prüft die Bindung, nicht Zufall.
        let sauber = portal(&hub, &konto, "dev-tablet");
        let mut tab = k.geraet("dev-tablet");
        assert_eq!(
            tab.herunterladen(&sauber).unwrap().beitraege_neu,
            1,
            "{feld}: Gegenprobe"
        );
    } // Die kryptografische Bindung muss greifen, wo die Form gültig bleibt.
    let am_kern: Vec<&str> = transport
        .lock()
        .unwrap()
        .iter()
        .filter(|(_, t)| !t)
        .map(|(f, _)| *f)
        .collect();
    for f in [
        "objekt_id",
        "unterhaltung_ref",
        "geraet_id",
        "key_epoch",
        "ciphertext",
    ] {
        assert!(
            am_kern.contains(&f),
            "{f} erreicht den Kern nicht: {am_kern:?}"
        );
    }
}

/// Hilfszug: holt Verzeichnis und Pakete ohne Objekte (ein `herunterladen` gegen einen Hub, der keine
/// Objekte liefert).
trait Vorbereiten {
    fn verzeichnis_und_pakete_holen(&mut self, hub: &Fake);
}

impl Vorbereiten for Abgleich {
    fn verzeichnis_und_pakete_holen(&mut self, hub: &Fake) {
        let leer = PortalHub::new(
            FakeHttp {
                hub: hub.transport.hub.clone(),
                konto: hub.transport.konto.clone(),
                geraet: hub.transport.geraet.clone(),
                log: Mutex::new(Vec::new()),
                manipulation: Some(Box::new(|pfad, b| {
                    if pfad == "/lesen" {
                        *b = json!({ "objekte": [], "cursor": "0", "mehr": false });
                    }
                })),
                offline: AtomicBool::new(false),
            },
            &hub.transport.geraet,
        );
        let b = self.herunterladen(&leer).unwrap();
        assert_eq!(b.schluessel_neu, 1, "Schlüssel der Epoche 2");
    }
}

#[test]
fn e3_8_manipulierte_schluesselpakete_werden_abgewiesen() {
    type Aendern = Box<dyn Fn(&mut Value) + Send + Sync>;
    let faelle: Vec<(&str, Aendern)> = vec![
        (
            "chiffrat",
            Box::new(|p| {
                let c = p["chiffrat"].as_str().unwrap().to_owned();
                let erstes = if c.starts_with('A') { "B" } else { "A" };
                p["chiffrat"] = json!(format!("{erstes}{}", &c[1..]));
            }),
        ),
        ("epoche", Box::new(|p| p["epoche"] = json!("3"))),
        (
            "absender",
            Box::new(|p| p["absender"] = json!("dev-tablet")),
        ),
        (
            "verzeichnis_nummer",
            Box::new(|p| p["verzeichnis_nummer"] = json!("0")),
        ),
        ("signatur", {
            Box::new(|p| {
                let s = p["signatur"].as_str().unwrap().to_owned();
                let erstes = if s.starts_with('0') { "1" } else { "0" };
                p["signatur"] = json!(format!("{erstes}{}", &s[1..]));
            })
        }),
    ];
    for (feld, aendern) in faelle {
        let (hub, konto, k, _) = konto_mit_epoche_2();
        let angriff = mit_manipulation(&hub, &konto, "dev-iphone", move |pfad, b| {
            if pfad == "/pakete" {
                b["pakete"]
                    .as_array_mut()
                    .unwrap()
                    .iter_mut()
                    .for_each(&aendern);
            }
        });
        let mut ip = k.geraet("dev-iphone");
        let b = ip.herunterladen(&angriff).unwrap();
        assert_eq!(
            b.schluessel_neu, 0,
            "{feld}: kein Schlüssel aus einem manipulierten Paket"
        );
        assert!(b.pakete_abgewiesen >= 1, "{feld}: {b:?}");
        assert!(ip.bund().schluessel(2).is_none(), "{feld}");
        assert!(ip.bund().schluessel(3).is_none(), "{feld}");
        // Gegenprobe ohne Manipulation.
        let sauber = portal(&hub, &konto, "dev-iphone");
        assert_eq!(
            ip.herunterladen(&sauber).unwrap().schluessel_neu,
            1,
            "{feld}"
        );
    }
}

#[test]
fn e3_8_manipuliertes_oder_verborgenes_verzeichnis_wird_nicht_uebernommen() {
    // Ein veränderter Widerrufseintrag (andere Epoche, anderes Ziel) passt nicht zur Signatur.
    type Aendern = Box<dyn Fn(&mut Value) + Send + Sync>;
    let faelle: Vec<(&str, Aendern)> = vec![
        (
            "widerruf.geraet_id",
            Box::new(|e| e["aenderung"]["widerruf"]["geraet_id"] = json!("dev-iphone")),
        ),
        (
            "widerruf.epoche",
            Box::new(|e| e["aenderung"]["widerruf"]["epoche"] = json!("5")),
        ),
        (
            "unterzeichner",
            Box::new(|e| e["unterzeichner"] = json!("dev-tablet")),
        ),
    ];
    for (feld, aendern) in faelle {
        let (hub, konto, k, _) = konto_mit_epoche_2();
        let laenge = k.verzeichnis().laenge();
        let angriff = mit_manipulation(&hub, &konto, "dev-tablet", move |pfad, b| {
            if pfad != "/verzeichnis" {
                return;
            }
            for e in b["eintraege"].as_array_mut().unwrap() {
                if e.get("aenderung").and_then(|a| a.get("widerruf")).is_some() {
                    aendern(e);
                }
            }
        });
        let mut tab = k.geraet("dev-tablet");
        let ergebnis = tab.herunterladen(&angriff);
        if let Ok(b) = &ergebnis {
            assert_eq!(b.verzeichnis_neu, 0, "{feld}: {b:?}");
        }
        assert_eq!(tab.verzeichnis().laenge(), laenge, "{feld}");
        assert_eq!(tab.epoche(), 1, "{feld}");
        assert!(
            tab.verzeichnis()
                .stand()
                .mitglied("dev-iphone")
                .unwrap()
                .widerrufen
                .is_none(),
            "{feld}: kein untergeschobener Widerruf"
        );
    }
}

#[test]
fn e3_0_hub_sieht_keinen_klartext_und_nur_die_steuerfelder() {
    let (hub, konto) = konto_hub();
    let k = Testkonto::mit_hub(&["dev-pc"], &hub);
    let p = portal(&hub, &konto, "dev-pc");
    let mut pc = k.geraet("dev-pc");
    let r = pc
        .unterhaltung_anlegen(
            kopf("Streng vertraulicher Titel"),
            Einschraenkung::default(),
        )
        .unwrap();
    pc.beitrag_anlegen(&r, beitrag("a", None, 0, "Inhalt-Geheimnis-4711"))
        .unwrap();
    pc.hochladen(&p).unwrap();
    let draht: String = p
        .transport
        .log
        .lock()
        .unwrap()
        .iter()
        .filter_map(|a| a.body.as_ref().map(Value::to_string))
        .collect();
    assert!(!draht.contains("Inhalt-Geheimnis-4711"));
    assert!(!draht.contains("Streng vertraulicher Titel"));
}

// ---------------------------------------------------------------------------------------------
// E3.3: Sitzungsübernahme mit atomarer Vergabe ((a) bis (c)), je gegen MemHub und gegen den
// Fake-HTTP-Hub
// ---------------------------------------------------------------------------------------------

const LEASE_GERAETE: [&str; 12] = [
    "dev-pc",
    "dev-iphone",
    "dev-pruefer",
    "dev-start",
    "dev-0",
    "dev-1",
    "dev-2",
    "dev-3",
    "dev-4",
    "dev-5",
    "dev-6",
    "dev-7",
];

/// Erzeugt für ein Gerät einen Zugang zum Hub.
type HubFabrik = Box<dyn Fn(&str) -> Box<dyn HubSpeicher + Send + Sync> + Send>;

/// Ein Konto und für jedes Gerät ein Zugang zu demselben Hub.
struct Zugang {
    konto: Testkonto,
    hub: HubFabrik,
}

fn zugang_memhub() -> Zugang {
    let h = Arc::new(MemHub::new());
    let konto = Testkonto::mit_hub(&LEASE_GERAETE, &h);
    Zugang {
        konto,
        hub: Box::new(move |_| Box::new(h.clone())),
    }
}

fn zugang_http() -> Zugang {
    let (hub, k) = konto_hub();
    let konto = Testkonto::mit_hub(&LEASE_GERAETE, &hub);
    k.lock().unwrap().je_zugelassen = true;
    Zugang {
        konto,
        hub: Box::new(move |name| Box::new(PortalHub::new(FakeHttp::neu(&hub, &k, name), name))),
    }
}

fn beide(f: impl Fn(&str, &Zugang)) {
    f("MemHub", &zugang_memhub());
    f("PortalHub/HTTP", &zugang_http());
}

#[test]
fn e3_3_a_antwort_ueber_mindestens_drei_verlaengerungen_bleibt_im_zweig() {
    beide(|name, z| {
        let (hub_pc, hub_ip) = ((z.hub)("dev-pc"), (z.hub)("dev-iphone"));
        let mut pc = z.konto.geraet("dev-pc");
        let r = pc
            .unterhaltung_anlegen(kopf("Lease"), Einschraenkung::default())
            .unwrap();
        let halt = pc.lease_uebernehmen(&*hub_pc, &r, T0, None).unwrap();
        assert_eq!(halt.generation, 1, "{name}");
        let g = halt.generation;
        pc.beitrag_anlegen(&r, beitrag("frage", None, g, "Frage"))
            .unwrap();
        let mut aktuell = halt;
        for i in 1..=4u64 {
            let neu = pc
                .lease_verlaengern(&*hub_pc, &r, T0 + 10 * i, &aktuell)
                .unwrap();
            assert_eq!(
                neu.generation, g,
                "{name}: Verlängerung ändert die generation nicht"
            );
            assert!(
                neu.revision > aktuell.revision,
                "{name}: Revision steigt, ist aber kein Fencing-Wert"
            );
            aktuell = neu;
        }
        pc.beitrag_anlegen(&r, beitrag("antwort", Some("frage"), g, "Antwort"))
            .unwrap();
        pc.hochladen(&*hub_pc).unwrap();

        let mut iphone = z.konto.geraet("dev-iphone");
        iphone.herunterladen(&*hub_ip).unwrap();
        let stand = iphone.lease_lesen(&*hub_ip, &r).unwrap().unwrap();
        assert_eq!(
            (stand.lease.inhaber.as_str(), stand.lease.generation),
            ("dev-pc", 1),
            "{name}"
        );
        let sicht = iphone
            .unterhaltung(&r)
            .unwrap()
            .verlauf()
            .ansicht(stand.lease.generation);
        assert_eq!(sicht.hauptlinie, vec!["frage", "antwort"], "{name}");
        assert!(
            sicht.zweige.is_empty(),
            "{name}: die lange Antwort verzweigt nicht"
        );
    });
}

#[test]
fn e3_3_b_uebernahme_waehrend_laufender_antwort_alte_wird_zweig_neue_haengt_am_gewaehlten() {
    beide(|name, z| {
        let (hub_pc, hub_ip) = ((z.hub)("dev-pc"), (z.hub)("dev-iphone"));
        let mut pc = z.konto.geraet("dev-pc");
        let r = pc
            .unterhaltung_anlegen(kopf("Übernahme"), Einschraenkung::default())
            .unwrap();
        let halt_pc = pc.lease_uebernehmen(&*hub_pc, &r, T0, None).unwrap();
        pc.beitrag_anlegen(&r, beitrag("frage", None, 1, "Frage"))
            .unwrap();
        pc.hochladen(&*hub_pc).unwrap();

        let mut iphone = z.konto.geraet("dev-iphone");
        iphone.herunterladen(&*hub_ip).unwrap();
        let gelesen = iphone.lease_lesen(&*hub_ip, &r).unwrap();
        let halt_ip = iphone
            .lease_uebernehmen(&*hub_ip, &r, T0 + 12, gelesen.as_ref())
            .unwrap();
        assert_eq!(halt_ip.generation, 2, "{name}: Übernahme erhöht um genau 1");
        assert_eq!(
            pc.lease_verlaengern(&*hub_pc, &r, T0 + 20, &halt_pc),
            Err(LeaseFehler::Verloren {
                aktuelle_generation: 2
            }),
            "{name}: der alte Inhaber merkt den Verlust bei der nächsten Verlängerung"
        );
        iphone
            .beitrag_anlegen(&r, beitrag("neu", Some("frage"), 2, "Antwort vom iPhone"))
            .unwrap();
        pc.beitrag_anlegen(&r, beitrag("alt", Some("frage"), 1, "Antwort vom PC"))
            .unwrap();
        pc.hochladen(&*hub_pc).unwrap();
        iphone.hochladen(&*hub_ip).unwrap();
        pc.herunterladen(&*hub_pc).unwrap();
        iphone.herunterladen(&*hub_ip).unwrap();
        for g in [&pc, &iphone] {
            let sicht = g.unterhaltung(&r).unwrap().verlauf().ansicht(2);
            assert_eq!(sicht.hauptlinie, vec!["frage", "neu"], "{name}");
            assert_eq!(sicht.zweige.len(), 1, "{name}");
            assert_eq!(sicht.zweige[0].eintraege, vec!["alt"], "{name}");
            assert_eq!(
                sicht.zweige[0].grund,
                ZweigGrund::VeralteteGeneration,
                "{name}"
            );
            assert!(sicht.zweige[0].veraltet, "{name}");
        }
    });
}

#[test]
fn e3_3_c_gleichzeitige_uebernahme_ergibt_genau_einen_inhaber() {
    beide(|name, z| {
        let r = Hex128::zufall().unwrap();
        let mut geraete: Vec<(Abgleich, _)> = (0..6)
            .map(|i| {
                let n = format!("dev-{i}");
                (z.konto.geraet(&n), (z.hub)(&n))
            })
            .collect();
        // Runde 1: keine Lease vorhanden, alle lesen „nichts“ und übernehmen.
        let ergebnisse: Vec<_> = geraete
            .iter_mut()
            .map(|(g, h)| g.lease_uebernehmen(&**h, &r, T0, None))
            .collect();
        assert_eq!(ergebnisse.iter().filter(|e| e.is_ok()).count(), 1, "{name}");
        assert!(
            ergebnisse
                .iter()
                .filter_map(|e| e.as_ref().err())
                .all(|e| *e == LeaseFehler::Konflikt),
            "{name}"
        );
        // Runde 2: alle lesen denselben Stand, alle übernehmen mit generation + 1.
        let gelesen = {
            let (g, h) = &mut geraete[0];
            g.lease_lesen(&**h, &r).unwrap().unwrap()
        };
        let ergebnisse: Vec<_> = geraete
            .iter_mut()
            .map(|(g, h)| g.lease_uebernehmen(&**h, &r, T0 + 100, Some(&gelesen)))
            .collect();
        let erfolge: Vec<usize> = ergebnisse
            .iter()
            .enumerate()
            .filter(|(_, e)| e.is_ok())
            .map(|(i, _)| i)
            .collect();
        assert_eq!(erfolge.len(), 1, "{name}: genau ein Inhaber");
        let mut pruefer = z.konto.geraet("dev-pruefer");
        let neu = pruefer
            .lease_lesen(&*(z.hub)("dev-pruefer"), &r)
            .unwrap()
            .unwrap();
        assert_eq!(neu.lease.generation, 2, "{name}");
        assert_eq!(neu.lease.inhaber, format!("dev-{}", erfolge[0]), "{name}");
    });
}

#[test]
fn e3_3_c_gleichzeitige_uebernahme_mit_threads_ergibt_genau_einen_inhaber() {
    for (name, z) in [
        ("MemHub", zugang_memhub()),
        ("PortalHub/HTTP", zugang_http()),
    ] {
        for runde in 0..10u64 {
            let r = Hex128::zufall().unwrap();
            {
                let mut a = z.konto.geraet("dev-start");
                assert_eq!(
                    a.lease_uebernehmen(&*(z.hub)("dev-start"), &r, T0, None)
                        .unwrap()
                        .generation,
                    1
                );
            }
            let barriere = Arc::new(Barrier::new(8));
            let handles: Vec<_> = (0..8)
                .map(|i| {
                    let (barriere, r) = (barriere.clone(), r.clone());
                    let n = format!("dev-{i}");
                    let hub = (z.hub)(&n);
                    let mut g = z.konto.geraet(&n);
                    thread::spawn(move || {
                        let gelesen = g.lease_lesen(&*hub, &r).unwrap();
                        barriere.wait();
                        (
                            n,
                            g.lease_uebernehmen(&*hub, &r, T0 + 100 + runde, gelesen.as_ref()),
                        )
                    })
                })
                .collect();
            let ergebnisse: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();
            let sieger: Vec<_> = ergebnisse.iter().filter(|(_, e)| e.is_ok()).collect();
            assert_eq!(sieger.len(), 1, "{name} Runde {runde}: genau ein Inhaber");
            let mut pruefer = z.konto.geraet("dev-pruefer");
            let stand = pruefer
                .lease_lesen(&*(z.hub)("dev-pruefer"), &r)
                .unwrap()
                .unwrap();
            assert_eq!(
                (stand.lease.generation, stand.lease.inhaber.as_str()),
                (2, sieger[0].0.as_str()),
                "{name}"
            );
        }
    }
}

#[test]
fn schranke_verhindert_jeden_weiteren_netzaufruf_vor_dem_transport() {
    let (hub, konto) = konto_hub();
    let sperre = Arc::new(AtomicBool::new(false));
    let p =
        PortalHub::new(FakeHttp::neu(&hub, &konto, "dev-pc"), "dev-pc").mit_sperre(sperre.clone());
    assert_eq!(p.zulassen(None), Ok(1));
    let auth = AuthGeraet::new("dev-pc");
    assert!(p.verzeichnis_lesen(&auth, 0).is_ok());
    let vorher = p.transport.log.lock().unwrap().len();
    assert!(vorher >= 2);

    sperre.store(true, Ordering::SeqCst);
    assert_eq!(
        p.verzeichnis_lesen(&auth, 0),
        Err(HubFehler::Nichtverfuegbar)
    );
    assert_eq!(p.pakete_lesen(&auth), Err(HubFehler::Nichtverfuegbar));
    assert_eq!(
        p.lesen(&auth, &LeseFilter::default()),
        Err(HubFehler::Nichtverfuegbar)
    );
    assert_eq!(p.zulassen(None), Err(HubFehler::Nichtverfuegbar));
    assert!(p.freigeben("dev-x").is_err());
    assert_eq!(p.letzter_code().as_deref(), Some(CODE_ABGEBROCHEN));
    assert_eq!(
        p.transport.log.lock().unwrap().len(),
        vorher,
        "nach dem Ausschalten kein einziger weiterer Aufruf"
    );

    sperre.store(false, Ordering::SeqCst);
    assert!(
        p.verzeichnis_lesen(&auth, 0).is_ok(),
        "ohne Schranke geht es wieder"
    );
}
