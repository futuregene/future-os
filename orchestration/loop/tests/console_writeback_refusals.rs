//! `console.rs`'s remaining reachable failure arms: the projection's best-effort
//! warning, and the `worker stop --all` release loop.
//!
//! Both are documented-behaviour paths that a "happy path" drive never reaches: the
//! projection fails only when the state directory is unusable, and `--all` is the
//! broadcast form of a stop.

mod common;

use common::mock_agent::{spawn_mock, MockState};
use common::{cli, cli_ok, cli_root, first_todo_id, init_goal, open_store};

fn rt() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap()
}

/// Plant a DIRECTORY where `ACTIVE_GOAL_STATE.md` belongs, so `fs::write` fails.
/// Portability note: no permissions model is involved, so this behaves identically on
/// Windows and on a root-owned Linux CI (a read-only file would not).
fn wedge_the_projection(root: &str) {
    let goals = std::path::Path::new(root).join("goals");
    for entry in std::fs::read_dir(&goals)
        .expect("the goal dir must exist after `goal init`")
        .flatten()
    {
        let target = entry.path().join("ACTIVE_GOAL_STATE.md");
        if target.exists() {
            std::fs::remove_file(&target).unwrap();
        }
        std::fs::create_dir(&target).expect("plant a directory where the projection belongs");
    }
}

/// The projection write is best-effort by contract: a mutating command must still
/// succeed (and warn) when `ACTIVE_GOAL_STATE.md` cannot be written. Making that
/// failure fatal would mean an unusable state directory blocks every todo mutation.
///
/// Also asserts the second half of the contract: the LEDGER write is not affected, so
/// the command's real effect survives even though the human-readable projection did not.
#[test]
fn a_mutating_command_succeeds_and_warns_when_the_projection_cannot_be_written() {
    let cr = cli_root();
    let goal = init_goal(&cr, "projection unwritable");
    wedge_the_projection(&cr.root);

    // Each of these ends with `refresh_next_action` + `sync_compat`.
    cli_ok(&["todo", "add", "--goal", &goal, "--text", "still works"]);

    let store = open_store(&cr);
    let state = store.replay(&goal).unwrap().unwrap();
    assert!(
        state.todos.iter().any(|t| t.text == "still works"),
        "a projection failure must not block the ledger write"
    );
    // And the wedge is still in place, so the failure really did happen (rather than
    // the write having quietly succeeded somewhere else).
    let wedged = std::path::Path::new(&cr.root)
        .join("goals")
        .join(&goal)
        .join("ACTIVE_GOAL_STATE.md");
    assert!(
        wedged.is_dir(),
        "the fixture must still block the projection path"
    );
}

