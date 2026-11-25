use super::kodierung::{lies_json, Wert};
use super::speicher::{
    schreibe_atomar_mit, SpeicherFehler, VerzeichnisSpeicher, DATEI, MAX_BYTES, VERZEICHNIS,
};
use super::verzeichnis::{kodiertes_verzeichnis, pruefe_verzeichnis};
use crate::managers::integrations::secrets::{
    schnittstelle_get_text_in, schnittstelle_put_in, schnittstelle_ref,
};
use std::fs::{self, File};
use std::io::{self, Write};
use std::sync::atomic::{AtomicU64, Ordering};

fn verzeichnis_wert() -> Wert {
    lies_json(
        r#"{"schema":"lva.schnittstellen/1","eintraege":[{
            "id":"probe","name":"Probe","beschreibung":"","art":"http",
            "zugriff":"lesend","abfluss":"netz","zeitlimit_ms":15000,
            "http":{"methode":"GET","adresse":"https://api.example.org/send",
                "netz":"oeffentlich","antwort_max_bytes":20480,"parameter":[],
                "kopfzeilen":[],"geheimnis":{"kopf":"authorization","praefix":"Bearer ",
                    "schluessel":"probe-schluessel"}}}]}"#,
    )
    .unwrap()
}

// Schuetzt das Lesen eines fehlenden Speichers ohne Nebenwirkungen.
#[test]
fn lesen_ohne_datei_ist_none() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join(VERZEICHNIS);
    assert!(VerzeichnisSpeicher::neu(dir.clone())
        .lesen()
        .unwrap()
        .is_none());
    assert!(!dir.exists());
}

// Schuetzt kanonisches Schreiben, unveraenderte Bytes und normalisierte Lesewerte.
#[test]
fn speichern_und_lesen_geben_dieselben_kanonischen_bytes() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join(VERZEICHNIS);
    let s = VerzeichnisSpeicher::neu(dir.clone());
    let neu = verzeichnis_wert();
    let bytes = s.speichern(&neu, None).unwrap();
    assert_eq!(bytes, kodiertes_verzeichnis(&neu).unwrap().as_bytes());
    assert_eq!(bytes, fs::read(dir.join(DATEI)).unwrap());
    let gelesen = s.lesen().unwrap().unwrap();
    assert_eq!(bytes, gelesen.bytes);
    assert_eq!(gelesen.verzeichnis, pruefe_verzeichnis(&neu).unwrap());
}

// Schuetzt die Schema-Pruefung vor jeder Anlage eines Verzeichnisses.
#[test]
fn ungueltiges_verzeichnis_wird_nicht_geschrieben() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join(VERZEICHNIS);
    let s = VerzeichnisSpeicher::neu(dir.clone());
    let mut neu = verzeichnis_wert();
    neu.setzen("schema", "kaputt");
    let fehler = pruefe_verzeichnis(&neu).unwrap_err();
    assert_eq!(
        s.speichern(&neu, None),
        Err(SpeicherFehler::Ungueltig(fehler))
    );
    assert!(!dir.join(DATEI).exists());
    assert!(!dir.exists());
}

// Schuetzt alle CAS-Zweige und den Vergleich der rohen statt normalisierten Bytes.
#[test]
fn cas_verlangt_genau_den_gelesenen_stand() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join(VERZEICHNIS);
    let s = VerzeichnisSpeicher::neu(dir.clone());
    let neu = verzeichnis_wert();
    let mut alt = s.speichern(&neu, None).unwrap();
    // Gueltiger, aber nicht kanonischer Stand: auch Leerraum gehoert zum CAS.
    alt.push(b' ');
    fs::write(dir.join(DATEI), &alt).unwrap();
    assert_eq!(s.speichern(&neu, None), Err(SpeicherFehler::Konflikt));
    assert_eq!(fs::read(dir.join(DATEI)).unwrap(), alt);
    assert_eq!(
        s.speichern(&neu, Some(&alt[..alt.len() - 1])),
        Err(SpeicherFehler::Konflikt)
    );
    assert_eq!(fs::read(dir.join(DATEI)).unwrap(), alt);
    let gelesen = s.lesen().unwrap().unwrap();
    assert_eq!(gelesen.bytes, alt);
    let bytes = s.speichern(&neu, Some(&gelesen.bytes)).unwrap();
    assert_eq!(fs::read(dir.join(DATEI)).unwrap(), bytes);
}

// Schuetzt CAS bei verschwundener Datei, bevor irgendetwas angelegt wird.
#[test]
fn cas_ohne_datei_legt_nichts_an() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join(VERZEICHNIS);
    let s = VerzeichnisSpeicher::neu(dir.clone());
    assert_eq!(
        s.speichern(&verzeichnis_wert(), Some(b"alt")),
        Err(SpeicherFehler::Konflikt)
    );
    assert!(!dir.exists());
}

