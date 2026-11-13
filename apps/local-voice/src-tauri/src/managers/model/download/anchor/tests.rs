use super::*;
use crate::managers::model::download::linktest::{
    folder_link, is_link, swap_must_have_happened, symlink_dir, symlink_file,
};
use std::fmt::Debug;
use std::io::{Read, Seek, SeekFrom, Write};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use tempfile::TempDir;

const SENTINEL: &[u8] = b"fremde-datei-nicht-anfassen";

fn open(root: &Path, parts: &[&str]) -> Anchor {
    Anchor::open(root, parts, true).unwrap().unwrap()
}

fn assert_refused<T: Debug>(result: io::Result<T>) {
    let err = result.expect_err("haette abgelehnt werden muessen");
    assert!(safe::is_refusal(&err), "keine Ablehnung, sondern: {err}");
    assert!(
        !err.to_string().contains(std::path::MAIN_SEPARATOR),
        "Ablehnungstext nennt einen Pfad: {err}"
    );
}

fn write(path: &Path, bytes: &[u8]) {
    std::fs::write(path, bytes).unwrap();
}

fn read(path: &Path) -> Vec<u8> {
    std::fs::read(path).unwrap()
}

fn names_in(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

/// Ersetzt `name` durch eine NEUE Datei (andere Kennung) mit `bytes`. `false`, wenn
/// das Dateisystem es nicht zulaesst (Windows, solange eine Datei ohne
/// `FILE_SHARE_DELETE` offen ist: dort ist der Tausch dann ohnehin ausgeschlossen).
fn replace_file(dir: &Path, name: &str, bytes: &[u8]) -> bool {
    let tmp = dir.join(format!("{name}.tausch"));
    write(&tmp, bytes);
    if std::fs::rename(&tmp, dir.join(name)).is_ok() {
        return true;
    }
    let _ = std::fs::remove_file(&tmp);
    false
}

// ---- die Kette ---------------------------------------------------------------

#[test]
fn the_chain_is_created_and_a_missing_tail_is_none_without_create() {
    let dir = TempDir::new().unwrap();
    let root = dir.path().join("models");
    assert!(Anchor::open(&root, &["diarization", "id"], false)
        .unwrap()
        .is_none());
    assert!(!root.exists(), "ohne create darf nichts angelegt werden");
    open(&root, &["diarization", "id"]);
    assert!(root.join("diarization").join("id").is_dir());
    let again = Anchor::open(&root, &["diarization", "id"], false).unwrap();
    assert!(again.is_some());
    again.unwrap().still_at().unwrap();
}

#[test]
fn a_linked_folder_in_the_chain_is_refused_and_nothing_appears_outside() {
    for leaf_only in [false, true] {
        let dir = TempDir::new().unwrap();
        let root = dir.path().join("models");
        let outside = dir.path().join("draussen");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        write(&outside.join("fremd.bin"), SENTINEL);
        if leaf_only {
            std::fs::create_dir_all(root.join("diarization")).unwrap();
            folder_link(&root.join("diarization").join("id"), &outside);
        } else {
            folder_link(&root.join("diarization"), &outside);
        }
        for create in [true, false] {
            assert_refused(Anchor::open(&root, &["diarization", "id"], create));
        }
        assert_eq!(names_in(&outside), ["fremd.bin"], "leaf_only={leaf_only}");
        assert_eq!(read(&outside.join("fremd.bin")), SENTINEL);
    }
}

#[test]
fn a_symlinked_folder_in_the_chain_is_refused_too() {
    let dir = TempDir::new().unwrap();
    let root = dir.path().join("models");
    let outside = dir.path().join("draussen");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::create_dir_all(&outside).unwrap();
    if !symlink_dir(&root.join("diarization"), &outside) {
        return;
    }
    assert_refused(Anchor::open(&root, &["diarization", "id"], true));
    assert!(
        names_in(&outside).is_empty(),
        "ausserhalb wurde etwas angelegt"
    );
}

#[test]
fn a_link_that_points_inside_the_root_is_refused_too() {
    // Strenger als noetig, aber eindeutig: unterhalb der Wurzel kein Link.
    let dir = TempDir::new().unwrap();
    let root = dir.path().join("models");
    std::fs::create_dir_all(root.join("echt")).unwrap();
    folder_link(&root.join("diarization"), &root.join("echt"));
    assert_refused(Anchor::open(&root, &["diarization"], true));
}

#[test]
fn the_root_itself_may_be_a_link() {
    let dir = TempDir::new().unwrap();
    let real = dir.path().join("platte-d");
    std::fs::create_dir_all(&real).unwrap();
    let root = dir.path().join("models");
    folder_link(&root, &real);
    let anchor = open(&root, &["diarization", "id"]);
    anchor.still_at().unwrap();
    assert!(real.join("diarization").join("id").is_dir());
}

#[test]
fn a_file_where_a_folder_belongs_is_refused() {
    let dir = TempDir::new().unwrap();
    let root = dir.path().join("models");
    std::fs::create_dir_all(&root).unwrap();
    write(&root.join("diarization"), b"datei");
    assert_refused(Anchor::open(&root, &["diarization", "id"], true));
    assert_eq!(read(&root.join("diarization")), b"datei");
}

#[test]
fn only_single_plain_names_are_accepted() {
    let dir = TempDir::new().unwrap();
    let anchor = open(dir.path(), &[]);
    for bad in [
        "", ".", "..", "../x", "a/b", "a\\b", "C:", "C:x", "x:strom", "nul", "x.", "x ", "a*b",
    ] {
        assert_refused(anchor.plain_len(bad));
        assert_refused(anchor.open_read(bad));
        assert_refused(anchor.open_partial(bad));
        assert_refused(anchor.remove(bad));
        assert_refused(anchor.unlink(bad));
        assert_refused(anchor.rename_over(bad, "ok"));
    }
    assert_refused(Anchor::open(dir.path(), &["a/b"], true));
    assert_refused(Anchor::open(dir.path(), &[".."], true));
    assert!(names_in(dir.path()).is_empty());
}

// ---- Dateien ueber den Anker ---------------------------------------------------

#[test]
fn ordinary_files_open_resume_rename_and_remove_through_the_anchor() {
    let dir = TempDir::new().unwrap();
    let anchor = open(dir.path(), &["m"]);
    let folder = dir.path().join("m");
    assert_eq!(anchor.plain_len("a.partial").unwrap(), None);
    assert_eq!(
        anchor.open_read("a.partial").unwrap_err().kind(),
        io::ErrorKind::NotFound
    );

    let mut f = anchor.open_partial("a.partial").unwrap();
    f.write_all(b"hello").unwrap();
    drop(f);
    assert_eq!(anchor.plain_len("a.partial").unwrap(), Some(5));
    // Fortsetzen: der Handle ist nicht abgeschnitten, die Position setzt der Aufrufer.
    let mut f = anchor.open_partial("a.partial").unwrap();
    assert_eq!(f.metadata().unwrap().len(), 5);
    f.seek(SeekFrom::End(0)).unwrap();
    f.write_all(b"world").unwrap();
    drop(f);
    let mut text = String::new();
    anchor
        .open_read("a.partial")
        .unwrap()
        .read_to_string(&mut text)
        .unwrap();
    assert_eq!(text, "helloworld");

    anchor.rename_over("a.partial", "a.bin").unwrap();
    assert_eq!(names_in(&folder), ["a.bin"]);
    // Ueber eine vorhandene normale Datei.
    write(&folder.join("b.partial"), b"yy");
    anchor.rename_over("b.partial", "a.bin").unwrap();
    assert_eq!(read(&folder.join("a.bin")), b"yy");
    anchor.remove("a.bin").unwrap();
    anchor.remove("a.bin").unwrap();
    assert!(names_in(&folder).is_empty());
    let err = anchor.rename_over("fehlt", "ziel").unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::NotFound);
    assert!(!safe::is_refusal(&err));
}

