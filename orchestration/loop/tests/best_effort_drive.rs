//! The last named lines in `store.rs`, `executor.rs` and `run_index.rs`.
//!
//! Each was identified from `cargo llvm-cov report --text` (the per-line export),
//! not from a percentage:
//! - `store.rs:1577` — `verify_ledger` must COUNT a line whose event kind is
//!   unknown to this binary (a newer producer wrote it) instead of reporting it
//!   as corruption: forward compatibility is the whole point of the skip.
//! - `executor.rs:337` — the run-header write is best-effort; when the live log
//!   cannot be opened the turn must still proceed.

mod common;

use common::mock_agent::{completed_events, spawn_mock, MockState};
use common::{cli, cli_root, init_goal, open_store};
use future_loop::store::{Event, Store};

/// `verify_ledger` separates "this binary does not know the kind" from "the line
/// is corrupt". Getting that wrong makes every forward-compatible ledger look
/// damaged, so the unknown kind must be counted and named.
#[test]
fn verify_ledger_counts_an_unknown_event_kind_instead_of_calling_it_corruption() {
    let cr = cli_root();
    let gid = init_goal(&cr, "unknown event kind");
    let mut store = open_store(&cr);
    store
        .append(Event::GoalStarted {
            goal_id: gid.clone(),
            ts: 1,
        })
        .unwrap();
    drop(store);

    // Append a well-formed line whose `kind` this binary does not implement.
    let ledger = std::path::Path::new(&cr.root)
        .join("goals")
        .join(&gid)
        .join("events.jsonl");
    let mut text = std::fs::read_to_string(&ledger).unwrap();
    text.push_str("{\"event_id\":\"evt-future-1\",\"kind\":\"some_future_event\",\"goal_id\":\"");
    text.push_str(&gid);
    text.push_str("\",\"ts\":2}\n");
    std::fs::write(&ledger, text).unwrap();

    // Also a genuinely corrupt line, so the two classifications appear together.
    let mut text = std::fs::read_to_string(&ledger).unwrap();
    text.push_str("this is not json\n");
    std::fs::write(&ledger, text).unwrap();

    let store = Store::open(&cr.root).unwrap();
    let goal_dir = store.goal_dir(&gid);
    let verified = future_loop::store::verify_ledger(
        &goal_dir,
        &gid,
        future_loop::store::EVENT_STORE_SCHEMA_VERSION,
    )
    .unwrap();
    // The report must be serializable and must not claim the ledger is damaged
    // because of the unknown kind.
    let json = serde_json::to_value(&verified).unwrap();
    let rendered = json.to_string();
    assert!(
        rendered.contains("some_future_event"),
        "the unknown kind must be named in the report: {rendered}"
    );
    assert!(
        rendered.contains("not json") || rendered.contains("unparsable"),
        "the corrupt line must still be flagged: {rendered}"
    );
    // A fully-known ledger verifies with no unknown kinds at all.
    let clean = init_goal(&cr, "clean ledger");
    let store2 = open_store(&cr);
    let clean_report = serde_json::to_value(
        future_loop::store::verify_ledger(
            &store2.goal_dir(&clean),
            &clean,
            future_loop::store::EVENT_STORE_SCHEMA_VERSION,
        )
        .unwrap(),
    )
    .unwrap()
    .to_string();
    assert!(
        !clean_report.contains("some_future_event"),
        "a clean ledger must not report unknown kinds: {clean_report}"
    );
}

/// The run-header write into `<root>/runs/<run>.live.jsonl` is explicitly
/// best-effort (`let _ = writeln!`), so a live log that cannot be created must
/// not abort the turn: the run still executes and records its outcome. Making
/// `<root>/runs` a regular FILE is the portable way to deny that write.
#[test]
fn a_run_completes_even_when_the_live_log_cannot_be_created() {
    let cr = cli_root();
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap();
    let (addr, _shared) = rt.block_on(spawn_mock(MockState {
        events: completed_events("mock-run-1"),
        ..Default::default()
    }));
    std::env::set_var("FUTURE_LOOP_AGENT_ADDR", &addr);

    // Block the runs directory with a file of the same name.
    let runs = std::path::Path::new(&cr.root).join("runs");
    std::fs::write(&runs, b"not a directory").unwrap();
    assert!(runs.is_file(), "the fixture must block the runs dir");

    let goal = init_goal(&cr, "unwritable live log");
    let outcome = cli(&["run", "--goal", &goal, "--anonymous", "--max-turns", "2"]);
    // The turn must have RUN (not aborted on the live-log failure) …
    let store = common::open_store(&cr);
    let history = store.replay(&goal).unwrap().unwrap().history;
    assert!(
        !history.is_empty(),
        "the turn must still execute and record its run: {outcome:?}"
    );
    // … and the missing live log must not have turned into a run error.
    assert!(
        history
            .last()
            .unwrap()
            .error
            .as_deref()
            .is_none_or(|e| !e.contains("live")),
        "the live-log failure must not be reported as the run's error: {history:?}"
    );
}
