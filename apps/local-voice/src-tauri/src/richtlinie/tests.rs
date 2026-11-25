use super::*;
use std::io::Cursor;

fn lesen(bytes: &[u8]) -> Snapshot {
    aus_leser(Cursor::new(bytes), bytes.len() as u64)
}

#[test]
fn vorgaben_und_teilprofil() {
    assert_eq!(lesen(b"{}").ausgabe.richtlinie, Richtlinie::default());
    let p = lesen(br#"{"kein_autostart":true}"#);
    assert!(p.ausgabe.richtlinie.kein_autostart);
    assert!(!p.ausgabe.richtlinie.kein_systemton);
    assert_eq!(p.ausgabe.herkunft, Herkunft::Datei);
}

#[test]
fn ungueltige_inhalte_sind_streng() {
    for bytes in [b"".as_slice(), b"{", b"null", b"[]", b"true",
        br#"{"kein_autostart":null}"#, br#"{"kein_autostart":1}"#,
        br#"{"kein_autostart":"true"}"#, b"\xff"] {
        let p = lesen(bytes);
        assert_eq!(p.ausgabe.richtlinie, Richtlinie::streng());
        assert_eq!(p.ausgabe.herkunft, Herkunft::DateiFehlerhaft);
    }
}

#[test]
fn unbekannte_nur_namen_einmal_ohne_werte() {
    let p = lesen(br#"{"neuer_schalter":0,"neuer_schalter":"GEHEIMER_WERT","kein_systemton":true}"#);
    assert_eq!(p.unbekannt, ["neuer_schalter"]);
    assert!(p.ausgabe.richtlinie.kein_systemton);
    assert_eq!(p.ausgabe.herkunft, Herkunft::Datei);
    crate::testlog::installieren();
    p.protokollieren();
    let log = crate::testlog::zeilen().join("\n");
    assert!(log.contains("neuer_schalter"));
    assert!(!log.contains("GEHEIMER_WERT"));
    assert_eq!(p.meldungen().len(), 1);
}

#[test]
fn fehlerprotokoll_ohne_inhalt_oder_pfad() {
    crate::testlog::installieren();
    let p = lesen(b"SYNTHETISCHER_VERTRAULICHER_INHALT");
    p.protokollieren();
    let log = crate::testlog::zeilen().join("\n");
    assert!(log.contains("Firmenrichtlinie fehlerhaft; strengster Zustand aktiv"));
    assert!(!log.contains("SYNTHETISCHER_VERTRAULICHER_INHALT"));
}

#[test]
fn groesse_vor_und_waehrend_lesen_begrenzt() {
    struct NichtLesen;
    impl Read for NichtLesen {
        fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
            panic!("zu grosse Datei darf nicht gelesen werden")
        }
    }
    assert_eq!(aus_leser(NichtLesen, LIMIT + 1).ausgabe.herkunft, Herkunft::DateiFehlerhaft);
    let mut bytes = vec![b' '; LIMIT as usize];
    bytes[..2].copy_from_slice(b"{}");
    assert_eq!(lesen(&bytes).ausgabe.herkunft, Herkunft::Datei);
    bytes.push(b' ');
    // Metadaten noch klein, Inhalt inzwischen gewachsen.
    assert_eq!(aus_leser(Cursor::new(bytes), 2).ausgabe.herkunft, Herkunft::DateiFehlerhaft);
}

#[test]
fn lesefehler_ist_nicht_fehlend() {
    struct Kaputt;
    impl Read for Kaputt {
        fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
            Err(io::Error::new(io::ErrorKind::PermissionDenied, "synthetisch"))
        }
    }
    assert_eq!(aus_leser(Kaputt, 2).ausgabe.herkunft, Herkunft::DateiFehlerhaft);
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("richtlinie.json");
    assert_eq!(aus_datei(&path).ausgabe.herkunft, Herkunft::Keine);
    std::fs::create_dir(&path).unwrap();
    assert_eq!(aus_datei(&path).ausgabe.herkunft, Herkunft::DateiFehlerhaft);
}

