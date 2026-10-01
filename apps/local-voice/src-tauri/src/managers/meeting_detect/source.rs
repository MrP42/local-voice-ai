//! Quelle der Mikrofonnutzung (M5 F16): welche Programme haben das Mikrofon
//! offen? Windows fuehrt darueber Buch (`CapabilityAccessManager\ConsentStore`),
//! lesbar ohne Administratorrechte und ohne das Mikrofon selbst anzufassen.
//!
//! Aufbau (V1, V5): unter `...\ConsentStore\microphone` liegt je Store-App ein
//! Unterschluessel (`MSTeams_8wekyb3d8bbwe`) und unter `NonPackaged` je
//! klassischem Programm einer, dessen Name der Pfad mit `#` statt `\` ist. Die
//! Werte `LastUsedTimeStart` und `LastUsedTimeStop` sind FILETIME (100 ns seit
//! 1601). Aktiv heisst `Stop == 0`.
//!
//! Kosten: ein Durchlauf liest rund hundert Schluessel, gemessen 1,6 ms
//! (V4); keine COM-Objekte, keine Threads.

use std::path::PathBuf;

/// Ein Registry-Eintrag mit Mikrofonnutzung.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MicUsage {
    /// `np:<Pfad mit #>` oder `pkg:<Paketfamilie>`.
    pub app_key: String,
    /// Nur bei klassischen Programmen (fuer die Pruefung, ob der Prozess laeuft).
    pub exe_path: Option<PathBuf>,
    /// `LastUsedTimeStart` als FILETIME.
    pub last_start: u64,
    /// `LastUsedTimeStop == 0` (und `Start != 0`).
    pub active: bool,
}

/// Liefert den aktuellen Stand. Der Fehlertext ist fuer das Log gedacht.
pub trait MicUsageSource: Send {
    fn snapshot(&mut self) -> Result<Vec<MicUsage>, String>;
}

/// FILETIME (100-ns-Schritte seit 1601-01-01) in Unix-Millisekunden.
pub fn filetime_to_unix_ms(filetime: u64) -> u64 {
    (filetime / 10_000).saturating_sub(11_644_473_600_000)
}

/// Unix-Millisekunden in FILETIME (fuer Tests und Diagnose).
pub fn unix_ms_to_filetime(ms: u64) -> u64 {
    (ms + 11_644_473_600_000) * 10_000
}

/// Die Registry als Quelle (nur Windows; sonst immer ein Fehler, damit die
/// Einstellung "auf diesem System nicht verfuegbar" zeigt).
#[derive(Default)]
pub struct RegistrySource;

impl RegistrySource {
    pub fn new() -> Self {
        Self
    }
}

#[cfg(windows)]
const BASE: &str =
    r"Software\Microsoft\Windows\CurrentVersion\CapabilityAccessManager\ConsentStore\microphone";

#[cfg(windows)]
impl MicUsageSource for RegistrySource {
    fn snapshot(&mut self) -> Result<Vec<MicUsage>, String> {
        use winreg::enums::HKEY_CURRENT_USER;
        use winreg::RegKey;

        fn read(key: &RegKey, app_key: String, exe: Option<PathBuf>) -> Option<MicUsage> {
            let start: u64 = key.get_value("LastUsedTimeStart").ok()?;
            let stop: u64 = key.get_value("LastUsedTimeStop").ok()?;
            Some(MicUsage {
                app_key,
                exe_path: exe,
                last_start: start,
                active: stop == 0 && start != 0,
            })
        }

        let root = RegKey::predef(HKEY_CURRENT_USER)
            .open_subkey(BASE)
            .map_err(|e| format!("Mikrofon-Nutzungsprotokoll nicht lesbar: {e}"))?;
        let mut out = Vec::new();
        for name in root.enum_keys().flatten() {
            let Ok(sub) = root.open_subkey(&name) else {
                continue;
            };
            if name == "NonPackaged" {
                for exe_name in sub.enum_keys().flatten() {
                    let Ok(entry) = sub.open_subkey(&exe_name) else {
                        continue;
                    };
                    let path = PathBuf::from(exe_name.replace('#', "\\"));
                    if let Some(usage) = read(&entry, format!("np:{exe_name}"), Some(path)) {
                        out.push(usage);
                    }
                }
            } else if let Some(usage) = read(&sub, format!("pkg:{name}"), None) {
                out.push(usage);
            }
        }
        Ok(out)
    }
}

#[cfg(not(windows))]
impl MicUsageSource for RegistrySource {
    fn snapshot(&mut self) -> Result<Vec<MicUsage>, String> {
        Err("Mikrofon-Erkennung gibt es nur unter Windows".to_string())
    }
}

/// Test-Quelle: liefert die eingestellten Snapshots der Reihe nach, danach den
/// letzten immer wieder.
#[cfg(test)]
pub struct FakeSource {
    pub steps: Vec<Result<Vec<MicUsage>, String>>,
    pub next: usize,
}

#[cfg(test)]
impl MicUsageSource for FakeSource {
    fn snapshot(&mut self) -> Result<Vec<MicUsage>, String> {
        let i = self.next.min(self.steps.len().saturating_sub(1));
        self.next += 1;
        self.steps.get(i).cloned().unwrap_or_else(|| Ok(Vec::new()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filetime_round_trips_through_unix_ms() {
        let ms = 1_790_000_000_123;
        assert_eq!(filetime_to_unix_ms(unix_ms_to_filetime(ms)), ms);
        assert_eq!(filetime_to_unix_ms(0), 0, "vor 1970 bleibt es bei 0");
    }

    #[test]
    fn fake_source_repeats_its_last_step() {
        let mut src = FakeSource {
            steps: vec![Ok(vec![]), Err("x".into())],
            next: 0,
        };
        assert!(src.snapshot().unwrap().is_empty());
        assert!(src.snapshot().is_err());
        assert!(src.snapshot().is_err());
    }

    #[cfg(windows)]
    #[test]
    fn registry_source_reads_this_machines_consent_store() {
        // Read-only smoke test: the key exists on every Windows 10/11 profile
        // that has ever used a microphone; where it does not, the error must be
        // a readable message instead of a panic.
        match RegistrySource::new().snapshot() {
            Ok(rows) => {
                for row in rows {
                    assert!(row.app_key.starts_with("np:") || row.app_key.starts_with("pkg:"));
                }
            }
            Err(e) => assert!(e.contains("nicht lesbar"), "{e}"),
        }
    }
}