// Schuetzt die Groessengrenze und erlaubt genau MAX_BYTES mit JSON-Leerraum.
#[test]
fn groessengrenze_beim_lesen_und_ueberschreiben() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join(VERZEICHNIS);
    fs::create_dir(&dir).unwrap();
    let s = VerzeichnisSpeicher::neu(dir.clone());
    let neu = verzeichnis_wert();
    let mut bytes = kodiertes_verzeichnis(&neu).unwrap().into_bytes();
    assert!(bytes.len() < MAX_BYTES);
    bytes.resize(MAX_BYTES, b' ');
    fs::write(dir.join(DATEI), &bytes).unwrap();
    let gelesen = s.lesen().unwrap().unwrap();
    assert_eq!(gelesen.bytes, bytes);
    assert_eq!(gelesen.verzeichnis, pruefe_verzeichnis(&neu).unwrap());
    bytes.push(b' ');
    fs::write(dir.join(DATEI), &bytes).unwrap();
    assert!(matches!(s.lesen(), Err(SpeicherFehler::ZuGross)));
    assert_eq!(fs::read(dir.join(DATEI)).unwrap(), bytes);
    assert!(matches!(
        s.speichern(&neu, Some(&bytes)),
        Err(SpeicherFehler::ZuGross | SpeicherFehler::Beschaedigt)
    ));
    assert_eq!(fs::read(dir.join(DATEI)).unwrap(), bytes);
}

// Schuetzt UTF-8, JSON und Schema: selbst passende CAS-Bytes erlauben keine Reparatur.
#[test]
fn beschaedigte_datei_bleibt_stehen() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join(VERZEICHNIS);
    fs::create_dir(&dir).unwrap();
    let s = VerzeichnisSpeicher::neu(dir.clone());
    for bytes in [b"\xff".as_slice(), b"kein JSON", b"{\"schema\":\"kaputt\"}"] {
        fs::write(dir.join(DATEI), bytes).unwrap();
        assert!(matches!(s.lesen(), Err(SpeicherFehler::Beschaedigt)));
        assert_eq!(
            s.speichern(&verzeichnis_wert(), Some(bytes)),
            Err(SpeicherFehler::Beschaedigt)
        );
        assert_eq!(fs::read(dir.join(DATEI)).unwrap(), bytes);
    }
}

// Schuetzt regulaere Eintraege: ein Verzeichnis am Dateipfad wird nie veraendert.
#[test]
fn datei_ist_keine_regulaere_datei() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join(VERZEICHNIS);
    let ziel = dir.join(DATEI);
    fs::create_dir_all(&ziel).unwrap();
    fs::write(ziel.join("fremd"), b"unveraendert").unwrap();
    let s = VerzeichnisSpeicher::neu(dir);
    assert!(matches!(s.lesen(), Err(SpeicherFehler::UnsicherePfad)));
    assert_eq!(
        s.speichern(&verzeichnis_wert(), None),
        Err(SpeicherFehler::UnsicherePfad)
    );
    assert_eq!(fs::read(ziel.join("fremd")).unwrap(), b"unveraendert");
}

// Schuetzt atomaren Austausch und Aufraeumen bei einem teilweisen Schreibfehler.
#[test]
fn schreibfehler_hinterlaesst_weder_hilfsdatei_noch_aenderung() {
    let tmp = tempfile::tempdir().unwrap();
    fs::write(tmp.path().join(DATEI), b"alt").unwrap();
    let halb = |d: &mut File, b: &[u8]| -> io::Result<()> {
        d.write_all(&b[..b.len() / 2])?;
        Err(io::Error::from(io::ErrorKind::WriteZero))
    };
    let zaehler = AtomicU64::new(0);
    assert_eq!(
        schreibe_atomar_mit(tmp.path(), DATEI, b"neuer Stand", &halb, &zaehler),
        Err(SpeicherFehler::Schreiben)
    );
    assert_eq!(fs::read(tmp.path().join(DATEI)).unwrap(), b"alt");
    assert_eq!(fs::read_dir(tmp.path()).unwrap().count(), 1);
}

// Schuetzt create_new und den naechsten Namen: fremde Hilfsdateien bleiben unberuehrt.
#[test]
fn hilfsdatei_namensstoss() {
    let tmp = tempfile::tempdir().unwrap();
    let zaehler = AtomicU64::new(7);
    let fremd = tmp
        .path()
        .join(format!(".lva-schn-{}-7.tmp", std::process::id()));
    fs::write(&fremd, b"fremd").unwrap();
    schreibe_atomar_mit(tmp.path(), DATEI, b"neu", &|d, b| d.write_all(b), &zaehler).unwrap();
    assert_eq!(zaehler.load(Ordering::Relaxed), 9);
    assert_eq!(fs::read(&fremd).unwrap(), b"fremd");
    assert_eq!(fs::read(tmp.path().join(DATEI)).unwrap(), b"neu");
    assert_eq!(fs::read_dir(tmp.path()).unwrap().count(), 2);
}

