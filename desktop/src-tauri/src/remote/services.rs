//! Desktop pairing/file adapters; shared business/state contracts belong to Server.
pub(crate) use future_remote_server::services::{
    BusinessHost, ReplySink, SessionsCatalog, StateHost,
};
use futures::future::BoxFuture;
use serde_json::Value;

use super::protocol::PairingCreds;
type HostResult<T> = Result<T, crate::AppError>;
pub(crate) type Invitation = (PairingCreds, String, Option<i64>);
pub(crate) trait PairingHost: Send + Sync {
    /// The device in control, else the first one paired.
    fn load(&self) -> Option<PairingCreds>;
    /// Every device this machine has paired, in pairing order.
    fn load_slots(&self) -> Vec<PairingCreds>;
    /// Which device drives this machine as persisted; `None` if none does.
    fn active_slot(&self) -> Option<String>;
    fn set_active(&self, pair_id: &str) -> HostResult<()>;
    /// Forget one device, leaving the rest paired.
    fn remove_slot(&self, pair_id: &str) -> HostResult<()>;
    fn save(&self, creds: &PairingCreds) -> HostResult<()>;
    fn clear(&self) -> HostResult<()>;
    /// `keep_existing` decides whether the platform adds a device beside the
    /// existing ones or replaces them all; this side only forwards it.
    fn invite(&self, keep_existing: bool) -> BoxFuture<'_, HostResult<Invitation>>;
    fn refresh(&self, creds: PairingCreds) -> BoxFuture<'_, HostResult<PairingCreds>>;
    fn queue_revoke(&self, creds: &PairingCreds) -> HostResult<()>;
    fn retry_revokes(&self) -> BoxFuture<'_, HostResult<()>>;
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
