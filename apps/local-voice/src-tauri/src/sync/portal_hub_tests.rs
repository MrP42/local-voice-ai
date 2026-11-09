//! Tests des `PortalHub` (lva#388): Fehlerabbildung gegen die Doku des Hubs, Drahtformat und die
//! Abnahmefälle aus ADR-0028 (E3.0 Bindung der Steuerfelder, E3.3 (a) bis (c) Lease) gegen den
//! Referenz-Hub `MemHub` und gegen einen Fake-HTTP-Hub, der die Endpunkte der Hub-Doku nachbildet.
//! Alle Daten sind synthetisch.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Barrier, Mutex};
use std::thread;

use serde_json::{json, Value};
use wai_assistent::sync::{
    Abgleich, AuthGeraet, Beitrag, Einschraenkung, Hex128, HubFehler, HubSpeicher, Kopf,
    LeaseFehler, LeseFilter, MemHub, Modus, Sammlung, Schluessel, Schluesselbund, Schreiben,
    Umschlag, ZweigGrund,
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

pub(crate) struct FakeHttp {
    hub: Arc<MemHub>,
    konto: Arc<Mutex<Konto>>,
    /// Gerät, an das das Token gebunden ist.
    geraet: String,
    log: Mutex<Vec<Anfrage>>,
    /// Ein Angreifer mit Hubzugang verändert Antworten von `/lesen` (Objekt für Objekt).
    manipulation: Option<Box<dyn Fn(&mut Value) + Send + Sync>>,
    /// Netz weg.
    offline: AtomicBool,
}

impl FakeHttp {
    fn neu(hub: &Arc<MemHub>, konto: &Arc<Mutex<Konto>>, geraet: &str) -> Self {
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
        HubFehler::EpocheKonflikt { aktuell } => (
            409,
            "epoche_konflikt",
            json!({ "aktuell": aktuell.to_string() }),
        ),
        HubFehler::NichtZugelassen => (403, "nicht_zugelassen", json!({})),
        HubFehler::Widerrufen => (403, "widerrufen", json!({})),
        HubFehler::GeraetStimmtNichtUeberein => (403, "geraet_stimmt_nicht_ueberein", json!({})),
        HubFehler::GrabsteinAbgewiesen => (410, "grabstein", json!({})),
        HubFehler::NichtVeraenderlich => (422, "nicht_veraenderlich", json!({})),
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
        Ok(self.verarbeiten(a).unwrap_or_else(|e| e))
    }
}

impl FakeHttp {
    fn verarbeiten(&self, a: &Anfrage) -> Result<Antwort, Antwort> {
        let auth = self.auth();
        let fehler = |f: HubFehler| fehler_antwort(&f);
        let body = a.body.clone().unwrap_or(Value::Null);
        let umschlag = |v: &Value| -> Result<Umschlag, Antwort> {
            serde_json::from_value(v.clone()).map_err(|_| fehler_antwort(&HubFehler::Vertrag("u")))
        };
        match (a.methode, a.pfad) {
            (Methode::Post, "/geraet") => {
                let id = body["geraet_id"].as_str().unwrap_or_default().to_owned();
                if id != self.geraet {
                    return Err(fehler(HubFehler::GeraetStimmtNichtUeberein));
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
                Ok(ok(json!({ "geraet_id": id, "epoche": "1" })))
            }
            (Methode::Post, "/freigeben") => {
                self.hub.epoche(&auth).map_err(fehler)?;
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
                let sammlung: Sammlung = serde_json::from_value(json!(q(a, "sammlung")))
                    .map_err(|_| fehler(HubFehler::Vertrag("sammlung")))?;
                let id = Hex128::parse(q(a, "objekt_id").unwrap_or_default())
                    .map_err(|_| fehler(HubFehler::Vertrag("objekt_id")))?;
                let o = self.hub.holen(&auth, sammlung, &id).map_err(fehler)?;
                Ok(ok(json!({ "objekt": o })))
            }
            (Methode::Get, "/lesen") => {
                let nach = zaehler_string(&json!(q(a, "nach_revision").unwrap_or("0")))?;
                let limit: usize = q(a, "limit").and_then(|l| l.parse().ok()).unwrap_or(100);
                let filter = LeseFilter {
                    nach_revision: nach,
                    unterhaltung_ref: q(a, "unterhaltung_ref").and_then(|r| Hex128::parse(r).ok()),
                    sammlung: q(a, "sammlung").and_then(|s| serde_json::from_value(json!(s)).ok()),
                };
                let alle = self.hub.lesen(&auth, &filter).map_err(fehler)?;
                let seite: Vec<&Umschlag> = alle.iter().take(limit.min(100)).collect();
                let mehr = alle.len() > seite.len();
                let cursor = seite.last().and_then(|u| u.revision).unwrap_or(nach);
                let mut objekte: Vec<Value> = seite
                    .iter()
                    .map(|u| serde_json::to_value(u).unwrap())
                    .collect();
                if let Some(m) = &self.manipulation {
                    objekte.iter_mut().for_each(|o| m(o));
                }
                Ok(ok(json!({
                    "objekte": objekte,
                    "cursor": cursor.to_string(),
                    "mehr": mehr
                })))
            }
            (Methode::Post, "/widerruf") => {
                let e = zaehler_string(&body["erwartete_epoche"])?;
                let vorgang = Hex128::parse(body["vorgang"].as_str().unwrap_or_default())
                    .map_err(|_| fehler(HubFehler::Vertrag("vorgang")))?;
                let ziel = body["widerrufenes_geraet"].as_str().unwrap_or_default();
                let n = self
                    .hub
                    .widerrufen(&auth, ziel, e, &vorgang)
                    .map_err(fehler)?;
                Ok(ok(json!({ "epoche": n.to_string() })))
            }
            (Methode::Get, "/epoche") => {
                let e = self.hub.epoche(&auth).map_err(fehler)?;
                Ok(ok(json!({ "epoche": e.to_string() })))
            }
            (Methode::Get, "/geraete") => {
                let g = self.hub.zugelassene(&auth).map_err(fehler)?;
                Ok(ok(json!({ "geraete": g })))
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

fn geraet(name: &str, bund: &Schluesselbund) -> Abgleich {
    let mut a = Abgleich::new(name, bund.clone());
    a.freigabe_einschalten();
    a
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
            "epoche_konflikt",
            json!({ "aktuell": "4" }),
            EpocheKonflikt { aktuell: 4 },
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
    // Fehlende Pflichtzusatzfelder: nie raten.
    assert_eq!(
        abbilden(409, &fehlerkoerper("epoche_konflikt", json!({}))).0,
        HubFehler::Vertrag("Hub-Antwort ungueltig")
    );
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
    let h = fest(403, fehlerkoerper("widerrufen", json!({})));
    assert_eq!(h.epoche(&auth), Err(HubFehler::Widerrufen));
    assert_eq!(h.zugelassene(&auth), Err(HubFehler::Widerrufen));
    assert_eq!(
        h.lesen(&auth, &LeseFilter::default()),
        Err(HubFehler::Widerrufen)
    );
    assert_eq!(h.letzter_code().as_deref(), Some("widerrufen"));
    assert_eq!(h.zulassen(None), Err(HubFehler::Widerrufen));

    let offline = PortalHub::new(Fest(Err(TransportFehler)), "dev-pc");
    assert_eq!(offline.epoche(&auth), Err(HubFehler::Nichtverfuegbar));
    assert_eq!(offline.letzter_code().as_deref(), Some("netz"));
}

#[test]
fn erfolg_loescht_den_gemerkten_fehlercode() {
    let (hub, konto) = konto_hub();
    let p = portal(&hub, &konto, "dev-pc");
    let auth = AuthGeraet::new("dev-pc");
    assert_eq!(p.epoche(&auth), Ok(1));
    assert_eq!(p.letzter_code(), None);
}

#[test]
fn fremde_identitaet_und_fremdes_geraet_im_umschlag_gehen_nie_ans_netz() {
    let (hub, konto) = konto_hub();
    let p = portal(&hub, &konto, "dev-pc");
    let vorher = p.transport.log.lock().unwrap().len();
    assert_eq!(
        p.epoche(&AuthGeraet::new("dev-anders")),
        Err(HubFehler::GeraetStimmtNichtUeberein)
    );

    let bund = Schluesselbund::zufaellig().unwrap();
    let mut fremd = geraet("dev-anders", &bund);
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
fn zu_grosses_chiffrat_wird_vor_dem_netz_abgewiesen() {
    let (hub, konto) = konto_hub();
    let p = portal(&hub, &konto, "dev-pc");
    let bund = Schluesselbund::zufaellig().unwrap();
    let mut pc = geraet("dev-pc", &bund);
    let r = pc
        .unterhaltung_anlegen(kopf("T"), Einschraenkung::default())
        .unwrap();
    pc.beitrag_anlegen(&r, beitrag("a", None, 1, &"x".repeat(3_300_000)))
        .unwrap();
    let vorher = p.transport.log.lock().unwrap().len();
    let f = pc.hochladen(&p).unwrap_err();
    assert!(matches!(
        f,
        wai_assistent::sync::SyncFehler::Hub(HubFehler::Vertrag(_))
    ));
    assert_eq!(
        p.transport.log.lock().unwrap().len(),
        vorher,
        "kein Netzaufruf"
    );
    assert_eq!(p.letzter_code().as_deref(), Some("ciphertext_zu_gross"));
}

// ---------------------------------------------------------------------------------------------
// Drahtformat
// ---------------------------------------------------------------------------------------------

/// Alle Zähler gehen als String, nie als JSON-Zahl, und jede Anfrage trägt nur die
/// dokumentierten Felder.
#[test]
fn zaehler_gehen_als_int64string_ueber_die_leitung() {
    let (hub, konto) = konto_hub();
    let p = portal(&hub, &konto, "dev-pc");
    let bund = Schluesselbund::zufaellig().unwrap();
    let mut pc = geraet("dev-pc", &bund);
    let r = pc
        .unterhaltung_anlegen(kopf("T"), Einschraenkung::default())
        .unwrap();
    pc.beitrag_anlegen(&r, beitrag("a", None, 1, "Text"))
        .unwrap();
    pc.hochladen(&p).unwrap();
    pc.kopf_aendern(&r, kopf("T2")).unwrap();
    pc.hochladen(&p).unwrap(); // cas mit erwarteter Revision
    pc.herunterladen(&p).unwrap();
    p.widerrufen(
        &AuthGeraet::new("dev-pc"),
        "dev-zweit",
        1,
        &Hex128::zufall().unwrap(),
    )
    .ok();

    let log = p.transport.log.lock().unwrap().clone();
    let zahlen_als_zahl = |w: &Value| -> bool {
        fn suche(w: &Value) -> bool {
            match w {
                Value::Number(_) => true,
                Value::Array(a) => a.iter().any(suche),
                Value::Object(o) => o.values().any(suche),
                _ => false,
            }
        }
        suche(w)
    };
    let cas: Vec<_> = log.iter().filter(|a| a.pfad == "/cas").collect();
    assert!(!cas.is_empty());
    for a in &log {
        if let Some(b) = &a.body {
            assert!(!zahlen_als_zahl(b), "{}: JSON-Zahl im Körper", a.pfad);
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
        .filter(|a| a.pfad == "/lesen")
        .all(|a| q(a, "nach_revision").is_some()));
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
    let p = portal(&hub, &konto, "dev-pc");
    let bund = Schluesselbund::zufaellig().unwrap();
    let mut pc = geraet("dev-pc", &bund);
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
    assert_eq!(seiten, 3);
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
fn lesen_traut_einem_kaputten_hub_nicht() {
    let auth = AuthGeraet::new("dev-pc");
    // `mehr` ohne Fortschritt: sonst endlose Schleife.
    let h = fest(200, json!({ "objekte": [], "cursor": "0", "mehr": true }));
    assert!(matches!(
        h.lesen(&auth, &LeseFilter::default()),
        Err(HubFehler::Vertrag(_))
    ));
    // Unbrauchbare Objekte.
    let h = fest(
        200,
        json!({ "objekte": [{ "x": 1 }], "cursor": "1", "mehr": false }),
    );
    assert!(matches!(
        h.lesen(&auth, &LeseFilter::default()),
        Err(HubFehler::Vertrag(_))
    ));
    // Fehlende Pflichtfelder in anderen Antworten.
    assert!(matches!(
        fest(200, json!({})).epoche(&auth),
        Err(HubFehler::Vertrag(_))
    ));
    assert!(matches!(
        fest(200, json!({ "geraete": [1] })).zugelassene(&auth),
        Err(HubFehler::Vertrag(_))
    ));
}

#[test]
fn rundlauf_ueber_zwei_geraete_per_http_ohne_verlust_und_dopplung() {
    let (hub, konto) = konto_hub();
    let pc_hub = portal(&hub, &konto, "dev-pc");
    let ip_hub = portal(&hub, &konto, "dev-iphone");
    let bund = Schluesselbund::zufaellig().unwrap();
    let mut pc = geraet("dev-pc", &bund);
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
    let mut ip = geraet("dev-iphone", &bund);
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
    assert_eq!(tablet.epoche(&AuthGeraet::new("dev-tablet")), Ok(1));
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
fn widerruf_per_http_und_geraeteliste() {
    let (hub, konto) = konto_hub();
    let pc_hub = portal(&hub, &konto, "dev-pc");
    let _tablet = portal(&hub, &konto, "dev-tablet");
    let bund = Schluesselbund::zufaellig().unwrap();
    let mut pc = geraet("dev-pc", &bund);
    assert_eq!(
        pc.empfaenger_fuer_epoche(&pc_hub).unwrap(),
        vec!["dev-tablet".to_owned()]
    );
    let epoche = pc.widerrufen(&pc_hub, "dev-tablet").unwrap();
    assert_eq!(epoche, 2);
    assert_eq!(pc.bund().aktuelle_epoche(), 2);
    assert!(pc.empfaenger_fuer_epoche(&pc_hub).unwrap().is_empty());
    // Das widerrufene Gerät kommt nicht mehr hinein.
    let tablet_hub = PortalHub::new(FakeHttp::neu(&hub, &konto, "dev-tablet"), "dev-tablet");
    assert_eq!(
        tablet_hub.epoche(&AuthGeraet::new("dev-tablet")),
        Err(HubFehler::Widerrufen)
    );
}

#[test]
fn hubausfall_bricht_den_lauf_mit_nichtverfuegbar_ab_und_verliert_nichts() {
    let (hub, konto) = konto_hub();
    let aus = portal(&hub, &konto, "dev-pc");
    netz(&aus, false);
    let bund = Schluesselbund::zufaellig().unwrap();
    let mut pc = geraet("dev-pc", &bund);
    let r = pc
        .unterhaltung_anlegen(kopf("T"), Einschraenkung::default())
        .unwrap();
    pc.beitrag_anlegen(&r, beitrag("a", None, 0, "t")).unwrap();
    assert!(matches!(
        pc.hochladen(&aus),
        Err(wai_assistent::sync::SyncFehler::Hub(
            HubFehler::Nichtverfuegbar
        ))
    ));
    // Später wieder da: alles geht nach.
    let wieder = PortalHub::new(FakeHttp::neu(&hub, &konto, "dev-pc"), "dev-pc");
    assert_eq!(pc.hochladen(&wieder).unwrap().beitraege, 1);
}

// ---------------------------------------------------------------------------------------------
// E3.0: sichtbare Steuerfelder sind kryptografisch gebunden
// ---------------------------------------------------------------------------------------------

/// Bund mit gleichem Schlüsselmaterial in Epoche 2: nur die AAD kann die Epoche noch binden.
fn bund_mit_zweiter_epoche(bund: &Schluesselbund) -> Schluesselbund {
    let mut b = bund.clone();
    b.epoche_aufnehmen(
        2,
        Schluessel::aus_bytes(*bund.schluessel(1).unwrap().bytes()),
    )
    .unwrap();
    b
}

#[test]
fn e3_0_vertauschte_steuerfelder_auf_der_leitung_werden_je_feld_verworfen() {
    let bund = Schluesselbund::zufaellig().unwrap();
    let leser_bund = bund_mit_zweiter_epoche(&bund);
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
            Box::new(|o| o["geraet_id"] = json!("dev-fremd")),
        ),
        ("key_epoch", Box::new(|o| o["key_epoch"] = json!("2"))),
    ];
    for (feld, aendern) in faelle {
        let (hub, konto) = konto_hub();
        let pc_hub = portal(&hub, &konto, "dev-pc");
        // Epoche 2 am Hub, damit „key_epoch 2“ gültig wäre und nur die AAD es verwirft.
        hub.zulassen("dev-weg");
        let mut pc = geraet("dev-pc", &bund);
        let r = pc
            .unterhaltung_anlegen(kopf("T"), Einschraenkung::default())
            .unwrap();
        pc.beitrag_anlegen(&r, beitrag("a", None, 0, "geheim"))
            .unwrap();
        pc.hochladen(&pc_hub).unwrap();
        pc_hub
            .widerrufen(
                &AuthGeraet::new("dev-pc"),
                "dev-weg",
                1,
                &Hex128::zufall().unwrap(),
            )
            .unwrap();

        let mut angriff = portal(&hub, &konto, "dev-iphone");
        // Nur Beiträge verändern (der Hub liefert manipulierte Objekte).
        angriff.transport.manipulation = Some(Box::new(move |o: &mut Value| {
            if o["sammlung"] == "beitrag" {
                aendern(o);
            }
        }));
        let mut ip = geraet("dev-iphone", &leser_bund);
        let bericht = ip.herunterladen(&angriff).unwrap();
        assert_eq!(
            bericht.beitraege_neu, 0,
            "{feld}: manipuliertes Objekt darf nicht ankommen"
        );
        assert_eq!(
            bericht.verworfen, 1,
            "{feld}: genau das manipulierte Objekt wird verworfen"
        );
        assert!(
            ip.unterhaltung(&r).map_or(true, |u| u.verlauf().is_empty()),
            "{feld}: nichts landet im Verlauf"
        );
        // Und ohne Manipulation kommt es an: der Test prüft die Bindung, nicht Zufall.
        let sauber = portal(&hub, &konto, "dev-tablet");
        let mut tab = geraet("dev-tablet", &leser_bund);
        assert_eq!(
            tab.herunterladen(&sauber).unwrap().beitraege_neu,
            1,
            "{feld}: Gegenprobe"
        );
    }
}

#[test]
fn e3_0_hub_sieht_keinen_klartext_und_nur_die_steuerfelder() {
    let (hub, konto) = konto_hub();
    let p = portal(&hub, &konto, "dev-pc");
    let bund = Schluesselbund::zufaellig().unwrap();
    let mut pc = geraet("dev-pc", &bund);
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
    assert!(
        !draht.contains("dev-pc") || draht.contains("\"geraet_id\":\"dev-pc\""),
        "Gerät nur als Steuerfeld"
    );
}

// ---------------------------------------------------------------------------------------------
// E3.3: Sitzungsübernahme mit atomarer Vergabe ((a) bis (c)), je gegen MemHub und gegen den
// Fake-HTTP-Hub
// ---------------------------------------------------------------------------------------------

/// Erzeugt für jedes Gerät einen Zugang zu demselben Hub.
type Zugang = Box<dyn Fn(&str) -> Box<dyn HubSpeicher + Send + Sync>>;

fn zugang_memhub() -> Zugang {
    let h = Arc::new(MemHub::new());
    for g in ["dev-pc", "dev-iphone", "dev-pruefer", "dev-start"]
        .into_iter()
        .chain([
            "dev-0", "dev-1", "dev-2", "dev-3", "dev-4", "dev-5", "dev-6", "dev-7",
        ])
    {
        h.zulassen(g);
    }
    Box::new(move |_| Box::new(h.clone()))
}

fn zugang_http() -> Zugang {
    let (hub, konto) = konto_hub();
    for g in [
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
    ] {
        hub.zulassen(g);
    }
    konto.lock().unwrap().je_zugelassen = true;
    Box::new(move |name| Box::new(PortalHub::new(FakeHttp::neu(&hub, &konto, name), name)))
}

fn beide(f: impl Fn(&str, &Zugang)) {
    f("MemHub", &zugang_memhub());
    f("PortalHub/HTTP", &zugang_http());
}

#[test]
fn e3_3_a_antwort_ueber_mindestens_drei_verlaengerungen_bleibt_im_zweig() {
    beide(|name, zugang| {
        let bund = Schluesselbund::zufaellig().unwrap();
        let (hub_pc, hub_ip) = (zugang("dev-pc"), zugang("dev-iphone"));
        let mut pc = geraet("dev-pc", &bund);
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

        let mut iphone = geraet("dev-iphone", &bund);
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
    beide(|name, zugang| {
        let bund = Schluesselbund::zufaellig().unwrap();
        let (hub_pc, hub_ip) = (zugang("dev-pc"), zugang("dev-iphone"));
        let mut pc = geraet("dev-pc", &bund);
        let r = pc
            .unterhaltung_anlegen(kopf("Übernahme"), Einschraenkung::default())
            .unwrap();
        let halt_pc = pc.lease_uebernehmen(&*hub_pc, &r, T0, None).unwrap();
        pc.beitrag_anlegen(&r, beitrag("frage", None, 1, "Frage"))
            .unwrap();
        pc.hochladen(&*hub_pc).unwrap();

        let mut iphone = geraet("dev-iphone", &bund);
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
    beide(|name, zugang| {
        let bund = Schluesselbund::zufaellig().unwrap();
        let r = Hex128::zufall().unwrap();
        let mut geraete: Vec<(Abgleich, _)> = (0..6)
            .map(|i| {
                let n = format!("dev-{i}");
                (geraet(&n, &bund), zugang(&n))
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
        let gelesen = geraete[0]
            .0
            .lease_lesen(&*geraete[0].1, &r)
            .unwrap()
            .unwrap();
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
        let neu = geraete[0]
            .0
            .lease_lesen(&*geraete[0].1, &r)
            .unwrap()
            .unwrap();
        assert_eq!(neu.lease.generation, 2, "{name}");
        assert_eq!(neu.lease.inhaber, format!("dev-{}", erfolge[0]), "{name}");
    });
}

#[test]
fn e3_3_c_gleichzeitige_uebernahme_mit_threads_ergibt_genau_einen_inhaber() {
    let bund = Schluesselbund::zufaellig().unwrap();
    for (name, zugang) in [
        ("MemHub", zugang_memhub()),
        ("PortalHub/HTTP", zugang_http()),
    ] {
        let zugang = Arc::new(Mutex::new(zugang));
        for runde in 0..10u64 {
            let r = Hex128::zufall().unwrap();
            {
                let z = zugang.lock().unwrap();
                let mut a = geraet("dev-start", &bund);
                assert_eq!(
                    a.lease_uebernehmen(&*z("dev-start"), &r, T0, None)
                        .unwrap()
                        .generation,
                    1
                );
            }
            let barriere = Arc::new(Barrier::new(8));
            let handles: Vec<_> = (0..8)
                .map(|i| {
                    let (bund, barriere, r) = (bund.clone(), barriere.clone(), r.clone());
                    let n = format!("dev-{i}");
                    let hub = zugang.lock().unwrap()(&n);
                    thread::spawn(move || {
                        let mut g = geraet(&n, &bund);
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
            let z = zugang.lock().unwrap();
            let pruefer = geraet("dev-pruefer", &bund);
            let stand = pruefer
                .lease_lesen(&*z("dev-pruefer"), &r)
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
