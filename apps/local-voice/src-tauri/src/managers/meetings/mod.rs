pub mod chat; // M4-P4c
pub mod chunker;
pub mod diarize; // M3-P3a
pub mod dsp;
pub mod echo;
pub mod export;
pub mod final_pass;
pub mod followup; // P6f (B13)
pub mod hallucination;
pub mod import;
pub mod llm_call;
pub mod mail; // M6-P6c
pub mod mic_capture;
pub mod minutes;
pub mod notes;
pub mod pdf; // M6-P6b
pub mod recorder;
pub mod retention;
pub mod retranscribe;
pub mod search;
pub mod segmenter;
pub mod signal_watch; // M2-P2e
pub mod simulate;
pub mod speakers; // M3-P3b
pub mod stats;
pub mod store;
pub mod subtitle;

use std::path::{Path, PathBuf};

/// M2-P2c2: Dateiname der Mikrofonspur ohne Echo, neben `mic.wav` im
/// Besprechungsordner. Sie steht in keiner DB-Spalte (keine Migration, B5).
pub const MIC_AEC_FILE: &str = "mic_aec.wav";

/// Dateien, die aus einer Audiodatei der Besprechung abgeleitet sind und mit
/// ihr geloescht werden muessen (Aufbewahrung, Loeschen). Heute: `mic_aec.wav`
/// neben einer `mic.wav`. Pfad als Text, weil die Loeschwege Texte fuehren.
pub fn derived_audio_paths(path: &str) -> Vec<String> {
    let p = Path::new(path);
    if p.file_name().and_then(|n| n.to_str()) != Some("mic.wav") {
        return Vec::new();
    }
    p.with_file_name(MIC_AEC_FILE)
        .to_str()
        .map(|s| vec![s.to_string()])
        .unwrap_or_default()
}

/// Overrides the meetings data directory (DB + per-meeting audio folders).
/// Exists so the acceptance harness can run against a sandbox and NEVER
/// touches the productive meetings.db (M8 acceptance ruling: a test harness
/// must not be able to write production data). Models and all other app data
/// stay at their normal location on purpose — only the meetings store moves.
pub const MEETINGS_DIR_ENV: &str = "LVA_MEETINGS_DIR";

/// Pure decision behind `meetings_data_dir`: a non-empty override wins over
/// the default `<default_parent>/meetings`.
pub fn meetings_dir_from(env_override: Option<&str>, default_parent: &Path) -> PathBuf {
    match env_override.map(str::trim) {
        Some(dir) if !dir.is_empty() => PathBuf::from(dir),
        _ => default_parent.join("meetings"),
    }
}

/// The meetings data directory, honoring `LVA_MEETINGS_DIR`.
pub fn meetings_data_dir(app: &tauri::AppHandle) -> anyhow::Result<PathBuf> {
    let default_parent = crate::portable::app_data_dir(app)?;
    Ok(meetings_dir_from(
        std::env::var(MEETINGS_DIR_ENV).ok().as_deref(),
        &default_parent,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_echo_free_track_is_derived_only_from_a_live_mic_wav() {
        let mic = Path::new("C:/m/01ABC").join("mic.wav");
        let derived = derived_audio_paths(mic.to_str().unwrap());
        assert_eq!(
            derived,
            vec![Path::new("C:/m/01ABC")
                .join(MIC_AEC_FILE)
                .to_str()
                .unwrap()
                .to_string()]
        );
        assert!(derived_audio_paths("C:/m/01ABC/system.wav").is_empty());
        assert!(derived_audio_paths("C:/import/interview.wav").is_empty());
    }

    #[test]
    fn without_override_the_meetings_dir_lives_under_the_app_data_dir() {
        let d = meetings_dir_from(None, Path::new("C:/data"));
        assert_eq!(d, Path::new("C:/data").join("meetings"));
    }

    #[test]
    fn a_sandbox_override_replaces_the_directory_entirely() {
        let d = meetings_dir_from(Some("C:/tmp/harness"), Path::new("C:/data"));
        assert_eq!(d, PathBuf::from("C:/tmp/harness"));
    }

    #[test]
    fn empty_or_blank_overrides_are_ignored() {
        assert_eq!(
            meetings_dir_from(Some(""), Path::new("C:/data")),
            Path::new("C:/data").join("meetings")
        );
        assert_eq!(
            meetings_dir_from(Some("   "), Path::new("C:/data")),
            Path::new("C:/data").join("meetings")
        );
    }
}
