//! Same scenario inputs as the Node driver; production retry/cancel code with
//! a simulated transport and Tokio's paused clock. No broker or application.
use super::*;
use crate::session::{
    command_plaintext, retry_request, retryable_for_features, COMMAND_ATTEMPTS, COMMAND_TIMEOUT,
};
use serde::Deserialize;
use std::collections::HashSet;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Suite {
    version: u32,
    suite: String,
    bridge_instance_id: String,
    request_timeout_ms: u64,
    clock_slack_ms: Languages<u64>,
    response: Value,
    cases: Vec<Case>,
}
#[derive(Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
struct Languages<T> {
    rust: T,
    typescript: T,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Case {
    id: String,
    kind: String,
    command: Value,
    features: Vec<String>,
    outcomes: Vec<TransportOutcome>,
    cancel_at_ms: Option<u64>,
    expected: Languages<Expected>,
    difference: Option<String>,
}
#[derive(Deserialize, PartialEq, Debug)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Expected {
    attempt_at_ms: Vec<u64>,
    outcome: String,
    recoveries: u64,
    probes: u64,
    complete_at_ms: u64,
}
#[derive(Deserialize, Clone, Copy)]
#[serde(rename_all = "snake_case")]
enum TransportOutcome {
    Success,
    Transport,
    Timeout,
    Business,
    Authorization,
}

fn suite() -> Suite {
    let suite: Suite = serde_json::from_str(include_str!("../fixtures/requests.json")).unwrap();
    assert_eq!(suite.version, 1, "unknown request fixture version");
    assert_eq!(
        suite.suite, "client.request-policy",
        "unknown request fixture suite"
    );
    assert_eq!(
        u128::from(suite.request_timeout_ms),
        COMMAND_TIMEOUT.as_millis()
    );
    const MAX_SAFE_INTEGER: u64 = 9_007_199_254_740_991;
    assert!(!suite.bridge_instance_id.is_empty());
    assert!(suite.request_timeout_ms > 0 && suite.request_timeout_ms <= MAX_SAFE_INTEGER);
    assert!(
        suite.clock_slack_ms.rust <= MAX_SAFE_INTEGER
            && suite.clock_slack_ms.typescript <= MAX_SAFE_INTEGER
    );
    let mut ids = HashSet::new();
    for case in &suite.cases {
        assert!(
            case.id.starts_with("client.request-policy.") && ids.insert(&case.id),
            "invalid/duplicate case ID"
        );
        assert_eq!(case.kind, "request", "unsupported shared case kind");
        assert!(
            case.command["type"].is_string() && case.command["id"].is_string(),
            "missing command identity"
        );
        assert!(!case.outcomes.is_empty());
        assert!(case
            .cancel_at_ms
            .is_none_or(|time| time <= MAX_SAFE_INTEGER));
        assert!(case.features.iter().all(|feature| !feature.is_empty()));
        if case.expected.rust != case.expected.typescript {
            assert!(
                case.difference
                    .as_ref()
                    .is_some_and(|note| !note.trim().is_empty()),
                "undocumented language difference: {}",
                case.id
            );
        }
        for expected in [&case.expected.rust, &case.expected.typescript] {
            assert!(!expected.attempt_at_ms.is_empty());
            assert!([
                expected.recoveries,
                expected.probes,
                expected.complete_at_ms
            ]
            .iter()
            .all(|value| *value <= MAX_SAFE_INTEGER));
            assert!(expected
                .attempt_at_ms
                .iter()
                .all(|time| *time <= MAX_SAFE_INTEGER));
            assert!(expected.attempt_at_ms.len() <= case.outcomes.len());
            assert!(expected
                .attempt_at_ms
                .windows(2)
                .all(|pair| pair[0] <= pair[1]));
            assert!(*expected.attempt_at_ms.last().unwrap() <= expected.complete_at_ms);
            assert!([
                "success",
                "transport",
                "business",
                "authorization",
                "cancelled"
            ]
            .contains(&expected.outcome.as_str()));
        }
    }
    suite
}
fn outcome(result: &Result<Value, crate::Error>) -> &str {
    match result {
        Ok(_) => "success",
        Err(crate::Error::RemoteTransport(_)) => "transport",
        Err(crate::Error::RemoteAuthorization(_)) => "authorization",
        Err(crate::Error::Message(message)) if message == "remote_client_stopped" => "cancelled",
        Err(crate::Error::Message(message)) if message == "fixture_refused" => "business",
        _ => "unexpected",
    }
}
fn assert_time(actual: u128, expected: u64, slack: u64, id: &str) {
    assert!(
        actual >= u128::from(expected) && actual <= u128::from(expected + slack),
        "{id}: virtual time {actual}, expected {expected}..={}",
        expected + slack
    );
}

