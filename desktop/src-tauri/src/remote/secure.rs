//! Remote v2 endpoint state. Broker ACLs are not an authentication boundary.
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use future_remote_crypto::{Channel, Handshake, Pattern};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PairingIdentity {
    pub private_key: String,
    pub public_key: String,
    pub peer_public_key: Option<String>,
    pub secret: Option<String>,
    pub expires_at: i64,
}
impl std::fmt::Debug for PairingIdentity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PairingIdentity")
            .field("paired", &self.peer_public_key.is_some())
            .finish_non_exhaustive()
    }
}
impl PairingIdentity {
    pub fn new(expires_at: i64) -> Result<Self, crate::AppError> {
        let (private, public) = future_remote_crypto::generate_identity().map_err(error)?;
        let mut secret = [0; 32];
        rand::rngs::OsRng.fill_bytes(&mut secret);
        Ok(Self {
            private_key: URL_SAFE_NO_PAD.encode(private.as_slice()),
            public_key: URL_SAFE_NO_PAD.encode(public),
            peer_public_key: None,
            secret: Some(URL_SAFE_NO_PAD.encode(secret)),
            expires_at,
        })
    }
}
fn error(_: impl std::fmt::Display) -> crate::AppError {
    crate::AppError::Message("remote_secure_channel_invalid".into())
}

/// Wire error for a refused handshake, kept byte-stable for clients: every one
/// of them branches on `success: false`, not on this string.
pub(super) const REFUSED: &str = "remote_secure_channel_invalid";

/// A refusal that carries *why*. The reply still says `success: false` with
/// [`REFUSED`] as its prefix, and the reason rides along after it for the
/// desktop's log and the phone's console. Without it a QR the bridge can no
/// longer authenticate (`invitation`, `peer`, a PSK mismatch) is
/// indistinguishable from a network fault, which is exactly how a pairing that
/// could never succeed used to present: no code, no log, on either side.
fn refused(reason: &str) -> crate::AppError {
    crate::AppError::Message(format!("{REFUSED} ({reason})"))
}
fn decode(value: &str) -> Result<Vec<u8>, crate::AppError> {
    if value.len() > 12_000 {
        return Err(error("length"));
    }
    URL_SAFE_NO_PAD.decode(value).map_err(error)
}
fn key(value: &str) -> Result<[u8; 32], crate::AppError> {
    decode(value)?.try_into().map_err(|_| error("key"))
}

