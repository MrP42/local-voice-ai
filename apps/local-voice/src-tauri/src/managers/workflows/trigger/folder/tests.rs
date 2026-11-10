//! Ordner-Ausloeser (B3, AK5): Stabilitaet, Ledger, Umbenennen, Konfliktkopie, Platzhalter.
//!
//! Die Dateien liegen in echten Ordnern (`tempfile`); Uhr und Takt sind von Hand gestellt, die
//! Auskunft des Systems (Platzhalter, Sperre) kommt aus einer Attrappe. Nichts beruehrt
//! produktive Daten.

use std::collections::HashSet;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use serde_json::{json, Value};

use super::*;
use crate::ablage::eintrag::{self, Gattung};
use crate::managers::integrations::model::{Capability, GrantMode, NewIntegration};
use crate::managers::integrations::store as integrations;
use crate::managers::workflows::engine::Engine;
use crate::managers::workflows::store::{self, RunFilter};
use crate::managers::workflows::test_support::{
    armed_workflow, def, engine_with, note, roomy_gate, set_grant, FakeClock, Fx, T0,
};
use crate::managers::workflows::trigger::test_sink::FlakySink;

const TICK: i64 = 15_000;

/// Attrappe fuer die Auskunft des Systems.
#[derive(Default)]
struct TestProbe {
    cloud: Mutex<HashSet<String>>,
    locked: Mutex<HashSet<String>>,
    locked_calls: AtomicUsize,
}

impl TestProbe {
    fn name(path: &Path) -> String {
        path.file_name().unwrap().to_string_lossy().into_owned()
    }
    fn set_cloud(&self, name: &str, on: bool) {
        let mut c = self.cloud.lock().unwrap();
        if on {
            c.insert(name.to_string());
        } else {
            c.remove(name);
        }
    }
    fn set_locked(&self, name: &str, on: bool) {
        let mut c = self.locked.lock().unwrap();
        if on {
            c.insert(name.to_string());
        } else {
            c.remove(name);
        }
    }
}

impl FileProbe for TestProbe {
    fn cloud_only(&self, path: &Path, _roh: &Roh) -> bool {
        self.cloud.lock().unwrap().contains(&Self::name(path))
    }
    fn locked(&self, path: &Path) -> bool {
        self.locked_calls.fetch_add(1, Ordering::SeqCst);
        self.locked.lock().unwrap().contains(&Self::name(path))
    }
}

struct World {
    fx: Fx,
    clock: Arc<FakeClock>,
    engine: Engine,
    inbox: tempfile::TempDir,
    wf: String,
    state: State,
    probe: TestProbe,
    now: i64,
}

fn trigger(extra: Value) -> Value {
    let mut t = json!({"type": KIND, "integration": "eingang", "stable_seconds": 2});
    for (k, v) in extra.as_object().unwrap() {
        t[k] = v.clone();
    }
    t
}

fn add_folder_integration(fx: &Fx, id: &str, path: &Path) {
    let mut n = NewIntegration::new(Kind::Folder, "Eingang");
    n.id = Some(id.to_string());
    n.config = json!({"path": path.to_string_lossy(), "subfolder": ""});
    integrations::create(&fx.conn(), &n, T0).unwrap();
}

fn world_with(extra: Value) -> World {
    let fx = Fx::new();
    let clock = FakeClock::new();
    let engine = engine_with(&fx, &clock, roomy_gate());
    let inbox = tempfile::tempdir().unwrap();
    add_folder_integration(&fx, "eingang", inbox.path());
    let mut d = def(vec![note("a")]);
    d["trigger"] = trigger(extra);
    let wf = armed_workflow(&engine, &d);
    World {
        fx,
        clock,
        engine,
        inbox,
        wf,
        state: State::default(),
        probe: TestProbe::default(),
        now: T0 + 10 * TICK,
    }
}

fn world() -> World {
    world_with(json!({}))
}

impl World {
    fn put(&self, name: &str, bytes: &[u8]) -> PathBuf {
        let p = self.inbox.path().join(name);
        fs::write(&p, bytes).unwrap();
        p
    }

    /// Ein Takt; die Uhr geht dabei um einen Takt weiter.
    fn tick(&mut self) -> TickReport {
        self.now += TICK;
        self.clock.set(self.now);
        on_tick(
            &self.engine,
            &self.state,
            &self.fx.db_path,
            &self.probe,
            self.now,
        )
    }

    /// Ein Takt mit einer anderen Auskunft des Systems (z. B. `SystemProbe` mit Test-Naht).
    fn tick_with(&mut self, probe: &dyn FileProbe) -> TickReport {
        self.now += TICK;
        self.clock.set(self.now);
        on_tick(&self.engine, &self.state, &self.fx.db_path, probe, self.now)
    }

    /// Mehrere Takte; fasst die Berichte zusammen.
    fn ticks(&mut self, n: usize) -> TickReport {
        let mut all = TickReport::default();
        for _ in 0..n {
            let r = self.tick();
            all.absorb(r);
        }
        all
    }

    fn runs(&self) -> Vec<store::RunRow> {
        store::list_runs(
            &self.fx.conn(),
            &RunFilter {
                workflow_id: Some(self.wf.clone()),
                state: None,
            },
            100,
        )
        .unwrap()
    }
}