// Schuetzt die Wurzelpruefung vor Zugriffen durch Symlinks oder Junctions.
#[cfg(any(unix, windows))]
#[test]
fn verzeichnis_ist_link() {
    let tmp = tempfile::tempdir().unwrap();
    let ziel = tmp.path().join("ziel");
    let link = tmp.path().join(VERZEICHNIS);
    fs::create_dir(&ziel).unwrap();
    #[cfg(unix)]
    if std::os::unix::fs::symlink(&ziel, &link).is_err() {
        return;
    }
    #[cfg(windows)]
    {
        let ausgabe = std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(&link)
            .arg(&ziel)
            .output();
        if !ausgabe.is_ok_and(|a| a.status.success()) {
            return;
        }
    }
    let s = VerzeichnisSpeicher::neu(link);
    assert!(matches!(s.lesen(), Err(SpeicherFehler::UnsicherePfad)));
    assert_eq!(
        s.speichern(&verzeichnis_wert(), None),
        Err(SpeicherFehler::UnsicherePfad)
    );
    assert_eq!(fs::read_dir(ziel).unwrap().count(), 0);
}

// Schuetzt die Eintragspruefung: eine verlinkte Datei wird nie gelesen oder ersetzt.
#[cfg(any(unix, windows))]
#[test]
fn datei_ist_link() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join(VERZEICHNIS);
    fs::create_dir(&dir).unwrap();
    let ziel = tmp.path().join("draussen.json");
    let bytes = kodiertes_verzeichnis(&verzeichnis_wert())
        .unwrap()
        .into_bytes();
    fs::write(&ziel, &bytes).unwrap();
    let link = dir.join(DATEI);
    #[cfg(unix)]
    if std::os::unix::fs::symlink(&ziel, &link).is_err() {
        return;
    }
    #[cfg(windows)]
    if std::os::windows::fs::symlink_file(&ziel, &link).is_err() {
        return;
    }
    let s = VerzeichnisSpeicher::neu(dir);
    assert!(matches!(s.lesen(), Err(SpeicherFehler::UnsicherePfad)));
    assert_eq!(
        s.speichern(&verzeichnis_wert(), Some(&bytes)),
        Err(SpeicherFehler::UnsicherePfad)
    );
    assert_eq!(fs::read(ziel).unwrap(), bytes);
    assert!(fs::symlink_metadata(link).unwrap().file_type().is_symlink());
}

// Schuetzt die Trennung: nur der Geheimnisname gelangt in den Verzeichnisspeicher.
#[test]
fn geheimnis_steht_nie_im_verzeichnis() {
    let tmp = tempfile::tempdir().unwrap();
    let geheimnisse = tmp.path().join("geheimnisse");
    let dir = tmp.path().join(VERZEICHNIS);
    let schluessel = "probe-schluessel";
    schnittstelle_put_in(&geheimnisse, schluessel, b"SUPERGEHEIM-123").unwrap();
    let wert = schnittstelle_get_text_in(&geheimnisse, schluessel)
        .unwrap()
        .unwrap();
    assert!(wert.as_str() == "SUPERGEHEIM-123");
    let s = VerzeichnisSpeicher::neu(dir.clone());
    let bytes = s.speichern(&verzeichnis_wert(), None).unwrap();
    assert!(std::str::from_utf8(&bytes).unwrap().contains(schluessel));
    for eintrag in fs::read_dir(dir).unwrap() {
        let inhalt = fs::read(eintrag.unwrap().path()).unwrap();
        assert!(!inhalt
            .windows(b"SUPERGEHEIM".len())
            .any(|w| w == b"SUPERGEHEIM"));
    }
}

// Schützt die Namensprüfung vor jedem Zugriff: ungültige Namen legen keine Dateien an.
#[test]
fn ungueltige_geheimnisschluessel_werden_abgewiesen() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("geheimnisse");
    let lang = "a".repeat(65);
    for schluessel in ["A", "", "a/b", lang.as_str(), ".x"] {
        assert!(schnittstelle_ref(schluessel).is_none());
        assert!(schnittstelle_get_text_in(&dir, schluessel).is_err());
        assert!(schnittstelle_put_in(&dir, schluessel, b"SUPERGEHEIM-123").is_err());
    }
    assert!(!dir.exists());
}

// Schützt den begrenzten, stabilen Namensraum und getrennte Geheimnisse bei ähnlichen Namen.
#[test]
fn geheimniszugang_erlaubt_gueltige_namen_und_trennt_ihre_werte() {
    let tmp = tempfile::tempdir().unwrap();
    let lang = format!("a{}", "b".repeat(63));
    let namen = ["probe-schluessel", "a.b_c-0", "0", lang.as_str()];
    for (index, schluessel) in namen.iter().enumerate() {
        assert!(schnittstelle_ref(schluessel).is_some());
        assert!(schnittstelle_get_text_in(tmp.path(), schluessel)
            .unwrap()
            .is_none());
        let wert = format!("SUPERGEHEIM-{index}");
        schnittstelle_put_in(tmp.path(), schluessel, wert.as_bytes()).unwrap();
    }
    for (index, schluessel) in namen.iter().enumerate() {
        let wert = schnittstelle_get_text_in(tmp.path(), schluessel)
            .unwrap()
            .unwrap();
        assert!(wert.as_str() == format!("SUPERGEHEIM-{index}"));
    }
}
