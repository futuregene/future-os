//! Coverage drive for `agent_client.rs` (thin gRPC transport) and
//! `executor.rs` (turn execution + writeback) against an in-process mock
//! FutureAgent server.

mod common;

use common::mock_agent::{completed_events, ev, spawn_mock, AttachPlan, MockState};
use future_loop::agent_client::AgentClient;
use future_loop::executor::{execute_turn, turn_succeeded, writeback};
use future_loop::state::{Goal, RunRecord, Todo, TodoStatus};

fn rt() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap()
}

fn sample_goal_with_todo() -> (Goal, Todo) {
    let mut goal = Goal::new("goal_x", "cover the executor", "/tmp");
    let todo = Todo::advancement("todo_x", "write the artifact");
    goal.todos.push(todo.clone());
    (goal, todo)
}

fn sample_record(state: &str) -> RunRecord {
    RunRecord {
        turn: 1,
        todo_id: "todo_x".into(),
        run_id: "run-x".into(),
        terminal_state: state.into(),
        error: None,
        tokens_in_delta: 1,
        tokens_out_delta: 2,
        cost_delta: 0.01,
        tools: vec!["shell".into()],
        evidence: "artifact written".into(),
        recorded_at: 1_700_000_000,
        spend_source: None,
        validation: None,
        failure_kind: None,
    }
}

// ── connect / call transport ───────────────────────────────────────────────

#[test]
fn connect_failure_is_wrapped() {
    rt().block_on(async {
        // Port 1 is never listenable.
        let err = match AgentClient::connect("127.0.0.1:1").await {
            Ok(_) => panic!("connect to port 1 should fail"),
            Err(e) => e,
        };
        assert!(format!("{err:#}").contains("Failed to connect"));
        // The http:// prefix is stripped before dialing.
        let err = match AgentClient::connect("http://127.0.0.1:1").await {
            Ok(_) => panic!("connect to port 1 should fail"),
            Err(e) => e,
        };
        assert!(format!("{err:#}").contains("Failed to connect"));
    });
}

#[test]
fn session_and_command_happy_paths() {
    rt().block_on(async {
        let (addr, shared) = spawn_mock(MockState::default()).await;
        let mut client = AgentClient::connect(&addr).await.unwrap();

        let session = client.new_session("/tmp").await.unwrap();
        assert_eq!(session, "mock-session-1");

        client
            .append_system_prompt(&session, "boundary")
            .await
            .unwrap();
        client.steer(&session, "note").await.unwrap();
        client.set_model(&session, "future/k3").await.unwrap();
        client.set_thinking_level(&session, "low").await.unwrap();

        let run_id = client.prompt(&session, "do work", "req-1").await.unwrap();
        assert_eq!(run_id, "mock-run-1");

        let totals = client.session_totals(&session).await.unwrap();
        assert_eq!(totals.tokens_in, 0);

        // session_alive: a live session probes true; a missing one probes false.
        assert!(
            client.session_alive(&session).await,
            "live session must probe alive"
        );
        assert!(
            !client.session_alive("missing-session").await,
            "a missing session must probe dead"
        );

        let models = client.list_models().await.unwrap();
        assert!(models["models"].is_array());

        client.delete_session(&session).await.unwrap();

        let recorded = shared.lock().unwrap().recorded.clone();
        for expected in [
            "new_session",
            "append_system_prompt",
            "steer",
            "set_model",
            "set_thinking_level",
            "prompt",
            "get_state",
            "list_models",
            "delete_session",
        ] {
            assert!(
                recorded.contains(&expected.to_string()),
                "missing {expected}"
            );
        }
    });
}

