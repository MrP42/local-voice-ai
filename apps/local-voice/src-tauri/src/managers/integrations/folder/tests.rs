use super::*;

fn sandbox() -> (tempfile::TempDir, Sandbox) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("Ablage");
    std::fs::create_dir(&root).unwrap();
    let sb = Sandbox::open(root.to_str().unwrap()).unwrap();
    (dir, sb)
}

/// Legt eine Junction (Windows) oder einen Symlink (sonst) `link` -> `target` an.
fn make_link(link: &Path, target: &Path) -> bool {
    #[cfg(windows)]
    {
        std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(link)
            .arg(target)
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(target, link).is_ok()
    }
}

#[test]
fn parent_directory_escapes_are_refused() {
    let (_d, sb) = sandbox();
    for rel in [
        "../x.md",
        "a/../../x.md",
        "a\\..\\..\\x.md",
        "..",
        "./../x.md",
        "sub/../../außen.md",
    ] {
        let err = sb.resolve_file(rel, true).unwrap_err();
        assert!(matches!(err, FolderError::Escape(_)), "{rel}: {err:?}");
    }
}

#[test]
fn absolute_drive_and_unc_paths_are_refused() {
    let (_d, sb) = sandbox();
    for rel in [
        "C:\\Windows\\x.md",
        "C:x.md",
        "/etc/passwd",
        "\\\\server\\share\\x.md",
        "\\x.md",
        "//server/share/x.md",
    ] {
        let err = sb.resolve_file(rel, true).unwrap_err();
        assert!(matches!(err, FolderError::Escape(_)), "{rel}: {err:?}");
    }
}

#[test]
fn streams_device_names_and_trailing_dots_are_refused() {
    let (_d, sb) = sandbox();
    for rel in [
        "x.md:stream",
        "CON",
        "nul.txt",
        "sub/COM1.md",
        "lpt9",
        "endet.",
        "endet ",
        "a<b.md",
        "a|b.md",
        "frage?.md",
    ] {
        assert!(
            sb.resolve_file(rel, true).is_err(),
            "{rel} wurde nicht abgelehnt"
        );
    }
    // Nichts davon hat etwas angelegt.
    assert_eq!(std::fs::read_dir(sb.root()).unwrap().count(), 0);
}

#[test]
fn a_nested_path_is_created_inside_the_root_only() {
    let (_d, sb) = sandbox();
    let placed = sb
        .write_bytes_new("Protokolle/2026/Q3", "Sprint.md", b"# Hallo\n")
        .unwrap();
    assert_eq!(placed.rel, "Protokolle/2026/Q3/Sprint.md");
    assert!(sb.contains(&placed.path.canonicalize().unwrap()));
    assert_eq!(std::fs::read(&placed.path).unwrap(), b"# Hallo\n");
    assert_eq!(placed.bytes, 8);
}

#[test]
fn an_existing_file_is_never_overwritten() {
    let (_d, sb) = sandbox();
    let a = sb.write_bytes_new("", "Bericht.md", b"eins").unwrap();
    let b = sb.write_bytes_new("", "Bericht.md", b"zwei").unwrap();
    let c = sb.write_bytes_new("", "Bericht.md", b"drei").unwrap();
    assert_eq!(a.rel, "Bericht.md");
    assert_eq!(b.rel, "Bericht (2).md");
    assert_eq!(c.rel, "Bericht (3).md");
    assert_eq!(std::fs::read(&a.path).unwrap(), b"eins");
    assert_eq!(std::fs::read(&b.path).unwrap(), b"zwei");
}

#[test]
fn file_names_from_titles_are_made_safe() {
    assert_eq!(
        sanitize_file_name("Q3: Planung / Budget?").as_deref(),
        Some("Q3_ Planung _ Budget_")
    );
    assert_eq!(
        sanitize_file_name("..\\..\\boese.md").as_deref(),
        Some("_.._boese.md")
    );
    assert_eq!(sanitize_file_name("  ...  "), None);
    assert_eq!(sanitize_file_name(""), None);
    assert_eq!(sanitize_file_name("NUL").as_deref(), Some("_NUL"));
    assert_eq!(sanitize_file_name("Notiz. ").as_deref(), Some("Notiz"));
    let long = "ä".repeat(300);
    assert_eq!(
        sanitize_file_name(&long).unwrap().chars().count(),
        MAX_COMPONENT_CHARS
    );
}

#[test]
fn a_hostile_title_cannot_leave_the_folder() {
    let (d, sb) = sandbox();
    let placed = sb.write_bytes_new("", "..\\..\\außen.md", b"x").unwrap();
    assert!(sb.contains(&placed.path.canonicalize().unwrap()));
    assert!(!d.path().join("außen.md").exists());
}

#[test]
fn a_failing_writer_leaves_no_file_behind() {
    let (_d, sb) = sandbox();
    let err = sb
        .write_new("", "Kaputt.pdf", |_| Err("PDF nicht verfügbar".to_string()))
        .unwrap_err();
    assert_eq!(err, FolderError::Io("PDF nicht verfügbar".to_string()));
    assert_eq!(std::fs::read_dir(sb.root()).unwrap().count(), 0);
}

#[test]
fn an_oversized_result_is_removed() {
    let (_d, sb) = sandbox();
    // Der Schreiber legt eine sparse Datei jenseits der Grenze an.
    let err = sb
        .write_new("", "Riese.bin", |p| {
            let f = OpenOptions::new()
                .write(true)
                .open(p)
                .map_err(|e| e.to_string())?;
            f.set_len(MAX_FILE_BYTES + 1).map_err(|e| e.to_string())
        })
        .unwrap_err();
    assert_eq!(err, FolderError::TooLarge);
    assert_eq!(std::fs::read_dir(sb.root()).unwrap().count(), 0);
}

