//! Tests des Prüfers (lva#604, Teilpaket 2).
//!
//! Alle Barrieren laufen über die Testquelle: ihr Haken greift am Anfang einer bestimmten
//! Lesung und schiebt dort einen Widerruf oder Richtlinienwechsel ein. Die Nummern der Lesungen
//! im ersten Prüfversuch sind: 1 = R1, 2 = R2 (dazwischen die Quellenprüfung), danach bei
//! Abschlussvergleich 3 = R3. Kein Schlaf, kein Wettlauf.
//!
//! Die Tests am Ende (`echter_speicher`) laufen gegen die App-Quelle mit einer echten
//! Besprechungsdatenbank und einer echten Einstellungsdatei in einem Temp-Verzeichnis.

use super::*;
use crate::llm_versand::fehler::CODE_BESPRECHUNG_CLOUD_GESPERRT;

fn richtlinie() -> KiSperre {
    KiSperre::Pruefen {
        kundenprofil_nur_lokal: false,
        cloud_bei_externen: false,
        eigene_domaenen: vec!["example.org".into()],
    }
}

fn richtlinie_nur_lokal() -> KiSperre {
    KiSperre::Pruefen {
        kundenprofil_nur_lokal: true,
        cloud_bei_externen: false,
        eigene_domaenen: vec!["example.org".into()],
    }
}

fn cloud() -> Ziel {
    Ziel::http("openai", "https://api.example.com/v1", "m")
}

fn lokal() -> Ziel {
    Ziel::http("ollama", "http://127.0.0.1:11434/v1", "m")
}

fn cli() -> Ziel {
    Ziel::cli("claude-cli", "claude", "m")
}

fn gesperrt(grund: &'static str) -> VersandFehler {
    VersandFehler::gesperrt(grund)
}

/// Frische Quelle (Zähler bei 0) mit der Besprechung `m1` freigegeben; die Freigabe hebt die
/// Revision von 1 auf 2.
fn quelle_mit_m1() -> TestQuelle {
    let quelle = TestQuelle::neu(1, richtlinie());
    quelle.freigeben("m1");
    quelle
}

fn lesung(revision: i64) -> Lesung {
    Lesung {
        revision,
        sperre: richtlinie(),
    }
}

/// Die Prüfung muss gesperrt haben; liefert den Fehler.
fn gesperrt_mit(ergebnis: Result<Geprueft, VersandFehler>) -> VersandFehler {
    ergebnis.expect_err("die Prüfung hätte sperren müssen")
}

fn ist_send_sync<T: Send + Sync>() {}

// ---------------------------------------------------------------------------
// Vier Grundfälle
// ---------------------------------------------------------------------------

#[test]
fn lokal_braucht_keine_freigabe_auch_nicht_bei_nur_lokal_oder_nicht_sichtbarer_quelle() {
    for sperre in [richtlinie(), richtlinie_nur_lokal()] {
        for quellen in [
            Quellen::besprechung("m-nicht-sichtbar"),
            Quellen::besprechungen(["a", "b"]),
            Quellen::unbestimmt(),
            Quellen::keine(),
        ] {
            let quelle = TestQuelle::neu(1, sperre.clone());
            let geprueft = freigabe_ausstellen(&quelle, &lokal(), &quellen)
                .expect("lokal ist ohne Freigabe erlaubt");
            assert!(geprueft.r2.is_none(), "lokal hat keinen Abschlussvergleich");
            assert_eq!(quelle.lesungen(), 0, "lokal liest nichts");
            assert_eq!(quelle.sichtbarkeitsabfragen(), 0);
        }
    }
}

#[test]
fn lokal_haengt_auch_nicht_an_einer_kaputten_quelle() {
    let quelle = TestQuelle::neu(1, richtlinie());
    quelle.setze_lesung(Err(gesperrt(grund::LESEFEHLER)));
    assert!(freigabe_ausstellen(&quelle, &lokal(), &Quellen::besprechung("m1")).is_ok());
    assert_eq!(quelle.lesungen(), 0);
}

#[test]
fn cloud_mit_freigabe_ist_erlaubt() {
    let quelle = quelle_mit_m1();
    let geprueft = freigabe_ausstellen(&quelle, &cloud(), &Quellen::besprechung("m1"))
        .expect("freigegebene Besprechung darf in die Cloud");
    assert_eq!(
        geprueft.r2,
        Some(lesung(2)),
        "R2 ist der Stand nach der Freigabe"
    );
    assert_eq!(quelle.lesungen(), 2, "R1 und R2");
    assert_eq!(quelle.sichtbarkeitsabfragen(), 1);
}

