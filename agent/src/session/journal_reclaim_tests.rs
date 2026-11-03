use super::*;
use crate::rpc::SseEvent;
use serde_json::json;
use std::time::Duration;

/// One settled run of `chunks` text events plus its terminal, and a raw journal
/// to reclaim.
fn settled_run(store: &SqliteStore, run: &str, chunks: usize) {
    store.bind_legacy_events("s").unwrap();
    let run_owned = run.to_owned();
    store
        .db
        .call(move |db| {
            db.execute(
                "INSERT OR IGNORE INTO runs(session_id,run_id,status,completed_at_ms)
                 VALUES ('s',?1,'completed',1)",
                [&run_owned],
            )?;
            Ok(())
        })
        .unwrap();
    let mut events = vec![SseEvent {
        run_id: run.into(),
        idx: 0,
        ..SseEvent::new("agent_start", json!({}))
    }];
    for index in 1..=chunks {
        events.push(SseEvent {
            run_id: run.into(),
            idx: index as i64,
            ..SseEvent::new("text_chunk", json!({"text": "x"}))
        });
    }
    events.push(SseEvent {
        run_id: run.into(),
        idx: chunks as i64 + 1,
        ..SseEvent::new("agent_end", json!({"state": "completed"}))
    });
    store
        .append_events(
            "s",
            events
                .into_iter()
                .map(|event| serde_json::to_value(event).unwrap())
                .collect(),
        )
        .unwrap();
}

fn raw_rows(store: &SqliteStore) -> i64 {
    store
        .db
        .call(|db| Ok(db.query_row("SELECT count(*) FROM run_events", [], |r| r.get(0))?))
        .unwrap()
}

/// Reporting is what makes "the database keeps growing" diagnosable. It must see
/// the pending work, the largest run, and the durable counters.
#[test]
fn stats_report_rows_the_largest_run_and_durable_evidence() {
    let dir = tempfile::tempdir().unwrap();
    let store = SqliteStore::open(&dir.path().join("agent.db")).unwrap();
    settled_run(&store, "big", 300);
    settled_run(&store, "small", 5);
    let stats = store.journal_stats().unwrap();
    assert_eq!(stats["runEvents"]["rows"], 309);
    assert!(stats["runEvents"]["bytes"].as_i64().unwrap() > 0);
    assert_eq!(stats["largestRun"]["runId"], "big");
    assert_eq!(stats["largestRun"]["events"], 302);
    assert_eq!(stats["settledUncompacted"]["runs"], 2);
    assert_eq!(stats["settledUncompacted"]["events"], 309);
    assert_eq!(stats["reclaimed"]["runs"], 0);
    assert_eq!(stats["reclaimed"]["atMs"], 0, "never reclaimed yet");
    // Free pages and database size are both reported: a retired journal that has
    // not been folded back yet must not look like a failed reclamation.
    assert!(stats["database"]["bytes"].as_i64().unwrap() > 0);
    assert!(stats["database"]["freeBytes"].as_i64().unwrap() >= 0);
}

/// The sweep is off unless asked for: a plan deletes nothing and reports the
/// same numbers the applied pass would.
#[test]
fn a_plan_deletes_nothing_and_promises_the_same_work() {
    let dir = tempfile::tempdir().unwrap();
    let store = SqliteStore::open(&dir.path().join("agent.db")).unwrap();
    settled_run(&store, "run", 10);
    let plan = store
        .reclaim_settled_journals(Duration::from_secs(5), true)
        .unwrap();
    assert_eq!(plan["dryRun"], true);
    assert_eq!(plan["reclaimedRuns"], 1);
    assert_eq!(plan["reclaimedEvents"], 12);
    assert_eq!(plan["entries"].as_array().unwrap().len(), 1);
    assert_eq!(plan["hasMore"], false);
    assert!(
        plan["quickCheck"].is_null(),
        "a plan does not check integrity"
    );
    assert_eq!(raw_rows(&store), 12, "a plan must not delete rows");
    assert_eq!(store.journal_stats().unwrap()["reclaimed"]["runs"], 0);
}

