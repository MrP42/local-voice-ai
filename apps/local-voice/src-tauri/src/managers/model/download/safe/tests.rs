use super::*;
use std::io::{Read, Write};
use tempfile::TempDir;

const SENTINEL: &[u8] = b"fremde-datei-nicht-anfassen";

/// Eine Datei ausserhalb des Modellordners, die nie veraendert werden darf.
fn sentinel(dir: &TempDir) -> std::path::PathBuf {
    let path = dir.path().join("fremd.bin");
    fs::write(&path, SENTINEL).unwrap();
    path
}

fn assert_refused<T: std::fmt::Debug>(result: io::Result<T>) {
    let err = result.expect_err("haette abgelehnt werden muessen");
    assert!(is_refusal(&err), "keine Ablehnung, sondern: {err}");
    assert!(
        !err.to_string().contains(std::path::MAIN_SEPARATOR),
        "Ablehnungstext nennt einen Pfad: {err}"
    );
}

#[test]
fn a_plain_file_reports_its_length_and_an_absent_one_nothing() {
    let dir = TempDir::new().unwrap();
    let file = dir.path().join("a.bin");
    assert_eq!(plain_len(&file).unwrap(), None);
    fs::write(&file, b"12345").unwrap();
    assert_eq!(plain_len(&file).unwrap(), Some(5));
}

#[test]
fn a_directory_is_not_a_plain_file() {
    let dir = TempDir::new().unwrap();
    assert_refused(plain_len(dir.path()));
    assert_refused(open_truncated(dir.path()));
}

#[test]
fn ordinary_files_open_truncate_append_rename_and_remove() {
    let dir = TempDir::new().unwrap();
    let file = dir.path().join("a.partial");
    let mut f = open_truncated(&file).unwrap();
    f.write_all(b"hello").unwrap();
    drop(f);
    let mut f = open_append(&file).unwrap();
    f.write_all(b"world").unwrap();
    drop(f);
    let mut text = String::new();
    open_read(&file).unwrap().read_to_string(&mut text).unwrap();
    assert_eq!(text, "helloworld");
    // Neu anlegen leert.
    drop(open_truncated(&file).unwrap());
    assert_eq!(fs::read(&file).unwrap(), b"");
    fs::write(&file, b"x").unwrap();
    let dest = dir.path().join("a.bin");
    rename_over(&file, &dest).unwrap();
    assert!(!file.exists() && dest.exists());
    // Ueber eine vorhandene normale Datei.
    fs::write(&file, b"yy").unwrap();
    rename_over(&file, &dest).unwrap();
    assert_eq!(fs::read(&dest).unwrap(), b"yy");
    remove(&dest).unwrap();
    remove(&dest).unwrap();
    assert!(!dest.exists());
}

#[test]
fn rename_of_a_missing_source_is_not_found() {
    let dir = TempDir::new().unwrap();
    let err = rename_over(&dir.path().join("fehlt"), &dir.path().join("ziel")).unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::NotFound);
    assert!(!is_refusal(&err));
}

#[test]
fn an_ordinary_error_is_not_a_refusal() {
    assert!(!is_refusal(&io::Error::from(io::ErrorKind::NotFound)));
    assert!(!is_refusal(&io::Error::other("x")));
}

#[test]
fn a_directory_below_the_root_is_created_and_a_missing_tail_is_fine_without_create() {
    let dir = TempDir::new().unwrap();
    let root = dir.path().join("models");
    let deep = root.join("diarization").join("id");
    dir_inside(&root, &deep, false).unwrap();
    assert!(!root.exists(), "ohne create darf nichts angelegt werden");
    dir_inside(&root, &deep, true).unwrap();
    assert!(deep.is_dir());
    dir_inside(&root, &deep, false).unwrap();
}

#[test]
fn a_directory_outside_the_root_is_refused() {
    let dir = TempDir::new().unwrap();
    let root = dir.path().join("models");
    fs::create_dir_all(&root).unwrap();
    assert_refused(dir_inside(&root, &dir.path().join("anders"), true));
    assert_refused(dir_inside(&root, &root.join("..").join("anders"), true));
    assert!(!dir.path().join("anders").exists());
}

#[cfg(unix)]
mod unix {
    use super::*;
    use std::os::unix::fs::symlink;

    #[test]
    fn a_symlink_is_refused_by_every_operation_and_the_target_stays_untouched() {
        let dir = TempDir::new().unwrap();
        let outside = sentinel(&dir);
        let link = dir.path().join("link.partial");
        symlink(&outside, &link).unwrap();

        assert_refused(plain_len(&link));
        assert_refused(open_read(&link));
        assert_refused(open_append(&link));
        assert_refused(open_truncated(&link));
        assert_refused(remove(&link));
        let other = dir.path().join("andere.bin");
        fs::write(&other, b"x").unwrap();
        assert_refused(rename_over(&link, &dir.path().join("neu")));
        assert_refused(rename_over(&other, &link));

        assert_eq!(fs::read(&outside).unwrap(), SENTINEL);
        assert!(fs::symlink_metadata(&link)
            .unwrap()
            .file_type()
            .is_symlink());
        assert_eq!(fs::read(&other).unwrap(), b"x");
    }

