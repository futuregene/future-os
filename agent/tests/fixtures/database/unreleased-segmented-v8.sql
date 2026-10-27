-- Negative admission fixture only: unpublished candidate at 537bcddf.
-- This is not a supported released-schema upgrade source.
-- Schema 8 adds canonical segmented content without touching legacy bodies.
-- An absent session_formats row means legacy format 1. Only new sessions opt in.
CREATE INDEX IF NOT EXISTS sessions_pending_content ON sessions(id) WHERE revision=-1;
CREATE TABLE IF NOT EXISTS session_formats (
    session_id TEXT PRIMARY KEY NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    format INTEGER NOT NULL CHECK(format=2)
);
-- Only explicit transcript rewrites advance this fence; append-only runs do
-- not invalidate an in-flight read of an already captured transcript prefix.
CREATE TABLE IF NOT EXISTS content_history_versions (
    session_id TEXT PRIMARY KEY NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    generation INTEGER NOT NULL CHECK(generation>=0),
    metadata_generation INTEGER NOT NULL DEFAULT 0 CHECK(metadata_generation>=0)
);
CREATE TABLE IF NOT EXISTS content_objects (
    id INTEGER PRIMARY KEY,
    byte_length INTEGER NOT NULL DEFAULT 0 CHECK(byte_length>=0),
    sealed INTEGER NOT NULL DEFAULT 0 CHECK(sealed IN (0,1)),
    sha256 TEXT CHECK(sha256 IS NULL OR length(sha256)=64),
    json_sha256 TEXT CHECK(json_sha256 IS NULL OR length(json_sha256)=64),
    CHECK((sealed=0 AND sha256 IS NULL) OR (sealed=1 AND sha256 IS NOT NULL))
);
CREATE INDEX IF NOT EXISTS content_objects_digest ON content_objects(sha256,byte_length) WHERE sealed=1;
CREATE INDEX IF NOT EXISTS content_objects_json_digest ON content_objects(json_sha256) WHERE sealed=1 AND json_sha256 IS NOT NULL;
CREATE TABLE IF NOT EXISTS content_segments (
    content_id INTEGER NOT NULL REFERENCES content_objects(id) ON DELETE CASCADE,
    byte_offset INTEGER NOT NULL CHECK(byte_offset>=0),
    body TEXT NOT NULL CHECK(length(CAST(body AS BLOB)) BETWEEN 1 AND 16384),
    PRIMARY KEY(content_id,byte_offset)
) WITHOUT ROWID;
-- References carry ownership; segments do not repeat session/run identifiers.
-- Sharing a body never grants a caller access to the other session's records.
CREATE TABLE IF NOT EXISTS content_references (
    session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    owner_kind TEXT NOT NULL CHECK(owner_kind IN ('entry','stream','tool','checkpoint')),
    owner_id TEXT NOT NULL,
    field TEXT NOT NULL,
    encoding TEXT NOT NULL DEFAULT 'text' CHECK(encoding IN ('text','json','shell_summary','tool_args_string_v1')),
    content_id INTEGER NOT NULL REFERENCES content_objects(id),
    byte_offset INTEGER NOT NULL DEFAULT 0 CHECK(byte_offset>=0),
    byte_length INTEGER NOT NULL CHECK(byte_length>=0),
    PRIMARY KEY(session_id,owner_kind,owner_id,field)
) WITHOUT ROWID;
CREATE INDEX IF NOT EXISTS content_references_body ON content_references(content_id);
CREATE TABLE IF NOT EXISTS stream_runs (
    id INTEGER PRIMARY KEY,
    session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    run_id TEXT NOT NULL,
    epoch INTEGER NOT NULL,
    durable_cursor INTEGER NOT NULL DEFAULT -1 CHECK(durable_cursor>=-1),
    UNIQUE(session_id,run_id,epoch)
);
CREATE TABLE IF NOT EXISTS stream_changes (
    id INTEGER PRIMARY KEY,
    stream_id INTEGER NOT NULL REFERENCES stream_runs(id) ON DELETE CASCADE,
    first_cursor INTEGER NOT NULL CHECK(first_cursor>=0),
    last_cursor INTEGER NOT NULL CHECK(last_cursor>=first_cursor),
    event_type TEXT NOT NULL,
    metadata_json TEXT NOT NULL CHECK(json_valid(metadata_json)),
    UNIQUE(stream_id,last_cursor)
);
CREATE INDEX IF NOT EXISTS stream_changes_pricing ON stream_changes(stream_id,id)
    WHERE event_type IN ('usage','model_changed');
CREATE INDEX IF NOT EXISTS stream_changes_entry_kind ON stream_changes(stream_id,json_extract(metadata_json,'$.data.entry_id'),event_type,last_cursor)
    WHERE event_type IN ('message_end','tool_start','tool_input_end');
