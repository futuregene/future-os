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
    owner_kind TEXT NOT NULL CHECK(owner_kind IN ('entry','tool','checkpoint')),
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
-- Draft layout identity: old schema-8 candidates are rejected, never converted.
CREATE TABLE IF NOT EXISTS stream_layout (
    singleton INTEGER PRIMARY KEY CHECK(singleton=1),
    layout TEXT NOT NULL CHECK(layout='compact-owned-stream-v2')
);
INSERT OR IGNORE INTO stream_layout VALUES(1,'compact-owned-stream-v2');
-- Dictionary values are exact strings, including empty/NUL and Unicode.
CREATE TABLE IF NOT EXISTS stream_symbols (
    id INTEGER PRIMARY KEY,
    stream_id INTEGER NOT NULL REFERENCES stream_runs(id) ON DELETE CASCADE,
    value BLOB NOT NULL CHECK((typeof(value)='blob' AND length(value)=16) OR (typeof(value)='text' AND length(CAST(value AS BLOB))<=16384)),
    UNIQUE(stream_id,value)
);
CREATE TRIGGER IF NOT EXISTS stream_symbol_immutable BEFORE UPDATE ON stream_symbols BEGIN
    SELECT RAISE(ABORT,'stream symbols are immutable');
END;
-- Staging ownership shares the same short key dictionary as semantic changes.
-- No encoded [run,epoch,key] string is repeated in ownership or reverse indices.
CREATE TABLE IF NOT EXISTS stream_body_owners (
    stream_id INTEGER NOT NULL REFERENCES stream_runs(id) ON DELETE CASCADE,
    key_id INTEGER NOT NULL,
    content_id INTEGER NOT NULL REFERENCES content_objects(id),
    byte_offset INTEGER NOT NULL CHECK(byte_offset>=0),
    byte_length INTEGER NOT NULL CHECK(byte_length>=0),
    encoding TEXT NOT NULL CHECK(encoding IN ('text','json','shell_summary','tool_args_string_v1')),
    PRIMARY KEY(stream_id,key_id),
    FOREIGN KEY(key_id) REFERENCES stream_symbols(id)
) WITHOUT ROWID;
CREATE TRIGGER IF NOT EXISTS stream_body_owner_scope_insert BEFORE INSERT ON stream_body_owners BEGIN
    SELECT RAISE(ABORT,'stream body key belongs to another stream') WHERE EXISTS
        (SELECT 1 FROM stream_symbols WHERE id=NEW.key_id AND stream_id!=NEW.stream_id);
END;
CREATE TRIGGER IF NOT EXISTS stream_body_owner_scope_update BEFORE UPDATE OF stream_id,key_id ON stream_body_owners BEGIN
    SELECT RAISE(ABORT,'stream body key belongs to another stream') WHERE EXISTS
        (SELECT 1 FROM stream_symbols WHERE id=NEW.key_id AND stream_id!=NEW.stream_id);
END;
CREATE INDEX IF NOT EXISTS stream_body_owners_content ON stream_body_owners(content_id);
-- Teardown looks symbol references up one parent row at a time (deleting a
-- session cascades `stream_runs` → `stream_symbols` → every child key), and only
-- an index whose leading column is that key makes it a lookup. Without these
-- seven SQLite scans the whole child table once per deleted symbol: measured
-- 2026-10-29, deleting one 30k-change conversation spent 113 s of CPU here,
-- inside the store's single ordered writer, which stalled every other session's
-- writes for 71 s. This is the body-owner side; `stream_contexts` below has the
-- other six.
CREATE INDEX IF NOT EXISTS stream_body_owners_key ON stream_body_owners(key_id);
CREATE TRIGGER IF NOT EXISTS stream_body_owner_removed AFTER DELETE ON stream_body_owners BEGIN
    INSERT OR IGNORE INTO content_collection(content_id) VALUES (OLD.content_id);
