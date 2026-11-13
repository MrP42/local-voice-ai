use super::*;
use crate::managers::model::download::linktest::{folder_link, is_link as is_a_link, symlink_file};
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

/// Jede Operation dieses Moduls lehnt `link` ab, ohne etwas zu veraendern.
fn assert_every_operation_refuses(link: &Path) {
    assert_refused(plain_len(link));
    assert_refused(open_read(link));
    assert_refused(open_append(link));
    assert_refused(open_truncated(link));
    assert_refused(remove(link));
    assert!(is_a_link(link), "der Link wurde ersetzt oder entfernt");
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
fn ordinary_files_open_truncate_append_and_remove() {
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
    remove(&file).unwrap();
    remove(&file).unwrap();
    assert!(!file.exists());
}

#[test]
fn an_ordinary_error_is_not_a_refusal() {
    assert!(!is_refusal(&io::Error::from(io::ErrorKind::NotFound)));
    assert!(!is_refusal(&io::Error::other("x")));
}

/// Ein Symlink auf eine Datei: Unix immer, Windows nur mit Entwicklermodus oder
/// Administrator (sonst rot, ausser `LVA_SKIP_SYMLINK_TESTS=1`, dann mit Meldung).
#[test]
fn a_file_symlink_is_refused_by_every_operation_and_the_target_stays_untouched() {
    let dir = TempDir::new().unwrap();
    let outside = sentinel(&dir);
    let link = dir.path().join("link.partial");
    if !symlink_file(&link, &outside) {
        return;
    }
    assert_every_operation_refuses(&link);
    assert_eq!(fs::read(&outside).unwrap(), SENTINEL);
}

/// Das Gegenstueck ohne Rechte: ein Ordner-Link (Unix: Symlink, Windows: Junction) an
/// der Stelle einer Datei. Er wird von jeder Operation abgelehnt; der fremde Ordner
/// dahinter bleibt, wie er ist.
#[test]
fn a_folder_link_in_place_of_a_file_is_refused_by_every_operation() {
    let dir = TempDir::new().unwrap();
    let foreign = dir.path().join("fremder-ordner");
    fs::create_dir_all(&foreign).unwrap();
    fs::write(foreign.join("fremd.bin"), SENTINEL).unwrap();
    let link = dir.path().join("link.partial");
    folder_link(&link, &foreign);

    assert_every_operation_refuses(&link);

    assert_eq!(fs::read(foreign.join("fremd.bin")).unwrap(), SENTINEL);
    assert_eq!(fs::read_dir(&foreign).unwrap().count(), 1);
}

/// Ein Link ins Leere (Symlink auf eine Datei, die es nicht gibt): hinter dem Link
/// entsteht nichts.
#[test]
fn a_dangling_file_symlink_is_refused_and_nothing_is_created_behind_it() {
    let dir = TempDir::new().unwrap();
    let target = dir.path().join("noch-nicht-da");
    let link = dir.path().join("link.partial");
    if !symlink_file(&link, &target) {
        return;
    }
    assert_refused(plain_len(&link));
    assert_refused(open_truncated(&link));
    assert_refused(open_append(&link));
    assert!(!target.exists(), "hinter dem Link wurde etwas angelegt");
}

/// Dasselbe ohne Rechte: eine Junction bzw. ein Symlink auf einen Ordner, der danach
/// verschwindet.
#[test]
fn a_dangling_folder_link_is_refused_and_nothing_is_created_behind_it() {
    let dir = TempDir::new().unwrap();
    let target = dir.path().join("wird-geloescht");
    fs::create_dir_all(&target).unwrap();
    let link = dir.path().join("link.partial");
    folder_link(&link, &target);
    fs::remove_dir(&target).unwrap();

    assert_refused(plain_len(&link));
    assert_refused(open_truncated(&link));
    assert_refused(open_append(&link));
    assert!(!target.exists(), "hinter dem Link wurde etwas angelegt");
}

/// Eine Datei mit zweitem Namen (Hardlink) wird nie durchgeschrieben: nicht
/// angehaengt, nicht abgeschnitten. Gilt auf jedem System (unter Windows zaehlt
/// `nNumberOfLinks`); `std::fs::hard_link` braucht keine Rechte.
#[test]
fn a_hard_link_is_not_written_through() {
    let dir = TempDir::new().unwrap();
    let outside = sentinel(&dir);
    let link = dir.path().join("hart.partial");
    fs::hard_link(&outside, &link).unwrap();
    assert_refused(open_append(&link));
    assert_refused(open_truncated(&link));
    assert_eq!(fs::read(&outside).unwrap(), SENTINEL);
    assert_eq!(fs::read(&link).unwrap(), SENTINEL);
    // Lesen ist unbedenklich.
    let mut text = Vec::new();
    open_read(&link).unwrap().read_to_end(&mut text).unwrap();
    assert_eq!(text, SENTINEL);
}

/// Ein Hardlink auf eine LEERE Datei (der Fall "frischer Download"): auch dann wird
/// nichts hineingeschrieben, und die Pruefung geschieht vor dem Abschneiden.
#[test]
fn a_hard_link_to_an_empty_file_is_refused_before_anything_is_written() {
    let dir = TempDir::new().unwrap();
    let outside = dir.path().join("leer.bin");
    fs::write(&outside, b"").unwrap();
    let link = dir.path().join("hart.partial");
    fs::hard_link(&outside, &link).unwrap();
    assert_refused(open_append(&link));
    assert_refused(open_truncated(&link));
    assert_eq!(fs::read(&outside).unwrap(), b"");
}

/// Die Pruefung des Hardlinks gilt vor dem Abschneiden, nicht danach: eine Datei, die
/// nach dem Abschneiden als verlinkt erkannt wuerde, waere schon geleert.
#[test]
fn a_hard_linked_file_keeps_its_bytes_when_truncation_is_refused() {
    let dir = TempDir::new().unwrap();
    let outside = sentinel(&dir);
    let link = dir.path().join("hart.partial");
    fs::hard_link(&outside, &link).unwrap();
    assert!(open_truncated(&link).is_err());
    assert_eq!(fs::metadata(&outside).unwrap().len(), SENTINEL.len() as u64);
    assert_eq!(fs::read(&outside).unwrap(), SENTINEL);
}
