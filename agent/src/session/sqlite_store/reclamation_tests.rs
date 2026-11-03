use super::*;
use rusqlite::Connection;
use serde_json::Value;

/// Live + free pages of the database file, in pages.
fn pages(store: &SqliteStore) -> (i64, i64) {
    store
        .db
        .call(|db| {
            let count: i64 = db.pragma_query_value(None, "page_count", |row| row.get(0))?;
            let free: i64 = db.pragma_query_value(None, "freelist_count", |row| row.get(0))?;
            Ok((count, free))
        })
        .unwrap()
}

fn event(idx: i64) -> Value {
    event_for("run", idx)
}

fn event_for(run: &str, idx: i64) -> Value {
    serde_json::json!({
        "run_id": run,
        "idx": idx,
        "epoch": 1,
        "timestamp": "2026-01-01T00:00:00Z",
        "kind": "text",
        "payload": "x".repeat(400),
    })
}

/// New databases enable incremental reclamation without a full rewrite.
#[test]
fn opening_new_database_enables_incremental_auto_vacuum() {
    let directory = tempfile::tempdir().unwrap();
    let store = SqliteStore::open(&directory.path().join("agent.db")).unwrap();
    let mode: i64 = store
        .db
        .call(|db| Ok(db.pragma_query_value(None, "auto_vacuum", |row| row.get(0))?))
        .unwrap();
    assert_eq!(mode, 2, "INCREMENTAL");
}

/// Explicitly drain freed pages when verifying incremental reclamation.
fn drain_reclaim(store: &SqliteStore) {
    store.db.reclaim(1, i64::MAX).unwrap();
}

fn pages_raw(path: &std::path::Path) -> (i64, i64) {
    let connection = Connection::open(path).unwrap();
    (
        connection
            .pragma_query_value(None, "page_count", |row| row.get(0))
            .unwrap(),
        connection
            .pragma_query_value(None, "freelist_count", |row| row.get(0))
            .unwrap(),
    )
}

/// Build a database with a large freelist, the way SQLite leaves one: delete
/// rows without ever calling `incremental_vacuum`.
fn bloat(path: &std::path::Path, rows: i64, blob: usize, mode: &str) {
    let mode = mode.to_owned();
    let store = SqliteStore::open(path).unwrap();
    store
        .db
        .call(move |db| {
            db.execute_batch(&format!("PRAGMA auto_vacuum = {mode}; VACUUM;"))?;
            db.execute_batch("CREATE TABLE IF NOT EXISTS bloat (payload TEXT)")?;
            db.execute_batch(&format!(
                "WITH RECURSIVE c(i) AS (SELECT 1 UNION ALL SELECT i+1 FROM c WHERE i<{rows})
                 INSERT INTO bloat SELECT hex(randomblob({blob})) FROM c"
            ))?;
            db.execute_batch("DELETE FROM bloat")?;
            Ok(())
        })
        .unwrap();
}

/// Existing NONE-mode databases keep their layout and reusable free pages.
#[test]
fn opening_old_database_preserves_layout_and_free_pages() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("agent.db");
    bloat(&path, 20_000, 400, "NONE");
    let before = pages_raw(&path);
    assert!(before.1 > 2_000, "fixture freelist: {}", before.1);
    let reopened = SqliteStore::open(&path).unwrap();
    assert_eq!(pages(&reopened), before);
    let mode: i64 = reopened
        .db
        .call(|db| Ok(db.pragma_query_value(None, "auto_vacuum", |row| row.get(0))?))
        .unwrap();
    assert_eq!(mode, 0, "opening must not rebuild an old database");
}

/// The reported symptom: a session delete freed its events but the file kept
/// every page. Deleting the largest storage a session has must shrink it.
#[test]
fn deleting_a_session_returns_its_pages_to_the_filesystem() {
    let directory = tempfile::tempdir().unwrap();
    let store = SqliteStore::open(&directory.path().join("agent.db")).unwrap();
    store.bind_legacy_events("doomed").unwrap();
    // Past the 4 MiB reclaiming threshold, in one transaction.
    store
        .append_events("doomed", (0..24_000).map(event).collect())
        .unwrap();
    let (with_session, free_before) = pages(&store);
    assert!(free_before < 1024);

    store.delete("doomed").unwrap();
    drain_reclaim(&store);
    let (after, free_after) = pages(&store);
    assert!(
        after < with_session,
        "the file must shrink: {with_session} → {after} pages"
    );
    assert!(
        free_after < 1024,
        "and not just hand the pages to the freelist: {free_after} free"
    );
}

/// Pruning a run is the other path that frees bulk storage.
#[test]
fn pruning_run_events_reclaims_their_pages() {
    let directory = tempfile::tempdir().unwrap();
    let store = SqliteStore::open(&directory.path().join("agent.db")).unwrap();
    store.bind_legacy_events("keeps").unwrap();
    store
        .append_events("keeps", (0..24_000).map(event).collect())
        .unwrap();
    let (peak, _) = pages(&store);

    store.prune_events("keeps", "run").unwrap();
    drain_reclaim(&store);
    let (after, free) = pages(&store);
    assert!(
        after < peak,
        "pruning must shrink the file: {peak} → {after}"
    );
    assert!(free < 1024, "{free} pages still free");
}

/// The failure from the log. `replace` reads `legacy_imports` before it
/// writes, so a deferred transaction loses its snapshot the instant another
/// connection commits — SQLite answers `SQLITE_BUSY_SNAPSHOT` immediately
/// rather than waiting out `busy_timeout`, and the append failed with
/// "database is locked" while the skill registry was writing this file.
/// Owning the write lock at `BEGIN` makes the wait happen instead.
#[test]
fn a_session_write_waits_out_a_competing_writer() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("agent.db");
    let store = SqliteStore::open(&path).unwrap();

    let holder = Connection::open(&path).unwrap();
    holder
        .busy_timeout(std::time::Duration::from_secs(5))
        .unwrap();
    holder
        .execute_batch(
            "BEGIN IMMEDIATE; INSERT OR REPLACE INTO storage_meta(key,value) VALUES('held','1');",
        )
        .unwrap();

    // Release well inside the timeout, from another thread.
    let handle = std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(250));
        holder.execute_batch("COMMIT").unwrap();
    });

    let entry = serde_json::json!({
        "id": "u",
        "type": "user",
        "role": "user",
        "timestamp": "2026-01-01T00:00:00Z",
        "content": "hello",
        "meta": {"run_id": ""},
    });
    store
        .replace("session", vec![entry])
        .expect("the write should wait for the competing writer, not fail");
    handle.join().unwrap();

    // And it really landed, rather than being dropped along the way.
    let entries: Vec<Value> = store.entries("session").unwrap();
    assert_eq!(entries.len(), 1);
}