#[test]
fn a_directory_is_not_a_plain_file() {
    let dir = TempDir::new().unwrap();
    let anchor = open(dir.path(), &[]);
    std::fs::create_dir(dir.path().join("ordner")).unwrap();
    assert_refused(anchor.plain_len("ordner"));
    assert_refused(anchor.open_read("ordner"));
    assert_refused(anchor.open_partial("ordner"));
    assert_refused(anchor.remove("ordner"));
    assert_refused(anchor.unlink("ordner"));
    assert!(dir.path().join("ordner").is_dir());
}

#[test]
fn a_symlinked_file_is_refused_by_every_operation_and_the_target_stays_untouched() {
    let dir = TempDir::new().unwrap();
    let outside = TempDir::new().unwrap();
    let anchor = open(dir.path(), &[]);
    let target = outside.path().join("fremd.bin");
    write(&target, SENTINEL);
    let link = dir.path().join("link.partial");
    if !symlink_file(&link, &target) {
        return;
    }
    let other = dir.path().join("andere.bin");
    write(&other, b"x");

    assert_refused(anchor.plain_len("link.partial"));
    assert_refused(anchor.open_read("link.partial"));
    assert_refused(anchor.open_partial("link.partial"));
    assert_refused(anchor.remove("link.partial"));
    assert_refused(anchor.rename_over("link.partial", "neu"));
    assert_refused(anchor.rename_over("andere.bin", "link.partial"));

    assert_eq!(read(&target), SENTINEL);
    assert!(is_link(&link));
    assert_eq!(read(&other), b"x");
}

/// Die Junction-Entsprechung zum Symlink-Fall: braucht unter Windows keine Rechte.
/// An der Stelle der Datei liegt ein Link auf einen Ordner mit einer fremden Datei.
#[test]
fn a_folder_link_in_place_of_a_file_is_refused_by_every_operation() {
    let dir = TempDir::new().unwrap();
    let outside = TempDir::new().unwrap();
    write(&outside.path().join("fremd.bin"), SENTINEL);
    let anchor = open(dir.path(), &[]);
    let link = dir.path().join("link.partial");
    folder_link(&link, outside.path());

    assert_refused(anchor.plain_len("link.partial"));
    assert_refused(anchor.open_read("link.partial"));
    assert_refused(anchor.open_partial("link.partial"));
    assert_refused(anchor.remove("link.partial"));
    assert_refused(anchor.rename_over("link.partial", "neu"));

    assert_eq!(names_in(outside.path()), ["fremd.bin"]);
    assert_eq!(read(&outside.path().join("fremd.bin")), SENTINEL);
    assert!(is_link(&link));
}

#[test]
fn a_dangling_symlink_is_refused_and_nothing_is_created_behind_it() {
    let dir = TempDir::new().unwrap();
    let outside = TempDir::new().unwrap();
    let anchor = open(dir.path(), &[]);
    let behind = outside.path().join("noch-nicht-da.bin");
    if !symlink_file(&dir.path().join("link.partial"), &behind) {
        return;
    }
    assert_refused(anchor.plain_len("link.partial"));
    assert_refused(anchor.open_partial("link.partial"));
    assert_refused(anchor.open_read("link.partial"));
    assert!(!behind.exists(), "hinter dem Link wurde etwas angelegt");
}

