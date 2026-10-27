use super::*;

#[test]
fn active_incomplete_and_gapped_journals_are_kept() {
    for status in ["running", "completed"] {
        let dir = tempfile::tempdir().unwrap();
        let store = fixture(&dir.path().join("agent.db"), status, 10);
        if status == "completed" {
            store
                .db
                .call(|db| {
                    db.execute("DELETE FROM run_events WHERE idx=11", [])?;
                    // Settled a moment ago: the transcript terminal precedes the
                    // final event broadcast, so the missing tail may still be on
                    // its way. Age is what turns a truncated stream into a
                    // foldable one, so this one has none yet.
                    db.execute(
                        "UPDATE runs SET completed_at_ms=?1 WHERE run_id='run'",
                        [crate::session::journal_compaction::now_ms()],
                    )?;
                    Ok(())
                })
                .unwrap();
        }
        assert_eq!(store.compact_run("s", "run").unwrap(), 0);
        assert!(store
            .replay_page("s", "run", -1, false, None)
            .unwrap()
            .snapshot
            .is_none());
    }
    let dir = tempfile::tempdir().unwrap();
    let store = fixture(&dir.path().join("agent.db"), "completed", 10);
    store
        .db
        .call(|db| {
            db.execute("DELETE FROM run_events WHERE idx=5", [])?;
            Ok(())
        })
        .unwrap();
    assert!(store.compact_run("s", "run").is_err());
    assert_eq!(store.events("s", "run").unwrap().len(), 11);
}

/// Rows still in the raw journal — deliberately read from the table rather than
/// through `events()`, which answers from a published snapshot.
fn raw_row_count(store: &SqliteStore) -> i64 {
    store
        .db
        .call(|db| {
            Ok(db.query_row(
                "SELECT count(*) FROM run_events WHERE session_id='s' AND run_id='run'",
                [],
                |row| row.get(0),
            )?)
        })
        .unwrap()
}

/// The case the grace window exists for: a run a crash cut off mid-stream. Its
/// stream never gets an `agent_end`, so nothing will ever complete it — the
/// journal is final and, on a real store, 50 of these held 762k events (179 MiB)
/// forever. `runs` settles that the run is over; age proves no producer is left.
#[test]
fn an_aged_incomplete_stream_is_folded_from_the_settled_run() {
    let dir = tempfile::tempdir().unwrap();
    let store = fixture(&dir.path().join("agent.db"), "interrupted", 10);
    store
        .db
        .call(|db| {
            // What the crash left: no terminal event.
            db.execute("DELETE FROM run_events WHERE idx=11", [])?;
            Ok(())
        })
        .unwrap();
    let outcome = store
        .compact_run_cancellable("s", "run", &std::sync::atomic::AtomicBool::new(false))
        .unwrap();
    assert_eq!(outcome.skip, None, "settled and quiescent: foldable");
    assert!(
        outcome.truncated,
        "published from a stream without agent_end"
    );
    assert_eq!(
        raw_row_count(&store),
        0,
        "the raw journal is retired, which is the whole point"
    );
    // `events()` reads the published snapshot: the same stream, folded —
    // `agent_start` plus one coalesced text segment, with the truncated tail
    // kept as it was.
    assert_eq!(store.events("s", "run").unwrap().len(), 2);
    // The replay a late client attaches to is the same stream, folded.
    let page = store.replay_page("s", "run", -1, false, None).unwrap();
    let snapshot = page.snapshot.expect("published snapshot");
    assert_eq!(snapshot.cursor, 10);
    assert_eq!(
        snapshot
            .events
            .last()
            .map(|event| event.event_type.as_str()),
        Some("text_chunk"),
        "the truncated tail is preserved as-is, not invented"
    );
    let stats = store.journal_stats().unwrap();
    assert_eq!(stats["truncatedPublishes"], 1);
}

/// Age alone is not enough when there is no settle time to age: a legacy-imported
/// run whose `completed_at_ms` is unknown cannot prove its producer is gone, so it
/// keeps its journal until someone can.
#[test]
fn an_incomplete_stream_without_a_settle_time_is_kept() {
    let dir = tempfile::tempdir().unwrap();
    let store = fixture(&dir.path().join("agent.db"), "interrupted", 10);
    store
        .db
        .call(|db| {
            db.execute("DELETE FROM run_events WHERE idx=11", [])?;
            db.execute(
                "UPDATE runs SET completed_at_ms=NULL WHERE run_id='run'",
                [],
            )?;
            Ok(())
        })
        .unwrap();
    let outcome = store
        .compact_run_cancellable("s", "run", &std::sync::atomic::AtomicBool::new(false))
        .unwrap();
    assert_eq!(outcome.skip, Some(SkipReason::NotTerminal));
    assert_eq!(store.events("s", "run").unwrap().len(), 11);
}

