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
    /// The desktop-side `takeover` moved control: the windowed desktop raises
    /// its main window so the person at the machine sees the takeover; a
    /// headless host no-ops.
    fn on_control_claimed(&self) {}
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

use crate::PairingCreds;
type HostResult<T> = Result<T, crate::Error>;

/// What a host hands back after minting an invitation.
pub struct Invitation {
    pub creds: PairingCreds,
    /// The complete `futureos://remote/pair` link, as the UI shows it and the
    /// phone scans it.
    pub code: String,
    /// Unix seconds after which `code` no longer works.
    pub expires_at: Option<i64>,
    /// Invitations forgotten on the platform so this one could be minted, by
    /// pair id.
    ///
    /// Only ever non-empty after the platform refused the mint for the
    /// per-desktop cap: an unclaimed invitation counts against that cap for
    /// minutes, so an `add` may have to clear its own orphans first. Every
    /// ordinary mint reports none.
    pub cleared: Vec<String>,
}

/// Printed without the credential or the link.
///
/// A code is a bearer invitation and the credentials behind it open one
/// namespace in the broker; both belong in neither a log line nor a panic
/// message. What a failing test needs to see is the pair id and what the mint
/// cleared.
impl std::fmt::Debug for Invitation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Invitation")
            .field("pair_id", &self.creds.pair_id)
            .field("expires_at", &self.expires_at)
            .field("cleared", &self.cleared)
            .finish_non_exhaustive()
    }
}

pub trait PairingHost: Send + Sync {
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
    ///
    /// `shown` names the pair ids whose invitation this desktop is displaying.
    /// Filling the platform's per-desktop cap with invitations that can never
    /// be claimed is the host's problem to solve, and `shown` is the one thing
    /// it must not solve it by revoking.
    fn invite(
        &self,
        keep_existing: bool,
        shown: &[String],
    ) -> BoxFuture<'_, HostResult<Invitation>>;
    fn refresh(&self, creds: PairingCreds) -> BoxFuture<'_, HostResult<PairingCreds>>;
    fn queue_revoke(&self, creds: &PairingCreds) -> HostResult<()>;
    /// Queue a revoke for an invitation that never became a pairing.
    ///
    /// An invitation nobody claimed has no credential here — it enters the book
    /// only when a device completes the handshake — so `queue_revoke` cannot
    /// describe it. The platform still holds the row it minted, and that row
    /// counts against the per-desktop cap until it expires: several presses of
    /// "add a device" and then "cancel" therefore left the machine unable to add
    /// anything, with nothing on screen to explain it. The pair id is enough to
    /// delete the row, and the delivery is retried like any other revoke.
    fn queue_revoke_invitation(&self, pair_id: &str) -> HostResult<()>;
    fn retry_revokes(&self) -> BoxFuture<'_, HostResult<()>>;
}
pub trait FileHost: Send + Sync {
    fn upload_chunk(&self, transfer: &str, index: u64, bytes: &[u8]) -> Result<Value, String>;
    fn download_chunk(&self, transfer: &str, index: u64) -> HostResult<Vec<u8>>;
    fn prune_transfers(&self);
    fn clear_transfers(&self);
    fn clear_preview_cache(&self);
}
pub trait RemoteHost: BusinessHost + PairingHost + StateHost + FileHost {}
impl<T: BusinessHost + PairingHost + StateHost + FileHost> RemoteHost for T {}
