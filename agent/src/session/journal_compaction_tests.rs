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
    store.bind_legacy_events("s").unwrap();
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
    store.bind_legacy_events("s").unwrap();
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

/// The budget is a target, not a veto: a run whose fold is over it used to be
/// skipped forever (`Ok(0)`), which is exactly how `run_events` grew to 1.4M
/// rows on a real database. It is now published anyway, and the publish is
/// counted so the operator can see that this run's visible content alone is
/// large.
#[test]
fn a_run_larger_than_the_snapshot_budget_is_reclaimed_anyway() {
    // `DeltaFolder` counts a non-coalescable event as `data.len() + 512`, so the
    // fixture needs `n * (data.len() + 512) > SNAPSHOT_BYTE_BUDGET`. **A single
    // larger event does not work**: the reader caps each *stored* payload at its
    // 4 MiB page budget and returns NULL above that (`oversized_event`), and the
    // stored form is the event re-serialized, so it is bigger than `data`. Many
    // events of ~2 MiB keep every payload readable and still cross the budget.
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
        raw as usize,
        "an over-budget run must still be reclaimed"
    );
    assert_eq!(snapshot_rows(&store), 1, "the snapshot must be published");
    let kept: i64 = store
        .db
        .call(|db| Ok(db.query_row("SELECT count(*) FROM run_events", [], |r| r.get(0))?))
        .unwrap();
    assert_eq!(kept, 0, "the raw journal must be retired");
    // The publish is visible as a counter, not just as a log line.
    let stats = store.journal_stats().unwrap();
    assert_eq!(stats["oversizePublishes"], 1);
    assert_eq!(stats["runEvents"]["rows"], 0);
}

/// The same budget applies to the **serialized** payload, a different quantity
/// from the fold estimate: serializing the snapshot embeds each event's `data`
/// string *as a JSON string*, so every escape in it is re-expanded (7/6 growth
/// for escape-heavy text). Two events sit inside the estimate and cross it after
/// serialization — and the run is still reclaimed.
#[test]
fn an_escape_heavy_payload_is_published_even_when_serialization_crosses_the_budget() {
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
    assert_eq!(store.compact_run("s", "run").unwrap(), 4);
    assert_eq!(snapshot_rows(&store), 1);
    let page = store.replay_page("s", "run", -1, false, None).unwrap();
    let snapshot = page.snapshot.expect("published snapshot");
    assert_eq!(snapshot.cursor, 3);
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&snapshot.events[1].data).unwrap()["k"]
            .as_str()
            .map(str::len),
        Some(333_332),
        "no content may be dropped to fit the budget"
    );
}

/// Everything a client can see, reduced without the production fold: the text
/// stream in order, and each tool call with its announced arguments and result.
fn visible(events: &[SseEvent]) -> (String, Vec<(String, String, String)>) {
    let mut text = String::new();
    let mut tools: Vec<(String, String, String)> = Vec::new();
    for event in events {
        let Ok(data) = serde_json::from_str::<serde_json::Value>(&event.data) else {
            continue;
        };
        match event.event_type.as_str() {
            "text_chunk" | "thinking_delta" => {
                if let Some(chunk) = data["text"].as_str() {
                    text.push_str(chunk);
                }
            }
            "tool_start" => {
                let id = data["tool_id"].as_str().unwrap_or_default().to_owned();
                if !tools.iter().any(|(known, _, _)| *known == id) {
                    tools.push((id.clone(), String::new(), String::new()));
                }
                if data["phase"] == "execution" {
                    if let Some(entry) = tools.iter_mut().find(|(known, _, _)| *known == id) {
                        entry.1 = data["tool_args"].to_string();
                    }
                }
            }
            "tool_end" => {
                let id = data["tool_id"].as_str().unwrap_or_default();
                if let Some(entry) = tools.iter_mut().find(|(known, _, _)| known == id) {
                    entry.2 = data["text"].as_str().unwrap_or_default().to_owned();
                }
            }
            _ => {}
        }
    }
    (text, tools)
}

fn replay_all(store: &SqliteStore) -> Vec<SseEvent> {
    let page = store.replay_page("s", "run", -1, false, None).unwrap();
    page.snapshot
        .map(|snapshot| snapshot.events)
        .unwrap_or(page.events)
}