#[test]
fn datei_und_kanonischer_exe_ordner() {
    let tmp = tempfile::tempdir().unwrap();
    let exe = tmp.path().join("app.exe");
    std::fs::write(&exe, b"synthetisch").unwrap();
    std::fs::write(tmp.path().join("richtlinie.json"), b"{}").unwrap();
    assert_eq!(aus_exe(Ok(exe)).ausgabe.herkunft, Herkunft::Datei);
    assert_eq!(aus_exe(Err(io::Error::other("synthetisch"))).ausgabe.herkunft, Herkunft::DateiFehlerhaft);
    assert_eq!(aus_exe(Ok(tmp.path().join("fehlt.exe"))).ausgabe.herkunft, Herkunft::DateiFehlerhaft);
}

#[test]
fn kanonische_exe_nimmt_nur_datei_am_ziel() {
    use crate::managers::model::download::linktest::symlink_file;
    let tmp = tempfile::tempdir().unwrap();
    let echt = tmp.path().join("echt");
    let start = tmp.path().join("start");
    std::fs::create_dir(&echt).unwrap();
    std::fs::create_dir(&start).unwrap();
    let exe = echt.join("app.exe");
    let link = start.join("app.exe");
    std::fs::write(&exe, b"synthetisch").unwrap();
    std::fs::write(start.join("richtlinie.json"), b"{}").unwrap();
    std::fs::create_dir(echt.join("Data")).unwrap();
    std::fs::write(echt.join("Data/richtlinie.json"), b"kaputt").unwrap();
    if !symlink_file(&link, &exe) {
        return;
    }
    assert_eq!(aus_exe(Ok(link.clone())).ausgabe.herkunft, Herkunft::Keine);
    std::fs::write(echt.join("richtlinie.json"), br#"{"kein_systemton":true}"#).unwrap();
    let p = aus_exe(Ok(link));
    assert_eq!(p.ausgabe.herkunft, Herkunft::Datei);
    assert!(p.ausgabe.richtlinie.kein_systemton);
    // Gegenprobe: am Startpfad liegt ausdruecklich ein anderes Profil.
    assert!(!aus_datei(&start.join("richtlinie.json")).ausgabe.richtlinie.kein_systemton);
}

#[test]
fn einmal_gelesen_kein_neuladen() {
    let tmp = tempfile::tempdir().unwrap();
    let exe = tmp.path().join("app.exe");
    let path = tmp.path().join("richtlinie.json");
    std::fs::write(&exe, b"synthetisch").unwrap();
    std::fs::write(&path, b"{}").unwrap();
    let speicher = OnceLock::new();
    assert_eq!(snapshot(&speicher, || Ok(exe.clone())).ausgabe.richtlinie, Richtlinie::default());
    std::fs::write(&path, b"kaputt").unwrap();
    assert_eq!(snapshot(&speicher, || panic!("zweites current_exe")).ausgabe.richtlinie, Richtlinie::default());
    // Gegenprobe: ein neuer Start sieht die geaenderte Datei.
    assert_eq!(snapshot(&OnceLock::new(), || Ok(exe)).ausgabe.herkunft, Herkunft::DateiFehlerhaft);
}

#[cfg(windows)]
#[test]
fn junction_und_datei_symlink_auch_defekt_abgelehnt() {
    use crate::managers::model::download::linktest::{folder_link, symlink_file};
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("richtlinie.json");
    let ziel = tmp.path().join("ziel");
    std::fs::create_dir(&ziel).unwrap();
    folder_link(&path, &ziel);
    assert_eq!(aus_datei(&path).ausgabe.herkunft, Herkunft::DateiFehlerhaft);
    std::fs::remove_dir(&path).unwrap();
    std::fs::remove_dir(&ziel).unwrap();
    if !symlink_file(&path, &ziel) { return; }
    assert_eq!(aus_datei(&path).ausgabe.herkunft, Herkunft::DateiFehlerhaft);
    std::fs::write(&ziel, b"{}").unwrap();
    assert_eq!(aus_datei(&path).ausgabe.herkunft, Herkunft::DateiFehlerhaft);
    std::fs::remove_file(&path).unwrap();
    // Gegenprobe: regulaere Datei wird akzeptiert.
    std::fs::write(&path, b"{}").unwrap();
    assert_eq!(aus_datei(&path).ausgabe.herkunft, Herkunft::Datei);
}

#[test]
fn richtlinie_cli_flag_und_frueher_weg() {
    use clap::Parser;
    let args = crate::CliArgs::try_parse_from(["local-voice-ai", "--richtlinie-zeigen"]).unwrap();
    assert!(args.richtlinie_zeigen);
    assert!(!crate::CliArgs::try_parse_from(["local-voice-ai"]).unwrap().richtlinie_zeigen);
    let main = include_str!("../main.rs");
    assert!(main.find("if cli_args.richtlinie_zeigen").unwrap() < main.find("if cli_args.mcp").unwrap());
    assert!(main.contains("richtlinie::zeigen(&mut std::io::stdout().lock())"));
}

#[cfg(unix)]
#[test]
fn links_auch_defekte_und_fifo_abgelehnt() {
    use std::os::unix::fs::symlink;
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("richtlinie.json");
    let ziel = tmp.path().join("ziel");
    symlink(&ziel, &path).unwrap();
    assert_eq!(aus_datei(&path).ausgabe.herkunft, Herkunft::DateiFehlerhaft);
    std::fs::write(&ziel, b"{}").unwrap();
    assert_eq!(aus_datei(&path).ausgabe.herkunft, Herkunft::DateiFehlerhaft);
    // current_exe wird vor der Wahl des Richtlinienordners kanonisch aufgeloest.
    let exe_link = tmp.path().join("app-link");
    symlink(&ziel, &exe_link).unwrap();
    std::fs::remove_file(&path).unwrap();
    assert_eq!(aus_exe(Ok(exe_link)).ausgabe.herkunft, Herkunft::Keine);
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;
    let name = CString::new(path.as_os_str().as_bytes()).unwrap();
    assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
    assert_eq!(aus_datei(&path).ausgabe.herkunft, Herkunft::DateiFehlerhaft);
}

#[cfg(windows)]
#[test]
fn gesperrte_datei_ist_fehlerhaft() {
    use std::os::windows::fs::OpenOptionsExt;
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("richtlinie.json");
    std::fs::write(&path, b"{}").unwrap();
    let _sperre = OpenOptions::new().read(true).share_mode(0).open(&path).unwrap();
    assert_eq!(aus_datei(&path).ausgabe.herkunft, Herkunft::DateiFehlerhaft);
}

#[test]
fn oder_fuer_jeden_schalter() {
    for &name in FELDNAMEN {
        let a: Richtlinie = serde_json::from_value(serde_json::json!({name: true})).unwrap();
        assert_eq!(zusammenfuehren(a, Richtlinie::default()), a);
        assert_eq!(zusammenfuehren(Richtlinie::default(), a), a);
        assert_eq!(zusammenfuehren(a, Richtlinie::streng()), Richtlinie::streng());
    }
}

#[test]
fn cli_json_enthaelt_schalter_herkunft_und_durchgesetzt() {
    let p = lesen(br#"{"kein_autostart":true}"#);
    let mut out = Vec::new();
    schreiben(&p.ausgabe, &mut out).unwrap();
    let value: serde_json::Value = serde_json::from_slice(&out).unwrap();
    assert_eq!(value.as_object().unwrap().len(), FELDNAMEN.len() + 2);
    assert_eq!(value["durchgesetzt"], serde_json::json!(DURCHGESETZT));
    assert_eq!(DURCHGESETZT, FELDNAMEN);
    assert_eq!(value["herkunft"], "datei");
    assert_eq!(value["kein_autostart"], true);
    assert_eq!(value["kein_systemton"], false);
    let strict = serde_json::to_value(Snapshot::fehlerhaft().ausgabe).unwrap();
    assert_eq!(strict["herkunft"], "datei_fehlerhaft");
    assert_eq!(strict.as_object().unwrap().values().filter(|v| v.as_bool() == Some(true)).count(), 6);
}

#[test]
fn feldnamen_entsprechen_serialisierter_struktur() {
    let value = serde_json::to_value(Richtlinie::default()).unwrap();
    let felder = value.as_object().unwrap();
    assert_eq!(felder.len(), FELDNAMEN.len());
    let namen: std::collections::HashSet<_> = FELDNAMEN.iter().copied().collect();
    assert_eq!(namen.len(), FELDNAMEN.len());
    assert!(felder.keys().all(|name| namen.contains(name.as_str())));
    assert!(DURCHGESETZT.iter().all(|name| namen.contains(name)));
    // Gegenprobe: ein erfundener Schalter darf nicht als durchgesetzt gelten.
    assert!(!namen.contains("synthetisch_unbekannt"));
}
