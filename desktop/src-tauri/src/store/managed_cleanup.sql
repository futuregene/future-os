CREATE TABLE IF NOT EXISTS managed_directory_cleanup (
    kind TEXT NOT NULL CHECK(kind IN ('images', 'chat', 'review')),
    directory_id TEXT NOT NULL,
    first_seen_at INTEGER NOT NULL,
    delete_requested_at INTEGER,
    PRIMARY KEY(kind, directory_id)
);
