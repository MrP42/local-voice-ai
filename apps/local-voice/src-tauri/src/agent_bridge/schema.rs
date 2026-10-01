//! Migration Index 8 des meetings-Stores (A7): Zugaenge der Agentenbruecke, ihre
//! Werkzeugrechte und die Zuordnung offener Freigaben zu einem Zugang.
//!
//! Nur CREATE: vorhandene Zeilen aller anderen Tabellen bleiben unberuehrt, die
//! neuen Tabellen sind leer. Wie jede Migration laeuft der Schritt in EINER
//! Transaktion und rollt bei Abbruch vollstaendig zurueck. Der SQL-Text steht hier
//! (wie bei U7 in `queue_store.rs`), damit der Schritt in `store.rs` nur aus einer
//! Zeile besteht und beim Zusammenfuehren mit anderen Zweigen nur diese Zeile
//! umnummeriert werden muss.
//!
//! Das gebuendelte SQLite erzwingt Fremdschluessel (Vorgabe `SQLITE_DEFAULT_FOREIGN_KEYS=1`):
//! `agent_tool_grants` haengt per `ON DELETE CASCADE` an `agent_clients`. `clients::delete`
//! raeumt die abhaengigen Zeilen trotzdem ausdruecklich mit ab (gleiches Ergebnis, auch auf
//! einer Verbindung ohne Fremdschluessel).
//!
//! - `agent_clients`: je Zugang eine Zeile. Das Token steht NIE hier, nur sein
//!   SHA-256 (`token_hash`); `revoked_at` macht es unbrauchbar, die Zeile bleibt,
//!   damit ein zurueckgezogenes Token als „zurueckgezogen“ erkannt und im Audit so
//!   benannt wird. `integration_id` ist die Integration der Art `agent`, ueber die
//!   die Rechte laufen (Obergrenze); Standard ist `agents`.
//! - `agent_tool_grants`: Recht des Zugangs je Werkzeug (`off`/`ask`/`allow`). Fehlt die
//!   Zeile, gilt „aus“.
//! - `agent_approvals`: welche Freigabe zu welchem Zugang und Werkzeug gehoert, damit
//!   ein Zugang nur den Stand seiner eigenen Freigaben erfaehrt.

/// Stelle des Schritts in `MIGRATIONS` (nach A1 = 5, A3 = 6, U7 = 7). Beim
/// Zusammenfuehren mit Zweigen, die ebenfalls Schritte anhaengen, zusammen mit der
/// Zeile in `store.rs` anpassen; die Tests richten sich danach.
pub const MIGRATION_INDEX: usize = 8;

pub const AGENT_BRIDGE_MIGRATION: &str = "
CREATE TABLE agent_clients (
  id TEXT PRIMARY KEY,
  label TEXT NOT NULL,
  integration_id TEXT NOT NULL,
  token_hash TEXT NOT NULL UNIQUE,
  created_at INTEGER NOT NULL,
  last_used_at INTEGER,
  revoked_at INTEGER);

CREATE TABLE agent_tool_grants (
  client_id TEXT NOT NULL REFERENCES agent_clients(id) ON DELETE CASCADE,
  tool TEXT NOT NULL,
  mode TEXT NOT NULL CHECK (mode IN ('off','ask','allow')),
  PRIMARY KEY (client_id, tool));

CREATE TABLE agent_approvals (
  approval_id TEXT PRIMARY KEY,
  client_id TEXT NOT NULL,
  tool TEXT NOT NULL,
  created_at INTEGER NOT NULL);
CREATE INDEX agent_approvals_client ON agent_approvals(client_id, created_at);
";

#[cfg(test)]
mod tests;
