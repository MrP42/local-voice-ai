CREATE TABLE audiobook_control (
    meeting_id TEXT PRIMARY KEY,
    desired_state TEXT NOT NULL DEFAULT 'running' CHECK (desired_state IN ('running', 'paused', 'cancelled')),
    keep_partial INTEGER NOT NULL DEFAULT 1,
    generation INTEGER NOT NULL DEFAULT 0,
    next_sample INTEGER CHECK (next_sample >= 0)
);
CREATE TABLE audiobook_chunk (
    meeting_id TEXT NOT NULL,
    generation INTEGER NOT NULL,
    start_sample INTEGER NOT NULL,
    end_sample INTEGER NOT NULL CHECK (end_sample > start_sample),
    PRIMARY KEY (meeting_id, generation, start_sample)
);
INSERT INTO audiobook_control (meeting_id)
    SELECT meeting_id FROM import_part WHERE kind = 'audiobook';
CREATE TRIGGER audiobook_part_insert AFTER INSERT ON import_part
    WHEN NEW.kind = 'audiobook'
    BEGIN INSERT OR IGNORE INTO audiobook_control (meeting_id) VALUES (NEW.meeting_id); END;
CREATE TRIGGER audiobook_part_delete AFTER DELETE ON import_part
    BEGIN
        DELETE FROM audiobook_chunk WHERE meeting_id = OLD.meeting_id;
        DELETE FROM audiobook_control WHERE meeting_id = OLD.meeting_id;
    END;
