//! Desktop supplies the cleanup runtime and GUI follow-up actions.
#[cfg(feature = "gui")]
pub(crate) use future_host::shutdown::snapshot;
pub(crate) use future_host::shutdown::Operation;
#[cfg(feature = "gui")]
pub(crate) use future_host::shutdown::StopSnapshot;
use std::sync::Arc;
#[cfg(feature = "gui")]
use std::sync::Mutex;

pub(crate) fn begin(reason: &'static str) -> Arc<Operation> {
    begin_with(reason, || {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
    })
}

pub(super) fn begin_with(
    reason: &'static str,
    build: impl FnOnce() -> std::io::Result<tokio::runtime::Runtime> + Send + 'static,
) -> Arc<Operation> {
    future_host::shutdown::begin(reason, move |operation| {
        super::perform_stop(reason, build, operation).map_err(|error| error.to_string())
    })
}

#[cfg(test)]
pub(super) use future_host::shutdown::reset_for_test;

// Serialize destructive follow-up actions after the common cleanup barrier.
// GUI close requests must not exit between cleanup and installer handoff.
#[cfg(feature = "gui")]
static ACTION: Mutex<Option<&'static str>> = Mutex::new(None);
#[cfg(feature = "gui")]
pub(crate) struct ActionGuard;
#[cfg(feature = "gui")]
impl Drop for ActionGuard {
    fn drop(&mut self) {
        *ACTION.lock().unwrap() = None;
    }
}
#[cfg(feature = "gui")]
pub(crate) fn claim_action(reason: &'static str) -> Result<ActionGuard, crate::AppError> {
    let mut action = ACTION.lock().unwrap();
    if let Some(active) = *action {
        return Err(format!("Desktop {active} is already in progress").into());
    }
    if let Some(operation) = snapshot() {
        if matches!(operation.reason.as_str(), "quit" | "exit" | "headless_exit")
            || reason == "update"
        {
            return Err(
                "Desktop is already stopping; cannot begin another maintenance action".into(),
            );
        }
    }
    *action = Some(reason);
    Ok(ActionGuard)
}
#[cfg(all(feature = "gui", test))]
pub(crate) fn maintenance_active() -> bool {
    ACTION.lock().unwrap().is_some()
}

#[cfg(feature = "gui")]
pub(crate) fn begin_quit() -> Option<Arc<Operation>> {
    // Same lock order as claim_action: an exit cannot slip between claiming
    // maintenance and establishing its stop operation, or vice versa.
    let action = ACTION.lock().unwrap();
    if action.is_some() && snapshot().is_none_or(|operation| operation.state != "failed") {
        return None;
    }
    Some(begin("quit"))
}

#[cfg(all(test, feature = "gui"))]
mod tests {
    use super::*;

    #[cfg(feature = "gui")]
    #[test]
    fn destructive_actions_are_exclusive_and_release_after_failure() {
        let _home = crate::auth_store::test_support::HomeGuard::new("stop-actions");
        let _mock = crate::agent_bridge::test_support::mock_agent();
        let action = claim_action("update").unwrap();
        assert!(maintenance_active());
        assert!(claim_action("reset").is_err());
        drop(action);
        assert!(!maintenance_active());
        assert!(claim_action("restart").is_ok());
        let operation = begin_quit().unwrap();
        assert!(claim_action("reset").is_err());
        operation.wait().unwrap();
        assert!(claim_action("update").is_err());
    }

    /// A quit cannot begin while another maintenance action owns the barrier, and a
    /// quit that repeats while a stop is already running is *answered* with that stop
    /// instead of starting a second one.
    #[cfg(feature = "gui")]
    #[test]
    fn a_quit_cannot_start_a_second_stop() {
        let _home = crate::auth_store::test_support::HomeGuard::new("stop-second-quit");
        let _mock = crate::agent_bridge::test_support::mock_agent();
        reset_for_test();
        let claimed = claim_action("restart").unwrap();
        assert!(
            begin_quit().is_none(),
            "a quit cannot begin while another maintenance action owns the barrier"
        );
        drop(claimed);
        let operation = begin_quit().expect("the quit begins once the barrier is free");
        let repeated = begin_quit().expect("the repeat quit is answered, not refused");
        assert!(
            Arc::ptr_eq(&operation, &repeated),
            "the second quit must join the running stop instead of starting another"
        );
        operation.wait().unwrap();
        reset_for_test();
    }
}