#[test]
fn command_error_paths() {
    rt().block_on(async {
        // success=false with an error message.
        let (addr, _) = spawn_mock(MockState::fail("new_session")).await;
        let mut client = AgentClient::connect(&addr).await.unwrap();
        let err = client.new_session("/tmp").await.unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("mock_error"), "{msg}");
        assert!(msg.contains("mock failure for new_session"), "{msg}");

        // Transport-level gRPC error (tonic status).
        let st = MockState {
            grpc_error: true,
            ..Default::default()
        };
        let (addr, _) = spawn_mock(st).await;
        let mut client = AgentClient::connect(&addr).await.unwrap();
        let err = client.list_models().await.unwrap_err();
        assert!(format!("{err:#}").contains("gRPC 'list_models' failed"));

        // invalid JSON payload.
        let st = MockState {
            invalid_json: ["list_models".to_string()].into_iter().collect(),
            ..Default::default()
        };
        let (addr, _) = spawn_mock(st).await;
        let mut client = AgentClient::connect(&addr).await.unwrap();
        let err = client.list_models().await.unwrap_err();
        assert!(format!("{err:#}").contains("invalid JSON"));

        // new_session payload without sessionId.
        let st = MockState {
            raw: [("new_session".to_string(), "{}".to_string())]
                .into_iter()
                .collect(),
            ..Default::default()
        };
        let (addr, _) = spawn_mock(st).await;
        let mut client = AgentClient::connect(&addr).await.unwrap();
        let err = client.new_session("/tmp").await.unwrap_err();
        assert!(format!("{err:#}").contains("missing sessionId"));

        // prompt payload without run_id.
        let st = MockState {
            raw: [("prompt".to_string(), "{\"nope\":1}".to_string())]
                .into_iter()
                .collect(),
            ..Default::default()
        };
        let (addr, _) = spawn_mock(st).await;
        let mut client = AgentClient::connect(&addr).await.unwrap();
        let err = client.prompt("s", "m", "r").await.unwrap_err();
        assert!(format!("{err:#}").contains("missing run_id"));
    });
}

#[test]
fn abort_is_transport_covered() {
    rt().block_on(async {
        let (addr, _) = spawn_mock(MockState::default()).await;
        let mut client = AgentClient::connect(&addr).await.unwrap();
        client.abort("sess").await.unwrap();
    });
}

// ── run_turn event stream handling ─────────────────────────────────────────

#[test]
fn run_turn_completed_with_live_log() {
    rt().block_on(async {
        let (addr, _) = spawn_mock(MockState {
            events: completed_events("mock-run-1"),
            ..Default::default()
        })
        .await;
        let mut client = AgentClient::connect(&addr).await.unwrap();
        let dir = tempfile::tempdir().unwrap();
        let live = dir.path().join("live.jsonl");
        let summary = client
            .run_turn("sess", "mock-run-1", Some(&live), None)
            .await
            .unwrap();
        assert_eq!(summary.terminal_state, "completed");
        assert_eq!(summary.tools, vec!["shell".to_string()]);
        assert!(summary.text.contains("artifact written"));
        assert!(summary.usage.is_some());
        assert_eq!(summary.duration_ms, Some(7));
        let log = std::fs::read_to_string(&live).unwrap();
        assert!(log.contains("tool_start"), "{log}");
        assert!(log.contains("\"tool\":\"shell\""), "{log}");
    });
}

#[test]
fn run_turn_skips_foreign_and_malformed_events() {
    rt().block_on(async {
        let events = vec![
            // Stale tail from another run — ignored.
            ev("other-run", 0, "text_chunk", "{\"text\":\"foreign\"}"),
            ev(
                "other-run",
                1,
                "tool_start",
                "{\"tool_name\":\"foreign_tool\"}",
            ),
            // Empty data → parse_data None → skipped.
            ev("mine", 2, "ping", ""),
            // Invalid JSON data → skipped.
            ev("mine", 3, "ping", "{nope"),
            // tool_start without tool_name → no push.
            ev("mine", 4, "tool_start", "{}"),
            // text_chunk without text → no append.
            ev("mine", 5, "text_chunk", "{}"),
            // agent_end without state → default "completed", with error.
            ev("mine", 6, "agent_end", "{\"error\":\"soft fail\"}"),
        ];
        let (addr, _) = spawn_mock(MockState {
            events,
            ..Default::default()
        })
        .await;
        let mut client = AgentClient::connect(&addr).await.unwrap();
        let summary = client.run_turn("sess", "mine", None, None).await.unwrap();
        assert_eq!(summary.terminal_state, "completed");
        assert_eq!(summary.error.as_deref(), Some("soft fail"));
        assert!(summary.tools.is_empty());
        assert!(summary.text.is_empty());
    });
}

