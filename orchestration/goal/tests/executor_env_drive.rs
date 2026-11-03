//! Executor validator spawn-failure arm: with an EMPTY PATH the hardcoded
//! `sh` cannot be spawned → ValidationStatus::Inconclusive. In its own test
//! binary because PATH is process-global.

mod common;

use common::mock_agent::{completed_events, spawn_mock, MockState};
use future_goal::agent_client::AgentClient;
use future_goal::executor::execute_turn;
use future_goal::state::{Goal, Todo, ValidationStatus};

#[test]
fn validator_spawn_failure_is_inconclusive() {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap();
    rt.block_on(async {
        let (addr, _) = spawn_mock(MockState {
            events: completed_events("mock-run-1"),
            ..Default::default()
        })
        .await;
        let mut client = AgentClient::connect(&addr).await.unwrap();
        let mut goal = Goal::new("g", "exec", "/tmp");
        let mut todo = Todo::advancement("t1", "validated");
        todo.validator = Some("exit 0".to_string());
        goal.todos.push(todo.clone());

        let saved = std::env::var_os("PATH");
        std::env::set_var("PATH", "/nonexistent-dir-xyz");
        let worker_config = future_goal::worker_config::WorkerConfig {
            model: "future/test-model".to_string(),
            thinking_level: "medium".to_string(),
            source: future_goal::worker_config::ConfigSource::Default,
        };
        let record = execute_turn(
            &mut client,
            "sess",
            &goal,
            None,
            &todo,
            1,
            None,
            true,
            None,
            None,
            None,
            &worker_config,
        )
        .await
        .unwrap();
        if let Some(p) = saved {
            std::env::set_var("PATH", p);
        }
        let v = record.validation.unwrap();
        if cfg!(unix) {
            // No `sh` on PATH → spawn error → inconclusive.
            assert_eq!(v.status, ValidationStatus::Inconclusive, "{}", v.summary);
            assert!(!v.ok);
        }
    });
}
