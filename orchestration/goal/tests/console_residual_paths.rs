//! Residual console paths that need no agent: argv-level refusals and the
//! documented classification defaults.
//!
//! Everything here goes through the real CLI entry point (`future_goal::console::run`),
//! so the assertions are about what an operator actually sees.

mod common;

use common::{cli_err, cli_ok, cli_root, init_goal, open_store, todo_id_by_text};

/// A typo in the command name must name the command and point at help: the
/// dispatcher is the only thing that can tell an operator what it understood.
#[test]
fn an_unknown_command_is_refused_by_name() {
    let _cr = cli_root();
    let error = cli_err(&["definitely-not-a-command"]);
    assert!(
        error.contains("unknown command `definitely-not-a-command`"),
        "{error}"
    );
    assert!(error.contains("--help"), "{error}");
}

/// The documented defaults of `todo add` are the todo classes the goal plane
/// uses for plain work: an agent todo with no class is an advancement todo.
#[test]
fn a_defaulted_todo_is_an_advancement_todo_owned_by_an_agent() {
    let cr = cli_root();
    let goal = init_goal(&cr, "default todo class");
    cli_ok(&["todo", "add", "--goal", &goal, "--text", "plain work"]);
    let todo_id = todo_id_by_text(&cr.root, &goal, "plain work");
    let store = open_store(&cr);
    let todo = store
        .replay(&goal)
        .unwrap()
        .unwrap()
        .todo(&todo_id)
        .cloned()
        .expect("the todo was recorded");
    assert_eq!(
        todo.class,
        future_goal::state::TaskClass::Advancement,
        "a todo with no --class is an advancement todo"
    );

    // The pairs the CLI refuses outright (they never reach the class table).
    let error = cli_err(&[
        "todo",
        "add",
        "--goal",
        &goal,
        "--text",
        "unsupported",
        "--role",
        "user",
        "--class",
        "advancement",
    ]);
    assert!(error.contains("unknown --role/--class combo"), "{error}");
}
