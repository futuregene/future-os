//! Coverage drive — console command group A: goal / todo / gate / lease /
//! agent / backup / authority / replan / profile. All in-process through
//! `console::run` with serialized roots.

mod common;

use common::{
    add_todo, cli, cli_err, cli_ok, cli_root, first_todo_id, init_goal, open_store, todo_id_by_text,
};
use future_loop::state::{now_epoch, TodoStatus};
use future_loop::store::{Event, Store};

// ── goal ───────────────────────────────────────────────────────────────────

#[test]
fn goal_init_variants_and_errors() {
    let cr = cli_root();
    // Missing --objective.
    let err = cli_err(&["goal", "init", "--goal-id", "goal_x"]);
    assert!(err.contains("--objective"), "{err}");
    // `goal` with no args at all takes the init path and fails the same way.
    let err = cli_err(&["goal"]);
    assert!(err.contains("--objective"), "{err}");
    // --goal-doc writes GOAL.md into the goal cwd.
    let gid = init_goal(&cr, "doc goal");
    cli_ok(&[
        "goal",
        "init",
        "--objective",
        "with doc",
        "--goal-id",
        "goal_withdoc",
        "--cwd",
        &cr.cwd,
        "--goal-doc",
        "# Hello",
    ]);
    let doc = std::path::Path::new(&cr.cwd).join("GOAL.md");
    assert_eq!(std::fs::read_to_string(doc).unwrap(), "# Hello\n");
    let _ = gid;

    // Re-init with the same --goal-id is idempotent: it must NOT append a
    // duplicate onboarding bootstrap todo.
    cli_ok(&[
        "goal",
        "init",
        "--objective",
        "with doc again",
        "--goal-id",
        "goal_withdoc",
    ]);
    let store = open_store(&cr);
    let g = store.replay("goal_withdoc").unwrap().unwrap();
    let onboarding_count = g
        .todos
        .iter()
        .filter(|t| t.action_kind.as_deref() == Some("onboarding_connection_validation"))
        .count();
    assert_eq!(
        onboarding_count, 1,
        "re-init must not duplicate the onboarding todo"
    );
}

#[test]
fn goal_cancel_and_delete() {
    let cr = cli_root();
    let gid = init_goal(&cr, "cancel me");
    cli_ok(&[
        "goal",
        "cancel",
        "--goal",
        &gid,
        "--reason",
        "no longer needed",
    ]);
    let store = open_store(&cr);
    let g = store.replay(&gid).unwrap().unwrap();
    assert_eq!(g.status, "cancelled");
    // Cancel: missing goal id / unknown goal.
    assert!(cli_err(&["goal", "cancel"]).contains("--goal required"));
    assert!(cli_err(&["goal", "cancel", "--goal", "goal_nope"]).contains("not found"));

    // Delete requires --force.
    let gid2 = init_goal(&cr, "delete me");
    assert!(cli_err(&["goal", "delete", "--goal", &gid2]).contains("--force"));
    cli_ok(&["goal", "delete", "--goal", &gid2, "--force"]);
    let store = open_store(&cr);
    assert!(
        store.replay(&gid2).unwrap().is_none(),
        "deleted goal is gone"
    );
    assert!(cli_err(&["goal", "delete"]).contains("--goal required"));
}

// ── todo add (class/flag matrix) ───────────────────────────────────────────

#[test]
fn todo_add_class_matrix() {
    let cr = cli_root();
    let gid = init_goal(&cr, "todo classes");

    // user_gate via (user, user_gate) and via (_, user_gate).
    cli_ok(&[
        "todo",
        "add",
        "--goal",
        &gid,
        "--role",
        "user",
        "--class",
        "user_gate",
        "--text",
        "gate one",
        "--gate-question",
        "q1?",
    ]);
    cli_ok(&[
        "todo",
        "add",
        "--goal",
        &gid,
        "--class",
        "user_gate",
        "--text",
        "gate two",
    ]);
    // user_action / blocker / monitor / unknown-class fallback.
    cli_ok(&[
        "todo",
        "add",
        "--goal",
        &gid,
        "--role",
        "user",
        "--class",
        "user_action",
        "--text",
        "user does this",
    ]);
    let blocker = {
        cli_ok(&[
            "todo",
            "add",
            "--goal",
            &gid,
            "--class",
            "blocker",
            "--text",
            "blocked on ext",
            "--blocks",
            "todo_later",
        ]);
        todo_id_by_text(&cr.root, &gid, "blocked on ext")
    };
    cli_ok(&[
        "todo",
        "add",
        "--goal",
        &gid,
        "--class",
        "monitor",
        "--text",
        "watch it",
        "--cadence",
        "15m",
        "--monitor-target",
        "file:x",
        "--monitor-policy",
        "exists",
    ]);
    // Coordination class: orchestration bookkeeping, never agent work.
    cli_ok(&[
        "todo",
        "add",
        "--goal",
        &gid,
        "--class",
        "coordination",
        "--text",
        "final validation",
    ]);
    // Owner assignment on a shared advancement todo.
    cli_ok(&[
        "todo",
        "add",
        "--goal",
        &gid,
        "--owner",
        "worker-a",
        "--text",
        "worker-a only",
    ]);
    // Unknown class is rejected fail-closed (was: silently fell back to
    // advancement and polluted the ledger).
    assert!(cli_err(&[
        "todo",
        "add",
        "--goal",
        &gid,
        "--class",
        "bogus-class",
        "--text",
        "falls back",
    ])
    .contains("unknown --role/--class combo"));
    let store = open_store(&cr);
    let g = store.replay(&gid).unwrap().unwrap();
    let b = g.todos.iter().find(|t| t.id == blocker).unwrap();
    assert_eq!(b.blocked_by_gate.as_deref(), Some("todo_later"));
    assert!(b.class == future_loop::state::TaskClass::Blocker);
    let m = g.todos.iter().find(|t| t.text == "watch it").unwrap();
    assert!(m.resume_when.is_some(), "cadence sets the first due time");
    assert_eq!(m.monitor_target.as_deref(), Some("file:x"));
    assert_eq!(m.monitor_policy.as_deref(), Some("exists"));
    let coord = g
        .todos
        .iter()
        .find(|t| t.text == "final validation")
        .unwrap();
    assert!(
        coord.class == future_loop::state::TaskClass::Coordination,
        "coordination class must be preserved"
    );
    let owned = g.todos.iter().find(|t| t.text == "worker-a only").unwrap();
    assert_eq!(owned.owner.as_deref(), Some("worker-a"));
    assert_eq!(m.monitor_cadence.as_deref(), Some("15m"));
}

