//! Coverage drive — console command group C (P3/P4): catalog /
//! scope / lane / supervisor / task-graph / attention / inbox.

mod common;

use common::{cli_err, cli_ok, cli_root, first_todo_id, init_goal, open_store};
use future_loop::state::now_epoch;

// ── scope / lane ───────────────────────────────────────────────────────────

#[test]
fn scope_and_lane_surface() {
    let cr = cli_root();
    let gid = init_goal(&cr, "scope goal");
    cli_ok(&["agent", "register", "--goal", &gid, "--agent-id", "w1"]);
    let first = first_todo_id(&cr.root, &gid);
    cli_ok(&[
        "todo",
        "claim",
        "--goal",
        &gid,
        "--todo-id",
        &first,
        "--agent-id",
        "w1",
    ]);
    cli_ok(&["scope", "--goal", &gid, "--agent-id", "w1"]);
    cli_ok(&[
        "scope",
        "--goal",
        &gid,
        "--agent-id",
        "w1",
        "--exclude",
        "todo_x,todo_y",
    ]);
    // lane: no runs yet → None path. Then seed a run → recommendation paths.
    cli_ok(&["lane", "--goal", &gid, "--agent-id", "w1"]);
    {
        let store = open_store(&cr);
        store
            .append_run(&gid, &common::run_record(&first, "completed", now_epoch()))
            .unwrap();
    }
    cli_ok(&["lane", "--goal", &gid, "--agent-id", "w1"]);
    // errors.
    assert!(cli_err(&["scope", "--goal", &gid]).contains("--agent-id required"));
    assert!(cli_err(&["scope", "--agent-id", "w1"]).contains("--goal required"));
    assert!(cli_err(&["scope", "--goal", "goal_nope", "--agent-id", "w1"]).contains("not found"));
    assert!(cli_err(&["lane", "--goal", &gid]).contains("--agent-id required"));
    assert!(cli_err(&["lane", "--agent-id", "w1"]).contains("--goal required"));
    assert!(cli_err(&["lane", "--goal", "goal_nope", "--agent-id", "w1"]).contains("not found"));
}

// ── supervisor ─────────────────────────────────────────────────────────────

#[test]
fn supervisor_surface() {
    let cr = cli_root();
    let gid = init_goal(&cr, "supervisor goal");
    cli_ok(&[
        "supervisor",
        "propose",
        "--goal",
        &gid,
        "--agent-id",
        "sup",
        "--decision-id",
        "d1",
        "--target-agent-id",
        "w1",
        "--kind",
        "observe",
        "--summary",
        "watching",
    ]);
    cli_ok(&[
        "supervisor",
        "propose",
        "--goal",
        &gid,
        "--agent-id",
        "sup",
        "--decision-id",
        "d2",
        "--target-agent-id",
        "w1",
        "--kind",
        "execute",
        "--capabilities",
        "shell,web",
    ]);
    for (decision, outcome, caps) in [
        ("d2", "executed", "shell,web"),
        ("d2", "failed", "shell"),
        ("d2", "bogus-outcome", "shell"), // → Rejected via the catch-all arm
    ] {
        cli_ok(&[
            "supervisor",
            "receipt",
            "--goal",
            &gid,
            "--decision-id",
            decision,
            "--receipt-id",
            &format!("r-{outcome}"),
            "--adapter-id",
            "adapter-1",
            "--outcome",
            outcome,
            "--authority-ref",
            "auth-1",
            "--host-capabilities",
            caps,
        ]);
    }
    // An observe decision rejects host-execution receipts, and an executed
    // receipt must carry the decision's required capabilities.
    assert!(cli_err(&[
        "supervisor",
        "receipt",
        "--goal",
        &gid,
        "--decision-id",
        "d1",
        "--receipt-id",
        "r-bad",
        "--adapter-id",
        "a",
        "--outcome",
        "executed",
    ])
    .contains("observe"));
    assert!(cli_err(&[
        "supervisor",
        "receipt",
        "--goal",
        &gid,
        "--decision-id",
        "d2",
        "--receipt-id",
        "r-bad2",
        "--adapter-id",
        "a",
        "--outcome",
        "executed",
        "--host-capabilities",
        "shell",
    ])
    .contains("capabilities"));
    cli_ok(&["supervisor", "events", "--goal", &gid]);
    // errors.
    assert!(cli_err(&["supervisor", "propose", "--goal", &gid]).contains("--agent-id"));
    assert!(
        cli_err(&["supervisor", "propose", "--goal", &gid, "--agent-id", "sup",])
            .contains("--decision-id")
    );
    assert!(cli_err(&[
        "supervisor",
        "propose",
        "--goal",
        &gid,
        "--agent-id",
        "sup",
        "--decision-id",
        "d",
    ])
    .contains("--target-agent-id"));
    assert!(cli_err(&["supervisor", "receipt", "--goal", &gid]).contains("--decision-id"));
    assert!(cli_err(&[
        "supervisor",
        "receipt",
        "--goal",
        &gid,
        "--decision-id",
        "d",
    ])
    .contains("--receipt-id"));
    assert!(cli_err(&[
        "supervisor",
        "receipt",
        "--goal",
        &gid,
        "--decision-id",
        "d",
        "--receipt-id",
        "r",
    ])
    .contains("--adapter-id"));
    assert!(cli_err(&["supervisor", "events"]).contains("--goal required"));
    assert!(cli_err(&["supervisor", "bogus", "--goal", &gid]).contains("propose|receipt|events"));
    // Ghost flags removed: flags the reject array accepted but the parse
    // closure never read are now hard `unknown flag` errors.
    assert!(cli_err(&[
        "supervisor",
        "propose",
        "--goal",
        &gid,
        "--agent-id",
        "sup",
        "--decision-id",
        "d",
        "--target-agent-id",
        "w",
        "--outcome",
        "x",
    ])
    .contains("unknown flag `--outcome`"));
    assert!(cli_err(&[
        "supervisor",
        "receipt",
        "--goal",
        &gid,
        "--decision-id",
        "d",
        "--receipt-id",
        "r",
        "--adapter-id",
        "a",
        "--agent-id",
        "sup",
    ])
    .contains("unknown flag `--agent-id`"));
    assert!(
        cli_err(&["supervisor", "events", "--goal", &gid, "--format", "json",])
            .contains("unknown flag `--format`")
    );
}

