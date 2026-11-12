//! Coverage drive for the gRPC-backed console commands: `run`, `models`,
//! and the `doctor --agent-addr` probe — all against the in-process mock
//! FutureAgent (FUTURE_GOAL_AGENT_ADDR points the CLI at the mock).
//!
//! Everything here is serialized through CLI_LOCK (process-global env), and
//! each test spawns its own mock server on an ephemeral port.

mod common;

use common::mock_agent::{completed_events, ev, spawn_mock, AttachPlan, MockState};
#[cfg(unix)]
use common::todo_id_by_text;
use common::{add_todo, cli, cli_err, cli_ok, cli_root, first_todo_id, init_goal, open_store};
use future_goal::state::{now_epoch, TodoStatus};

fn rt() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap()
}

/// Spawn a mock agent and point the CLI at it. Returns the runtime guard
/// (keeps the server task alive) and the shared state.
fn mock_env(state: MockState) -> (tokio::runtime::Runtime, common::mock_agent::SharedState) {
    let rt = rt();
    let (addr, shared) = rt.block_on(spawn_mock(state));
    std::env::set_var("FUTURE_GOAL_AGENT_ADDR", &addr);
    (rt, shared)
}

// ── run identity gate (no server needed) ───────────────────────────────────

#[test]
fn run_requires_agent_id_or_anonymous() {
    let cr = cli_root();
    let goal = init_goal(&cr, "identity gate");
    let err = cli_err(&["run", "--goal", &goal]);
    assert!(err.contains("requires --agent-id"), "{err}");
}

#[test]
fn run_missing_goal_fails() {
    let _cr = cli_root();
    let err = cli_err(&["run", "--goal", "goal_nope", "--agent-id", "a1"]);
    assert!(err.contains("not found"), "{err}");
}

// ── run turn loop against the mock ─────────────────────────────────────────

