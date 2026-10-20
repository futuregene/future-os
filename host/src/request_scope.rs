//! Per-host command admission and exact-run ownership. Existing external
//! services retain every run that this host did not submit.
use future_rpc::connection::VerifiedAgentClient;
use std::collections::HashMap;
use std::sync::{Condvar, LazyLock, Mutex};
use std::time::{Duration, Instant};

type OwnedRun = (String, String);
struct Admission {
    closed: bool,
    active: usize,
}
static ADMISSION: Mutex<Admission> = Mutex::new(Admission {
    closed: false,
    active: 0,
});
static CHANGED: Condvar = Condvar::new();
static OWNED: LazyLock<Mutex<HashMap<OwnedRun, VerifiedAgentClient>>> =
    LazyLock::new(Default::default);
pub struct CommandGuard;
impl Drop for CommandGuard {
    fn drop(&mut self) {
        ADMISSION.lock().unwrap().active -= 1;
        CHANGED.notify_all();
    }
}
#[allow(clippy::result_large_err)] // Tonic callers preserve native Status errors.
pub fn admit() -> Result<CommandGuard, tonic::Status> {
    let mut admission = ADMISSION.lock().unwrap();
    if admission.closed {
        return Err(tonic::Status::unavailable("Desktop is stopping"));
    }
    admission.active += 1;
    Ok(CommandGuard)
}
pub fn is_stopping() -> bool {
    ADMISSION.lock().unwrap().closed
}
pub fn own_run(session: String, run: String, client: VerifiedAgentClient) {
    if !session.is_empty() && !run.is_empty() {
        OWNED.lock().unwrap().insert((session, run), client);
    }
}
pub fn release_run(session: &str, run: &str) {
    OWNED.lock().unwrap().remove(&(session.into(), run.into()));
}
pub fn owned_count() -> usize {
    OWNED.lock().unwrap().len()
}

/// A collector can fail or hand queued work to an observer. Reconcile even
/// locally terminal rows, but never confuse a lost connection with a terminal.
pub async fn reconcile_owned_runs() {
    let runs: Vec<_> = OWNED
        .lock()
        .unwrap()
        .iter()
        .map(|(key, client)| (key.clone(), client.clone()))
        .collect();
    let _ = tokio::time::timeout(Duration::from_secs(5), async {
        for ((session, run), mut client) in runs {
            let command =
                crate::agent_commands::get_run_state_command(session.clone(), run.clone());
            let Ok(Ok(response)) =
                tokio::time::timeout(Duration::from_secs(1), client.execute_command(command)).await
            else {
                continue;
            };
            let response = response.into_inner();
            if response.success
                && owned_run_is_terminal(&future_rpc::decode::response_data(&response), &run)
            {
                release_run(&session, &run);
            }
        }
    })
    .await;
}

fn owned_run_is_terminal(state: &serde_json::Value, run: &str) -> bool {
    // Active/queued evidence takes precedence over terminal snapshots.
    if state
        .pointer("/activeRun/runId")
        .and_then(serde_json::Value::as_str)
        == Some(run)
        || state
            .get("queuedRuns")
            .and_then(serde_json::Value::as_array)
            .is_some_and(|runs| {
                runs.iter().any(|entry| {
                    entry.get("runId").and_then(serde_json::Value::as_str) == Some(run)
                })
            })
    {
        return false;
    }
    state
        .pointer("/interruptedRun/runId")
        .and_then(serde_json::Value::as_str)
        == Some(run)
        || (state
            .pointer("/requestedRun/runId")
            .and_then(serde_json::Value::as_str)
            == Some(run)
            && matches!(
                state
                    .pointer("/requestedRun/status")
                    .and_then(serde_json::Value::as_str),
                Some("completed" | "cancelled" | "error" | "failed")
            ))
}