#[test]
fn todo_add_flag_matrix() {
    let cr = cli_root();
    let gid = init_goal(&cr, "todo flags");

    // Full flag cocktail.
    cli_ok(&[
        "todo",
        "add",
        "--goal",
        &gid,
        "--text",
        "kitchen sink",
        "--priority",
        "P0",
        "--action-kind",
        "deploy",
        "--title",
        "Sink",
        "--task-repository",
        "repo-1",
        "--continuation-policy",
        "resume",
        "--required-write-scope",
        "src, tests",
        "--note",
        "a note",
        "--goal-bound",
        "--verify",
        "exit 0",
        "--max-validation-attempts",
        "2",
    ]);
    // Priority prefix retitles a default title but not a custom one.
    cli_ok(&[
        "todo",
        "add",
        "--goal",
        &gid,
        "--text",
        "plain P2",
        "--priority",
        "P2",
    ]);
    cli_ok(&[
        "todo",
        "add",
        "--goal",
        &gid,
        "--text",
        "titled P0",
        "--priority",
        "P0",
        "--title",
        "Custom Title",
    ]);
    // Bogus priority is rejected fail-closed (previously remapped to P1).
    assert!(cli_err(&[
        "todo",
        "add",
        "--goal",
        &gid,
        "--text",
        "odd priority",
        "--priority",
        "P9",
    ])
    .contains("unknown --priority"));
    // Defer paths: numeric --resume-when, textual --resume-when, --defer-secs.
    cli_ok(&[
        "todo",
        "add",
        "--goal",
        &gid,
        "--text",
        "deferred num",
        "--resume-when",
        "30",
    ]);
    cli_ok(&[
        "todo",
        "add",
        "--goal",
        &gid,
        "--text",
        "deferred text",
        "--resume-when",
        "later",
    ]);
    cli_ok(&[
        "todo",
        "add",
        "--goal",
        &gid,
        "--text",
        "deferred secs",
        "--defer-secs",
        "5",
    ]);
    // --max-validation-attempts clamps to >= 1.
    cli_ok(&[
        "todo",
        "add",
        "--goal",
        &gid,
        "--text",
        "clamped",
        "--max-validation-attempts",
        "0",
    ]);
    // --blocks on an advancement todo (dependency chain applies to all classes).
    cli_ok(&[
        "todo", "add", "--goal", &gid, "--text", "chained", "--blocks", "a,b",
    ]);
    // global-gate user_gate forces goal_bound.
    cli_ok(&[
        "todo",
        "add",
        "--goal",
        &gid,
        "--text",
        "global gate",
        "--class",
        "user_gate",
        "--global-gate",
    ]);
    // Bad cadence string: no resume derivation (stays monitor default).
    cli_ok(&[
        "todo",
        "add",
        "--goal",
        &gid,
        "--text",
        "bad cadence",
        "--class",
        "monitor",
        "--cadence",
        "whenever",
    ]);

    let store = open_store(&cr);
    let g = store.replay(&gid).unwrap().unwrap();
    let sink = g
        .todos
        .iter()
        .find(|t| t.text.contains("kitchen sink"))
        .unwrap();
    assert_eq!(sink.priority, future_loop::state::Priority::P0);
    assert!(sink.text.starts_with("[P0] "), "{}", sink.text);
    assert_eq!(sink.title, "Sink", "custom title is kept");
    assert_eq!(sink.action_kind.as_deref(), Some("deploy"));
    assert_eq!(sink.task_repository.as_deref(), Some("repo-1"));
    assert_eq!(sink.continuation_policy.as_deref(), Some("resume"));
    assert_eq!(
        sink.required_write_scope,
        vec!["src".to_string(), "tests".to_string()]
    );
    assert_eq!(sink.note.as_deref(), Some("a note"));
    assert!(sink.goal_bound);
    assert_eq!(sink.validator.as_deref(), Some("exit 0"));
    assert_eq!(sink.max_validation_attempts, 2);

    let plain = g
        .todos
        .iter()
        .find(|t| t.text.contains("plain P2"))
        .unwrap();
    assert!(plain.text.starts_with("[P2] "), "{}", plain.text);
    assert!(
        plain.title.starts_with("[P2] "),
        "default title retitled: {}",
        plain.title
    );
    let titled = g
        .todos
        .iter()
        .find(|t| t.text.contains("titled P0"))
        .unwrap();
    assert_eq!(titled.title, "Custom Title");
    // "odd priority" (P9) is now rejected at the CLI, so it never lands in
    // the ledger — nothing to assert about its projected state.
    for needle in ["deferred num", "deferred text", "deferred secs"] {
        let t = g.todos.iter().find(|t| t.text.contains(needle)).unwrap();
        assert_eq!(t.status, TodoStatus::Deferred, "{needle}");
        assert!(t.resume_when.is_some(), "{needle}");
    }
    let clamped = g.todos.iter().find(|t| t.text.contains("clamped")).unwrap();
    assert_eq!(clamped.max_validation_attempts, 1);
    let chained = g.todos.iter().find(|t| t.text.contains("chained")).unwrap();
    assert_eq!(chained.blocked_by_gate.as_deref(), Some("a,b"));
    let gg = g
        .todos
        .iter()
        .find(|t| t.text.contains("global gate"))
        .unwrap();
    assert!(
        gg.global_gate && gg.goal_bound,
        "global gate implies goal_bound"
    );

    // Error paths.
    assert!(cli_err(&["todo", "add", "--text", "x"]).contains("--goal required"));
    assert!(cli_err(&["todo", "add", "--goal", &gid]).contains("--text required"));
    assert!(cli_err(&["todo", "add", "--goal", "goal_nope", "--text", "x"]).contains("not found"));
    assert!(cli_err(&["todo"]).contains("add|claim|complete"));
    assert!(cli_err(&["todo", "frobnicate"]).contains("unknown todo subcommand"));
}

