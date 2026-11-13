//! Verknuepfungen fuer Tests, ehrlich statt still uebersprungen.
//!
//! Ein Test, der einen Link braucht und ihn nicht anlegen kann, darf nicht
//! gruen werden, als haette er etwas bewiesen. Regeln:
//!
//! - [`folder_link`] (Unix: Symlink, Windows: Junction) braucht keine Rechte. Geht
//!   es nicht, schlaegt der Test fehl.
//! - [`symlink_file`] / [`symlink_dir`] brauchen unter Windows Entwicklermodus oder
//!   Administratorrechte. Geht es nicht, schlaegt der Test fehl, ausser
//!   `LVA_SKIP_SYMLINK_TESTS=1` ist gesetzt: dann meldet er `UEBERSPRUNGEN` auf
//!   stderr, gibt `false` zurueck und der Aufrufer kehrt zurueck. Unter Unix
//!   scheitert das Anlegen nie still.
//! - Jeder Windows-Symlink-Fall hat einen Junction-Fall nebenan, der ohne Rechte
//!   laeuft; so bleibt die Abdeckung auch dort, wo Symlinks uebersprungen werden.

use std::path::Path;

/// Umgebungsvariable, die das Ueberspringen der Symlink-Tests erlaubt (nur Windows).
#[cfg(windows)]
pub(crate) const SKIP_VAR: &str = "LVA_SKIP_SYMLINK_TESTS";

#[cfg(windows)]
fn skip_requested() -> bool {
    std::env::var(SKIP_VAR).is_ok_and(|v| v == "1")
}

/// Meldet ein erlaubtes Ueberspringen auf dem echten stderr: `eprintln!` wuerde von
/// libtest bei bestandenen Tests verschluckt (nur mit `--nocapture` sichtbar), ein
/// uebersprungener Nachweis soll aber immer sichtbar sein (gleicher Weg wie `report`
/// in `nemotron::engine::model_tests`).
#[cfg(windows)]
fn skipped(what: &str, why: &std::io::Error) {
    use std::io::Write;
    let _ = writeln!(
        std::io::stderr(),
        "UEBERSPRUNGEN: {what} nicht anlegbar ({why}); {SKIP_VAR}=1 ist gesetzt"
    );
}

/// Der Pfad mit Windows-Trennern. `std` fasst `/` und `\` als Trenner, `cmd` (`mklink`)
/// liest ein unquotiertes `/a` aber als Schalter, und ein relatives Symlink-Ziel wird so
/// abgelegt, wie es ankommt. Darum gehen Link und Ziel nie mit gemischten Trennern an
/// diese Aufrufe (die Tests bauen Pfade ausserdem Stueck fuer Stueck).
#[cfg(windows)]
fn windows_form(path: &Path) -> std::path::PathBuf {
    std::path::PathBuf::from(path.to_string_lossy().replace('/', "\\"))
}

/// Ein Link auf einen Ordner, der keine Rechte braucht: unter Unix ein Symlink,
/// unter Windows eine Junction (das Ziel muss dort ein absoluter Pfad sein). Scheitert das
/// Anlegen, schlaegt der Test fehl, mit der vollen Ausgabe von `mklink` (stdout UND
/// stderr, dort steht die Fehlermeldung) und beiden Pfaden.
pub(crate) fn folder_link(link: &Path, target: &Path) {
    #[cfg(unix)]
    std::os::unix::fs::symlink(target, link)
        .unwrap_or_else(|e| panic!("Ordner-Link nicht anlegbar: {e}"));
    #[cfg(windows)]
    {
        let (link, target) = (windows_form(link), windows_form(target));
        let made = std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(&link)
            .arg(&target)
            .output()
            .unwrap_or_else(|e| panic!("mklink nicht startbar: {e}"));
        assert!(
            made.status.success(),
            "Junction nicht anlegbar (Status {:?}): stdout={:?} stderr={:?}; Link={}, Ziel={}",
            made.status.code(),
            String::from_utf8_lossy(&made.stdout),
            String::from_utf8_lossy(&made.stderr),
            link.display(),
            target.display()
        );
    }
}

/// Symlink auf eine Datei. `true`: angelegt. `false`: nur unter Windows und nur
/// mit `LVA_SKIP_SYMLINK_TESTS=1`, wenn es sich nicht anlegen liess. Sonst Fehlschlag.
pub(crate) fn symlink_file(link: &Path, target: &Path) -> bool {
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(target, link)
            .unwrap_or_else(|e| panic!("Symlink nicht anlegbar: {e}"));
        true
    }
    #[cfg(windows)]
    {
        match std::os::windows::fs::symlink_file(windows_form(target), windows_form(link)) {
            Ok(()) => true,
            Err(e) if skip_requested() => {
                skipped("Datei-Symlink", &e);
                false
            }
            Err(e) => panic!(
                "Datei-Symlink nicht anlegbar ({e}); Entwicklermodus oder Administrator noetig, \
                 oder {SKIP_VAR}=1 setzen"
            ),
        }
    }
}

/// Symlink auf einen Ordner, wie [`symlink_file`].
pub(crate) fn symlink_dir(link: &Path, target: &Path) -> bool {
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(target, link)
            .unwrap_or_else(|e| panic!("Symlink nicht anlegbar: {e}"));
        true
    }
    #[cfg(windows)]
    {
        match std::os::windows::fs::symlink_dir(windows_form(target), windows_form(link)) {
            Ok(()) => true,
            Err(e) if skip_requested() => {
                skipped("Ordner-Symlink", &e);
                false
            }
            Err(e) => panic!(
                "Ordner-Symlink nicht anlegbar ({e}); Entwicklermodus oder Administrator noetig, \
                 oder {SKIP_VAR}=1 setzen"
            ),
        }
    }
}

/// Ist `path` ein Link, ohne ihm zu folgen (Unix: Symlink; Windows: Symlink oder Junction)?
pub(crate) fn is_link(path: &Path) -> bool {
    let Ok(meta) = std::fs::symlink_metadata(path) else {
        return false;
    };
    if meta.file_type().is_symlink() {
        return true;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if meta.file_attributes() & 0x400 != 0 {
            return true;
        }
    }
    false
}

/// Nur Windows darf einen Tausch am offenen Handle verwehren (Freigabe-Modus ohne
/// `FILE_SHARE_DELETE`). Unter Unix MUSS der Tausch gelingen; sonst haette der
/// Test nichts geprueft und wuerde stumm gruen.
pub(crate) fn swap_must_have_happened() {
    if !cfg!(windows) {
        panic!("der Tausch hat unter Unix nicht stattgefunden: der Test prueft nichts");
    }
}
