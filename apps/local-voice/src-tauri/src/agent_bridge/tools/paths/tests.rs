use super::*;

fn bad(raw: &str) -> PathError {
    check_syntax(raw).expect_err(&format!("{raw:?} haette abgewiesen werden muessen"))
}

#[test]
fn an_ordinary_local_media_path_passes_the_syntax_check() {
    for ok in [
        r"C:\Aufnahmen\Besprechung.wav",
        "C:/Aufnahmen/Besprechung.MP3",
        r"d:\a\b\c\video.mp4",
        r"C:\Ordner mit Leerzeichen\Ünïcode ß.m4a",
    ] {
        assert!(check_syntax(ok).is_ok(), "{ok}");
    }
}

#[test]
fn unc_and_device_paths_are_refused() {
    assert_eq!(bad(r"\\server\freigabe\a.wav"), PathError::Network);
    assert_eq!(bad("//server/freigabe/a.wav"), PathError::Network);
    assert_eq!(bad(r"\\.\pipe\x.wav"), PathError::DevicePath);
    assert_eq!(bad(r"\\?\C:\a.wav"), PathError::DevicePath);
    assert_eq!(bad(r"\\?\UNC\server\a\b.wav"), PathError::DevicePath);
    assert_eq!(bad("//./COM1/a.wav"), PathError::DevicePath);
}

#[test]
fn relative_and_drive_relative_paths_are_refused() {
    for raw in [
        "a.wav",
        r"..\a.wav",
        r"\Windows\a.wav",
        "C:a.wav",
        "/etc/a.wav",
        "~/a.wav",
    ] {
        assert_eq!(bad(raw), PathError::NotAbsolute, "{raw}");
    }
}

#[test]
fn parent_directory_parts_are_refused_even_if_they_would_stay_inside() {
    assert_eq!(
        bad(r"C:\Aufnahmen\..\Aufnahmen\a.wav"),
        PathError::Traversal
    );
    assert_eq!(bad(r"C:\..\a.wav"), PathError::Traversal);
    assert_eq!(bad("C:/a/../b.wav"), PathError::Traversal);
}

#[test]
fn alternate_data_streams_and_colons_are_refused() {
    assert_eq!(bad(r"C:\a.wav:strom"), PathError::StreamSyntax);
    assert_eq!(bad(r"C:\a.wav::$DATA"), PathError::StreamSyntax);
    assert_eq!(bad(r"C:\dir:x\a.wav"), PathError::StreamSyntax);
}

#[test]
fn reserved_device_names_are_refused_with_and_without_extension() {
    for raw in [
        r"C:\CON",
        r"C:\a\NUL.wav",
        r"C:\a\com1.mp3",
        r"C:\a\LPT9.wav",
        r"C:\a\Aux.txt.wav",
        r"C:\CONIN$.wav",
    ] {
        assert!(matches!(bad(raw), PathError::ReservedName(_)), "{raw}");
    }
    // Aehnlich, aber kein Geraet.
    assert!(check_syntax(r"C:\a\console.wav").is_ok());
    assert!(check_syntax(r"C:\a\com10.wav").is_ok());
}

#[test]
fn wildcards_trailing_dots_and_spaces_are_refused() {
    for raw in [
        r"C:\a\*.wav",
        r"C:\a\b?.wav",
        r"C:\a\<b>.wav",
        "C:\\a\\x\".wav",
        r"C:\a\x|y.wav",
    ] {
        assert_eq!(bad(raw), PathError::BadName, "{raw}");
    }
    assert_eq!(bad(r"C:\a\b.wav."), PathError::BadName);
    assert_eq!(bad(r"C:\a. \b.wav"), PathError::BadName);
}

#[test]
fn control_and_invisible_characters_are_refused() {
    for raw in [
        "C:\\a\\b\u{0}.wav",
        "C:\\a\\b\n.wav",
        "C:\\a\\\u{202E}vaw.exe",
        "C:\\a\\\u{200B}b.wav",
    ] {
        assert_eq!(bad(raw), PathError::BadCharacters, "{raw:?}");
    }
}

#[test]
fn only_audio_and_video_formats_are_accepted() {
    assert_eq!(bad(r"C:\a\b.exe"), PathError::BadExtension("exe".into()));
    assert_eq!(
        bad(r"C:\a\b.wav.exe"),
        PathError::BadExtension("exe".into())
    );
    assert_eq!(bad(r"C:\a\b"), PathError::BadExtension(String::new()));
    assert_eq!(bad(r"C:\a\.wav"), PathError::BadExtension(String::new()));
    assert_eq!(bad(r"C:\a\b.docx"), PathError::BadExtension("docx".into()));
    assert!(check_syntax(r"C:\a\b.WEBM").is_ok());
}

