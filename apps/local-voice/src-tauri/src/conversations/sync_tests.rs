//! Tests der Anbindung des Gesprächsverlaufs an den Assistent-Sync (lva#388): Migration (E11),
//! Freigabe (E3.6), Freigabeliste (E10), Grabsteine (E3.2) und der Rundlauf zwischen zwei Geräten.
//! Hub ist der Referenz-Hub `MemHub`, einmal auch der Fake-HTTP-Hub über `PortalHub`. Jedes Gerät hat
//! wie in der App einen dauerhaften Abgleich (E3.8), hier im Arbeitsspeicher eines Testkontos.
//! Alle Daten sind synthetisch.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;

use wai_assistent::sync::{
    Abgleich, AltUnterhaltung, AuthGeraet, Beitrag, Einschraenkung, Hex128, HubFehler, HubSpeicher,
    Kopf, LeseFilter, MemHub, Modus, Sammlung, Schluesselbund, Schluesselpaket, Schreiben,
    Testkonto, Umschlag, Verzeichniseintrag, ZielTyp,
};

use super::*;
use crate::sync::portal_hub::tests::{konto_hub, netz, portal};

const KONTO: &str = "konto-test";

/// Gekoppeltes Testkonto; je Gerät ein dauerhafter Abgleich (wie der Zustandsspeicher der App).
struct Konto {
    k: Testkonto,
    geraete: RefCell<BTreeMap<String, Abgleich>>,
}

impl Konto {
    fn bund(&self) -> &Schluesselbund {
        self.k.bund()
    }

    /// Ruft `f` mit dem dauerhaften Abgleich des Geräts auf (beim ersten Mal angelegt).
    fn mit<R>(&self, g: &str, f: impl FnOnce(&mut Abgleich) -> R) -> R {
        let mut m = self.geraete.borrow_mut();
        let a = m.entry(g.to_owned()).or_insert_with(|| self.k.geraet(g));
        f(a)
    }
}

/// Konto mit diesen Geräten, das Verzeichnis liegt beim Hub.
fn konto_auf(h: &MemHub, geraete: &[&str]) -> Konto {
    Konto {
        k: Testkonto::mit_hub(geraete, h),
        geraete: RefCell::new(BTreeMap::new()),
    }
}

fn konto(geraete: &[&str]) -> (Konto, Arc<MemHub>) {
    let h = Arc::new(MemHub::new());
    (konto_auf(&h, geraete), h)
}

fn eintrag(role: &str, text: &str) -> NewEntry {
    NewEntry {
        id: None,
        role: role.to_string(),
        mode: if role == "user" { "spoken" } else { "typed" }.to_string(),
        text: text.to_string(),
        status: "complete".to_string(),
        created_at: None,
        author_id: None,
        triggered_by: None,
        model: if role == "assistant" {
            Some("modell-x".to_string())
        } else {
            None
        },
        attachments: Vec::new(),
        variants: Vec::new(),
        active_variant: None,
        sources: Vec::new(),
        untrusted: None,
        reply_to: None,
        ext: None,
    }
}

