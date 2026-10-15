//! Stop Desktop producers before draining the Agent. External ownership is
//! preserved; only prompts submitted by this Desktop receive exact-run aborts.
use future_rpc::connection::VerifiedAgentClient;
use std::{
    collections::HashMap,
    sync::{Condvar, LazyLock, Mutex},
    time::{Duration, Instant},
};

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
static STOP_LOCK: Mutex<()> = Mutex::new(());
static STOP_FAILURE: Mutex<Option<String>> = Mutex::new(None);

pub(crate) struct CommandGuard;
impl Drop for CommandGuard {
    fn drop(&mut self) {
        ADMISSION.lock().unwrap().active -= 1;
        CHANGED.notify_all();
    }
}
#[allow(clippy::result_large_err)] // Tonic callers preserve native Status errors.
pub(crate) fn admit() -> Result<CommandGuard, tonic::Status> {
    let mut admission = ADMISSION.lock().unwrap();
    if admission.closed {
        return Err(tonic::Status::unavailable("Desktop is stopping"));
    }
    admission.active += 1;
    Ok(CommandGuard)
}
pub(crate) fn is_stopping() -> bool {
    ADMISSION.lock().unwrap().closed
}
pub(crate) fn own_run(session: String, run: String, client: VerifiedAgentClient) {
    if !session.is_empty() && !run.is_empty() {
        OWNED.lock().unwrap().insert((session, run), client);
    }
}
pub(crate) fn release_run(session: &str, run: &str) {
    OWNED.lock().unwrap().remove(&(session.into(), run.into()));
}
pub(crate) fn owned_count() -> usize {
    OWNED.lock().unwrap().len()
}

pub(crate) fn stop(reason: &'static str) -> Result<(), crate::AppError> {
    stop_with_runtime(reason, || {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
    })
}
fn stop_with_runtime(
    reason: &'static str,
    build: impl FnOnce() -> std::io::Result<tokio::runtime::Runtime>,
) -> Result<(), crate::AppError> {
    let _serial = STOP_LOCK.lock().unwrap();
    if let Some(error) = STOP_FAILURE.lock().unwrap().clone() {
        return Err(error.into());
    }
    // Cancel the Agent immediately and bound the Desktop caller drain separately.
    let deadline = Instant::now() + Duration::from_secs(3);
    let mut errors = Vec::new();
    ADMISSION.lock().unwrap().closed = true;
    crate::tasks::stop_admission();
    crate::runtime::freeze_tasks();
    #[cfg(all(feature = "gui", not(test)))]
    if let Some(manager) = crate::terminal::server::manager() {
        manager.freeze();
    }
    // Cancel accepted Agent commands while waiting for their Desktop callers;
    // a blocked shell or approval must not prevent global cancellation.
    crate::agent_host::host().request_stop();
    {
        let mut admission = ADMISSION.lock().unwrap();
        admission.closed = true;
        while admission.active != 0 {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                errors.push("Desktop commands did not finish before shutdown".to_owned());
                break;
            }
            admission = CHANGED.wait_timeout(admission, remaining).unwrap().0;
        }
    }
    let cleanup_runtime = match build() {
        Ok(runtime) => Some(runtime),
        Err(error) => {
            errors.push(format!("Desktop cleanup runtime unavailable: {error}"));
            crate::remote::stop();
            None
        }
    };
    if let Some(runtime) = &cleanup_runtime {
        runtime.block_on(async {
            crate::remote::stop_gracefully(reason).await;
            if !crate::agent_host::host().owns_agent() {
                let runs = std::mem::take(&mut *OWNED.lock().unwrap());
                let _ = tokio::time::timeout(Duration::from_secs(3), async {
                    for ((session, run), mut client) in runs {
                        // The queued run may become active between calls. Both
                        // operations are fenced to this Desktop's exact run.
                        for command in ["cancel_queued_run", "abort"] {
                            let _ = client
                                .execute_command(future_rpc::proto::RpcCommand {
                                    r#type: command.into(),
                                    session_id: session.clone(),
                                    run_id: run.clone(),
                                    ..Default::default()
                                })
                                .await;
                        }
                    }
                })
                .await;
            }
        });
    }
    #[cfg(all(feature = "gui", not(test)))]
    if let Some(manager) = crate::terminal::server::manager() {
        if let Err(error) = manager.shutdown_checked(crate::terminal::manager::CLOSE_GRACE) {
            errors.push(error);
        }
    }
    if let Some(runtime) = cleanup_runtime {
        runtime.shutdown_timeout(Duration::from_secs(1));
    }
    // Stop waits until the native service scope has released storage and lock.
    if let Err(error) = crate::agent_host::host().stop() {
        errors.push(error.to_string());
    }
    if let Err(error) = crate::runtime::cancel_all_tasks(Duration::from_secs(5)) {
        errors.push(error.to_string());
    }
    OWNED.lock().unwrap().clear();
    if errors.is_empty() {
        Ok(())
    } else {
        let error = errors.join("; ");
        *STOP_FAILURE.lock().unwrap() = Some(error.clone());
        Err(error.into())
    }
}

#[cfg(all(test, feature = "gui"))]
pub(crate) fn fail_for_test(error: &str) {
    *STOP_FAILURE.lock().unwrap() = Some(error.to_owned());
}

#[cfg(test)]
pub(crate) fn reset_for_test() {
    let mut admission = ADMISSION.lock().unwrap();
    // Some read-only mock RPC tests intentionally share no HOME fixture.
    // Preserve their live guards rather than resetting the counter.
    admission.closed = false;
    *STOP_FAILURE.lock().unwrap() = None;
    OWNED.lock().unwrap().clear();
    crate::tasks::reset_for_test();
}

#[cfg(test)]
mod tests {
    #[test]
    fn runtime_failure_still_drops_background_work_and_blocks_retry() {
        let _home = crate::auth_store::test_support::HomeGuard::new("shutdown-runtime-failure");
        let _mock = crate::agent_bridge::test_support::mock_agent();
        struct Done(std::sync::mpsc::Sender<()>);
        impl Drop for Done {
            fn drop(&mut self) {
                let _ = self.0.send(());
            }
        }
        let (tx, completed) = std::sync::mpsc::channel();
        let done = Done(tx);
        let _task = crate::runtime::spawn(async move {
            let _done = done;
            std::future::pending::<()>().await;
        });
        let error = super::stop_with_runtime("fixture", || {
            Err(std::io::Error::other("injected runtime failure"))
        })
        .unwrap_err();
        assert!(error.to_string().contains("injected runtime failure"));
        completed
            .recv_timeout(std::time::Duration::from_secs(1))
            .unwrap();
        assert!(super::stop("fixture").is_err());
        assert!(super::is_stopping());
    }

    #[test]
    fn closing_admission_rejects_new_commands_without_losing_active_guards() {
        let _home = crate::auth_store::test_support::HomeGuard::new("shutdown-admission");
        let _mock = crate::agent_bridge::test_support::mock_agent();
        let command = super::admit().unwrap();
        super::ADMISSION.lock().unwrap().closed = true;
        assert!(super::admit().is_err());
        drop(command);
        assert!(super::is_stopping());
    }
}