fn context_of(run: &store::RunRow) -> Value {
    serde_json::from_str(&run.context_json).unwrap()
}

// ---------------------------------------------------------------------------
// Stabilitaet
// ---------------------------------------------------------------------------

#[test]
fn a_file_is_taken_only_after_two_quiet_ticks_and_carries_its_data() {
    let mut w = world();
    w.put("Kundentermin.WAV", b"RIFF....audio");
    let first = w.tick();
    assert!(first.is_empty(), "beim ersten Sehen nie: {first:?}");
    assert_eq!(w.state.waiting(), 1);
    let second = w.tick();
    assert_eq!(second.started.len(), 1, "{second:?}");

    let runs = w.runs();
    assert_eq!(runs.len(), 1);
    assert!(runs[0].trigger_key.starts_with("file:"));
    assert_eq!(
        runs[0].trigger_key.len(),
        "file:".len() + 64,
        "SHA-256 in Hex"
    );
    let t = &context_of(&runs[0])["trigger"];
    assert_eq!(t["integration"], "eingang");
    assert_eq!(t["name"], "Kundentermin.WAV");
    assert_eq!(t["extension"], "wav", "klein geschrieben");
    assert_eq!(t["size"], 13);
    assert_eq!(t["content_hash"], runs[0].trigger_key["file:".len()..]);
    let path = t["path"].as_str().unwrap();
    assert!(path.ends_with("Kundentermin.WAV"), "{path}");
    assert!(!path.starts_with(r"\\?\"), "kein Verbatim-Praefix: {path}");
    assert!(Path::new(path).is_file(), "der gemeldete Pfad ist lesbar");
}

#[test]
fn a_file_written_in_parts_waits_until_it_stops_growing() {
    let mut w = world();
    let p = w.put("lang.wav", b"teil1");
    assert!(w.tick().is_empty());
    // Zwischen zwei Takten waechst sie: die Uhr beginnt von vorn.
    for part in ["teil2", "teil3", "teil4"] {
        let mut f = fs::OpenOptions::new().append(true).open(&p).unwrap();
        f.write_all(part.as_bytes()).unwrap();
        drop(f);
        let r = w.tick();
        assert!(r.started.is_empty(), "waehrend des Schreibens: {r:?}");
    }
    assert!(w.runs().is_empty());
    // Danach zwei ruhige Takte: jetzt ist sie fertig.
    let r = w.tick();
    assert_eq!(r.started.len(), 1, "{r:?}");
    let size = context_of(&w.runs()[0])["trigger"]["size"]
        .as_u64()
        .unwrap();
    assert_eq!(size, 20, "die ganze Datei, nicht ein Teil");
}

#[test]
fn stable_seconds_is_respected_not_only_the_number_of_ticks() {
    let mut w = world_with(json!({"stable_seconds": 60}));
    w.put("a.wav", b"inhalt");
    // 15, 30, 45 s: noch zu frisch (beim ersten Sehen beginnt die Uhr).
    for i in 0..4 {
        assert!(w.tick().started.is_empty(), "Takt {i}");
    }
    let r = w.tick();
    assert_eq!(r.started.len(), 1, "nach 60 s: {r:?}");
}

#[test]
fn a_file_held_open_by_a_writer_is_not_taken_until_it_is_released() {
    let mut w = world();
    w.put("gesperrt.wav", b"inhalt");
    w.probe.set_locked("gesperrt.wav", true);
    for _ in 0..4 {
        assert!(w.tick().started.is_empty());
    }
    assert!(w.runs().is_empty());
    assert!(w.probe.locked_calls.load(Ordering::SeqCst) >= 1);
    w.probe.set_locked("gesperrt.wav", false);
    // Nach der Freigabe beginnt die Stabilitaet von vorn: ein Takt zum Sehen, einer zum Nehmen.
    assert_eq!(w.ticks(2).started.len(), 1);
}

#[cfg(windows)]
#[test]
fn the_system_probe_sees_a_writer_on_windows() {
    use std::os::windows::fs::OpenOptionsExt;
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("w.wav");
    fs::write(&p, b"x").unwrap();
    let probe = SystemProbe;
    assert!(!probe.locked(&p), "niemand schreibt");
    let writer = fs::OpenOptions::new()
        .write(true)
        .share_mode(1 | 2) // lesen und schreiben fuer andere erlaubt, wie ein Kopiervorgang
        .open(&p)
        .unwrap();
    assert!(probe.locked(&p), "ein Schreiber haelt sie offen");
    drop(writer);
    assert!(!probe.locked(&p), "wieder frei");
    let roh = eintrag::roh(&p).unwrap();
    assert!(
        !probe.cloud_only(&p, &roh),
        "eine gewoehnliche Datei ist lokal"
    );
}

// ---------------------------------------------------------------------------
// Attribute aus `ablage::eintrag` (Issue 451): Entscheidung rein, Windows-Werte ueber die Test-Naht
// ---------------------------------------------------------------------------

const ATTR_ARCHIV: u32 = 0x20;
const ATTR_REPARSE: u32 = 0x400;
const ATTR_SPARSE: u32 = 0x200;
const ATTR_UNPINNED: u32 = 0x10_0000;
const ONEDRIVE_TAG: u32 = 0x9000_701A;

fn roh_with(gattung: Gattung, attribute: u32, tag: u32) -> Roh {
    Roh {
        gattung,
        attribute,
        tag,
        groesse: 10,
        zeit: 1,
        nlink: 1,
    }
}

#[test]
fn the_decision_to_fire_comes_from_the_attributes_alone() {
    use EntryState::*;
    let datei = |a, t| entry_state(&roh_with(Gattung::Datei, a, t));
    // Wie Windows einen nur-online-Platzhalter liefert: mit und ohne REPARSE_POINT/OFFLINE.
    assert_eq!(
        datei(0x50_0020, 0),
        CloudOnly,
        "0x500020 (ohne Reparse/Offline)"
    );
    assert_eq!(
        datei(0x50_1620, ONEDRIVE_TAG),
        CloudOnly,
        "0x501620 (mit Reparse/Offline)"
    );
    assert_eq!(datei(0x40_0000, 0), CloudOnly, "nur RECALL_ON_DATA_ACCESS");
    assert_eq!(datei(0x4_0000, 0), CloudOnly, "nur RECALL_ON_OPEN");
    assert_eq!(datei(0x1000, 0), CloudOnly, "nur OFFLINE");
    assert_eq!(
        datei(0x1000 | ATTR_REPARSE, 0xA000_000C),
        CloudOnly,
        "nur online schlaegt den Tag"
    );
    // Lokal: gewoehnlich, angeheftet, „Platz freigeben“ ohne Abruf-Bit, hydrierter Platzhalter.
    assert_eq!(datei(ATTR_ARCHIV, 0), Local);
    assert_eq!(datei(0, 0), Local);
    assert_eq!(datei(0x8_0000 | ATTR_ARCHIV, 0), Local, "PINNED");
    assert_eq!(
        datei(ATTR_UNPINNED | ATTR_ARCHIV, 0),
        Local,
        "UNPINNED allein"
    );
    assert_eq!(
        datei(ATTR_REPARSE | ATTR_SPARSE | ATTR_ARCHIV, ONEDRIVE_TAG),
        Local,
        "hydrierter Cloud-Platzhalter"
    );
    // Verknuepfungen und unbekannte Reparse-Points: nie.
    assert_eq!(datei(ATTR_REPARSE, 0xA000_000C), Detour, "Symlink");
    assert_eq!(datei(ATTR_REPARSE, 0xA000_0003), Detour, "Junction");
    assert_eq!(datei(ATTR_REPARSE, 0x8000_0018), Detour, "fremdes Tag");
    // Datentransparent (kein Abruf, normales Oeffnen liefert den Inhalt): lokale Dateien.
    assert_eq!(
        datei(ATTR_REPARSE | ATTR_ARCHIV, 0x8000_0017),
        Local,
        "WOF (compact /exe)"
    );
    assert_eq!(
        datei(ATTR_REPARSE | ATTR_SPARSE, 0x8000_0013),
        Local,
        "Dedup"
    );
    assert_eq!(
        datei(ATTR_REPARSE | 0x40_0000, 0x8000_0017),
        CloudOnly,
        "nur online schlaegt auch ein transparentes Tag"
    );
    assert_eq!(
        datei(0x8000_0017, 0x8000_0017),
        Local,
        "ohne Reparse-Bit ist das Tag belanglos"
    );
    // Keine Datei.
    assert_eq!(entry_state(&roh_with(Gattung::Ordner, 0x10, 0)), NotAFile);
    assert_eq!(entry_state(&roh_with(Gattung::Link, 0, 0)), NotAFile);
    assert_eq!(entry_state(&roh_with(Gattung::Anderes, 0, 0)), NotAFile);
}

#[test]
fn the_system_probe_judges_the_find_data_attributes() {
    let p = Path::new("x.wav");
    for (attrs, cloud) in [
        (0x50_0020, true),
        (0x50_1620, true),
        (0x4_0000, true),
        (0x1000, true),
        (ATTR_ARCHIV, false),
        (0, false),
        (0x8_0000, false),
        (ATTR_UNPINNED, false),
    ] {
        let roh = roh_with(Gattung::Datei, attrs, 0);
        assert_eq!(SystemProbe.cloud_only(p, &roh), cloud, "{attrs:#x}");
    }
}

#[test]
fn placeholders_reported_by_the_find_data_never_fire_and_are_never_hashed() {
    let mut w = world();
    let root = fs::canonicalize(w.inbox.path()).unwrap();
    for (name, attrs, tag) in [
        ("a-ohne-bits.wav", 0x50_0020, 0),
        ("b-voll.wav", 0x50_1620, ONEDRIVE_TAG),
        ("c-open.wav", 0x4_0000, 0),
        ("d-offline.wav", 0x1000, 0),
    ] {
        w.put(name, b"inhalt");
        eintrag::naht::setzen(&root.join(name), attrs, tag);
    }
    w.put("lokal.wav", b"lokal");
    eintrag::naht::setzen(&root.join("lokal.wav"), ATTR_ARCHIV, 0);
    // Wuerde eine der vier gehasht, laege ihr Inhalt im Ledger; ein Handle, das „nur online“ meldet,
    // lehnt `sha256_file` ausserdem ab (`the_hash_refuses_a_handle_that_became_online`).
    let mut all = TickReport::default();
    for _ in 0..6 {
        all.absorb(w.tick_with(&SystemProbe));
    }
    assert_eq!(all.started.len(), 1, "nur die lokale Datei: {all:?}");
    assert_eq!(all.skipped.len(), 4, "jede einmal gemeldet: {all:?}");
    assert_eq!(w.state.cloud_only().len(), 4);
    for line in &all.skipped {
        assert!(
            !line.contains(".wav"),
            "kein Dateiname im Bericht (Log): {line}"
        );
    }
    let mut names: Vec<String> = w.state.cloud_only().into_iter().map(|c| c.name).collect();
    names.sort();
    assert_eq!(
        names,
        [
            "a-ohne-bits.wav",
            "b-voll.wav",
            "c-open.wav",
            "d-offline.wav"
        ],
        "die Namen erreichen die Oberflaeche nur ueber `State::cloud_only`"
    );
    let runs = w.runs();
    assert_eq!(runs.len(), 1);
    assert_eq!(context_of(&runs[0])["trigger"]["name"], "lokal.wav");
    assert_eq!(
        w.state.waiting(),
        1,
        "nur die lokale Datei hat eine Stabilitaetsuhr"
    );
    eintrag::naht::leeren();
}

#[test]
fn a_placeholder_that_comes_down_is_taken_like_any_file() {
    let mut w = world();
    let root = fs::canonicalize(w.inbox.path()).unwrap();
    let p = w.put("spaeter.wav", b"inhalt");
    eintrag::naht::setzen(&root.join("spaeter.wav"), 0x50_0020, 0);
    assert!(w.tick_with(&SystemProbe).started.is_empty());
    assert_eq!(w.state.cloud_only().len(), 1);
    eintrag::naht::setzen(&p, ATTR_ARCHIV, 0);
    eintrag::naht::setzen(&root.join("spaeter.wav"), ATTR_ARCHIV, 0);
    let mut all = TickReport::default();
    for _ in 0..3 {
        all.absorb(w.tick_with(&SystemProbe));
    }
    assert_eq!(all.started.len(), 1, "{all:?}");
    assert!(w.state.cloud_only().is_empty());
    eintrag::naht::leeren();
}

#[test]
fn a_detour_is_ignored_silently() {
    let mut w = world();
    let root = fs::canonicalize(w.inbox.path()).unwrap();
    w.put("link.wav", b"inhalt");
    eintrag::naht::setzen(&root.join("link.wav"), ATTR_REPARSE, 0xA000_000C);
    let mut all = TickReport::default();
    for _ in 0..4 {
        all.absorb(w.tick_with(&SystemProbe));
    }
    assert!(all.started.is_empty() && all.skipped.is_empty(), "{all:?}");
    assert_eq!(w.state.waiting(), 0);
    eintrag::naht::leeren();
}

#[test]
fn the_hash_refuses_a_handle_that_became_online() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("x.wav");
    fs::write(&p, b"inhalt").unwrap();
    assert_eq!(sha256_file(&p).unwrap().len(), 64, "lokal: wird gehasht");
    // Zwischen Abtastung und Oeffnen wieder ausgelagert: das Handle meldet „nur online“.
    eintrag::naht::handle_setzen(&p, 0x50_0020, 0);
    let err = sha256_file(&p).expect_err("nicht lesen");
    assert_eq!(err.kind(), std::io::ErrorKind::Other);
    assert!(open_no_recall(&p).is_err());
    eintrag::naht::handle_setzen(&p, 0x50_1620, ONEDRIVE_TAG);
    assert!(sha256_file(&p).is_err());
    eintrag::naht::leeren();
    assert!(sha256_file(&p).is_ok(), "wieder lokal");
}

#[test]
fn a_file_that_goes_online_while_it_is_hashed_is_not_taken() {
    let mut w = world();
    let root = fs::canonicalize(w.inbox.path()).unwrap();
    w.put("fluechtig.wav", b"inhalt");
    // Die Abtastung sieht eine lokale Datei, das Handle beim Hashen „nur online“.
    eintrag::naht::handle_setzen(&root.join("fluechtig.wav"), 0x50_0020, 0);
    let mut all = TickReport::default();
    for _ in 0..4 {
        all.absorb(w.tick_with(&SystemProbe));
    }
    assert!(all.started.is_empty(), "{all:?}");
    assert!(w.runs().is_empty());
    eintrag::naht::leeren();
    assert_eq!(w.ticks(2).started.len(), 1);
}

/// Handle-Urteil fuer transparente Tags: eine WOF-komprimierte Datei wird gehasht, ein fremdes Tag nicht.
#[test]
fn the_hash_accepts_a_transparent_reparse_handle_and_refuses_a_foreign_one() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("w.wav");
    fs::write(&p, b"inhalt").unwrap();
    eintrag::naht::handle_setzen(&p, ATTR_REPARSE | ATTR_ARCHIV, 0x8000_0017);
    assert!(sha256_file(&p).is_ok(), "WOF");
    eintrag::naht::handle_setzen(&p, ATTR_REPARSE | ATTR_SPARSE, 0x8000_0013);
    assert!(sha256_file(&p).is_ok(), "Dedup");
    eintrag::naht::handle_setzen(&p, ATTR_REPARSE, 0x8000_0018);
    assert!(sha256_file(&p).is_err(), "fremdes Tag");
    eintrag::naht::handle_setzen(&p, ATTR_REPARSE, 0xA000_000C);
    assert!(sha256_file(&p).is_err(), "Symlink");
    eintrag::naht::leeren();
}