END;
-- Descriptor is [stable fields, direct field, direct encoding, symbol map, kind].
-- Digest narrows lookup; the complete descriptor is always compared on reuse.
CREATE TABLE IF NOT EXISTS stream_contexts (
    id INTEGER PRIMARY KEY,
    stream_id INTEGER NOT NULL REFERENCES stream_runs(id) ON DELETE CASCADE,
    digest BLOB NOT NULL CHECK(length(digest)=32),
    descriptor_json TEXT NOT NULL CHECK(json_valid(descriptor_json) AND length(CAST(descriptor_json AS BLOB))<=16384),
    symbol_bytes INTEGER NOT NULL CHECK(symbol_bytes BETWEEN 0 AND 16384),
    event_kind INTEGER GENERATED ALWAYS AS (json_extract(descriptor_json,'$[4]')) VIRTUAL NOT NULL CHECK(event_kind BETWEEN 0 AND 18),
    entry_id INTEGER GENERATED ALWAYS AS (json_extract(descriptor_json,'$[3].entry_id')) VIRTUAL,
    tool_id INTEGER GENERATED ALWAYS AS (json_extract(descriptor_json,'$[3].tool_id')) VIRTUAL,
    content_key INTEGER GENERATED ALWAYS AS (json_extract(descriptor_json,'$[3].content_key')) VIRTUAL,
    block_key INTEGER GENERATED ALWAYS AS (json_extract(descriptor_json,'$[3].block_id')) VIRTUAL,
    tool_name_key INTEGER GENERATED ALWAYS AS (json_extract(descriptor_json,'$[3].tool_name')) VIRTUAL,
    tool_type_key INTEGER GENERATED ALWAYS AS (json_extract(descriptor_json,'$[3].tool_type')) VIRTUAL,
    tc_index INTEGER GENERATED ALWAYS AS (json_extract(descriptor_json,'$[0].data.tc_index')) VIRTUAL,
    direct_field TEXT GENERATED ALWAYS AS (json_extract(descriptor_json,'$[1]')) VIRTUAL,
    direct_encoding TEXT GENERATED ALWAYS AS (json_extract(descriptor_json,'$[2]')) VIRTUAL,
    FOREIGN KEY(entry_id) REFERENCES stream_symbols(id),
    FOREIGN KEY(tool_id) REFERENCES stream_symbols(id),
    FOREIGN KEY(content_key) REFERENCES stream_symbols(id),
    FOREIGN KEY(block_key) REFERENCES stream_symbols(id),
    FOREIGN KEY(tool_name_key) REFERENCES stream_symbols(id),
    FOREIGN KEY(tool_type_key) REFERENCES stream_symbols(id)
);
-- Integer primary keys already ensure global uniqueness. Scoped checks use
-- primary-key reads rather than redundant composite unique write indices.
CREATE TRIGGER IF NOT EXISTS stream_context_symbol_scope BEFORE INSERT ON stream_contexts BEGIN
    SELECT RAISE(ABORT,'context symbol belongs to another stream') WHERE EXISTS
        (SELECT 1 FROM stream_symbols WHERE id IN
            (NEW.entry_id,NEW.tool_id,NEW.content_key,NEW.block_key,NEW.tool_name_key,NEW.tool_type_key)
            AND stream_id!=NEW.stream_id);
END;
CREATE TRIGGER IF NOT EXISTS stream_context_immutable BEFORE UPDATE ON stream_contexts BEGIN
    SELECT RAISE(ABORT,'stream contexts are immutable');
