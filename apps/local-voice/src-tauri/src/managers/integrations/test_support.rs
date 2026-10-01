//! Gemeinsame Bausteine der Tests des Registers (nur `cfg(test)`).

use std::path::PathBuf;

use rusqlite::Connection;

use super::model::{Integration, Kind, NewIntegration};
use super::store;
use crate::managers::meetings::store::MeetingStore;

/// Ein frischer Store in einem eigenen Ordner (bleibt bis Prozessende stehen,
/// damit Verbindungen aus Threads ihn nicht verlieren).
pub struct Fx {
    pub store: MeetingStore,
    pub db_path: PathBuf,
}

impl Fx {
    pub fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("meetings.db");
        let store = MeetingStore::open_at(&db_path).unwrap();
        std::mem::forget(dir);
        Self { store, db_path }
    }

    pub fn conn(&self) -> Connection {
        self.store.get_connection().unwrap()
    }
}

/// Eine Ordner-Integration (lesen und schreiben), wie sie die Oberflaeche anlegt.
pub fn folder(conn: &Connection, label: &str) -> Integration {
    let mut n = NewIntegration::new(Kind::Folder, label);
    n.config = serde_json::json!({ "path": "C:/Ablage" });
    store::create(conn, &n, 1_000).unwrap()
}

pub fn of_kind(conn: &Connection, kind: Kind, label: &str) -> Integration {
    store::create(conn, &NewIntegration::new(kind, label), 1_000).unwrap()
}

/// Schreibt eine Kalenderquelle so, wie `calendar_source_add` es tut.
pub fn add_calendar_source(conn: &Connection, id: &str, kind: &str, label: &str, now_ms: i64) {
    conn.execute(
        "INSERT INTO calendar_sources (id, kind, label, account_hint, enabled, has_attendee_data,
                                       created_at, updated_at)
         VALUES (?1, ?2, ?3, 'outlook.office365.com', 1, 0, ?4, ?4)",
        rusqlite::params![id, kind, label, now_ms],
    )
    .unwrap();
}
