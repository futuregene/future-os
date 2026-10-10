use super::*;
use crate::rpc::SseBroadcaster;
use serde_json::json;

fn fixture(path: &std::path::Path, status: &str, chunks: usize) -> SqliteStore {
    let store = SqliteStore::open(path).unwrap();
    store.bind_events("s").unwrap();
    let status = status.to_owned();
    store.db.call(move |db| {
        db.execute("INSERT INTO runs(session_id,run_id,status,completed_at_ms) VALUES ('s','run',?1,1)",[status])?;
        Ok(())
    }).unwrap();
    let mut events = vec![SseEvent {
        run_id: "run".into(),
        idx: 0,
        ..SseEvent::new("agent_start", json!({}))
    }];
    for idx in 1..=chunks {
        events.push(SseEvent {
            run_id: "run".into(),
            idx: idx as i64,
            ..SseEvent::new("text_chunk", json!({"text":"中文𠮷"}))
        });
    }
    events.push(SseEvent {
        run_id: "run".into(),
        idx: chunks as i64 + 1,
        ..SseEvent::new("agent_end", json!({"state":"completed"}))
    });
    store
        .append_events(
            "s",
            events
                .into_iter()
                .map(|e| serde_json::to_value(e).unwrap())
                .collect(),
        )
        .unwrap();
    store
}

fn text(snapshot: &CompactRunSnapshot) -> String {
    snapshot
        .events
        .iter()
        .filter(|e| e.event_type == "text_chunk")
        .map(|e| {
            serde_json::from_str::<serde_json::Value>(&e.data).unwrap()["text"]
                .as_str()
                .unwrap()
                .to_owned()
        })
        .collect()
}

#[test]
fn completed_journal_is_replayable_after_reopen_and_old_cursor_resync() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("agent.db");
    let store = fixture(&path, "completed", 1000);
    assert_eq!(store.compact_run("s", "run").unwrap(), 1002);
    assert!(store.has_events("s", "run").unwrap());
    let page = store.replay_page("s", "run", 123, false, Some(1)).unwrap();
    assert!(page.known && page.events.is_empty());
    let snapshot = page.snapshot.unwrap();
    assert_eq!(snapshot.cursor, 1001);
    assert_eq!(text(&snapshot), "中文𠮷".repeat(1000));
    assert!(snapshot.events.len() < 10);
    drop(store);
    let manager = crate::session::Manager::new(dir.path().to_path_buf());
    let broadcaster = SseBroadcaster::new();
    broadcaster.configure_journal("s", &manager).unwrap();
    let historical = broadcaster.events_since("run", 123).unwrap();
    assert!(historical.1.is_empty());
    assert_eq!(historical.3.unwrap().cursor, 1001);
    assert!(broadcaster.events_since("run", 1001).unwrap().3.is_none());
    broadcaster.start_run_with_sequence("run".into(), 0, Some(1));
    let reopened = broadcaster.run_snapshot("run").unwrap();
    assert_eq!(reopened.cursor, 1001);
    assert_eq!(
        broadcaster
            .events_since("run", 123)
            .unwrap()
            .3
            .unwrap()
            .cursor,
        1001
    );
    assert_eq!(
        broadcaster
            .attach("run", 123)
            .unwrap()
            .projection
            .unwrap()
            .cursor,
        1001
    );
    assert!(broadcaster
        .attach("run", 1001)
        .unwrap()
        .projection
        .is_none());
    assert!(broadcaster.events_since("unknown", -1).is_err());
}

#[test]
fn transaction_failure_retains_every_raw_row_and_explicit_calls_are_idempotent() {
    let dir = tempfile::tempdir().unwrap();
    let store = fixture(&dir.path().join("agent.db"), "completed", 1000);
    store.db.call(|db| {
        db.execute_batch("CREATE TRIGGER reject_cleanup BEFORE DELETE ON run_events WHEN OLD.idx=500 BEGIN SELECT RAISE(ABORT,'injected'); END;")?;
        Ok(())
    }).unwrap();
    assert!(store.compact_run("s", "run").is_err());
    let page = store.replay_page("s", "run", -1, false, None).unwrap();
    assert!(page.snapshot.is_none());
    assert_eq!(page.events.len(), 1002);
    store
        .db
        .call(|db| {
            db.execute_batch("DROP TRIGGER reject_cleanup")?;
            Ok(())
        })
        .unwrap();
    assert_eq!(store.compact_run("s", "run").unwrap(), 1002);
    assert_eq!(store.compact_run("s", "run").unwrap(), 0);
}

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