type SharedChannel = Arc<Mutex<Channel>>;
struct Pending {
    noise: Handshake,
    created: Instant,
}
struct Candidate {
    channel: SharedChannel,
    created: Instant,
}
struct Server {
    candidates: Vec<Candidate>,
    creds: super::protocol::PairingCreds,
    pending: HashMap<String, Pending>,
    current: Option<SharedChannel>,
}
#[derive(Clone, Default)]
pub(super) struct Transport {
    server: Option<Arc<Mutex<Server>>>,
    #[cfg(test)]
    legacy_fixture: bool,
}
impl Transport {
    pub fn new(creds: &super::protocol::PairingCreds) -> Self {
        Self {
            server: (creds.handshake_version == 2 && creds.secure.is_some()).then(|| {
                Arc::new(Mutex::new(Server {
                    creds: creds.clone(),
                    pending: HashMap::new(),
                    candidates: Vec::new(),
                    current: None,
                }))
            }),
            #[cfg(test)]
            legacy_fixture: creds.handshake_version == 1,
        }
    }
    #[cfg(test)]
    pub fn legacy_fixture() -> Self {
        Self {
            server: None,
            legacy_fixture: true,
        }
    }
    pub fn enabled(&self) -> bool {
        self.server.is_some()
    }
    pub fn clear(&self) {
        if let Some(server) = &self.server {
            let mut server = server.lock().unwrap();
            server.current = None;
            server.pending.clear();
            server.candidates.clear();
        }
    }
    pub fn seal(&self, context: &str, bytes: &[u8]) -> Result<Option<Vec<u8>>, crate::AppError> {
        #[cfg(test)]
        if self.legacy_fixture {
            return Ok(Some(bytes.to_vec()));
        }
        let Some(server) = &self.server else {
            return Ok(None);
        };
        let channel = server.lock().unwrap().current.clone();
        channel
            .map(|c| c.lock().unwrap().seal(context, bytes).map_err(error))
            .transpose()
    }
    pub fn open(&self, subject: &str, bytes: &[u8]) -> Result<(Vec<u8>, Reply), crate::AppError> {
        #[cfg(test)]
        if self.legacy_fixture {
            return Ok((
                bytes.to_vec(),
                Reply {
                    channel: None,
                    context: String::new(),
                },
            ));
        }
        let server = self.server.as_ref().ok_or_else(|| error("disabled"))?;
        let channels = {
            let mut server = server.lock().unwrap();
            server
                .candidates
                .retain(|c| c.created.elapsed() < Duration::from_secs(30));
            server
                .current
                .iter()
                .cloned()
                .chain(server.candidates.iter().map(|c| c.channel.clone()))
                .collect::<Vec<_>>()
        };
        for channel in channels {
            let opened = channel.lock().unwrap().open(subject, bytes);
            if let Ok(plaintext) = opened {
                let context = future_remote_crypto::reply_context(subject, bytes).map_err(error)?;
                return Ok((
                    plaintext,
                    Reply {
                        channel: Some(channel),
                        context,
                    },
                ));
            }
        }
        Err(error("unauthenticated"))
    }
    pub fn is_active(&self, reply: &Reply) -> bool {
        #[cfg(test)]
        if self.legacy_fixture {
            return true;
        }
        let Some(server) = &self.server else {
            return false;
        };
        let server = server.lock().unwrap();
        server
            .current
            .as_ref()
            .zip(reply.channel.as_ref())
            .is_some_and(|(a, b)| Arc::ptr_eq(a, b))
    }
    /// Commit only after the phone has subscribed and flushed its replacement
    /// socket. Failed readiness must leave the previous channel serving.
    pub fn activate(&self, reply: &Reply) -> Result<(), crate::AppError> {
        let channel = reply.channel.as_ref().ok_or_else(|| error("channel"))?;
        let mut server = self
            .server
            .as_ref()
            .ok_or_else(|| error("server"))?
            .lock()
            .unwrap();
        if server
            .current
            .as_ref()
            .is_some_and(|c| Arc::ptr_eq(c, channel))
        {
            return Ok(());
        }
        let index = server
            .candidates
            .iter()
            .position(|c| {
                Arc::ptr_eq(&c.channel, channel) && c.created.elapsed() < Duration::from_secs(30)
            })
            .ok_or_else(|| error("candidate"))?;
        let candidate = server.candidates.remove(index);
        server.current = Some(candidate.channel);
        Ok(())
    }
    /// Raw handshake messages are bounded and cannot invoke any business API.
    /// An untrusted candidate never replaces the current authenticated channel.
    pub fn handshake(&self, payload: &[u8], bridge: &str) -> Result<Value, crate::AppError> {
        if payload.len() > 16_384 {
            return Err(refused("oversized"));
        }
        let request: HandshakeRequest =
            serde_json::from_slice(payload).map_err(|_| refused("malformed"))?;
        let mut server = self
            .server
            .as_ref()
            .ok_or_else(|| refused("disabled"))?
            .lock()
            .unwrap();
        server
            .pending
            .retain(|_, p| p.created.elapsed() < Duration::from_secs(30));
        server
            .candidates
            .retain(|c| c.created.elapsed() < Duration::from_secs(30));
        let identity = server
            .creds
            .secure
            .as_ref()
            .ok_or_else(|| refused("identity"))?
            .clone();
        let prologue =
            future_remote_crypto::prologue(&server.creds.pair_id, &server.creds.desktop_id)
                .map_err(|_| refused("prologue"))?;
        let input = decode(&request.message).map_err(|_| refused("message"))?;
        match request.kind.as_str() {
            "secure_open" => {
                if server.pending.len() + server.candidates.len() >= 16 {
                    return Err(refused("busy"));
                }
                let pairing = request.pairing;
                let psk = if pairing {
                    if identity.peer_public_key.is_some() {
                        // The phone is re-offering a `pairing=true` opening for a
                        // pair that already pinned a key. Only a fresh invitation
                        // can pair again.
                        return Err(refused("invitation_already_used"));
                    }
                    if super::unix_timestamp() >= identity.expires_at.max(0) as u64 {
                        return Err(refused("invitation_expired"));
                    }
                    Some(
                        key(identity
                            .secret
                            .as_deref()
                            .ok_or_else(|| refused("secret"))?)
                        .map_err(|_| refused("secret"))?,
                    )
                } else {
                    None
                };
                if !pairing && identity.peer_public_key.is_none() {
                    return Err(refused("peer_unknown"));
                }
                let mut noise = Handshake::new(
                    if pairing {
                        Pattern::Pair
                    } else {
                        Pattern::Reconnect
                    },
                    false,
                    &key(&identity.private_key).map_err(|_| refused("private_key"))?,
                    None,
                    psk.as_ref(),
                    &prologue,
                )
                .map_err(|_| refused("pattern"))?;
                if !noise
                    .read(&input)
                    .map_err(|_| refused("open_undecryptable"))?
                    .is_empty()
                {
                    return Err(refused("open_payload"));
                }
                if !pairing
                    && noise.remote_public_key().map_err(|_| refused("peer"))?
                        != key(identity
                            .peer_public_key
                            .as_deref()
                            .ok_or_else(|| refused("peer_unknown"))?)
                        .map_err(|_| refused("peer"))?
                {
                    // The peer proved a key other than the one this pair pinned.
                    return Err(refused("peer_mismatch"));
                }
                let response = noise.write(b"").map_err(|_| refused("open_response"))?;
                if pairing {
                    let mut token = [0; 16];
                    rand::rngs::OsRng.fill_bytes(&mut token);
                    let id = URL_SAFE_NO_PAD.encode(token);
                    server.pending.insert(
                        id.clone(),
                        Pending {
                            noise,
                            created: Instant::now(),
                        },
                    );
                    Ok(json!({ "message": URL_SAFE_NO_PAD.encode(response), "id": id }))
                } else {
                    let channel = Arc::new(Mutex::new(
                        noise.finish().map_err(|_| refused("open_finish"))?,
                    ));
                    let confirmation = confirmation(&server.creds.pair_id, bridge, &channel)
                        .map_err(|_| refused("confirmation"))?;
                    server.candidates.push(Candidate {
                        channel,
                        created: Instant::now(),
                    });
                    Ok(
                        json!({ "message": URL_SAFE_NO_PAD.encode(response), "confirmation": confirmation }),
                    )
                }
            }
            "secure_finish" => {
                let mut pending = server
                    .pending
                    .remove(&request.id)
                    .ok_or_else(|| refused("challenge_expired"))?;
                if identity.peer_public_key.is_some() {
                    return Err(refused("invitation_already_used"));
                }
                if super::unix_timestamp() >= identity.expires_at.max(0) as u64 {
                    return Err(refused("invitation_expired"));
                }
                if !pending
                    .noise
                    .read(&input)
                    .map_err(|_| refused("finish_undecryptable"))?
                    .is_empty()
                {
                    return Err(refused("finish_payload"));
                }
                let peer = URL_SAFE_NO_PAD.encode(
                    pending
                        .noise
                        .remote_public_key()
                        .map_err(|_| refused("peer"))?,
                );
                let channel = Arc::new(Mutex::new(
                    pending.noise.finish().map_err(|_| refused("finish"))?,
                ));
                let mut creds = server.creds.clone();
                let identity = creds.secure.as_mut().ok_or_else(|| refused("identity"))?;
                identity.peer_public_key = Some(peer);
                identity.secret = None;
                // Persist binding before accepting any command. A lost response
                // can recover through IK using the phone's already stored key.
                super::pairing::save_creds(&creds).map_err(|_| refused("save_creds"))?;
                let confirmation = confirmation(&creds.pair_id, bridge, &channel)
                    .map_err(|_| refused("confirmation"))?;
                server.creds = creds;
                server.pending.clear();
                server.candidates.push(Candidate {
                    channel,
                    created: Instant::now(),
                });
                Ok(json!({ "confirmation": confirmation }))
            }
            other => Err(refused(if other.is_empty() {
                "missing_type"
            } else {
                "unknown_type"
            })),
        }
    }
}
#[derive(Deserialize)]
struct HandshakeRequest {
    #[serde(rename = "type")]
    kind: String,
    message: String,
    #[serde(default)]
    id: String,
    #[serde(default)]
    pairing: bool,
}
fn confirmation(
    pair_id: &str,
    bridge: &str,
    channel: &SharedChannel,
) -> Result<String, crate::AppError> {
    let body = json!({ "confirmed": true, "pairId": pair_id, "bridgeInstanceId": bridge,
        "features": ["e2ee_v2", "file_transfer_v1", "file_download_v2", "approval_tier_v1", "continue_run_v1", "prompt_receipt_v1", "session_files_v1", "skills_v1", "selective_events_v1", "workspace_pinning_v1", "desktop_settings_v1", "skill_management_v1", "compaction_v1", "provider_management_v1"],
        "presence": super::build_presence_payload(pair_id, bridge) });
    let payload = serde_json::to_vec(&body).map_err(error)?;
    Ok(URL_SAFE_NO_PAD.encode(
        channel
            .lock()
            .unwrap()
            .seal("handshake-confirm", &payload)
            .map_err(error)?,
    ))
}

