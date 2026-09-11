//! Host contracts use owned command DTOs and asynchronous replies, never NATS or UI handles.
use super::protocol::IncomingCmd;
use futures::future::BoxFuture;
use serde_json::Value;

pub(crate) trait ReplySink: Send + Sync {
    fn send<'a>(&'a self, success: bool, data: Value, error: Option<String>) -> BoxFuture<'a, ()>;
}
pub(crate) trait BusinessHost: Send + Sync {
    fn execute<'a>(&'a self, command: IncomingCmd, reply: &'a dyn ReplySink) -> BoxFuture<'a, ()>;
}

use super::protocol::PairingCreds;
type HostResult<T> = Result<T, crate::AppError>;
pub(crate) type Invitation = (PairingCreds, String, Option<i64>);
pub(crate) trait PairingHost: Send + Sync {
    fn load(&self) -> Option<PairingCreds>;
    fn save(&self, creds: &PairingCreds) -> HostResult<()>;
    fn clear(&self) -> HostResult<()>;
    fn invite(&self) -> BoxFuture<'_, HostResult<Invitation>>;
    fn refresh(&self, creds: PairingCreds) -> BoxFuture<'_, HostResult<PairingCreds>>;
    fn queue_revoke(&self, creds: &PairingCreds) -> HostResult<()>;
    fn retry_revokes(&self) -> BoxFuture<'_, HostResult<()>>;
}
pub(crate) trait StateHost: Send + Sync {
    fn catalog_epoch(&self) -> String;
    fn sessions(&self, pair: &str) -> Option<(Value, String)>;
    fn workspaces(&self) -> Option<(Value, String)>;
    fn catalog_dirty(&self) -> bool;
    fn agent_available(&self) -> bool;
    fn monitor_agent(&self) -> BoxFuture<'_, ()>;
}
pub(crate) trait FileHost: Send + Sync {
    fn upload_chunk(&self, transfer: &str, index: u64, bytes: &[u8]) -> Result<Value, String>;
    fn download_chunk(&self, transfer: &str, index: u64) -> HostResult<Vec<u8>>;
    fn prune_transfers(&self);
    fn clear_transfers(&self);
    fn clear_preview_cache(&self);
}
pub(crate) trait RemoteHost: BusinessHost + PairingHost + StateHost + FileHost {}
impl<T: BusinessHost + PairingHost + StateHost + FileHost> RemoteHost for T {}
