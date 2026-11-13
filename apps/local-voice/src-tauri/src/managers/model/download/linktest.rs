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

/// Meldet ein erlaubtes Ueberspringen (sichtbar auch ohne `--nocapture`).
#[cfg(windows)]
fn skipped(what: &str, why: &std::io::Error) {
    eprintln!("UEBERSPRUNGEN: {what} nicht anlegbar ({why}); {SKIP_VAR}=1 ist gesetzt");
}

/// Ein Link auf einen Ordner, der keine Rechte braucht: unter Unix ein Symlink,
/// unter Windows eine Junction. Scheitert das Anlegen, schlaegt der Test fehl.
pub(crate) fn folder_link(link: &Path, target: &Path) {
    #[cfg(unix)]
    std::os::unix::fs::symlink(target, link)
        .unwrap_or_else(|e| panic!("Ordner-Link nicht anlegbar: {e}"));
    #[cfg(windows)]
    {
        let made = std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(link)
            .arg(target)
            .output()
            .unwrap_or_else(|e| panic!("mklink nicht startbar: {e}"));
        assert!(
            made.status.success(),
            "Junction nicht anlegbar: {}",
            String::from_utf8_lossy(&made.stdout)
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
        match std::os::windows::fs::symlink_file(target, link) {
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
        match std::os::windows::fs::symlink_dir(target, link) {
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
