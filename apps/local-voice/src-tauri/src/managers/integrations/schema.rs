//! Migration Index 5 des meetings-Stores (A1): Register, Rechte, Audit,
//! Freigaben und Provenienz (E2: alles in `meetings.db`, gleiche Kette).
//!
//! Nur CREATE plus eine Rueckfuellung aus `calendar_sources`: vorhandene Zeilen
//! anderer Tabellen bleiben unberuehrt. Wie jede Migration laeuft der Schritt in
//! EINER Transaktion und rollt bei Abbruch vollstaendig zurueck.
//!
//! Die Kalenderquellen bleiben in `calendar_sources` (R6); das Register spiegelt
//! sie unter derselben ID. Die Trigger halten den Spiegel in derselben
//! Transaktion wie jede Aenderung an `calendar_sources` aktuell (App, Headless,
//! jeder Schreibweg), `adopt::reconcile` heilt Abweichungen beim Oeffnen. Sie
//! beruehren NUR Eintraege der Kalenderarten (`kind IN ('ics','graph')`): ein
//! Eintrag anderer Art mit zufaellig gleicher Kennung bleibt unangetastet
//! (`store::create` vergibt solche Kennungen ohnehin nicht).
//!
//! Die Migration ist noch nicht ausgeliefert (A1 unveroeffentlicht) und wurde
//! deshalb fuer A1n direkt angepasst, nicht durch einen weiteren Schritt ergaenzt.
//!
//! Fremdschluessel sind nur Dokumentation: die Verbindungen des Stores schalten
//! `PRAGMA foreign_keys` nicht ein, deshalb loescht `store::delete` die Rechte
//! ausdruecklich mit.

pub const INTEGRATIONS_MIGRATION: &str = "
-- Provenienz (Querschnitt fuer A, B, C): je erzeugtem Inhalt ein oder mehrere Eintraege.
CREATE TABLE provenance (
  id TEXT PRIMARY KEY,
  subject_kind TEXT NOT NULL CHECK (subject_kind IN ('transcript','transcript_variant','document',
    'summary','knowledge_note','tts_audio','export','run_output')),
  subject_id TEXT NOT NULL,
  subject_revision INTEGER,
  created_at INTEGER NOT NULL,
  operation TEXT NOT NULL,
  actor_kind TEXT NOT NULL CHECK (actor_kind IN ('user','auto','workflow','agent_external','agent_local')),
  actor_ref TEXT,
  provider TEXT,
  locality TEXT CHECK (locality IS NULL OR locality IN ('local','remote')),
  model_id TEXT, model_label TEXT,
  usage_event_id INTEGER,
  prompt_tokens INTEGER, completion_tokens INTEGER, duration_ms INTEGER,
  sources_json TEXT NOT NULL DEFAULT '[]',
  confidence REAL CHECK (confidence IS NULL OR (confidence >= 0 AND confidence <= 1)),
  params_json TEXT);
CREATE INDEX provenance_subject ON provenance(subject_kind, subject_id, created_at);

-- Register. `config_json` enthaelt keine Geheimnisse (die liegen per DPAPI in secrets/).
CREATE TABLE integrations (
  id TEXT PRIMARY KEY,
  kind TEXT NOT NULL,
  label TEXT NOT NULL,
  enabled INTEGER NOT NULL DEFAULT 1,
  direction TEXT NOT NULL CHECK (direction IN ('read','write','both')),
  config_json TEXT NOT NULL DEFAULT '{}',
  account_hint TEXT,
  data_class TEXT,
  created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL,
  last_ok_at INTEGER, last_error TEXT);
CREATE INDEX integrations_kind ON integrations(kind);

-- Rechte je Integration, Faehigkeit und Aufrufer. Fehlt eine Zeile, gilt die
-- Vorgabe (`grants::default_mode`): aus fuer externe Agenten, fragen fuer
-- Schreibendes, erlaubt fuer Lesendes von Workflow und lokalem Agenten.
CREATE TABLE integration_grants (
  integration_id TEXT NOT NULL REFERENCES integrations(id) ON DELETE CASCADE,
  capability TEXT NOT NULL,
  caller TEXT NOT NULL CHECK (caller IN ('workflow','agent_external','agent_local')),
  mode TEXT NOT NULL CHECK (mode IN ('off','ask','allow')),
  PRIMARY KEY (integration_id, capability, caller));

