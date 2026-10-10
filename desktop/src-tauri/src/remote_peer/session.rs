//! The live connection to one remote host: NATS socket, Noise handshake, and
//! the encrypted request/reply used for every command.
//!
//! This is the client mirror of `remote::commands` + `remote::secure`. The
//! shapes are fixed by the host and shared with the phone, so this module is
//! where "the desktop must look exactly like a phone" is enforced:
//!
//! * commands go to `p.{pair}.cmd.{sessionKey}` and are sealed with the subject
//!   as associated data;
//! * a reply is sealed against `reply:{subject}:{header}`, derived from the
//!   request bytes — a reply cannot be moved to another subject or replayed
//!   onto another request;
//! * the handshake pins the host's Noise key **before** any traffic: the key
//!   arrives in the invitation, so the first handshake is authenticated by the
//!   QR itself rather than by trust-on-first-use.
//!
//! Nothing here decides *policy* (when to connect, when to give up, what to do
//! with an event). It performs one connection attempt and exposes it.

use super::creds::PeerCreds;
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use serde_json::{json, Value};
use std::time::Duration;

/// Handshake legs are small and the host expires pending candidates after 30 s;
/// 10 s matches the phone and leaves room for one retry inside that window.
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);
/// Ordinary commands default, matching the phone's request timeout.
pub(crate) const COMMAND_TIMEOUT: Duration = Duration::from_secs(10);

pub(crate) struct PeerSession {
    pair_id: String,
    client: async_nats::Client,
    /// Per-connection traffic keys. Never persisted, never reused across
    /// connections: a new socket always starts from a new handshake.
    ///
    /// Behind a `std` mutex rather than an async one because the guard is never
    /// held across an `await`: sealing and opening are pure CPU, and holding a
    /// lock while awaiting a reply would serialise the event subscription loop
    /// behind a slow command.
    channel: std::sync::Arc<std::sync::Mutex<future_remote_crypto::Channel>>,
    /// The host's bridge instance, from the handshake confirmation. Sent with
    /// every command so a host that restarted mid-conversation can be told
    /// apart from the one the client handshook with.
    bridge_instance_id: String,
    features: Vec<String>,
    presence: Value,
}

impl PeerSession {
    pub(crate) fn pair_id(&self) -> &str {
        &self.pair_id
    }
    pub(crate) fn bridge_instance_id(&self) -> &str {
        &self.bridge_instance_id
    }

    pub(crate) fn features(&self) -> &[String] {
        &self.features
    }

    /// Whether the host said, at handshake time, that its agent was reachable.
    /// A reachable bridge with an unavailable agent is `LC003` in the support
    /// table: the link is up and every command fails.
    pub(crate) fn agent_available(&self) -> bool {
        agent_available(&self.presence)
    }

    /// The traffic-key mutex, shared with the event stream task. The task takes
    /// this same lock, so a stream and a concurrent command both work without
    /// holding it across an await.
    pub(crate) fn channel_for_stream(
        &self,
    ) -> std::sync::Arc<std::sync::Mutex<future_remote_crypto::Channel>> {
        self.channel.clone()
    }

    /// The underlying connection, so a test can drive a handshake on a socket it
    /// has closed.
    #[cfg(test)]
    pub(crate) fn client_for_test(&self) -> &async_nats::Client {
        &self.client
    }

    /// Drain the connection: every subscription this session holds ends.
    ///
    /// `drain` rather than a bare drop, because draining is what actually
    /// *ends the streams* — the state a process shutdown leaves behind, and the
    /// only way a stream task can observe that its socket is gone. Killing the
    /// broker does not do it: the client reconnects and keeps the subscriptions
    /// open.
    #[cfg(test)]
    pub(crate) async fn close_socket(&self) -> Result<(), crate::AppError> {
        self.client
            .drain()
            .await
            .map_err(|error| crate::AppError::RemoteTransport(error.to_string()))
    }

