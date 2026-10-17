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

/// One run whose raw events are exactly `bodies`, closed with `agent_end`.
/// `bodies` are `(event_type, data)` pairs at idx 1.. so the fixture ends with
/// a proper terminal and every idx stays contiguous (compaction revalidates the
/// count it read).
fn run_with_bodies(path: &std::path::Path, bodies: Vec<(&str, String)>) -> SqliteStore {
    let store = SqliteStore::open(path).unwrap();
    store.bind_events("s").unwrap();
    store
        .db
        .call(|db| {
            db.execute(
                "INSERT INTO runs(session_id,run_id,status,completed_at_ms) VALUES ('s','run','completed',1)",
                [],
            )?;
            Ok(())
        })
        .unwrap();
    let mut events = vec![SseEvent {
        run_id: "run".into(),
        idx: 0,
        ..SseEvent::new("agent_start", json!({}))
    }];
    for (index, (kind, data)) in bodies.into_iter().enumerate() {
        let mut event = SseEvent::new(kind, json!({}));
        event.data = data;
        event.run_id = "run".into();
        event.idx = index as i64 + 1;
        events.push(event);
    }
    let mut terminal = SseEvent::new("agent_end", json!({"state": "completed"}));
    terminal.run_id = "run".into();
    terminal.idx = events.len() as i64;
    events.push(terminal);
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

fn snapshot_rows(store: &SqliteStore) -> i64 {
    store
        .db
        .call(|db| Ok(db.query_row("SELECT count(*) FROM run_snapshots", [], |r| r.get(0))?))
        .unwrap()
}

/// One run-encoded event whose `data` is valid JSON but **escape-heavy**: a
/// string of control characters, each of which a JSON serializer writes as the
/// six-byte `\u0001`. This is what separates the two quantities the two budget
/// checks measure.
fn escape_heavy_data(control_chars: usize) -> String {
    json!({ "k": "\u{1}".repeat(control_chars) }).to_string()
}

/// The folded projection's own size check: a run whose events add up to more
/// than `SNAPSHOT_BYTE_BUDGET` is left as a raw journal. The check runs per push,
/// so the arm fires on the event that crosses the budget.
#[test]
fn a_run_larger_than_the_snapshot_budget_keeps_its_raw_journal() {
    // `DeltaFolder` counts a non-coalescable event as `data.len() + 512`, so the
    // fixture needs `n * (data.len() + 512) > SNAPSHOT_BYTE_BUDGET`. **A single
    // larger event does not work**: the reader caps each *stored* payload at its
    // 4 MiB page budget and returns NULL (an earlier `Ok(0)`) above that, and the
    // stored form is the event re-serialized, so it is bigger than `data`. Many
    // events of ~2 MiB keep every payload readable and still cross the estimate.
    let data = escape_heavy_data(333_332);
    let per_event = data.len() + 512;
    let events = SNAPSHOT_BYTE_BUDGET / per_event + 1;
    assert!(
        per_event * events > SNAPSHOT_BYTE_BUDGET,
        "the fixture must cross the budget: {per_event} x {events}"
    );
    let dir = tempfile::tempdir().unwrap();
    let store = run_with_bodies(
        &dir.path().join("agent.db"),
        (0..events)
            .map(|_| ("provider_payload", data.clone()))
            .collect(),
    );
    let raw: i64 = store
        .db
        .call(|db| Ok(db.query_row("SELECT count(*) FROM run_events", [], |r| r.get(0))?))
        .unwrap();
    assert!(raw > 0, "the fixture has raw events to retire");

    assert_eq!(
        store.compact_run("s", "run").unwrap(),
        0,
        "an over-budget run must be left uncompacted"
    );
    assert_eq!(snapshot_rows(&store), 0, "nothing may be published");
    let kept: i64 = store
        .db
        .call(|db| Ok(db.query_row("SELECT count(*) FROM run_events", [], |r| r.get(0))?))
        .unwrap();
    assert_eq!(kept, raw, "the raw journal must be untouched");
}

/// The **serialized** snapshot's size check, a different quantity from the
/// estimate above: `DeltaFolder::estimated_bytes` counts an event's `data` string
/// once, while serializing the snapshot embeds that string *as a JSON string*, so
/// every escape in it is re-expanded. Two escape-heavy events sit comfortably
/// inside the estimate's budget and blow past the serializer's.
#[test]
fn an_escape_heavy_payload_can_exceed_the_budget_after_serialization() {
    // Each control character is already six bytes in `data`, and embedding that
    // string in the snapshot escapes its backslashes again (7/6 growth) — enough
    // to cross the budget while the estimate, which counts `data` once, stays
    // inside it. The window is roughly 1.8-2.09 MiB per event.
    let data = escape_heavy_data(333_332);
    let per_event = data.len() + 512;
    assert!(
        per_event * 2 <= SNAPSHOT_BYTE_BUDGET,
        "the estimate must stay inside the budget for two events: {}",
        per_event * 2
    );
    let reserialized = serde_json::to_string(&data).unwrap().len();
    assert!(
        reserialized * 2 > SNAPSHOT_BYTE_BUDGET,
        "the serialized pair must exceed the budget: {}",
        reserialized * 2
    );

    let dir = tempfile::tempdir().unwrap();
    let store = run_with_bodies(
        &dir.path().join("agent.db"),
        vec![
            ("provider_payload", data.clone()),
            ("provider_payload", data),
        ],
    );
    assert_eq!(
        store.compact_run("s", "run").unwrap(),
        0,
        "an over-budget serialized snapshot must not be published"
    );
    assert_eq!(snapshot_rows(&store), 0);
    let kept: i64 = store
        .db
        .call(|db| Ok(db.query_row("SELECT count(*) FROM run_events", [], |r| r.get(0))?))
        .unwrap();
    assert!(kept > 0, "the raw journal is preserved");
}

/// A raw provider delta that is **not** the plain `{text, type}` shape an old
/// journal is allowed to collapse: its `text` is not a string, so the compactor
/// cannot prove the delta duplicates the authoritative text stream and must keep
/// the whole raw journal. This is the second half of that proof — the key-set
/// half is the `&=` on the lines above.
#[test]
fn a_provider_delta_without_a_text_field_keeps_its_raw_journal() {
    let dir = tempfile::tempdir().unwrap();
    let store = run_with_bodies(
        &dir.path().join("agent.db"),
        vec![
            // A plain visible chunk, so the run is otherwise representable.
            ("text_chunk", json!({"text": "visible"}).to_string()),
            // The delta has a `text` key whose value is not a string: the keys
            // pass the shape test, the value fails the text test.
            ("text_delta", json!({"text": 5}).to_string()),
        ],
    );
    let raw: i64 = store
        .db
        .call(|db| Ok(db.query_row("SELECT count(*) FROM run_events", [], |r| r.get(0))?))
        .unwrap();

    assert_eq!(
        store.compact_run("s", "run").unwrap(),
        0,
        "an unprovable provider delta must keep the raw journal"
    );
    assert_eq!(snapshot_rows(&store), 0, "nothing may be published");
    let kept: i64 = store
        .db
        .call(|db| Ok(db.query_row("SELECT count(*) FROM run_events", [], |r| r.get(0))?))
        .unwrap();
    assert_eq!(kept, raw, "every raw event must survive");
}

/// The sibling shape: the delta's keys include one outside `{text, type}`, so the
/// key-set test itself refuses it. Both halves of the guard are reachable and
/// both keep the raw journal.
#[test]
fn a_provider_delta_with_a_foreign_key_keeps_its_raw_journal() {
    let dir = tempfile::tempdir().unwrap();
    let store = run_with_bodies(
        &dir.path().join("agent.db"),
        vec![
            ("text_chunk", json!({"text": "visible"}).to_string()),
            (
                "text_delta",
                json!({"text": "x", "provider_metadata": {"v": 1}}).to_string(),
            ),
        ],
    );
    assert_eq!(
        store.compact_run("s", "run").unwrap(),
        0,
        "a foreign key must keep the raw journal"
    );
    assert_eq!(snapshot_rows(&store), 0);
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
