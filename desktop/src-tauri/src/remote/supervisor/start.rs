//! Desktop entry points forward lifecycle policy to its owned Server.
use super::*;
pub async fn start(_input: RemoteStartInput) -> Result<RemoteStatus, crate::AppError> {
    SUPERVISOR.start().await.map_err(Into::into)
}
pub async fn enable() -> Result<RemoteStatus, crate::AppError> {
    SUPERVISOR.enable().await.map_err(Into::into)
}
pub async fn add_device() -> Result<RemoteStatus, crate::AppError> {
    SUPERVISOR.add_device().await.map_err(Into::into)
}
pub async fn pair_new_device() -> Result<RemoteStatus, crate::AppError> {
    SUPERVISOR.pair_new_device().await.map_err(Into::into)
}
#[cfg(test)]
pub(in crate::remote) async fn start_once(replace: bool) -> Result<RemoteStatus, crate::AppError> {
    SUPERVISOR.start_once(replace).await.map_err(Into::into)
}
#[cfg(test)]
pub(in crate::remote) fn start_failure(
    error: crate::AppError,
) -> Result<RemoteStatus, crate::AppError> {
    SUPERVISOR.start_failure(error.into()).map_err(Into::into)
}

#[cfg(test)]
pub(in crate::remote) fn spawn_runtime_reconnect(pair: String) {
    SUPERVISOR.spawn_runtime_reconnect(pair);
}
