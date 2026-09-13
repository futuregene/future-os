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
            return Err(error("length"));
        }
        let request: HandshakeRequest = serde_json::from_slice(payload).map_err(error)?;
        let mut server = self
            .server
            .as_ref()
            .ok_or_else(|| error("disabled"))?
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
            .ok_or_else(|| error("identity"))?
            .clone();
        let prologue =
            future_remote_crypto::prologue(&server.creds.pair_id, &server.creds.desktop_id)
                .map_err(error)?;
        let input = decode(&request.message)?;
        match request.kind.as_str() {
            "secure_open" => {
                if server.pending.len() + server.candidates.len() >= 16 {
                    return Err(error("busy"));
                }
                let pairing = request.pairing;
                let psk = if pairing {
                    if identity.peer_public_key.is_some()
                        || super::unix_timestamp() >= identity.expires_at.max(0) as u64
                    {
                        return Err(error("invitation"));
                    }
                    Some(key(identity
                        .secret
                        .as_deref()
                        .ok_or_else(|| error("secret"))?)?)
                } else {
                    None
                };
                if !pairing && identity.peer_public_key.is_none() {
                    return Err(error("peer"));
                }
                let mut noise = Handshake::new(
                    if pairing {
                        Pattern::Pair
                    } else {
                        Pattern::Reconnect
                    },
                    false,
                    &key(&identity.private_key)?,
                    None,
                    psk.as_ref(),
                    &prologue,
                )
                .map_err(error)?;
                if !noise.read(&input).map_err(error)?.is_empty() {
                    return Err(error("payload"));
                }
                if !pairing
                    && noise.remote_public_key().map_err(error)?
                        != key(identity
                            .peer_public_key
                            .as_deref()
                            .ok_or_else(|| error("peer"))?)?
                {
                    return Err(error("peer"));
                }
                let response = noise.write(b"").map_err(error)?;
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
                    let channel = Arc::new(Mutex::new(noise.finish().map_err(error)?));
                    let confirmation = confirmation(&server.creds.pair_id, bridge, &channel)?;
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
                    .ok_or_else(|| error("expired"))?;
                if identity.peer_public_key.is_some()
                    || super::unix_timestamp() >= identity.expires_at.max(0) as u64
                {
                    return Err(error("invitation"));
                }
                if !pending.noise.read(&input).map_err(error)?.is_empty() {
                    return Err(error("payload"));
                }
                let peer =
                    URL_SAFE_NO_PAD.encode(pending.noise.remote_public_key().map_err(error)?);
                let channel = Arc::new(Mutex::new(pending.noise.finish().map_err(error)?));
                let mut creds = server.creds.clone();
                let identity = creds.secure.as_mut().ok_or_else(|| error("identity"))?;
                identity.peer_public_key = Some(peer);
                identity.secret = None;
                // Persist binding before accepting any command. A lost response
                // can recover through IK using the phone's already stored key.
                super::pairing::save_creds(&creds)?;
                let confirmation = confirmation(&creds.pair_id, bridge, &channel)?;
                server.creds = creds;
                server.pending.clear();
                server.candidates.push(Candidate {
                    channel,
                    created: Instant::now(),
                });
                Ok(json!({ "confirmation": confirmation }))
            }
            _ => Err(error("type")),
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
        "features": ["e2ee_v2", "file_transfer_v1", "file_download_v2", "approval_tier_v1", "continue_run_v1", "prompt_receipt_v1", "session_files_v1"],
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
) -> Result<(), crate::AppError> {
    if let Some(wire) = transport.seal(&subject, &bytes)? {
        client
            .publish(subject, wire.into())
            .await
            .map_err(|e| crate::AppError::RemoteTransport(e.to_string()))?;
    }
    Ok(())
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
}