/// Gespräch mit abwechselnd Nutzer- und Assistenten-Beiträgen.
fn gespraech(s: &ConversationStore, titel: &str, n: usize) -> String {
    let c = s.create(Some(titel.to_string())).unwrap();
    for i in 0..n {
        let rolle = if i % 2 == 0 { "user" } else { "assistant" };
        s.append_entry(&c.id, eintrag(rolle, &format!("{titel} Beitrag {i}")))
            .unwrap();
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    c.id
}

fn texte(s: &ConversationStore, id: &str) -> Vec<String> {
    s.get(id)
        .unwrap()
        .entries
        .into_iter()
        .map(|e| e.text)
        .collect()
}

fn ids(s: &ConversationStore, id: &str) -> Vec<String> {
    s.get(id)
        .unwrap()
        .entries
        .into_iter()
        .map(|e| e.id)
        .collect()
}

fn lokale_ids(s: &ConversationStore) -> Vec<String> {
    s.list(true).unwrap().into_iter().map(|z| z.id).collect()
}

fn gespraech_mit_titel(s: &ConversationStore, titel: &str) -> Option<String> {
    s.list(true)
        .unwrap()
        .into_iter()
        .find(|z| z.title == titel)
        .map(|z| z.id)
}

fn lauf_ok<H: HubSpeicher + ?Sized>(
    s: &ConversationStore,
    h: &H,
    b: &Konto,
    g: &str,
) -> LaufBericht {
    b.mit(g, |a| lauf(s, h, a, KONTO, true))
        .unwrap_or_else(|e| panic!("Lauf scheitert: {e}"))
}

fn loeschmarken(h: &MemHub) -> usize {
    h.anzahl(Sammlung::Loeschmarke)
}

fn objekte_ohne_marken(h: &MemHub) -> usize {
    h.alle_umschlaege()
        .iter()
        .filter(|u| u.sammlung != Sammlung::Loeschmarke)
        .count()
}

/// Hub, der Aufrufe zählt und sich ausschalten lässt (Hubausfall).
struct Zaehlhub {
    inner: Arc<MemHub>,
    aufrufe: AtomicUsize,
    ausfall: AtomicBool,
}

impl Zaehlhub {
    fn neu(inner: &Arc<MemHub>) -> Self {
        Self {
            inner: inner.clone(),
            aufrufe: AtomicUsize::new(0),
            ausfall: AtomicBool::new(false),
        }
    }

    fn zaehlen(&self) -> Result<(), HubFehler> {
        self.aufrufe.fetch_add(1, Ordering::SeqCst);
        if self.ausfall.load(Ordering::SeqCst) {
            Err(HubFehler::Nichtverfuegbar)
        } else {
            Ok(())
        }
    }
}

impl HubSpeicher for Zaehlhub {
    fn schreiben(&self, a: &AuthGeraet, u: &Umschlag) -> Result<Schreiben, HubFehler> {
        self.zaehlen()?;
        self.inner.schreiben(a, u)
    }
    fn cas(
        &self,
        a: &AuthGeraet,
        s: Sammlung,
        i: &Hex128,
        e: Option<u64>,
        u: &Umschlag,
    ) -> Result<u64, HubFehler> {
        self.zaehlen()?;
        self.inner.cas(a, s, i, e, u)
    }
    fn holen(
        &self,
        a: &AuthGeraet,
        s: Sammlung,
        i: &Hex128,
    ) -> Result<Option<Umschlag>, HubFehler> {
        self.zaehlen()?;
        self.inner.holen(a, s, i)
    }
    fn lesen(&self, a: &AuthGeraet, f: &LeseFilter) -> Result<Vec<Umschlag>, HubFehler> {
        self.zaehlen()?;
        self.inner.lesen(a, f)
    }
    fn fassungen_lesen(
        &self,
        a: &AuthGeraet,
        s: Sammlung,
        i: &Hex128,
    ) -> Result<Vec<Umschlag>, HubFehler> {
        self.zaehlen()?;
        self.inner.fassungen_lesen(a, s, i)
    }
    fn verzeichnis_anhaengen(
        &self,
        a: &AuthGeraet,
        e: &Verzeichniseintrag,
    ) -> Result<(), HubFehler> {
        self.zaehlen()?;
        self.inner.verzeichnis_anhaengen(a, e)
    }
    fn verzeichnis_lesen(
        &self,
        a: &AuthGeraet,
        ab: u64,
    ) -> Result<Vec<Verzeichniseintrag>, HubFehler> {
        self.zaehlen()?;
        self.inner.verzeichnis_lesen(a, ab)
    }
    fn pakete_ablegen(&self, a: &AuthGeraet, p: &[Schluesselpaket]) -> Result<(), HubFehler> {
        self.zaehlen()?;
        self.inner.pakete_ablegen(a, p)
    }
    fn pakete_lesen(&self, a: &AuthGeraet) -> Result<Vec<Schluesselpaket>, HubFehler> {
        self.zaehlen()?;
        self.inner.pakete_lesen(a)
    }
}

// ---------------------------------------------------------------------------------------------
// Kennungen und Migration (E11)
// ---------------------------------------------------------------------------------------------

#[test]
fn ref_entspricht_der_migration() {
    let b = Testkonto::neu(&["dev-a"]);
    let mut a = b.geraet("dev-a");
    let alt = AltUnterhaltung {
        id: "conv_01ARZ3NDEKTSV4RRFFQ69G5FAV".into(),
        titel: None,
        bereich: None,
        geraet_lokal: false,
        modell_lokal: false,
        eintraege: Vec::new(),
    };
    a.migrieren("bestand-1", std::slice::from_ref(&alt));
    let erwartet = ref_aus_lokaler_id(b.bund(), "bestand-1", &alt.id);
    assert!(
        a.unterhaltung(&erwartet).is_some(),
        "die Ableitung des Hosts stimmt mit dem Kern überein"
    );
    assert_ne!(
        erwartet,
        ref_aus_lokaler_id(b.bund(), "bestand-2", &alt.id),
        "Datenbestände kollidieren nie"
    );
}

#[test]
fn datenbestand_id_ist_stabil_und_zufaellig() {
    let s = ConversationStore::open_in_memory().unwrap();
    let a = s.sync_datenbestand_id().unwrap();
    assert_eq!(a.len(), 32);
    assert_eq!(a, s.sync_datenbestand_id().unwrap());
    assert_ne!(
        a,
        ConversationStore::open_in_memory()
            .unwrap()
            .sync_datenbestand_id()
            .unwrap()
    );
}

#[test]
fn migration_ist_wiederholbar_und_dupliziert_nichts() {
    let (b, h) = konto(&["dev-a"]);
    let s = ConversationStore::open_in_memory().unwrap();
    gespraech(&s, "Eins", 4);
    gespraech(&s, "Zwei", 3);
    let r1 = lauf_ok(&s, &h, &b, "dev-a");
    assert_eq!((r1.beitraege_hoch, r1.koepfe_hoch), (7, 2));
    let objekte = objekte_ohne_marken(&h);
    let r2 = lauf_ok(&s, &h, &b, "dev-a");
    assert_eq!(r2, LaufBericht::default(), "zweiter Lauf: nichts zu tun");
    assert_eq!(objekte_ohne_marken(&h), objekte);
    assert_eq!(s.list(true).unwrap().len(), 2, "lokal nichts verdoppelt");
}

#[test]
fn neue_geraete_id_derselben_daten_ergibt_keine_dopplung_am_hub() {
    let (b, h) = konto(&["dev-a", "dev-a2"]);
    let s = ConversationStore::open_in_memory().unwrap();
    gespraech(&s, "Eins", 4);
    lauf_ok(&s, &h, &b, "dev-a");
    let objekte = objekte_ohne_marken(&h);
    let r = lauf_ok(&s, &h, &b, "dev-a2");
    assert_eq!(
        r.beitraege_hoch, 0,
        "dieselben Objekte sind schon da (Hub-Idempotenz)"
    );
    assert_eq!(objekte_ohne_marken(&h), objekte);
}

#[test]
fn migration_erfindet_keine_angaben() {
    let (b, h) = konto(&["dev-a", "dev-b"]);
    let s = ConversationStore::open_in_memory().unwrap();
    let id = gespraech(&s, "Eins", 2);
    lauf_ok(&s, &h, &b, "dev-a");
    // Gerät B liest: fehlendes Modell (Nutzerbeitrag) bleibt `unbekannt`, das gesetzte bleibt.
    let mut b_dev = b.k.geraet("dev-b");
    b_dev.herunterladen(&h).unwrap();
    let bs: Vec<Beitrag> = b_dev
        .unterhaltungen()
        .flat_map(|(_, u)| u.verlauf().beitraege().cloned())
        .collect();
    let nutzer = bs.iter().find(|x| x.autor == "user").unwrap();
    let ki = bs.iter().find(|x| x.autor == "assistant").unwrap();
    assert_eq!(nutzer.modell, "unbekannt");
    assert_eq!(ki.modell, "modell-x");
    assert_eq!(nutzer.modus, Modus::Spoken);
    assert_eq!(nutzer.lease_generation, 0);
    let _ = id;
}

// ---------------------------------------------------------------------------------------------
// E3.6: ohne Freigabe kein Netzaufruf
// ---------------------------------------------------------------------------------------------

#[test]
fn e3_6_ohne_freigabe_kein_hub_zugriff_und_kein_lesen_der_gespraeche() {
    let (b, h) = konto(&["dev-a"]);
    let z = Zaehlhub::neu(&h);
    let s = ConversationStore::open_in_memory().unwrap();
    gespraech(&s, "Eins", 2);
    let r = b.mit("dev-a", |a| lauf(&s, &z, a, KONTO, false));
    assert!(matches!(r, Err(LaufFehler::NichtFreigegeben)));
    assert_eq!(z.aufrufe.load(Ordering::SeqCst), 0, "kein Hub-Aufruf");
    assert_eq!(h.schreibversuche(), 0);
    assert_eq!(
        s.sync_zuordnung().unwrap().len(),
        0,
        "auch lokal wird nichts vermerkt"
    );
    assert_eq!(s.sync_meta("datenbestand_id").unwrap(), None);
}

#[test]
fn vor_dem_ersten_lauf_gibt_es_keine_grabsteine_fuer_nie_synchrone_gespraeche() {
    let (b, h) = konto(&["dev-a"]);
    let s = ConversationStore::open_in_memory().unwrap();
    let id = gespraech(&s, "Nie synchron", 2);
    s.delete(&id).unwrap();
    assert!(
        s.sync_grabsteine_offen(KONTO).unwrap().is_empty(),
        "nie für den Hub bestimmt: nichts vormerken"
    );
    lauf_ok(&s, &h, &b, "dev-a");
    assert_eq!(loeschmarken(&h), 0);
    assert_eq!(h.alle_umschlaege().len(), 0);
}

// ---------------------------------------------------------------------------------------------
// E10: confidential und geraet_lokal gehen nie an den Hub
// ---------------------------------------------------------------------------------------------

fn setze_klasse(s: &ConversationStore, id: &str, klasse: &str, sonder: bool) {
    s.sync_datenklasse_setzen(id, klasse, sonder).unwrap();
}

#[test]
fn e10_vertrauliche_und_lokale_gespraeche_werden_nie_hochgeladen() {
    let (b, h) = konto(&["dev-a"]);
    let s = ConversationStore::open_in_memory().unwrap();
    let normal = gespraech(&s, "Normal", 3);
    let lokal = gespraech(&s, "Nur lokal", 3);
    s.sync_geraet_lokal_setzen(&lokal, true).unwrap();
    let eingeschraenkt = gespraech(&s, "Streng", 3);
    setze_klasse(&s, &eingeschraenkt, "RESTRICTED", false);
    let vertraulich = gespraech(&s, "Vertraulich", 3);
    setze_klasse(&s, &vertraulich, "confidential", false);
    let unbekannt = gespraech(&s, "Unbekannte Klasse", 3);
    setze_klasse(&s, &unbekannt, "GEHEIMSTUFE", false);
    let sonder = gespraech(&s, "Sonderkategorie", 3);
    setze_klasse(&s, &sonder, "INTERNAL", true);
    gespraech_mit_klasse_personal(&s);

    let r = lauf_ok(&s, &h, &b, "dev-a");
    assert_eq!(
        r.beitraege_hoch,
        3 + 3,
        "nur das normale und das personenbezogene Gespräch"
    );
    assert_eq!(r.nur_lokal, 15, "fünf blockierte Gespräche à drei Beiträge");

    let datenbestand = s.sync_datenbestand_id().unwrap();
    for (id, name) in [
        (&lokal, "lokal"),
        (&eingeschraenkt, "restricted"),
        (&vertraulich, "confidential"),
        (&unbekannt, "unbekannt"),
        (&sonder, "sonderkategorie"),
    ] {
        let r = ref_aus_lokaler_id(b.bund(), &datenbestand, id);
        assert_eq!(
            h.anzahl_je_unterhaltung(&r),
            0,
            "{name}: nichts am Hub, auch kein Kopf"
        );
    }
    let r_normal = ref_aus_lokaler_id(b.bund(), &datenbestand, &normal);
    assert_eq!(
        h.anzahl_je_unterhaltung(&r_normal),
        4,
        "drei Beiträge und der Kopf"
    );
    // Der Hub-Inhalt enthält keinen Klartext der blockierten Gespräche.
    let draht = serde_json::to_string(&h.alle_umschlaege()).unwrap();
    for t in ["Nur lokal", "Streng", "Vertraulich", "Sonderkategorie"] {
        assert!(!draht.contains(t));
    }
}

fn gespraech_mit_klasse_personal(s: &ConversationStore) {
    let id = gespraech(s, "Personenbezogen", 3);
    setze_klasse(s, &id, "PERSONAL", false);
}

#[test]
fn e10_geraet_lokal_wird_auch_nach_spaeterem_setzen_nicht_mehr_hochgeladen_und_nichts_uebernommen()
{
    let (b, h) = konto(&["dev-a", "dev-b"]);
    let sa = ConversationStore::open_in_memory().unwrap();
    let sb = ConversationStore::open_in_memory().unwrap();
    let id = gespraech(&sa, "Wechsel", 2);
    lauf_ok(&sa, &h, &b, "dev-a");
    lauf_ok(&sb, &h, &b, "dev-b");
    let id_b = gespraech_mit_titel(&sb, "Wechsel").unwrap();
    // B markiert das übernommene Gespräch als „nur auf diesem Gerät“, A schreibt weiter.
    sb.sync_geraet_lokal_setzen(&id_b, true).unwrap();
    sa.append_entry(&id, eintrag("user", "Neu auf A")).unwrap();
    lauf_ok(&sa, &h, &b, "dev-a");
    let r = lauf_ok(&sb, &h, &b, "dev-b");
    assert_eq!(
        texte(&sb, &id_b).len(),
        2,
        "B übernimmt nichts mehr in ein lokales Gespräch"
    );
    assert_eq!(r.beitraege_neu, 0);
    sb.append_entry(&id_b, eintrag("user", "Nur auf B"))
        .unwrap();
    let objekte = objekte_ohne_marken(&h);
    lauf_ok(&sb, &h, &b, "dev-b");
    assert_eq!(
        objekte_ohne_marken(&h),
        objekte,
        "und lädt nichts mehr hoch"
    );
}

// ---------------------------------------------------------------------------------------------
// Rundlauf zwischen zwei Geräten
// ---------------------------------------------------------------------------------------------

#[test]
fn rundlauf_zwei_geraete_ohne_verlust_und_dopplung() {
    let (b, h) = konto(&["dev-a", "dev-b"]);
    let sa = ConversationStore::open_in_memory().unwrap();
    let sb = ConversationStore::open_in_memory().unwrap();
    let id_a = gespraech(&sa, "Rundlauf", 6);
    lauf_ok(&sa, &h, &b, "dev-a");
    let r = lauf_ok(&sb, &h, &b, "dev-b");
    assert_eq!((r.unterhaltungen_neu, r.beitraege_neu), (1, 6));
    let id_b = gespraech_mit_titel(&sb, "Rundlauf").expect("Gespräch kam an");
    assert_ne!(id_a, id_b, "das lokale Gespräch von B hat seine eigene ID");
    assert_eq!(
        ids(&sa, &id_a),
        ids(&sb, &id_b),
        "gleiche Beitrags-IDs, gleiche Reihenfolge"
    );
    assert_eq!(texte(&sa, &id_a), texte(&sb, &id_b));
    let e = &sb.get(&id_b).unwrap().entries;
    assert_eq!(e[1].role, "assistant");
    assert_eq!(e[1].model.as_deref(), Some("modell-x"));
    assert_eq!(e[0].mode, "spoken");

    // B antwortet, A holt es ab.
    sb.append_entry(&id_b, eintrag("user", "Weiter auf B"))
        .unwrap();
    lauf_ok(&sb, &h, &b, "dev-b");
    let r = lauf_ok(&sa, &h, &b, "dev-a");
    assert_eq!((r.beitraege_neu, r.unterhaltungen_neu), (1, 0));
    assert_eq!(texte(&sa, &id_a).last().unwrap(), "Weiter auf B");
    assert_eq!(texte(&sa, &id_a), texte(&sb, &id_b));

    // Stabil: weitere Läufe ändern nichts.
    let objekte = objekte_ohne_marken(&h);
    for _ in 0..2 {
        assert_eq!(lauf_ok(&sa, &h, &b, "dev-a"), LaufBericht::default());
        assert_eq!(lauf_ok(&sb, &h, &b, "dev-b"), LaufBericht::default());
    }
    assert_eq!(objekte_ohne_marken(&h), objekte);
    assert_eq!(sa.list(true).unwrap().len(), 1);
    assert_eq!(sb.list(true).unwrap().len(), 1);
}

#[test]
fn offline_fortsetzung_auf_beiden_geraeten_fuehrt_ohne_verlust_zusammen() {
    let (b, h) = konto(&["dev-a", "dev-b"]);
    let sa = ConversationStore::open_in_memory().unwrap();
    let sb = ConversationStore::open_in_memory().unwrap();
    let id_a = gespraech(&sa, "Offline", 2);
    lauf_ok(&sa, &h, &b, "dev-a");
    lauf_ok(&sb, &h, &b, "dev-b");
    let id_b = gespraech_mit_titel(&sb, "Offline").unwrap();
    sa.append_entry(&id_a, eintrag("user", "Nur A")).unwrap();
    sb.append_entry(&id_b, eintrag("user", "Nur B")).unwrap();
    for _ in 0..2 {
        lauf_ok(&sa, &h, &b, "dev-a");
        lauf_ok(&sb, &h, &b, "dev-b");
    }
    let (ta, tb) = (texte(&sa, &id_a), texte(&sb, &id_b));
    assert_eq!(ta.len(), 4);
    assert!(ta.contains(&"Nur A".to_string()) && ta.contains(&"Nur B".to_string()));
    let mut sa_sortiert = ta.clone();
    let mut sb_sortiert = tb.clone();
    sa_sortiert.sort();
    sb_sortiert.sort();
    assert_eq!(sa_sortiert, sb_sortiert, "beide haben dieselben Beiträge");
}

#[test]
fn fehlerhafte_beitraege_bleiben_lokal() {
    let (b, h) = konto(&["dev-a", "dev-b"]);
    let sa = ConversationStore::open_in_memory().unwrap();
    let sb = ConversationStore::open_in_memory().unwrap();
    let id = gespraech(&sa, "Fehler", 2);
    let mut e = eintrag("assistant", "Fehlermeldung statt Antwort");
    e.status = "error".to_string();
    sa.append_entry(&id, e).unwrap();
    lauf_ok(&sa, &h, &b, "dev-a");
    lauf_ok(&sb, &h, &b, "dev-b");
    let id_b = gespraech_mit_titel(&sb, "Fehler").unwrap();
    assert_eq!(texte(&sb, &id_b).len(), 2);
}

#[test]
fn titel_und_anheftung_wandern_und_aeltere_staende_ueberschreiben_nichts() {
    let (b, h) = konto(&["dev-a", "dev-b"]);
    let sa = ConversationStore::open_in_memory().unwrap();
    let sb = ConversationStore::open_in_memory().unwrap();
    let id_a = gespraech(&sa, "Alt", 2);
    lauf_ok(&sa, &h, &b, "dev-a");
    lauf_ok(&sb, &h, &b, "dev-b");
    let id_b = gespraech_mit_titel(&sb, "Alt").unwrap();

    sa.update(&id_a, Some("Neu benannt".into()), Some(true), None)
        .unwrap();
    lauf_ok(&sa, &h, &b, "dev-a");
    // B hat lokal nichts geändert: ein Lauf von B darf den Kopf von A nicht zurücksetzen.
    lauf_ok(&sb, &h, &b, "dev-b");
    let c = sb.get(&id_b).unwrap().conversation;
    assert_eq!((c.title.as_str(), c.pinned), ("Neu benannt", true));
    lauf_ok(&sa, &h, &b, "dev-a");
    let c = sa.get(&id_a).unwrap().conversation;
    assert_eq!(
        (c.title.as_str(), c.pinned),
        ("Neu benannt", true),
        "kein Zurückschwingen"
    );

    // Lokale Änderung auf B gewinnt gegen den (inzwischen gleichen) Hub-Stand und geht an A.
    sb.update(&id_b, Some("Von B".into()), None, None).unwrap();
    lauf_ok(&sb, &h, &b, "dev-b");
    lauf_ok(&sa, &h, &b, "dev-a");
    assert_eq!(sa.get(&id_a).unwrap().conversation.title, "Von B");
}

// ---------------------------------------------------------------------------------------------
// E3.2: Grabsteine
// ---------------------------------------------------------------------------------------------

#[test]
fn e3_2_geloeschtes_gespraech_setzt_grabstein_und_verschwindet_auf_dem_anderen_geraet() {
    let (b, h) = konto(&["dev-a", "dev-b"]);
    let sa = ConversationStore::open_in_memory().unwrap();
    let sb = ConversationStore::open_in_memory().unwrap();
    let id_a = gespraech(&sa, "Weg damit", 3);
    let bleibt = gespraech(&sa, "Bleibt", 2);
    lauf_ok(&sa, &h, &b, "dev-a");
    lauf_ok(&sb, &h, &b, "dev-b");
    let id_b = gespraech_mit_titel(&sb, "Weg damit").unwrap();

    sa.delete(&id_a).unwrap();
    assert_eq!(
        sa.sync_grabsteine_offen(KONTO).unwrap().len(),
        1,
        "vorgemerkt, noch nicht beim Hub"
    );
    let r = lauf_ok(&sa, &h, &b, "dev-a");
    assert_eq!(r.grabsteine_hoch, 1);
    assert!(sa.sync_grabsteine_offen(KONTO).unwrap().is_empty());
    assert_eq!(loeschmarken(&h), 1);

    // B hat offline noch weitergeschrieben: der Grabstein löscht, nichts wird wiederbelebt.
    sb.append_entry(&id_b, eintrag("user", "Offline auf B"))
        .unwrap();
    let r = lauf_ok(&sb, &h, &b, "dev-b");
    assert_eq!(r.unterhaltungen_geloescht, 1);
    assert!(gespraech_mit_titel(&sb, "Weg damit").is_none());
    assert!(gespraech_mit_titel(&sb, "Bleibt").is_some());

    // Spätere Läufe beleben nichts, auch nicht von A aus.
    for _ in 0..2 {
        lauf_ok(&sa, &h, &b, "dev-a");
        lauf_ok(&sb, &h, &b, "dev-b");
    }
    assert!(gespraech_mit_titel(&sa, "Weg damit").is_none());
    assert!(gespraech_mit_titel(&sb, "Weg damit").is_none());
    assert_eq!(
        h.anzahl_je_unterhaltung(&ref_aus_lokaler_id(
            b.bund(),
            &sa.sync_datenbestand_id().unwrap(),
            &id_a
        )),
        0,
        "am Hub gelöscht"
    );
    assert_eq!(sa.get(&bleibt).unwrap().entries.len(), 2);
    assert_eq!(loeschmarken(&h), 1);
}

#[test]
fn e3_2_grabstein_eines_gespraechs_von_einem_anderen_geraet_traegt_dessen_hub_kennung() {
    let (b, h) = konto(&["dev-a", "dev-b"]);
    let sa = ConversationStore::open_in_memory().unwrap();
    let sb = ConversationStore::open_in_memory().unwrap();
    gespraech(&sa, "Gemeinsam", 2);
    lauf_ok(&sa, &h, &b, "dev-a");
    lauf_ok(&sb, &h, &b, "dev-b");
    let id_b = gespraech_mit_titel(&sb, "Gemeinsam").unwrap();
    // B löscht das übernommene Gespräch: der Grabstein trägt die Hub-Kennung aus der Zuordnung.
    sb.delete(&id_b).unwrap();
    assert_eq!(lauf_ok(&sb, &h, &b, "dev-b").grabsteine_hoch, 1);
    let r = lauf_ok(&sa, &h, &b, "dev-a");
    assert_eq!(r.unterhaltungen_geloescht, 1);
    assert!(gespraech_mit_titel(&sa, "Gemeinsam").is_none());
}

#[test]
fn e3_2_grabstein_geht_bei_hubausfall_nicht_verloren() {
    let (b, h) = konto(&["dev-a"]);
    let s = ConversationStore::open_in_memory().unwrap();
    let id = gespraech(&s, "Ausfall", 2);
    lauf_ok(&s, &h, &b, "dev-a");
    s.delete(&id).unwrap();

    let z = Zaehlhub::neu(&h);
    z.ausfall.store(true, Ordering::SeqCst);
    let r = b.mit("dev-a", |a| lauf(&s, &z, a, KONTO, true));
    assert!(matches!(r, Err(LaufFehler::Sync(_))));
    assert_eq!(
        s.sync_grabsteine_offen(KONTO).unwrap().len(),
        1,
        "bleibt vorgemerkt"
    );
    assert_eq!(loeschmarken(&h), 0);

    z.ausfall.store(false, Ordering::SeqCst);
    let r = lauf_ok(&s, &z, &b, "dev-a");
    assert_eq!(r.grabsteine_hoch, 1);
    assert!(s.sync_grabsteine_offen(KONTO).unwrap().is_empty());
    assert_eq!(loeschmarken(&h), 1);
}

#[test]
fn e10_geloeschtes_lokales_gespraech_das_nie_beim_hub_war_erzeugt_keine_loeschmarke() {
    let (b, h) = konto(&["dev-a"]);
    let s = ConversationStore::open_in_memory().unwrap();
    let lokal = gespraech(&s, "Lokal", 2);
    s.sync_geraet_lokal_setzen(&lokal, true).unwrap();
    let vertraulich = gespraech(&s, "Vertraulich", 2);
    setze_klasse(&s, &vertraulich, "RESTRICTED", false);
    gespraech(&s, "Normal", 2);
    lauf_ok(&s, &h, &b, "dev-a");
    s.delete(&lokal).unwrap();
    s.delete(&vertraulich).unwrap();
    assert!(
        s.sync_grabsteine_offen(KONTO).unwrap().is_empty(),
        "nie synchron: kein Grabstein, kein Metadatenabfluss"
    );
    lauf_ok(&s, &h, &b, "dev-a");
    assert_eq!(loeschmarken(&h), 0);
}

#[test]
fn eingetroffener_beitrags_grabstein_loescht_den_beitrag_lokal() {
    let (b, h) = konto(&["dev-a", "dev-b", "dev-x"]);
    let sa = ConversationStore::open_in_memory().unwrap();
    let id_a = gespraech(&sa, "Beitragsloeschung", 3);
    lauf_ok(&sa, &h, &b, "dev-a");
    let ref_a = ref_aus_lokaler_id(b.bund(), &sa.sync_datenbestand_id().unwrap(), &id_a);
    let opfer = ids(&sa, &id_a)[1].clone();

    // Ein weiteres Gerät (reiner Kern) löscht einen Beitrag.
    let mut x = b.k.geraet("dev-x");
    x.herunterladen(&h).unwrap();
    x.beitrag_loeschen(&ref_a, &opfer).unwrap();
    x.hochladen(&h).unwrap();
    let oid = x.beitrag_objekt_id(&ref_a, &opfer);
    assert!(h.alle_umschlaege().iter().any(|u| u
        .ziel
        .as_ref()
        .is_some_and(|z| z.typ == ZielTyp::Objekt && z.wert == oid)));

    let r = lauf_ok(&sa, &h, &b, "dev-a");
    assert_eq!(r.beitraege_geloescht, 1);
    assert!(!ids(&sa, &id_a).contains(&opfer));
    assert_eq!(ids(&sa, &id_a).len(), 2);
    // Wiederholung belebt ihn nicht (der Beitrag ist am Hub weg, lokal weg).
    assert_eq!(lauf_ok(&sa, &h, &b, "dev-a"), LaufBericht::default());
}

// ---------------------------------------------------------------------------------------------
// Fremde Gespräche: Datenklasse übernommen und ehrlich behandelt
// ---------------------------------------------------------------------------------------------

#[test]
fn gespraech_mit_vertraulichen_beitraegen_von_aussen_wird_lokal_gesperrt() {
    let (b, h) = konto(&["dev-x", "dev-a"]);
    // Ein anderes Gerät (iOS) lädt ein Gespräch mit vertraulichem Beitrag hoch.
    let mut x = b.k.geraet("dev-x");
    let r = x
        .unterhaltung_anlegen(
            Kopf {
                titel: "Von iOS".into(),
                bereich: "x".into(),
                modellwunsch: "x".into(),
                angeheftet: false,
            },
            Einschraenkung::default(),
        )
        .unwrap();
    let id = "entry_01ARZ3NDEKTSV4RRFFQ69G5FAV";
    let mut beitrag = Beitrag::neu(
        id,
        None,
        0,
        "user",
        Modus::Typed,
        "Vertraulicher Vertragsinhalt",
    );
    beitrag.datenklasse = wai_assistent::sync::Datenklasse::Confidential;
    x.beitrag_anlegen(&r, beitrag).unwrap();
    x.hochladen(&h).unwrap();

    let s = ConversationStore::open_in_memory().unwrap();
    let bericht = lauf_ok(&s, &h, &b, "dev-a");
    assert_eq!((bericht.unterhaltungen_neu, bericht.beitraege_neu), (1, 1));
    let lokal = gespraech_mit_titel(&s, "Von iOS").unwrap();
    assert_eq!(
        s.get(&lokal).unwrap().conversation.scope.data_class,
        "RESTRICTED"
    );
    // Danach geht von diesem Gerät aus nichts mehr aus diesem Gespräch an den Hub.
    s.append_entry(&lokal, eintrag("user", "Antwort darauf"))
        .unwrap();
    let objekte = objekte_ohne_marken(&h);
    let bericht = lauf_ok(&s, &h, &b, "dev-a");
    assert_eq!(bericht.beitraege_hoch, 0);
    assert_eq!(objekte_ohne_marken(&h), objekte);
}

#[test]
fn beitraege_mit_fremder_kennung_werden_gezaehlt_aber_nicht_abgelegt() {
    let (b, h) = konto(&["dev-x", "dev-a"]);
    let mut x = b.k.geraet("dev-x");
    let r = x
        .unterhaltung_anlegen(
            Kopf {
                titel: "Fremde IDs".into(),
                bereich: "x".into(),
                modellwunsch: "x".into(),
                angeheftet: false,
            },
            Einschraenkung::default(),
        )
        .unwrap();
    x.beitrag_anlegen(
        &r,
        Beitrag::neu(
            "550e8400-e29b-41d4-a716-446655440000",
            None,
            0,
            "user",
            Modus::Typed,
            "iOS-Kennung",
        ),
    )
    .unwrap();
    x.beitrag_anlegen(
        &r,
        Beitrag::neu(
            "entry_01ARZ3NDEKTSV4RRFFQ69G5FAV",
            None,
            0,
            "Patrick",
            Modus::Spoken,
            "Gültig",
        ),
    )
    .unwrap();
    x.hochladen(&h).unwrap();
    let s = ConversationStore::open_in_memory().unwrap();
    let bericht = lauf_ok(&s, &h, &b, "dev-a");
    // Weder die fremde Kennung (iOS) noch der unbekannte Autor wird erfunden (E11): beide bleiben am Hub.
    assert_eq!((bericht.beitraege_neu, bericht.nicht_uebernommen), (0, 2));
    let id = gespraech_mit_titel(&s, "Fremde IDs").unwrap();
    assert!(s.get(&id).unwrap().entries.is_empty());
}

// ---------------------------------------------------------------------------------------------
// Persistenz und Schema
// ---------------------------------------------------------------------------------------------

#[test]
fn migration_des_speichers_behaelt_bestehende_daten_und_setzt_sichere_standardwerte() {
    let dir = tempfile::tempdir().unwrap();
    let pfad = dir.path().join("conversations.db");
    let altes_gespraech = {
        let s = ConversationStore::open_upgrading(&pfad, |conn| {
            conn.execute(
                "INSERT INTO conversations (id, schema_version, ts, trace_id, title, created_at, updated_at,
                    kind, scope, participants, context, pinned, archived)
                 VALUES ('conv_01ARZ3NDEKTSV4RRFFQ69G5FAV', '0.1.3', '2026-01-01T00:00:00.000Z',
                    '00000000000000000000000000000000', 'Alt', '2026-01-01T00:00:00.000Z',
                    '2026-01-01T00:00:00.000Z', 'voice',
                    '{\"data_class\":\"INTERNAL\",\"special_category\":false}', '[]', '[]', 0, 0)",
                [],
            )
            .unwrap();
        })
        .unwrap();
        s.list(true).unwrap()[0].id.clone()
    };
    let s = ConversationStore::open(&pfad).unwrap();
    let stand = s.sync_alle().unwrap();
    assert_eq!(stand.len(), 1);
    assert_eq!(stand[0].detail.conversation.id, altes_gespraech);
    assert!(
        !stand[0].geraet_lokal,
        "bestehende Gespräche sind nicht lokal gesperrt"
    );
    assert!(stand[0].sync_kopf.is_none());
    // Löschen eines Altgesprächs, das nie synchron war: kein Grabstein.
    s.delete(&altes_gespraech).unwrap();
    assert!(s.sync_grabsteine_offen(KONTO).unwrap().is_empty());
}