/// `goal cancel` stops the goal's workers through `stop_goal_workers`, which is the
/// SECOND caller of `abort_worker_sessions` and `release_leases_of_stopped` (the first
/// being `worker stop`). Two properties only this path has:
///
/// * with no live sessions it still calls `abort_worker_sessions` with an empty target
///   list (unlike `worker stop`, which returns before the call), so the empty-input
///   arm is reached; and
/// * a cancelled goal's stopped workers release their claims here, not in
///   `cmd_worker_stop`.
#[test]
fn goal_cancel_stops_workers_and_releases_their_claims() {
    let cr = cli_root();
    let goal = init_goal(&cr, "cancel stops workers");
    cli_ok(&["agent", "register", "--goal", &goal, "--agent-id", "wc"]);

    // ── No live session: the empty-targets arm. ─────────────────────────────
    cli_ok(&[
        "goal",
        "cancel",
        "--goal",
        &goal,
        "--reason",
        "no workers yet",
    ]);
    let store = open_store(&cr);
    assert_eq!(
        store.replay(&goal).unwrap().unwrap().status,
        "cancelled",
        "the cancel must still land"
    );

    // ── With a live session holding a lease: the release arm. ──────────────
    let goal2 = init_goal(&cr, "cancel a busy worker");
    cli_ok(&["agent", "register", "--goal", &goal2, "--agent-id", "wc2"]);
    let bootstrap = first_todo_id(&cr.root, &goal2);
    cli_ok(&[
        "todo",
        "supersede",
        "--goal",
        &goal2,
        "--todo-id",
        &bootstrap,
        "--reason",
        "fixture",
    ]);
    cli_ok(&["todo", "add", "--goal", &goal2, "--text", "held by wc2"]);
    let store = open_store(&cr);
    let todo = store
        .replay(&goal2)
        .unwrap()
        .unwrap()
        .todos
        .iter()
        .find(|t| t.text == "held by wc2")
        .unwrap()
        .id
        .clone();
    drop(store);
    cli_ok(&[
        "lease",
        "claim",
        "--goal",
        &goal2,
        "--todo-id",
        &todo,
        "--agent-id",
        "wc2",
        "--lease-secs",
        "600",
    ]);
    let runs = std::path::Path::new(&cr.root).join("runs");
    std::fs::create_dir_all(&runs).unwrap();
    let header = serde_json::json!({
        "type": "run_header",
        "wall_ts": 1_700_000_000u64,
        "run_id": "run-wc2",
        "session_id": "sess-wc2",
        "agent_id": "wc2",
        "todo_id": todo,
        "goal_id": goal2,
    });
    std::fs::write(runs.join("run_wc2.live.jsonl"), format!("{header}\n")).unwrap();

    let rt = rt();
    let (addr, _state) = rt.block_on(spawn_mock(MockState::default()));
    std::env::set_var("FUTURE_LOOP_AGENT_ADDR", &addr);

    cli_ok(&[
        "goal",
        "cancel",
        "--goal",
        &goal2,
        "--reason",
        "stop it all",
    ]);

    let store = open_store(&cr);
    let after = store.replay(&goal2).unwrap().unwrap();
    assert!(
        after.todo(&todo).unwrap().claimed_by.is_none(),
        "cancelling a goal must release its stopped worker's claim: {:?}",
        after.todo(&todo)
    );
    let released = store
        .events(&goal2)
        .unwrap()
        .into_iter()
        .filter(|e| matches!(e.event, future_loop::store::Event::TodoReleased { .. }))
        .count();
    assert_eq!(released, 1, "one release through the cancel path");
}
/// A run that mints a session but never reaches a prompt must DISCARD that session:
/// nothing references it, yet it still appears in `session list` / the desktop sidebar
/// under the goal objective, and no later pass heals it (a session with no entries
/// never reappears in `list_sessions`). The delete is best-effort by contract - a
/// failure must be reported and must NOT fail the run.
///
/// The same run also reaches the retention-persist arm: it ends by writing
/// `ACTIVE_GOAL_STATE.md`, and the wedge makes that write fail.
#[test]
fn a_run_that_never_prompts_discards_its_session_and_survives_a_persist_failure() {
    let cr = cli_root();
    let goal = init_goal(&cr, "terminal goal, unused session");
    // Close the only todo: the first decision is then "terminal", so the loop breaks
    // before executing anything and `executed_turn` stays false.
    let bootstrap = first_todo_id(&cr.root, &goal);
    cli_ok(&[
        "todo",
        "complete",
        "--goal",
        &goal,
        "--todo-id",
        &bootstrap,
        "--no-follow-up",
        "--evidence",
        "closed before the run",
    ]);
    // Plant the wedge only AFTER the setup commands, so the run that matters is the
    // one that hits the persist failure.
    wedge_the_projection(&cr.root);

    let rt = rt();
    let mut state = MockState::default();
    // The discard must fail, and the run must carry on regardless. (Do NOT also fail
    // `new_session`: that aborts before the loop and the discard is never reached.)
    state.fail_commands.insert("delete_session".to_string());
    let (addr, shared) = rt.block_on(spawn_mock(state));
    std::env::set_var("FUTURE_LOOP_AGENT_ADDR", &addr);

    // A non-zero exit is acceptable (a terminal goal has nothing to run); what must
    // hold is that the process completed rather than aborting on the failed delete.
    let _ = cli(&["run", "--goal", &goal, "--anonymous"]);

    let observed = shared.lock().unwrap();
    assert_eq!(
        observed.prompts, 0,
        "a terminal goal must not prompt, or the used/unused distinction is not what \
         this test exercises"
    );
    assert!(
        observed.sessions_created >= 1,
        "the run mints its session BEFORE deciding, which is why it needs discarding"
    );
    drop(observed);

    // The run still recorded its outcome, so the failed best-effort writes did not
    // abort it: the goal is still readable and its state is intact.
    let store = open_store(&cr);
    let after = store.replay(&goal).unwrap().expect("the goal must survive");
    assert!(
        after.todos.iter().all(|t| matches!(
            t.status,
            future_loop::state::TodoStatus::Done | future_loop::state::TodoStatus::Superseded
        )),
        "the pre-existing terminal state must be unchanged"
    );
}

