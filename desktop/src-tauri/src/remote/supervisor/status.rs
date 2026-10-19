//! Desktop status projection and optional validation retry use the same instance.
use super::*;
pub fn status() -> RemoteStatus {
    SUPERVISOR.status()
}