#[test]
fn oversize_snapshot_keeps_raw_paged_replay() {
    let dir = tempfile::tempdir().unwrap();
    let store = fixture(&dir.path().join("agent.db"), "completed", 1);
    let body = json!({"text":"x".repeat(SNAPSHOT_BYTES)}).to_string();
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

#[test]
fn oversized_raw_journal_keeps_paging_without_an_atomic_bulk_delete() {
    let dir = tempfile::tempdir().unwrap();
    let store = fixture(
        &dir.path().join("agent.db"),
        "completed",
        MAX_RETIRE_EVENTS as usize,
    );
    assert_eq!(store.compact_run("s", "run").unwrap(), 0);
    let page = store.replay_page("s", "run", 0, false, Some(1)).unwrap();
    assert_eq!(page.events.len(), 1);
    assert!(page.snapshot.is_none());
    assert_eq!(
        store
            .db
            .call(|db| Ok(db.query_row(
                "SELECT count(*) FROM run_events WHERE run_id='run'",
                [],
                |r| r.get::<_, i64>(0)
            )?))
            .unwrap(),
        MAX_RETIRE_EVENTS + 2
    );
}

#[test]
fn pricing_and_session_events_survive_and_explicit_prune_deletes_snapshot() {
    let dir = tempfile::tempdir().unwrap();
    let store = fixture(&dir.path().join("agent.db"), "completed", 1000);
    store.db.call(|db| {
        db.execute("UPDATE run_events SET payload=json_set(payload,'$.event_type','usage','$.data',?1) WHERE idx=500",[json!({"output_tokens":7}).to_string()])?;
        Ok(())
    }).unwrap();
    store.append_event("s",json!({"run_id":"","session_idx":0,"idx":-1,"event_type":"model_changed","data":json!({"model":"test"}).to_string()})).unwrap();
    let pricing = store.pricing_event_payloads("s").unwrap();
    store.compact_run("s", "run").unwrap();
    assert_eq!(store.pricing_event_payloads("s").unwrap(), pricing);
    assert_eq!(store.events("s", "").unwrap().len(), 1);
    store.prune_events("s", "run").unwrap();
    assert!(!store.has_events("s", "run").unwrap());
    assert_eq!(store.pricing_event_payloads("s").unwrap().len(), 1);
}

#[test]
fn maintenance_only_compacts_notified_runs_and_never_retries_failed_runs() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("agent.db");
    let store = fixture(&path, "completed", 1000);
    let maintenance = JournalMaintenance::start(store.clone()).unwrap();
    maintenance.flush();
    assert!(store
        .replay_page("s", "run", -1, false, None)
        .unwrap()
        .snapshot
        .is_none());
    store.db.call(|db| {
        db.execute_batch("CREATE TRIGGER reject_cleanup BEFORE DELETE ON run_events WHEN OLD.idx=500 BEGIN SELECT RAISE(ABORT,'injected'); END;")?;
        Ok(())
    }).unwrap();
    maintenance.request("s", "run");
    maintenance.flush();
    assert_eq!(store.events("s", "run").unwrap().len(), 1002);
    store
        .db
        .call(|db| {
            db.execute_batch("DROP TRIGGER reject_cleanup")?;
            Ok(())
        })
        .unwrap();
    maintenance.flush();
    assert!(store
        .replay_page("s", "run", -1, false, None)
        .unwrap()
        .snapshot
        .is_none());
    drop(maintenance);
    drop(store);
    let store = SqliteStore::open(&path).unwrap();
    let maintenance = JournalMaintenance::start(store.clone()).unwrap();
    maintenance.flush();
    assert_eq!(store.events("s", "run").unwrap().len(), 1002);
    assert!(store
        .replay_page("s", "run", -1, false, None)
        .unwrap()
        .snapshot
        .is_none());
    // A new completion notification works without touching the old journal.
    store.db.call(|db| {
        db.execute("INSERT INTO runs(session_id,run_id,status,completed_at_ms) VALUES ('s','new','completed',1)", [])?;
        Ok(())
    }).unwrap();
    for (idx, kind) in [(0, "agent_start"), (1, "agent_end")] {
        let event = SseEvent {
            run_id: "new".into(),
            idx,
            ..SseEvent::new(kind, json!({}))
        };
        store
            .append_event("s", serde_json::to_value(event).unwrap())
            .unwrap();
    }
    maintenance.request("s", "new");
    maintenance.flush();
    assert!(store
        .replay_page("s", "new", -1, false, None)
        .unwrap()
        .snapshot
        .is_some());
    assert_eq!(store.events("s", "run").unwrap().len(), 1002);
    store.delete("s").unwrap();
    assert!(!store.has_events("s", "new").unwrap());
}