#[test]
fn the_hash_reports_size_and_time_from_the_handle() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("w.wav");
    fs::write(&p, b"inhalt").unwrap();
    let h = hash_file(&p).unwrap();
    let roh = eintrag::roh(&p).unwrap();
    assert_eq!(h.size, roh.groesse);
    assert_eq!(h.mtime_ms, roh.zeit);
    assert_eq!(h.hash, sha256_file(&p).unwrap());
}

/// Eine Attrappe, die die Datei zwischen Stabilitaetspruefung und Hash veraendert (wie ein Schreiber,
/// dessen Verzeichniseintrag noch nicht nachgezogen hat).
struct WriterProbe(PathBuf, AtomicUsize);

impl FileProbe for WriterProbe {
    fn cloud_only(&self, _path: &Path, _roh: &Roh) -> bool {
        false
    }
    fn locked(&self, _path: &Path) -> bool {
        if self.1.fetch_add(1, Ordering::SeqCst) == 0 {
            let mut f = fs::OpenOptions::new().append(true).open(&self.0).unwrap();
            f.write_all(b"-mehr").unwrap();
        }
        false
    }
}

#[test]
fn a_file_that_grows_before_the_hash_is_not_taken_until_it_is_quiet_again() {
    let mut w = world();
    let p = w.put("waechst.wav", b"inhalt");
    let probe = WriterProbe(p, AtomicUsize::new(0));
    let mut all = TickReport::default();
    for _ in 0..2 {
        all.absorb(w.tick_with(&probe));
    }
    assert!(
        all.started.is_empty(),
        "gewachsen waehrend des Hashens: {all:?}"
    );
    let mut after = TickReport::default();
    for _ in 0..3 {
        after.absorb(w.tick_with(&probe));
    }
    assert_eq!(after.started.len(), 1, "danach ruhig: {after:?}");
    assert_eq!(context_of(&w.runs()[0])["trigger"]["size"], 11);
}

