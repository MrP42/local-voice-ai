//! ADR-0026 PR-7 (Epic wai-sdk#1047): Besprechungsordner und Buecher <-> Suite-Projekt, Loeschkaskade
//! und die Recherche. Ohne Netz: Fake-Suchanbieter, Fake-Modell und ein lokaler Testserver (Loopback
//! erlaubt). Echter `MeetingStore` in einem Temp-Ordner. Nur synthetische Daten (`example.*`).

use super::audit::{AuditEreignis, AuditSenke};
use super::dienst::ProjektDienst;
use super::quellen::{Inhalt, ProdQuellen};
use super::recherche::{
    AufrufFut, FreigabeAktion, ModellQuelle, RechercheDienst, RechercheStore, RueckrufModell, Webzugang,
    modell_erlaubt, pruefe_adresse,
};
use super::{Fehler, lokaler_principal};
use crate::managers::meetings::search::index::tests::{ready_meeting, tmp_store};
use crate::managers::meetings::store::MeetingStore;
use async_trait::async_trait;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use wai_projekte::Klasse;
use wai_recherche::{
    AbrufKonfig, Datenklasse, IpPruefer, Modell, ModellAntwort, Pruefurteil, RechercheFehler,
    SichererAbruf, Suchanbieter, Suchanfrage, Treffer,
};
use wai_security::{
    ActorId, ClientClaims, DataClass, Principal, Role, ServerSession, TenantId, resolve_principal,
};

const FRAGE: &str = "Bis wann ist die kommunale Waermeplanung verpflichtend?";
const ADRESSE: &str = "https://suche.example.test";

// ---------------------------------------------------------------- Fakes