/// The applied pass retires the raw journal, keeps the run replayable, records
/// the durable evidence and gates on `quick_check`.
#[test]
fn the_sweep_retires_settled_journals_and_checks_integrity() {
    let dir = tempfile::tempdir().unwrap();
    let store = SqliteStore::open(&dir.path().join("agent.db")).unwrap();
    settled_run(&store, "run", 40);
    let report = store
        .reclaim_settled_journals(Duration::from_secs(30), false)
        .unwrap();
    assert_eq!(report["dryRun"], false);
    assert_eq!(report["reclaimedRuns"], 1);
    assert_eq!(report["reclaimedEvents"], 42);
    assert_eq!(report["hasMore"], false);
    assert_eq!(report["quickCheck"], "ok");
    assert_eq!(raw_rows(&store), 0);
    // The run is still replayable: a snapshot replaced the raw events.
    let page = store.replay_page("s", "run", -1, false, None).unwrap();
    let snapshot = page.snapshot.expect("published snapshot");
    assert_eq!(snapshot.cursor, 41);
    assert_eq!(
        snapshot
            .events
            .iter()
            .filter(|event| event.event_type == "text_chunk")
            .count(),
        1,
        "40 single-character chunks coalesce into one bounded segment"
    );
    let stats = store.journal_stats().unwrap();
    assert_eq!(stats["reclaimed"]["runs"], 1);
    assert_eq!(stats["reclaimed"]["events"], 42);
    assert!(stats["reclaimed"]["atMs"].as_i64().unwrap() > 0);
    assert_eq!(stats["runEvents"]["rows"], 0);
    assert_eq!(stats["settledUncompacted"]["runs"], 0);
}

/// A retirement interrupted between slices leaves raw rows that a published
/// snapshot already covers. They are pure waste, and the sweep must retire them
/// from the *leftover* path rather than waiting for a rebuild that never happens
/// (the run already has a snapshot, so it is not a candidate).
#[test]
fn leftover_rows_of_an_interrupted_retirement_are_retired() {
    let dir = tempfile::tempdir().unwrap();
    let store = SqliteStore::open(&dir.path().join("agent.db")).unwrap();
    settled_run(&store, "run", 5);
    store.compact_run("s", "run").unwrap();
    // Recreate the state a crash between slices leaves: published + raw rows.
    store
        .db
        .call(|db| {
            db.execute(
                "INSERT INTO run_events(session_id,run_id,epoch,idx,payload)
                 VALUES ('s','run',0,99,'{}')",
                [],
            )?;
            Ok(())
        })
        .unwrap();
    assert_eq!(raw_rows(&store), 1);
    let report = store
        .reclaim_settled_journals(Duration::from_secs(30), false)
        .unwrap();
    assert_eq!(report["leftoverRuns"], 1);
    assert_eq!(report["leftoverEvents"], 1);
    assert_eq!(raw_rows(&store), 0);
    assert_eq!(report["quickCheck"], "ok");
    // And the snapshot still answers the replay.
    assert!(store
        .replay_page("s", "run", -1, false, None)
        .unwrap()
        .snapshot
        .is_some());
}

/// A running session's journal is not work: its producer may still append, so
/// neither the plan nor the applied pass touches it.
#[test]
fn a_run_that_has_not_settled_is_never_a_candidate() {
    let dir = tempfile::tempdir().unwrap();
    let store = SqliteStore::open(&dir.path().join("agent.db")).unwrap();
    settled_run(&store, "run", 4);
    store
        .db
        .call(|db| {
            db.execute("UPDATE runs SET status='running' WHERE run_id='run'", [])?;
            Ok(())
        })
        .unwrap();
    let report = store
        .reclaim_settled_journals(Duration::from_secs(30), false)
        .unwrap();
    assert_eq!(report["entries"].as_array().unwrap().len(), 0);
    assert_eq!(report["hasMore"], false);
    assert_eq!(raw_rows(&store), 6, "the live journal is untouched");
}