#[test]
fn die_freigabe_loest_nur_fuer_ziel_und_quellen_der_pruefung_ein() {
    let quelle = quelle_mit_m1();
    let quellen = Quellen::besprechung("m1");
    let Geprueft { freigabe, .. } = freigabe_ausstellen(&quelle, &cloud(), &quellen).unwrap();
    let umgebogen = Ziel::http("openai", "https://anderer-host.example/v1", "m");
    assert_eq!(
        freigabe.einloesen(&umgebogen, &quellen),
        Err(gesperrt(grund::ZIEL_GEWECHSELT))
    );
    let Geprueft { freigabe, .. } = freigabe_ausstellen(&quelle, &cloud(), &quellen).unwrap();
    assert_eq!(freigabe.einloesen(&cloud(), &quellen), Ok(()));
}

#[test]
fn cloud_ohne_freigabe_ist_gesperrt() {
    let quelle = TestQuelle::neu(1, richtlinie());
    let fehler = gesperrt_mit(freigabe_ausstellen(
        &quelle,
        &cloud(),
        &Quellen::besprechung("m1"),
    ));
    assert_eq!(fehler, gesperrt(grund::NICHT_SICHTBAR));
    assert_eq!(
        quelle.lesungen(),
        1,
        "nach der Sperre wird nicht weiter gelesen"
    );
}

#[test]
fn cloud_mit_nur_lokal_sperrt_mit_dem_besprechungscode_nicht_mit_compliance() {
    // Auch eine freigegebene Besprechung: das Kundenprofil „nur lokal“ sperrt ganz.
    let quelle = TestQuelle::neu(1, richtlinie_nur_lokal());
    quelle.freigeben("m1");
    let fehler = gesperrt_mit(freigabe_ausstellen(
        &quelle,
        &cloud(),
        &Quellen::besprechung("m1"),
    ));
    assert_eq!(fehler, gesperrt(grund::NUR_LOKAL));
    assert!(
        matches!(fehler, VersandFehler::Gesperrt { code, .. } if code == CODE_BESPRECHUNG_CLOUD_GESPERRT),
        "{fehler:?}"
    );
    assert!(!matches!(fehler, VersandFehler::Compliance { .. }));
    assert_eq!(fehler.code(), "besprechung_cloud_gesperrt");
    assert!(fehler.is_terminal());
}

#[test]
fn jeder_sperrgrund_der_besprechungspruefung_traegt_den_besprechungscode() {
    let faelle: Vec<(TestQuelle, Quellen, &'static str)> = vec![
        (
            TestQuelle::neu(1, richtlinie_nur_lokal()),
            Quellen::besprechung("m1"),
            grund::NUR_LOKAL,
        ),
        (
            TestQuelle::neu(1, richtlinie()),
            Quellen::besprechung("m1"),
            grund::NICHT_SICHTBAR,
        ),
        (
            TestQuelle::neu(1, richtlinie()),
            Quellen::unbestimmt(),
            grund::UNBESTIMMT,
        ),
    ];
    for (quelle, quellen, erwartet) in faelle {
        let fehler = gesperrt_mit(freigabe_ausstellen(&quelle, &cloud(), &quellen));
        assert_eq!(fehler, gesperrt(erwartet));
        assert!(matches!(
            fehler,
            VersandFehler::Gesperrt { code, .. } if code == CODE_BESPRECHUNG_CLOUD_GESPERRT
        ));
    }
}

#[test]
fn ein_cli_ziel_zaehlt_als_cloud() {
    let quelle = TestQuelle::neu(1, richtlinie());
    assert_eq!(
        gesperrt_mit(freigabe_ausstellen(
            &quelle,
            &cli(),
            &Quellen::besprechung("m1")
        )),
        gesperrt(grund::NICHT_SICHTBAR)
    );
    quelle.freigeben("m1");
    assert!(freigabe_ausstellen(&quelle, &cli(), &Quellen::besprechung("m1")).is_ok());
}

// ---------------------------------------------------------------------------
// Quellen
// ---------------------------------------------------------------------------

#[test]
fn unbestimmt_sperrt_bei_cloud_ohne_zu_lesen() {
    let quelle = quelle_mit_m1();
    let fehler = gesperrt_mit(freigabe_ausstellen(
        &quelle,
        &cloud(),
        &Quellen::unbestimmt(),
    ));
    assert_eq!(fehler, gesperrt(grund::UNBESTIMMT));
    assert_eq!(quelle.lesungen(), 0);
}

#[test]
fn eine_vereinigung_mit_unbestimmt_sperrt_auch_freigegebene_quellen() {
    let quelle = quelle_mit_m1();
    let quellen = Quellen::besprechung("m1").vereinigen(Quellen::unbestimmt());
    assert_eq!(
        gesperrt_mit(freigabe_ausstellen(&quelle, &cloud(), &quellen)),
        gesperrt(grund::UNBESTIMMT)
    );
    // Eine leere ID ist selbst unbestimmt.
    assert_eq!(
        gesperrt_mit(freigabe_ausstellen(
            &quelle,
            &cloud(),
            &Quellen::besprechung("")
        )),
        gesperrt(grund::UNBESTIMMT)
    );
}