#[tokio::test(start_paused = true)]
async fn shared_request_scenarios_use_the_production_retry_loop_and_stop_gate() {
    let suite = suite();
    for case in &suite.cases {
        let host = Arc::new(MemoryHost::default());
        let client = Client::new(host);
        let started = tokio::time::Instant::now();
        let observations = Arc::new(StdMutex::new(Vec::new()));
        let plaintext = command_plaintext(case.command.clone(), &suite.bridge_instance_id).unwrap();
        let attempts = if retryable_for_features(&case.command, &case.features) {
            COMMAND_ATTEMPTS
        } else {
            1
        };
        let exchange = |body: &[u8]| {
            let (index, payload) = {
                let mut observations = observations.lock().unwrap();
                let index = observations.len();
                let payload: Value = serde_json::from_slice(body).unwrap();
                observations.push((started.elapsed().as_millis(), payload.clone()));
                (index, payload)
            };
            assert!(payload.is_object());
            let scripted = *case.outcomes.get(index).expect("unexpected extra request");
            let response = suite.response.clone();
            async move {
                match scripted {
                    TransportOutcome::Success => Ok(response),
                    TransportOutcome::Transport => {
                        Err(crate::Error::RemoteTransport("fixture_transport".into()))
                    }
                    TransportOutcome::Timeout => {
                        tokio::time::sleep(COMMAND_TIMEOUT).await;
                        Err(crate::Error::RemoteTransport("fixture_timeout".into()))
                    }
                    TransportOutcome::Business => {
                        Err(crate::Error::Message("fixture_refused".into()))
                    }
                    TransportOutcome::Authorization => Err(crate::Error::RemoteAuthorization(
                        "fixture_authorization".into(),
                    )),
                }
            }
        };
        let request = client.while_running(retry_request(&plaintext, attempts, exchange));
        tokio::pin!(request);
        let cancel = async {
            if let Some(after) = case.cancel_at_ms {
                tokio::time::sleep(Duration::from_millis(after)).await;
                client.stop_admission();
            } else {
                std::future::pending::<()>().await;
            }
        };
        tokio::pin!(cancel);
        let result = tokio::select! {
            result = &mut request => result,
            _ = &mut cancel => request.await,
        };
        let expected = &case.expected.rust;
        assert_eq!(outcome(&result), expected.outcome, "{}", case.id);
        if let Ok(data) = result {
            assert_eq!(data, suite.response, "{}", case.id);
        }
        assert_time(
            started.elapsed().as_millis(),
            expected.complete_at_ms,
            suite.clock_slack_ms.rust,
            &case.id,
        );
        let recorded = observations.lock().unwrap().clone();
        assert_eq!(recorded.len(), expected.attempt_at_ms.len(), "{}", case.id);
        let mut expected_body = case.command.clone();
        expected_body
            .as_object_mut()
            .unwrap()
            .entry("bridgeInstanceId")
            .or_insert_with(|| json!(suite.bridge_instance_id));
        for ((time, body), expected_time) in recorded.iter().zip(&expected.attempt_at_ms) {
            assert_time(*time, *expected_time, suite.clock_slack_ms.rust, &case.id);
            assert_eq!(
                body, &expected_body,
                "{}: retry changed command identity/body",
                case.id
            );
        }
        // The session loop has no recovery/probe side channel; the outer Rust
        // runtime owns connection rebuilding. TS explicitly reports that delta.
        assert_eq!(expected.recoveries, 0, "{}", case.id);
        assert_eq!(expected.probes, 0, "{}", case.id);
        client.shutdown(Duration::from_secs(1)).await.unwrap();
        tokio::time::advance(Duration::from_secs(60)).await;
        assert_eq!(
            observations.lock().unwrap().len(),
            recorded.len(),
            "{}: late resend after stop",
            case.id
        );
        println!("remote-client shared rust {} PASS", case.id);
    }
}