/// A journal the snapshot cannot represent must not end the sweep: it is
/// reported per run, counted durably, and the next candidate is still reclaimed.
#[test]
fn an_unrepresentable_journal_is_reported_and_the_sweep_continues() {
    let dir = tempfile::tempdir().unwrap();
    let store = SqliteStore::open(&dir.path().join("agent.db")).unwrap();
    settled_run(&store, "bad", 3);
    settled_run(&store, "good", 3);
    store
        .db
        .call(|db| {
            db.execute(
                "UPDATE run_events SET payload=json_set(payload,'$.future_field','keep')
                 WHERE run_id='bad' AND idx=1",
                [],
            )?;
            Ok(())
        })
        .unwrap();
    let report = store
        .reclaim_settled_journals(Duration::from_secs(30), false)
        .unwrap();
    let entries = report["entries"].as_array().unwrap();
    assert_eq!(entries.len(), 2);
    let failed = entries
        .iter()
        .find(|entry| entry["runId"] == "bad")
        .unwrap();
    assert_eq!(failed["skip"], "error");
    assert!(failed["error"]
        .as_str()
        .unwrap()
        .contains("unknown event envelope"));
    let reclaimed = entries
        .iter()
        .find(|entry| entry["runId"] == "good")
        .unwrap();
    assert_eq!(reclaimed["skip"], Value::Null);
    let stats = store.journal_stats().unwrap();
    assert_eq!(stats["skipped"]["error"], 1);
    assert_eq!(stats["settledUncompacted"]["runs"], 1, "only 'bad' is left");
    // The refused journal is untouched and still replayable raw.
    let page = store.replay_page("s", "bad", -1, false, None).unwrap();
    assert!(page.snapshot.is_none());
    assert_eq!(page.events.len(), 5);
}

/// A short work budget ends the call with `hasMore`, so a caller keeps looping
/// instead of holding the shared SQLite worker for the whole database — but the
/// first candidate always runs, so a call never reports `hasMore` with nothing
/// done (that would spin forever on a database whose scan outlived the budget).
#[test]
fn the_work_budget_ends_a_call_after_at_least_one_run() {
    let dir = tempfile::tempdir().unwrap();
    let store = SqliteStore::open(&dir.path().join("agent.db")).unwrap();
    settled_run(&store, "one", 3);
    settled_run(&store, "two", 3);
    let report = store
        .reclaim_settled_journals(Duration::ZERO, false)
        .unwrap();
    assert_eq!(report["hasMore"], true, "the second run is still queued");
    assert_eq!(report["reclaimedRuns"], 1);
    assert!(report["quickCheck"].is_null(), "not finished yet");
    assert_eq!(raw_rows(&store), 5, "one journal was retired, one was not");
    let rest = store
        .reclaim_settled_journals(Duration::from_secs(30), false)
        .unwrap();
    assert_eq!(rest["hasMore"], false);
    assert_eq!(rest["quickCheck"], "ok");
    assert_eq!(raw_rows(&store), 0);
}

