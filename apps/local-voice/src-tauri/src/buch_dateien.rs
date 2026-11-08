//! lva#365 P3: Notizen, Anhänge und Transkript-Text im Buch-Ordner und ihre Sync-Sammlung
//! `book_file` (Spec `2026-11-07-buch-ordner-und-projekte.md`, 6.2). Kern ohne Tauri: Er
//! kennt nur den Buch-Ordner und seinen Stand (`sync-dateien.json`); Schlüssel, Blobs und
//! Hub bleiben in `sync/book_files.rs`.
//!
//! Ein Datensatz je Datei:
//! `{ v, id, book, edition, role, path, blob, updated, device, deleted }`.
//! - `id`: `bf_` + 40 Hex von SHA-256(Buch-ID + Rolle + relativer Pfad, ohne Trenner). Die
//!   Rolle ist eines von drei festen Wörtern und der Pfad beginnt mit dem Ordner der Rolle;
//!   die Verkettung ist damit eindeutig. Gleiche Datei, gleiche ID auf jedem Gerät.
//! - `path`: relativ zum Buch-Ordner (`anhaenge/…`, `notizen/…`, `transkript/…`), jedes
//!   Glied nach 4.4 bereinigt, nie absolut, nie `..`, nie versteckt.
//! - Konflikt: neueres `updated` gewinnt, bei Gleichstand die größere Geräte-ID. Die
//!   unterlegene Fassung bleibt als `Name (Konflikt Gerät).ext` erhalten.
//! - Löschen: nur ausdrücklich in der App. Eine lokal fehlende Datei bleibt „fehlt lokal“
//!   (vom Hub wiederherstellen oder in der App löschen). Grabstein (`deleted: true`) 30
//!   Tage; der Empfänger legt die Datei in den Papierkorb
//!   (`library/.papierkorb/dateien/<buch>/…`), nie ins Nichts.
//! - Datensätze mit `v` > [`VERSION`] werden aufbewahrt, aber nie geschrieben.

use crate::library_layout::{bereinigter_name, freier_name, schreibe_atomar, vergleichsschluessel};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

/// Name der Sync-Sammlung.
pub const SAMMLUNG: &str = "book_file";
/// Höchste Datensatz-Version, die dieses Gerät schreibt.
pub const VERSION: u64 = 1;
/// Stand der Dateien eines Buchs (im Buch-Ordner).
pub const STAND: &str = "sync-dateien.json";
/// Ablage für halb geladene Dateien (im Buch-Ordner, gleicher Datenträger wie das Ziel).
pub const ZWISCHENABLAGE: &str = ".sync-dateien";
/// So lange reist ein Grabstein mit.
pub const GRABSTEIN_MS: f64 = 30.0 * 24.0 * 60.0 * 60.0 * 1000.0;
/// Datensätze, deren Buch hier (noch) fehlt: `library/<PARKPLATZ>/<id>.json`.
pub const PARKPLATZ: &str = ".sync-dateien-wartend";
/// Unterordner des Bibliotheks-Papierkorbs für einzelne Dateien.
const PAPIERKORB_DATEIEN: &str = "dateien";
const PAPIERKORB: &str = ".papierkorb";
/// Höchste Tiefe unter dem Ordner der Rolle.
const TIEFE: usize = 4;
const STAND_VERSION: u32 = 1;

/// Was eine Datei im Buch-Ordner ist.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rolle {
    Anhang,
    Notiz,
    Transkript,
}

impl Rolle {
    pub const ALLE: [Rolle; 3] = [Rolle::Anhang, Rolle::Notiz, Rolle::Transkript];

    /// Name im Datensatz (`role`).
    pub fn name(self) -> &'static str {
        match self {
            Rolle::Anhang => "anhang",
            Rolle::Notiz => "notiz",
            Rolle::Transkript => "transkript",
        }
    }

    /// Ordner im Buch-Ordner.
    pub fn ordner(self) -> &'static str {
        match self {
            Rolle::Anhang => "anhaenge",
            Rolle::Notiz => "notizen",
            Rolle::Transkript => "transkript",
        }
    }

    /// Rolle nach dem ersten Glied eines relativen Pfads.
    pub fn aus_pfad(rel: &str) -> Option<Rolle> {
        let erstes = rel.split('/').next()?;
        Rolle::ALLE.into_iter().find(|r| r.ordner() == erstes)
    }
}

/// Stabile ID einer Datei im Buch.
pub fn datei_id(buch: &str, rolle: Rolle, rel: &str) -> String {
    let digest = Sha256::digest(format!("{buch}{}{rel}", rolle.name()).as_bytes());
    let hex: String = digest.iter().map(|b| format!("{b:02x}")).collect();
    format!("bf_{}", &hex[..40])
}

/// Eine gültige Datensatz-ID (auch als Dateiname auf dem Parkplatz).
pub fn gueltige_id(id: &str) -> bool {
    id.strip_prefix("bf_").is_some_and(|h| {
        h.len() == 40
            && h.bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    })
}

/// Ein relativer Pfad, den jedes Gerät gleich und sicher ablegen kann: Ordner der Rolle
/// vorn, darunter 1 bis [`TIEFE`] Glieder, jedes schon nach 4.4 bereinigt und nicht
/// versteckt. Liefert die Rolle.
pub fn sicherer_pfad(rel: &str) -> Result<Rolle, String> {
    let schlecht = || "Unzulässiger Pfad im Buch-Ordner".to_string();
    let rolle = Rolle::aus_pfad(rel).ok_or_else(schlecht)?;
    let glieder: Vec<&str> = rel.split('/').collect();
    if glieder.len() < 2 || glieder.len() > TIEFE + 1 {
        return Err(schlecht());
    }
    for glied in &glieder[1..] {
        if glied.starts_with('.') || bereinigter_name(glied).ok().as_deref() != Some(*glied) {
            return Err(schlecht());
        }
    }
    Ok(rolle)
}

/// Fassung, zu der eine Datei gehört: beim Transkript der Dateistamm
/// (`transkript/<fassung>.md`), sonst unbestimmt.
fn fassung_aus_pfad(rolle: Rolle, rel: &str) -> String {
    if rolle != Rolle::Transkript {
        return String::new();
    }
    match rel.split('/').collect::<Vec<_>>().as_slice() {
        [_, name] => name.strip_suffix(".md").unwrap_or("").to_string(),
        _ => String::new(),
    }
}

// ---------------------------------------------------------------------------
// Stand
// ---------------------------------------------------------------------------

/// Eine Datei im Stand: vorhanden, gelöscht (Grabstein) oder erwartet (Download offen).
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
struct Eintrag {
    /// SHA-256 des Inhalts (Hex); leer, solange nur erwartet.
    #[serde(default)]
    sha256: String,
    #[serde(default)]
    groesse: u64,
    #[serde(default)]
    mtime_ms: i64,
    updated: f64,
    device: String,
    #[serde(default)]
    edition: String,
    #[serde(default)]
    deleted: bool,
    /// Blob-Verweis des abgeglichenen Datensatzes; gilt für den Inhalt `blob_sha`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    blob: Option<Value>,
    #[serde(default)]
    blob_sha: String,
    /// Der Inhalt kommt noch von einem anderen Gerät.
    #[serde(default)]
    erwartet: bool,
    /// Inhalt der Datei, die der erwartete Inhalt ersetzen darf (sonst Konfliktkopie).
    #[serde(default)]
    ersetzt_sha: String,
    /// Felder neuerer Geräte im Datensatz, unverändert weitergegeben.
    #[serde(default, skip_serializing_if = "Map::is_empty")]
    extra: Map<String, Value>,
    /// In der App ausdrücklich gelöscht, während die Datei lokal fehlt: wird beim nächsten
    /// Sammeln zum Grabstein. Nur so entstehen Grabsteine für fehlende Dateien.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    entfernt: bool,
    /// „Vom Hub wiederherstellen“ für eine lokal fehlende Datei: das Sammeln liefert sie
    /// in [`Sammlung::wiederherstellen`], der Aufrufer bildet den Verweis.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    wiederholen: bool,
}

#[derive(Serialize, Deserialize, Default, PartialEq, Debug)]
struct Stand {
    #[serde(default)]
    v: u32,
    /// Relativer Pfad → Eintrag.
    #[serde(default)]
    dateien: BTreeMap<String, Eintrag>,
    /// Datensätze neuerer Version (ID → Rohwert): nur lesen.
    #[serde(default)]
    fremd: BTreeMap<String, Value>,
}

/// Stand eines Buchs. Ein unlesbarer Inhalt wird beiseitegelegt (`<STAND>.defekt-<ms>`)
/// und neu begonnen: das nächste Sammeln meldet die Dateien wieder, Konflikte werden zu
/// Kopien, nichts wird gelöscht. Ein Lesefehler dagegen bleibt ein Fehler.
fn stand_laden(buch_dir: &Path) -> Result<Stand, String> {
    let pfad = buch_dir.join(STAND);
    match fs::read(&pfad) {
        Ok(roh) => match serde_json::from_slice(&roh) {
            Ok(stand) => Ok(stand),
            Err(_) => {
                let ms = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_millis())
                    .unwrap_or_default();
                fs::rename(&pfad, buch_dir.join(format!("{STAND}.defekt-{ms}")))
                    .map_err(|e| format!("{STAND} unlesbar und nicht beiseitelegbar: {e}"))?;
                log::warn!("Sync: unlesbarer Datei-Stand eines Buchs beiseitegelegt");
                Ok(Stand::default())
            }
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Stand::default()),
        Err(e) => Err(format!("{STAND} nicht lesbar: {e}")),
    }
}

fn stand_sichern(buch_dir: &Path, stand: &mut Stand) -> Result<(), String> {
    stand.v = STAND_VERSION;
    let bytes = serde_json::to_vec_pretty(stand).map_err(|e| e.to_string())?;
    schreibe_atomar(&buch_dir.join(STAND), &bytes)
}

// ---------------------------------------------------------------------------
// Dateien
// ---------------------------------------------------------------------------

fn sha256_datei(pfad: &Path) -> Result<String, String> {
    let mut datei = fs::File::open(pfad).map_err(|e| format!("Datei nicht lesbar: {e}"))?;
    let mut h = Sha256::new();
    let mut puffer = vec![0u8; 1024 * 1024];
    loop {
        let n = datei
            .read(&mut puffer)
            .map_err(|e| format!("Datei nicht lesbar: {e}"))?;
        if n == 0 {
            break;
        }
        h.update(&puffer[..n]);
    }
    Ok(h.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

fn stat(meta: &fs::Metadata) -> (u64, i64) {
    let mtime = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as i64)
        .unwrap_or_default();
    (meta.len(), mtime)
}

/// Prüfsumme einer Datei, die sich währenddessen nicht verändert hat (Größe und
/// Änderungszeit vor und nach dem Lesen gleich), mit den Angaben danach.
fn sha256_stabil(pfad: &Path) -> Result<(String, fs::Metadata), String> {
    for _ in 0..3 {
        let vor = fs::metadata(pfad).map_err(|e| format!("Datei nicht lesbar: {e}"))?;
        let sha = sha256_datei(pfad)?;
        let nach = fs::metadata(pfad).map_err(|e| format!("Datei nicht lesbar: {e}"))?;
        if stat(&vor) == stat(&nach) {
            return Ok((sha, nach));
        }
    }
    Err("Die Datei ändert sich gerade; der nächste Lauf versucht es erneut.".into())
}

/// Vergleichsschlüssel eines relativen Pfads: jedes Glied nach NFC und ohne Groß- und
/// Kleinschreibung. Gleicher Schlüssel heißt: auf manchen Systemen derselbe Ort.
fn schluessel(rel: &str) -> String {
    rel.split('/')
        .map(vergleichsschluessel)
        .collect::<Vec<_>>()
        .join("/")
}

/// Bezeichnen `a` und `b` verschieden geschrieben denselben Ort (oder liegen in Ordnern,
/// die sich nur so unterscheiden)? Entscheidend ist das erste ungleiche Glied.
fn kollidieren(a: &str, b: &str) -> bool {
    a.split('/')
        .zip(b.split('/'))
        .find(|(x, y)| x != y)
        .is_some_and(|(x, y)| vergleichsschluessel(x) == vergleichsschluessel(y))
}

/// Liegt unter dem Buch-Ordner schon ein Glied von `rel`, das nur anders geschrieben ist
/// (Groß-/Kleinschreibung, Unicode-Form)?
fn kollision_auf_platte(buch_dir: &Path, rel: &str) -> Result<bool, String> {
    let mut dir = buch_dir.to_path_buf();
    for glied in rel.split('/') {
        let eintraege = match fs::read_dir(&dir) {
            Ok(e) => e,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(false),
            Err(e) => return Err(format!("Ordner nicht lesbar: {e}")),
        };
        let gesucht = vergleichsschluessel(glied);
        let mut genau = false;
        for eintrag in eintraege {
            let eintrag = eintrag.map_err(|e| format!("Ordner nicht lesbar: {e}"))?;
            let name = eintrag.file_name().to_string_lossy().into_owned();
            if name == glied {
                genau = true;
            } else if vergleichsschluessel(&name) == gesucht {
                return Ok(true);
            }
        }
        if !genau {
            return Ok(false);
        }
        dir.push(glied);
    }
    Ok(false)
}

/// `von` nach `nach` verschieben (gleicher Datenträger), ohne je etwas zu ersetzen: liegt
/// am Ziel schon etwas, kommt `AlreadyExists` und nichts ist geschehen.
fn verschieben_ohne_ersetzen(von: &Path, nach: &Path) -> std::io::Result<()> {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        #[link(name = "kernel32")]
        extern "system" {
            fn MoveFileExW(von: *const u16, nach: *const u16, flags: u32) -> i32;
        }
        // Lange Pfade: über die aufgelöste Form (`\\?\…`) gehen.
        let quelle = fs::canonicalize(von)?;
        let ziel = match (nach.parent(), nach.file_name()) {
            (Some(dir), Some(name)) => fs::canonicalize(dir)?.join(name),
            _ => return Err(std::io::ErrorKind::InvalidInput.into()),
        };
        let breit = |p: &Path| -> Vec<u16> { p.as_os_str().encode_wide().chain([0]).collect() };
        let (a, b) = (breit(&quelle), breit(&ziel));
        // SAFETY: nullterminierte Zeichenketten, nur gelesen. Flags 0: nie ersetzen, nur
        // auf demselben Datenträger.
        if unsafe { MoveFileExW(a.as_ptr(), b.as_ptr(), 0) } != 0 {
            return Ok(());
        }
        Err(std::io::Error::last_os_error())
    }
    #[cfg(unix)]
    {
        // Eine harte Verknüpfung ersetzt nie; danach den alten Namen entfernen.
        match fs::hard_link(von, nach) {
            Ok(()) => {
                // Bleibt der alte Name stehen, liegt der Inhalt doppelt: kein Verlust.
                let _ = fs::remove_file(von);
                return Ok(());
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => return Err(e),
            // Ordner oder Datenträger ohne harte Verknüpfungen: Umbenennen ohne Ersetzen.
            Err(_) => {}
        }
        umbenennen_ohne_ersetzen(von, nach)
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = (von, nach);
        Err(std::io::ErrorKind::Unsupported.into())
    }
}

#[cfg(unix)]
fn umbenennen_ohne_ersetzen(von: &Path, nach: &Path) -> std::io::Result<()> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;
    let c = |p: &Path| {
        CString::new(p.as_os_str().as_bytes())
            .map_err(|_| std::io::Error::from(std::io::ErrorKind::InvalidInput))
    };
    let (a, b) = (c(von)?, c(nach)?);
    if umbenennen_roh(&a, &b)? == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

#[cfg(any(target_os = "macos", target_os = "ios"))]
fn umbenennen_roh(a: &std::ffi::CStr, b: &std::ffi::CStr) -> std::io::Result<i32> {
    // SAFETY: nullterminierte Zeichenketten, nur gelesen.
    Ok(unsafe { libc::renamex_np(a.as_ptr(), b.as_ptr(), libc::RENAME_EXCL) })
}

#[cfg(any(all(target_os = "linux", target_env = "gnu"), target_os = "android"))]
fn umbenennen_roh(a: &std::ffi::CStr, b: &std::ffi::CStr) -> std::io::Result<i32> {
    // SAFETY: nullterminierte Zeichenketten, nur gelesen.
    Ok(unsafe {
        libc::renameat2(
            libc::AT_FDCWD,
            a.as_ptr(),
            libc::AT_FDCWD,
            b.as_ptr(),
            libc::RENAME_NOREPLACE as _,
        )
    })
}

#[cfg(all(
    unix,
    not(any(
        target_os = "macos",
        target_os = "ios",
        all(target_os = "linux", target_env = "gnu"),
        target_os = "android"
    ))
))]
fn umbenennen_roh(_a: &std::ffi::CStr, _b: &std::ffi::CStr) -> std::io::Result<i32> {
    Err(std::io::ErrorKind::Unsupported.into())
}

