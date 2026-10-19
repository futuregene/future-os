//! Desktop services for the shared controlled-endpoint handshake.
use super::protocol::PairingCreds;
use future_remote_server::secure::HandshakeHost;
pub use future_remote_server::secure::PairingIdentity;
pub(super) use future_remote_server::secure::{Reply, REPLY};
use serde_json::Value;
use std::sync::Arc;

struct DesktopHandshakeHost;
impl HandshakeHost for DesktopHandshakeHost {
    fn save_binding(&self, creds: &PairingCreds) -> Result<(), future_remote_server::Error> {
        super::pairing::save_creds(creds)
            .map_err(|error| future_remote_server::Error::Message(error.to_string()))
    }
    fn presence(&self, pair: &str, bridge: &str) -> Value {
        super::build_presence_payload(pair, bridge)
    }
}
#[derive(Clone, Default)]
pub(super) struct Transport {
    pub(super) inner: future_remote_server::secure::Transport,
}
impl Transport {
    pub fn new(creds: &PairingCreds) -> Self {
        Self {
            inner: future_remote_server::secure::Transport::new(
                creds,
                Arc::new(DesktopHandshakeHost),
            ),
        }
    }
    #[cfg(test)]
    pub fn legacy_fixture() -> Self {
        Self {
            inner: future_remote_server::secure::Transport::legacy_fixture(),
        }
    }
    pub fn enabled(&self) -> bool {
        self.inner.enabled()
    }
    pub fn clear(&self) {
        self.inner.clear()
    }
    pub fn seal(&self, context: &str, bytes: &[u8]) -> Result<Option<Vec<u8>>, crate::AppError> {
        self.inner.seal(context, bytes).map_err(Into::into)
    }
    pub fn open(&self, subject: &str, bytes: &[u8]) -> Result<(Vec<u8>, Reply), crate::AppError> {
        self.inner.open(subject, bytes).map_err(Into::into)
    }
    pub fn is_active(&self, reply: &Reply) -> bool {
        self.inner.is_active(reply)
    }
    pub fn activate(&self, reply: &Reply) -> Result<(), crate::AppError> {
        self.inner.activate(reply).map_err(Into::into)
    }
    pub fn handshake(&self, bytes: &[u8], bridge: &str) -> Result<Value, crate::AppError> {
        self.inner.handshake(bytes, bridge).map_err(Into::into)
    }
}
pub(super) async fn publish(
    client: &async_nats::Client,
    transport: &Transport,
    subject: String,
    bytes: Vec<u8>,
) -> Result<bool, crate::AppError> {
    future_remote_server::secure::publish(client, &transport.inner, subject, bytes)
        .await
        .map_err(Into::into)
}
#[cfg(test)]
mod tests {

    use super::*;
    use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
    use future_remote_crypto::{Channel, Handshake, Pattern};
    use serde_json::{json, Value};
    fn error(_: impl std::fmt::Display) -> crate::AppError {
        crate::AppError::Message("remote_secure_channel_invalid".into())
    }
    fn decode(value: &str) -> Result<Vec<u8>, crate::AppError> {
        URL_SAFE_NO_PAD.decode(value).map_err(error)
    }
    fn key(value: &str) -> Result<[u8; 32], crate::AppError> {
        decode(value)?.try_into().map_err(|_| error("key"))
    }
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
            control_url: None,
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
}
