use super::*;
use crate::rpc::SseBroadcaster;
use serde_json::json;

use crate::session::journal_compactor::CompletedRunCompactor;
use std::sync::mpsc;

#[test]
fn obsolete_snapshot_cleanup_state_is_removed_without_rewriting_existing_payloads() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("agent.db");
    let store = fixture(&path, "completed", 1000);
    store.compact_run("s", "run").unwrap();
    let original: String = store.db.call(|db| {
        let count: i64 = db.query_row(
            "SELECT count(*) FROM pragma_table_info('run_snapshots') WHERE name='raw_pending'",
            [], |r|r.get(0))?;
        assert_eq!(count, 0, "new databases have no retired field");
        let payload: String = db.query_row("SELECT payload FROM run_snapshots",[],|r|r.get(0))?;
        db.execute_batch("ALTER TABLE run_snapshots ADD COLUMN raw_pending INTEGER NOT NULL DEFAULT 0;
            UPDATE run_snapshots SET raw_pending=1;
            CREATE INDEX run_snapshots_cleanup ON run_snapshots(session_id,run_id) WHERE raw_pending=1;
            CREATE TRIGGER forbid_snapshot_update BEFORE UPDATE ON run_snapshots BEGIN SELECT RAISE(ABORT,'snapshot rewritten'); END;
            CREATE TRIGGER forbid_snapshot_delete BEFORE DELETE ON run_snapshots BEGIN SELECT RAISE(ABORT,'snapshot deleted'); END;
            CREATE TRIGGER forbid_snapshot_insert BEFORE INSERT ON run_snapshots BEGIN SELECT RAISE(ABORT,'snapshot inserted'); END;
            PRAGMA user_version=5;")?;
        Ok(payload)
    }).unwrap();
    drop(store);
    let store = SqliteStore::open(&path).unwrap();
    let (version, index_count, payload, legacy_flag) = store
        .db
        .call(|db| {
            Ok((
                db.pragma_query_value(None, "user_version", |r| r.get::<_, i64>(0))?,
                db.query_row(
                    "SELECT count(*) FROM sqlite_schema WHERE name='run_snapshots_cleanup'",
                    [],
                    |r| r.get::<_, i64>(0),
                )?,
                db.query_row("SELECT payload FROM run_snapshots", [], |r| {
                    r.get::<_, String>(0)
                })?,
                db.query_row("SELECT raw_pending FROM run_snapshots", [], |r| {
                    r.get::<_, i64>(0)
                })?,
            ))
        })
        .unwrap();
    assert_eq!(version, crate::session::database::CURRENT_SCHEMA_VERSION);
    assert_eq!(index_count, 0);
    assert_eq!(payload, original);
    assert_eq!(
        legacy_flag, 1,
        "old columns are left intact rather than copying the table"
    );
    assert_eq!(
        text(
            &store
                .replay_page("s", "run", -1, false, None)
                .unwrap()
                .snapshot
                .unwrap()
        ),
        "中文𠮷".repeat(1000)
    );
}

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

fn text(snapshot: &RunJournalSnapshot) -> String {
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

#[path = "journal_compaction_tests/replay.rs"]
mod replay;
#[path = "journal_compaction_tests/scheduling.rs"]
mod scheduling;
#[path = "journal_compaction_tests/transactions.rs"]
mod transactions;
#[path = "journal_compaction_tests/validation.rs"]
mod validation;