#[test]
fn grabstein_und_zuordnung_ueberleben_einen_neustart() {
    let dir = tempfile::tempdir().unwrap();
    let pfad = dir.path().join("conversations.db");
    let (b, h) = konto(&["dev-a"]);
    let id = {
        let s = ConversationStore::open(&pfad).unwrap();
        let id = gespraech(&s, "Neustart", 2);
        lauf_ok(&s, &h, &b, "dev-a");
        s.delete(&id).unwrap();
        id
    };
    let s = ConversationStore::open(&pfad).unwrap();
    assert_eq!(
        s.sync_grabsteine_offen(KONTO).unwrap()[0].conversation_id,
        id
    );
    assert_eq!(lauf_ok(&s, &h, &b, "dev-a").grabsteine_hoch, 1);
}

// ---------------------------------------------------------------------------------------------
// Über den Fake-HTTP-Hub (PortalHub): derselbe Rundlauf, Drahtformat inbegriffen
// ---------------------------------------------------------------------------------------------

#[test]
fn rundlauf_und_grabstein_ueber_den_portal_hub() {
    let (hub, pk) = konto_hub();
    let b = konto_auf(&hub, &["dev-a", "dev-b"]);
    let pa = portal(&hub, &pk, "dev-a");
    let pb = portal(&hub, &pk, "dev-b");
    let sa = ConversationStore::open_in_memory().unwrap();
    let sb = ConversationStore::open_in_memory().unwrap();
    let id_a = gespraech(&sa, "Über HTTP", 5);
    let privat = gespraech(&sa, "Privat", 2);
    sa.sync_geraet_lokal_setzen(&privat, true).unwrap();
    let r = lauf_ok(&sa, &pa, &b, "dev-a");
    assert_eq!((r.beitraege_hoch, r.koepfe_hoch, r.nur_lokal), (5, 1, 2));
    let r = lauf_ok(&sb, &pb, &b, "dev-b");
    assert_eq!((r.unterhaltungen_neu, r.beitraege_neu), (1, 5));
    let id_b = gespraech_mit_titel(&sb, "Über HTTP").unwrap();
    assert_eq!(texte(&sa, &id_a), texte(&sb, &id_b));
    assert!(gespraech_mit_titel(&sb, "Privat").is_none());

    sa.delete(&id_a).unwrap();
    lauf_ok(&sa, &pa, &b, "dev-a");
    let r = lauf_ok(&sb, &pb, &b, "dev-b");
    assert_eq!(r.unterhaltungen_geloescht, 1);
    assert_eq!(lokale_ids(&sb).len(), 0);
}