/// Ohne jedes Recht anlegbar (auch unter Windows) und der Fall, den `std` dort nicht
/// sieht: eine normale Datei mit zwei Namen. Geschrieben oder abgeschnitten wird
/// nicht, Lesen ist unbedenklich.
#[test]
fn a_hard_link_is_never_written_through_or_truncated() {
    let dir = TempDir::new().unwrap();
    let anchor = open(dir.path(), &[]);
    let foreign = dir.path().join("fremd.bin");
    write(&foreign, SENTINEL);
    std::fs::hard_link(&foreign, dir.path().join("hart.partial")).unwrap();

    assert_refused(anchor.open_partial("hart.partial"));
    assert_eq!(read(&foreign), SENTINEL, "fremde Datei beschrieben");
    let mut text = Vec::new();
    anchor
        .open_read("hart.partial")
        .unwrap()
        .read_to_end(&mut text)
        .unwrap();
    assert_eq!(text, SENTINEL);

    // Der zweite Name verschwindet: wieder eine gewoehnliche Datei.
    std::fs::remove_file(&foreign).unwrap();
    let mut f = anchor.open_partial("hart.partial").unwrap();
    f.seek(SeekFrom::End(0)).unwrap();
    f.write_all(b"+").unwrap();
}

#[test]
fn unlink_removes_a_link_but_never_its_target() {
    let dir = TempDir::new().unwrap();
    let outside = TempDir::new().unwrap();
    let anchor = open(dir.path(), &[]);
    let target = outside.path().join("fremd.bin");
    write(&target, SENTINEL);
    let link = dir.path().join("link.onnx");
    if !symlink_file(&link, &target) {
        return;
    }
    anchor.unlink("link.onnx").unwrap();
    assert!(
        std::fs::symlink_metadata(&link).is_err(),
        "Link blieb liegen"
    );
    assert_eq!(read(&target), SENTINEL);
    anchor.unlink("link.onnx").unwrap();
}

#[test]
fn names_follows_the_file_and_not_the_name() {
    let dir = TempDir::new().unwrap();
    let anchor = open(dir.path(), &[]);
    write(&dir.path().join("a.bin"), b"eins");
    let file = anchor.open_read("a.bin").unwrap();
    assert!(anchor.names("a.bin", &file).unwrap());
    if replace_file(dir.path(), "a.bin", b"zwei") {
        assert!(
            !anchor.names("a.bin", &file).unwrap(),
            "Tausch nicht bemerkt"
        );
    }
    assert!(!anchor.names("fehlt", &file).unwrap());
}

// ---- der Ordner selbst ---------------------------------------------------------

/// Der Ordner wird im Dateisystem durch einen Link auf einen fremden Ordner ersetzt,
/// nachdem er geoeffnet wurde. Unix: alle Operationen landen weiter im alten
/// (verschobenen) Ordner, im fremden entsteht nichts, `still_at` meldet es. Windows:
/// das Umbenennen scheitert, solange das Handle offen ist.
#[test]
fn a_folder_replaced_by_a_link_after_opening_is_never_followed() {
    let dir = TempDir::new().unwrap();
    let root = dir.path().join("models");
    let foreign = dir.path().join("fremd");
    std::fs::create_dir_all(&foreign).unwrap();
    write(&foreign.join("fremd.bin"), SENTINEL);
    write(&foreign.join("x.partial"), SENTINEL);
    let anchor = open(&root, &["diarization", "id"]);
    let folder = root.join("diarization").join("id");

    let moved = dir.path().join("weggeschoben");
    if std::fs::rename(&folder, &moved).is_err() {
        #[cfg(unix)]
        panic!("Unix: das Umbenennen eines offenen Ordners muss gehen");
        #[cfg(not(unix))]
        {
            // Windows: der Ordner laesst sich nicht ersetzen, solange das Handle offen ist.
            anchor.still_at().unwrap();
            let mut f = anchor.open_partial("x.partial").unwrap();
            f.write_all(b"in-den-eigenen-ordner").unwrap();
            assert!(folder.join("x.partial").is_file());
            assert_eq!(read(&foreign.join("x.partial")), SENTINEL);
            return;
        }
    }
    folder_link(&folder, &foreign);

    assert_refused(anchor.still_at());
    // Operationen laufen im alten Ordner.
    let mut f = anchor.open_partial("x.partial").unwrap();
    f.write_all(b"Daten").unwrap();
    drop(f);
    anchor.rename_over("x.partial", "x.bin").unwrap();
    anchor.remove("x.bin").unwrap();
    write(&moved.join("y.bin"), b"alt");
    anchor.unlink("y.bin").unwrap();

    assert_eq!(names_in(&foreign), ["fremd.bin", "x.partial"]);
    assert_eq!(read(&foreign.join("fremd.bin")), SENTINEL);
    assert_eq!(read(&foreign.join("x.partial")), SENTINEL);
    assert!(names_in(&moved).is_empty());
    // Ein neues Oeffnen ueber den Pfad lehnt den Link ab.
    assert_refused(Anchor::open(&root, &["diarization", "id"], false));
}

#[test]
fn still_at_notices_another_folder_with_the_same_name() {
    let dir = TempDir::new().unwrap();
    let root = dir.path().join("models");
    let anchor = open(&root, &["diarization"]);
    let folder = root.join("diarization");
    let moved = dir.path().join("weg");
    if std::fs::rename(&folder, &moved).is_err() {
        return; // Windows: nicht ersetzbar, solange das Handle offen ist
    }
    std::fs::create_dir(&folder).unwrap();
    assert_refused(anchor.still_at());
}

