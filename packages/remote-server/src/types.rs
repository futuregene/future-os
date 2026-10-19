//! Controlled-endpoint runtime status DTOs, independent of an application entry point.
use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum RemotePhase {
    Stopped,
    Connecting,
    Ready,
    Reconnecting,
    Refreshing,
    Failed,
    Revoked,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RemoteFailureReason {
    Network,
    SystemSleep,
    CredentialExpired,
    CredentialRevoked,
    AccountAuthorization,
    ServiceAuthorization,
    RemoteServer,
    Protocol,
    GenerationUnhealthy,
    Local,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecoveryProgress {
    pub attempt: u64,
    pub max_attempts: Option<u64>,
    pub since: u64,
    pub next_retry_at: Option<u64>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteStatus {
    pub agent_available: bool,
    pub phase: RemotePhase,
    pub reason: Option<RemoteFailureReason>,
    pub recovery: Option<RecoveryProgress>,
    pub nats_url: String,
    pub pair_id: String,
    /// One-shot pairing code (base64url) returned only by a successful start, for the UI to display/copy.
    pub pairing_code: Option<String>,
    /// Unix-seconds expiry of `pairing_code` (for the UI countdown); `None`
    /// when there's no code.
    pub pairing_code_expires_at: Option<i64>,
    /// Desktop identity bound into the QR invitation and signed handshake.
    pub desktop_id: String,
    pub desktop_public_key: String,
    /// Test-only web client URL for this machine; `None` outside the test
    /// environment or if the web server failed to bind.
    pub web_url: Option<String>,
    /// Test-only web client URL a phone on the same LAN can reach; `None` if
    /// unavailable or outside the test environment.
    pub web_lan_url: Option<String>,
    /// Non-critical local web listener failure. It never changes the main
    /// Remote phase or readiness.
    pub warning_code: Option<String>,
    /// Who is on the other end, as that client described itself on
    /// `secure_ready`.
    ///
    /// `None` until a client declares it, which an older client never does — so
    /// the UI has to work without it and must not read "unknown" as "nothing is
    /// connected". It exists because the connection stopped being phone-only:
    /// the wording cannot say what the device is unless the device says so.
    ///
    /// On a machine with several devices this describes the one in control; the
    /// full set is [`RemoteStatus::devices`].
    pub client: Option<ClientIdentity>,
    /// Every device this desktop has paired, one entry each.
    ///
    /// The single-device fields above are the projection of the *controlling*
    /// one (or the first device when nothing has claimed control), so a
    /// frontend written before multi-device keeps working. A newer frontend
    /// reads this list to show who is paired, who is online, and who is
    /// driving.
    pub devices: Vec<RemoteDevice>,
}

/// Whether a paired device is the one driving this desktop.
///
/// `Standby` is a device that is still paired and may still be connected — it
/// simply is not allowed to command this machine until its user asks for
/// control back or the other device disconnects. Encoded as a string on the
/// wire so an older client that does not know the field ignores it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DeviceControl {
    Active,
    Standby,
}

/// One paired device, as the Remote page lists it.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteDevice {
    pub pair_id: String,
    pub phase: RemotePhase,
    pub control: DeviceControl,
    /// Transport ready: the bridge is connected and serving this pairing.
    pub connected: bool,
    pub client: Option<ClientIdentity>,
    /// An invitation not yet claimed, for this device's slot.
    pub pairing_code: Option<String>,
    pub pairing_code_expires_at: Option<i64>,
    pub desktop_id: String,
}

/// A client's description of itself, sent on `secure_ready`.
///
/// `name` is the device's own name and `kind` is its category. Both are opaque
/// strings from the client: this side renders them and never branches on them,
/// so a new kind costs nothing here.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClientIdentity {
    pub name: String,
    pub kind: String,
}

pub fn retryable_start_status(status: &RemoteStatus) -> bool {
    let reconnecting = matches!(status.phase, RemotePhase::Reconnecting);
    let retryable_reason = matches!(
        status.reason,
        Some(RemoteFailureReason::Network | RemoteFailureReason::RemoteServer)
    );
    reconnecting && retryable_reason
}

pub fn runtime_active(status: &RemoteStatus) -> bool {
    matches!(
        status.phase,
        RemotePhase::Connecting
            | RemotePhase::Ready
            | RemotePhase::Reconnecting
            | RemotePhase::Refreshing
    )
}

pub fn empty(agent_available: bool) -> RemoteStatus {
    RemoteStatus {
        client: None,
        devices: Vec::new(),
        phase: RemotePhase::Stopped,
        reason: None,
        recovery: None,
        nats_url: String::new(),
        pair_id: String::new(),
        pairing_code: None,
        pairing_code_expires_at: None,
        desktop_id: String::new(),
        desktop_public_key: String::new(),
        web_url: None,
        web_lan_url: None,
        agent_available,
        warning_code: None,
    }
}