#[test]
fn jede_quelle_muss_sichtbar_sein() {
    let quelle = quelle_mit_m1();
    quelle.freigeben("m2");
    let beide = Quellen::besprechungen(["m1", "m2"]);
    assert!(freigabe_ausstellen(&quelle, &cloud(), &beide).is_ok());

    let mit_dritter = Quellen::besprechungen(["m1", "m2", "m3"]);
    assert_eq!(
        gesperrt_mit(freigabe_ausstellen(&quelle, &cloud(), &mit_dritter)),
        gesperrt(grund::NICHT_SICHTBAR)
    );
}

#[test]
fn keine_bei_cloud_ist_erlaubt_ohne_zu_lesen() {
    // Weder „nur lokal“ noch eine kaputte Quelle noch ein fehlender Spiegel stören einen Aufruf
    // ohne Besprechungsinhalt.
    for sperre in [richtlinie(), richtlinie_nur_lokal()] {
        let quelle = TestQuelle::neu(1, sperre);
        quelle.setze_lesung(Err(gesperrt(grund::SPIEGEL_FEHLT)));
        let geprueft = freigabe_ausstellen(&quelle, &cloud(), &Quellen::keine())
            .expect("keine() ist bei Cloud erlaubt");
        assert!(geprueft.r2.is_none());
        assert_eq!(quelle.lesungen(), 0);
        assert_eq!(quelle.sichtbarkeitsabfragen(), 0);
    }
}

#[test]
fn eine_leere_menge_von_besprechungen_ist_keine() {
    let quelle = TestQuelle::neu(1, richtlinie());
    let leer: [&str; 0] = [];
    assert!(freigabe_ausstellen(&quelle, &cloud(), &Quellen::besprechungen(leer)).is_ok());
    assert_eq!(quelle.lesungen(), 0);
}

// ---------------------------------------------------------------------------
// Spiegel und Lesefehler
// ---------------------------------------------------------------------------

#[test]
fn fehlender_oder_abweichender_spiegel_sperrt() {
    for g in [grund::SPIEGEL_FEHLT, grund::SPIEGEL_ABWEICHEND] {
        let quelle = quelle_mit_m1();
        quelle.setze_lesung(Err(gesperrt(g)));
        assert_eq!(
            gesperrt_mit(freigabe_ausstellen(
                &quelle,
                &cloud(),
                &Quellen::besprechung("m1")
            )),
            gesperrt(g)
        );
        assert_eq!(quelle.lesungen(), 1);
    }
}

#[test]
fn ein_lesefehler_in_r1_sperrt() {
    let quelle = quelle_mit_m1();
    quelle.setze_lesung(Err(gesperrt(grund::LESEFEHLER)));
    assert_eq!(
        gesperrt_mit(freigabe_ausstellen(
            &quelle,
            &cloud(),
            &Quellen::besprechung("m1")
        )),
        gesperrt(grund::LESEFEHLER)
    );
    assert_eq!(quelle.lesungen(), 1);
    assert_eq!(quelle.sichtbarkeitsabfragen(), 0);
}

#[test]
fn ein_lesefehler_in_r2_sperrt() {
    let quelle = quelle_mit_m1();
    quelle.bei_lesung(|q, nr| {
        if nr == 2 {
            q.setze_lesung(Err(gesperrt(grund::LESEFEHLER)));
        }
    });
    assert_eq!(
        gesperrt_mit(freigabe_ausstellen(
            &quelle,
            &cloud(),
            &Quellen::besprechung("m1")
        )),
        gesperrt(grund::LESEFEHLER)
    );
    assert_eq!(
        quelle.lesungen(),
        2,
        "kein weiterer Versuch nach einem Lesefehler"
    );
}

#[test]
fn ein_fremder_fehler_der_quelle_wird_zum_lesefehler() {
    // Eine Quelle darf nur `Gesperrt` liefern; tut sie es nicht, öffnet das nichts.
    for fremd in [
        VersandFehler::Transport,
        VersandFehler::Abgebrochen,
        VersandFehler::Budget {
            meldung: "x".into(),
        },
    ] {
        let quelle = quelle_mit_m1();
        quelle.setze_lesung(Err(fremd));
        assert_eq!(
            gesperrt_mit(freigabe_ausstellen(
                &quelle,
                &cloud(),
                &Quellen::besprechung("m1")
            )),
            gesperrt(grund::LESEFEHLER)
        );
    }
}