END;
CREATE INDEX IF NOT EXISTS stream_contexts_digest ON stream_contexts(stream_id,digest);
-- The six symbol keys: the other half of the teardown indexes above. The scoped
-- indexes below lead with `stream_id`, and three of them are partial on
-- `event_kind`, so none of them can serve a lookup by symbol alone.
-- `stream_contexts` is append-only (see the immutability trigger above), so each
-- of these costs one insert and never an update.
CREATE INDEX IF NOT EXISTS stream_contexts_entry_key ON stream_contexts(entry_id);
CREATE INDEX IF NOT EXISTS stream_contexts_tool_key ON stream_contexts(tool_id);
CREATE INDEX IF NOT EXISTS stream_contexts_content_key ON stream_contexts(content_key);
CREATE INDEX IF NOT EXISTS stream_contexts_block_key ON stream_contexts(block_key);
CREATE INDEX IF NOT EXISTS stream_contexts_tool_name_key ON stream_contexts(tool_name_key);
CREATE INDEX IF NOT EXISTS stream_contexts_tool_type_key ON stream_contexts(tool_type_key);
CREATE INDEX IF NOT EXISTS stream_contexts_entry ON stream_contexts(stream_id,entry_id,id) WHERE event_kind IN (4,11,13);
CREATE INDEX IF NOT EXISTS stream_contexts_tool ON stream_contexts(stream_id,entry_id,tool_id,id) WHERE event_kind IN (11,12,13);
CREATE INDEX IF NOT EXISTS stream_contexts_key ON stream_contexts(stream_id,entry_id,content_key,id) WHERE event_kind=10;
CREATE TABLE IF NOT EXISTS stream_changes (
    id INTEGER PRIMARY KEY,
    stream_id INTEGER NOT NULL REFERENCES stream_runs(id) ON DELETE CASCADE,
    context_id INTEGER NOT NULL,
    first_cursor INTEGER NOT NULL CHECK(first_cursor>=0),
    last_cursor INTEGER NOT NULL CHECK(last_cursor>=first_cursor),
    event_kind INTEGER NOT NULL CHECK(event_kind BETWEEN 0 AND 18),
    unknown_kind TEXT CHECK((event_kind=0 AND unknown_kind IS NOT NULL) OR (event_kind!=0 AND unknown_kind IS NULL)),
    event_type TEXT GENERATED ALWAYS AS (CASE event_kind WHEN 1 THEN 'agent_start' WHEN 2 THEN 'agent_end' WHEN 3 THEN 'message_start' WHEN 4 THEN 'message_end' WHEN 5 THEN 'text_start' WHEN 6 THEN 'text_chunk' WHEN 7 THEN 'text_end' WHEN 8 THEN 'thinking_start' WHEN 9 THEN 'thinking_delta' WHEN 10 THEN 'thinking_end' WHEN 11 THEN 'tool_start' WHEN 12 THEN 'tool_delta' WHEN 13 THEN 'tool_input_end' WHEN 14 THEN 'tool_end' WHEN 15 THEN 'tool_result' WHEN 16 THEN 'usage' WHEN 17 THEN 'model_changed' WHEN 18 THEN 'tool_sandboxed' ELSE unknown_kind END) VIRTUAL,
    flags INTEGER NOT NULL CHECK(flags BETWEEN 0 AND 7),
    metadata_json TEXT NOT NULL CHECK(json_valid(metadata_json) AND length(CAST(metadata_json AS BLOB))<=16384),
    content_id INTEGER REFERENCES content_objects(id),
    byte_offset INTEGER CHECK(byte_offset>=0),
    byte_length INTEGER CHECK(byte_length>=0),
    CHECK((content_id IS NULL AND byte_offset IS NULL AND byte_length IS NULL AND flags=0)
       OR (content_id IS NOT NULL AND byte_offset IS NOT NULL AND byte_length IS NOT NULL)),
    FOREIGN KEY(context_id) REFERENCES stream_contexts(id) ON DELETE CASCADE,
    UNIQUE(stream_id,last_cursor)
);
CREATE TRIGGER IF NOT EXISTS stream_change_scope_insert BEFORE INSERT ON stream_changes BEGIN
    SELECT RAISE(ABORT,'change context scope or kind mismatch') WHERE EXISTS
        (SELECT 1 FROM stream_contexts WHERE id=NEW.context_id AND (stream_id!=NEW.stream_id OR event_kind!=NEW.event_kind));
END;
CREATE TRIGGER IF NOT EXISTS stream_change_scope_update BEFORE UPDATE OF context_id,stream_id,event_kind ON stream_changes BEGIN
    SELECT RAISE(ABORT,'change context scope or kind mismatch') WHERE EXISTS
        (SELECT 1 FROM stream_contexts WHERE id=NEW.context_id AND (stream_id!=NEW.stream_id OR event_kind!=NEW.event_kind));
END;
CREATE INDEX IF NOT EXISTS stream_changes_context ON stream_changes(context_id,event_kind,last_cursor);
CREATE INDEX IF NOT EXISTS stream_changes_body ON stream_changes(content_id) WHERE content_id IS NOT NULL;
CREATE INDEX IF NOT EXISTS stream_changes_pricing ON stream_changes(stream_id,id)
    WHERE event_type IN ('usage','model_changed');
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
-- One authority for range lookup, recovery and collection, covering the fast
-- path and uncommon multi-field facts without duplicating persisted ranges.
CREATE VIEW IF NOT EXISTS stream_content_ranges AS
    SELECT c.id AS change_id,x.direct_field AS field,x.direct_encoding AS encoding,
           c.content_id,c.byte_offset,c.byte_length
      FROM stream_changes c JOIN stream_contexts x ON x.id=c.context_id AND x.stream_id=c.stream_id
      WHERE c.content_id IS NOT NULL
    UNION ALL SELECT change_id,field,encoding,content_id,byte_offset,byte_length FROM stream_change_content;
CREATE TRIGGER IF NOT EXISTS stream_direct_removed AFTER DELETE ON stream_changes
WHEN OLD.content_id IS NOT NULL BEGIN
    INSERT OR IGNORE INTO content_collection(content_id) VALUES (OLD.content_id);
END;
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
