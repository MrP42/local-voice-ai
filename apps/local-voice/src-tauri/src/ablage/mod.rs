//! Dokumentenablage, Stufen D1, D2 und D6 (Epic local-voice-ai#395, Issues #407/#408, wai-sdk ADR-0029).
//!
//! Lokale Ablage im Datenordner der App: Ordner, Dateien mit Fassungen, Papierkorb, sichere Ablage mit
//! Journal und Wiederanlauf, gerätelokale KI-Freigaben (D1); Textfassungen im begrenzten Unterprozess,
//! lokaler Index und Volltextsuche ohne KI (D2). Kein Sync (D4), keine KI (D3).
//!
//! * [`zusammenfuehren`]: reine Merge-Logik der synchronisierten Datensätze (keine I/O, keine Uhr).
//! * [`aufbewahrung`]: welche Fassungen bleiben (reine Funktion).
//! * [`freigabe`]: `freigabe.json`, gerätelokal, mit `.vorher` und Zustand „unklar“.
//! * [`speicher`]: `ablage.json` (Journal + Baum), Dateien, Papierkorb.
//! * [`extrakt`]: Bytes → Abschnitte mit Ankern (Markenformat v1), läuft im Unterprozess.
//! * [`textlauf`]: der Unterprozess `lva-extrakt` mit Zeit- und Speichergrenze.
//! * [`textfassung`]: Warteschlange (ein Arbeiter, abbrechbar, Wiederanlauf aus dem Stand).
//! * [`index`]: `index.db` über `wai-retrieval`, Abgleich und Suche.
//! * [`semantik`]: D6, lokale Vektoren (`multilingual-e5-small`, ONNX) für die hybride Suche (Opt-in).
//! * [`befehle`]: die Tauri-Befehle.
//!
//! Datenschutz: nie Datei- oder Ordnernamen, Pfade oder Inhalte in Log oder Fehlertext; Fehler tragen
//! nur einen stabilen Code und einen allgemeinen Text.

pub mod aufbewahrung;
pub mod befehle;
pub mod extrakt;
pub mod freigabe;
pub mod index;
pub mod semantik;
pub mod speicher;
pub mod textfassung;
pub mod textlauf;
pub mod zusammenfuehren;

#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_d2;

use serde::Serialize;
use std::fmt;
use std::fs;
use std::io::Write;
use std::path::Path;
use ulid::Ulid;

/// Fehler mit stabilem Code (Tests, Oberfläche) und allgemeinem Text ohne Namen oder Inhalte.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, specta::Type)]
pub struct Fehler {
    pub code: String,
    pub message: String,
}

impl Fehler {
    pub fn neu(code: &str, message: impl Into<String>) -> Self {
        Self {
            code: code.to_string(),
            message: message.into(),
        }
    }

    pub fn ungueltig(message: &str) -> Self {
        Self::neu("ablage.ungueltig", message)
    }

    pub fn speicher(message: &str) -> Self {
        Self::neu("ablage.speicher", message)
    }
}

impl fmt::Display for Fehler {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for Fehler {}

/// An die Oberfläche geht `<code>: <text>`; sie übersetzt den Code und zeigt sonst den Text.
impl From<Fehler> for String {
    fn from(f: Fehler) -> Self {
        format!("{}: {}", f.code, f.message)
    }
}

pub type Res<T> = Result<T, Fehler>;

pub const PRAEFIX_ORDNER: &str = "ord_";
pub const PRAEFIX_DATEI: &str = "dat_";
pub const PRAEFIX_FASSUNG: &str = "fas_";
pub const PRAEFIX_GERAET: &str = "g_";
#[allow(dead_code)]
pub const PRAEFIX_WAHL: &str = "wahl_";

/// Neue Kennung `<präfix><ULID>`.
pub fn neue_id(praefix: &str) -> String {
    format!("{praefix}{}", Ulid::new())
}

/// Ist `id` eine Kennung `<präfix><ULID>`? Strenge Prüfung, auch gegen Pfadtrenner und `..`.
pub fn id_gueltig(praefix: &str, id: &str) -> bool {
    id.strip_prefix(praefix)
        .is_some_and(|r| r.len() == 26 && Ulid::from_string(r).is_ok())
}

pub fn jetzt_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

static SPERRE: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Eine Sperre für alle Schreib- und Lesevorgänge der Ablage in diesem Prozess.
pub(crate) fn sperren() -> std::sync::MutexGuard<'static, ()> {
    SPERRE.lock().unwrap_or_else(|e| e.into_inner())
}