// ---------------------------------------------------------------------------
// Linearisierung R1 -> R2
// ---------------------------------------------------------------------------

#[test]
fn revisionswechsel_zwischen_r1_und_r2_fuehrt_zur_neupruefung() {
    let quelle = quelle_mit_m1();
    quelle.bei_lesung(|q, nr| {
        if nr == 2 {
            q.anheben();
        }
    });
    let geprueft = freigabe_ausstellen(&quelle, &cloud(), &Quellen::besprechung("m1"))
        .expect("der zweite Versuch ist stabil");
    assert_eq!(quelle.lesungen(), 4, "R1, R2 (abweichend), R1', R2'");
    assert_eq!(quelle.sichtbarkeitsabfragen(), 2, "jeder Versuch prüft neu");
    assert_eq!(geprueft.r2, Some(lesung(3)));
}

#[test]
fn richtlinienwechsel_bei_gleicher_revision_fuehrt_ebenfalls_zur_neupruefung() {
    // Verglichen wird alles, nicht nur die Revision.
    let quelle = quelle_mit_m1();
    quelle.bei_lesung(|q, nr| {
        if nr == 2 {
            let revision = q.stand().unwrap().revision;
            q.setze_lesung(Ok(Lesung {
                revision,
                sperre: KiSperre::Pruefen {
                    kundenprofil_nur_lokal: false,
                    cloud_bei_externen: true,
                    eigene_domaenen: vec!["example.org".into()],
                },
            }));
        }
    });
    let geprueft = freigabe_ausstellen(&quelle, &cloud(), &Quellen::besprechung("m1")).unwrap();
    assert_eq!(quelle.lesungen(), 4);
    assert!(matches!(
        geprueft.r2,
        Some(Lesung {
            sperre: KiSperre::Pruefen {
                cloud_bei_externen: true,
                ..
            },
            ..
        })
    ));
}

#[test]
fn ein_dauerhafter_wechsel_endet_nach_drei_versuchen_mit_gesperrt() {
    let quelle = quelle_mit_m1();
    quelle.bei_lesung(|q, _| q.anheben());
    let fehler = gesperrt_mit(freigabe_ausstellen(
        &quelle,
        &cloud(),
        &Quellen::besprechung("m1"),
    ));
    assert_eq!(fehler, gesperrt(grund::PRUEFVERSUCHE_ERSCHOEPFT));
    assert_eq!(PRUEFVERSUCHE, 3);
    assert_eq!(
        quelle.lesungen(),
        2 * PRUEFVERSUCHE,
        "je Versuch R1 und R2, nicht mehr"
    );
}

#[test]
fn widerruf_zwischen_quellenpruefung_und_r2_sperrt() {
    // Lesung 2 ist R2; die Quellenprüfung (sichtbar) lag davor und sah m1 noch freigegeben.
    let quelle = quelle_mit_m1();
    quelle.bei_lesung(|q, nr| {
        if nr == 2 {
            q.widerrufen("m1");
        }
    });
    let fehler = gesperrt_mit(freigabe_ausstellen(
        &quelle,
        &cloud(),
        &Quellen::besprechung("m1"),
    ));
    assert_eq!(fehler, gesperrt(grund::NICHT_SICHTBAR));
    assert_eq!(
        quelle.lesungen(),
        3,
        "R1, R2 (Revision gewechselt), R1' (dann ist m1 unsichtbar)"
    );
}

#[test]
fn widerruf_und_neue_freigabe_zwischen_r1_und_r2_zaehlen_als_aenderung() {
    // Widerruf und erneute Freigabe zwischen R1 und R2 (A → B → A): die Revision ist
    // gestiegen, der Versuch zählt nicht; der nächste sieht den neuen Stand.
    let quelle = quelle_mit_m1();
    quelle.bei_lesung(|q, nr| {
        if nr == 2 {
            q.widerrufen("m1");
            q.freigeben("m1");
        }
    });
    let geprueft = freigabe_ausstellen(&quelle, &cloud(), &Quellen::besprechung("m1")).unwrap();
    assert_eq!(quelle.lesungen(), 4);
    assert_eq!(geprueft.r2, Some(lesung(4)));
}

// ---------------------------------------------------------------------------
// Abschlussvergleich R3
// ---------------------------------------------------------------------------

/// Eine bestandene Prüfung der freigegebenen Besprechung `m1` samt R2.
fn geprueft_m1(quelle: &TestQuelle) -> (Versandfreigabe, Lesung) {
    let Geprueft { freigabe, r2 } =
        freigabe_ausstellen(quelle, &cloud(), &Quellen::besprechung("m1")).unwrap();
    (
        freigabe,
        r2.expect("Cloud mit Besprechung hat einen Abschlussvergleich"),
    )
}

