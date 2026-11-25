//! TF2: eigener Migrationsschritt; beim Merge nach PR #738 einordnen.
pub const POLLS_MIGRATION: &str = r#"
CREATE TABLE meeting_polls (
    id TEXT PRIMARY KEY,
    integration_id TEXT NOT NULL,
    account_key TEXT NOT NULL,
    definition_json TEXT NOT NULL,
    status TEXT NOT NULL CHECK(status IN ('entwurf','offen','entschieden','abgebrochen')),
    created_at INTEGER NOT NULL,
    closed_at INTEGER
);
CREATE INDEX idx_polls_open ON meeting_polls(integration_id, status);
CREATE TABLE meeting_poll_slots (
    poll_id TEXT NOT NULL REFERENCES meeting_polls(id) ON DELETE CASCADE,
    nr INTEGER NOT NULL,
    start INTEGER NOT NULL,
    end INTEGER NOT NULL,
    PRIMARY KEY(poll_id, nr)
);
CREATE TABLE meeting_poll_people (
    poll_id TEXT NOT NULL REFERENCES meeting_polls(id) ON DELETE CASCADE,
    address TEXT NOT NULL,
    code TEXT NOT NULL,
    salt INTEGER NOT NULL,
    conversation_id TEXT,
    conversation_checked_at INTEGER,
    send_state TEXT NOT NULL DEFAULT 'pending',
    sent_at INTEGER,
    PRIMARY KEY(poll_id, address),
    UNIQUE(poll_id, code)
);
CREATE TABLE meeting_poll_answers (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    poll_id TEXT NOT NULL REFERENCES meeting_polls(id) ON DELETE CASCADE,
    message_id TEXT NOT NULL,
    sender TEXT NOT NULL,
    person TEXT,
    received_at INTEGER NOT NULL,
    result_json TEXT NOT NULL,
    invalid_code INTEGER NOT NULL DEFAULT 0,
    confirmed INTEGER NOT NULL DEFAULT 0,
    UNIQUE(poll_id, message_id)
);
CREATE INDEX idx_poll_answers_sender ON meeting_poll_answers(poll_id,sender,invalid_code);
CREATE TABLE meeting_poll_sync (
    integration_id TEXT PRIMARY KEY,
    account_key TEXT NOT NULL,
    delta_token TEXT CHECK(delta_token IS NULL OR delta_token IN ('poll_delta0','poll_delta1')),
    next_at INTEGER NOT NULL DEFAULT 0
);
"#;
