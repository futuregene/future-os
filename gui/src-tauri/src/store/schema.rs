/// Full database schema for the desktop GUI store.
///
/// The app is pre-release, so there is no incremental migration history: this
/// is the single source of truth and is applied idempotently (every statement
/// uses `IF NOT EXISTS`). Change tables/columns here directly.
pub(super) const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS workspaces (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    kind TEXT NOT NULL CHECK (kind IN ('user', 'temporary')),
    path TEXT NOT NULL,
    description TEXT,
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
    model_provider TEXT,
    model_id TEXT,
    thinking_level TEXT,
    agent_session_id TEXT,
    last_message_at INTEGER,
    last_opened_at INTEGER,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    archived_at INTEGER,
    deleted_at INTEGER
);

CREATE TABLE IF NOT EXISTS messages (
    id TEXT PRIMARY KEY,
    thread_id TEXT NOT NULL REFERENCES threads(id),
    run_id TEXT REFERENCES runs(id),
    role TEXT NOT NULL,
    content_type TEXT NOT NULL DEFAULT 'markdown',
    content TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'complete',
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS runs (
    id TEXT PRIMARY KEY,
    thread_id TEXT NOT NULL REFERENCES threads(id),
    trigger_message_id TEXT REFERENCES messages(id),
    status TEXT NOT NULL,
    model_provider TEXT,
    model_id TEXT,
    started_at INTEGER,
    ended_at INTEGER,
    error_message TEXT,
    error_type TEXT,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS run_events (
    id TEXT PRIMARY KEY,
    run_id TEXT NOT NULL REFERENCES runs(id),
    type TEXT NOT NULL,
    payload TEXT,
    sequence INTEGER NOT NULL,
    created_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS tool_calls (
    id TEXT PRIMARY KEY,
    run_id TEXT NOT NULL REFERENCES runs(id),
    name TEXT NOT NULL,
    kind TEXT NOT NULL,
    input TEXT,
    status TEXT NOT NULL,
    started_at INTEGER,
    ended_at INTEGER,
    created_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS tool_outputs (
    id TEXT PRIMARY KEY,
    tool_call_id TEXT NOT NULL REFERENCES tool_calls(id),
    kind TEXT NOT NULL,
    content TEXT,
    created_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS approval_requests (
    id TEXT PRIMARY KEY,
    thread_id TEXT NOT NULL REFERENCES threads(id),
    run_id TEXT REFERENCES runs(id),
    tool_call_id TEXT REFERENCES tool_calls(id),
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
    reviewer TEXT NOT NULL DEFAULT 'user',
    decision_scope TEXT NOT NULL DEFAULT 'once',
    decision_source TEXT NOT NULL DEFAULT 'user',
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS sandbox_config (
    id TEXT PRIMARY KEY,
    workspace_id TEXT REFERENCES workspaces(id),
    mode TEXT NOT NULL DEFAULT 'workspace-write',
    writable_roots TEXT,
    network_access INTEGER NOT NULL DEFAULT 0,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS approval_policy_config (
    id TEXT PRIMARY KEY,
    workspace_id TEXT REFERENCES workspaces(id),
    policy TEXT NOT NULL DEFAULT 'on-request',
    reviewer TEXT NOT NULL DEFAULT 'user',
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS approval_rules (
    id TEXT PRIMARY KEY,
    workspace_id TEXT REFERENCES workspaces(id),
    scope TEXT NOT NULL,
    match_kind TEXT NOT NULL,
    match_value TEXT NOT NULL,
    decision TEXT NOT NULL,
    enabled INTEGER NOT NULL DEFAULT 1,
    created_at INTEGER NOT NULL,
    expires_at INTEGER
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
    tool_call_id TEXT REFERENCES tool_calls(id),
    title TEXT NOT NULL,
    summary TEXT,
    status TEXT NOT NULL,
    files_changed INTEGER NOT NULL DEFAULT 0,
    additions INTEGER NOT NULL DEFAULT 0,
    deletions INTEGER NOT NULL DEFAULT 0,
    -- Shadow review (source_kind = 'run_snapshot') columns; see gui/ER.md §4.10.
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
    -- Shadow review columns; see gui/ER.md §4.10.
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
    type TEXT NOT NULL,
    path TEXT,
    content TEXT,
    content_storage TEXT,
    summary TEXT,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    deleted_at INTEGER
);

CREATE TABLE IF NOT EXISTS research_collections (
    id TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL REFERENCES workspaces(id),
    name TEXT NOT NULL,
    description TEXT,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS research_resources (
    id TEXT PRIMARY KEY,
    collection_id TEXT NOT NULL REFERENCES research_collections(id),
    source_artifact_id TEXT REFERENCES artifacts(id),
    title TEXT NOT NULL,
    type TEXT NOT NULL,
    source_uri TEXT,
    content TEXT,
    content_storage TEXT,
    summary TEXT,
    metadata TEXT,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS data_sources (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    kind TEXT NOT NULL,
    scope TEXT NOT NULL,
    workspace_id TEXT REFERENCES workspaces(id),
    config TEXT,
    readonly INTEGER NOT NULL DEFAULT 1,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    deleted_at INTEGER
);

CREATE TABLE IF NOT EXISTS data_credentials (
    id TEXT PRIMARY KEY,
    data_source_id TEXT NOT NULL REFERENCES data_sources(id),
    credential_ref TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS skills (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    description TEXT,
    kind TEXT NOT NULL,
    version TEXT,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS skill_enablements (
    id TEXT PRIMARY KEY,
    skill_id TEXT NOT NULL REFERENCES skills(id),
    scope TEXT NOT NULL,
    workspace_id TEXT REFERENCES workspaces(id),
    enabled INTEGER NOT NULL DEFAULT 1,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

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

CREATE INDEX IF NOT EXISTS idx_threads_workspace ON threads(workspace_id);
CREATE INDEX IF NOT EXISTS idx_threads_recent ON threads(status, pinned, last_message_at, updated_at);
CREATE INDEX IF NOT EXISTS idx_messages_thread ON messages(thread_id, created_at);
CREATE INDEX IF NOT EXISTS idx_runs_thread ON runs(thread_id, created_at);
CREATE INDEX IF NOT EXISTS idx_reference_targets_scope ON reference_targets(scope, workspace_id, target_type);
CREATE INDEX IF NOT EXISTS idx_review_snapshots_run ON review_snapshots(run_id, phase);
CREATE INDEX IF NOT EXISTS idx_review_snapshots_workspace ON review_snapshots(workspace_id, created_at);
CREATE INDEX IF NOT EXISTS idx_review_changesets_run ON review_changesets(run_id);
-- FK columns used by hot list/join/cleanup queries (B-12).
CREATE INDEX IF NOT EXISTS idx_tool_calls_run ON tool_calls(run_id);
CREATE INDEX IF NOT EXISTS idx_tool_outputs_call ON tool_outputs(tool_call_id);
CREATE INDEX IF NOT EXISTS idx_review_file_changes_changeset ON review_file_changes(changeset_id);
CREATE INDEX IF NOT EXISTS idx_approval_requests_thread ON approval_requests(thread_id);
CREATE INDEX IF NOT EXISTS idx_approval_requests_run_status ON approval_requests(run_id, status);
CREATE INDEX IF NOT EXISTS idx_artifacts_workspace ON artifacts(workspace_id, deleted_at);
"#;

/// Columns added to pre-existing tables after their initial `CREATE`. SQLite's
/// `CREATE TABLE IF NOT EXISTS` will not add columns to a table that already
/// exists, so these `ALTER`s run idempotently (a duplicate-column error is
/// swallowed). Every column here must be nullable or carry a `DEFAULT`.
pub(super) const ADDED_COLUMNS: &[(&str, &str)] = &[
    ("threads", "thinking_level TEXT"),
    (
        "review_changesets",
        "source_kind TEXT NOT NULL DEFAULT 'run_snapshot'",
    ),
    ("review_changesets", "workspace_id TEXT"),
    ("review_changesets", "before_snapshot_id TEXT"),
    ("review_changesets", "after_snapshot_id TEXT"),
    (
        "review_changesets",
        "binary_files INTEGER NOT NULL DEFAULT 0",
    ),
    (
        "review_changesets",
        "omitted_files INTEGER NOT NULL DEFAULT 0",
    ),
    (
        "review_changesets",
        "completeness TEXT NOT NULL DEFAULT 'complete'",
    ),
    (
        "review_changesets",
        "confidence TEXT NOT NULL DEFAULT 'normal'",
    ),
    ("review_changesets", "overlapped INTEGER NOT NULL DEFAULT 0"),
    ("review_changesets", "error_message TEXT"),
    ("review_file_changes", "previous_path TEXT"),
    ("review_file_changes", "binary INTEGER NOT NULL DEFAULT 0"),
    ("review_file_changes", "before_size INTEGER"),
    ("review_file_changes", "after_size INTEGER"),
    ("review_file_changes", "mime TEXT"),
    (
        "review_file_changes",
        "diff_truncated INTEGER NOT NULL DEFAULT 0",
    ),
    ("review_file_changes", "omission_reason TEXT"),
];

/// Indexes that reference columns from `ADDED_COLUMNS`. These must run *after*
/// the `ALTER`s, not inside `SCHEMA`, or they fail with "no such column" on a
/// database created before those columns existed.
pub(super) const ADDED_INDEXES: &[&str] =
    &["CREATE INDEX IF NOT EXISTS idx_review_changesets_thread \
     ON review_changesets(thread_id, source_kind, created_at)"];