#[test]
fn empty_and_overlong_paths_are_refused() {
    assert_eq!(bad(""), PathError::Empty);
    assert_eq!(bad("   "), PathError::Empty);
    let long = format!(r"C:\{}\a.wav", "x".repeat(600));
    assert_eq!(bad(&long), PathError::TooLong);
    assert_eq!(bad(r"C:\"), PathError::NotAFile);
}

#[test]
fn the_verbatim_prefix_is_removed_and_network_targets_are_refused() {
    assert_eq!(
        strip_verbatim(Path::new(r"\\?\C:\a\b.wav")).unwrap(),
        PathBuf::from(r"C:\a\b.wav")
    );
    assert_eq!(
        strip_verbatim(Path::new(r"\\?\UNC\srv\share\b.wav")),
        Err(PathError::Network)
    );
    assert_eq!(
        strip_verbatim(Path::new(r"\\?\unc\srv\share\b.wav")),
        Err(PathError::Network)
    );
    assert_eq!(
        strip_verbatim(Path::new(r"\\?\GLOBALROOT\Device\x")),
        Err(PathError::DevicePath)
    );
    assert_eq!(
        strip_verbatim(Path::new(r"\\srv\share\b.wav")),
        Err(PathError::Network)
    );
}

#[cfg(windows)]
mod on_disk {
    use super::*;

    fn temp() -> tempfile::TempDir {
        tempfile::tempdir().unwrap()
    }

    fn check(path: &Path) -> Result<PathBuf, PathError> {
        check_media_path(&path.to_string_lossy())
    }

    #[test]
    fn an_existing_file_resolves_to_a_plain_drive_path() {
        let dir = temp();
        let file = dir.path().join("a.wav");
        std::fs::write(&file, b"RIFF....WAVE").unwrap();
        let resolved = check(&file).unwrap();
        assert!(
            !resolved.to_string_lossy().starts_with(r"\\"),
            "{resolved:?}"
        );
        assert!(resolved.is_file());
    }

    #[test]
    fn missing_empty_and_directory_targets_are_refused() {
        let dir = temp();
        assert_eq!(
            check(&dir.path().join("nicht.wav")),
            Err(PathError::NotFound)
        );
        let empty = dir.path().join("leer.wav");
        std::fs::write(&empty, b"").unwrap();
        assert_eq!(check(&empty), Err(PathError::EmptyFile));
        let as_dir = dir.path().join("ordner.wav");
        std::fs::create_dir(&as_dir).unwrap();
        assert_eq!(check(&as_dir), Err(PathError::NotAFile));
    }

    #[test]
    fn a_file_over_the_size_limit_is_refused() {
        let dir = temp();
        let file = dir.path().join("gross.wav");
        std::fs::write(&file, vec![0u8; 2048]).unwrap();
        let syntax = check_syntax(&file.to_string_lossy()).unwrap();
        assert_eq!(resolve_with(&syntax, 1024), Err(PathError::TooLarge(2048)));
        assert!(resolve_with(&syntax, 4096).is_ok());
    }

    #[test]
    fn a_link_to_a_forbidden_format_is_refused_after_resolving() {
        // Eine Datei `harmlos.wav`, die in Wahrheit eine `.exe` ist (Hardlink oder Kopie
        // taeuscht nur den Namen vor): der aufgeloeste Name zaehlt bei Verknuepfungen.
        let dir = temp();
        let target = dir.path().join("programm.exe");
        std::fs::write(&target, b"MZ").unwrap();
        let link = dir.path().join("harmlos.wav");
        if std::os::windows::fs::symlink_file(&target, &link).is_err() {
            // Ohne Recht fuer symbolische Links (kein Entwicklermodus) ist der Fall nicht pruefbar.
            return;
        }
        assert_eq!(check(&link), Err(PathError::BadExtension("exe".into())));
    }

    #[test]
    fn a_junction_to_another_local_folder_is_followed_to_a_local_target() {
        let dir = temp();
        let real = dir.path().join("echt");
        std::fs::create_dir(&real).unwrap();
        std::fs::write(real.join("a.wav"), b"RIFF....WAVE").unwrap();
        let junction = dir.path().join("verweis");
        let ok = std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(&junction)
            .arg(&real)
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);
        if !ok {
            return;
        }
        let resolved = check(&junction.join("a.wav")).unwrap();
        assert!(resolved.to_string_lossy().contains("echt"), "{resolved:?}");
    }
}
