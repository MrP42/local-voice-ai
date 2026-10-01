//! Laufzeit-Verfuegbarkeit des Vorlesens (Issue #29): nur Dateisystem, kein
//! Prozessstart, kein Modellladen -- die Pruefung darf beim Zeichnen der
//! Einstellungen laufen.
//!
//! Eine Stimme gilt erst als verwendbar, wenn ihre Laufzeit VOLLSTAENDIG da ist
//! und auf dieser Plattform startet: Piper braucht neben `piper(.exe)` die
//! espeak-ng-Daten und die Bibliotheken, gegen die es gelinkt ist (fehlt eine,
//! scheitert der Start erst beim Vorlesen: `dyld: Library not loaded`); Fish
//! Speech braucht Python-Umgebung, Server-Skript und Gewichte.
//!
//! Die Funktionsnamen (`piper_ready`, `piper_libraries`, `fish_python`,
//! `fish_supported`, `fish_ready`) sind mit dem macOS-Zweig (PR #32) abgestimmt.

use serde::Serialize;
use specta::Type;
use std::path::{Path, PathBuf};

/// Dateien, die neben dem Piper-Programm liegen muessen (je Plattform).
pub fn piper_libraries(platform: &str) -> &'static [&'static str] {
    match platform {
        "windows-x64" => &[
            "espeak-ng.dll",
            "piper_phonemize.dll",
            "onnxruntime.dll",
            "onnxruntime_providers_shared.dll",
        ],
        "macos-x64" | "macos-aarch64" => &[
            "libespeak-ng.1.dylib",
            "libpiper_phonemize.1.dylib",
            "libonnxruntime.1.14.1.dylib",
        ],
        _ => &[
            "libespeak-ng.so.1",
            "libpiper_phonemize.so.1",
            "libonnxruntime.so.1.14.1",
        ],
    }
}

/// Name des Piper-Programms auf dieser Plattform.
pub fn piper_binary(platform: &str) -> &'static str {
    if platform.starts_with("windows") {
        "piper.exe"
    } else {
        "piper"
    }
}

/// Alles, was zu einem lauffaehigen Piper gehoert, relativ zum Laufzeitordner.
fn piper_required(platform: &str) -> Vec<String> {
    let mut all = vec![piper_binary(platform).to_string()];
    all.extend(
        ["espeak-ng-data/phontab", "espeak-ng-data/phondata"]
            .iter()
            .map(|s| s.to_string()),
    );
    all.extend(piper_libraries(platform).iter().map(|s| s.to_string()));
    all
}

/// Fehlende (oder leere) Dateien der Piper-Laufzeit in `dir`; leer = vollstaendig.
pub fn piper_missing(dir: &Path, platform: &str) -> Vec<String> {
    piper_required(platform)
        .into_iter()
        .filter(|name| {
            let path = dir.join(name);
            if name == piper_binary(platform) {
                !executable(&path)
            } else {
                !nonempty_file(&path)
            }
        })
        .collect()
}

/// Ist die Piper-Laufzeit in `dir` vollstaendig?
pub fn piper_ready(dir: &Path, platform: &str) -> bool {
    piper_missing(dir, platform).is_empty()
}

/// Plattformen, fuer die Piper-Laufzeiten im Katalog stehen.
pub fn piper_supported(platform: &str) -> bool {
    matches!(platform, "windows-x64" | "macos-x64" | "macos-aarch64")
}

fn nonempty_file(path: &Path) -> bool {
    path.metadata().is_ok_and(|m| m.is_file() && m.len() > 0)
}

fn executable(path: &Path) -> bool {
    if !nonempty_file(path) {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if !path
            .metadata()
            .is_ok_and(|m| m.permissions().mode() & 0o111 != 0)
        {
            return false;
        }
    }
    true
}

/// Python der Fish-Speech-Umgebung.
pub fn fish_python(dir: &Path, platform: &str) -> PathBuf {
    if platform.starts_with("windows") {
        dir.join(".venv").join("Scripts").join("python.exe")
    } else {
        dir.join(".venv").join("bin").join("python")
    }
}

