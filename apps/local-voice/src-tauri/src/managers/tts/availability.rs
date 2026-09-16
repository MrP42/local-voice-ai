//! Filesystem-only readiness shared by downloads, selection and synthesis.
//! No process starts or model loads while rendering settings.
use std::path::{Path, PathBuf};

pub fn piper_ready(dir: &Path, platform: &str) -> bool {
    let binary = if platform.starts_with("windows") {
        "piper.exe"
    } else {
        "piper"
    };
    executable(&dir.join(binary))
        && ["espeak-ng-data/phontab", "espeak-ng-data/phondata"]
            .iter()
            .chain(piper_libraries(platform).iter())
            .all(|name| nonempty_file(&dir.join(name)))
}

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

pub fn fish_dir(base: &Path, configured: &str, platform: &str) -> PathBuf {
    if configured.is_empty() || (platform != "windows-x64" && configured == r"C:\AI\fish-speech") {
        base.join("tts").join("fish")
    } else {
        PathBuf::from(configured)
    }
}

pub fn fish_python(dir: &Path, platform: &str) -> PathBuf {
    if platform.starts_with("windows") {
        dir.join(".venv").join("Scripts").join("python.exe")
    } else {
        dir.join(".venv").join("bin").join("python")
    }
}

pub fn fish_supported(platform: &str) -> bool {
    matches!(platform, "windows-x64" | "macos-aarch64" | "linux-x64")
}

pub fn fish_ready(dir: &Path, platform: &str) -> bool {
    fish_supported(platform)
        && executable(&fish_python(dir, platform))
        && [
            "tools/api_server.py",
            "checkpoints/s2-pro/model.pth",
            "checkpoints/s2-pro/codec.pth",
        ]
        .iter()
        .all(|name| nonempty_file(&dir.join(name)))
}

/// Complete fixture for engine/downloader tests; never used in production.
#[cfg(test)]
pub fn write_piper_fixture(dir: &Path, platform: &str) {
    let binary = if platform.starts_with("windows") {
        "piper.exe"
    } else {
        "piper"
    };
    for name in [binary, "espeak-ng-data/phontab", "espeak-ng-data/phondata"]
        .iter()
        .chain(piper_libraries(platform).iter())
    {
        let path = dir.join(name);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, b"fixture").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::atomic::{AtomicUsize, Ordering};
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "lva-tts-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }
        fn file(&self, name: &str) {
            let path = self.0.join(name);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(&path, b"fixture").unwrap();
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
            }
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn old_macos_archive_without_libraries_is_not_ready() {
        let f = Fixture::new();
        f.file("piper");
        f.file("espeak-ng-data/phontab");
        assert!(!piper_ready(&f.0, "macos-x64"));
    }
    #[test]
    fn missing_windows_dll_is_not_ready() {
        let f = Fixture::new();
        for name in [
            "piper.exe",
            "espeak-ng-data/phontab",
            "espeak-ng-data/phondata",
            "espeak-ng.dll",
            "piper_phonemize.dll",
            "onnxruntime.dll",
        ] {
            f.file(name);
        }
        assert!(!piper_ready(&f.0, "windows-x64"), "provider DLL is missing");
        f.file("onnxruntime_providers_shared.dll");
        assert!(piper_ready(&f.0, "windows-x64"));
    }
    #[test]
    fn complete_macos_runtime_requires_nonempty_data_and_libraries() {
        let f = Fixture::new();
        for name in [
            "piper",
            "espeak-ng-data/phontab",
            "espeak-ng-data/phondata",
            "libespeak-ng.1.dylib",
            "libpiper_phonemize.1.dylib",
            "libonnxruntime.1.14.1.dylib",
        ] {
            f.file(name);
        }
        assert!(piper_ready(&f.0, "macos-x64"));
        fs::write(f.0.join("libespeak-ng.1.dylib"), b"").unwrap();
        assert!(!piper_ready(&f.0, "macos-x64"));
    }
    #[test]
    fn foreign_default_fish_path_resolves_to_managed_data_without_changing_custom_paths() {
        assert_eq!(
            fish_dir(Path::new("/data"), r"C:\AI\fish-speech", "macos-x64"),
            Path::new("/data/tts/fish")
        );
        assert_eq!(
            fish_dir(Path::new("/data"), "/Volumes/TTS/Fish", "macos-aarch64"),
            Path::new("/Volumes/TTS/Fish")
        );
        assert_eq!(
            fish_dir(Path::new("/data"), r"C:\AI\fish-speech", "windows-x64"),
            PathBuf::from(r"C:\AI\fish-speech")
        );
    }
    #[test]
    fn fish_missing_weights_or_unsupported_architecture_is_not_ready() {
        let f = Fixture::new();
        f.file(".venv/bin/python");
        f.file("tools/api_server.py");
        assert!(!fish_ready(&f.0, "macos-aarch64"));
        f.file("checkpoints/s2-pro/model.pth");
        f.file("checkpoints/s2-pro/codec.pth");
        assert!(fish_ready(&f.0, "macos-aarch64"));
        assert!(!fish_ready(&f.0, "macos-x64"));
    }
}
