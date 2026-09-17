use super::*;

mod shutdown;
mod start;
mod state;
mod status;

#[cfg(test)]
pub(super) use self::shutdown::notify_mobile_unpair;
#[allow(unused_imports)]
pub(super) use self::shutdown::{
    abort_generation, disable_handshake, mobile_disconnect_notice, mobile_unpair_notice,
    power_transition, send_mobile_disconnect_notice, stop_runtime, stop_with, PowerEvent,
    PowerTransition,
};
#[allow(unused_imports)]
pub use self::shutdown::{
    handle_system_resume, handle_system_suspend, notify_mobile_disconnect, stop, stop_gracefully,
    unpair,
};
pub use self::start::start;
#[allow(unused_imports)]
pub(super) use self::start::{
    classify_nats_connect_error, connect_nats, establish, record_runtime_failure,
    spawn_revoke_cleanup, spawn_runtime_reconnect, spawn_runtime_supervisor, spawn_start_retry,
    spawn_web_reconnect, start_failure, start_generation, start_once,
};
#[cfg(not(test))]
pub(super) use self::state::RUNTIME_HEALTHY_RESET_SECS;
#[allow(unused_imports)]
pub(super) use self::state::{
    shared_runtime, BridgeRuntimeShared, ConnectedNats, RemoteState, Supervisor,
    MAX_RUNTIME_RECONNECT_ATTEMPTS, MAX_WEB_RECONNECT_ATTEMPTS, RUNTIME_FAILURE_WINDOW_MS,
    SUPERVISOR,
};
pub use self::status::status;