// ---- Ordner im Fenster vor dem Oeffnen durch einen Link ersetzt --------------------

/// Ein Ordner-Link in beiden Formen: ohne Rechte (Unix: Symlink, Windows: Junction)
/// und, nur unter Windows, als echter Symlink (rot ohne Rechte, ausser
/// `LVA_SKIP_SYMLINK_TESTS=1`). `false`: nicht angelegt (erlaubtes Ueberspringen).
fn link_dir(real_symlink: bool, link: &Path, target: &Path) -> bool {
    if real_symlink {
        symlink_dir(link, target)
    } else {
        folder_link(link, target);
        true
    }
}

/// Die Formen, die dieses System kennt: unter Unix ist beides derselbe Symlink.
fn link_forms() -> &'static [bool] {
    if cfg!(windows) {
        &[false, true]
    } else {
        &[false]
    }
}

/// `root` plus die durch `/` getrennten Bestandteile von `spec`, Stueck fuer Stueck gefuegt.
/// Nie `root.join("w/a")`: unter Windows entstuende `...\root\w/a`, und `cmd` liest bei
/// `mklink` das unquotierte `/a` als Schalter (Junction nicht anlegbar).
fn rel(root: &Path, spec: &str) -> PathBuf {
    spec.split('/')
        .fold(root.to_path_buf(), |path, part| path.join(part))
}

/// Benennt `victim` (Ordner oder Datei) nach `moved` um und legt danach an seine Stelle,
/// was `make` anlegt. `true`: getauscht. Scheitert das Umbenennen oder das Anlegen
/// (erlaubtes Ueberspringen unter Windows), ist `victim` danach wieder an seinem Platz:
/// sonst fehlte es, und der Nicht-getauscht-Zweig des Tests stuende vor einem leeren Platz.
fn swap_dir(victim: &Path, moved: &Path, make: impl FnOnce() -> bool) -> bool {
    if std::fs::rename(victim, moved).is_err() {
        return false;
    }
    if make() {
        return true;
    }
    std::fs::rename(moved, victim).expect("weg benannten Ordner zurueckbenennen");
    false
}

/// Alle Dateien unter `dir` mit Inhalt, sortiert (Ordner mit `None`): ein Abbild, das
/// jede angelegte, geaenderte oder geloeschte Datei verraet.
fn snapshot(dir: &Path) -> Vec<(String, Option<Vec<u8>>)> {
    fn walk(base: &Path, dir: &Path, out: &mut Vec<(String, Option<Vec<u8>>)>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            let rel = path
                .strip_prefix(base)
                .unwrap()
                .to_string_lossy()
                .into_owned();
            if path.is_dir() {
                out.push((rel, None));
                walk(base, &path, out);
            } else {
                out.push((rel, Some(std::fs::read(&path).unwrap())));
            }
        }
    }
    let mut out = Vec::new();
    walk(dir, dir, &mut out);
    out.sort();
    out
}

/// Der Ordner wird UNMITTELBAR vor `open_dir` (nach der Namenspruefung) durch einen
/// Link auf einen Nachbarordner im selben Elternordner ersetzt. Ein relativer Link
/// innerhalb des Elternordners ist fuer `cap-std` kein Ausbruch, `open_dir` folgt ihm
/// also, und das Handle waere der Nachbar. Erwartung: Ablehnung, bevor irgendetwas im
/// Ordner getan wird; der Nachbarbaum samt fremder `.partial` und Sentinel bleibt Byte
/// fuer Byte gleich. Der Fall `index` 1 ersetzt den letzten Ordner der Kette, der Fall
/// 0 den ersten. Unter Windows zusaetzlich als Junction und als Symlink (Skip-Regel
/// aus `linktest`); lassen sich Ordner dort nicht tauschen, gibt es den Tausch nicht.
#[test]
fn a_folder_replaced_by_a_link_just_before_it_is_opened_is_refused_and_the_sibling_stays() {
    for &real_symlink in link_forms() {
        for (index, victim, target, sibling) in [(1usize, "w/a", "b", "w/b"), (0, "w", "x", "x")] {
            for create in [false, true] {
                let dir = TempDir::new().unwrap();
                let root = dir.path().join("root");
                for tree in ["w/b", "x"] {
                    let tree = rel(&root, tree);
                    std::fs::create_dir_all(&tree).unwrap();
                    write(&tree.join("fremd.bin"), SENTINEL);
                    write(&tree.join("model.onnx_data.partial"), SENTINEL);
                }
                std::fs::create_dir_all(rel(&root, "w/a")).unwrap();
                let sibling_path = rel(&root, sibling);
                let before = snapshot(&sibling_path);

                let victim_path = rel(&root, victim);
                let moved = victim_path.with_extension("alt");
                // Unix: relativer Link im Elternordner; Windows: die Junction braucht
                // einen absoluten Pfad.
                let link_to = if cfg!(windows) {
                    victim_path.parent().unwrap().join(target)
                } else {
                    PathBuf::from(target)
                };
                let swapped = Arc::new(AtomicBool::new(false));
                let _hook = {
                    let swapped = swapped.clone();
                    let (victim_path, moved) = (victim_path.clone(), moved.clone());
                    hook::set(&root, move |point| {
                        if point != Point::BeforeDir(index) || swapped.load(Ordering::SeqCst) {
                            return;
                        }
                        let done = swap_dir(&victim_path, &moved, || {
                            link_dir(real_symlink, &victim_path, &link_to)
                        });
                        swapped.store(done, Ordering::SeqCst);
                    })
                };

                let result = Anchor::open(&root, &["w", "a"], create);

                let what = format!("index={index} create={create} symlink={real_symlink}");
                if swapped.load(Ordering::SeqCst) {
                    assert_refused(result);
                    assert_eq!(
                        snapshot(&sibling_path),
                        before,
                        "Nachbarbaum veraendert ({what})"
                    );
                    assert_eq!(
                        read(&sibling_path.join("model.onnx_data.partial")),
                        SENTINEL
                    );
                } else {
                    swap_must_have_happened();
                    assert!(result.unwrap().is_some(), "{what}");
                }
            }
        }
    }
}

