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
    /// The same snapshot as [`StateHost::sessions`] plus the two signatures a
    /// publisher tells apart, so deciding between a whole snapshot and a
    /// recency packet never costs a second store read.
    fn sessions_catalog(&self, pair: &str) -> Option<SessionsCatalog>;
    fn workspaces(&self) -> Option<(Value, String)>;
    /// `(epoch, sessions revision, workspaces revision)` as last computed.
    /// Must not touch the store: the presence heartbeat calls it every tick.
    fn catalog_revisions(&self) -> (String, u64, u64);
    fn catalog_dirty(&self) -> bool;
    fn agent_available(&self) -> bool;
    fn monitor_agent(&self) -> BoxFuture<'_, ()>;
}

/// A sessions snapshot and its two signatures — see
/// [`crate::remote_host::catalog`]'s module header for why there are two.
pub(crate) struct SessionsCatalog {
    /// What a client receives whole, exactly as [`StateHost::sessions`] answers
    /// it, including the `version` envelope both kinds of packet carry.
    pub(crate) payload: Value,
    /// The rows with their recency field removed. A change here is content a
    /// client cannot patch from a recency packet: a row appeared, went away,
    /// was renamed, started or stopped streaming, was pinned, or moved
    /// workspace.
    pub(crate) structural: String,
    /// The *whole* rows. Either signature moving bumps the revision.
    pub(crate) signature: String,
    /// Every row's `(sessionId, lastMessageAt)`, in catalog order.
    ///
    /// Absolute values, not a diff: a client handed the newest of them is
    /// correct whether or not it saw the ones before, so a lost packet costs
    /// ordering freshness until the next one rather than correctness. Rows are
    /// emitted for every session, not only the moved ones — the publisher is
    /// what filters, against the values *it* last delivered.
    pub(crate) activity: Vec<(String, i64)>,
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