/// A stopped worker whose lease is ALREADY EXPIRED must not produce a release.
///
/// `release_leases_of_stopped` filters on `LeaseStatus::Active`, and the alternative
/// arm is the real case this covers: the TTL lapsed while the worker sat idle, so the
/// claim is already gone and "releasing" it would append a `TodoReleased` for a lease
/// nobody holds - a phantom event that a reader would have to reconcile.
///
/// Proven reachable from the merged report rather than assumed: `6951` (the
/// not-my-agent skip) had 144 hits and `6962` (the actual append) 308, i.e. the
/// `matches!` filter took its TRUE path 308 times while its line recorded 0 - the
/// uncovered region is the false arm, which no test had ever produced.
#[test]
fn a_stopped_worker_with_an_expired_lease_produces_no_release_event() {
    let cr = cli_root();
    let goal = init_goal(&cr, "expired lease at stop time");
    cli_ok(&["agent", "register", "--goal", &goal, "--agent-id", "wx"]);
    let bootstrap = first_todo_id(&cr.root, &goal);
    cli_ok(&[
        "todo",
        "supersede",
        "--goal",
        &goal,
        "--todo-id",
        &bootstrap,
        "--reason",
        "fixture",
    ]);
    cli_ok(&["todo", "add", "--goal", &goal, "--text", "idle claim"]);
    let store = open_store(&cr);
    let todo = store
        .replay(&goal)
        .unwrap()
        .unwrap()
        .todos
        .iter()
        .find(|t| t.text == "idle claim")
        .unwrap()
        .id
        .clone();
    drop(store);

    // A 1-second lease, claimed now, so it is ACTIVE for exactly a moment.
    cli_ok(&[
        "lease",
        "claim",
        "--goal",
        &goal,
        "--todo-id",
        &todo,
        "--agent-id",
        "wx",
        "--lease-secs",
        "1",
    ]);
    // The worker is still bound to a live session (so the stop has a target), but it
    // has not renewed: by the time we stop it, its claim has lapsed.
    let runs = std::path::Path::new(&cr.root).join("runs");
    std::fs::create_dir_all(&runs).unwrap();
    let header = serde_json::json!({
        "type": "run_header",
        "wall_ts": 1_700_000_000u64,
        "run_id": "run-wx",
        "session_id": "sess-wx",
        "agent_id": "wx",
        "todo_id": todo,
        "goal_id": goal,
    });
    std::fs::write(runs.join("run_wx.live.jsonl"), format!("{header}\n")).unwrap();

    let rt = rt();
    let (addr, _state) = rt.block_on(spawn_mock(MockState::default()));
    std::env::set_var("FUTURE_LOOP_AGENT_ADDR", &addr);

    // Let the TTL lapse. Short enough to keep the suite fast; long enough that the
    // lease is unambiguously expired by the time the stop runs.
    std::thread::sleep(std::time::Duration::from_secs(2));

    cli_ok(&["worker", "stop", "--goal", &goal, "--agent-id", "wx"]);

    let store = open_store(&cr);
    let released: Vec<String> = store
        .events(&goal)
        .unwrap()
        .into_iter()
        .filter_map(|e| match e.event {
            future_loop::store::Event::TodoReleased { todo_id, .. } => Some(todo_id),
            _ => None,
        })
        .collect();
    assert!(
        released.is_empty(),
        "an already-expired lease must NOT be released - there is no claim left to \
         drop, and a TodoReleased here would be a phantom event: {released:?}"
    );
    // The stop itself still happened: the lapsed lease is not a reason to skip it.
    let stopped = store
        .events(&goal)
        .unwrap()
        .into_iter()
        .filter(|e| matches!(e.event, future_loop::store::Event::WorkerStopped { .. }))
        .count();
    assert_eq!(stopped, 1, "the stop signal must still be written");
}

