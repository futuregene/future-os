//! `console.rs`'s remaining named lines, batch 2.
//!
//! Selected from `cargo llvm-cov report --text` (the per-line export): each test
//! here exists to execute a specific line the report names as never reached, and
//! each asserts the operator-visible consequence - not merely that the line ran.
//!
//! These projections print to stdout, so the CLI runs as a subprocess and its
//! stdout is what gets asserted. All state lives in a tempdir created here.

use std::process::Command;

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_future-loop")
}

fn tmp_root(tag: &str) -> (String, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join(format!("root-{tag}"));
    std::fs::create_dir_all(&root).unwrap();
    (root.to_string_lossy().into_owned(), dir)
}

/// Run the CLI with an explicit state root; return (stdout, stderr, code).
fn run(root: &str, args: &[&str]) -> (String, String, i32) {
    let out = Command::new(bin())
        .env("FUTURE_LOOP_ROOT", root)
        // Never reach a developer's real agent from a test.
        .env("FUTURE_LOOP_AGENT_ADDR", "http://127.0.0.1:1")
        .args(args)
        .output()
        .expect("binary runs");
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
        out.status.code().unwrap_or(-1),
    )
}

fn ok(root: &str, args: &[&str]) -> String {
    let (stdout, stderr, code) = run(root, args);
    assert_eq!(code, 0, "cli {args:?} failed: {stderr}");
    stdout
}

fn init_goal(root: &str, objective: &str) -> String {
    let gid = format!("goal_{}", &uuid::Uuid::new_v4().simple().to_string()[..12]);
    let cwd = format!("{root}/cwd");
    std::fs::create_dir_all(&cwd).unwrap();
    ok(
        root,
        &[
            "goal",
            "init",
            "--objective",
            objective,
            "--goal-id",
            &gid,
            "--cwd",
            &cwd,
        ],
    );
    gid
}

fn todos(root: &str, goal: &str) -> Vec<future_loop::state::Todo> {
    future_loop::store::Store::open(root)
        .unwrap()
        .replay(goal)
        .unwrap()
        .unwrap()
        .todos
}

fn add_todo(root: &str, goal: &str, text: &str) -> String {
    ok(root, &["todo", "add", "--goal", goal, "--text", text]);
    todos(root, goal)
        .into_iter()
        .find(|t| t.text == text)
        .expect("the todo was added")
        .id
}

/// `task-graph` must SURFACE a dependency cycle rather than hide it: a graph with a
/// cycle has no topological order, so a bare edge list would look healthy while the
/// scheduler could never make progress.
///
/// The CLI refuses to CREATE a cycle (`todo update --blocks` fails with "dependency
/// cycle involving todo"), so the graph command's cycle arm is only reachable for a
/// ledger written by another path - an older build, or a hand-edited event. That is
/// exactly why the projection has to render it: the alternative is trusting an
/// invariant the reader cannot verify. The back-edge is therefore appended through
/// the store API, which is the same public surface any such writer uses.
#[test]
fn task_graph_renders_a_cycle_and_a_topological_order() {
    let (root, _dir) = tmp_root("graph");
    let gid = init_goal(&root, "task graph cycle");
    let bot = add_todo(&root, &gid, "bottom");
    let top = add_todo(&root, &gid, "top");
    ok(
        &root,
        &[
            "todo",
            "update",
            "--goal",
            &gid,
            "--todo-id",
            &bot,
            "--blocks",
            &top,
        ],
    );
    // The CLI guard refuses this; the store does not (it is append-only history).
    let (_, stderr, code) = run(
        &root,
        &[
            "todo",
            "update",
            "--goal",
            &gid,
            "--todo-id",
            &top,
            "--blocks",
            &bot,
        ],
    );
    assert_ne!(code, 0, "the CLI must refuse to introduce a cycle");
    assert!(
        stderr.contains("dependency cycle involving todo"),
        "and must say why: {stderr}"
    );
    let mut store = future_loop::store::Store::open(&root).unwrap();
    store
        .append(future_loop::store::Event::TodoUpdated {
            goal_id: gid.clone(),
            todo_id: top.clone(),
            text: None,
            status: None,
            evidence: None,
            note: None,
            priority: None,
            resume_when: None,
            blocks: Some(vec![bot.clone()]),
            acceptance: None,
            owner: None,
            ts: 1_700_000_001,
        })
        .unwrap();
    drop(store);

    let out = ok(&root, &["task-graph", "--goal", &gid]);
    assert!(out.contains("cycle:"), "a cycle must be rendered: {out}");
    assert!(
        out.contains(&bot) && out.contains(&top),
        "the cycle must name both mutually-blocking todos: {out}"
    );
    assert!(
        !out.contains("topological order:"),
        "a cyclic graph has no topological order: {out}"
    );

    // The acyclic control: the other arm of the same `if/else`.
    let clean = init_goal(&root, "task graph order");
    let first = add_todo(&root, &clean, "first");
    ok(
        &root,
        &[
            "todo", "add", "--goal", &clean, "--text", "second", "--blocks", &first,
        ],
    );
    let out = ok(&root, &["task-graph", "--goal", &clean]);
    assert!(
        out.contains("topological order:"),
        "an acyclic graph must render its order: {out}"
    );
    assert!(
        !out.contains("cycle:"),
        "and must not report a cycle: {out}"
    );
}

