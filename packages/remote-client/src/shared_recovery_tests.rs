//! Actual supervisor budget/stop scenarios with explicit failing host IO.
use super::*;
use serde::Deserialize;
use std::collections::HashSet;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Suite {
    version: u32,
    suite: String,
    now_ms: u64,
    rust_observation_ms: u64,
    typescript_budget_ms: u64,
    cases: Vec<Case>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Case {
    id: String,
    #[serde(deserialize_with = "nullable_count")]
    close_after_attempts: Option<usize>,
    expected: Languages,
    difference: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Languages {
    rust: usize,
    typescript: usize,
}
fn nullable_count<'de, D: serde::Deserializer<'de>>(input: D) -> Result<Option<usize>, D::Error> {
    Option::deserialize(input)
}
fn suite() -> Suite {
    let suite: Suite = serde_json::from_str(include_str!("../fixtures/recovery.json")).unwrap();
    assert_eq!(suite.version, 1);
    assert_eq!(suite.suite, "client.recovery");
    assert!(suite.now_ms > 0 && suite.now_ms <= 9_007_199_254_740_991);
    assert!(suite.rust_observation_ms > 0 && suite.rust_observation_ms <= 10_000);
    assert!(suite.typescript_budget_ms > 0 && suite.typescript_budget_ms <= 600_000);
    assert!(!suite.cases.is_empty());
    let mut ids = HashSet::new();
    for case in &suite.cases {
        assert!(case.id.starts_with("client.recovery.") && ids.insert(&case.id));
        assert!(!case.difference.trim().is_empty());
        assert!(case.expected.rust <= 100 && case.expected.typescript <= 100);
        assert!(case.close_after_attempts.is_none_or(|count| count <= 100));
        if let Some(count) = case.close_after_attempts {
            assert_eq!(case.expected.rust, count);
            assert_eq!(case.expected.typescript, count);
        }
    }
    suite
}
async fn settle() {
    for _ in 0..16 {
        tokio::task::yield_now().await;
    }
}

#[tokio::test(start_paused = true)]
async fn shared_supervisor_budget_and_stop() {
    let suite = suite();
    for case in suite.cases {
        let host = Arc::new(MemoryHost::default());
        host.reject_load.store(true, Ordering::SeqCst);
        let client = Client::new(host.clone());
        // The real reconnect task calls resume/connect_with_emitter. Only the
        // explicit host storage boundary fails, before creating any socket.
        let task = client.spawn_supervisor("desktop_fixture".into(), None);
        let mut closed = false;
        for tick in 0..=suite.rust_observation_ms {
            settle().await;
            let attempts = host.rejected_loads.load(Ordering::SeqCst);
            if case.close_after_attempts == Some(attempts) {
                client.shutdown(Duration::from_secs(1)).await.unwrap();
                closed = true;
                break;
            }
            if case.close_after_attempts.is_none() && task.is_finished() {
                break;
            }
            assert!(
                tick < suite.rust_observation_ms,
                "{}: supervisor did not settle",
                case.id
            );
            tokio::time::advance(Duration::from_millis(1)).await;
        }
        assert_eq!(
            host.rejected_loads.load(Ordering::SeqCst),
            case.expected.rust,
            "{}",
            case.id
        );
        assert_eq!(closed, case.close_after_attempts.is_some());
        assert_eq!(client.is_stopped(), closed);
        assert!(task.is_finished(), "{}: ownership remains live", case.id);
        // Natural exhaustion must also be inert before the role is closed.
        tokio::time::advance(Duration::from_secs(600)).await;
        settle().await;
        assert_eq!(
            host.rejected_loads.load(Ordering::SeqCst),
            case.expected.rust,
            "{}: late retry",
            case.id
        );
        client.shutdown(Duration::from_secs(1)).await.unwrap();
        println!("remote-client shared rust {} PASS", case.id);
    }
}