#[test]
fn ohne_aenderung_besteht_der_abschlussvergleich() {
    let quelle = quelle_mit_m1();
    let (_, r2) = geprueft_m1(&quelle);
    assert_eq!(abschluss_pruefen(&quelle, &r2), Ok(()));
    assert_eq!(quelle.lesungen(), 3, "R1, R2, R3");
}

#[test]
fn widerruf_zwischen_r2_und_r3_wird_erkannt() {
    let quelle = quelle_mit_m1();
    let (_, r2) = geprueft_m1(&quelle);
    quelle.widerrufen("m1");
    assert_eq!(
        abschluss_pruefen(&quelle, &r2),
        Err(gesperrt(grund::WIDERRUF_NACH_START))
    );
}

#[test]
fn widerruf_und_erneute_freigabe_nach_r2_werden_erkannt() {
    let quelle = quelle_mit_m1();
    let (_, r2) = geprueft_m1(&quelle);
    quelle.widerrufen("m1");
    quelle.freigeben("m1");
    assert_eq!(
        abschluss_pruefen(&quelle, &r2),
        Err(gesperrt(grund::WIDERRUF_NACH_START)),
        "die Revision steigt monoton, A → B → A fällt auf"
    );
}

#[test]
fn ein_richtlinienwechsel_nach_r2_wird_erkannt() {
    let quelle = quelle_mit_m1();
    let (_, r2) = geprueft_m1(&quelle);
    quelle.setze_lesung(Ok(Lesung {
        revision: r2.revision,
        sperre: richtlinie_nur_lokal(),
    }));
    assert_eq!(
        abschluss_pruefen(&quelle, &r2),
        Err(gesperrt(grund::WIDERRUF_NACH_START))
    );
}

#[test]
fn ein_lesefehler_in_r3_sperrt() {
    let quelle = quelle_mit_m1();
    let (_, r2) = geprueft_m1(&quelle);
    quelle.setze_lesung(Err(gesperrt(grund::LESEFEHLER)));
    assert_eq!(
        abschluss_pruefen(&quelle, &r2),
        Err(gesperrt(grund::LESEFEHLER))
    );
    quelle.setze_lesung(Err(VersandFehler::Transport));
    assert_eq!(
        abschluss_pruefen(&quelle, &r2),
        Err(gesperrt(grund::LESEFEHLER)),
        "auch ein fremder Fehler der Quelle sperrt"
    );
}

#[test]
fn abschliessen_gibt_das_transportergebnis_unveraendert_zurueck_wenn_nichts_geaendert_wurde() {
    let quelle = quelle_mit_m1();
    let (_, r2) = geprueft_m1(&quelle);
    assert_eq!(
        abschliessen(&quelle, Some(&r2), Ok("antwort")),
        Ok("antwort")
    );
    assert_eq!(
        abschliessen::<&str>(&quelle, Some(&r2), Err(VersandFehler::Http { status: 502 })),
        Err(VersandFehler::Http { status: 502 }),
        "ein Transportfehler bleibt einer, solange R3 = R2"
    );
}

#[test]
fn abschliessen_liest_r3_auch_nach_einem_transportfehler() {
    let quelle = quelle_mit_m1();
    let (_, r2) = geprueft_m1(&quelle);
    let davor = quelle.lesungen();
    let _ = abschliessen::<()>(&quelle, Some(&r2), Err(VersandFehler::Transport));
    assert_eq!(
        quelle.lesungen(),
        davor + 1,
        "R3 wird gelesen, obwohl der Transport scheiterte"
    );
}

#[test]
fn abschliessen_sperrt_vorrangig_vor_jedem_transportergebnis() {
    // Widerruf nach R2: Antwort, jeder Transportfehler und sogar ein Abbruch liefern Gesperrt.
    let ergebnisse: Vec<Result<&str, VersandFehler>> = vec![
        Ok("geheime antwort"),
        Err(VersandFehler::Transport),
        Err(VersandFehler::Http { status: 500 }),
        Err(VersandFehler::Zeitueberschreitung),
        Err(VersandFehler::Abgebrochen),
        Err(VersandFehler::Kontextueberlauf),
        Err(VersandFehler::Absturz),
    ];
    for transport in ergebnisse {
        let quelle = quelle_mit_m1();
        let (_, r2) = geprueft_m1(&quelle);
        quelle.widerrufen("m1");
        let ergebnis = abschliessen(&quelle, Some(&r2), transport);
        assert_eq!(ergebnis, Err(gesperrt(grund::WIDERRUF_NACH_START)));
        assert!(ergebnis.unwrap_err().is_terminal());
    }
}

