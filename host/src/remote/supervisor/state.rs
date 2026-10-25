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
        // Built through the type's own setter rather than a struct literal.
        // With `test-support` the struct also carries a private
        // `loopback_fixture` field, so neither a literal nor the
        // `..Default::default()` update syntax is constructible from this crate
        // (`E0451`); without it the struct has one field and the update syntax
        // is `needless_update`. One literal cannot satisfy both, and a blanket
        // `allow` would hide the second case forever.
        //
        // The setter also removes the lint that started this: `let mut x =
        // Default::default(); x.field = …` is `field_reassign_with_default`, and
        // the crate's own `test-support` feature was hiding it — with that
        // feature off, `-p future-host --lib` saw the lint while the
        // workspace-wide pass, where a dev-dependency enables the feature for
        // the whole graph, did not.
        let options = future_remote_server::connection::ConnectionOptions::default()
            .with_root_bundle(trust::root_bundle());
        #[cfg(any(test, feature = "test-support"))]
        let options = if crate::remote::fixture_mode() {
            options.for_loopback_fixture()
        } else {
            options
        };
        options
    }
    fn attach_events(&self) {
        crate::remote_host::attach_events();
    }
    fn options(&self) -> RuntimeOptions {
        #[cfg(any(test, feature = "test-support"))]
        {
            if !crate::remote::fixture_mode() {
                return RuntimeOptions::default();
            }
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
                // Production's own value, unless a test raised the switch: the
                // recovery path is what some tests are about, and it has no other
                // way to run here.
                automatic_recovery: crate::remote::SUPERVISOR
                    .hooks
                    .recovery_enabled
                    .load(std::sync::atomic::Ordering::Acquire),
                revoke_cleanup: false,
            }
        }
        #[cfg(not(any(test, feature = "test-support")))]
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
            let (listener, port) = bind_web_listener().await?;
            Ok(ValidationSurface {
                task: Box::pin(run_web_server(listener, port)),
                url: format!("http://localhost:{port}"),
                lan_url: lan_ip().map(|ip| format!("http://{ip}:{port}")),
            })
        })
    }
    fn gzip_override(&self) -> bool {
        commands::operator_gzip_enabled()
    }
    fn is_stopping(&self) -> bool {
        crate::request_scope::is_stopping()
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
