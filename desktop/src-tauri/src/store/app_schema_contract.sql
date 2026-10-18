-- Contract 1 adopts the existing, independently transactional release migrations.
-- This file is immutable once released. Append a new contract for future changes.
CREATE TABLE IF NOT EXISTS app_schema_migrations (
    version INTEGER PRIMARY KEY NOT NULL,
    checksum TEXT NOT NULL,
    applied_at INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS app_database_identity (
    id TEXT PRIMARY KEY NOT NULL CHECK(id = 'main'),
    database_id TEXT NOT NULL,
    restore_generation TEXT NOT NULL
);