#[test]
fn abschliessen_ohne_r2_liest_nichts_und_reicht_das_ergebnis_durch() {
    // Lokales Ziel oder `keine()`: kein Abschlussvergleich, keine Lesung.
    let quelle = TestQuelle::neu(1, richtlinie());
    quelle.setze_lesung(Err(gesperrt(grund::LESEFEHLER)));
    assert_eq!(abschliessen(&quelle, None, Ok(7)), Ok(7));
    assert_eq!(
        abschliessen::<i32>(&quelle, None, Err(VersandFehler::Transport)),
        Err(VersandFehler::Transport)
    );
    assert_eq!(quelle.lesungen(), 0);
}

// ---------------------------------------------------------------------------
// Sonstiges
// ---------------------------------------------------------------------------

#[test]
fn quellen_sind_send_und_sync() {
    ist_send_sync::<TestQuelle>();
    ist_send_sync::<AppFreigabeQuelle>();
}

#[test]
fn parallele_pruefungen_stoeren_sich_nicht() {
    const THREADS: usize = 8;
    const DURCHGAENGE: usize = 40;
    let quelle = quelle_mit_m1();
    std::thread::scope(|s| {
        for _ in 0..THREADS {
            s.spawn(|| {
                for _ in 0..DURCHGAENGE {
                    let (_, r2) = geprueft_m1(&quelle);
                    assert_eq!(abschluss_pruefen(&quelle, &r2), Ok(()));
                }
            });
        }
    });
    assert_eq!(quelle.lesungen(), THREADS * DURCHGAENGE * 3);
}

#[test]
fn debug_der_pruefung_verraet_weder_domaenen_noch_quellen() {
    let quelle = quelle_mit_m1();
    let geprueft = freigabe_ausstellen(&quelle, &cloud(), &Quellen::besprechung("m1")).unwrap();
    let text = format!("{geprueft:?}");
    assert!(!text.contains("example.org"), "{text}");
    assert!(!text.contains("m1"), "{text}");
}

// ---------------------------------------------------------------------------
// App-Quelle gegen eine echte Datenbank und eine echte Einstellungsdatei
// ---------------------------------------------------------------------------

mod echter_speicher {
    use std::path::{Path, PathBuf};
    use std::sync::Arc;

    use besprechungsplanung::assistent::Einstufung;
    use serde_json::{json, Value};

    use super::*;
    use crate::managers::calendar::model::{event_key, Attendee, CalEvent, CalendarKind};
    use crate::managers::meetings::ki_freigabe::{self, Freigabenachweis, Rechtsgrundlage};
    use crate::managers::meetings::search::index::tests::ready_meeting;
    use crate::managers::meetings::store::{MeetingStore, SyncMeta};

    const T1: i64 = 1_790_000_000_000;

    struct Aufbau {
        _verzeichnis: tempfile::TempDir,
        einstellungen: PathBuf,
        store: Arc<MeetingStore>,
        besprechung: String,
    }

    impl Aufbau {
        /// Einstellungsdatei (eigene Domäne `example.org`), Datenbank, eine fertige Besprechung.
        /// `mit_spiegel`: so, wie die App den Richtlinienspiegel nach jeder Änderung schreibt.
        fn neu(extra: Value, mit_spiegel: bool) -> Self {
            let verzeichnis = tempfile::tempdir().unwrap();
            let einstellungen = verzeichnis.path().join("settings_store.json");
            einstellungen_schreiben(&einstellungen, extra);
            let store =
                Arc::new(MeetingStore::open_at(&verzeichnis.path().join("meetings.db")).unwrap());
            let besprechung = ready_meeting(&store, "Jour fixe", 1_790_000_000).id;
            let aufbau = Self {
                _verzeichnis: verzeichnis,
                einstellungen,
                store,
                besprechung,
            };
            if mit_spiegel {
                aufbau.spiegeln();
            }
            aufbau
        }

        fn spiegeln(&self) {
            let sperre = crate::mcp::read_ki_sperre(&self.einstellungen);
            self.store
                .ki_richtlinie_setzen(&sperre.richtlinie().unwrap())
                .unwrap();
        }

        fn quelle(&self) -> AppFreigabeQuelle {
            AppFreigabeQuelle::neu(self.einstellungen.clone(), Arc::clone(&self.store))
        }

        fn quellen(&self) -> Quellen {
            Quellen::besprechung(&self.besprechung)
        }