/// `von` unter einem freien Namen (nach 4.4) in `dir` ablegen, nie ersetzen; taucht der
/// gewählte Name gleichzeitig auf, wird der nächste versucht. Liefert den Namen.
fn ablegen_frei(von: &Path, dir: &Path, wunsch: &str) -> Result<String, String> {
    for _ in 0..50 {
        let name = freier_name(dir, wunsch)?;
        match verschieben_ohne_ersetzen(von, &dir.join(&name)) {
            Ok(()) => return Ok(name),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(format!("Datei nicht ablegbar: {e}")),
        }
    }
    Err("Kein freier Name gefunden".into())
}

/// Kein Glied von `rel` unter `wurzel` ist eine Verknüpfung; ein fehlendes Glied beendet
/// die Prüfung.
fn ohne_verknuepfung(wurzel: &Path, rel: &str) -> Result<(), String> {
    let mut pfad = wurzel.to_path_buf();
    for teil in rel.split('/') {
        pfad.push(teil);
        match fs::symlink_metadata(&pfad) {
            Ok(m) if m.file_type().is_symlink() => {
                return Err("Der Pfad führt über eine Verknüpfung; nichts angefasst.".into())
            }
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(e) => return Err(format!("Pfad nicht lesbar: {e}")),
        }
    }
    Ok(())
}

/// Was [`dateien_finden`] sah: die Dateien und die Teilbäume, die es nicht lesen konnte
/// oder durfte (Verknüpfung, Lesefehler, zu tief, fehlender Ordner einer Rolle, Name nicht
/// abgleichbar, Namenskollision). Was darunter im Stand steht, gilt weiter als vorhanden
/// und wird nie zum Grabstein. Verglichen wird nach [`schluessel`]: ein anders
/// geschriebener Eintrag desselben Orts ist ebenso verdeckt.
#[derive(Default)]
struct Fund {
    dateien: BTreeMap<String, fs::Metadata>,
    verdeckt: Vec<String>,
    /// Dateien, die da sind, aber nicht abgeglichen werden (Namenskollision).
    gehalten: BTreeMap<String, fs::Metadata>,
}

impl Fund {
    fn verdeckt(&self, rel: &str) -> bool {
        let k = schluessel(rel);
        self.verdeckt.iter().any(|v| {
            let v = schluessel(v);
            k == v
                || k.strip_prefix(v.as_str())
                    .is_some_and(|r| r.starts_with('/'))
        })
    }
}

/// Dateien unter den Ordnern der Rollen (relativ zum Buch-Ordner). Versteckte Einträge und
/// Verknüpfungen werden nie gelesen; Namen, die andere Systeme nicht ablegen könnten,
/// bleiben lokal (Hinweis).
fn dateien_finden(buch_dir: &Path, hinweise: &mut Vec<String>) -> Fund {
    fn gehen(dir: &Path, rel: &str, tiefe: usize, aus: &mut Fund, hinweise: &mut Vec<String>) {
        let Ok(eintraege) = fs::read_dir(dir) else {
            aus.verdeckt.push(rel.to_string());
            return;
        };
        for eintrag in eintraege {
            let Ok(eintrag) = eintrag else {
                aus.verdeckt.push(rel.to_string());
                continue;
            };
            let name = eintrag.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') {
                continue;
            }
            let kind = format!("{rel}/{name}");
            let Ok(meta) = fs::symlink_metadata(eintrag.path()) else {
                aus.verdeckt.push(kind);
                continue;
            };
            if meta.file_type().is_symlink() {
                aus.verdeckt.push(kind);
                continue;
            }
            if bereinigter_name(&name).ok().as_deref() != Some(name.as_str()) {
                // Auch eine andere Unicode-Form eines bekannten Namens: der Eintrag im
                // Stand bleibt stehen.
                hinweise.push(format!("{kind}: Name wird nicht abgeglichen"));
                aus.verdeckt.push(kind);
                continue;
            }
            if meta.is_dir() {
                if tiefe < TIEFE {
                    gehen(&eintrag.path(), &kind, tiefe + 1, aus, hinweise);
                } else {
                    aus.verdeckt.push(kind);
                }
            } else if meta.is_file() {
                aus.dateien.insert(kind, meta);
            }
        }
    }
    let mut aus = Fund::default();
    for rolle in Rolle::ALLE {
        let dir = buch_dir.join(rolle.ordner());
        match fs::symlink_metadata(&dir) {
            Ok(m) if m.is_dir() && !m.file_type().is_symlink() => {
                gehen(&dir, rolle.ordner(), 1, &mut aus, hinweise);
            }
            // Fehlt der Ordner oder ist er nicht lesbar, löscht das nichts.
            _ => aus.verdeckt.push(rolle.ordner().to_string()),
        }
    }
    // Namen, die sich nur in Groß-/Kleinschreibung oder Unicode-Form unterscheiden (auch
    // ihre Ordner), wären auf anderen Systemen derselbe Ort: keiner von ihnen wird
    // abgeglichen, nichts davon wird zum Grabstein.
    let mut klassen: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for rel in aus.dateien.keys() {
        let glieder: Vec<&str> = rel.split('/').collect();
        for i in 2..=glieder.len() {
            let teil = glieder[..i].join("/");
            klassen.entry(schluessel(&teil)).or_default().insert(teil);
        }
    }
    for gleich in klassen.into_values().filter(|g| g.len() > 1) {
        for teil in gleich {
            hinweise.push(format!(
                "{teil}: Name unterscheidet sich nur in Groß-/Kleinschreibung oder \
                 Unicode-Form von einem anderen und wird nicht abgeglichen"
            ));
            aus.verdeckt.push(teil);
        }
    }
    if !aus.verdeckt.is_empty() {
        let verdeckt: Vec<String> = aus
            .dateien
            .keys()
            .filter(|rel| aus.verdeckt(rel))
            .cloned()
            .collect();
        for rel in verdeckt {
            if let Some(meta) = aus.dateien.remove(&rel) {
                aus.gehalten.insert(rel, meta);
            }
        }
    }
    aus
}

/// Konfliktname neben `rel`: `Name (Konflikt <wer>).ext`, frei nach 4.4, und auch nicht
/// schon im Stand vergeben (eine noch nicht geladene Konfliktkopie belegt ihren Namen).
fn konflikt_rel(buch_dir: &Path, rel: &str, wer: &str, stand: &Stand) -> Result<String, String> {
    let (ordner, name) = rel.rsplit_once('/').ok_or("Pfad ohne Ordner")?;
    let (stamm, endung) = match name.rfind('.') {
        Some(pos) if pos > 0 => (&name[..pos], &name[pos..]),
        _ => (name, ""),
    };
    for n in 1..1000u32 {
        let zusatz = if n == 1 {
            String::new()
        } else {
            format!(" {n}")
        };
        let frei = freier_name(
            &buch_dir.join(ordner),
            &format!("{stamm} (Konflikt {wer}{zusatz}){endung}"),
        )?;
        let kandidat = format!("{ordner}/{frei}");
        if !stand.dateien.contains_key(&kandidat) {
            return Ok(kandidat);
        }
    }
    Err("Kein freier Name für eine Konfliktkopie".into())
}

/// Eine Datei in den Papierkorb der Bibliothek legen (`.papierkorb/dateien/<buch>/<rel>`),
/// nie überschreiben.
fn in_papierkorb(buch_dir: &Path, buch: &str, rel: &str) -> Result<(), String> {
    let wurzel = buch_dir.parent().ok_or("Buch-Ordner ohne Bibliothek")?;
    let (ordner, name) = rel.rsplit_once('/').ok_or("Pfad ohne Ordner")?;
    let ziel_dir = wurzel
        .join(PAPIERKORB)
        .join(PAPIERKORB_DATEIEN)
        .join(buch)
        .join(ordner);
    fs::create_dir_all(&ziel_dir).map_err(|e| format!("Papierkorb: {e}"))?;
    ablegen_frei(&buch_dir.join(rel), &ziel_dir, name)
        .map(|_| ())
        .map_err(|e| format!("Nicht in den Papierkorb gelegt: {e}"))
}

/// Die Datei `quelle` als Konfliktkopie neben `rel` ablegen (Name nach [`konflikt_rel`]),
/// nie ersetzen. Liefert den relativen Pfad der Kopie.
fn konflikt_ablegen(
    buch_dir: &Path,
    quelle: &Path,
    rel: &str,
    wer: &str,
    stand: &Stand,
) -> Result<String, String> {
    for _ in 0..50 {
        let kopie = konflikt_rel(buch_dir, rel, wer, stand)?;
        match verschieben_ohne_ersetzen(quelle, &buch_dir.join(&kopie)) {
            Ok(()) => return Ok(kopie),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(format!("Konfliktkopie nicht anlegbar: {e}")),
        }
    }
    Err("Kein freier Name für eine Konfliktkopie".into())
}

// ---------------------------------------------------------------------------
// Sammeln
// ---------------------------------------------------------------------------

/// Eine Datei, deren Blob das Gerät selbst bilden (und hochladen) muss.
#[derive(Clone, Debug, PartialEq)]
pub struct Quelle {
    pub pfad: PathBuf,
    pub sha256: String,
    pub groesse: u64,
    pub mtime_ms: i64,
}

/// Ein Datensatz dieses Geräts. `wert` trägt `blob`, wenn er bekannt ist; sonst bildet
/// ihn der Aufrufer aus `quelle`.
#[derive(Clone, Debug, PartialEq)]
pub struct Lokal {
    pub id: String,
    pub wert: Value,
    pub quelle: Option<Quelle>,
}

#[derive(Default, Debug)]
pub struct Sammlung {
    pub lokal: Vec<Lokal>,
    /// IDs, die es gibt, die in diesem Lauf aber nicht gesendet werden (neuere Version,
    /// Prüfsumme über dem Budget). Sie gelten nicht als gelöscht.
    pub vorhanden: Vec<String>,
    pub hinweise: Vec<String>,
    /// Lokal fehlende Dateien, die vom Hub zurückkommen sollen und deren Verweis der
    /// Aufrufer aus der Quelle bildet (dann [`erwarten`]).
    pub wiederherstellen: Vec<Lokal>,
}

fn datensatz(buch: &str, rel: &str, e: &Eintrag) -> Option<Value> {
    let rolle = Rolle::aus_pfad(rel)?;
    let mut m = e.extra.clone();
    m.insert("v".into(), json!(VERSION));
    m.insert("id".into(), json!(datei_id(buch, rolle, rel)));
    m.insert("book".into(), json!(buch));
    m.insert("edition".into(), json!(e.edition));
    m.insert("role".into(), json!(rolle.name()));
    m.insert("path".into(), json!(rel));
    m.insert("updated".into(), json!(e.updated));
    m.insert("device".into(), json!(e.device));
    m.insert("deleted".into(), json!(e.deleted));
    m.remove("blob");
    let blob_gilt = e.erwartet || (!e.sha256.is_empty() && e.blob_sha == e.sha256);
    if !e.deleted && blob_gilt {
        if let Some(b) = &e.blob {
            m.insert("blob".into(), b.clone());
        }
    }
    Some(Value::Object(m))
}

fn lokal_von(buch_dir: &Path, buch: &str, rel: &str, e: &Eintrag) -> Option<Lokal> {
    let rolle = Rolle::aus_pfad(rel)?;
    let wert = datensatz(buch, rel, e)?;
    let quelle = (!e.deleted && !e.erwartet && wert.get("blob").is_none()).then(|| Quelle {
        pfad: buch_dir.join(rel),
        sha256: e.sha256.clone(),
        groesse: e.groesse,
        mtime_ms: e.mtime_ms,
    });
    Some(Lokal {
        id: datei_id(buch, rolle, rel),
        wert,
        quelle,
    })
}

