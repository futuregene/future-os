//! NATS chunk framing only; Desktop owns file capabilities and access policy.

use std::time::Duration;
/// Periodic expiry sweep cadence inside the transfer loop. Tests shrink it so
/// the sweep path runs without a one-minute wait.
fn cleanup_tick() -> Duration {
    #[cfg(test)]
    const TICK: Duration = Duration::from_millis(20);
    #[cfg(not(test))]
    const TICK: Duration = Duration::from_secs(60);
    TICK
}

#[cfg(test)]
pub fn spawn_transfer_loop(
    client: async_nats::Client,
    pair_id: String,
    active: std::sync::Arc<std::sync::atomic::AtomicBool>,
) -> tokio::task::JoinHandle<()> {
    spawn_transfer_loop_with_ready(
        client,
        pair_id,
        active,
        super::secure::Transport::legacy_fixture(),
        None,
    )
}

pub(super) fn spawn_transfer_loop_with_ready(
    client: async_nats::Client,
    pair_id: String,
    active: std::sync::Arc<std::sync::atomic::AtomicBool>,
    security: super::secure::Transport,
    ready: Option<tokio::sync::oneshot::Sender<()>>,
) -> tokio::task::JoinHandle<()> {
    let endpoint = future_remote_server::transfer::TransferLoop::new(
        crate::remote_host::owned_host(),
        security.inner,
        active,
        cleanup_tick(),
    );
    tokio::spawn(endpoint.run(client, pair_id, ready))
}

pub(crate) fn clear_transfers() {
    super::host().clear_transfers();
}
pub(crate) fn clear_preview_cache() {
    super::host().clear_preview_cache();
}