/// A read-only ledger: `replay` still reads it, but the next APPEND fails. That is the
/// portable way to reach the ledger-write `?` arms (`cmd_run`'s
/// `record_turn_decision`, the follow-through refresh, the backfill append); the
/// alternatives are not cross-platform - a deleted file breaks `replay` too, and
/// permissions tricks differ between Windows and a root-owned Linux CI.
///
/// The contract under test: a ledger that cannot be written must be REPORTED and must
/// not be silently presented as a successful turn.
#[test]
fn a_read_only_ledger_fails_the_turn_instead_of_silently_succeeding() {
    let cr = cli_root();
    let goal = init_goal(&cr, "read-only ledger");
    cli_ok(&["agent", "register", "--goal", &goal, "--agent-id", "wr"]);
    cli_ok(&[
        "todo",
        "add",
        "--goal",
        &goal,
        "--text",
        "cannot be recorded",
    ]);

    let ledger = std::path::Path::new(&cr.root)
        .join("goals")
        .join(&goal)
        .join("events.jsonl");
    assert!(ledger.is_file(), "the ledger must exist after setup");
    let original = std::fs::metadata(&ledger).unwrap().permissions();
    let mut locked = original.clone();
    locked.set_readonly(true);
    std::fs::set_permissions(&ledger, locked).expect("the ledger must become read-only");

    let rt = rt();
    let (addr, shared) = rt.block_on(spawn_mock(MockState {
        events: common::mock_agent::completed_events("mock-run-1"),
        ..Default::default()
    }));
    std::env::set_var("FUTURE_LOOP_AGENT_ADDR", &addr);

    let outcome = cli(&[
        "run",
        "--goal",
        &goal,
        "--agent-id",
        "wr",
        "--max-turns",
        "4",
    ]);
    // Restore the ORIGINAL permissions before asserting, so a failure does not leave a
    // read-only file behind for the tempdir cleanup. Restoring the saved value (rather
    // than forcing a writable mode) also avoids `clippy::permissions_set_readonly_false`
    // and preserves a mode that may not have been 0644.
    let _ = std::fs::set_permissions(&ledger, original);

    let err = outcome.expect_err("a turn that cannot be recorded must not report success");
    assert!(
        err.to_lowercase().contains("read")
            || err.to_lowercase().contains("ledger")
            || err.to_lowercase().contains("append")
            || err.to_lowercase().contains("denied")
            || err.to_lowercase().contains("permission"),
        "the failure must name the ledger problem, not something unrelated: {err}"
    );
    // It got as far as minting a session and deciding, which is what makes the
    // failure a ledger-write failure rather than a setup failure.
    let observed = shared.lock().unwrap();
    assert!(
        observed.sessions_created >= 1,
        "the run must have started (session minted) before the append failed"
    );
}