// ── todo claim / complete ──────────────────────────────────────────────────

#[test]
fn todo_claim_paths() {
    let cr = cli_root();
    let gid = init_goal(&cr, "claim paths");
    let t = first_todo_id(&cr.root, &gid);
    // Unregistered agent is rejected.
    let err = cli_err(&[
        "todo",
        "claim",
        "--goal",
        &gid,
        "--todo-id",
        &t,
        "--agent-id",
        "ghost",
    ]);
    assert!(err.contains("not registered"), "{err}");
    cli_ok(&["agent", "register", "--goal", &gid, "--agent-id", "w1"]);
    cli_ok(&["agent", "register", "--goal", &gid, "--agent-id", "w2"]);
    cli_ok(&[
        "todo",
        "claim",
        "--goal",
        &gid,
        "--todo-id",
        &t,
        "--agent-id",
        "w1",
        "--lease-secs",
        "120",
    ]);
    // Same-agent re-claim is idempotent (renew)…
    cli_ok(&[
        "todo",
        "claim",
        "--goal",
        &gid,
        "--todo-id",
        &t,
        "--agent-id",
        "w1",
    ]);
    // …but a DIFFERENT agent cannot take a live lease.
    assert!(cli_err(&[
        "todo",
        "claim",
        "--goal",
        &gid,
        "--todo-id",
        &t,
        "--agent-id",
        "w2"
    ])
    .contains("cannot be claimed"));
    // Missing todo / goal.
    assert!(cli_err(&[
        "todo",
        "claim",
        "--goal",
        &gid,
        "--todo-id",
        "todo_nope",
        "--agent-id",
        "w1"
    ])
    .contains("not found"));
    assert!(
        cli_err(&["todo", "claim", "--goal", "goal_nope", "--todo-id", &t]).contains("not found")
    );
    assert!(cli_err(&["todo", "claim", "--goal", &gid]).contains("--todo-id required"));
    assert!(cli_err(&["todo", "claim"]).contains("--goal required"));
}

