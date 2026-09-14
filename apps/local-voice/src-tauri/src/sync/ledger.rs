//! Lokales Sync-Ledger: je Objekt die zuletzt bestätigte Server-Revision und
//! der Klartext-Hash dieses Stands, dazu der Pull-Cursor und die
//! Dead-Letter-Liste. Aus Hash-Abweichung folgt „dirty" — so bleibt jeder
//! Schreibpfad der App unberührt (Spec Abschnitt 6).

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
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
    /// Vom Hub abgewiesene Objekte: Schlüssel → Grund. Werden nicht erneut
    /// gesendet, bis sich ihr Inhalt ändert.
    #[serde(default)]
    pub dead: BTreeMap<String, String>,
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
        let k = key(collection, object_id);
        // Ein geänderter Inhalt bekommt eine neue Chance.
        self.dead.contains_key(&k) && self.objects.get(&k).is_some_and(|e| e.hash == hash)
    }

    pub fn mark_dead(&mut self, collection: &str, object_id: &str, hash: &str, reason: &str) {
        let k = key(collection, object_id);
        self.objects
            .entry(k.clone())
            .or_default()
            .hash = hash.to_string();
        self.dead.insert(k, reason.to_string());
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
    std::fs::write(p, raw).map_err(|e| format!("Ledger schreiben: {e}"))
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
        l.set("page", "p1", Entry { revision: 1, hash: "h1".into(), deleted: false });
        assert!(!l.is_dirty("page", "p1", "h1"));
        assert!(l.is_dirty("page", "p1", "h2"));
        l.set("page", "p1", Entry { revision: 2, hash: "h1".into(), deleted: true });
        assert!(l.is_dirty("page", "p1", "h1"), "Tombstone bestätigt, Seite lebt wieder");
    }

    #[test]
    fn dead_letter_gilt_nur_fuer_denselben_inhalt() {
        let mut l = Ledger::default();
        l.mark_dead("page", "p1", "h1", "payload_size");
        assert!(l.is_dead("page", "p1", "h1"));
        assert!(!l.is_dead("page", "p1", "h2"), "geänderter Inhalt wird erneut versucht");
        l.set("page", "p1", Entry { revision: 1, hash: "h2".into(), deleted: false });
        assert!(l.dead.is_empty(), "Bestätigung löscht den Dead-Letter");
    }
}
