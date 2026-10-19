//! Independent endpoint-context checks; no application runtime or network.
use super::*;
use crate::{lifecycle::AccessEpoch, secure::HandshakeHost};

#[derive(Default)]
struct MemoryRuntime {
    access: AccessEpoch,
    coalesce: Arc<AtomicBool>,
    activity: Arc<AtomicBool>,
    lean: Arc<AtomicBool>,
    identity: Arc<Mutex<Option<crate::types::ClientIdentity>>>,
    gzip: bool,
}
impl HandshakeHost for MemoryRuntime {
    fn save_binding(&self, _: &crate::PairingCreds) -> Result<(), crate::Error> {
        Ok(())
    }
    fn presence(&self, pair: &str, bridge: &str) -> Value {
        json!({"pairId": pair, "bridgeInstanceId": bridge, "online": true})
    }
}
impl CommandRuntime for MemoryRuntime {
    fn access(&self) -> &AccessEpoch {
        &self.access
    }
    fn client_identity(&self, _: &str) -> Option<Arc<Mutex<Option<crate::types::ClientIdentity>>>> {
        Some(self.identity.clone())
    }
    fn coalesce_events(&self, _: &str) -> Arc<AtomicBool> {
        self.coalesce.clone()
    }
    fn catalog_activity(&self, _: &str) -> Arc<AtomicBool> {
        self.activity.clone()
    }
    fn lean_events(&self, _: &str) -> Arc<AtomicBool> {
        self.lean.clone()
    }
    fn in_control(&self, _: &str) -> bool {
        true
    }
    fn held_by_another(&self, _: &str) -> bool {
        false
    }
    fn claim_control_soon(&self, _: &str) -> Option<String> {
        None
    }
    fn light_presence(&self, pair: &str, bridge: &str) -> Value {
        self.presence(pair, bridge)
    }
    fn publish_control(&self, _: &str, _: &str) {}
    fn unpair_slot<'a>(
        &'a self,
        _: &'a str,
    ) -> futures::future::BoxFuture<'a, Result<(), crate::Error>> {
        Box::pin(async { Ok(()) })
    }
    fn gzip_override(&self) -> bool {
        self.gzip
    }
    fn spawn(&self, future: futures::future::BoxFuture<'static, ()>) {
        tokio::spawn(future);
    }
}
struct NoBusiness;
impl BusinessHost for NoBusiness {
    fn execute<'a>(
        &'a self,
        _: IncomingCmd,
        reply: &'a dyn ReplySink,
    ) -> futures::future::BoxFuture<'a, ()> {
        Box::pin(async move {
            reply
                .send(false, Value::Null, Some("unused fixture business".into()))
                .await;
        })
    }
}
fn endpoint(runtime: Arc<MemoryRuntime>) -> HandshakeState {
    let creds = crate::PairingCreds {
        handshake_version: 2,
        secure: Some(crate::secure::PairingIdentity::new(i64::MAX).unwrap()),
        pair_id: "pair_fixture".into(),
        desktop_id: "desktop_fixture".into(),
        nkey_seed: "unused fixture".into(),
        user_jwt: "unused fixture".into(),
        nats_url: "tls://relay.invalid".into(),
        nats_ws_url: "wss://relay.invalid".into(),
        jwt_expires_at: i64::MAX,
        control_url: None,
    };
    let epoch = runtime.access.current();
    HandshakeState::new(
        creds,
        Arc::new(AtomicBool::new(true)),
        "bridge_fixture".into(),
        CommandServices {
            runtime,
            business: Arc::new(NoBusiness),
            reply_ttl: Duration::from_secs(600),
        },
    )
    .with_access(epoch)
}

#[test]
fn runtime_instances_isolate_access_identity_and_declared_capabilities() {
    let first = Arc::new(MemoryRuntime::default());
    let second = Arc::new(MemoryRuntime::default());
    let a = endpoint(first.clone());
    let b = endpoint(second.clone());
    apply_declared_features(
        &a,
        "pair_fixture",
        &[
            future_remote_protocol::EVENT_COALESCING_FEATURE.into(),
            future_remote_protocol::CATALOG_ACTIVITY_FEATURE.into(),
            future_remote_protocol::LEAN_EVENTS_FEATURE.into(),
            REPLY_GZIP_FEATURE.into(),
            "future_unknown_capability".into(),
        ],
    );
    assert!(first.coalesce.load(Ordering::Acquire));
    assert!(first.activity.load(Ordering::Acquire));
    assert!(first.lean.load(Ordering::Acquire));
    assert!(a.gzip_replies.load(Ordering::Acquire));
    assert!(!second.coalesce.load(Ordering::Acquire));
    assert!(!second.activity.load(Ordering::Acquire));
    assert!(!second.lean.load(Ordering::Acquire));
    assert!(!b.gzip_replies.load(Ordering::Acquire));
    record_client_identity(
        first.as_ref(),
        "pair_fixture",
        &IncomingCmd {
            device_name: " fixture controller ".into(),
            device_kind: "future_kind".into(),
            ..Default::default()
        },
    );
    assert_eq!(
        first.identity.lock().unwrap().as_ref().unwrap().name,
        "fixture controller"
    );
    assert!(second.identity.lock().unwrap().is_none());
    apply_declared_features(&a, "pair_fixture", &[]);
    assert!(!first.coalesce.load(Ordering::Acquire));
    assert!(!first.activity.load(Ordering::Acquire));
    assert!(!first.lean.load(Ordering::Acquire));
    assert!(!a.gzip_replies.load(Ordering::Acquire));
    first.access.invalidate(|| {});
    assert!(!a.access_current());
    assert!(b.access_current());
}

#[test]
fn operator_gzip_is_scoped_to_the_runtime_serving_the_request() {
    let compress: Arc<dyn CommandRuntime> = Arc::new(MemoryRuntime {
        gzip: true,
        ..Default::default()
    });
    let plain: Arc<dyn CommandRuntime> = Arc::new(MemoryRuntime::default());
    let body = json!({"data": "fixture text ".repeat(10_000)});
    let expected = serde_json::to_vec(&body).unwrap();
    assert!(COMMAND_RUNTIME
        .sync_scope(compress, || encode_reply_payload(&body))
        .starts_with(&[0x1f, 0x8b]));
    assert_eq!(
        COMMAND_RUNTIME.sync_scope(plain, || encode_reply_payload(&body)),
        expected
    );
    assert_eq!(encode_reply_payload(&body), expected);
    assert!(!remote_json_gzip_enabled());
}