/// Verknüpfung (Symlink, Junction) oder Reparse-Point: wird nie gefolgt.
pub fn ist_umweg(meta: &fs::Metadata) -> bool {
    if meta.file_type().is_symlink() {
        return true;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
        if meta.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return true;
        }
    }
    false
}

/// Hat die offene Datei mehr als einen Verzeichniseintrag (Hartlink)? Ein Hartlink kann auf eine Datei
/// außerhalb der Ablage zeigen (oder von dort kommen) und wird deshalb nicht gelesen, aufgenommen oder
/// als Index benutzt. Unix `nlink`, Windows `nNumberOfLinks` (`GetFileInformationByHandle`); ein Fehler
/// zählt als Hartlink (im Zweifel ablehnen).
pub fn datei_hartverknuepft(f: &fs::File) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        f.metadata().map(|m| m.nlink() > 1).unwrap_or(true)
    }
    #[cfg(windows)]
    {
        use std::os::windows::io::AsRawHandle;
        use windows::Win32::Foundation::HANDLE;
        use windows::Win32::Storage::FileSystem::{
            GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION,
        };
        let mut info = BY_HANDLE_FILE_INFORMATION::default();
        // SAFETY: gültiges, offenes Dateihandle; `info` ist ein beschreibbarer Ausgabepuffer.
        let ok = unsafe { GetFileInformationByHandle(HANDLE(f.as_raw_handle()), &mut info) };
        ok.map(|_| info.nNumberOfLinks > 1).unwrap_or(true)
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = f;
        false
    }
}

/// Wie [`datei_hartverknuepft`], für einen Pfad (ohne Folgen einer Verknüpfung geöffnet; nicht
/// öffenbar zählt als Hartlink).
pub fn pfad_hartverknuepft(p: &Path) -> bool {
    let mut o = fs::OpenOptions::new();
    o.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        o.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        o.custom_flags(0x0020_0000).share_mode(0x1 | 0x2 | 0x4);
    }
    o.open(p).map(|f| datei_hartverknuepft(&f)).unwrap_or(true)
}

/// Ordnereinträge dauerhaft machen (Unix: fsync auf den Ordner; sonst nichts).
pub fn ordner_sync(dir: &Path) {
    #[cfg(unix)]
    if let Ok(d) = fs::File::open(dir) {
        let _ = d.sync_all();
    }
    #[cfg(not(unix))]
    let _ = dir;
}

/// Schreibt `bytes` über eine eigene Zwischendatei (`create_new`, `sync_all`) und benennt sie in `ziel`
/// um. Ein Absturz hinterlässt die alte oder die neue Datei, nie eine halbe.
pub fn schreibe_atomar(ziel: &Path, bytes: &[u8]) -> Res<()> {
    let dir = ziel
        .parent()
        .ok_or_else(|| Fehler::speicher("Pfad ohne Ordner."))?;
    let tmp = dir.join(format!(".{}.tmp", Ulid::new()));
    let schreiben = (|| -> std::io::Result<()> {
        let mut f = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
        fs::rename(&tmp, ziel)
    })();
    if schreiben.is_err() {
        let _ = fs::remove_file(&tmp);
        return Err(Fehler::speicher("Datei nicht speicherbar."));
    }
    ordner_sync(dir);
    Ok(())
}

/// Sind `a` und `b` gültige, verschiedene Ordner-IDs? (Kleine Hilfe für die Befehle.)
pub fn ordner_id_pruefen(id: &str) -> Res<()> {
    if id_gueltig(PRAEFIX_ORDNER, id) {
        Ok(())
    } else {
        Err(Fehler::ungueltig("Kennung des Ordners ungültig."))
    }
}

pub fn datei_id_pruefen(id: &str) -> Res<()> {
    if id_gueltig(PRAEFIX_DATEI, id) {
        Ok(())
    } else {
        Err(Fehler::ungueltig("Kennung der Datei ungültig."))
    }
}
