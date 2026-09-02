//! G-26 CLI registry contract tests — golden coverage of the command
//! surface. Runs the REAL built binary (`CARGO_BIN_EXE_future-loop`) against a
//! temp `FUTURE_LOOP_ROOT`: the registry-driven help must list every
//! pre-existing command in its group, the pre-existing command BEHAVIOR must
//! be unchanged (goal init → status → todo add → quota should-run), and the
//! P3 additions (scope / supervisor / handoff / task-graph / attention /
//! registry) must dispatch.

use std::process::Command;

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_future-loop")
}

fn tmp_root(tag: &str) -> String {
    let dir = std::env::temp_dir().join(format!(
        "future-loop-p3-cli-{tag}-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir.to_string_lossy().into_owned()
}

/// Run the binary with an isolated FUTURE_LOOP_ROOT. Returns (stdout, stderr,
/// exit code).
fn run(root: &str, args: &[&str]) -> (String, String, i32) {
    let output = Command::new(bin())
        .env("FUTURE_LOOP_ROOT", root)
        .args(args)
        .output()
        .expect("loopx binary runs");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code().unwrap_or(-1),
    )
}

/// ── Help surface: every pre-existing command is listed in its group ──────
#[test]
fn help_lists_all_pre_existing_commands_in_groups() {
    let root = tmp_root("help");
    let (out, _, code) = run(&root, &["--help"]);
    assert_eq!(code, 0);
    for expected in [
        "── goal ──",
        "goal init --objective",
        "status [--goal G]",
        "── todo ──",
        "todo add|update|claim|complete|archive",
        "gate resolve --goal G",
        "── agent ──",
        "agent onboard --goal G",
        "| list|contract|recipe|succession|collective --goal G",
        "── ops ──",
        "quota should-run --goal G",
        "scheduler tick|show|record-host-failure",
        "backup --goal G",
        "run --goal G",
        "── work-items ──",
        "attention [--goal G] [--all]",
        "── cli ──",
        "registry [--format json|--json]",
    ] {
        assert!(
            out.contains(expected),
            "help must contain `{expected}`\n{out}"
        );
    }
    // The registry keeps the experimental-visibility flag: both plain and
    // include-experimental help renders exit 0.
    let (_, _, code_x) = run(&root, &["--help", "--include-experimental"]);
    assert_eq!(code_x, 0);
}

/// ── Golden: pre-existing command behavior is unchanged ────────────────────
#[test]
fn pre_existing_goal_todo_quota_flow_is_unchanged() {
    let root = tmp_root("flow");
    let (out, err, code) = run(
        &root,
        &[
            "goal",
            "init",
            "--objective",
            "golden objective",
            "--goal-id",
            "g1",
            "--cwd",
            "/tmp",
        ],
    );
    assert_eq!(code, 0, "goal init failed: {err}");
    assert!(out.contains("goal g1 created"), "goal init output: {out}");

    // todo add + claim + status.
    let (_out, err, code) = run(
        &root,
        &[
            "todo",
            "add",
            "--goal",
            "g1",
            "--role",
            "agent",
            "--class",
            "advancement",
            "--text",
            "work item",
        ],
    );
    assert_eq!(code, 0, "todo add failed: {err}");
    let (out, _, code) = run(&root, &["status", "--goal", "g1"]);
    assert_eq!(code, 0);
    assert!(
        out.contains("agent open=2"),
        "status shows the added todo: {out}"
    );

    // quota should-run (deterministic decision packet).
    let (out, err, code) = run(&root, &["quota", "should-run", "--goal", "g1"]);
    assert_eq!(code, 0, "quota should-run failed: {err}");
    assert!(
        out.contains("should-run") || out.contains("decision"),
        "quota renders: {out}"
    );
}

/// ── Golden: unknown commands fail with the aggregated-help hint ──────────
#[test]
fn unknown_command_fails_closed_with_hint() {
    let root = tmp_root("unknown");
    let (out, err, code) = run(&root, &["frobnicate"]);
    assert_ne!(code, 0);
    assert!(
        err.contains("unknown command `frobnicate`"),
        "stderr: {err}"
    );
    let _ = out;
}

/// ── P3: agent scope / supervisor / task-graph / attention ──────────────
#[test]
fn p3_multi_agent_and_work_item_commands() {
    let root = tmp_root("p3");
    let (_, err, code) = run(
        &root,
        &[
            "goal",
            "init",
            "--objective",
            "obj",
            "--goal-id",
            "g1",
            "--cwd",
            "/tmp",
        ],
    );
    assert_eq!(code, 0, "goal init: {err}");

    // Two agents claim two todos; each frontier excludes the other's claim.
    let (out, err, code) = run(
        &root,
        &[
            "todo",
            "add",
            "--goal",
            "g1",
            "--role",
            "agent",
            "--class",
            "advancement",
            "--text",
            "A work",
        ],
    );
    assert_eq!(code, 0, "todo add A: {err}");
    let _ = out;
    run(
        &root,
        &[
            "todo",
            "add",
            "--goal",
            "g1",
            "--role",
            "agent",
            "--class",
            "advancement",
            "--text",
            "B work",
        ],
    );
    run(
        &root,
        &["agent", "onboard", "--goal", "g1", "--agent-id", "agent-a"],
    );
    run(
        &root,
        &["agent", "onboard", "--goal", "g1", "--agent-id", "agent-b"],
    );
    // Todo ids from status output (todo-<hex>).
    let (out, _, _) = run(&root, &["status", "--goal", "g1"]);
    let ids: Vec<String> = out
        .lines()
        .flat_map(|l| l.split_whitespace())
        .filter_map(|w| w.strip_suffix("=open").map(|s| s.to_string()))
        .filter(|s| s.starts_with("todo_"))
        .collect();
    assert!(ids.len() >= 3, "status shows todos: {out}");
    let (t_a, t_b) = (&ids[0], &ids[1]);
    run(
        &root,
        &[
            "todo",
            "claim",
            "--goal",
            "g1",
            "--todo-id",
            t_a,
            "--agent-id",
            "agent-a",
        ],
    );
    run(
        &root,
        &[
            "todo",
            "claim",
            "--goal",
            "g1",
            "--todo-id",
            t_b,
            "--agent-id",
            "agent-b",
        ],
    );

    let (out_a, _, _) = run(&root, &["scope", "--goal", "g1", "--agent-id", "agent-a"]);
    let (out_b, _, _) = run(&root, &["scope", "--goal", "g1", "--agent-id", "agent-b"]);
    // The visible-agent line holds A's claim but never B's; B's claim appears
    // only as the boundary marker (outside this frontier).
    let visible_a = out_a
        .lines()
        .find(|l| l.contains("visible agent todos"))
        .unwrap();
    assert!(visible_a.contains(t_a), "A's frontier holds A's claim");
    assert!(
        !visible_a.contains(t_b),
        "A's visible frontier never shows B's claim"
    );
    assert!(
        out_a.contains(&format!("{t_b}  ← outside this frontier")),
        "B's claim is marked outside A's frontier"
    );
    let visible_b = out_b
        .lines()
        .find(|l| l.contains("visible agent todos"))
        .unwrap();
    assert!(visible_b.contains(t_b), "B's frontier holds B's claim");
    assert!(
        !visible_b.contains(t_a),
        "B's visible frontier never shows A's claim"
    );
    assert!(
        out_b.contains(&format!("{t_a}  ← outside this frontier")),
        "A's claim is marked outside B's frontier"
    );

    // Supervisor proposal + receipt + events.
    let (_, err, code) = run(
        &root,
        &[
            "supervisor",
            "propose",
            "--goal",
            "g1",
            "--agent-id",
            "sup-1",
            "--decision-id",
            "d1",
            "--target-agent-id",
            "agent-b",
            "--kind",
            "execute",
            "--capabilities",
            "github",
            "--summary",
            "merge",
        ],
    );
    assert_eq!(code, 0, "propose: {err}");
    let (_, err, code) = run(
        &root,
        &[
            "supervisor",
            "receipt",
            "--goal",
            "g1",
            "--decision-id",
            "d1",
            "--receipt-id",
            "r1",
            "--adapter-id",
            "a",
            "--outcome",
            "executed",
            "--authority-ref",
            "auth",
            "--host-capabilities",
            "github",
        ],
    );
    assert_eq!(code, 0, "receipt: {err}");
    let (out, _, _) = run(&root, &["supervisor", "events", "--goal", "g1"]);
    assert!(
        out.contains("\"execution_status\": \"executed\""),
        "projection: {out}"
    );

    // Supervisor register (up-channel target) + steer (down-channel interrupt).
    let (_, err, code) = run(
        &root,
        &[
            "supervisor",
            "register",
            "--goal",
            "g1",
            "--session-id",
            "sup-sess-1",
        ],
    );
    assert_eq!(code, 0, "register: {err}");
    // The registration folds into the goal's supervisor_session_id on replay.
    let (out, _, code) = run(&root, &["status", "--goal", "g1"]);
    assert_eq!(code, 0);
    assert!(out.contains("g1"), "status renders: {out}");
    let (_, err, code) = run(
        &root,
        &[
            "supervisor",
            "steer",
            "--goal",
            "g1",
            "--instruction",
            "do it differently",
        ],
    );
    assert_eq!(code, 0, "steer: {err}");

    // Task-graph + attention.
    let (out, _, code) = run(&root, &["task-graph", "--goal", "g1"]);
    assert_eq!(code, 0);
    assert!(out.contains("task graph:"));
    let (out, _, code) = run(&root, &["attention", "--goal", "g1"]);
    assert_eq!(code, 0);
    assert!(out.contains("attention queue:"));

    // registry introspection.
    let (out, _, code) = run(&root, &["registry", "--json"]);
    assert_eq!(code, 0);
    assert!(out.contains("\"group\": \"goal\""));
}

/// ── P3 acceptance: two agent sessions never cross frontiers (CLI view) ───
#[test]
fn two_agent_sessions_hold_disjoint_frontiers() {
    let root = tmp_root("disjoint");
    run(
        &root,
        &[
            "goal",
            "init",
            "--objective",
            "obj",
            "--goal-id",
            "g1",
            "--cwd",
            "/tmp",
        ],
    );
    run(
        &root,
        &[
            "todo",
            "add",
            "--goal",
            "g1",
            "--role",
            "agent",
            "--class",
            "advancement",
            "--text",
            "A work",
        ],
    );
    run(
        &root,
        &[
            "todo",
            "add",
            "--goal",
            "g1",
            "--role",
            "agent",
            "--class",
            "advancement",
            "--text",
            "B work",
        ],
    );
    run(
        &root,
        &["agent", "onboard", "--goal", "g1", "--agent-id", "agent-a"],
    );
    run(
        &root,
        &["agent", "onboard", "--goal", "g1", "--agent-id", "agent-b"],
    );
    let (out, _, _) = run(&root, &["status", "--goal", "g1"]);
    let ids: Vec<String> = out
        .lines()
        .flat_map(|l| l.split_whitespace())
        .filter_map(|w| w.strip_suffix("=open").map(|s| s.to_string()))
        .filter(|s| s.starts_with("todo_"))
        .collect();
    run(
        &root,
        &[
            "todo",
            "claim",
            "--goal",
            "g1",
            "--todo-id",
            &ids[0],
            "--agent-id",
            "agent-a",
        ],
    );
    run(
        &root,
        &[
            "todo",
            "claim",
            "--goal",
            "g1",
            "--todo-id",
            &ids[1],
            "--agent-id",
            "agent-b",
        ],
    );
    // claim is exclusive while the holder is ALIVE; the CLI subprocess that
    // claimed for agent-b has exited, so its dead holder's lease is reclaimed
    // by the next claim (lease liveness — killed runs must not wedge the
    // frontier). The live-holder refusal is covered in-process by
    // claim_lease_contract::claim_refuses_live_holder_pid.
    let (out, err, code) = run(
        &root,
        &[
            "todo",
            "claim",
            "--goal",
            "g1",
            "--todo-id",
            &ids[1],
            "--agent-id",
            "agent-a",
        ],
    );
    assert_eq!(code, 0, "dead holder's lease must be reclaimed: {err}");
    assert!(out.contains("claimed"), "reclaim: {out}");
}

/// ── P4: version / doctor / history / turn / todo-event / evidence-log ────
#[test]
fn p4_diagnostics_commands() {
    let root = tmp_root("p4diag");
    let (out, err, code) = run(&root, &["version"]);
    assert_eq!(code, 0, "version: {err}");
    assert!(out.contains("future-loop 0.1.0"));

    let (_, err, code) = run(
        &root,
        &[
            "goal",
            "init",
            "--objective",
            "obj",
            "--goal-id",
            "g1",
            "--cwd",
            "/tmp",
        ],
    );
    assert_eq!(code, 0, "goal init: {err}");
    let (out, err, code) = run(
        &root,
        &[
            "todo",
            "add",
            "--goal",
            "g1",
            "--role",
            "agent",
            "--class",
            "advancement",
            "--text",
            "implement feature",
        ],
    );
    assert_eq!(code, 0, "todo add: {err}");
    let _ = out;

    // doctor passes on a healthy root.
    let (out, err, code) = run(&root, &["doctor"]);
    assert_eq!(code, 0, "doctor failed: {err}");
    assert!(out.contains("ALL CHECKS PASSED"), "doctor: {out}");

    // history (no runs yet) and turn envelope render.
    let (out, _, code) = run(&root, &["history", "--goal", "g1"]);
    assert_eq!(code, 0);
    assert!(out.contains("no runs recorded"));
    let (out, _, _) = run(&root, &["status", "--goal", "g1"]);
    let todo = out
        .lines()
        .flat_map(|l| l.split_whitespace())
        .filter_map(|w| w.strip_suffix("=open").map(|s| s.to_string()))
        .find(|s| s.starts_with("todo_"))
        .unwrap();
    let (out, err, code) = run(&root, &["turn", "--goal", "g1", "--todo-id", &todo]);
    assert_eq!(code, 0, "turn: {err}");
    assert!(out.contains("future_loop_turn_envelope_v0"), "turn: {out}");
    assert!(out.contains("Complete the todo and report what you did and observed."));

    // todo-event + evidence-log after completion with evidence.
    let (_, err, code) = run(
        &root,
        &[
            "todo",
            "complete",
            "--goal",
            "g1",
            "--todo-id",
            &todo,
            "--no-follow-up",
            "--evidence",
            "implemented and verified",
        ],
    );
    assert_eq!(code, 0, "complete: {err}");
    let (out, _, code) = run(&root, &["todo-event", "--goal", "g1", "--todo-id", &todo]);
    assert_eq!(code, 0);
    assert!(out.contains("todo_added"), "todo-event: {out}");
    assert!(out.contains("todo_completed"), "todo-event: {out}");
    let (out, _, code) = run(&root, &["evidence-log", "--goal", "g1"]);
    assert_eq!(code, 0);
    assert!(
        out.contains("implemented and verified"),
        "evidence-log: {out}"
    );
}

/// ── P4: canary smoke through the CLI (release gate) ──────────────────────
#[test]
fn p4_canary_smoke_via_cli() {
    let root = tmp_root("p4canary");
    let (_, err, code) = run(
        &root,
        &[
            "goal",
            "init",
            "--objective",
            "obj",
            "--goal-id",
            "g1",
            "--cwd",
            "/tmp",
        ],
    );
    assert_eq!(code, 0, "goal init: {err}");
    let (out, err, code) = run(&root, &["canary", "smoke"]);
    assert_eq!(code, 0, "canary smoke: {err}");
    assert!(out.contains("result: ALL PASSED"), "canary: {out}");
    // --json renders the machine-readable run
    let (out, _, code) = run(&root, &["canary", "smoke", "--json"]);
    assert_eq!(code, 0);
    assert!(out.contains("\"schema_version\": \"canary_smoke_run_v0\""));
    // unknown profile fails closed
    let (_, err, code) = run(&root, &["canary", "smoke", "--profile", "nope"]);
    assert_ne!(code, 0);
    assert!(err.contains("unknown smoke profile"));
}

/// ── P1-6: canary premerge gate through the CLI ───────────────────────────
#[test]
fn p1_6_canary_premerge_via_cli() {
    let root = tmp_root("p16premerge");
    // The premerge gate runs on an isolated root (the operator root is
    // irrelevant); it must pass and be non-vacuous.
    let (out, err, code) = run(&root, &["canary", "premerge"]);
    assert_eq!(code, 0, "canary premerge: {err}");
    assert!(out.contains("gate: PASS"), "premerge: {out}");
    // --json renders the machine-readable gate report.
    let (out, _, code) = run(&root, &["canary", "premerge", "--json"]);
    assert_eq!(code, 0);
    assert!(
        out.contains("\"schema_version\": \"canary_premerge_gate_v0\""),
        "{out}"
    );
    assert!(out.contains("\"passed\": true"), "{out}");
    // Unknown subcommands fail closed (P0-3 hard-error rule).
    let (_, err, code) = run(&root, &["canary", "bogus"]);
    assert_ne!(code, 0);
    assert!(err.contains("unknown canary subcommand"), "{err}");
}

/// ── P4: help surface lists the new groups ────────────────────────────────
#[test]
fn p4_help_lists_new_groups_and_commands() {
    let root = tmp_root("p4help");
    let (out, _, code) = run(&root, &["--help"]);
    assert_eq!(code, 0);
    for expected in [
        "── canary ──",
        "canary smoke [--profile",
        "── ops ──",
        "version",
        "doctor [--goal G]",
        "history --goal G",
        "turn --goal G --todo-id T",
        "todo-event --goal G --todo-id T",
        "evidence-log --goal G",
    ] {
        assert!(
            out.contains(expected),
            "help must contain `{expected}`\n{out}"
        );
    }
}
