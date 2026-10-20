use super::*;
use crate::credentials::{PeerBook, PeerCreds, PeerRecord};
use crate::host::{Claimed, Refreshed};
use futures::future::BoxFuture;
use std::sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering},
    Mutex as StdMutex,
};
use tokio::sync::Notify;

struct MemoryHost {
    book: StdMutex<PeerBook>,
    entered: Notify,
    release: Notify,
    updates: AtomicUsize,
    refreshes: AtomicUsize,
    finished_refreshes: AtomicUsize,
    reject_tasks: AtomicBool,
    reject_load: AtomicBool,
    rejected_loads: AtomicUsize,
    /// This installation's own device id. The default is deliberately *not* a
    /// `desktop_` id, so the invitations the other tests use (which must name a
    /// `desktop_…` host) can never look like this machine's own.
    own_device: StdMutex<String>,
}
impl Default for MemoryHost {
    fn default() -> Self {
        Self {
            book: StdMutex::default(),
            entered: Notify::new(),
            release: Notify::new(),
            updates: AtomicUsize::default(),
            refreshes: AtomicUsize::default(),
            finished_refreshes: AtomicUsize::default(),
            reject_tasks: AtomicBool::default(),
            reject_load: AtomicBool::default(),
            rejected_loads: AtomicUsize::default(),
            own_device: StdMutex::new("device_fixture".to_string()),
        }
    }
}
impl MemoryHost {
    /// Pretend this installation is the desktop `device`, which is how a test
    /// arranges an invitation this machine issued itself.
    fn own_device(self, device: &str) -> Self {
        *self.own_device.lock().unwrap() = device.to_string();
        self
    }
}

struct RefreshCompletion<'a>(&'a AtomicUsize);
impl Drop for RefreshCompletion<'_> {
    fn drop(&mut self) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}