/// An unknown top-level command must name itself and point at `--help`; a bare
/// "error" leaves the operator guessing whether they mistyped or the feature is gone.
#[test]
fn unknown_command_is_reported_with_the_help_hint() {
    let (root, _dir) = tmp_root("unknown");
    let (_, stderr, code) = run(&root, &["definitely-not-a-command"]);
    assert_ne!(code, 0, "an unknown command must fail");
    assert!(
        stderr.contains("unknown command `definitely-not-a-command`"),
        "the error must name the command: {stderr}"
    );
    assert!(stderr.contains("--help"), "and point at the help: {stderr}");
}

/// The counterpart to the `_ => advancement` fallback in `todo_add`: the input
/// validator accepts EXACTLY the set of role/class pairs the later constructor
/// match enumerates, so garbage is refused up front rather than quietly becoming
/// an advancement todo. (This is why that fallback arm has no reachable input -
/// see the module doc's waiver ledger.)
#[test]
fn todo_add_refuses_an_unknown_role_class_combo() {
    let (root, _dir) = tmp_root("roleclass");
    let gid = init_goal(&root, "unknown role/class");
    let (_, stderr, code) = run(
        &root,
        &[
            "todo",
            "add",
            "--goal",
            &gid,
            "--text",
            "odd shape",
            "--role",
            "robot",
            "--class",
            "not-a-class",
        ],
    );
    assert_ne!(code, 0, "an unknown combo must be refused");
    assert!(
        stderr.contains("unknown --role/--class combo"),
        "the refusal must name the combo: {stderr}"
    );
    assert!(
        !todos(&root, &gid).iter().any(|t| t.text == "odd shape"),
        "a refused todo must not reach the ledger"
    );

    // Every combo the validator accepts must construct the class it names.
    for (role, class, want) in [
        (
            "agent",
            "advancement",
            future_loop::state::TaskClass::Advancement,
        ),
        ("agent", "monitor", future_loop::state::TaskClass::Monitor),
        ("agent", "blocker", future_loop::state::TaskClass::Blocker),
        (
            "agent",
            "coordination",
            future_loop::state::TaskClass::Coordination,
        ),
        (
            "robot",
            "user_gate",
            future_loop::state::TaskClass::UserGate,
        ),
        (
            "user",
            "user_action",
            future_loop::state::TaskClass::UserAction,
        ),
    ] {
        let text = format!("combo {role}/{class}");
        let mut args = vec![
            "todo", "add", "--goal", &gid, "--text", &text, "--role", role, "--class", class,
        ];
        if class == "user_gate" {
            args.push("--gate-question");
            args.push("decide?");
        }
        ok(&root, &args);
        // A user_gate todo's text IS its gate question (the question defaults to
        // the text only when none is given), so look it up by what it becomes.
        let lookup: &str = if class == "user_gate" {
            "decide?"
        } else {
            text.as_str()
        };
        let t = todos(&root, &gid)
            .into_iter()
            .find(|t| t.text == lookup)
            .expect("the todo exists");
        assert_eq!(t.class, want, "{role}/{class} must build {want:?}");
    }
}

/// Completing without `--force` records WHICH basis was used, and a todo carrying a
/// validator says the machine check did not run - otherwise a reviewer cannot tell a
/// hand-verified completion from a machine-verified one.
#[test]
fn completing_without_force_records_the_manual_review_basis() {
    let (root, _dir) = tmp_root("basis");
    let gid = init_goal(&root, "manual review basis");
    ok(
        &root,
        &[
            "todo",
            "add",
            "--goal",
            &gid,
            "--text",
            "verified work",
            "--verify",
            "true",
        ],
    );
    let verified = todos(&root, &gid)
        .into_iter()
        .find(|t| t.text == "verified work")
        .expect("the todo exists")
        .id;
    let plain = add_todo(&root, &gid, "plain work");

    for todo in [&verified, &plain] {
        ok(
            &root,
            &[
                "todo",
                "complete",
                "--goal",
                &gid,
                "--todo-id",
                todo,
                "--evidence",
                "looked at it",
                "--no-follow-up",
            ],
        );
    }

    let notes: Vec<(String, Option<String>)> = future_loop::store::Store::open(&root)
        .unwrap()
        .events(&gid)
        .unwrap()
        .into_iter()
        .filter_map(|e| match e.event {
            future_loop::store::Event::DeliveryOutcomeRecorded { todo_id, note, .. } => {
                Some((todo_id, note))
            }
            _ => None,
        })
        .collect();
    let note_for = |id: &str| {
        notes
            .iter()
            .find(|(t, _)| t == id)
            .and_then(|(_, n)| n.clone())
            .unwrap_or_default()
    };
    assert!(
        note_for(&plain).contains("completion_basis=manual_review"),
        "a hand-verified, validator-less completion must say so: {notes:?}"
    );
    assert!(
        note_for(&verified).contains("machine validator NOT executed"),
        "skipping the machine validator must be recorded, not implied: {notes:?}"
    );
}