/// A **single** stored event above the paged-read budget is the one permanent
/// refusal left: a bounded read cannot see it, so no byte-faithful snapshot can
/// be built. It is recorded as `oversized_event` rather than silently skipped,
/// and raw paging keeps working.
#[test]
fn a_single_event_above_the_page_budget_keeps_raw_paged_replay() {
    let dir = tempfile::tempdir().unwrap();
    let store = fixture(&dir.path().join("agent.db"), "completed", 1);
    let body = json!({"text":"x".repeat(SNAPSHOT_BYTE_BUDGET)}).to_string();
    store
        .db
        .call(move |db| {
            db.execute(
                "UPDATE run_events SET payload=json_set(payload,'$.data',?1) WHERE idx=1",
                [body],
            )?;
            Ok(())
        })
        .unwrap();
    assert_eq!(store.compact_run("s", "run").unwrap(), 0);
    let page = store.replay_page("s", "run", 0, false, Some(1)).unwrap();
    assert_eq!(page.events.len(), 1);
    assert!(page.snapshot.is_none());
}

/// A journal above the per-transaction delete bound is retired across several
/// slices instead of being skipped forever. This is the dominant blocker on a
/// real database: measured on a 1 GB `agent.db`, 1,213,282 of 1,304,373 stuck
/// rows sat in 15 runs whose only disqualifier was that row count.
#[test]
fn oversized_raw_journal_is_retired_in_bounded_slices() {
    let dir = tempfile::tempdir().unwrap();
    let chunks = RETIRE_CHUNK_ROWS as usize + 2;
    let store = fixture(&dir.path().join("agent.db"), "completed", chunks);
    let rows = RETIRE_CHUNK_ROWS + 4;
    let raw = |store: &SqliteStore| -> i64 {
        store
            .db
            .call(|db| {
                Ok(db.query_row(
                    "SELECT count(*) FROM run_events WHERE run_id='run'",
                    [],
                    |r| r.get::<_, i64>(0),
                )?)
            })
            .unwrap()
    };
    assert_eq!(raw(&store), rows);
    assert_eq!(store.compact_run("s", "run").unwrap(), rows as usize);
    assert_eq!(raw(&store), 0, "every raw row must be retired");
    let page = store.replay_page("s", "run", -1, false, None).unwrap();
    let snapshot = page
        .snapshot
        .expect("the slice chain must publish one snapshot");
    assert_eq!(snapshot.cursor, rows - 1);
    assert_eq!(text(&snapshot), "中文𠮷".repeat(chunks));
    // Durable counters are the evidence that this was one run, not two.
    let stats = store.journal_stats().unwrap();
    assert_eq!(stats["reclaimed"]["events"], rows);
    assert_eq!(stats["reclaimed"]["runs"], 1);
}

#[test]
fn duplicate_epoch_at_page_boundary_is_not_silently_retired() {
    let dir = tempfile::tempdir().unwrap();
    let store = fixture(&dir.path().join("agent.db"), "completed", 300);
    let event = SseEvent {
        run_id: "run".into(),
        epoch: 1,
        idx: 255,
        ..SseEvent::new("text_chunk", json!({"text":"another epoch"}))
    };
    store
        .append_event("s", serde_json::to_value(event).unwrap())
        .unwrap();
    assert!(store.compact_run("s", "run").is_err());
    assert_eq!(store.events("s", "run").unwrap().len(), 303);
}

#[test]
fn unknown_envelope_and_non_duplicate_provider_text_keep_raw_journal() {
    for unknown in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let store = fixture(&dir.path().join("agent.db"), "completed", 1);
        store.db.call(move |db| {
            if unknown {
                db.execute("UPDATE run_events SET payload=json_set(payload,'$.future_field','must survive') WHERE idx=1",[])?;
            } else {
                db.execute("UPDATE run_events SET payload=json_set(payload,'$.event_type','text_delta') WHERE idx=1",[])?;
            }
            Ok(())
        }).unwrap();
        if unknown {
            assert!(store.compact_run("s", "run").is_err());
        } else {
            assert_eq!(store.compact_run("s", "run").unwrap(), 0);
        }
        assert_eq!(store.events("s", "run").unwrap().len(), 3);
        assert!(store
            .replay_page("s", "run", -1, false, None)
            .unwrap()
            .snapshot
            .is_none());
    }
}