#[derive(Clone)]
pub(super) struct Reply {
    channel: Option<SharedChannel>,
    context: String,
}
impl Reply {
    pub fn seal(&self, plaintext: &[u8]) -> Result<Vec<u8>, crate::AppError> {
        #[cfg(test)]
        if self.channel.is_none() {
            return Ok(plaintext.to_vec());
        }
        self.channel
            .as_ref()
            .ok_or_else(|| error("channel"))?
            .lock()
            .unwrap()
            .seal(&self.context, plaintext)
            .map_err(error)
    }
}
tokio::task_local! { pub(super) static REPLY: Reply; }

pub(super) async fn publish(
    client: &async_nats::Client,
    transport: &Transport,
    subject: String,
    bytes: Vec<u8>,
) -> Result<bool, crate::AppError> {
    // `seal` yields `None` while the channel has no established key yet: before
    // the client's handshake completes, and again after a credential refresh
    // cleared it. The payload is dropped on the floor. Reporting that instead of
    // a silent `Ok(())` is what lets a *change-driven* publisher retry — a
    // periodic one would never notice, which is exactly how the old catalog
    // self-heal timer hid this.
    let Some(wire) = transport.seal(&subject, &bytes)? else {
        return Ok(false);
    };
    client
        .publish(subject, wire.into())
        .await
        .map_err(|e| crate::AppError::RemoteTransport(e.to_string()))?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::remote::test_support::{init_store, now_secs, HomeGuard};
    fn creds() -> super::super::protocol::PairingCreds {
        super::super::protocol::PairingCreds {
            handshake_version: 2,
            secure: Some(PairingIdentity::new(now_secs() + 300).unwrap()),
            pair_id: "pair_secure".into(),
            desktop_id: "desktop_secure".into(),
            nkey_seed: "unused".into(),
            user_jwt: "unused".into(),
            nats_url: "unused".into(),
            nats_ws_url: "unused".into(),
            jwt_expires_at: now_secs() + 300,
        }
    }
    fn prepare_connection(
        transport: &Transport,
        creds: &super::super::protocol::PairingCreds,
        private: &[u8],
        pairing: bool,
    ) -> Result<(Channel, Reply), crate::AppError> {
        let identity = creds.secure.as_ref().unwrap();
        let public = key(&identity.public_key)?;
        let psk = if pairing {
            Some(key(identity.secret.as_deref().unwrap())?)
        } else {
            None
        };
        let prologue = future_remote_crypto::prologue(&creds.pair_id, &creds.desktop_id).unwrap();
        let mut noise = Handshake::new(
            if pairing {
                Pattern::Pair
            } else {
                Pattern::Reconnect
            },
            true,
            private,
            (!pairing).then_some(public.as_slice()),
            psk.as_ref(),
            &prologue,
        )
        .map_err(error)?;
        let open = json!({ "type": "secure_open", "pairing": pairing, "message": URL_SAFE_NO_PAD.encode(noise.write(b"").map_err(error)?) });
        let mut response =
            transport.handshake(&serde_json::to_vec(&open).unwrap(), "bridge_secure")?;
        noise
            .read(&decode(response["message"].as_str().unwrap())?)
            .map_err(error)?;
        assert_eq!(noise.remote_public_key().unwrap(), public);
        if pairing {
            let finish = json!({ "type": "secure_finish", "id": response["id"], "message": URL_SAFE_NO_PAD.encode(noise.write(b"").map_err(error)?) });
            response =
                transport.handshake(&serde_json::to_vec(&finish).unwrap(), "bridge_secure")?;
        }
        let mut channel = noise.finish().map_err(error)?;
        let confirm: Value = serde_json::from_slice(
            &channel
                .open(
                    "handshake-confirm",
                    &decode(response["confirmation"].as_str().unwrap())?,
                )
                .map_err(error)?,
        )
        .unwrap();
        assert_eq!(confirm["confirmed"], true);
        assert!(confirm["features"]
            .as_array()
            .unwrap()
            .contains(&json!("skills_v1")));
        let wire = channel.seal("ready", b"ready").unwrap();
        let (_, reply) = transport.open("ready", &wire).unwrap();
        Ok((channel, reply))
    }
    fn connect(
        transport: &Transport,
        creds: &super::super::protocol::PairingCreds,
        private: &[u8],
        pairing: bool,
    ) -> Result<Channel, crate::AppError> {
        let (channel, reply) = prepare_connection(transport, creds, private, pairing)?;
        transport.activate(&reply)?;
        Ok(channel)
    }
    /// A refusal must say *why*, and must keep the byte-stable prefix clients
    /// branch on. Distinguishing `invitation_expired` from a PSK mismatch turned
    /// an undiagnosable "pairing just fails" into a log line.
    #[test]
    fn a_refused_handshake_names_the_reason() {
        let _home = HomeGuard::new("secure-refusal-reason");
        init_store();
        let creds = creds();
        let transport = Transport::new(&creds);
        let refusal = |payload: &str| {
            transport
                .handshake(payload.as_bytes(), "bridge_secure")
                .unwrap_err()
                .to_string()
        };
        // A QR whose PSK is not this identity's: the phone and the desktop hold
        // different invitations.
        assert_eq!(
            refusal(r#"{"type":"secure_open","pairing":true,"message":"AAAA"}"#),
            format!("{REFUSED} (open_undecryptable)")
        );
        assert_eq!(
            refusal(r#"{"type":"nonsense","message":"AAAA"}"#),
            format!("{REFUSED} (unknown_type)")
        );
        assert_eq!(refusal("not json"), format!("{REFUSED} (malformed)"));
        assert_eq!(
            refusal(r#"{"type":"secure_finish","id":"missing","message":"AAAA"}"#),
            format!("{REFUSED} (challenge_expired)")
        );
        // An expired invitation is reported as expired, not as a bad signature.
        let mut expired = creds.clone();
        expired.secure.as_mut().unwrap().expires_at = now_secs() - 1;
        let transport = Transport::new(&expired);
        assert_eq!(
            transport
                .handshake(
                    br#"{"type":"secure_open","pairing":true,"message":"AAAA"}"#,
                    "bridge_secure"
                )
                .unwrap_err()
                .to_string(),
            format!("{REFUSED} (invitation_expired)")
        );
    }

    /// A transport backed by a `Server` the test built by hand. The refusal
    /// arms below depend on server state (a missing secret, an unreadable
    /// private key, a pinned peer) that `Transport::new` cannot produce, but
    /// which a real persisted credential can carry after an upgrade or a
    /// partial write — so each arm must name its own refusal.
    fn server_with(creds: super::super::protocol::PairingCreds) -> Transport {
        Transport {
            server: Some(Arc::new(Mutex::new(Server {
                creds,
                pending: HashMap::new(),
                candidates: Vec::new(),
                current: None,
            }))),
            legacy_fixture: false,
        }
    }

    fn refusal(transport: &Transport, payload: &str) -> String {
        transport
            .handshake(payload.as_bytes(), "bridge_secure")
            .unwrap_err()
            .to_string()
    }

    /// Perform only the `secure_open` leg of a pairing, leaving one pending
    /// challenge on the server so the `secure_finish` arms are reachable.
    fn open_pairing(
        transport: &Transport,
        creds: &super::super::protocol::PairingCreds,
        private: &[u8],
    ) -> Value {
        let identity = creds.secure.as_ref().unwrap();
        let prologue = future_remote_crypto::prologue(&creds.pair_id, &creds.desktop_id).unwrap();
        let mut noise = Handshake::new(
            Pattern::Pair,
            true,
            private,
            None,
            Some(&key(identity.secret.as_deref().unwrap()).unwrap()),
            &prologue,
        )
        .unwrap();
        let open = json!({
            "type": "secure_open",
            "pairing": true,
            "message": URL_SAFE_NO_PAD.encode(noise.write(b"").unwrap()),
        });
        transport
            .handshake(&serde_json::to_vec(&open).unwrap(), "bridge_secure")
            .expect("a fresh invitation opens")
    }

    fn finish_payload(id: &str, message: &str) -> String {
        json!({ "type": "secure_finish", "id": id, "message": message }).to_string()
    }

    /// Every refusal the handshake can produce was named only after a real
    /// pairing failure had to be diagnosed by reading raw bytes. These are the
    /// arms the happy-path tests never take: a malformed field, a credential an
    /// upgrade left incomplete, a prologue the platform will not accept, and a
    /// challenge that was consumed, reused or has expired.
    #[test]
    fn malformed_and_incomplete_credentials_are_refused_by_name() {
        let _home = HomeGuard::new("secure-refusal-arms");
        init_store();

        // A field past the 12 000-byte bound is refused before base64 decoding,
        // and a decodable field that is not a 32-byte key is refused after it.
        assert!(decode(&"A".repeat(12_001)).is_err());
        assert!(decode(&"A".repeat(12_000)).is_ok());
        assert!(key("AAAA").is_err());

        // `secure_open` on an invitation whose PSK was never minted (an upgrade
        // that kept the identity but lost the secret).
        let mut no_secret = creds();
        no_secret.secure.as_mut().unwrap().secret = None;
        assert_eq!(
            refusal(
                &server_with(no_secret),
                r#"{"type":"secure_open","pairing":true,"message":"AAAA"}"#
            ),
            format!("{REFUSED} (secret)")
        );

        // An unreadable desktop private key.
        let mut bad_key = creds();
        bad_key.secure.as_mut().unwrap().private_key = "AAAA".into();
        assert_eq!(
            refusal(
                &server_with(bad_key),
                r#"{"type":"secure_open","pairing":true,"message":"AAAA"}"#
            ),
            format!("{REFUSED} (private_key)")
        );

        // Identifiers the prologue builder rejects: an empty pair id binds
        // nothing, so no key may be derived from it.
        let mut no_prologue = creds();
        no_prologue.pair_id = String::new();
        assert_eq!(
            refusal(
                &server_with(no_prologue),
                r#"{"type":"secure_open","pairing":true,"message":"AAAA"}"#
            ),
            format!("{REFUSED} (prologue)")
        );

        // A message that is not base64 at all.
        assert_eq!(
            refusal(
                &server_with(creds()),
                r#"{"type":"secure_open","pairing":true,"message":"not base64!"}"#
            ),
            format!("{REFUSED} (message)")
        );

        // No secure identity at all: the transport cannot prove anything, and
        // must say so rather than fall through to a plaintext lane.
        let mut no_identity = creds();
        no_identity.secure = None;
        assert_eq!(
            refusal(
                &server_with(no_identity),
                r#"{"type":"secure_open","pairing":false,"message":"AAAA"}"#
            ),
            format!("{REFUSED} (identity)")
        );

        // An empty `type` is distinguished from an unknown one.
        assert_eq!(
            refusal(&server_with(creds()), r#"{"type":"","message":"AAAA"}"#),
            format!("{REFUSED} (missing_type)")
        );
    }

    /// The `secure_finish` leg re-checks everything the `secure_open` leg did,
    /// because the two arrive as separate messages: a client may reconnect
    /// between them, an invitation may expire while the phone is scanning, and
    /// a peer may replay a challenge the pair already consumed. Each of those
    /// has to be a named refusal, not a silent fallback.
    #[test]
    fn a_consumed_expired_or_undecryptable_challenge_is_refused_by_name() {
        let _home = HomeGuard::new("secure-finish-arms");
        init_store();
        let creds = creds();
        let (private, _) = future_remote_crypto::generate_identity().unwrap();

        // A challenge that decodes but does not open: the pending entry is
        // consumed by the attempt, so the refusal is about the bytes.
        let transport = server_with(creds.clone());
        let opened = open_pairing(&transport, &creds, &private);
        let id = opened["id"].as_str().unwrap().to_string();
        assert_eq!(
            refusal(&transport, &finish_payload(&id, "AAAA")),
            format!("{REFUSED} (finish_undecryptable)")
        );

        // A second attempt finds no pending entry: the challenge was consumed
        // by the attempt above, so this is `challenge_expired`, not a replay.
        assert_eq!(
            refusal(&transport, &finish_payload(&id, "AAAA")),
            format!("{REFUSED} (challenge_expired)")
        );

        // The pair pinned a peer key between the two legs (another invitation
        // completed): this challenge must not bind a second one.
        let transport = server_with(creds.clone());
        let opened = open_pairing(&transport, &creds, &private);
        let id = opened["id"].as_str().unwrap().to_string();
        transport
            .server
            .as_ref()
            .unwrap()
            .lock()
            .unwrap()
            .creds
            .secure
            .as_mut()
            .unwrap()
            .peer_public_key = Some("pinned by another pairing".into());
        assert_eq!(
            refusal(&transport, &finish_payload(&id, "AAAA")),
            format!("{REFUSED} (invitation_already_used)")
        );

        // The invitation expired while the phone was scanning it.
        let transport = server_with(creds.clone());
        let opened = open_pairing(&transport, &creds, &private);
        let id = opened["id"].as_str().unwrap().to_string();
        transport
            .server
            .as_ref()
            .unwrap()
            .lock()
            .unwrap()
            .creds
            .secure
            .as_mut()
            .unwrap()
            .expires_at = now_secs() - 1;
        assert_eq!(
            refusal(&transport, &finish_payload(&id, "AAAA")),
            format!("{REFUSED} (invitation_expired)")
        );
    }

    /// A credential row that is not a v2 secure pairing has no server, so the
    /// transport is inert in both directions: it must not seal (which would
    /// publish plaintext on a lane the client believes is encrypted) and it
    /// must not open (which would accept unauthenticated bytes as commands).
    #[test]
    fn a_transport_without_a_server_refuses_to_open_or_seal() {
        let _home = HomeGuard::new("secure-inert");
        init_store();
        let mut row = creds();
        row.handshake_version = 2;
        row.secure = None;
        let transport = Transport::new(&row);
        assert!(!transport.enabled());
        assert!(transport.seal("presence", b"private").unwrap().is_none());
        match transport.open("command", b"plaintext") {
            Ok(_) => panic!("an inert transport must not open"),
            // `open` has no reason to name: unlike a handshake refusal it never
            // reaches a peer, so the byte-stable prefix is the whole message.
            Err(error) => assert_eq!(error.to_string(), "remote_secure_channel_invalid"),
        }
    }

    #[test]
    fn readiness_is_an_authenticated_commit_and_stop_invalidates_candidates() {
        let _home = HomeGuard::new("secure-readiness");
        init_store();
        let creds = creds();
        let transport = Transport::new(&creds);
        let (private, _) = future_remote_crypto::generate_identity().unwrap();
        let mut old = connect(&transport, &creds, &private, true).unwrap();
        let stored = super::super::pairing::load_creds().unwrap();
        let (mut candidate, ready) =
            prepare_connection(&transport, &stored, &private, false).unwrap();
        let (_, old_reply) = transport
            .open(
                "command",
                &old.seal("command", b"old still serving").unwrap(),
            )
            .unwrap();
        assert!(transport.is_active(&old_reply));
        let (_, premature) = transport
            .open("command", &candidate.seal("command", b"not ready").unwrap())
            .unwrap();
        assert!(!transport.is_active(&premature));
        transport.activate(&ready).unwrap();
        assert!(!transport.is_active(&old_reply));
        assert!(transport.is_active(&ready));
        let (_, pending) = prepare_connection(&transport, &stored, &private, false).unwrap();
        transport.clear();
        assert!(transport.activate(&pending).is_err());
    }
    #[test]
    fn pairing_pins_identity_rejects_reuse_and_recovers_after_restart() {
        let _home = HomeGuard::new("secure-pair");
        init_store();
        let creds = creds();
        let transport = Transport::new(&creds);
        assert!(transport.seal("presence", b"private").unwrap().is_none());
        assert!(transport.open("command", b"plaintext").is_err());
        let (private, public) = future_remote_crypto::generate_identity().unwrap();
        let mut channel = connect(&transport, &creds, &private, true).unwrap();
        let stored = super::super::pairing::load_creds().unwrap();
        assert_eq!(
            stored.secure.as_ref().unwrap().peer_public_key.as_deref(),
            Some(URL_SAFE_NO_PAD.encode(public).as_str())
        );
        assert!(stored.secure.as_ref().unwrap().secret.is_none());
        // This is exactly the stale snapshot held by an in-flight JWT refresh.
        super::super::pairing::save_creds(&creds).unwrap();
        assert!(super::super::pairing::load_creds()
            .unwrap()
            .secure
            .unwrap()
            .secret
            .is_none());
        assert!(connect(&transport, &creds, &private, true).is_err());
        // The refusal names the reason: this is the only place a pairing that
        // can never succeed becomes visible, since the phone only learns that
        // the desktop would not authenticate it.
        assert!(connect(&transport, &creds, &private, true)
            .map(|_| ())
            .unwrap_err()
            .to_string()
            .ends_with("(invitation_already_used)"));
        let wire = channel.seal("command", b"approval").unwrap();
        assert!(transport.open("another-command", &wire).is_err());
        let (plain, reply) = transport.open("command", &wire).unwrap();
        assert_eq!(plain, b"approval");
        assert!(transport.open("command", &wire).is_err());
        let answer = reply.seal(b"approved").unwrap();
        assert_eq!(
            channel
                .open(
                    &future_remote_crypto::reply_context("command", &wire).unwrap(),
                    &answer
                )
                .unwrap(),
            b"approved"
        );
        let restarted = Transport::new(&stored);
        let mut fresh = connect(&restarted, &stored, &private, false).unwrap();
        assert!(restarted.open("command", &wire).is_err());
        let new = fresh.seal("command", b"private").unwrap();
        assert_eq!(restarted.open("command", &new).unwrap().0, b"private");
        let (attacker, _) = future_remote_crypto::generate_identity().unwrap();
        assert!(connect(&restarted, &stored, &attacker, false).is_err());
        // A rejected candidate cannot deactivate the genuine current channel.
        let valid = fresh.seal("command", b"still authenticated").unwrap();
        assert!(restarted.open("command", &valid).is_ok());
        restarted.clear();
        assert!(restarted
            .open("command", &fresh.seal("command", b"after stop").unwrap())
            .is_err());
    }
    #[test]
    fn wrong_secret_expiry_and_persistence_failure_never_activate() {
        let _home = HomeGuard::new("secure-reject");
        init_store();
        let creds = creds();
        let transport = Transport::new(&creds);
        let (private, _) = future_remote_crypto::generate_identity().unwrap();
        let mut wrong = creds.clone();
        wrong.secure.as_mut().unwrap().secret = Some(URL_SAFE_NO_PAD.encode([0; 32]));
        assert!(connect(&transport, &wrong, &private, true).is_err());
        assert!(transport.seal("presence", b"private").unwrap().is_none());
        super::super::pairing::INJECT_SAVE_FAILURE.store(true, std::sync::atomic::Ordering::SeqCst);
        assert!(connect(&transport, &creds, &private, true).is_err());
        assert!(super::super::pairing::load_creds().is_none());
        assert!(transport.seal("presence", b"private").unwrap().is_none());
        let mut expired = creds.clone();
        expired.secure.as_mut().unwrap().expires_at = now_secs() - 1;
        assert!(connect(&Transport::new(&expired), &expired, &private, true).is_err());
        assert!(transport
            .handshake(br#"{"type":"approval_decision","message":""}"#, "bridge")
            .is_err());
        assert!(transport.handshake(&vec![b'x'; 16_385], "bridge").is_err());
    }

    /// The identity is a private key. Its `Debug` must exist (it is derived by
    /// anything that logs the enclosing struct) and must report only whether the
    /// pair is established, never the key material.
    #[test]
    fn a_pairing_identity_never_debugs_its_key_material() {
        let identity = PairingIdentity::new(now_secs() + 60).unwrap();
        let rendered = format!("{identity:?}");
        assert!(rendered.contains("PairingIdentity"));
        assert!(
            rendered.contains("paired: false"),
            "unpaired is the state before the handshake: {rendered}"
        );
        assert!(!rendered.contains(&identity.private_key));
        assert!(!rendered.contains(identity.secret.as_deref().unwrap()));
        let mut pinned = identity.clone();
        pinned.peer_public_key = Some("Upeer".into());
        assert!(format!("{pinned:?}").contains("paired: true"));
    }

    /// Handshake fields are attacker-controlled. `decode` bounds the base64
    /// before it is ever parsed, and a value that is not base64 at all is a
    /// channel error rather than a panic.
    #[test]
    fn handshake_fields_are_bounded_before_decoding() {
        assert!(decode("AAAA").is_ok());
        assert_eq!(
            decode(&"A".repeat(12_001)).unwrap_err().to_string(),
            "remote_secure_channel_invalid"
        );
        assert!(decode("not base64!").is_err());
        assert!(key("AAAA").is_err(), "a short key is rejected, not padded");
    }

    /// A transport that was never handed v2 credentials is inert rather than
    /// panicking: a legacy row can be loaded and the bridge must not treat it as
    /// an authenticated channel.
    #[test]
    fn a_transport_without_a_server_seals_nothing_and_is_never_active() {
        let legacy = Transport::new(&super::super::protocol::PairingCreds {
            handshake_version: 1,
            secure: None,
            ..creds()
        });
        assert!(!legacy.enabled(), "a v1 row has no secure channel to offer");
        assert_eq!(
            legacy.seal("presence", b"legacy lane").unwrap().as_deref(),
            Some(&b"legacy lane"[..]),
            "the v1 lane is plaintext by contract"
        );

        let unpaired = Transport::new(&super::super::protocol::PairingCreds {
            handshake_version: 2,
            secure: None,
            ..creds()
        });
        assert!(!unpaired.enabled());
        assert!(unpaired.seal("presence", b"secret").unwrap().is_none());
        assert!(Transport::default()
            .seal("presence", b"secret")
            .unwrap()
            .is_none());
        let detached = Reply {
            channel: None,
            context: String::new(),
        };
        assert!(!unpaired.is_active(&detached));
        // A well-formed handshake reaches the missing-server guard (the malformed
        // ones are refused earlier, which `a_refused_handshake_names_the_reason`
        // pins), so an unconfigured transport is *inert* rather than panicking.
        assert_eq!(
            unpaired
                .handshake(br#"{"type":"secure_open","message":"AAAA"}"#, "bridge")
                .unwrap_err()
                .to_string(),
            format!("{REFUSED} (disabled)")
        );
    }

    /// Re-activating the channel that is already current is a no-op, not an
    /// error: a phone that resends its readiness (after a lost reply) must not
    /// be told its own active channel is unknown.
    #[test]
    fn activating_the_current_channel_twice_is_idempotent() {
        let _home = HomeGuard::new("secure-reactivate");
        init_store();
        let creds = creds();
        let transport = Transport::new(&creds);
        let (private, _) = future_remote_crypto::generate_identity().unwrap();
        let (_channel, reply) = prepare_connection(&transport, &creds, &private, true).unwrap();
        transport.activate(&reply).unwrap();
        transport.activate(&reply).unwrap();
        assert!(transport.is_active(&reply));
        // A reply that never opened a channel cannot be activated.
        assert_eq!(
            transport
                .activate(&Reply {
                    channel: None,
                    context: String::new()
                })
                .unwrap_err()
                .to_string(),
            "remote_secure_channel_invalid"
        );
    }

    /// An opening that names no `type` — or a type this transport does not
    /// implement — is refused with the two distinct reasons the phone's console
    /// prints. A client whose body was cut mid-write sends the empty one.
    #[test]
    fn an_opening_without_a_known_type_is_refused_by_name() {
        let _home = HomeGuard::new("secure-missing-type");
        init_store();
        let transport = Transport::new(&creds());
        let reason = |payload: &str| {
            transport
                .handshake(payload.as_bytes(), "bridge_secure")
                .unwrap_err()
                .to_string()
        };
        assert_eq!(
            reason(r#"{"type":"","message":"AAAA"}"#),
            format!("{REFUSED} (missing_type)")
        );
        // A reconnect by a phone this desktop has never paired with has no
        // pinned key to check the noise proof against.
        assert_eq!(
            reason(r#"{"type":"secure_open","pairing":false,"message":"AAAA"}"#),
            format!("{REFUSED} (peer_unknown)")
        );
    }

    /// The challenge table is bounded: a relay that opens connections forever
    /// must not grow the desktop's memory, and the refusal has to be a named
    /// error the phone can show rather than a silent drop.
    #[test]
    fn an_opening_flood_is_refused_once_the_challenge_table_is_full() {
        let _home = HomeGuard::new("secure-busy");
        init_store();
        let creds = creds();
        let transport = Transport::new(&creds);
        let (private, _) = future_remote_crypto::generate_identity().unwrap();
        let open = {
            let identity = creds.secure.as_ref().unwrap();
            let prologue =
                future_remote_crypto::prologue(&creds.pair_id, &creds.desktop_id).unwrap();
            let psk = key(identity.secret.as_deref().unwrap()).unwrap();
            let mut noise =
                Handshake::new(Pattern::Pair, true, &private, None, Some(&psk), &prologue).unwrap();
            json!({ "type": "secure_open", "pairing": true, "message": URL_SAFE_NO_PAD.encode(noise.write(b"").unwrap()) })
        };
        let open = serde_json::to_vec(&open).unwrap();
        for attempt in 0..16 {
            assert!(
                transport.handshake(&open, "bridge_secure").is_ok(),
                "challenge {attempt} is admitted"
            );
        }
        assert_eq!(
            transport
                .handshake(&open, "bridge_secure")
                .unwrap_err()
                .to_string(),
            format!("{REFUSED} (busy)")
        );
    }

    /// The two payload guards on a handshake: the opening message must carry no
    /// application bytes, and the finish must carry none either. A client that
    /// smuggles data alongside the noise proof is refused rather than having it
    /// silently ignored.
    #[test]
    fn a_handshake_that_carries_application_bytes_is_refused() {
        let _home = HomeGuard::new("secure-open-payload");
        init_store();
        let creds = creds();
        let transport = Transport::new(&creds);
        let (private, _) = future_remote_crypto::generate_identity().unwrap();
        let identity = creds.secure.as_ref().unwrap();
        let psk = key(identity.secret.as_deref().unwrap()).unwrap();
        let prologue = future_remote_crypto::prologue(&creds.pair_id, &creds.desktop_id).unwrap();
        let mut noise =
            Handshake::new(Pattern::Pair, true, &private, None, Some(&psk), &prologue).unwrap();
        let open = json!({
            "type": "secure_open",
            "pairing": true,
            "message": URL_SAFE_NO_PAD.encode(noise.write(b"smuggled").unwrap()),
        });
        assert_eq!(
            transport
                .handshake(&serde_json::to_vec(&open).unwrap(), "bridge_secure")
                .unwrap_err()
                .to_string(),
            format!("{REFUSED} (open_payload)")
        );

        // A clean opening is admitted, and then the *finish* carries the bytes.
        let mut noise =
            Handshake::new(Pattern::Pair, true, &private, None, Some(&psk), &prologue).unwrap();
        let open = json!({
            "type": "secure_open",
            "pairing": true,
            "message": URL_SAFE_NO_PAD.encode(noise.write(b"").unwrap()),
        });
        let response = transport
            .handshake(&serde_json::to_vec(&open).unwrap(), "bridge_secure")
            .unwrap();
        noise
            .read(&decode(response["message"].as_str().unwrap()).unwrap())
            .unwrap();
        let finish = json!({
            "type": "secure_finish",
            "id": response["id"],
            "message": URL_SAFE_NO_PAD.encode(noise.write(b"smuggled").unwrap()),
        });
        assert_eq!(
            transport
                .handshake(&serde_json::to_vec(&finish).unwrap(), "bridge_secure")
                .unwrap_err()
                .to_string(),
            format!("{REFUSED} (finish_payload)")
        );
    }

    /// A challenge that is answered after a *different* socket already pinned
    /// this pair — and one that is answered after the invitation's own expiry —
    /// must not silently rebind the desktop to a second key.
    #[test]
    fn a_stale_challenge_cannot_bind_a_second_key() {
        let _home = HomeGuard::new("secure-stale-challenge");
        init_store();
        let creds = creds();
        let transport = Transport::new(&creds);
        let (private, _) = future_remote_crypto::generate_identity().unwrap();
        let identity = creds.secure.as_ref().unwrap();
        let psk = key(identity.secret.as_deref().unwrap()).unwrap();
        let prologue = future_remote_crypto::prologue(&creds.pair_id, &creds.desktop_id).unwrap();
        let challenge = |transport: &Transport| {
            let mut noise =
                Handshake::new(Pattern::Pair, true, &private, None, Some(&psk), &prologue).unwrap();
            let open = json!({
                "type": "secure_open",
                "pairing": true,
                "message": URL_SAFE_NO_PAD.encode(noise.write(b"").unwrap()),
            });
            let response = transport
                .handshake(&serde_json::to_vec(&open).unwrap(), "bridge_secure")
                .unwrap();
            noise
                .read(&decode(response["message"].as_str().unwrap()).unwrap())
                .unwrap();
            (noise, response["id"].clone())
        };
        let message_for = |noise: &mut Handshake, id: &Value| {
            json!({
                "type": "secure_finish",
                "id": id,
                "message": URL_SAFE_NO_PAD.encode(noise.write(b"").unwrap()),
            })
        };

        // Another socket pinned the key while this challenge was outstanding.
        let (mut noise, id) = challenge(&transport);
        let finish = message_for(&mut noise, &id);
        transport
            .server
            .as_ref()
            .unwrap()
            .lock()
            .unwrap()
            .creds
            .secure
            .as_mut()
            .unwrap()
            .peer_public_key = Some("Uanother".into());
        assert_eq!(
            transport
                .handshake(&serde_json::to_vec(&finish).unwrap(), "bridge_secure")
                .unwrap_err()
                .to_string(),
            format!("{REFUSED} (invitation_already_used)")
        );

        // The invitation expired while the phone was scanning.
        let expired = Transport::new(&creds);
        let (mut noise, id) = challenge(&expired);
        let finish = message_for(&mut noise, &id);
        expired
            .server
            .as_ref()
            .unwrap()
            .lock()
            .unwrap()
            .creds
            .secure
            .as_mut()
            .unwrap()
            .expires_at = now_secs() - 1;
        assert_eq!(
            expired
                .handshake(&serde_json::to_vec(&finish).unwrap(), "bridge_secure")
                .unwrap_err()
                .to_string(),
            format!("{REFUSED} (invitation_expired)")
        );
    }
}
