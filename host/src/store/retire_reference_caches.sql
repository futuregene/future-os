-- These caches have no production reader or writer. Drop children first.
DROP TABLE IF EXISTS object_references;
DROP TABLE IF EXISTS reference_targets;
DROP TABLE IF EXISTS workspace_files;
-- The UNIQUE(run_id, phase) autoindex serves the same lookup.
DROP INDEX IF EXISTS idx_review_snapshots_run;