// ---------------------------------------------------------------------------
// Ledger und Doppelverarbeitung
// ---------------------------------------------------------------------------

#[test]
fn the_ledger_prevents_a_second_run_after_a_restart_and_does_not_even_hash_again() {
    let mut w = world();
    w.put("einmal.wav", b"inhalt eins");
    assert_eq!(w.ticks(2).started.len(), 1);
    let locked_before = w.probe.locked_calls.load(Ordering::SeqCst);

    // Neustart: neuer Zustand (keine Beobachtungen), neue Engine auf derselben Datenbank.
    w.state = State::default();
    w.engine = engine_with(&w.fx, &w.clock, roomy_gate());
    let r = w.ticks(4);
    assert!(r.started.is_empty(), "{r:?}");
    assert_eq!(r.duplicates, 0, "nicht einmal ein Einreihversuch");
    assert_eq!(w.runs().len(), 1);
    assert_eq!(
        w.probe.locked_calls.load(Ordering::SeqCst),
        locked_before,
        "die Datei wurde nach dem Neustart nicht erneut angefasst"
    );
    // Das Ledger kennt Pfad und Inhalt.
    let conn = w.fx.conn();
    assert_eq!(ledger::count(&conn, "p:").unwrap(), 1);
    assert_eq!(ledger::count(&conn, "h:").unwrap(), 1);
}

