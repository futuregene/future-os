//! Services supplied by the application that owns a controller instance.
use crate::{
    credentials::{PeerBook, PeerCreds},
    ConnectionOptions, Error,
};
use future_remote_protocol::invitation::Invitation;
use futures::future::BoxFuture;

pub struct Claimed {
    pub pair_id: String,
    pub user_jwt: String,
    pub refresh_token: String,
    pub nats_url: Option<String>,
    pub nats_ws_url: String,
    pub jwt_expires_at: i64,
}

impl std::fmt::Debug for Claimed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Claimed")
            .field("pair_id", &self.pair_id)
            .field("user_jwt", &"<redacted>")
            .field("refresh_token", &"<redacted>")
            .field("nats_url", &self.nats_url)
            .field("nats_ws_url", &self.nats_ws_url)
            .field("jwt_expires_at", &self.jwt_expires_at)
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct Refreshed {
    pub user_jwt: String,
    pub nats_url: String,
    pub nats_ws_url: String,
    pub jwt_expires_at: i64,
}

impl std::fmt::Debug for Refreshed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Refreshed")
            .field("jwt_expires_at", &self.jwt_expires_at)
            .finish_non_exhaustive()
    }
}

pub trait ClientHost: Send + Sync {
    fn device_id(&self) -> Result<String, Error>;
    fn connection_options(&self) -> ConnectionOptions;
    fn command_id(&self) -> String;
    fn load(&self) -> Result<PeerBook, Error>;
    fn upsert(&self, creds: PeerCreds) -> Result<(), Error>;
    /// Atomically update only the grant that produced the refresh response.
    fn update_credentials(&self, expected: &PeerCreds, fresh: &Refreshed) -> Result<bool, Error>;
    fn clear_secret(&self, expected: &PeerCreds) -> Result<bool, Error>;
    fn remove(&self, expected: &PeerCreds) -> Result<bool, Error>;
    fn set_label(
        &self,
        desktop_id: &str,
        name: Option<&str>,
        icon: Option<&str>,
    ) -> Result<(), Error>;
    fn claim<'a>(
        &'a self,
        invitation: &'a Invitation,
        device_id: &'a str,
        public_key: &'a str,
        device_name: &'a str,
    ) -> BoxFuture<'a, Result<Claimed, Error>>;
    fn refresh<'a>(&'a self, creds: &'a PeerCreds) -> BoxFuture<'a, Result<Refreshed, Error>>;
    fn revoke<'a>(&'a self, creds: &'a PeerCreds) -> BoxFuture<'a, Result<(), Error>>;
    /// Register the future with the host's own runtime/shutdown registry.
    fn spawn(&self, future: BoxFuture<'static, ()>);
}
