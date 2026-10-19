//! Desktop executes optional validation retry requests returned by Server status.
use super::*;
pub fn status() -> RemoteStatus {
    status_focused(None)
}
pub(in crate::remote) fn status_focused(prefer: Option<&str>) -> RemoteStatus {
    let projection = SUPERVISOR.project_status(prefer, web_client_enabled());
    if let Some(pair) = projection.retry_web {
        spawn_web_reconnect(pair);
    }
    projection.status
}
