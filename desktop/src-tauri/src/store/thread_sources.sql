CREATE TABLE IF NOT EXISTS thread_sources (
    thread_id TEXT PRIMARY KEY NOT NULL REFERENCES threads(id) ON DELETE CASCADE,
    session_id TEXT NOT NULL,
    database_id TEXT NOT NULL,
    restore_generation TEXT NOT NULL
);