#[test]
fn a_link_pointing_outside_is_refused_and_nothing_leaks() {
    let (d, sb) = sandbox();
    let outside = d.path().join("außerhalb");
    std::fs::create_dir(&outside).unwrap();
    let link = sb.root().join("verknuepft");
    if !make_link(&link, &outside) {
        eprintln!("Verknüpfung nicht anlegbar (fehlende Rechte): Test übersprungen");
        return;
    }
    let err = sb.resolve_file("verknuepft/x.md", true).unwrap_err();
    assert_eq!(err, FolderError::Escape("Verknüpfung nach außen"));
    let err = sb
        .write_bytes_new("verknuepft", "x.md", b"geheim")
        .unwrap_err();
    assert_eq!(err, FolderError::Escape("Verknüpfung nach außen"));
    assert_eq!(std::fs::read_dir(&outside).unwrap().count(), 0);
}

#[test]
fn a_link_staying_inside_is_harmless_and_allowed() {
    let (_d, sb) = sandbox();
    let real = sb.root().join("echt");
    std::fs::create_dir(&real).unwrap();
    let link = sb.root().join("kurz");
    if !make_link(&link, &real) {
        eprintln!("Verknüpfung nicht anlegbar (fehlende Rechte): Test übersprungen");
        return;
    }
    let placed = sb.write_bytes_new("kurz", "x.md", b"ok").unwrap();
    assert!(sb.contains(&placed.path));
    assert!(real.join("x.md").exists());
}

#[test]
fn a_sibling_folder_sharing_the_prefix_is_outside() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("Ablage");
    let evil = dir.path().join("Ablage-boese");
    std::fs::create_dir(&root).unwrap();
    std::fs::create_dir(&evil).unwrap();
    let sb = Sandbox::open(root.to_str().unwrap()).unwrap();
    assert!(!sb.contains(&evil.canonicalize().unwrap()));
    assert!(sb.contains(&root.canonicalize().unwrap().join("x")));
    let link = root.join("zum-nachbarn");
    if make_link(&link, &evil) {
        assert!(sb.resolve_dir("zum-nachbarn", false).is_err());
    }
}

#[cfg(unix)]
#[test]
fn a_symlinked_file_is_never_overwritten() {
    let (d, sb) = sandbox();
    let outside = d.path().join("fremd.txt");
    std::fs::write(&outside, "fremd").unwrap();
    std::os::unix::fs::symlink(&outside, sb.root().join("Bericht.md")).unwrap();
    assert!(sb.resolve_file("Bericht.md", false).is_err());
    let placed = sb.write_bytes_new("", "Bericht.md", b"neu").unwrap();
    assert_eq!(placed.rel, "Bericht (2).md");
    assert_eq!(std::fs::read_to_string(&outside).unwrap(), "fremd");
}

#[test]
fn the_root_must_be_absolute_present_and_a_folder() {
    assert_eq!(Sandbox::open("").unwrap_err(), FolderError::RootMissing);
    assert_eq!(
        Sandbox::open("relativ\\ordner").unwrap_err(),
        FolderError::RootRelative
    );
    let dir = tempfile::tempdir().unwrap();
    let missing = dir.path().join("gibt-es-nicht");
    assert_eq!(
        Sandbox::open(missing.to_str().unwrap()).unwrap_err(),
        FolderError::RootNotFound
    );
    let file = dir.path().join("datei.txt");
    std::fs::write(&file, "x").unwrap();
    assert_eq!(
        Sandbox::open(file.to_str().unwrap()).unwrap_err(),
        FolderError::RootNotAFolder
    );
}

#[test]
fn replace_bytes_swaps_atomically_and_keeps_the_name() {
    let (_d, sb) = sandbox();
    let placed = sb.write_bytes_new("", "Notiz.md", b"alt").unwrap();
    let again = sb.replace_bytes(&placed.path, b"neu und laenger").unwrap();
    assert_eq!(again.rel, "Notiz.md");
    assert_eq!(std::fs::read(&placed.path).unwrap(), b"neu und laenger");
    // Keine Reste der Zwischendatei.
    let names: Vec<_> = std::fs::read_dir(sb.root())
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
        .collect();
    assert_eq!(names, vec!["Notiz.md".to_string()]);
}

#[test]
fn replace_bytes_refuses_a_path_outside_the_root() {
    let (d, sb) = sandbox();
    let outside = d.path().join("fremd.md");
    std::fs::write(&outside, "fremd").unwrap();
    let err = sb.replace_bytes(&outside, b"ueberschrieben").unwrap_err();
    assert_eq!(err, FolderError::Escape("Verknüpfung nach außen"));
    assert_eq!(std::fs::read_to_string(&outside).unwrap(), "fremd");
}

#[test]
fn config_json_is_read_with_defaults() {
    let c = FolderConfig::from_config_json(r#"{"path":"C:\\Ablage"}"#).unwrap();
    assert_eq!(c.path, "C:\\Ablage");
    assert_eq!(c.subfolder, "");
    let c =
        FolderConfig::from_config_json(r#"{"path":" D:/x ","subfolder":" Protokolle "}"#).unwrap();
    assert_eq!(c.path, "D:/x");
    assert_eq!(c.subfolder, "Protokolle");
    assert_eq!(
        FolderConfig::from_config_json("{}").unwrap_err(),
        FolderError::RootMissing
    );
    assert_eq!(
        FolderConfig::from_config_json("kaputt").unwrap_err(),
        FolderError::Config
    );
}