/// `worker stop --all` takes every live session wholesale (the `--agent-id` form
/// filters). With a reachable agent the aborts SUCCEED, so each stopped worker's
/// claim is released at once and named - a stopped worker can neither renew nor
/// complete it, and only its owner may release it, so leaving it would block peers
/// until the TTL lapses.
///
/// This is the broadcast counterpart of the two single-worker tests (one with an
/// unreachable agent, which must KEEP the claims, and one against an idle worker,
/// which releases the stale claim).
#[test]
fn worker_stop_all_releases_every_stopped_workers_claim() {
    let cr = cli_root();
    let goal = init_goal(&cr, "stop all");
    cli_ok(&["agent", "register", "--goal", &goal, "--agent-id", "wa"]);
    cli_ok(&["agent", "register", "--goal", &goal, "--agent-id", "wb"]);
    let bootstrap = first_todo_id(&cr.root, &goal);
    cli_ok(&[
        "todo",
        "supersede",
        "--goal",
        &goal,
        "--todo-id",
        &bootstrap,
        "--reason",
        "fixture",
    ]);
    cli_ok(&["todo", "add", "--goal", &goal, "--text", "for wa"]);
    cli_ok(&["todo", "add", "--goal", &goal, "--text", "for wb"]);

    let store = open_store(&cr);
    let state = store.replay(&goal).unwrap().unwrap();
    let todo_for = |text: &str| {
        state
            .todos
            .iter()
            .find(|t| t.text == text)
            .expect("the todo exists")
            .id
            .clone()
    };
    let (ta, tb) = (todo_for("for wa"), todo_for("for wb"));
    drop(store);

    for (todo, agent) in [(&ta, "wa"), (&tb, "wb")] {
        cli_ok(&[
            "lease",
            "claim",
            "--goal",
            &goal,
            "--todo-id",
            todo,
            "--agent-id",
            agent,
            "--lease-secs",
            "600",
        ]);
    }
    // The bound sessions are what `bound_worker_sessions` finds.
    let runs = std::path::Path::new(&cr.root).join("runs");
    std::fs::create_dir_all(&runs).unwrap();
    for (agent, todo) in [("wa", &ta), ("wb", &tb)] {
        let header = serde_json::json!({
            "type": "run_header",
            "wall_ts": 1_700_000_000u64,
            "run_id": format!("run-{agent}"),
            "session_id": format!("sess-{agent}"),
            "agent_id": agent,
            "todo_id": todo,
            "goal_id": goal,
        });
        std::fs::write(
            runs.join(format!("run_{agent}.live.jsonl")),
            format!("{header}\n"),
        )
        .unwrap();
    }

    let rt = rt();
    let (addr, _state) = rt.block_on(spawn_mock(MockState::default()));
    std::env::set_var("FUTURE_LOOP_AGENT_ADDR", &addr);

    cli_ok(&["worker", "stop", "--goal", &goal, "--all"]);

    let store = open_store(&cr);
    let after = store.replay(&goal).unwrap().unwrap();
    for (todo, agent) in [(&ta, "wa"), (&tb, "wb")] {
        assert!(
            after.todo(todo).unwrap().claimed_by.is_none(),
            "`--all` must release {agent}'s claim, not leave it to the TTL: {:?}",
            after.todo(todo)
        );
    }
    let released = store
        .events(&goal)
        .unwrap()
        .into_iter()
        .filter(|e| matches!(e.event, future_loop::store::Event::TodoReleased { .. }))
        .count();
    assert_eq!(released, 2, "one release per stopped worker");
    // The broadcast is a single agent-less stop event, not one per worker.
    let stops: Vec<Option<String>> = store
        .events(&goal)
        .unwrap()
        .into_iter()
        .filter_map(|e| match e.event {
            future_loop::store::Event::WorkerStopped { agent_id, .. } => Some(agent_id),
            _ => None,
        })
        .collect();
    assert_eq!(
        stops,
        vec![None],
        "`--all` writes ONE broadcast stop, unlike the per-agent form: {stops:?}"
    );
}