#[test]
fn without_a_ledger_the_engine_still_makes_it_one_run() {
    let mut w = world();
    w.put("x.wav", b"gleicher inhalt");
    assert_eq!(w.ticks(2).started.len(), 1);
    // Das Ledger ist weg (zu alt, beschaedigt): der Schluessel `file:<hash>` haelt.
    w.fx.conn()
        .execute("DELETE FROM workflow_file_ledger", [])
        .unwrap();
    w.state = State::default();
    let r = w.ticks(2);
    assert!(r.started.is_empty(), "{r:?}");
    assert_eq!(r.duplicates, 1, "die Engine erkennt den Schluessel wieder");
    assert_eq!(w.runs().len(), 1);
    assert_eq!(
        ledger::count(&w.fx.conn(), "h:").unwrap(),
        1,
        "und das Ledger ist wieder gefuellt"
    );
}

#[test]
fn renaming_a_processed_file_makes_no_second_run() {
    let mut w = world();
    let p = w.put("Besprechung.wav", b"audio audio audio");
    assert_eq!(w.ticks(2).started.len(), 1);
    fs::rename(&p, w.inbox.path().join("Besprechung final.wav")).unwrap();
    let r = w.ticks(4);
    assert!(r.started.is_empty(), "{r:?}");
    assert_eq!(r.duplicates, 1, "inhaltsgleich erkannt, einmal");
    assert_eq!(w.runs().len(), 1);
    // Weitere Takte: nichts mehr zu tun (der neue Pfad steht im Ledger).
    let again = w.ticks(3);
    assert!(again.is_empty(), "{again:?}");
}