#[test]
fn run_turn_long_text_truncates_at_char_boundary() {
    rt().block_on(async {
        // 8100 chars with a multibyte char straddling the 8000 boundary.
        let mut text = "a".repeat(7998);
        text.push('界'); // 3 bytes at offset 7998..8001
        text.push_str(&"b".repeat(200));
        let data = format!("{{\"text\":\"{text}\"}}");
        let events = vec![
            ev("mine", 0, "text_chunk", &data),
            ev("mine", 1, "agent_end", "{\"state\":\"completed\"}"),
        ];
        let (addr, _) = spawn_mock(MockState {
            events,
            ..Default::default()
        })
        .await;
        let mut client = AgentClient::connect(&addr).await.unwrap();
        let summary = client.run_turn("sess", "mine", None, None).await.unwrap();
        assert!(summary.text.len() <= 8003, "{}", summary.text.len());
        assert!(summary.text.starts_with("aaaa"));
    });
}

#[test]
fn run_turn_error_event_and_stream_failure() {
    rt().block_on(async {
        // Explicit error event ends the turn as error.
        let events = vec![
            ev("mine", 0, "text_chunk", "{\"text\":\"partial\"}"),
            ev("mine", 1, "error", "{\"error\":\"boom\"}"),
            ev("mine", 2, "agent_end", "{\"state\":\"completed\"}"),
        ];
        let (addr, _) = spawn_mock(MockState {
            events,
            ..Default::default()
        })
        .await;
        let mut client = AgentClient::connect(&addr).await.unwrap();
        let summary = client.run_turn("sess", "mine", None, None).await.unwrap();
        assert_eq!(summary.terminal_state, "error");
        assert_eq!(summary.error.as_deref(), Some("boom"));

        // Error event without payload → "unknown error".
        let events = vec![ev("mine", 0, "error", "{}")];
        let (addr, _) = spawn_mock(MockState {
            events,
            ..Default::default()
        })
        .await;
        let mut client = AgentClient::connect(&addr).await.unwrap();
        let summary = client.run_turn("sess", "mine", None, None).await.unwrap();
        assert_eq!(summary.error.as_deref(), Some("unknown error"));

        // Mid-stream transport failure.
        let mut st = MockState {
            events: vec![ev("mine", 0, "text_chunk", "{\"text\":\"x\"}")],
            ..Default::default()
        };
        st.stream_fail_after = Some(1);
        let (addr, _) = spawn_mock(st).await;
        let mut client = AgentClient::connect(&addr).await.unwrap();
        let err = client
            .run_turn("sess", "mine", None, None)
            .await
            .unwrap_err();
        assert!(format!("{err:#}").contains("stream error"));

        // Attach failure.
        let st = MockState {
            stream_error: true,
            ..Default::default()
        };
        let (addr, _) = spawn_mock(st).await;
        let mut client = AgentClient::connect(&addr).await.unwrap();
        let err = client
            .run_turn("sess", "mine", None, None)
            .await
            .unwrap_err();
        assert!(format!("{err:#}").contains("Failed to attach"));

        // Stream closes without agent_end → incomplete.
        let events = vec![ev("mine", 0, "text_chunk", "{\"text\":\"x\"}")];
        let (addr, _) = spawn_mock(MockState {
            events,
            ..Default::default()
        })
        .await;
        let mut client = AgentClient::connect(&addr).await.unwrap();
        let summary = client.run_turn("sess", "mine", None, None).await.unwrap();
        assert_eq!(summary.terminal_state, "incomplete");
    });
}

#[test]
fn run_turn_live_log_without_tool_name() {
    rt().block_on(async {
        // tool_start WITHOUT tool_name → the live-log line has no tool field.
        let events = vec![
            ev("mine", 0, "tool_start", "{}"),
            ev("mine", 1, "agent_end", "{\"state\":\"completed\"}"),
        ];
        let (addr, _) = spawn_mock(MockState {
            events,
            ..Default::default()
        })
        .await;
        let mut client = AgentClient::connect(&addr).await.unwrap();
        let dir = tempfile::tempdir().unwrap();
        let live = dir.path().join("live.jsonl");
        let summary = client
            .run_turn("sess", "mine", Some(&live), None)
            .await
            .unwrap();
        assert!(summary.tools.is_empty());
        let log = std::fs::read_to_string(&live).unwrap();
        assert!(log.contains("tool_start"), "{log}");
        assert!(!log.contains("\"tool\":"), "{log}");
    });
}