#[test]
fn space_reclamation_never_deletes_existing_raw_journals() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("agent.db");
    let store = fixture(&path, "completed", 3);
    store.compact_run("s", "run").unwrap();
    // Simulate a previously published snapshot with a redundant raw tail.
    store.db.call(|db| {
        db.execute("INSERT INTO run_events(session_id,run_id,epoch,idx,payload) VALUES ('s','run',0,0,'{}')",[])?;
        db.execute("UPDATE run_snapshots SET raw_pending=1",[])?;
        Ok(())
    }).unwrap();
    store
        .db
        .call(super::super::database::reclaim_idle_step)
        .unwrap();
    drop(store);
    let store = SqliteStore::open(&path).unwrap();
    store
        .db
        .call(super::super::database::reclaim_idle_step)
        .unwrap();
    assert_eq!(
        store
            .db
            .call(|db| Ok(db.query_row(
                "SELECT count(*) FROM run_events WHERE run_id='run'",
                [],
                |r| r.get::<_, i64>(0)
            )?))
            .unwrap(),
        1
    );
    assert!(store
        .replay_page("s", "run", -1, false, None)
        .unwrap()
        .snapshot
        .is_some());
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
fn atomic_retirement_preserves_pricing_sequence_and_leaves_no_pending_rows() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("agent.db");
    let store = fixture(&path, "completed", 1000);
    store.db.call(|db| {
        db.execute("UPDATE run_events SET payload=json_set(payload,'$.event_type','usage','$.data',?1) WHERE idx=500",[json!({"output_tokens":7}).to_string()])?;
        Ok(())
    }).unwrap();
    let pricing = store.pricing_event_payloads("s").unwrap();
    store.compact_run("s", "run").unwrap();
    let raw: i64 = store
        .db
        .call(|db| {
            Ok(db.query_row(
                "SELECT count(*) FROM run_events WHERE run_id='run'",
                [],
                |r| r.get(0),
            )?)
        })
        .unwrap();
    assert_eq!(raw, 0);
    assert_eq!(
        store
            .db
            .call(|db| Ok(db.query_row(
                "SELECT raw_pending FROM run_snapshots WHERE run_id='run'",
                [],
                |r| r.get::<_, i64>(0)
            )?))
            .unwrap(),
        0
    );
    assert!(store.events("s", "run").unwrap().len() < 10);
    assert_eq!(store.pricing_event_payloads("s").unwrap(), pricing);
    let event = SseEvent {
        run_id: "next".into(),
        idx: 0,
        ..SseEvent::new("usage", json!({"output_tokens":8}))
    };
    store
        .append_event("s", serde_json::to_value(event).unwrap())
        .unwrap();
    let ordered = store.pricing_event_payloads("s").unwrap();
    assert_eq!(ordered[0], pricing[0]);
    assert_eq!(ordered.len(), 2);
    let sequence: i64 = store
        .db
        .call(|db| {
            Ok(db.query_row(
                "SELECT sequence FROM run_events WHERE run_id='next'",
                [],
                |r| r.get(0),
            )?)
        })
        .unwrap();
    assert!(sequence > 1002);
    assert_eq!(
        text(
            &store
                .replay_page("s", "run", -1, false, None)
                .unwrap()
                .snapshot
                .unwrap()
        ),
        "中文𠮷".repeat(499) + &"中文𠮷".repeat(500)
    );
}

