//! Desktop entry points forward lifecycle policy to its owned Server.
use super::*;
pub async fn start(_input: RemoteStartInput) -> Result<RemoteStatus, crate::Error> {
    let status = SUPERVISOR.start().await.map_err(crate::Error::from);
    lifecycle("serving", &status);
    status
}
pub async fn enable() -> Result<RemoteStatus, crate::Error> {
    let status = SUPERVISOR.enable().await.map_err(crate::Error::from);
    lifecycle("enabled", &status);
    status
}
/// One line per switch decision, in the file a later investigation reads.
///
/// The bridge that stops serving is the question a user asks after the fact
/// ("the remote desktop died and the switch was off"), and until this existed
/// the answer lived on a stderr a windowless launch cannot write to.
fn lifecycle(action: &str, status: &Result<RemoteStatus, crate::Error>) {
    match status {
        Ok(status) => tracing::info!(event = "lifecycle", action = %action,
            "remote: {action} (phase={:?}, reason={:?})",
            status.phase,
            status.reason
        ),
        Err(error) => {
            tracing::error!(event = "lifecycle", action = %action, error = %error,"remote: {action} failed (local fault): {error}")
        }
    }
}
pub async fn add_device() -> Result<RemoteStatus, crate::Error> {
    match SUPERVISOR.add_device().await {
        Ok(status) => Ok(status),
        Err(error) => {
            // The GUI shows a classified sentence, so the cause itself has to
            // reach the component log so a windowed launch also retains the
            // evidence behind an "add failed" report.
            tracing::error!(event = "add_device", error = %error,"remote: add device failed: {error}");
            Err(error.into())
        }
    }
}
pub async fn pair_new_device() -> Result<RemoteStatus, crate::Error> {
    SUPERVISOR.pair_new_device().await.map_err(Into::into)
}
#[cfg(test)]
pub(in crate::remote) async fn start_once(replace: bool) -> Result<RemoteStatus, crate::Error> {
    SUPERVISOR.start_once(replace).await.map_err(Into::into)
}
#[cfg(test)]
pub(in crate::remote) fn start_failure(error: crate::Error) -> Result<RemoteStatus, crate::Error> {
    SUPERVISOR.start_failure(error.into()).map_err(Into::into)
}

#[cfg(test)]
pub(in crate::remote) fn spawn_runtime_reconnect(pair: String, generation_id: u64) {
    SUPERVISOR.spawn_runtime_reconnect(pair, generation_id);
}