fn creds() -> PeerCreds {
    PeerCreds {
        pair_id: "pair_fixture".into(),
        desktop_id: "desktop_fixture".into(),
        device_id: "device_fixture".into(),
        nkey_seed: nkeys::KeyPair::new_user().seed().unwrap(),
        user_jwt: "old-token-fixture".into(),
        refresh_token: "refresh-fixture".into(),
        nats_url: "nats://127.0.0.1:1".into(),
        nats_ws_url: "wss://example.invalid".into(),
        jwt_expires_at: 1,
        token_url: "https://example.invalid/auth/token".into(),
        secure: None,
    }
}
impl ClientHost for MemoryHost {
    fn device_id(&self) -> Result<String, crate::Error> {
        Ok(self.own_device.lock().unwrap().clone())
    }
    fn connection_options(&self) -> crate::ConnectionOptions {
        crate::ConnectionOptions::new("fixture", "cli")
    }
    fn command_id(&self) -> String {
        "cmd_fixture".into()
    }
    fn load(&self) -> Result<PeerBook, crate::Error> {
        if self.reject_load.load(Ordering::SeqCst) {
            self.rejected_loads.fetch_add(1, Ordering::SeqCst);
            return Err(crate::Error::RemoteTransport("fixture unavailable".into()));
        }
        Ok(self.book.lock().unwrap().clone())
    }
    fn upsert(&self, creds: PeerCreds) -> Result<(), crate::Error> {
        self.book.lock().unwrap().peers.push(PeerRecord {
            creds,
            label: Default::default(),
        });
        Ok(())
    }
    fn update_credentials(
        &self,
        expected: &PeerCreds,
        fresh: &Refreshed,
    ) -> Result<bool, crate::Error> {
        self.updates.fetch_add(1, Ordering::SeqCst);
        let mut book = self.book.lock().unwrap();
        let Some(record) = book
            .peers
            .iter_mut()
            .find(|record| record.creds.pair_id == expected.pair_id)
        else {
            return Ok(false);
        };
        record.creds.user_jwt = fresh.user_jwt.clone();
        Ok(true)
    }
    fn clear_secret(&self, _: &PeerCreds) -> Result<bool, crate::Error> {
        Ok(true)
    }
    fn remove(&self, expected: &PeerCreds) -> Result<bool, crate::Error> {
        let mut book = self.book.lock().unwrap();
        let before = book.peers.len();
        book.peers.retain(|r| r.creds.pair_id != expected.pair_id);
        Ok(book.peers.len() != before)
    }
    fn set_label(&self, _: &str, _: Option<&str>, _: Option<&str>) -> Result<(), crate::Error> {
        Ok(())
    }
    fn claim<'a>(
        &'a self,
        _: &'a future_remote_protocol::invitation::Invitation,
        _: &'a str,
        _: &'a str,
        _: &'a str,
    ) -> BoxFuture<'a, Result<Claimed, crate::Error>> {
        async move {
            self.entered.notify_one();
            self.release.notified().await;
            Ok(Claimed {
                pair_id: "pair_fixture".into(),
                user_jwt: "token-fixture".into(),
                refresh_token: "refresh-fixture".into(),
                nats_url: Some("nats://127.0.0.1:1".into()),
                nats_ws_url: "wss://example.invalid".into(),
                jwt_expires_at: 4_102_444_800,
            })
        }
        .boxed()
    }
    fn refresh<'a>(&'a self, _: &'a PeerCreds) -> BoxFuture<'a, Result<Refreshed, crate::Error>> {
        async move {
            self.refreshes.fetch_add(1, Ordering::SeqCst);
            let _completion = RefreshCompletion(&self.finished_refreshes);
            self.entered.notify_one();
            self.release.notified().await;
            Ok(Refreshed {
                user_jwt: "new-token-fixture".into(),
                nats_url: "nats://127.0.0.1:1".into(),
                nats_ws_url: "wss://example.invalid".into(),
                jwt_expires_at: 4_102_444_800,
            })
        }
        .boxed()
    }
    fn revoke<'a>(&'a self, _: &'a PeerCreds) -> BoxFuture<'a, Result<(), crate::Error>> {
        async { Ok(()) }.boxed()
    }
    fn spawn(&self, future: BoxFuture<'static, ()>) {
        if self.reject_tasks.load(Ordering::SeqCst) {
            drop(future);
        } else {
            drop(tokio::spawn(future));
        }
    }
}

#[tokio::test]
async fn two_instances_share_no_connection_intent_or_stop_state() {
    let host = Arc::new(MemoryHost::default());
    host.upsert(creds()).unwrap();
    let first = Client::new(host.clone());
    let second = Client::new(host);
    first.disconnect("desktop_fixture").await;
    assert!(first.list().await.unwrap()[0].stopped);
    assert!(!second.list().await.unwrap()[0].stopped);
    first.shutdown(Duration::from_secs(1)).await.unwrap();
    assert!(first.is_stopped());
    assert!(!second.is_stopped());
    assert_eq!(second.list().await.unwrap().len(), 1);
    second.shutdown(Duration::from_secs(1)).await.unwrap();
}

struct Dropped(Arc<AtomicBool>);
impl Drop for Dropped {
    fn drop(&mut self) {
        self.0.store(true, Ordering::SeqCst);
    }
}

#[tokio::test]
async fn shutdown_waits_for_owned_future_destruction() {
    let client = Client::new(Arc::new(MemoryHost::default()));
    let dropped = Arc::new(AtomicBool::new(false));
    let entered = Arc::new(Notify::new());
    let guard = Dropped(dropped.clone());
    let ready = entered.clone();
    let task = client.spawn(async move {
        let _guard = guard;
        ready.notify_one();
        std::future::pending::<()>().await;
    });
    tokio::time::timeout(Duration::from_secs(5), entered.notified())
        .await
        .expect("task started");
    client.shutdown(Duration::from_secs(1)).await.unwrap();
    assert!(dropped.load(Ordering::SeqCst));
    assert!(task.is_finished());
    assert!(task.await.is_err());
    assert_eq!(
        client
            .connect("desktop_fixture")
            .await
            .unwrap_err()
            .to_string(),
        "remote_client_stopped"
    );
}