#[test]
fn a_conflict_copy_from_the_sync_client_makes_no_second_run() {
    let mut w = world();
    w.put("Aufnahme.wav", b"identischer inhalt");
    assert_eq!(w.ticks(2).started.len(), 1);
    // OneDrive: Name-RECHNER.ext und Name (1).ext.
    w.put("Aufnahme-DESKTOP-ABC123.wav", b"identischer inhalt");
    w.put("Aufnahme (1).wav", b"identischer inhalt");
    let r = w.ticks(5);
    assert!(r.started.is_empty(), "{r:?}");
    assert_eq!(w.runs().len(), 1, "genau ein Lauf fuer den einen Inhalt");
}

#[test]
fn new_content_under_an_old_name_is_a_new_run() {
    let mut w = world();
    let p = w.put("Notiz.wav", b"erste fassung");
    assert_eq!(w.ticks(2).started.len(), 1);
    fs::write(&p, b"zweite, andere fassung").unwrap();
    let r = w.ticks(3);
    assert_eq!(r.started.len(), 1, "{r:?}");
    assert_eq!(w.runs().len(), 2);
}

#[test]
fn touching_a_file_without_changing_it_makes_no_run() {
    let mut w = world();
    let p = w.put("t.wav", b"unveraendert");
    assert_eq!(w.ticks(2).started.len(), 1);
    let f = fs::OpenOptions::new().write(true).open(&p).unwrap();
    f.set_modified(std::time::SystemTime::now() + std::time::Duration::from_secs(3600))
        .unwrap();
    drop(f);
    let r = w.ticks(4);
    assert!(r.started.is_empty(), "{r:?}");
    assert_eq!(r.duplicates, 1);
    assert_eq!(w.runs().len(), 1);
}

#[test]
fn different_files_each_get_a_run_and_hashing_is_limited_per_tick() {
    let mut w = world();
    for i in 0..5 {
        w.put(&format!("f{i}.wav"), format!("inhalt {i}").as_bytes());
    }
    assert!(w.tick().is_empty());
    // Ab dem zweiten Takt sind alle stabil; je Takt werden hoechstens zwei gehasht.
    assert_eq!(w.tick().started.len(), MAX_HASH_PER_SCAN);
    assert_eq!(w.tick().started.len(), MAX_HASH_PER_SCAN);
    assert_eq!(w.tick().started.len(), 1);
    assert!(w.tick().is_empty());
    assert_eq!(w.runs().len(), 5);
}

#[test]
fn two_workflows_on_the_same_folder_each_get_their_own_run() {
    let mut w = world();
    let mut d = def(vec![note("b")]);
    d["name"] = json!("Zweiter");
    d["trigger"] = trigger(json!({}));
    let second = armed_workflow(&w.engine, &d);
    w.put("beide.wav", b"fuer beide");
    let r = w.ticks(2);
    assert_eq!(r.started.len(), 2, "{r:?}");
    assert_eq!(w.runs().len(), 1);
    let conn = w.fx.conn();
    let other = store::list_runs(
        &conn,
        &RunFilter {
            workflow_id: Some(second),
            state: None,
        },
        10,
    )
    .unwrap();
    assert_eq!(other.len(), 1);
}

// ---------------------------------------------------------------------------
// Auswahl der Dateien
// ---------------------------------------------------------------------------

#[test]
fn only_wanted_files_count_not_temp_files_folders_or_other_types() {
    let mut w = world();
    for name in [
        "~$bericht.wav",
        "aufnahme.wav.tmp",
        ".versteckt.wav",
        "desktop.ini",
        "download.crdownload",
        "notiz.txt",
        "ohneendung",
    ] {
        w.put(name, b"x");
    }
    fs::create_dir(w.inbox.path().join("Ordner.wav")).unwrap();
    fs::write(w.inbox.path().join("Ordner.wav").join("tief.wav"), b"x").unwrap();
    w.put("gut.mp3", b"echt");
    let r = w.ticks(3);
    assert_eq!(r.started.len(), 1, "{r:?}");
    assert_eq!(context_of(&w.runs()[0])["trigger"]["name"], "gut.mp3");
    assert_eq!(w.state.waiting(), 1, "nur die eine Datei wird beobachtet");
}

#[test]
fn the_extension_list_of_the_trigger_narrows_the_choice() {
    let mut w = world_with(json!({"extensions": ["PDF", ".md"]}));
    w.put("a.wav", b"audio");
    w.put("b.pdf", b"pdf");
    w.put("c.md", b"md");
    let r = w.ticks(3);
    assert_eq!(r.started.len(), 2, "{r:?}");
    let mut names: Vec<String> = w
        .runs()
        .iter()
        .map(|r| {
            context_of(r)["trigger"]["name"]
                .as_str()
                .unwrap()
                .to_string()
        })
        .collect();
    names.sort();
    assert_eq!(names, vec!["b.pdf", "c.md"]);
}

#[test]
fn the_subfolder_field_selects_another_folder_and_the_root_is_not_scanned() {
    let mut w = world_with(json!({"subfolder": "Protokolle/Eingang"}));
    let sub = w.inbox.path().join("Protokolle").join("Eingang");
    fs::create_dir_all(&sub).unwrap();
    w.put("im-wurzelordner.wav", b"nein");
    fs::write(sub.join("im-unterordner.wav"), b"ja").unwrap();
    let r = w.ticks(3);
    assert_eq!(r.started.len(), 1, "{r:?}");
    assert_eq!(
        context_of(&w.runs()[0])["trigger"]["name"],
        "im-unterordner.wav"
    );
}

