-- Frozen table/index DDL from d8ba9842:agent/src/session/database.rs.
-- Do not replace with the current schema plus an old version stamp.
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
        PRAGMA application_id = 1179997266;
        PRAGMA user_version = 2;
CREATE UNIQUE INDEX IF NOT EXISTS run_events_custom_id
ON run_events(session_id,event_id) WHERE event_id IS NOT NULL;
