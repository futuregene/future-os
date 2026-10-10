//! `console.rs`'s `?` arms, reached by the one wedge that scales.
//!
//! `5741` was covered because `backfill`'s append loop is its FIRST ledger write, so a
//! read-only ledger makes the failure land exactly there. That generalises: for any
//! command whose first ledger write is the arm we want, the SAME wedge works - we just
//! run that one command against a wedged ledger. Each case gets its own goal pair.
//!
//! **The matrix is self-validating.** Every case is run TWICE:
//!
//! * on a control goal with a writable ledger - it MUST SUCCEED, which proves the
//!   command and its setup are valid (a wrong fixture would fail here and be caught,
//!   rather than being silently counted as a ledger failure); and
//! * on a wedged goal - it MUST FAIL.
//!
//! A case is only "good" when control-succeeds AND wedged-fails. Any other combination is
//! reported, so neither a broken fixture nor a command that reports success without
//! recording anything can pass unnoticed. (The first version of this file relied on
//! matching error text and let a wrong `gate resolve` fixture through: the command failed
//! on "not a user_gate", not on the ledger.)
//!
//! The wedge is a per-goal read-only `events.jsonl`: readable (so `replay` succeeds and
//! the command reaches its write) but not appendable - portable, since it involves no
//! permission model on the read side and behaves the same on Windows and on a
//! root-owned Linux CI.

mod common;

use common::{cli, cli_ok, cli_root, init_goal, open_store};

/// Make this goal's ledger read-only and return the saved permissions.
fn wedge_ledger(root: &str, goal: &str) -> std::fs::Permissions {
    let ledger = std::path::Path::new(root)
        .join("goals")
        .join(goal)
        .join("events.jsonl");
    assert!(
        ledger.is_file(),
        "the ledger must exist before it is wedged"
    );
    let original = std::fs::metadata(&ledger).unwrap().permissions();
    let mut locked = original.clone();
    locked.set_readonly(true);
    std::fs::set_permissions(&ledger, locked).expect("the ledger must become read-only");
    original
}

fn unwedge(root: &str, goal: &str, original: std::fs::Permissions) {
    let ledger = std::path::Path::new(root)
        .join("goals")
        .join(goal)
        .join("events.jsonl");
    let _ = std::fs::set_permissions(&ledger, original);
}

/// The onboarding todo, present in every freshly initialised goal.
fn onboarding_todo(root: &str, goal: &str) -> String {
    future_loop::store::Store::open(root)
        .unwrap()
        .replay(goal)
        .unwrap()
        .unwrap()
        .todos
        .iter()
        .find(|t| t.class == future_loop::state::TaskClass::Advancement)
        .expect("the onboarding todo must exist")
        .id
        .clone()
}

/// The id of the todo with this text.
fn todo_with(root: &str, goal: &str, text: &str) -> String {
    future_loop::store::Store::open(root)
        .unwrap()
        .replay(goal)
        .unwrap()
        .unwrap()
        .todos
        .iter()
        .find(|t| t.text == text)
        .unwrap_or_else(|| panic!("todo `{text}` must exist"))
        .id
        .clone()
}

/// Register the worker the cases assume.
fn register_worker(_root: &str, goal: &str) {
    cli_ok(&["agent", "register", "--goal", goal, "--agent-id", "w1"]);
}

/// Add a `user_gate` todo, which `gate resolve` requires.
fn add_gate(_root: &str, goal: &str) {
    cli_ok(&[
        "todo",
        "add",
        "--goal",
        goal,
        "--text",
        "approve the plan?",
        "--class",
        "user_gate",
        "--gate-question",
        "approve the plan?",
    ]);
}

/// Run one case on a control goal (must succeed) and a wedged goal (must fail).
///
/// Returns `None` when the case behaved, or a description of how it did not.
fn check_case(
    cr: &common::CliRoot,
    label: &str,
    setup: impl Fn(&str, &str),
    argv: impl Fn(&str, &str) -> Vec<String>,
) -> Option<String> {
    // Control: the same command, unwedged.
    let control = init_goal(cr, &format!("control: {label}"));
    setup(&cr.root, &control);
    let control_args = argv(&control, &cr.root);
    let control_request: Vec<&str> = control_args.iter().map(String::as_str).collect();
    match cli(&control_request) {
        Ok(()) => {}
        Err(e) => {
            return Some(format!(
                "{label}: the CONTROL run failed, so the fixture is wrong (not a ledger \
                 failure): {e}"
            ))
        }
    }

    // Wedged: identical command, ledger read-only.
    let wedged = init_goal(cr, &format!("wedged: {label}"));
    setup(&cr.root, &wedged);
    let original = wedge_ledger(&cr.root, &wedged);
    let wedged_args = argv(&wedged, &cr.root);
    let wedged_request: Vec<&str> = wedged_args.iter().map(String::as_str).collect();
    let outcome = cli(&wedged_request);
    unwedge(&cr.root, &wedged, original);
    match outcome {
        Ok(()) => Some(format!(
            "{label}: reported SUCCESS with an unwritable ledger (a state change it never \
             recorded)"
        )),
        Err(_) => None,
    }
}

fn args(parts: &[&str]) -> Vec<String> {
    parts.iter().map(|s| s.to_string()).collect()
}

/// Resolve `{g}`/`{t}`/`{gate}` against this goal.
fn resolve(argv: &[String], goal: &str, root: &str, want_gate: bool) -> Vec<String> {
    let t = onboarding_todo(root, goal);
    let gate = if want_gate {
        todo_with(root, goal, "approve the plan?")
    } else {
        String::new()
    };
    argv.iter()
        .map(|a| {
            a.replace("{g}", goal)
                .replace("{gate}", &gate)
                .replace("{t}", &t)
        })
        .collect()
}