/// Der geprueft Ordner wird UNMITTELBAR vor `open_dir` weg benannt und ein ganz NORMALER
/// fremder Geschwisterordner (mit fremder Datei und fremder `.partial` als Wachposten)
/// unter seinen Namen verschoben: kein Link, nichts, was `symlink_metadata` oder die
/// Attribute verraten. Unter Unix kennt schon die Namenslesung dev/ino; unter Windows hat
/// nur der festgehaltene Ursprung (`Origin`) eine Kennung. Erwartet: Ablehnung vor jeder
/// Arbeit im Ordner; der fremde Baum (jetzt unter dem Namen des Opfers) und der weg
/// benannte Ursprung bleiben Byte fuer Byte gleich. Index 1 tauscht den letzten Ordner
/// der Kette, Index 0 den ersten. Lassen sich Ordner dort nicht tauschen (Windows), gibt
/// es den Tausch nicht.
#[test]
fn a_folder_renamed_away_and_replaced_by_a_normal_sibling_just_before_opening_is_refused() {
    for (index, victim, sibling) in [(1usize, "w/a", "w/b"), (0, "w", "x")] {
        for create in [false, true] {
            let dir = TempDir::new().unwrap();
            let root = dir.path().join("root");
            for tree in ["w/b", "x"] {
                let tree = rel(&root, tree);
                std::fs::create_dir_all(&tree).unwrap();
                write(&tree.join("fremd.bin"), SENTINEL);
                write(&tree.join("model.onnx_data.partial"), SENTINEL);
            }
            std::fs::create_dir_all(rel(&root, "w/a")).unwrap();
            let (victim_path, sibling_path) = (rel(&root, victim), rel(&root, sibling));
            let moved = victim_path.with_extension("alt");
            let (origin_before, sibling_before) = (snapshot(&victim_path), snapshot(&sibling_path));

            let swapped = Arc::new(AtomicBool::new(false));
            let _hook = {
                let swapped = swapped.clone();
                let (victim_path, sibling_path, moved) =
                    (victim_path.clone(), sibling_path.clone(), moved.clone());
                hook::set(&root, move |point| {
                    if point != Point::BeforeDir(index) || swapped.load(Ordering::SeqCst) {
                        return;
                    }
                    let done = swap_dir(&victim_path, &moved, || {
                        std::fs::rename(&sibling_path, &victim_path).is_ok()
                    });
                    swapped.store(done, Ordering::SeqCst);
                })
            };

            let result = Anchor::open(&root, &["w", "a"], create);

            let what = format!("index={index} create={create}");
            if swapped.load(Ordering::SeqCst) {
                assert_refused(result);
                assert_eq!(
                    snapshot(&victim_path),
                    sibling_before,
                    "fremder Baum veraendert ({what})"
                );
                assert_eq!(
                    read(&victim_path.join("model.onnx_data.partial")),
                    SENTINEL,
                    "{what}"
                );
                assert_eq!(
                    snapshot(&moved),
                    origin_before,
                    "weg benannter Ordner veraendert ({what})"
                );
            } else {
                swap_must_have_happened();
                assert!(result.unwrap().is_some(), "{what}");
            }
        }
    }
}

/// Die Datei wird UNMITTELBAR vor dem Oeffnen (nach der Namenspruefung) durch einen
/// relativen Link `name -> fremd/neu` ersetzt (Unterordner `fremd` im Ankerordner; das Ziel
/// fehlt oder ist eine fremde Datei). `cap-std` folgt einem Link innerhalb des Ordners
/// dort, wo es von Hand aufloest, auch mit `create`: die Datei entstuende am Linkziel.
/// Erwartet: Ablehnung, an `fremd/` aendert sich nichts (nichts angelegt, nichts
/// beschrieben), die weg benannte echte Datei bleibt Byte fuer Byte gleich. Fuer das
/// Anlegen (`open_partial` ohne Datei), das Fortsetzen (`open_partial` mit Datei) und das
/// Lesen (`open_read`). Windows: Datei-Symlinks brauchen Rechte (Skip-Regel aus `linktest`).
#[test]
fn a_file_replaced_by_a_link_just_before_it_is_opened_is_refused_and_nothing_is_created() {
    const NAME: &str = "model.onnx_data.partial";
    for (op, existing) in [("create", false), ("resume", true), ("read", true)] {
        for target_exists in [false, true] {
            let dir = TempDir::new().unwrap();
            let anchor = open(dir.path(), &["m"]);
            let folder = dir.path().join("m");
            let behind = folder.join("fremd");
            std::fs::create_dir(&behind).unwrap();
            if target_exists {
                write(&behind.join("neu"), SENTINEL);
            }
            if existing {
                write(&folder.join(NAME), b"echt");
            }
            let before = snapshot(&behind);

            let swapped = Arc::new(AtomicBool::new(false));
            let _hook = {
                let (folder, swapped) = (folder.clone(), swapped.clone());
                hook::set(anchor.root(), move |point| {
                    if point != Point::BeforeFile || swapped.load(Ordering::SeqCst) {
                        return;
                    }
                    let (link, moved) = (folder.join(NAME), folder.join("echt.alt"));
                    let target = Path::new("fremd").join("neu");
                    let make = || symlink_file(&link, &target);
                    let done = if existing {
                        swap_dir(&link, &moved, make)
                    } else {
                        make()
                    };
                    swapped.store(done, Ordering::SeqCst);
                })
            };

            let result = match op {
                "read" => anchor.open_read(NAME),
                _ => anchor.open_partial(NAME),
            };

            let what = format!("op={op} target_exists={target_exists}");
            if swapped.load(Ordering::SeqCst) {
                assert_refused(result);
                assert_eq!(snapshot(&behind), before, "am Linkziel geaendert ({what})");
                assert!(is_link(&folder.join(NAME)), "{what}");
                if existing {
                    assert_eq!(read(&folder.join("echt.alt")), b"echt", "{what}");
                }
            } else {
                swap_must_have_happened();
                assert!(result.is_ok(), "{what}");
            }
        }
    }
}