#[test]
fn a_file_that_vanishes_between_ticks_is_forgotten_without_a_run() {
    let mut w = world();
    let p = w.put("weg.wav", b"kurz da");
    assert!(w.tick().is_empty());
    assert_eq!(w.state.waiting(), 1);
    fs::remove_file(&p).unwrap();
    let r = w.ticks(2);
    assert!(r.is_empty(), "{r:?}");
    assert_eq!(w.state.waiting(), 0);
    assert!(w.runs().is_empty());
}

// ---------------------------------------------------------------------------
// OneDrive-Platzhalter
// ---------------------------------------------------------------------------

#[test]
fn a_cloud_only_placeholder_is_reported_once_never_read_and_taken_after_it_is_local() {
    let mut w = world();
    w.put("nur-cloud.wav", b"inhalt");
    w.probe.set_cloud("nur-cloud.wav", true);
    let r = w.ticks(6);
    assert!(r.started.is_empty(), "{r:?}");
    assert_eq!(
        r.skipped.len(),
        1,
        "einmal gemeldet, nicht bei jedem Takt: {r:?}"
    );
    assert!(r.skipped[0].contains("Cloud"), "{}", r.skipped[0]);
    assert!(
        !r.skipped[0].contains("nur-cloud.wav"),
        "`skipped` geht ins Log: kein Dateiname: {}",
        r.skipped[0]
    );
    assert_eq!(
        w.probe.locked_calls.load(Ordering::SeqCst),
        0,
        "nicht einmal die Sperre wurde geprueft (jedes Oeffnen waere ein Download)"
    );
    let listed = w.state.cloud_only();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].name, "nur-cloud.wav");
    assert_eq!(
        w.state.waiting(),
        0,
        "keine Stabilitaetsuhr fuer einen Platzhalter"
    );

    // Der Nutzer behaelt sie auf dem Geraet: jetzt zaehlt sie wie jede Datei.
    w.probe.set_cloud("nur-cloud.wav", false);
    let r = w.ticks(2);
    assert_eq!(r.started.len(), 1, "{r:?}");
    assert!(w.state.cloud_only().is_empty());
}

#[test]
fn the_cloud_attribute_bits_are_recognised() {
    assert!(attributes_cloud_only(0x0040_0000), "RECALL_ON_DATA_ACCESS");
    assert!(attributes_cloud_only(0x0004_0000), "RECALL_ON_OPEN");
    assert!(attributes_cloud_only(0x0000_1000), "OFFLINE");
    assert!(
        attributes_cloud_only(0x0040_0020),
        "mit anderen Bits gemischt"
    );
    // Normale, angeheftete (PINNED) und „Platz freigeben“-Dateien (UNPINNED) sind lokal.
    assert!(!attributes_cloud_only(0x0000_0020), "ARCHIVE");
    assert!(!attributes_cloud_only(0x0008_0000), "PINNED");
    assert!(!attributes_cloud_only(0x0010_0000), "UNPINNED");
    assert!(!attributes_cloud_only(0));
}

// ---------------------------------------------------------------------------
// Fehlerfaelle des Einreihens, Rechte, Konfiguration
// ---------------------------------------------------------------------------

#[test]
fn a_failed_enqueue_loses_nothing_and_is_retried_by_the_next_tick() {
    let mut w = world();
    w.put("wichtig.wav", b"muss ankommen");
    w.tick();
    let sink = FlakySink::new(&w.engine, 1); // das erste Einreihen scheitert (Platte voll)
    w.now += TICK;
    let r = on_tick(&sink, &w.state, &w.fx.db_path, &w.probe, w.now);
    assert_eq!(r.errors.len(), 1, "{r:?}");
    assert!(r.started.is_empty());
    assert_eq!(
        ledger::count(&w.fx.conn(), "").unwrap(),
        0,
        "ungesehen: nichts im Ledger"
    );
    w.now += TICK;
    let r = on_tick(&sink, &w.state, &w.fx.db_path, &w.probe, w.now);
    assert_eq!(r.started.len(), 1, "der naechste Takt holt es nach: {r:?}");
    assert_eq!(w.runs().len(), 1);
}

#[test]
fn a_missing_integration_or_folder_is_reported_once_per_change_not_every_tick() {
    let mut w = world();
    let gone = w.inbox.path().to_path_buf();
    // Ordner verschwunden (z. B. OneDrive nicht eingehaengt).
    w.state = State::default();
    let moved = gone.with_extension("weg");
    fs::rename(&gone, &moved).unwrap();
    let first = w.tick();
    assert_eq!(first.skipped.len(), 1, "{first:?}");
    assert!(first.skipped[0].contains("Eingang") || first.skipped[0].contains("nicht"));
    let more = w.ticks(4);
    assert!(
        more.skipped.is_empty(),
        "dasselbe Problem nicht erneut: {more:?}"
    );
    // Ordner wieder da: das Problem ist geloest, Dateien werden genommen.
    fs::rename(&moved, &gone).unwrap();
    w.put("wieder.wav", b"da");
    assert_eq!(w.ticks(2).started.len(), 1);
    // Integration geloescht: neues Problem, einmal gemeldet.
    integrations::delete(&w.fx.conn(), "eingang").unwrap();
    let r = w.ticks(3);
    assert_eq!(r.skipped.len(), 1, "{r:?}");
    assert!(r.skipped[0].contains("gibt es nicht"));
}

