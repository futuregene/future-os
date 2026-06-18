pub(super) const INITIAL_MIGRATION: &str = "001_initial_schema";
pub(super) const ADD_ERROR_TYPE_MIGRATION: &str = "002_add_error_type";
pub(super) const APPROVAL_MODEL_V2_MIGRATION: &str = "003_approval_model_v2";

pub(super) const MIGRATIONS: &[(&str, &str)] = &[
    (INITIAL_MIGRATION, INITIAL_SCHEMA),
    (ADD_ERROR_TYPE_MIGRATION, ADD_ERROR_TYPE_SQL),
    (APPROVAL_MODEL_V2_MIGRATION, APPROVAL_MODEL_V2_SQL),
];

/// Adds error_type column to runs table for structured error classification.
/// Values: 'stream_disconnected', 'command_failed', 'model_failed',
///         'abort_requested', 'timeout', 'unknown'.
/// NULL is allowed for runs that ended successfully or were created before
/// this migration.
pub(super) const ADD_ERROR_TYPE_SQL: &str = r#"
ALTER TABLE runs ADD COLUMN error_type TEXT;
"#;

/// P2 Approval model upgrade.
///
/// Adds structured action payload, sandbox boundary, reviewer and decision
/// scope/source columns to `approval_requests`. Also creates placeholder
/// tables for sandbox configuration, approval policy configuration and
/// approval rules. The placeholder tables are not yet read or written by
/// the agent or GUI; they reserve schema space for future sandbox
/// enforcement, automatic approval policies and rule-based shortcuts.
pub(super) const APPROVAL_MODEL_V2_SQL: &str = r#"
ALTER TABLE approval_requests ADD COLUMN action_category TEXT;
ALTER TABLE approval_requests ADD COLUMN action_payload TEXT;
ALTER TABLE approval_requests ADD COLUMN sandbox_boundary TEXT;
ALTER TABLE approval_requests ADD COLUMN reviewer TEXT NOT NULL DEFAULT 'user';
ALTER TABLE approval_requests ADD COLUMN decision_scope TEXT NOT NULL DEFAULT 'once';
ALTER TABLE approval_requests ADD COLUMN decision_source TEXT NOT NULL DEFAULT 'user';

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
"#;

pub(super) const INITIAL_SCHEMA: &str = r#"
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
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
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

CREATE INDEX IF NOT EXISTS idx_threads_workspace ON threads(workspace_id);
CREATE INDEX IF NOT EXISTS idx_threads_recent ON threads(status, pinned, last_message_at, updated_at);
CREATE INDEX IF NOT EXISTS idx_messages_thread ON messages(thread_id, created_at);
CREATE INDEX IF NOT EXISTS idx_runs_thread ON runs(thread_id, created_at);
CREATE INDEX IF NOT EXISTS idx_reference_targets_scope ON reference_targets(scope, workspace_id, target_type);
"#;
