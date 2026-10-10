-- Fresh Desktop schema from release tag v1.2.1, kept immutable for upgrade coverage.
CREATE TABLE IF NOT EXISTS workspaces (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    kind TEXT NOT NULL CHECK (kind IN ('user', 'temporary')),
    path TEXT NOT NULL,
    description TEXT,
    pinned INTEGER NOT NULL DEFAULT 0,
    cleanup_status TEXT NOT NULL DEFAULT 'active',
    cleanup_requested_at INTEGER,
    cleaned_at INTEGER,
    last_opened_at INTEGER,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    deleted_at INTEGER
);

CREATE TABLE IF NOT EXISTS threads (
    id TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL REFERENCES workspaces(id),
    mode TEXT NOT NULL CHECK (mode IN ('chat', 'workspace')),
    title TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'active',
    pinned INTEGER NOT NULL DEFAULT 0,
    readonly INTEGER NOT NULL DEFAULT 0,
    -- model_provider, model_id, thinking_level removed — now from agent get_state
    agent_session_id TEXT,
    parent_session_id TEXT,
    -- Stable owner of shared attachment originals/thumbnails. Forks inherit
    -- this root so deleting an ancestor cannot invalidate child history.
    asset_root_id TEXT,
    last_message_at INTEGER,
    last_opened_at INTEGER,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    archived_at INTEGER,
    deleted_at INTEGER
);

-- messages, run_events, tool_calls, tool_outputs — removed.
-- Message history is now read from agent session JSONL.