/// Fish S2 laeuft auf Windows (CUDA), Apple Silicon und Linux; Intel-Macs
/// haben kein passendes PyTorch.
pub fn fish_supported(platform: &str) -> bool {
    matches!(platform, "windows-x64" | "macos-aarch64" | "linux-x64")
}

/// Fehlende Teile einer Fish-Speech-Installation in `dir` (was `ensure_server`
/// zum Start braucht, plus der Gewichte-Ordner); leer = startfaehig.
pub fn fish_missing(dir: &Path, platform: &str) -> Vec<String> {
    let mut out = Vec::new();
    let python = fish_python(dir, platform);
    if !executable(&python) {
        out.push(
            python
                .strip_prefix(dir)
                .unwrap_or(&python)
                .to_string_lossy()
                .replace('\\', "/"),
        );
    }
    if !nonempty_file(&dir.join("tools").join("api_server.py")) {
        out.push("tools/api_server.py".to_string());
    }
    let has_weights = std::fs::read_dir(dir.join("checkpoints"))
        .map(|mut it| it.any(|e| e.is_ok()))
        .unwrap_or(false);
    if !has_weights {
        out.push("checkpoints".to_string());
    }
    out
}

/// Ist Fish Speech in `dir` startfaehig und auf dieser Plattform unterstuetzt?
pub fn fish_ready(dir: &Path, platform: &str) -> bool {
    fish_supported(platform) && fish_missing(dir, platform).is_empty()
}

/// Meldung, wenn Fish Speech beim Vorlesen nicht startklar ist. Nennt, was
/// fehlt, und den Weg ohne Fish -- keinen Verweis auf einen Entwicklerbericht
/// (Issue #29: eine installierte App darf die Entwicklerinstallation nicht
/// voraussetzen).
pub fn fish_not_set_up_message(dir: &Path) -> String {
    let platform = current_platform_id();
    if !fish_supported(platform) {
        return "Fish Speech wird auf diesem System nicht unterstützt. \
                Unter Modelle > Vorlesestimmen lässt sich stattdessen Piper einrichten."
            .to_string();
    }
    format!(
        "Fish Speech ist nicht eingerichtet (im Ordner '{}' fehlt: {}). \
         Ohne Fish Speech liest die einfache Piper-Stimme vor: \
         unter Modelle > Vorlesestimmen einrichten und im Stimmen-Menü wählen.",
        dir.display(),
        fish_missing(dir, platform).join(", ")
    )
}

/// Zustand einer Laufzeit fuer die Oberflaeche.
#[derive(Debug, Clone, Serialize, Type)]
pub struct RuntimeState {
    /// Gibt es diese Laufzeit fuer die Plattform/Architektur ueberhaupt?
    pub supported: bool,
    /// Vollstaendig vorhanden und startfaehig.
    pub ready: bool,
    /// Fehlende Dateien (leer, wenn bereit oder nicht unterstuetzt).
    pub missing: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Type)]
pub struct TtsRuntimeStatus {
    /// `windows-x64`, `macos-aarch64`, ... (wie im Katalog).
    pub platform: String,
    pub piper: RuntimeState,
    pub fish: RuntimeState,
    /// Ordner, in dem Fish erwartet wird (Einstellung `tts_fish_dir`).
    pub fish_dir: String,
}

/// Zustand beider Laufzeiten. `app_data`: Datenordner der App (Piper liegt
/// unter `tts/piper/<plattform>`), `fish_dir`: Fish-Ordner aus den Einstellungen.
pub fn runtime_status(app_data: &Path, fish_dir: &Path, platform: &str) -> TtsRuntimeStatus {
    let piper_dir = app_data.join("tts").join("piper").join(platform);
    let piper_ok = piper_supported(platform);
    let piper_missing_files = if piper_ok {
        piper_missing(&piper_dir, platform)
    } else {
        Vec::new()
    };
    let fish_ok = fish_supported(platform);
    let fish_missing_files = if fish_ok {
        fish_missing(fish_dir, platform)
    } else {
        Vec::new()
    };
    TtsRuntimeStatus {
        platform: platform.to_string(),
        piper: RuntimeState {
            supported: piper_ok,
            ready: piper_ok && piper_missing_files.is_empty(),
            missing: piper_missing_files,
        },
        fish: RuntimeState {
            supported: fish_ok,
            ready: fish_ok && fish_missing_files.is_empty(),
            missing: fish_missing_files,
        },
        fish_dir: fish_dir.to_string_lossy().into_owned(),
    }
}