/// Der eigene Datensatz `id` aus dem gespeicherten Stand, ohne die Platte erneut zu lesen
/// (so, wie ihn das letzte Sammeln gesendet hätte). `None`: kein eigener Datensatz.
pub fn lokal_aus_stand(buch_dir: &Path, buch: &str, id: &str) -> Result<Option<Lokal>, String> {
    let stand = stand_laden(buch_dir)?;
    if stand.fremd.contains_key(id) {
        return Ok(None);
    }
    Ok(stand
        .dateien
        .iter()
        .filter(|(rel, _)| Rolle::aus_pfad(rel).is_some_and(|r| datei_id(buch, r, rel) == id))
        .find_map(|(rel, e)| lokal_von(buch_dir, buch, rel, e)))
}

/// Bytes, die ein Lauf für Prüfsummen neu lesen darf. Eine Datei über dem Rest bekommt
/// je Lauf einmal eine Ausnahme, und zwar die erste hinter dem Zeiger des letzten Laufs
/// (`<buch>/<pfad>`, Reihenfolge der Zeichenketten). So kommt jede große Datei reihum dran,
/// auch wenn eine davor sich ständig ändert oder dauerhaft scheitert.
#[derive(Debug, Default)]
pub struct Budget {
    rest: u64,
    /// Zeiger des letzten Laufs: Ausnahmen nur dahinter.
    ab: Option<String>,
    /// Die Ausnahme dieses Laufs (der nächste Zeiger).
    ausnahme: Option<String>,
}

/// Datei mit dem Zeiger (in der Bibliothek, neben dem Parkplatz).
pub const REIHE: &str = ".sync-dateien-reihe.json";

impl Budget {
    /// Der gespeicherte Zeiger der Bibliothek.
    pub fn zeiger_laden(bibliothek: &Path) -> Option<String> {
        fs::read(bibliothek.join(REIHE))
            .ok()
            .and_then(|roh| serde_json::from_slice::<Option<String>>(&roh).ok())
            .flatten()
    }

    /// Budget mit einem Zeiger (etwa dem im Speicher gehaltenen, wenn Sichern scheiterte).
    pub fn mit_zeiger(rest: u64, ab: Option<String>) -> Self {
        Budget {
            rest,
            ab,
            ausnahme: None,
        }
    }

    /// Der Zeiger für den nächsten Lauf.
    pub fn naechster(&self) -> Option<String> {
        self.ausnahme.clone()
    }

    /// Den Zeiger für den nächsten Lauf sichern: die Ausnahme dieses Laufs; gab es keine,
    /// beginnt die Reihe wieder vorn.
    pub fn sichern(&self, bibliothek: &Path) -> Result<(), String> {
        if self.ab == self.ausnahme {
            return Ok(());
        }
        let bytes = serde_json::to_vec(&self.ausnahme).map_err(|e| e.to_string())?;
        schreibe_atomar(&bibliothek.join(REIHE), &bytes)
    }

    pub fn rest(&self) -> u64 {
        self.rest
    }

    fn nehmen(&mut self, wo: &str, n: u64) -> bool {
        if n <= self.rest {
            self.rest -= n;
            return true;
        }
        let dahinter = self.ab.as_deref().is_none_or(|ab| wo > ab);
        if self.ausnahme.is_some() || !dahinter {
            return false;
        }
        self.ausnahme = Some(wo.to_string());
        self.rest = 0;
        true
    }
}

/// Den Stand eines Buchs fortschreiben und seine Datensätze liefern. Neue und geänderte
/// Dateien bekommen `updated` (Änderungszeit, mindestens eins mehr als zuvor) und dieses
/// Gerät; verschwundene werden Grabsteine, Grabsteine über 30 Tage verfallen. `budget`
/// begrenzt die Bytes, die für Prüfsummen neu gelesen werden. Der Aufrufer hält die
/// Bibliotheks-Sperre.
pub fn sammeln(
    buch_dir: &Path,
    buch: &str,
    geraet: &str,
    jetzt_ms: f64,
    budget: &mut Budget,
) -> Result<Sammlung, String> {
    let mut stand = stand_laden(buch_dir)?;
    let vorher = serde_json::to_value(&stand).map_err(|e| e.to_string())?;
    let mut aus = Sammlung::default();
    let fund = dateien_finden(buch_dir, &mut aus.hinweise);
    let gefunden = &fund.dateien;
    // Eine lokal fehlende Datei wird nie von selbst zum Grabstein (gelöscht im Explorer,
    // Ordner verschoben, Laufwerk weg …): sie bleibt „fehlt lokal“ und vorhanden, bis sie
    // wiederkommt oder in der App gelöscht wird. Grabsteine und erwartete Dateien hängen
    // nicht an der Platte.
    let mut zurueck: BTreeSet<String> = stand
        .dateien
        .iter()
        .filter(|(_, e)| !e.deleted && !e.erwartet && !e.entfernt)
        .filter(|(rel, _)| !gefunden.contains_key(*rel))
        .map(|(rel, _)| rel.clone())
        .collect();
    zurueck.extend(fund.gehalten.keys().cloned());
    // Ein Eintrag im Stand, dessen Ort auf der Platte anders geschrieben ist (Groß-/
    // Kleinschreibung, Unicode-Form, auch eines Ordners): weder Grabstein noch neue Datei.
    for (rel, e) in &stand.dateien {
        if e.deleted || e.entfernt || gefunden.contains_key(rel) {
            continue;
        }
        let andere: Vec<&String> = gefunden.keys().filter(|g| kollidieren(g, rel)).collect();
        if andere.is_empty() {
            continue;
        }
        zurueck.insert(rel.clone());
        for g in andere {
            if zurueck.insert(g.clone()) {
                aus.hinweise.push(format!(
                    "{g}: anders geschrieben als eine bekannte Datei (Groß-/Kleinschreibung \
                     oder Unicode-Form); wird nicht abgeglichen"
                ));
            }
        }
    }

    for (rel, meta) in gefunden {
        let Some(rolle) = Rolle::aus_pfad(rel) else {
            continue;
        };
        if zurueck.contains(rel) {
            continue;
        }
        let alt = stand.dateien.get(rel).cloned();
        if alt.as_ref().is_some_and(|e| e.erwartet) {
            continue; // die neue Fassung kommt noch; die alte bleibt bis dahin liegen
        }
        let (groesse, mtime_ms) = stat(meta);
        let bekannt = alt
            .as_ref()
            .filter(|e| !e.deleted && e.groesse == groesse && e.mtime_ms == mtime_ms)
            .filter(|e| !e.sha256.is_empty())
            .map(|e| e.sha256.clone());
        let (sha, groesse, mtime_ms) = match bekannt {
            Some(s) => (s, groesse, mtime_ms),
            None => {
                if !budget.nehmen(&format!("{buch}/{rel}"), groesse) {
                    zurueck.insert(rel.clone());
                    continue;
                }
                match sha256_stabil(&buch_dir.join(rel)) {
                    Ok((s, m)) => {
                        let (g, t) = stat(&m);
                        (s, g, t)
                    }
                    Err(e) => {
                        aus.hinweise.push(format!("{rel}: {e}"));
                        zurueck.insert(rel.clone());
                        continue;
                    }
                }
            }
        };
        let eintrag = match alt {
            Some(e) if !e.deleted && e.sha256 == sha => Eintrag {
                groesse,
                mtime_ms,
                entfernt: false,
                wiederholen: false,
                ..e
            },
            alt => {
                let zuvor = alt.as_ref().map(|e| e.updated).unwrap_or(f64::MIN);
                Eintrag {
                    sha256: sha,
                    groesse,
                    mtime_ms,
                    updated: (mtime_ms as f64).max(zuvor + 1.0),
                    device: geraet.to_string(),
                    edition: alt
                        .as_ref()
                        .map(|e| e.edition.clone())
                        .filter(|f| !f.is_empty())
                        .unwrap_or_else(|| fassung_aus_pfad(rolle, rel)),
                    ..Eintrag::default()
                }
            }
        };
        stand.dateien.insert(rel.clone(), eintrag);
    }

    let mut weg: Vec<String> = Vec::new();
    for (rel, e) in stand.dateien.iter_mut() {
        if gefunden.contains_key(rel) {
            continue;
        }
        if e.entfernt {
            *e = Eintrag {
                updated: jetzt_ms.max(e.updated + 1.0),
                device: geraet.to_string(),
                edition: e.edition.clone(),
                deleted: true,
                ..Eintrag::default()
            };
        } else if e.deleted && jetzt_ms - e.updated > GRABSTEIN_MS {
            weg.push(rel.clone());
        } else if e.wiederholen && !e.erwartet {
            if let Some(l) = lokal_von(buch_dir, buch, rel, e) {
                aus.wiederherstellen.push(l);
            }
        }
    }
    for rel in weg {
        stand.dateien.remove(&rel);
    }

    for rel in zurueck.iter().filter(|r| !stand.dateien.contains_key(*r)) {
        if let Some(rolle) = Rolle::aus_pfad(rel) {
            aus.vorhanden.push(datei_id(buch, rolle, rel));
        }
    }
    for (rel, e) in &stand.dateien {
        let Some(rolle) = Rolle::aus_pfad(rel) else {
            continue;
        };
        let id = datei_id(buch, rolle, rel);
        // Ein Datensatz neuerer Version überdeckt den eigenen: nur aufbewahren, nie mit
        // dem älteren Stand überschreiben.
        if zurueck.contains(rel) || stand.fremd.contains_key(&id) {
            aus.vorhanden.push(id);
            continue;
        }
        if let Some(l) = lokal_von(buch_dir, buch, rel, e) {
            aus.lokal.push(l);
        }
    }
    aus.vorhanden.extend(stand.fremd.keys().cloned());

    if serde_json::to_value(&stand).map_err(|e| e.to_string())? != vorher {
        stand_sichern(buch_dir, &mut stand)?;
    }
    Ok(aus)
}

// ---------------------------------------------------------------------------
// Anwenden
// ---------------------------------------------------------------------------

/// Fehler beim Anwenden: unbrauchbarer Datensatz (überspringen), Ablage gerade nicht
/// beschreibbar (nächster Lauf) oder Datensatz zurückhalten, weil sein Name hier mit einer
/// anderen Datei zusammenfiele (aufbewahren, bis die Kollision weg ist; nichts geht
/// verloren).
#[derive(Debug, PartialEq)]
pub enum AnwendFehler {
    Ungueltig(String),
    Wiederholen(String),
    Halten(String),
}

impl From<String> for AnwendFehler {
    fn from(s: String) -> Self {
        AnwendFehler::Wiederholen(s)
    }
}

/// Ein empfangener Datensatz, geprüft.
#[derive(Debug, Clone, PartialEq)]
pub struct Fremd {
    pub book: String,
    pub rel: String,
    pub edition: String,
    pub updated: f64,
    pub device: String,
    pub deleted: bool,
    pub blob: Option<Value>,
    extra: Map<String, Value>,
}

const BEKANNTE_FELDER: [&str; 10] = [
    "v", "id", "book", "edition", "role", "path", "blob", "updated", "device", "deleted",
];

/// Version eines Datensatzes (fehlt sie, ist er ungültig).
pub fn version(wert: &Value) -> Option<u64> {
    wert.get("v").and_then(Value::as_u64)
}

/// Einen Datensatz der bekannten Version prüfen: ID passt zu Buch, Rolle und Pfad; der
/// Pfad ist sicher; ein lebender Datensatz trägt einen Blob.
pub fn pruefen(id: &str, wert: &Value) -> Result<Fremd, String> {
    let m = wert.as_object().ok_or("Datensatz ist kein Objekt")?;
    let text = |k: &str| m.get(k).and_then(Value::as_str).map(str::to_string);
    if version(wert) != Some(VERSION) {
        return Err("Datensatz ohne gültige Version".into());
    }
    let book = text("book").ok_or("Datensatz ohne Buch")?;
    let rel = text("path").ok_or("Datensatz ohne Pfad")?;
    let rolle = sicherer_pfad(&rel)?;
    if text("role").as_deref() != Some(rolle.name()) {
        return Err("Rolle passt nicht zum Pfad".into());
    }
    if text("id").as_deref() != Some(id) || datei_id(&book, rolle, &rel) != id {
        return Err("ID passt nicht zu Buch und Pfad".into());
    }
    let updated = m
        .get("updated")
        .and_then(Value::as_f64)
        .filter(|u| u.is_finite())
        .ok_or("Datensatz ohne Zeit")?;
    let device = text("device")
        .filter(|d| !d.is_empty())
        .ok_or("Datensatz ohne Gerät")?;
    let deleted = m.get("deleted").and_then(Value::as_bool).unwrap_or(false);
    let blob = m.get("blob").filter(|b| !b.is_null()).cloned();
    let blob_gut = |b: &Value| {
        b.get("blob")
            .and_then(Value::as_str)
            .is_some_and(|s| !s.is_empty())
            && b.get("size").and_then(Value::as_u64).is_some()
    };
    if !deleted && !blob.as_ref().is_some_and(blob_gut) {
        return Err("Datensatz ohne Blob".into());
    }
    let extra = m
        .iter()
        .filter(|(k, _)| !BEKANNTE_FELDER.contains(&k.as_str()))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    Ok(Fremd {
        book,
        rel,
        edition: text("edition").unwrap_or_default(),
        updated,
        device,
        deleted,
        blob: if deleted { None } else { blob },
        extra,
    })
}

/// Verweisen zwei Blob-Angaben auf denselben Inhalt (Kennung und Größe)?
fn gleicher_blob(a: &Value, b: &Value) -> bool {
    let teil = |v: &Value| {
        (
            v.get("blob").and_then(Value::as_str).map(str::to_string),
            v.get("size").and_then(Value::as_u64),
        )
    };
    let (ka, ga) = teil(a);
    ka.is_some() && ga.is_some() && (ka, ga) == teil(b)
}

/// Gewinnt `a` gegen `b`? Neueres `updated`, bei Gleichstand die größere Geräte-ID.
fn gewinnt(a: (f64, &str), b: (f64, &str)) -> bool {
    a.0 > b.0 || (a.0 == b.0 && a.1 > b.1)
}

fn kurz(geraet: &str) -> String {
    format!("Gerät {}", geraet.chars().take(6).collect::<String>())
}