#[test]
fn hubausfall_ueber_den_portal_hub_laesst_lokale_daten_unberuehrt() {
    let (hub, pk) = konto_hub();
    let b = konto_auf(&hub, &["dev-a"]);
    let aus = portal(&hub, &pk, "dev-a");
    let s = ConversationStore::open_in_memory().unwrap();
    let id = gespraech(&s, "Offline", 3);
    // Netz weg: der Transport meldet Netzfehler, der Lauf bricht ab.
    netz(&aus, false);
    assert!(matches!(
        b.mit("dev-a", |a| lauf(&s, &aus, a, KONTO, true)),
        Err(LaufFehler::Sync(_))
    ));
    assert_eq!(texte(&s, &id).len(), 3);
    assert_eq!(aus.letzter_code().as_deref(), Some("netz"));
    netz(&aus, true);
    assert_eq!(lauf_ok(&s, &aus, &b, "dev-a").beitraege_hoch, 3);
}

// ---------------------------------------------------------------------------------------------
// Prüfrunde (lva#415): Löschfenster, Korrekturen, Konten, dauerhaftes Löschwissen
// ---------------------------------------------------------------------------------------------

/// Texte aller Beiträge, wie der Hub sie (für ein Gerät mit dem Schlüssel) enthält.
fn hub_texte(h: &MemHub, b: &Konto) -> Vec<String> {
    let mut a =
        b.k.geraet(b.k.verzeichnis().eintraege()[0].unterzeichner.as_str());
    a.herunterladen(h).unwrap();
    let mut t: Vec<String> = a
        .unterhaltungen()
        .flat_map(|(_, u)| {
            u.verlauf()
                .beitraege()
                .map(|x| x.text.clone())
                .collect::<Vec<_>>()
        })
        .collect();
    t.sort();
    t
}