// ── O5: session event-stream gap recovery (backoff + cursor resume) ───────

#[test]
fn run_turn_recovers_from_single_stream_gap() {
    rt().block_on(async {
        // Attach 1 serves the first two events then drops the stream with the
        // agent's DataLoss "event stream gap" status; attach 2 (resumed from
        // the observed cursor) serves the rest to agent_end.
        let (addr, shared) = spawn_mock(MockState {
            events: completed_events("mock-run-1"),
            stream_attach_plan: vec![AttachPlan::GapAfter(2), AttachPlan::Complete],
            ..Default::default()
        })
        .await;
        let mut client = AgentClient::connect(&addr).await.unwrap();
        let summary = client
            .run_turn("sess", "mock-run-1", None, None)
            .await
            .unwrap();
        assert_eq!(summary.terminal_state, "completed");
        assert_eq!(summary.tools, vec!["shell".to_string()]);
        assert_eq!(summary.text, "artifact written", "no duplicated text");
        assert!(summary.usage.is_some());
        assert_eq!(summary.duration_ms, Some(7));
        let shared = shared.lock().unwrap();
        assert_eq!(
            shared.attach_after_idx,
            vec![-1, 1],
            "reconnect resumes from the last observed idx"
        );
    });
}

#[test]
fn run_turn_second_consecutive_gap_fails_with_original_error() {
    rt().block_on(async {
        // Both attaches drop mid-stream: the retry also fails, so the turn
        // must terminate carrying the ORIGINAL error.
        let (addr, shared) = spawn_mock(MockState {
            events: completed_events("mock-run-1"),
            stream_attach_plan: vec![AttachPlan::GapAfter(1), AttachPlan::GapAfter(1)],
            ..Default::default()
        })
        .await;
        let mut client = AgentClient::connect(&addr).await.unwrap();
        let err = client
            .run_turn("sess", "mock-run-1", None, None)
            .await
            .unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("mock gap #1"), "original error carried: {msg}");
        assert!(msg.contains("retry also failed"), "{msg}");
        let shared = shared.lock().unwrap();
        assert_eq!(shared.attach_after_idx, vec![-1, 0]);
    });
}

#[test]
fn run_turn_non_gap_stream_error_fails_immediately() {
    rt().block_on(async {
        // A non-DataLoss transport error is NOT the agent's reconnect
        // contract — no retry, exactly one attach.
        let (addr, shared) = spawn_mock(MockState {
            events: completed_events("mock-run-1"),
            stream_attach_plan: vec![AttachPlan::HardErrorAfter(2)],
            ..Default::default()
        })
        .await;
        let mut client = AgentClient::connect(&addr).await.unwrap();
        let err = client
            .run_turn("sess", "mock-run-1", None, None)
            .await
            .unwrap_err();
        assert!(format!("{err:#}").contains("stream error"));
        let shared = shared.lock().unwrap();
        assert_eq!(
            shared.attach_after_idx,
            vec![-1],
            "non-gap errors never retry"
        );
    });
}

// ── execute_turn ───────────────────────────────────────────────────────────

#[test]
fn execute_turn_completed_no_validator() {
    rt().block_on(async {
        let (addr, shared) = spawn_mock(MockState {
            events: completed_events("mock-run-1"),
            tokens_in: 10,
            tokens_out: 20,
            cost: 0.5,
            ..Default::default()
        })
        .await;
        let mut client = AgentClient::connect(&addr).await.unwrap();
        let (goal, todo) = sample_goal_with_todo();
        let dir = tempfile::tempdir().unwrap();
        let runs_dir = dir.path().join("runs");
        std::fs::create_dir_all(&runs_dir).unwrap();
        // boundary_injected=false → append_system_prompt happens first.
        let record = execute_turn(
            &mut client,
            "sess",
            &goal,
            &todo,
            1,
            None,
            false,
            None,
            Some(runs_dir.clone()),
            None,
        )
        .await
        .unwrap();
        assert_eq!(record.terminal_state, "completed");
        assert_eq!(record.run_id, "mock-run-1");
        // get_state returns the same totals before/after → zero deltas.
        assert_eq!(record.tokens_in_delta, 0);
        assert!(record.validation.is_none(), "no validator ⇒ not required");
        assert!(runs_dir.join("mock-run-1.live.jsonl").exists());
        assert!(shared
            .lock()
            .unwrap()
            .recorded
            .contains(&"append_system_prompt".to_string()));
    });
}

