//! Desktop operator settings and real application command integration tests.
#[cfg(test)]
use future_remote_protocol::IncomingCmd;
#[cfg(test)]
pub(super) use future_remote_server::commands::new_reply_slots;
#[cfg(test)]
pub(super) use future_remote_server::commands::{apply_declared_features, REPLY_GZIP_FEATURE};
#[cfg(test)]
pub(super) use future_remote_server::commands::{HandshakeState, ReplySlots};
#[cfg(test)]
use future_remote_server::services::ReplySink;
#[cfg(test)]
use serde_json::Value;
#[cfg(test)]
use std::sync::{atomic::AtomicBool, Arc};

const REMOTE_JSON_GZIP_ENV: &str = "FUTURE_REMOTE_JSON_GZIP";
pub(super) fn operator_gzip_enabled() -> bool {
    std::env::var(REMOTE_JSON_GZIP_ENV).is_ok_and(|value| {
        matches!(
            value.trim().to_ascii_lowercase().as_str(),
            "1" | "true" | "yes" | "on"
        )
    })
}
#[cfg(test)]
pub(super) fn new_handshake_state(
    creds: super::pairing::PairingCreds,
    confirmed: Arc<AtomicBool>,
    bridge: String,
) -> HandshakeState {
    super::SUPERVISOR.new_handshake_state(creds, confirmed, bridge)
}

#[cfg(test)]
use crate::remote_host::business::*;
#[cfg(test)]
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
#[cfg(test)]
use future_remote_protocol::handshake::{handshake_transcript, HandshakeTranscript};
#[cfg(test)]
use future_remote_server::commands::{command_loop, test_support};
#[cfg(test)]
use test_support::{
    handle_command, handle_command_singleflight, handle_pair_handshake,
    handle_pair_handshake_confirm, publish_reply_payload,
};
#[cfg(test)]
fn remote_json_gzip_enabled() -> bool {
    operator_gzip_enabled()
}
#[cfg(test)]
fn record_client_identity(pair: &str, command: &IncomingCmd) {
    super::SUPERVISOR.record_client_identity(pair, command);
}
#[cfg(test)]
struct BorrowedBusiness(&'static dyn future_remote_server::services::BusinessHost);
#[cfg(test)]
impl future_remote_server::services::BusinessHost for BorrowedBusiness {
    fn execute<'a>(
        &'a self,
        command: IncomingCmd,
        reply: &'a dyn future_remote_server::services::ReplySink,
    ) -> futures::future::BoxFuture<'a, ()> {
        self.0.execute(command, reply)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_identity_keeps_raw_slashes_and_distinguishes_providers() {
        for provider in ["one", "two"] {
            assert_eq!(
                qualified_model_id("family/model", provider),
                Some(format!("{provider}/family/model"))
            );
            let qualified = format!("{provider}/family/model");
            assert_eq!(qualified_model_id(&qualified, provider), Some(qualified));
        }
        assert_eq!(
            qualified_model_id("openrouter/openrouter/auto", "openrouter").as_deref(),
            Some("openrouter/openrouter/auto")
        );
        assert_eq!(
            qualified_model_id("p/model", "").as_deref(),
            Some("p/model")
        );
        assert_eq!(qualified_model_id("", "p"), None);
    }

    /// The operator override is what lets the wire path be exercised without a
    /// client that declares the capability. It must accept exactly the
    /// documented spellings and let anything else fall back to the per-connection
    /// capability (off outside a command).
    #[test]
    fn the_gzip_override_is_an_operator_switch() {
        let _home = crate::remote::test_support::HomeGuard::new("remote-gzip-env");
        let previous = std::env::var(REMOTE_JSON_GZIP_ENV).ok();
        for (value, expected) in [
            ("1", true),
            ("true", true),
            ("YES", true),
            (" on ", true),
            ("0", false),
            ("off", false),
            ("", false),
            ("maybe", false),
        ] {
            std::env::set_var(REMOTE_JSON_GZIP_ENV, value);
            assert_eq!(
                remote_json_gzip_enabled(),
                expected,
                "value {value:?} must be {expected}"
            );
        }
        std::env::remove_var(REMOTE_JSON_GZIP_ENV);
        assert!(
            !remote_json_gzip_enabled(),
            "an unset override leaves the connection in charge"
        );

        // Restoring the operator's own value is the test's cleanup, and both of
        // its arms matter: an operator who launched the app with the switch
        // already set must get that value back, and one who did not must not be
        // left with a variable this test invented. Assert both, so the cleanup
        // is a checked behaviour rather than an unexecuted branch.
        let restore = |value: &Option<String>| match value {
            Some(value) => std::env::set_var(REMOTE_JSON_GZIP_ENV, value),
            None => std::env::remove_var(REMOTE_JSON_GZIP_ENV),
        };
        restore(&Some("1".to_string()));
        assert_eq!(
            std::env::var(REMOTE_JSON_GZIP_ENV).as_deref(),
            Ok("1"),
            "a pre-set override survives the test"
        );
        restore(&None);
        assert!(
            std::env::var(REMOTE_JSON_GZIP_ENV).is_err(),
            "an absent override stays absent"
        );
        restore(&previous);
    }
}

#[cfg(test)]
mod bridge_tests {
    #![allow(clippy::await_holding_lock)]
    use super::super::test_support::{
        await_publish, ensure_mock_agent, init_store, jwt, mock_agent_lock, nats_connect,
        nats_connect_once, now_secs, unique, wait_until, FakeNats, HomeGuard,
    };
    use super::*;
    use crate::remote::SUPERVISOR;
    use crate::remote_host::files as transfer;
    use flate2::read::GzDecoder;
    use serde_json::json;
    use std::io::Read as _;
    #[cfg(test)]
    use std::sync::atomic::Ordering;
    #[cfg(test)]
    use std::time::Duration;

    #[test]
    fn thread_title_derivation_matches_the_gui_draft() {
        // Whitespace-only / empty input falls back to the default chat title.
        assert_eq!(derive_thread_title(""), "New Chat");
        assert_eq!(derive_thread_title("  \n\t  "), "New Chat");
        // Whitespace collapses; 28 chars is the cut, ellipsized beyond it.
        assert_eq!(
            derive_thread_title("hello   there\nworld"),
            "hello there world"
        );
        let long = "abcdefghijklmnopqrstuvwxyz0123456789";
        assert_eq!(derive_thread_title(long), "abcdefghijklmnopqrstuvwxyz01...");
        // A phone message may open with a reference to another conversation;
        // the title is the question, not the link that addressed the peer.
        assert_eq!(
            derive_thread_title(
                "[手机截图镜像修复](futureos://session/20261016-142101-2a944dead697482d8a6adad9942fe403) 这个会话在流式回复中"
            ),
            "这个会话在流式回复中"
        );
    }

    fn bridge_creds() -> crate::remote::pairing::PairingCreds {
        let key_pair = nkeys::KeyPair::new_user();
        crate::remote::pairing::PairingCreds {
            handshake_version: 1,
            secure: None,
            pair_id: format!("pair_{}", unique("cmd")),
            desktop_id: format!("desktop_{}", unique("cmd")),
            nkey_seed: key_pair.seed().unwrap().to_string(),
            user_jwt: jwt(now_secs() + 3600),
            nats_url: "nats://127.0.0.1:1".to_string(),
            nats_ws_url: "ws://127.0.0.1:1".to_string(),
            jwt_expires_at: now_secs() + 3600,
            control_url: None,
        }
    }

    /// The same credentials with a real v2 secure identity, so the bridge serves
    /// the encrypted lane: every command must arrive as a sealed `FRE2` record.
    fn secure_bridge_creds() -> crate::remote::pairing::PairingCreds {
        let mut creds = bridge_creds();
        creds.handshake_version = 2;
        creds.secure = Some(
            crate::remote::secure::PairingIdentity::new(now_secs() + 300)
                .expect("a fresh secure identity"),
        );
        creds
    }

    /// A running command loop against a fake NATS: returns the client handle a
    /// test drives, plus the pieces it may need to poke.
    struct Bridge {
        client: async_nats::Client,
        nats: FakeNats,
        pair_id: String,
        handshake: HandshakeState,
        reply_slots: ReplySlots,
        loop_handle: tokio::task::JoinHandle<()>,
    }

    impl Bridge {
        async fn start() -> Self {
            Self::start_with(super::super::host(), None).await
        }
        async fn start_with_host(
            host: &'static dyn future_remote_server::services::BusinessHost,
        ) -> Self {
            Self::start_with(host, None).await
        }
        /// A bridge whose connection was admitted under `access_epoch`. `None`
        /// is the untracked connection every other fixture uses; `Some` is what
        /// a supervised generation is admitted under, so a stale one can be
        /// built by admitting under an epoch that has since moved on.
        async fn start_with(
            host: &'static dyn future_remote_server::services::BusinessHost,
            access_epoch: Option<u64>,
        ) -> Self {
            Self::spawn_flavour(host, access_epoch, false).await.0
        }
        /// A bridge with a real secure identity, paired over the encrypted
        /// lane. Returns the invitation a phone would scan.
        async fn start_secure(access_epoch: Option<u64>) -> (Self, String) {
            Self::spawn_flavour(super::super::host(), access_epoch, true).await
        }

        async fn spawn_flavour(
            host: &'static dyn future_remote_server::services::BusinessHost,
            access_epoch: Option<u64>,
            secure: bool,
        ) -> (Self, String) {
            let nats = FakeNats::start().await;
            let client = nats_connect(&nats).await;
            let creds = if secure {
                secure_bridge_creds()
            } else {
                bridge_creds()
            };
            let pair_id = creds.pair_id.clone();
            let invitation = if secure {
                let identity = creds.secure.as_ref().expect("secure identity");
                format!(
                    "futureos://remote/pair?v=2&desktopId={}&secureKey={}&secret={}",
                    creds.desktop_id,
                    identity.public_key,
                    identity.secret.as_deref().expect("invitation secret"),
                )
            } else {
                String::new()
            };
            let mut handshake = new_handshake_state(
                creds,
                Arc::new(AtomicBool::new(false)),
                format!("bridge_{}", unique("cmd")),
            );
            if let Some(epoch) = access_epoch {
                handshake = handshake.with_access(epoch);
            }
            test_support::set_business(&mut handshake, Arc::new(BorrowedBusiness(host)));
            let reply_slots = new_reply_slots();
            let loop_handle = tokio::spawn(command_loop(
                client.clone(),
                pair_id.clone(),
                reply_slots.clone(),
                handshake.clone(),
            ));
            nats.wait_for_sub(&format!("p.{pair_id}.cmd.>"), Duration::from_secs(5))
                .await;
            // The fixture is the device driving this machine; the control gate
            // is real, so a fixture that did not say so would be a standby and
            // see every command refused.
            SUPERVISOR.restore_active(Some(pair_id.clone()));
            (
                Bridge {
                    client,
                    nats,
                    pair_id,
                    handshake,
                    reply_slots,
                    loop_handle,
                },
                invitation,
            )
        }

        /// Activate the bridge (as a completed handshake would).
        fn activate(&self) {
            self.handshake.active_flag().store(true, Ordering::Release);
        }

        /// Send a command and await its reply envelope.
        async fn call(&self, cmd: Value) -> Value {
            serde_json::from_slice(&self.call_raw(cmd).await).expect("reply is JSON")
        }

        /// Send a command and return the reply bytes exactly as the client
        /// would receive them, before any decoding.
        async fn call_raw(&self, cmd: Value) -> Vec<u8> {
            let subject = format!("p.{}.cmd.rpc", self.pair_id);
            let message = self
                .client
                .request(subject, serde_json::to_vec(&cmd).unwrap().into())
                .await
                .expect("bridge reply");
            message.payload.to_vec()
        }

        async fn stop(self) {
            self.loop_handle.abort();
            SUPERVISOR.release_control(&self.pair_id);
            // Tests redirect process-global HOME while they exercise the
            // SQLite-backed bridge. Do not release that fixture until the
            // cancelled command loop has stopped running on every Tokio worker.
            let _ = self.loop_handle.await;
        }
    }

