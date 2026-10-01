//! Das Ledger der Ausloeser mit Dingen aus der Welt (B3): Dateien im Ordner und Videos eines
//! Kanals. Tabelle `workflow_file_ledger` (angelegt in B1, Migration Index 11).
//!
//! **Wozu, wenn die Engine schon `UNIQUE (workflow_id, trigger_key)` hat?** Die Engine
//! bewahrt beendete Laeufe nur begrenzt auf (200 je Ablauf); danach waere derselbe Ausloeser
//! wieder neu. Das Ledger lebt laenger, haelt fest, WAS gesehen wurde (auch ohne Lauf: die
//! Videos des ersten Abrufs, inhaltsgleiche Kopien), und erspart das erneute Lesen und
//! Hashen grosser Dateien nach jedem Neustart.
//!
//! Schluessel (alle je Ablauf, weil jeder Ablauf seine eigenen Laeufe hat):
//!
//! | Schluessel                          | Bedeutung                                              |
//! |-------------------------------------|--------------------------------------------------------|
//! | `p:<ablauf>:<integration>:<pfad>`   | Pfad im Ordner (klein geschrieben): Groesse, Zeit, Hash |
//! | `h:<ablauf>:<hash>`                 | Inhalt (SHA-256) schon verarbeitet                     |
//! | `yt:<ablauf>:<kanal>:<video>`       | Video des Kanals gesehen                               |
//! | `ytinit:<ablauf>:<kanal>`           | der erste Abruf des Kanals ist gelaufen                |
//!
//! Der Schluessel `yt:<kanal>:<video>` des Briefings ist der Schluessel des LAUFS
//! (`trigger_key`); das Ledger stellt ihm die Kennung des Ablaufs voran, damit zwei Ablaeufe
//! auf demselben Kanal je ihr Video bekommen.
//!
//! Aufbewahrung: je Familie gedeckelt (aelteste zuerst), die Anfangsmarken (`ytinit:`) und
//! die Eintraege der letzten Zeit bleiben. Alle Zugriffe sind einzelne Statements.

use rusqlite::{params, Connection, OptionalExtension};

/// Hoechstzahl Eintraege der Familien `p:` und `h:` (je Familie).
pub const FILE_CAP: i64 = 20_000;
/// Hoechstzahl Eintraege der Familie `yt:`.
pub const VIDEO_CAP: i64 = 50_000;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub path_key: String,
    pub size: Option<i64>,
    pub mtime: Option<i64>,
    pub content_hash: Option<String>,
    pub run_id: Option<String>,
    pub seen_at: i64,
}

impl Entry {
    pub fn new(path_key: impl Into<String>, seen_at: i64) -> Self {
        Self {
            path_key: path_key.into(),
            size: None,
            mtime: None,
            content_hash: None,
            run_id: None,
            seen_at,
        }
    }
}

pub fn path_key(workflow_id: &str, integration: &str, rel_path: &str) -> String {
    format!("p:{workflow_id}:{integration}:{}", rel_path.to_lowercase())
}

pub fn content_key(workflow_id: &str, hash: &str) -> String {
    format!("h:{workflow_id}:{hash}")
}

pub fn video_key(workflow_id: &str, channel_id: &str, video_id: &str) -> String {
    format!("yt:{workflow_id}:{channel_id}:{video_id}")
}

pub fn channel_init_key(workflow_id: &str, channel_id: &str) -> String {
    format!("ytinit:{workflow_id}:{channel_id}")
}

pub fn get(conn: &Connection, key: &str) -> rusqlite::Result<Option<Entry>> {
    conn.query_row(
        "SELECT path_key, size, mtime, content_hash, run_id, seen_at
         FROM workflow_file_ledger WHERE path_key = ?1",
        params![key],
        |r| {
            Ok(Entry {
                path_key: r.get(0)?,
                size: r.get(1)?,
                mtime: r.get(2)?,
                content_hash: r.get(3)?,
                run_id: r.get(4)?,
                seen_at: r.get(5)?,
            })
        },
    )
    .optional()
}

pub fn exists(conn: &Connection, key: &str) -> rusqlite::Result<bool> {
    conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM workflow_file_ledger WHERE path_key = ?1)",
        params![key],
        |r| r.get(0),
    )
}

/// Legt den Eintrag an oder ersetzt ihn (ein Statement).
pub fn put(conn: &Connection, e: &Entry) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO workflow_file_ledger (path_key, size, mtime, content_hash, run_id, seen_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)
         ON CONFLICT(path_key) DO UPDATE SET
           size = excluded.size, mtime = excluded.mtime, content_hash = excluded.content_hash,
           run_id = excluded.run_id, seen_at = excluded.seen_at",
        params![
            e.path_key,
            e.size,
            e.mtime,
            e.content_hash,
            e.run_id,
            e.seen_at
        ],
    )?;
    Ok(())
}

/// Wie viele Eintraege beginnen mit `prefix`?
pub fn count(conn: &Connection, prefix: &str) -> rusqlite::Result<i64> {
    conn.query_row(
        "SELECT COUNT(*) FROM workflow_file_ledger
         WHERE substr(path_key, 1, length(?1)) = ?1",
        params![prefix],
        |r| r.get(0),
    )
}

/// Haelt die Familie `family` (Schluessel-Anfang, z. B. `p:`) bei hoechstens `cap` Eintraegen:
/// die aeltesten (`seen_at`) fallen weg. Gibt die Zahl entfernter Zeilen zurueck.
pub fn prune_family(conn: &Connection, family: &str, cap: i64) -> rusqlite::Result<usize> {
    conn.execute(
        "DELETE FROM workflow_file_ledger
         WHERE substr(path_key, 1, length(?1)) = ?1
           AND path_key IN (
             SELECT path_key FROM workflow_file_ledger
             WHERE substr(path_key, 1, length(?1)) = ?1
             ORDER BY seen_at DESC, path_key DESC
             LIMIT -1 OFFSET ?2)",
        params![family, cap.max(0)],
    )
}

#[cfg(test)]
mod tests;
