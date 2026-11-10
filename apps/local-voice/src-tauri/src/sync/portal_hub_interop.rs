//! Interop des echten Rust-Clients gegen den echten PHP-Hub im wai-portal (ADR-0028 E3.8).
//!
//! Läuft nur auf Anforderung (`--ignored`) und nur mit `LVA_PORTAL_HUB_URL`, `LVA_PORTAL_HUB_EMAIL` und
//! `LVA_PORTAL_HUB_PASSWORT`; mit `LVA_PORTAL_HUB_DB` (Pfad der SQLite-Datei des Hubs) kommen die
//! Manipulationsfälle hinzu (ein Angreifer mit Datenbankzugang verändert Objekt und Schlüsselpaket).
//! Start mit frischem Hub: `apps/local-voice/scripts/portal-hub-interop.sh`.
//!
//! Der Weg ist der der App: `client::login` je Gerät (eigenes Token mit Scope `voice.sync`),
//! `HubClient` als Transport (`ClientTransport`), `PortalHub` als `HubSpeicher`, der Kern dahinter und
//! der verschlüsselte Zustandsblock (`DateiSpeicher`) als Zustandsspeicher. Nur synthetische Daten.

use wai_assistent::sync::{
    Abgleich, AuthGeraet, Beitrag, Einschraenkung, HubFehler, HubSpeicher, Kopf, Kopplung,
    Kopplungsangebot, Kopplungsantwort, LeseFilter, Modus, Sammlung, SyncFehler,
};
use zeroize::Zeroizing;

use super::*;
use crate::sync::assistent_sync::ClientTransport;
use crate::sync::assistent_zustand::{DateiSpeicher, KopplungsQr};
use crate::sync::client::{self, HubClient};

const PC: &str = "lva-pc";
const IPHONE: &str = "lva-iphone";
const TABLET: &str = "lva-tablet";

struct Umgebung {
    url: String,
    email: String,
    passwort: String,
    db: Option<String>,
}

fn umgebung() -> Option<Umgebung> {
    let wert = |n: &str| std::env::var(n).ok().filter(|v| !v.is_empty());
    Some(Umgebung {
        url: wert("LVA_PORTAL_HUB_URL")?,
        email: wert("LVA_PORTAL_HUB_EMAIL")?,
        passwort: wert("LVA_PORTAL_HUB_PASSWORT")?,
        db: wert("LVA_PORTAL_HUB_DB"),
    })
}

type Echt = PortalHub<ClientTransport>;

/// Anmeldung eines Geräts wie in der App: eigenes Token, an diese Kennung gebunden.
fn anmelden(u: &Umgebung, geraet: &str) -> Echt {
    let antwort =
        tauri::async_runtime::block_on(client::login(&u.url, &u.email, &u.passwort, geraet))
            .expect("Anmeldung am Portal");
    PortalHub::new(
        ClientTransport {
            client: HubClient::new(&u.url, &antwort.token),
        },
        geraet,
    )
}

fn speicher(dir: &std::path::Path, geraet: &str) -> Box<DateiSpeicher> {
    Box::new(DateiSpeicher::neu(dir, "interop-konto", geraet, &[7u8; 32]).unwrap())
}

fn kopf(titel: &str) -> Kopf {
    Kopf {
        titel: titel.into(),
        bereich: "Privat".into(),
        modellwunsch: "standard".into(),
        angeheftet: false,
    }
}

fn beitrag(id: &str, eltern: Option<&str>, text: &str) -> Beitrag {
    Beitrag::neu(id, eltern, 0, "user", Modus::Typed, text)
}

fn jetzt() -> u64 {
    u64::try_from(chrono::Utc::now().timestamp()).unwrap()
}