#[test]
fn execute_turn_with_decision_summary_and_prev() {
    rt().block_on(async {
        let (addr, _) = spawn_mock(MockState {
            events: completed_events("mock-run-1"),
            ..Default::default()
        })
        .await;
        let mut client = AgentClient::connect(&addr).await.unwrap();
        let (goal, todo) = sample_goal_with_todo();
        let packet = future_loop::decision::decide_for(&goal, std::time::SystemTime::now(), None);
        let prev = sample_record("completed");
        let record = execute_turn(
            &mut client,
            "sess",
            &goal,
            &todo,
            2,
            Some(&prev),
            true,
            Some(&packet),
            None,
            None,
        )
        .await
        .unwrap();
        assert_eq!(record.terminal_state, "completed");
    });
}

#[test]
fn execute_turn_error_turn_skips_validator() {
    rt().block_on(async {
        let events = vec![ev(
            "mock-run-1",
            0,
            "agent_end",
            "{\"state\":\"error\",\"error\":\"x\"}",
        )];
        let (addr, _) = spawn_mock(MockState {
            events,
            ..Default::default()
        })
        .await;
        let mut client = AgentClient::connect(&addr).await.unwrap();
        let (goal, mut todo) = sample_goal_with_todo();
        todo.validator = Some("exit 0".to_string());
        let record = execute_turn(
            &mut client,
            "sess",
            &goal,
            &todo,
            1,
            None,
            true,
            None,
            None,
            None,
        )
        .await
        .unwrap();
        assert_eq!(record.terminal_state, "error");
        assert!(record.validation.is_none(), "failed turn never validates");
    });
}

#[cfg(unix)]
#[test]
fn execute_turn_validator_pass_and_fail() {
    rt().block_on(async {
        let (addr, _) = spawn_mock(MockState {
            events: completed_events("mock-run-1"),
            ..Default::default()
        })
        .await;
        let mut client = AgentClient::connect(&addr).await.unwrap();
        let (goal, mut todo) = sample_goal_with_todo();
        todo.validator = Some("exit 0".to_string());
        let record = execute_turn(
            &mut client,
            "sess",
            &goal,
            &todo,
            1,
            None,
            true,
            None,
            None,
            None,
        )
        .await
        .unwrap();
        let v = record.validation.unwrap();
        assert!(v.ok, "validator exit 0 passes: {}", v.summary);

        // Fresh client/session for the failing validator.
        let (addr, _) = spawn_mock(MockState {
            events: completed_events("mock-run-1"),
            ..Default::default()
        })
        .await;
        let mut client = AgentClient::connect(&addr).await.unwrap();
        let (goal, mut todo) = sample_goal_with_todo();
        todo.validator = Some("exit 3".to_string());
        let record = execute_turn(
            &mut client,
            "sess",
            &goal,
            &todo,
            1,
            None,
            true,
            None,
            None,
            None,
        )
        .await
        .unwrap();
        let v = record.validation.unwrap();
        assert!(!v.ok, "validator exit 3 fails");
        assert_eq!(v.exit_code, Some(3));
    });
}

// ── turn_succeeded / writeback (pure) ──────────────────────────────────────

#[test]
fn turn_succeeded_matrix() {
    let mut r = sample_record("completed");
    assert!(turn_succeeded(&r));
    r.terminal_state = "error".into();
    assert!(!turn_succeeded(&r));
    // Validation gates success.
    let mut r = sample_record("completed");
    r.validation = Some(future_loop::state::task_validation_receipt(
        future_loop::state::ValidationStatus::Failed,
        "exit 1",
        "failed",
        Some(future_loop::state::RecoveryKind::RepairRequired),
        Some(1),
    ));
    assert!(!turn_succeeded(&r));
}

