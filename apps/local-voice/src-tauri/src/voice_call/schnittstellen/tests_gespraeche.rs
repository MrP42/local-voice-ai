use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::{Arc, Barrier};
use std::thread;

use super::anbindung::{Abbruch, Schnappschuss};
use super::gespraeche::Gespraeche;
use super::kodierung::{lies_json, Wert};

fn eintrag(schreibend: bool) -> Wert {
    let mut e = lies_json(
        r#"{
        "id":"probe", "name":"Probe", "beschreibung":"", "art":"http",
        "zugriff":"schreibend", "abfluss":"netz", "zeitlimit_ms":15000,
        "http":{"methode":"POST", "adresse":"https://api.example.org/send",
        "netz":"oeffentlich", "antwort_max_bytes":20480,
        "parameter":[{"name":"text", "beschreibung":"", "typ":"text", "ort":"koerper",
        "pflicht":true, "max_zeichen":500}], "kopfzeilen":[], "geheimnis":null}
    }"#,
    )
    .unwrap();
    if !schreibend {
        e.setzen("zugriff", "lesend");
    }
    assert!(super::verzeichnis::pruefe_verzeichnis(&super::objekt([
        ("schema", "lva.schnittstellen/1".into()),
        ("eintraege", Wert::Liste(vec![e.clone()])),
    ]))
    .is_ok());
    e
}