#[test]
fn pinned_reader_does_not_block_foreground_write_or_destroy_snapshot() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("agent.db");
    let store = fixture(&path, "completed", 1000);
    let reader = rusqlite::Connection::open(&path).unwrap();
    reader
        .execute_batch("BEGIN; SELECT count(*) FROM run_events;")
        .unwrap();
    store.compact_run("s", "run").unwrap();
    assert!(!store
        .db
        .call(super::super::database::reclaim_idle_step)
        .unwrap());
    let event = SseEvent {
        run_id: "next".into(),
        idx: 0,
        ..SseEvent::new("text_chunk", json!({"text":"new"}))
    };
    store
        .append_event("s", serde_json::to_value(event).unwrap())
        .unwrap();
    assert!(store
        .replay_page("s", "run", 123, false, None)
        .unwrap()
        .snapshot
        .is_some());
    reader.execute_batch("ROLLBACK").unwrap();
}

#[test]
fn folder_preserves_reasoning_blocks_and_replacement_tool_fragments() {
    let mut folder = DeltaFolder::bounded();
    for (idx, (kind, data)) in [
        ("thinking_delta", json!({"text":"one","block_id":"a"})),
        ("thinking_delta", json!({"text":"two","block_id":"b"})),
        (
            "tool_delta",
            json!({"text":"old","tool_id":"t","snapshot":true}),
        ),
        (
            "tool_delta",
            json!({"text":"new","tool_id":"t","snapshot":true}),
        ),
        ("tool_end", json!({"tool_id":"t","text":"result"})),
    ]
    .into_iter()
    .enumerate()
    {
        folder.push(SseEvent {
            run_id: "r".into(),
            idx: idx as i64,
            ..SseEvent::new(kind, data)
        });
    }
    let events = folder.finish();
    assert_eq!(events.len(), 5);
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&events[3].data).unwrap()["text"],
        "new"
    );
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

// A real subprocess exit skips Rust destructors, including Transaction::drop.
// Run only this entry point in the child, so the injected fault cannot affect
// concurrently executing parent tests.
#[test]
fn crash_child() {
    let Ok(path) = std::env::var("FUTURE_COMPACTION_CRASH_DB") else {
        return;
    };
    let store = fixture(std::path::Path::new(&path), "completed", 1000);
    store.compact_run("s", "run").unwrap();
    panic!("crash injection did not fire");
}

#[test]
fn process_death_before_and_after_commit_reopens_without_data_loss() {
    for phase in ["after_snapshot", "after_delete", "after_commit"] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("agent.db");
        let child = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "session::journal_compaction::tests::crash_child",
                "--nocapture",
            ])
            .env("FUTURE_COMPACTION_CRASH_DB", &path)
            .env("FUTURE_COMPACTION_CRASH_PHASE", phase)
            .status()
            .unwrap();
        assert_eq!(child.code(), Some(73));
        let store = SqliteStore::open(&path).unwrap();
        let page = store.replay_page("s", "run", -1, false, None).unwrap();
        if phase == "after_commit" {
            assert_eq!(text(&page.snapshot.unwrap()), "中文𠮷".repeat(1000));
        } else {
            assert!(page.snapshot.is_none());
            assert_eq!(page.events.len(), 1002);
            // No automatic recovery compaction; raw paging remains usable.
            assert_eq!(
                store
                    .replay_page("s", "run", 123, false, Some(1))
                    .unwrap()
                    .events
                    .len(),
                1
            );
            continue;
        }
        assert!(store
            .replay_page("s", "run", 123, false, None)
            .unwrap()
            .snapshot
            .is_some());
    }
}