    #[test]
    fn a_dangling_symlink_is_refused_and_nothing_is_created_behind_it() {
        let dir = TempDir::new().unwrap();
        let target = dir.path().join("noch-nicht-da");
        let link = dir.path().join("link.partial");
        symlink(&target, &link).unwrap();
        assert_refused(plain_len(&link));
        assert_refused(open_truncated(&link));
        assert_refused(open_append(&link));
        assert!(!target.exists(), "hinter dem Link wurde etwas angelegt");
    }

    #[test]
    fn a_hard_link_is_not_written_through() {
        let dir = TempDir::new().unwrap();
        let outside = sentinel(&dir);
        let link = dir.path().join("hart.partial");
        fs::hard_link(&outside, &link).unwrap();
        assert_refused(open_append(&link));
        assert_refused(open_truncated(&link));
        assert_eq!(fs::read(&outside).unwrap(), SENTINEL);
        // Lesen ist unbedenklich.
        let mut text = Vec::new();
        open_read(&link).unwrap().read_to_end(&mut text).unwrap();
        assert_eq!(text, SENTINEL);
    }

    #[test]
    fn a_linked_directory_below_the_root_is_refused_even_when_it_points_outside() {
        let dir = TempDir::new().unwrap();
        let root = dir.path().join("models");
        let outside = dir.path().join("draussen");
        fs::create_dir_all(&root).unwrap();
        fs::create_dir_all(&outside).unwrap();
        symlink(&outside, root.join("diarization")).unwrap();
        let target = root.join("diarization").join("id");
        assert_refused(dir_inside(&root, &target, true));
        assert_refused(dir_inside(&root, &target, false));
        assert!(
            fs::read_dir(&outside).unwrap().next().is_none(),
            "ausserhalb wurde etwas angelegt"
        );
    }

    #[test]
    fn a_linked_directory_that_points_inside_the_root_is_refused_too() {
        // Strenger als noetig, aber eindeutig: unterhalb der Wurzel kein Link.
        let dir = TempDir::new().unwrap();
        let root = dir.path().join("models");
        fs::create_dir_all(root.join("echt")).unwrap();
        symlink(root.join("echt"), root.join("diarization")).unwrap();
        assert_refused(dir_inside(&root, &root.join("diarization"), true));
    }

    #[test]
    fn the_root_itself_may_be_a_link() {
        let dir = TempDir::new().unwrap();
        let real = dir.path().join("platte-d");
        fs::create_dir_all(&real).unwrap();
        let root = dir.path().join("models");
        symlink(&real, &root).unwrap();
        let target = root.join("diarization").join("id");
        dir_inside(&root, &target, true).unwrap();
        assert!(real.join("diarization").join("id").is_dir());
    }

    #[test]
    fn a_file_where_a_directory_belongs_is_refused() {
        let dir = TempDir::new().unwrap();
        let root = dir.path().join("models");
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("diarization"), b"datei").unwrap();
        assert_refused(dir_inside(
            &root,
            &root.join("diarization").join("id"),
            true,
        ));
        assert_eq!(fs::read(root.join("diarization")).unwrap(), b"datei");
    }
}

/// Windows: ein Symlink braucht Entwicklermodus oder Administratorrechte, eine
/// Junction (Ordner-Reparse-Punkt) nicht. Der Symlink-Fall wird uebersprungen,
/// wenn er sich nicht anlegen laesst (mit Meldung, nicht still).
#[cfg(windows)]
mod windows {
    use super::*;
    use std::os::windows::fs::symlink_file;
    use std::process::Command;

    #[test]
    fn a_file_symlink_is_refused_when_it_can_be_created() {
        let dir = TempDir::new().unwrap();
        let outside = sentinel(&dir);
        let link = dir.path().join("link.partial");
        if let Err(e) = symlink_file(&outside, &link) {
            eprintln!("uebersprungen: Symlink nicht anlegbar ({e}); Entwicklermodus oder Administrator noetig");
            return;
        }
        assert_refused(plain_len(&link));
        assert_refused(open_read(&link));
        assert_refused(open_append(&link));
        assert_refused(open_truncated(&link));
        assert_refused(remove(&link));
        assert_eq!(fs::read(&outside).unwrap(), SENTINEL);
    }

    #[test]
    fn a_junction_below_the_root_is_refused() {
        let dir = TempDir::new().unwrap();
        let root = dir.path().join("models");
        let outside = dir.path().join("draussen");
        fs::create_dir_all(&root).unwrap();
        fs::create_dir_all(&outside).unwrap();
        let junction = root.join("diarization");
        let made = Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(&junction)
            .arg(&outside)
            .output();
        if !made.is_ok_and(|o| o.status.success()) {
            eprintln!("uebersprungen: Junction nicht anlegbar");
            return;
        }
        assert_refused(dir_inside(&root, &junction.join("id"), true));
        assert!(
            fs::read_dir(&outside).unwrap().next().is_none(),
            "ausserhalb wurde etwas angelegt"
        );
    }
}
