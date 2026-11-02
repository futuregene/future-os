//! Per-instance command adapter; no application-global lookup or ownership cycle.
use super::*;
use crate::{
    commands::{CommandRuntime, CommandServices, HandshakeState},
    secure::HandshakeHost,
};
use serde_json::Value;
use std::sync::Weak;
struct Context {
    server: Weak<Supervisor>,
    access: Arc<lifecycle::AccessEpoch>,
    epoch: u64,
}
impl HandshakeHost for Context {
    fn save_binding(&self, creds: &PairingCreds) -> Result<(), crate::Error> {
        let server = self.server.upgrade().ok_or_else(stopped)?;
        // Both encrypted and legacy confirmation paths call this from inside
        // access.commit. Reacquiring that mutex here would deadlock a pairing.
        if self.access.published() != self.epoch {
            return Err(stopped());
        }
        server.host.save(creds)
    }
    fn presence(&self, pair: &str, bridge: &str) -> Value {
        self.server
            .upgrade()
            .map(|server| {
                // The directory belongs to the device driving this machine, the
                // same rule `get_presence` keeps: a standby device is paired and
                // connected, but it is not the one reading this machine's
                // conversations, and handing it the list here would make "only
                // one controls" a statement about commands alone. The handshake
                // was the exception — it answered with the whole catalogue for
                // every paired device, including a standby one, which is both a
                // wider disclosure than `get_presence` allows and the reason the
                // confirmation grows with the machine's usage at all.
                //
                // A standby device is not cut off: it gets the liveness packet
                // (who holds the machine, whether the agent is up), it completes
                // the handshake, and it reads the catalogue from
                // `list_sessions` if it takes control.
                if server.in_control(pair) {
                    server.build_presence_payload(pair, bridge)
                } else {
                    server.light_presence_payload(pair, bridge)
                }
            })
            .unwrap_or(Value::Null)
    }
}
fn stopped() -> crate::Error {
    crate::Error::Message("remote_server_stopped".into())
}
impl CommandRuntime for Context {
    fn access(&self) -> &lifecycle::AccessEpoch {
        &self.access
    }
    fn client_identity(&self, pair: &str) -> Option<Arc<Mutex<Option<ClientIdentity>>>> {
        self.server.upgrade()?.client_identity(pair)
    }
    fn client_link(&self, pair: &str) -> Option<Arc<ClientLink>> {
        Some(self.server.upgrade()?.client_link(pair))
    }
    fn note_client_device(&self, pair: &str, device_id: &str) {
        if let Some(server) = self.server.upgrade() {
            server.note_client_device(pair, device_id);
        }
    }
    fn client_left(&self, pair: &str) {
        if let Some(server) = self.server.upgrade() {
            server.client_left(pair);
        }
    }
    fn coalesce_events(&self, pair: &str) -> Arc<AtomicBool> {
        self.server
            .upgrade()
            .map(|server| server.coalesce_events(pair))
            .unwrap_or_default()
    }
    fn catalog_activity(&self, pair: &str) -> Arc<AtomicBool> {
        self.server
            .upgrade()
            .map(|server| server.catalog_activity(pair))
            .unwrap_or_default()
    }
    fn lean_events(&self, pair: &str) -> Arc<AtomicBool> {
        self.server
            .upgrade()
            .map(|server| server.lean_events(pair))
            .unwrap_or_default()
    }
    fn segmented_content(&self, pair: &str) -> Arc<AtomicBool> {
        self.server
            .upgrade()
            .map(|server| server.segmented_content(pair))
            .unwrap_or_default()
    }
    fn in_control(&self, pair: &str) -> bool {
        self.server
            .upgrade()
            .is_some_and(|server| server.in_control(pair))
    }
    fn alone_and_unowned(&self, pair: &str) -> bool {
        self.server
            .upgrade()
            .is_some_and(|server| server.alone_and_unowned(pair))
    }
    fn held_by_another(&self, pair: &str) -> bool {
        self.server
            .upgrade()
            .is_none_or(|server| server.held_by_another(pair))
    }
    fn claim_control_soon(&self, pair: &str, epoch: Option<u64>) -> Option<String> {
        self.server
            .upgrade()?
            .claim_control_for(pair, Some(epoch.unwrap_or(self.epoch)))
    }
    fn light_presence(&self, pair: &str, bridge: &str) -> Value {
        self.server
            .upgrade()
            .map(|server| server.light_presence_payload(pair, bridge))
            .unwrap_or(Value::Null)
    }
    fn publish_control(&self, pair: &str, control: &str) {
        if let Some(server) = self.server.upgrade() {
            server.publish_control(pair, control);
        }
    }
    fn unpair_slot<'a>(&'a self, pair: &'a str) -> BoxFuture<'a, Result<(), crate::Error>> {
        Box::pin(async move {
            if self.access.current() != self.epoch {
                return Err(stopped());
            }
            self.server
                .upgrade()
                .ok_or_else(stopped)?
                .unpair_slot(pair)
                .await
                .map(|_| ())
        })
    }
    fn gzip_override(&self) -> bool {
        self.server
            .upgrade()
            .is_some_and(|server| server.executor.gzip_override())
    }
    fn spawn(&self, future: BoxFuture<'static, ()>) {
        if let Some(server) = self.server.upgrade() {
            server.spawn_command(self.epoch, future);
        }
    }
}
impl Supervisor {
    pub fn new_handshake_state(
        self: &Arc<Self>,
        creds: PairingCreds,
        confirmed: Arc<AtomicBool>,
        bridge: String,
    ) -> HandshakeState {
        HandshakeState::new(
            creds,
            confirmed,
            bridge,
            CommandServices {
                runtime: Arc::new(Context {
                    server: Arc::downgrade(self),
                    access: self.access.clone(),
                    epoch: self.access.current(),
                }),
                business: self.host.clone(),
                reply_ttl: self.executor.options().reply_ttl,
            },
        )
    }
}