async fn server() -> u16 {
    let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = l.local_addr().unwrap().port();
    tokio::spawn(async move {
        loop {
            let Ok((mut s, _)) = l.accept().await else {
                return;
            };
            tokio::spawn(async move {
                let mut buf = vec![0u8; 8192];
                let n = s.read(&mut buf).await.unwrap_or(0);
                let req = String::from_utf8_lossy(&buf[..n]).to_string();
                let pfad = req.split_whitespace().nth(1).unwrap_or("/").to_string();
                let (status, body) = if pfad == "/a" {
                    (
                        "200 OK",
                        "<html><head><title>Waermeplanung Frist</title></head><body><main><p>Die kommunale Waermeplanung ist bis 2028 verpflichtend.</p></main></body></html>".to_string(),
                    )
                } else {
                    ("404 Not Found", String::new())
                };
                let antwort = format!(
                    "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = s.write_all(antwort.as_bytes()).await;
                let _ = s.shutdown().await;
            });
        }
    });
    port
}

struct FakeSuche {
    port: u16,
    anfragen: Mutex<Vec<String>>,
}

#[async_trait]
impl Suchanbieter for FakeSuche {
    async fn suchen(&self, a: &Suchanfrage) -> Result<Vec<Treffer>, RechercheFehler> {
        self.anfragen.lock().unwrap().push(a.text.clone());
        Ok(vec![Treffer {
            titel: "Treffer a".into(),
            url: format!("http://127.0.0.1:{}/a", self.port),
            auszug: "x".into(),
            datum: None,
        }])
    }
}

struct FakeWeb {
    suche: Arc<FakeSuche>,
    abruf: Arc<SichererAbruf>,
}

impl Webzugang for FakeWeb {
    fn anbieter(&self, _adresse: &str) -> Result<Arc<dyn Suchanbieter>, RechercheFehler> {
        Ok(self.suche.clone())
    }
    fn abruf(&self) -> Arc<SichererAbruf> {
        self.abruf.clone()
    }
}

struct FakeModell;

#[async_trait]
impl Modell for FakeModell {
    fn name(&self) -> &str {
        "fake-modell"
    }
    async fn antworten(
        &self,
        anweisung: &str,
        _eingabe: &str,
        _k: Datenklasse,
    ) -> Result<ModellAntwort, RechercheFehler> {
        let text = if anweisung.starts_with("Du planst") {
            r#"{"queries":["waermeplanung frist","waermeplanung gemeinden"]}"#.to_string()
        } else {
            serde_json::json!({
                "sentences": [{"text": "Die kommunale Waermeplanung ist bis 2028 verpflichtend.", "citations": [1]}],
                "follow_ups": ["Wer ist betroffen?", "Gibt es Foerdermittel?", "Was passiert bei Verzug?"]
            })
            .to_string()
        };
        Ok(ModellAntwort { text, tokens: 10 })
    }
    async fn satz_gestuetzt(
        &self,
        _s: &str,
        _a: &str,
        _k: Datenklasse,
    ) -> Result<Option<Pruefurteil>, RechercheFehler> {
        Ok(Some(Pruefurteil {
            gestuetzt: true,
            tokens: 1,
        }))
    }
}

struct FakeModelle;
impl ModellQuelle for FakeModelle {
    fn modell(&self, _model_id: &str) -> Arc<dyn Modell> {
        Arc::new(FakeModell)
    }
}

#[derive(Default)]
struct Mitschnitt(Mutex<Vec<AuditEreignis>>);

impl AuditSenke for Mitschnitt {
    fn aufzeichnen(&self, e: AuditEreignis) {
        self.0.lock().unwrap().push(e);
    }
}

// ---------------------------------------------------------------- Umgebung

struct Env {
    _dir: tempfile::TempDir,
    _meetings_dir: tempfile::TempDir,
    store: Arc<MeetingStore>,
    p: Principal,
    projekte: Arc<ProjektDienst>,
    dienst: Arc<RechercheDienst>,
    rstore: RechercheStore,
    suche: Arc<FakeSuche>,
    audit: Arc<Mitschnitt>,
    buecher: Arc<Mutex<Vec<Inhalt>>>,
    zeit: Arc<Mutex<Duration>>,
}

fn person(mandant: &str, rolle: Role) -> Principal {
    let session = ServerSession {
        tenant: TenantId(mandant.into()),
        actor: ActorId("jemand".into()),
        role: rolle,
        max_class: DataClass::Restricted,
        tool_rights: Default::default(),
    };
    resolve_principal(&session, &ClientClaims::default()).unwrap()
}

async fn env(konfiguriert: bool) -> Env {
    env_mit(konfiguriert, Arc::new(FakeModelle)).await
}

async fn env_mit(konfiguriert: bool, modelle: Arc<dyn ModellQuelle>) -> Env {
    super::tls_anbieter();
    let dir = tempfile::tempdir().unwrap();
    let (meetings_dir, store) = tmp_store();
    let store = Arc::new(store);
    let p = lokaler_principal();
    let buecher = Arc::new(Mutex::new(Vec::<Inhalt>::new()));
    let b = buecher.clone();
    let quellen = Arc::new(ProdQuellen::neu(store.clone(), move || {
        Ok(b.lock().unwrap().clone())
    }));
    let rstore = RechercheStore::open(dir.path().join("recherche.db"))
        .await
        .unwrap();
    let audit = Arc::new(Mitschnitt::default());
    let projekte = Arc::new(
        ProjektDienst::open(
            dir.path().join("projekte.db"),
            quellen,
            rstore.clone(),
            p.clone(),
        )
        .await
        .unwrap()
        .with_audit(audit.clone()),
    );
    let port = server().await;
    let mut konfig = AbrufKonfig::default();
    konfig.erlaubte_ports.push(port);
    let suche = Arc::new(FakeSuche {
        port,
        anfragen: Mutex::new(vec![]),
    });
    let web = Arc::new(FakeWeb {
        suche: suche.clone(),
        abruf: Arc::new(SichererAbruf::mit(IpPruefer::loopback_erlaubt(), konfig)),
    });
    let start = Instant::now();
    let zeit = Arc::new(Mutex::new(Duration::ZERO));
    let uhr_zeit = zeit.clone();
    let dienst = Arc::new(
        RechercheDienst::open(
            rstore.clone(),
            projekte.clone(),
            modelle,
            web,
            p.clone(),
        )
        .await
        .unwrap()
        .with_audit(audit.clone())
        .with_uhr(Arc::new(move || start + *uhr_zeit.lock().unwrap())),
    );
    if konfiguriert {
        dienst.konfigurieren(&p, Some(ADRESSE)).await.unwrap();
    }
    Env {
        _dir: dir,
        _meetings_dir: meetings_dir,
        store,
        p,
        projekte,
        dienst,
        rstore,
        suche,
        audit,
        buecher,
        zeit,
    }
}

/// Ordner mit einer Besprechung, verknuepft mit einem Suite-Projekt (Klasse PERSONAL).
async fn verknuepft(e: &Env, web: bool) -> (String, String, String) {
    verknuepft_klasse(e, web, Klasse::Personal).await
}

async fn verknuepft_klasse(e: &Env, web: bool, klasse: Klasse) -> (String, String, String) {
    let m = ready_meeting(&e.store, "Waermewende Auftakt", 1_750_000_000).id;
    let ordner = e.store.folder_save(None, "Waermewende", None).unwrap().id;
    e.store.set_meeting_folders(&m, std::slice::from_ref(&ordner)).unwrap();
    let suite = e
        .projekte
        .anlegen("Waermewende", klasse, false)
        .await
        .unwrap();
    if web {
        e.projekte.web_freigabe(&suite.id, true).await.unwrap();
    }
    e.projekte
        .ordner_verknuepfen(&ordner, &suite.id)
        .await
        .unwrap();
    (ordner, m, suite.id)
}

fn alles_audit(e: &Env) -> String {
    format!("{:?}", e.audit.0.lock().unwrap())
}

fn code(f: Fehler) -> &'static str {
    f.code
}

