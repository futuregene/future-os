//! Test adapters into the instance-owned transport implementation.
use super::*;
#[cfg(test)]
pub(super) async fn build_transport(
    client: &async_nats::Client,
    pair: &str,
    handshake: &commands::HandshakeState,
    slots: commands::ReplySlots,
) -> Result<future_remote_server::supervisor::TransportTasks, crate::AppError> {
    SUPERVISOR
        .build_transport(client, pair, handshake, slots)
        .await
        .map_err(Into::into)
}
#[cfg(test)]
pub(super) fn spawn_credential_refresh(
    pair: String,
    slots: commands::ReplySlots,
    confirmed: Arc<AtomicBool>,
    handshake: commands::HandshakeState,
) -> future_remote_server::supervisor::TaskHandle {
    SUPERVISOR.spawn_credential_refresh(pair, slots, confirmed, handshake, None)
}