#[test]
fn todo_complete_contract() {
    let cr = cli_root();
    let gid = init_goal(&cr, "complete contract");
    let first = first_todo_id(&cr.root, &gid);
    // Silent completion is rejected.
    assert!(
        cli_err(&["todo", "complete", "--goal", &gid, "--todo-id", &first])
            .contains("--no-follow-up or --successor")
    );
    // Completion with a successor.
    let s = add_todo(&cr, &gid, "successor task");
    cli_ok(&[
        "todo",
        "complete",
        "--goal",
        &gid,
        "--todo-id",
        &first,
        "--successor",
        &s,
        "--evidence",
        "did the thing",
    ]);
    {
        let store = open_store(&cr);
        let g = store.replay(&gid).unwrap().unwrap();
        let t = g.todos.iter().find(|t| t.id == first).unwrap();
        assert_eq!(t.status, TodoStatus::Done);
        assert_eq!(t.successor_ids, vec![s.clone()]);
        assert_eq!(t.evidence.as_deref(), Some("did the thing"));
    }
    // Gate freeze: an open user gate blocks completing other todos.
    cli_ok(&[
        "todo",
        "add",
        "--goal",
        &gid,
        "--text",
        "approval",
        "--class",
        "user_gate",
        "--gate-question",
        "ok?",
    ]);
    let err = cli_err(&[
        "todo",
        "complete",
        "--goal",
        &gid,
        "--todo-id",
        &s,
        "--no-follow-up",
        "--evidence",
        "fixture evidence for completion contract",
    ]);
    assert!(err.contains("open gate"), "{err}");
    // Resolve the gate, then completion succeeds (a gate's text IS the
    // question — Todo::user_gate takes the question as the todo text).
    let gate = todo_id_by_text(&cr.root, &gid, "ok?");
    cli_ok(&[
        "gate",
        "resolve",
        "--goal",
        &gid,
        "--todo-id",
        &gate,
        "--decision",
        "approved",
        "--note",
        "stamp",
    ]);
    cli_ok(&[
        "todo",
        "complete",
        "--goal",
        &gid,
        "--todo-id",
        &s,
        "--no-follow-up",
        "--evidence",
        "fixture evidence for completion contract",
        "--force",
    ]);
    // Unknown todo / goal.
    assert!(cli_err(&[
        "todo",
        "complete",
        "--goal",
        &gid,
        "--todo-id",
        "todo_nope",
        "--no-follow-up"
    ])
    .contains("not found"));
    assert!(cli_err(&["todo", "complete", "--todo-id", &s]).contains("--goal required"));
    assert!(cli_err(&["todo", "complete", "--goal", &gid]).contains("--todo-id required"));
    assert!(cli_err(&[
        "todo",
        "complete",
        "--goal",
        "goal_nope",
        "--todo-id",
        &s,
        "--no-follow-up"
    ])
    .contains("not found"));
}

#[test]
fn todo_complete_evidence_floor_and_force() {
    let cr = cli_root();
    let gid = init_goal(&cr, "evidence floor");
    let first = first_todo_id(&cr.root, &gid);
    // Advancement completion without evidence is refused (the empty-closure
    // failure mode: an agent marks a delivery done with nothing to show).
    let err = cli_err(&[
        "todo",
        "complete",
        "--goal",
        &gid,
        "--todo-id",
        &first,
        "--no-follow-up",
    ]);
    assert!(err.contains("--evidence"), "{err}");
    // Whitespace-only evidence is refused too.
    let err = cli_err(&[
        "todo",
        "complete",
        "--goal",
        &gid,
        "--todo-id",
        &first,
        "--no-follow-up",
        "--evidence",
        "   ",
    ]);
    assert!(err.contains("--evidence"), "{err}");
    // --force is the explicit operator override for mechanical closeouts.
    cli_ok(&[
        "todo",
        "complete",
        "--goal",
        &gid,
        "--todo-id",
        &first,
        "--no-follow-up",
        "--evidence",
        "fixture evidence for completion contract",
        "--force",
    ]);
    // Any non-empty evidence satisfies the floor (strength belongs to
    // --acceptance / --verify, which are opt-in contracts).
    let s = add_todo(&cr, &gid, "second task");
    cli_ok(&[
        "todo",
        "complete",
        "--goal",
        &gid,
        "--todo-id",
        &s,
        "--no-follow-up",
        "--evidence",
        "did the thing",
    ]);
    {
        let store = open_store(&cr);
        let g = store.replay(&gid).unwrap().unwrap();
        let t = g.todos.iter().find(|t| t.id == s).unwrap();
        assert_eq!(t.status, TodoStatus::Done);
        assert_eq!(t.evidence.as_deref(), Some("did the thing"));
    }
}

#[test]
fn todo_complete_acceptance_contract() {
    let cr = cli_root();
    let gid = init_goal(&cr, "acceptance contract");
    let first = first_todo_id(&cr.root, &gid);
    // `--acceptance` is a todo-add flag; `todo complete` rejects it.
    let err = cli_err(&[
        "todo",
        "complete",
        "--goal",
        &gid,
        "--todo-id",
        &first,
        "--no-follow-up",
        "--evidence",
        "fixture evidence for completion contract",
        "--acceptance",
        "x",
    ]);
    assert!(err.contains("unknown"), "{err}");
    let t = add_todo(&cr, &gid, "submit the payload");
    // Declare the acceptance contract: evidence must contain BOTH tokens.
    cli_ok(&[
        "todo",
        "update",
        "--goal",
        &gid,
        "--todo-id",
        &t,
        "--acceptance",
        "attempt,scored",
    ]);
    // Evidence missing one token is refused.
    let err = cli_err(&[
        "todo",
        "complete",
        "--goal",
        &gid,
        "--todo-id",
        &t,
        "--no-follow-up",
        "--evidence",
        "created attempt 12345",
    ]);
    assert!(err.contains("acceptance contract"), "{err}");
    assert!(err.contains("scored"), "{err}");
    // Matching evidence (case-insensitive) completes.
    cli_ok(&[
        "todo",
        "complete",
        "--goal",
        &gid,
        "--todo-id",
        &t,
        "--no-follow-up",
        "--evidence",
        "ATTEMPT 12345 SCORED 99 on the platform",
    ]);
    // --force overrides an unmet contract.
    let t2 = add_todo(&cr, &gid, "submit again");
    cli_ok(&[
        "todo",
        "update",
        "--goal",
        &gid,
        "--todo-id",
        &t2,
        "--acceptance",
        "attempt,scored",
    ]);
    cli_ok(&[
        "todo",
        "complete",
        "--goal",
        &gid,
        "--todo-id",
        &t2,
        "--no-follow-up",
        "--evidence",
        "operator closeout without a scored attempt",
        "--force",
    ]);
    // The acceptance contract survives the store round-trip.
    let store = open_store(&cr);
    let g = store.replay(&gid).unwrap().unwrap();
    let todo = g.todos.iter().find(|x| x.id == t).unwrap();
    assert_eq!(todo.acceptance.as_deref(), Some("attempt,scored"));
    assert_eq!(
        todo.evidence.as_deref(),
        Some("ATTEMPT 12345 SCORED 99 on the platform")
    );
}

