//! Independent real Remote endpoints with explicit in-memory business services.
//! No Desktop, Agent, HOME, account service or application test environment.
#![cfg(feature = "network-tests")]

mod broker;
mod client_host;
pub mod contract;
mod server_host;
pub use broker::Broker;
pub use client_host::MemoryClientHost;
pub use server_host::MemoryServerHost;

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use future_remote_client::credentials::{PeerCreds, PeerIdentity};
use future_remote_server::{secure::PairingIdentity, supervisor::Supervisor, PairingCreds};
use serde_json::{json, Value};
use std::{sync::Arc, time::Duration};

pub const DESKTOP: &str = "interop_desktop";
pub const PAIR: &str = "interop_pair";
pub const CHUNK_BYTES: usize = 4;

pub struct Fixture {
    pub server: Arc<Supervisor>,
    pub host: Arc<MemoryServerHost>,
    pub client_credentials: PeerCreds,
    pub desktop_nkey: String,
}
impl Fixture {
    pub async fn start(nats_url: &str, websocket_url: &str) -> anyhow::Result<Self> {
        Self::start_with_ids(nats_url, websocket_url, DESKTOP, PAIR).await
    }
    pub async fn start_named(
        nats_url: &str,
        websocket_url: &str,
        label: &str,
    ) -> anyhow::Result<Self> {
        anyhow::ensure!(
            !label.is_empty()
                && label.len() <= 32
                && label
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_'),
            "invalid fixture label"
        );
        Self::start_with_ids(
            nats_url,
            websocket_url,
            &format!("interop_{label}"),
            &format!("pair_{label}"),
        )
        .await
    }
    async fn start_with_ids(
        nats_url: &str,
        websocket_url: &str,
        desktop: &str,
        pair: &str,
    ) -> anyhow::Result<Self> {
        for endpoint in [nats_url, websocket_url] {
            let parsed = url::Url::parse(endpoint)?;
            anyhow::ensure!(
                parsed.host_str() == Some("127.0.0.1"),
                "fixture endpoints must be IPv4 loopback"
            );
        }
        let expires = future_remote_server::supervisor::unix_timestamp() as i64 + 3600;
        // These are ephemeral fixture keys. They only cross private harness IPC,
        // never test reports, command arguments, snapshots or error formatting.
        let identity = PairingIdentity::new(expires)?;
        let (private, public) = future_remote_crypto::generate_identity()?;
        let desktop_key = nkeys::KeyPair::new_user();
        let client_key = nkeys::KeyPair::new_user();
        let jwt = format!(
            "e30.{}.fixture",
            URL_SAFE_NO_PAD.encode(serde_json::to_vec(&json!({"exp": expires}))?)
        );
        let client_credentials = PeerCreds {
            pair_id: pair.into(),
            desktop_id: desktop.into(),
            device_id: "interop_client".into(),
            nkey_seed: client_key.seed()?,
            user_jwt: jwt.clone(),
            refresh_token: "fixture_refresh".into(),
            nats_url: nats_url.into(),
            nats_ws_url: websocket_url.into(),
            jwt_expires_at: expires,
            token_url: "https://fixture.invalid/auth/token".into(),
            secure: Some(PeerIdentity {
                private_key: URL_SAFE_NO_PAD.encode(private.as_slice()),
                public_key: URL_SAFE_NO_PAD.encode(public),
                peer_public_key: Some(identity.public_key.clone()),
                secret: identity.secret.clone(),
            }),
        };
        let creds = PairingCreds {
            handshake_version: 2,
            secure: Some(identity),
            pair_id: pair.into(),
            desktop_id: desktop.into(),
            nkey_seed: desktop_key.seed()?,
            user_jwt: jwt,
            nats_url: nats_url.into(),
            nats_ws_url: websocket_url.into(),
            jwt_expires_at: expires,
            control_url: None,
        };
        let host = Arc::new(MemoryServerHost::new(creds, contract::suite()?.file_bytes));
        let server = Supervisor::new(host.clone(), Arc::new(server_host::Executor));
        let status = server.start().await?;
        anyhow::ensure!(
            status.phase == future_remote_server::types::RemotePhase::Ready,
            "standalone Server did not become connected"
        );
        Ok(Self {
            server,
            host,
            client_credentials,
            desktop_nkey: desktop_key.public_key(),
        })
    }
    pub fn emit(&self) -> anyhow::Result<()> {
        self.emit_data(&contract::suite()?.event.data)
    }
    pub fn emit_data(&self, data: &str) -> anyhow::Result<()> {
        let event = contract::suite()?.event;
        self.server.publish_event(
            &event.session_id,
            &event.kind,
            data,
            &event.run_id,
            event.idx,
            event.epoch,
            &event.event_id,
            &event.timestamp,
            event.session_idx,
            event.run_sequence,
        );
        Ok(())
    }
    pub fn metrics(&self) -> Value {
        self.host.metrics()
    }
    pub async fn shutdown(&self) -> anyhow::Result<()> {
        self.server.shutdown(Duration::from_secs(5)).await?;
        Ok(())
    }
}
