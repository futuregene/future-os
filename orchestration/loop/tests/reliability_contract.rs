//! Fault-oriented contracts: delivery survives disconnects, steering is scoped,
//! and observation does not silently become a correctness claim.
mod common;
use common::mock_agent::*;
use common::*;
use future_loop::{
    agents::{control, supervision},
    store::Event,
};

fn rt() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap()
}
fn mock_env(state: MockState) -> (tokio::runtime::Runtime, SharedState) {
    let rt = rt();
    let (addr, shared) = rt.block_on(spawn_mock(state));
    std::env::set_var("FUTURE_LOOP_AGENT_ADDR", addr);
    (rt, shared)
}

#[test]
fn outbox_batches_and_retries_an_immutable_payload_after_disconnect() {
    let cr = cli_root();
    let goal = init_goal(&cr, "outbox");
    let mut store = open_store(&cr);
    store
        .append(Event::SupervisorRegistered {
            goal_id: goal.clone(),
            session_id: "supervisor".into(),
            ts: 1,
        })
        .unwrap();
    for n in 0..3 {
        supervision::queue(
            &mut store,
            &goal,
            "completed",
            "t",
            &format!("result {n}"),
            &format!("key-{n}"),
        )
        .unwrap();
    }
    rt().block_on(async {
        let (addr, shared) = spawn_mock(MockState::fail("prompt")).await;
        let mut client = future_loop::agent_client::AgentClient::connect(&addr)
            .await
            .unwrap();
        assert!(supervision::flush(&mut store, &goal, &mut client)
            .await
            .is_err());
        let batches: Vec<_> = store
            .events(&goal)
            .unwrap()
            .into_iter()
            .filter_map(|e| match e.event {
                Event::SupervisorBatchPrepared {
                    batch_id, message, ..
                } => Some((batch_id, message)),
                _ => None,
            })
            .collect();
        assert_eq!(batches.len(), 1);
        assert!(supervision::pending_delivery(&store, &goal).unwrap());
        // New input while disconnected must NOT mutate the in-flight batch.
        supervision::queue(&mut store, &goal, "completed", "t", "new result", "key-new").unwrap();
        drop(store);
        let mut store = open_store(&cr);
        shared.lock().unwrap().fail_commands.clear();
        supervision::flush(&mut store, &goal, &mut client)
            .await
            .unwrap();
        assert_eq!(
            shared.lock().unwrap().prompt_messages,
            vec![batches[0].1.clone()]
        );
        assert!(supervision::pending_delivery(&store, &goal).unwrap());
        supervision::flush(&mut store, &goal, &mut client)
            .await
            .unwrap();
        supervision::flush(&mut store, &goal, &mut client)
            .await
            .unwrap();
        assert_eq!(
            shared.lock().unwrap().prompt_calls.len(),
            2,
            "no duplicate wakeups"
        );
        assert!(!supervision::pending_delivery(&store, &goal).unwrap());
    });
}

#[test]
fn steer_is_acknowledged_only_after_completed_writeback() {
    let cr = cli_root();
    let (_rt, shared) = mock_env(MockState {
        stream_error: true,
        ..Default::default()
    });
    let goal = init_goal(&cr, "steering restart");
    cli_ok(&[
        "supervisor",
        "steer",
        "--goal",
        &goal,
        "--agent-id",
        "a",
        "--instruction",
        "preserve this correction",
    ]);
    let _ = cli_err(&[
        "run",
        "--goal",
        &goal,
        "--agent-id",
        "a",
        "--max-turns",
        "1",
    ]);
    assert_eq!(
        control::pending(&open_store(&cr), &goal, Some("a"))
            .unwrap()
            .len(),
        1
    );
    {
        let mut mock = shared.lock().unwrap();
        mock.stream_error = false;
        mock.events = completed_events("mock-run-2");
    }
    cli_ok(&[
        "run",
        "--goal",
        &goal,
        "--agent-id",
        "a",
        "--max-turns",
        "3",
    ]);
    assert!(control::pending(&open_store(&cr), &goal, Some("a"))
        .unwrap()
        .is_empty());
    let messages = shared.lock().unwrap().prompt_messages.clone();
    assert!(messages
        .iter()
        .all(|m| m.contains("preserve this correction")));
}

#[test]
fn independent_work_completes_but_reverse_and_global_gate_edges_block() {
    let cr = cli_root();
    let goal = init_goal(&cr, "gate scope");
    let blocked = add_todo(&cr, &goal, "blocked work");
    let independent = add_todo(&cr, &goal, "independent work");
    cli_ok(&[
        "todo",
        "add",
        "--goal",
        &goal,
        "--text",
        "approve",
        "--class",
        "user_gate",
        "--blocks",
        &blocked,
    ]);
    cli_ok(&[
        "todo",
        "complete",
        "--goal",
        &goal,
        "--todo-id",
        &independent,
        "--no-follow-up",
        "--evidence",
        "reviewed independent artifact",
    ]);
    let store = open_store(&cr);
    let state = store.replay(&goal).unwrap().unwrap();
    assert!(state.is_blocked(state.todo(&blocked).unwrap()));
    assert!(!state.runnable_advancement().any(|t| t.id == blocked));
    assert!(cli_err(&[
        "todo",
        "complete",
        "--goal",
        &goal,
        "--todo-id",
        &blocked,
        "--no-follow-up",
        "--evidence",
        "not approved"
    ])
    .contains("open gate"));
}

