//! What the store does when the schema underneath it is damaged.
//!
//! Every reader and writer runs inside the session database. A partially
//! migrated or externally damaged file must make the operation *fail loudly*:
//! a silently skipped read would serve a page that no longer matches the
//! transcript, and a silently skipped write would lose a durable event. Each
//! case therefore breaks exactly one table from a second connection and asserts
//! the error names it.

use super::*;
use serde_json::json;

fn session_entry(id: &str) -> serde_json::Value {
    json!({"id": id, "type": "user", "role": "user",
        "timestamp": "2026-01-01T00:00:00Z", "content": [{"type":"text","text":"question"}],
        "meta": {"run_id": "r1"}})
}

fn event(kind: &str, index: i64) -> serde_json::Value {
    json!({"event_type": kind, "data": "{\"ok\":true}", "session_id": "s", "run_id": "r",
        "epoch": 1, "idx": index, "session_idx": -1, "run_sequence": 1,
        "timestamp": "2026-01-01T00:00:00Z"})
}

/// Open a store, seed one session, then damage it from a second connection.
/// The store's worker keeps its own handle: SQLite lets another connection drop
/// a table out from under it, which is exactly the "damaged file" shape.
fn damaged(tables: &[&str]) -> (tempfile::TempDir, SqliteStore) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("agent.db");
    let store = SqliteStore::open(&path).unwrap();
    store.replace("s", vec![session_entry("u1")]).unwrap();
    store.append_event("s", event("text_chunk", 0)).unwrap();
    if !tables.is_empty() {
        let raw = rusqlite::Connection::open(&path).unwrap();
        for table in tables {
            // `entry_records` and `history_display` are views over the block
            // store, so the drop has to match the object kind.
            let kind: String = raw
                .query_row(
                    "SELECT type FROM sqlite_master WHERE name=?1",
                    [table],
                    |row| row.get(0),
                )
                .unwrap();
            let keyword = if kind == "view" { "VIEW" } else { "TABLE" };
            raw.execute_batch(&format!("DROP {keyword} {table}"))
                .unwrap();
            assert!(
                raw.query_row(
                    "SELECT count(*) FROM sqlite_master WHERE name=?1",
                    [table],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap()
                    == 0,
                "{table} is really gone"
            );
        }
    }
    (dir, store)
}

fn message(error: anyhow::Error) -> String {
    format!("{error:#}")
}

#[test]
fn every_event_reader_reports_the_table_it_cannot_reach() {
    // The pricing projection reads run_events *and* run_snapshots.
    let (_dir, store) = damaged(&["run_events"]);
    let error = message(store.pricing_event_payloads("s").unwrap_err());
    assert!(error.contains("run_events"), "{error}");

    // The typed replay page first asks whether the run is known at all.
    let (_dir, store) = damaged(&["run_events"]);
    let error = message(
        store
            .typed_events_page::<serde_json::Value>("s", "r", -1, false, None)
            .unwrap_err(),
    );
    assert!(error.contains("run_events"), "{error}");

    // Existence is a cheap pre-check for both tables.
    let (_dir, store) = damaged(&["run_snapshots"]);
    let error = message(store.has_events("s", "r").unwrap_err());
    assert!(error.contains("run_snapshots"), "{error}");

    // A batch append must not lose the events it was handed.
    let (_dir, store) = damaged(&["run_events"]);
    let error = message(
        store
            .append_events("s", vec![event("text_chunk", 1)])
            .unwrap_err(),
    );
    assert!(error.contains("run_events"), "{error}");

    // The compaction page reader is the maintenance path's view of the same
    // table; a damaged file must not look like "no more events".
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("agent.db");
    let store = SqliteStore::open(&path).unwrap();
    store.replace("s", vec![session_entry("u1")]).unwrap();
    let raw = rusqlite::Connection::open(&path).unwrap();
    raw.execute_batch("DROP TABLE run_events").unwrap();
    drop(raw);
    let error = match store
        .db
        .call(|db| super::events::read_compaction_event_rows(db, "s", "r", 0, 0, 10))
    {
        Ok(_) => panic!("a damaged event table must not read as an empty page"),
        Err(error) => message(error),
    };
    assert!(error.contains("run_events"), "{error}");
}

#[test]
fn a_transcript_rewrite_reports_the_table_it_cannot_reach() {
    // The rewrite checks the migration status first: a file whose legacy
    // import table is gone must refuse rather than rewrite a half-known history.
    let (_dir, store) = damaged(&["legacy_imports"]);
    let error = message(store.replace("s", vec![session_entry("u1")]).unwrap_err());
    assert!(error.contains("legacy_imports"), "{error}");

    // With the migration table intact, the next statement the rewrite needs is
    // the position probe on the entry table.
    let (_dir, store) = damaged(&["entry_records"]);
    let error = message(store.replace("s", vec![session_entry("u1")]).unwrap_err());
    assert!(error.contains("entry_records"), "{error}");

    // A damaged entries table surfaces on read rather than as an empty session.
    let (_dir, store) = damaged(&["entry_records"]);
    let error = message(store.entries("s").unwrap_err());
    assert!(error.contains("entry_records"), "{error}");
}