#[tokio::test]
async fn rejected_unpolled_tasks_also_finish_and_drop_their_captures() {
    let host = Arc::new(MemoryHost::default());
    host.reject_tasks.store(true, Ordering::SeqCst);
    let client = Client::new(host);
    let dropped = Arc::new(AtomicBool::new(false));
    let guard = Dropped(dropped.clone());
    let task = client.spawn(async move {
        let _guard = guard;
        std::future::pending::<()>().await;
    });
    assert!(task.is_finished());
    assert!(dropped.load(Ordering::SeqCst));
    client.shutdown(Duration::from_secs(1)).await.unwrap();
}

/// Pairing a machine with itself is refused before anything is claimed.
///
/// The invitation carries the host's desktop id, and this process's own device id
/// is exactly what that host would write there for itself. The claim used to go
/// through: the platform counted a device against this desktop's allowance (five,
/// for the whole account), and the Remote page gained a row for a "remote" machine
/// that was this one — with a second bridge to the same rooms.
#[tokio::test]
async fn an_invitation_issued_by_this_machine_is_refused_before_claiming() {
    // The platform only mints `desktop_…` ids, so this is what a machine's own
    // invitation names when the host side and the client side are one install.
    let host = Arc::new(MemoryHost::default().own_device("desktop_self_fixture"));
    let client = Client::new(host.clone());
    let code = URL_SAFE_NO_PAD.encode(
        json!({"v":2,"nonce":"fixture","claim_url":"https://example.invalid/pair/claim","exp":4_102_444_800u64})
            .to_string(),
    );
    let key = URL_SAFE_NO_PAD.encode([7u8; 32]);
    // `MemoryHost` reports `desktop_self_fixture` as this installation, so this is
    // the machine's own invitation.
    let invitation = format!(
        "futureos://remote/pair?v=2&code={code}&desktopId=desktop_self_fixture&desktopKey=UFIXTURE&secureKey={key}&secret={key}"
    );

    let error = client
        .pair_with_emitter(&invitation, None)
        .await
        .expect_err("a machine must not pair with itself");
    assert!(error.to_string().contains("remote_self_pairing"), "{error}");
    assert!(
        host.load().unwrap().peers.is_empty(),
        "nothing may be stored for a refused pairing"
    );
    client.shutdown(Duration::from_secs(1)).await.unwrap();
}

#[tokio::test]
async fn shutdown_cancels_a_pending_claim_before_credentials_are_persisted() {
    let host = Arc::new(MemoryHost::default());
    let client = Client::new(host.clone());
    let caller = client.clone();
    let code = URL_SAFE_NO_PAD.encode(json!({"v":2,"nonce":"fixture","claim_url":"https://example.invalid/pair/claim","exp":4_102_444_800u64}).to_string());
    let key = URL_SAFE_NO_PAD.encode([7u8; 32]);
    let invitation = format!("futureos://remote/pair?v=2&code={code}&desktopId=desktop_fixture&desktopKey=UFIXTURE&secureKey={key}&secret={key}");
    let pending = tokio::spawn(async move { caller.pair_with_emitter(&invitation, None).await });
    tokio::time::timeout(Duration::from_secs(5), host.entered.notified())
        .await
        .expect("host service entered");
    client.shutdown(Duration::from_secs(1)).await.unwrap();
    let error = tokio::time::timeout(Duration::from_secs(1), pending)
        .await
        .unwrap()
        .unwrap()
        .unwrap_err();
    assert_eq!(error.to_string(), "remote_client_stopped");
    assert!(host.load().unwrap().peers.is_empty());
}