#[test]
fn todo_complete_gate_class_bypasses_freeze() {
    let cr = cli_root();
    let gid = init_goal(&cr, "gate class complete");
    cli_ok(&[
        "todo",
        "add",
        "--goal",
        &gid,
        "--text",
        "gate A",
        "--class",
        "user_gate",
    ]);
    cli_ok(&[
        "todo",
        "add",
        "--goal",
        &gid,
        "--text",
        "gate B",
        "--class",
        "user_gate",
    ]);
    let a = todo_id_by_text(&cr.root, &gid, "gate A");
    // A user gate is a decision point, not a work item: `todo complete` on it
    // is rejected and directed to `gate resolve` (the manual-close counterpart
    // of the run loop's gate handling). Resolve it via `gate resolve` instead.
    let err = cli_err(&[
        "todo",
        "complete",
        "--goal",
        &gid,
        "--todo-id",
        &a,
        "--no-follow-up",
        "--evidence",
        "fixture evidence for completion contract",
        "--force",
    ]);
    assert!(
        err.contains("gate resolve"),
        "error must point at gate resolve: {err}"
    );
    // The gate-freeze contract still holds: while gate B is open, gate A
    // cannot be resolved into work, but `gate resolve` closes it cleanly.
    cli_ok(&[
        "gate",
        "resolve",
        "--goal",
        &gid,
        "--todo-id",
        &a,
        "--decision",
        "approved",
    ]);
    let store = open_store(&cr);
    let g = store.replay(&gid).unwrap().unwrap();
    assert_eq!(
        g.todos.iter().find(|t| t.id == a).unwrap().status,
        TodoStatus::Done
    );
}

// ── todo archive / supersede / update ──────────────────────────────────────

#[test]
fn todo_archive_supersede_update() {
    let cr = cli_root();
    let gid = init_goal(&cr, "lifecycle");
    let victim = add_todo(&cr, &gid, "archive me");
    cli_ok(&["todo", "archive", "--goal", &gid, "--todo-id", &victim]);
    assert!(cli_err(&["todo", "archive", "--goal", &gid]).contains("--todo-id required"));
    assert!(cli_err(&["todo", "archive", "--todo-id", &victim]).contains("--goal required"));
    assert!(cli_err(&[
        "todo",
        "archive",
        "--goal",
        "goal_nope",
        "--todo-id",
        &victim
    ])
    .contains("not found"));

    let sup = add_todo(&cr, &gid, "supersede me");
    cli_ok(&[
        "todo",
        "supersede",
        "--goal",
        &gid,
        "--todo-id",
        &sup,
        "--reason",
        "obsolete",
    ]);
    {
        let store = open_store(&cr);
        let g = store.replay(&gid).unwrap().unwrap();
        assert_eq!(
            g.todos.iter().find(|t| t.id == sup).unwrap().status,
            TodoStatus::Superseded
        );
    }
    assert!(cli_err(&[
        "todo",
        "supersede",
        "--goal",
        &gid,
        "--todo-id",
        "todo_nope"
    ])
    .contains("not found"));
    // Supersede a done todo is rejected.
    let done = add_todo(&cr, &gid, "done one");
    cli_ok(&[
        "todo",
        "complete",
        "--goal",
        &gid,
        "--todo-id",
        &done,
        "--no-follow-up",
        "--evidence",
        "fixture evidence for completion contract",
        "--force",
    ]);
    assert!(
        cli_err(&["todo", "supersede", "--goal", &gid, "--todo-id", &done])
            .contains("already done")
    );

    // update: full field set.
    let u = add_todo(&cr, &gid, "update me");
    cli_ok(&[
        "todo",
        "update",
        "--goal",
        &gid,
        "--todo-id",
        &u,
        "--text",
        "updated text",
        "--status",
        "blocked",
        "--evidence",
        "half done",
        "--note",
        "n",
        "--priority",
        "P0",
        "--resume-when",
        "45",
        "--blocks",
        "x,y",
        "--owner",
        "worker-a",
    ]);
    {
        let store = open_store(&cr);
        let g = store.replay(&gid).unwrap().unwrap();
        let t = g.todos.iter().find(|t| t.id == u).unwrap();
        assert!(t.text.contains("updated text"), "{}", t.text);
        assert_eq!(t.evidence.as_deref(), Some("half done"));
        assert_eq!(t.note.as_deref(), Some("n"));
        assert_eq!(t.priority, future_loop::state::Priority::P0);
        assert_eq!(t.blocked_by_gate.as_deref(), Some("x,y"));
        assert_eq!(t.owner.as_deref(), Some("worker-a"));
        assert!(
            t.resume_when.is_some(),
            "numeric resume-when sets a real deadline"
        );
    }
    // Clear owner assignment via empty --owner.
    cli_ok(&[
        "todo",
        "update",
        "--goal",
        &gid,
        "--todo-id",
        &u,
        "--owner",
        "",
    ]);
    {
        let store = open_store(&cr);
        let g = store.replay(&gid).unwrap().unwrap();
        let t = g.todos.iter().find(|t| t.id == u).unwrap();
        assert_eq!(t.owner, None, "empty --owner clears the assignment");
    }
    // Clear blocks, textual resume-when, unknown status ignored.
    cli_ok(&[
        "todo",
        "update",
        "--goal",
        &gid,
        "--todo-id",
        &u,
        "--blocks",
        "",
    ]);
    cli_ok(&[
        "todo",
        "update",
        "--goal",
        &gid,
        "--todo-id",
        &u,
        "--resume-when",
        "when ready",
    ]);
    cli_ok(&[
        "todo",
        "update",
        "--goal",
        &gid,
        "--todo-id",
        &u,
        "--status",
        "bogus",
    ]);
    {
        let store = open_store(&cr);
        let g = store.replay(&gid).unwrap().unwrap();
        let t = g.todos.iter().find(|t| t.id == u).unwrap();
        assert_eq!(t.blocked_by_gate, None);
        assert_eq!(t.resume_when_text.as_deref(), Some("when ready"));
    }
    // --status done is rejected; unknown flags hard-error.
    assert!(cli_err(&[
        "todo",
        "update",
        "--goal",
        &gid,
        "--todo-id",
        &u,
        "--status",
        "done"
    ])
    .contains("not allowed"));
    assert!(cli_err(&[
        "todo",
        "update",
        "--goal",
        &gid,
        "--todo-id",
        &u,
        "--frobnicate",
        "1"
    ])
    .contains("unknown flag"));
    assert!(
        cli_err(&["todo", "update", "--goal", &gid, "--todo-id", "todo_nope"])
            .contains("not found")
    );
    assert!(
        cli_err(&["todo", "update", "--goal", "goal_nope", "--todo-id", &u]).contains("not found")
    );
    assert!(cli_err(&["todo", "update", "--goal", &gid]).contains("--todo-id required"));
}