/// Kopplung über QR-Code (Angebot und Freigabecode) und Rückkanal (Antwort als JSON).
fn koppeln(pc: &mut Abgleich, pc_hub: &Echt, neu_hub: &Echt, dir: &std::path::Path) -> Abgleich {
    let geraet = neu_hub_kennung(neu_hub);
    let freigabe = pc_hub.freigeben(&geraet).expect("freigeben");
    let angebot = pc.kopplung_anbieten().unwrap();
    let qr = KopplungsQr {
        geraet_id: geraet.clone(),
        freigabe_code: Zeroizing::new(freigabe.code.clone()),
        angebot: Kopplungsangebot::aus_json(&serde_json::to_vec(&angebot).unwrap()).unwrap(),
    };
    let gelesen = KopplungsQr::aus_text(&qr.als_text().unwrap()).unwrap();
    neu_hub
        .zulassen(Some(&gelesen.freigabe_code))
        .expect("Zulassung per Freigabe");
    let kopplung = Kopplung::neu(&gelesen.geraet_id, gelesen.angebot).unwrap();
    assert!(
        kopplung
            .abschliessen(neu_hub, speicher(dir, &geraet))
            .unwrap()
            .is_none(),
        "vor der Aufnahme kein Abschluss"
    );
    let antwort =
        Kopplungsantwort::aus_json(&serde_json::to_vec(&kopplung.antwort().unwrap()).unwrap())
            .unwrap();
    pc.geraet_aufnehmen(pc_hub, &angebot, &antwort)
        .expect("aufnehmen");
    kopplung
        .abschliessen(neu_hub, speicher(dir, &geraet))
        .unwrap()
        .expect("aufgenommen")
}

fn neu_hub_kennung(h: &Echt) -> String {
    h.geraet_id.clone()
}

fn db(u: &Umgebung) -> Option<rusqlite::Connection> {
    u.db.as_ref().map(|p| {
        let c = rusqlite::Connection::open(p).expect("Hub-Datenbank");
        c.busy_timeout(std::time::Duration::from_secs(5)).unwrap();
        c
    })
}

/// Ändert das erste Zeichen einer Base64-Zeichenkette (gültige Form, falscher Inhalt).
fn kippen(s: &str) -> String {
    let erstes = if s.starts_with('A') { 'B' } else { 'A' };
    format!("{erstes}{}", &s[1..])
}