#[test]
fn loeschen_im_fenster_nach_der_momentaufnahme_hinterlaesst_einen_grabstein_und_belebt_nichts() {
    let (b, h) = konto(&["dev-a", "dev-b"]);
    let s = ConversationStore::open_in_memory().unwrap();
    let id = gespraech(&s, "Fenster", 2);
    // Das Gespräch war noch nie beim Hub: genau das ist das gefährliche Fenster.
    let mut geloescht = false;
    let r = b
        .mit("dev-a", |a| {
            lauf_mit(&s, &*h, a, KONTO, true, &mut || {
                s.delete(&id).unwrap();
                geloescht = true;
            })
        })
        .unwrap();
    assert!(geloescht);
    assert!(lokale_ids(&s).is_empty(), "nichts wiederbelebt");
    assert_eq!(r.unterhaltungen_neu, 0);
    // Der Grabstein ist vorgemerkt (sync_jemals wurde mit der Momentaufnahme gesetzt) oder schon oben.
    let offen = s.sync_grabsteine_offen(KONTO).unwrap().len();
    let marken = loeschmarken(&h);
    assert!(offen + marken >= 1, "Löschwissen darf nicht verloren gehen");
    // Der Folgelauf schließt es ab, und ein zweites Gerät bekommt das Gespräch nie.
    lauf_ok(&s, &*h, &b, "dev-a");
    assert!(s.sync_grabsteine_offen(KONTO).unwrap().is_empty());
    assert_eq!(loeschmarken(&h), 1);
    let sb = ConversationStore::open_in_memory().unwrap();
    lauf_ok(&sb, &*h, &b, "dev-b");
    assert!(
        lokale_ids(&sb).is_empty(),
        "das Gespräch taucht nirgends wieder auf"
    );
    lauf_ok(&s, &*h, &b, "dev-a");
    assert!(lokale_ids(&s).is_empty());
}