/// Cancellation acknowledgments are not proof of completion. Retain unknown
/// runs and return their exact identities to the final installation barrier.
pub async fn stop_owned_runs(budget: Duration) -> Vec<String> {
    use futures::{stream, StreamExt};
    let runs: Vec<_> = OWNED
        .lock()
        .unwrap()
        .iter()
        .map(|(key, client)| (key.clone(), client.clone()))
        .collect();
    let deadline = tokio::time::Instant::now() + budget;
    stream::iter(runs)
        .map(|((session, run), mut client)| async move {
            let mut last_error = "no verified terminal state".to_owned();
            let stopped = tokio::time::timeout_at(deadline, async {
                loop {
                    // Both commands are exact-run fenced. A run_not_queued reply
                    // is normal if the queued run became active or already ended.
                    for kind in ["cancel_queued_run", "abort", "get_state"] {
                        let command = future_rpc::proto::RpcCommand {
                            r#type: kind.into(),
                            session_id: session.clone(),
                            run_id: run.clone(),
                            ..Default::default()
                        };
                        match tokio::time::timeout(
                            Duration::from_millis(500),
                            client.execute_command(command),
                        )
                        .await
                        {
                            Ok(Ok(response)) => {
                                let response = response.into_inner();
                                if kind == "get_state"
                                    && response.success
                                    && owned_run_is_terminal(
                                        &future_rpc::decode::response_data(&response),
                                        &run,
                                    )
                                {
                                    return;
                                }
                                if !response.success {
                                    last_error = response.error;
                                } else if kind == "get_state" {
                                    last_error =
                                        "get_state did not confirm the exact run terminal".into();
                                }
                            }
                            Ok(Err(error)) => last_error = error.to_string(),
                            Err(_) => last_error = format!("{kind} timed out"),
                        }
                    }
                    tokio::time::sleep(Duration::from_millis(50)).await;
                }
            })
            .await;
            if stopped.is_ok() {
                release_run(&session, &run);
                None
            } else {
                Some(format!(
                    "Desktop run {session}/{run} could not be confirmed stopped: {last_error}"
                ))
            }
        })
        .buffer_unordered(8)
        .filter_map(async |error| error)
        .collect()
        .await
}

pub fn close_admission() {
    ADMISSION.lock().unwrap().closed = true;
}
pub fn active_callers() -> usize {
    ADMISSION.lock().unwrap().active
}
pub fn clear_owned_runs() {
    OWNED.lock().unwrap().clear();
}
pub fn wait_for_callers(deadline: Instant) {
    let mut admission = ADMISSION.lock().unwrap();
    admission.closed = true;
    while admission.active != 0 {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            break;
        }
        admission = CHANGED.wait_timeout(admission, remaining).unwrap().0;
    }
}
#[cfg(any(test, feature = "test-support"))]
pub fn reset_for_test() {
    // Live guards from read-only RPC fixtures must keep their caller count.
    ADMISSION.lock().unwrap().closed = false;
    clear_owned_runs();
}
/// Test-only RAII guard that closes admission for as long as it lives and hands the
/// flag back on drop.
///
/// A bare setter would leave a fixture responsible for restoring process-global
/// state on every path, including a panicking assertion. `reset_for_test` would
/// also restore it, but it additionally stops tasks and clears the publishable
/// stop, which is not what a fixture that merely borrowed the flag should do to
/// the tests running beside it. This guard touches exactly the bit it changed,
/// and only when it changed it: a process that was already stopping stays
/// stopping.
#[cfg(any(test, feature = "test-support"))]
pub struct AdmissionClosedForTest {
    was_closed: bool,
}

#[cfg(any(test, feature = "test-support"))]
impl Drop for AdmissionClosedForTest {
    fn drop(&mut self) {
        if !self.was_closed {
            ADMISSION.lock().unwrap().closed = false;
        }
    }
}

#[cfg(any(test, feature = "test-support"))]
pub fn close_admission_guard_for_test() -> AdmissionClosedForTest {
    let mut admission = ADMISSION.lock().unwrap();
    let was_closed = admission.closed;
    admission.closed = true;
    AdmissionClosedForTest { was_closed }
}

#[cfg(test)]
mod tests {
    #[test]
    fn ownership_requires_positive_terminal_evidence_and_preserves_active_or_queued_work() {
        use serde_json::json;
        assert!(!super::owned_run_is_terminal(&json!({}), "run"));
        let terminal = json!({"requestedRun":{"runId":"run","status":"completed"}});
        assert!(super::owned_run_is_terminal(&terminal, "run"));
        assert!(!super::owned_run_is_terminal(
            &json!({"activeRun":{"runId":"run"},"requestedRun":{"runId":"run","status":"completed"}}),
            "run"
        ));
        assert!(!super::owned_run_is_terminal(
            &json!({"queuedRuns":[{"runId":"run"}],"requestedRun":{"runId":"run","status":"completed"}}),
            "run"
        ));
        assert!(!super::owned_run_is_terminal(
            &json!({"requestedRun":{"runId":"run","status":"unknown"}}),
            "run"
        ));
        assert!(super::owned_run_is_terminal(
            &json!({"interruptedRun":{"runId":"run"}}),
            "run"
        ));
    }
}