/// The reason reclamation is allowed to exist: it must not change a single
/// message. Everything a client renders — text in order, tool arguments, tool
/// results — is compared across the retirement, through the real replay entry
/// point rather than the fold the compactor happens to use.
#[test]
fn reclamation_replays_exactly_what_the_raw_journal_did() {
    let dir = tempfile::tempdir().unwrap();
    // A tool call written the way the agent emits it: `input` start, streamed
    // fragments, an `execution` start carrying the same parsed arguments, then
    // the result. Plus thinking and a usage event, so every projection kind the
    // snapshot has to keep is present.
    let mut bodies: Vec<(&str, String)> = Vec::new();
    for index in 0..40 {
        bodies.push((
            "text_chunk",
            json!({"text": format!("chunk {index} ")}).to_string(),
        ));
    }
    bodies.push((
        "tool_start",
        json!({"tool_id":"t1","tool_name":"shell","phase":"input","tool_args":""}).to_string(),
    ));
    for fragment in ["{\"path\":", "\"/tmp/x\"", "}"] {
        bodies.push((
            "tool_delta",
            json!({"tool_id":"t1","text":fragment}).to_string(),
        ));
    }
    bodies.push(("thinking_delta", json!({"text":"considering"}).to_string()));
    bodies.push((
        "tool_start",
        json!({"tool_id":"t1","phase":"execution",
               "tool_args": {"path":"/tmp/x"}})
        .to_string(),
    ));
    bodies.push((
        "tool_end",
        json!({"tool_id":"t1","text":"42 lines"}).to_string(),
    ));
    bodies.push(("usage", json!({"output_tokens": 7}).to_string()));

    let store = SqliteStore::open(&dir.path().join("agent.db")).unwrap();
    store.bind_legacy_events("s").unwrap();
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
                .map(|event| serde_json::to_value(event).unwrap())
                .collect(),
        )
        .unwrap();

    let before = visible(&replay_all(&store));
    assert!(
        before.0.contains("chunk 39"),
        "fixture text: {}",
        before.0.len()
    );
    assert_eq!(before.1.len(), 1, "one tool call");
    let retired = store.compact_run("s", "run").unwrap();
    assert!(retired > 0, "the journal must have been reclaimed");
    let after = visible(&replay_all(&store));
    assert_eq!(before.0, after.0, "the text stream changed");
    assert_eq!(before.1, after.1, "a tool call changed");
    // The duplicate argument fragments are what the snapshot is allowed to drop;
    // the parsed arguments they spell out still reach the client from the start.
    let replayed = replay_all(&store);
    assert!(replayed
        .iter()
        .all(|event| event.event_type != "tool_delta"));
    assert!(replayed.iter().any(|event| event.event_type == "tool_end"));
}

/// A late append must never produce a snapshot that disagrees with the journal
/// it replaced. The writer either lands before the commit — and then the build is
/// refused because the source changed — or it is refused because the run is now
/// published. Either way a replay is gap-free and carries every accepted event.
#[test]
fn a_concurrent_append_never_produces_an_inconsistent_snapshot() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("agent.db");
    let store = fixture(&path, "completed", 20_000);
    let last = 20_001i64;
    let writer = SqliteStore::open(&path).unwrap();
    let compacting = store.clone();
    let handle = std::thread::spawn(move || compacting.compact_run("s", "run"));
    std::thread::sleep(std::time::Duration::from_millis(2));
    let appended = writer.append_event(
        "s",
        json!({"run_id":"run","epoch":0,"idx":last + 1,"event_type":"text_chunk",
               "data": json!({"text":"late"}).to_string()}),
    );
    let compacted = handle.join().unwrap();
    assert!(
        compacted.is_ok() || compacted.is_err(),
        "either outcome is legitimate; the replay is what must stay consistent"
    );

    let page = store.replay_page("s", "run", -1, false, None).unwrap();
    let published = page.snapshot.is_some();
    let events = page
        .snapshot
        .map(|snapshot| snapshot.events)
        .unwrap_or(page.events);
    assert_eq!(events.first().map(|event| event.idx), Some(0));
    assert!(
        events.windows(2).all(|pair| pair[1].idx == pair[0].idx + 1),
        "a replayed journal must not have a gap or a duplicate"
    );
    match (published, appended.is_ok()) {
        (true, false) => assert_eq!(
            events.last().map(|event| event.idx),
            Some(last),
            "the published snapshot must cover exactly the journal it replaced"
        ),
        (false, true) => assert_eq!(
            events.last().map(|event| event.idx),
            Some(last + 1),
            "a retained raw journal must still carry the accepted append"
        ),
        (true, true) => assert_eq!(
            events.last().map(|event| event.idx),
            Some(last + 1),
            "a snapshot published after the append must include it"
        ),
        (false, false) => assert_eq!(
            events.last().map(|event| event.idx),
            Some(last),
            "a refused append leaves the raw journal as it was"
        ),
    }
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