    /// The whole feature is only real if the flag reaches the encoder through
    /// the running command loop. Drive it end to end: a large reply is plain
    /// JSON until the connection declares the capability, and compressed after.
    #[tokio::test]
    async fn declared_capability_reaches_the_running_command_loop() {
        struct Big;
        impl future_remote_server::services::BusinessHost for Big {
            fn execute<'a>(
                &'a self,
                _cmd: IncomingCmd,
                sink: &'a dyn ReplySink,
            ) -> futures::future::BoxFuture<'a, ()> {
                Box::pin(async move {
                    // Comfortably past the gzip threshold, and repetitive enough
                    // that compression is unambiguous.
                    sink.send(
                        true,
                        json!({ "entries": ["repeated history ".repeat(2_000)] }),
                        None,
                    )
                    .await
                })
            }
        }
        static BIG: Big = Big;
        let bridge = Bridge::start_with_host(&BIG).await;
        bridge.activate();
        // A distinct id per call: a repeated id is served from the single-flight
        // response cache, and the cached bytes carry the encoding of the first
        // reply — which would mask the flag change this test is about.
        let command = |id: &str| json!({"id": id, "type": "get_state"});

        // Before the declaration: plain JSON, byte for byte.
        let plain = bridge.call_raw(command("gzip-before")).await;
        assert_eq!(plain.first(), Some(&b'{'), "expected an uncompressed reply");

        // The client now declares the capability on secure_ready.
        apply_declared_features(
            &bridge.handshake,
            &bridge.pair_id,
            &[REPLY_GZIP_FEATURE.to_string()],
        );

        let compressed = bridge.call_raw(command("gzip-declared")).await;
        assert_eq!(
            compressed.first().copied(),
            Some(0x1f),
            "expected a gzip reply after the declaration"
        );
        assert_eq!(compressed.get(1), Some(&0x8b));
        assert!(compressed.len() < plain.len() / 4);

        // The compressed reply must decode back to exactly what was sent
        // uncompressed: this is the client's promise, not an approximation.
        let mut decoder = GzDecoder::new(compressed.as_slice());
        let mut decoded = Vec::new();
        decoder.read_to_end(&mut decoded).unwrap();
        assert_eq!(decoded, plain);

        // A later connection that declares only the other capability must not
        // inherit this one.
        apply_declared_features(
            &bridge.handshake,
            &bridge.pair_id,
            &["event_coalescing_v1".to_string()],
        );
        let plain_again = bridge.call_raw(command("gzip-withdrawn")).await;
        assert_eq!(
            plain_again.first(),
            Some(&b'{'),
            "withdrawing the declaration must restore plain JSON"
        );
        assert_eq!(plain_again, plain);
        bridge.stop().await;
    }

    #[tokio::test]
    async fn protocol_routes_to_a_substitute_host_and_rejects_a_stale_access_identity() {
        struct Echo;
        impl future_remote_server::services::BusinessHost for Echo {
            fn execute<'a>(
                &'a self,
                cmd: IncomingCmd,
                sink: &'a dyn ReplySink,
            ) -> futures::future::BoxFuture<'a, ()> {
                Box::pin(async move {
                    sink.send(true, json!({"host": "substitute", "id": cmd.id}), None)
                        .await
                })
            }
        }
        static ECHO: Echo = Echo;
        let bridge = Bridge::start_with_host(&ECHO).await;
        bridge.activate();
        let response = bridge
            .call(json!({"id":"host-test", "type":"get_state"}))
            .await;
        assert_eq!(response["data"]["host"], "substitute");
        let response = bridge
            .call(json!({"id":"stale-test", "type":"prompt", "bridgeInstanceId":"old-access"}))
            .await;
        assert_eq!(response["error"], "remote_access_changed");
        bridge.stop().await;
    }

    fn handshake_cmd(
        creds: &crate::remote::pairing::PairingCreds,
        client_key: &nkeys::KeyPair,
    ) -> Value {
        json!({
            "id": unique("cmd"),
            "type": "pair_handshake",
            "protocolVersion": 1,
            "pairId": creds.pair_id,
            "deviceId": "dev_test",
            "clientPublicKey": client_key.public_key(),
            "clientNonce": "nonce-0123456789abcdef",
            "expectedDesktopId": creds.desktop_id,
            "expectedDesktopPublicKey": crate::remote::pairing::public_key(creds).unwrap(),
        })
    }

    /// A corrupted desktop NKey seed fails the handshake before any signing:
    /// the client gets a failure reply and the bridge stays inactive.
    #[tokio::test]
    async fn handshake_rejects_a_bad_desktop_seed() {
        let _home = HomeGuard::new("cmd-bad-seed");
        let nats = FakeNats::start().await;
        let client = nats_connect(&nats).await;
        let mut creds = bridge_creds();
        creds.nkey_seed = "not-a-valid-seed".to_string();
        let handshake = new_handshake_state(
            creds,
            Arc::new(AtomicBool::new(false)),
            format!("bridge_{}", unique("cmd")),
        );
        let reply_subject = format!("rep_{}", unique("hs"));
        let mut tap = nats.tap();
        let msg = async_nats::Message {
            subject: "p.pair.cmd.pair_handshake".into(),
            reply: Some(reply_subject.clone().into()),
            payload: Vec::new().into(),
            headers: None,
            status: None,
            description: None,
            length: 0,
        };
        let cmd = IncomingCmd {
            cmd_type: "pair_handshake".to_string(),
            ..Default::default()
        };
        handle_pair_handshake(&client, &msg, &cmd, &handshake).await;
        let reply = await_publish(&mut tap, &reply_subject, Duration::from_secs(5)).await;
        assert_eq!(reply.json()["success"], json!(false));
        assert!(!handshake.active_flag().load(Ordering::Acquire));
    }

    #[tokio::test]
    async fn rejects_garbage_and_requires_handshake() {
        let _home = HomeGuard::new("cmd-gate");
        let bridge = Bridge::start().await;

        // Unparseable payload → error reply.
        let mut tap = bridge.nats.tap();
        let reply_subject = format!("rep-{}", unique("garbage"));
        bridge.nats.inject(
            &format!("p.{}.cmd.rpc", bridge.pair_id),
            Some(&reply_subject),
            b"{not json".to_vec(),
        );
        let reply = await_publish(&mut tap, &reply_subject, Duration::from_secs(5)).await;
        assert_eq!(reply.json()["success"], json!(false));
        assert!(reply.json()["error"]
            .as_str()
            .unwrap()
            .contains("Failed to parse command JSON"));

        // A well-formed non-handshake command before activation is refused.
        let reply = bridge
            .call(json!({ "id": unique("cmd"), "type": "list_sessions" }))
            .await;
        assert_eq!(reply["success"], json!(false));
        assert_eq!(reply["error"], json!("pairing_handshake_required"));

        // A command with no reply subject is processed but not answered.
        bridge.nats.inject(
            &format!("p.{}.cmd.rpc", bridge.pair_id),
            None,
            b"{ not json either".to_vec(),
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
        bridge.stop().await;
    }

    #[tokio::test]
    async fn handshake_roundtrip_activates_the_bridge() {
        let _home = HomeGuard::new("cmd-handshake");
        init_store();
        let bridge = Bridge::start().await;
        let client_key = nkeys::KeyPair::new_user();
        let creds = bridge.handshake.credentials().clone();

        // Identity mismatch → refused.
        let mut bad = handshake_cmd(&creds, &client_key);
        bad["protocolVersion"] = json!(99);
        let reply = bridge.call(bad).await;
        assert_eq!(reply["error"], json!("pairing_identity_mismatch"));

        // Valid challenge → desktop nonce + signature.
        let reply = bridge.call(handshake_cmd(&creds, &client_key)).await;
        assert_eq!(reply["success"], json!(true), "got: {reply}");
        let data = &reply["data"];
        assert_eq!(data["pairId"], json!(creds.pair_id));
        let desktop_nonce = data["desktopNonce"].as_str().unwrap().to_string();
        // The desktop signature verifies against the transcript.
        let transcript = handshake_transcript(&HandshakeTranscript {
            pair_id: &creds.pair_id,
            desktop_id: &creds.desktop_id,
            desktop_public_key: &crate::remote::pairing::public_key(&creds).unwrap(),
            bridge_instance_id: bridge.handshake.bridge_instance_id(),
            device_id: "dev_test",
            client_public_key: &client_key.public_key(),
            client_nonce: "nonce-0123456789abcdef",
            desktop_nonce: &desktop_nonce,
        });
        let signature = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(data["desktopSignature"].as_str().unwrap())
            .unwrap();
        nkeys::KeyPair::from_public_key(&crate::remote::pairing::public_key(&creds).unwrap())
            .unwrap()
            .verify(transcript.as_bytes(), &signature)
            .unwrap();

        // Unknown challenge → expired.
        let reply = bridge
            .call(json!({
                "id": unique("cmd"),
                "type": "pair_handshake_confirm",
                "deviceId": "dev_test",
                "desktopNonce": "never-issued",
                "clientSignature": "x",
            }))
            .await;
        assert_eq!(reply["error"], json!("pairing_challenge_expired"));

        // A forged client signature → rejected.
        let forged = client_key.sign(b"a different transcript entirely").unwrap();
        let reply = bridge
            .call(json!({
                "id": unique("cmd"),
                "type": "pair_handshake_confirm",
                "deviceId": "dev_test",
                "desktopNonce": desktop_nonce,
                "clientSignature": base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(forged),
            }))
            .await;
        assert_eq!(reply["error"], json!("pairing_signature_invalid"));

        // A fresh challenge + correctly signed transcript activates the bridge
        // and persists the credential.
        let reply = bridge.call(handshake_cmd(&creds, &client_key)).await;
        let desktop_nonce = reply["data"]["desktopNonce"].as_str().unwrap().to_string();
        let transcript = handshake_transcript(&HandshakeTranscript {
            pair_id: &creds.pair_id,
            desktop_id: &creds.desktop_id,
            desktop_public_key: &crate::remote::pairing::public_key(&creds).unwrap(),
            bridge_instance_id: bridge.handshake.bridge_instance_id(),
            device_id: "dev_test",
            client_public_key: &client_key.public_key(),
            client_nonce: "nonce-0123456789abcdef",
            desktop_nonce: &desktop_nonce,
        });
        let signature = client_key.sign(transcript.as_bytes()).unwrap();
        let reply = bridge
            .call(json!({
                "id": unique("cmd"),
                "type": "pair_handshake_confirm",
                "deviceId": "dev_test",
                "desktopNonce": desktop_nonce,
                "clientSignature": base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(signature),
            }))
            .await;
        assert_eq!(reply["success"], json!(true), "got: {reply}");
        assert_eq!(reply["data"]["confirmed"], json!(true));
        assert_eq!(
            reply["data"]["features"],
            json!([
                "file_transfer_v1",
                "file_download_v2",
                "approval_tier_v1",
                "continue_run_v1",
                "prompt_receipt_v1",
                "session_files_v1",
                "skills_v1",
                "selective_events_v1",
                "workspace_pinning_v1",
                "workspace_create_v1",
                "desktop_settings_v1",
                "skill_management_v1",
                "compaction_v1",
                "provider_management_v1",
                "fork_v1",
                "tasks_v1",
                "task_deps_v1"
            ])
        );
        assert!(bridge.handshake.active_flag().load(Ordering::Acquire));
        assert!(crate::remote::pairing::load_creds().is_some());

        // Now ordinary commands pass the gate.
        let reply = bridge
            .call(json!({ "id": unique("cmd"), "type": "list_sessions" }))
            .await;
        assert_eq!(reply["success"], json!(true));

        // A candidate handshake preserves the authenticated session and does
        // not invalidate another outstanding challenge.
        let reply = bridge.call(handshake_cmd(&creds, &client_key)).await;
        let desktop_nonce = reply["data"]["desktopNonce"].as_str().unwrap().to_string();
        assert!(bridge.handshake.active_flag().load(Ordering::Acquire));
        let overlapping = bridge.call(handshake_cmd(&creds, &client_key)).await;
        assert_eq!(overlapping["success"], json!(true));
        let live = bridge
            .call(json!({ "id": unique("cmd"), "type": "list_sessions" }))
            .await;
        assert_eq!(live["success"], json!(true));
        let transcript = handshake_transcript(&HandshakeTranscript {
            pair_id: &creds.pair_id,
            desktop_id: &creds.desktop_id,
            desktop_public_key: &crate::remote::pairing::public_key(&creds).unwrap(),
            bridge_instance_id: bridge.handshake.bridge_instance_id(),
            device_id: "dev_test",
            client_public_key: &client_key.public_key(),
            client_nonce: "nonce-0123456789abcdef",
            desktop_nonce: &desktop_nonce,
        });
        let signature = client_key.sign(transcript.as_bytes()).unwrap();
        let reply = bridge
            .call(json!({
                "id": unique("cmd"),
                "type": "pair_handshake_confirm",
                "deviceId": "dev_test",
                "desktopNonce": desktop_nonce,
                "clientSignature": base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(signature),
            }))
            .await;
        assert_eq!(reply["success"], json!(true), "got: {reply}");
        assert!(bridge.handshake.active_flag().load(Ordering::Acquire));

        // A replayed confirmation is a reconnect, not a request for the
        // machine: with another device in control it must stay there, or the
        // two would evict each other every time either one reconnects.
        // A real second device: the pointer must name one this machine has, or
        // it is stale by definition and ignored.
        let other = "pair_elsewhere";
        let mut other_creds = bridge_creds();
        other_creds.pair_id = other.to_string();
        super::super::pairing::save_creds(&other_creds).expect("seed the other device");
        super::super::pairing::set_active(other).expect("persist the other device's control");
        SUPERVISOR.restore_active(Some(other.to_string()));
        let reply = bridge.call(handshake_cmd(&creds, &client_key)).await;
        let desktop_nonce = reply["data"]["desktopNonce"].as_str().unwrap().to_string();
        let transcript = handshake_transcript(&HandshakeTranscript {
            pair_id: &creds.pair_id,
            desktop_id: &creds.desktop_id,
            desktop_public_key: &crate::remote::pairing::public_key(&creds).unwrap(),
            bridge_instance_id: bridge.handshake.bridge_instance_id(),
            device_id: "dev_test",
            client_public_key: &client_key.public_key(),
            client_nonce: "nonce-0123456789abcdef",
            desktop_nonce: &desktop_nonce,
        });
        let signature = client_key.sign(transcript.as_bytes()).unwrap();
        let reply = bridge
            .call(json!({
                "id": unique("cmd"),
                "type": "pair_handshake_confirm",
                "deviceId": "dev_test",
                "desktopNonce": desktop_nonce,
                "clientSignature": base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(signature),
            }))
            .await;
        assert_eq!(reply["success"], json!(true), "got: {reply}");
        assert_eq!(
            SUPERVISOR.active_pair().as_deref(),
            Some(other),
            "a replayed confirmation must not take control"
        );
        // …and the reply must not hand it the conversation list either.
        assert_eq!(reply["data"]["presence"]["sessions"], Value::Null);

        SUPERVISOR.restore_active(None);
        let _ = super::super::pairing::remove_slot(other);
        bridge.stop().await;
    }

    #[tokio::test]
    async fn handshake_confirm_rejects_a_device_mismatch() {
        let _home = HomeGuard::new("cmd-device-mismatch");
        let bridge = Bridge::start().await;
        let client_key = nkeys::KeyPair::new_user();
        let creds = bridge.handshake.credentials().clone();
        let reply = bridge.call(handshake_cmd(&creds, &client_key)).await;
        let desktop_nonce = reply["data"]["desktopNonce"].as_str().unwrap().to_string();
        let reply = bridge
            .call(json!({
                "id": unique("cmd"),
                "type": "pair_handshake_confirm",
                "deviceId": "dev_other",
                "desktopNonce": desktop_nonce,
                "clientSignature": "eA",
            }))
            .await;
        assert_eq!(reply["error"], json!("pairing_identity_mismatch"));
        bridge.stop().await;
    }

    #[tokio::test]
    async fn handshake_confirm_reports_credential_save_failures() {
        let home = HomeGuard::new("cmd-save-fail");
        let bridge = Bridge::start().await;
        let client_key = nkeys::KeyPair::new_user();
        let creds = bridge.handshake.credentials().clone();
        let reply = bridge.call(handshake_cmd(&creds, &client_key)).await;
        let desktop_nonce = reply["data"]["desktopNonce"].as_str().unwrap().to_string();
        let transcript = handshake_transcript(&HandshakeTranscript {
            pair_id: &creds.pair_id,
            desktop_id: &creds.desktop_id,
            desktop_public_key: &crate::remote::pairing::public_key(&creds).unwrap(),
            bridge_instance_id: bridge.handshake.bridge_instance_id(),
            device_id: "dev_test",
            client_public_key: &client_key.public_key(),
            client_nonce: "nonce-0123456789abcdef",
            desktop_nonce: &desktop_nonce,
        });
        let signature = client_key.sign(transcript.as_bytes()).unwrap();
        // No HOME → the credential persist fails and the error surfaces.
        std::env::remove_var("HOME");
        std::env::remove_var("USERPROFILE");
        let reply = bridge
            .call(json!({
                "id": unique("cmd"),
                "type": "pair_handshake_confirm",
                "deviceId": "dev_test",
                "desktopNonce": desktop_nonce,
                "clientSignature": base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(signature),
            }))
            .await;
        assert_eq!(reply["success"], json!(false));
        bridge.stop().await;
        drop(home);
    }

    /// Activated bridge with a store and mock agent behind it.
    async fn active_bridge(label: &str) -> (HomeGuard, Bridge) {
        let home = HomeGuard::new(label);
        init_store();
        ensure_mock_agent();
        let bridge = Bridge::start().await;
        bridge.activate();
        (home, bridge)
    }

    /// The history lane's half of the lean feed, end to end through the real
    /// command path. The unit tests pin the rewrite itself; this pins that the
    /// command consults the declared flag, that an undeclared client still gets
    /// every field, and that a later connection cannot inherit the declaration.
    ///
    /// `mock_agent_lock` serializes this family, so no sibling test can observe
    /// the process-wide flag while it is flipped here.
    #[tokio::test]
    async fn lean_history_trims_only_for_a_client_that_declared_it() {
        use crate::remote_host::lean::LEAN_EVENTS_FEATURE;
        let _lock = mock_agent_lock();
        let (_home, bridge) = active_bridge("cmd-lean-history").await;
        let agent = ensure_mock_agent();
        agent.clear_scripts();
        let session = unique("sess-lean");
        agent.set_session_entries(
            &session,
            json!({ "entries": [
                {
                    "id": "u1", "kind": "user", "role": "user", "createdAtMs": 1,
                    "blocks": [{"kind": "text", "text": "question"}],
                },
                {
                    "id": "a1", "kind": "assistant", "role": "assistant", "createdAtMs": 2,
                    "runId": "run-1",
                    "blocks": [
                        {"kind": "reasoning", "text": "private reasoning body"},
                        {"kind": "text", "text": "visible answer"},
                        {"kind": "tool_call", "name": "read", "toolCallId": "c1",
                         "arguments": {"path": "/tmp/a", "offset": 5, "limit": 10}},
                        {"kind": "tool_call", "name": "shell", "toolCallId": "c2",
                         "arguments": {"command": "rm -rf build", "timeout": 30}},
                    ],
                },
                {
                    "id": "t1", "kind": "tool", "role": "tool", "createdAtMs": 3,
                    "blocks": [
                        {"kind": "tool_result", "toolCallId": "c1", "text": "tool output body"},
                    ],
                },
            ] }),
        );

        let read = |id: &'static str| {
            let bridge = &bridge;
            let session = session.clone();
            async move {
                bridge
                    .call(json!({ "id": unique(id), "type": "get_session_entries", "sessionId": session }))
                    .await
            }
        };
        // The paged shape is the one the phone actually uses (`useTimelineController`
        // always sends `before`), and it is a separate code path from the full
        // read above, so both are asserted throughout.
        let read_paged = |id: &'static str| {
            let bridge = &bridge;
            let session = session.clone();
            async move {
                bridge
                    .call(json!({
                        "id": unique(id),
                        "type": "get_session_entries",
                        "sessionId": session,
                        "before": i64::MAX,
                        "limit": 10,
                    }))
                    .await
            }
        };

        // Undeclared: the page is exactly what it always was. A distinct id per
        // call, because a repeated id is served from the single-flight response
        // cache and would mask the flag change.
        let full = read("lean-before").await;
        assert_eq!(full["success"], json!(true));
        assert_eq!(
            full["data"]["entries"][1]["blocks"][0]["text"],
            json!("private reasoning body")
        );
        assert_eq!(
            full["data"]["entries"][1]["blocks"][2]["arguments"]["offset"],
            json!(5)
        );
        assert_eq!(
            full["data"]["entries"][1]["blocks"][3]["arguments"]["command"],
            json!("rm -rf build"),
            "an undeclared client's page still carries every command"
        );
        assert_eq!(
            full["data"]["entries"][2]["blocks"][0]["text"],
            json!("tool output body")
        );
        let full_paged = read_paged("lean-before-paged").await;
        assert_eq!(
            full_paged["data"]["entries"][1]["blocks"][0]["text"],
            json!("private reasoning body")
        );

        apply_declared_features(
            &bridge.handshake,
            &bridge.pair_id,
            &[LEAN_EVENTS_FEATURE.to_string()],
        );
        let lean = read("lean-after").await;
        assert_eq!(lean["success"], json!(true));
        let blocks = lean["data"]["entries"][1]["blocks"].as_array().unwrap();
        // The reasoning block survives without its body, so the row still shows.
        assert_eq!(blocks[0]["kind"], json!("reasoning"));
        assert!(blocks[0].get("text").is_none());
        // Visible text is untouched.
        assert_eq!(blocks[1]["text"], json!("visible answer"));
        // A file row keeps only the keys a target can be derived from.
        assert_eq!(blocks[2]["arguments"], json!({"path": "/tmp/a"}));
        assert_eq!(blocks[2]["toolCallId"], json!("c1"));
        // A shell row keeps its identity and loses its arguments whole: the
        // command is what the client fetches when the row is opened, and the
        // call identity (plus the entry's runId) is what it needs to do so.
        assert_eq!(blocks[3]["toolCallId"], json!("c2"));
        assert!(blocks[3].get("arguments").is_none());
        assert_eq!(lean["data"]["entries"][1]["runId"], json!("run-1"));
        // The result block keeps its identity and loses its body.
        let result = &lean["data"]["entries"][2]["blocks"][0];
        assert_eq!(result["toolCallId"], json!("c1"));
        assert!(result.get("text").is_none());
        // Nothing structural moved: the page still carries all three entries.
        assert_eq!(lean["data"]["entries"].as_array().unwrap().len(), 3);
        assert_eq!(
            lean["data"]["entries"][0]["blocks"][0]["text"],
            json!("question")
        );

        let lean_paged = read_paged("lean-after-paged").await;
        assert_eq!(lean_paged["success"], json!(true));
        let paged_blocks = lean_paged["data"]["entries"][1]["blocks"]
            .as_array()
            .unwrap();
        assert!(
            paged_blocks[0].get("text").is_none(),
            "the paged path must trim too"
        );
        assert_eq!(paged_blocks[2]["arguments"], json!({"path": "/tmp/a"}));
        assert_eq!(paged_blocks[3]["toolCallId"], json!("c2"));
        assert!(
            paged_blocks[3].get("arguments").is_none(),
            "the paged path drops a shell call's arguments too"
        );
        assert!(lean_paged["data"]["entries"][2]["blocks"][0]
            .get("text")
            .is_none());
        assert_eq!(lean_paged["data"]["entries"].as_array().unwrap().len(), 3);

        // A later connection that declares only another capability must not
        // inherit this one, or an older build on the same pairing would receive
        // history it cannot read.
        apply_declared_features(
            &bridge.handshake,
            &bridge.pair_id,
            &["event_coalescing_v1".to_string()],
        );
        let full_again = read("lean-withdrawn").await;
        assert_eq!(
            full_again["data"]["entries"][1]["blocks"][0]["text"],
            json!("private reasoning body"),
            "withdrawing the declaration must restore the full page"
        );
        assert_eq!(
            full_again["data"]["entries"][2]["blocks"][0]["text"],
            json!("tool output body")
        );
        let full_again_paged = read_paged("lean-withdrawn-paged").await;
        assert_eq!(
            full_again_paged["data"]["entries"][1]["blocks"][0]["text"],
            json!("private reasoning body")
        );
    }

    /// The lazy-fetch half of the lean history lane, end to end: only a client
    /// that declared the feed can ask for a shell call's arguments back, the
    /// identity it sent is what reaches the Agent, and a missing identity is
    /// refused before the Agent is bothered.
    #[tokio::test]
    async fn tool_call_args_is_reachable_only_for_a_client_that_declared_lean() {
        use crate::remote_host::lean::LEAN_EVENTS_FEATURE;
        let _lock = mock_agent_lock();
        let (_home, bridge) = active_bridge("cmd-tool-args").await;
        let agent = ensure_mock_agent();
        agent.clear_scripts();
        let session = unique("sess-args");
        let run = unique("run-args");
        let ask = |id: &'static str| {
            let bridge = &bridge;
            let session = session.clone();
            let run = run.clone();
            async move {
                bridge
                    .call(json!({
                        "id": unique(id),
                        "type": "get_tool_call_args",
                        "sessionId": session,
                        "runId": run,
                        "toolCallId": "c1",
                    }))
                    .await
            }
        };

        // Undeclared: the command must be as unreachable as it is for an older
        // bridge — the same "unsupported" the dispatch catch-all gives — and it
        // must not reach the Agent.
        agent.clear_requests();
        let reply = ask("args-undeclared").await;
        assert_eq!(reply["success"], json!(false));
        assert!(
            reply["error"]
                .as_str()
                .unwrap()
                .contains("Unsupported command"),
            "got: {reply}"
        );
        assert!(!agent.served("get_tool_call_args", &session));

        apply_declared_features(
            &bridge.handshake,
            &bridge.pair_id,
            &[LEAN_EVENTS_FEATURE.to_string()],
        );
        agent.script_for(
            "get_tool_call_args",
            &session,
            true,
            json!({"toolCallId": "c1", "name": "shell", "arguments": {"command": "ls -la", "timeout": 30}}),
            "",
        );
        let reply = ask("args-declared").await;
        assert_eq!(reply["success"], json!(true), "got: {reply}");
        assert_eq!(reply["data"]["arguments"]["command"], json!("ls -la"));
        let forwarded = agent
            .last_full_request("get_tool_call_args")
            .expect("the declared client's read must reach the Agent");
        assert_eq!(forwarded.session_id, session);
        assert_eq!(forwarded.run_id, run);
        assert_eq!(forwarded.tool_call_id.as_deref(), Some("c1"));

        // A call identity is required: refuse locally, never ask the Agent for
        // "whatever call happens to be first".
        agent.clear_requests();
        let reply = bridge
            .call(json!({
                "id": unique("args-no-call"),
                "type": "get_tool_call_args",
                "sessionId": session,
                "runId": run,
            }))
            .await;
        assert_eq!(reply["success"], json!(false));
        assert!(
            reply["error"].as_str().unwrap().contains("toolCallId"),
            "got: {reply}"
        );
        assert!(!agent.served("get_tool_call_args", &session));

        // An Agent-side failure (unknown call) is surfaced, not swallowed.
        agent.script_for(
            "get_tool_call_args",
            &session,
            false,
            json!(null),
            "tool call not found",
        );
        let reply = ask("args-missing").await;
        assert_eq!(reply["success"], json!(false));
        assert!(reply["error"]
            .as_str()
            .unwrap()
            .contains("tool call not found"));

        // Withdrawing the declaration takes the command away again.
        apply_declared_features(
            &bridge.handshake,
            &bridge.pair_id,
            &["event_coalescing_v1".to_string()],
        );
        let reply = ask("args-withdrawn").await;
        assert_eq!(reply["success"], json!(false));
        assert!(reply["error"]
            .as_str()
            .unwrap()
            .contains("Unsupported command"));
    }

    /// The mock's *own* default answer for the lazy fetch, unscripted: a call
    /// identity the recorded page does not contain comes back with a null name
    /// and null arguments — the shape the real Agent gives for a call it cannot
    /// resolve. A mock that invented a body would let a client-side fetch bug
    /// look like a working fetch, which is why the empty answer is asserted
    /// rather than left to whatever the default happens to do.
    #[tokio::test]
    async fn the_mock_answers_an_unknown_tool_call_identity_with_nothing() {
        use crate::remote_host::lean::LEAN_EVENTS_FEATURE;
        let _lock = mock_agent_lock();
        let (_home, bridge) = active_bridge("cmd-tool-args-unknown").await;
        let agent = ensure_mock_agent();
        agent.clear_scripts();
        let session = unique("sess-args-unknown");
        let run = unique("run-args-unknown");
        // A page that *does* carry a call, so the miss is the identity and not
        // an empty page.
        agent.set_session_entries(
            &session,
            json!({ "entries": [{
                "id": "a1", "kind": "assistant", "role": "assistant",
                "runId": run, "blocks": [
                    {"kind": "tool_call", "name": "shell", "toolCallId": "c-recorded",
                     "arguments": {"command": "ls -la"}},
                ],
            }] }),
        );
        apply_declared_features(
            &bridge.handshake,
            &bridge.pair_id,
            &[LEAN_EVENTS_FEATURE.to_string()],
        );

        let reply = bridge
            .call(json!({
                "id": unique("args-unknown"),
                "type": "get_tool_call_args",
                "sessionId": session,
                "runId": run,
                "toolCallId": "c-absent",
            }))
            .await;
        assert_eq!(reply["success"], json!(true), "got: {reply}");
        assert_eq!(reply["data"]["toolCallId"], json!("c-absent"));
        assert_eq!(
            reply["data"]["name"],
            json!(null),
            "an unresolvable call has no name to report: {reply}"
        );
        assert_eq!(
            reply["data"]["arguments"],
            json!(null),
            "and nothing to hand back: {reply}"
        );

        // The same request for the recorded identity *does* answer with the
        // arguments, so the empty answer above is a miss and not a broken mock.
        let reply = bridge
            .call(json!({
                "id": unique("args-recorded"),
                "type": "get_tool_call_args",
                "sessionId": session,
                "runId": run,
                "toolCallId": "c-recorded",
            }))
            .await;
        assert_eq!(reply["success"], json!(true), "got: {reply}");
        assert_eq!(reply["data"]["name"], json!("shell"));
        assert_eq!(reply["data"]["arguments"]["command"], json!("ls -la"));

        // Withdraw the declaration before returning: the lean feed is a
        // process-global flag (`remote_host::lean`), so a test that leaves it on
        // silently turns every later test in this binary into a declared client
        // — the sibling test above ends the same way, for the same reason. (This
        // is what a first cut of this test got wrong: it passed on its own and
        // reddened `a_client_that_did_not_declare_keeps_the_full_lane`.)
        apply_declared_features(
            &bridge.handshake,
            &bridge.pair_id,
            &["event_coalescing_v1".to_string()],
        );
        assert!(
            !crate::remote_host::lean::enabled(),
            "the declaration must not outlive the test"
        );
    }

    #[tokio::test]
    async fn presence_and_catalog_commands() {
        let _lock = mock_agent_lock();
        let (_home, bridge) = active_bridge("cmd-presence").await;

        let reply = bridge
            .call(json!({ "id": unique("cmd"), "type": "get_presence" }))
            .await;
        assert_eq!(reply["success"], json!(true));
        assert_eq!(reply["data"]["online"], json!(true));
        assert_eq!(reply["data"]["pairId"], json!(bridge.pair_id));

        // Empty store → empty session/workspace lists.
        let reply = bridge
            .call(json!({ "id": unique("cmd"), "type": "list_sessions" }))
            .await;
        assert_eq!(reply["data"]["sessions"], json!([]));
        let reply = bridge
            .call(json!({ "id": unique("cmd"), "type": "list_workspaces" }))
            .await;
        assert_eq!(reply["success"], json!(true));

        // A thread with an agent session shows up, streaming while its run is
        // active.
        let session = unique("sess");
        let thread = crate::store::create_thread(crate::store::CreateThreadInput {
            mode: "chat".to_string(),
            title: Some("From remote".to_string()),
            workspace_id: None,
            workspace_path: None,
            workspace_name: None,
            agent_session_id: Some(session.clone()),
        })
        .unwrap();
        let cwd = crate::agent_bridge::workspace_path_for_thread(&thread.id).unwrap();
        let file = std::path::Path::new(&cwd).join("mobile-file.txt");
        std::fs::create_dir_all(&cwd).unwrap();
        std::fs::write(&file, "hello").unwrap();
        let listing = bridge
            .call(json!({
                "id": unique("cmd"), "type": "list_session_files",
                "sessionId": session, "chunkedRead": true
            }))
            .await;
        assert_eq!(listing["success"], json!(true));
        // The root is the session workspace, transported in the ordinary
        // spelling the phone renders (no Windows `\\?\` extended-length
        // prefix), so compare the directories rather than the raw spelling.
        let root_path = listing["data"]["rootPath"].as_str().unwrap();
        assert!(!root_path.starts_with(r"\\?\"));
        assert_eq!(
            std::path::Path::new(root_path).canonicalize().unwrap(),
            std::path::Path::new(&cwd).canonicalize().unwrap()
        );
        assert!(listing["data"]["entries"]
            .as_array()
            .unwrap()
            .iter()
            .any(|entry| entry["name"] == "mobile-file.txt" && entry["size"] == 5));
        let missing = bridge
            .call(json!({
                "id": unique("cmd"), "type": "list_session_files", "sessionId": "missing-session"
            }))
            .await;
        assert_eq!(missing["success"], json!(false));
        crate::store::create_run(crate::store::CreateRunInput {
            id: None,
            thread_id: thread.id.clone(),
            trigger_message_id: None,
            model_provider: None,
            model_id: None,
        })
        .unwrap();
        let reply = bridge
            .call(json!({ "id": unique("cmd"), "type": "list_sessions" }))
            .await;
        let sessions = reply["data"]["sessions"].as_array().unwrap();
        let row = sessions
            .iter()
            .find(|row| row["sessionId"] == json!(session))
            .expect("thread listed");
        assert_eq!(row["title"], json!("From remote"));
        assert_eq!(row["streaming"], json!(true));
        assert!(row["parentSessionId"].is_null());
        crate::store::sync_thread_parent_session(&session, "parent-session").unwrap();
        let reply = bridge
            .call(json!({ "id": unique("cmd"), "type": "list_sessions" }))
            .await;
        assert_eq!(
            reply["data"]["sessions"][0]["parentSessionId"],
            json!("parent-session")
        );

        bridge.stop().await;
    }

    #[tokio::test]
    async fn catalog_commands_report_store_failures() {
        let _lock = mock_agent_lock();
        // No init_store and no .future dir → the DB connect fails.
        let _home = HomeGuard::new("cmd-store-down");
        ensure_mock_agent();
        let bridge = Bridge::start().await;
        bridge.activate();

        let reply = bridge
            .call(json!({ "id": unique("cmd"), "type": "list_sessions" }))
            .await;
        assert_eq!(reply["success"], json!(false));
        let reply = bridge
            .call(json!({ "id": unique("cmd"), "type": "list_workspaces" }))
            .await;
        assert_eq!(reply["success"], json!(false));
        // Settings handlers report the same store failure.
        let reply = bridge
            .call(json!({ "id": unique("cmd"), "type": "get_settings" }))
            .await;
        assert_eq!(reply["success"], json!(false));
        let reply = bridge
            .call(json!({ "id": unique("cmd"), "type": "set_approval_tier", "tier": "sandbox" }))
            .await;
        assert_eq!(reply["success"], json!(false));
        bridge.stop().await;
    }

    #[tokio::test]
    async fn oversized_reads_cross_the_real_reply_path_without_losing_history_or_projection() {
        let _lock = mock_agent_lock();
        let (_home, bridge) = active_bridge("large-read-pages").await;
        let agent = ensure_mock_agent();
        agent.clear_scripts();
        let session = unique("sess");
        let large = "中文".repeat(200_000);
        agent.script_typed_for("get_events_since", &session, json!({
            "runId": "r", "events": [], "truncated": true,
            "projection": {"runId": "r", "cursor": 20, "events": [{"type": "text_chunk", "idx": 20, "data": large}]}
        }));
        let legacy = bridge.call(json!({"id": unique("cmd"), "type": "get_events_since", "sessionId": session, "runId": "r", "sinceIdx": -1})).await;
        assert_eq!(legacy["success"], false);
        assert_eq!(legacy["error"], "remote_reply_too_large");

        agent.script_typed_for("get_events_since", &session, json!({
            "runId": "r", "events": [], "truncated": true,
            "projection": {"runId": "r", "cursor": 20, "events": [{"type": "text_chunk", "idx": 20, "data": large}]}
        }));
        let mut page = bridge.call(json!({"id": unique("cmd"), "type": "get_events_since", "sessionId": session, "runId": "r", "sinceIdx": -1, "chunkedRead": true})).await;
        // The live source changes while the phone reads: pages must still be
        // from the original immutable snapshot, not a mixture of generations.
        agent.script_typed_for(
            "get_events_since",
            &session,
            json!({"runId": "r", "events": []}),
        );
        let mut bytes = Vec::new();
        loop {
            assert_eq!(page["success"], true);
            assert!(serde_json::to_vec(&page).unwrap().len() < 512 * 1024);
            let part = &page["data"]["readChunk"];
            bytes.extend(
                URL_SAFE_NO_PAD
                    .decode(part["data"].as_str().unwrap())
                    .unwrap(),
            );
            if part["nextOffset"] == part["totalBytes"] {
                break;
            }
            page = bridge.call(json!({"id": unique("cmd"), "type": "get_read_chunk", "sessionId": session, "runId": "r", "replyId": part["id"], "offset": part["nextOffset"]})).await;
        }
        let restored: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(restored["projection"]["cursor"], 20);
        assert_eq!(restored["projection"]["events"][0]["data"], large);

        let mut entries = vec![
            json!({"id":"u", "kind":"user", "role":"user", "blocks":[{"kind":"text", "text":"question"}]}),
        ];
        entries.extend((0..12).map(|i| json!({"id":format!("a{i}"), "kind":"assistant", "role":"assistant", "runId":"r", "blocks":[{"kind":"text", "text":"x".repeat(100*1024)}]})));
        agent.set_session_entries(&session, json!({"entries": entries}));
        let mut page = bridge.call(json!({"id":unique("cmd"), "type":"get_session_entries", "sessionId":session, "before":i64::MAX, "limit":10, "chunkedRead":true})).await;
        let mut bytes = Vec::new();
        loop {
            assert_eq!(page["success"], true);
            let part = &page["data"]["readChunk"];
            bytes.extend(
                URL_SAFE_NO_PAD
                    .decode(part["data"].as_str().unwrap())
                    .unwrap(),
            );
            if part["nextOffset"] == part["totalBytes"] {
                break;
            }
            page = bridge.call(json!({"id":unique("cmd"), "type":"get_read_chunk", "sessionId":session, "replyId":part["id"], "offset":part["nextOffset"]})).await;
        }
        let restored: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(restored["entries"].as_array().unwrap().len(), 13);
        assert_eq!(restored["entries"][0]["id"], "u");
        assert_eq!(restored["entries"][12]["id"], "a11");
        bridge.stop().await;
    }

    #[tokio::test]
    async fn history_commands_page_and_fail() {
        let _lock = mock_agent_lock();
        let (_home, bridge) = active_bridge("cmd-history").await;
        let agent = ensure_mock_agent();
        agent.clear_scripts();
        let session = unique("sess");
        let thread = crate::store::create_thread(crate::store::CreateThreadInput {
            mode: "chat".to_string(),
            title: Some("History".to_string()),
            workspace_id: None,
            workspace_path: None,
            workspace_name: None,
            agent_session_id: Some(session.clone()),
        })
        .unwrap();
        let history_run = crate::store::create_run(crate::store::CreateRunInput {
            id: Some(unique("run-history")),
            thread_id: thread.id,
            trigger_message_id: None,
            model_provider: None,
            model_id: None,
        })
        .unwrap();
        crate::store::update_run_status_if_active(crate::store::UpdateRunStatusInput {
            run_id: history_run.id.clone(),
            status: "failed".to_string(),
            error_message: None,
            error_type: None,
        })
        .unwrap();

        let reply = bridge
            .call(json!({ "id": unique("cmd"), "type": "get_messages", "sessionId": session }))
            .await;
        assert_eq!(reply["success"], json!(true));
        assert_eq!(reply["data"]["total"], json!(2));
        assert_eq!(reply["data"]["hasMore"], json!(false));

        // An explicit positive limit pages instead of using the default.
        let reply = bridge
            .call(json!({ "id": unique("cmd"), "type": "get_messages", "sessionId": session, "limit": 1 }))
            .await;
        assert_eq!(reply["success"], json!(true));
        assert_eq!(reply["data"]["messages"].as_array().unwrap().len(), 1);
        assert_eq!(reply["data"]["hasMore"], json!(true));

        // Agent failure → the remote-specific error text.
        agent.script_for(
            "get_messages",
            &session,
            false,
            json!(null),
            "agent exploded",
        );
        let reply = bridge
            .call(json!({ "id": unique("cmd"), "type": "get_messages", "sessionId": session }))
            .await;
        assert_eq!(reply["success"], json!(false));
        assert!(reply["error"]
            .as_str()
            .unwrap()
            .contains("conversation history is unavailable"));

        // Entries page through too.
        agent.set_session_entries(
            &session,
            json!({ "entries": [{
                "id": "persisted-assistant",
                "kind": "assistant",
                "role": "assistant",
                "blocks":[{"kind":"text","text":"partial"}],
                "createdAtMs":1000,"runId":history_run.id,"run":{"status":"failed","durationMs":1000,"error":"synthetic failure"}
            }] }),
        );
        let reply = bridge
            .call(
                json!({ "id": unique("cmd"), "type": "get_session_entries", "sessionId": session }),
            )
            .await;
        assert_eq!(reply["success"], json!(true));
        assert_eq!(reply["data"]["entries"].as_array().unwrap().len(), 1);
        assert_eq!(
            reply["data"]["entries"][0]["run"]["status"],
            json!("failed")
        );
        assert!(reply["data"]["entries"][0]["run"]["durationMs"].is_number());

        // Mobile's tail request is forwarded as an Agent backward cursor; the
        // desktop must not expand it back into a full-history forward loop.
        let reply = bridge
            .call(json!({
                    "id": unique("cmd"),
                    "type": "get_session_entries",
                    "sessionId": session,
                    "before": i64::MAX,
                    "limit": 10
            }))
            .await;
        assert_eq!(reply["success"], json!(true));
        assert_eq!(reply["data"]["entries"].as_array().unwrap().len(), 1);

        // Entries honor an explicit positive limit too.
        let reply = bridge
            .call(json!({ "id": unique("cmd"), "type": "get_session_entries", "sessionId": session, "limit": 5 }))
            .await;
        assert_eq!(reply["success"], json!(true));
        assert_eq!(reply["data"]["entries"].as_array().unwrap().len(), 1);

        // Production Agent responses carry typed payload only (`data` is
        // empty); mobile history must still flow through the remote bridge.
        agent.script_typed_for(
            "get_session_entries",
            &session,
            json!({"entries": [{
                "id": "typed-mobile-entry",
                "role": "assistant",
                "kind":"assistant","createdAtMs":1000,"blocks":[{"kind":"text","text":"typed mobile history"}]
            }]}),
        );
        let reply = bridge
            .call(
                json!({ "id": unique("cmd"), "type": "get_session_entries", "sessionId": session }),
            )
            .await;
        assert_eq!(reply["success"], json!(true));
        assert_eq!(
            reply["data"]["entries"][0]["blocks"][0]["text"],
            json!("typed mobile history")
        );

        agent.script_for("get_session_entries", &session, false, json!(null), "nope");
        let reply = bridge
            .call(
                json!({ "id": unique("cmd"), "type": "get_session_entries", "sessionId": session }),
            )
            .await;
        assert_eq!(reply["success"], json!(false));

        // Events backfill (success and failure).
        let reply = bridge
            .call(json!({ "id": unique("cmd"), "type": "get_events_since", "sessionId": session, "runId": "run-1", "sinceIdx": -1 }))
            .await;
        assert_eq!(reply["success"], json!(true));
        assert_eq!(reply["data"]["events"], json!([]));
        // A positive limit uses the caller's page size (limit > 0 branch).
        let reply = bridge
            .call(json!({ "id": unique("cmd"), "type": "get_events_since", "sessionId": session, "runId": "run-1", "sinceIdx": -1, "limit": 5 }))
            .await;
        assert_eq!(reply["success"], json!(true));
        assert_eq!(reply["data"]["events"], json!([]));
        agent.script_for(
            "get_events_since",
            &session,
            false,
            json!(null),
            "stale run",
        );
        let reply = bridge
            .call(json!({ "id": unique("cmd"), "type": "get_events_since", "sessionId": session, "runId": "run-1", "sinceIdx": -1 }))
            .await;
        assert_eq!(reply["success"], json!(false));

        bridge.stop().await;
    }

    #[tokio::test]
    async fn mobile_snapshot_bootstrap_is_opt_in_chunked_and_legacy_safe() {
        use base64::Engine as _;
        let _lock = mock_agent_lock();
        let (_home, bridge) = active_bridge("snapshot-bootstrap").await;
        let agent = ensure_mock_agent();
        agent.clear_scripts();
        let session = unique("snapshot-session");
        let snapshot = json!({"runSnapshot":true,"events":[],"watermark":2000,"nextSinceIdx":2000,"hasMore":false,
        "projection":{"runId":"r","cursor":2000,"events":[
            {"type":"agent_start","idx":0,"data":"{}"},
            {"type":"text_chunk","idx":2000,"data":json!({"text":"x".repeat(600_000)}).to_string()}
        ]}});
        agent.script_for("get_run_snapshot", &session, true, snapshot.clone(), "");
        let mut reply = bridge
            .call(
                json!({"id":unique("cmd"),"type":"get_events_since","sessionId":session,
            "runId":"r","sinceIdx":-1,"limit":1000,"chunkedRead":true,"preferSnapshot":true}),
            )
            .await;
        assert_eq!(reply["success"], true);
        let id = reply["data"]["readChunk"]["id"]
            .as_str()
            .unwrap()
            .to_owned();
        let mut bytes = Vec::new();
        loop {
            let chunk = &reply["data"]["readChunk"];
            assert_eq!(chunk["offset"].as_u64().unwrap() as usize, bytes.len());
            bytes.extend(
                base64::engine::general_purpose::URL_SAFE_NO_PAD
                    .decode(chunk["data"].as_str().unwrap())
                    .unwrap(),
            );
            if bytes.len() == chunk["totalBytes"].as_u64().unwrap() as usize {
                break;
            }
            reply = bridge
                .call(
                    json!({"id":unique("cmd"),"type":"get_read_chunk","sessionId":session,
                "runId":"r","replyId":id,"offset":bytes.len()}),
                )
                .await;
            assert_eq!(reply["success"], true);
        }
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&bytes).unwrap(),
            snapshot
        );
        assert!(!agent
            .requests()
            .iter()
            .any(|(command, sid)| command == "get_events_since" && sid == &session));
        // An old Agent explicitly declining the new command keeps the old
        // bounded raw replay path, instead of wedging cold opens.
        agent.script_for(
            "get_run_snapshot",
            &session,
            false,
            json!(null),
            "unknown command: get_run_snapshot",
        );
        agent.script_typed_for(
            "get_events_since",
            &session,
            json!({"runId":"r","events":[{"type":"agent_start","idx":0}]}),
        );
        let legacy = bridge
            .call(
                json!({"id":unique("cmd"),"type":"get_events_since","sessionId":session,
            "runId":"r","sinceIdx":-1,"chunkedRead":true,"preferSnapshot":true}),
            )
            .await;
        assert_eq!(legacy["success"], true);
        assert!(legacy["data"]["runSnapshot"].is_null());
        assert_eq!(legacy["data"]["events"].as_array().unwrap().len(), 1);
        // Transport/storage failures must not silently trigger full downloads.
        agent.script_for(
            "get_run_snapshot",
            &session,
            false,
            json!(null),
            "storage unavailable",
        );
        let failed = bridge
            .call(
                json!({"id":unique("cmd"),"type":"get_events_since","sessionId":session,
            "runId":"r","sinceIdx":-1,"chunkedRead":true,"preferSnapshot":true}),
            )
            .await;
        assert_eq!(failed["success"], false);
        // Snapshot preference cannot change an incremental request's semantics.
        let before = agent
            .requests()
            .iter()
            .filter(|(command, _)| command == "get_run_snapshot")
            .count();
        let tail = bridge
            .call(
                json!({"id":unique("cmd"),"type":"get_events_since","sessionId":session,
            "runId":"r","sinceIdx":2000,"chunkedRead":true,"preferSnapshot":true}),
            )
            .await;
        assert_eq!(tail["success"], true);
        assert_eq!(
            agent
                .requests()
                .iter()
                .filter(|(command, _)| command == "get_run_snapshot")
                .count(),
            before
        );
        bridge.stop().await;
    }

    /// A missing transcript must not be reported as a successful empty history.
    /// Clients retain their previous messages and receive a recoverable source
    /// error, while a session without settings still answers an empty state.
    #[tokio::test]
    async fn a_missing_agent_session_reports_unavailable_history_and_empty_state() {
        let _lock = mock_agent_lock();
        let (_home, bridge) = active_bridge("cmd-missing-session").await;
        let agent = ensure_mock_agent();
        agent.clear_scripts();
        let session = unique("gone");
        // The Agent's own wording for a session id it cannot resolve.
        let missing = "session not found — pass a valid session_id (new_session creates one)";

        for command in ["get_session_entries", "get_state"] {
            for _ in 0..2 {
                agent.script_for(command, &session, false, json!(null), missing);
            }
        }

        // Neither paging direction may erase cached history by reporting a
        // missing source as a successful empty page.
        let reply = bridge
            .call(json!({
                "id": unique("cmd"), "type": "get_session_entries", "sessionId": session,
                "before": i64::MAX, "limit": 10
            }))
            .await;
        assert_eq!(reply["success"], json!(false));
        assert!(reply["error"]
            .as_str()
            .unwrap()
            .contains("history is unavailable"));

        // The forward (non-`before`) read reports the same missing source.
        let reply = bridge
            .call(
                json!({ "id": unique("cmd"), "type": "get_session_entries", "sessionId": session }),
            )
            .await;
        assert_eq!(reply["success"], json!(false));
        assert!(reply["error"]
            .as_str()
            .unwrap()
            .contains("history is unavailable"));

        // A session without a settings snapshot still has an empty state.
        let reply = bridge
            .call(json!({ "id": unique("cmd"), "type": "get_state", "sessionId": session }))
            .await;
        assert_eq!(reply["success"], json!(true));
        assert_eq!(reply["data"], json!({}));

        // Only the missing identity is special-cased; a genuine failure still
        // surfaces to the client.
        agent.script_for(
            "get_session_entries",
            &session,
            false,
            json!(null),
            "agent exploded",
        );
        let reply = bridge
            .call(
                json!({ "id": unique("cmd"), "type": "get_session_entries", "sessionId": session }),
            )
            .await;
        assert_eq!(reply["success"], json!(false));
        assert_eq!(reply["error"], json!("agent exploded"));

        bridge.stop().await;
    }

    #[tokio::test]
    async fn replay_continuations_keep_agent_has_more_and_stop_at_the_pinned_watermark() {
        let _lock = mock_agent_lock();
        let (_home, bridge) = active_bridge("bounded-replay").await;
        let agent = ensure_mock_agent();
        agent.clear_scripts();
        let session = unique("session");
        let events = |start: i64, end: i64| -> Value {
            json!((start..=end)
                .map(|idx| json!({"type": "text_chunk", "idx": idx, "data": "{}"}))
                .collect::<Vec<_>>())
        };
        agent.script_typed_for(
            "get_events_since",
            &session,
            json!({
                "runId": "r", "events": events(0, 4), "hasMore": false
            }),
        );
        let first = bridge
            .call(json!({
                "id": unique("cmd"), "type": "get_events_since", "sessionId": session,
                "runId": "r", "sinceIdx": -1, "limit": 2
            }))
            .await;
        assert_eq!(first["success"], true);
        assert_eq!(first["data"]["watermark"], 4);
        assert_eq!(first["data"]["nextSinceIdx"], 1);
        assert_eq!(first["data"]["hasMore"], true);

        agent.script_typed_for(
            "get_events_since",
            &session,
            json!({
                "runId": "r", "events": events(2, 3), "hasMore": true
            }),
        );
        let middle = bridge
            .call(json!({
                "id": unique("cmd"), "type": "get_events_since", "sessionId": session,
                "runId": "r", "sinceIdx": 1, "limit": 2, "replayUntilIdx": 4
            }))
            .await;
        assert_eq!(middle["success"], true);
        assert_eq!(middle["data"]["watermark"], 4);
        assert_eq!(middle["data"]["nextSinceIdx"], 3);
        assert_eq!(
            middle["data"]["hasMore"], true,
            "Agent still has another page"
        );

        // The task kept generating, but this replay must finish at its
        // original watermark rather than chasing the growing tail forever.
        agent.script_typed_for(
            "get_events_since",
            &session,
            json!({
                "runId": "r", "events": events(4, 5), "hasMore": true
            }),
        );
        let last = bridge
            .call(json!({
                "id": unique("cmd"), "type": "get_events_since", "sessionId": session,
                "runId": "r", "sinceIdx": 3, "limit": 2, "replayUntilIdx": 4
            }))
            .await;
        assert_eq!(last["success"], true);
        assert_eq!(last["data"]["watermark"], 4);
        assert_eq!(last["data"]["nextSinceIdx"], 4);
        assert_eq!(last["data"]["hasMore"], false);
        assert_eq!(last["data"]["events"].as_array().unwrap().len(), 1);
        bridge.stop().await;
    }

    #[tokio::test]
    async fn transfer_control_commands() {
        let _lock = mock_agent_lock();
        let (_home, bridge) = active_bridge("cmd-transfer").await;
        let agent = ensure_mock_agent();
        agent.clear_scripts();

        // upload_init validation + success.
        let reply = bridge
            .call(json!({ "id": unique("cmd"), "type": "upload_init", "name": "a.txt", "kind": "file", "originalSize": 0, "transferSize": 0 }))
            .await;
        assert_eq!(reply["success"], json!(false));
        let reply = bridge
            .call(json!({ "id": unique("cmd"), "type": "upload_init", "name": "a.txt", "kind": "file", "originalSize": 4, "transferSize": 4 }))
            .await;
        assert_eq!(reply["success"], json!(true));
        let upload_id = reply["data"]["uploadId"].as_str().unwrap().to_string();

        // upload_complete on an incomplete upload → error; after the bytes, ok.
        let reply = bridge
            .call(
                json!({ "id": unique("cmd"), "type": "upload_complete", "transferId": upload_id }),
            )
            .await;
        assert_eq!(reply["success"], json!(false));
        transfer::write_upload_chunk(&upload_id, 0, b"data").unwrap();
        let reply = bridge
            .call(
                json!({ "id": unique("cmd"), "type": "upload_complete", "transferId": upload_id }),
            )
            .await;
        assert_eq!(reply["success"], json!(true), "got: {reply}");
        assert_eq!(reply["data"]["contentHash"].as_str().unwrap().len(), 64);

        // upload_cancel always succeeds.
        let reply = bridge
            .call(json!({ "id": unique("cmd"), "type": "upload_cancel", "transferId": upload_id }))
            .await;
        assert_eq!(reply["success"], json!(true));

        // download_prepare failure (unknown attachment) and success.
        let session = unique("sess");
        agent.set_session_entries(&session, json!({ "entries": [] }));
        let reply = bridge
            .call(json!({ "id": unique("cmd"), "type": "download_prepare", "sessionId": session, "filePath": "/tmp/none.txt" }))
            .await;
        assert_eq!(reply["success"], json!(false));

        let dir = std::env::temp_dir().join(unique("futureos-cmd-dl"));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("take.txt");
        std::fs::write(&file, b"take me").unwrap();
        agent.set_session_entries(
            &session,
            json!({"entries":[{"metadata":{"attachments":[{"path": file.to_string_lossy()}]}}]}),
        );
        let reply = bridge
            .call(json!({ "id": unique("cmd"), "type": "download_prepare", "sessionId": session, "filePath": file.to_string_lossy() }))
            .await;
        assert_eq!(reply["success"], json!(true), "got: {reply}");
        let transfer_id = reply["data"]["transferId"].as_str().unwrap().to_string();

        let reply = bridge
            .call(json!({ "id": unique("cmd"), "type": "download_cancel", "transferId": transfer_id }))
            .await;
        assert_eq!(reply["success"], json!(true));

        std::fs::remove_dir_all(dir).ok();
        bridge.stop().await;
    }

    #[tokio::test]
    async fn prompt_creates_threads_and_rejects_busy_sessions() {
        let _lock = mock_agent_lock();
        let (_home, bridge) = active_bridge("cmd-prompt").await;
        let agent = ensure_mock_agent();

        // Workspace mode without a workspace id → validation error.
        let reply = bridge
            .call(json!({ "id": unique("cmd"), "type": "prompt", "message": "hi", "mode": "workspace" }))
            .await;
        assert_eq!(reply["success"], json!(false));
        assert!(reply["error"]
            .as_str()
            .unwrap()
            .contains("Select a workspace"));

        // Chat mode with an empty session id → lazy thread + agent session.
        let prompt_id = unique("cmd");
        agent.complete_next_run_stream();
        let reply = bridge
            .call(json!({ "id": prompt_id, "type": "prompt", "message": "hello there", "modelId": "m1", "providerId": "p1", "level": "high" }))
            .await;
        assert_eq!(reply["success"], json!(true), "got: {reply}");
        let session = reply["data"]["sessionId"].as_str().unwrap().to_string();
        assert!(session.starts_with("mock-session-"));
        let thread_id = reply["data"]["threadId"].as_str().unwrap().to_string();
        // The run the ack carried settles through the normal collector.
        let run_id = reply["data"]["runId"].as_str().unwrap().to_string();
        wait_until("the run must settle", Duration::from_secs(5), || {
            crate::store::get_run(&run_id)
                .expect("run lookup")
                .expect("run row")
                .status
                != "running"
        })
        .await;
        assert_eq!(
            crate::store::get_run(&run_id)
                .expect("run lookup")
                .expect("run row")
                .status,
            "completed"
        );

        // The run id is a durable receipt keyed by the mobile command id. It
        // remains queryable after the in-memory single-flight cache expires,
        // and resending the prompt returns the same ack without a second run.
        let receipt = bridge
            .call(
                json!({ "id": unique("cmd"), "type": "get_prompt_receipt", "promptId": prompt_id }),
            )
            .await;
        assert_eq!(receipt["data"]["runId"], json!(run_id));
        tokio::time::sleep(Duration::from_millis(650)).await;
        let retried = bridge
            .call(json!({ "id": prompt_id, "type": "prompt", "message": "hello there" }))
            .await;
        assert_eq!(retried["data"]["runId"], json!(run_id));

        // A follow-up prompt on the idle session reuses the thread.
        agent.complete_next_run_stream();
        let reply = bridge
            .call(json!({ "id": unique("cmd"), "type": "prompt", "sessionId": session, "message": "again" }))
            .await;
        assert_eq!(reply["success"], json!(true), "got: {reply}");
        assert_eq!(reply["data"]["threadId"], json!(thread_id));

        // While a run for the session is active, another prompt is refused.
        crate::store::create_run(crate::store::CreateRunInput {
            id: None,
            thread_id: thread_id.clone(),
            trigger_message_id: None,
            model_provider: None,
            model_id: None,
        })
        .unwrap();
        let reply = bridge
            .call(json!({ "id": unique("cmd"), "type": "prompt", "sessionId": session, "message": "too soon" }))
            .await;
        assert_eq!(reply["success"], json!(false));
        assert!(reply["error"].as_str().unwrap().contains("still running"));

        bridge.stop().await;
    }

    #[tokio::test]
    async fn prompt_workspace_mode_and_prepare_failures() {
        let _lock = mock_agent_lock();
        let (_home, bridge) = active_bridge("cmd-prompt-ws").await;
        let agent = ensure_mock_agent();
        agent.clear_scripts();

        // Workspace mode with a real workspace id → thread bound to it.
        let workspace_dir = std::env::temp_dir().join(unique("futureos-ws-prompt"));
        std::fs::create_dir_all(&workspace_dir).unwrap();
        let workspace = crate::store::create_workspace(crate::store::CreateWorkspaceInput {
            name: Some("Prompt WS".to_string()),
            path: workspace_dir.to_string_lossy().to_string(),
            description: None,
            create_directory: None,
        })
        .unwrap();
        let reply = bridge
            .call(json!({
                "id": unique("cmd"), "type": "prompt", "message": "workspace hello",
                "mode": "workspace", "workspaceId": workspace.id,
            }))
            .await;
        assert_eq!(reply["success"], json!(true), "got: {reply}");
        let ws_thread = reply["data"]["threadId"].as_str().unwrap().to_string();
        assert_eq!(
            crate::store::list_threads()
                .unwrap()
                .iter()
                .find(|t| t.id == ws_thread)
                .map(|t| t.mode.as_str()),
            Some("workspace")
        );

        // Agent-side session provisioning failure → the just-created orphan
        // thread is removed and the error surfaces.
        agent.script(
            "new_session",
            false,
            json!(null),
            "agent rejected the session",
        );
        let before = crate::store::list_threads().unwrap().len();
        let reply = bridge
            .call(json!({ "id": unique("cmd"), "type": "prompt", "message": "fail provisioning" }))
            .await;
        assert_eq!(reply["success"], json!(false));
        assert!(reply["error"].as_str().unwrap().contains("rejected"));
        assert_eq!(crate::store::list_threads().unwrap().len(), before);

        // Prepare failure after attachments were claimed → the claimed copies
        // roll back (no leaked files under the thread's image dir).
        let reply = bridge
            .call(json!({ "id": unique("cmd"), "type": "upload_init", "name": "note.txt", "kind": "file", "originalSize": 3, "transferSize": 3 }))
            .await;
        assert_eq!(reply["success"], json!(true), "got: {reply}");
        let upload_id = reply["data"]["uploadId"].as_str().unwrap().to_string();
        transfer::write_upload_chunk(&upload_id, 0, b"hey").unwrap();
        transfer::complete_upload(&upload_id).unwrap();
        INJECT_PREPARE_FAILURE.store(true, Ordering::Relaxed);
        let reply = bridge
            .call(json!({
                "id": unique("cmd"), "type": "prompt", "message": "with attachment",
                "attachments": [{ "uploadId": upload_id }],
            }))
            .await;
        assert_eq!(reply["success"], json!(false));
        assert!(reply["error"]
            .as_str()
            .unwrap()
            .contains("injected prepare failure"));
        let thread = crate::store::list_threads()
            .unwrap()
            .into_iter()
            .find(|t| t.title == "with attachment")
            .expect("thread for the failed prepare");
        let origin = crate::store::thread_images_dir(&thread.id)
            .unwrap()
            .join("origin");
        let leaked = origin
            .read_dir()
            .map(|mut entries| entries.next().is_some())
            .unwrap_or(false);
        assert!(!leaked, "claimed attachment copies must roll back");

        std::fs::remove_dir_all(&workspace_dir).ok();
        bridge.stop().await;
    }

    #[tokio::test]
    async fn desktop_settings_management_is_shared_and_allowlisted() {
        let _lock = mock_agent_lock();
        let (_home, bridge) = active_bridge("cmd-desktop-settings").await;
        let agent = ensure_mock_agent();
        agent.clear_scripts();
        let defaults = bridge.call(json!({ "type": "get_desktop_settings" })).await;
        assert_eq!(defaults["success"], true);
        assert_eq!(defaults["data"]["autoTitleFirstTurn"], true);
        // The phone renders this snapshot, so a writable field missing from the
        // reply reads as absent and snaps its toggle back to the default — the
        // recommendation switch could not be turned off that way.
        assert_eq!(defaults["data"]["skillRecommend"], true);
        assert!(defaults["data"].get("communityEdition").is_none());

        // Deliberately every writable field: `assert_eq!(updated["data"], patch)`
        // then fails if the reply drops one, which is the bug class this guards.
        let patch = json!({ "autoTitleFirstTurn": false, "autoUpgradeSkills": false,
            "autoConnectRemote": true, "skillRecommend": false,
            "hiddenModels": ["p1/shared"] });
        let updated = bridge
            .call(json!({ "type": "update_desktop_settings", "settings": patch }))
            .await;
        assert_eq!(updated["success"], true, "{updated}");
        assert_eq!(updated["data"], patch);
        let stored = crate::store::get_app_settings().unwrap();
        assert!(!stored.auto_title_first_turn);
        assert!(!stored.auto_upgrade_skills);
        assert!(stored.auto_connect_remote);
        assert!(!stored.skill_recommend);
        assert_eq!(stored.hidden_models, ["p1/shared"]);

        for invalid in [
            json!({"communityEdition": true}),
            json!({"autoTitleFirstTurn": "true"}),
            json!({"skillRecommend": "true"}),
            json!({"approvalTier": "off"}),
            json!({"hiddenModels": [""]}),
            Value::Null,
        ] {
            let reply = bridge
                .call(json!({ "type": "update_desktop_settings", "settings": invalid }))
                .await;
            assert_eq!(reply["success"], false, "{reply}");
            assert!(
                !crate::store::get_app_settings()
                    .unwrap()
                    .auto_title_first_turn
            );
        }
        // Desktop-originated changes are read directly, with no mobile copy.
        crate::store::update_app_settings(crate::store::UpdateAppSettingsInput {
            auto_title_first_turn: Some(true),
            ..Default::default()
        })
        .unwrap();
        let reply = bridge.call(json!({ "type": "get_desktop_settings" })).await;
        assert_eq!(reply["data"]["autoTitleFirstTurn"], true);
        assert_eq!(reply["data"]["hiddenModels"], json!(["p1/shared"]));

        // Management must include hidden entries, otherwise hiding all models
        // would leave the phone unable to enable them again.
        let models =
            json!([{ "id": "shared", "provider": "p1" }, { "id": "shared", "provider": "p2" }]);
        agent.script("list_models", true, json!({ "models": models }), "");
        let reply = bridge.call(json!({ "type": "list_settings_models" })).await;
        assert_eq!(reply["data"]["models"], models);
        bridge.stop().await;
    }

    #[tokio::test]
    async fn skill_management_forwards_agent_results_without_mutating_desktop_files() {
        let _lock = mock_agent_lock();
        let (_home, bridge) = active_bridge("cmd-skill-management").await;
        let agent = ensure_mock_agent();
        agent.clear_scripts();
        agent.script(
            "install_skill",
            false,
            json!(null),
            "invalid skill id or version",
        );
        agent.script("uninstall_skill", false, json!(null), "invalid skill id");
        for kind in ["install_skill", "uninstall_skill"] {
            let reply = bridge
                .call(json!({ "type": kind, "skillId": "../escape", "version": "1.0" }))
                .await;
            assert_eq!(reply["success"], false, "{reply}");
        }
        agent.script("install_skill", false, json!(null), "invalid skill version");
        let reply = bridge
            .call(json!({ "type": "install_skill", "skillId": "acme", "version": "../escape" }))
            .await;
        assert_eq!(reply["success"], false);
        agent.script("install_skill", true, json!({}), "");
        let reply = bridge
            .call(json!({ "type": "install_skill", "skillId": "acme", "version": "1.0.0" }))
            .await;
        assert_eq!(reply["success"], true);
        agent.script("uninstall_skill", true, json!({"removed": true}), "");
        let reply = bridge
            .call(json!({ "type": "uninstall_skill", "skillId": "acme" }))
            .await;
        assert_eq!(reply["success"], true, "{reply}");
        assert_eq!(reply["data"]["removed"], true);
        assert!(agent.served("install_skill", ""));
        assert!(agent.served("uninstall_skill", ""));
        assert!(!crate::auth_store::agent_dir()
            .unwrap()
            .join("skills/acme")
            .exists());
        agent.script("uninstall_skill", true, json!({"removed": false}), "");
        let reply = bridge
            .call(json!({ "type": "uninstall_skill", "skillId": "acme" }))
            .await;
        assert_eq!(reply["data"]["removed"], false);
        bridge.stop().await;
    }

    #[tokio::test]
    async fn provider_management_reuses_desktop_validation_and_agent_writes() {
        let _lock = mock_agent_lock();
        let (_home, bridge) = active_bridge("cmd-provider-management").await;
        let agent = ensure_mock_agent();
        agent.clear_scripts();
        // The mock agent's request log is process-wide, so a test running after
        // another agent-writing test must compare counts, not presence.
        let writes = |command: &str| {
            agent
                .requests()
                .iter()
                .filter(|(served, _)| served == command)
                .count()
        };
        let (set_auth_before, upsert_before) = (writes("set_auth"), writes("upsert_provider"));

        // The phone reads the same snapshot the Settings dialog does, with no
        // API key material in it.
        let reply = bridge.call(json!({ "type": "list_providers" })).await;
        assert_eq!(reply["success"], true, "{reply}");
        assert_eq!(reply["data"]["builtin"][0]["id"], "future");
        assert_eq!(reply["data"]["builtin"][0]["hasApiKey"], false);
        assert!(reply["data"]["custom"].is_array());
        assert!(!serde_json::to_string(&reply["data"])
            .unwrap()
            .to_lowercase()
            .contains("\"key\""));

        // The account provider's credential is not writable from a phone.
        let reply = bridge
            .call(json!({ "type": "update_builtin_provider", "provider": { "id": "future", "apiKey": "sk-x", "updateApiKey": true } }))
            .await;
        assert_eq!(reply["success"], false, "{reply}");
        assert_eq!(writes("set_auth"), set_auth_before);

        // An unknown built-in is rejected before the agent is called.
        let reply = bridge
            .call(json!({ "type": "update_builtin_provider", "provider": { "id": "nope", "baseUrl": "https://x.example.com" } }))
            .await;
        assert_eq!(reply["success"], false, "{reply}");
        let reply = bridge
            .call(json!({ "type": "update_builtin_provider", "provider": { "id": "azure-openai-responses", "baseUrl": "not a url" } }))
            .await;
        assert_eq!(reply["success"], false, "{reply}");
        // Neither an empty provider id nor a payload that is not an object may
        // reach the agent.
        for invalid in [json!({}), json!([]), Value::Null] {
            let reply = bridge
                .call(json!({ "type": "update_builtin_provider", "provider": invalid }))
                .await;
            assert_eq!(reply["success"], false, "{reply}");
        }
        assert_eq!(writes("upsert_provider"), upsert_before);

        // An accepted built-in write reaches the agent as its own config write,
        // with the key and Base URL applied in one atomic upsert.
        let reply = bridge
            .call(json!({
                "type": "update_builtin_provider",
                "provider": { "id": "deepseek", "apiKey": "sk-live", "updateApiKey": true }
            }))
            .await;
        assert_eq!(reply["success"], true, "{reply}");
        assert_eq!(writes("upsert_provider"), upsert_before + 1);

        // A malformed custom provider payload never reaches the agent…
        for invalid in [
            json!({ "id": "acme", "name": "Acme", "api": "openai-completions", "baseUrl": "", "create": true }),
            json!({ "id": "Acme!", "baseUrl": "https://api.example.com", "create": true }),
            json!({ "id": "acme", "baseUrl": "https://api.example.com", "models": [{ "id": "m", "contextWindow": 10, "maxTokens": 99 }] }),
            json!({ "id": "acme", "baseUrl": "https://api.example.com", "models": [{ "id": "m", "inputCost": -1, "contextWindow": 100, "maxTokens": 10 }] }),
            json!([]),
        ] {
            let reply = bridge
                .call(json!({ "type": "upsert_custom_provider", "provider": invalid }))
                .await;
            assert_eq!(reply["success"], false, "{reply}");
        }
        assert_eq!(writes("upsert_provider"), upsert_before + 1);

        // …while a valid one is applied through the shared upsert path.
        let upserts = writes("upsert_provider");
        let reply = bridge
            .call(json!({
                "type": "upsert_custom_provider",
                "provider": {
                    "id": "acme",
                    "name": "Acme",
                    "api": "openai-completions",
                    "baseUrl": "https://api.example.com/v1",
                    "apiKey": "sk-acme",
                    "create": true,
                    "models": [{
                        "id": "acme-large",
                        "name": "Acme Large",
                        "supportsImages": true,
                        "reasoning": true,
                        "contextWindow": 128000,
                        "maxTokens": 16384,
                        "inputCost": 1.5,
                        "outputCost": 6,
                        "cacheReadCost": 0.15,
                        "cacheWriteCost": 0
                    }]
                }
            }))
            .await;
        assert_eq!(reply["success"], true, "{reply}");
        assert_eq!(writes("upsert_provider"), upserts + 1);

        // Built-in providers stay undeletable from a phone, while a custom one
        // is removed through the shared delete path.
        let deletes = writes("delete_provider");
        let reply = bridge
            .call(json!({ "type": "delete_custom_provider", "providerId": "deepseek" }))
            .await;
        assert_eq!(reply["success"], false, "{reply}");
        let reply = bridge
            .call(json!({ "type": "delete_custom_provider", "providerId": "acme" }))
            .await;
        assert_eq!(reply["success"], true, "{reply}");
        assert_eq!(writes("delete_provider"), deletes + 1);
        bridge.stop().await;
    }

    #[tokio::test]
    async fn model_catalog_respects_desktop_visibility() {
        let _agent_lock = mock_agent_lock();
        let (_home, bridge) = active_bridge("cmd-model-visibility").await;
        let agent = ensure_mock_agent();
        agent.clear_scripts();
        let models = json!([
            { "id": "shared", "provider": "p1" },
            { "id": "shared", "provider": "p2" },
            { "id": "org/model", "provider": "p1" },
            { "id": "bare" }
        ]);
        for cmd_type in ["list_models", "get_available_models"] {
            crate::store::update_app_settings(crate::store::UpdateAppSettingsInput {
                hidden_models: Some(vec![
                    "p1/shared".into(),
                    "p1/org/model".into(),
                    "bare".into(),
                ]),
                ..Default::default()
            })
            .unwrap();
            agent.script("list_models", true, json!({ "models": models }), "");
            let reply = bridge
                .call(json!({ "id": unique("cmd"), "type": cmd_type }))
                .await;
            assert_eq!(reply["success"], true, "{reply}");
            assert_eq!(reply["data"]["models"].as_array().unwrap().len(), 1);
            assert_eq!(reply["data"]["models"][0]["id"], "shared");
            assert_eq!(reply["data"]["models"][0]["provider"], "p2");
            assert_eq!(reply["data"]["allModelsHidden"], false);

            crate::store::update_app_settings(crate::store::UpdateAppSettingsInput {
                hidden_models: Some(vec![
                    "p1/shared".into(),
                    "p2/shared".into(),
                    "p1/org/model".into(),
                    "bare".into(),
                ]),
                ..Default::default()
            })
            .unwrap();
            agent.script("list_models", true, json!({ "models": models }), "");
            let reply = bridge
                .call(json!({ "id": unique("cmd"), "type": cmd_type }))
                .await;
            assert_eq!(reply["data"]["models"], json!([]));
            assert_eq!(reply["data"]["allModelsHidden"], true);

            // Empty Agent catalogues still mean warm-up, even with hidden IDs.
            agent.script("list_models", true, json!({ "models": [] }), "");
            let reply = bridge
                .call(json!({ "id": unique("cmd"), "type": cmd_type }))
                .await;
            assert_eq!(reply["data"]["allModelsHidden"], false);

            crate::store::update_app_settings(crate::store::UpdateAppSettingsInput {
                hidden_models: Some(vec![]),
                ..Default::default()
            })
            .unwrap();
            agent.script("list_models", true, json!({ "models": models }), "");
            let reply = bridge
                .call(json!({ "id": unique("cmd"), "type": cmd_type }))
                .await;
            let visible = reply["data"]["models"].as_array().unwrap();
            assert_eq!(visible.len(), 4);
            assert_eq!(visible[2]["id"], "org/model");
            assert_eq!(visible[3]["id"], "bare");
            assert_eq!(reply["data"]["allModelsHidden"], false);
        }
        bridge.stop().await;
    }

    #[tokio::test]
    async fn session_control_commands() {
        let _lock = mock_agent_lock();
        let (_home, bridge) = active_bridge("cmd-session-ctl").await;
        let agent = ensure_mock_agent();
        agent.clear_scripts();
        let session = unique("sess");

        // abort: success and agent failure.
        let reply = bridge
            .call(json!({ "id": unique("cmd"), "type": "abort", "sessionId": session }))
            .await;
        assert_eq!(reply["success"], json!(true), "got: {reply}");
        agent.script_for("abort", &session, false, json!(null), "cannot abort");
        let reply = bridge
            .call(json!({ "id": unique("cmd"), "type": "abort", "sessionId": session }))
            .await;
        assert_eq!(reply["success"], json!(false));

        // Manual compaction requested from the phone: the Desktop forwards it
        // to the same session-scoped RPC and relays the acceptance, while an
        // Agent rejection (busy run, unknown session) stays a failure.
        agent.script_for(
            "compact",
            &session,
            true,
            json!({ "accepted": true, "operationId": "cmp_remote" }),
            "",
        );
        let reply = bridge
            .call(json!({ "id": unique("cmd"), "type": "compact_context", "sessionId": session }))
            .await;
        assert_eq!(reply["success"], json!(true), "got: {reply}");
        assert_eq!(reply["data"]["operationId"], json!("cmp_remote"));
        assert!(agent.served("compact", &session));
        agent.script_for(
            "compact",
            &session,
            false,
            json!(null),
            "finish or stop the active run before manual compaction",
        );
        let reply = bridge
            .call(json!({ "id": unique("cmd"), "type": "compact_context", "sessionId": session }))
            .await;
        assert_eq!(reply["success"], json!(false));
        assert!(
            reply["error"]
                .as_str()
                .unwrap_or_default()
                .contains("active run"),
            "got: {reply}"
        );
        let reply = bridge
            .call(json!({ "id": unique("cmd"), "type": "compact_context", "sessionId": "" }))
            .await;
        assert_eq!(reply["success"], json!(false));

        // get_state.
        let reply = bridge
            .call(json!({ "id": unique("cmd"), "type": "get_state", "sessionId": session }))
            .await;
        assert_eq!(reply["success"], json!(true));
        agent.script_for("get_state", &session, false, json!(null), "gone");
        let reply = bridge
            .call(json!({ "id": unique("cmd"), "type": "get_state", "sessionId": session }))
            .await;
        assert_eq!(reply["success"], json!(false));

        // list_models / get_available_models share a handler.
        for cmd_type in ["list_models", "get_available_models"] {
            let reply = bridge
                .call(json!({ "id": unique("cmd"), "type": cmd_type }))
                .await;
            assert_eq!(reply["success"], json!(true), "{cmd_type}: {reply}");
        }
        agent.script("list_models", false, json!(null), "no models");
        let reply = bridge
            .call(json!({ "id": unique("cmd"), "type": "list_models" }))
            .await;
        assert_eq!(reply["success"], json!(false));

        // Read the Agent's installed skills, not a mobile-local catalogue.
        agent.script(
            "list_installed_skills",
            true,
            json!([
                { "id": "research", "name": "research", "description": "Research",
                  "nameZh": "研究", "descriptionZh": null, "version": "1.0.0" }
            ]),
            "",
        );
        let reply = bridge
            .call(json!({ "id": unique("cmd"), "type": "list_skills" }))
            .await;
        assert_eq!(reply["success"], json!(true));
        assert_eq!(reply["data"]["skills"].as_array().unwrap().len(), 1);
        assert_eq!(reply["data"]["skills"][0]["name"], "research");
        assert_eq!(reply["data"]["skills"][0]["nameZh"], "研究");
        agent.script(
            "list_installed_skills",
            false,
            json!(null),
            "skills unavailable",
        );
        let reply = bridge
            .call(json!({ "id": unique("cmd"), "type": "list_skills" }))
            .await;
        assert_eq!(reply["success"], json!(false));

        // set_model / set_thinking_level / set_session_name (success + failure).
        let reply = bridge
            .call(json!({ "id": unique("cmd"), "type": "set_model", "sessionId": session, "modelId": "m1", "providerId": "p1" }))
            .await;
        assert_eq!(reply["success"], json!(true));
        assert!(agent.served("set_model", &session));
        agent.script_for("set_model", &session, false, json!(null), "bad model");
        let reply = bridge
            .call(json!({ "id": unique("cmd"), "type": "set_model", "sessionId": session, "modelId": "m1" }))
            .await;
        assert_eq!(reply["success"], json!(false));

        let reply = bridge
            .call(json!({ "id": unique("cmd"), "type": "set_thinking_level", "sessionId": session, "level": "high" }))
            .await;
        assert_eq!(reply["success"], json!(true));
        agent.script_for(
            "set_thinking_level",
            &session,
            false,
            json!(null),
            "bad level",
        );
        let reply = bridge
            .call(json!({ "id": unique("cmd"), "type": "set_thinking_level", "sessionId": session, "level": "high" }))
            .await;
        assert_eq!(reply["success"], json!(false));

        let reply = bridge
            .call(json!({ "id": unique("cmd"), "type": "set_session_name", "sessionId": session, "name": "Renamed" }))
            .await;
        assert_eq!(reply["success"], json!(true));
        agent.script_for(
            "set_session_name",
            &session,
            false,
            json!(null),
            "no rename",
        );
        let reply = bridge
            .call(json!({ "id": unique("cmd"), "type": "set_session_name", "sessionId": session, "name": "Renamed" }))
            .await;
        assert_eq!(reply["success"], json!(false));

        // A rename that resolves to a GUI thread emits remote activity
        // (the Ok(Some) find_thread_by_agent_session branch).
        let thread = crate::store::create_thread(crate::store::CreateThreadInput {
            mode: "chat".to_string(),
            title: Some("Phone".to_string()),
            workspace_id: None,
            workspace_path: None,
            workspace_name: None,
            agent_session_id: Some(session.clone()),
        })
        .unwrap();
        let reply = bridge
            .call(json!({ "id": unique("cmd"), "type": "set_session_name", "sessionId": session, "name": "Renamed" }))
            .await;
        assert_eq!(reply["success"], json!(true));

        // set_session_pinned: success and unknown-thread failure.
        let reply = bridge
            .call(json!({ "id": unique("cmd"), "type": "set_session_pinned", "threadId": thread.id, "pinned": true }))
            .await;
        assert_eq!(reply["success"], json!(true), "got: {reply}");
        let reply = bridge
            .call(json!({ "id": unique("cmd"), "type": "set_session_pinned", "threadId": "missing-thread", "pinned": false }))
            .await;
        assert_eq!(reply["success"], json!(false));

        // set_workspace_pinned: missing id, success, and unknown-workspace failure.
        let reply = bridge
            .call(json!({ "id": unique("cmd"), "type": "set_workspace_pinned", "pinned": true }))
            .await;
        assert_eq!(reply["success"], json!(false));
        assert!(reply["error"]
            .as_str()
            .unwrap()
            .contains("missing workspace_id"));
        let reply = bridge
            .call(json!({ "id": unique("cmd"), "type": "set_workspace_pinned", "workspaceId": thread.workspace_id, "pinned": true }))
            .await;
        assert_eq!(reply["success"], json!(true), "got: {reply}");
        assert!(
            crate::store::get_workspace(&thread.workspace_id)
                .unwrap()
                .unwrap()
                .pinned
        );
        let reply = bridge
            .call(json!({ "id": unique("cmd"), "type": "set_workspace_pinned", "workspaceId": "missing-workspace", "pinned": true }))
            .await;
        assert_eq!(reply["success"], json!(false));

        // delete_session: missing id, success, and idempotent success for a
        // thread that is already gone (a child of a parent this phone deleted,
        // or a row removed in the meantime) — the phone walks a selection one
        // session at a time, so "already gone" must not read as a failure.
        let reply = bridge
            .call(json!({ "id": unique("cmd"), "type": "delete_session" }))
            .await;
        assert_eq!(reply["success"], json!(false));
        assert!(reply["error"]
            .as_str()
            .unwrap()
            .contains("missing thread_id"));
        let reply = bridge
            .call(json!({ "id": unique("cmd"), "type": "delete_session", "threadId": thread.id }))
            .await;
        assert_eq!(reply["success"], json!(true));
        let reply = bridge
            .call(json!({ "id": unique("cmd"), "type": "delete_session", "threadId": thread.id }))
            .await;
        assert_eq!(reply["success"], json!(true), "repeat delete is idempotent");
        let reply = bridge
            .call(json!({ "id": unique("cmd"), "type": "delete_session", "threadId": "missing-thread" }))
            .await;
        assert_eq!(
            reply["success"],
            json!(true),
            "already gone is not a failure"
        );

        // Unknown command type.
        let reply = bridge
            .call(json!({ "id": unique("cmd"), "type": "teleport", "sessionId": session }))
            .await;
        assert_eq!(reply["success"], json!(false));
        assert!(reply["error"]
            .as_str()
            .unwrap()
            .contains("Unsupported command"));

        bridge.stop().await;
    }

    #[tokio::test]
    async fn delete_workspace_command_cascades_and_validates() {
        let _lock = mock_agent_lock();
        let (_home, bridge) = active_bridge("cmd-delete-ws").await;

        // A missing id is a malformed request, not a deletion.
        let reply = bridge
            .call(json!({ "id": unique("cmd"), "type": "delete_workspace" }))
            .await;
        assert_eq!(reply["success"], json!(false));
        assert!(reply["error"]
            .as_str()
            .unwrap()
            .contains("missing workspace_id"));

        // A workspace and the threads inside it go together; the user's own
        // directory on disk is never touched.
        let workspace_dir = std::env::temp_dir().join(unique("futureos-ws-delete"));
        std::fs::create_dir_all(&workspace_dir).unwrap();
        let workspace = crate::store::create_workspace(crate::store::CreateWorkspaceInput {
            name: Some("Phone Delete WS".to_string()),
            path: workspace_dir.to_string_lossy().to_string(),
            description: None,
            create_directory: None,
        })
        .unwrap();
        crate::store::create_thread(crate::store::CreateThreadInput {
            mode: "workspace".to_string(),
            title: Some("In workspace".to_string()),
            workspace_id: Some(workspace.id.clone()),
            workspace_path: None,
            workspace_name: None,
            agent_session_id: None,
        })
        .unwrap();
        assert_eq!(crate::store::list_workspaces().unwrap().len(), 1);
        assert_eq!(crate::store::list_threads().unwrap().len(), 1);

        let pinned = bridge
            .call(json!({ "id": unique("cmd"), "type": "set_workspace_pinned",
            "workspaceId": workspace.id, "pinned": true }))
            .await;
        assert_eq!(pinned["success"], true);
        assert_eq!(pinned["data"]["workspaces"][0]["pinned"], true);
        let pulled = bridge
            .call(json!({ "id": unique("cmd"), "type": "list_workspaces" }))
            .await;
        assert_eq!(
            pinned["data"], pulled["data"],
            "ack and pull share the versioned source"
        );
        let unpinned = bridge
            .call(json!({ "id": unique("cmd"), "type": "set_workspace_pinned",
            "workspaceId": workspace.id, "pinned": false }))
            .await;
        assert_eq!(unpinned["data"]["workspaces"][0]["pinned"], false);
        assert!(
            unpinned["data"]["version"]["revision"].as_u64().unwrap()
                > pinned["data"]["version"]["revision"].as_u64().unwrap()
        );

        let reply = bridge
            .call(json!({ "id": unique("cmd"), "type": "delete_workspace", "workspaceId": workspace.id.clone() }))
            .await;
        assert_eq!(reply["success"], json!(true), "got: {reply}");
        assert!(crate::store::list_workspaces().unwrap().is_empty());
        assert!(crate::store::list_threads().unwrap().is_empty());
        assert!(
            workspace_dir.exists(),
            "user files survive a workspace delete"
        );

        // The workspace is gone, so a repeat delete is an error.
        let reply = bridge
            .call(json!({ "id": unique("cmd"), "type": "delete_workspace", "workspaceId": workspace.id }))
            .await;
        assert_eq!(reply["success"], json!(false));

        bridge.stop().await;
    }

    #[tokio::test]
    async fn settings_commands_read_and_update() {
        let _lock = mock_agent_lock();
        let (_home, bridge) = active_bridge("cmd-settings").await;

        let reply = bridge
            .call(json!({ "id": unique("cmd"), "type": "get_settings" }))
            .await;
        assert_eq!(reply["success"], json!(true), "got: {reply}");
        assert_eq!(reply["data"]["approvalTier"], json!("off"));
        assert_eq!(
            reply["data"]["sandboxAvailable"],
            json!(cfg!(target_os = "macos"))
        );

        let reply = bridge
            .call(json!({ "id": unique("cmd"), "type": "set_approval_tier", "tier": "sandbox" }))
            .await;
        assert_eq!(reply["success"], json!(true), "got: {reply}");
        #[cfg(target_os = "macos")]
        assert_eq!(reply["data"]["approvalTier"], json!("sandbox"));
        #[cfg(not(target_os = "macos"))]
        assert_eq!(reply["data"]["approvalTier"], json!("manual"));

        // A non-sandbox tier takes the else branch (no host probe at all).
        let reply = bridge
            .call(json!({ "id": unique("cmd"), "type": "set_approval_tier", "tier": "manual" }))
            .await;
        assert_eq!(reply["success"], json!(true), "got: {reply}");
        assert_eq!(reply["data"]["approvalTier"], json!("manual"));

        bridge.stop().await;
    }

    #[tokio::test]
    async fn continue_run_does_not_ack_before_agent_acceptance() {
        let _lock = mock_agent_lock();
        let (_home, bridge) = active_bridge("cmd-continue").await;
        let agent = ensure_mock_agent();
        let session = unique("sess");

        // A thread bound to the session + a failed run with terminal events.
        let thread = crate::store::create_thread(crate::store::CreateThreadInput {
            mode: "chat".to_string(),
            title: Some("Continue".to_string()),
            workspace_id: None,
            workspace_path: None,
            workspace_name: None,
            agent_session_id: Some(session.clone()),
        })
        .unwrap();
        let run = crate::store::create_run(crate::store::CreateRunInput {
            id: Some("run-failed".to_string()),
            thread_id: thread.id.clone(),
            trigger_message_id: None,
            model_provider: None,
            model_id: None,
        })
        .unwrap();
        crate::store::append_run_event(crate::store::AppendRunEventInput {
            run_id: run.id.clone(),
            event_type: "tool_result".to_string(),
            payload: Some(r#"{"text":"did the thing"}"#.to_string()),
            sequence: 1,
        })
        .unwrap();
        crate::store::flush_run_event_log_for_test(&run.id);
        crate::store::update_run_status_if_active(crate::store::UpdateRunStatusInput {
            run_id: run.id.clone(),
            status: "failed".to_string(),
            error_message: None,
            error_type: None,
        })
        .unwrap();

        // A continuation must reuse its thread-bound session rather than
        // provisioning a replacement. Reject the actual prompt acceptance so
        // the local run can never become a successful mobile receipt.
        agent.script_for("prompt", &session, false, json!(null), "rejected");

        let command_id = unique("cmd");
        let reply = bridge
            .call(json!({ "id": command_id, "type": "continue_run", "sessionId": session, "runId": run.id }))
            .await;
        assert_eq!(reply["success"], json!(false), "got: {reply}");
        assert!(reply["error"].as_str().unwrap().contains("rejected"));
        assert!(remote_prompt_receipt(&command_id).unwrap().is_none());
        assert!(agent.served("prompt", &session));
        assert!(
            !agent.served("new_session", &session),
            "continuation must preserve the bound session"
        );

        bridge.stop().await;
    }

    #[tokio::test]
    async fn continue_run_rejects_a_busy_session() {
        let _lock = mock_agent_lock();
        let (_home, bridge) = active_bridge("cmd-continue-busy").await;
        let session = unique("sess");

        let thread = crate::store::create_thread(crate::store::CreateThreadInput {
            mode: "chat".to_string(),
            title: Some("Busy".to_string()),
            workspace_id: None,
            workspace_path: None,
            workspace_name: None,
            agent_session_id: Some(session.clone()),
        })
        .unwrap();
        let source = crate::store::create_run(crate::store::CreateRunInput {
            id: Some("run-failed-source".to_string()),
            thread_id: thread.id.clone(),
            trigger_message_id: None,
            model_provider: None,
            model_id: None,
        })
        .unwrap();
        crate::store::update_run_status_if_active(crate::store::UpdateRunStatusInput {
            run_id: source.id.clone(),
            status: "failed".to_string(),
            error_message: Some("boom".to_string()),
            error_type: Some("model_failed".to_string()),
        })
        .unwrap();
        // A separate active run on the session → the busy guard fires before
        // anything is persisted or spawned.
        crate::store::create_run(crate::store::CreateRunInput {
            id: Some("run-active".to_string()),
            thread_id: thread.id.clone(),
            trigger_message_id: None,
            model_provider: None,
            model_id: None,
        })
        .unwrap();

        let reply = bridge
            .call(json!({ "id": unique("cmd"), "type": "continue_run", "sessionId": session, "runId": source.id }))
            .await;
        assert_eq!(reply["success"], json!(false));
        assert!(reply["error"].as_str().unwrap().contains("still running"));

        bridge.stop().await;
    }

    #[tokio::test]
    async fn continue_run_rejects_missing_wrong_session_and_nonfailed_sources() {
        let _lock = mock_agent_lock();
        let (_home, bridge) = active_bridge("cmd-continue-source-guards").await;
        let session = unique("sess");
        let thread = crate::store::create_thread(crate::store::CreateThreadInput {
            mode: "chat".to_string(),
            title: Some("Guarded continuation".to_string()),
            workspace_id: None,
            workspace_path: None,
            workspace_name: None,
            agent_session_id: Some(session.clone()),
        })
        .unwrap();
        let completed = crate::store::create_run(crate::store::CreateRunInput {
            id: Some(unique("run-completed")),
            thread_id: thread.id.clone(),
            trigger_message_id: None,
            model_provider: None,
            model_id: None,
        })
        .unwrap();
        crate::store::update_run_status_if_active(crate::store::UpdateRunStatusInput {
            run_id: completed.id.clone(),
            status: "completed".to_string(),
            error_message: None,
            error_type: None,
        })
        .unwrap();

        let missing = bridge
            .call(json!({ "id": unique("cmd"), "type": "continue_run", "sessionId": session, "runId": "missing-run" }))
            .await;
        assert_eq!(missing["success"], json!(false));
        assert!(missing["error"]
            .as_str()
            .unwrap()
            .contains("no longer exists"));

        let nonfailed = bridge
            .call(json!({ "id": unique("cmd"), "type": "continue_run", "sessionId": session, "runId": completed.id }))
            .await;
        assert_eq!(nonfailed["success"], json!(false));
        assert!(nonfailed["error"].as_str().unwrap().contains("failed run"));

        let failed = crate::store::create_run(crate::store::CreateRunInput {
            id: Some(unique("run-failed")),
            thread_id: thread.id,
            trigger_message_id: None,
            model_provider: None,
            model_id: None,
        })
        .unwrap();
        crate::store::update_run_status_if_active(crate::store::UpdateRunStatusInput {
            run_id: failed.id.clone(),
            status: "failed".to_string(),
            error_message: Some("boom".to_string()),
            error_type: Some("model_failed".to_string()),
        })
        .unwrap();
        let wrong_session = bridge
            .call(json!({ "id": unique("cmd"), "type": "continue_run", "sessionId": "another-session", "runId": failed.id }))
            .await;
        assert_eq!(wrong_session["success"], json!(false));
        assert!(wrong_session["error"]
            .as_str()
            .unwrap()
            .contains("does not belong"));

        bridge.stop().await;
    }

    #[tokio::test]
    async fn build_continue_prompt_folds_recent_terminal_events() {
        let _lock = mock_agent_lock();
        let _home = HomeGuard::new("cmd-continue-prompt");
        init_store();
        let thread = crate::store::create_thread(crate::store::CreateThreadInput {
            mode: "chat".to_string(),
            title: Some("Prompt".to_string()),
            workspace_id: None,
            workspace_path: None,
            workspace_name: None,
            agent_session_id: None,
        })
        .unwrap();
        let run = crate::store::create_run(crate::store::CreateRunInput {
            id: None,
            thread_id: thread.id.clone(),
            trigger_message_id: None,
            model_provider: None,
            model_id: None,
        })
        .unwrap();
        // A non-terminal event is ignored; terminal events are folded newest-first.
        crate::store::append_run_event(crate::store::AppendRunEventInput {
            run_id: run.id.clone(),
            event_type: "text_chunk".to_string(),
            payload: Some("ignored".to_string()),
            sequence: 1,
        })
        .unwrap();
        crate::store::append_run_event(crate::store::AppendRunEventInput {
            run_id: run.id.clone(),
            event_type: "tool_result".to_string(),
            payload: Some("tool output".to_string()),
            sequence: 2,
        })
        .unwrap();
        crate::store::append_run_event(crate::store::AppendRunEventInput {
            run_id: run.id.clone(),
            event_type: "error".to_string(),
            payload: Some("boom".to_string()),
            sequence: 3,
        })
        .unwrap();
        crate::store::flush_run_event_log_for_test(&run.id);

        let prompt = build_continue_prompt(&run.id);
        assert!(prompt.contains("继续上一个任务。"), "{prompt}");
        assert!(prompt.contains("已执行内容摘要:"), "{prompt}");
        assert!(prompt.contains("error"), "{prompt}");
        assert!(prompt.contains("tool_result"), "{prompt}");
        assert!(
            !prompt.contains("text_chunk"),
            "non-terminal ignored: {prompt}"
        );

        // An unknown run (no events) still yields the default continue prompt.
        let empty = build_continue_prompt("no-such-run");
        assert_eq!(empty, "继续上一个任务。");
    }

    #[tokio::test]
    async fn approval_decision_ownership_and_outcomes() {
        let _lock = mock_agent_lock();
        let (_home, bridge) = active_bridge("cmd-approval").await;
        let agent = ensure_mock_agent();
        agent.clear_scripts();
        let session = unique("sess");

        // Unknown approval request.
        let reply = bridge
            .call(json!({ "id": unique("cmd"), "type": "approval_decision", "sessionId": session, "entryId": "nope", "mode": "approved" }))
            .await;
        assert_eq!(reply["success"], json!(false));
        assert!(reply["error"]
            .as_str()
            .unwrap()
            .contains("could not be loaded"));

        // A real approval owned by a DIFFERENT session → ownership mismatch.
        let thread = crate::store::create_thread(crate::store::CreateThreadInput {
            mode: "chat".to_string(),
            title: None,
            workspace_id: None,
            workspace_path: None,
            workspace_name: None,
            agent_session_id: Some(session.clone()),
        })
        .unwrap();
        let run = crate::store::create_run(crate::store::CreateRunInput {
            id: None,
            thread_id: thread.id.clone(),
            trigger_message_id: None,
            model_provider: None,
            model_id: None,
        })
        .unwrap();
        crate::store::ensure_approval_request(crate::store::EnsureApprovalRequestInput {
            approval_request_id: Some("appr-1".to_string()),
            run_id: run.id.clone(),
            tool_call_id: Some("tool-1".to_string()),
            kind: "command".to_string(),
            title: "Run ls".to_string(),
            summary: None,
            risk_level: None,
            requested_action: None,
            action_category: None,
            action_payload: None,
            sandbox_boundary: None,
            save_suggestion: None,
            reviewer: None,
        })
        .unwrap();

        let reply = bridge
            .call(json!({ "id": unique("cmd"), "type": "approval_decision", "sessionId": "someone-else", "entryId": "appr-1", "mode": "approved" }))
            .await;
        assert_eq!(reply["success"], json!(false));
        assert!(reply["error"].as_str().unwrap().contains("does not belong"));

        // Owning session decides → the agent is notified and the reply is ok.
        let reply = bridge
            .call(json!({ "id": unique("cmd"), "type": "approval_decision", "sessionId": session, "entryId": "appr-1", "mode": "approved" }))
            .await;
        assert_eq!(reply["success"], json!(true), "got: {reply}");
        assert!(agent.served("approval_decision", &session));
        let record = crate::store::get_approval_request("appr-1")
            .unwrap()
            .unwrap();
        assert_eq!(record.status, "approved");

        // An agent-side stale approval cancels locally but still replies ok.
        crate::store::ensure_approval_request(crate::store::EnsureApprovalRequestInput {
            approval_request_id: Some("appr-2".to_string()),
            run_id: run.id.clone(),
            tool_call_id: Some("tool-2".to_string()),
            kind: "command".to_string(),
            title: "Run pwd".to_string(),
            summary: None,
            risk_level: None,
            requested_action: None,
            action_category: None,
            action_payload: None,
            sandbox_boundary: None,
            save_suggestion: None,
            reviewer: None,
        })
        .unwrap();
        agent.script_for(
            "approval_decision",
            &session,
            false,
            json!(null),
            "approval request is not pending",
        );
        let reply = bridge
            .call(json!({ "id": unique("cmd"), "type": "approval_decision", "sessionId": session, "entryId": "appr-2", "mode": "approved" }))
            .await;
        assert_eq!(reply["success"], json!(true), "got: {reply}");
        let record = crate::store::get_approval_request("appr-2")
            .unwrap()
            .unwrap();
        assert_eq!(record.status, "cancelled");

        // A genuine agent rejection surfaces as an error.
        crate::store::ensure_approval_request(crate::store::EnsureApprovalRequestInput {
            approval_request_id: Some("appr-3".to_string()),
            run_id: run.id.clone(),
            tool_call_id: Some("tool-3".to_string()),
            kind: "command".to_string(),
            title: "Run rm".to_string(),
            summary: None,
            risk_level: None,
            requested_action: None,
            action_category: None,
            action_payload: None,
            sandbox_boundary: None,
            save_suggestion: None,
            reviewer: None,
        })
        .unwrap();
        agent.script_for("approval_decision", &session, false, json!(null), "boom");
        let reply = bridge
            .call(json!({ "id": unique("cmd"), "type": "approval_decision", "sessionId": session, "entryId": "appr-3", "mode": "denied" }))
            .await;
        assert_eq!(reply["success"], json!(false));

        bridge.stop().await;
    }

    #[tokio::test]
    async fn duplicate_command_ids_get_one_execution_and_cached_replies() {
        let _lock = mock_agent_lock();
        let (_home, bridge) = active_bridge("cmd-singleflight").await;
        let agent = ensure_mock_agent();
        agent.clear_scripts();
        let session = unique("sess");

        let command_id = unique("cmdid");
        let cmd = || json!({ "id": command_id, "type": "get_state", "sessionId": session });
        // Two concurrent deliveries → a single agent call, identical replies.
        let (first, second) = tokio::join!(bridge.call(cmd()), bridge.call(cmd()));
        assert_eq!(first, second);
        let executions = agent
            .requests()
            .iter()
            .filter(|(command, sid)| command == "get_state" && sid == &session)
            .count();
        assert_eq!(executions, 1, "retried command must not re-execute");
        let conflict = bridge
            .call(json!({ "id": command_id, "type": "delete_session", "sessionId": session }))
            .await;
        assert_eq!(conflict["error"], json!("command_id_conflict"));

        // Retirement runs on the Host executor, separately from this test's
        // runtime. Observe the real eviction instead of assuming its task has
        // run after a fixed sleep on a loaded native runner. Never clear the
        // cache or repeat the command until the actual slot is gone.
        //
        // The wait proves the slot *is* retired, and its ceiling is only a
        // liveness guard — "the task exists, runs and removes this entry" —
        // not a latency bound, because the scheduling is not this test's to
        // bound. That executor is the process-wide host runtime every other
        // test in this binary shares, and the command handlers block its
        // workers on store and file work (`crate::store::*` runs inline, not
        // through `spawn_blocking`), so its timer can go unpolled for seconds
        // while the suite is busy. Measured on the Windows runner: this test
        // took 5.32 s and spent all of it in a 5 s ceiling over a 500 ms TTL,
        // with 1700 sibling tests passing before, during and after it — a
        // frozen task registry (whose spawns are silently dropped) is ruled
        // out by that, since it is process-wide and sticky. A minute keeps
        // >10x the stall actually observed, and a real regression still fails
        // it far inside the job's budget.
        tokio::time::timeout(Duration::from_secs(60), async {
            while bridge.reply_slots.contains_command(&command_id) {
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
        })
        .await
        .expect("the configured reply-slot TTL must retire the real cache entry");
        let third = bridge.call(cmd()).await;
        assert_eq!(third["success"], json!(true));
        let executions = agent
            .requests()
            .iter()
            .filter(|(command, sid)| command == "get_state" && sid == &session)
            .count();
        assert_eq!(executions, 2, "expired cache entries re-execute");

        // A cached reply replayed to a delivery WITHOUT a reply subject is
        // simply dropped (no publish, no panic).
        bridge.nats.inject(
            &format!("p.{}.cmd.rpc", bridge.pair_id),
            None,
            serde_json::to_vec(&cmd()).unwrap(),
        );
        tokio::time::sleep(Duration::from_millis(150)).await;
        bridge.stop().await;
    }

    #[tokio::test]
    async fn command_loop_exits_for_generation_supervisor_when_stream_ends() {
        let _home = HomeGuard::new("cmd-self-heal");
        let nats = FakeNats::start().await;
        let client = nats_connect_once(&nats).await;
        let creds = bridge_creds();
        let pair_id = creds.pair_id.clone();
        let handshake = new_handshake_state(
            creds,
            Arc::new(AtomicBool::new(true)),
            format!("bridge_{}", unique("cmd")),
        );
        let handle = tokio::spawn(command_loop(
            client,
            pair_id.clone(),
            new_reply_slots(),
            handshake,
        ));
        nats.wait_for_sub(&format!("p.{pair_id}.cmd.>"), Duration::from_secs(5))
            .await;
        // A critical subscription is generation-local. The loop exits so the
        // single runtime supervisor can apply its 10-minute recovery budget.
        nats.kill();
        wait_until(
            "supervisor must observe the dead task",
            Duration::from_secs(5),
            || handle.is_finished(),
        )
        .await;
    }

    /// A pairing id whose queue group NATS rejects makes the loop's critical
    /// subscription impossible. The loop must report that and return — not sit
    /// on a stream that can never receive anything — because returning is what
    /// lets the runtime supervisor spend its recovery budget on a new
    /// generation. The old version of this test only killed the server:
    /// async-nats accepts a subscribe optimistically, so `queue_subscribe` still
    /// succeeded and the branch under test never ran.
    #[tokio::test]
    async fn a_pairing_id_that_cannot_form_a_queue_group_returns_instead_of_waiting() {
        let nats = FakeNats::start().await;
        let client = nats_connect(&nats).await;
        let mut creds = bridge_creds();
        // Whitespace cannot appear in a NATS queue group: the loop's
        // `queue_subscribe` is refused before anything reaches the broker.
        creds.pair_id = "pair with a space".to_string();
        let pair_id = creds.pair_id.clone();
        let handshake = new_handshake_state(
            creds,
            Arc::new(AtomicBool::new(false)),
            format!("bridge_{}", unique("cmd")),
        );
        let mut tap = nats.tap();
        // Returned rather than hung: the await below is the assertion that the
        // failure path terminated the loop.
        command_loop(client, pair_id.clone(), new_reply_slots(), handshake).await;
        // And with no subscription the loop is deaf: a command addressed to it
        // is dropped, never answered.
        nats.inject(
            &format!("p.{pair_id}.cmd.rpc"),
            Some("rep_no_subscription"),
            serde_json::to_vec(&json!({"id": "x", "type": "get_presence"})).unwrap(),
        );
        crate::remote::test_support::assert_no_publish(
            &mut tap,
            "rep_no_subscription",
            Duration::from_millis(200),
        )
        .await;
    }

    #[tokio::test]
    async fn unpair_command_acknowledges_and_spawns_cleanup() {
        let _lock = mock_agent_lock();
        let (_home, bridge) = active_bridge("cmd-unpair").await;
        let reply = bridge
            .call(json!({ "id": unique("cmd"), "type": "unpair" }))
            .await;
        assert_eq!(reply["success"], json!(true));
        // Give the spawned cleanup a moment to run (it is a no-op without a
        // persisted pairing).
        tokio::time::sleep(Duration::from_millis(50)).await;
        bridge.stop().await;
    }

    #[test]
    fn history_keeps_agent_outcome_without_desktop_inference() {
        let entry = json!({"id":"m","kind":"assistant","role":"assistant","runId":"run","createdAtMs":1000,"blocks":[],"run":{"status":"failed","error":"synthetic","durationMs":7}});
        let page = prepare_backward_entries_page(
            "unmirrored",
            json!({"entries":[entry.clone()],"hasMore":false,"nextOffset":0}),
        );
        assert_eq!(page["entries"][0], entry);
    }

    // ── Business handlers reached only through the command loop ─────────────
    //
    // These cover the *phone-visible* half of `remote_host::business`: the
    // settings/skill/provider/history handlers, their error arms, and the
    // transport's paged-read envelope. They go through the real bridge (fake
    // NATS → command loop → real Desktop host) rather than calling the handlers
    // directly, so a regression in the dispatch table fails them too.

    /// `list_settings_models` and `list_available_skills` answer a failure with
    /// `success:false` and the agent's own message. A phone that gets a silent
    /// empty list cannot tell "no skills" from "the desktop cannot ask".
    #[tokio::test]
    async fn skill_and_model_listings_report_an_agent_failure() {
        let _lock = mock_agent_lock();
        let (_home, bridge) = active_bridge("cmd-skill-lists").await;
        let agent = ensure_mock_agent();
        agent.clear_scripts();

        // The catalogue is decoded into typed skills, so the agent answers with
        // a bare array (a `{"skills": [...]}` envelope is rejected) carrying the
        // two fields the contract does not default (`latestVersion`, `builtin`).
        agent.script(
            "list_available_skills",
            true,
            json!([{
                "id": "future-web",
                "name": "future-web",
                "latestVersion": null,
                "builtin": true,
            }]),
            "",
        );
        let reply = bridge
            .call(json!({ "id": unique("cmd"), "type": "list_available_skills" }))
            .await;
        assert_eq!(reply["success"], json!(true));
        assert_eq!(
            reply["data"]["skills"][0]["id"],
            json!("future-web"),
            "got: {reply}"
        );

        agent.script("list_available_skills", false, Value::Null, "agent offline");
        let reply = bridge
            .call(json!({ "id": unique("cmd"), "type": "list_available_skills" }))
            .await;
        assert_eq!(reply["success"], json!(false));
        assert!(
            reply["error"].as_str().unwrap().contains("agent offline"),
            "got: {reply}"
        );

        agent.script("list_models", false, Value::Null, "models down");
        let reply = bridge
            .call(json!({ "id": unique("cmd"), "type": "list_settings_models" }))
            .await;
        assert_eq!(reply["success"], json!(false));
        assert!(
            reply["error"].as_str().unwrap().contains("models down"),
            "got: {reply}"
        );
        bridge.stop().await;
    }

    /// A recommendation is best-effort by contract: a failure is reported as
    /// "no recommendation" with `success:true`, because a phone whose desktop is
    /// mid-upgrade still has to send the user's message. A malformed candidate
    /// list must not change that.
    #[tokio::test]
    async fn a_skill_recommendation_never_blocks_the_message() {
        let _lock = mock_agent_lock();
        let (_home, bridge) = active_bridge("cmd-suggest-skill").await;
        let agent = ensure_mock_agent();
        agent.clear_scripts();

        agent.script(
            "suggest_skill",
            true,
            json!({ "skill": { "name": "future-paper", "description": "papers" } }),
            "",
        );
        let reply = bridge
            .call(json!({
                "id": unique("cmd"),
                "type": "suggest_skill",
                "query": "find me a paper",
                "candidates": [
                    { "name": "future-paper", "description": "papers" },
                    "not an object",
                    { "description": "anonymous" }
                ]
            }))
            .await;
        assert_eq!(reply["success"], json!(true), "got: {reply}");
        assert_eq!(reply["data"]["skill"]["name"], json!("future-paper"));

        // The agent found nothing: a null skill, still a success.
        agent.script("suggest_skill", true, json!({ "skill": Value::Null }), "");
        let reply = bridge
            .call(json!({
                "id": unique("cmd"),
                "type": "suggest_skill",
                "query": "nothing matches",
                "candidates": []
            }))
            .await;
        assert_eq!(reply["success"], json!(true), "got: {reply}");
        assert!(reply["data"]["skill"].is_null());

        // The agent failed: still a success, with the reason carried alongside
        // so the phone can log it without treating the draft as rejected.
        agent.script("suggest_skill", false, Value::Null, "engine busy");
        let reply = bridge
            .call(json!({
                "id": unique("cmd"),
                "type": "suggest_skill",
                "query": "anything",
                "candidates": []
            }))
            .await;
        assert_eq!(reply["success"], json!(true), "got: {reply}");
        assert!(reply["data"]["skill"].is_null());
        assert!(
            reply["error"].as_str().unwrap().contains("engine busy"),
            "got: {reply}"
        );
        bridge.stop().await;
    }

    /// The recommendation ledger is desktop-owned: the phone may record what it
    /// showed (so the desktop can suppress that skill next time), but a record
    /// with no skill id is a client bug and must be refused rather than stored
    /// under an empty key.
    #[tokio::test]
    async fn recording_a_shown_recommendation_validates_and_persists() {
        let _lock = mock_agent_lock();
        let (_home, bridge) = active_bridge("cmd-record-reco").await;

        let reply = bridge
            .call(json!({ "id": unique("cmd"), "type": "skill_reco_today" }))
            .await;
        assert_eq!(reply["success"], json!(true), "got: {reply}");
        assert!(reply["data"].get("today").is_some(), "got: {reply}");

        let reply = bridge
            .call(json!({
                "id": unique("cmd"),
                "type": "record_skill_reco",
                "skillId": "",
                "messageHash": "hash"
            }))
            .await;
        assert_eq!(reply["success"], json!(false));
        assert!(
            reply["error"]
                .as_str()
                .unwrap()
                .contains("skill_id is required"),
            "got: {reply}"
        );

        let reply = bridge
            .call(json!({
                "id": unique("cmd"),
                "type": "record_skill_reco",
                "skillId": "future-web",
                "messageHash": unique("hash")
            }))
            .await;
        assert_eq!(reply["success"], json!(true), "got: {reply}");

        let reply = bridge
            .call(json!({ "id": unique("cmd"), "type": "skill_reco_today" }))
            .await;
        assert_eq!(reply["success"], json!(true));
        let today = reply["data"]["today"].to_string();
        assert!(
            !today.is_empty(),
            "a recorded recommendation is readable back: {reply}"
        );
        bridge.stop().await;
    }

    /// `set_approval_tier` is the one settings write the phone owns. A tier the
    /// desktop cannot honour is downgraded (not silently stored, and not an
    /// error): the phone is told which tier is actually in force. When the
    /// desktop cannot even ask, the write is refused.
    // macOS sandbox availability is fixed; only Windows/Linux consume a probe.
    #[cfg(any(target_os = "windows", target_os = "linux"))]
    #[tokio::test]
    async fn the_approval_tier_reflects_what_the_desktop_can_enforce() {
        let _lock = mock_agent_lock();
        let (_home, bridge) = active_bridge("cmd-approval-tier").await;
        let agent = ensure_mock_agent();
        agent.clear_scripts();

        agent.script(
            "probe_sandbox",
            true,
            json!({ "available": true, "code": "available", "backend": "seatbelt" }),
            "",
        );
        let reply = bridge
            .call(json!({ "id": unique("cmd"), "type": "set_approval_tier", "tier": "sandbox" }))
            .await;
        assert_eq!(reply["success"], json!(true), "got: {reply}");
        assert_eq!(reply["data"]["approvalTier"], json!("sandbox"));

        agent.script(
            "probe_sandbox",
            true,
            json!({ "available": false, "code": "unavailable", "backend": "none" }),
            "",
        );
        let reply = bridge
            .call(json!({ "id": unique("cmd"), "type": "set_approval_tier", "tier": "sandbox" }))
            .await;
        assert_eq!(reply["success"], json!(true), "got: {reply}");
        assert_eq!(
            reply["data"]["approvalTier"],
            json!("manual"),
            "an unenforceable tier is downgraded, not stored: {reply}"
        );

        // `manual` never needs a probe, so it must succeed even when the probe
        // is failing.
        agent.script("probe_sandbox", false, Value::Null, "probe exploded");
        let reply = bridge
            .call(json!({ "id": unique("cmd"), "type": "set_approval_tier", "tier": "manual" }))
            .await;
        assert_eq!(reply["success"], json!(true), "got: {reply}");
        assert_eq!(reply["data"]["approvalTier"], json!("manual"));

        // Asking for a sandbox tier while the probe fails is refused: the phone
        // must not be told a tier is in force that the desktop cannot check.
        let reply = bridge
            .call(json!({ "id": unique("cmd"), "type": "set_approval_tier", "tier": "sandbox" }))
            .await;
        assert_eq!(reply["success"], json!(false));
        assert!(
            reply["error"].as_str().unwrap().contains("probe exploded"),
            "got: {reply}"
        );
        bridge.stop().await;
    }

    #[tokio::test]
    async fn automatic_approval_tier_requires_a_future_account() {
        let _lock = mock_agent_lock();
        let (_home, bridge) = active_bridge("cmd-auto-approval-account").await;
        let agent = ensure_mock_agent();
        for (signed_in, expected) in [(false, "sandbox"), (true, "auto"), (false, "sandbox")] {
            if signed_in {
                crate::auth_store::set_future_login("fixture-key", "https://future.example/api")
                    .unwrap();
            } else {
                crate::auth_store::clear_future_key().unwrap();
            }
            agent.clear_scripts();
            agent.script(
                "probe_sandbox",
                true,
                json!({"available":true,"code":"available","backend":"fixture"}),
                "",
            );
            let response = bridge
                .call(json!({"id":unique("cmd"),"type":"set_approval_tier","tier":"auto"}))
                .await;
            assert_eq!(response["success"], json!(true), "{response}");
            assert_eq!(response["data"]["approvalTier"], json!(expected));
            assert_eq!(
                crate::store::get_app_settings().unwrap().approval_tier,
                expected
            );
        }
        agent.clear_scripts();
        bridge.stop().await;
    }

    /// `get_settings` reports whether a sandbox even exists on this desktop, and
    /// refuses the whole read when the probe fails rather than answering with a
    /// guessed `false` the phone would show as a missing feature.
    #[tokio::test]
    async fn desktop_settings_reports_platform_sandbox_availability() {
        let _lock = mock_agent_lock();
        let (_home, bridge) = active_bridge("cmd-settings-probe").await;
        let agent = ensure_mock_agent();
        agent.clear_scripts();
        agent.clear_requests();

        agent.script(
            "probe_sandbox",
            true,
            json!({ "available": true, "code": "available", "backend": "seatbelt" }),
            "",
        );
        let reply = bridge
            .call(json!({ "id": unique("cmd"), "type": "get_settings" }))
            .await;
        assert_eq!(reply["success"], json!(true), "got: {reply}");
        assert_eq!(reply["data"]["sandboxAvailable"], json!(true));

        agent.script("probe_sandbox", false, Value::Null, "probe exploded");
        let reply = bridge
            .call(json!({ "id": unique("cmd"), "type": "get_settings" }))
            .await;
        if cfg!(target_os = "macos") {
            // macOS has built-in Seatbelt availability; it does not ask the
            // Agent to probe the Windows/Linux optional backend.
            assert_eq!(reply["success"], json!(true), "got: {reply}");
            assert_eq!(reply["data"]["sandboxAvailable"], json!(true));
            assert!(!agent.served("probe_sandbox", ""));
        } else {
            assert_eq!(reply["success"], json!(false));
            assert!(
                reply["error"].as_str().unwrap().contains("probe exploded"),
                "got: {reply}"
            );
        }
        bridge.stop().await;
    }

    /// A phone that never declared the chunked-read capability must keep
    /// receiving one plain reply per command, even for a result larger than the
    /// page threshold: the phone has no reader for a `readChunk` envelope and
    /// would render nothing at all. The reply stays a decoded JSON value (and
    /// under the relay's plaintext budget), so this is also the boundary where
    /// paging and the reply size limit meet.
    #[tokio::test]
    async fn a_client_without_the_capability_is_never_sent_a_page() {
        let _lock = mock_agent_lock();
        let (_home, bridge) = active_bridge("cmd-no-chunked-read").await;
        let agent = ensure_mock_agent();
        agent.clear_scripts();
        agent.script(
            "list_models",
            true,
            json!({ "models": [], "blob": "x".repeat(560 * 1024) }),
            "",
        );
        let reply = bridge
            .call(json!({ "id": unique("cmd"), "type": "list_settings_models" }))
            .await;
        assert_eq!(reply["success"], json!(true), "got: {reply}");
        assert!(
            reply["data"].get("readChunk").is_none(),
            "no page envelope without the declaration: {reply}"
        );
        assert_eq!(
            reply["data"]["blob"].as_str().unwrap().len(),
            560 * 1024,
            "the whole reply reaches a client that cannot page"
        );
        bridge.stop().await;
    }

    /// `generate_session_title` is a brand-new mobile command: it must reach the
    /// agent through the desktop (never the phone's own model), and a failure
    /// must be the agent's message rather than an empty title the phone would
    /// store.
    #[tokio::test]
    async fn generate_session_title_reports_success_and_failure() {
        let _lock = mock_agent_lock();
        let (_home, bridge) = active_bridge("cmd-session-title").await;
        let agent = ensure_mock_agent();
        agent.clear_scripts();
        let session = unique("sess");

        agent.script_for(
            "generate_session_title",
            &session,
            true,
            json!({ "title": "Deploying the bridge" }),
            "",
        );
        let reply = bridge
            .call(json!({
                "id": unique("cmd"), "type": "generate_session_title",
                "sessionId": session, "mode": "chat"
            }))
            .await;
        assert_eq!(reply["success"], json!(true), "got: {reply}");
        assert_eq!(reply["data"]["title"], json!("Deploying the bridge"));

        agent.script_for(
            "generate_session_title",
            &session,
            false,
            Value::Null,
            "no model configured",
        );
        let reply = bridge
            .call(json!({
                "id": unique("cmd"), "type": "generate_session_title",
                "sessionId": session, "mode": "chat"
            }))
            .await;
        assert_eq!(reply["success"], json!(false));
        assert!(
            reply["error"]
                .as_str()
                .unwrap()
                .contains("no model configured"),
            "got: {reply}"
        );
        bridge.stop().await;
    }

    /// The gate the whole feature rests on: a standby device's business command
    /// is refused *here*, in this process, before any handler runs — so the
    /// instant control moves, nothing already in flight can still execute. The
    /// one command it may speak is `takeover`, which is exactly what hands the
    /// machine over.
    #[tokio::test]
    async fn a_standby_device_is_refused_and_a_takeover_moves_control() {
        let _lock = mock_agent_lock();
        let (_home, bridge) = active_bridge("cmd-standby").await;
        // Another *paired* device holds the machine. It has to be real: a
        // pointer naming a device this desktop does not have is stale and is
        // ignored, which is a different rule (and a different test).
        let other = "pair_other_controller".to_string();
        crate::remote::pairing::save_creds(&super::super::pairing::PairingCreds {
            handshake_version: 2,
            secure: None,
            pair_id: other.clone(),
            desktop_id: "desktop_other".to_string(),
            nkey_seed: nkeys::KeyPair::new_user().seed().unwrap().to_string(),
            user_jwt: String::new(),
            nats_url: "nats://127.0.0.1:1".to_string(),
            nats_ws_url: "ws://127.0.0.1:1".to_string(),
            jwt_expires_at: 0,
            control_url: None,
        })
        .expect("seed the other device");
        super::super::pairing::set_active(&other).expect("persist the other device's control");
        SUPERVISOR.restore_active(Some(other.clone()));
        assert!(!SUPERVISOR.in_control(&bridge.pair_id));

        let reply = bridge
            .call(json!({ "id": unique("cmd"), "type": "list_sessions" }))
            .await;
        assert_eq!(reply["success"], json!(false), "got: {reply}");
        assert_eq!(reply["error"], json!("superseded"));

        // The one command a standby device may speak.
        let reply = bridge
            .call(json!({ "id": unique("cmd"), "type": "takeover" }))
            .await;
        assert_eq!(reply["success"], json!(true), "got: {reply}");
        assert_eq!(reply["data"]["control"], json!("active"));
        assert_eq!(reply["data"]["previous"], json!(other));
        assert!(SUPERVISOR.in_control(&bridge.pair_id));

        // …and business commands work again, from the same connection.
        let reply = bridge
            .call(json!({ "id": unique("cmd"), "type": "get_presence" }))
            .await;
        assert_eq!(reply["success"], json!(true), "got: {reply}");

        SUPERVISOR.restore_active(None);
        super::super::pairing::remove_slot(&other).expect("drop the other device");
        bridge.stop().await;
    }

    /// A device that is already in control does not "take over" again: the
    /// answer says nothing changed, so a client cannot report a handoff that
    /// never happened.
    #[tokio::test]
    async fn a_takeover_by_the_controller_changes_nothing() {
        let _lock = mock_agent_lock();
        let (_home, bridge) = active_bridge("cmd-takeover-self").await;
        let reply = bridge
            .call(json!({ "id": unique("cmd"), "type": "takeover" }))
            .await;
        assert_eq!(reply["success"], json!(true), "got: {reply}");
        assert_eq!(reply["data"]["control"], json!("active"));
        assert_eq!(reply["data"]["changed"], json!(false));
        assert_eq!(reply["data"]["previous"], Value::Null);
        bridge.stop().await;
    }

    /// A replay whose snapshot has already moved past the watermark the phone
    /// is filling is refused instead of answered with a page it cannot place:
    /// the phone would otherwise stitch events from two different projections
    /// into one transcript.
    #[tokio::test]
    async fn a_replay_whose_projection_moved_on_is_refused() {
        let _lock = mock_agent_lock();
        let (_home, bridge) = active_bridge("cmd-replay-window").await;
        let agent = ensure_mock_agent();
        agent.clear_scripts();
        let session = unique("sess");

        agent.script_typed_for(
            "get_events_since",
            &session,
            json!({
                "runId": "r",
                "events": [{"idx": 0, "type": "text_chunk", "data": "{}"}],
                "hasMore": false,
                "projection": {"cursor": 10}
            }),
        );
        // The caller pins the replay to index 2, but the projection has already
        // folded through 10 — the window moved under it.
        let reply = bridge
            .call(json!({
                "id": unique("cmd"), "type": "get_events_since", "sessionId": session,
                "runId": "r", "sinceIdx": 0, "limit": 5, "replayUntilIdx": 2
            }))
            .await;
        assert_eq!(reply["success"], json!(false), "got: {reply}");
        assert_eq!(reply["error"], json!("replay_window_changed"));

        // With the watermark at or past the projection cursor the page is
        // served, so the guard is about the ordering, not about the cursor
        // simply existing.
        agent.script_typed_for(
            "get_events_since",
            &session,
            json!({
                "runId": "r",
                "events": [{"idx": 0, "type": "text_chunk", "data": "{}"}],
                "hasMore": false,
                "projection": {"cursor": 2}
            }),
        );
        let reply = bridge
            .call(json!({
                "id": unique("cmd"), "type": "get_events_since", "sessionId": session,
                "runId": "r", "sinceIdx": 0, "limit": 5, "replayUntilIdx": 2
            }))
            .await;
        assert_eq!(reply["success"], json!(true), "got: {reply}");
        assert_eq!(reply["data"]["watermark"], json!(2));
        bridge.stop().await;
    }

    /// A phone gap-fill asks for the untrimmed page and the reply says so, so
    /// the client can tell a page that ends flush with its cursor from one the
    /// bridge trimmed. The flag must not appear on a page nobody asked to keep
    /// whole. The backward page is served from the mock's own `get_session_entries`
    /// payload (it ignores `before`), which is why this drives the real
    /// `before`-cursor path through the handler rather than a scripted reply.
    #[tokio::test]
    async fn an_untrimmed_page_is_marked_as_such() {
        let _lock = mock_agent_lock();
        let (_home, bridge) = active_bridge("cmd-untrimmed").await;
        let agent = ensure_mock_agent();
        agent.clear_scripts();
        let session = unique("sess");
        agent.set_session_entries(
            &session,
            json!({ "entries": [{"id": "e1", "role": "user", "blocks": []}], "hasMore": false }),
        );

        let untrimmed = bridge
            .call(json!({
                "id": unique("cmd"), "type": "get_session_entries", "sessionId": session,
                "before": 10, "limit": 5, "chunkedRead": true, "untrimmed": true
            }))
            .await;
        assert_eq!(untrimmed["success"], json!(true), "got: {untrimmed}");
        assert_eq!(untrimmed["data"]["untrimmed"], json!(true));
        assert_eq!(untrimmed["data"]["entries"][0]["id"], json!("e1"));

        let trimmed = bridge
            .call(json!({
                "id": unique("cmd"), "type": "get_session_entries", "sessionId": session,
                "before": 10, "limit": 5, "chunkedRead": true
            }))
            .await;
        assert_eq!(trimmed["success"], json!(true), "got: {trimmed}");
        assert!(
            trimmed["data"].get("untrimmed").is_none(),
            "only the requested page is marked: {trimmed}"
        );
        bridge.stop().await;
    }

    /// A backward-history read the agent refuses is reported as a failure with
    /// the agent's own reason. Answering an empty page instead would look to the
    /// user like the conversation has no history at all.
    #[tokio::test]
    async fn a_failed_backward_page_is_not_an_empty_conversation() {
        let _lock = mock_agent_lock();
        let (_home, bridge) = active_bridge("cmd-backward-fail").await;
        let agent = ensure_mock_agent();
        agent.clear_scripts();
        let session = unique("sess");
        agent.script_for(
            "get_session_entries",
            &session,
            false,
            Value::Null,
            "history backend down",
        );
        let reply = bridge
            .call(json!({
                "id": unique("cmd"), "type": "get_session_entries", "sessionId": session,
                "before": 10, "limit": 5
            }))
            .await;
        assert_eq!(reply["success"], json!(false), "got: {reply}");
        assert!(
            reply["error"]
                .as_str()
                .unwrap()
                .contains("history backend down"),
            "got: {reply}"
        );
        bridge.stop().await;
    }

    /// A command id *is* the identity of the work it starts: a mobile outbox
    /// retries the same `prompt` after a lost reply, and the desktop must answer
    /// that retry with the run it already created — never with a second one.
    /// Reusing the id for a *different* command is refused rather than
    /// reinterpreted, which is what keeps an id from being silently reshaped by
    /// whatever arrived last.
    #[tokio::test]
    async fn a_prompt_id_is_the_identity_of_its_run() {
        let _lock = mock_agent_lock();
        let (_home, bridge) = active_bridge("cmd-replay-receipt").await;
        let agent = ensure_mock_agent();
        agent.clear_scripts();
        let session = unique("sess");
        // A phone prompts a conversation it listed, so the fixture needs the
        // thread `list_sessions` would have shown for that id.
        crate::remote::test_support::thread_for_session(&session);
        let prompt_id = unique("cmd");
        let body = || {
            json!({
                "id": prompt_id, "type": "prompt", "sessionId": session,
                "message": "hello", "modelId": "m1", "providerId": "p1"
            })
        };

        agent.complete_next_run_stream();
        let first = bridge.call(body()).await;
        assert_eq!(first["success"], json!(true), "got: {first}");
        let run_id = first["data"]["runId"].as_str().unwrap().to_string();

        // The immediate retry (inside the reply-slot window) is answered from
        // the slot, byte for byte.
        let duplicate = bridge.call(body()).await;
        assert_eq!(
            duplicate, first,
            "a retry replays the reply, it does not re-run"
        );

        // The same id with a different command is a conflict, not a new
        // meaning for the id.
        let conflict = bridge
            .call(json!({
                "id": prompt_id, "type": "continue_run", "sessionId": session, "runId": run_id
            }))
            .await;
        assert_eq!(conflict["success"], json!(false), "got: {conflict}");
        assert_eq!(conflict["error"], json!("command_id_conflict"));

        // Let the run settle, then let the reply slot expire (500 ms in tests).
        // The poll panics with its own message if the run never settles, so a
        // hang here can never be mistaken for a pass.
        wait_until("the run must settle", Duration::from_secs(5), || {
            crate::store::get_run(&run_id)
                .expect("run lookup")
                .expect("run row")
                .status
                != "running"
        })
        .await;
        tokio::time::sleep(Duration::from_millis(650)).await;

        // With the slot gone the stored receipt still answers: the phone gets
        // the run it already has. A second run would carry a different id, so
        // this is the assertion that proves no duplicate was created.
        let replayed = bridge.call(body()).await;
        assert_eq!(replayed["success"], json!(true), "got: {replayed}");
        assert_eq!(replayed["data"]["runId"], json!(run_id));
        assert_eq!(replayed["data"]["threadId"], first["data"]["threadId"]);
        bridge.stop().await;
    }

    /// `get_prompt_receipt` answers `null` for an unknown id and the receipt for
    /// a known one, so the phone can poll a prompt it may never have sent.
    #[tokio::test]
    async fn a_prompt_receipt_is_null_for_an_unknown_id() {
        let _lock = mock_agent_lock();
        let (_home, bridge) = active_bridge("cmd-receipt").await;
        let agent = ensure_mock_agent();
        agent.complete_next_run_stream();

        let reply = bridge
            .call(json!({
                "id": unique("cmd"), "type": "get_prompt_receipt",
                "promptId": unique("never-sent")
            }))
            .await;
        assert_eq!(reply["success"], json!(true), "got: {reply}");
        assert!(reply["data"].is_null());

        let prompt_id = unique("cmd");
        // Named by its own thread, as a listed conversation is.
        let session = unique("sess");
        crate::remote::test_support::thread_for_session(&session);
        agent.complete_next_run_stream();
        let started = bridge
            .call(json!({
                "id": prompt_id, "type": "prompt", "sessionId": session,
                "message": "hello", "modelId": "m1", "providerId": "p1"
            }))
            .await;
        assert_eq!(started["success"], json!(true), "got: {started}");
        let receipt = bridge
            .call(json!({
                "id": unique("cmd"), "type": "get_prompt_receipt", "promptId": prompt_id
            }))
            .await;
        assert_eq!(receipt["success"], json!(true), "got: {receipt}");
        assert_eq!(receipt["data"]["runId"], started["data"]["runId"]);
        assert_eq!(receipt["data"]["threadId"], started["data"]["threadId"]);
        bridge.stop().await;
    }

    // ── The encrypted lane, and the access epoch ────────────────────────────
    //
    // Every command passes two gates before a handler runs: the connection must
    // be admitted under the *live* access epoch, and a v2 connection must also
    // present a record sealed by the channel it proved. Each test below pins one
    // gate with a phone-visible outcome (a named refusal, or silence) plus the
    // thing the gate exists to prevent — the host running, a credential written,
    // a reply published in the clear.

    /// The point of the secure lane: a paired phone's commands are sealed,
    /// answered sealed, and answered at all only after `secure_ready` activated
    /// the very channel it proved.
    #[tokio::test]
    async fn a_paired_phone_gets_its_commands_answered_over_the_encrypted_lane() {
        let _home = HomeGuard::new("cmd-secure-lane");
        init_store();
        let (bridge, invitation) = Bridge::start_secure(None).await;
        let mut channel =
            crate::remote::test_support::secure_pair(&bridge.client, &invitation, &bridge.pair_id)
                .await;
        assert!(
            bridge.handshake.active_flag().load(Ordering::Acquire),
            "the channel the phone proved must be the active one"
        );

        let subject = format!("p.{}.cmd.rpc", bridge.pair_id);
        let request = channel
            .seal(
                &subject,
                &serde_json::to_vec(&json!({"id": "secure-business", "type": "get_presence"}))
                    .unwrap(),
            )
            .unwrap();
        let context = future_remote_crypto::reply_context(&subject, &request).unwrap();
        let reply = bridge
            .client
            .request(subject, request.into())
            .await
            .unwrap();
        let reply: Value =
            serde_json::from_slice(&channel.open(&context, &reply.payload).unwrap()).unwrap();
        assert_eq!(reply["success"], json!(true), "got: {reply}");
        assert_eq!(reply["data"]["online"], json!(true));
        bridge.stop().await;
    }

    /// A record that cannot be opened is dropped: it must not reach the
    /// dispatcher as if it were plaintext, and it must not be answered.
    #[tokio::test]
    async fn an_unopenable_secure_record_is_dropped_without_a_reply() {
        let _home = HomeGuard::new("cmd-secure-garbage");
        let (bridge, _invitation) = Bridge::start_secure(None).await;
        let mut tap = bridge.nats.tap();
        bridge.nats.inject(
            &format!("p.{}.cmd.rpc", bridge.pair_id),
            Some("rep_unopenable"),
            b"FRE2this is not a sealed record".to_vec(),
        );
        crate::remote::test_support::assert_no_publish(
            &mut tap,
            "rep_unopenable",
            Duration::from_millis(250),
        )
        .await;
        bridge.stop().await;
    }

    /// Handshake messages are never business commands, even when the paired
    /// peer seals them inside a record the desktop can open: answering a
    /// `pair_handshake_confirm` through `handle_command` would tell an already
    /// authenticated phone to pair first.
    #[tokio::test]
    async fn a_handshake_sealed_inside_a_secure_record_is_never_a_business_command() {
        let _home = HomeGuard::new("cmd-secure-handshake");
        init_store();
        let (bridge, invitation) = Bridge::start_secure(None).await;
        let mut channel =
            crate::remote::test_support::secure_pair(&bridge.client, &invitation, &bridge.pair_id)
                .await;
        let subject = format!("p.{}.cmd.rpc", bridge.pair_id);
        let mut tap = bridge.nats.tap();
        for cmd_type in ["pair_handshake", "pair_handshake_confirm"] {
            let reply_subject = format!("rep_{cmd_type}");
            let wire = channel
                .seal(
                    &subject,
                    &serde_json::to_vec(&json!({"id": "sealed-handshake", "type": cmd_type}))
                        .unwrap(),
                )
                .unwrap();
            bridge.nats.inject(&subject, Some(&reply_subject), wire);
            crate::remote::test_support::assert_no_publish(
                &mut tap,
                &reply_subject,
                Duration::from_millis(200),
            )
            .await;
        }
        bridge.stop().await;
    }

    /// Readiness is a commit, not a consequence of finishing the handshake: a
    /// channel that has only *proved* the invitation is a candidate, and
    /// commands sealed with it must be dropped until `secure_ready` swaps it in.
    /// Otherwise a half-open candidate would answer on behalf of a session it
    /// has not been admitted to.
    #[tokio::test]
    async fn a_channel_that_has_not_declared_readiness_is_not_yet_active() {
        let _home = HomeGuard::new("cmd-secure-not-ready");
        init_store();
        let (bridge, invitation) = Bridge::start_secure(None).await;
        let mut channel = crate::remote::test_support::open_secure_channel(
            &bridge.client,
            &invitation,
            &bridge.pair_id,
        )
        .await;
        assert!(
            !bridge.handshake.active_flag().load(Ordering::Acquire),
            "a proven invitation is not readiness"
        );
        let subject = format!("p.{}.cmd.rpc", bridge.pair_id);
        let wire = channel
            .seal(
                &subject,
                &serde_json::to_vec(&json!({"id": "too-early", "type": "get_presence"})).unwrap(),
            )
            .unwrap();
        let mut tap = bridge.nats.tap();
        bridge.nats.inject(&subject, Some("rep_too_early"), wire);
        crate::remote::test_support::assert_no_publish(
            &mut tap,
            "rep_too_early",
            Duration::from_millis(250),
        )
        .await;
        bridge.stop().await;
    }

    /// A handshake opening with no reply subject is processed — the desktop has
    /// no way to know the peer is not listening — but nothing at all is
    /// published, because there is nowhere to publish it.
    #[tokio::test]
    async fn a_handshake_opening_without_a_reply_subject_sends_nothing() {
        let _home = HomeGuard::new("cmd-handshake-no-reply");
        let (bridge, _invitation) = Bridge::start_secure(None).await;
        let mut tap = bridge.nats.tap();
        bridge.nats.inject(
            &format!("p.{}.cmd.handshake", bridge.pair_id),
            None,
            b"{\"type\":\"secure_open\"}".to_vec(),
        );
        // The tap sees the injection itself first (it is the broker's own
        // publish feed), so the check is that *nothing follows it*.
        let injected = tokio::time::timeout(Duration::from_millis(250), tap.recv())
            .await
            .expect("the injection is observed")
            .expect("a live tap");
        assert_eq!(injected.reply, None, "the opening carried no reply subject");
        assert!(
            tokio::time::timeout(Duration::from_millis(250), tap.recv())
                .await
                .is_err(),
            "a one-way opening must not answer into thin air"
        );
        bridge.stop().await;
    }

    /// A connection admitted under an epoch that has since moved on owns no
    /// access at all: even a sealed record is dropped before it is opened.
    #[tokio::test]
    async fn a_secure_connection_admitted_under_a_stale_epoch_answers_nothing() {
        let _home = HomeGuard::new("cmd-secure-stale-epoch");
        let (bridge, _invitation) = Bridge::start_secure(Some(u64::MAX)).await;
        let mut tap = bridge.nats.tap();
        bridge.nats.inject(
            &format!("p.{}.cmd.rpc", bridge.pair_id),
            Some("rep_secure_stale"),
            b"FRE2x".to_vec(),
        );
        crate::remote::test_support::assert_no_publish(
            &mut tap,
            "rep_secure_stale",
            Duration::from_millis(250),
        )
        .await;
        bridge.stop().await;
    }

    /// Counts how often the bridge actually handed a command to the host.
    struct CountingHost;
    static COUNTING: CountingHost = CountingHost;
    static COUNTING_RUNS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    impl future_remote_server::services::BusinessHost for CountingHost {
        fn execute<'a>(
            &'a self,
            cmd: IncomingCmd,
            sink: &'a dyn ReplySink,
        ) -> futures::future::BoxFuture<'a, ()> {
            Box::pin(async move {
                COUNTING_RUNS.fetch_add(1, Ordering::AcqRel);
                sink.send(true, json!({"id": cmd.id}), None).await
            })
        }
    }

    /// The admission check on the plaintext lane: a command delivered to a
    /// generation whose epoch is gone must not reach the host at all. The live
    /// half of the same test proves the fixture can run a command, so the
    /// silence is about the epoch and not about the harness.
    #[tokio::test]
    async fn a_command_admitted_under_a_stale_epoch_never_reaches_the_host() {
        let _home = HomeGuard::new("cmd-stale-epoch-host");
        let before = COUNTING_RUNS.load(Ordering::Acquire);
        let stale = Bridge::start_with(&COUNTING, Some(u64::MAX)).await;
        stale.activate();
        let mut tap = stale.nats.tap();
        stale.nats.inject(
            &format!("p.{}.cmd.rpc", stale.pair_id),
            Some("rep_stale_host"),
            serde_json::to_vec(&json!({"id": "stale", "type": "get_state"})).unwrap(),
        );
        crate::remote::test_support::assert_no_publish(
            &mut tap,
            "rep_stale_host",
            Duration::from_millis(250),
        )
        .await;
        assert_eq!(
            COUNTING_RUNS.load(Ordering::Acquire),
            before,
            "a stale epoch must not run the command"
        );

        let live = Bridge::start_with(&COUNTING, Some(SUPERVISOR.access.current())).await;
        live.activate();
        let reply = live
            .call(json!({"id": unique("cmd"), "type": "get_state"}))
            .await;
        assert_eq!(reply["success"], json!(true), "got: {reply}");
        assert_eq!(COUNTING_RUNS.load(Ordering::Acquire), before + 1);
        live.stop().await;
        stale.stop().await;
    }

    /// The same gate inside `handle_command`, which is also reached directly by
    /// the unpair path: a stale epoch must be refused before the payload is even
    /// decoded, let alone answered.
    #[tokio::test]
    async fn a_stale_epoch_drops_the_command_before_it_is_decoded() {
        let _home = HomeGuard::new("cmd-stale-epoch-decoded");
        let nats = FakeNats::start().await;
        let client = nats_connect(&nats).await;
        let handshake = new_handshake_state(
            bridge_creds(),
            Arc::new(AtomicBool::new(false)),
            format!("bridge_{}", unique("cmd")),
        )
        .with_access(u64::MAX);
        let mut tap = nats.tap();
        let msg = async_nats::Message {
            subject: "p.pair_stale.cmd.rpc".into(),
            reply: Some("rep_stale_decoded".into()),
            payload: serde_json::to_vec(&json!({"id": "x", "type": "get_state"}))
                .unwrap()
                .into(),
            headers: None,
            status: None,
            description: None,
            length: 0,
        };
        handle_command(&client, msg, handshake).await;
        crate::remote::test_support::assert_no_publish(
            &mut tap,
            "rep_stale_decoded",
            Duration::from_millis(250),
        )
        .await;
    }

    /// The single-flight retry path re-checks the epoch *after* it acquires the
    /// reply slot. A retry that waited for an in-flight delivery and won the
    /// lock only after the epoch moved on must be dropped, not served a reply
    /// the new generation never produced.
    #[tokio::test]
    async fn an_epoch_revoked_while_a_retry_waited_for_its_slot_drops_the_retry() {
        let _home = HomeGuard::new("cmd-epoch-contended");
        let nats = FakeNats::start().await;
        let client = nats_connect(&nats).await;
        let handshake = new_handshake_state(
            bridge_creds(),
            Arc::new(AtomicBool::new(false)),
            format!("bridge_{}", unique("cmd")),
        )
        .with_access(SUPERVISOR.access.current());
        let slots = new_reply_slots();
        let request = json!({"id": "contended", "type": "get_state"});
        let response = test_support::insert_reply_slot(&slots, "contended".into(), request.clone());
        let in_flight = response.lock().await;
        let msg = async_nats::Message {
            subject: "p.pair.cmd.rpc".into(),
            reply: Some("rep_contended".into()),
            payload: serde_json::to_vec(&request).unwrap().into(),
            headers: None,
            status: None,
            description: None,
            length: 0,
        };
        let mut retry = Box::pin(handle_command_singleflight(&client, msg, slots, handshake));
        // One poll carries the retry past its admission check and parks it on
        // the slot; nothing yields in between, so this is a fact, not a race.
        assert!(
            futures::FutureExt::now_or_never(retry.as_mut()).is_none(),
            "a retry must wait for the in-flight delivery, not answer from it"
        );
        SUPERVISOR.access.invalidate(|| {});
        drop(in_flight);
        retry.await;
        let mut tap = nats.tap();
        crate::remote::test_support::assert_no_publish(
            &mut tap,
            "rep_contended",
            Duration::from_millis(250),
        )
        .await;
    }

    /// Every identity field in a pairing handshake is checked by name. The name
    /// is the whole point: it is what lets the desktop tell the user *why* a
    /// scanned invitation did not work. A refusal must also leave no challenge
    /// behind and must not activate the bridge.
    #[tokio::test]
    async fn a_handshake_that_disagrees_on_one_identity_field_is_refused() {
        let _home = HomeGuard::new("cmd-identity-ladder");
        let bridge = Bridge::start().await;
        let creds = bridge.handshake.credentials().clone();
        let client_key = nkeys::KeyPair::new_user();
        let other_key = nkeys::KeyPair::new_user();
        for (field, value) in [
            ("pairId", json!("pair_someone_else")),
            ("expectedDesktopId", json!("desktop_someone_else")),
            ("expectedDesktopPublicKey", json!(other_key.public_key())),
            ("deviceId", json!("phone")),
            ("clientPublicKey", json!("not-a-user-key")),
            ("clientNonce", json!("too-short")),
        ] {
            let mut cmd = handshake_cmd(&creds, &client_key);
            cmd[field] = value;
            let reply = bridge.call(cmd).await;
            assert_eq!(reply["success"], json!(false), "field {field}: {reply}");
            assert_eq!(
                reply["error"],
                json!("pairing_identity_mismatch"),
                "field {field}: {reply}"
            );
        }
        assert!(
            test_support::pending_count(&bridge.handshake) == 0,
            "a refused handshake must not leave a challenge behind"
        );
        assert!(
            !bridge.handshake.active_flag().load(Ordering::Acquire),
            "a refused handshake must not activate the bridge"
        );
        bridge.stop().await;
    }

    /// The challenge table is bounded, so an unauthenticated peer cannot grow it
    /// without limit. The refusal is silent (there is no reply to give), but it
    /// must not displace a challenge the table already holds either — that would
    /// let a flood knock a legitimately pairing phone out.
    #[tokio::test]
    async fn the_challenge_table_stops_accepting_a_flood() {
        let _home = HomeGuard::new("cmd-challenge-flood");
        let bridge = Bridge::start().await;
        let creds = bridge.handshake.credentials().clone();
        for _ in 0..32 {
            let key = nkeys::KeyPair::new_user();
            let reply = bridge.call(handshake_cmd(&creds, &key)).await;
            assert_eq!(reply["success"], json!(true), "got: {reply}");
        }
        assert_eq!(test_support::pending_count(&bridge.handshake), 32);

        let mut tap = bridge.nats.tap();
        bridge.nats.inject(
            &format!("p.{}.cmd.pair_handshake", bridge.pair_id),
            Some("rep_flood"),
            serde_json::to_vec(&handshake_cmd(&creds, &nkeys::KeyPair::new_user())).unwrap(),
        );
        crate::remote::test_support::assert_no_publish(
            &mut tap,
            "rep_flood",
            Duration::from_millis(250),
        )
        .await;
        assert_eq!(
            test_support::pending_count(&bridge.handshake),
            32,
            "the refused challenge must not displace one already issued"
        );
        bridge.stop().await;
    }

    /// `pair_handshake_confirm` commits the pairing under the epoch the
    /// connection was admitted under. Under the live epoch the phone is
    /// confirmed and the credential is persisted; under a stale one the commit is
    /// refused, and nothing is written or answered — a credential saved after
    /// `stop()` invalidated the generation would outlive the bridge that proved
    /// it.
    #[tokio::test]
    async fn a_confirm_commits_only_under_its_own_access_epoch() {
        let _home = HomeGuard::new("cmd-confirm-epoch");
        init_store();
        for (label, epoch) in [("live", SUPERVISOR.access.current()), ("stale", u64::MAX)] {
            let nats = FakeNats::start().await;
            let client = nats_connect(&nats).await;
            let creds = bridge_creds();
            let client_key = nkeys::KeyPair::new_user();
            let bridge_instance_id = format!("bridge_{}", unique("cmd"));
            let state = new_handshake_state(
                creds.clone(),
                Arc::new(AtomicBool::new(false)),
                bridge_instance_id.clone(),
            )
            .with_access(epoch);
            // A challenge the desktop already issued for this client.
            let desktop_nonce = nkeys::KeyPair::new_user().public_key();
            let transcript = handshake_transcript(&HandshakeTranscript {
                pair_id: &creds.pair_id,
                desktop_id: &creds.desktop_id,
                desktop_public_key: &crate::remote::pairing::public_key(&creds).unwrap(),
                bridge_instance_id: &bridge_instance_id,
                device_id: "dev_test",
                client_public_key: &client_key.public_key(),
                client_nonce: "nonce-0123456789abcdef",
                desktop_nonce: &desktop_nonce,
            });
            test_support::insert_pending(
                &state,
                desktop_nonce.clone(),
                transcript.clone(),
                "dev_test".into(),
                client_key.public_key(),
                std::time::Instant::now(),
            );
            crate::remote::pairing::clear_creds().expect("start with no persisted pairing");

            let reply_subject = format!("rep_confirm_{label}");
            let cmd: IncomingCmd = serde_json::from_value(json!({
                "id": unique("cmd"),
                "type": "pair_handshake_confirm",
                "deviceId": "dev_test",
                "desktopNonce": desktop_nonce,
                "clientSignature": URL_SAFE_NO_PAD
                    .encode(client_key.sign(transcript.as_bytes()).unwrap()),
            }))
            .unwrap();
            let msg = async_nats::Message {
                subject: format!("p.{}.cmd.pair_handshake_confirm", creds.pair_id).into(),
                reply: Some(reply_subject.clone().into()),
                payload: Vec::new().into(),
                headers: None,
                status: None,
                description: None,
                length: 0,
            };
            let mut tap = nats.tap();
            handle_pair_handshake_confirm(&client, &msg, &cmd, &state).await;
            if label == "live" {
                let reply: Value = await_publish(&mut tap, &reply_subject, Duration::from_secs(5))
                    .await
                    .json();
                assert_eq!(reply["success"], json!(true), "got: {reply}");
                assert_eq!(reply["data"]["confirmed"], json!(true));
                assert!(state.active_flag().load(Ordering::Acquire));
                assert!(
                    crate::remote::pairing::load_creds().is_some(),
                    "a confirmed pairing is persisted"
                );
            } else {
                // A generation whose epoch is gone is refused by name at the
                // access screen, before the commit is ever reached.
                let refused: Value =
                    await_publish(&mut tap, &reply_subject, Duration::from_secs(5))
                        .await
                        .json();
                assert_eq!(refused["success"], json!(false), "got: {refused}");
                assert_eq!(refused["error"], json!("pairing_identity_mismatch"));
                assert!(
                    !state.active_flag().load(Ordering::Acquire),
                    "a stale epoch must not activate the bridge"
                );
                assert!(
                    crate::remote::pairing::load_creds().is_none(),
                    "a stale epoch must not persist a credential"
                );
            }
        }
    }

    /// A reply that never reached a broker is not a delivered reply: the failure
    /// is logged and the function returns without pretending otherwise. The
    /// command channel closes when the connection handler exits, which is what a
    /// drained or expired connection leaves behind.
    #[tokio::test]
    async fn a_reply_that_cannot_be_published_is_not_reported_as_delivered() {
        let nats = FakeNats::start().await;
        let client = nats_connect(&nats).await;
        client.drain().await.expect("drain the client");
        // Poll the condition the reply path depends on rather than guessing a
        // delay: the handler exits once the drain completes, and that closes the
        // command channel every `publish` goes through.
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while client
            .publish("probe".to_string(), Vec::<u8>::new().into())
            .await
            .is_ok()
        {
            assert!(
                std::time::Instant::now() < deadline,
                "the drained client never closed its command channel"
            );
            tokio::time::sleep(Duration::from_millis(5)).await;
        }

        let msg = async_nats::Message {
            subject: "p.pair.cmd.rpc".into(),
            reply: Some("rep_unpublishable".into()),
            payload: Vec::new().into(),
            headers: None,
            status: None,
            description: None,
            length: 0,
        };
        let mut tap = nats.tap();
        publish_reply_payload(&client, &msg, b"payload".to_vec()).await;
        crate::remote::test_support::assert_no_publish(
            &mut tap,
            "rep_unpublishable",
            Duration::from_millis(250),
        )
        .await;
    }

    /// The unpair cleanup is best-effort: the phone already has its
    /// acknowledgement, so a local persistence failure is logged and the command
    /// still reports success — the user's unpair must not appear to fail because
    /// the desktop could not write its bookkeeping.
    #[tokio::test]
    async fn an_unpair_that_cannot_persist_is_still_acknowledged() {
        let _home = HomeGuard::new("cmd-unpair-persist-failure");
        init_store();
        crate::remote::pairing::save_creds(&secure_bridge_creds()).expect("seed a pairing");
        // The HOME the pairing lives under disappears between the ack and the
        // spawned cleanup, so the destructive work cannot be recorded.
        let previous_userprofile = std::env::var("USERPROFILE").ok();
        std::env::remove_var("HOME");
        std::env::remove_var("USERPROFILE");
        // The premise is asserted, not assumed: with no HOME the local unpair
        // cannot record anything, which is the only reason the spawned cleanup
        // below can fail. The generation this first call stops is a throwaway —
        // the connection under test is admitted under the epoch captured after
        // it.
        assert!(
            crate::remote::unpair().await.is_err(),
            "a pairing under an unset HOME must fail to persist"
        );

        let nats = FakeNats::start().await;
        let client = nats_connect(&nats).await;
        let handshake = new_handshake_state(
            bridge_creds(),
            Arc::new(AtomicBool::new(false)),
            format!("bridge_{}", unique("cmd")),
        )
        .with_access(SUPERVISOR.access.current());
        handshake.active_flag().store(true, Ordering::Release);
        let reply_subject = "rep_unpair_persist".to_string();
        let msg = async_nats::Message {
            subject: "p.pair.cmd.unpair".into(),
            reply: Some(reply_subject.clone().into()),
            payload: serde_json::to_vec(&json!({"id": unique("cmd"), "type": "unpair"}))
                .unwrap()
                .into(),
            headers: None,
            status: None,
            description: None,
            length: 0,
        };
        let mut tap = nats.tap();
        handle_command(&client, msg, handshake).await;
        let ack: Value = await_publish(&mut tap, &reply_subject, Duration::from_secs(5))
            .await
            .json();
        assert_eq!(ack["success"], json!(true), "got: {ack}");
        // The acknowledgement is published before the cleanup runs; yield so the
        // spawned task observably reaches the failure while HOME is still gone.
        tokio::task::yield_now().await;
        // `home_dir` ignores an empty override exactly as it ignores an absent
        // one, so restoring to the empty string needs no branch on whether this
        // process happened to carry a USERPROFILE.
        std::env::set_var("USERPROFILE", previous_userprofile.unwrap_or_default());
    }

    /// A phone-initiated unpair acknowledges first and does the destructive work
    /// in a spawned task — which must re-check the epoch, because `stop()`
    /// invalidates it in between. Both halves are asserted: with a live epoch the
    /// local pairing is cleared, and with a revoked one it is left exactly as it
    /// was, so an unpair from a torn-down generation cannot destroy a pairing the
    /// user never asked to remove.
    #[tokio::test]
    async fn an_unpair_whose_epoch_was_revoked_leaves_the_pairing_alone() {
        let _home = HomeGuard::new("cmd-unpair-epoch");
        init_store();
        for (label, stale) in [("live", false), ("revoked", true)] {
            let nats = FakeNats::start().await;
            let client = nats_connect(&nats).await;
            // The device that asks to be unpaired must be the one on disk: an
            // unpair is "remove me", so a fixture whose two halves name
            // different pairings would be asking to remove a device that was
            // never saved.
            let creds = secure_bridge_creds();
            crate::remote::pairing::save_creds(&creds).expect("seed a pairing");
            let handshake = new_handshake_state(
                creds,
                Arc::new(AtomicBool::new(false)),
                format!("bridge_{}", unique("cmd")),
            )
            .with_access(SUPERVISOR.access.current());
            handshake.active_flag().store(true, Ordering::Release);
            let reply_subject = format!("rep_unpair_{label}");
            let msg = async_nats::Message {
                subject: "p.pair.cmd.unpair".into(),
                reply: Some(reply_subject.clone().into()),
                payload: serde_json::to_vec(&json!({"id": unique("cmd"), "type": "unpair"}))
                    .unwrap()
                    .into(),
                headers: None,
                status: None,
                description: None,
                length: 0,
            };
            let mut tap = nats.tap();
            handle_command(&client, msg, handshake).await;
            if stale {
                // The generation is torn down before the spawned cleanup runs;
                // the current-thread test runtime has not scheduled it yet.
                SUPERVISOR.access.invalidate(|| {});
            }
            let ack: Value = await_publish(&mut tap, &reply_subject, Duration::from_secs(5))
                .await
                .json();
            assert_eq!(ack["success"], json!(true), "got: {ack}");
            if stale {
                // Nothing observable marks the skipped cleanup, so give it the
                // same window the live case needs to complete.
                tokio::time::sleep(Duration::from_millis(200)).await;
                assert!(
                    crate::remote::pairing::load_creds().is_some(),
                    "a revoked epoch must not clear the pairing"
                );
            } else {
                wait_until(
                    "the live unpair clears the local pairing",
                    Duration::from_secs(5),
                    || crate::remote::pairing::load_creds().is_none(),
                )
                .await;
            }
        }
    }

    /// A standby device may ask about itself — that is how the phone shows
    /// "connected to the machine, not driving it" — but the answer carries its
    /// own liveness only. The conversation list belongs to the machine, and
    /// handing it to every paired device would make "only one controls" a
    /// statement about commands alone.
    #[tokio::test]
    async fn a_standby_device_is_answered_with_its_own_liveness_only() {
        let _home = HomeGuard::new("cmd-standby-presence");
        let bridge = Bridge::start().await;
        bridge.activate();
        let controller = format!("pair_{}", unique("ctl"));
        SUPERVISOR.remember_device(&controller);
        SUPERVISOR.restore_active(Some(controller.clone()));
        assert!(
            !SUPERVISOR.in_control(&bridge.pair_id),
            "the fixture must actually be the standby one"
        );

        let reply = bridge
            .call(json!({ "id": unique("cmd"), "type": "get_presence" }))
            .await;

        assert_eq!(reply["success"], json!(true), "got: {reply}");
        assert_eq!(reply["data"]["control"], json!("standby"));
        assert_eq!(reply["data"]["pairId"], json!(bridge.pair_id));
        assert!(
            reply["data"].get("sessions").is_none(),
            "a standby device must not receive the machine's conversations: {reply}"
        );
        assert!(reply["data"].get("workspaces").is_none(), "{reply}");
        bridge.stop().await;
    }

    /// Proving presence is not commanding the machine: a standby device's beat
    /// is answered instead of refused `superseded`, and its pairing reads as
    /// present. Without this a standby device that sits idle would look gone —
    /// which is the same row a client that really left gets.
    #[tokio::test]
    async fn a_standby_device_may_say_it_is_still_there() {
        let _home = HomeGuard::new("cmd-client-presence-standby");
        let bridge = Bridge::start().await;
        bridge.activate();
        let controller = format!("pair_{}", unique("ctl"));
        SUPERVISOR.remember_device(&controller);
        SUPERVISOR.restore_active(Some(controller.clone()));
        assert!(
            !SUPERVISOR.in_control(&bridge.pair_id),
            "the fixture must actually be the standby one"
        );

        let reply = bridge
            .call(json!({ "id": unique("cmd"), "type": "client_presence" }))
            .await;

        assert_eq!(reply["success"], json!(true), "got: {reply}");
        assert!(
            SUPERVISOR.client_link(&bridge.pair_id).present(),
            "a beat is proof the client is there"
        );
        SUPERVISOR.forget_device(&controller);
        bridge.stop().await;
    }

    /// Going off screen stops the device reading as present, but keeps the
    /// machine it holds: a suspended app is coming back, and taking its machine
    /// away every time it is backgrounded would be a different product decision.
    #[tokio::test]
    async fn a_backgrounded_client_stops_being_present_without_losing_the_machine() {
        let _home = HomeGuard::new("cmd-client-background");
        let bridge = Bridge::start().await;
        bridge.activate();
        SUPERVISOR.client_link(&bridge.pair_id).observe();
        assert_eq!(
            SUPERVISOR.active_pair().as_deref(),
            Some(bridge.pair_id.as_str())
        );

        let reply = bridge
            .call(json!({
                "id": unique("cmd"),
                "type": "client_disconnect",
                "reason": "background"
            }))
            .await;

        assert_eq!(reply["success"], json!(true), "got: {reply}");
        assert!(!SUPERVISOR.client_link(&bridge.pair_id).present());
        assert_eq!(
            SUPERVISOR.active_pair().as_deref(),
            Some(bridge.pair_id.as_str()),
            "the machine it was holding is still its own"
        );
        bridge.stop().await;
    }

    /// The user's own disconnect gives up the machine: the pointer it held is
    /// released, which is what stops the page drawing "in control" over a
    /// client that has gone.
    #[tokio::test]
    async fn a_user_disconnect_gives_the_machine_up() {
        let _home = HomeGuard::new("cmd-client-leaves");
        let bridge = Bridge::start().await;
        bridge.activate();
        SUPERVISOR.client_link(&bridge.pair_id).observe();

        let reply = bridge
            .call(json!({
                "id": unique("cmd"),
                "type": "client_disconnect",
                "reason": "disconnected"
            }))
            .await;

        assert_eq!(reply["success"], json!(true), "got: {reply}");
        assert!(!SUPERVISOR.client_link(&bridge.pair_id).present());
        assert_eq!(
            SUPERVISOR.active_pair(),
            None,
            "the device that left does not keep the machine"
        );
        bridge.stop().await;
    }
}

#[cfg(test)]
mod client_identity_tests {
    use super::*;

    /// Recording for a pairing with no live runtime is a no-op rather than a
    /// panic: the accessor has nothing to hand back, and a `secure_ready` that
    /// arrived for a runtime already being torn down must not take the bridge
    /// with it.
    #[test]
    fn recording_without_a_runtime_does_nothing() {
        record_client_identity(
            "pair_with_no_runtime",
            &IncomingCmd {
                device_name: "Nowhere".into(),
                device_kind: "desktop".into(),
                ..Default::default()
            },
        );
        // Nothing recorded, and nothing to read back: the pairing is unknown.
        assert!(
            super::super::SUPERVISOR
                .client_identity("pair_with_no_runtime")
                .is_none(),
            "an unknown pairing has no slot to record into"
        );
    }
}