/// `no_follow` allein (ohne ein von Hand gesetztes `O_NOFOLLOW`) lehnt einen Link im
/// letzten Stueck ab, auch mit `create` und Schreiben, und legt hinter ihm nichts an. Das
/// ist die Naht zu `cap-std`: ohne sie folgt es dem Link dort, wo es von Hand aufloest.
#[test]
fn no_follow_alone_refuses_a_link_and_creates_nothing_behind_it() {
    let dir = TempDir::new().unwrap();
    let folder = Dir::open_ambient_dir(dir.path(), ambient_authority()).unwrap();
    std::fs::create_dir(dir.path().join("fremd")).unwrap();
    if !symlink_file(&dir.path().join("link"), &Path::new("fremd").join("neu")) {
        return;
    }
    for (write, create) in [(false, false), (true, false), (true, true)] {
        let mut options = OpenOptions::new();
        options.read(true);
        if write {
            options.write(true);
        }
        if create {
            options.create(true);
        }
        no_follow(&mut options);
        assert!(
            folder.open_with("link", &options).is_err(),
            "Link geoeffnet (write={write} create={create})"
        );
    }
    assert!(
        names_in(&dir.path().join("fremd")).is_empty(),
        "hinter dem Link wurde etwas angelegt"
    );
}

// ---- check_set -----------------------------------------------------------------