/// Einen empfangenen Datensatz anwenden. `lokal_offen`: der lokale Stand dieser Datei
/// wurde seit dem letzten Abgleich geändert (Ledger). Dann entscheidet [`gewinnt`]; die
/// unterlegene Fassung bleibt als Konfliktkopie. Neuere Versionen werden nur aufbewahrt.
/// Der Aufrufer hält die Bibliotheks-Sperre. Liefert, ob sich lokal etwas geändert hat.
pub fn anwenden(
    buch_dir: &Path,
    buch: &str,
    id: &str,
    wert: &Value,
    lokal_offen: bool,
    lokaler_blob: Option<&Value>,
    geraet_name: &str,
) -> Result<bool, AnwendFehler> {
    let mut stand = stand_laden(buch_dir)?;
    if version(wert).is_some_and(|v| v > VERSION) {
        if !gueltige_id(id) || wert.get("book").and_then(Value::as_str) != Some(buch) {
            return Err(AnwendFehler::Ungueltig(
                "Datensatz neuerer Version passt nicht".into(),
            ));
        }
        if stand.fremd.get(id) == Some(wert) {
            return Ok(false);
        }
        stand.fremd.insert(id.to_string(), wert.clone());
        stand_sichern(buch_dir, &mut stand)?;
        return Ok(true);
    }
    let r = pruefen(id, wert).map_err(AnwendFehler::Ungueltig)?;
    if r.book != buch {
        return Err(AnwendFehler::Ungueltig(
            "Datensatz gehört zu einem anderen Buch".into(),
        ));
    }
    ohne_verknuepfung(buch_dir, &r.rel)?;
    // Gleicher Ort, andere Schreibweise (Groß-/Kleinschreibung, Unicode-Form, auch eines
    // Ordners): auf vielen Systemen dieselbe Datei. Nicht anwenden, sondern aufbewahren.
    if !r.deleted {
        let im_stand = stand
            .dateien
            .iter()
            .any(|(rel, e)| !e.deleted && kollidieren(rel, &r.rel));
        if im_stand || kollision_auf_platte(buch_dir, &r.rel)? {
            return Err(AnwendFehler::Halten(
                "Name fällt mit einer vorhandenen Datei zusammen (Groß-/Kleinschreibung oder \
                 Unicode-Form)"
                    .into(),
            ));
        }
    }
    let fremd_weg = stand.fremd.remove(id).is_some();
    let alt = stand.dateien.get(&r.rel).cloned();
    if let Some(e) = &alt {
        if datensatz(buch, &r.rel, e).as_ref() == Some(wert) {
            if fremd_weg {
                stand_sichern(buch_dir, &mut stand)?;
            }
            return Ok(false);
        }
    }
    let mut ersetzt_sha = alt
        .as_ref()
        .filter(|e| !e.deleted && !e.erwartet)
        .map(|e| e.sha256.clone())
        .unwrap_or_default();

    // Derselbe Inhalt wie die lokale Datei (Blob-Kennung aus Schlüssel und Prüfsumme):
    // kein Konflikt und kein Download, nur die fremden Angaben übernehmen.
    if let (Some(e), Some(fremd), Some(eigen)) = (&alt, &r.blob, lokaler_blob) {
        if !r.deleted && !ersetzt_sha.is_empty() && gleicher_blob(fremd, eigen) {
            let neu = Eintrag {
                updated: r.updated,
                device: r.device.clone(),
                edition: r.edition.clone(),
                blob: Some(fremd.clone()),
                blob_sha: e.sha256.clone(),
                extra: r.extra.clone(),
                ..e.clone()
            };
            stand.dateien.insert(r.rel.clone(), neu);
            stand_sichern(buch_dir, &mut stand)?;
            return Ok(false);
        }
    }

    if let (true, Some(e)) = (lokal_offen, &alt) {
        if gewinnt((e.updated, &e.device), (r.updated, &r.device)) {
            // Lokal gewinnt: der lokale Stand bleibt und geht hinaus; der fremde Inhalt
            // kommt als Konfliktkopie daneben.
            if let (false, Some(blob)) = (r.deleted, &r.blob) {
                let kopie = konflikt_rel(buch_dir, &r.rel, &kurz(&r.device), &stand)?;
                stand.dateien.insert(
                    kopie,
                    Eintrag {
                        updated: r.updated,
                        device: r.device.clone(),
                        edition: r.edition.clone(),
                        blob: Some(blob.clone()),
                        erwartet: true,
                        ..Eintrag::default()
                    },
                );
            }
            stand_sichern(buch_dir, &mut stand)?;
            return Ok(true);
        }
        // Fremd gewinnt: die lokale Fassung beiseitelegen, sie wird zur eigenen Datei.
        let pfad = buch_dir.join(&r.rel);
        if !e.deleted && !e.erwartet && pfad.is_file() {
            konflikt_ablegen(buch_dir, &pfad, &r.rel, geraet_name, &stand)?;
            ersetzt_sha.clear();
        }
    }

    let pfad = buch_dir.join(&r.rel);
    let neu = if r.deleted {
        if !ersetzt_sha.is_empty() && pfad.is_file() {
            // Nur einen unveränderten Inhalt wegräumen; ein frisch geänderter bleibt und
            // wird beim nächsten Sammeln als eigene Änderung erkannt.
            if sha256_datei(&pfad)? == ersetzt_sha {
                in_papierkorb(buch_dir, buch, &r.rel)?;
            }
        }
        Eintrag {
            updated: r.updated,
            device: r.device.clone(),
            edition: r.edition.clone(),
            deleted: true,
            extra: r.extra.clone(),
            ..Eintrag::default()
        }
    } else {
        let gleicher_inhalt = alt.as_ref().filter(|e| {
            !e.deleted && !e.erwartet && !ersetzt_sha.is_empty() && e.blob_sha == e.sha256
        });
        match gleicher_inhalt {
            Some(e) if e.blob == r.blob => Eintrag {
                updated: r.updated,
                device: r.device.clone(),
                edition: r.edition.clone(),
                extra: r.extra.clone(),
                ..e.clone()
            },
            _ => Eintrag {
                updated: r.updated,
                device: r.device.clone(),
                edition: r.edition.clone(),
                blob: r.blob.clone(),
                erwartet: true,
                ersetzt_sha,
                extra: r.extra.clone(),
                ..Eintrag::default()
            },
        }
    };
    stand.dateien.insert(r.rel.clone(), neu);
    stand_sichern(buch_dir, &mut stand)?;
    Ok(true)
}

/// Der Hub hat den Datensatz gelöscht (Grabstein verfallen, Buch weg): den Eintrag
/// vergessen; eine unveränderte Datei geht in den Papierkorb.
pub fn vergessen(buch_dir: &Path, buch: &str, id: &str) -> Result<bool, String> {
    let mut stand = stand_laden(buch_dir)?;
    let mut geaendert = stand.fremd.remove(id).is_some();
    let treffer: Vec<String> = stand
        .dateien
        .keys()
        .filter(|rel| Rolle::aus_pfad(rel).is_some_and(|r| datei_id(buch, r, rel) == id))
        .cloned()
        .collect();
    for rel in treffer {
        let Some(e) = stand.dateien.remove(&rel) else {
            continue;
        };
        geaendert = true;
        let pfad = buch_dir.join(&rel);
        if !e.deleted && !e.erwartet && pfad.is_file() && sha256_datei(&pfad)? == e.sha256 {
            in_papierkorb(buch_dir, buch, &rel)?;
        }
    }
    if geaendert {
        stand_sichern(buch_dir, &mut stand)?;
    }
    Ok(geaendert)
}

// ---------------------------------------------------------------------------
// Herunterladen
// ---------------------------------------------------------------------------

/// Eine Datei, deren Inhalt ein anderes Gerät gemeldet hat und die hier fehlt.
#[derive(Debug, Clone, PartialEq)]
pub struct Erwartet {
    pub rel: String,
    pub blob: Value,
}

pub fn erwartet(buch_dir: &Path) -> Result<Vec<Erwartet>, String> {
    Ok(stand_laden(buch_dir)?
        .dateien
        .into_iter()
        .filter(|(_, e)| e.erwartet)
        .filter_map(|(rel, e)| e.blob.map(|blob| Erwartet { rel, blob }))
        .collect())
}

/// Wohin ein Download zuerst geschrieben wird.
pub fn zwischenpfad(buch_dir: &Path, blob_id: &str) -> PathBuf {
    buch_dir
        .join(ZWISCHENABLAGE)
        .join(format!("{blob_id}.part"))
}

/// Eine geladene Datei ablegen. Ist der Eintrag inzwischen überholt, wird sie verworfen
/// (`Ok(None)`). Eine Datei am Ziel, die nicht der erwartete Vorgänger ist (seither lokal
/// geändert oder neu), bleibt als Konfliktkopie; der Vorgänger geht in den Papierkorb.
/// Der Aufrufer hält die Bibliotheks-Sperre.
pub fn ablegen(
    buch_dir: &Path,
    buch: &str,
    rel: &str,
    blob: &Value,
    geladen: &Path,
    geraet_name: &str,
) -> Result<Option<PathBuf>, String> {
    let mut stand = stand_laden(buch_dir)?;
    let Some(e) = stand.dateien.get(rel).cloned() else {
        let _ = fs::remove_file(geladen);
        return Ok(None);
    };
    if !e.erwartet || e.blob.as_ref() != Some(blob) {
        let _ = fs::remove_file(geladen);
        return Ok(None);
    }
    sicherer_pfad(rel)?;
    ohne_verknuepfung(buch_dir, rel)?;
    let ziel = buch_dir.join(rel);
    let sha = sha256_datei(geladen)?;
    let mut fertig = |meta: &fs::Metadata| -> Result<Option<PathBuf>, String> {
        let (groesse, mtime_ms) = stat(meta);
        stand.dateien.insert(
            rel.to_string(),
            Eintrag {
                sha256: sha.clone(),
                groesse,
                mtime_ms,
                blob_sha: sha.clone(),
                erwartet: false,
                ersetzt_sha: String::new(),
                ..e.clone()
            },
        );
        stand_sichern(buch_dir, &mut stand)?;
        Ok(Some(ziel.clone()))
    };
    // Taucht am Ziel zwischen Prüfen und Ablegen etwas auf, wird es nie ersetzt: dann
    // beginnt die Prüfung von vorn.
    for _ in 0..5 {
        match fs::symlink_metadata(&ziel) {
            Ok(m) if m.is_file() => {
                let (vorhanden, meta) = sha256_stabil(&ziel)?;
                if vorhanden == sha {
                    // Liegt schon da: nichts ersetzen, keine Kopie.
                    let _ = fs::remove_file(geladen);
                    return fertig(&meta);
                }
                if !e.ersetzt_sha.is_empty() && vorhanden == e.ersetzt_sha {
                    in_papierkorb(buch_dir, buch, rel)?;
                } else {
                    let stand_jetzt = stand_laden(buch_dir)?;
                    konflikt_ablegen(buch_dir, &ziel, rel, geraet_name, &stand_jetzt)?;
                }
            }
            Ok(_) => return Err("Am Ziel liegt keine Datei; nichts angefasst.".into()),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
            Err(err) => return Err(format!("Ziel nicht lesbar: {err}")),
        }
        if let Some(dir) = ziel.parent() {
            fs::create_dir_all(dir).map_err(|err| format!("Ordner nicht anlegbar: {err}"))?;
        }
        // Größe und Änderungszeit bleiben beim Verschieben erhalten.
        let meta = fs::metadata(geladen).map_err(|err| format!("Datei: {err}"))?;
        match verschieben_ohne_ersetzen(geladen, &ziel) {
            Ok(()) => return fertig(&meta),
            Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(err) => return Err(format!("Datei nicht ablegbar: {err}")),
        }
    }
    Err("Am Ziel ändert sich gerade etwas; der nächste Lauf versucht es erneut.".into())
}

/// Bevor ein Buch-Ordner gelöscht oder in ein anderes Buch zusammengeführt wird: die
/// Ordner der Notizen, Anhänge und Transkripte in den Papierkorb der Bibliothek legen
/// (`.papierkorb/dateien/<buch>/<ordner>`), nie überschreiben. Eine Verknüpfung bleibt
/// stehen (ihr Ziel liegt woanders und wird nie mitgelöscht). Der Aufrufer hält die
/// Bibliotheks-Sperre.
pub fn ordner_retten(buch_dir: &Path, buch: &str) -> Result<(), String> {
    let wurzel = buch_dir.parent().ok_or("Buch-Ordner ohne Bibliothek")?;
    for rolle in Rolle::ALLE {
        let dir = buch_dir.join(rolle.ordner());
        let echt =
            fs::symlink_metadata(&dir).is_ok_and(|m| m.is_dir() && !m.file_type().is_symlink());
        if !echt {
            continue;
        }
        let leer = fs::read_dir(&dir).is_ok_and(|mut e| e.next().is_none());
        if leer {
            continue;
        }
        let ziel_dir = wurzel.join(PAPIERKORB).join(PAPIERKORB_DATEIEN).join(buch);
        fs::create_dir_all(&ziel_dir).map_err(|e| format!("Papierkorb: {e}"))?;
        ablegen_frei(&dir, &ziel_dir, rolle.ordner())
            .map_err(|e| format!("Notizen und Anhänge nicht in den Papierkorb gelegt: {e}"))?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Parkplatz: Datensätze, deren Buch (noch) fehlt
// ---------------------------------------------------------------------------

/// Einen Datensatz aufbewahren, bis sein Buch ankommt.
pub fn parken(bibliothek: &Path, id: &str, wert: &Value) -> Result<(), String> {
    if !gueltige_id(id) {
        return Err(format!("Ungültige Datensatz-ID {id}"));
    }
    let bytes = serde_json::to_vec(wert).map_err(|e| e.to_string())?;
    schreibe_atomar(
        &bibliothek.join(PARKPLATZ).join(format!("{id}.json")),
        &bytes,
    )
}

/// Geparkte Datensätze (ID, Wert); unlesbare werden übergangen.
pub fn geparkt(bibliothek: &Path) -> Vec<(String, Value)> {
    let Ok(eintraege) = fs::read_dir(bibliothek.join(PARKPLATZ)) else {
        return Vec::new();
    };
    let mut aus: Vec<(String, Value)> = eintraege
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            let id = name.strip_suffix(".json")?.to_string();
            let wert = serde_json::from_slice(&fs::read(e.path()).ok()?).ok()?;
            gueltige_id(&id).then_some((id, wert))
        })
        .collect();
    aus.sort_by(|a, b| a.0.cmp(&b.0));
    aus
}

pub fn ausparken(bibliothek: &Path, id: &str) -> Result<(), String> {
    match fs::remove_file(bibliothek.join(PARKPLATZ).join(format!("{id}.json"))) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(format!("Geparkter Datensatz nicht entfernbar: {e}")),
    }
}

