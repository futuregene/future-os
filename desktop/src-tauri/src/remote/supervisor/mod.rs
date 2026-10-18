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
    handle_system_resume, handle_system_suspend, notify_mobile_disconnect, stop, stop_gracefully,
    unpair, unpair_slot,
};
#[allow(unused_imports)]
pub use self::start::add_device;
pub use self::start::start;
#[allow(unused_imports)]
pub(super) use self::start::{
    classify_nats_connect_error, connect_nats, establish_slots, forget_slot, promote_a_lone_device,
    record_runtime_failure, spawn_revoke_cleanup, spawn_runtime_reconnect,
    spawn_runtime_supervisor, spawn_start_retry, spawn_web_reconnect, start_failure,
    start_generation, start_once,
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

/// Make `pair_id` the device that drives this machine.
///
/// The device is told on its own channel, and the previous controller is told
/// to stand by, exactly as when the device asks for itself.
pub async fn takeover(pair_id: &str) -> Result<RemoteStatus, crate::AppError> {
    let previous = SUPERVISOR.claim_control_soon(pair_id);
    if let Some(previous) = previous {
        super::publish_standby(&previous);
    }
    super::publish_control(pair_id, "active");
    Ok(status())
}