// ── task-graph / attention ─────────────────────────────────────────────────

#[test]
fn task_graph_surface() {
    let cr = cli_root();
    let gid = init_goal(&cr, "graph goal");
    // --blocks must reference real todos (the graph fails closed on unknown
    // refs), so chain off the onboarding todo.
    let first = first_todo_id(&cr.root, &gid);
    cli_ok(&[
        "todo", "add", "--goal", &gid, "--text", "second", "--blocks", &first,
    ]);
    cli_ok(&["task-graph", "--goal", &gid]);
    // A cycle does NOT fail — it is reported (⚠ cycle) with no topo order.
    let gid2 = init_goal(&cr, "cycle goal");
    let x = first_todo_id(&cr.root, &gid2);
    cli_ok(&[
        "todo", "add", "--goal", &gid2, "--text", "y", "--blocks", &x,
    ]);
    let y = common::todo_id_by_text(&cr.root, &gid2, "y");
    cli_ok(&[
        "todo",
        "update",
        "--goal",
        &gid2,
        "--todo-id",
        &x,
        "--blocks",
        &y,
    ]);
    cli_ok(&["task-graph", "--goal", &gid2]);
    // Unknown refs fail closed.
    let gid3 = init_goal(&cr, "unknown ref goal");
    cli_ok(&[
        "todo",
        "add",
        "--goal",
        &gid3,
        "--text",
        "dangling",
        "--blocks",
        "todo_ghost",
    ]);
    assert!(cli_err(&["task-graph", "--goal", &gid3]).contains("unknown todo"));
    assert!(cli_err(&["task-graph"]).contains("--goal required"));
    assert!(cli_err(&["task-graph", "--goal", "goal_nope"]).contains("not found"));
}

#[test]
fn attention_surface() {
    let cr = cli_root();
    let gid = init_goal(&cr, "attention goal");
    cli_ok(&["attention", "--goal", &gid]);
    cli_ok(&["attention", "--all"]);
    // A goal with an open gate ranks differently in the queue.
    cli_ok(&[
        "todo",
        "add",
        "--goal",
        &gid,
        "--text",
        "needs approval",
        "--class",
        "user_gate",
    ]);
    cli_ok(&["attention", "--goal", &gid]);
    cli_ok(&["attention", "--all"]);
    assert!(cli_err(&["attention"]).contains("--all"));
    // Unknown goal id is skipped (no error).
    cli_ok(&["attention", "--goal", "goal_nope"]);
}

// ── inbox ──────────────────────────────────────────────────────────────────

#[test]
fn inbox_surface() {
    let cr = cli_root();
    // Empty project → zero pending.
    cli_ok(&["inbox", "--project", &cr.cwd]);
    // Craft inbox events under <project>/.future/loop/inbox/.
    let inbox = std::path::Path::new(&cr.cwd).join(".future/loop/inbox");
    std::fs::create_dir_all(&inbox).unwrap();
    std::fs::write(
        inbox.join("m1.json"),
        r#"{"message_id":"m1","create_time":"2026-08-10T00:00:00Z","content":"@operator can you review?","reply_context_verified":false,"reply_to_operator":false}"#,
    )
    .unwrap();
    std::fs::write(
        inbox.join("m2.json"),
        r#"{"message_id":"m2","create_time":"2026-08-10T00:01:00Z","content":"reply here","reply_context_verified":true,"reply_to_operator":true}"#,
    )
    .unwrap();
    // A malformed event file is skipped, a non-json file ignored.
    std::fs::write(inbox.join("bad.json"), "{nope").unwrap();
    std::fs::write(inbox.join("note.txt"), "hello").unwrap();
    cli_ok(&["inbox", "--project", &cr.cwd]);
    cli_ok(&[
        "inbox",
        "--project",
        &cr.cwd,
        "--scope",
        "all",
        "--name",
        "operator",
    ]);
    cli_ok(&["inbox", "--project", &cr.cwd, "--scope", "mentions"]);
}