// ---------------------------------------------------------------- Verknuepfung Ordner und Buecher

#[tokio::test]
async fn verknuepfen_ordnet_zu_und_loesen_nimmt_nur_die_eintraege_weg() {
    let e = env(false).await;
    let (ordner, m, suite) = verknuepft(&e, false).await;
    let liste = e.projekte.eintraege(&suite).await.unwrap();
    assert_eq!(liste.len(), 1);
    assert_eq!(liste[0].kind, "meeting");
    assert!(liste[0].available);
    assert_eq!(
        liste[0].link.as_deref(),
        Some(format!("wai://lva/besprechung/{m}").as_str())
    );
    assert_eq!(liste[0].assigned_by.as_deref(), Some("automation"));
    assert_eq!(
        e.store.ordner_verweise().unwrap()[0].projekt_id.as_deref(),
        Some(suite.as_str())
    );
    // Erneutes Verknuepfen ist idempotent.
    let b = e.projekte.ordner_verknuepfen(&ordner, &suite).await.unwrap();
    assert_eq!((b.zugeordnet, b.bereits), (0, 1));

    let entfernt = e.projekte.ordner_loesen(&ordner).await.unwrap();
    assert_eq!(entfernt, 1);
    assert!(e.projekte.eintraege(&suite).await.unwrap().is_empty());
    assert_eq!(e.store.ordner_verweise().unwrap()[0].projekt_id, None);
    // Die Besprechung selbst bleibt.
    assert_eq!(e.store.besprechungen_kurz().unwrap().len(), 1);
}

#[tokio::test]
async fn ein_ordner_zeigt_nur_auf_ein_projekt_und_unbekannte_ziele_scheitern() {
    let e = env(false).await;
    let (ordner, _m, _suite) = verknuepft(&e, false).await;
    let zweites = e
        .projekte
        .anlegen("Zweites", Klasse::Personal, false)
        .await
        .unwrap();
    let err = e
        .projekte
        .ordner_verknuepfen(&ordner, &zweites.id)
        .await
        .unwrap_err();
    assert_eq!(code(err), "projekte.invalid");
    let err = e
        .projekte
        .ordner_verknuepfen("gibt-es-nicht", &zweites.id)
        .await
        .unwrap_err();
    assert_eq!(code(err), "projekte.not_found");
    let err = e
        .projekte
        .ordner_verknuepfen(&ordner, "../x")
        .await
        .unwrap_err();
    assert_eq!(code(err), "projekte.invalid");
}

#[tokio::test]
async fn besprechung_verlaesst_den_ordner_und_damit_den_projektordner() {
    let e = env(false).await;
    let (_ordner, m, suite) = verknuepft(&e, false).await;
    assert_eq!(e.projekte.eintraege(&suite).await.unwrap().len(), 1);
    e.store.set_meeting_folders(&m, &[]).unwrap();
    let b = e.projekte.projekt_abgleichen(&suite).await.unwrap();
    assert_eq!(b.entfernt, 1);
    assert!(e.projekte.eintraege(&suite).await.unwrap().is_empty());
}

#[tokio::test]
async fn projektklasse_begrenzt_die_zuordnung_von_besprechungen() {
    let e = env(false).await;
    let (_o, _m, suite) = verknuepft_klasse(&e, false, Klasse::Internal).await;
    // Besprechungen sind PERSONAL: ein internes Projekt nimmt sie nicht auf.
    assert!(e.projekte.eintraege(&suite).await.unwrap().is_empty());
    let b = e.projekte.projekt_abgleichen(&suite).await.unwrap();
    assert_eq!(b.zugeordnet, 0);
    assert!(b.uebersprungen >= 1);
}

#[tokio::test]
async fn buecher_werden_als_verweis_zugeordnet_und_entfernt() {
    let e = env(false).await;
    e.buecher.lock().unwrap().push(Inhalt {
        id: "book_1730000000000".into(),
        titel: "Beispielbuch".into(),
    });
    let suite = e
        .projekte
        .anlegen("Lesekreis", Klasse::Internal, false)
        .await
        .unwrap();
    e.projekte
        .buch_zuordnen(&suite.id, "book_1730000000000")
        .await
        .unwrap();
    // Idempotent.
    e.projekte
        .buch_zuordnen(&suite.id, "book_1730000000000")
        .await
        .unwrap();
    let liste = e.projekte.eintraege(&suite.id).await.unwrap();
    assert_eq!(liste.len(), 1);
    assert_eq!(liste[0].kind, "book");
    assert_eq!(
        liste[0].link.as_deref(),
        Some("wai://lva/buch/book_1730000000000")
    );
    // Unbekanntes Buch scheitert.
    assert!(
        e.projekte
            .buch_zuordnen(&suite.id, "book_9")
            .await
            .is_err()
    );
    // Buch loeschen: der Eintrag verschwindet beim Abgleich.
    e.buecher.lock().unwrap().clear();
    assert_eq!(e.projekte.abgleichen().await.unwrap(), 1);
    assert!(e.projekte.eintraege(&suite.id).await.unwrap().is_empty());
    assert!(e.projekte.offene_meldungen().await.unwrap().is_empty());
}