#[test]
fn the_right_files_read_off_skips_the_workflow_and_ask_still_enqueues() {
    let mut w = world();
    w.put("recht.wav", b"inhalt");
    set_grant(
        &w.fx.conn(),
        "eingang",
        Capability::FilesRead,
        GrantMode::Off,
    );
    let r = w.ticks(3);
    assert!(r.started.is_empty(), "{r:?}");
    assert_eq!(r.skipped.len(), 1);
    assert!(r.skipped[0].contains("Recht") || r.skipped[0].contains("aus"));
    assert_eq!(w.state.waiting(), 0, "ohne Recht wird nichts beobachtet");
    // „fragen“: eingereiht; gefragt wird beim Import-Schritt (Tor).
    set_grant(
        &w.fx.conn(),
        "eingang",
        Capability::FilesRead,
        GrantMode::Ask,
    );
    assert_eq!(w.ticks(2).started.len(), 1);
}

#[test]
fn a_disabled_workflow_never_touches_the_folder() {
    let mut w = world();
    w.put("a.wav", b"x");
    w.engine.set_enabled(&w.wf, false).unwrap();
    let r = w.ticks(4);
    assert!(r.is_empty(), "{r:?}");
    assert_eq!(w.state.waiting(), 0);
    assert_eq!(w.probe.locked_calls.load(Ordering::SeqCst), 0);
    assert!(w.runs().is_empty());
}

#[test]
fn a_second_scan_at_the_same_time_is_refused_not_queued() {
    let w = world();
    w.put("a.wav", b"x");
    let guard = w.state.try_begin().expect("frei");
    assert!(w.state.try_begin().is_none(), "ein Platz");
    let r = on_tick(&w.engine, &w.state, &w.fx.db_path, &w.probe, w.now + TICK);
    assert_eq!(r.skipped.len(), 1, "{r:?}");
    assert_eq!(
        w.state.waiting(),
        0,
        "der verweigerte Scan hat nichts angefasst"
    );
    drop(guard);
    assert!(
        w.state.try_begin().is_some(),
        "der Drop gibt den Platz frei"
    );
}

#[test]
fn the_definition_check_names_a_bad_subfolder_and_bad_extensions() {
    let w = world();
    let mut d = def(vec![note("a")]);
    d["trigger"] = trigger(json!({"subfolder": "../draussen", "extensions": ["wav", "a/b"]}));
    let err = w.engine.save_workflow(None, &d).unwrap_err();
    let store::WorkflowError::Invalid(issues) = err else {
        panic!("erwartet Invalid");
    };
    let paths: Vec<&str> = issues.iter().map(|i| i.path.as_str()).collect();
    assert!(paths.contains(&"/trigger/subfolder"), "{issues:?}");
    assert!(paths.contains(&"/trigger/extensions"), "{issues:?}");
    // Ein gueltiger Ausloeser wird gespeichert.
    d["trigger"] = trigger(json!({"subfolder": "Eingang/2026", "extensions": [".wav", "MP3"]}));
    w.engine.save_workflow(None, &d).unwrap();
}

// ---------------------------------------------------------------------------
// Kleine reine Funktionen
// ---------------------------------------------------------------------------

#[test]
fn temp_and_system_names_are_ignored() {
    for n in [
        "~$a.docx",
        "a.tmp",
        "A.PART",
        ".hidden",
        "x.crdownload",
        "Desktop.ini",
        "THUMBS.DB",
        "b.wav.partial",
    ] {
        assert!(is_ignored_name(n), "{n}");
    }
    for n in ["a.wav", "Protokoll 1.mp3", "Notiz~1.wav", "tmp.wav"] {
        assert!(!is_ignored_name(n), "{n}");
    }
}

#[test]
fn verbatim_windows_paths_are_shown_the_way_users_know_them() {
    assert_eq!(
        display_path(Path::new(r"\\?\C:\Eingang\a.wav")),
        r"C:\Eingang\a.wav"
    );
    assert_eq!(
        display_path(Path::new(r"\\?\UNC\server\share\a.wav")),
        r"\\server\share\a.wav"
    );
    assert_eq!(display_path(Path::new("/tmp/a.wav")), "/tmp/a.wav");
}

#[test]
fn the_hash_is_the_sha256_of_the_content_regardless_of_name() {
    let dir = tempfile::tempdir().unwrap();
    let a = dir.path().join("a.bin");
    let b = dir.path().join("kopie von a.bin");
    fs::write(&a, b"abc").unwrap();
    fs::write(&b, b"abc").unwrap();
    assert_eq!(
        sha256_file(&a).unwrap(),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
    assert_eq!(sha256_file(&a).unwrap(), sha256_file(&b).unwrap());
    // Ueber mehrere Bloecke.
    let big = dir.path().join("big.bin");
    fs::write(&big, vec![7u8; HASH_BLOCK * 2 + 5]).unwrap();
    assert_eq!(sha256_file(&big).unwrap().len(), 64);
}