#[tokio::test]
async fn disconnect_prevents_a_pending_refresh_from_replacing_the_grant() {
    let host = Arc::new(MemoryHost::default());
    host.upsert(creds()).unwrap();
    let client = Client::new(host.clone());
    let caller = client.clone();
    let pending = tokio::spawn(async move { caller.connect("desktop_fixture").await });
    tokio::time::timeout(Duration::from_secs(5), host.entered.notified())
        .await
        .expect("host service entered");
    client.disconnect("desktop_fixture").await;
    host.release.notify_one();
    let error = tokio::time::timeout(Duration::from_secs(1), pending)
        .await
        .unwrap()
        .unwrap()
        .unwrap_err();
    assert_eq!(error.to_string(), "peer_not_connected");
    assert_eq!(host.updates.load(Ordering::SeqCst), 0);
    assert_eq!(
        host.load().unwrap().peers[0].creds.user_jwt,
        "old-token-fixture"
    );
    assert!(client.list().await.unwrap()[0].stopped);
    assert!(client.list().await.unwrap()[0].error.is_none());
    client.shutdown(Duration::from_secs(1)).await.unwrap();
}

#[tokio::test]
async fn dropping_a_transfer_registration_releases_the_waiter_synchronously() {
    let client = Client::new(Arc::new(MemoryHost::default()));
    let (sender, receiver) = tokio::sync::oneshot::channel();
    let registration =
        client.register_transfer_chunk("desktop_fixture", "transfer_fixture", 0, sender);
    drop(registration);
    assert!(receiver.await.is_err());
    assert!(client.inner.chunks.lock().unwrap().is_empty());
    client.shutdown(Duration::from_secs(1)).await.unwrap();
}

#[tokio::test]
async fn cancelling_an_old_chunk_waiter_cannot_remove_a_newer_retry() {
    let client = Client::new(Arc::new(MemoryHost::default()));
    let (first, first_reply) = tokio::sync::oneshot::channel();
    let old = client.register_transfer_chunk("desktop_fixture", "transfer_fixture", 0, first);
    let (second, second_reply) = tokio::sync::oneshot::channel();
    let current = client.register_transfer_chunk("desktop_fixture", "transfer_fixture", 0, second);
    assert!(first_reply.await.is_err());
    drop(old);
    client
        .deliver_chunk(
            "desktop_fixture",
            "transfer_fixture".into(),
            0,
            vec![1, 2, 3],
        )
        .await;
    assert_eq!(second_reply.await.unwrap(), vec![1, 2, 3]);
    drop(current);
    assert!(client.inner.chunks.lock().unwrap().is_empty());
    client.shutdown(Duration::from_secs(1)).await.unwrap();
}

#[path = "shared_request_tests.rs"]
mod shared_requests;

#[tokio::test]
async fn explicit_connect_retires_an_inflight_supervisor_before_loading_credentials() {
    let host = Arc::new(MemoryHost::default());
    host.upsert(creds()).unwrap();
    let client = Client::new(host.clone());
    client.install_supervisor_for_test("desktop_fixture").await;
    tokio::time::timeout(Duration::from_secs(5), host.entered.notified())
        .await
        .expect("supervisor entered renewal");
    let old = client.inner.state.lock().await.supervisors["desktop_fixture"].clone();
    host.reject_load.store(true, Ordering::SeqCst);
    let error = client.connect("desktop_fixture").await.unwrap_err();
    assert!(error.to_string().contains("fixture unavailable"));
    tokio::time::timeout(Duration::from_secs(1), old.completed())
        .await
        .expect("superseded renewal was dropped");
    assert_eq!(host.finished_refreshes.load(Ordering::SeqCst), 1);
    assert_eq!(host.updates.load(Ordering::SeqCst), 0);
    assert!(client.inner.state.lock().await.supervisors.is_empty());
    client.shutdown(Duration::from_secs(1)).await.unwrap();
}

#[path = "shared_credentials_tests.rs"]
mod shared_credentials;

#[path = "shared_recovery_tests.rs"]
mod shared_recovery;
