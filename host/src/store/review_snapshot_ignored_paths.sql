-- Contract 6 adds the record behind `ignored_count`: which paths the
-- Workspace's ignore rules kept out of a snapshot, and the rule that excluded
-- each one. The runner skips this ALTER when a fresh database (or one written
-- by a build that shipped the column before this contract) already has it.
ALTER TABLE review_snapshots ADD COLUMN ignored_paths TEXT;
