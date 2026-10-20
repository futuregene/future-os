//! Stop Desktop producers before draining the Agent. External ownership is
//! preserved; only prompts submitted by this Desktop receive exact-run aborts.
use std::time::Duration;

mod coordinator;
pub(crate) use coordinator::begin;
#[cfg(feature = "gui")]
pub(crate) use coordinator::{begin_quit, claim_action};
#[cfg(feature = "gui")]
pub(crate) use coordinator::{snapshot, StopSnapshot};

#[cfg(test)]
use future_host::request_scope::close_admission;
#[cfg(any(feature = "gui", test))]
pub(crate) use future_host::request_scope::is_stopping;

#[cfg(feature = "gui")]
pub(crate) use future_host::request_scope::owned_count;

#[cfg(test)]
pub(crate) use future_host::request_scope::admit;

pub(crate) fn stop(reason: &'static str) -> Result<(), crate::AppError> {
    begin(reason).wait().map_err(Into::into)
}
fn perform_stop(
    reason: &'static str,
    build: impl FnOnce() -> std::io::Result<tokio::runtime::Runtime>,
    operation: &coordinator::Operation,
) -> Result<(), crate::AppError> {
    let mut services = future_host::shutdown::ServiceStop::prepare(operation, reason, || {
        #[cfg(all(feature = "gui", not(test)))]
        if let Some(manager) = crate::terminal::server::manager() {
            manager.freeze();
        }
    })?;
    let cleanup_runtime = match build() {
        Ok(runtime) => Some(runtime),
        Err(error) => {
            services.runtime_unavailable(error);
            None
        }
    };
    if let Some(runtime) = &cleanup_runtime {
        runtime.block_on(services.disconnect());
    }
    #[cfg(all(feature = "gui", not(test)))]
    if let Some(manager) = crate::terminal::server::manager() {
        operation.stage("terminals");
        if let Err(error) =
            manager.shutdown_checked(operation.remaining(crate::terminal::manager::CLOSE_GRACE))
        {
            services.record_error(error);
        }
    }
    if let Some(runtime) = cleanup_runtime {
        runtime.shutdown_timeout(operation.remaining(Duration::from_secs(1)));
    }
    services.finish()
}

#[cfg(all(test, feature = "gui"))]
pub(crate) fn fail_for_test(error: &str) {
    future_host::shutdown::record_failure(error.to_owned());
}

#[cfg(test)]
pub(crate) fn reset_for_test() {
    coordinator::reset_for_test();
    future_host::request_scope::reset_for_test();
    future_host::shutdown::clear_failure_for_test();
    crate::tasks::reset_for_test();
    future_host::remote_peer::runtime::reset_after_shutdown_for_test();
    crate::remote::reset_after_shutdown_for_test();
}

#[cfg(all(test, feature = "gui"))]
pub(crate) use future_host::request_scope::close_admission_guard_for_test;

#[cfg(test)]
mod tests {
    #[test]
    fn closing_admission_rejects_new_commands_without_losing_active_guards() {
        let _home = crate::auth_store::test_support::HomeGuard::new("shutdown-admission");
        let _mock = crate::agent_bridge::test_support::mock_agent();
        super::reset_for_test();
        let command = super::admit().unwrap();
        super::close_admission();
        assert!(super::admit().is_err());
        drop(command);
        assert!(super::is_stopping());
        // A closed admission with **no** published stop operation is the state
        // the status poll reports as "stopping". `coordinator::begin` sets both
        // globals in one critical section, but they are separate: this fixture
        // closes admission alone, which is the only construction of that state.
        #[cfg(feature = "gui")]
        {
            let status = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("a runtime for the status poll")
                .block_on(crate::agent_supervisor::agent_status());
            assert_eq!(status.phase, "stopping");
            assert!(
                status.shutdown.is_none(),
                "the stopping arm has no stop snapshot: {:?}",
                status.shutdown
            );
        }
        super::reset_for_test();
    }
}