#[test]
fn korrigierter_beitrag_entfernt_den_alten_text_beim_hub() {
    let (b, h) = konto(&["dev-a", "dev-b"]);
    let (sa, sb) = (
        ConversationStore::open_in_memory().unwrap(),
        ConversationStore::open_in_memory().unwrap(),
    );
    let id = gespraech(&sa, "Korrektur", 2);
    lauf_ok(&sa, &*h, &b, "dev-a");
    lauf_ok(&sb, &*h, &b, "dev-b");
    let alt = texte(&sa, &id)[1].clone();
    assert!(hub_texte(&h, &b).contains(&alt));

    let eintrag_id = ids(&sa, &id)[1].clone();
    sa.update_entry(&eintrag_id, "Korrigierter Text".into(), "complete".into())
        .unwrap();
    let r = lauf_ok(&sa, &*h, &b, "dev-a");
    assert!(r.beitraege_hoch >= 1 && r.grabsteine_hoch >= 1);
    let t = hub_texte(&h, &b);
    assert!(t.contains(&"Korrigierter Text".to_string()));
    assert!(!t.contains(&alt), "der alte Text steht nicht mehr beim Hub");

    // Lokal bleibt die ID, nichts wird doppelt; ein weiterer Lauf ist ruhig.
    assert_eq!(ids(&sa, &id)[1], eintrag_id);
    let r = lauf_ok(&sa, &*h, &b, "dev-a");
    assert_eq!(
        (r.beitraege_hoch, r.grabsteine_hoch, r.beitraege_neu),
        (0, 0, 0)
    );
    assert_eq!(texte(&sa, &id).len(), 2);

    // Das andere Gerät bekommt die Korrektur und verliert den alten Text.
    lauf_ok(&sb, &*h, &b, "dev-b");
    let idb = gespraech_mit_titel(&sb, "Korrektur").unwrap();
    let tb = texte(&sb, &idb);
    assert!(tb.contains(&"Korrigierter Text".to_string()), "{tb:?}");
    assert!(!tb.contains(&alt));
    assert_eq!(tb.len(), 2);

    // Zweite Korrektur: wieder nur der aktuelle Text beim Hub.
    sa.update_entry(&eintrag_id, "Noch einmal".into(), "complete".into())
        .unwrap();
    lauf_ok(&sa, &*h, &b, "dev-a");
    let t = hub_texte(&h, &b);
    assert!(t.contains(&"Noch einmal".to_string()));
    assert!(!t.contains(&"Korrigierter Text".to_string()) && !t.contains(&alt));
    assert_eq!(texte(&sa, &id).len(), 2);
}

