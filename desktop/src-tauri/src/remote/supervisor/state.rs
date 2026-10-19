//! Desktop owns this one Server supervisor; the library has no global instance.
use super::*;
#[cfg(test)]
pub(in crate::remote) use future_remote_server::supervisor::SingleSlotMap;
#[cfg(not(test))]
pub(in crate::remote) use future_remote_server::supervisor::RUNTIME_HEALTHY_RESET_SECS;
pub(in crate::remote) use future_remote_server::supervisor::{
    BridgeRuntimeShared, ConnectedNats, RemoteState, Supervisor, MAX_RUNTIME_RECONNECT_ATTEMPTS,
    MAX_WEB_RECONNECT_ATTEMPTS, RUNTIME_FAILURE_WINDOW_MS,
};
struct DesktopExecutor;
impl future_remote_server::supervisor::TaskExecutor for DesktopExecutor {
    fn spawn(&self, future: futures::future::BoxFuture<'static, ()>) {
        crate::runtime::spawn(future);
    }
    fn spawn_native(&self, job: Box<dyn FnOnce() + Send + 'static>) {
        crate::runtime::spawn_native(job);
    }
}
pub(in crate::remote) static SUPERVISOR: LazyLock<Arc<Supervisor>> =
    LazyLock::new(|| Supervisor::new(crate::remote_host::owned_host(), Arc::new(DesktopExecutor)));
pub(in crate::remote) fn shared_runtime(
    creds: &pairing::PairingCreds,
    confirmed: bool,
    rotate: bool,
) -> BridgeRuntimeShared {
    SUPERVISOR.shared_runtime(creds, confirmed, rotate)
}
