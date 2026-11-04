-- Contract 5 upgrades existing version-4 databases before core validation.
-- The runner skips this ALTER when a fresh or nightly-110 database already
-- has the column, preserving any existing archive timestamps.
ALTER TABLE workspaces ADD COLUMN archived_at INTEGER;
