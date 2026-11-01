//! Desktop OS/command adapters for Server lifecycle.
use super::*;
pub async fn unpair() -> Result<RemoteStatus, crate::Error> {
    SUPERVISOR.unpair().await.map_err(Into::into)
}
pub async fn unpair_slot(pair: &str) -> Result<RemoteStatus, crate::Error> {
    SUPERVISOR.unpair_slot(pair).await.map_err(Into::into)
}
pub async fn disconnect_device(pair: &str) -> Result<RemoteStatus, crate::Error> {
    SUPERVISOR.disconnect_device(pair).await.map_err(Into::into)
}
pub fn stop() -> RemoteStatus {
    SUPERVISOR.stop()
}
pub async fn stop_gracefully(reason: &str) -> RemoteStatus {
    let status = SUPERVISOR.stop_gracefully(reason).await;
    future_remote_protocol::remote_log!(
        "remote: stopped (reason={reason}, phase={:?}, failure={:?})",
        status.phase,
        status.reason
    );
    status
}
pub async fn notify_mobile_disconnect(reason: &str) {
    SUPERVISOR.notify_mobile_disconnect(reason).await
}
pub async fn handle_system_suspend() {
    SUPERVISOR.handle_system_suspend().await
}
pub fn handle_system_resume() {
    SUPERVISOR.handle_system_resume()
}
#[cfg(test)]
pub(in crate::remote) fn stop_slot(pair: &str) {
    SUPERVISOR.stop_slot(pair);
}
#[cfg(test)]
pub(in crate::remote) async fn notify_mobile_unpair() {
    for (client, subject, payload) in SUPERVISOR.mobile_unpair_notices() {
        future_remote_server::supervisor::send_mobile_disconnect_notice(client, subject, payload)
            .await;
    }
}