CREATE TABLE IF NOT EXISTS runs (
    id TEXT PRIMARY KEY,
    thread_id TEXT NOT NULL REFERENCES threads(id),
    trigger_message_id TEXT,
    status TEXT NOT NULL,
    model_provider TEXT,
    model_id TEXT,
    started_at INTEGER,
    ended_at INTEGER,
    error_message TEXT,
    error_type TEXT,
    archived_at INTEGER,
    remote_accepted_at INTEGER,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

-- Applied database migrations. Fresh installs record every migration after the
-- complete schema is created; upgrades apply only the migrations they lack.
CREATE TABLE IF NOT EXISTS schema_migrations (
    version TEXT PRIMARY KEY,
    applied_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS approval_requests (
    id TEXT PRIMARY KEY,
    thread_id TEXT NOT NULL REFERENCES threads(id),
    run_id TEXT REFERENCES runs(id),
    tool_call_id TEXT,
    kind TEXT NOT NULL,
    status TEXT NOT NULL,
    title TEXT NOT NULL,
    summary TEXT,
    risk_level TEXT,
    requested_action TEXT,
    decision_note TEXT,
    decided_at INTEGER,
    action_category TEXT,
    action_payload TEXT,
    sandbox_boundary TEXT,
    save_suggestion TEXT,
    reviewer TEXT NOT NULL DEFAULT 'user',
    decision_scope TEXT NOT NULL DEFAULT 'once',
    decision_source TEXT NOT NULL DEFAULT 'user',
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

-- Snapshots taken before/after a Run for the shadow review pipeline.
CREATE TABLE IF NOT EXISTS review_snapshots (
    id TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL REFERENCES workspaces(id),
    thread_id TEXT NOT NULL REFERENCES threads(id),
    run_id TEXT NOT NULL REFERENCES runs(id),
    phase TEXT NOT NULL CHECK (phase IN ('before', 'after')),
    commit_id TEXT,
    tree_id TEXT,
    status TEXT NOT NULL,
    file_count INTEGER NOT NULL DEFAULT 0,
    total_bytes INTEGER NOT NULL DEFAULT 0,
    ignored_count INTEGER NOT NULL DEFAULT 0,
    omitted_count INTEGER NOT NULL DEFAULT 0,
    error_message TEXT,
    created_at INTEGER NOT NULL,
    UNIQUE(run_id, phase)
);

CREATE TABLE IF NOT EXISTS review_changesets (
    id TEXT PRIMARY KEY,
    thread_id TEXT NOT NULL REFERENCES threads(id),
    run_id TEXT REFERENCES runs(id),
    tool_call_id TEXT,
    title TEXT NOT NULL,
    summary TEXT,
    status TEXT NOT NULL,
    files_changed INTEGER NOT NULL DEFAULT 0,
    additions INTEGER NOT NULL DEFAULT 0,
    deletions INTEGER NOT NULL DEFAULT 0,
    -- Shadow review (source_kind = 'run_snapshot') columns; see desktop/ER.md §4.10.
    source_kind TEXT NOT NULL DEFAULT 'run_snapshot',
    workspace_id TEXT REFERENCES workspaces(id),
    before_snapshot_id TEXT REFERENCES review_snapshots(id),
    after_snapshot_id TEXT REFERENCES review_snapshots(id),
    binary_files INTEGER NOT NULL DEFAULT 0,
    omitted_files INTEGER NOT NULL DEFAULT 0,
    completeness TEXT NOT NULL DEFAULT 'complete',
    confidence TEXT NOT NULL DEFAULT 'normal',
    overlapped INTEGER NOT NULL DEFAULT 0,
    error_message TEXT,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS review_file_changes (
    id TEXT PRIMARY KEY,
    changeset_id TEXT NOT NULL REFERENCES review_changesets(id),
    target_type TEXT NOT NULL,
    target_id TEXT,
    path TEXT,
    change_type TEXT NOT NULL,
    before_ref TEXT,
    after_ref TEXT,
    diff TEXT,
    summary TEXT,
    additions INTEGER NOT NULL DEFAULT 0,
    deletions INTEGER NOT NULL DEFAULT 0,
    -- Shadow review columns; see desktop/ER.md §4.10.
    previous_path TEXT,
    binary INTEGER NOT NULL DEFAULT 0,
    before_size INTEGER,
    after_size INTEGER,
    mime TEXT,
    diff_truncated INTEGER NOT NULL DEFAULT 0,
    omission_reason TEXT,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS artifacts (
    id TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL REFERENCES workspaces(id),
    thread_id TEXT REFERENCES threads(id),
    run_id TEXT REFERENCES runs(id),
    title TEXT NOT NULL,
    artifact_type TEXT NOT NULL,
    path TEXT,
    content TEXT,
    content_storage TEXT,
    summary TEXT,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    deleted_at INTEGER
);

-- NOTE: Research (`research_collections`, `research_resources`) was removed
-- before the first release. No tables are created, and `apply_schema` drops
-- any pre-existing research tables via `DROPPED_TABLES`.

-- NOTE: `data_sources`, `data_credentials`, `skills`, and `skill_enablements`
-- were removed on 2026-07-07. They were created by an early schema but never had
-- any CRUD code: the Data feature was abandoned and Skills went to a
-- platform-catalogue + filesystem model (see `crate::skills`), not the DB.
-- `apply_schema` drops them from pre-existing databases via `DROPPED_TABLES`.

CREATE TABLE IF NOT EXISTS workspace_files (
    id TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL REFERENCES workspaces(id),
    path TEXT NOT NULL,
    name TEXT NOT NULL,
    mime_type TEXT,
    size INTEGER,
    last_seen_at INTEGER,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS reference_targets (
    id TEXT PRIMARY KEY,
    target_type TEXT NOT NULL,
    target_id TEXT NOT NULL,
    scope TEXT NOT NULL,
    workspace_id TEXT REFERENCES workspaces(id),
    title TEXT NOT NULL,
    subtitle TEXT,
    search_text TEXT,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS object_references (
    id TEXT PRIMARY KEY,
    source_type TEXT NOT NULL,
    source_id TEXT NOT NULL,
    reference_target_id TEXT NOT NULL REFERENCES reference_targets(id),
    created_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS app_settings (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL,
    updated_at INTEGER NOT NULL
);

-- A local tombstone/outbox: GUI deletion is immediate, while delivery of the
-- Agent delete command is retried after a sidecar/network outage.  The Agent
-- operation is idempotent, so retry is safe.
CREATE TABLE IF NOT EXISTS agent_delete_outbox (
    session_id TEXT PRIMARY KEY,
    requested_at INTEGER NOT NULL,
    attempts INTEGER NOT NULL DEFAULT 0,
    last_error TEXT
);

-- One row per skill recommendation actually shown to the user. The client owns
-- the trigger rules, so their state lives here: `day` answers "how many
-- recommendations today", `skill_id` answers "has this skill been shown today"
-- and `message_hash` answers "has this message already produced one". Calls
-- that recommend nothing leave no row — the daily budget counts
-- recommendations, not calls (see `store/skill_reco.rs`).
CREATE TABLE IF NOT EXISTS skill_reco_events (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    day TEXT NOT NULL,
    skill_id TEXT NOT NULL,
    message_hash TEXT NOT NULL,
    created_at INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_threads_workspace ON threads(workspace_id);
CREATE INDEX IF NOT EXISTS idx_threads_recent ON threads(status, pinned, last_message_at, updated_at);
-- idx_threads_agent_session_unique is applied by a versioned migration after
-- duplicate legacy bindings are detached; creating it here would make SCHEMA
-- fail before that repair can run on an upgraded database.
-- idx_messages_thread removed with messages table
CREATE INDEX IF NOT EXISTS idx_runs_thread ON runs(thread_id, created_at);
CREATE INDEX IF NOT EXISTS idx_runs_trigger_message ON runs(trigger_message_id);
CREATE INDEX IF NOT EXISTS idx_reference_targets_scope ON reference_targets(scope, workspace_id, target_type);
CREATE INDEX IF NOT EXISTS idx_review_snapshots_run ON review_snapshots(run_id, phase);
CREATE INDEX IF NOT EXISTS idx_review_snapshots_workspace ON review_snapshots(workspace_id, created_at);
CREATE INDEX IF NOT EXISTS idx_review_changesets_run ON review_changesets(run_id);
-- FK columns used by hot list/join/cleanup queries (B-12).
-- idx_tool_calls_run removed with tool_calls table
-- idx_tool_outputs_call removed with tool_outputs table
CREATE INDEX IF NOT EXISTS idx_review_file_changes_changeset ON review_file_changes(changeset_id);
CREATE INDEX IF NOT EXISTS idx_approval_requests_thread ON approval_requests(thread_id);
CREATE INDEX IF NOT EXISTS idx_approval_requests_run_status ON approval_requests(run_id, status);
CREATE INDEX IF NOT EXISTS idx_artifacts_workspace ON artifacts(workspace_id, deleted_at);
-- Every read is "today's rows", so index the day the queries all filter on.
CREATE INDEX IF NOT EXISTS idx_skill_reco_events_day ON skill_reco_events(day);
