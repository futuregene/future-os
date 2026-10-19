//! Explicit business and state services supplied by a controlled-endpoint host.
//! Interfaces carry owned wire values and async replies; no UI or application handles.
use future_remote_protocol::IncomingCmd;
use futures::future::BoxFuture;
use serde_json::Value;

pub trait ReplySink: Send + Sync {
    fn send<'a>(&'a self, success: bool, data: Value, error: Option<String>) -> BoxFuture<'a, ()>;
}
pub trait BusinessHost: Send + Sync {
    fn execute<'a>(&'a self, command: IncomingCmd, reply: &'a dyn ReplySink) -> BoxFuture<'a, ()>;
}

pub trait StateHost: Send + Sync {
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

/// A sessions snapshot plus structural/content signatures used to choose
/// full snapshots or absolute activity updates without reading the store twice.
pub struct SessionsCatalog {
    /// What a client receives whole, exactly as [`StateHost::sessions`] answers
    /// it, including the `version` envelope both kinds of packet carry.
    pub payload: Value,
    /// The rows with their recency field removed. A change here is content a
    /// client cannot patch from a recency packet: a row appeared, went away,
    /// was renamed, started or stopped streaming, was pinned, or moved
    /// workspace.
    pub structural: String,
    /// The *whole* rows. Either signature moving bumps the revision.
    pub signature: String,
    /// Every row's `(sessionId, lastMessageAt)`, in catalog order.
    ///
    /// Absolute values, not a diff: a client handed the newest of them is
    /// correct whether or not it saw the ones before, so a lost packet costs
    /// ordering freshness until the next one rather than correctness. Rows are
    /// emitted for every session, not only the moved ones — the publisher is
    /// what filters, against the values *it* last delivered.
    pub activity: Vec<(String, i64)>,
}