    /// Subscribe to the host's event, presence and file-transfer streams.
    ///
    /// `p.{pair}.evt.>` carries session events, `p.{pair}.presence` the host's
    /// liveness, and `p.{pair}.xfer.down.>` the file bytes a `pull` produces.
    /// All three are decrypted with the same channel as commands, so pushed
    /// bytes are authenticated exactly as strongly as a reply.
    ///
    /// The transfer subscription is taken at connect time rather than around a
    /// download because of the order the host uses: it publishes the chunk
    /// *before* it acknowledges the pull (so the bytes are observably on the
    /// wire first). A subscription opened after the pull request would therefore
    /// race the very publish it is waiting for, and NATS Core does not replay.
    pub(crate) async fn subscribe(
        &self,
    ) -> Result<
        (
            async_nats::Subscriber,
            async_nats::Subscriber,
            async_nats::Subscriber,
        ),
        crate::AppError,
    > {
        let events = subscribe(&self.client, format!("p.{}.evt.>", self.pair_id)).await?;
        let presence = subscribe(&self.client, format!("p.{}.presence", self.pair_id)).await?;
        // Chunks only, by shape: `xfer.down.*.chunk.*` is exactly the subject the
        // host publishes a pulled chunk on. The wildcard is what makes the
        // receive path's parse total — a delivery that is not a chunk cannot
        // arrive, so there is no "what else could this be" to answer.
        let transfers = subscribe(
            &self.client,
            format!("p.{}.xfer.down.*.chunk.*", self.pair_id),
        )
        .await?;
        Ok((events, presence, transfers))
    }

    /// Where a chunk pull is requested: the host's upload lane, since the client
    /// is pulling *from* it.
    pub(crate) fn transfer_pull_subject(&self, transfer_id: &str, index: u64) -> String {
        format!("p.{}.xfer.up.{transfer_id}.pull.{index}", self.pair_id)
    }

    /// A command subject for `session_key`: the host routes on the trailing
    /// token, and `list` / `new` are its two non-session lanes.
    pub(crate) fn command_subject(&self, session_key: &str) -> String {
        command_subject(&self.pair_id, session_key)
    }

    /// Send one encrypted command and decode its reply.
    ///
    /// `bridge_instance_id` is attached when the caller is addressing an
    /// already-established conversation: a host that has since restarted
    /// refuses it instead of applying it to a session it never saw.
    pub(crate) async fn request(
        &mut self,
        subject: &str,
        mut payload: Value,
        timeout: Duration,
    ) -> Result<Value, crate::AppError> {
        if let Some(object) = payload.as_object_mut() {
            object
                .entry("bridgeInstanceId")
                .or_insert_with(|| json!(self.bridge_instance_id));
        }
        let plaintext = serde_json::to_vec(&payload)
            .map_err(|error| crate::AppError::Message(format!("encode command: {error}")))?;
        let wire = self
            .channel
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .seal(subject, &plaintext)
            .map_err(|_| crate::AppError::Message("remote_secure_channel_invalid".into()))?;
        let response = tokio::time::timeout(
            timeout,
            self.client
                .request(subject.to_string(), wire.clone().into()),
        )
        .await
        .map_err(|_| crate::AppError::RemoteTransport("Remote request timed out".into()))?
        .map_err(|error| {
            crate::AppError::RemoteTransport(format!("Remote request failed: {error}"))
        })?;
        let context = future_remote_crypto::reply_context(subject, &wire)
            .map_err(|_| crate::AppError::Message("remote_secure_channel_invalid".into()))?;
        let opened = self
            .channel
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .open(&context, &response.payload)
            .map_err(|_| crate::AppError::Message("remote_secure_channel_invalid".into()))?;
        let reply: Value = serde_json::from_slice(&opened)
            .map_err(|error| crate::AppError::Message(format!("decode reply: {error}")))?;
        if reply.get("success").and_then(Value::as_bool) != Some(true) {
            let detail = reply
                .get("error")
                .and_then(Value::as_str)
                .unwrap_or("command_failed");
            return Err(crate::AppError::Message(detail.to_string()));
        }
        Ok(reply.get("data").cloned().unwrap_or(Value::Null))
    }
}

/// The one place the trailing routing token is built. An empty session key is
/// the "new conversation" lane the host serves, not a subject ending in a dot
/// (which the host's `cmd.>` subscription would still receive, but whose dedup
/// and reply routing differ).
pub(crate) fn command_subject(pair_id: &str, session_key: &str) -> String {
    let key = if session_key.is_empty() {
        "new"
    } else {
        session_key
    };
    format!("p.{pair_id}.cmd.{key}")
}