#[test]
fn commands_surface_a_ledger_they_cannot_write() {
    let cr = cli_root();
    let mut failures: Vec<String> = Vec::new();

    // No setup needed: the goal itself is enough.
    for (label, argv) in [
        (
            "todo add",
            args(&["todo", "add", "--goal", "{g}", "--text", "unrecorded"]),
        ),
        (
            "agent register",
            args(&["agent", "register", "--goal", "{g}", "--agent-id", "w9"]),
        ),
        (
            "goal cancel",
            args(&[
                "goal",
                "cancel",
                "--goal",
                "{g}",
                "--reason",
                "unrecordable",
            ]),
        ),
        (
            "todo complete",
            args(&[
                "todo",
                "complete",
                "--goal",
                "{g}",
                "--todo-id",
                "{t}",
                "--evidence",
                "done",
                "--no-follow-up",
            ]),
        ),
        (
            "agent onboard",
            args(&["agent", "onboard", "--goal", "{g}", "--agent-id", "w9"]),
        ),
        (
            "supervisor register",
            args(&[
                "supervisor",
                "register",
                "--goal",
                "{g}",
                "--session-id",
                "sess-1",
            ]),
        ),
        (
            "supervisor steer",
            args(&[
                "supervisor",
                "steer",
                "--goal",
                "{g}",
                "--instruction",
                "change tack",
            ]),
        ),
        (
            "replan",
            args(&["replan", "--goal", "{g}", "--delta-kind", "no_followup"]),
        ),
    ] {
        let argv = argv.clone();
        if let Some(m) = check_case(&cr, label, register_worker, move |g, root| {
            resolve(&argv, g, root, false)
        }) {
            failures.push(m);
        }
    }

    // A registered agent is needed (that write is a separate arm, covered above).
    for (label, argv) in [
        (
            "todo claim",
            args(&[
                "todo",
                "claim",
                "--goal",
                "{g}",
                "--todo-id",
                "{t}",
                "--agent-id",
                "w1",
            ]),
        ),
        (
            "lease claim",
            args(&[
                "lease",
                "claim",
                "--goal",
                "{g}",
                "--todo-id",
                "{t}",
                "--agent-id",
                "w1",
                "--lease-secs",
                "600",
            ]),
        ),
        (
            "todo update",
            args(&[
                "todo",
                "update",
                "--goal",
                "{g}",
                "--todo-id",
                "{t}",
                "--priority",
                "P0",
            ]),
        ),
        (
            "todo supersede",
            args(&["todo", "supersede", "--goal", "{g}", "--todo-id", "{t}"]),
        ),
        (
            "scheduler ack",
            args(&["scheduler", "ack", "--goal", "{g}", "--action", "noop"]),
        ),
    ] {
        let argv = argv.clone();
        if let Some(m) = check_case(&cr, label, register_worker, move |g, root| {
            resolve(&argv, g, root, false)
        }) {
            failures.push(m);
        }
    }

    // `delivery record` records the VERIFICATION of an already-delivered todo, so its
    // fixture must first complete one - that completion is what records the delivery.
    if let Some(m) = check_case(
        &cr,
        "delivery record",
        |root, goal| {
            register_worker(root, goal);
            let todo = onboarding_todo(root, goal);
            cli_ok(&[
                "todo",
                "complete",
                "--goal",
                goal,
                "--todo-id",
                &todo,
                "--evidence",
                "done",
                "--no-follow-up",
            ]);
        },
        move |g, root| {
            resolve(
                &args(&[
                    "delivery",
                    "record",
                    "--goal",
                    "{g}",
                    "--todo-id",
                    "{t}",
                    "--outcome",
                    "verified",
                    "--note",
                    "checked",
                ]),
                g,
                root,
                false,
            )
        },
    ) {
        failures.push(m);
    }

    // `gate resolve` only applies to a `user_gate` todo, so the fixture must add one.
    if let Some(m) = check_case(
        &cr,
        "gate resolve",
        |root, goal| {
            register_worker(root, goal);
            add_gate(root, goal);
        },
        move |g, root| {
            resolve(
                &args(&[
                    "gate",
                    "resolve",
                    "--goal",
                    "{g}",
                    "--todo-id",
                    "{gate}",
                    "--decision",
                    "approve",
                ]),
                g,
                root,
                true,
            )
        },
    ) {
        failures.push(m);
    }

    assert!(
        failures.is_empty(),
        "every command must surface a ledger it cannot write, and every fixture must be \
         valid:\n{}",
        failures.join("\n")
    );
}

/// The control side of the matrix is asserted per case above; this test additionally
/// proves the ledger really does grow on the control path, so the cases cannot be passing
/// while nothing is written at all.
#[test]
fn the_control_path_actually_writes_events() {
    let cr = cli_root();
    let goal = init_goal(&cr, "control writes events");
    register_worker(&cr.root, &goal);
    let before = open_store(&cr).events(&goal).unwrap().len();
    cli_ok(&[
        "todo",
        "claim",
        "--goal",
        &goal,
        "--todo-id",
        &onboarding_todo(&cr.root, &goal),
        "--agent-id",
        "w1",
    ]);
    cli_ok(&["agent", "onboard", "--goal", &goal, "--agent-id", "w1"]);
    let after = open_store(&cr).events(&goal).unwrap().len();
    assert!(
        after > before,
        "the control path must append events ({before} -> {after})"
    );
}
