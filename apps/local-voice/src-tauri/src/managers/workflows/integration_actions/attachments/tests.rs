use std::path::{Path, PathBuf};

use serde_json::json;

use super::*;
use crate::managers::integrations::model::{IntegrationPatch, NewIntegration};
use crate::managers::workflows::test_support::{set_grant, Fx, T0};

fn temp_dir() -> PathBuf {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().to_path_buf();
    std::mem::forget(dir);
    p
}

fn folder(conn: &rusqlite::Connection, id: &str, dir: &Path) {
    let mut n = NewIntegration::new(Kind::Folder, id);
    n.id = Some(id.to_string());
    n.config = json!({ "path": dir.to_string_lossy() });
    integrations_store::create(conn, &n, T0).unwrap();
}

fn write(dir: &Path, name: &str, bytes: &[u8]) -> String {
    let p = dir.join(name);
    std::fs::write(&p, bytes).unwrap();
    p.to_string_lossy().to_string()
}

struct W {
    fx: Fx,
    root: PathBuf,
}

fn w() -> W {
    let fx = Fx::new();
    let root = temp_dir();
    folder(&fx.conn(), "folder-protokolle", &root);
    W { fx, root }
}

#[test]
fn a_file_inside_an_enabled_folder_is_checked_and_read_once() {
    let w = w();
    let path = write(&w.root, "Protokoll Überprüfung.docx", b"PK-inhalt");
    let got = check(&w.fx.conn(), std::slice::from_ref(&path)).unwrap();
    assert_eq!(got.len(), 1);
    let a = &got[0];
    assert_eq!(a.name, "Protokoll Überprüfung.docx");
    assert_eq!(a.size, 9);
    assert_eq!(a.bytes, b"PK-inhalt");
    assert_eq!(a.folder, "folder-protokolle");
    assert_eq!(
        a.read_mode,
        GrantMode::Allow,
        "Vorgabe: Lesen ist fuer Ablaeufe erlaubt"
    );
    assert_eq!(a.sha256.len(), 16);
    assert!(a.content_type.contains("wordprocessingml"));
    assert!(!a.display.starts_with(r"\\?\"), "{}", a.display);
    assert!(a.display.ends_with("Protokoll Überprüfung.docx"));
    // Der Inhalt steht nie im Debug-Text.
    assert!(!format!("{a:?}").contains("PK-inhalt"));
}

#[test]
fn the_checksum_follows_the_content() {
    let w = w();
    let path = write(&w.root, "a.txt", b"eins");
    let first = check(&w.fx.conn(), std::slice::from_ref(&path)).unwrap()[0]
        .sha256
        .clone();
    std::fs::write(&path, b"zwei").unwrap();
    let second = check(&w.fx.conn(), std::slice::from_ref(&path)).unwrap()[0]
        .sha256
        .clone();
    assert_ne!(
        first, second,
        "geaenderter Inhalt, andere Bindung der Freigabe"
    );
    let again = check(&w.fx.conn(), &[path]).unwrap()[0].sha256.clone();
    assert_eq!(second, again);
}

#[test]
fn a_file_outside_every_folder_is_refused() {
    let w = w();
    let elsewhere = temp_dir();
    let path = write(&elsewhere, "geheim.txt", b"x");
    assert_eq!(
        check(&w.fx.conn(), &[path]),
        Err(AttachError::Outside("geheim.txt".to_string()))
    );
}

#[test]
fn a_path_that_climbs_out_with_dotdot_is_refused() {
    let w = w();
    let elsewhere = temp_dir();
    write(&elsewhere, "geheim.txt", b"x");
    std::fs::create_dir_all(w.root.join("unter")).unwrap();
    // <wurzel>/unter/../../<anderer>/geheim.txt zeigt ausserhalb der Wurzel.
    let up = w
        .root
        .join("unter")
        .join("..")
        .join("..")
        .join(elsewhere.file_name().unwrap())
        .join("geheim.txt");
    // Je nach Lage der Ordner kann der Umweg ins Leere fuehren: dann fehlt die Datei; auf keinen
    // Fall wird sie angenommen.
    let r = check(&w.fx.conn(), &[up.to_string_lossy().to_string()]);
    assert!(
        matches!(
            r,
            Err(AttachError::Outside(_)) | Err(AttachError::Missing(_))
        ),
        "{r:?}"
    );
    // Und mit sicherem Ziel ausserhalb: das Ergebnis ist eindeutig „ausserhalb“.
    let outside_root = w.root.parent().unwrap().to_path_buf();
    let sibling = outside_root.join("lva-b5-aussen.txt");
    std::fs::write(&sibling, b"x").unwrap();
    let up = w.root.join("..").join("lva-b5-aussen.txt");
    let r = check(&w.fx.conn(), &[up.to_string_lossy().to_string()]);
    let _ = std::fs::remove_file(&sibling);
    assert_eq!(
        r,
        Err(AttachError::Outside("lva-b5-aussen.txt".to_string()))
    );
}

#[test]
fn bad_paths_and_missing_empty_or_non_regular_files_are_named() {
    let w = w();
    let conn = w.fx.conn();
    for bad in [
        "",
        "relativ/datei.txt",
        "{{steps.doc.path}}",
        "C:\\x\\\u{0}y",
    ] {
        assert!(
            matches!(
                check(&conn, &[bad.to_string()]),
                Err(AttachError::BadPath(_))
            ),
            "{bad:?}"
        );
    }
    let missing = w.root.join("gibtsnicht.docx").to_string_lossy().to_string();
    assert_eq!(
        check(&conn, &[missing]),
        Err(AttachError::Missing("gibtsnicht.docx".to_string()))
    );
    let empty = write(&w.root, "leer.txt", b"");
    assert_eq!(
        check(&conn, &[empty]),
        Err(AttachError::Empty("leer.txt".to_string()))
    );
    std::fs::create_dir(w.root.join("ordner")).unwrap();
    let dir = w.root.join("ordner").to_string_lossy().to_string();
    assert!(matches!(
        check(&conn, &[dir]),
        Err(AttachError::NotAFile(_, _))
    ));
}

#[test]
fn a_symlink_is_never_accepted() {
    let w = w();
    let target = write(&w.root, "echt.txt", b"inhalt");
    let link = w.root.join("link.txt");
    #[cfg(windows)]
    let made = std::os::windows::fs::symlink_file(&target, &link);
    #[cfg(not(windows))]
    let made = std::os::unix::fs::symlink(&target, &link);
    if made.is_err() {
        eprintln!("Symlink nicht anlegbar (Rechte): Test uebersprungen");
        return;
    }
    assert!(matches!(
        check(&w.fx.conn(), &[link.to_string_lossy().to_string()]),
        Err(AttachError::NotAFile(_, _))
    ));
}

#[test]
fn a_disabled_folder_does_not_count() {
    let w = w();
    let path = write(&w.root, "a.txt", b"x");
    integrations_store::update(
        &w.fx.conn(),
        "folder-protokolle",
        &IntegrationPatch {
            enabled: Some(false),
            ..Default::default()
        },
        T0,
    )
    .unwrap();
    assert_eq!(
        check(&w.fx.conn(), &[path]),
        Err(AttachError::Outside("a.txt".to_string()))
    );
}

#[test]
fn reading_off_refuses_and_ask_is_reported_so_the_mail_asks_too() {
    let w = w();
    let path = write(&w.root, "a.txt", b"x");
    set_grant(
        &w.fx.conn(),
        "folder-protokolle",
        Capability::FilesRead,
        GrantMode::Off,
    );
    let e = check(&w.fx.conn(), std::slice::from_ref(&path)).unwrap_err();
    assert!(
        matches!(&e, AttachError::ReadOff { name, reason, .. } if name == "a.txt" && *reason == "grant_off"),
        "{e:?}"
    );
    set_grant(
        &w.fx.conn(),
        "folder-protokolle",
        Capability::FilesRead,
        GrantMode::Ask,
    );
    assert_eq!(
        check(&w.fx.conn(), std::slice::from_ref(&path)).unwrap()[0].read_mode,
        GrantMode::Ask
    );
    set_grant(
        &w.fx.conn(),
        "folder-protokolle",
        Capability::FilesRead,
        GrantMode::Allow,
    );
    assert_eq!(
        check(&w.fx.conn(), &[path]).unwrap()[0].read_mode,
        GrantMode::Allow
    );
}

#[test]
fn a_second_folder_around_the_same_file_can_carry_it_when_the_first_is_off() {
    let w = w();
    let path = write(&w.root, "a.txt", b"x");
    folder(&w.fx.conn(), "folder-gesamt", w.root.parent().unwrap());
    set_grant(
        &w.fx.conn(),
        "folder-protokolle",
        Capability::FilesRead,
        GrantMode::Off,
    );
    let got = check(&w.fx.conn(), &[path]).unwrap();
    assert_eq!(got[0].folder, "folder-gesamt");
}

#[test]
fn count_and_total_size_are_limited_and_nothing_is_cut() {
    let w = w();
    let conn = w.fx.conn();
    let many: Vec<String> = (0..=MAX_ATTACHMENTS)
        .map(|i| write(&w.root, &format!("d{i}.txt"), b"x"))
        .collect();
    assert_eq!(
        check(&conn, &many),
        Err(AttachError::TooMany(MAX_ATTACHMENTS + 1))
    );
    assert_eq!(
        check(&conn, &many[..MAX_ATTACHMENTS]).unwrap().len(),
        MAX_ATTACHMENTS
    );

    let big = vec![b'x'; 1_500_000];
    let a = write(&w.root, "gross1.bin", &big);
    let b = write(&w.root, "gross2.bin", &big);
    assert_eq!(check(&conn, std::slice::from_ref(&a)).unwrap().len(), 1);
    assert_eq!(check(&conn, &[a, b]), Err(AttachError::TooBig));
}

#[test]
fn the_content_type_follows_the_extension() {
    assert!(content_type_of("a.DOCX").contains("wordprocessingml"));
    assert_eq!(content_type_of("a.pdf"), "application/pdf");
    assert_eq!(content_type_of("a.md"), "text/markdown");
    assert_eq!(content_type_of("a.exe"), "application/octet-stream");
    assert_eq!(content_type_of("ohne-endung"), "application/octet-stream");
}

#[test]
fn control_characters_in_a_file_name_cannot_reach_a_header() {
    assert_eq!(
        file_name_of(Path::new("C:\\x\\a\u{7}b\nc.txt")),
        "a_b_c.txt"
    );
}