#[test]
fn writeback_surface_only_turn_accumulates_outcome_streak() {
    let (mut goal, todo) = sample_goal_with_todo();
    // No tools + empty evidence → surface-only turn → streak grows.
    let mut record = sample_record("completed");
    record.todo_id = todo.id.clone();
    record.tools = vec![];
    record.evidence = String::new();
    writeback(&mut goal, &record, None, Some((true, vec![])));
    assert_eq!(goal.outcome_streak, 1);
    // A material turn (tools + evidence) resets the streak to zero.
    let mut material = sample_record("completed");
    material.todo_id = todo.id.clone();
    writeback(&mut goal, &material, None, Some((true, vec![])));
    assert_eq!(goal.outcome_streak, 0);
}

#[test]
fn writeback_missing_todo_guards() {
    // monitor poll for a todo that does not exist → early return, no panic.
    let (mut goal, _) = sample_goal_with_todo();
    let mut record = sample_record("completed");
    record.todo_id = "ghost".into();
    writeback(&mut goal, &record, Some(true), None);
    writeback(&mut goal, &record, Some(false), None);
    assert!(goal.history.is_empty());
    // succeeded turn for a missing todo → completion skipped, history pushed.
    writeback(&mut goal, &record, None, Some((true, vec![])));
    assert_eq!(goal.history.len(), 1);
    // failed turn for a missing todo → no failed_attempts bump.
    let mut failed = sample_record("error");
    failed.todo_id = "ghost".into();
    writeback(&mut goal, &failed, None, None);
    assert_eq!(goal.todos.len(), 1);
}

#[test]
fn writeback_monitor_paths() {
    let (mut goal, todo) = sample_goal_with_todo();
    let mut monitor = todo.clone();
    monitor.id = "mon_1".into();
    monitor.class = future_loop::state::TaskClass::Monitor;
    goal.todos.push(monitor);
    let mut record = sample_record("completed");
    record.todo_id = "mon_1".into();

    // changed → monitor done, history appended.
    writeback(&mut goal, &record, Some(true), None);
    let m = goal.todo("mon_1").unwrap();
    assert_eq!(m.status, TodoStatus::Done);
    assert_eq!(goal.history.len(), 1);

    // no-change → counter bumped, deferred, NOT recorded in history.
    let (mut goal, _) = sample_goal_with_todo();
    let mut monitor = Todo::advancement("mon_1", "watch");
    monitor.class = future_loop::state::TaskClass::Monitor;
    goal.todos.push(monitor);
    let mut record = sample_record("completed");
    record.todo_id = "mon_1".into();
    writeback(&mut goal, &record, Some(false), None);
    let m = goal.todo("mon_1").unwrap();
    assert_eq!(m.consecutive_no_change, 1);
    assert!(m.resume_when.is_some());
    assert!(goal.history.is_empty());
}

#[test]
fn writeback_completion_and_repair_paths() {
    // success with explicit completion intent.
    let (mut goal, _) = sample_goal_with_todo();
    let record = sample_record("completed");
    writeback(&mut goal, &record, None, Some((true, vec![])));
    assert_eq!(goal.todo("todo_x").unwrap().status, TodoStatus::Done);
    assert!(goal.todo("todo_x").unwrap().no_follow_up);
    assert_eq!(goal.history.len(), 1);
    assert_eq!(goal.outcome_streak, 0, "material turn resets the streak");

    // success with completion None → defaults to no-follow-up.
    let (mut goal, _) = sample_goal_with_todo();
    writeback(&mut goal, &sample_record("completed"), None, None);
    assert!(goal.todo("todo_x").unwrap().no_follow_up);

    // failure → repair attempt + outcome streak (no tools/evidence).
    let (mut goal, _) = sample_goal_with_todo();
    let mut record = sample_record("error");
    record.tools = vec![];
    record.evidence = String::new();
    writeback(&mut goal, &record, None, None);
    assert_eq!(goal.todo("todo_x").unwrap().failed_attempts, 1);
    assert_eq!(goal.outcome_streak, 1);
}
