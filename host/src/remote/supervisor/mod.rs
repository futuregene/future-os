use super::*;
#[cfg(test)]
mod re_pair;
mod shutdown;
mod start;
mod state;
mod status;
#[cfg(test)]
pub(crate) use self::re_pair::re_pair;
#[allow(unused_imports)]
pub use self::shutdown::{
    disconnect_device, handle_system_resume, handle_system_suspend, notify_mobile_disconnect, stop,
    stop_gracefully, unpair, unpair_slot,
};
#[cfg(test)]
pub(super) use self::shutdown::{notify_mobile_unpair, stop_slot};
#[allow(unused_imports)]
pub use self::start::{add_device, enable, pair_new_device, start};
#[cfg(test)]
pub(super) use self::start::{spawn_runtime_reconnect, start_failure, start_once};
pub(super) use self::state::SUPERVISOR;
#[cfg(test)]
pub(super) use self::state::{shared_runtime, SingleSlotMap};
pub use self::status::status;
#[cfg(test)]
pub(super) use future_remote_server::supervisor::{
    abort_generation, RemoteState, MAX_RUNTIME_RECONNECT_ATTEMPTS, MAX_WEB_RECONNECT_ATTEMPTS,
};

/// Make `pair_id` the device that drives this machine.
///
/// The device is told on its own channel, and the previous controller is told
/// to stand by, exactly as when the device asks for itself.
///
/// A device with no runtime here cannot take control: there is no bridge to
/// send it commands, so moving the pointer to it would strand the device that
/// *is* connected (it would be told to stand by for nobody). The page hides the
/// action for such a device; this is the same rule kept where the pointer moves.
pub async fn takeover(pair_id: &str) -> Result<RemoteStatus, crate::Error> {
    SUPERVISOR.takeover(pair_id).await.map_err(Into::into)
}

/// Close the Server and await all role work inside the exit coordinator's remaining budget.
pub async fn wait_for_tasks(budget: std::time::Duration) -> Result<(), crate::Error> {
    SUPERVISOR.shutdown(budget).await.map_err(Into::into)
}

#[cfg(test)]
pub(super) async fn establish_slots(
) -> Result<Vec<future_remote_server::supervisor::SlotPlan>, crate::Error> {
    SUPERVISOR.establish_slots().await.map_err(Into::into)
}
#[cfg(test)]
pub(super) fn promote_a_lone_device() {
    SUPERVISOR.promote_a_lone_device();
}
#[cfg(test)]
pub(super) fn status_focused(prefer: Option<&str>) -> RemoteStatus {
    SUPERVISOR.status_focused(prefer)
}