/// `--model` and `--thinking-level` are applied to the session before the first
/// prompt, so a run's model choice cannot be inherited from whatever the session
/// default happened to be. Both are optional and must stay optional: an unset
/// flag must issue no call at all.
/// The live-log tee and the sentence-boundary text join, both driven by the
/// stream's payload shape rather than by the run's outcome.
///
/// - A `tool_start` carrying `phase` must copy it into the live line (consumers
///   dedup input-vs-execution starts on it), a `usage` event must tee its usage
///   object, and a `text_chunk` must tee its text.
/// - Two consecutive `text_chunk` events where the first ends a sentence and the
///   second begins a letter must be joined with a space; mid-word and
///   `3.14`/`example.com` must not.
#[test]
fn live_log_tees_phases_usage_and_joins_sentence_boundaries() {
    let cr = cli_root();
    let (_rt, _shared) = mock_env(MockState {
        events: vec![
            ev("mock-run-1", 0, "agent_start", "{}"),
            // Both tool phases: `input` then `execution`.
            ev(
                "mock-run-1",
                1,
                "tool_start",
                r#"{"tool_name":"shell","phase":"input"}"#,
            ),
            ev(
                "mock-run-1",
                2,
                "tool_start",
                r#"{"tool_name":"shell","phase":"execution"}"#,
            ),
            // A sentence end followed by a letter: the join must add a space.
            ev(
                "mock-run-1",
                3,
                "text_chunk",
                r#"{"text":"first sentence."}"#,
            ),
            ev(
                "mock-run-1",
                4,
                "text_chunk",
                r#"{"text":"Second sentence"}"#,
            ),
            // A "sentence end" followed by a digit must NOT get a space.
            ev("mock-run-1", 5, "text_chunk", r#"{"text":"pi is 3."}"#),
            ev("mock-run-1", 6, "text_chunk", r#"{"text":"14 exactly"}"#),
            // Usage is teed for the dashboard.
            ev(
                "mock-run-1",
                7,
                "usage",
                r#"{"usage":{"total_tokens":42,"credit_cost":0.25}}"#,
            ),
            ev(
                "mock-run-1",
                8,
                "agent_end",
                r#"{"state":"completed","tokens_in":1,"tokens_out":2}"#,
            ),
        ],
        ..Default::default()
    });
    let goal = init_goal(&cr, "stream shaping");
    cli_ok(&["run", "--goal", &goal, "--anonymous", "--max-turns", "2"]);

    // The live log is the tee target; the mock's first prompt mints `mock-run-1`.
    let live = std::path::Path::new(&cr.root)
        .join("runs")
        .join("mock-run-1.live.jsonl");
    let text = std::fs::read_to_string(&live)
        .unwrap_or_else(|e| panic!("the run must write {}: {e}", live.display()));
    assert!(
        text.contains(r#""phase":"input""#),
        "the input phase must be teed: {text}"
    );
    assert!(
        text.contains(r#""phase":"execution""#),
        "the execution phase must be teed: {text}"
    );
    assert!(
        text.contains(r#""usage""#) && text.contains("42"),
        "the usage object must be teed for the dashboard: {text}"
    );
    assert!(
        text.contains("Second sentence") && text.contains("14 exactly"),
        "text chunks must be teed verbatim: {text}"
    );

    // The join is observable in the run's evidence (the assistant summary).
    let store = open_store(&cr);
    let record = store
        .replay(&goal)
        .unwrap()
        .unwrap()
        .history
        .last()
        .cloned()
        .expect("the run must record a history entry");
    let summary = record.evidence;
    assert!(
        summary.contains("first sentence. Second sentence"),
        "a sentence end followed by a letter must be joined with a space: {summary}"
    );
    assert!(
        summary.contains("pi is 3.14"),
        "a period followed by a digit must NOT be spaced: {summary}"
    );
}

/// A turn whose first prompt returns `duplicate_request_conflict` must be
/// retried once with a **suffixed** client_request_id, and the retried turn must
/// complete normally. This is the recovery path for a worker that was stopped and
/// restarted inside the same turn: its previous aborted prompt still holds the
/// run-queue key with a different payload digest.
#[test]
fn a_duplicate_request_conflict_is_retried_with_a_suffixed_key() {
    let cr = cli_root();
    let (_rt, shared) = mock_env(MockState {
        events: completed_events("mock-run-1"),
        // Exactly one failure, then the retry succeeds.
        fail_once: [(
            "prompt".to_string(),
            "agent error: duplicate_request_conflict (mock)".to_string(),
        )]
        .into_iter()
        .collect(),
        ..Default::default()
    });
    let goal = init_goal(&cr, "conflict retry");

    cli_ok(&["run", "--goal", &goal, "--anonymous", "--max-turns", "2"]);

    // Two prompt attempts: the rejected one and the retry. `recorded` counts
    // every command (including failures), which `prompt_calls` does not.
    let attempts = shared
        .lock()
        .unwrap()
        .recorded
        .iter()
        .filter(|c| c.as_str() == "prompt")
        .count();
    assert_eq!(attempts, 2, "one retry, not a storm: {attempts} attempts");
    // The retry's idempotency key differs from the original (a genuinely new
    // request) and keeps the turn/todo prefix — which is what preserves dedup
    // protection while letting the relaunch survive.
    let ids = shared.lock().unwrap().prompt_request_ids.clone();
    assert_eq!(
        ids.len(),
        1,
        "only the accepted attempt is recorded: {ids:?}"
    );
    let accepted = &ids[0];
    assert!(
        accepted.contains("-r"),
        "the accepted key must carry the retry suffix: {accepted}"
    );
    assert!(
        accepted.starts_with("turn-1-"),
        "the suffixed key keeps the turn/todo prefix: {accepted}"
    );
    let store = open_store(&cr);
    let recorded = store.replay(&goal).unwrap().unwrap().history.len();
    assert!(recorded >= 1, "the retried turn must record its run");
}

/// When the RETRY itself fails, the error must carry the whole story: the
/// retry's own failure plus the original key it replaced. Losing that context
/// would leave an operator staring at a bare conflict message with no hint that
/// a retry happened or which original key is stuck in the agent's run queue.
#[test]
fn a_failed_retry_reports_the_original_key_it_replaced() {
    let cr = cli_root();
    let (_rt, shared) = mock_env(MockState {
        events: completed_events("mock-run-1"),
        // Both the original prompt and its retry are rejected.
        fail_queue: [(
            "prompt".to_string(),
            vec![
                "agent error: duplicate_request_conflict (first)".to_string(),
                "agent error: duplicate_request_conflict (retry also failed)".to_string(),
            ],
        )]
        .into_iter()
        .collect(),
        ..Default::default()
    });
    let goal = init_goal(&cr, "failed retry");

    let err = cli(&["run", "--goal", &goal, "--anonymous", "--max-turns", "2"])
        .expect_err("a doubly-failed prompt must surface as an error");
    assert!(
        err.contains("prompt retry with suffixed client_request_id"),
        "the retry's failure must be labelled as a retry: {err}"
    );
    assert!(
        err.contains("original key: turn-"),
        "the message must name the original key that was replaced: {err}"
    );
    // Exactly two attempts were made: the original and one retry, not a loop.
    let attempts = shared
        .lock()
        .unwrap()
        .recorded
        .iter()
        .filter(|c| c.as_str() == "prompt")
        .count();
    assert_eq!(attempts, 2, "one retry, never a retry storm: {attempts}");
}

/// A prompt failure that is NOT the duplicate-conflict must propagate: retrying
/// an arbitrary agent error would double-execute a turn that may have run.
#[test]
fn a_non_conflict_prompt_failure_is_not_retried() {
    let cr = cli_root();
    let (_rt, shared) = mock_env(MockState {
        events: completed_events("mock-run-1"),
        fail_commands: ["prompt".to_string()].into_iter().collect(),
        ..Default::default()
    });
    let goal = init_goal(&cr, "no blind retry");
    let _ = cli(&["run", "--goal", &goal, "--anonymous", "--max-turns", "2"]);
    let attempts = shared
        .lock()
        .unwrap()
        .recorded
        .iter()
        .filter(|c| c.as_str() == "prompt")
        .count();
    assert_eq!(
        attempts, 1,
        "a generic prompt failure must fail fast, not retry: {attempts} attempts"
    );
}

#[test]
fn run_applies_model_and_thinking_level_only_when_asked() {
    let cr = cli_root();
    let (_rt, shared) = mock_env(MockState {
        events: completed_events("mock-run-1"),
        ..Default::default()
    });
    let goal = init_goal(&cr, "model selection");

    // Neither flag set: no selection command reaches the agent.
    cli_ok(&["run", "--goal", &goal, "--anonymous", "--max-turns", "2"]);
    let recorded = shared.lock().unwrap().recorded.clone();
    assert!(
        !recorded.iter().any(|c| c == "set_model"),
        "an unset --model must not issue a selection call: {recorded:?}"
    );
    assert!(
        !recorded.iter().any(|c| c == "set_thinking_level"),
        "an unset --thinking-level must not issue a selection call: {recorded:?}"
    );

    // Both set: each is applied for the run.
    cli_ok(&[
        "run",
        "--goal",
        &goal,
        "--anonymous",
        "--model",
        "future/test-model",
        "--thinking-level",
        "high",
        "--max-turns",
        "2",
    ]);
    let recorded = shared.lock().unwrap().recorded.clone();
    assert!(
        recorded.iter().any(|c| c == "set_model"),
        "--model must reach the agent: {recorded:?}"
    );
    assert!(
        recorded.iter().any(|c| c == "set_thinking_level"),
        "--thinking-level must reach the agent: {recorded:?}"
    );
}

// ── incomplete retry: continue, do not replay ──────────────────────────────
//
// A truncated model stream is an infra event: the todo stays runnable, no
// repair budget is spent, and the next turn carries a `CONTINUE:` note. What
// that note MEANS is "pick up where you left off" — so the two properties
// asserted here are the ones that make it true rather than decorative:
//   1. the truncated turn's own partial text reaches the next turn's envelope
//      (that text is the only carrier of "where you left off" a worker has —
//      an interactive session keeps the assistant message in history, a worker
//      does not), and
//   2. the interrupted todo is not re-split, re-added or otherwise reset, and
//      `failed_attempts` stays 0.
// `incomplete_continue_note_carries_no_progress_when_the_cut_left_no_body`
// pins the inverse: with `output_len == 0` there is no text to carry, which is
// exactly the shape that degrades into "redo the whole turn".

/// One turn that ends `incomplete` from an output-limit cut AFTER writing a
/// partial body, then a second turn that completes. The scripted run ids are
/// `mock-run-1` / `mock-run-2` (the mock numbers them in prompt order).
fn truncated_then_completed_events() -> Vec<future_rpc::proto::StreamEvent> {
    vec![
        ev("mock-run-1", 0, "agent_start", "{}"),
        ev(
            "mock-run-1",
            1,
            "tool_start",
            "{\"tool_name\":\"write\",\"phase\":\"execution\"}",
        ),
        // The partial body — a prefix, not a finished answer.
        ev(
            "mock-run-1",
            2,
            "text_chunk",
            "{\"text\":\"PARTIAL-PREFIX: batch 3 of 7 written to heavy/w2/out3.json\"}",
        ),
        ev(
            "mock-run-1",
            3,
            "agent_end",
            "{\"state\":\"incomplete\",\"error\":\"[OUTPUT_LIMIT] finish_length\",\
             \"truncation\":{\"turns_so_far\":6,\"output_len\":512,\
             \"tool_calls_so_far\":1,\"detected_by\":\"finish_length\"}}",
        ),
        ev("mock-run-2", 10, "agent_start", "{}"),
        ev(
            "mock-run-2",
            11,
            "text_chunk",
            "{\"text\":\"batches 4-7 written; all seven artifacts present\"}",
        ),
        ev("mock-run-2", 12, "agent_end", "{\"state\":\"completed\"}"),
    ]
}

#[test]
fn incomplete_continue_note_does_not_replay_completed_work() {
    let cr = cli_root();
    let (_rt, shared) = mock_env(MockState {
        events: truncated_then_completed_events(),
        ..Default::default()
    });
    let goal = init_goal(&cr, "continue after truncation");
    let todo = first_todo_id(&cr.root, &goal);
    let todos_before = open_store(&cr).replay(&goal).unwrap().unwrap().todos.len();
    // max-turns 2: the truncated turn and its continuation. The run exits on
    // this budget (the todo is runnable again after the continuation turn's
    // evidence is judged), which is the outcome under test — not a closure.
    let _ = cli(&["run", "--goal", &goal, "--anonymous", "--max-turns", "2"]);

    let st = shared.lock().unwrap();
    let worker_prompts: Vec<&String> = st
        .prompt_messages
        .iter()
        .zip(st.prompt_calls.iter())
        .filter(|(_, call)| call.0 != "sup-sess")
        .map(|(message, _)| message)
        .collect();
    assert_eq!(
        worker_prompts.len(),
        2,
        "one truncated turn + one continuation: {:?}",
        st.prompt_calls
    );

    // (a) The continuation turn is TOLD to continue, not to start over.
    let continuation = worker_prompts[1];
    assert!(
        continuation.contains("CONTINUE:"),
        "the second turn must carry the continue note: {continuation}"
    );
    assert!(
        continuation.contains(&format!("todo {todo}")),
        "the note names the interrupted todo: {continuation}"
    );
    assert!(
        continuation.contains("do not restart work that already completed"),
        "the note forbids replaying finished work: {continuation}"
    );

    // (b) ... and it can actually obey: the truncated body is in the envelope
    //     (the only "where you left off" a worker has).
    assert!(
        continuation.contains("PARTIAL-PREFIX: batch 3 of 7"),
        "the truncated turn's partial text must reach the next envelope: {continuation}"
    );

    // (c) No replay machinery was engaged: the todo was not re-added or
    //     re-split, and the truncation spent no repair budget.
    drop(st);
    let g = open_store(&cr).replay(&goal).unwrap().unwrap();
    assert_eq!(
        g.todos.len(),
        todos_before,
        "a continuation must not re-add or re-split the todo: {:?}",
        g.todos.iter().map(|t| t.id.clone()).collect::<Vec<_>>()
    );
    let on_todo: Vec<&future_goal::state::RunRecord> =
        g.history.iter().filter(|r| r.todo_id == todo).collect();
    let t = g.todo(&todo).expect("the todo survives the truncation");
    // The durable form of "no repair budget was spent": the ledger's own
    // classification of the truncated turn. (`t.failed_attempts` is NOT a valid
    // witness here — it is an in-memory counter that `writeback` increments but
    // no event carries, so a replayed goal always reads 0; see the durability
    // gap noted in docs/architecture/response-outcomes.zh-CN.md.)
    assert_eq!(
        on_todo[0].failure_kind,
        Some(future_goal::state::FailureKind::InfraRecoverable),
        "a truncated stream is infra, not a science failure: {t:?}"
    );
    // The continuation, not a replay, carried the todo to completion: two
    // records on the same todo, in order, and no third "restart" record.
    assert_eq!(
        on_todo.len(),
        2,
        "exactly two attempts on the todo (truncated + continuation): {:?}",
        on_todo
            .iter()
            .map(|r| (r.terminal_state.clone(), r.error.clone()))
            .collect::<Vec<_>>()
    );
    assert_eq!(on_todo[0].terminal_state, "incomplete");
    assert_eq!(on_todo[1].terminal_state, "completed");
    assert_eq!(
        t.status,
        TodoStatus::Done,
        "the continuation completed the same todo: {t:?}"
    );
}

/// The inverse of assertion (b): when the cut left NO body (`output_len == 0`,
/// the shape measured on a thinking-only truncation loop — 32768 completion
/// tokens all spent on reasoning), there is nothing to carry, so the
/// continuation turn's envelope is indistinguishable from a first attempt.
///
/// This test pins the CURRENT behaviour and the gap it leaves: `turns_so_far`
/// / `output_len` / `tool_calls_so_far` are recorded in the ledger but never
/// reach the worker, so the model cannot know how much of its budget the
/// interrupted attempt consumed and re-derives the same plan until the retry
/// bound stops the run. When the envelope starts carrying the truncation
/// progress (or a token-overlap judge replaces the note), the negative
/// assertion below is what flips.
#[test]
fn incomplete_turn_without_a_body_does_not_surface_its_progress() {
    let cr = cli_root();
    let (_rt, shared) = mock_env(MockState {
        events: vec![
            ev("mock-run-1", 0, "agent_start", "{}"),
            // No text_chunk at all: the whole budget went to reasoning.
            ev(
                "mock-run-1",
                1,
                "agent_end",
                "{\"state\":\"incomplete\",\"error\":\"[OUTPUT_LIMIT] finish_length\",\
                 \"truncation\":{\"turns_so_far\":14,\"output_len\":0,\
                 \"tool_calls_so_far\":0,\"detected_by\":\"finish_length\"}}",
            ),
            ev("mock-run-2", 10, "agent_start", "{}"),
            ev(
                "mock-run-2",
                11,
                "text_chunk",
                "{\"text\":\"second attempt\"}",
            ),
            ev("mock-run-2", 12, "agent_end", "{\"state\":\"completed\"}"),
        ],
        ..Default::default()
    });
    let goal = init_goal(&cr, "bodyless truncation");
    let _ = cli(&["run", "--goal", &goal, "--anonymous", "--max-turns", "2"]);

    let st = shared.lock().unwrap();
    assert_eq!(st.prompt_messages.len(), 2, "{:?}", st.prompt_calls);
    let continuation = &st.prompt_messages[1];
    // The note itself still fires — it is the only signal the worker gets.
    assert!(
        continuation.contains("CONTINUE:"),
        "the continue note must still fire: {continuation}"
    );
    // ... but it carries no progress: no turns, no output length, no tool count.
    for fragment in [
        "turns_so_far",
        "output_len",
        "tool_calls_so_far",
        "14 turns",
    ] {
        assert!(
            !continuation.contains(fragment),
            "the envelope does not yet carry truncation progress ({fragment}), \
             so a bodyless cut is indistinguishable from a first attempt: {continuation}"
        );
    }
    // The previous-turn evidence block is emitted but EMPTY — the block that is
    // supposed to say "where you left off" says nothing.
    assert!(
        continuation.contains("Evidence from the previous turn"),
        "the previous-turn block is still emitted: {continuation}"
    );
    assert!(
        !continuation.contains("PARTIAL"),
        "a bodyless cut carries no partial text: {continuation}"
    );
    // Nothing was produced by the truncated turn either, so no partial-body
    // evidence exists to carry.
    let g = open_store(&cr).replay(&goal).unwrap().unwrap();
    let truncated = g
        .history
        .iter()
        .find(|r| r.terminal_state == "incomplete")
        .expect("the incomplete turn is on the ledger");
    assert_eq!(
        truncated.evidence, "",
        "a bodyless truncation leaves no evidence: {truncated:?}"
    );
    assert_eq!(
        truncated
            .truncation
            .as_ref()
            .and_then(|t| t.get("turns_so_far"))
            .and_then(|v| v.as_u64()),
        Some(14),
        "the lost progress IS recorded — just not surfaced to the worker: {truncated:?}"
    );
}

/// ORDERING: the agent's in-run output-limit continuation takes over BEFORE the
/// goal plane's `incomplete` retry can see anything.
///
/// `response-outcomes.zh-CN.md` has the full table; the short version is that
/// the two retry paths live at different layers and the inner one wins:
///
///   agent run loop  ── finish_length ──▶ continue inside the SAME run
///                    (MAX_OUTPUT_LIMIT_CONTINUATIONS = 2, bounded by
///                     `outputContinuation` + `max_retries > 0`)
///   goal worker     ── sees only the run's TERMINAL state ──▶ nothing to retry
///
/// So a worker whose agent has continuation enabled never observes the cut:
/// it gets one `completed` state, its todo closes on the first turn, and
/// `incomplete_continue_note` (the `CONTINUE:` injection) never fires. This
/// test pins that, because the outer retry exists and would otherwise be the
/// first thing anyone assumed had handled the truncation.
///
/// The complement — the fallback that DOES reach the worker — is
/// `incomplete_continue_note_does_not_replay_completed_work`, which scripts a
/// run whose continuation budget was already exhausted (it ends `incomplete`).
#[test]
fn in_run_continuation_settles_before_the_worker_can_retry() {
    let cr = cli_root();
    let (_rt, shared) = mock_env(MockState {
        events: vec![
            ev("mock-run-1", 0, "agent_start", "{}"),
            ev(
                "mock-run-1",
                1,
                "text_chunk",
                "{\"text\":\"PARTIAL-PREFIX: batch 3 of 7 written\"}",
            ),
            // The output budget ran out and the SAME run kept writing. The
            // goal plane's stream consumer does not special-case this event
            // (`_ => {}` in `agent_client.rs`), which is exactly the point:
            // the cut never becomes an observable state out here.
            ev(
                "mock-run-1",
                2,
                "output_stream_continuing",
                "{\"attempt\":1,\"max_continuations\":2}",
            ),
            ev(
                "mock-run-1",
                3,
                "text_chunk",
                "{\"text\":\" — batches 4-7 written by the continuation\"}",
            ),
            // ONE terminal event: the run reports the continuation's outcome,
            // not the cut's.
            ev("mock-run-1", 4, "agent_end", "{\"state\":\"completed\"}"),
        ],
        ..Default::default()
    });
    let goal = init_goal(&cr, "continuation settles in-run");
    let todo = first_todo_id(&cr.root, &goal);
    cli_ok(&["run", "--goal", &goal, "--anonymous", "--max-turns", "3"]);

    let st = shared.lock().unwrap();
    let worker_prompts: Vec<&String> = st
        .prompt_messages
        .iter()
        .zip(st.prompt_calls.iter())
        .filter(|(_, call)| call.0 != "sup-sess")
        .map(|(message, _)| message)
        .collect();
    // The whole cut-and-continue happened inside one agent run, so the worker
    // spent exactly one turn on the todo.
    assert_eq!(
        worker_prompts.len(),
        1,
        "the continuation is in-run: one prompt, not a retry: {:?}",
        st.prompt_calls
    );
    for (i, prompt) in worker_prompts.iter().enumerate() {
        assert!(
            !prompt.contains("CONTINUE:"),
            "the goal-plane continue note must NOT fire when the agent already \
             continued in-run (prompt {i}): {prompt}"
        );
    }
    drop(st);

    let g = open_store(&cr).replay(&goal).unwrap().unwrap();
    // One ledger record, and it is the continuation's outcome.
    let on_todo: Vec<&future_goal::state::RunRecord> =
        g.history.iter().filter(|r| r.todo_id == todo).collect();
    assert_eq!(
        on_todo.len(),
        1,
        "the cut must not become a second ledger record: {:?}",
        on_todo
            .iter()
            .map(|r| (r.terminal_state.clone(), r.error.clone()))
            .collect::<Vec<_>>()
    );
    assert_eq!(
        on_todo[0].terminal_state, "completed",
        "the worker sees the continuation's outcome: {:?}",
        on_todo[0]
    );
    assert_eq!(
        on_todo[0].truncation, None,
        "no truncation reaches the ledger — the cut was absorbed in-run: {:?}",
        on_todo[0]
    );
    assert_eq!(
        g.todo(&todo).map(|t| t.status),
        Some(TodoStatus::Done),
        "the todo closed on the continuation's turn"
    );
}

#[test]
fn run_anonymous_single_todo_to_terminal() {
    let cr = cli_root();
    let (_rt, shared) = mock_env(MockState {
        events: completed_events("mock-run-1"),
        ..Default::default()
    });
    let goal = init_goal(&cr, "single todo closure");
    cli_ok(&["run", "--goal", &goal, "--anonymous", "--max-turns", "3"]);
    let store = open_store(&cr);
    let g = store.replay(&goal).unwrap().unwrap();
    assert!(g.todos.iter().all(|t| t.status == TodoStatus::Done));
    assert!(g.terminal_closure().is_some(), "validated closure");
    assert_eq!(g.history.len(), 1);
    assert_eq!(shared.lock().unwrap().prompts, 1);
}

#[test]
fn run_with_agent_id_auto_registers_without_inventing_successors() {
    let cr = cli_root();
    let (_rt, shared) = mock_env(MockState {
        events: vec![
            ev("mock-run-1", 0, "tool_start", "{\"tool_name\":\"write\"}"),
            ev("mock-run-1", 1, "text_chunk", "{\"text\":\"one done\"}"),
            ev("mock-run-1", 2, "agent_end", "{\"state\":\"completed\"}"),
            ev("mock-run-2", 0, "tool_start", "{\"tool_name\":\"write\"}"),
            ev("mock-run-2", 1, "text_chunk", "{\"text\":\"two done\"}"),
            ev("mock-run-2", 2, "agent_end", "{\"state\":\"completed\"}"),
        ],
        ..Default::default()
    });
    let goal = init_goal(&cr, "two todos");
    add_todo(&cr, &goal, "second task");
    cli_ok(&[
        "run",
        "--goal",
        &goal,
        "--agent-id",
        "worker-7",
        "--lease-secs",
        "120",
        "--max-turns",
        "5",
    ]);
    let store = open_store(&cr);
    let g = store.replay(&goal).unwrap().unwrap();
    assert!(g.todos.iter().all(|t| t.status == TodoStatus::Done));
    assert!(g.registered_agents.contains(&"worker-7".to_string()));
    assert_eq!(shared.lock().unwrap().prompts, 2);
    // Runnable peers are independent work, not implicit semantic successors.
    let first = g.todos.first().unwrap();
    assert!(first.successor_ids.is_empty(), "unexpected edge: {first:?}");
    assert!(first.no_follow_up, "this slice has no declared follow-up");
}

#[test]
fn run_max_turns_bails() {
    let cr = cli_root();
    let (_rt, _shared) = mock_env(MockState {
        events: completed_events("mock-run-1"),
        ..Default::default()
    });
    let goal = init_goal(&cr, "many todos");
    add_todo(&cr, &goal, "task two");
    add_todo(&cr, &goal, "task three");
    let err = cli_err(&["run", "--goal", &goal, "--anonymous", "--max-turns", "2"]);
    assert!(err.contains("max-turns"), "{err}");
}

#[test]
fn run_workspace_conflict_and_force_workspace() {
    let cr = cli_root();
    let goal = init_goal(&cr, "workspace conflict run");
    // Two agents declaring the same workspace.
    cli_ok(&[
        "agent",
        "onboard",
        "--goal",
        &goal,
        "--agent-id",
        "a1",
        "--workspace",
        "/ws1",
    ]);
    cli_ok(&[
        "agent",
        "onboard",
        "--goal",
        &goal,
        "--agent-id",
        "a2",
        "--workspace",
        "/ws1",
    ]);
    // a1 claims the onboarding todo (live lease in the shared workspace).
    let onboarding = first_todo_id(&cr.root, &goal);
    cli_ok(&[
        "todo",
        "claim",
        "--goal",
        &goal,
        "--todo-id",
        &onboarding,
        "--agent-id",
        "a1",
    ]);
    // A second open todo for a2 to pick up after forcing.
    let second = add_todo(&cr, &goal, "second task");
    let (_rt, shared) = mock_env(MockState {
        events: completed_events("mock-run-1"),
        ..Default::default()
    });
    // a2 running in the same workspace is refused (degrade to serial).
    let err = cli_err(&[
        "run",
        "--goal",
        &goal,
        "--agent-id",
        "a2",
        "--max-turns",
        "2",
    ]);
    assert!(err.contains("workspace conflict"), "{err}");
    // The refused run still minted a session (session creation precedes the
    // guard, which is evaluated per turn) but never prompted it, so the session
    // must be discarded rather than left as an empty conversation titled with
    // the goal objective.
    {
        let st = shared.lock().unwrap();
        assert_eq!(st.sessions_created, 1, "session is minted before the guard");
        assert!(
            st.live_sessions.is_empty(),
            "unused session must be discarded, got {:?}",
            st.live_sessions
        );
    }
    // --force-workspace lets a2 claim the second todo and complete it.
    cli_ok(&[
        "run",
        "--goal",
        &goal,
        "--agent-id",
        "a2",
        "--force-workspace",
        "--max-turns",
        "3",
    ]);
    let store = open_store(&cr);
    let g = store.replay(&goal).unwrap().unwrap();
    assert_eq!(g.todo(&second).unwrap().status, TodoStatus::Done);
}

#[test]
fn run_uses_selected_todo_scope_instead_of_auto_registered_cwd() {
    let cr = cli_root();
    let goal = init_goal(&cr, "disjoint task writes");
    let onboarding = first_todo_id(&cr.root, &goal);
    cli_ok(&[
        "todo",
        "complete",
        "--goal",
        &goal,
        "--todo-id",
        &onboarding,
        "--evidence",
        "setup checked",
        "--no-follow-up",
    ]);
    for (agent, path) in [("a1", "papers/a.md"), ("a2", "papers/b.md")] {
        cli_ok(&[
            "todo",
            "add",
            "--goal",
            &goal,
            "--text",
            agent,
            "--owner",
            agent,
            "--required-write-scope",
            path,
        ]);
        // Exercise the same automatic whole-cwd fallback used by run.
        let mut store = open_store(&cr);
        future_goal::console::ensure_run_identity(&mut store, &goal, Some(agent), false).unwrap();
    }
    let first = common::todo_id_by_text(&cr.root, &goal, "a1");
    let second = common::todo_id_by_text(&cr.root, &goal, "a2");
    cli_ok(&[
        "todo",
        "claim",
        "--goal",
        &goal,
        "--todo-id",
        &first,
        "--agent-id",
        "a1",
    ]);
    let (_rt, shared) = mock_env(MockState {
        events: completed_events("mock-run-1"),
        ..Default::default()
    });
    cli_ok(&[
        "run",
        "--goal",
        &goal,
        "--agent-id",
        "a2",
        "--max-turns",
        "3",
    ]);
    let store = open_store(&cr);
    let replay = store.replay(&goal).unwrap().unwrap();
    assert_eq!(replay.todo(&second).unwrap().status, TodoStatus::Done);
    assert!(replay.todo(&first).unwrap().claimed_by.is_some());
    assert_eq!(shared.lock().unwrap().prompts, 1);
    assert!(!store.events(&goal).unwrap().iter().any(|e| matches!(
        e.event,
        future_goal::store::Event::WorkspaceLockAcquired { forced: true, .. }
    )));
}

#[test]
fn run_failing_turns_exhaust_repair_budget() {
    let cr = cli_root();
    let events = vec![ev(
        "mock-run-1",
        0,
        "agent_end",
        "{\"state\":\"error\",\"error\":\"boom\"}",
    )];
    let (_rt, shared) = mock_env(MockState {
        events,
        ..Default::default()
    });
    let goal = init_goal(&cr, "failing turns");
    let onboarding = first_todo_id(&cr.root, &goal);
    cli_ok(&[
        "todo",
        "complete",
        "--goal",
        &goal,
        "--todo-id",
        &onboarding,
        "--no-follow-up",
        "--evidence",
        "operator closed the auto-created onboarding todo",
    ]);
    // Pre-seed a todo that already consumed one repair attempt (the ledger
    // does not persist failed_attempts from failed runs — see coverage
    // report), so this turn's failure crosses MAX_REPAIR_ATTEMPTS(=1).
    let mut seeded = future_goal::state::Todo::advancement("todo_seeded", "keeps failing");
    seeded.failed_attempts = 1;
    let mut store = open_store(&cr);
    store
        .append(future_goal::store::Event::TodoAdded {
            goal_id: goal.clone(),
            todo: seeded,
            ts: now_epoch(),
        })
        .unwrap();
    store.set_next_action(&goal, "keeps failing").unwrap();
    drop(store);
    // ARCHITECTURE-SIMPLIFICATION: the repair budget no longer stops the run
    // loop — a failed todo stays runnable and the failure count is surfaced
    // as an advisory (the agent decides to supersede / re-split). The run
    // loop keeps delivering until the max-turns bound fires (a budget bail,
    // not a repair-budget stop). Assert the loop behavior: it runs all the
    // way to max-turns with the signal in every delivery reason.
    let err = cli_err(&["run", "--goal", &goal, "--anonymous", "--max-turns", "3"]);
    assert!(
        err.contains("max-turns"),
        "a failed todo stays runnable — the run loop runs to max-turns (budget bail), got: {err}"
    );
    assert_eq!(
        shared.lock().unwrap().prompts,
        3,
        "a failed todo stays runnable — 3 prompts before the max-turns bail"
    );
}

#[test]
fn run_open_gate_breaks_with_ask_user() {
    let cr = cli_root();
    let (_rt, shared) = mock_env(MockState {
        events: completed_events("mock-run-1"),
        ..Default::default()
    });
    let goal = init_goal(&cr, "gate blocks the run");
    cli_ok(&[
        "todo",
        "add",
        "--goal",
        &goal,
        "--text",
        "plan approval gate",
        "--class",
        "user_gate",
        "--gate-question",
        "approve the plan?",
    ]);
    cli_ok(&["run", "--goal", &goal, "--anonymous"]);
    // Escalate-not-freeze: the gate opens (no supervisor registered → the
    // report is dropped) and the gate-independent bootstrap fallback still
    // runs one turn. The gate's OWN dependents stay frozen (there are none
    // here), and `todo complete` behind an open gate is rejected separately.
    assert_eq!(
        shared.lock().unwrap().prompts,
        1,
        "the gate opens + the fallback runs one turn; gate dependents would stay frozen"
    );
}

/// ARCHITECTURE.md "escalate, not freeze": an open gate is a signal to the
/// supervisor, not a full stop — the decision kernel's gate-independent
/// fallback keeps running while the gate waits. The run then hits max-turns
/// (the gate stays open, the budget bails) which is a NON-zero exit; accept
/// either outcome and assert on the prompt count.
#[test]
fn run_open_gate_runs_fallback_todo() {
    let cr = cli_root();
    let (_rt, shared) = mock_env(MockState {
        events: completed_events("mock-run-1"),
        ..Default::default()
    });
    let goal = init_goal(&cr, "gate runs fallback");
    // The onboarding bootstrap todo is the gate-independent fallback here;
    // add a gate that does NOT block it.
    cli_ok(&[
        "todo",
        "add",
        "--goal",
        &goal,
        "--text",
        "plan approval gate",
        "--class",
        "user_gate",
        "--gate-question",
        "approve the plan?",
    ]);
    // max-turns 1 + a still-open gate → the budget bail after one fallback
    // turn (the completion contract requires closure intent, so the mock
    // turn's success cannot close the goal behind an open gate).
    let _ = cli(&[
        "run",
        "--goal",
        &goal,
        "--agent-id",
        "fb",
        "--max-turns",
        "1",
    ]);
    assert_eq!(
        shared.lock().unwrap().prompts,
        1,
        "the gate-independent fallback runs while the gate waits"
    );
}

/// Append a hand-crafted monitor todo directly to the ledger (due_in=0 →
/// already due; the CLI cannot express sub-minute cadences).
fn append_monitor(cr: &common::CliRoot, goal: &str, id: &str, due_in_secs: u64) {
    let mut store = open_store(cr);
    let todo = future_goal::state::Todo::monitor(
        id,
        "watch the artifact",
        std::time::Duration::from_secs(due_in_secs),
    );
    store
        .append(future_goal::store::Event::TodoAdded {
            goal_id: goal.to_string(),
            todo,
            ts: future_goal::state::now_epoch(),
        })
        .unwrap();
    store.set_next_action(goal, "watch the artifact").unwrap();
}

#[test]
fn run_monitor_poll_changed_and_no_change() {
    let cr = cli_root();
    // Advancement outranks a due monitor, so turn 1 is the onboarding todo
    // and turn 2 is the poll; script the EXISTS evidence for the poll turn.
    let (_rt, _shared) = mock_env(MockState {
        events: vec![
            ev(
                "mock-run-1",
                0,
                "text_chunk",
                "{\"text\":\"onboarding done\"}",
            ),
            ev("mock-run-1", 1, "agent_end", "{\"state\":\"completed\"}"),
            ev(
                "mock-run-2",
                0,
                "text_chunk",
                "{\"text\":\"the file EXISTS now\"}",
            ),
            ev("mock-run-2", 1, "agent_end", "{\"state\":\"completed\"}"),
        ],
        ..Default::default()
    });
    let goal = init_goal(&cr, "monitor poll");
    append_monitor(&cr, &goal, "mon_due", 0);
    cli_ok(&["run", "--goal", &goal, "--anonymous", "--max-turns", "4"]);
    let store = open_store(&cr);
    let g = store.replay(&goal).unwrap().unwrap();
    let m = g.todos.iter().find(|t| t.id == "mon_due").unwrap();
    assert_eq!(
        m.status,
        TodoStatus::Done,
        "changed poll closes the monitor: {m:?}"
    );
}

#[test]
fn run_monitor_poll_no_change_never_spends() {
    let cr = cli_root();
    let (_rt, _shared) = mock_env(MockState {
        events: vec![
            ev(
                "mock-run-1",
                0,
                "text_chunk",
                "{\"text\":\"nothing new yet\"}",
            ),
            ev("mock-run-1", 1, "agent_end", "{\"state\":\"completed\"}"),
            ev(
                "mock-run-2",
                0,
                "text_chunk",
                "{\"text\":\"onboarding done\"}",
            ),
            ev("mock-run-2", 1, "agent_end", "{\"state\":\"completed\"}"),
        ],
        ..Default::default()
    });
    let goal = init_goal(&cr, "monitor no-change");
    append_monitor(&cr, &goal, "mon_due", 0);
    cli_ok(&["run", "--goal", &goal, "--anonymous", "--max-turns", "4"]);
    let store = open_store(&cr);
    let g = store.replay(&goal).unwrap().unwrap();
    let m = g.todos.iter().find(|t| t.id == "mon_due").unwrap();
    assert_eq!(m.consecutive_no_change, 1, "no-change counter: {m:?}");
    assert_ne!(m.status, TodoStatus::Done);
    // A no-change poll never spends quota (other turns may).
    let spent_on_monitor = store
        .events(&goal)
        .unwrap()
        .iter()
        .filter(|se| matches!(&se.event, future_goal::store::Event::QuotaSpent { todo_id, .. } if todo_id == "mon_due"))
        .count();
    assert_eq!(spent_on_monitor, 0);
}

#[test]
fn run_monitor_not_due_waits() {
    let cr = cli_root();
    let (_rt, shared) = mock_env(MockState {
        events: completed_events("mock-run-1"),
        ..Default::default()
    });
    let goal = init_goal(&cr, "monitor waiting");
    // Complete the only advancement todo so the (not-due) monitor is all that
    // remains → WaitMonitor → graceful "waiting…" stop.
    let onboarding = first_todo_id(&cr.root, &goal);
    cli_ok(&[
        "todo",
        "complete",
        "--goal",
        &goal,
        "--todo-id",
        &onboarding,
        "--no-follow-up",
        "--evidence",
        "operator closed the auto-created onboarding todo",
    ]);
    append_monitor(&cr, &goal, "mon_future", 3600);
    cli_ok(&["run", "--goal", &goal, "--anonymous", "--max-turns", "3"]);
    let st = shared.lock().unwrap();
    assert_eq!(st.prompts, 0, "no turn while waiting");
    // The run minted a session, then decided to wait — nothing was ever
    // prompted, so the empty session must not be left behind.
    assert_eq!(st.sessions_created, 1);
    assert!(
        st.live_sessions.is_empty(),
        "unused session must be discarded, got {:?}",
        st.live_sessions
    );
}

#[cfg(unix)]
#[test]
fn run_validator_pass_completes_todo() {
    let cr = cli_root();
    let (_rt, _shared) = mock_env(MockState {
        events: completed_events("mock-run-1"),
        ..Default::default()
    });
    let goal = init_goal(&cr, "validator pass");
    let onboarding = first_todo_id(&cr.root, &goal);
    cli_ok(&[
        "todo",
        "complete",
        "--goal",
        &goal,
        "--todo-id",
        &onboarding,
        "--no-follow-up",
        "--evidence",
        "operator closed the auto-created onboarding todo",
    ]);
    cli_ok(&[
        "todo",
        "add",
        "--goal",
        &goal,
        "--text",
        "validated real task",
        "--verify",
        "exit 0",
    ]);
    let vt = todo_id_by_text(&cr.root, &goal, "validated real task");
    cli_ok(&["run", "--goal", &goal, "--anonymous", "--max-turns", "3"]);
    let store = open_store(&cr);
    let g = store.replay(&goal).unwrap().unwrap();
    let todo = g.todos.iter().find(|t| t.id == vt).unwrap();
    assert_eq!(todo.status, TodoStatus::Done, "validator passed → done");
    let record = g.history.last().unwrap();
    assert!(record.validation.as_ref().unwrap().ok);
}

#[test]
fn run_validator_budget_exhaustion_stops_run() {
    let cr = cli_root();
    let (_rt, shared) = mock_env(MockState {
        events: completed_events("mock-run-1"),
        ..Default::default()
    });
    let goal = init_goal(&cr, "validator budget");
    let onboarding = first_todo_id(&cr.root, &goal);
    cli_ok(&[
        "todo",
        "complete",
        "--goal",
        &goal,
        "--todo-id",
        &onboarding,
        "--no-follow-up",
        "--evidence",
        "operator closed the auto-created onboarding todo",
    ]);
    cli_ok(&[
        "todo",
        "add",
        "--goal",
        &goal,
        "--text",
        "never validates",
        "--verify",
        "exit 1",
        "--max-validation-attempts",
        "1",
    ]);
    // Validation budget (1) hit after the first failed validation → stop, Ok.
    // Assert the loop stopped after exactly one turn (the budget branch
    // fired); the ledger keeps the todo open.
    cli_ok(&["run", "--goal", &goal, "--anonymous", "--max-turns", "6"]);
    assert_eq!(
        shared.lock().unwrap().prompts,
        1,
        "validation budget break after one turn"
    );
    let store = open_store(&cr);
    let g = store.replay(&goal).unwrap().unwrap();
    let todo = g
        .todos
        .iter()
        .find(|t| t.text.contains("never validates"))
        .unwrap();
    assert_eq!(todo.status, TodoStatus::Open, "todo stays open: {todo:?}");
}

#[test]
fn run_with_model_and_thinking_flags() {
    let cr = cli_root();
    let (_rt, shared) = mock_env(MockState {
        events: completed_events("mock-run-1"),
        ..Default::default()
    });
    let goal = init_goal(&cr, "model flags");
    cli_ok(&[
        "run",
        "--goal",
        &goal,
        "--anonymous",
        "--model",
        "future/k3",
        "--thinking-level",
        "low",
    ]);
    let recorded = shared.lock().unwrap().recorded.clone();
    assert!(recorded.contains(&"set_model".to_string()));
    assert!(recorded.contains(&"set_thinking_level".to_string()));
}

// ── bidirectional messaging: up-channel turn-boundary reports ────────────────

/// A registered supervisor receives an `enqueue_coalescing` report when a todo
/// completes (up-channel ②, exercised through the real `run` loop). Batches
/// delivered while the orchestrator is busy fold into one reconciliation run,
/// so a report that never gets a run of its own is expected here.
#[test]
fn run_notifies_supervisor_on_completion() {
    let cr = cli_root();
    let (_rt, shared) = mock_env(MockState {
        events: completed_events("mock-run-1"),
        ..Default::default()
    });
    let goal = init_goal(&cr, "notify on complete");
    cli_ok(&[
        "supervisor",
        "register",
        "--goal",
        &goal,
        "--session-id",
        "sup-sess",
    ]);
    cli_ok(&["run", "--goal", &goal, "--anonymous", "--max-turns", "3"]);
    let calls = shared.lock().unwrap().prompt_calls.clone();
    assert_eq!(shared.lock().unwrap().new_session_parents, vec!["sup-sess"]);
    // Exactly one report to the supervisor (the worker's own turn prompt goes
    // to the worker session, not sup-sess).
    let reports: Vec<_> = calls.iter().filter(|(sid, _)| sid == "sup-sess").collect();
    assert_eq!(reports.len(), 1, "one completion report: {calls:?}");
    assert_eq!(reports[0].1, "enqueue_coalescing");
}

#[test]
fn run_explicit_parent_overrides_supervisor_lineage() {
    let cr = cli_root();
    let (_rt, shared) = mock_env(MockState {
        events: completed_events("mock-run-1"),
        ..Default::default()
    });
    let goal = init_goal(&cr, "nested worker");
    cli_ok(&[
        "supervisor",
        "register",
        "--goal",
        &goal,
        "--session-id",
        "sup-sess",
    ]);
    cli_ok(&[
        "run",
        "--goal",
        &goal,
        "--anonymous",
        "--max-turns",
        "3",
        "--parent-session",
        "parent-worker",
    ]);
    assert_eq!(
        shared.lock().unwrap().new_session_parents,
        vec!["parent-worker"]
    );
}

/// A registered supervisor receives a report when a todo fails on a hard
/// error (up-channel ③; infra-recoverable 429/truncation is NOT reported).
#[test]
fn run_notifies_supervisor_on_hard_failure() {
    let cr = cli_root();
    let events = vec![ev(
        "mock-run-1",
        0,
        "agent_end",
        "{\"state\":\"error\",\"error\":\"boom\"}",
    )];
    let (_rt, shared) = mock_env(MockState {
        events,
        ..Default::default()
    });
    let goal = init_goal(&cr, "notify on failure");
    cli_ok(&[
        "supervisor",
        "register",
        "--goal",
        &goal,
        "--session-id",
        "sup-sess",
    ]);
    // One failed turn, then max-turns bail (a failed todo stays runnable).
    let _ = cli_err(&["run", "--goal", &goal, "--anonymous", "--max-turns", "1"]);
    let calls = shared.lock().unwrap().prompt_calls.clone();
    let reports: Vec<_> = calls.iter().filter(|(sid, _)| sid == "sup-sess").collect();
    assert_eq!(reports.len(), 1, "one failure report: {calls:?}");
    assert_eq!(reports[0].1, "enqueue_coalescing");
}

/// A registered supervisor receives a report when a worker exhausts its
/// incomplete-retry budget: the stream closes before `agent_end` on every
/// turn (terminal_state=incomplete), and after N consecutive turns the run
/// `break`s — previously silently, leaving the supervisor blind to the stop.
///
/// The mock assigns run ids `mock-run-1`, `mock-run-2`, … on successive
/// prompts, and its replay loop sends every scripted event whose `run_id`
/// matches the current run; events that never match (and no `agent_end`)
/// produce the incomplete state on every turn.
#[test]
fn run_notifies_supervisor_on_incomplete_budget_exhausted() {
    let cr = cli_root();
    let (_rt, shared) = mock_env(MockState {
        // One tool event bound to a run id the mock never serves: every turn
        // sees a stream with no matching events and no agent_end → incomplete.
        events: vec![ev(
            "mock-run-x",
            0,
            "tool_start",
            "{\"tool_name\":\"write\"}",
        )],
        ..Default::default()
    });
    let goal = init_goal(&cr, "notify on incomplete budget");
    cli_ok(&[
        "supervisor",
        "register",
        "--goal",
        &goal,
        "--session-id",
        "sup-sess",
    ]);
    // One todo, --max-turns 5, budget 2: turn1 + turn2 incomplete → break at
    // the exhausted budget (turn 2), not at max-turns.
    cli_ok(&[
        "run",
        "--goal",
        &goal,
        "--anonymous",
        "--max-turns",
        "5",
        "--max-incomplete-retries",
        "2",
    ]);
    let st = shared.lock().unwrap();
    let reports: Vec<_> = st
        .prompt_calls
        .iter()
        .zip(st.prompt_messages.iter())
        .filter(|((sid, _), _)| sid == "sup-sess")
        .collect();
    assert_eq!(
        reports.len(),
        1,
        "one incomplete-budget report: {:?}",
        st.prompt_calls
    );
    assert_eq!(reports[0].0 .1, "enqueue_coalescing");
    assert!(
        reports[0]
            .1
            .contains("stopped before completion (incomplete_budget)"),
        "incomplete-budget stop reported: {}",
        reports[0].1
    );
}

/// A transport failure (h2 reset / mid-stream non-gap error) is an infra event,
/// not a verdict on the work: the turn is RECORDED and the worker keeps going.
/// It used to end the whole run on the spot, leaving no run record behind —
/// measured in one goal: 31 launches vanished from the ledger this way, and the
/// supervisor had to relaunch a worker that had died mid-task. A *streak* of
/// them still stops the run (an agent that is gone will not come back by
/// retrying), and that stop is reported, because a worker that stops mid-task
/// must not leave the supervisor polling.
#[test]
fn run_bounds_consecutive_transport_failures_and_reports_the_stop() {
    let cr = cli_root();
    let (_rt, shared) = mock_env(MockState {
        // A non-DataLoss mid-stream error: no reconnect, the turn fails. One
        // scripted attach per failed turn (the bound is three).
        stream_attach_plan: vec![
            AttachPlan::TransportAfter(0),
            AttachPlan::TransportAfter(0),
            AttachPlan::TransportAfter(0),
            AttachPlan::TransportAfter(0),
        ],
        ..Default::default()
    });
    let goal = init_goal(&cr, "bound transport");
    cli_ok(&[
        "supervisor",
        "register",
        "--goal",
        &goal,
        "--session-id",
        "sup-sess",
    ]);
    cli_ok(&["run", "--goal", &goal, "--anonymous", "--max-turns", "8"]);
    let st = shared.lock().unwrap();
    let worker_turns = st
        .prompt_calls
        .iter()
        .filter(|call| call.0.as_str() != "sup-sess")
        .count();
    assert_eq!(
        worker_turns, 3,
        "one recorded turn per transport failure, then the bound stops the run: {:?}",
        st.prompt_calls
    );
    let reports: Vec<_> = st
        .prompt_calls
        .iter()
        .zip(st.prompt_messages.iter())
        .filter(|((sid, _), _)| sid == "sup-sess")
        .collect();
    assert_eq!(
        reports.len(),
        1,
        "one transport-budget report: {:?}",
        st.prompt_calls
    );
    assert_eq!(reports[0].0 .1, "enqueue_coalescing");
    assert!(
        reports[0]
            .1
            .contains("stopped before completion (transport_budget)"),
        "transport-budget stop reported: {}",
        reports[0].1
    );
}

/// A registered supervisor receives a report when a user gate opens
/// (up-channel ①, AskUser turn mode).
#[test]
fn run_notifies_supervisor_on_ask_user() {
    let cr = cli_root();
    let (_rt, shared) = mock_env(MockState {
        events: completed_events("mock-run-1"),
        ..Default::default()
    });
    let goal = init_goal(&cr, "notify on gate");
    cli_ok(&[
        "supervisor",
        "register",
        "--goal",
        &goal,
        "--session-id",
        "sup-sess",
    ]);
    cli_ok(&[
        "todo",
        "add",
        "--goal",
        &goal,
        "--text",
        "plan gate",
        "--class",
        "user_gate",
        "--gate-question",
        "approve?",
    ]);
    cli_ok(&["run", "--goal", &goal, "--anonymous"]);
    let calls = shared.lock().unwrap().prompt_calls.clone();
    let reports: Vec<_> = calls.iter().filter(|(sid, _)| sid == "sup-sess").collect();
    // Escalate-not-freeze: the gate report still fires each turn the gate is
    // seen (dedup per process; the mock's incomplete turns repeat the turn,
    // so dedup is keyed per (gate ids) within one run client), and the
    // fallback turn runs alongside. The exact count depends on how the run
    // ends; assert the gate report fires at least once and carries the
    // gate question text.
    assert!(
        !reports.is_empty(),
        "the supervisor learns of the gate: {calls:?}"
    );
    assert_eq!(reports[0].1, "enqueue_coalescing");
}

/// Without a registered supervisor, a completed turn produces NO report
/// (the durable user gate remains the authoritative channel).
#[test]
fn run_without_supervisor_sends_no_report() {
    let cr = cli_root();
    let (_rt, shared) = mock_env(MockState {
        events: completed_events("mock-run-1"),
        ..Default::default()
    });
    let goal = init_goal(&cr, "no supervisor");
    cli_ok(&["run", "--goal", &goal, "--anonymous", "--max-turns", "3"]);
    let calls = shared.lock().unwrap().prompt_calls.clone();
    // Only the worker's own turn prompt; nothing to sup-sess (no registration).
    assert_eq!(calls.len(), 1, "one worker prompt, zero reports: {calls:?}");
    assert_ne!(calls[0].0, "sup-sess");
}

// ── bidirectional messaging: down-channel steering into the turn envelope ───

/// A `WorkerSteered` instruction folded into `pending_steer` is injected into
/// the next turn's envelope (down-channel, non-interrupt path — the instruction
/// is delivered without re-injecting across turns).
#[test]
fn run_injects_pending_steer_into_turn_envelope() {
    let cr = cli_root();
    let (_rt, shared) = mock_env(MockState {
        events: completed_events("mock-run-1"),
        ..Default::default()
    });
    let goal = init_goal(&cr, "steer envelope");
    // Pre-seed a steering instruction targeting this worker (broadcast).
    let mut store = open_store(&cr);
    store
        .append(future_goal::store::Event::WorkerSteered {
            goal_id: goal.clone(),
            agent_id: None,
            instruction: "switch to plan B".to_string(),
            ts: now_epoch(),
        })
        .unwrap();
    drop(store);
    cli_ok(&["run", "--goal", &goal, "--anonymous", "--max-turns", "3"]);
    let messages = shared.lock().unwrap().prompt_messages.clone();
    assert_eq!(messages.len(), 1, "one turn: {messages:?}");
    assert!(
        messages[0].contains("switch to plan B"),
        "steer instruction injected into the turn envelope: {}",
        messages[0]
    );
    assert!(
        messages[0].contains("SUPERVISOR STEERING"),
        "steer header present: {}",
        messages[0]
    );
}

/// A steer targeted at a DIFFERENT worker is NOT injected into this worker's
/// envelope (agent_id scoping).
#[test]
fn run_ignores_steer_targeting_another_worker() {
    let cr = cli_root();
    let (_rt, shared) = mock_env(MockState {
        events: completed_events("mock-run-1"),
        ..Default::default()
    });
    let goal = init_goal(&cr, "steer scoping");
    let mut store = open_store(&cr);
    store
        .append(future_goal::store::Event::WorkerSteered {
            goal_id: goal.clone(),
            agent_id: Some("other-worker".to_string()),
            instruction: "not for me".to_string(),
            ts: now_epoch(),
        })
        .unwrap();
    drop(store);
    cli_ok(&[
        "run",
        "--goal",
        &goal,
        "--agent-id",
        "this-worker",
        "--max-turns",
        "3",
    ]);
    let messages = shared.lock().unwrap().prompt_messages.clone();
    assert!(
        !messages[0].contains("not for me"),
        "foreign steer must not leak into this worker: {}",
        messages[0]
    );
}

// ── models ─────────────────────────────────────────────────────────────────

#[test]
fn models_text_and_json() {
    let cr = cli_root();
    let (_rt, _shared) = mock_env(MockState::default());
    cli_ok(&["models"]);
    cli_ok(&["models", "--format", "json"]);
    cli_ok(&["models", "--json"]);
    let _ = cr;
}

#[test]
fn models_sparse_payload_defaults() {
    let _cr = cli_root();
    let st = MockState {
        models_payload: Some("{\"models\":[{\"id\":\"bare\"}]}".to_string()),
        ..Default::default()
    };
    let (_rt, _shared) = mock_env(st);
    cli_ok(&["models"]);
}

#[test]
fn models_unreachable_agent_errors() {
    let _cr = cli_root();
    std::env::set_var("FUTURE_GOAL_AGENT_ADDR", "127.0.0.1:1");
    let err = cli_err(&["models"]);
    assert!(err.contains("Failed to connect"), "{err}");
}

// ── doctor ─────────────────────────────────────────────────────────────────

#[test]
fn doctor_fresh_root_and_goal_filter() {
    let cr = cli_root();
    cli_ok(&["doctor"]);
    let goal = init_goal(&cr, "doctor goal");
    cli_ok(&["doctor", "--goal", &goal]);
    let err = cli_err(&["doctor", "--goal", "goal_nope"]);
    assert!(err.contains("not found"), "{err}");
}

#[test]
fn doctor_agent_probe() {
    let _cr = cli_root();
    let (_rt, _shared) = mock_env(MockState::default());
    let addr = std::env::var("FUTURE_GOAL_AGENT_ADDR").unwrap();
    cli_ok(&["doctor", "--agent-addr", &addr]);
    let err = cli_err(&["doctor", "--agent-addr", "127.0.0.1:1"]);
    assert!(err.contains("gRPC"), "{err}");
}

// ── cli (error-path sanity through the mock-free surface) ──────────────────

#[test]
fn cli_result_helper_passthrough() {
    let cr = cli_root();
    // `cli` returns Ok for a trivial command; keep the helper itself honest.
    assert!(cli(&["version"]).is_ok());
    let _ = cr;
}
