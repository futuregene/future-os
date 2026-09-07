//! End-to-end (mock-agent) completion regressions: contracts are enforced on
//! automatic writeback, final results survive long streams, and owner-scoped
//! tasks cannot disappear from the supervisor's view.
mod common;
use common::mock_agent::*;
use common::*;
use future_loop::state::TodoStatus;
use future_loop::store::Event;

fn mock(text: &str) -> (tokio::runtime::Runtime, SharedState) {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap();
    let events = replies("mock-run-1", &[text]);
    let (addr, state) = runtime.block_on(spawn_mock(MockState {
        events,
        ..Default::default()
    }));
    std::env::set_var("FUTURE_LOOP_AGENT_ADDR", addr);
    (runtime, state)
}

fn replies(run: &str, chunks: &[&str]) -> Vec<future_rpc::proto::StreamEvent> {
    let mut events = vec![ev(run, 0, "agent_start", "{}")];
    for (index, text) in chunks.iter().enumerate() {
        events.push(ev(
            run,
            index as i64 + 1,
            "text_chunk",
            &serde_json::json!({"text": text}).to_string(),
        ));
    }
    events.push(ev(
        run,
        chunks.len() as i64 + 1,
        "agent_end",
        r#"{"state":"completed"}"#,
    ));
    events
}

fn prepared_goal(cr: &CliRoot) -> String {
    let goal = init_goal(cr, "completion contract regression");
    cli_ok(&[
        "todo",
        "complete",
        "--goal",
        &goal,
        "--todo-id",
        &first_todo_id(&cr.root, &goal),
        "--no-follow-up",
        "--evidence",
        "connection checked",
    ]);
    goal
}

#[test]
fn automatic_and_manual_paths_reject_missing_tokens_then_accept_a_repaired_handoff() {
    let cr = cli_root();
    let (_runtime, state) = mock("attempt 123 queued");
    let goal = prepared_goal(&cr);
    cli_ok(&[
        "todo",
        "add",
        "--goal",
        &goal,
        "--text",
        "deliver result",
        "--owner",
        "solver",
        "--acceptance",
        "attempt,scored",
    ]);
    let todo = todo_id_by_text(&cr.root, &goal, "deliver result");
    let _ = cli_err(&[
        "run",
        "--goal",
        &goal,
        "--agent-id",
        "solver",
        "--max-turns",
        "1",
    ]);
    let store = open_store(&cr);
    let current = store.replay(&goal).unwrap().unwrap();
    assert_eq!(current.todo(&todo).unwrap().status, TodoStatus::Open);
    let record = current.history.iter().find(|r| r.todo_id == todo).unwrap();
    assert!(record
        .error
        .as_ref()
        .unwrap()
        .contains("acceptance contract unmet"));
    assert!(!store.events(&goal).unwrap().iter().any(|e| matches!(
        &e.event, Event::TodoCompleted { todo_id, .. } if todo_id == &todo
    )));
    assert!(cli_err(&[
        "todo",
        "complete",
        "--goal",
        &goal,
        "--todo-id",
        &todo,
        "--no-follow-up",
        "--evidence",
        "attempt 123 queued",
    ])
    .contains("acceptance contract unmet"));
    state.lock().unwrap().events = replies(
        "mock-run-2",
        &["attempt 123 SCORED 0; scientific requirements not met"],
    );
    cli_ok(&[
        "run",
        "--goal",
        &goal,
        "--agent-id",
        "solver",
        "--max-turns",
        "3",
    ]);
    assert_eq!(
        open_store(&cr)
            .replay(&goal)
            .unwrap()
            .unwrap()
            .todo(&todo)
            .unwrap()
            .status,
        TodoStatus::Done
    );
}

#[test]
fn normal_model_return_without_evidence_does_not_close_a_todo() {
    let cr = cli_root();
    let (_runtime, _) = mock("   ");
    let goal = prepared_goal(&cr);
    let todo = add_todo(&cr, &goal, "write an artifact");
    let _ = cli_err(&[
        "run",
        "--goal",
        &goal,
        "--agent-id",
        "solver",
        "--max-turns",
        "1",
    ]);
    let goal = open_store(&cr).replay(&goal).unwrap().unwrap();
    assert_eq!(goal.todo(&todo).unwrap().status, TodoStatus::Open);
    assert!(goal
        .history
        .last()
        .unwrap()
        .error
        .as_ref()
        .unwrap()
        .contains("non-empty"));
}

#[test]
fn late_result_survives_stream_ledger_and_structured_notice_with_other_owners_pending() {
    let cr = cli_root();
    let (_runtime, state) = mock("");
    let early = "Opening exploration. ".repeat(1000);
    let final_text = "FINAL: attempt 456 scored 91. Verified artifact work/result.json; remaining uncertainty documented.";
    state.lock().unwrap().events = replies("mock-run-1", &[&early, final_text]);
    let goal = prepared_goal(&cr);
    for (owner, text) in [("alice", "alice work"), ("bob", "bob work")] {
        cli_ok(&[
            "todo",
            "add",
            "--goal",
            &goal,
            "--text",
            text,
            "--owner",
            owner,
            "--acceptance",
            "attempt,scored",
        ]);
    }
    let alice = todo_id_by_text(&cr.root, &goal, "alice work");
    let bob = todo_id_by_text(&cr.root, &goal, "bob work");
    cli_ok(&[
        "todo",
        "add",
        "--goal",
        &goal,
        "--text",
        "global review",
        "--class",
        "coordination",
        "--blocks",
        &format!("{alice},{bob}"),
    ]);
    cli_ok(&[
        "run",
        "--goal",
        &goal,
        "--agent-id",
        "alice",
        "--max-turns",
        "3",
    ]);
    let store = open_store(&cr);
    let current = store.replay(&goal).unwrap().unwrap();
    assert_eq!(current.todo(&alice).unwrap().status, TodoStatus::Done);
    assert_eq!(current.todo(&bob).unwrap().status, TodoStatus::Open);
    assert!(
        current.todo(&alice).unwrap().successor_ids.is_empty(),
        "unrelated work is not a semantic successor"
    );
    assert!(!current.is_terminal());
    let record = current.history.iter().find(|r| r.todo_id == alice).unwrap();
    assert!(record.evidence.ends_with(final_text));
    let notice = store
        .events(&goal)
        .unwrap()
        .into_iter()
        .find_map(|e| match e.event {
            Event::SupervisorNote {
                todo_id,
                note_kind,
                message,
                ..
            } if todo_id == alice && note_kind == "completed" => Some(message),
            _ => None,
        })
        .unwrap();
    let receipt: serde_json::Value = serde_json::from_str(&notice).unwrap();
    assert_eq!(receipt["pending_other_todos"], 2);
    assert_eq!(receipt["agent_id"], "alice");
    assert_eq!(receipt["run_id"], record.run_id);
    assert_eq!(receipt["session_id"], "mock-session-1");
    assert_eq!(receipt["delivery"], "awaiting_review");
    assert!(receipt["evidence_tail"]
        .as_str()
        .unwrap()
        .contains(final_text));
    assert!(!notice.contains("last todo"));
    let journal = std::fs::read_to_string(receipt["full_text_journal"].as_str().unwrap()).unwrap();
    let full_text: String = journal
        .lines()
        .filter_map(|line| {
            let value: serde_json::Value = serde_json::from_str(line).unwrap();
            value["text"].as_str().map(str::to_owned)
        })
        .collect();
    assert_eq!(full_text, format!("{early}{final_text}"));
}
