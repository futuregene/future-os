//! Desktop owns one Server instance and supplies execution/platform adapters.
use super::*;
#[cfg(test)]
pub(in crate::remote) use future_remote_server::supervisor::SingleSlotMap;
use future_remote_server::supervisor::{
    RuntimeOptions, Supervisor, TaskExecutor, ValidationSurface,
};
struct DesktopExecutor;
impl TaskExecutor for DesktopExecutor {
    fn spawn(&self, future: futures::future::BoxFuture<'static, ()>) {
        crate::runtime::spawn(future);
    }
    fn spawn_native(&self, job: Box<dyn FnOnce() + Send + 'static>) {
        crate::runtime::spawn_native(job);
    }
    fn connection_options(&self) -> future_remote_server::connection::ConnectionOptions {
        let mut options = future_remote_server::connection::ConnectionOptions::default();
        options.root_bundle = trust::root_bundle();
        #[cfg(test)]
        let options = options.for_loopback_fixture();
        options
    }
    fn attach_events(&self) {
        crate::remote_host::attach_events();
    }
    fn options(&self) -> RuntimeOptions {
        #[cfg(test)]
        {
            RuntimeOptions {
                publisher: future_remote_server::supervisor::PublisherTiming::new(
                    std::time::Duration::from_millis(10),
                    std::time::Duration::from_millis(10),
                )
                .unwrap(),
                refresh_tick: std::time::Duration::from_millis(10),
                expired_retry: std::time::Duration::from_millis(5),
                transfer_cleanup: std::time::Duration::from_millis(20),
                reply_ttl: std::time::Duration::from_millis(500),
                automatic_recovery: false,
                revoke_cleanup: false,
            }
        }
        #[cfg(not(test))]
        {
            RuntimeOptions::default()
        }
    }
    fn validation_enabled(&self) -> bool {
        web_client_enabled()
    }
    fn bind_validation(
        &self,
    ) -> futures::future::BoxFuture<'_, Result<ValidationSurface, future_remote_server::Error>>
    {
        Box::pin(async {
            let listener = bind_web_listener().await?;
            Ok(ValidationSurface {
                task: Box::pin(run_web_server(listener)),
                url: format!("http://localhost:{WEB_PORT}"),
                lan_url: lan_ip().map(|ip| format!("http://{ip}:{WEB_PORT}")),
            })
        })
    }
    fn gzip_override(&self) -> bool {
        commands::operator_gzip_enabled()
    }
    fn is_stopping(&self) -> bool {
        crate::desktop_shutdown::is_stopping()
    }
}
pub(in crate::remote) static SUPERVISOR: LazyLock<Arc<Supervisor>> =
    LazyLock::new(|| Supervisor::new(crate::remote_host::owned_host(), Arc::new(DesktopExecutor)));
#[cfg(test)]
pub(in crate::remote) fn shared_runtime(
    creds: &pairing::PairingCreds,
    confirmed: bool,
    rotate: bool,
) -> future_remote_server::supervisor::BridgeRuntimeShared {
    SUPERVISOR.shared_runtime(creds, confirmed, rotate)
}
