//! Desktop OS/command adapters for Server lifecycle.
use super::*;

/// The machine-wide teardown half of this module is where an unsupervised test
/// does its damage: a `stop` that lands while another test's `start` is in
/// flight cancels it, and the failure is reported in *that* test's name. In a
/// test build every such write therefore has to come from inside a fixture
/// scope; production has no such notion and no such assertion.
fn assert_supervised(what: &str) {
    #[cfg(test)]
    crate::remote::test_support::assert_fixture_scope(what);
    #[cfg(not(test))]
    let _ = what;
}

pub async fn unpair() -> Result<RemoteStatus, crate::Error> {
    assert_supervised("`remote unpair`");
    SUPERVISOR.unpair().await.map_err(Into::into)
}
pub async fn unpair_slot(pair: &str) -> Result<RemoteStatus, crate::Error> {
    SUPERVISOR.unpair_slot(pair).await.map_err(Into::into)
}
pub async fn disconnect_device(pair: &str) -> Result<RemoteStatus, crate::Error> {
    SUPERVISOR.disconnect_device(pair).await.map_err(Into::into)
}
pub fn stop() -> RemoteStatus {
    assert_supervised("`remote stop`");
    SUPERVISOR.stop()
}
pub async fn stop_gracefully(reason: &str) -> RemoteStatus {
    assert_supervised("a graceful remote stop");
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
    assert_supervised("a system suspend");
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