        /// Verknüpft die Besprechung mit einem internen Termin und bestätigt sie für KI-Clients.
        fn freigeben(&self) {
            let store = &self.store;
            store
                .calendar_source_add("src1", CalendarKind::Ics, "Kalender", None, 1_000)
                .unwrap();
            let teilnehmende = ["a@example.org", "b@example.org"].map(|e| Attendee {
                email: Some(e.into()),
                name: None,
                organizer: false,
                is_self: false,
                partstat: None,
            });
            let termin = CalEvent {
                key: event_key("src1", "u-1", T1),
                source_id: "src1".into(),
                uid: "u-1".into(),
                title: "Termin".into(),
                starts_at: T1,
                ends_at: T1 + 1_800_000,
                all_day: false,
                cancelled: false,
                location: None,
                join_url: None,
                description: None,
                instance_start: None,
                attendees: teilnehmende.to_vec(),
            };
            store
                .calendar_replace_events(
                    "src1",
                    &[termin.clone()],
                    &SyncMeta {
                        has_attendee_data: true,
                        etag: None,
                        last_modified: None,
                        now_ms: T1 + 3_600_000,
                    },
                )
                .unwrap();
            store
                .link_meeting_event(&self.besprechung, &termin, "manual", 3_000)
                .unwrap();
            let sperre = crate::mcp::read_ki_sperre(&self.einstellungen);
            let fingerabdruck = ki_freigabe::stand(store, &sperre, &self.besprechung)
                .unwrap()
                .ergebnis
                .fingerabdruck;
            let nachweis = Freigabenachweis {
                informiert: true,
                rechtsgrundlage: Some(Rechtsgrundlage::OeffentlicheAufgabe),
                einwilligungen_liegen_vor: false,
            };
            ki_freigabe::bestaetigen(
                store,
                &sperre,
                &self.besprechung,
                Einstufung::Intern,
                &fingerabdruck,
                &nachweis,
                4_000,
            )
            .unwrap();
        }
    }

    fn einstellungen_schreiben(pfad: &Path, extra: Value) {
        let mut settings = json!({ "meeting_own_domains": ["example.org"] });
        for (k, v) in extra.as_object().unwrap() {
            settings[k] = v.clone();
        }
        std::fs::write(pfad, json!({ "settings": settings }).to_string()).unwrap();
    }

    #[test]
    fn freigegebene_besprechung_geht_in_die_cloud() {
        let a = Aufbau::neu(json!({}), true);
        a.freigeben();
        let geprueft = freigabe_ausstellen(&a.quelle(), &cloud(), &a.quellen())
            .expect("bestätigt, intern, Spiegel passt");
        let r2 = geprueft.r2.expect("Cloud mit Besprechung");
        assert!(r2.revision > 0, "die Trigger haben die Revision angehoben");
        assert_eq!(abschluss_pruefen(&a.quelle(), &r2), Ok(()));
    }

    #[test]
    fn unbestaetigte_besprechung_ist_gesperrt_lokal_aber_erlaubt() {
        let a = Aufbau::neu(json!({}), true);
        assert_eq!(
            gesperrt_mit(freigabe_ausstellen(&a.quelle(), &cloud(), &a.quellen())),
            gesperrt(grund::NICHT_SICHTBAR)
        );
        assert!(freigabe_ausstellen(&a.quelle(), &lokal(), &a.quellen()).is_ok());
    }

    #[test]
    fn widerruf_vor_der_pruefung_sperrt() {
        let a = Aufbau::neu(json!({}), true);
        a.freigeben();
        a.store.ki_bestaetigung_entfernen(&a.besprechung).unwrap();
        assert_eq!(
            gesperrt_mit(freigabe_ausstellen(&a.quelle(), &cloud(), &a.quellen())),
            gesperrt(grund::NICHT_SICHTBAR)
        );
    }

    #[test]
    fn widerruf_nach_r2_wird_im_abschlussvergleich_erkannt() {
        let a = Aufbau::neu(json!({}), true);
        a.freigeben();
        let r2 = freigabe_ausstellen(&a.quelle(), &cloud(), &a.quellen())
            .unwrap()
            .r2
            .unwrap();
        a.store.ki_bestaetigung_entfernen(&a.besprechung).unwrap();
        assert_eq!(
            abschliessen(&a.quelle(), Some(&r2), Ok("antwort")),
            Err(gesperrt(grund::WIDERRUF_NACH_START)),
            "die echte Revision steigt durch den Trigger"
        );
    }

    #[test]
    fn richtlinienwechsel_nach_r2_wird_im_abschlussvergleich_erkannt() {
        let a = Aufbau::neu(json!({}), true);
        a.freigeben();
        let r2 = freigabe_ausstellen(&a.quelle(), &cloud(), &a.quellen())
            .unwrap()
            .r2
            .unwrap();
        // Wie die App: Datei und Spiegel nacheinander.
        einstellungen_schreiben(
            &a.einstellungen,
            json!({ "meeting_cloud_with_externals": true }),
        );
        a.spiegeln();
        assert_eq!(
            abschluss_pruefen(&a.quelle(), &r2),
            Err(gesperrt(grund::WIDERRUF_NACH_START))
        );
    }

