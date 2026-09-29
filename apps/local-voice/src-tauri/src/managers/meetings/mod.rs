pub mod chunker;
pub mod echo;
pub mod export;
pub mod import;
pub mod mic_capture;
pub mod minutes;
pub mod notes;
pub mod recorder;
pub mod retention;
pub mod retranscribe;
pub mod stats;
pub mod store;
pub mod subtitle;

use std::path::{Path, PathBuf};

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
