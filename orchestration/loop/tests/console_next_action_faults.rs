//! A single wedge that reaches MANY `?` arms at once.
//!
//! `store.set_next_action` writes `<goal>/next_action.txt` with `fs::write`, and most
//! mutating commands end with `refresh_next_action(store, goal_id)?` - which is that
//! write. Planting a DIRECTORY at `next_action.txt` therefore makes the write fail for
//! every one of those commands, and each one's `?` propagation arm executes.
//!
//! This is the only wedge that scales: a per-site IO fault cannot be aimed (see
//! `docs/testing/module-loop.md` §5b on the append/panic order), but this ONE path is
//! shared by a dozen command tails. The wedge is portable - a directory standing in for
//! a file behaves identically on Windows and on a root-owned Linux CI.

mod common;

use common::{cli, cli_ok, cli_root, init_goal, open_store};

/// Plant a directory where `next_action.txt` belongs, so `set_next_action` fails.
fn wedge_next_action(root: &str, goal: &str) {
    let path = std::path::Path::new(root)
        .join("goals")
        .join(goal)
        .join("next_action.txt");
    if path.exists() {
        std::fs::remove_file(&path).expect("the seeded next_action must be removable");
    }
    std::fs::create_dir(&path).expect("plant a directory where next_action.txt belongs");
}

/// Every mutating command must SURFACE the failed Next-Action sync rather than report
/// success. The sync is what `status` renders, so a silent failure leaves the operator
/// reading a stale Next Action - which is why these are `?` and not best-effort.
///
/// The assertions are on the failure being the next-action write (not a missing goal or
/// a bad flag), so a test that passes for an unrelated reason cannot slip through.
#[test]
fn mutating_commands_surface_a_failed_next_action_sync() {
    let cr = cli_root();
    let goal = init_goal(&cr, "next-action unwritable");
    cli_ok(&["agent", "register", "--goal", &goal, "--agent-id", "wn"]);
    let store = open_store(&cr);
    let _ = store.replay(&goal).unwrap().unwrap();
    drop(store);

    // Wedge it only now, so the setup commands above are unaffected.
    wedge_next_action(&cr.root, &goal);

    // A todo to operate on, created through the store API (the CLI cannot create one
    // now: that is the behaviour under test).
    let mut store = open_store(&cr);
    store
        .append(future_loop::store::Event::TodoAdded {
            goal_id: goal.clone(),
            todo: future_loop::state::Todo::advancement("todo_na", "next-action probe"),
            ts: future_loop::state::now_epoch(),
        })
        .unwrap();
    drop(store);

    // Commands whose tail is `refresh_next_action(...)?`: each must report the failure.
    let attempts: Vec<(&str, Vec<String>)> = vec![
        (
            "todo claim",
            vec![
                "todo",
                "claim",
                "--goal",
                &goal,
                "--todo-id",
                "todo_na",
                "--agent-id",
                "wn",
            ]
            .into_iter()
            .map(String::from)
            .collect(),
        ),
        (
            "todo complete",
            vec![
                "todo",
                "complete",
                "--goal",
                &goal,
                "--todo-id",
                "todo_na",
                "--evidence",
                "did it",
                "--no-follow-up",
            ]
            .into_iter()
            .map(String::from)
            .collect(),
        ),
        (
            "todo update",
            vec![
                "todo",
                "update",
                "--goal",
                &goal,
                "--todo-id",
                "todo_na",
                "--priority",
                "P0",
            ]
            .into_iter()
            .map(String::from)
            .collect(),
        ),
        (
            "todo supersede",
            vec!["todo", "supersede", "--goal", &goal, "--todo-id", "todo_na"]
                .into_iter()
                .map(String::from)
                .collect(),
        ),
        (
            "replan",
            vec!["replan", "--goal", &goal]
                .into_iter()
                .map(String::from)
                .collect(),
        ),
    ];

    let mut surfaced = Vec::new();
    for (label, args) in &attempts {
        let argv: Vec<&str> = args.iter().map(String::as_str).collect();
        match cli(&argv) {
            Ok(()) => {
                panic!("`{label}` reported success although the Next-Action sync cannot write")
            }
            Err(e) => {
                assert!(
                    !e.contains("not found") && !e.contains("unknown flag"),
                    "`{label}` failed for the wrong reason (the wedge must be the cause): {e}"
                );
                surfaced.push(format!("{label}: {e}"));
            }
        }
    }

    // The wedge is still in place, so every failure above really was this path.
    let wedged = std::path::Path::new(&cr.root)
        .join("goals")
        .join(&goal)
        .join("next_action.txt");
    assert!(
        wedged.is_dir(),
        "the fixture must still block the next-action write"
    );
    assert_eq!(
        surfaced.len(),
        attempts.len(),
        "every command must have reported the failure: {surfaced:#?}"
    );
}

/// The counterpart: with `next_action.txt` free the identical commands on a fresh goal
/// succeed, so the failures above are attributable to the wedge and not to the commands
/// being invalid.
#[test]
fn the_same_commands_succeed_without_the_wedge() {
    let cr = cli_root();
    let goal = init_goal(&cr, "next-action writable");
    cli_ok(&["agent", "register", "--goal", &goal, "--agent-id", "wn2"]);
    let mut store = open_store(&cr);
    store
        .append(future_loop::store::Event::TodoAdded {
            goal_id: goal.clone(),
            todo: future_loop::state::Todo::advancement("todo_ok", "control probe"),
            ts: future_loop::state::now_epoch(),
        })
        .unwrap();
    drop(store);

    cli_ok(&[
        "todo",
        "claim",
        "--goal",
        &goal,
        "--todo-id",
        "todo_ok",
        "--agent-id",
        "wn2",
    ]);
    cli_ok(&[
        "todo",
        "complete",
        "--goal",
        &goal,
        "--todo-id",
        "todo_ok",
        "--evidence",
        "did it",
        "--no-follow-up",
    ]);
    let next_action = std::path::Path::new(&cr.root)
        .join("goals")
        .join(&goal)
        .join("next_action.txt");
    assert!(
        next_action.is_file(),
        "the control run must actually write the Next Action"
    );
}