/// A run the snapshot cannot represent stays a candidate forever, so `hasMore`
/// must not answer `true` for it: a caller would loop on the same refusal instead
/// of finishing the sweep (measured: 1,042 identical refusals of the same two
/// runs before this rule).
#[test]
fn an_unretirable_run_does_not_keep_the_sweep_looping() {
    let dir = tempfile::tempdir().unwrap();
    let store = SqliteStore::open(&dir.path().join("agent.db")).unwrap();
    settled_run(&store, "stuck", 3);
    store
        .db
        .call(|db| {
            // The last event is not `agent_end`, and the run settled just now, so
            // its producer may still append that tail: inside the grace window
            // nothing may retire it, and the sweep must not loop on it either.
            db.execute("DELETE FROM run_events WHERE run_id='stuck' AND idx=4", [])?;
            db.execute(
                "UPDATE runs SET completed_at_ms=?1 WHERE run_id='stuck'",
                [crate::session::journal_compaction::now_ms()],
            )?;
            Ok(())
        })
        .unwrap();
    let first = store
        .reclaim_settled_journals(Duration::from_secs(30), false)
        .unwrap();
    assert_eq!(first["entries"][0]["skip"], "not_terminal");
    assert_eq!(first["hasMore"], false, "nothing else can be done now");
    let second = store
        .reclaim_settled_journals(Duration::from_secs(30), false)
        .unwrap();
    assert_eq!(second["reclaimedRuns"], 0);
    assert_eq!(second["hasMore"], false);
}

#[test]
fn explicit_legacy_reclaim_never_rewrites_canonical_conversations() {
    let dir = tempfile::tempdir().unwrap();
    let store = SqliteStore::open(&dir.path().join("agent.db")).unwrap();
    store.reserve_segmented("s").unwrap();
    store.replace("s",vec![json!({"id":"user","type":"user","role":"user","timestamp":"2026-10-26T00:00:00Z","content":"preserved🙂".repeat(20000)})]).unwrap();
    let before = store.entries("s").unwrap();
    let outcome = store
        .compact_run_cancellable("s", "run", &AtomicBool::new(false))
        .unwrap();
    assert_eq!(
        outcome.skip.map(|reason| reason.as_str()),
        Some("canonical_format")
    );
    assert_eq!(outcome.deleted, 0);
    assert_eq!(store.entries("s").unwrap(), before);
    assert_eq!(store.journal_stats().unwrap()["runSnapshots"]["rows"], 0);
}

/// The other half of that rule: the same truncated run, once its settle time is
/// older than the grace window, is exactly what the sweep should reclaim. This
/// is the 50-run / 762k-event (179 MiB) leftover the first measurement found.
#[test]
fn the_sweep_folds_a_truncated_stream_once_its_run_has_settled() {
    let dir = tempfile::tempdir().unwrap();
    let store = SqliteStore::open(&dir.path().join("agent.db")).unwrap();
    settled_run(&store, "crash", 40);
    store
        .db
        .call(|db| {
            db.execute("DELETE FROM run_events WHERE run_id='crash' AND idx=41", [])?;
            db.execute(
                "UPDATE runs SET status='interrupted' WHERE run_id='crash'",
                [],
            )?;
            Ok(())
        })
        .unwrap();
    let report = store
        .reclaim_settled_journals(Duration::from_secs(30), false)
        .unwrap();
    assert_eq!(report["reclaimedRuns"], 1);
    assert_eq!(raw_rows(&store), 0);
    assert_eq!(
        report["entries"][0]["skip"],
        serde_json::Value::Null,
        "kept means the run was refused; this one had to be folded"
    );
    let stats = store.journal_stats().unwrap();
    assert_eq!(stats["truncatedPublishes"], 1);
}

/// The startup catch-up is the reason a store no longer drifts: it folds what a
/// crash or a stop left behind without anybody asking. Its bounds are the point,
/// so this asserts both — the plan stops at `max_runs`, the leftovers are
/// announced with `hasMore`, and a second pass continues.
#[test]
fn the_startup_catch_up_is_bounded_and_resumable() {
    let dir = tempfile::tempdir().unwrap();
    let store = SqliteStore::open(&dir.path().join("agent.db")).unwrap();
    store.bind_legacy_events("s").unwrap();
    for run in ["r0", "r1", "r2", "r3", "r4", "r5"] {
        settled_run(&store, run, 2);
    }
    let first = store
        .catch_up_settled_journals(2, Duration::from_secs(30))
        .unwrap();
    assert_eq!(first.folded, 2, "max_runs is a hard bound");
    assert!(first.has_more, "the rest is the next start's work");
    assert_eq!(raw_rows(&store), 16, "four runs still hold their journal");

    let second = store
        .catch_up_settled_journals(2, Duration::from_secs(30))
        .unwrap();
    assert_eq!(second.folded, 2);
    assert!(second.has_more);

    let third = store
        .catch_up_settled_journals(2, Duration::from_secs(30))
        .unwrap();
    assert_eq!(third.folded, 2);
    assert!(!third.has_more, "the backlog is drained");
    assert_eq!(raw_rows(&store), 0);
    assert_eq!(third.errors, 0);
    assert!(
        third.refused.is_empty(),
        "nothing was kept: {:?}",
        third.refused
    );

    // A drained store says so instead of folding anything on the next start.
    let idle = store
        .catch_up_settled_journals(2, Duration::from_secs(30))
        .unwrap();
    assert_eq!(idle.examined, 0);
    assert_eq!(idle.folded, 0);
}

