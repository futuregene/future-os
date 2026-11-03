-- Frozen retired DDL from 93536d67 (Desktop contract 3). Upgrade fixture only.
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

CREATE INDEX IF NOT EXISTS idx_reference_targets_scope ON reference_targets(scope, workspace_id, target_type);
CREATE INDEX IF NOT EXISTS idx_review_snapshots_run ON review_snapshots(run_id, phase);