// ---------------------------------------------------------------------------
// Oberfläche: auflisten, Notiz anlegen, Anhänge hinzufügen, entfernen
// ---------------------------------------------------------------------------

/// Eine Datei im Buch-Ordner für die Inhaltsliste.
#[derive(Debug, Clone, PartialEq)]
pub struct Datei {
    pub rel: String,
    pub rolle: Rolle,
    /// Dateiname (letztes Glied).
    pub name: String,
    pub groesse: u64,
    pub geaendert_ms: i64,
    /// Fassung, zu der die Datei gehört (Transkript), sonst leer.
    pub fassung: String,
    /// Von einem anderen Gerät gemeldet, noch nicht geladen.
    pub erwartet: bool,
    /// Konfliktkopie (`… (Konflikt …)`).
    pub konflikt: bool,
    /// Bekannt, fehlt aber lokal (im Explorer gelöscht, Ordner fehlt): vom Hub
    /// wiederherstellen oder in der App löschen.
    pub fehlt: bool,
    /// Von einem anderen Gerät gemeldet, hier zurückgehalten: der Name fiele mit einer
    /// anderen Datei zusammen (Groß-/Kleinschreibung, Unicode-Form).
    pub zurueckgehalten: bool,
}

fn ist_konflikt(name: &str) -> bool {
    let stamm = name.rsplit_once('.').map_or(name, |(s, _)| s);
    stamm.ends_with(')') && stamm.contains(" (Konflikt ")
}

/// Notizen, Anhänge und Transkripte eines Buchs: was auf der Platte liegt, was noch kommt,
/// was zurückgehalten wird (Parkplatz) und was fehlt, weil sein Ordner fehlt. Geordnet
/// nach Rolle (Notiz, Anhang, Transkript), dann Pfad.
pub fn auflisten(buch_dir: &Path, buch: &str) -> Result<Vec<Datei>, String> {
    let stand = stand_laden(buch_dir)?;
    let mut hinweise = Vec::new();
    let fund = dateien_finden(buch_dir, &mut hinweise);
    let mut gefunden = fund.dateien;
    gefunden.extend(fund.gehalten);
    let mut aus: Vec<Datei> = Vec::new();
    let name_von = |rel: &str| rel.rsplit('/').next().unwrap_or(rel).to_string();
    let fassung_von = |rolle: Rolle, rel: &str, fassung: &str| {
        if fassung.is_empty() {
            fassung_aus_pfad(rolle, rel)
        } else {
            fassung.to_string()
        }
    };
    let neu = |rel: &str, rolle: Rolle, groesse: u64, geaendert_ms: i64, fassung: String| {
        let name = name_von(rel);
        Datei {
            rel: rel.to_string(),
            rolle,
            konflikt: ist_konflikt(&name),
            name,
            groesse,
            geaendert_ms,
            fassung,
            erwartet: false,
            fehlt: false,
            zurueckgehalten: false,
        }
    };
    for (rel, meta) in &gefunden {
        let Some(rolle) = Rolle::aus_pfad(rel) else {
            continue;
        };
        let (groesse, geaendert_ms) = stat(meta);
        let e = stand.dateien.get(rel);
        let fassung = fassung_von(rolle, rel, e.map_or("", |e| e.edition.as_str()));
        aus.push(Datei {
            erwartet: e.is_some_and(|e| e.erwartet),
            ..neu(rel, rolle, groesse, geaendert_ms, fassung)
        });
    }
    for (rel, e) in &stand.dateien {
        if e.deleted || e.entfernt || gefunden.contains_key(rel) {
            continue;
        }
        let Some(rolle) = Rolle::aus_pfad(rel) else {
            continue;
        };
        // Erwartet (auch zum Wiederherstellen angefordert) oder lokal fehlend.
        let kommt = e.erwartet || e.wiederholen;
        let groesse = if e.erwartet {
            e.blob
                .as_ref()
                .and_then(|b| b.get("size"))
                .and_then(Value::as_u64)
                .unwrap_or(0)
        } else {
            e.groesse
        };
        let fassung = fassung_von(rolle, rel, &e.edition);
        aus.push(Datei {
            erwartet: kommt,
            fehlt: !kommt,
            ..neu(rel, rolle, groesse, e.updated as i64, fassung)
        });
    }
    if let Some(bibliothek) = buch_dir.parent() {
        for (id, wert) in geparkt(bibliothek) {
            let Ok(r) = pruefen(&id, &wert) else {
                continue;
            };
            if r.book != buch || r.deleted || aus.iter().any(|d| d.rel == r.rel) {
                continue;
            }
            let Some(rolle) = Rolle::aus_pfad(&r.rel) else {
                continue;
            };
            let groesse = r
                .blob
                .as_ref()
                .and_then(|b| b.get("size"))
                .and_then(Value::as_u64)
                .unwrap_or(0);
            let fassung = fassung_von(rolle, &r.rel, &r.edition);
            aus.push(Datei {
                zurueckgehalten: true,
                ..neu(&r.rel, rolle, groesse, r.updated as i64, fassung)
            });
        }
    }
    let rang = |r: Rolle| match r {
        Rolle::Notiz => 0,
        Rolle::Anhang => 1,
        Rolle::Transkript => 2,
    };
    aus.sort_by(|a, b| (rang(a.rolle), &a.rel).cmp(&(rang(b.rolle), &b.rel)));
    Ok(aus)
}

/// Neue, leere Notiz `notizen/<Titel>.md` (Name nach 4.4, nie überschreiben).
pub fn notiz_anlegen(buch_dir: &Path, titel: &str) -> Result<String, String> {
    let titel = titel.trim();
    if titel.is_empty() {
        return Err("Bitte einen Titel für die Notiz angeben.".into());
    }
    ohne_verknuepfung(buch_dir, Rolle::Notiz.ordner())?;
    let dir = buch_dir.join(Rolle::Notiz.ordner());
    fs::create_dir_all(&dir).map_err(|e| format!("Notizen-Ordner nicht anlegbar: {e}"))?;
    let name = freier_name(&dir, &format!("{titel}.md"))?;
    let rel = format!("{}/{name}", Rolle::Notiz.ordner());
    let kopf = format!("# {}\n\n", titel.replace(['\r', '\n'], " "));
    let mut datei = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(buch_dir.join(&rel))
        .map_err(|e| format!("Notiz nicht anlegbar: {e}"))?;
    std::io::Write::write_all(&mut datei, kopf.as_bytes())
        .and_then(|()| datei.sync_all())
        .map_err(|e| format!("Notiz nicht schreibbar: {e}"))?;
    Ok(rel)
}

/// Dateien als Anhänge in `anhaenge/` kopieren (Namen nach 4.4, bei Kollision
/// `Name (2).ext`, nie überschreiben). Verknüpfungen und Ordner werden abgewiesen, bevor
/// etwas kopiert wird; scheitert eine Kopie, bleibt keine halbe Datei zurück.
pub fn anhaenge_hinzufuegen(buch_dir: &Path, quellen: &[PathBuf]) -> Result<Vec<String>, String> {
    for q in quellen {
        let meta =
            fs::symlink_metadata(q).map_err(|e| format!("{} nicht lesbar: {e}", q.display()))?;
        if meta.file_type().is_symlink() || !meta.is_file() {
            return Err(format!("{} ist keine gewöhnliche Datei.", q.display()));
        }
    }
    ohne_verknuepfung(buch_dir, Rolle::Anhang.ordner())?;
    ohne_verknuepfung(buch_dir, ZWISCHENABLAGE)?;
    let dir = buch_dir.join(Rolle::Anhang.ordner());
    fs::create_dir_all(&dir).map_err(|e| format!("Anhänge-Ordner nicht anlegbar: {e}"))?;
    let zwischen = buch_dir.join(ZWISCHENABLAGE);
    fs::create_dir_all(&zwischen).map_err(|e| format!("Zwischenablage nicht anlegbar: {e}"))?;
    let mut aus = Vec::new();
    for q in quellen {
        let wunsch = q
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .ok_or_else(|| format!("{} hat keinen Dateinamen.", q.display()))?;
        // Erst vollständig in eine eigene Zwischendatei kopieren, dann ohne Ersetzen unter
        // einem freien Namen ablegen. Aufgeräumt wird nur diese eigene Zwischendatei.
        let (tmp, mut nach) = eigene_zwischendatei(&zwischen)?;
        let kopiert = (|| -> std::io::Result<()> {
            let mut von = fs::File::open(q)?;
            std::io::copy(&mut von, &mut nach)?;
            nach.sync_all()
        })();
        drop(nach);
        if let Err(e) = kopiert {
            let _ = fs::remove_file(&tmp);
            return Err(format!("{} nicht kopiert: {e}", q.display()));
        }
        match ablegen_frei(&tmp, &dir, &wunsch) {
            Ok(name) => aus.push(format!("{}/{name}", Rolle::Anhang.ordner())),
            Err(e) => {
                let _ = fs::remove_file(&tmp);
                return Err(e);
            }
        }
    }
    Ok(aus)
}

/// Eine neue, nur diesem Aufruf gehörende Datei in `dir` (`create_new`).
fn eigene_zwischendatei(dir: &Path) -> Result<(PathBuf, fs::File), String> {
    let basis = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or_default();
    for n in 0..100u32 {
        let pfad = dir.join(format!("anhang-{}-{basis}-{n}.part", std::process::id()));
        match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&pfad)
        {
            Ok(f) => return Ok((pfad, f)),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(format!("Zwischendatei nicht anlegbar: {e}")),
        }
    }
    Err("Keine freie Zwischendatei".into())
}

/// Pfad einer Datei im Buch-Ordner zum Öffnen: sicherer Pfad, keine Verknüpfung,
/// vorhanden.
pub fn pfad(buch_dir: &Path, rel: &str) -> Result<PathBuf, String> {
    sicherer_pfad(rel)?;
    ohne_verknuepfung(buch_dir, rel)?;
    let p = buch_dir.join(rel);
    if !p.is_file() {
        return Err("Die Datei ist (noch) nicht da.".into());
    }
    Ok(p)
}

/// Eine Datei aus dem Buch-Ordner in den Papierkorb der Bibliothek legen. Der nächste
/// Abgleich macht daraus einen Grabstein.
/// Fehlt die Datei lokal (im Explorer gelöscht, Ordner fehlt), ist das Entfernen in der App
/// die ausdrückliche Löschung: der Eintrag wird beim nächsten Sammeln zum Grabstein.
pub fn entfernen(buch_dir: &Path, buch: &str, rel: &str) -> Result<(), String> {
    sicherer_pfad(rel)?;
    ohne_verknuepfung(buch_dir, rel)?;
    if buch_dir.join(rel).is_file() {
        return in_papierkorb(buch_dir, buch, rel);
    }
    let mut stand = stand_laden(buch_dir)?;
    match stand.dateien.get_mut(rel) {
        Some(e) if !e.deleted && !e.entfernt => {
            e.entfernt = true;
            e.wiederholen = false;
            stand_sichern(buch_dir, &mut stand)
        }
        _ => Err("Die Datei ist (noch) nicht da.".into()),
    }
}

/// Eine lokal fehlende Datei vom Hub zurückholen. Ist ihr Verweis schon bekannt, wird sie
/// sofort erwartet; sonst bildet ihn das nächste Sammeln ([`Sammlung::wiederherstellen`]).
pub fn wiederherstellen(buch_dir: &Path, rel: &str) -> Result<(), String> {
    sicherer_pfad(rel)?;
    ohne_verknuepfung(buch_dir, rel)?;
    if buch_dir.join(rel).exists() {
        return Err("Die Datei ist schon da.".into());
    }
    let mut stand = stand_laden(buch_dir)?;
    let Some(e) = stand.dateien.get_mut(rel) else {
        return Err("Die Datei ist nicht bekannt.".into());
    };
    if e.deleted || e.erwartet || e.entfernt {
        return Err("Die Datei lässt sich nicht wiederherstellen.".into());
    }
    if e.blob.is_some() && !e.sha256.is_empty() && e.blob_sha == e.sha256 {
        e.erwartet = true;
        e.ersetzt_sha.clear();
    } else {
        e.wiederholen = true;
    }
    stand_sichern(buch_dir, &mut stand)
}

/// Der Aufrufer hat den Verweis einer wiederherzustellenden Datei gebildet: ab jetzt wird
/// ihr Inhalt erwartet (geladen und abgelegt wie jeder Download).
pub fn erwarten(buch_dir: &Path, rel: &str, blob: &Value) -> Result<(), String> {
    let mut stand = stand_laden(buch_dir)?;
    let Some(e) = stand.dateien.get_mut(rel) else {
        return Ok(());
    };
    if !e.wiederholen || e.deleted || e.entfernt || buch_dir.join(rel).exists() {
        return Ok(());
    }
    e.wiederholen = false;
    e.erwartet = true;
    e.ersetzt_sha.clear();
    e.blob = Some(blob.clone());
    stand_sichern(buch_dir, &mut stand)
}

#[cfg(test)]
mod tests {
    use super::*;

    const ICH: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const DU: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
    const TAG: f64 = 24.0 * 60.0 * 60.0 * 1000.0;

    fn buch(name: &str) -> PathBuf {
        let wurzel = std::env::temp_dir().join(format!("buch-dateien-{name}"));
        let _ = fs::remove_dir_all(&wurzel);
        let dir = wurzel.join("library").join("book_1");
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn schreiben(dir: &Path, rel: &str, inhalt: &[u8]) {
        let p = dir.join(rel);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, inhalt).unwrap();
    }

    fn alles(dir: &Path, jetzt: f64) -> Sammlung {
        sammeln(
            dir,
            "book_1",
            ICH,
            jetzt,
            &mut Budget::mit_zeiger(u64::MAX, None),
        )
        .unwrap()
    }