fn snapshot(schreibend: bool) -> Schnappschuss {
    Schnappschuss::neu(
        &eintrag(schreibend),
        &lies_json(r#"{"text":"einmal"}"#).unwrap(),
        None,
    )
    .unwrap()
}

#[test]
fn runden_desselben_gespraechs_teilen_die_sperre() {
    // rot, wenn jede Antwort eine eigene Gespraechssperre erhaelt oder Drop sie nicht freigibt.
    let gespraeche = Gespraeche::neu();
    let s = snapshot(false);
    let abbruch = Abbruch::neu();
    let mut erste = gespraeche.beginne("c1", "a1").unwrap();
    let mut zweite = gespraeche.beginne("c1", "a2").unwrap();
    let lauf = erste.naechster_aufruf(&s, &abbruch).unwrap();
    assert!(erste.laeuft());
    assert!(!zweite.laeuft());
    assert_eq!(
        zweite.naechster_aufruf(&s, &abbruch).err(),
        Some("abgelehnt")
    );
    drop(lauf);
    assert!(!erste.laeuft());
    let lauf = zweite.naechster_aufruf(&s, &abbruch).unwrap();
    assert!(zweite.laeuft());
    assert_eq!(
        erste.naechster_aufruf(&s, &abbruch).err(),
        Some("abgelehnt")
    );
    drop(lauf);
    assert!(!zweite.laeuft());
}

#[test]
fn andere_gespraeche_haben_unabhaengige_sperren() {
    // rot, wenn verschiedene Gespraeche eine globale Sperre teilen.
    let gespraeche = Gespraeche::neu();
    let s = snapshot(false);
    let abbruch = Abbruch::neu();
    let mut erste = gespraeche.beginne("c1", "a1").unwrap();
    let mut zweite = gespraeche.beginne("c2", "a1").unwrap();
    let erster_lauf = erste.naechster_aufruf(&s, &abbruch).unwrap();
    let zweiter_lauf = zweite.naechster_aufruf(&s, &abbruch).unwrap();
    assert!(erste.laeuft());
    assert!(zweite.laeuft());
    drop(erster_lauf);
    assert!(zweite.laeuft());
    drop(zweiter_lauf);
}

#[test]
fn fuenf_aufrufe_je_antwort_ohne_verbrauch_durch_ablehnung() {
    // rot, wenn die Grenze fehlt oder ein wegen Belegung abgelehnter Aufruf mitzaehlt.
    let gespraeche = Gespraeche::neu();
    let s = snapshot(false);
    let abbruch = Abbruch::neu();
    let mut runde = gespraeche.beginne("c1", "a1").unwrap();
    let lauf = runde.naechster_aufruf(&s, &abbruch).unwrap();
    assert_eq!(
        runde.naechster_aufruf(&s, &abbruch).err(),
        Some("abgelehnt")
    );
    drop(lauf);
    for _ in 0..4 {
        drop(runde.naechster_aufruf(&s, &abbruch).unwrap());
    }
    for _ in 0..2 {
        assert_eq!(
            runde.naechster_aufruf(&s, &abbruch).err(),
            Some("abgelehnt")
        );
        assert!(!runde.laeuft());
    }
    let mut andere = gespraeche.beginne("c1", "a2").unwrap();
    drop(andere.naechster_aufruf(&s, &abbruch).unwrap());
}

#[test]
fn idempotentes_beginne_teilt_zaehler_und_laufzustand() {
    // rot, wenn beginne fuer dieselben IDs einen neuen Zaehler oder Laufzustand liefert.
    let gespraeche = Gespraeche::neu();
    let s = snapshot(false);
    let abbruch = Abbruch::neu();
    let mut erste = gespraeche.beginne("c1", "a1").unwrap();
    let lauf = erste.naechster_aufruf(&s, &abbruch).unwrap();
    let mut zweite = gespraeche.beginne("c1", "a1").unwrap();
    assert!(zweite.laeuft());
    assert_eq!(
        zweite.naechster_aufruf(&s, &abbruch).err(),
        Some("abgelehnt")
    );
    drop(lauf);
    assert!(!zweite.laeuft());
    for _ in 0..4 {
        drop(zweite.naechster_aufruf(&s, &abbruch).unwrap());
    }
    assert_eq!(
        erste.naechster_aufruf(&s, &abbruch).err(),
        Some("abgelehnt")
    );
    let mut dritte = gespraeche.beginne("c1", "a1").unwrap();
    assert_eq!(
        dritte.naechster_aufruf(&s, &abbruch).err(),
        Some("abgelehnt")
    );
}

#[test]
fn neue_antwort_startet_mit_eigenem_zaehler() {
    // rot, wenn neue Antworten den Zaehler der vorherigen Antwort uebernehmen oder zuruecksetzen.
    let gespraeche = Gespraeche::neu();
    let s = snapshot(false);
    let abbruch = Abbruch::neu();
    let mut erste = gespraeche.beginne("c1", "a1").unwrap();
    for _ in 0..5 {
        drop(erste.naechster_aufruf(&s, &abbruch).unwrap());
    }
    let mut zweite = gespraeche.beginne("c1", "a2").unwrap();
    for _ in 0..5 {
        drop(zweite.naechster_aufruf(&s, &abbruch).unwrap());
    }
    assert_eq!(
        erste.naechster_aufruf(&s, &abbruch).err(),
        Some("abgelehnt")
    );
    assert_eq!(
        zweite.naechster_aufruf(&s, &abbruch).err(),
        Some("abgelehnt")
    );
}

#[test]
fn ende_wartet_auf_drop_und_erlaubt_danach_neustart() {
    // rot, wenn ende laufende Eintraege entfernt, abbricht oder den Neustart verhindert.
    let gespraeche = Gespraeche::neu();
    let s = snapshot(false);
    let abbruch = Abbruch::neu();
    let mut runde = gespraeche.beginne("c1", "a1").unwrap();
    for _ in 0..4 {
        drop(runde.naechster_aufruf(&s, &abbruch).unwrap());
    }
    let lauf = runde.naechster_aufruf(&s, &abbruch).unwrap();
    gespraeche.ende("c1");
    gespraeche.aufraeumen();
    assert_eq!(gespraeche.anzahl(), 1);
    for antwort in ["a1", "a2"] {
        assert_eq!(
            gespraeche.beginne("c1", antwort).err(),
            Some("gespraech_beendet")
        );
    }
    assert!(!abbruch.ausgeloest());
    assert!(!lauf.abbruch().ausgeloest());
    assert!(runde.laeuft());
    drop(lauf);
    gespraeche.aufraeumen();
    assert_eq!(gespraeche.anzahl(), 0);
    let mut neu = gespraeche.beginne("c1", "a1").unwrap();
    for _ in 0..5 {
        drop(neu.naechster_aufruf(&s, &abbruch).unwrap());
    }
    assert_eq!(neu.naechster_aufruf(&s, &abbruch).err(), Some("abgelehnt"));
    assert_eq!(gespraeche.anzahl(), 1);
}

#[test]
fn fremde_ids_setzen_weder_zaehler_noch_sperre_zurueck() {
    // rot, wenn fremde IDs beim Beginnen oder Beenden einen bestehenden Zustand zuruecksetzen.
    let gespraeche = Gespraeche::neu();
    let s = snapshot(false);
    let abbruch = Abbruch::neu();
    let mut runde = gespraeche.beginne("c1", "a1").unwrap();
    let lauf = runde.naechster_aufruf(&s, &abbruch).unwrap();
    let mut fremde = gespraeche.beginne("c2", "a1").unwrap();
    drop(fremde.naechster_aufruf(&s, &abbruch).unwrap());
    gespraeche.ende("c2");
    gespraeche.ende("unbekannt");
    gespraeche.aufraeumen();
    let mut andere = gespraeche.beginne("c1", "a2").unwrap();
    assert_eq!(
        andere.naechster_aufruf(&s, &abbruch).err(),
        Some("abgelehnt")
    );
    assert_eq!(
        runde.naechster_aufruf(&s, &abbruch).err(),
        Some("abgelehnt")
    );
    drop(lauf);
    for _ in 0..4 {
        drop(runde.naechster_aufruf(&s, &abbruch).unwrap());
    }
    assert_eq!(
        runde.naechster_aufruf(&s, &abbruch).err(),
        Some("abgelehnt")
    );
    let mut wieder = gespraeche.beginne("c1", "a1").unwrap();
    assert_eq!(
        wieder.naechster_aufruf(&s, &abbruch).err(),
        Some("abgelehnt")
    );
}

#[test]
fn panik_im_lauf_gibt_die_sperre_frei() {
    // rot, wenn beim Abwickeln einer Panik die Gespraechssperre oder der Laufzustand bleibt.
    let gespraeche = Gespraeche::neu();
    let s = snapshot(false);
    let abbruch = Abbruch::neu();
    let mut erste = gespraeche.beginne("c1", "a1").unwrap();
    let ergebnis = catch_unwind(AssertUnwindSafe(|| {
        let _lauf = erste.naechster_aufruf(&s, &abbruch).unwrap();
        assert!(erste.laeuft());
        panic!("Testpanik im Lauf");
    }));
    assert!(ergebnis.is_err());
    assert!(!erste.laeuft());
    let mut zweite = gespraeche.beginne("c1", "a2").unwrap();
    drop(zweite.naechster_aufruf(&s, &abbruch).unwrap());
    drop(erste.naechster_aufruf(&s, &abbruch).unwrap());
}

#[test]
fn achte_antwort_bleibt_gemerkt_neunte_verdraengt_die_aelteste_freie() {
    // rot, wenn weniger als acht Antworten bleiben oder eine andere als die aelteste weicht.
    let gespraeche = Gespraeche::neu();
    let s = snapshot(false);
    let abbruch = Abbruch::neu();
    for nummer in 0..8 {
        let mut runde = gespraeche.beginne("c1", &format!("a{nummer}")).unwrap();
        for _ in 0..5 {
            drop(runde.naechster_aufruf(&s, &abbruch).unwrap());
        }
    }
    for nummer in 0..8 {
        let mut runde = gespraeche.beginne("c1", &format!("a{nummer}")).unwrap();
        assert_eq!(
            runde.naechster_aufruf(&s, &abbruch).err(),
            Some("abgelehnt")
        );
    }
    assert!(gespraeche.beginne("c1", "a8").is_ok());
    for nummer in 1..8 {
        let mut runde = gespraeche.beginne("c1", &format!("a{nummer}")).unwrap();
        assert_eq!(
            runde.naechster_aufruf(&s, &abbruch).err(),
            Some("abgelehnt")
        );
    }
    let mut neu = gespraeche.beginne("c1", "a0").unwrap();
    for _ in 0..5 {
        drop(neu.naechster_aufruf(&s, &abbruch).unwrap());
    }
}

#[test]
fn laufende_aelteste_antwort_wird_nicht_verdraengt() {
    // rot, wenn eine laufende Antwort verdraengt oder keine freie Antwort uebersprungen wird.
    let gespraeche = Gespraeche::neu();
    let s = snapshot(false);
    let abbruch = Abbruch::neu();
    let mut erste = gespraeche.beginne("c1", "a0").unwrap();
    for nummer in 1..8 {
        let mut runde = gespraeche.beginne("c1", &format!("a{nummer}")).unwrap();
        for _ in 0..5 {
            drop(runde.naechster_aufruf(&s, &abbruch).unwrap());
        }
    }
    let lauf = erste.naechster_aufruf(&s, &abbruch).unwrap();
    assert!(gespraeche.beginne("c1", "a8").is_ok());
    let mut gemerkt = gespraeche.beginne("c1", "a0").unwrap();
    assert!(gemerkt.laeuft());
    drop(lauf);
    for nummer in 2..8 {
        let mut runde = gespraeche.beginne("c1", &format!("a{nummer}")).unwrap();
        assert_eq!(
            runde.naechster_aufruf(&s, &abbruch).err(),
            Some("abgelehnt")
        );
    }
    for _ in 0..4 {
        drop(gemerkt.naechster_aufruf(&s, &abbruch).unwrap());
    }
    assert_eq!(
        erste.naechster_aufruf(&s, &abbruch).err(),
        Some("abgelehnt")
    );
    let mut neu = gespraeche.beginne("c1", "a1").unwrap();
    for _ in 0..5 {
        drop(neu.naechster_aufruf(&s, &abbruch).unwrap());
    }
}

#[test]
fn ungueltige_kennungen_werden_ohne_eingabetext_abgelehnt() {
    // rot, wenn ungueltige Kennungen angenommen oder im Fehler zurueckgegeben werden.
    let gespraeche = Gespraeche::neu();
    for kennung in [
        "".into(),
        "a".repeat(65),
        "a b".into(),
        "a\n".into(),
        "ä".into(),
    ] {
        assert_eq!(
            gespraeche.beginne(&kennung, "antwort").err(),
            Some("kennung_ungueltig")
        );
        assert_eq!(
            gespraeche.beginne("gespraech", &kennung).err(),
            Some("kennung_ungueltig")
        );
        assert_eq!(gespraeche.anzahl(), 0);
    }
    assert!(gespraeche.beginne("Az09._:-", &"a".repeat(64)).is_ok());
    assert!(gespraeche.beginne(&"b".repeat(64), "Az09._:-").is_ok());
}

#[test]
fn hoechstens_64_gespraeche_bis_zum_ende() {
    // rot, wenn die Grenze fehlt, bestehende IDs zaehlen oder freie Enden nicht entfernt werden.
    let gespraeche = Gespraeche::neu();
    for nummer in 0..64 {
        assert!(gespraeche.beginne(&format!("g{nummer}"), "a").is_ok());
    }
    assert_eq!(gespraeche.anzahl(), 64);
    assert_eq!(
        gespraeche.beginne("weiteres", "a").err(),
        Some("zu_viele_gespraeche")
    );
    assert!(gespraeche.beginne("g0", "a").is_ok());
    assert!(gespraeche.beginne("g0", "b").is_ok());
    gespraeche.ende("unbekannt");
    assert_eq!(gespraeche.anzahl(), 64);
    gespraeche.ende("g0");
    assert_eq!(gespraeche.anzahl(), 63);
    assert!(gespraeche.beginne("weiteres", "a").is_ok());
}

#[test]
fn alter_handle_bleibt_nach_ende_und_neustart_abgelehnt() {
    // rot, wenn ein alter Handle nach dem Entfernen eine eigene Sperre weiterverwenden kann.
    let gespraeche = Gespraeche::neu();
    let s = snapshot(false);
    let abbruch = Abbruch::neu();
    let mut alt = gespraeche.beginne("c1", "a1").unwrap();
    let mut klon = alt.clone();
    gespraeche.ende("c1");
    let mut neu = gespraeche.beginne("c1", "a1").unwrap();
    assert_eq!(alt.naechster_aufruf(&s, &abbruch).err(), Some("abgelehnt"));
    assert!(!alt.laeuft());
    let lauf = neu.naechster_aufruf(&s, &abbruch).unwrap();
    assert_eq!(klon.naechster_aufruf(&s, &abbruch).err(), Some("abgelehnt"));
    drop(lauf);
    assert_eq!(alt.naechster_aufruf(&s, &abbruch).err(), Some("abgelehnt"));
    for _ in 0..4 {
        drop(neu.naechster_aufruf(&s, &abbruch).unwrap());
    }
    assert_eq!(neu.naechster_aufruf(&s, &abbruch).err(), Some("abgelehnt"));
}

#[test]
fn verdraengter_handle_bleibt_auch_nach_neuanlage_abgelehnt() {
    // rot, wenn Verdraengung alte Klone nicht sperrt oder den neuen Zaehler uebernimmt.
    let gespraeche = Gespraeche::neu();
    let s = snapshot(false);
    let abbruch = Abbruch::neu();
    let mut alt = gespraeche.beginne("c1", "a0").unwrap();
    let mut klon = alt.clone();
    drop(alt.naechster_aufruf(&s, &abbruch).unwrap());
    for nummer in 1..=8 {
        gespraeche.beginne("c1", &format!("a{nummer}")).unwrap();
    }
    assert_eq!(alt.naechster_aufruf(&s, &abbruch).err(), Some("abgelehnt"));
    assert!(!alt.laeuft());
    let mut neu = gespraeche.beginne("c1", "a0").unwrap();
    for _ in 0..5 {
        let lauf = neu.naechster_aufruf(&s, &abbruch).unwrap();
        assert_eq!(klon.naechster_aufruf(&s, &abbruch).err(), Some("abgelehnt"));
        drop(lauf);
    }
    assert_eq!(neu.naechster_aufruf(&s, &abbruch).err(), Some("abgelehnt"));
    assert_eq!(alt.naechster_aufruf(&s, &abbruch).err(), Some("abgelehnt"));
}

fn konkurrierender_start(ende: bool) {
    for _ in 0..64 {
        let gespraeche = Gespraeche::neu();
        let mut alt = gespraeche.beginne("c1", "a0").unwrap();
        if !ende {
            for nummer in 1..8 {
                gespraeche.beginne("c1", &format!("a{nummer}")).unwrap();
            }
        }
        let barriere = Arc::new(Barrier::new(2));
        gespraeche.setze_barriere_fuer_test(barriere.clone());
        thread::scope(|threads| {
            let barriere = &barriere;
            let start = threads.spawn(move || {
                let s = snapshot(false);
                let abbruch = Abbruch::neu();
                barriere.wait();
                let lauf = alt.naechster_aufruf(&s, &abbruch);
                barriere.wait();
                barriere.wait();
                let gestartet = lauf.is_ok();
                drop(lauf);
                (alt, gestartet)
            });
            if ende {
                gespraeche.ende("c1");
            } else {
                gespraeche.beginne("c1", "a8").unwrap();
            }
            let s = snapshot(false);
            let abbruch = Abbruch::neu();
            let (neuer_lauf, fehler) = match gespraeche.beginne("c1", "a0") {
                Ok(mut neu) => (neu.naechster_aufruf(&s, &abbruch).ok(), None),
                Err(fehler) => (None, Some(fehler)),
            };
            barriere.wait();
            let (mut alt, gestartet) = start.join().unwrap();
            if let Some(fehler) = fehler {
                assert!(ende);
                assert_eq!(fehler, "gespraech_beendet");
            }
            assert!(!(gestartet && neuer_lauf.is_some()));
            drop(neuer_lauf);
            if ende {
                assert_eq!(alt.naechster_aufruf(&s, &abbruch).err(), Some("abgelehnt"));
            } else if gestartet {
                // Ein gestarteter Eintrag muss weiter denselben Zaehler teilen.
                let mut gemerkt = gespraeche.beginne("c1", "a0").unwrap();
                for _ in 0..4 {
                    drop(gemerkt.naechster_aufruf(&s, &abbruch).unwrap());
                }
                assert_eq!(alt.naechster_aufruf(&s, &abbruch).err(), Some("abgelehnt"));
            }
        });
    }
}

#[test]
fn start_konkurriert_mit_ende() {
    // rot, wenn Ende einen laufenden Eintrag entfernt oder alte Handles starten laesst.
    konkurrierender_start(true);
}

#[test]
fn start_konkurriert_mit_verdraengung() {
    // rot, wenn ein laufender Eintrag verdraengt wird und zwei unabhaengige Runden entstehen.
    konkurrierender_start(false);
}

#[test]
fn vergifteter_karten_mutex_bleibt_benutzbar() {
    // rot, wenn beginne oder ende nach echter Mutex-Vergiftung erneut panieren.
    let gespraeche = Gespraeche::neu();
    gespraeche.beginne("c1", "a1").unwrap();
    thread::scope(|threads| {
        assert!(threads
            .spawn(|| gespraeche.vergifte_karte_fuer_test())
            .join()
            .is_err());
    });
    let mut runde = gespraeche.beginne("c1", "a1").unwrap();
    drop(
        runde
            .naechster_aufruf(&snapshot(false), &Abbruch::neu())
            .unwrap(),
    );
    gespraeche.ende("c1");
    assert_eq!(gespraeche.anzahl(), 0);
    assert!(gespraeche.beginne("c1", "a1").is_ok());
}