-- Audit: jede Aktion eines Nicht-Nutzers. Aufbewahrung gedeckelt (`audit::MAX_ROWS`).
CREATE TABLE audit_log (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  ts INTEGER NOT NULL,
  caller TEXT NOT NULL,
  integration_id TEXT,
  capability TEXT,
  target TEXT,
  outcome TEXT NOT NULL CHECK (outcome IN ('ok','denied','error','pending')),
  detail_json TEXT);
CREATE INDEX audit_log_ts ON audit_log(ts);
CREATE INDEX audit_log_integration ON audit_log(integration_id, ts);

-- Freigabe-Anfragen („fragen“). `args_hash` bindet eine Genehmigung an genau
-- diese Ausfuehrung; `used` macht sie einmalig.
CREATE TABLE approvals (
  id TEXT PRIMARY KEY,
  created_at INTEGER NOT NULL,
  caller TEXT NOT NULL,
  integration_id TEXT,
  tool_or_capability TEXT NOT NULL,
  args_preview TEXT,
  args_hash TEXT,
  state TEXT NOT NULL CHECK (state IN ('pending','approved','denied','expired','used')),
  decided_at INTEGER);
CREATE INDEX approvals_state ON approvals(state, created_at);

-- Uebernahme der Kalenderquellen (gleiche ID, Richtung lesend, keine Rechte noetig:
-- es gelten die Vorgaben). Geloeschte Quellen werden nicht uebernommen.
INSERT INTO integrations (id, kind, label, enabled, direction, config_json, account_hint,
                          data_class, created_at, updated_at, last_ok_at, last_error)
  SELECT id, kind, label, enabled, 'read', '{}', account_hint, NULL,
         created_at, updated_at, last_ok_at, last_error
  FROM calendar_sources WHERE deleted_at IS NULL
  ON CONFLICT(id) DO NOTHING;

-- Spiegel halten: jede Aenderung an calendar_sources landet in derselben
-- Transaktion im Register.
CREATE TRIGGER calendar_sources_mirror_ai AFTER INSERT ON calendar_sources
WHEN new.deleted_at IS NULL
BEGIN
  INSERT INTO integrations (id, kind, label, enabled, direction, config_json, account_hint,
                            data_class, created_at, updated_at, last_ok_at, last_error)
  VALUES (new.id, new.kind, new.label, new.enabled, 'read', '{}', new.account_hint, NULL,
          new.created_at, new.updated_at, new.last_ok_at, new.last_error)
  ON CONFLICT(id) DO NOTHING;
END;
CREATE TRIGGER calendar_sources_mirror_au AFTER UPDATE ON calendar_sources
BEGIN
  UPDATE integrations SET label = new.label, enabled = new.enabled,
         account_hint = new.account_hint, last_ok_at = new.last_ok_at,
         last_error = new.last_error, updated_at = new.updated_at
    WHERE id = new.id AND kind IN ('ics','graph') AND new.deleted_at IS NULL;
  INSERT INTO integrations (id, kind, label, enabled, direction, config_json, account_hint,
                            data_class, created_at, updated_at, last_ok_at, last_error)
    SELECT new.id, new.kind, new.label, new.enabled, 'read', '{}', new.account_hint, NULL,
           new.created_at, new.updated_at, new.last_ok_at, new.last_error
    WHERE new.deleted_at IS NULL
    ON CONFLICT(id) DO NOTHING;
  DELETE FROM integration_grants WHERE integration_id = new.id AND new.deleted_at IS NOT NULL
    AND EXISTS (SELECT 1 FROM integrations WHERE id = new.id AND kind IN ('ics','graph'));
  DELETE FROM integrations WHERE id = new.id AND kind IN ('ics','graph')
    AND new.deleted_at IS NOT NULL;
END;
CREATE TRIGGER calendar_sources_mirror_ad AFTER DELETE ON calendar_sources
BEGIN
  DELETE FROM integration_grants WHERE integration_id = old.id
    AND EXISTS (SELECT 1 FROM integrations WHERE id = old.id AND kind IN ('ics','graph'));
  DELETE FROM integrations WHERE id = old.id AND kind IN ('ics','graph');
END;
";