    fn nach_pfad<'a>(s: &'a Sammlung, rel: &str) -> &'a Lokal {
        s.lokal
            .iter()
            .find(|l| l.wert["path"] == rel)
            .unwrap_or_else(|| panic!("{rel} fehlt"))
    }

    fn blob(n: u8) -> Value {
        json!({ "blob": format!("{n:02x}").repeat(32), "size": 5, "chunks": 1 })
    }

    fn fremder(rel: &str, updated: f64, inhalt: Option<Value>) -> (String, Value) {
        let rolle = Rolle::aus_pfad(rel).unwrap();
        let id = datei_id("book_1", rolle, rel);
        let mut v = json!({
            "v": 1, "id": id, "book": "book_1", "edition": "", "role": rolle.name(),
            "path": rel, "updated": updated, "device": DU, "deleted": inhalt.is_none(),
        });
        if let Some(b) = inhalt {
            v["blob"] = b;
        }
        (id, v)
    }

    /// Lädt jeden erwarteten Inhalt „herunter“ (hier: schreibt `inhalt`) und legt ihn ab.
    fn laden(dir: &Path, inhalt: &[u8]) -> Vec<PathBuf> {
        erwartet(dir)
            .unwrap()
            .into_iter()
            .filter_map(|e| {
                let tmp = zwischenpfad(dir, "x");
                fs::create_dir_all(tmp.parent().unwrap()).unwrap();
                fs::write(&tmp, inhalt).unwrap();
                ablegen(dir, "book_1", &e.rel, &e.blob, &tmp, "Mein PC").unwrap()
            })
            .collect()
    }

    #[test]
    fn id_ist_stabil_und_entspricht_der_spec() {
        assert_eq!(
            datei_id("book_1", Rolle::Anhang, "anhaenge/Cashflow-Tabelle.xlsx"),
            "bf_ce4f353fbfa464d98f752e46a581809c114b23c2"
        );
        assert!(gueltige_id("bf_ce4f353fbfa464d98f752e46a581809c114b23c2"));
        assert!(!gueltige_id("bf_../x"));
    }

    #[test]
    fn pfade_werden_streng_geprueft() {
        assert_eq!(sicherer_pfad("notizen/a.md"), Ok(Rolle::Notiz));
        assert_eq!(sicherer_pfad("anhaenge/Ordner/b.pdf"), Ok(Rolle::Anhang));
        for schlecht in [
            "notizen",
            "notizen/",
            "/notizen/a.md",
            "notizen/../buch.json",
            "notizen/.versteckt",
            "notizen/a:b.md",
            "notizen/CON.md",
            "fassungen/x.md",
            "notizen\\a.md",
            "notizen/a/b/c/d/e.md",
        ] {
            assert!(sicherer_pfad(schlecht).is_err(), "{schlecht}");
        }
    }

    #[test]
    fn sammeln_findet_dateien_und_merkt_aenderungen() {
        let dir = buch("sammeln");
        schreiben(&dir, "anhaenge/Tabelle.xlsx", b"eins");
        schreiben(&dir, "notizen/Gedanken.md", b"# Notiz");
        schreiben(&dir, "transkript/page_7.md", b"Text");
        schreiben(&dir, "notizen/.versteckt", b"x");
        schreiben(&dir, "fassungen/page_7/state.json", b"{}");
        let s = alles(&dir, 1000.0);
        assert_eq!(s.lokal.len(), 3, "{:?}", s.lokal);
        let t = nach_pfad(&s, "transkript/page_7.md");
        assert_eq!(t.wert["edition"], "page_7");
        assert_eq!(t.wert["role"], "transkript");
        assert_eq!(t.wert["device"], ICH);
        assert!(t.wert.get("blob").is_none() && t.quelle.is_some());
        assert_eq!(
            t.id,
            datei_id("book_1", Rolle::Transkript, "transkript/page_7.md")
        );
        // Unverändert: gleiche Datensätze, der Stand wird nicht neu geschrieben.
        let zeit = fs::metadata(dir.join(STAND)).unwrap().modified().unwrap();
        let s2 = alles(&dir, 2000.0);
        assert_eq!(s.lokal, s2.lokal);
        assert_eq!(
            fs::metadata(dir.join(STAND)).unwrap().modified().unwrap(),
            zeit
        );
        // Geändert: neues `updated`, größer als zuvor.
        let vorher = nach_pfad(&s, "anhaenge/Tabelle.xlsx").wert["updated"]
            .as_f64()
            .unwrap();
        schreiben(&dir, "anhaenge/Tabelle.xlsx", b"zwei!");
        let s3 = alles(&dir, 3000.0);
        let neu = nach_pfad(&s3, "anhaenge/Tabelle.xlsx");
        assert!(neu.wert["updated"].as_f64().unwrap() > vorher);
        assert_ne!(
            neu.quelle.as_ref().unwrap().sha256,
            nach_pfad(&s, "anhaenge/Tabelle.xlsx")
                .quelle
                .as_ref()
                .unwrap()
                .sha256
        );
    }

    #[test]
    fn budget_haelt_zurueck_ohne_grabstein() {
        let dir = buch("budget");
        schreiben(&dir, "anhaenge/gross.bin", &[7u8; 64]);
        schreiben(&dir, "anhaenge/zweite.bin", &[8u8; 64]);
        // Die erste Datei eines Laufs darf immer, auch über dem Budget; die zweite wartet.
        let mut budget = Budget::mit_zeiger(10, None);
        let s = sammeln(&dir, "book_1", ICH, 1.0, &mut budget).unwrap();
        assert_eq!(budget.rest(), 0);
        assert_eq!(s.lokal.len(), 1);
        nach_pfad(&s, "anhaenge/gross.bin");
        assert_eq!(
            s.vorhanden,
            vec![datei_id("book_1", Rolle::Anhang, "anhaenge/zweite.bin")]
        );
        // Nächster Lauf: die erste ist bekannt (kein Lesen), die zweite kommt dran.
        let s = sammeln(&dir, "book_1", ICH, 2.0, &mut Budget::mit_zeiger(10, None)).unwrap();
        assert_eq!(s.lokal.len(), 2);
        assert!(s.vorhanden.is_empty());
    }

    #[test]
    fn geloeschte_datei_wird_grabstein_und_verfaellt_nach_30_tagen() {
        let dir = buch("grabstein");
        schreiben(&dir, "notizen/a.md", b"a");
        // Echte Uhrzeit: `updated` einer neuen Datei ist ihre Änderungszeit.
        let jetzt = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as f64
            + 1000.0;
        alles(&dir, jetzt);
        let id = datei_id("book_1", Rolle::Notiz, "notizen/a.md");
        // Im Explorer gelöscht: fehlt lokal, aber kein Grabstein.
        fs::remove_file(dir.join("notizen/a.md")).unwrap();
        let s = alles(&dir, jetzt + 1.0);
        assert!(s.lokal.is_empty() && s.vorhanden.contains(&id));
        let liste = auflisten(&dir, "book_1").unwrap();
        assert!(liste.len() == 1 && liste[0].fehlt && !liste[0].erwartet);
        // In der App gelöscht: Grabstein.
        entfernen(&dir, "book_1", "notizen/a.md").unwrap();
        let s = alles(&dir, jetzt + TAG);
        let g = nach_pfad(&s, "notizen/a.md");
        assert_eq!(g.wert["deleted"], true);
        assert_eq!(g.wert["updated"], json!(jetzt + TAG));
        assert!(g.wert.get("blob").is_none() && g.quelle.is_none());
        assert_eq!(alles(&dir, jetzt + 30.0 * TAG).lokal.len(), 1);
        assert!(alles(&dir, jetzt + 31.5 * TAG).lokal.is_empty());
    }

    #[test]
    fn empfangene_datei_wird_geladen_und_ist_danach_nicht_geaendert() {
        let dir = buch("empfang");
        let (id, wert) = fremder("anhaenge/Neu.pdf", 5000.0, Some(blob(1)));
        assert_eq!(
            anwenden(&dir, "book_1", &id, &wert, false, None, "Mein PC"),
            Ok(true)
        );
        // Vor dem Laden: derselbe Datensatz (kein Echo als Änderung).
        assert_eq!(
            nach_pfad(&alles(&dir, 6000.0), "anhaenge/Neu.pdf").wert,
            wert
        );
        let abgelegt = laden(&dir, b"inhalt");
        assert_eq!(abgelegt, vec![dir.join("anhaenge/Neu.pdf")]);
        assert_eq!(fs::read(dir.join("anhaenge/Neu.pdf")).unwrap(), b"inhalt");
        let s = alles(&dir, 7000.0);
        let l = nach_pfad(&s, "anhaenge/Neu.pdf");
        assert_eq!(l.wert, wert, "nach dem Laden gleich dem empfangenen");
        assert!(l.quelle.is_none());
        // Noch einmal angewendet: nichts zu tun.
        assert_eq!(
            anwenden(&dir, "book_1", &id, &wert, false, None, "Mein PC"),
            Ok(false)
        );
    }

    #[test]
    fn konflikt_lokal_neuer_behaelt_die_fremde_fassung_als_kopie() {
        let dir = buch("konflikt-lokal");
        schreiben(&dir, "notizen/a.md", b"lokal");
        let lokal = alles(&dir, 1.0);
        let lokal_zeit = nach_pfad(&lokal, "notizen/a.md").wert["updated"]
            .as_f64()
            .unwrap();
        let (id, wert) = fremder("notizen/a.md", lokal_zeit - 10.0, Some(blob(2)));
        assert_eq!(
            anwenden(&dir, "book_1", &id, &wert, true, None, "Mein PC"),
            Ok(true)
        );
        assert_eq!(fs::read(dir.join("notizen/a.md")).unwrap(), b"lokal");
        let kopie = format!("notizen/a (Konflikt Gerät {}).md", &DU[..6]);
        laden(&dir, b"fremd");
        assert_eq!(fs::read(dir.join(&kopie)).unwrap(), b"fremd");
        let s = alles(&dir, 2.0);
        assert_eq!(nach_pfad(&s, "notizen/a.md").wert["device"], ICH);
        assert_eq!(nach_pfad(&s, &kopie).wert["blob"], blob(2));
    }

    #[test]
    fn konflikt_fremd_neuer_legt_die_lokale_fassung_beiseite() {
        let dir = buch("konflikt-fremd");
        schreiben(&dir, "notizen/a.md", b"lokal");
        let lokal = alles(&dir, 1.0);
        let lokal_zeit = nach_pfad(&lokal, "notizen/a.md").wert["updated"]
            .as_f64()
            .unwrap();
        let (id, wert) = fremder("notizen/a.md", lokal_zeit + 10.0, Some(blob(3)));
        assert_eq!(
            anwenden(&dir, "book_1", &id, &wert, true, None, "Mein PC"),
            Ok(true)
        );
        assert_eq!(
            fs::read(dir.join("notizen/a (Konflikt Mein PC).md")).unwrap(),
            b"lokal"
        );
        laden(&dir, b"fremd");
        assert_eq!(fs::read(dir.join("notizen/a.md")).unwrap(), b"fremd");
        let s = alles(&dir, 2.0);
        assert_eq!(nach_pfad(&s, "notizen/a.md").wert, wert);
        assert_eq!(
            nach_pfad(&s, "notizen/a (Konflikt Mein PC).md").wert["device"],
            ICH
        );
    }

    #[test]
    fn gleichstand_entscheidet_die_groessere_geraete_id() {
        assert!(gewinnt((5.0, DU), (5.0, ICH)));
        assert!(!gewinnt((5.0, ICH), (5.0, DU)));
        assert!(gewinnt((6.0, ICH), (5.0, DU)));
    }

    #[test]
    fn fremder_grabstein_legt_unveraenderte_datei_in_den_papierkorb() {
        let dir = buch("papierkorb");
        schreiben(&dir, "anhaenge/x.txt", b"x");
        alles(&dir, 1.0);
        let (id, wert) = fremder("anhaenge/x.txt", 9.0 * TAG, None);
        assert_eq!(
            anwenden(&dir, "book_1", &id, &wert, false, None, "Mein PC"),
            Ok(true)
        );
        assert!(!dir.join("anhaenge/x.txt").exists());
        let korb = dir
            .parent()
            .unwrap()
            .join(".papierkorb/dateien/book_1/anhaenge/x.txt");
        assert_eq!(fs::read(korb).unwrap(), b"x");
        let s = alles(&dir, 9.5 * TAG);
        assert_eq!(nach_pfad(&s, "anhaenge/x.txt").wert, wert);
    }

    #[test]
    fn hub_loeschung_vergisst_den_eintrag() {
        let dir = buch("vergessen");
        schreiben(&dir, "anhaenge/y.txt", b"y");
        alles(&dir, 1.0);
        let id = datei_id("book_1", Rolle::Anhang, "anhaenge/y.txt");
        assert_eq!(vergessen(&dir, "book_1", &id), Ok(true));
        assert!(!dir.join("anhaenge/y.txt").exists());
        assert!(dir
            .parent()
            .unwrap()
            .join(".papierkorb/dateien/book_1/anhaenge/y.txt")
            .is_file());
        assert_eq!(vergessen(&dir, "book_1", &id), Ok(false));
    }

    #[test]
    fn neuere_version_wird_nur_aufbewahrt() {
        let dir = buch("version");
        let (id, mut wert) = fremder("anhaenge/z.bin", 1.0, Some(blob(4)));
        wert["v"] = json!(2);
        wert["neu"] = json!("etwas");
        assert_eq!(
            anwenden(&dir, "book_1", &id, &wert, false, None, "Mein PC"),
            Ok(true)
        );
        let s = alles(&dir, 2.0);
        assert!(s.lokal.is_empty());
        assert_eq!(s.vorhanden, vec![id.clone()]);
        assert!(erwartet(&dir).unwrap().is_empty());
        assert_eq!(
            anwenden(&dir, "book_1", &id, &wert, false, None, "Mein PC"),
            Ok(false)
        );
    }

    #[test]
    fn unbekannte_felder_gehen_unveraendert_weiter() {
        let dir = buch("extra");
        let (id, mut wert) = fremder("notizen/e.md", 1.0, Some(blob(5)));
        wert["farbe"] = json!("blau");
        anwenden(&dir, "book_1", &id, &wert, false, None, "Mein PC").unwrap();
        laden(&dir, b"e");
        assert_eq!(nach_pfad(&alles(&dir, 2.0), "notizen/e.md").wert, wert);
    }

    #[test]
    fn ungueltige_datensaetze_werden_abgewiesen() {
        let dir = buch("ungueltig");
        let (id, wert) = fremder("notizen/a.md", 1.0, Some(blob(6)));
        let falsch = |f: &dyn Fn(&mut Value)| {
            let mut w = wert.clone();
            f(&mut w);
            anwenden(&dir, "book_1", &id, &w, false, None, "Mein PC")
        };
        for r in [
            falsch(&|w| w["path"] = json!("notizen/../buch.json")),
            falsch(&|w| w["role"] = json!("anhang")),
            falsch(&|w| w["book"] = json!("book_2")),
            falsch(&|w| w["v"] = json!(0)),
            falsch(&|w| {
                w.as_object_mut().unwrap().remove("blob");
            }),
            falsch(&|w| w["device"] = json!("")),
        ] {
            assert!(matches!(r, Err(AnwendFehler::Ungueltig(_))), "{r:?}");
        }
        assert!(erwartet(&dir).unwrap().is_empty());
    }

    #[test]
    fn lokale_aenderung_waehrend_des_ladens_bleibt_als_kopie() {
        let dir = buch("ablegen-konflikt");
        schreiben(&dir, "notizen/b.md", b"alt");
        alles(&dir, 1.0);
        let (id, wert) = fremder("notizen/b.md", 9.0e12, Some(blob(7)));
        anwenden(&dir, "book_1", &id, &wert, false, None, "Mein PC").unwrap();
        schreiben(&dir, "notizen/b.md", b"inzwischen lokal");
        laden(&dir, b"neu");
        assert_eq!(fs::read(dir.join("notizen/b.md")).unwrap(), b"neu");
        assert_eq!(
            fs::read(dir.join("notizen/b (Konflikt Mein PC).md")).unwrap(),
            b"inzwischen lokal"
        );
    }

    #[test]
    fn parkplatz_bewahrt_auf_bis_das_buch_kommt() {
        let dir = buch("parken");
        let bib = dir.parent().unwrap();
        let (id, wert) = fremder("notizen/p.md", 1.0, Some(blob(8)));
        parken(bib, &id, &wert).unwrap();
        assert!(parken(bib, "../x", &wert).is_err());
        assert_eq!(geparkt(bib), vec![(id.clone(), wert)]);
        ausparken(bib, &id).unwrap();
        assert!(geparkt(bib).is_empty());
    }

    #[test]
    fn notiz_und_anhaenge_landen_im_buch_ordner_und_in_der_liste() {
        let dir = buch("oberflaeche");
        // Ein Name, der bereinigt werden muss: „?“ ist unter Windows gar nicht anlegbar,
        // dort steht ein führendes Leerzeichen (erlaubt, wird aber entfernt).
        #[cfg(unix)]
        let roh = "Tabelle?.xlsx";
        #[cfg(not(unix))]
        let roh = " Tabelle_.xlsx";
        let quelle = dir.parent().unwrap().join(roh);
        fs::write(&quelle, b"tabelle").unwrap();
        let a = anhaenge_hinzufuegen(&dir, std::slice::from_ref(&quelle)).unwrap();
        let b = anhaenge_hinzufuegen(&dir, std::slice::from_ref(&quelle)).unwrap();
        assert_eq!(a, vec!["anhaenge/Tabelle_.xlsx".to_string()]);
        assert_eq!(b, vec!["anhaenge/Tabelle_ (2).xlsx".to_string()]);
        // Kopiert wird über eine eigene Zwischendatei; danach liegt dort nichts mehr.
        assert_eq!(
            fs::read_dir(dir.join(ZWISCHENABLAGE)).unwrap().count(),
            0,
            "keine Zwischendatei übrig"
        );
        assert!(anhaenge_hinzufuegen(&dir, std::slice::from_ref(&dir)).is_err());
        let n = notiz_anlegen(&dir, "Gedanken: Kapitel 1").unwrap();
        assert_eq!(n, "notizen/Gedanken_ Kapitel 1.md");
        assert_eq!(
            fs::read_to_string(dir.join(&n)).unwrap(),
            "# Gedanken: Kapitel 1\n\n"
        );
        assert_eq!(
            notiz_anlegen(&dir, "Gedanken: Kapitel 1").unwrap(),
            "notizen/Gedanken_ Kapitel 1 (2).md"
        );
        assert!(notiz_anlegen(&dir, "  ").is_err());
        // Ein erwarteter Inhalt erscheint schon vor dem Laden.
        let (id, wert) = fremder("transkript/page_9.md", 1.0, Some(blob(9)));
        anwenden(&dir, "book_1", &id, &wert, false, None, "Mein PC").unwrap();
        let liste = auflisten(&dir, "book_1").unwrap();
        let rels: Vec<&str> = liste.iter().map(|d| d.rel.as_str()).collect();
        assert_eq!(
            rels,
            vec![
                "notizen/Gedanken_ Kapitel 1 (2).md",
                "notizen/Gedanken_ Kapitel 1.md",
                "anhaenge/Tabelle_ (2).xlsx",
                "anhaenge/Tabelle_.xlsx",
                "transkript/page_9.md",
            ]
        );
        let t = liste.last().unwrap();
        assert!(t.erwartet && t.fassung == "page_9" && t.groesse == 5);
        assert!(!liste[0].konflikt);
        // Entfernen: in den Papierkorb, nicht weg.
        entfernen(&dir, "book_1", "anhaenge/Tabelle_.xlsx").unwrap();
        assert!(!dir.join("anhaenge/Tabelle_.xlsx").exists());
        assert!(dir
            .parent()
            .unwrap()
            .join(".papierkorb/dateien/book_1/anhaenge/Tabelle_.xlsx")
            .is_file());
        assert!(entfernen(&dir, "book_1", "../buch.json").is_err());
        assert!(
            pfad(&dir, "transkript/page_9.md").is_err(),
            "noch nicht geladen"
        );
        assert!(ist_konflikt("a (Konflikt Mein PC).md") && !ist_konflikt("Konflikt.md"));
    }

    /// Review P3a/2: Ablegen ersetzt nie, was am Ziel liegt.
    #[test]
    fn verschieben_ersetzt_nie() {
        let dir = buch("ohne-ersetzen");
        schreiben(&dir, "anhaenge/a.txt", b"neu");
        schreiben(&dir, "anhaenge/b.txt", b"fremd");
        let fehler =
            verschieben_ohne_ersetzen(&dir.join("anhaenge/a.txt"), &dir.join("anhaenge/b.txt"))
                .unwrap_err();
        assert_eq!(fehler.kind(), std::io::ErrorKind::AlreadyExists);
        assert_eq!(fs::read(dir.join("anhaenge/a.txt")).unwrap(), b"neu");
        assert_eq!(fs::read(dir.join("anhaenge/b.txt")).unwrap(), b"fremd");
        // Frei: verschoben, die Quelle ist weg.
        verschieben_ohne_ersetzen(&dir.join("anhaenge/a.txt"), &dir.join("anhaenge/c.txt"))
            .unwrap();
        assert!(!dir.join("anhaenge/a.txt").exists());
        assert_eq!(fs::read(dir.join("anhaenge/c.txt")).unwrap(), b"neu");
        // Ein Ordner geht ebenso (Papierkorb ganzer Rollen-Ordner).
        fs::create_dir_all(dir.join("x/inhalt")).unwrap();
        verschieben_ohne_ersetzen(&dir.join("x/inhalt"), &dir.join("x/anders")).unwrap();
        assert!(dir.join("x/anders").is_dir());
        fs::create_dir_all(dir.join("x/inhalt")).unwrap();
        assert!(verschieben_ohne_ersetzen(&dir.join("x/inhalt"), &dir.join("x/anders")).is_err());
    }

    /// Review P3a/2: ein Download, dessen Ziel inzwischen belegt ist, ersetzt es nicht;
    /// der Vorgänger wird zur Konfliktkopie.
    #[test]
    fn ablegen_ersetzt_keine_neue_datei_am_ziel() {
        let dir = buch("ablegen-belegt");
        let (id, wert) = fremder("notizen/n.md", 10.0, Some(blob(1)));
        anwenden(&dir, "book_1", &id, &wert, false, None, "Mein PC").unwrap();
        schreiben(&dir, "notizen/n.md", b"lokal neu");
        laden(&dir, b"fremd");
        assert_eq!(fs::read(dir.join("notizen/n.md")).unwrap(), b"fremd");
        assert_eq!(
            fs::read(dir.join("notizen/n (Konflikt Mein PC).md")).unwrap(),
            b"lokal neu"
        );
    }

    /// Review P3a/3: Groß-/Kleinschreibung trennt zwei Dateien nicht überall. Keine von
    /// beiden wird abgeglichen, nichts wird Grabstein; die Liste zeigt beide.
    #[cfg(target_os = "linux")]
    #[test]
    fn gross_klein_kollision_haelt_zurueck() {
        let dir = buch("kollision");
        schreiben(&dir, "notizen/a.md", b"klein");
        let s = alles(&dir, 1000.0);
        let id_klein = nach_pfad(&s, "notizen/a.md").id.clone();
        // Zweite Schreibweise dazu: beide halten, keine neue ID, kein Grabstein.
        schreiben(&dir, "notizen/A.md", b"gross");
        let s = alles(&dir, 2000.0);
        assert!(s.lokal.iter().all(|l| l.wert["path"] != "notizen/A.md"));
        assert!(s.lokal.iter().all(|l| l.id != id_klein));
        assert!(s.vorhanden.contains(&id_klein));
        assert!(s
            .hinweise
            .iter()
            .any(|h| h.contains("Groß-/Kleinschreibung")));
        assert_eq!(auflisten(&dir, "book_1").unwrap().len(), 2);
        // Umbenannt (nur Groß/Klein): der alte Eintrag bleibt, kein Grabstein.
        fs::remove_file(dir.join("notizen/a.md")).unwrap();
        let s = alles(&dir, 3000.0);
        assert!(s.lokal.iter().all(|l| l.id != id_klein));
        assert!(s.vorhanden.contains(&id_klein));
        assert!(s.lokal.iter().all(|l| l.wert["path"] != "notizen/A.md"));
        // Auch Ordner: anhaenge/Ordner und anhaenge/ordner.
        schreiben(&dir, "anhaenge/Ordner/x.txt", b"x");
        schreiben(&dir, "anhaenge/ordner/y.txt", b"y");
        let s = alles(&dir, 4000.0);
        assert!(s
            .lokal
            .iter()
            .all(|l| !l.wert["path"].as_str().unwrap().starts_with("anhaenge/")));
    }

    /// Review P3a/4: eine Datei in anderer Unicode-Form (NFD) verdeckt den bekannten
    /// NFC-Eintrag, statt ihn zu löschen.
    #[cfg(target_os = "linux")]
    #[test]
    fn andere_unicode_form_loescht_nicht() {
        let dir = buch("nfd");
        schreiben(&dir, "notizen/\u{e4}.md", b"a");
        let s = alles(&dir, 1000.0);
        let id = nach_pfad(&s, "notizen/\u{e4}.md").id.clone();
        fs::rename(
            dir.join("notizen/\u{e4}.md"),
            dir.join("notizen/a\u{308}.md"),
        )
        .unwrap();
        let s = alles(&dir, 2000.0);
        assert!(s.lokal.iter().all(|l| l.id != id), "kein Grabstein");
        assert!(s.vorhanden.contains(&id));
    }

    /// Review P3a/3: ein empfangener Name, der hier mit einer anderen Datei zusammenfiele,
    /// wird zurückgehalten (nicht angewendet, nicht verworfen).
    #[test]
    fn empfangene_kollision_wird_zurueckgehalten() {
        let dir = buch("kollision-empfang");
        schreiben(&dir, "notizen/a.md", b"eins");
        alles(&dir, 1000.0);
        let (id, wert) = fremder("notizen/A.md", 5000.0, Some(blob(1)));
        assert!(matches!(
            anwenden(&dir, "book_1", &id, &wert, false, None, "Mein PC"),
            Err(AnwendFehler::Halten(_))
        ));
        let (id, wert) = fremder("notizen/Unterordner/x.md", 5000.0, Some(blob(2)));
        schreiben(&dir, "notizen/unterordner/y.md", b"zwei");
        assert!(matches!(
            anwenden(&dir, "book_1", &id, &wert, false, None, "Mein PC"),
            Err(AnwendFehler::Halten(_))
        ));
        assert!(erwartet(&dir).unwrap().is_empty());
        // Ein Grabstein unter anderer Schreibweise berührt nichts und wird nicht gehalten.
        let (id, wert) = fremder("notizen/A.md", 6000.0, None);
        assert_eq!(
            anwenden(&dir, "book_1", &id, &wert, false, None, "Mein PC"),
            Ok(true)
        );
        assert_eq!(fs::read(dir.join("notizen/a.md")).unwrap(), b"eins");
    }

    /// Review P3a/8: die Ausnahme vom Budget geht reihum. Eine große Datei, die sich
    /// ständig ändert, hält die nächste nicht auf.
    #[test]
    fn budget_ausnahme_geht_reihum() {
        let dir = buch("budget-reihe");
        let bibliothek = dir.parent().unwrap().to_path_buf();
        schreiben(&dir, "anhaenge/a.bin", &[1u8; 64]);
        schreiben(&dir, "anhaenge/b.bin", &[2u8; 64]);
        let lauf = |jetzt: f64| {
            let mut b = Budget::mit_zeiger(10, Budget::zeiger_laden(&bibliothek));
            let s = sammeln(&dir, "book_1", ICH, jetzt, &mut b).unwrap();
            b.sichern(&bibliothek).unwrap();
            s
        };
        let gesendet = |s: &Sammlung| -> Vec<String> {
            s.lokal
                .iter()
                .map(|l| l.wert["path"].as_str().unwrap().to_string())
                .collect()
        };
        assert_eq!(gesendet(&lauf(1.0)), vec!["anhaenge/a.bin"]);
        // a ändert sich ständig (anderer Inhalt, andere Größe): trotzdem kommt b dran.
        schreiben(&dir, "anhaenge/a.bin", &[3u8; 65]);
        let s = lauf(2.0);
        assert!(gesendet(&s).contains(&"anhaenge/b.bin".to_string()));
        let id_a = datei_id("book_1", Rolle::Anhang, "anhaenge/a.bin");
        assert!(s.vorhanden.contains(&id_a), "a wartet, kein Grabstein");
        // Nichts mehr hinter dem Zeiger: die Reihe beginnt vorn, a kommt wieder dran.
        lauf(3.0);
        let s = lauf(4.0);
        let a = nach_pfad(&s, "anhaenge/a.bin");
        assert_eq!(a.quelle.as_ref().unwrap().groesse, 65);
    }

    /// Review P3a/3a: ein von außen entfernter Rollenordner löscht nichts; die Liste zeigt
    /// „Ordner fehlt“. Entfernen in der App ist die ausdrückliche Löschung.
    #[test]
    fn fehlender_ordner_nur_in_der_app_loeschbar() {
        let dir = buch("ordner-fehlt");
        schreiben(&dir, "notizen/a.md", b"a");
        schreiben(&dir, "notizen/b.md", b"b");
        let s = alles(&dir, 1000.0);
        let id_a = nach_pfad(&s, "notizen/a.md").id.clone();
        let id_b = nach_pfad(&s, "notizen/b.md").id.clone();
        fs::remove_dir_all(dir.join("notizen")).unwrap();
        let s = alles(&dir, 2000.0);
        assert!(s.lokal.iter().all(|l| l.id != id_a && l.id != id_b));
        assert!(s.vorhanden.contains(&id_a) && s.vorhanden.contains(&id_b));
        let liste = auflisten(&dir, "book_1").unwrap();
        assert_eq!(liste.len(), 2);
        assert!(liste.iter().all(|d| d.fehlt && !d.erwartet));
        // In der App entfernt: Grabstein, die andere bleibt zurückgehalten.
        entfernen(&dir, "book_1", "notizen/a.md").unwrap();
        let s = alles(&dir, 3000.0);
        assert_eq!(nach_pfad(&s, "notizen/a.md").wert["deleted"], true);
        assert!(s.vorhanden.contains(&id_b));
        assert_eq!(auflisten(&dir, "book_1").unwrap().len(), 1);
        // Ordner wiederhergestellt: b ist wieder normal da.
        schreiben(&dir, "notizen/b.md", b"b");
        let s = alles(&dir, 4000.0);
        assert_eq!(nach_pfad(&s, "notizen/b.md").wert["deleted"], false);
        assert!(entfernen(&dir, "book_1", "notizen/a.md").is_err());
    }

    /// Review P3a/3a: „Neue Notiz“ legt den fehlenden Ordner neu an; die alten Notizen
    /// bleiben „fehlt lokal“ und lassen sich vom Hub wiederherstellen.
    #[test]
    fn neu_angelegter_ordner_loescht_nichts_und_wiederherstellen() {
        let dir = buch("neuer-ordner");
        schreiben(&dir, "notizen/alt.md", b"alt");
        let s = alles(&dir, 1000.0);
        let id = nach_pfad(&s, "notizen/alt.md").id.clone();
        fs::remove_dir_all(dir.join("notizen")).unwrap();
        alles(&dir, 2000.0);
        let neu = notiz_anlegen(&dir, "Neu").unwrap();
        let s = alles(&dir, 3000.0);
        assert!(s.lokal.iter().all(|l| l.id != id), "kein Grabstein");
        assert!(s.vorhanden.contains(&id));
        nach_pfad(&s, &neu);
        // Eigene Datei: den Verweis bildet der Aufrufer beim nächsten Sammeln.
        wiederherstellen(&dir, "notizen/alt.md").unwrap();
        assert!(auflisten(&dir, "book_1")
            .unwrap()
            .iter()
            .any(|d| d.rel == "notizen/alt.md" && d.erwartet && !d.fehlt));
        let s = alles(&dir, 4000.0);
        assert_eq!(s.wiederherstellen.len(), 1);
        assert_eq!(s.wiederherstellen[0].id, id);
        erwarten(&dir, "notizen/alt.md", &blob(7)).unwrap();
        assert_eq!(laden(&dir, b"alt"), vec![dir.join("notizen/alt.md")]);
        let s = alles(&dir, 5000.0);
        assert_eq!(nach_pfad(&s, "notizen/alt.md").wert["deleted"], false);
        assert!(alles(&dir, 6000.0).wiederherstellen.is_empty());
        // Fremde Datei mit bekanntem Verweis: sofort erwartet.
        let (fid, fremd) = fremder("notizen/f.md", 7000.0, Some(blob(8)));
        anwenden(&dir, "book_1", &fid, &fremd, false, None, "Mein PC").unwrap();
        laden(&dir, b"fremd");
        fs::remove_file(dir.join("notizen/f.md")).unwrap();
        wiederherstellen(&dir, "notizen/f.md").unwrap();
        assert_eq!(erwartet(&dir).unwrap().len(), 1);
        assert!(wiederherstellen(&dir, "notizen/Neu.md").is_err(), "ist da");
    }

    /// Review P3a/3b: zwei Datensätze A.md und a.md, auf der Platte eine Datei (auf jedem
    /// System): kein Grabstein, kein Verlust, einer zurückgehalten und in der Liste.
    #[test]
    fn zwei_schreibweisen_eine_datei() {
        let dir = buch("zwei-schreibweisen");
        let bibliothek = dir.parent().unwrap().to_path_buf();
        let (id_gross, gross) = fremder("notizen/A.md", 5000.0, Some(blob(1)));
        anwenden(&dir, "book_1", &id_gross, &gross, false, None, "Mein PC").unwrap();
        laden(&dir, b"gross");
        let (id_klein, klein) = fremder("notizen/a.md", 6000.0, Some(blob(2)));
        // So hält es `sync/book_files.rs`: zurückgehalten heißt geparkt.
        assert!(matches!(
            anwenden(&dir, "book_1", &id_klein, &klein, false, None, "Mein PC"),
            Err(AnwendFehler::Halten(_))
        ));
        parken(&bibliothek, &id_klein, &klein).unwrap();
        let s = alles(&dir, 7000.0);
        assert_eq!(nach_pfad(&s, "notizen/A.md").wert["deleted"], false);
        assert!(s.lokal.iter().all(|l| l.id != id_klein));
        assert_eq!(fs::read(dir.join("notizen/A.md")).unwrap(), b"gross");
        let liste = auflisten(&dir, "book_1").unwrap();
        let rels: Vec<(&str, bool)> = liste
            .iter()
            .map(|d| (d.rel.as_str(), d.zurueckgehalten))
            .collect();
        assert_eq!(rels, vec![("notizen/A.md", false), ("notizen/a.md", true)]);
        assert_eq!(geparkt(&bibliothek).len(), 1);
    }

    /// Review P3a/1: ein Datensatz neuerer Version überdeckt den eigenen; kein Ping-Pong.
    #[test]
    fn neuere_version_wird_nicht_mit_alter_ueberschrieben() {
        let dir = buch("v2");
        schreiben(&dir, "notizen/a.md", b"eins");
        let s = alles(&dir, 1000.0);
        let id = nach_pfad(&s, "notizen/a.md").id.clone();
        let (_, mut v2) = fremder("notizen/a.md", 5000.0, Some(blob(1)));
        v2["v"] = json!(2);
        assert_eq!(
            anwenden(&dir, "book_1", &id, &v2, false, None, "Mein PC"),
            Ok(true)
        );
        let s2 = alles(&dir, 6000.0);
        assert!(s2.lokal.iter().all(|l| l.id != id), "{:?}", s2.lokal);
        assert!(s2.vorhanden.contains(&id));
        assert!(lokal_aus_stand(&dir, "book_1", &id).unwrap().is_none());
    }

    /// Review P3a/3: ein nicht lesbarer oder verknüpfter Rollenordner wird nie zu
    /// Grabsteinen; ein gelöschter schon.
    #[cfg(unix)]
    #[test]
    fn verdeckter_ordner_loescht_nichts() {
        let dir = buch("verdeckt");
        schreiben(&dir, "anhaenge/a.pdf", b"eins");
        schreiben(&dir, "notizen/n.md", b"zwei");
        let s = alles(&dir, 1000.0);
        let id = nach_pfad(&s, "anhaenge/a.pdf").id.clone();
        let anderswo = dir.parent().unwrap().join("anderswo");
        fs::rename(dir.join("anhaenge"), &anderswo).unwrap();
        std::os::unix::fs::symlink(&anderswo, dir.join("anhaenge")).unwrap();
        let s2 = alles(&dir, 2000.0);
        assert!(s2.lokal.iter().all(|l| l.id != id));
        assert!(s2.vorhanden.contains(&id));
        // Ganzer Ordner fehlt (verschoben, Platzhalter, Laufwerk weg): auch kein Grabstein.
        fs::remove_file(dir.join("anhaenge")).unwrap();
        let s3 = alles(&dir, 2500.0);
        assert!(s3.lokal.iter().all(|l| l.id != id));
        assert!(s3.vorhanden.contains(&id));
        // Ordner neu angelegt (etwa durch „Neue Notiz“ oder einen Download), die alte
        // Datei fehlt weiter: immer noch kein Grabstein.
        fs::create_dir(dir.join("anhaenge")).unwrap();
        schreiben(&dir, "anhaenge/neu.pdf", b"neu");
        let s3 = alles(&dir, 3000.0);
        assert!(s3.lokal.iter().all(|l| l.id != id));
        assert!(s3.vorhanden.contains(&id));
        // Erst das Löschen in der App macht den Grabstein.
        entfernen(&dir, "book_1", "anhaenge/a.pdf").unwrap();
        let s3 = alles(&dir, 3500.0);
        assert_eq!(nach_pfad(&s3, "anhaenge/a.pdf").wert["deleted"], true);
        assert_eq!(nach_pfad(&s3, "notizen/n.md").wert["deleted"], false);
    }

    /// Review P3a/4: gleicher Inhalt auf beiden Seiten ist kein Konflikt und kein Download.
    #[test]
    fn gleicher_inhalt_ohne_konflikt_und_ohne_download() {
        let dir = buch("gleich");
        schreiben(&dir, "anhaenge/a.pdf", b"hallo");
        let s = alles(&dir, 1000.0);
        let id = nach_pfad(&s, "anhaenge/a.pdf").id.clone();
        let (_, v) = fremder("anhaenge/a.pdf", 5000.0, Some(blob(7)));
        assert_eq!(
            anwenden(&dir, "book_1", &id, &v, true, Some(&blob(7)), "Mein PC"),
            Ok(false)
        );
        assert!(erwartet(&dir).unwrap().is_empty());
        let namen: Vec<_> = fs::read_dir(dir.join("anhaenge"))
            .unwrap()
            .flatten()
            .collect();
        assert_eq!(namen.len(), 1);
        // Der eigene Datensatz trägt jetzt den fremden Blob und die fremden Angaben.
        let l = lokal_aus_stand(&dir, "book_1", &id).unwrap().unwrap();
        assert_eq!(l.wert, v);
        assert!(l.quelle.is_none());
        // Anderer Inhalt: wie bisher erwartet bzw. Konflikt.
        let (_, w) = fremder("anhaenge/a.pdf", 6000.0, Some(blob(8)));
        anwenden(&dir, "book_1", &id, &w, false, Some(&blob(7)), "Mein PC").unwrap();
        assert_eq!(erwartet(&dir).unwrap().len(), 1);
    }

    /// Review P3a/4: liegt der geladene Inhalt schon am Ziel, entsteht keine Kopie.
    #[test]
    fn ablegen_mit_gleichem_inhalt_am_ziel_ohne_kopie() {
        let dir = buch("ablegen-gleich");
        let (id, v) = fremder("notizen/b.md", 5000.0, Some(blob(2)));
        anwenden(&dir, "book_1", &id, &v, false, None, "Mein PC").unwrap();
        schreiben(&dir, "notizen/b.md", b"gleich");
        assert_eq!(laden(&dir, b"gleich").len(), 1);
        let namen: Vec<_> = fs::read_dir(dir.join("notizen"))
            .unwrap()
            .flatten()
            .collect();
        assert_eq!(namen.len(), 1);
        assert!(erwartet(&dir).unwrap().is_empty());
    }

    /// Review P3a/5: ein unlesbarer Stand wird beiseitegelegt, nichts geht verloren.
    #[test]
    fn defekter_stand_wird_beiseitegelegt() {
        let dir = buch("defekt");
        schreiben(&dir, "notizen/a.md", b"eins");
        alles(&dir, 1000.0);
        fs::write(dir.join(STAND), b"{kaputt").unwrap();
        let s = alles(&dir, 2000.0);
        assert_eq!(nach_pfad(&s, "notizen/a.md").wert["deleted"], false);
        assert!(fs::read_dir(&dir).unwrap().flatten().any(|e| e
            .file_name()
            .to_string_lossy()
            .starts_with(&format!("{STAND}.defekt-"))));
        assert!(fs::read(dir.join("notizen/a.md")).unwrap() == b"eins");
    }

    /// Review P3a/9: zwei Konflikte vor dem ersten Download bekommen zwei Namen.
    #[test]
    fn zwei_konflikte_ueberschreiben_sich_nicht() {
        let dir = buch("zwei-konflikte");
        schreiben(&dir, "notizen/a.md", b"eigen");
        let s = alles(&dir, 1000.0);
        let id = nach_pfad(&s, "notizen/a.md").id.clone();
        // Lokal gewinnt beide Male (neuer), die fremden Fassungen werden Kopien.
        let (_, a) = fremder("notizen/a.md", 1.0, Some(blob(3)));
        anwenden(&dir, "book_1", &id, &a, true, None, "Mein PC").unwrap();
        let (_, b) = fremder("notizen/a.md", 2.0, Some(blob(4)));
        anwenden(&dir, "book_1", &id, &b, true, None, "Mein PC").unwrap();
        let offen = erwartet(&dir).unwrap();
        assert_eq!(offen.len(), 2, "{offen:?}");
        assert_ne!(offen[0].rel, offen[1].rel);
    }

    /// Review P3a/8: Löschen eines Buchs nimmt Notizen und Anhänge nie endgültig mit.
    #[test]
    fn ordner_retten_legt_in_den_papierkorb() {
        let dir = buch("retten");
        schreiben(&dir, "notizen/a.md", b"eins");
        schreiben(&dir, "anhaenge/b.pdf", b"zwei");
        fs::create_dir_all(dir.join("transkript")).unwrap();
        ordner_retten(&dir, "book_1").unwrap();
        let korb = dir
            .parent()
            .unwrap()
            .join(PAPIERKORB)
            .join(PAPIERKORB_DATEIEN)
            .join("book_1");
        assert_eq!(fs::read(korb.join("notizen/a.md")).unwrap(), b"eins");
        assert_eq!(fs::read(korb.join("anhaenge/b.pdf")).unwrap(), b"zwei");
        assert!(!dir.join("notizen").exists() && !dir.join("anhaenge").exists());
        // Zweimal: nichts überschrieben.
        schreiben(&dir, "notizen/a.md", b"drei");
        ordner_retten(&dir, "book_1").unwrap();
        assert_eq!(fs::read(korb.join("notizen/a.md")).unwrap(), b"eins");
        assert_eq!(fs::read_dir(&korb).unwrap().count(), 3);
    }
}