#[test]
fn gespraeche_eines_anderen_kontos_gehen_nie_in_das_neue_konto_und_grabsteine_bleiben() {
    // Zwei Konten, je mit eigenem Hub und eigenem dauerhaftem Zustand desselben Geräts.
    let ((b1, h1), (b2, h2)) = (konto(&["dev-a"]), konto(&["dev-a"]));
    let s = ConversationStore::open_in_memory().unwrap();
    let altes = gespraech(&s, "Altes Konto", 2);
    let (weg, bleibt) = (gespraech(&s, "Wird gelöscht", 2), altes.clone());
    b1.mit("dev-a", |a| lauf(&s, &*h1, a, "konto-1", true))
        .unwrap();
    s.delete(&weg).unwrap();
    // Hub 1 ist gerade nicht erreichbar: der Grabstein bleibt offen und gehört zu Konto 1.
    assert_eq!(s.sync_grabsteine_offen("konto-1").unwrap().len(), 1);

    // Anderes Konto am anderen Hub: die Gespräche des alten Kontos werden nicht hochgeladen.
    let neu = gespraech(&s, "Neues Konto", 2);
    let r = b2
        .mit("dev-a", |a| lauf(&s, &*h2, a, "konto-2", true))
        .unwrap();
    assert_eq!(r.beitraege_hoch, 2, "nur das neue Gespräch geht hoch");
    assert_eq!(h2.anzahl(Sammlung::Unterhaltung), 1);
    assert!(hub_texte(&h2, &b2)
        .iter()
        .all(|t| t.starts_with("Neues Konto")));
    assert!(
        s.get(&bleibt).is_ok() && s.get(&neu).is_ok(),
        "nichts lokal gelöscht"
    );
    // Kein Grabstein des alten Kontos geht in das neue.
    assert_eq!(loeschmarken(&h2), 0);
    assert!(s.sync_grabsteine_offen("konto-2").unwrap().is_empty());
    assert_eq!(
        s.sync_grabsteine_offen("konto-1").unwrap().len(),
        1,
        "aufbewahrt, nie gelöscht"
    );
    // Zurück zu Konto 1: der Grabstein geht jetzt hoch.
    let r = b1
        .mit("dev-a", |a| lauf(&s, &*h1, a, "konto-1", true))
        .unwrap();
    assert_eq!(r.grabsteine_hoch, 1);
    assert_eq!(loeschmarken(&h1), 1);
}