// ── gate ───────────────────────────────────────────────────────────────────

#[test]
fn gate_resolve_errors() {
    let cr = cli_root();
    let gid = init_goal(&cr, "gate errors");
    assert!(cli_err(&["gate", "resolve", "--goal", &gid]).contains("--todo-id required"));
    assert!(cli_err(&["gate", "resolve", "--todo-id", "t"]).contains("--goal required"));
    assert!(
        cli_err(&["gate", "resolve", "--goal", &gid, "--todo-id", "t"])
            .contains("--decision required")
    );
    assert!(cli_err(&[
        "gate",
        "resolve",
        "--goal",
        "goal_nope",
        "--todo-id",
        "t",
        "--decision",
        "x"
    ])
    .contains("not found"));
    // Resolving a non-gate todo is rejected fail-closed (no phantom
    // GateResolved event); resolving an unknown id also errors.
    let first = first_todo_id(&cr.root, &gid);
    assert!(cli_err(&[
        "gate",
        "resolve",
        "--goal",
        &gid,
        "--todo-id",
        &first,
        "--decision",
        "fine",
    ])
    .contains("not a user_gate"));
    assert!(cli_err(&[
        "gate",
        "resolve",
        "--goal",
        &gid,
        "--todo-id",
        "todo_ghost",
        "--decision",
        "fine",
    ])
    .contains("not found"));
    // A real user_gate still resolves.
    cli_ok(&[
        "todo",
        "add",
        "--goal",
        &gid,
        "--text",
        "gate q",
        "--role",
        "user",
        "--class",
        "user_gate",
    ]);
    let g2 = open_store(&cr).replay(&gid).unwrap().unwrap();
    let gate_id = g2
        .todos
        .iter()
        .find(|t| t.class == future_loop::state::TaskClass::UserGate)
        .unwrap()
        .id
        .clone();
    cli_ok(&[
        "gate",
        "resolve",
        "--goal",
        &gid,
        "--todo-id",
        &gate_id,
        "--decision",
        "fine",
    ]);
    let store = open_store(&cr);
    let g = store.replay(&gid).unwrap().unwrap();
    let t = g.todos.iter().find(|t| t.id == gate_id).unwrap();
    assert_eq!(t.status, TodoStatus::Done);
    assert_eq!(t.decision.as_deref(), Some("fine"));
}

// ── lease ──────────────────────────────────────────────────────────────────