/// What the host's handshake confirmation has to say for the connection to be
/// usable.
///
/// A pure validator, deliberately: every rejection here is a *protocol* fault a
/// real host should never produce, so the only way to exercise them is to feed
/// in the malformed confirmation a buggy or hostile peer would send — which
/// cannot be arranged against a real host by construction.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Confirmed {
    pub(crate) bridge_instance_id: String,
    pub(crate) features: Vec<String>,
    pub(crate) presence: Value,
}

impl Confirmed {
    pub(crate) fn from_confirmation(
        confirmation: &Value,
        pair_id: &str,
    ) -> Result<Self, crate::AppError> {
        let invalid = || crate::AppError::Message("remote_secure_channel_invalid".into());
        let bridge_instance_id = confirmation
            .get("bridgeInstanceId")
            .and_then(Value::as_str)
            .filter(|id| !id.is_empty())
            .ok_or_else(invalid)?
            .to_string();
        // The confirmation is the host's statement about *this* pairing. A reply
        // that confirms a different pair, or that does not claim v2, is a
        // protocol mismatch rather than a degraded session — accepting it would
        // mean trusting an identity the handshake never proved.
        if confirmation.get("confirmed").and_then(Value::as_bool) != Some(true)
            || confirmation.get("pairId").and_then(Value::as_str) != Some(pair_id)
        {
            return Err(invalid());
        }
        let features: Vec<String> = confirmation
            .get("features")
            .and_then(Value::as_array)
            .map(|values| {
                values
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default();
        if !features.iter().any(|feature| feature == "e2ee_v2") {
            return Err(invalid());
        }
        Ok(Self {
            bridge_instance_id,
            features,
            presence: confirmation.get("presence").cloned().unwrap_or(Value::Null),
        })
    }
}

/// The host's own statement about its agent, read from the handshake presence.
///
/// Absent or non-boolean reads as `false`, not `true`: the value gates a UI that
/// says "everything is fine", so an older host that does not report the field
/// must not be assumed healthy — the user would find out from a failed command
/// instead.
pub(crate) fn agent_available(presence: &Value) -> bool {
    presence
        .get("agentAvailable")
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

/// A completed connection, plus what the handshake learned about the pairing.
pub(crate) struct Connected {
    pub session: PeerSession,
    /// True when this connection consumed the invitation PSK, i.e. it was the
    /// first successful pairing. The caller must then drop the stored secret:
    /// the host has already dropped its own copy, and keeping ours would leave
    /// a dead-but-real bearer credential on disk.
    pub consumed_invitation: bool,
}

/// Connect and authenticate. One attempt, no retry policy (the supervisor owns
/// that), but the two handshake patterns are tried in the only order that is
/// safe: PSK pairing first, then `IK` with the pinned key.
pub(crate) async fn connect(creds: &PeerCreds) -> Result<Connected, crate::AppError> {
    let key_pair = std::sync::Arc::new(nkeys::KeyPair::from_seed(&creds.nkey_seed).map_err(
        |error| crate::AppError::Message(format!("Invalid stored device NKey: {error}")),
    )?);
    let signer = key_pair.clone();
    let options = async_nats::ConnectOptions::with_jwt(creds.user_jwt.clone(), move |nonce| {
        let signer = signer.clone();
        async move { signer.sign(&nonce).map_err(async_nats::AuthError::new) }
    })
    .custom_inbox_prefix(format!("p.{}.rep.{}", creds.pair_id, creds.device_id));
    // Unit tests use an in-process, plaintext fake broker. There is no runtime
    // switch that permits a production TLS downgrade.
    #[cfg(not(test))]
    let options = options.require_tls(true);
    let client = options.connect(&creds.nats_url).await.map_err(|error| {
        crate::AppError::RemoteTransport(format!("Failed to connect to NATS: {error}"))
    })?;

    let identity = creds
        .secure
        .as_ref()
        .ok_or_else(|| crate::AppError::Message("peer_pairing_identity_missing".to_string()))?;
    let private = URL_SAFE_NO_PAD
        .decode(&identity.private_key)
        .map_err(|_| crate::AppError::Message("remote_secure_channel_invalid".into()))?;
    let peer_key = identity
        .peer_public_key
        .as_ref()
        .ok_or_else(|| crate::AppError::Message("peer_pairing_identity_missing".to_string()))
        .and_then(|key| {
            URL_SAFE_NO_PAD
                .decode(key)
                .map_err(|_| crate::AppError::Message("remote_secure_channel_invalid".into()))
        })?;
    let prologue = future_remote_crypto::prologue(&creds.pair_id, &creds.desktop_id)
        .map_err(|_| crate::AppError::Message("remote_secure_channel_invalid".into()))?;

    // Pairing first when the invitation secret is still held; otherwise (and
    // after a pairing that already succeeded) `IK` against the pinned key.
    if let Some(secret) = identity.secret.as_deref() {
        match handshake(
            &client,
            creds,
            &private,
            &peer_key,
            Some(secret),
            &prologue,
            true,
        )
        .await
        {
            Ok(session) => {
                return Ok(Connected {
                    session,
                    consumed_invitation: true,
                })
            }
            // The response to a *successful* pairing can be lost, and the host
            // deletes its PSK the moment it binds this identity — so a retry
            // with the PSK would fail forever while the pairing is in fact
            // valid. `IK` proves the same local identity against the pinned
            // key; it is not a TOFU or plaintext fallback.
            Err(error) => {
                eprintln!("remote_peer: pairing handshake failed ({error}); retrying with IK");
            }
        }
    }
    let session = handshake(&client, creds, &private, &peer_key, None, &prologue, false).await?;
    Ok(Connected {
        session,
        consumed_invitation: false,
    })
}

/// One handshake exchange. `pairing` selects `Noise_XXpsk0` (with `secret`);
/// otherwise `Noise_IK` against the pinned peer key.
#[allow(clippy::too_many_arguments)]
async fn handshake(
    client: &async_nats::Client,
    creds: &PeerCreds,
    private: &[u8],
    peer_key: &[u8],
    secret: Option<&str>,
    prologue: &[u8],
    pairing: bool,
) -> Result<PeerSession, crate::AppError> {
    let secret_bytes: Option<[u8; 32]> = match secret {
        Some(secret) => Some(
            URL_SAFE_NO_PAD
                .decode(secret)
                .map_err(|_| crate::AppError::Message("remote_secure_channel_invalid".into()))?
                .try_into()
                .map_err(|_| crate::AppError::Message("remote_secure_channel_invalid".into()))?,
        ),
        None => None,
    };
    let pattern = if pairing {
        future_remote_crypto::Pattern::Pair
    } else {
        future_remote_crypto::Pattern::Reconnect
    };
    let mut noise = future_remote_crypto::Handshake::new(
        pattern,
        true,
        private,
        // XXpsk0 learns the responder's key during the handshake and checks it
        // against the invitation afterwards; IK requires it up front.
        (!pairing).then_some(peer_key),
        secret_bytes.as_ref(),
        prologue,
    )
    .map_err(|_| crate::AppError::Message("remote_secure_channel_invalid".into()))?;

    let subject = format!("p.{}.cmd.handshake", creds.pair_id);
    let first = noise
        .write(b"")
        .map_err(|_| crate::AppError::Message("remote_secure_channel_invalid".into()))?;
    let mut response = exchange(
        client,
        &subject,
        json!({
            "type": "secure_open",
            "pairing": pairing,
            "message": URL_SAFE_NO_PAD.encode(&first),
        }),
    )
    .await?;
    let message = URL_SAFE_NO_PAD
        .decode(
            response
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or_default(),
        )
        .map_err(|_| crate::AppError::Message("remote_secure_channel_invalid".into()))?;
    noise
        .read(&message)
        .map_err(|_| crate::AppError::Message("remote_secure_channel_invalid".into()))?;
    // The host must be the machine the invitation named. Checked here, before
    // the final message is sent, so a swapped peer never receives our half.
    if noise
        .remote_public_key()
        .map_err(|_| crate::AppError::Message("remote_secure_channel_invalid".into()))?
        != peer_key
    {
        return Err(crate::AppError::Message(
            "remote_secure_channel_invalid".into(),
        ));
    }
    if pairing {
        let second = noise
            .write(b"")
            .map_err(|_| crate::AppError::Message("remote_secure_channel_invalid".into()))?;
        response = exchange(
            client,
            &subject,
            json!({
                "type": "secure_finish",
                "id": response.get("id").and_then(Value::as_str).unwrap_or_default(),
                "message": URL_SAFE_NO_PAD.encode(&second),
            }),
        )
        .await?;
    }
    let mut channel = noise
        .finish()
        .map_err(|_| crate::AppError::Message("remote_secure_channel_invalid".into()))?;
    let encrypted = URL_SAFE_NO_PAD
        .decode(
            response
                .get("confirmation")
                .and_then(Value::as_str)
                .unwrap_or_default(),
        )
        .map_err(|_| crate::AppError::Message("remote_secure_channel_invalid".into()))?;
    let confirmation = channel
        .open("handshake-confirm", &encrypted)
        .map_err(|_| crate::AppError::Message("remote_secure_channel_invalid".into()))?;
    let confirmation: Value = serde_json::from_slice(&confirmation)
        .map_err(|_| crate::AppError::Message("remote_secure_channel_invalid".into()))?;
    let confirmed = Confirmed::from_confirmation(&confirmation, &creds.pair_id)?;

    let mut session = PeerSession {
        pair_id: creds.pair_id.clone(),
        client: client.clone(),
        channel: std::sync::Arc::new(std::sync::Mutex::new(channel)),
        bridge_instance_id: confirmed.bridge_instance_id,
        features: confirmed.features,
        presence: confirmed.presence,
    };
    // Note the *absence* of declared capabilities: this client deliberately
    // asks for the conservative lanes (no coalescing, no reply gzip, no lean
    // events). `secure_ready` is what commits the host's candidate channel, so
    // it must still be sent — declaring nothing keeps the protocol readable
    // from the wire and leaves the optimisation flags for a later, measured
    // change.
    session
        .request(
            &session.command_subject("handshake"),
            json!({ "type": "secure_ready", "features": [] }),
            HANDSHAKE_TIMEOUT,
        )
        .await?;
    Ok(session)
}

/// Subscribe to one subject, with the failure classified the same way for both
/// streams.
///
/// One implementation rather than two identical `map_err`s: the second one would
/// only ever be reachable by the call that happens to run first, so it could
/// never be exercised.
async fn subscribe(
    client: &async_nats::Client,
    subject: String,
) -> Result<async_nats::Subscriber, crate::AppError> {
    client.subscribe(subject).await.map_err(|error| {
        crate::AppError::RemoteTransport(format!("Remote subscribe failed: {error}"))
    })
}

/// One unencrypted handshake leg, exercised directly by a test.
///
/// The only way to make a handshake fail is against a socket the test itself
/// controls: a broker that accepts a connection and then stops answering is not
/// something the fake broker can be scripted into mid-handshake.
#[cfg(test)]
pub(crate) async fn exchange_for_test(
    client: &async_nats::Client,
    subject: &str,
    body: Value,
) -> Result<Value, crate::AppError> {
    exchange(client, subject, body).await
}

/// One unencrypted handshake leg. Every field this client reads is optional by
/// construction — the host answers refusals with `success: false` — so a
/// missing field is reported as a protocol error rather than a panic.
async fn exchange(
    client: &async_nats::Client,
    subject: &str,
    body: Value,
) -> Result<Value, crate::AppError> {
    let bytes = serde_json::to_vec(&body)
        .map_err(|error| crate::AppError::Message(format!("encode handshake: {error}")))?;
    let response = tokio::time::timeout(
        HANDSHAKE_TIMEOUT,
        client.request(subject.to_string(), bytes.into()),
    )
    .await
    .map_err(|_| crate::AppError::RemoteTransport("Remote handshake timed out".into()))?
    .map_err(|error| {
        crate::AppError::RemoteTransport(format!("Remote handshake failed: {error}"))
    })?;
    let reply: Value = serde_json::from_slice(&response.payload)
        .map_err(|_| crate::AppError::Message("remote_secure_channel_invalid".into()))?;
    if reply.get("success").and_then(Value::as_bool) != Some(true) {
        // The host appends *why* after the stable token (`invitation`, `peer`,
        // …). Keeping it is what turns "pairing failed" into an actionable
        // message on both sides.
        let detail = reply
            .get("error")
            .and_then(Value::as_str)
            .unwrap_or("unknown");
        return Err(crate::AppError::Message(format!(
            "remote_secure_channel_invalid ({detail})"
        )));
    }
    Ok(reply.get("data").cloned().unwrap_or(Value::Null))
}

#[cfg(all(test, not(miri)))]
mod tests {
    use super::*;

    #[test]
    fn command_subjects_use_the_hosts_two_lane_names() {
        assert_eq!(command_subject("pair_1", ""), "p.pair_1.cmd.new");
        assert_eq!(command_subject("pair_1", "list"), "p.pair_1.cmd.list");
        assert_eq!(command_subject("pair_1", "sess_9"), "p.pair_1.cmd.sess_9");
    }

    /// Asserted here rather than through a live connection: whether an agent
    /// happens to be reachable is a property of the machine running the test,
    /// not of the client, so an end-to-end check of it would be asserting the
    /// environment. What the client owns is the *reading*.
    #[test]
    fn agent_availability_is_read_from_the_hosts_own_statement() {
        assert!(agent_available(&json!({ "agentAvailable": true })));
        assert!(!agent_available(&json!({ "agentAvailable": false })));
        // An older host that omits the field, or reports it as something else,
        // must not be treated as healthy: the UI says "everything is fine" on
        // `true`, and the user would otherwise find out from a failed command.
        assert!(!agent_available(&json!({})));
        assert!(!agent_available(&json!({ "agentAvailable": "yes" })));
        assert!(!agent_available(&Value::Null));
    }

    fn confirmation(overrides: Value) -> Value {
        let mut base = json!({
            "confirmed": true,
            "pairId": "pair_1",
            "bridgeInstanceId": "bridge-1",
            "features": ["e2ee_v2"],
            "presence": { "agentAvailable": true },
        });
        // A non-object override cannot be merged; the cases below are all
        // objects, and asserting that keeps the helper from silently ignoring
        // one that is not.
        let target = base.as_object_mut().expect("a JSON object literal");
        for (key, value) in overrides.as_object().expect("object overrides") {
            target.insert(key.clone(), value.clone());
        }
        base
    }

    #[test]
    fn a_well_formed_confirmation_is_accepted() {
        let confirmed = Confirmed::from_confirmation(&confirmation(json!({})), "pair_1")
            .expect("valid confirmation");
        assert_eq!(confirmed.bridge_instance_id, "bridge-1");
        assert_eq!(confirmed.features, vec!["e2ee_v2".to_string()]);
        assert_eq!(confirmed.presence["agentAvailable"], json!(true));
    }

    /// Every case here is a protocol fault no real host produces, so handing the
    /// validator what a buggy or hostile one would send is the only way to reach
    /// the arms — and they are exactly the arms that decide whether a client
    /// trusts a claimed identity.
    #[test]
    fn a_confirmation_that_does_not_prove_this_pairing_is_refused() {
        let cases = [
            // A relay replaying another pairing's confirmation.
            ("another pair", json!({ "pairId": "pair_other" })),
            ("missing pair", json!({ "pairId": null })),
            // A host that never affirmed the handshake.
            ("not confirmed", json!({ "confirmed": false })),
            ("confirmed missing", json!({ "confirmed": null })),
            // No bridge identity: nothing to tell one host incarnation from another.
            ("no bridge", json!({ "bridgeInstanceId": null })),
            ("empty bridge", json!({ "bridgeInstanceId": "" })),
            // A peer that cannot speak the record layer we are about to use.
            ("no features", json!({ "features": null })),
            ("other features", json!({ "features": ["something_else"] })),
            ("features wrong type", json!({ "features": "e2ee_v2" })),
        ];
        for (label, overrides) in cases {
            let result = Confirmed::from_confirmation(&confirmation(overrides), "pair_1");
            assert!(result.is_err(), "{label} must be refused, but was accepted");
            assert_eq!(
                result.err().map(|error| error.to_string()).as_deref(),
                Some("remote_secure_channel_invalid"),
                "{label}"
            );
        }
    }

    /// A host that reports no presence at all is still usable: only the fields
    /// the client actually relies on are required.
    #[test]
    fn a_missing_presence_is_not_a_protocol_fault() {
        let confirmed =
            Confirmed::from_confirmation(&confirmation(json!({ "presence": null })), "pair_1")
                .expect("presence is optional");
        assert_eq!(confirmed.presence, Value::Null);
        assert!(!agent_available(&confirmed.presence));
    }
}