/// A worker session whose run header names a run with no `.live.jsonl` must fail
/// with the path - the operator needs to know WHICH log is missing.
#[test]
fn worker_tail_reports_a_missing_live_log() {
    let (root, _dir) = tmp_root("tailmissing");
    let gid = init_goal(&root, "tail a missing log");
    let todo = add_todo(&root, &gid, "work");
    let runs = std::path::Path::new(&root).join("runs");
    std::fs::create_dir_all(&runs).unwrap();
    // The run_id comes from the HEADER, not the file name, so a header naming a run
    // that was never written leaves the log missing while the session is discoverable.
    let header = serde_json::json!({
        "type": "run_header",
        "wall_ts": 1_700_000_000u64,
        "run_id": "run_never_written",
        "session_id": "sess-1",
        "agent_id": "w1",
        "todo_id": todo,
        "goal_id": gid,
    });
    std::fs::write(runs.join("discoverable.live.jsonl"), format!("{header}\n")).unwrap();

    let (_, stderr, code) = run(&root, &["worker", "tail", "--goal", &gid]);
    assert_ne!(code, 0, "a missing live log must fail");
    assert!(
        stderr.contains("live log not found"),
        "the missing log must be named: {stderr}"
    );
    assert!(
        stderr.contains("run_never_written.live.jsonl"),
        "the missing path must be shown: {stderr}"
    );
}

/// `lane --agent-id` renders the compact lane recommendation for the agent's
/// latest attributed run; without it the operator sees nothing and cannot tell an
/// idle agent from a lane whose runs were never attributed.
#[test]
fn lane_renders_the_latest_attributed_run() {
    let (root, _dir) = tmp_root("lane");
    let gid = init_goal(&root, "lane render");
    let todo = add_todo(&root, &gid, "work for lane_w1");
    let store = future_loop::store::Store::open(&root).unwrap();
    let mut rec = future_loop::state::RunRecord {
        agent_id: Some("lane_w1".into()),
        turn: 3,
        todo_id: todo,
        run_id: "run-lane-1".into(),
        terminal_state: "completed".into(),
        error: None,
        tokens_in_delta: 1,
        tokens_out_delta: 2,
        cost_delta: 0.01,
        tools: vec!["shell".into()],
        evidence: "shipped the fix".into(),
        recorded_at: 1_700_000_500,
        spend_source: Some("run".into()),
        validation: None,
        failure_kind: None,
        truncation: None,
    };
    store.append_run(&gid, &rec).unwrap();

    // An agent with no attributed run says so instead of printing an empty lane.
    let out = ok(&root, &["lane", "--goal", &gid, "--agent-id", "nobody"]);
    assert!(
        out.contains("no lane run for agent `nobody` yet"),
        "an unknown lane must be named as such: {out}"
    );

    let out = ok(&root, &["lane", "--goal", &gid, "--agent-id", "lane_w1"]);
    assert!(out.contains("agent lane `lane_w1`"), "{out}");
    assert!(out.contains("classification=completed"), "{out}");
    assert!(
        out.contains("recommended action: shipped the fix"),
        "the run's evidence is the recommendation shown: {out}"
    );

    // A run whose evidence is blank renders NO recommendation line rather than an
    // empty one (the `None` arm).
    rec.run_id = "run-lane-2".into();
    rec.recorded_at = 1_700_000_600;
    rec.evidence = String::new();
    store.append_run(&gid, &rec).unwrap();
    drop(store);
    let out = ok(&root, &["lane", "--goal", &gid, "--agent-id", "lane_w1"]);
    assert!(
        !out.contains("recommended action:"),
        "a blank recommendation must not render a line: {out}"
    );
}

/// `agent list` renders the capabilities an agent declared at onboarding; with the
/// join skipped the column would be blank and the operator could not tell a
/// capability-less agent from a rendering bug.
#[test]
fn agent_list_renders_declared_capabilities() {
    let (root, _dir) = tmp_root("caps");
    let gid = init_goal(&root, "capabilities render");
    let mut store = future_loop::store::Store::open(&root).unwrap();
    store
        .append(future_loop::store::Event::AgentOnboarded {
            goal_id: gid.clone(),
            agent_id: "cappy".into(),
            capabilities: vec!["shell".into(), "code".into()],
            workspaces: vec![],
            ts: 1_700_000_000,
        })
        .unwrap();
    drop(store);

    let out = ok(&root, &["agent", "list", "--goal", &gid]);
    assert!(
        out.contains("shell,code"),
        "declared capabilities must render joined on commas: {out}"
    );
    // A capability-less agent still renders the placeholder, so the column is
    // never simply absent.
    let other = init_goal(&root, "no capabilities");
    ok(
        &root,
        &["agent", "onboard", "--goal", &other, "--agent-id", "bare"],
    );
    let out = ok(&root, &["agent", "list", "--goal", &other]);
    let row = out
        .lines()
        .find(|l| l.contains("bare"))
        .expect("the agent row must exist");
    assert!(
        row.split_whitespace().count() >= 5,
        "the row must still carry every column: {row}"
    );
}
