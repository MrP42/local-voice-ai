//! Lokales Sync-Ledger: je Objekt die zuletzt bestätigte Server-Revision und
//! der Klartext-Hash dieses Stands, dazu der Pull-Cursor und die
//! Dead-Letter-Liste. Aus Hash-Abweichung folgt „dirty" — so bleibt jeder
//! Schreibpfad der App unberührt (Spec Abschnitt 6).

use super::blobs::FileStamp;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use tauri::AppHandle;

const FILE_NAME: &str = "sync_ledger.json";

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct Entry {
    pub revision: i64,
    pub hash: String,
    #[serde(default)]
    pub deleted: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Ledger {
    #[serde(default)]
    pub cursor: i64,
    #[serde(default)]
    pub objects: BTreeMap<String, Entry>,
    /// Vom Hub abgewiesene Objekte: Schlüssel → Grund und der abgewiesene
    /// Hash. Werden nicht erneut gesendet, bis sich ihr Inhalt ändert. Der
    /// bestätigte Hash in `objects` bleibt davon unberührt — eine abgewiesene
    /// Seite gilt weiter als ungesichert (Review-Befund 14.09.).
    #[serde(default)]
    pub dead: BTreeMap<String, DeadEntry>,
    /// Blobs, die vollständig in der Dateiablage des Hubs liegen (selbst
    /// hochgeladen oder von einem anderen Gerät gemeldet).
    #[serde(default)]
    pub uploaded: BTreeSet<String>,
    /// Prüfsummen der Bibliotheksdateien (Pfad → Größe, Zeit, SHA-256).
    #[serde(default)]
    pub files: BTreeMap<String, FileStamp>,
    /// Dateien, deren Hoch- oder Herunterladen scheiterte: Blob → Versuche.
    #[serde(default)]
    pub failed: BTreeMap<String, Failure>,
    /// Stand der Neueinlese-Regel: liegt er unter `RESCAN_LEVEL`, holt der
    /// nächste Lauf alles ab Cursor 0 erneut (einmalig, z. B. nachdem Objekte
    /// wegen einer zu strengen Prüfung übersprungen wurden).
    #[serde(default)]
    pub rescan: u32,
    /// Konto, zu dem dieses Ledger gehört (`<user_id>@<hub>`, lva#400 U6). Ein Ledger eines
    /// anderen Kontos gilt als leer; fehlt die Angabe (ältere Fassung), wird sie übernommen.
    #[serde(default)]
    pub konto: Option<String>,
}

/// 1: iPhone-Buchkennungen `book_<ms>_<hex>` wurden bis 0.23.5 übersprungen.
pub const RESCAN_LEVEL: u32 = 1;

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct Failure {
    pub count: u32,
    pub last_ms: f64,
}

/// Wartezeit nach `count` Fehlschlägen: 1, 2, 4 … Minuten, höchstens 6 Stunden.
pub fn backoff_ms(count: u32) -> f64 {
    let minutes = 2f64.powi(count.saturating_sub(1).min(9) as i32).min(360.0);
    minutes * 60_000.0
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct DeadEntry {
    pub reason: String,
    pub hash: String,
}

pub fn key(collection: &str, object_id: &str) -> String {
    format!("{collection}/{object_id}")
}

impl Ledger {
    pub fn get(&self, collection: &str, object_id: &str) -> Option<&Entry> {
        self.objects.get(&key(collection, object_id))
    }

    pub fn set(&mut self, collection: &str, object_id: &str, entry: Entry) {
        let k = key(collection, object_id);
        self.dead.remove(&k);
        self.objects.insert(k, entry);
    }

    /// Ein Objekt ist dirty, wenn der Hash seines Klartexts nicht der
    /// zuletzt bestätigte ist (oder es noch nie bestätigt wurde).
    pub fn is_dirty(&self, collection: &str, object_id: &str, hash: &str) -> bool {
        match self.get(collection, object_id) {
            Some(e) => e.deleted || e.hash != hash,
            None => true,
        }
    }

    pub fn is_dead(&self, collection: &str, object_id: &str, hash: &str) -> bool {
        // Ein geänderter Inhalt bekommt eine neue Chance.
        self.dead
            .get(&key(collection, object_id))
            .is_some_and(|d| d.hash == hash)
    }

    pub fn note_failure(&mut self, blob: &str, now_ms: f64) {
        let f = self.failed.entry(blob.to_string()).or_default();
        f.count = f.count.saturating_add(1);
        f.last_ms = now_ms;
    }

    /// Ruht diese Datei nach einem Fehlschlag noch?
    pub fn backing_off(&self, blob: &str, now_ms: f64) -> bool {
        self.failed
            .get(blob)
            .is_some_and(|f| now_ms - f.last_ms < backoff_ms(f.count))
    }

    pub fn mark_dead(&mut self, collection: &str, object_id: &str, hash: &str, reason: &str) {
        self.dead.insert(
            key(collection, object_id),
            DeadEntry {
                reason: reason.to_string(),
                hash: hash.to_string(),
            },
        );
    }
}

pub fn path(app: &AppHandle) -> Result<PathBuf, String> {
    crate::portable::app_data_dir(app)
        .map(|d| d.join(FILE_NAME))
        .map_err(|e| format!("Kein Datenverzeichnis: {e}"))
}

pub fn load(app: &AppHandle) -> Ledger {
    let Ok(p) = path(app) else {
        return Ledger::default();
    };
    std::fs::read_to_string(p)
        .ok()
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_default()
}

pub fn save(app: &AppHandle, ledger: &Ledger) -> Result<(), String> {
    let p = path(app)?;
    let raw = serde_json::to_string(ledger).map_err(|e| e.to_string())?;
    crate::commands::pages::write_atomic(&p, raw.as_bytes())
}

pub fn remove(app: &AppHandle) {
    if let Ok(p) = path(app) {
        let _ = std::fs::remove_file(p);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dirty_folgt_dem_hash() {
        let mut l = Ledger::default();
        assert!(l.is_dirty("page", "p1", "h1"), "unbekannt = dirty");
        l.set(
            "page",
            "p1",
            Entry {
                revision: 1,
                hash: "h1".into(),
                deleted: false,
            },
        );
        assert!(!l.is_dirty("page", "p1", "h1"));
        assert!(l.is_dirty("page", "p1", "h2"));
        l.set(
            "page",
            "p1",
            Entry {
                revision: 2,
                hash: "h1".into(),
                deleted: true,
            },
        );
        assert!(
            l.is_dirty("page", "p1", "h1"),
            "Tombstone bestätigt, Seite lebt wieder"
        );
    }

    #[test]
    fn fehlschlaege_ruhen_mit_wachsender_pause() {
        let mut l = Ledger::default();
        assert!(!l.backing_off("b", 0.0));
        l.note_failure("b", 1_000.0);
        assert!(l.backing_off("b", 1_000.0 + 59_000.0));
        assert!(!l.backing_off("b", 1_000.0 + 61_000.0));
        l.note_failure("b", 0.0);
        assert!(
            l.backing_off("b", 119_000.0),
            "zweiter Fehlschlag: 2 Minuten"
        );
        assert_eq!(backoff_ms(50), 360.0 * 60_000.0);
    }

    #[test]
    fn dead_letter_gilt_nur_fuer_denselben_inhalt() {
        let mut l = Ledger::default();
        l.mark_dead("page", "p1", "h1", "payload_size");
        assert!(l.is_dead("page", "p1", "h1"));
        assert!(
            !l.is_dead("page", "p1", "h2"),
            "geänderter Inhalt wird erneut versucht"
        );
        assert!(
            l.is_dirty("page", "p1", "h1"),
            "abgewiesen bleibt ungesichert"
        );
        l.set(
            "page",
            "p1",
            Entry {
                revision: 1,
                hash: "h2".into(),
                deleted: false,
            },
        );
        assert!(l.dead.is_empty(), "Bestätigung löscht den Dead-Letter");
    }
}