#[test]
fn lease_lifecycle() {
    let cr = cli_root();
    let gid = init_goal(&cr, "lease lifecycle");
    let t = first_todo_id(&cr.root, &gid);

    // status on a free todo.
    cli_ok(&["lease", "status", "--goal", &gid, "--todo-id", &t]);
    // claim / renew / release / expire.
    cli_ok(&[
        "lease",
        "claim",
        "--goal",
        &gid,
        "--todo-id",
        &t,
        "--agent-id",
        "w1",
        "--lease-secs",
        "60",
    ]);
    cli_ok(&["lease", "status", "--goal", &gid, "--todo-id", &t]);
    cli_ok(&[
        "lease",
        "renew",
        "--goal",
        &gid,
        "--todo-id",
        &t,
        "--agent-id",
        "w1",
        "--lease-secs",
        "120",
    ]);
    cli_ok(&[
        "lease",
        "release",
        "--goal",
        &gid,
        "--todo-id",
        &t,
        "--agent-id",
        "w1",
    ]);
    // release again → missing-lease path (no event, still ok).
    cli_ok(&[
        "lease",
        "release",
        "--goal",
        &gid,
        "--todo-id",
        &t,
        "--agent-id",
        "w1",
    ]);
    // expire with no lease → had_lease=false path.
    cli_ok(&["lease", "expire", "--goal", &gid, "--todo-id", &t]);
    // Minimum TTL is 1s (0 normalizes to the 45min default); lease timestamps
    // have epoch-second resolution, so cross the boundary with a real sleep.
    cli_ok(&[
        "lease",
        "claim",
        "--goal",
        &gid,
        "--todo-id",
        &t,
        "--agent-id",
        "w1",
        "--lease-secs",
        "1",
    ]);
    std::thread::sleep(std::time::Duration::from_millis(1200));
    // status now reports EXPIRED; expire records the event (had_lease=true).
    cli_ok(&["lease", "status", "--goal", &gid, "--todo-id", &t]);
    cli_ok(&["lease", "expire", "--goal", &gid, "--todo-id", &t]);
    // Steal path: w2 claims, lease lapses, w3 takes it (TodoExpired+TodoClaimed).
    cli_ok(&[
        "lease",
        "claim",
        "--goal",
        &gid,
        "--todo-id",
        &t,
        "--agent-id",
        "w2",
        "--lease-secs",
        "1",
    ]);
    std::thread::sleep(std::time::Duration::from_millis(1200));
    cli_ok(&[
        "lease",
        "claim",
        "--goal",
        &gid,
        "--todo-id",
        &t,
        "--agent-id",
        "w3",
        "--lease-secs",
        "30",
    ]);
    // renew by the non-owner errors.
    assert!(cli(&[
        "lease",
        "renew",
        "--goal",
        &gid,
        "--todo-id",
        &t,
        "--agent-id",
        "w1"
    ])
    .is_err());
    // Errors.
    assert!(cli_err(&["lease"]).contains("subcommand"));
    assert!(cli_err(&["lease", "claim", "--goal", &gid]).contains("--todo-id required"));
    assert!(cli_err(&["lease", "claim", "--todo-id", &t]).contains("--goal required"));
    assert!(
        cli_err(&["lease", "claim", "--goal", "goal_nope", "--todo-id", &t]).contains("not found")
    );
    assert!(
        cli_err(&["lease", "status", "--goal", &gid, "--todo-id", "todo_nope"])
            .contains("not found")
    );
    assert!(
        cli_err(&["lease", "frobnicate", "--goal", &gid, "--todo-id", &t])
            .contains("claim|renew|release|expire|status")
    );
}

// ── agent register / onboard / list ────────────────────────────────────────

#[test]
fn agent_registry_surface() {
    let cr = cli_root();
    let gid = init_goal(&cr, "agents");
    // list with no agents.
    cli_ok(&["agent", "list", "--goal", &gid]);
    cli_ok(&["agent", "register", "--goal", &gid, "--agent-id", "w1"]);
    cli_ok(&["agent", "onboard", "--goal", &gid, "--agent-id", "w2"]);
    cli_ok(&["agent", "onboard", "--goal", &gid, "--agent-id", "w3"]);
    // w1 holds a live lease → running row in `agent list`.
    let t = first_todo_id(&cr.root, &gid);
    cli_ok(&[
        "todo",
        "claim",
        "--goal",
        &gid,
        "--todo-id",
        &t,
        "--agent-id",
        "w1",
    ]);
    cli_ok(&["agent", "list", "--goal", &gid]);
    // Errors.
    assert!(cli_err(&["agent", "register", "--goal", &gid]).contains("--agent-id required"));
    assert!(cli_err(&["agent", "register", "--agent-id", "w1"]).contains("--goal required"));
    assert!(cli_err(&[
        "agent",
        "register",
        "--goal",
        "goal_nope",
        "--agent-id",
        "w1"
    ])
    .contains("not found"));
    assert!(cli_err(&["agent", "onboard", "--goal", &gid]).contains("--agent-id required"));
    assert!(cli_err(&[
        "agent",
        "onboard",
        "--goal",
        "goal_nope",
        "--agent-id",
        "w1"
    ])
    .contains("not found"));
    assert!(cli_err(&["agent", "list"]).contains("--goal required"));
    assert!(cli_err(&["agent", "list", "--goal", "goal_nope"]).contains("not found"));
}

// ── backup ─────────────────────────────────────────────────────────────────

