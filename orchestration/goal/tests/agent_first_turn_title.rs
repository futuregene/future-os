//! First-turn title for a session `loop run` MINTED.
//!
//! A loop run names its own worker session so the desktop sidebar and
//! `session list` show something readable. Two boundaries matter and both are
//! asserted here against the mock agent's command log:
//!
//! - a **resumed** session (`--resume-session`, or a pinned id that was still
//!   alive) is somebody else's conversation and is never renamed;
//! - the title is **best-effort**: it is an extra model call after a turn that
//!   already succeeded, so a refusal, a timeout or an empty answer must leave
//!   the run successful and the ledger untouched.
//!
//! Everything is serialized through CLI_LOCK (process-global env), and each
//! test spawns its own mock server on an ephemeral port.

mod common;

use common::mock_agent::{completed_events, ev, spawn_mock, MockState, SharedState};
use common::{cli_err, cli_ok, cli_root, first_todo_id, init_goal, open_store};

fn rt() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap()
}

/// Spawn a mock agent and point the CLI at it.
fn mock_env(state: MockState) -> (tokio::runtime::Runtime, SharedState) {
    let rt = rt();
    let (addr, shared) = rt.block_on(spawn_mock(state));
    std::env::set_var("FUTURE_GOAL_AGENT_ADDR", &addr);
    (rt, shared)
}