#[tokio::test]
async fn geloeschte_besprechung_verschwindet_aus_dem_projektordner() {
    let e = env(false).await;
    let (_o, m, suite) = verknuepft(&e, false).await;
    e.store.soft_delete_meeting(&m).unwrap();
    assert!(e.projekte.eintraege(&suite).await.unwrap().is_empty());
    assert!(e.projekte.offene_meldungen().await.unwrap().is_empty());
}

// ---------------------------------------------------------------- Loeschkaskade und Aufbewahrung

#[tokio::test]
async fn loeschkaskade_recherche_und_suite_projekt() {
    let e = env(true).await;
    let (ordner, _m, suite) = verknuepft(&e, true).await;
    let id = freigeben_und_laufen(&e, &suite).await;
    // Die Recherche liegt als Eintrag im Projektordner (Meeting plus Recherche).
    let liste = e.projekte.eintraege(&suite).await.unwrap();
    assert_eq!(liste.len(), 2);
    assert!(
        liste
            .iter()
            .any(|x| x.link.as_deref() == Some(format!("wai://lva/recherche/{id}").as_str()))
    );

    // Recherche loeschen: der Eintrag verschwindet.
    e.dienst.loeschen(&id).await.unwrap();
    assert_eq!(e.projekte.eintraege(&suite).await.unwrap().len(), 1);
    assert!(e.dienst.liste(None).await.unwrap().is_empty());

    // Zweite Recherche, dann das Suite-Projekt loeschen: Recherchen und Verknuepfung gehen mit.
    let id2 = freigeben_und_laufen(&e, &suite).await;
    let r = e.projekte.loeschen(&suite).await.unwrap();
    assert_eq!(r.ordner_geloest, 1);
    assert_eq!(r.recherchen_geloescht, 1);
    assert!(e.dienst.lesen(&id2).await.is_err());
    assert_eq!(e.store.ordner_verweise().unwrap()[0].projekt_id, None);
    assert_eq!(e.store.ordner_verweise().unwrap()[0].id, ordner);
    assert!(e.projekte.projekte().await.unwrap().is_empty());
    // Es bleibt hoechstens die Meldung des Projekts selbst fuer andere Apps; Verweise der eigenen App
    // sind bestaetigt.
    assert!(
        e.projekte
            .offene_meldungen()
            .await
            .unwrap()
            .iter()
            .all(|(_, r)| r.as_deref().is_none_or(|r| !r.starts_with("wai://lva/")))
    );
    assert_eq!(e.store.besprechungen_kurz().unwrap().len(), 1);
}

