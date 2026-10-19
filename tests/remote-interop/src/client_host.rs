use future_remote_client::{
    credentials::{PeerBook, PeerCreds, PeerLabel, PeerRecord},
    host::{Claimed, ClientHost, Refreshed},
    ConnectionOptions, Error,
};
use future_remote_protocol::invitation::Invitation;
use futures::future::BoxFuture;
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Mutex,
};

pub struct MemoryClientHost {
    book: Mutex<PeerBook>,
    sequence: AtomicU64,
    connects: AtomicU64,
}
impl MemoryClientHost {
    pub fn new(creds: PeerCreds) -> Self {
        Self {
            book: Mutex::new(PeerBook {
                peers: vec![PeerRecord {
                    creds,
                    label: PeerLabel::default(),
                }],
            }),
            sequence: AtomicU64::new(0),
            connects: AtomicU64::new(0),
        }
    }
    pub fn connection_attempts(&self) -> u64 {
        self.connects.load(Ordering::SeqCst)
    }
}
impl ClientHost for MemoryClientHost {
    fn device_id(&self) -> Result<String, Error> {
        Ok("interop_client".into())
    }
    fn connection_options(&self) -> ConnectionOptions {
        self.connects.fetch_add(1, Ordering::SeqCst);
        ConnectionOptions::new("Standalone Rust", "test").for_loopback_fixture()
    }
    fn command_id(&self) -> String {
        format!(
            "rust_command_{}",
            self.sequence.fetch_add(1, Ordering::Relaxed)
        )
    }
    fn load(&self) -> Result<PeerBook, Error> {
        Ok(self.book.lock().unwrap().clone())
    }
    fn upsert(&self, creds: PeerCreds) -> Result<(), Error> {
        let mut book = self.book.lock().unwrap();
        book.peers
            .retain(|peer| peer.creds.desktop_id != creds.desktop_id);
        book.peers.push(PeerRecord {
            creds,
            label: PeerLabel::default(),
        });
        Ok(())
    }
    fn update_credentials(&self, expected: &PeerCreds, fresh: &Refreshed) -> Result<bool, Error> {
        let mut book = self.book.lock().unwrap();
        let Some(peer) = book.peers.iter_mut().find(|peer| peer.creds == *expected) else {
            return Ok(false);
        };
        peer.creds.user_jwt.clone_from(&fresh.user_jwt);
        peer.creds.nats_url.clone_from(&fresh.nats_url);
        peer.creds.nats_ws_url.clone_from(&fresh.nats_ws_url);
        peer.creds.jwt_expires_at = fresh.jwt_expires_at;
        Ok(true)
    }
    fn clear_secret(&self, expected: &PeerCreds) -> Result<bool, Error> {
        let mut book = self.book.lock().unwrap();
        let Some(peer) = book.peers.iter_mut().find(|peer| peer.creds == *expected) else {
            return Ok(false);
        };
        if let Some(identity) = peer.creds.secure.as_mut() {
            identity.secret = None;
        }
        Ok(true)
    }
    fn remove(&self, expected: &PeerCreds) -> Result<bool, Error> {
        let mut book = self.book.lock().unwrap();
        let count = book.peers.len();
        book.peers.retain(|peer| peer.creds != *expected);
        Ok(book.peers.len() != count)
    }
    fn set_label(&self, _: &str, _: Option<&str>, _: Option<&str>) -> Result<(), Error> {
        Ok(())
    }
    fn claim<'a>(
        &'a self,
        _: &'a Invitation,
        _: &'a str,
        _: &'a str,
        _: &'a str,
    ) -> BoxFuture<'a, Result<Claimed, Error>> {
        Box::pin(async {
            Err(Error::Message(
                "fixture does not contact an account service".into(),
            ))
        })
    }
    fn refresh<'a>(&'a self, creds: &'a PeerCreds) -> BoxFuture<'a, Result<Refreshed, Error>> {
        Box::pin(async move {
            Ok(Refreshed {
                user_jwt: creds.user_jwt.clone(),
                nats_url: creds.nats_url.clone(),
                nats_ws_url: creds.nats_ws_url.clone(),
                jwt_expires_at: creds.jwt_expires_at,
            })
        })
    }
    fn revoke<'a>(&'a self, _: &'a PeerCreds) -> BoxFuture<'a, Result<(), Error>> {
        Box::pin(async { Ok(()) })
    }
    fn spawn(&self, future: BoxFuture<'static, ()>) {
        tokio::spawn(future);
    }
}