/// Point `HOME` (and `USERPROFILE`) at the test's scratch dir for the duration
/// of a test.
///
/// The title language is read from the desktop app's store under `$HOME` — the
/// developer's own machine setting. Without isolation these tests would assert
/// on whatever that user happens to have chosen.
struct HomeGuard(Vec<(&'static str, Option<std::ffi::OsString>)>);

impl HomeGuard {
    fn isolate(path: &std::path::Path) -> Self {
        let keys = ["HOME", "USERPROFILE"];
        let saved = keys
            .iter()
            .map(|key| (*key, std::env::var_os(key)))
            .collect();
        for key in keys {
            std::env::set_var(key, path);
        }
        Self(saved)
    }
}

impl Drop for HomeGuard {
    fn drop(&mut self) {
        for (key, value) in self.0.drain(..) {
            match value {
                Some(value) => std::env::set_var(key, value),
                None => std::env::remove_var(key),
            }
        }
    }
}

/// A mock where the worker's session is on its first turn and a title is
/// available — the happy path for titling.
fn first_turn_mock() -> MockState {
    MockState {
        events: completed_events("mock-run-1"),
        query_count: 1,
        title: Some("Mocked title".to_string()),
        ..Default::default()
    }
}

/// A scratch loop root whose `HOME` is isolated too.
///
/// The title language is read from the desktop app's settings store under
/// `$HOME` — the developer's own machine preference. Pointing `HOME` at the
/// scratch dir keeps these tests about the code rather than about whoever is
/// running them.
fn isolated_root() -> (common::CliRoot, HomeGuard) {
    let root = cli_root();
    let guard = HomeGuard::isolate(root._dir.path());
    (root, guard)
}

/// The session a run minted is named from its first exchange.
#[test]
fn run_titles_the_session_it_creates() {
    let (cr, _home) = isolated_root();
    let goal = init_goal(&cr, "title a fresh session");
    let (_rt, shared) = mock_env(first_turn_mock());
    // One successful turn closes the todo; the loop stops on the terminal
    // decision afterwards.
    cli_ok(&["run", "--goal", &goal, "--anonymous", "--max-turns", "3"]);

    let state = shared.lock().unwrap();
    assert_eq!(
        state.title_languages,
        vec!["en"],
        "the generator got a language"
    );
    assert_eq!(
        state.renamed,
        vec!["Mocked title"],
        "the generated title must be applied to the minted session"
    );
    // The generated name replaces the objective-derived one the session was
    // created with, so it is a real rename rather than a repeat.
    assert_ne!(
        state.new_session_names, state.renamed,
        "the objective-derived title must be replaced"
    );
}

/// `--no-title` is the off switch for the extra model call.
#[test]
fn run_no_title_skips_generation() {
    let (cr, _home) = isolated_root();
    let goal = init_goal(&cr, "no title");
    let (_rt, shared) = mock_env(first_turn_mock());
    cli_ok(&[
        "run",
        "--goal",
        &goal,
        "--anonymous",
        "--max-turns",
        "3",
        "--no-title",
    ]);

    let state = shared.lock().unwrap();
    assert!(state.title_languages.is_empty());
    assert!(state.renamed.is_empty());
}

/// A resumed session is never touched: `--resume-session` points at a live
/// session, so this run did not create it.
#[test]
fn run_does_not_title_a_resumed_session() {
    let (cr, _home) = isolated_root();
    let goal = init_goal(&cr, "resume keeps its name");
    let mut mock = first_turn_mock();
    // The pinned session is alive, so the run takes the resume path.
    mock.live_sessions.insert("kept-session".to_string());
    let (_rt, shared) = mock_env(mock);
    cli_ok(&[
        "run",
        "--goal",
        &goal,
        "--anonymous",
        "--max-turns",
        "3",
        "--resume-session",
        "kept-session",
    ]);

    let state = shared.lock().unwrap();
    assert!(
        state.recorded.iter().any(|c| c == "prompt"),
        "the resumed session must still run its turn"
    );
    assert!(
        state.title_languages.is_empty(),
        "no title RPC for a resumed session: {:?}",
        state.recorded
    );
    assert!(state.renamed.is_empty());
}

/// A pinned session that is gone falls back to a fresh one — and because THIS
/// run minted it, the fresh session IS titled.
#[test]
fn run_titles_the_fresh_session_it_falls_back_to() {
    let (cr, _home) = isolated_root();
    let goal = init_goal(&cr, "dead pin falls back");
    let mut mock = first_turn_mock();
    // The mock only enforces session liveness once a session exists; seed one
    // (a session that is NOT the pinned id) so the pin really probes dead.
    mock.sessions_created = 1;
    mock.live_sessions.insert("other-live".to_string());
    let (_rt, shared) = mock_env(mock);
    cli_ok(&[
        "run",
        "--goal",
        &goal,
        "--anonymous",
        "--max-turns",
        "3",
        "--resume-session",
        "gone-session",
    ]);

    let state = shared.lock().unwrap();
    assert_eq!(
        state.new_session_names.len(),
        1,
        "the dead pin must fall back to a fresh session: {:?}",
        state.recorded
    );
    // That fresh session is this invocation's own, so it is named.
    assert_eq!(state.renamed, vec!["Mocked title"]);
}

/// A title failure (no title provider) must not fail the run: the turn's
/// ledger is intact and the exit status is still success.
#[test]
fn run_survives_a_title_failure() {
    let (cr, _home) = isolated_root();
    let goal = init_goal(&cr, "title provider missing");
    let mut mock = first_turn_mock();
    mock.title = None;
    let (_rt, shared) = mock_env(mock);
    cli_ok(&["run", "--goal", &goal, "--anonymous", "--max-turns", "3"]);

    let state = shared.lock().unwrap();
    assert_eq!(
        state.title_languages.len(),
        1,
        "the title call was attempted"
    );
    assert!(
        state.renamed.is_empty(),
        "a failed suggestion is not applied"
    );
    drop(state);
    // The turn that actually did the work is still recorded as completed.
    let store = open_store(&cr);
    let g = store.replay(&goal).unwrap().unwrap();
    let todo = g
        .todo(&first_todo_id(&cr.root, &goal))
        .expect("the todo still exists");
    assert_eq!(
        todo.status,
        future_goal::state::TodoStatus::Done,
        "a failed title must not disturb the turn's writeback"
    );
}

/// A name written while the model was working wins: the suggestion is dropped.
#[test]
fn run_does_not_overwrite_a_rename_made_during_generation() {
    let (cr, _home) = isolated_root();
    let goal = init_goal(&cr, "rename wins");
    let mut mock = first_turn_mock();
    // Simulate the user renaming the session DURING generation by having the
    // mock apply the name itself before answering the title request. The
    // post-generation re-check then sees a name it did not generate.
    mock.rename_during_title = Some("User's own name".to_string());
    let (_rt, shared) = mock_env(mock);
    cli_ok(&["run", "--goal", &goal, "--anonymous", "--max-turns", "3"]);

    let state = shared.lock().unwrap();
    assert_eq!(state.title_languages, vec!["en"]);
    assert!(
        state.renamed.is_empty(),
        "the stale suggestion must not be applied over the user's rename: {:?}",
        state.renamed
    );
    assert_eq!(
        state.session_name, "User's own name",
        "the name the user set during generation must survive"
    );
}

/// A session that is not on its first turn (history, or still streaming) must
/// not be titled — and must not spend a model call to find that out.
#[test]
fn run_skips_titling_when_the_session_is_past_its_first_turn() {
    let (cr, _home) = isolated_root();
    let goal = init_goal(&cr, "second turn is not first");
    let mut mock = first_turn_mock();
    mock.query_count = 4;
    let (_rt, shared) = mock_env(mock);
    cli_ok(&["run", "--goal", &goal, "--anonymous", "--max-turns", "3"]);

    let state = shared.lock().unwrap();
    assert!(state.title_languages.is_empty());
    assert!(state.renamed.is_empty());
}

/// A turn that FAILED is not titled: the session was minted, but nothing in it
/// is worth naming, and a failed run must not gain extra work.
#[test]
fn run_does_not_title_after_a_failed_turn() {
    let (cr, _home) = isolated_root();
    let goal = init_goal(&cr, "failed turn is not titled");
    let mut mock = first_turn_mock();
    mock.events = vec![ev(
        "mock-run-1",
        0,
        "agent_end",
        "{\"state\":\"error\",\"error\":\"boom\"}",
    )];
    let (_rt, shared) = mock_env(mock);
    // The turn fails and the todo stays runnable, so max-turns ends the run.
    let _ = cli_err(&["run", "--goal", &goal, "--anonymous", "--max-turns", "1"]);

    let state = shared.lock().unwrap();
    assert!(
        state.title_languages.is_empty(),
        "a failed turn must not generate a title"
    );
    assert!(state.renamed.is_empty());
}
