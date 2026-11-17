-- Contract 8 records only successful archive cleanup. NULL means the run's
-- output has not been purged (including legacy archives and failed cleanup).
ALTER TABLE runs ADD COLUMN logs_purged_at INTEGER;