CREATE INDEX IF NOT EXISTS stream_changes_entry_key ON stream_changes(stream_id,json_extract(metadata_json,'$.data.entry_id'),json_extract(metadata_json,'$.data.content_key'),event_type,last_cursor)
    WHERE event_type='thinking_end';
CREATE INDEX IF NOT EXISTS stream_changes_entry_tool ON stream_changes(stream_id,json_extract(metadata_json,'$.data.entry_id'),coalesce(json_extract(metadata_json,'$.data.tool_id'),''),last_cursor)
    WHERE event_type IN ('tool_start','tool_delta','tool_input_end');
CREATE TABLE IF NOT EXISTS stream_change_content (
    change_id INTEGER NOT NULL REFERENCES stream_changes(id) ON DELETE CASCADE,
    field TEXT NOT NULL,
    encoding TEXT NOT NULL DEFAULT 'text' CHECK(encoding IN ('text','json','shell_summary')),
    content_id INTEGER NOT NULL REFERENCES content_objects(id),
    byte_offset INTEGER NOT NULL CHECK(byte_offset>=0),
    byte_length INTEGER NOT NULL CHECK(byte_length>=0),
    PRIMARY KEY(change_id,field)
) WITHOUT ROWID;
CREATE INDEX IF NOT EXISTS stream_change_content_body ON stream_change_content(content_id);
-- Only explicit reference removal queues collection. No historical sweep.
CREATE TABLE IF NOT EXISTS content_collection (
    content_id INTEGER PRIMARY KEY REFERENCES content_objects(id) ON DELETE CASCADE
);
CREATE TRIGGER IF NOT EXISTS content_reference_removed AFTER DELETE ON content_references BEGIN
    INSERT OR IGNORE INTO content_collection(content_id) VALUES (OLD.content_id);
END;
CREATE TRIGGER IF NOT EXISTS stream_content_removed AFTER DELETE ON stream_change_content BEGIN
    INSERT OR IGNORE INTO content_collection(content_id) VALUES (OLD.content_id);
END;

-- Immutable header identity avoids hydrating a growing body on append retries.
CREATE TABLE IF NOT EXISTS entry_fingerprints (
    session_id TEXT NOT NULL,
    entry_id TEXT NOT NULL,
    digest TEXT NOT NULL CHECK(length(digest)=64),
    PRIMARY KEY(session_id,entry_id),
    FOREIGN KEY(session_id,entry_id) REFERENCES entries(session_id,entry_id) ON DELETE CASCADE
) WITHOUT ROWID;

-- Explicit-operation staging only. A crash discards these unpublished owners,
-- never scans or rewrites historical bodies. Completed resets use the same
-- bounded cleanup; their published entry owners protect every retained byte.
CREATE TABLE IF NOT EXISTS content_operations (
    operation_id TEXT PRIMARY KEY NOT NULL,
    session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    owner_prefix TEXT NOT NULL,
    expected_revision INTEGER NOT NULL CHECK(expected_revision>=0),
    kind TEXT NOT NULL CHECK(kind IN ('reset','metadata','checkpoint','entry'))
);
CREATE INDEX IF NOT EXISTS content_operations_session ON content_operations(session_id);
-- Entry preparation can finish before its transcript/acceptance transaction.
-- Exact owners, rather than a historical scan, select unpublished crash tails.
CREATE TABLE IF NOT EXISTS content_entry_stages (
    operation_id TEXT PRIMARY KEY NOT NULL REFERENCES content_operations(operation_id) ON DELETE CASCADE,
    session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    entry_id TEXT NOT NULL,
    run_id TEXT
);
CREATE INDEX IF NOT EXISTS content_entry_stages_owner ON content_entry_stages(session_id,entry_id,operation_id);
-- Logical retirement hides reset/pruned streams atomically. The explicit
-- queue is drained in bounded idle slices rather than a large cascade delete.
CREATE TABLE IF NOT EXISTS retired_streams (
    stream_id INTEGER PRIMARY KEY REFERENCES stream_runs(id) ON DELETE CASCADE
);
-- Legacy receipts never acquire canonical generation ownership.
CREATE TABLE IF NOT EXISTS checkpoint_generations (
    session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    input_key TEXT NOT NULL,
    history_generation INTEGER NOT NULL CHECK(history_generation>=0),
    PRIMARY KEY(session_id,input_key),
    FOREIGN KEY(session_id,input_key) REFERENCES compaction_operations(session_id,input_key) ON DELETE CASCADE
) WITHOUT ROWID;
CREATE INDEX IF NOT EXISTS checkpoint_generations_retirement ON checkpoint_generations(session_id,history_generation,input_key);
CREATE TABLE IF NOT EXISTS retired_checkpoint_generations (
    session_id TEXT PRIMARY KEY NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    through_generation INTEGER NOT NULL CHECK(through_generation>=0)
);
