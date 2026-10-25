//! Controlled-endpoint runtime status DTOs, independent of an application entry point.
use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum RemotePhase {
    Stopped,
    /// Enabled without pairing a new device or starting an empty bridge.
    Enabled,
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

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteStatus {
    pub agent_available: bool,
    pub phase: RemotePhase,
    pub reason: Option<RemoteFailureReason>,
    pub recovery: Option<RecoveryProgress>,
    pub nats_url: String,
    /// Unix-seconds expiry of the relay grant the bridge is authorized by.
    ///
    /// The credential renews a few minutes before this (`REFRESH_LEAD`), so a
    /// value in the past means the bridge is already unauthorized and can only
    /// come back through a new mint. Reported so a reader — a person on
    /// `future remote status`, or a UI that wants to show the countdown — can tell
    /// "renews at HH:MM" from "lapsed", instead of guessing from a phase that is
    /// only right after the fact. Null when nothing is running, the runtime never
    /// held a grant, or the answering build predates the field.
    pub credential_expires_at: Option<i64>,
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
    /// Invitations this desktop forgot on the platform to make room for
    /// [`RemoteStatus::pairing_code`], by pair id.
    ///
    /// Answered once, by the add-a-device call that had to clear them: the
    /// platform counts an invitation nobody scanned against the per-desktop cap
    /// for minutes, so an `add` may first have to forget its own orphans. Empty
    /// on every other status, including later polls of the same process — it
    /// describes that one request, like the code it made room for.
    pub cleared_invitations: Vec<String>,
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

impl std::fmt::Debug for RemoteStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RemoteStatus")
            .field("phase", &self.phase)
            .field("reason", &self.reason)
            .field("recovery", &self.recovery)
            .field("pair_id", &self.pair_id)
            .field("desktop_id", &self.desktop_id)
            .field("agent_available", &self.agent_available)
            .field("has_pairing_code", &self.pairing_code.is_some())
            .field("warning_code", &self.warning_code)
            .field("client", &self.client)
            .field("devices", &self.devices)
            .finish_non_exhaustive()
    }
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
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteDevice {
    pub pair_id: String,
    pub phase: RemotePhase,
    pub control: DeviceControl,
    /// Transport ready: the bridge is connected and serving this pairing.
    pub connected: bool,
    /// Whether the device's client is at the other end.
    ///
    /// The other half of [`RemoteDevice::connected`], and deliberately kept
    /// apart from it: a bridge can be serving a pairing whose client was
    /// disconnected or force-quit, and reporting only the bridge is what let
    /// the page say "in control" about a device that had left. `false` for an
    /// unclaimed invitation — nobody is behind it yet.
    pub present: bool,
    pub client: Option<ClientIdentity>,
    /// An invitation not yet claimed, for this device's slot.
    pub pairing_code: Option<String>,
    pub pairing_code_expires_at: Option<i64>,
    pub desktop_id: String,
}

impl std::fmt::Debug for RemoteDevice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RemoteDevice")
            .field("pair_id", &self.pair_id)
            .field("desktop_id", &self.desktop_id)
            .field("phase", &self.phase)
            .field("control", &self.control)
            .field("connected", &self.connected)
            .field("present", &self.present)
            .field("client", &self.client)
            .field("has_pairing_code", &self.pairing_code.is_some())
            .finish_non_exhaustive()
    }
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
        credential_expires_at: None,
        pair_id: String::new(),
        pairing_code: None,
        pairing_code_expires_at: None,
        desktop_id: String::new(),
        desktop_public_key: String::new(),
        web_url: None,
        web_lan_url: None,
        agent_available,
        warning_code: None,
        cleared_invitations: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn nested_status_debug_hides_invitations_while_serialization_keeps_them() {
        let mut status = empty(false);
        status.pairing_code = Some("private_top_invitation_fixture".into());
        status.nats_url = "tls://private_transport_fixture@fixture.invalid".into();
        status.web_url = Some("http://fixture.invalid/private_web_fixture".into());
        status.web_lan_url = Some("http://fixture.invalid/private_lan_fixture".into());
        status.devices.push(RemoteDevice {
            pair_id: "pair_fixture".into(),
            desktop_id: "desktop_fixture".into(),
            phase: RemotePhase::Ready,
            control: DeviceControl::Active,
            connected: true,
            present: true,
            client: None,
            pairing_code: Some("private_device_invitation_fixture".into()),
            pairing_code_expires_at: Some(42),
        });
        let debug = format!("{status:?}");
        for marker in [
            "private_top_invitation_fixture",
            "private_device_invitation_fixture",
            "private_transport_fixture",
            "private_web_fixture",
            "private_lan_fixture",
        ] {
            assert!(!debug.contains(marker));
        }
        assert!(debug.contains("pair_fixture"));
        let wire = serde_json::to_value(&status).unwrap();
        assert_eq!(wire["pairingCode"], "private_top_invitation_fixture");
        assert_eq!(
            wire["devices"][0]["pairingCode"],
            "private_device_invitation_fixture"
        );
        assert_eq!(wire["devices"][0]["pairingCodeExpiresAt"], 42);
        assert_eq!(wire["natsUrl"], status.nats_url);
        assert_eq!(wire["webUrl"], status.web_url.unwrap());
        assert_eq!(wire["webLanUrl"], status.web_lan_url.unwrap());
    }

    #[test]
    fn runtime_active_classifies_every_phase() {
        for phase in [
            RemotePhase::Connecting,
            RemotePhase::Ready,
            RemotePhase::Reconnecting,
            RemotePhase::Refreshing,
        ] {
            let status = RemoteStatus {
                phase,
                reason: None,
                ..empty(false)
            };
            assert!(runtime_active(&status), "{phase:?} should be active");
        }
        for phase in [
            RemotePhase::Stopped,
            RemotePhase::Enabled,
            RemotePhase::Failed,
            RemotePhase::Revoked,
        ] {
            let status = RemoteStatus {
                phase,
                reason: None,
                ..empty(false)
            };
            assert!(!runtime_active(&status), "{phase:?} should be inactive");
        }
    }
    #[test]
    fn only_transient_remote_start_failures_arm_background_recovery() {
        let network = RemoteStatus {
            phase: RemotePhase::Reconnecting,
            reason: Some(RemoteFailureReason::Network),
            ..empty(false)
        };
        let server = RemoteStatus {
            phase: RemotePhase::Reconnecting,
            reason: Some(RemoteFailureReason::RemoteServer),
            ..empty(false)
        };
        let revoked = RemoteStatus {
            phase: RemotePhase::Revoked,
            reason: Some(RemoteFailureReason::CredentialRevoked),
            ..empty(false)
        };
        assert!(retryable_start_status(&network));
        assert!(retryable_start_status(&server));
        assert!(!retryable_start_status(&revoked));
        assert!(!retryable_start_status(&RemoteStatus {
            phase: RemotePhase::Ready,
            reason: Some(RemoteFailureReason::Network),
            ..empty(false)
        }));
    }
}