fn sha(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

struct Pair {
    _dir: TempDir,
    anchor: Anchor,
    folder: PathBuf,
    shas: [String; 2],
}

const GRAPH: &[u8] = b"graph-bytes";
const WEIGHTS: &[u8] = b"weights-bytes-weights-bytes";

fn pair() -> Pair {
    let dir = TempDir::new().unwrap();
    let anchor = open(dir.path(), &["m"]);
    let folder = dir.path().join("m");
    write(&folder.join("model.onnx"), GRAPH);
    write(&folder.join("model.onnx_data"), WEIGHTS);
    Pair {
        anchor,
        folder,
        shas: [sha(GRAPH), sha(WEIGHTS)],
        _dir: dir,
    }
}

fn wanted(shas: &[String; 2]) -> [Wanted<'_>; 2] {
    [
        Wanted {
            name: "model.onnx",
            size: GRAPH.len() as u64,
            sha256: Some(&shas[0]),
        },
        Wanted {
            name: "model.onnx_data",
            size: WEIGHTS.len() as u64,
            sha256: Some(&shas[1]),
        },
    ]
}

fn run(p: &Pair, sink: Option<Sink<'_>>) -> io::Result<Option<Vec<Check>>> {
    check_set(&p.anchor, &wanted(&p.shas), &|| false, sink)
}

#[test]
fn a_valid_pair_is_valid_and_the_sink_gets_exactly_the_checked_bytes() {
    let p = pair();
    let mut got: [Vec<u8>; 2] = [Vec::new(), Vec::new()];
    let mut sink = |i: usize, block: &[u8]| got[i].extend_from_slice(block);
    let states = run(&p, Some(&mut sink)).unwrap().unwrap();
    assert_eq!(states, [Check::Valid, Check::Valid]);
    assert_eq!(got[0], GRAPH);
    assert_eq!(got[1], WEIGHTS);
    assert_eq!(
        run(&p, None).unwrap().unwrap(),
        [Check::Valid, Check::Valid]
    );
}

#[test]
fn missing_wrong_size_and_wrong_bytes_are_told_apart() {
    let p = pair();
    std::fs::remove_file(p.folder.join("model.onnx")).unwrap();
    write(
        &p.folder.join("model.onnx_data"),
        &vec![b'x'; WEIGHTS.len()],
    );
    assert_eq!(
        run(&p, None).unwrap().unwrap(),
        [Check::Missing, Check::Hash]
    );
    write(&p.folder.join("model.onnx"), b"kurz");
    write(&p.folder.join("model.onnx_data"), &WEIGHTS[..5]);
    assert_eq!(
        run(&p, None).unwrap().unwrap(),
        [Check::Size(4), Check::Size(5)]
    );
}

#[test]
fn without_a_hash_and_without_a_sink_only_the_size_counts() {
    let p = pair();
    write(
        &p.folder.join("model.onnx_data"),
        &vec![b'x'; WEIGHTS.len()],
    );
    let wanted = [Wanted {
        name: "model.onnx_data",
        size: WEIGHTS.len() as u64,
        sha256: None,
    }];
    let states = check_set(&p.anchor, &wanted, &|| false, None)
        .unwrap()
        .unwrap();
    assert_eq!(states, [Check::Valid]);
}

#[test]
fn cancelling_stops_the_read_without_a_verdict() {
    let p = pair();
    assert!(check_set(&p.anchor, &wanted(&p.shas), &|| true, None)
        .unwrap()
        .is_none());
}

#[test]
fn a_link_is_an_error_not_a_state_even_with_the_right_bytes() {
    let p = pair();
    let outside = TempDir::new().unwrap();
    let target = outside.path().join("fremd.bin");
    write(&target, WEIGHTS);
    std::fs::remove_file(p.folder.join("model.onnx_data")).unwrap();
    if !symlink_file(&p.folder.join("model.onnx_data"), &target) {
        return;
    }
    assert_refused(run(&p, None));
    assert_eq!(read(&target), WEIGHTS);
}

#[test]
fn a_folder_link_in_place_of_a_file_is_an_error_not_a_state() {
    let p = pair();
    let outside = TempDir::new().unwrap();
    std::fs::remove_file(p.folder.join("model.onnx_data")).unwrap();
    folder_link(&p.folder.join("model.onnx_data"), outside.path());
    assert_refused(run(&p, None));
}

/// Die noch UNGEPRUEFTEN Gewichte werden nach der Pruefung des Graphen (mitten in der
/// Pruefung) durch eine andere Datei gleicher Groesse ersetzt: kein `Valid`, das Handle
/// der Gewichte hat noch die alten Bytes, der Name zeigt aber auf eine andere Datei.
/// Unter Windows scheitert der Tausch, weil die Datei ohne `FILE_SHARE_DELETE` offen
/// ist; dann ist nichts getauscht und `Valid` richtig. (Der bereits GEPRUEFTE Graph ist
/// ein anderer Fall: `an_in_place_change_of_the_already_checked_graph_...`.)
#[test]
fn a_same_size_swap_of_the_not_yet_checked_weights_is_no_valid_pair() {
    let p = pair();
    let swapped = Arc::new(AtomicBool::new(false));
    let _hook = {
        let (folder, swapped) = (p.folder.clone(), swapped.clone());
        hook::set(p.anchor.root(), move |point| {
            if point == Point::Checked(0) {
                let done = replace_file(&folder, "model.onnx_data", &vec![b'x'; WEIGHTS.len()]);
                swapped.store(done, Ordering::SeqCst);
            }
        })
    };
    let states = run(&p, None).unwrap().unwrap();
    if swapped.load(Ordering::SeqCst) {
        assert_eq!(states, [Check::Valid, Check::Changed]);
        assert_eq!(
            read(&p.folder.join("model.onnx_data")),
            vec![b'x'; WEIGHTS.len()],
            "Testannahme: der Tausch hat stattgefunden"
        );
    } else {
        #[cfg(unix)]
        panic!("Unix: der Tausch muss gehen");
        #[cfg(not(unix))]
        assert_eq!(states, [Check::Valid, Check::Valid]);
    }
}

/// Grenze, ehrlich festgehalten: der schon gelesene Graph wird IN PLACE (gleicher Inode,
/// gleiche Groesse) ueberschrieben, nachdem er gelesen und gehasht wurde (`Checked(0)`).
/// Dieselbe Pruefung sieht das nicht: Handle und Name zeigen weiter auf dieselbe Datei,
/// das Ergebnis ist `Valid`, und der Sammler (`sink`) hat die ALTEN, geprueften Bytes
/// (eine Sitzung, daraus gebaut, ist unberuehrt). Der naechste Lauf sieht die neuen
/// Bytes am Hash. Deshalb ist `Valid` ein Hinweis und keine Zusage fuer den Bestand
/// danach: wer die Datei spaeter verwendet, muss sie selbst pruefen. Unter Windows
/// scheitert das Schreiben, weil die Pruefung die Datei mit `FILE_SHARE_READ` allein
/// offen haelt; dann gibt es die Aenderung nicht.
#[test]
fn an_in_place_change_of_the_already_checked_graph_is_not_seen_by_this_run_but_by_the_next() {
    let p = pair();
    let changed = Arc::new(AtomicBool::new(false));
    let mut got: [Vec<u8>; 2] = [Vec::new(), Vec::new()];
    let first = {
        let _hook = {
            let (folder, changed) = (p.folder.clone(), changed.clone());
            hook::set(p.anchor.root(), move |point| {
                if point == Point::Checked(0) {
                    let done = std::fs::write(folder.join("model.onnx"), vec![b'y'; GRAPH.len()]);
                    changed.store(done.is_ok(), Ordering::SeqCst);
                }
            })
        };
        let mut sink = |i: usize, block: &[u8]| got[i].extend_from_slice(block);
        run(&p, Some(&mut sink)).unwrap().unwrap()
    };
    let next = run(&p, None).unwrap().unwrap();

    if changed.load(Ordering::SeqCst) {
        assert_eq!(first, [Check::Valid, Check::Valid], "dokumentierte Grenze");
        assert_eq!(got[0], GRAPH, "der Sammler haelt die geprueften Bytes");
        assert_eq!(got[1], WEIGHTS);
        assert_eq!(
            next,
            [Check::Hash, Check::Valid],
            "der naechste Lauf sieht es"
        );
    } else {
        swap_must_have_happened();
        assert_eq!(next, [Check::Valid, Check::Valid]);
    }
}

/// Der Tausch gleich nach dem Oeffnen: die Bytes, die der Lader bekommt, sind die der
/// Handles (die alten), und die Datei zaehlt als geaendert, nie als gueltig.
#[test]
fn a_swap_right_after_opening_leaves_the_sink_with_the_old_bytes_and_the_state_changed() {
    let p = pair();
    let swapped = Arc::new(AtomicBool::new(false));
    let _hook = {
        let (folder, swapped) = (p.folder.clone(), swapped.clone());
        hook::set(p.anchor.root(), move |point| {
            if point == Point::Opened {
                let a = replace_file(&folder, "model.onnx", &vec![b'y'; GRAPH.len()]);
                let b = replace_file(&folder, "model.onnx_data", &vec![b'x'; WEIGHTS.len()]);
                swapped.store(a && b, Ordering::SeqCst);
            }
        })
    };
    let mut got: [Vec<u8>; 2] = [Vec::new(), Vec::new()];
    let mut sink = |i: usize, block: &[u8]| got[i].extend_from_slice(block);
    let states = run(&p, Some(&mut sink)).unwrap().unwrap();
    assert_eq!(
        got[0], GRAPH,
        "der Lader haette die getauschten Bytes bekommen"
    );
    assert_eq!(got[1], WEIGHTS);
    if swapped.load(Ordering::SeqCst) {
        assert_eq!(states, [Check::Changed, Check::Changed]);
    } else {
        #[cfg(unix)]
        panic!("Unix: der Tausch muss gehen");
    }
}

/// Eine Datei, die laenger ist als der Katalog sagt, wird nicht gelesen.
#[test]
fn an_oversized_file_is_not_read() {
    let p = pair();
    write(
        &p.folder.join("model.onnx_data"),
        &vec![b'x'; 10 * WEIGHTS.len()],
    );
    let reads = AtomicUsize::new(0);
    let mut sink = |_: usize, block: &[u8]| {
        reads.fetch_add(block.len(), Ordering::SeqCst);
    };
    let states = run(&p, Some(&mut sink)).unwrap().unwrap();
    assert_eq!(states[1], Check::Size(10 * WEIGHTS.len() as u64));
    assert_eq!(
        reads.load(Ordering::SeqCst),
        GRAPH.len(),
        "nur der Graph wurde gelesen"
    );
}

// ---- Datentraeger ohne Datei-Kennung ---------------------------------------------------

/// Ohne bekannte Kennung gibt es keinen Anker und keinen Namensvergleich, und das ist ein
/// eigener Fehler (`volume_unsupported`), keine Ablehnung wegen eines Links. Die Kennung
/// wird hier eingespielt (Unix liefert immer eine); das Verhalten der Windows-Abfrage
/// selbst ist nur typgeprueft.
#[cfg(unix)]
mod unknown_ids {
    use super::*;
    use crate::managers::model::download::safe::inject::UnknownIdentities;

    fn assert_volume_unsupported<T: Debug>(result: io::Result<T>) {
        let err = result.expect_err("haette abgelehnt werden muessen");
        assert!(
            safe::is_volume_unsupported(&err),
            "kein volume_unsupported, sondern: {err}"
        );
        assert!(
            !safe::is_refusal(&err),
            "als Link-Ablehnung gemeldet: {err}"
        );
        assert!(
            !err.to_string().contains(std::path::MAIN_SEPARATOR),
            "Text nennt einen Pfad: {err}"
        );
    }

    #[test]
    fn without_a_file_id_there_is_no_anchor_and_the_error_says_why() {
        let dir = TempDir::new().unwrap();
        let root = dir.path().join("models");
        {
            let _unknown = UnknownIdentities::on();
            assert_volume_unsupported(Anchor::open(&root, &["diarization", "id"], true));
            assert_volume_unsupported(Anchor::open(&root, &["diarization", "id"], false));
            assert_volume_unsupported(Anchor::open(&root, &[], false));
        }
        // Gegenprobe: mit Kennung geht es, und was vorher angelegt wurde, stoert nicht.
        let anchor = Anchor::open(&root, &["diarization", "id"], true)
            .unwrap()
            .unwrap();
        anchor.still_at().unwrap();
    }

    #[test]
    fn a_file_or_the_folder_check_without_a_file_id_is_volume_unsupported_not_a_refusal() {
        let dir = TempDir::new().unwrap();
        let anchor = open(dir.path(), &["m"]);
        let folder = dir.path().join("m");
        write(&folder.join("a.bin"), b"eins");
        // Ein Anker ohne Kettenstufen (der Ordner selbst ist die Wurzel): kein `step`, nur
        // `Anchor::open` und `still_at` sehen die Kennung des Ordners.
        let flat = open(&folder, &[]);
        let file = anchor.open_read("a.bin").unwrap();
        {
            let _unknown = UnknownIdentities::on();
            assert_volume_unsupported(anchor.open_read("a.bin"));
            assert_volume_unsupported(anchor.open_partial("neu.partial"));
            assert_volume_unsupported(anchor.names("a.bin", &file));
            assert_volume_unsupported(anchor.still_at());
            assert_volume_unsupported(flat.still_at());
            let wanted = [Wanted {
                name: "a.bin",
                size: 4,
                sha256: None,
            }];
            assert_volume_unsupported(check_set(&anchor, &wanted, &|| false, None));
        }
        assert!(anchor.names("a.bin", &file).unwrap());
        anchor.still_at().unwrap();
        flat.still_at().unwrap();
    }
}
