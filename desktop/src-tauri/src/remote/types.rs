use super::*;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteStartInput {}

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
    pub client: Option<ClientIdentity>,
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

pub(super) fn retryable_start_status(status: &RemoteStatus) -> bool {
    let reconnecting = matches!(status.phase, RemotePhase::Reconnecting);
    let retryable_reason = matches!(
        status.reason,
        Some(RemoteFailureReason::Network | RemoteFailureReason::RemoteServer)
    );
    reconnecting && retryable_reason
}

pub(super) fn runtime_active(status: &RemoteStatus) -> bool {
    matches!(
        status.phase,
        RemotePhase::Connecting
            | RemotePhase::Ready
            | RemotePhase::Reconnecting
            | RemotePhase::Refreshing
    )
}

pub(super) fn empty() -> RemoteStatus {
    RemoteStatus {
        client: None,
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
        agent_available: host().agent_available(),
        warning_code: None,
    }
}
