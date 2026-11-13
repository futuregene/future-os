//! Cold `get_state`: every format answers from bounded rows.
use super::*;

/// A conversation stored the way an imported (`format` 1) session is: the
/// bodies live in `history_shapes`, the headers stay in `entries`, and no
/// `session_formats` row exists. That is the shape the largest conversation on
/// the measuring machine has, and the one `get_state` used to load in full.
fn legacy_session(manager: &Manager, id: &str, messages: usize) {
    let store = manager.storage().unwrap();
    store.bind_legacy_events(id).unwrap();
    let mut entries = vec![serde_json::json!({
        "id": "info",
        "type": "session_info",
        "role": "system",
        "timestamp": "2026-11-04T00:00:00Z",
        "content": {"cwd": "/workspace", "model": "mock", "session_name": "kept",
                    "parent_session_id": "parent", "system_prompt": "rules".repeat(2000)}
    })];
    for index in 0..messages {
        entries.push(serde_json::json!({
            "id": format!("m-{index}"),
            "type": "user",
            "role": "user",
            "timestamp": "2026-11-04T00:00:01Z",
            "content": [{"type": "text", "text": format!("message {index}")}]
        }));
    }
    store.replace(id, entries).unwrap();
    // Downgrade the row the way an import leaves it, and drop the derived index
    // so the legacy read rebuilds it from `entries` instead of assuming rows.
    let session = id.to_owned();
    store
        .db
        .call(move |db| {
            db.execute(
                "DELETE FROM session_formats WHERE session_id=?1",
                [&session],
            )?;
            db.execute(
                "DELETE FROM history_index_state WHERE session_id=?1",
                [&session],
            )?;
            Ok(())
        })
        .unwrap();
}

#[test]
fn legacy_state_and_metadata_read_bounded_rows_not_the_transcript() {
    let dir = std::env::temp_dir().join(format!(
        "future_test_legacy_state_{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let manager = Manager::new(dir.clone());
    legacy_session(&manager, "legacy-state", 300);
    let store = manager.storage().unwrap();
    let format = store
        .db
        .read(|db| crate::session::content::format(db, "legacy-state"))
        .unwrap();
    assert_eq!(format, 1, "the fixture must actually be a legacy session");

    // The count and the lineage come from the header index and the session row.
    let before = store.db.metrics()["readerJob"]["count"].as_u64().unwrap();
    let (count, parent) = manager.state_metadata("legacy-state").unwrap();
    let jobs = store.db.metrics()["readerJob"]["count"].as_u64().unwrap() - before;
    assert_eq!(count, 300);
    assert_eq!(parent, "parent");
    assert!(
        jobs <= 2,
        "state_metadata must not walk the transcript ({jobs} jobs)"
    );
    assert!(
        manager.state_metadata("legacy-state").unwrap().0 == 300,
        "the second poll answers the same"
    );

    // The metadata read is bounded too: three rows, whatever the transcript is.
    let loaded = manager.load_metadata("legacy-state").unwrap();
    assert_eq!(loaded.name, "kept");
    assert_eq!(loaded.model, "mock");
    assert_eq!(loaded.parent_session_id, "parent");
    assert!(
        loaded.entries.len() <= 3,
        "cold metadata must not hydrate the transcript ({} entries)",
        loaded.entries.len()
    );
    // Reading it again is a plain bounded read: the derived index is ready now.
    let before = store.db.metrics()["readerJob"]["count"].as_u64().unwrap();
    manager.load_metadata("legacy-state").unwrap();
    let jobs = store.db.metrics()["readerJob"]["count"].as_u64().unwrap() - before;
    assert_eq!(store.storage_format("legacy-state").unwrap(), 2);
    legacy_session(&manager, "small-legacy-state", 1);
    manager.load_metadata("small-legacy-state").unwrap();
    let before = store.db.metrics()["readerJob"]["count"].as_u64().unwrap();
    manager.load_metadata("small-legacy-state").unwrap();
    let small_jobs = store.db.metrics()["readerJob"]["count"].as_u64().unwrap() - before;
    assert_eq!(
        jobs, small_jobs,
        "after migration metadata IO must be independent of transcript length"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn legacy_run_markers_are_read_through_the_header_index() {
    let dir = std::env::temp_dir().join(format!(
        "future_test_legacy_markers_{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let manager = Manager::new(dir.clone());
    legacy_session(&manager, "legacy-markers", 200);
    let store = manager.storage().unwrap();
    store
        .append(
            "legacy-markers",
            vec![
                serde_json::json!({"id":"start","type":"run_started",
                    "timestamp":"2026-11-04T00:00:02Z","content":{"run_id":"r-1","epoch":1}}),
                serde_json::json!({"id":"end","type":"run_terminal",
                    "timestamp":"2026-11-04T00:00:03Z","content":{"run_id":"r-1","state":"completed"}}),
            ],
        )
        .unwrap();

    assert_eq!(manager.unterminated_run_id("legacy-markers").unwrap(), None);
    let before = store.db.metrics()["readerJob"]["count"].as_u64().unwrap();
    manager.unterminated_run_id("legacy-markers").unwrap();
    let jobs = store.db.metrics()["readerJob"]["count"].as_u64().unwrap() - before;
    assert!(
        jobs <= 4,
        "marker detection must not walk the transcript ({jobs} jobs)"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
