use super::*;

mod re_pair;
mod shutdown;
mod start;
mod state;
mod status;

#[allow(unused_imports)]
pub(crate) use self::re_pair::re_pair;

#[cfg(test)]
pub(super) use self::shutdown::notify_mobile_unpair;
#[allow(unused_imports)]
pub(super) use self::shutdown::{
    abort_generation, disable_handshake, mobile_disconnect_notice, mobile_disconnect_notice_for,
    mobile_disconnect_notices, mobile_unpair_notices, power_transition,
    send_mobile_disconnect_notice, stop_runtime, stop_slot, stop_with, PowerEvent, PowerTransition,
};
#[allow(unused_imports)]
pub use self::shutdown::{
    disconnect_device, handle_system_resume, handle_system_suspend, notify_mobile_disconnect, stop,
    stop_gracefully, unpair, unpair_slot,
};
pub use self::start::start;
#[allow(unused_imports)]
pub use self::start::{add_device, pair_new_device};
#[allow(unused_imports)]
pub(super) use self::start::{
    connect_nats, establish_slots, forget_slot, promote_a_lone_device, record_runtime_failure,
    spawn_revoke_cleanup, spawn_runtime_reconnect, spawn_runtime_supervisor, spawn_start_retry,
    spawn_web_reconnect, start_failure, start_generation, start_once,
};
#[cfg(test)]
pub(super) use self::state::SingleSlotMap;
#[cfg(not(test))]
pub(super) use self::state::RUNTIME_HEALTHY_RESET_SECS;
#[allow(unused_imports)]
pub(super) use self::state::{
    shared_runtime, BridgeRuntimeShared, ConnectedNats, RemoteState, Supervisor,
    MAX_RUNTIME_RECONNECT_ATTEMPTS, MAX_WEB_RECONNECT_ATTEMPTS, RUNTIME_FAILURE_WINDOW_MS,
    SUPERVISOR,
};
pub use self::status::status;
#[allow(unused_imports)]
pub(super) use self::status::status_focused;

/// Make `pair_id` the device that drives this machine.
///
/// The device is told on its own channel, and the previous controller is told
/// to stand by, exactly as when the device asks for itself.
///
/// A device with no runtime here cannot take control: there is no bridge to
/// send it commands, so moving the pointer to it would strand the device that
/// *is* connected (it would be told to stand by for nobody). The page hides the
/// action for such a device; this is the same rule kept where the pointer moves.
pub async fn takeover(pair_id: &str) -> Result<RemoteStatus, crate::AppError> {
    if !SUPERVISOR.state.lock().unwrap().contains_key(pair_id) {
        return Err(crate::AppError::Message("device_offline".to_string()));
    }
    let previous = SUPERVISOR.claim_control_soon(pair_id);
    if let Some(previous) = previous {
        super::publish_standby(&previous);
    }
    super::publish_control(pair_id, "active");
    Ok(status())
}

/// Await Server-owned recovery/native work inside the caller's remaining budget.
pub async fn wait_for_tasks(budget: std::time::Duration) -> Result<(), crate::AppError> {
    SUPERVISOR.wait_tasks(budget).await.map_err(Into::into)
}