/// A catch-up must not spend its whole budget on runs it has to refuse — the
/// refusal is decided from one indexed read, before any fold is built.
#[test]
fn the_startup_catch_up_reports_why_it_kept_a_journal() {
    let dir = tempfile::tempdir().unwrap();
    let store = SqliteStore::open(&dir.path().join("agent.db")).unwrap();
    settled_run(&store, "fresh", 3);
    store
        .db
        .call(|db| {
            db.execute("DELETE FROM run_events WHERE run_id='fresh' AND idx=4", [])?;
            db.execute(
                "UPDATE runs SET completed_at_ms=?1 WHERE run_id='fresh'",
                [crate::session::journal_compaction::now_ms()],
            )?;
            Ok(())
        })
        .unwrap();
    let report = store
        .catch_up_settled_journals(4, Duration::from_secs(30))
        .unwrap();
    assert_eq!(report.folded, 0);
    assert_eq!(report.examined, 1);
    assert_eq!(report.refused, vec![("not_terminal".to_string(), 1)]);
    assert_eq!(raw_rows(&store), 4, "the journal is untouched");
}

/// The sweep's whole purpose is that the raw journal is the *only* thing it
/// removes: the conversation a reader sees is `entries` plus `message_blocks`,
/// and a reclamation that touched either would be data loss dressed as
/// maintenance. Measured on a real 4.4 GiB store, all 337 transcripts were
/// byte-identical across a sweep; this pins the same invariant in the store.
#[test]
fn a_sweep_leaves_the_conversation_untouched() {
    let dir = tempfile::tempdir().unwrap();
    let store = SqliteStore::open(&dir.path().join("agent.db")).unwrap();
    settled_run(&store, "run", 40);
    store
        .db
        .call(|db| {
            db.execute(
                "INSERT INTO entries(session_id,position,entry_id,entry_type,role,run_id,
                                     timestamp_ms,metadata_json,content_json)
                 VALUES ('s',0,'e0','message','user','run',1,'{}','{\"text\":\"hi\"}')",
                [],
            )?;
            db.execute(
                "INSERT INTO message_blocks(session_id,entry_position,ordinal,kind,text,
                                            metadata_json)
                 VALUES ('s',0,0,'text','hi','{}')",
                [],
            )?;
            Ok(())
        })
        .unwrap();
    let conversation = |db: &mut rusqlite::Connection| -> anyhow::Result<(i64, String, String)> {
        Ok((
            db.query_row("SELECT count(*) FROM entries", [], |row| row.get(0))?,
            db.query_row("SELECT content_json FROM entries", [], |row| row.get(0))?,
            db.query_row("SELECT text FROM message_blocks", [], |row| row.get(0))?,
        ))
    };
    let before = store.db.call(conversation).unwrap();
    let report = store
        .reclaim_settled_journals(Duration::from_secs(30), false)
        .unwrap();
    assert_eq!(report["reclaimedRuns"], 1);
    assert_eq!(raw_rows(&store), 0);
    assert_eq!(store.db.call(conversation).unwrap(), before);
}