#[test]
#[ignore = "braucht den PHP-Hub: scripts/portal-hub-interop.sh"]
fn interop_mit_dem_php_hub() {
    let Some(u) = umgebung() else {
        eprintln!("LVA_PORTAL_HUB_URL/_EMAIL/_PASSWORT fehlen: übersprungen");
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let (pc_hub, ip_hub, tab_hub) = (anmelden(&u, PC), anmelden(&u, IPHONE), anmelden(&u, TABLET));
    let auth_pc = AuthGeraet::new(PC);

    // 1. Gründen: Erstgerät, Anker des Verzeichnisses, erstes Gespräch.
    assert_eq!(pc_hub.zulassen(None), Ok(1), "Erstgerät");
    assert!(
        pc_hub.verzeichnis_lesen(&auth_pc, 0).unwrap().is_empty(),
        "frischer Hub"
    );
    let mut pc = Abgleich::gruenden(PC, speicher(dir.path(), PC)).unwrap();
    pc.freigabe_einschalten().unwrap();
    let r = pc
        .unterhaltung_anlegen(kopf("Interop"), Einschraenkung::default())
        .unwrap();
    pc.beitrag_anlegen(&r, beitrag("i-1", None, "Erster Beitrag"))
        .unwrap();
    let hoch = pc.hochladen(&pc_hub).unwrap();
    assert_eq!((hoch.beitraege, hoch.koepfe), (1, 1));
    assert_eq!(pc_hub.verzeichnis_lesen(&auth_pc, 0).unwrap().len(), 1);
    // Ein zweites Gerät ohne Freigabe kommt nicht hinein.
    assert_eq!(ip_hub.zulassen(None), Err(HubFehler::NichtZugelassen));

    // 2. Kopplung mit Schlüsselpaket: iPhone und Tablet.
    let mut ip = koppeln(&mut pc, &pc_hub, &ip_hub, dir.path());
    let mut tab = koppeln(&mut pc, &pc_hub, &tab_hub, dir.path());
    assert_eq!(ip.konto(), pc.konto());
    assert_eq!(pc_hub.verzeichnis_lesen(&auth_pc, 0).unwrap().len(), 3);
    assert_eq!(ip.herunterladen(&ip_hub).unwrap().beitraege_neu, 1);
    assert_eq!(tab.herunterladen(&tab_hub).unwrap().beitraege_neu, 1);

    // 3. Schreiben, Lesen und Vergleichsschreiben (Kopf), Fassungen.
    ip.kopf_aendern(&r, kopf("Interop geändert")).unwrap();
    ip.beitrag_anlegen(&r, beitrag("i-2", Some("i-1"), "Vom iPhone"))
        .unwrap();
    ip.hochladen(&ip_hub).unwrap();
    let b = pc.herunterladen(&pc_hub).unwrap();
    assert_eq!(b.beitraege_neu, 1);
    assert_eq!(
        pc.unterhaltung(&r).unwrap().kopf().titel,
        "Interop geändert"
    );
    let koepfe = pc_hub
        .lesen(
            &auth_pc,
            &LeseFilter {
                nach_revision: 0,
                unterhaltung_ref: Some(r.clone()),
                sammlung: Some(Sammlung::Unterhaltung),
            },
        )
        .unwrap();
    assert_eq!(koepfe.len(), 1);
    let fassungen = pc_hub
        .fassungen_lesen(&auth_pc, Sammlung::Unterhaltung, &koepfe[0].objekt_id)
        .unwrap();
    assert!(fassungen.len() >= 2, "jede Fassung des Kopfs bleibt");
    assert!(fassungen
        .windows(2)
        .all(|w| w[0].revision.unwrap() < w[1].revision.unwrap()));
    // Erstschreiben ist idempotent: noch einmal hochladen ändert nichts.
    assert_eq!(pc.hochladen(&pc_hub).unwrap().beitraege, 0);

    // 4. Lease (Vergleichsschreiben auf `sitzung`) und Übernahme.
    let halt = pc.lease_uebernehmen(&pc_hub, &r, jetzt(), None).unwrap();
    assert_eq!(halt.generation, 1);
    let gelesen = ip.lease_lesen(&ip_hub, &r).unwrap().unwrap();
    assert_eq!(gelesen.lease.inhaber, PC);
    let neu = ip
        .lease_uebernehmen(&ip_hub, &r, jetzt(), Some(&gelesen))
        .unwrap();
    assert_eq!(neu.generation, 2);
    assert!(matches!(
        pc.lease_verlaengern(&pc_hub, &r, jetzt(), &halt),
        Err(wai_assistent::sync::LeaseFehler::Verloren { .. })
    ));

    let verbindung = db(&u);

    // 5. Manipuliertes Objekt in der Hub-Datenbank: das iPhone verwirft es.
    if let Some(c) = &verbindung {
        let vorher: i64 = c
            .query_row(
                "SELECT COALESCE(MAX(revision), 0) FROM assistent_objekte",
                [],
                |z| z.get(0),
            )
            .unwrap();
        pc.beitrag_anlegen(&r, beitrag("i-3", Some("i-2"), "Wird verfälscht"))
            .unwrap();
        pc.hochladen(&pc_hub).unwrap();
        // Alle neuen Beiträge dieses Hochladens (der Beitrag und ein mitgeschicktes Ereignis).
        let neu: Vec<(i64, String)> = c
            .prepare(
                "SELECT id, ciphertext FROM assistent_objekte \
                 WHERE sammlung = 'beitrag' AND revision > ?1",
            )
            .unwrap()
            .query_map([vorher], |z| Ok((z.get(0)?, z.get(1)?)))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert!(!neu.is_empty());
        for (id, ct) in &neu {
            c.execute(
                "UPDATE assistent_objekte SET ciphertext = ?1 WHERE id = ?2",
                rusqlite::params![kippen(ct), id],
            )
            .unwrap();
        }
        let b = ip.herunterladen(&ip_hub).unwrap();
        assert_eq!(b.beitraege_neu, 0, "verfälschter Beitrag kommt nicht an");
        assert!(b.verworfen >= 1, "{b:?}");
        assert!(ip
            .unterhaltung(&r)
            .unwrap()
            .verlauf()
            .beitrag("i-3")
            .is_none());
    }

    // 6. Widerruf über das Verzeichnis; der Hub entzieht dem Tablet das Token.
    assert_eq!(pc.widerrufen(&pc_hub, TABLET), Ok(2));
    assert_eq!(pc.epoche(), 2);
    assert_eq!(pc.empfaenger(), vec![IPHONE.to_owned()]);
    assert!(tab_hub
        .verzeichnis_lesen(&AuthGeraet::new(TABLET), 0)
        .is_err());
    assert!(matches!(
        tab.herunterladen(&tab_hub),
        Err(SyncFehler::Hub(_))
    ));
    // Ein Selbstwiderruf scheitert schon im Kern.
    assert!(ip.widerrufen(&ip_hub, IPHONE).is_err());

    // 7. Manipuliertes Schlüsselpaket: abgewiesen; das unveränderte gilt.
    if let Some(c) = &verbindung {
        let (id, roh): (i64, String) = c
            .query_row(
                "SELECT id, paket FROM assistent_pakete WHERE empfaenger = ?1 AND epoche = 2",
                [IPHONE],
                |z| Ok((z.get(0)?, z.get(1)?)),
            )
            .unwrap();
        let mut p: serde_json::Value = serde_json::from_str(&roh).unwrap();
        p["chiffrat"] = serde_json::json!(kippen(p["chiffrat"].as_str().unwrap()));
        c.execute(
            "UPDATE assistent_pakete SET paket = ?1 WHERE id = ?2",
            rusqlite::params![p.to_string(), id],
        )
        .unwrap();
        let b = ip.herunterladen(&ip_hub).unwrap();
        assert_eq!(b.schluessel_neu, 0, "{b:?}");
        assert!(b.pakete_abgewiesen >= 1, "{b:?}");
        assert!(ip.bund().schluessel(2).is_none());
        c.execute(
            "UPDATE assistent_pakete SET paket = ?1 WHERE id = ?2",
            rusqlite::params![roh, id],
        )
        .unwrap();
    }
    let b = ip.herunterladen(&ip_hub).unwrap();
    assert_eq!(b.schluessel_neu, 1, "{b:?}");
    assert_eq!(ip.epoche(), 2);
    assert_eq!(
        ip.bund().schluessel(2).unwrap().bytes(),
        pc.bund().schluessel(2).unwrap().bytes()
    );

    // 8. Nach dem Widerruf: Schreiben in Epoche 2 und Lesen auf dem iPhone.
    pc.beitrag_anlegen(&r, beitrag("i-4", Some("i-2"), "Nach dem Widerruf"))
        .unwrap();
    pc.hochladen(&pc_hub).unwrap();
    ip.herunterladen(&ip_hub).unwrap();
    assert!(ip
        .unterhaltung(&r)
        .unwrap()
        .verlauf()
        .beitrag("i-4")
        .is_some());

    // 9. Neustart: der Zustand kommt aus dem verschlüsselten Block zurück.
    drop(pc);
    let pc = Abgleich::wiederherstellen(speicher(dir.path(), PC))
        .unwrap()
        .expect("gesicherter Zustand");
    assert_eq!(pc.epoche(), 2);
    assert!(pc
        .unterhaltung(&r)
        .unwrap()
        .verlauf()
        .beitrag("i-4")
        .is_some());
}
