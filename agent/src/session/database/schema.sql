CREATE TABLE IF NOT EXISTS sessions (
    id TEXT PRIMARY KEY NOT NULL,
    revision INTEGER NOT NULL DEFAULT 0,
    created_at_ms INTEGER,
    updated_at_ms INTEGER,
    current_metadata_json TEXT CHECK(current_metadata_json IS NULL OR json_valid(current_metadata_json)),
    title TEXT GENERATED ALWAYS AS (json_extract(current_metadata_json,'$.session_name')) VIRTUAL,
    cwd TEXT GENERATED ALWAYS AS (json_extract(current_metadata_json,'$.cwd')) VIRTUAL,
    model TEXT GENERATED ALWAYS AS (json_extract(current_metadata_json,'$.model')) VIRTUAL,
    thinking_level TEXT GENERATED ALWAYS AS (json_extract(current_metadata_json,'$.thinking_level')) VIRTUAL,
    parent_session_id TEXT GENERATED ALWAYS AS (nullif(json_extract(current_metadata_json,'$.parent_session_id'),'')) VIRTUAL
);
CREATE TABLE IF NOT EXISTS compaction_operations (
    session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    input_key TEXT NOT NULL,
    input_digest TEXT NOT NULL,
    operation_id TEXT NOT NULL,
    state TEXT NOT NULL CHECK(state IN ('started','completed','failed')),
    result_json TEXT CHECK(result_json IS NULL OR json_valid(result_json)),
    PRIMARY KEY(session_id,input_key)
);
CREATE TABLE IF NOT EXISTS fork_operations (
    request_id TEXT PRIMARY KEY NOT NULL,
    request_fingerprint TEXT NOT NULL,
    parent_session_id TEXT NOT NULL,
    child_session_id TEXT NOT NULL UNIQUE REFERENCES sessions(id) ON DELETE CASCADE,
    created_at_ms INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS fork_operations_parent
    ON fork_operations(parent_session_id, created_at_ms);
UPDATE sessions AS child
SET created_at_ms = (
    SELECT operation.created_at_ms
    FROM fork_operations operation
    WHERE operation.child_session_id = child.id
)
WHERE EXISTS (
    SELECT 1
    FROM fork_operations operation
    WHERE operation.child_session_id = child.id
      AND (
          child.created_at_ms IS NULL
          OR child.created_at_ms != operation.created_at_ms
      )
);
CREATE TABLE IF NOT EXISTS entries (
    session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    position INTEGER NOT NULL,
    entry_id TEXT NOT NULL,
    entry_type TEXT NOT NULL,
    role TEXT,
    run_id TEXT,
    timestamp_ms INTEGER,
    metadata_json TEXT NOT NULL CHECK(json_valid(metadata_json)),
    content_json TEXT CHECK(content_json IS NULL OR json_valid(content_json)),
    PRIMARY KEY(session_id, position)
);
CREATE TABLE IF NOT EXISTS message_blocks (
    session_id TEXT NOT NULL,
    entry_position INTEGER NOT NULL,
    ordinal INTEGER NOT NULL CHECK(ordinal>=0),
    kind TEXT,
    text TEXT,
    tool_call_id TEXT,
    tool_name TEXT,
    arguments_json TEXT CHECK(arguments_json IS NULL OR json_valid(arguments_json)),
    is_error INTEGER CHECK(is_error IN (0,1)),
    metadata_json TEXT NOT NULL CHECK(json_valid(metadata_json)),
    PRIMARY KEY(session_id,entry_position,ordinal),
    FOREIGN KEY(session_id,entry_position) REFERENCES entries(session_id,position) ON DELETE CASCADE
);
CREATE INDEX IF NOT EXISTS message_blocks_tool ON message_blocks(session_id,tool_call_id,kind);
CREATE UNIQUE INDEX IF NOT EXISTS entries_identity_unique ON entries(session_id, entry_id);
DROP INDEX IF EXISTS entries_identity;
CREATE INDEX IF NOT EXISTS entries_kind ON entries(session_id, entry_type, position);
CREATE INDEX IF NOT EXISTS entries_run ON entries(session_id,run_id,position);
CREATE TABLE IF NOT EXISTS runs (
    session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    run_id TEXT NOT NULL,
    status TEXT NOT NULL,
    run_sequence INTEGER,
    epoch INTEGER,
    started_at_ms INTEGER,
    completed_at_ms INTEGER,
    error TEXT,
    cache_write_tokens INTEGER,
    input_baseline INTEGER,
    cache_read_baseline INTEGER,
    input_tokens INTEGER,
    cache_read_tokens INTEGER,
    output_tokens INTEGER,
    duration_ms INTEGER,
    PRIMARY KEY(session_id,run_id)
);
CREATE INDEX IF NOT EXISTS runs_status ON runs(session_id,status,run_sequence);
CREATE TABLE IF NOT EXISTS run_events (
    sequence INTEGER PRIMARY KEY,
    session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    run_id TEXT NOT NULL,
    epoch INTEGER NOT NULL,
    idx INTEGER NOT NULL,
    event_id TEXT,
    payload TEXT NOT NULL CHECK(json_valid(payload)),
    UNIQUE(session_id, run_id, idx, epoch)
);
CREATE TABLE IF NOT EXISTS run_snapshots (
    session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    run_id TEXT NOT NULL,
    version INTEGER NOT NULL CHECK(version=1),
    cursor INTEGER NOT NULL CHECK(cursor>=0),
    payload TEXT NOT NULL CHECK(json_valid(payload)),
    pricing_payloads TEXT NOT NULL CHECK(json_valid(pricing_payloads)),
    PRIMARY KEY(session_id,run_id)
);
DROP INDEX IF EXISTS run_snapshots_cleanup;
CREATE TABLE IF NOT EXISTS legacy_imports (
    session_id TEXT PRIMARY KEY NOT NULL,
    status TEXT NOT NULL CHECK(status IN ('imported', 'skipped', 'deleted')),
    fingerprint TEXT NOT NULL,
    error_file TEXT,
    error_line INTEGER,
    error_kind TEXT,
    warnings INTEGER NOT NULL DEFAULT 0
);
CREATE TABLE IF NOT EXISTS storage_meta (
    key TEXT PRIMARY KEY NOT NULL,
    value TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS history_shapes (
    session_id TEXT NOT NULL,
    position INTEGER NOT NULL,
    payload TEXT NOT NULL,
    PRIMARY KEY(session_id,position),
    FOREIGN KEY(session_id,position) REFERENCES entries(session_id,position) ON DELETE CASCADE
);
CREATE TABLE IF NOT EXISTS history_display (
    session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    ordinal INTEGER NOT NULL,
    source_position INTEGER,
    is_user INTEGER NOT NULL,
    payload TEXT NOT NULL,
    PRIMARY KEY(session_id,ordinal)
);
CREATE INDEX IF NOT EXISTS history_users ON history_display(session_id,is_user,ordinal);
CREATE INDEX IF NOT EXISTS history_sources ON history_display(session_id,source_position,ordinal);
CREATE TABLE IF NOT EXISTS history_index_state (
    session_id TEXT PRIMARY KEY NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    projection_version INTEGER NOT NULL
);
CREATE TRIGGER IF NOT EXISTS history_shapes_inserted AFTER INSERT ON history_shapes BEGIN
    DELETE FROM history_index_state WHERE session_id=NEW.session_id;
END;
CREATE TRIGGER IF NOT EXISTS history_shapes_updated AFTER UPDATE ON history_shapes BEGIN
    DELETE FROM history_index_state WHERE session_id=OLD.session_id OR session_id=NEW.session_id;
END;
CREATE TRIGGER IF NOT EXISTS history_shapes_deleted AFTER DELETE ON history_shapes BEGIN
    DELETE FROM history_index_state WHERE session_id=OLD.session_id;
END;
CREATE TRIGGER IF NOT EXISTS history_display_inserted AFTER INSERT ON history_display BEGIN
    DELETE FROM history_index_state WHERE session_id=NEW.session_id;
END;
CREATE TRIGGER IF NOT EXISTS history_display_updated AFTER UPDATE ON history_display BEGIN
    DELETE FROM history_index_state WHERE session_id=OLD.session_id OR session_id=NEW.session_id;
END;
CREATE TRIGGER IF NOT EXISTS history_display_deleted AFTER DELETE ON history_display BEGIN
    DELETE FROM history_index_state WHERE session_id=OLD.session_id;
END;
PRAGMA application_id = 1179997266;

CREATE TABLE IF NOT EXISTS session_tombstones (
    session_id TEXT PRIMARY KEY NOT NULL,
    deleted_at_ms INTEGER NOT NULL
);
INSERT INTO session_tombstones(session_id,deleted_at_ms)
SELECT session_id,0 FROM legacy_imports WHERE status='deleted'
ON CONFLICT DO NOTHING;

CREATE TABLE IF NOT EXISTS queued_run_receipts (
    session_id TEXT NOT NULL,
    run_id TEXT NOT NULL,
    client_request_id TEXT NOT NULL,
    run_sequence INTEGER NOT NULL,
    fingerprint TEXT NOT NULL,
    payload_json TEXT CHECK(payload_json IS NULL OR json_valid(payload_json)),
    user_entry_json TEXT CHECK(user_entry_json IS NULL OR json_valid(user_entry_json)),
    status TEXT NOT NULL CHECK(status IN ('queued','cancel_requested','started','merged','cancelled','interrupted','recovery_failed')),
    PRIMARY KEY(session_id,run_id),
    UNIQUE(session_id,client_request_id)
);