#[tokio::test]
async fn aufbewahrung_des_projekts_gilt_auch_fuer_recherchen() {
    let e = env(true).await;
    let (_o, _m, suite) = verknuepft(&e, true).await;
    let id = freigeben_und_laufen(&e, &suite).await;
    // Ohne retention_days bleibt alles.
    assert_eq!(e.projekte.aufbewahrung_recherchen().await.unwrap(), 0);
    // Projekt mit Aufbewahrung von 1 Tag, Dokument zwei Tage alt.
    let alt = e.projekte.projekt(&suite).await.unwrap();
    e.projekte
        .verzeichnis()
        .projekt_aendern(
            &e.p,
            &suite,
            alt.revision,
            wai_projekte::Aenderung {
                retention_days: Some(Some(1)),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let vor_zwei_tagen = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64
        - 2 * 86_400_000;
    e.rstore.anlagezeit_setzen(&id, vor_zwei_tagen).await.unwrap();
    assert_eq!(e.projekte.aufbewahrung_recherchen().await.unwrap(), 1);
    assert!(e.dienst.lesen(&id).await.is_err());
    assert_eq!(e.projekte.eintraege(&suite).await.unwrap().len(), 1);
}

// ---------------------------------------------------------------- Recherche

/// Startet eine Recherche im Projekt (Besprechungskontext PERSONAL), gibt alle Anfragen frei und
/// liefert die Kennung des gespeicherten Dokuments.
async fn freigeben_und_laufen(e: &Env, suite: &str) -> String {
    let a = e
        .dienst
        .starten(FRAGE, "m1", Datenklasse::Personal, Some(suite.to_string()))
        .await;
    assert_eq!(a.status, "freigabe", "{a:?}");
    let offen = e.dienst.freigaben();
    let h = offen.iter().find(|h| Some(&h.id) == a.recherche_id.as_ref()).unwrap();
    let alle: Vec<String> = h.queries.iter().map(|q| q.id.clone()).collect();
    let f = e
        .dienst
        .freigabe_entscheiden(&h.id, &alle, FreigabeAktion::Senden)
        .await
        .unwrap();
    assert_eq!(f.status, "fertig", "{f:?}");
    f.recherche_id.unwrap()
}

#[tokio::test]
async fn ohne_suchadresse_ist_die_recherche_sauber_deaktiviert() {
    let e = env(false).await;
    let (_o, _m, suite) = verknuepft(&e, true).await;
    let hinweis = e.dienst.hinweis().expect("deaktiviert");
    assert!(hinweis.contains("SearXNG"), "{hinweis}");
    assert!(!e.dienst.status(&e.p).available);
    let a = e
        .dienst
        .starten(FRAGE, "m1", Datenklasse::Personal, Some(suite))
        .await;
    assert_eq!(a.status, "fehler");
    assert!(e.suche.anfragen.lock().unwrap().is_empty());
}

#[tokio::test]
async fn web_ist_ohne_verknuepftes_projekt_mit_freigabe_gesperrt() {
    let e = env(true).await;
    // Kein Projekt.
    let a = e
        .dienst
        .starten(FRAGE, "m1", Datenklasse::Personal, None)
        .await;
    assert_eq!(a.status, "fehler");
    assert!(a.message.unwrap().contains("Suite-Projekt"));
    // Projekt ohne Web-Freigabe.
    let (_o, _m, suite) = verknuepft(&e, false).await;
    let a = e
        .dienst
        .starten(FRAGE, "m1", Datenklasse::Personal, Some(suite.clone()))
        .await;
    assert_eq!(a.status, "fehler");
    assert!(a.message.unwrap().contains("nicht freigegeben"));
    // Unbekanntes Projekt.
    let a = e
        .dienst
        .starten(FRAGE, "m1", Datenklasse::Personal, Some("prj_unbekannt".into()))
        .await;
    assert_eq!(a.status, "fehler");
    assert!(e.suche.anfragen.lock().unwrap().is_empty());
    assert!(e.dienst.freigaben().is_empty());
    // Mit Freigabe haelt der Lauf an, statt zu suchen.
    e.projekte.web_freigabe(&suite, true).await.unwrap();
    let a = e
        .dienst
        .starten(FRAGE, "m1", Datenklasse::Personal, Some(suite))
        .await;
    assert_eq!(a.status, "freigabe");
    assert!(e.suche.anfragen.lock().unwrap().is_empty());
}

#[tokio::test]
async fn ab_personal_haelt_der_lauf_an_und_die_teilauswahl_sendet_nur_diese_anfrage() {
    let e = env(true).await;
    let (_o, _m, suite) = verknuepft(&e, true).await;
    let a = e
        .dienst
        .starten(FRAGE, "m1", Datenklasse::Personal, Some(suite.clone()))
        .await;
    assert_eq!(a.status, "freigabe");
    assert!(
        e.suche.anfragen.lock().unwrap().is_empty(),
        "vor der Freigabe verlaesst nichts die Umgebung"
    );
    let offen = e.dienst.freigaben();
    assert_eq!(offen.len(), 1);
    assert_eq!(offen[0].question, FRAGE);
    assert!(offen[0].queries.len() >= 2);
    assert_eq!(offen[0].data_class, "PERSONAL");
    assert!(offen[0].expires_in_secs > 0);
    assert!(offen[0].queries.iter().all(|q| q.hint_class.as_deref() == Some("PERSONAL")));

    let id = offen[0].id.clone();
    let erste = offen[0].queries[0].id.clone();
    let erste_text = offen[0].queries[0].text.clone();
    let antwort = e
        .dienst
        .freigabe_entscheiden(&id, &[erste], FreigabeAktion::Senden)
        .await
        .unwrap();
    assert_eq!(antwort.status, "fertig");
    assert_eq!(*e.suche.anfragen.lock().unwrap(), vec![erste_text]);
    // Die Freigabe ist einmal verbrauchbar.
    let nochmal = e
        .dienst
        .freigabe_entscheiden(&id, &["a1".into()], FreigabeAktion::Senden)
        .await
        .unwrap_err();
    assert_eq!(nochmal.code, "recherche.not_found");
    // Das Ergebnis liegt im Projekt und nennt Quellen.
    let d = e.dienst.sicht(&antwort.recherche_id.unwrap()).await.unwrap();
    assert_eq!(d.project_id.as_deref(), Some(suite.as_str()));
    assert!(!d.sources.is_empty());
    assert!(d.read_aloud.contains("Quelle"), "{}", d.read_aloud);
}

#[tokio::test]
async fn ablehnen_und_nur_im_projekt_senden_nichts() {
    let e = env(true).await;
    let (_o, _m, suite) = verknuepft(&e, true).await;
    for aktion in [FreigabeAktion::Abbrechen, FreigabeAktion::NurProjekt] {
        let a = e
            .dienst
            .starten(FRAGE, "m1", Datenklasse::Personal, Some(suite.clone()))
            .await;
        let a = e
            .dienst
            .freigabe_entscheiden(&a.recherche_id.unwrap(), &["a1".into()], aktion)
            .await
            .unwrap();
        assert_eq!(a.status, "abgelehnt");
        assert!(e.dienst.freigaben().is_empty());
    }
    assert!(e.suche.anfragen.lock().unwrap().is_empty());
}

#[tokio::test]
async fn unbekannte_und_leere_auswahl_sendet_nichts() {
    let e = env(true).await;
    let (_o, _m, suite) = verknuepft(&e, true).await;
    let a = e
        .dienst
        .starten(FRAGE, "m1", Datenklasse::Personal, Some(suite))
        .await;
    let id = a.recherche_id.unwrap();
    let err = e
        .dienst
        .freigabe_entscheiden(&id, &["a9".into()], FreigabeAktion::Senden)
        .await
        .unwrap_err();
    assert_eq!(err.code, "recherche.invalid");
    assert!(e.suche.anfragen.lock().unwrap().is_empty());
    assert_eq!(e.dienst.freigaben().len(), 1, "der Halt bleibt bestehen");
}

#[tokio::test]
async fn entzogene_webfreigabe_nach_dem_halt_blockt_das_senden_und_haelt_den_halt() {
    let e = env(true).await;
    let (_o, _m, suite) = verknuepft(&e, true).await;
    let a = e
        .dienst
        .starten(FRAGE, "m1", Datenklasse::Personal, Some(suite.clone()))
        .await;
    let id = a.recherche_id.unwrap();
    e.projekte.web_freigabe(&suite, false).await.unwrap();
    let f = e
        .dienst
        .freigabe_entscheiden(&id, &["a1".into(), "a2".into()], FreigabeAktion::Senden)
        .await
        .unwrap();
    assert_eq!(f.status, "fehler");
    assert!(f.message.unwrap().contains("Halt bleibt bestehen"));
    assert!(e.suche.anfragen.lock().unwrap().is_empty());
    assert_eq!(e.dienst.freigaben().len(), 1);
}

#[tokio::test]
async fn halt_verfaellt_nach_der_frist_und_wird_protokolliert() {
    let e = env(true).await;
    let (_o, _m, suite) = verknuepft(&e, true).await;
    let a = e
        .dienst
        .starten(FRAGE, "m1", Datenklasse::Personal, Some(suite))
        .await;
    let id = a.recherche_id.unwrap();
    *e.zeit.lock().unwrap() = Duration::from_secs(24 * 3600);
    assert!(e.dienst.freigaben().is_empty());
    let err = e
        .dienst
        .freigabe_entscheiden(&id, &["a1".into()], FreigabeAktion::Senden)
        .await
        .unwrap_err();
    assert_eq!(err.code, "recherche.not_found");
    assert!(e.suche.anfragen.lock().unwrap().is_empty());
    assert!(alles_audit(&e).contains("freigabe_abgelaufen"));
}

#[tokio::test]
async fn suchadresse_aendert_nur_die_administration() {
    let e = env(false).await;
    let nutzer = person("solo", Role::Member);
    let err = e
        .dienst
        .konfigurieren(&nutzer, Some(ADRESSE))
        .await
        .unwrap_err();
    assert_eq!(err.code, "recherche.forbidden");
    assert!(e.dienst.hinweis().is_some(), "nichts wurde gesetzt");
    assert!(!e.dienst.status(&nutzer).can_configure);
    let s = e.dienst.konfigurieren(&e.p, Some(ADRESSE)).await.unwrap();
    assert!(s.available && s.can_configure);
    assert!(e.dienst.hinweis().is_none());
    // Entfernen schaltet die Recherche wieder ab.
    let s = e.dienst.konfigurieren(&e.p, None).await.unwrap();
    assert!(!s.available);
}

#[test]
fn suchadresse_pruefung_sperrt_metadaten_link_local_und_klartext_im_netz() {
    for gut in [
        "https://suche.example.test",
        "http://127.0.0.1:8888",
        "http://192.168.1.20:8080/suche",
    ] {
        assert!(pruefe_adresse(gut).is_ok(), "{gut}");
    }
    for schlecht in [
        "http://suche.example.test",
        "http://169.254.169.254/latest",
        "http://metadata.google.internal/",
        "https://nutzer:geheim@suche.example.test",
        "ftp://suche.example.test",
        "kein url",
        "",
    ] {
        assert!(pruefe_adresse(schlecht).is_err(), "{schlecht}");
    }
}

#[tokio::test]
async fn unbrauchbare_adresse_aendert_nichts_auch_fuer_admin() {
    let e = env(true).await;
    let err = e
        .dienst
        .konfigurieren(&e.p, Some("http://169.254.169.254/"))
        .await
        .unwrap_err();
    assert_eq!(err.code, "recherche.invalid");
    assert!(e.dienst.status(&e.p).available);
    assert_eq!(e.dienst.status(&e.p).searxng_url.as_deref(), Some(ADRESSE));
}

#[tokio::test]
async fn audit_enthaelt_nur_kennungen_und_zaehler_keine_inhalte() {
    let e = env(true).await;
    let (ordner, _m, suite) = verknuepft(&e, true).await;
    let _ = freigeben_und_laufen(&e, &suite).await;
    let a = e
        .dienst
        .starten(FRAGE, "m1", Datenklasse::Personal, Some(suite.clone()))
        .await;
    e.dienst
        .freigabe_entscheiden(&a.recherche_id.unwrap(), &[], FreigabeAktion::Abbrechen)
        .await
        .unwrap();
    e.projekte.ordner_loesen(&ordner).await.unwrap();
    let eintraege = e.audit.0.lock().unwrap().clone();
    assert!(eintraege.len() >= 4, "{eintraege:?}");
    let alles = format!("{eintraege:?}");
    for verboten in [
        "Waermeplanung",
        "waermeplanung",
        "2028",
        "kommunale",
        "Auftakt",
        ADRESSE,
        "suche.example.test",
        "127.0.0.1",
    ] {
        assert!(!alles.contains(verboten), "Inhalt im Audit: {verboten}");
    }
    // Zaehlercode statt Inhalt: Anfragen, Suchen, Abrufe, Quellen, Klasse, Web.
    assert!(
        eintraege
            .iter()
            .any(|r| r.aktion == "lauf" && r.code.starts_with('q') && r.status == "abgeschlossen"),
        "{eintraege:?}"
    );
}

#[tokio::test]
async fn fremde_recherchen_sind_fuer_andere_mandanten_unsichtbar() {
    let e = env(true).await;
    let (_o, _m, suite) = verknuepft(&e, true).await;
    let id = freigeben_und_laufen(&e, &suite).await;
    let fremd = person("anderer-mandant", Role::Admin);
    assert!(e.rstore.lesen(&fremd, &id).await.unwrap().is_none());
    assert!(!e.rstore.loeschen(&fremd, &id).await.unwrap());
    assert!(e.rstore.lesen(&e.p, &id).await.unwrap().is_some());
}

// ---------------------------------------------------------------- Modell ueber den Rueckruf (M1)

/// Ein Modell, das nur `antworten` kennt: Planung, Antwort und Satzpruefung entscheidet der Text der
/// Anweisung (wie ein echter Sprachdienst, der nichts von `satz_gestuetzt` weiss).
fn rueckruf(lokal: bool, unterstuetzt: bool) -> RueckrufModell {
    RueckrufModell::neu("rueckruf-test", lokal, move |anweisung: String, _eingabe: String| -> AufrufFut {
        Box::pin(async move {
            Ok(if anweisung.starts_with("Du planst") {
                r#"{"queries":["waermeplanung frist","waermeplanung gemeinden"]}"#.to_string()
            } else if anweisung.starts_with("Pruefe") {
                format!(r#"{{"supported":{unterstuetzt}}}"#)
            } else {
                serde_json::json!({
                    "sentences": [{"text": "Die kommunale Waermeplanung ist bis 2028 verpflichtend.", "citations": [1]}],
                    "follow_ups": ["Wer ist betroffen?", "Gibt es Foerdermittel?", "Was passiert bei Verzug?"]
                })
                .to_string()
            })
        })
    })
}

struct RueckrufModelle {
    lokal: bool,
    unterstuetzt: bool,
}
impl ModellQuelle for RueckrufModelle {
    fn modell(&self, _model_id: &str) -> Arc<dyn Modell> {
        Arc::new(rueckruf(self.lokal, self.unterstuetzt))
    }
}

#[tokio::test]
async fn rueckruf_modell_beurteilt_saetze_ueber_antworten() {
    let m = rueckruf(true, true);
    let u = m
        .satz_gestuetzt("Satz", "Auszug", Datenklasse::Personal)
        .await
        .unwrap()
        .expect("ein Urteil, nicht None");
    assert!(u.gestuetzt);
    assert!(u.tokens > 0);
    let nein = rueckruf(true, false)
        .satz_gestuetzt("Satz", "Auszug", Datenklasse::Personal)
        .await
        .unwrap()
        .unwrap();
    assert!(!nein.gestuetzt);
}

#[tokio::test]
async fn cloud_modell_bei_personal_wird_abgelehnt_auch_fuer_die_satzpruefung() {
    assert!(modell_erlaubt(false, Datenklasse::Internal));
    assert!(!modell_erlaubt(false, Datenklasse::Personal));
    let cloud = rueckruf(false, true);
    assert!(cloud.antworten("a", "e", Datenklasse::Personal).await.is_err());
    assert!(cloud.satz_gestuetzt("s", "a", Datenklasse::Personal).await.is_err());
    assert!(cloud.satz_gestuetzt("s", "a", Datenklasse::Internal).await.unwrap().is_some());
    assert!(rueckruf(true, true).antworten("a", "e", Datenklasse::Restricted).await.is_ok());
}

#[tokio::test]
async fn produktionspfad_mit_rueckruf_modell_liefert_gestuetzte_saetze() {
    let e = env_mit(true, Arc::new(RueckrufModelle { lokal: true, unterstuetzt: true })).await;
    let (_o, _m, suite) = verknuepft(&e, true).await;
    let id = freigeben_und_laufen(&e, &suite).await;
    let d = e.dienst.sicht(&id).await.unwrap();
    assert!(!d.answer.is_empty());
    assert!(d.answer.iter().all(|a| !a.unsupported), "{:?}", d.answer);
    assert!(!d.sources.is_empty());
    assert_eq!(d.model, "rueckruf-test");

    // Verneint die Pruefung, gilt der Satz als "ohne Quelle" (nicht als gestuetzt).
    let e = env_mit(true, Arc::new(RueckrufModelle { lokal: true, unterstuetzt: false })).await;
    let (_o, _m, suite) = verknuepft(&e, true).await;
    let id = freigeben_und_laufen(&e, &suite).await;
    let d = e.dienst.sicht(&id).await.unwrap();
    assert!(d.answer.iter().all(|a| a.unsupported || a.citations.is_empty()), "{:?}", d.answer);
}

#[tokio::test]
async fn cloud_modell_bei_personal_scheitert_im_lauf_ohne_suche() {
    let e = env_mit(true, Arc::new(RueckrufModelle { lokal: false, unterstuetzt: true })).await;
    let (_o, _m, suite) = verknuepft(&e, true).await;
    let a = e
        .dienst
        .starten(FRAGE, "m1", Datenklasse::Personal, Some(suite))
        .await;
    assert_eq!(a.status, "fehler", "{a:?}");
    assert!(e.suche.anfragen.lock().unwrap().is_empty());
    assert!(e.dienst.freigaben().is_empty());
}

// ---------------------------------------------------------------- S3, S4

#[tokio::test]
async fn start_prueft_serverseitig_dass_das_projekt_zur_besprechung_gehoert() {
    let e = env(true).await;
    let (_o, m, suite) = verknuepft(&e, true).await;
    // Zweite Besprechung in keinem Projekt, zweites Projekt mit Web.
    let fremd = ready_meeting(&e.store, "Fremde Besprechung", 1_750_000_100).id;
    let andere = e.projekte.anlegen("Anderes", Klasse::Personal, false).await.unwrap();
    e.projekte.web_freigabe(&andere.id, true).await.unwrap();

    let ok = e
        .dienst
        .starten_fuer(Some(&m), FRAGE, "m", Datenklasse::Personal, Some(suite.clone()))
        .await;
    assert_eq!(ok.status, "freigabe", "{ok:?}");
    for (besprechung, projekt) in [(&fremd, &suite), (&m, &andere.id)] {
        let a = e
            .dienst
            .starten_fuer(Some(besprechung), FRAGE, "m", Datenklasse::Personal, Some(projekt.clone()))
            .await;
        assert_eq!(a.status, "fehler");
        assert_eq!(a.code.as_deref(), Some("projekt_passt_nicht"));
    }
    let ohne = e
        .dienst
        .starten_fuer(Some(&m), FRAGE, "m", Datenklasse::Personal, None)
        .await;
    assert_eq!(ohne.code.as_deref(), Some("projekt_passt_nicht"));
    assert!(e.suche.anfragen.lock().unwrap().is_empty());
}

#[tokio::test]
async fn verwaiste_recherche_dokumente_werden_aufgeraeumt() {
    let e = env(true).await;
    let (_o, _m, suite) = verknuepft(&e, true).await;
    let id = freigeben_und_laufen(&e, &suite).await;
    // Das Projekt verschwindet, ohne dass die Kaskade das Dokument erreicht (Absturz mittendrin).
    e.projekte
        .verz_fuer_test()
        .projekt_loeschen(&e.p, &suite)
        .await
        .unwrap();
    assert!(e.dienst.sicht(&id).await.is_ok(), "Dokument ist noch da");
    assert_eq!(e.projekte.verwaiste_recherchen_raeumen().await.unwrap(), 1);
    assert!(e.dienst.sicht(&id).await.is_err());
    assert_eq!(e.projekte.verwaiste_recherchen_raeumen().await.unwrap(), 0);
}