    #[test]
    fn kundenprofil_nur_lokal_sperrt_auch_die_freigegebene_besprechung() {
        let a = Aufbau::neu(json!({}), true);
        a.freigeben();
        einstellungen_schreiben(
            &a.einstellungen,
            json!({ "compliance_profile": "local_only" }),
        );
        a.spiegeln();
        assert_eq!(
            gesperrt_mit(freigabe_ausstellen(&a.quelle(), &cloud(), &a.quellen())),
            gesperrt(grund::NUR_LOKAL)
        );
        assert!(freigabe_ausstellen(&a.quelle(), &lokal(), &a.quellen()).is_ok());
    }

    #[test]
    fn ein_unbekanntes_kundenprofil_sperrt_wie_nur_lokal() {
        let a = Aufbau::neu(json!({}), true);
        a.freigeben();
        einstellungen_schreiben(
            &a.einstellungen,
            json!({ "compliance_profile": "gibt_es_nicht" }),
        );
        a.spiegeln();
        assert_eq!(
            gesperrt_mit(freigabe_ausstellen(&a.quelle(), &cloud(), &a.quellen())),
            gesperrt(grund::NUR_LOKAL)
        );
    }

    #[test]
    fn fehlender_spiegel_sperrt() {
        let a = Aufbau::neu(json!({}), false);
        assert_eq!(
            gesperrt_mit(freigabe_ausstellen(&a.quelle(), &cloud(), &a.quellen())),
            gesperrt(grund::SPIEGEL_FEHLT)
        );
    }

    #[test]
    fn abweichender_spiegel_sperrt() {
        let a = Aufbau::neu(json!({}), true);
        a.freigeben();
        // Nur die Datei ist umgestellt, der Spiegel noch nicht (Schreiben unterwegs).
        einstellungen_schreiben(
            &a.einstellungen,
            json!({ "meeting_cloud_with_externals": true }),
        );
        assert_eq!(
            gesperrt_mit(freigabe_ausstellen(&a.quelle(), &cloud(), &a.quellen())),
            gesperrt(grund::SPIEGEL_ABWEICHEND)
        );
    }

    #[test]
    fn fehlende_oder_kaputte_einstellungsdatei_ist_ein_lesefehler_nie_standard() {
        let a = Aufbau::neu(json!({}), true);
        a.freigeben();

        std::fs::write(&a.einstellungen, "{ das ist kein json").unwrap();
        assert_eq!(
            gesperrt_mit(freigabe_ausstellen(&a.quelle(), &cloud(), &a.quellen())),
            gesperrt(grund::LESEFEHLER)
        );

        std::fs::remove_file(&a.einstellungen).unwrap();
        assert_eq!(
            gesperrt_mit(freigabe_ausstellen(&a.quelle(), &cloud(), &a.quellen())),
            gesperrt(grund::LESEFEHLER)
        );
        assert!(
            !a.einstellungen.exists(),
            "die Quelle schreibt nichts, auch keine Standardwerte"
        );
    }

    #[test]
    fn defekte_datenbank_ist_ein_lesefehler() {
        let a = Aufbau::neu(json!({}), true);
        a.freigeben();
        rusqlite::Connection::open(a.store.db_path())
            .unwrap()
            .execute("DROP TABLE meeting_ai_revision", [])
            .unwrap();
        assert_eq!(
            gesperrt_mit(freigabe_ausstellen(&a.quelle(), &cloud(), &a.quellen())),
            gesperrt(grund::LESEFEHLER)
        );
    }

    #[test]
    fn lokal_und_keine_brauchen_weder_datei_noch_datenbank() {
        let a = Aufbau::neu(json!({}), false);
        std::fs::remove_file(&a.einstellungen).unwrap();
        assert!(freigabe_ausstellen(&a.quelle(), &lokal(), &a.quellen()).is_ok());
        assert!(freigabe_ausstellen(&a.quelle(), &cloud(), &Quellen::keine()).is_ok());
    }

    #[test]
    fn die_quelle_aendert_weder_datei_noch_datenbank() {
        let a = Aufbau::neu(json!({}), true);
        a.freigeben();
        let datei = std::fs::read(&a.einstellungen).unwrap();
        let revision = a.store.ki_stand().unwrap().rev;
        let r2 = freigabe_ausstellen(&a.quelle(), &cloud(), &a.quellen())
            .unwrap()
            .r2
            .unwrap();
        abschluss_pruefen(&a.quelle(), &r2).unwrap();
        assert_eq!(std::fs::read(&a.einstellungen).unwrap(), datei);
        assert_eq!(a.store.ki_stand().unwrap().rev, revision);
    }
}