#[test]
fn backup_create_list_restore() {
    let cr = cli_root();
    let gid = init_goal(&cr, "backups");
    cli_ok(&["backup", "--goal", &gid]);
    {
        let store = open_store(&cr);
        let backups = store.backups(&gid);
        assert_eq!(backups.len(), 1, "one backup created");
        cli_ok(&["backup", "--goal", &gid, "--list"]);
        // Restore from that backup dir.
        cli_ok(&["backup", "--goal", &gid, "--restore", &backups[0]]);
    }
    assert!(cli_err(&["backup"]).contains("--goal required"));
    assert!(cli_err(&[
        "backup",
        "--goal",
        &gid,
        "--restore",
        "/nonexistent-dir-xyz"
    ])
    .contains(""));
}

// ── authority / profile ────────────────────────────────────────────────────

#[test]
fn authority_and_profile() {
    let cr = cli_root();
    let gid = init_goal(&cr, "authority");
    cli_ok(&[
        "authority",
        "--goal",
        &gid,
        "--write-scope",
        "src,docs",
        "--require-approval",
        "publish,deploy",
    ]);
    {
        let store = open_store(&cr);
        let g = store.replay(&gid).unwrap().unwrap();
        assert_eq!(
            g.authority.write_scope,
            vec!["src".to_string(), "docs".to_string()]
        );
        assert_eq!(
            g.authority.requires_approval,
            vec!["publish".to_string(), "deploy".to_string()]
        );
    }
    assert!(cli_err(&["authority"]).contains("--goal required"));
    assert!(cli_err(&["authority", "--goal", "goal_nope"]).contains("not found"));

    cli_ok(&["profile", "set", "--goal", &gid, "--outcome-floor", "3"]);
    {
        let store = open_store(&cr);
        let g = store.replay(&gid).unwrap().unwrap();
        assert_eq!(g.execution_profile.outcome_floor_streak_threshold, 3);
    }
    assert!(
        cli_err(&["profile", "set", "--goal", &gid, "--outcome-floor", "x"]).contains("number")
    );
    assert!(cli_err(&["profile", "bogus", "--goal", &gid]).contains("must be `set`"));
    assert!(cli_err(&["profile", "set"]).contains("--goal required"));
    assert!(cli_err(&["profile", "set", "--goal", "goal_nope"]).contains("not found"));
}

// ── replan ─────────────────────────────────────────────────────────────────

#[test]
fn replan_ack_and_obligations() {
    let cr = cli_root();
    let gid = init_goal(&cr, "replan");
    // No obligations initially.
    cli_ok(&["replan", "obligations", "--goal", &gid]);
    // Craft an unfulfilled obligation: an advancement todo completed without
    // closure intent (bypasses the CLI completion contract via the raw event).
    {
        let mut store: Store = open_store(&cr);
        let todo = future_loop::state::Todo::advancement("todo_unclosed", "no closure intent");
        store
            .append(Event::TodoAdded {
                goal_id: gid.clone(),
                todo,
                ts: now_epoch(),
            })
            .unwrap();
        store
            .append(Event::TodoCompleted {
                goal_id: gid.clone(),
                todo_id: "todo_unclosed".to_string(),
                no_follow_up: false,
                successor_ids: vec![],
                evidence: None,
                ts: now_epoch(),
            })
            .unwrap();
    }
    cli_ok(&["replan", "obligations", "--goal", &gid]);
    // ack variants.
    cli_ok(&[
        "replan",
        "ack",
        "--goal",
        &gid,
        "--delta-kind",
        "vision_patch",
    ]);
    assert!(cli_err(&["replan", "ack", "--goal", &gid]).contains("--delta-kind"));
    assert!(
        cli_err(&["replan", "ack", "--goal", &gid, "--delta-kind", "nope"]).contains("frontier")
    );
    assert!(cli_err(&["replan", "ack"]).contains("--goal required"));
    assert!(cli_err(&[
        "replan",
        "ack",
        "--goal",
        "goal_nope",
        "--delta-kind",
        "vision_patch"
    ])
    .contains("not found"));
    assert!(cli_err(&["replan", "obligations"]).contains("--goal required"));
    assert!(cli_err(&["replan", "obligations", "--goal", "goal_nope"]).contains("not found"));
    // Ghost flags removed: `replan ack` no longer accepts --format/--json,
    // `replan obligations` no longer accepts --delta-kind.
    assert!(cli_err(&[
        "replan",
        "ack",
        "--goal",
        &gid,
        "--delta-kind",
        "vision_patch",
        "--format",
        "json"
    ])
    .contains("unknown flag `--format`"));
    assert!(cli_err(&[
        "replan",
        "obligations",
        "--goal",
        &gid,
        "--delta-kind",
        "vision_patch"
    ])
    .contains("unknown flag `--delta-kind`"));
}

// ── top-level dispatch quirks ──────────────────────────────────────────────

#[test]
fn dispatch_help_and_unknown() {
    let _cr = cli_root();
    cli_ok(&[]);
    cli_ok(&["--help"]);
    cli_ok(&["help"]);
    let err = cli_err(&["frobnicate"]);
    assert!(err.contains("unknown command"), "{err}");
    // --include-experimental is accepted globally.
    cli_ok(&["--help", "--include-experimental"]);
}
