-- Contract 8 gives a restored conversation a fresh idle period for automatic
-- archiving without overloading the general-purpose updated_at timestamp.
ALTER TABLE threads ADD COLUMN restored_at INTEGER;