#[test]
fn grabstein_mit_gespeicherter_fremder_kennung_wird_nur_mit_dieser_kennung_erledigt() {
    let (b, h) = konto(&["dev-a", "dev-b"]);
    let sa = ConversationStore::open_in_memory().unwrap();
    let sb = ConversationStore::open_in_memory().unwrap();
    gespraech(&sa, "Fremd", 2);
    lauf_ok(&sa, &*h, &b, "dev-a");
    lauf_ok(&sb, &*h, &b, "dev-b");
    let id_b = gespraech_mit_titel(&sb, "Fremd").unwrap();
    let gespeichert = sb
        .sync_zuordnung()
        .unwrap()
        .into_iter()
        .find(|(_, c)| *c == id_b)
        .unwrap()
        .0;
    sb.delete(&id_b).unwrap();
    // Ein Gerätezustand, der die gespeicherte Kennung nicht kennt (frisch gekoppelt, leerer Hub): nichts
    // wird unter einer abgeleiteten Kennung begraben, der Grabstein bleibt offen.
    let (anders, leer) = konto(&["dev-b"]);
    let r = lauf_ok(&sb, &*leer, &anders, "dev-b");
    assert_eq!(r.grabsteine_hoch, 0);
    assert_eq!(loeschmarken(&leer), 0);
    assert_eq!(sb.sync_grabsteine_offen(KONTO).unwrap().len(), 1);
    // Am richtigen Hub geht er mit der gespeicherten Kennung hoch und gilt dann als erledigt.
    let r = lauf_ok(&sb, &*h, &b, "dev-b");
    assert_eq!(r.grabsteine_hoch, 1);
    assert!(sb.sync_grabsteine_offen(KONTO).unwrap().is_empty());
    let tab = sb.sync_grabsteine_alle(KONTO).unwrap();
    assert_eq!(
        tab[0].unterhaltung_ref.as_deref(),
        Some(gespeichert.as_str())
    );
}

#[test]
fn loeschwissen_bleibt_dauerhaft_gegen_einen_aelteren_hubstand() {
    let (b, h) = konto(&["dev-a"]);
    let s = ConversationStore::open_in_memory().unwrap();
    let id = gespraech(&s, "Wiederauftauchen", 2);
    lauf_ok(&s, &*h, &b, "dev-a");
    let aelter = h.schnappschuss();
    s.delete(&id).unwrap();
    lauf_ok(&s, &*h, &b, "dev-a");
    assert_eq!(loeschmarken(&h), 1);
    assert!(
        s.sync_grabsteine_offen(KONTO).unwrap().is_empty(),
        "bestätigt"
    );
    assert_eq!(
        s.sync_grabsteine_alle(KONTO).unwrap().len(),
        1,
        "Löschwissen bleibt"
    );
    // Der Hub spielt einen älteren Stand zurück: er kennt das Gespräch noch und hat keine Löschmarke.
    h.zuruecksetzen(&aelter);
    if let Ok(r) = b.mit("dev-a", |a| lauf(&s, &*h, a, KONTO, true)) {
        assert_eq!(r.unterhaltungen_neu, 0);
    }
    assert!(lokale_ids(&s).is_empty(), "nichts kommt zurück");
}

#[test]
fn migration_legt_vor_dem_schemawechsel_eine_sicherung_an() {
    let dir = tempfile::tempdir().unwrap();
    let pfad = dir.path().join("conversations.db");
    ConversationStore::alte_datei_anlegen(&pfad, 3).unwrap();
    assert!(!dir.path().join("conversations.db.vor-v3.bak").exists());
    drop(ConversationStore::open(&pfad).unwrap());
    assert!(
        dir.path().join("conversations.db.vor-v3.bak").exists(),
        "Sicherung vor der Migration"
    );
}

#[test]
fn ausschalten_waehrend_eines_laufs_stoppt_jeden_weiteren_netzaufruf() {
    use crate::sync::portal_hub::tests::FakeHttp;
    use crate::sync::portal_hub::PortalHub;
    let (mh, pk) = konto_hub();
    let b = konto_auf(&mh, &["dev-a"]);
    let schranke = Arc::new(AtomicBool::new(false));
    let p = PortalHub::new(FakeHttp::neu(&mh, &pk, "dev-a"), "dev-a").mit_sperre(schranke.clone());
    p.zulassen(None).unwrap();
    let s = ConversationStore::open_in_memory().unwrap();
    gespraech(&s, "Läuft", 2);
    let stand = std::cell::Cell::new(0usize);
    // Der Nutzer schaltet mitten im Lauf aus: direkt nach der Momentaufnahme, vor dem Hochladen.
    let r = b.mit("dev-a", |a| {
        lauf_mit(&s, &p, a, KONTO, true, &mut || {
            schranke.store(true, Ordering::SeqCst);
            stand.set(p.transport.log.lock().unwrap().len());
        })
    });
    assert!(matches!(r, Err(LaufFehler::Sync(_))));
    assert_eq!(
        p.transport.log.lock().unwrap().len(),
        stand.get(),
        "nach dem Ausschalten kein einziger weiterer Aufruf"
    );
    assert_eq!(mh.anzahl(Sammlung::Beitrag), 0);
    assert_eq!(p.letzter_code().as_deref(), Some("abgebrochen"));
}
