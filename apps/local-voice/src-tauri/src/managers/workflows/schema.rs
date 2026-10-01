//! Migration der Workflow-Tabellen (B1). Das JSON-Schema der Definition steht in
//! `jsonschema.rs` (aus dem Katalog erzeugt, Abgleichdatei `schema/lva-workflow-1.schema.json`).
//!
//! Die Migration haengt HINTEN an `meetings::store::MIGRATIONS` (Index 8, nach A1 = 5,
//! A3 = 6, U7 = 7); der SQL-Text steht hier, damit der Eintrag in `store.rs` nur aus
//! EINER Zeile besteht (wie bei `queue_store::QUEUE_MIGRATION`). Bestehende Schritte
//! bleiben unveraendert. Parallel angelegte Migrationen anderer Pakete werden beim
//! Zusammenfuehren umnummeriert, nicht hier.
//!
//! Nur CREATE: vorhandene Zeilen anderer Tabellen bleiben unberuehrt. Wie jede
//! Migration laeuft der Schritt in EINER Transaktion und rollt bei Abbruch
//! vollstaendig zurueck (Test `store::tests::an_abort_inside_the_workflow_migration_*`).
//! Fremdschluessel sind nur Dokumentation (die Verbindungen schalten
//! `PRAGMA foreign_keys` nicht ein): `store::delete_workflow` raeumt Laeufe und
//! Schritte ausdruecklich mit ab.

pub const WORKFLOWS_MIGRATION: &str = "
-- Definitionen. `dry_run = 1`: der Ablauf ist noch nicht scharf geschaltet; jeder
-- Lauf plant nur (keine Datei, keine Mail, kein Modellstart).
CREATE TABLE workflows (
  id TEXT PRIMARY KEY,
  name TEXT NOT NULL,
  enabled INTEGER NOT NULL DEFAULT 0,
  dry_run INTEGER NOT NULL DEFAULT 1,
  schema_version INTEGER NOT NULL,
  definition_json TEXT NOT NULL,
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL);

-- Laeufe = die Warteschlange. `UNIQUE (workflow_id, trigger_key)` ist die
-- Idempotenz: derselbe Ausloeser ergibt hoechstens einen Lauf. Die Definition wird
-- beim Einreihen KOPIERT, ein spaeteres Bearbeiten aendert keinen laufenden Lauf.
-- `lease_*` ist der Mietvertrag des Arbeiters (Wiederaufnahme nach Absturz erst nach
-- Ablauf), `next_run_at` der fruehest erlaubte Start (Wiederholung, Warten, schwerer
-- Schritt ohne Platz).
CREATE TABLE workflow_runs (
  id TEXT PRIMARY KEY,
  workflow_id TEXT NOT NULL,
  workflow_name TEXT NOT NULL,
  trigger_key TEXT NOT NULL,
  origin TEXT NOT NULL CHECK (origin IN ('trigger','manual','agent')),
  state TEXT NOT NULL CHECK (state IN ('queued','running','awaiting_approval','done','failed','cancelled')),
  dry_run INTEGER NOT NULL DEFAULT 1,
  definition_json TEXT NOT NULL,
  context_json TEXT NOT NULL,
  next_run_at INTEGER,
  wait_reason TEXT,
  lease_owner TEXT,
  lease_until INTEGER,
  cancel_requested INTEGER NOT NULL DEFAULT 0,
  error TEXT,
  error_code TEXT,
  created_at INTEGER NOT NULL,
  started_at INTEGER,
  ended_at INTEGER,
  updated_at INTEGER NOT NULL,
  UNIQUE (workflow_id, trigger_key));
CREATE INDEX workflow_runs_due ON workflow_runs(state, next_run_at, created_at);
CREATE INDEX workflow_runs_workflow ON workflow_runs(workflow_id, created_at);

-- Laufprotokoll je Schritt und Versuch. Der Zustand `running` VOR dem Baustein und
-- `done` danach ist das Journal, aus dem die Wiederaufnahme liest.
CREATE TABLE workflow_run_steps (
  run_id TEXT NOT NULL,
  step_id TEXT NOT NULL,
  attempt INTEGER NOT NULL,
  ordinal INTEGER NOT NULL,
  action TEXT NOT NULL,
  state TEXT NOT NULL CHECK (state IN ('running','done','failed','retrying','denied','skipped',
    'planned','awaiting_approval','waiting','uncertain','interrupted')),
  error_class TEXT,
  input_json TEXT,
  output_json TEXT,
  error TEXT,
  approval_id TEXT,
  wake_at INTEGER,
  started_at INTEGER,
  ended_at INTEGER,
  PRIMARY KEY (run_id, step_id, attempt));
CREATE INDEX workflow_run_steps_run ON workflow_run_steps(run_id, ordinal, attempt);

-- Ledger fuer Ausloeser mit Dateien oder Eintraegen aus der Welt (Ordner, YouTube
-- `yt:<kanal>:<videoId>`): was gesehen und verarbeitet wurde. Angelegt in B1, damit B3
-- keine eigene Migration braucht; B1 selbst schreibt hier nichts.
CREATE TABLE workflow_file_ledger (
  path_key TEXT PRIMARY KEY,
  size INTEGER,
  mtime INTEGER,
  content_hash TEXT,
  run_id TEXT,
  seen_at INTEGER NOT NULL);
";