#[test]
fn fan_in_never_hides_late_sources_and_contract_is_always_visible() {
    use future_loop::state::{Goal, Todo, TodoStatus};
    let mut goal = Goal::new("g", "synthesize", ".");
    let ids: Vec<String> = (0..40).map(|n| format!("source-{n}")).collect();
    for id in &ids {
        let mut todo = Todo::advancement(id, "source");
        todo.status = TodoStatus::Done;
        todo.evidence = Some("evidence ".repeat(600));
        goal.add(todo);
    }
    let mut synthesis = Todo::advancement("synthesis", "read upstream artifacts")
        .blocking(&ids.iter().map(String::as_str).collect::<Vec<_>>());
    synthesis.validator = Some("python validate.py".into());
    synthesis.acceptance = Some("measured,replicated".into());
    let envelope = future_loop::turn_envelope::compose_turn_envelope(&goal, &synthesis, None);
    for id in ids {
        assert!(envelope.contains(&format!("upstream {id}:")));
    }
    assert!(envelope.contains("python validate.py"));
    assert!(envelope.contains("measured,replicated"));
    assert!(
        envelope.len() < 6000,
        "summaries are bounded, not copied wholesale"
    );
}

#[test]
fn input_phase_does_not_count_twice_and_shell_is_only_activity() {
    rt().block_on(async {
        let events = vec![
            ev(
                "r",
                0,
                "tool_start",
                r#"{"tool_name":"write","phase":"input"}"#,
            ),
            ev(
                "r",
                1,
                "tool_start",
                r#"{"tool_name":"write","phase":"execution"}"#,
            ),
            ev(
                "r",
                2,
                "tool_start",
                r#"{"tool_name":"shell","phase":"execution"}"#,
            ),
            ev("r", 3, "agent_end", r#"{"state":"completed"}"#),
        ];
        let (addr, _) = spawn_mock(MockState {
            events,
            ..Default::default()
        })
        .await;
        let mut client = future_loop::agent_client::AgentClient::connect(&addr)
            .await
            .unwrap();
        let progress = future_loop::agent_client::TurnProgressTracker::new(1);
        let summary = client
            .run_turn("s", "r", None, Some(&progress))
            .await
            .unwrap();
        assert_eq!(progress.snapshot().tool_calls_total, 2);
        assert_eq!(summary.tools, ["write", "shell"]);
    });
}

#[cfg(unix)]
#[test]
fn detached_watchdog_reports_death_of_the_only_worker() {
    let cr = cli_root();
    let (_rt, shared) = mock_env(MockState {
        hang_stream: true,
        ..Default::default()
    });
    let goal = init_goal(&cr, "sole worker liveness");
    let mut store = open_store(&cr);
    store
        .append(Event::SupervisorRegistered {
            goal_id: goal.clone(),
            session_id: "supervisor".into(),
            ts: 1,
        })
        .unwrap();
    // Always cancel the test goal, including on an assertion panic. This also
    // lets the independent watchdog exit before its temporary root is removed.
    struct Cleanup {
        root: String,
        goal: String,
    }
    impl Drop for Cleanup {
        fn drop(&mut self) {
            if let Ok(mut store) = future_loop::store::Store::open(&self.root) {
                let _ = store.append(Event::GoalCancelled {
                    goal_id: self.goal.clone(),
                    reason: "test cleanup".into(),
                    ts: 2,
                });
            }
        }
    }
    let cleanup = Cleanup {
        root: cr.root.clone(),
        goal: goal.clone(),
    };
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_future-loop"))
        .args([
            "run",
            "--goal",
            &goal,
            "--agent-id",
            "sole",
            "--max-turns",
            "1",
        ])
        .env_remove("FUTURE_LOOP_NO_DETACH")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    let pid = loop {
        if let Some(pid) = store
            .replay(&goal)
            .unwrap()
            .unwrap()
            .todos
            .iter()
            .find_map(|t| t.holder_pid)
        {
            break pid;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "worker never acquired its lease"
        );
        std::thread::sleep(std::time::Duration::from_millis(20));
    };
    // SAFETY: this pid belongs to the detached worker created by this test.
    assert_eq!(unsafe { libc::kill(pid as i32, libc::SIGKILL) }, 0);
    loop {
        if shared
            .lock()
            .unwrap()
            .prompt_messages
            .iter()
            .any(|m| m.contains("host_died"))
        {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "last-worker death was not delivered by the independent watchdog"
        );
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    drop(cleanup);
    while supervision::running(&store, &goal) {
        assert!(
            std::time::Instant::now() < deadline,
            "watchdog did not exit on cancellation"
        );
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
}

#[test]
fn dead_holder_is_recorded_without_any_agent_connection() {
    let cr = cli_root();
    let goal = init_goal(&cr, "dead worker offline");
    let todo = first_todo_id(&cr.root, &goal);
    // A PID obtained from our own reaped child, not an arbitrary real process.
    let mut child = std::process::Command::new(env!("CARGO_BIN_EXE_future-loop"))
        .arg("version")
        .stdout(std::process::Stdio::null())
        .spawn()
        .unwrap();
    let pid = child.id();
    child.wait().unwrap();
    let mut store = open_store(&cr);
    store
        .append(Event::TodoClaimed {
            goal_id: goal.clone(),
            todo_id: todo.clone(),
            agent_id: "dead".into(),
            lease_expires_at: future_loop::state::now_epoch() + 300,
            holder_pid: Some(pid),
            ts: 1,
        })
        .unwrap();
    supervision::record_dead_holders(&mut store, &goal).unwrap();
    supervision::record_dead_holders(&mut store, &goal).unwrap();
    assert_eq!(store.events(&goal).unwrap().iter().filter(|e| matches!(&e.event, Event::SupervisorNote { note_kind, .. } if note_kind == "host_died")).count(), 1);
}