/// Plattformkennung dieses Builds (Teil des Ablage-Vertrags mit dem Katalog).
pub fn current_platform_id() -> &'static str {
    if cfg!(target_os = "windows") {
        "windows-x64"
    } else if cfg!(target_os = "macos") {
        if cfg!(target_arch = "aarch64") {
            "macos-aarch64"
        } else {
            "macos-x64"
        }
    } else {
        "linux-x64"
    }
}

/// Vollstaendige Piper-Laufzeit als Testfixture; nie in Produktion.
#[cfg(test)]
pub fn write_piper_fixture(dir: &Path, platform: &str) {
    for name in piper_required(platform) {
        let path = dir.join(&name);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, b"fixture").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn touch(dir: &Path, name: &str) {
        let path = dir.join(name);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, b"fixture").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
    }

    #[test]
    fn leere_piper_laufzeit_ist_nicht_bereit_und_nennt_alles_fehlende() {
        let d = TempDir::new().unwrap();
        assert!(!piper_ready(d.path(), "windows-x64"));
        let missing = piper_missing(d.path(), "windows-x64");
        assert!(missing.contains(&"piper.exe".to_string()));
        assert!(missing.contains(&"espeak-ng.dll".to_string()));
        assert!(missing.contains(&"espeak-ng-data/phontab".to_string()));
    }

    #[test]
    fn fehlende_dll_macht_die_windows_laufzeit_unbrauchbar() {
        let d = TempDir::new().unwrap();
        write_piper_fixture(d.path(), "windows-x64");
        assert!(piper_ready(d.path(), "windows-x64"));
        std::fs::remove_file(d.path().join("onnxruntime.dll")).unwrap();
        assert!(!piper_ready(d.path(), "windows-x64"));
        assert_eq!(
            piper_missing(d.path(), "windows-x64"),
            vec!["onnxruntime.dll".to_string()]
        );
    }

    #[test]
    fn leere_datei_zaehlt_als_fehlend() {
        let d = TempDir::new().unwrap();
        write_piper_fixture(d.path(), "windows-x64");
        std::fs::write(d.path().join("espeak-ng.dll"), b"").unwrap();
        assert!(!piper_ready(d.path(), "windows-x64"));
    }

    /// Das offizielle macOS-Archiv (rhasspy/piper 2023.11.14-2) enthaelt nur
    /// `piper`, `piper_phonemize`, die Daten und ein dSYM -- keine der drei
    /// dylibs. Genau das meldet das Issue (`libespeak-ng.1.dylib` fehlt).
    #[test]
    fn das_offizielle_macos_archiv_ohne_dylibs_ist_nicht_bereit() {
        let d = TempDir::new().unwrap();
        touch(d.path(), "piper");
        touch(d.path(), "espeak-ng-data/phontab");
        touch(d.path(), "espeak-ng-data/phondata");
        assert!(!piper_ready(d.path(), "macos-x64"));
        assert!(piper_missing(d.path(), "macos-x64").contains(&"libespeak-ng.1.dylib".to_string()));
        write_piper_fixture(d.path(), "macos-x64");
        assert!(piper_ready(d.path(), "macos-x64"));
    }

    #[test]
    fn fish_braucht_python_skript_und_gewichte() {
        let d = TempDir::new().unwrap();
        assert!(!fish_ready(d.path(), "windows-x64"));
        assert_eq!(
            fish_missing(d.path(), "windows-x64"),
            vec![
                ".venv/Scripts/python.exe".to_string(),
                "tools/api_server.py".to_string(),
                "checkpoints".to_string()
            ]
        );
        touch(d.path(), ".venv/Scripts/python.exe");
        touch(d.path(), "tools/api_server.py");
        assert!(
            !fish_ready(d.path(), "windows-x64"),
            "ohne Gewichte nicht startfaehig"
        );
        touch(d.path(), "checkpoints/s2-pro/codec.pth");
        assert!(fish_ready(d.path(), "windows-x64"));
    }

    #[test]
    fn fish_ist_auf_intel_macs_nicht_unterstuetzt_auch_bei_voller_installation() {
        let d = TempDir::new().unwrap();
        touch(d.path(), ".venv/bin/python");
        touch(d.path(), "tools/api_server.py");
        touch(d.path(), "checkpoints/s2-pro/codec.pth");
        assert!(fish_ready(d.path(), "macos-aarch64"));
        assert!(!fish_ready(d.path(), "macos-x64"));
    }

    #[test]
    fn runtime_status_fasst_beide_laufzeiten_zusammen() {
        let data = TempDir::new().unwrap();
        let fish = TempDir::new().unwrap();
        let none = runtime_status(data.path(), fish.path(), "windows-x64");
        assert!(none.piper.supported && !none.piper.ready);
        assert!(none.fish.supported && !none.fish.ready);
        assert!(!none.piper.missing.is_empty() && !none.fish.missing.is_empty());

        write_piper_fixture(
            &data.path().join("tts").join("piper").join("windows-x64"),
            "windows-x64",
        );
        touch(fish.path(), ".venv/Scripts/python.exe");
        touch(fish.path(), "tools/api_server.py");
        touch(fish.path(), "checkpoints/s2-pro/codec.pth");
        let both = runtime_status(data.path(), fish.path(), "windows-x64");
        assert!(both.piper.ready && both.fish.ready);
        assert!(both.piper.missing.is_empty() && both.fish.missing.is_empty());
    }

    #[test]
    fn die_fish_meldung_nennt_fehlendes_und_den_piper_weg_ohne_entwicklerbericht() {
        let d = TempDir::new().unwrap();
        let msg = fish_not_set_up_message(d.path());
        assert!(
            msg.contains("nicht eingerichtet") || msg.contains("nicht unterstützt"),
            "{msg}"
        );
        assert!(msg.contains("Piper"), "{msg}");
        assert!(!msg.contains("INSTALL-REPORT"), "{msg}");
        assert!(!msg.contains("C:\\AI"), "{msg}");
        if cfg!(target_os = "windows") {
            assert!(msg.contains("tools/api_server.py"), "{msg}");
        }
    }

    /// Nur lesend: den Zustand einer ECHTEN Installation ausgeben (Windows-Pruefung
    /// zu Issue #29). `LVA_AVAIL_DATA` = App-Datenordner, `LVA_AVAIL_FISH` = Fish-Ordner.
    #[test]
    #[ignore = "liest eine echte Installation (Umgebungsvariablen LVA_AVAIL_DATA/LVA_AVAIL_FISH)"]
    fn status_of_a_real_installation() {
        let (Some(data), Some(fish)) = (
            std::env::var_os("LVA_AVAIL_DATA"),
            std::env::var_os("LVA_AVAIL_FISH"),
        ) else {
            return;
        };
        let status = runtime_status(Path::new(&data), Path::new(&fish), current_platform_id());
        eprintln!("REAL {status:?}");
        assert!(status.piper.ready, "{status:?}");
    }

    #[test]
    fn nicht_unterstuetzte_plattform_meldet_keine_fehlenden_dateien() {
        let d = TempDir::new().unwrap();
        let s = runtime_status(d.path(), d.path(), "macos-x64");
        assert!(!s.fish.supported && !s.fish.ready && s.fish.missing.is_empty());
        let s = runtime_status(d.path(), d.path(), "freebsd-x64");
        assert!(!s.piper.supported && !s.piper.ready);
    }
}