#[cfg(any(test, feature = "test-support"))]
impl Supervisor {
    pub fn record_client_identity(
        self: &Arc<Self>,
        pair: &str,
        command: &future_remote_protocol::IncomingCmd,
    ) {
        let context = Context {
            server: Arc::downgrade(self),
            access: self.access.clone(),
            epoch: self.access.current(),
        };
        crate::commands::test_support::record_client_identity(&context, pair, command);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::supervisor::test_support::{isolated, MemoryHost, TokioExecutor};
    fn credentials() -> PairingCreds {
        PairingCreds {
            handshake_version: 2,
            secure: Some(secure::PairingIdentity::new(unix_timestamp() as i64 + 300).unwrap()),
            pair_id: "pair_context".into(),
            desktop_id: "host_context".into(),
            nkey_seed: nkeys::KeyPair::new_user().seed().unwrap(),
            user_jwt: "fixture".into(),
            nats_url: "tls://relay.invalid:4222".into(),
            nats_ws_url: "wss://relay.invalid".into(),
            jwt_expires_at: 12345,
            control_url: None,
        }
    }
    #[test]
    fn cached_handshake_does_not_keep_its_supervisor_alive() {
        let server = isolated();
        let weak = Arc::downgrade(&server);
        let shared = server.shared_or_insert("pair_context");
        *shared.handshake.lock().unwrap() = Some(server.new_handshake_state(
            credentials(),
            Arc::new(AtomicBool::new(false)),
            "bridge_context".into(),
        ));
        drop(shared);
        drop(server);
        assert!(weak.upgrade().is_none());
    }
    #[tokio::test]
    async fn an_old_command_context_cannot_save_or_unpair_after_stop() {
        let host = Arc::new(MemoryHost::default());
        let server = Supervisor::new(host.clone(), Arc::new(TokioExecutor));
        let context = Context {
            server: Arc::downgrade(&server),
            access: server.access.clone(),
            epoch: server.access.current(),
        };
        let credentials = credentials();
        host.slots.lock().unwrap().push(credentials.clone());
        server.stop();
        assert!(context.save_binding(&credentials).is_err());
        assert!(context.unpair_slot(&credentials.pair_id).await.is_err());
        assert_eq!(host.slots.lock().unwrap().len(), 1);
    }
    #[test]
    fn binding_persistence_uses_the_callers_existing_commit_boundary() {
        let host = Arc::new(MemoryHost::default());
        let server = Supervisor::new(host.clone(), Arc::new(TokioExecutor));
        let epoch = server.access.current();
        let context = Context {
            server: Arc::downgrade(&server),
            access: server.access.clone(),
            epoch,
        };
        let credentials = credentials();
        server
            .access
            .commit(epoch, || context.save_binding(&credentials))
            .unwrap()
            .unwrap();
        assert_eq!(host.slots.lock().unwrap()[0].pair_id, credentials.pair_id);
    }

    /// The handshake answers with the catalogue for the device that drives the
    /// machine, and with liveness alone for the one standing by.
    ///
    /// `get_presence` already drew that line; the confirmation did not, so a
    /// standby device was handed the whole directory at the end of every
    /// handshake — wider than the command path allows, and the reason the
    /// confirmation is the one body in a connect that grows with this machine's
    /// usage.
    #[test]
    fn the_handshake_hands_the_directory_to_the_controller_only() {
        let host = Arc::new(MemoryHost::default());
        *host.directory.lock().unwrap() = Some((
            json!({ "pairId": "", "sessions": [{ "sessionId": "s1", "title": "目录" }] }),
            json!({ "pairId": "", "workspaces": [{ "id": "w1", "name": "工作区" }] }),
        ));
        let server = Supervisor::new(host.clone(), Arc::new(TokioExecutor));
        // Two paired devices, the first driving the machine. `in_control` reads
        // a pointer only while the device it names is one this machine knows,
        // so the controller has to be in the book for the second to be standby.
        server
            .known
            .lock()
            .unwrap()
            .insert("pair_controller".into());
        server.known.lock().unwrap().insert("pair_standby".into());
        server.restore_active(Some("pair_controller".into()));
        let context = Context {
            server: Arc::downgrade(&server),
            access: server.access.clone(),
            epoch: server.access.current(),
        };

        let controller = context.presence("pair_controller", "bridge_context");
        assert_eq!(controller["sessions"][0]["sessionId"], "s1");
        assert_eq!(controller["workspaces"][0]["id"], "w1");
        assert_eq!(controller["control"], "active");

        let standby = context.presence("pair_standby", "bridge_context");
        assert!(
            standby.get("sessions").is_none(),
            "a standby device is not handed the directory: {standby}"
        );
        assert!(
            standby.get("workspaces").is_none(),
            "nor the workspace list: {standby}"
        );
        assert_eq!(standby["control"], "standby");
        // What it does get: enough to complete a handshake and to know who holds
        // the machine, which is what lets it ask for a takeover by name.
        assert_eq!(standby["online"], true);
        assert_eq!(standby["pairId"], "pair_standby");
        assert_eq!(standby["bridgeInstanceId"], "bridge_context");
        assert_eq!(standby["holder"]["pairId"], "pair_controller");
    }
}
