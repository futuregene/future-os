use super::*;

#[cfg(test)]
mod contract_tests {
    use super::*;

    #[test]
    fn start_failure_maps_every_remote_error_code() {
        // A revoked credential surfaces as a localized Revoked status.
        let revoked = start_failure(crate::AppError::Remote {
            status: 401,
            code: Some("invalid_remote_credential".to_string()),
            message: "revoked".to_string(),
        })
        .unwrap();
        assert_eq!(revoked.phase, RemotePhase::Revoked);
        assert_eq!(revoked.reason, Some(RemoteFailureReason::CredentialRevoked));

        // A rejected bridge credential is terminal service authorization; it
        // falls through the phase catch-all (Failed) but carries the specific reason.
        let authz =
            start_failure(crate::AppError::RemoteAuthorization("rejected".to_string())).unwrap();
        assert_eq!(authz.phase, RemotePhase::Failed);
        assert_eq!(
            authz.reason,
            Some(RemoteFailureReason::ServiceAuthorization)
        );

        // A rejected FutureOS account key asks the user to sign in again; it is
        // distinct from a freshly issued bridge credential rejected by NATS.
        let account_authz = start_failure(crate::AppError::Remote {
            status: 401,
            code: Some("unauthorized".to_string()),
            message: "A valid session token is required.".to_string(),
        })
        .unwrap();
        assert_eq!(account_authz.phase, RemotePhase::Failed);
        assert_eq!(
            account_authz.reason,
            Some(RemoteFailureReason::AccountAuthorization)
        );

        // A desktop that never stored an account credential asks for the same
        // thing as a rejected one, and it is the error's *type* that says so:
        // pairing codes come from the platform, so there is nothing to retry
        // before the user signs in.
        let signed_out = start_failure(crate::AppError::NotSignedIn).unwrap();
        assert_eq!(signed_out.phase, RemotePhase::Failed);
        assert_eq!(
            signed_out.reason,
            Some(RemoteFailureReason::AccountAuthorization)
        );

        // A server error is retryable.
        let server = start_failure(crate::AppError::Remote {
            status: 500,
            code: Some("rate_limited".to_string()),
            message: "busy".to_string(),
        })
        .unwrap();
        assert_eq!(server.phase, RemotePhase::Reconnecting);
        assert_eq!(server.reason, Some(RemoteFailureReason::RemoteServer));

        // An uncategorized local failure still propagates as Err.
        assert!(start_failure(crate::AppError::Message("local".to_string())).is_err());

        *SUPERVISOR.last_error_code.lock().unwrap() = None;
    }
}

#[cfg(test)]
mod runtime_tests {
    use super::test_support::{
        await_publish, await_publish_matching, declare_features, fake_state, init_store, jwt,
        nats_connect, nats_connect_once, now_secs, pairing_creds, sign_in, unique, FakeNats,
        HomeGuard, MockPlatform,
    };
    use super::*;
    use serde_json::json;
    use std::time::Duration;

    #[test]
    fn web_client_is_limited_to_the_test_platform() {
        assert!(web_client_enabled_for_platform(
            crate::future_platform::TEST_PLATFORM_URL
        ));
        assert!(!web_client_enabled_for_platform(
            crate::future_platform::PRODUCTION_PLATFORM_URL
        ));
        assert!(!web_client_enabled_for_platform(
            "https://custom.example.com"
        ));
    }

    /// Credentials for one device, for tests that only need it paired (no
    /// bridge behind it).
    pub(super) fn test_creds_for_multi_device(pair_id: &str) -> pairing::PairingCreds {
        test_creds(pair_id, "nats://127.0.0.1:1", 3600)
    }

    pub(super) fn test_creds(
        pair_id: &str,
        nats_url: &str,
        expires_in: i64,
    ) -> pairing::PairingCreds {
        let key_pair = nkeys::KeyPair::new_user();
        pairing::PairingCreds {
            handshake_version: 2,
            secure: Some(secure::PairingIdentity::new(now_secs() + 300).unwrap()),
            pair_id: pair_id.to_string(),
            desktop_id: format!("desktop_{}", unique("rt")),
            nkey_seed: key_pair.seed().unwrap().to_string(),
            user_jwt: jwt(now_secs() + expires_in),
            nats_url: nats_url.to_string(),
            nats_ws_url: nats_url.replace("nats://", "ws://"),
            jwt_expires_at: now_secs() + expires_in,
            control_url: None,
        }
    }

    /// Install the state; tests MUST clean up via `stop()` so the next
    /// serialized test starts clean. Poison-tolerant: one test's failure must
    /// not cascade into every later lock.
    fn install_state(state: RemoteState) {
        let pair_id = state.pair_id.clone();
        let previous = SUPERVISOR
            .state
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .replace(state);
        assert!(previous.is_none(), "previous test leaked SUPERVISOR.state");
        // The fixture is a machine with one device, so it is the controller.
        SUPERVISOR.restore_active(Some(pair_id));
    }

    /// Wait until WEB_PORT is bindable again (stop() aborts the web task
    /// asynchronously — the socket lingers briefly).
    async fn wait_for_web_port_free() {
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        loop {
            if let Ok(listener) = std::net::TcpListener::bind(("0.0.0.0", WEB_PORT)) {
                drop(listener);
                return;
            }
            assert!(std::time::Instant::now() < deadline, "web port never freed");
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    #[test]
    fn status_of_stopped_bridge_surfaces_last_error_and_pairing() {
        let _home = HomeGuard::new("remote-stopped");
        assert!(SUPERVISOR.state.lock().unwrap().is_none());
        let current = status();
        assert!(!matches!(current.phase, RemotePhase::Ready));
        assert_eq!(current.reason, None);
        assert_eq!(current.pair_id, "");

        pairing::save_creds(&test_creds("pair_stopped", "nats://127.0.0.1:1", 3600)).unwrap();
        *SUPERVISOR.last_error_code.lock().unwrap() = Some("revoked".to_string());
        let current = status();
        assert_eq!(current.reason, Some(RemoteFailureReason::CredentialRevoked));
        assert_eq!(current.pair_id, "pair_stopped");

        // A transient first connect failure starts the process-lifetime retry
        // worker before `SUPERVISOR.state` exists. It is reconnecting, not a final error.
        *SUPERVISOR.last_error_code.lock().unwrap() = Some("network".to_string());
        SUPERVISOR.start_requested.store(true, Ordering::Release);
        SUPERVISOR
            .start_retry_running
            .store(true, Ordering::Release);
        let current = status();
        assert!(matches!(current.phase, RemotePhase::Reconnecting));
        assert_eq!(current.reason, Some(RemoteFailureReason::Network));

        SUPERVISOR
            .start_retry_running
            .store(false, Ordering::Release);
        SUPERVISOR.start_requested.store(false, Ordering::Release);
        *SUPERVISOR.last_error_code.lock().unwrap() = None;
        pairing::clear_creds().unwrap();
    }

    #[test]
    fn stop_without_state_is_an_empty_noop() {
        let _home = HomeGuard::new("remote-stop-empty");
        let stopped = stop();
        assert!(!matches!(stopped.phase, RemotePhase::Ready));
    }

    #[tokio::test]
    async fn graceful_stop_clears_the_bridge_before_sending_presence() {
        let _home = HomeGuard::new("remote-graceful-stop");
        let nats = FakeNats::start().await;
        install_state(fake_state(&nats, "pair_graceful_stop").await);
        SUPERVISOR.start_requested.store(true, Ordering::Release);

        let stopped = stop_gracefully("user_disconnect").await;

        assert!(matches!(stopped.phase, RemotePhase::Stopped));
        assert!(SUPERVISOR.state.lock().unwrap().is_none());
        assert!(!SUPERVISOR.start_requested.load(Ordering::Acquire));
    }

    #[tokio::test]
    async fn status_of_running_bridge_reports_health_and_pairing_code() {
        let _home = HomeGuard::new("remote-status");
        let nats = FakeNats::start().await;
        let mut state = fake_state(&nats, "pair_status").await;
        state.web_task = Some(SUPERVISOR.spawn_fixture_task(std::future::pending()));
        state.web_url = Some("http://localhost:8022".to_string());
        state.pairing_code = Some("code-123".to_string());
        state.pairing_code_expires_at = Some(now_secs() + 600);
        state.pairing_confirmed = Arc::new(AtomicBool::new(false));
        install_state(state);

        // Unconfirmed + fresh code → the code is re-exposed.
        let current = status();
        assert!(matches!(current.phase, RemotePhase::Ready));
        assert_eq!(current.pairing_code.as_deref(), Some("code-123"));
        assert_eq!(current.reason, None);

        // Expired code → hidden even while unconfirmed.
        {
            let mut guard = SUPERVISOR.state.lock().unwrap();
            let state = guard.as_mut().unwrap();
            state.pairing_code_expires_at = Some(now_secs() - 1);
        }
        assert_eq!(status().pairing_code, None);

        // Confirmed → hidden regardless of freshness.
        {
            let mut guard = SUPERVISOR.state.lock().unwrap();
            let state = guard.as_mut().unwrap();
            state.pairing_code_expires_at = Some(now_secs() + 600);
            state.pairing_confirmed.store(true, Ordering::Release);
        }
        assert_eq!(status().pairing_code, None);

        // A dead critical task first enters transparent automatic reconnect.
        {
            let mut guard = SUPERVISOR.state.lock().unwrap();
            let state = guard.as_mut().unwrap();
            state.cmd_task.abort();
            state.cmd_task = SUPERVISOR.spawn_fixture_task(async {});
        }
        SUPERVISOR.start_requested.store(true, Ordering::Release);
        SUPERVISOR
            .runtime_reconnect_attempts
            .store(0, Ordering::Release);
        tokio::time::sleep(Duration::from_millis(20)).await;
        let current = status();
        assert!(!matches!(current.phase, RemotePhase::Ready));
        assert!(matches!(current.phase, RemotePhase::Reconnecting));
        assert_eq!(
            current.reason,
            Some(RemoteFailureReason::GenerationUnhealthy)
        );

        // Only after the bounded automatic budget is exhausted does the UI
        // receive an actionable reconnect-required error.
        SUPERVISOR
            .runtime_reconnect_attempts
            .store(MAX_RUNTIME_RECONNECT_ATTEMPTS, Ordering::Release);
        let current = status();
        assert!(!matches!(current.phase, RemotePhase::Reconnecting));
        assert_eq!(
            current.reason,
            Some(RemoteFailureReason::GenerationUnhealthy)
        );

        // Authorization/configuration failures are terminal for this
        // generation: do not spend the reconnect budget on the same invalid
        // service configuration.
        {
            let guard = SUPERVISOR.state.lock().unwrap();
            let state = guard.as_ref().unwrap();
            state
                .nats_health
                .service_config_error
                .store(true, Ordering::Release);
        }
        SUPERVISOR
            .runtime_reconnect_attempts
            .store(0, Ordering::Release);
        let current = status();
        assert!(!matches!(current.phase, RemotePhase::Ready));
        assert!(!matches!(current.phase, RemotePhase::Reconnecting));
        assert_eq!(
            current.reason,
            Some(RemoteFailureReason::ServiceAuthorization)
        );
        SUPERVISOR
            .state
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .nats_health
            .service_config_error
            .store(false, Ordering::Release);

        // The reconnect-path authorization shape must never leave the UI on
        // "connected" merely because the old generation still owns SUPERVISOR.state.
        {
            let guard = SUPERVISOR.state.lock().unwrap();
            guard
                .as_ref()
                .unwrap()
                .nats_health
                .handle_event(&async_nats::Event::ClientError(
                    async_nats::ClientError::Other("authorization violation".to_string()),
                ));
        }
        let current = status();
        assert!(!matches!(current.phase, RemotePhase::Ready));
        assert!(matches!(current.phase, RemotePhase::Reconnecting));
        {
            let guard = SUPERVISOR.state.lock().unwrap();
            guard
                .as_ref()
                .unwrap()
                .nats_health
                .handle_event(&async_nats::Event::Connected);
        }

        // The optional Web listener retries silently, then becomes actionable
        // only after its own reconnect budget is exhausted. The fixture's slot
        // is the one that holds the process-wide listener, as a started slot
        // records itself.
        {
            let mut guard = SUPERVISOR.state.lock().unwrap();
            let state = guard.as_mut().unwrap();
            state.web_task = None;
            state.cmd_task = SUPERVISOR.spawn_fixture_task(std::future::pending());
        }
        let pair_id = SUPERVISOR.active_pair().expect("fixture controller");
        *SUPERVISOR.web_owner.lock().unwrap() = Some(pair_id);
        assert_eq!(status().warning_code, None);
        SUPERVISOR
            .web_reconnect_attempts
            .store(MAX_WEB_RECONNECT_ATTEMPTS, Ordering::Release);
        assert_eq!(status().warning_code.as_deref(), Some("web_bind"));

        let stopped = stop();
        assert!(!matches!(stopped.phase, RemotePhase::Ready));
        assert!(SUPERVISOR.state.lock().unwrap().is_none());
    }

    #[tokio::test]
    async fn publish_event_mirrors_caps_and_reports_drops() {
        let _home = HomeGuard::new("remote-publish");
        let nats = FakeNats::start().await;
        let state = fake_state(&nats, "pair_pub").await;
        install_state(state);
        let mut tap = nats.tap();

        publish_event(
            "sess-a",
            "text_chunk",
            r#"{"text":"hi"}"#,
            "run-1",
            3,
            1,
            "e1",
            "",
            -1,
            7,
        );
        let published =
            await_publish(&mut tap, "p.pair_pub.evt.sess-a", Duration::from_secs(5)).await;
        let body = published.json();
        assert_eq!(body["type"], json!("text_chunk"));
        assert_eq!(body["data"], json!(r#"{"text":"hi"}"#));
        assert_eq!(body["idx"], json!(3));

        // An oversized event keeps its identity but ships a truncation marker
        // (as the `data` string payload).
        let huge = "x".repeat(MAX_EVENT_BYTES + 10);
        publish_event(
            "sess-a",
            "tool_delta",
            &huge,
            "run-1",
            4,
            1,
            "e2",
            "",
            -1,
            8,
        );
        let published =
            await_publish(&mut tap, "p.pair_pub.evt.sess-a", Duration::from_secs(5)).await;
        let body = published.json();
        assert_eq!(body["type"], json!("tool_delta"));
        assert_eq!(body["idx"], json!(4));
        let data = body["data"].as_str().expect("truncated marker is a string");
        assert!(data.contains("\"_truncated\":true"), "got: {data}");

        // Snapshots ride the same channel as run_snapshot events.
        publish_snapshot(
            "sess-a",
            "run-1",
            9,
            &[crate::agent_proto::ProjectedRunEvent {
                r#type: "text_chunk".to_string(),
                data: r#"{"text":"folded"}"#.to_string(),
                idx: 2,
                payload: None,
            }],
            7,
        );
        let published =
            await_publish(&mut tap, "p.pair_pub.evt.sess-a", Duration::from_secs(5)).await;
        let body = published.json();
        assert_eq!(body["type"], json!("run_snapshot"));
        let data: serde_json::Value = serde_json::from_str(body["data"].as_str().unwrap()).unwrap();
        assert_eq!(data["snapshotCursor"], json!(9));
        assert_eq!(data["snapshotEvents"][0]["idx"], json!(2));

        stop();
    }

    /// The live lane's half of the lean feed. The unit tests pin the rewrite
    /// rules; this pins that `publish_event` actually consults the declared
    /// flag, drops the content events before they reach the queue, and forwards
    /// a tool result the client can still read an outcome from.
    ///
    /// `HomeGuard` holds `TEST_HOME_LOCK`, so no sibling test can publish while
    /// the process-wide flag is flipped.
    #[tokio::test]
    async fn lean_lane_drops_streamed_content_and_keeps_the_tool_outcome() {
        let _home = HomeGuard::new("remote-lean");
        let nats = FakeNats::start().await;
        install_state(fake_state(&nats, "pair_lean").await);
        let mut tap = nats.tap();

        // The declaration belongs to this connection; the publisher reads it
        // from the pairing that is in control.
        SUPERVISOR
            .lean_events("pair_lean")
            .store(true, Ordering::Release);
        publish_event(
            "sess-lean",
            "thinking_delta",
            r#"{"text":"long reasoning","block_id":"b"}"#,
            "r",
            1,
            0,
            "e1",
            "",
            -1,
            0,
        );
        publish_event(
            "sess-lean",
            "tool_delta",
            r#"{"text":"{\"path\"","tool_id":"c1"}"#,
            "r",
            2,
            0,
            "e2",
            "",
            -1,
            0,
        );
        publish_event(
            "sess-lean",
            "tool_start",
            r#"{"tool_args":{"command":"ls -la"}}"#,
            "r",
            3,
            0,
            "e3",
            "",
            -1,
            0,
        );
        publish_event(
            "sess-lean",
            "tool_end",
            r#"{"text":"boom\n[exit: 3]","exit_code":3,"tool_id":"c1"}"#,
            "r",
            4,
            0,
            "e4",
            "",
            -1,
            0,
        );
        // The argument-less `input` phase never reaches the lane either.
        publish_event(
            "sess-lean",
            "tool_start",
            r#"{"phase":"input","tool_name":"read","tool_id":"c2","tool_args":""}"#,
            "r",
            5,
            0,
            "e5",
            "",
            -1,
            0,
        );
        SUPERVISOR
            .lean_events("pair_lean")
            .store(false, Ordering::Release);

        // The first thing on the lane is the tool start, not the reasoning that
        // was published before it.
        let first = await_publish(
            &mut tap,
            "p.pair_lean.evt.sess-lean",
            Duration::from_secs(5),
        )
        .await;
        assert_eq!(first.json()["type"], json!("tool_start"));
        assert_eq!(first.json()["idx"], json!(3));

        let second = await_publish(
            &mut tap,
            "p.pair_lean.evt.sess-lean",
            Duration::from_secs(5),
        )
        .await;
        let body = second.json();
        assert_eq!(body["type"], json!("tool_end"));
        assert_eq!(body["idx"], json!(4));
        let data: serde_json::Value =
            serde_json::from_str(body["data"].as_str().expect("data is a string")).unwrap();
        assert!(data.get("text").is_none(), "captured output is dropped");
        assert_eq!(data["exit_code"], json!(3), "outcome survives");
        assert_eq!(data["tool_id"], json!("c1"), "identity survives");

        // The envelope is trimmed to the keys a subscriber reads. `eventId` is
        // the largest of the dropped ones (~96 B: `{session}:{run}:{epoch}:{idx}`).
        assert_eq!(body["runId"], json!("r"));
        assert_eq!(body["eventId"], json!(null), "eventId is dropped");
        // `sessionId` is the one kept key that is not about what a subscriber
        // *reads*: the host publishes this same body to the `state.events` mirror
        // (`publish_batch` sends one payload to both subjects), and that subject
        // names the host rather than a conversation — so on that lane the envelope
        // is the only place the conversation can be named at all. A client that
        // reads the mirror to keep the rest of its list fresh files each push by
        // this field; without it every one of them would be dropped.
        assert_eq!(
            body["sessionId"],
            json!("sess-lean"),
            "the conversation stays"
        );
        assert_eq!(body["timestamp"], json!(null), "timestamp is dropped");
        assert_eq!(body["schemaVersion"], json!(null));
        assert_eq!(body["epoch"], json!(null));
        assert_eq!(body["sessionIdx"], json!(null));
        assert_eq!(body["runSequence"], json!(null));
        assert!(body["data"].is_string(), "the payload itself stays");

        // Nothing trails the dropped events onto the lane.
        super::test_support::assert_no_publish(
            &mut tap,
            "p.pair_lean.evt.sess-lean",
            Duration::from_millis(200),
        )
        .await;

        stop();
    }

    /// The same events, with no declaration: the lane keeps its legacy shape, so
    /// an older client on this desktop is served exactly what it was before.
    #[tokio::test]
    async fn a_client_that_did_not_declare_keeps_the_full_lane() {
        let _home = HomeGuard::new("remote-lean-legacy");
        let nats = FakeNats::start().await;
        install_state(fake_state(&nats, "pair_legacy").await);
        let mut tap = nats.tap();

        publish_event(
            "sess-legacy",
            "thinking_delta",
            r#"{"text":"kept"}"#,
            "r",
            1,
            0,
            "e1",
            "",
            -1,
            0,
        );
        publish_event(
            "sess-legacy",
            "tool_end",
            r#"{"text":"boom\n[exit: 3]"}"#,
            "r",
            2,
            0,
            "e2",
            "",
            -1,
            0,
        );

        let first = await_publish(
            &mut tap,
            "p.pair_legacy.evt.sess-legacy",
            Duration::from_secs(5),
        )
        .await;
        assert_eq!(first.json()["type"], json!("thinking_delta"));
        let second = await_publish(
            &mut tap,
            "p.pair_legacy.evt.sess-legacy",
            Duration::from_secs(5),
        )
        .await;
        let body = second.json();
        assert_eq!(body["type"], json!("tool_end"));
        let data: serde_json::Value = serde_json::from_str(body["data"].as_str().unwrap()).unwrap();
        assert_eq!(
            data["text"],
            json!("boom\n[exit: 3]"),
            "the legacy lane must keep the footer the client parses"
        );

        // And the envelope keeps every legacy key: a client that did not declare
        // lean is served the byte-identical body it always was.
        assert_eq!(body["sessionId"], json!("sess-legacy"));
        assert_eq!(body["eventId"], json!("e2"));
        assert_eq!(body["schemaVersion"], json!(2));
        assert_eq!(body["sessionIdx"], json!(-1));
        assert_eq!(body["runSequence"], json!(0));
        assert!(body.get("timestamp").is_some(), "timestamp survives");
        assert!(body.get("epoch").is_some(), "epoch survives");

        stop();
    }

    #[tokio::test]
    async fn publish_event_reports_offline_and_full_queue_drops() {
        let _home = HomeGuard::new("remote-drops");
        // No state at all → immediate return.
        publish_event("sess", "t", "{}", "r", 0, 0, "e", "", -1, 0);

        let nats = FakeNats::start().await;
        let state = fake_state(&nats, "pair_drops").await;
        install_state(state);
        // Kill the broker and wait for the client to notice.
        nats.kill();
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        loop {
            let disconnected = {
                let guard = SUPERVISOR.state.lock().unwrap();
                let state = guard.as_ref().unwrap();
                state.client.connection_state() != async_nats::connection::State::Connected
            };
            if disconnected {
                break;
            }
            assert!(std::time::Instant::now() < deadline, "client never noticed");
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        // Offline → the drop-reporting path (episode start).
        publish_event("sess", "t", "{}", "r", 0, 0, "e", "", -1, 0);

        // Full/closed queue → the queue-full drop path. Reset this runtime's
        // cross-generation counters so this drop is the first in its episode.
        {
            let mut guard = SUPERVISOR.state.lock().unwrap();
            let state = guard.as_mut().unwrap();
            state.drop_counters.reset_for_test();
            let (tx, rx) = tokio::sync::mpsc::channel(1);
            drop(rx);
            state.event_tx = tx;
        }
        // Fake a connected client so the queue path (not the offline path) runs.
        let nats2 = FakeNats::start().await;
        let client2 = nats_connect(&nats2).await;
        {
            let mut guard = SUPERVISOR.state.lock().unwrap();
            guard.as_mut().unwrap().client = client2;
        }
        publish_event("sess", "t", "{}", "r", 1, 0, "e", "", -1, 0);

        // Recovery: a successful enqueue after the episode reports once.
        let (tx, rx) = tokio::sync::mpsc::channel(EVENT_QUEUE_CAPACITY);
        let drain = spawn_event_publisher(
            SUPERVISOR
                .state
                .lock()
                .unwrap()
                .as_ref()
                .unwrap()
                .client
                .clone(),
            rx,
        );
        {
            let mut guard = SUPERVISOR.state.lock().unwrap();
            guard.as_mut().unwrap().event_tx = tx;
        }
        let mut tap = nats2.tap();
        publish_event("sess", "t", "{}", "r", 2, 0, "e", "", -1, 0);
        await_publish(&mut tap, "p.pair_drops.evt.sess", Duration::from_secs(5)).await;

        drain.abort();
        stop();
    }

    #[tokio::test]
    async fn event_publisher_flushes_in_order_and_reports_failures() {
        let _home = HomeGuard::new("remote-drain");
        let nats = FakeNats::start().await;
        let client = nats_connect_once(&nats).await;
        let (tx, rx) = tokio::sync::mpsc::channel(EVENT_QUEUE_CAPACITY);
        let mut drain = spawn_event_publisher(client.clone(), rx);
        let mut tap = nats.tap();

        for index in 0..3 {
            tx.send(EventPublish {
                status_subject: None,
                subject: format!("p.pair_drain.evt.sess.{index}"),
                payload: format!("{{\"n\":{index}}}").into_bytes(),
            })
            .await
            .unwrap();
        }
        for index in 0..3 {
            let published = await_publish(
                &mut tap,
                &format!("p.pair_drain.evt.sess.{index}"),
                Duration::from_secs(5),
            )
            .await;
            assert_eq!(published.json()["n"], json!(index));
        }

        // A background status event remains available without subscribing to
        // every token, while legacy clients retain their original full topic.
        tx.send(EventPublish {
            status_subject: Some("p.pair_drain.state.events".to_string()),
            subject: "p.pair_drain.evt.background".to_string(),
            payload: br#"{"type":"approval_request","sessionId":"background"}"#.to_vec(),
        })
        .await
        .unwrap();
        let status = await_publish(
            &mut tap,
            "p.pair_drain.state.events",
            Duration::from_secs(5),
        )
        .await;
        let legacy = await_publish(
            &mut tap,
            "p.pair_drain.evt.background",
            Duration::from_secs(5),
        )
        .await;
        assert_eq!(status.json(), legacy.json());

        // An event over the broker's max_payload cap is refused client-side;
        // the critical publisher exits so the generation supervisor rebuilds
        // it under the shared system-failure budget.
        tx.send(EventPublish {
            status_subject: None,
            subject: "p.pair_drain.evt.sess.huge".to_string(),
            payload: vec![b'x'; 9 * 1024 * 1024],
        })
        .await
        .unwrap();
        tokio::time::timeout(Duration::from_secs(5), &mut drain)
            .await
            .expect("publisher exits after a critical write failure")
            .expect("publisher not panicked");
        assert!(tx
            .send(EventPublish {
                status_subject: None,
                subject: "p.pair_drain.evt.sess.after".to_string(),
                payload: b"{}".to_vec(),
            })
            .await
            .is_err());
    }

    #[tokio::test]
    async fn unpair_revokes_and_clears() {
        let _home = HomeGuard::new("remote-unpair");
        // No creds, not running → plain success.
        unpair().await.unwrap();

        let platform = MockPlatform::start().await;
        sign_in(platform.url());
        let creds = test_creds("pair_unpair", "nats://127.0.0.1:1", 3600);
        pairing::save_creds(&creds).unwrap();
        platform.push("/client/v1/remote/pair/revoke", 200, json!({}));
        unpair().await.unwrap();
        assert!(pairing::load_creds().is_none());
        assert!(
            platform.requests().is_empty(),
            "local unpair must not await HTTP"
        );
        pairing::retry_pending_revokes().await.unwrap();
        assert_eq!(platform.requests().len(), 1);

        // A revoke failure keeps compensation without undoing local unpair.
        pairing::save_creds(&creds).unwrap();
        platform.push(
            "/client/v1/remote/pair/revoke",
            500,
            json!({ "error": "boom", "message": "no" }),
        );
        assert!(unpair().await.is_ok());
        assert!(pairing::load_creds().is_none());
        pairing::retry_pending_revokes().await.unwrap();
        platform.push("/client/v1/remote/pair/revoke", 200, json!({}));
        pairing::retry_pending_revokes().await.unwrap();
        assert_eq!(platform.requests().len(), 3);
    }

    #[tokio::test]
    async fn stop_during_readiness_cancels_candidate_before_a_new_start() {
        let _home = HomeGuard::new("remote-stop-readiness");
        init_store();
        let platform = MockPlatform::start().await;
        let nats = FakeNats::start().await;
        sign_in(platform.url());
        platform.respond_pair_code(nats.url());
        let (entered_tx, entered_rx) = tokio::sync::oneshot::channel();
        let (release_tx, release_rx) = tokio::sync::oneshot::channel();
        *SUPERVISOR.hooks.readiness_pause.lock().unwrap() = Some((entered_tx, release_rx));
        let opening = tokio::spawn(start(RemoteStartInput {}));
        tokio::time::timeout(Duration::from_secs(5), entered_rx)
            .await
            .unwrap()
            .unwrap();
        stop();
        let stopped = tokio::time::timeout(Duration::from_secs(1), opening)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert!(!matches!(stopped.phase, RemotePhase::Ready));
        assert!(SUPERVISOR.state.lock().unwrap().is_none());
        assert!(SUPERVISOR.bridge_shared.lock().unwrap().is_empty());
        assert!(
            release_tx.send(()).is_err(),
            "cancel must drop the readiness future"
        );
        platform.respond_pair_code(nats.url());
        assert_eq!(
            start(RemoteStartInput {}).await.unwrap().phase,
            RemotePhase::Ready
        );
        stop();
        wait_for_web_port_free().await;
    }

    /// The platform hands out the SAME `pair_id` again when a desktop that is
    /// already known asks for a second invitation, and any generation rebuild
    /// before the phone pairs re-runs `establish()` — which mints a fresh
    /// invitation (new NKey, new secure identity, new PSK) because the first
    /// one was never confirmed. The QR the desktop displays must always be the
    /// invitation the running bridge can authenticate: scanning it is the only
    /// pairing path the user has.
    #[tokio::test]
    async fn a_reissued_pair_code_for_the_same_pair_id_still_pairs() {
        let _home = HomeGuard::new("remote-reissue");
        init_store();
        let platform = MockPlatform::start().await;
        let nats = FakeNats::start().await;
        sign_in(platform.url());
        let pair_id = format!("pair_{}", unique("reissue"));
        platform.respond_pair_code_for(&pair_id, nats.url());

        let first = start(RemoteStartInput {}).await.expect("first start");
        assert!(matches!(first.phase, RemotePhase::Ready));
        assert_eq!(first.pair_id, pair_id);
        assert!(first.pairing_code.is_some());

        // Nobody scanned the first invitation; the bridge is rebuilt (the
        // supervisor's `GenerationWatch` does exactly this, via
        // `spawn_runtime_reconnect`) and mints a replacement for the same pair.
        let second_code = platform.respond_pair_code_for(&pair_id, nats.url());
        let second = super::start_once(true).await.expect("rebuilt generation");
        assert!(matches!(second.phase, RemotePhase::Ready));
        assert_eq!(second.pair_id, pair_id);
        let shown = second.pairing_code.clone().expect("an invitation is shown");
        assert!(
            shown.contains(&second_code),
            "the desktop must display the invitation it just minted: {shown}"
        );

        // The phone scans exactly what the desktop is showing.
        let mobile = nats_connect(&nats).await;
        let _channel = super::test_support::secure_pair(&mobile, &shown, &second.pair_id).await;

        stop();
        wait_for_web_port_free().await;
    }

    /// Control for the test above: when the rebuild gets a NEW pair_id the
    /// identity is rebuilt with it, so the displayed invitation pairs. This
    /// isolates "the same pair_id came back" as the trigger.
    #[tokio::test]
    async fn a_reissued_pair_code_for_a_new_pair_id_still_pairs() {
        let _home = HomeGuard::new("remote-reissue-new-pair");
        init_store();
        let platform = MockPlatform::start().await;
        let nats = FakeNats::start().await;
        sign_in(platform.url());
        platform.respond_pair_code(nats.url());

        let first = start(RemoteStartInput {}).await.expect("first start");
        assert!(matches!(first.phase, RemotePhase::Ready));

        platform.respond_pair_code(nats.url());
        let second = super::start_once(true).await.expect("rebuilt generation");
        assert!(matches!(second.phase, RemotePhase::Ready));
        let shown = second.pairing_code.clone().expect("an invitation is shown");

        let mobile = nats_connect(&nats).await;
        let _channel = super::test_support::secure_pair(&mobile, &shown, &second.pair_id).await;

        stop();
        wait_for_web_port_free().await;
    }

    /// `re_pair` runs the Remote page's two buttons in the order that works: the
    /// stored pairing is revoked *before* the invitation is minted. The order is
    /// the whole content of the helper, and it is observable here — a start-first
    /// sequence would take the scripted refresh, show no code at all, and leave the
    /// old credential on disk.
    #[tokio::test]
    async fn re_pair_revokes_the_stored_pairing_before_issuing_an_invitation() {
        let _home = HomeGuard::new("remote-re-pair");
        init_store();
        let platform = MockPlatform::start().await;
        let nats = FakeNats::start().await;
        sign_in(platform.url());
        let old = pairing_creds("pair_old");
        pairing::save_creds(&old).expect("store the existing pairing");
        platform.respond_refresh(nats.url());
        platform.respond_pair_code(nats.url());

        let status = super::re_pair().await.expect("re-pair");

        assert!(matches!(status.phase, RemotePhase::Ready), "{status:?}");
        let code = status.pairing_code.clone().expect("a fresh invitation");
        assert_ne!(status.pair_id, old.pair_id, "the pairing must be replaced");
        assert!(
            pairing::load_creds().is_none(),
            "the revoked credential must not survive the new invitation"
        );
        // The invitation must be the one the running bridge can authenticate: the
        // window's QR is the only pairing path the user has.
        let mobile = nats_connect(&nats).await;
        let _channel = super::test_support::secure_pair(&mobile, &code, &status.pair_id).await;

        stop();
        wait_for_web_port_free().await;
    }

    /// A desktop the platform already knows gets the same `pair_id` back for a new
    /// invitation, so replacing the pairing must deliver the old binding's revoke
    /// *before* it mints the replacement — a revoke queued behind the new invitation would
    /// cancel the code the command prints. The invitation also retires that queued
    /// revoke, so the compensation loop cannot cancel it on its next tick.
    #[tokio::test]
    async fn re_pair_settles_the_old_revoke_before_minting_the_same_pair_id() {
        let _home = HomeGuard::new("remote-re-pair-same-pair-id");
        init_store();
        let platform = MockPlatform::start().await;
        let nats = FakeNats::start().await;
        sign_in(platform.url());
        let pair_id = format!("pair_{}", unique("same"));
        pairing::save_creds(&test_creds(&pair_id, "nats://127.0.0.1:1", 3600)).unwrap();
        platform.push("/client/v1/remote/pair/revoke", 200, json!({}));
        platform.respond_pair_code_for(&pair_id, nats.url());

        let status = super::re_pair().await.expect("re-pair");
        assert_eq!(status.pair_id, pair_id, "the platform reused the pair id");
        assert!(matches!(status.phase, RemotePhase::Ready), "{status:?}");
        assert!(status.pairing_code.is_some());

        let paths: Vec<String> = platform
            .requests()
            .into_iter()
            .map(|(_, path, _)| path)
            .collect();
        let revoke = paths
            .iter()
            .position(|path| path.ends_with("/pair/revoke"))
            .unwrap_or_else(|| panic!("the saved pairing was never revoked: {paths:?}"));
        let minted = paths
            .iter()
            .position(|path| path.ends_with("/pair/code"))
            .expect("an invitation is minted");
        assert!(
            revoke < minted,
            "the old binding must be gone before its replacement exists: {paths:?}"
        );

        // Nothing is left for a later compensation tick to revoke.
        pairing::retry_pending_revokes().await.unwrap();
        assert_eq!(
            platform.requests().len(),
            2,
            "the replacement invitation must retire its own queued revoke: {:?}",
            platform.requests()
        );

        let code = status.pairing_code.clone().expect("a fresh invitation");
        let mobile = nats_connect(&nats).await;
        let _channel = super::test_support::secure_pair(&mobile, &code, &status.pair_id).await;

        stop();
        wait_for_web_port_free().await;
    }

    /// The compensation loop runs once per tick and outlives the unpair that
    /// queued a revoke. An invitation that takes over the same pair id supersedes
    /// that revoke — otherwise the loop would revoke the live invitation moments
    /// after the user was handed its code.
    #[tokio::test]
    async fn a_new_invitation_retires_a_queued_revoke_for_its_pair_id() {
        let _home = HomeGuard::new("remote-retire-queued-revoke");
        init_store();
        let platform = MockPlatform::start().await;
        let nats = FakeNats::start().await;
        sign_in(platform.url());
        let pair_id = format!("pair_{}", unique("retired"));
        pairing::queue_revoke(&test_creds(&pair_id, "nats://127.0.0.1:1", 3600)).unwrap();
        platform.respond_pair_code_for(&pair_id, nats.url());

        let status = start(RemoteStartInput {}).await.expect("start");
        assert_eq!(status.pair_id, pair_id);
        assert!(status.pairing_code.is_some());

        pairing::retry_pending_revokes().await.unwrap();
        assert!(
            platform
                .requests()
                .iter()
                .all(|(_, path, _)| !path.ends_with("/pair/revoke")),
            "the live invitation was revoked by the compensation loop: {:?}",
            platform.requests()
        );

        stop();
        wait_for_web_port_free().await;
    }

    #[tokio::test]
    async fn start_runs_the_full_bridge_and_stop_winds_it_down() {
        let _home = HomeGuard::new("remote-start");
        init_store();
        let platform = MockPlatform::start().await;
        let nats = FakeNats::start().await;
        sign_in(platform.url());
        platform.respond_pair_code(nats.url());

        let started = start(RemoteStartInput {}).await.expect("start");
        assert!(matches!(started.phase, RemotePhase::Ready));
        assert!(started.pairing_code.is_some());
        assert_eq!(started.web_url.as_deref(), Some("http://localhost:8022"));
        assert_eq!(started.reason, None);

        let mut tap = nats.tap();
        // No session/catalog metadata leaves the endpoint before pairing.
        super::test_support::assert_no_publish(
            &mut tap,
            &format!("p.{}.presence", started.pair_id),
            Duration::from_millis(100),
        )
        .await;
        let mobile = nats_connect(&nats).await;
        let mut channel = super::test_support::secure_pair(
            &mobile,
            started.pairing_code.as_deref().unwrap(),
            &started.pair_id,
        )
        .await;
        // A broker can inject plaintext even after the real pair authenticated.
        // It must not reach the command dispatcher.
        let subject = format!("p.{}.cmd.list", started.pair_id);
        let forged = mobile
            .request(
                subject.clone(),
                serde_json::to_vec(&json!({"type":"get_presence"}))
                    .unwrap()
                    .into(),
            )
            .await
            .unwrap();
        let forged: serde_json::Value = serde_json::from_slice(&forged.payload).unwrap();
        assert_eq!(forged["success"], false);
        let request = channel
            .seal(
                &subject,
                &serde_json::to_vec(&json!({"type":"get_presence", "id":"secure-test"})).unwrap(),
            )
            .unwrap();
        let reply_context = future_remote_crypto::reply_context(&subject, &request).unwrap();
        let reply = mobile.request(subject, request.into()).await.unwrap();
        let reply: serde_json::Value =
            serde_json::from_slice(&channel.open(&reply_context, &reply.payload).unwrap()).unwrap();
        assert_eq!(reply["success"], true);
        // The presence heartbeat and both catalog snapshots now flow encrypted.
        //
        // Read the snapshots first. Each is published exactly once, on the tick
        // that detects it (there is no periodic re-send any more), while the
        // heartbeat repeats — and the await helpers *discard* what they drain
        // past. Waiting on the repeating subject first can therefore swallow the
        // one-shot snapshot and then wait for a second that never comes.
        await_publish(
            &mut tap,
            &format!("p.{}.state.sessions", started.pair_id),
            Duration::from_secs(5),
        )
        .await;
        await_publish(
            &mut tap,
            &format!("p.{}.state.workspaces", started.pair_id),
            Duration::from_secs(5),
        )
        .await;
        let mut presence_data = serde_json::Value::Null;
        await_publish_matching(
            &mut tap,
            &format!("p.{}.presence", started.pair_id),
            Duration::from_secs(5),
            |published| {
                presence_data = serde_json::from_slice(
                    &channel
                        .open(&published.subject, &published.payload)
                        .unwrap(),
                )
                .unwrap();
                presence_data["online"] == true
            },
        )
        .await;
        assert_eq!(presence_data["online"], json!(true));
        // An idle directory advertises its revision instead of re-sending.
        assert!(
            presence_data["catalogVersion"]["sessions"]
                .as_u64()
                .is_some_and(|revision| revision > 0),
            "the heartbeat must carry the catalog revision: {presence_data}"
        );

        // The event mirror is live.
        publish_event(
            "sess-live",
            "text_chunk",
            "{}",
            "run-1",
            1,
            0,
            "e",
            "",
            -1,
            0,
        );
        await_publish(
            &mut tap,
            &format!("p.{}.evt.sess-live", started.pair_id),
            Duration::from_secs(5),
        )
        .await;

        // The web client serves over HTTP.
        let mut stream = tokio::net::TcpStream::connect(("127.0.0.1", WEB_PORT))
            .await
            .expect("web server accepts");
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        stream
            .write_all(b"GET / HTTP/1.1\r\nConnection: close\r\n\r\n")
            .await
            .unwrap();
        let mut response = Vec::new();
        stream.read_to_end(&mut response).await.unwrap();
        let response = String::from_utf8_lossy(&response);
        assert!(response.starts_with("HTTP/1.1 200"), "got: {response}");
        assert!(response.contains("text/html"), "got: {response}");

        // Successful authenticated pairing consumes the invitation.
        let polled = status();
        assert!(polled.pairing_code.is_none());

        // stop() publishes offline presence and clears the state. Match on the
        // payload: the heartbeat publishes `online: true` to this same subject,
        // so a subject-only wait can return that heartbeat instead.
        stop();
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let offline = await_publish(
                    &mut tap,
                    &format!("p.{}.presence", started.pair_id),
                    Duration::from_secs(5),
                )
                .await;
                let data: serde_json::Value = serde_json::from_slice(
                    &channel.open(&offline.subject, &offline.payload).unwrap(),
                )
                .unwrap();
                if data["online"] == false {
                    break;
                }
            }
        })
        .await
        .expect("authenticated offline notice");
        assert!(SUPERVISOR.state.lock().unwrap().is_none());
        wait_for_web_port_free().await;
    }

    #[tokio::test]
    async fn start_retries_web_bind_before_reporting_failure() {
        let _home = HomeGuard::new("remote-web-bind");
        wait_for_web_port_free().await;
        let blocker =
            std::net::TcpListener::bind(("0.0.0.0", WEB_PORT)).expect("occupy the web port");
        let platform = MockPlatform::start().await;
        let nats = FakeNats::start().await;
        sign_in(platform.url());
        platform.respond_pair_code(nats.url());

        let started = start(RemoteStartInput {}).await.expect("start");
        assert!(matches!(started.phase, RemotePhase::Ready));
        assert_eq!(started.web_url, None);
        assert_eq!(started.web_lan_url, None);
        assert_eq!(started.warning_code.as_deref(), Some("web_bind"));
        assert_eq!(status().warning_code, None);
        SUPERVISOR
            .web_reconnect_attempts
            .store(MAX_WEB_RECONNECT_ATTEMPTS, Ordering::Release);
        assert_eq!(status().warning_code.as_deref(), Some("web_bind"));
        stop();
        drop(blocker);
    }

    #[tokio::test]
    async fn start_with_existing_credentials_refreshes_instead_of_pairing() {
        let _home = HomeGuard::new("remote-start-refresh");
        init_store();
        wait_for_web_port_free().await;
        let platform = MockPlatform::start().await;
        let nats = FakeNats::start().await;
        sign_in(platform.url());
        let creds = test_creds("pair_keep", nats.url(), 3600);
        pairing::save_creds(&creds).unwrap();
        platform.respond_refresh(nats.url());

        let started = start(RemoteStartInput {}).await.expect("start");
        assert!(matches!(started.phase, RemotePhase::Ready));
        assert_eq!(started.pairing_code, None, "existing pairing → no new code");
        assert!(pairing::load_creds().is_some(), "refreshed creds persisted");
        stop();
    }

    #[tokio::test]
    async fn start_replaces_revoked_and_legacy_credentials() {
        let _home = HomeGuard::new("remote-start-revoked");
        init_store();
        wait_for_web_port_free().await;
        let platform = MockPlatform::start().await;
        let nats = FakeNats::start().await;
        sign_in(platform.url());

        // Persisted v1 creds the server has revoked → fresh pairing code.
        let creds = test_creds("pair_revoked", nats.url(), 3600);
        pairing::save_creds(&creds).unwrap();
        platform.respond_refresh_revoked();
        platform.respond_pair_code(nats.url());
        let started = start(RemoteStartInput {}).await.expect("start");
        assert!(started.pairing_code.is_some());
        stop();
        wait_for_web_port_free().await;

        // Legacy (pre-handshake) creds are dropped and re-paired.
        let mut legacy = test_creds("pair_legacy", nats.url(), 3600);
        legacy.handshake_version = 0;
        pairing::save_creds(&legacy).unwrap();
        platform.respond_pair_code(nats.url());
        let started = start(RemoteStartInput {}).await.expect("start");
        assert!(started.pairing_code.is_some());
        stop();
    }

    #[tokio::test]
    async fn establish_surfaces_a_transient_refresh_failure() {
        let _home = HomeGuard::new("remote-establish-err");
        let platform = MockPlatform::start().await;
        let nats = FakeNats::start().await;
        sign_in(platform.url());
        let creds = test_creds("pair_estab", nats.url(), 3600);
        pairing::save_creds(&creds).unwrap();
        // A transient (non-revoked) refresh failure propagates to the caller
        // instead of minting a replacement pairing.
        platform.push(
            "/client/v1/remote/auth/token",
            500,
            json!({ "message": "busy" }),
        );
        let error = match super::supervisor::establish_slots().await {
            Ok(_) => panic!("a transient refresh failure must reach the caller"),
            Err(error) => error,
        };
        assert!(
            matches!(error, crate::AppError::Remote { status: 500, .. }),
            "expected a transient remote failure, got: {error}"
        );
        // The persisted credential is left untouched for the next attempt.
        assert_eq!(pairing::load_creds().unwrap().pair_id, "pair_estab");
    }

    #[tokio::test]
    async fn start_failures_map_to_localized_status_or_error() {
        let _home = HomeGuard::new("remote-start-fail");

        // Platform unreachable → categorized "network" status, not running.
        sign_in("http://127.0.0.1:9");
        let started = start(RemoteStartInput {})
            .await
            .expect("network maps to status");
        assert!(!matches!(started.phase, RemotePhase::Ready));
        assert_eq!(started.reason, Some(RemoteFailureReason::Network));
        // The code sticks for later status() polls.
        assert_eq!(status().reason, Some(RemoteFailureReason::Network));

        // Pairing issued but NATS unreachable → same categorized failure.
        let platform = MockPlatform::start().await;
        sign_in(platform.url());
        platform.respond_pair_code("nats://127.0.0.1:9");
        let started = start(RemoteStartInput {})
            .await
            .expect("nats failure maps to status");
        assert_eq!(started.reason, Some(RemoteFailureReason::Network));

        // An uncategorized local failure (the credential directory is
        // read-only, so clearing the legacy pairing fails) propagates as Err.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let nats = FakeNats::start().await;
            sign_in(platform.url());
            let mut legacy = test_creds("pair_dir", nats.url(), 3600);
            legacy.handshake_version = 0;
            pairing::save_creds(&legacy).unwrap();
            let config_dir = crate::home_dir().unwrap();
            let config_dir = std::path::Path::new(&config_dir).join(".future");
            let permissions = std::fs::metadata(&config_dir).unwrap().permissions();
            std::fs::set_permissions(&config_dir, std::fs::Permissions::from_mode(0o555)).unwrap();
            let result = start(RemoteStartInput {}).await;
            std::fs::set_permissions(&config_dir, permissions).unwrap();
            assert!(result.is_err());
        }
        *SUPERVISOR.last_error_code.lock().unwrap() = None;
    }

    #[test]
    fn bridge_shared_state_survives_generation_swaps_but_rotates_epoch() {
        let _home = HomeGuard::new("remote-shared-generation");
        SUPERVISOR.bridge_shared.lock().unwrap().clear();
        let creds = test_creds("pair_shared", "nats://127.0.0.1:4222", 3600);
        let first = shared_runtime(&creds, true, false);
        let same_credential_epoch = shared_runtime(&creds, true, false);
        assert!(first
            .reply_slots
            .shares_cache(&same_credential_epoch.reply_slots));
        assert!(Arc::ptr_eq(
            &first.pairing_confirmed,
            &same_credential_epoch.pairing_confirmed
        ));
        assert_eq!(
            first.bridge_instance_id,
            same_credential_epoch.bridge_instance_id
        );

        let rebuilt = shared_runtime(&creds, true, true);
        assert!(first.reply_slots.shares_cache(&rebuilt.reply_slots));
        assert!(Arc::ptr_eq(
            &first.pairing_confirmed,
            &rebuilt.pairing_confirmed
        ));
        assert_ne!(first.bridge_instance_id, rebuilt.bridge_instance_id);

        // A re-minted invitation for the same pair_id describes keys the cached
        // runtime cannot prove, so it must not inherit any of it — least of all
        // a `pairing_confirmed` that a previous, unrelated pairing set.
        let reissued = shared_runtime(
            &test_creds("pair_shared", "nats://127.0.0.1:4222", 3600),
            false,
            false,
        );
        assert!(!first.reply_slots.shares_cache(&reissued.reply_slots));
        assert!(!Arc::ptr_eq(
            &first.pairing_confirmed,
            &reissued.pairing_confirmed
        ));
        assert!(!reissued.pairing_confirmed.load(Ordering::Acquire));
        assert_ne!(first.bridge_instance_id, reissued.bridge_instance_id);
        SUPERVISOR.bridge_shared.lock().unwrap().clear();
    }

    #[tokio::test]
    async fn credential_refresh_swaps_generations() {
        let _home = HomeGuard::new("remote-refresh-swap");
        let platform = MockPlatform::start().await;
        let nats = FakeNats::start().await;
        let nats2 = FakeNats::start().await;
        sign_in(platform.url());
        // Expiring credential → refresh is due on the first tick.
        let creds = test_creds("pair_swap", nats.url(), 30);
        pairing::save_creds(&creds).unwrap();
        platform.respond_refresh(nats2.url());

        let state = fake_state(&nats, "pair_swap").await;
        let confirmed = state.pairing_confirmed.clone();
        install_state(state);
        let handshake = commands::new_handshake_state(
            creds.clone(),
            confirmed.clone(),
            "bridge_swap".to_string(),
        );
        let handle = spawn_credential_refresh(
            "pair_swap".to_string(),
            commands::new_reply_slots(),
            confirmed,
            handshake,
        );

        // The refresh runs, reconnects to the second server and swaps SUPERVISOR.state.
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        loop {
            let swapped = {
                let guard = SUPERVISOR.state.lock().unwrap();
                guard
                    .as_ref()
                    .map(|state| state.nats_url == nats2.url())
                    .unwrap_or(false)
            };
            if swapped {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "generation never swapped"
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        // The refreshed credential was persisted under the SUPERVISOR.state lock.
        assert_eq!(pairing::load_creds().unwrap().nats_url, nats2.url());
        assert!(platform
            .requests()
            .iter()
            .any(|(_, path, _)| path == "/client/v1/remote/auth/token"));

        handle.abort();
        stop();
    }

    #[tokio::test]
    async fn credential_refresh_logs_credential_save_failures() {
        let _home = HomeGuard::new("remote-refresh-savefail");
        let platform = MockPlatform::start().await;
        let nats = FakeNats::start().await;
        let nats2 = FakeNats::start().await;
        sign_in(platform.url());
        let creds = test_creds("pair_savefail", nats.url(), 30);
        pairing::save_creds(&creds).unwrap();
        // Two successful refreshes: the first hits the injected save failure,
        // the second proves the loop logged it and kept going.
        platform.respond_refresh(nats2.url());
        platform.respond_refresh(nats2.url());
        pairing::INJECT_SAVE_FAILURE.store(true, Ordering::Relaxed);

        let state = fake_state(&nats, "pair_savefail").await;
        let confirmed = state.pairing_confirmed.clone();
        install_state(state);
        let handshake = commands::new_handshake_state(
            creds.clone(),
            confirmed.clone(),
            "bridge_savefail".to_string(),
        );
        let handle = spawn_credential_refresh(
            "pair_savefail".to_string(),
            commands::new_reply_slots(),
            confirmed,
            handshake,
        );
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while platform.requests().len() < 2 {
            assert!(
                std::time::Instant::now() < deadline,
                "refresh never continued past the save failure"
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        handle.abort();
        stop();
    }

    #[tokio::test]
    async fn credential_refresh_stops_the_bridge_when_revoked() {
        let _home = HomeGuard::new("remote-refresh-revoked");
        let platform = MockPlatform::start().await;
        let nats = FakeNats::start().await;
        sign_in(platform.url());
        let creds = test_creds("pair_rev", nats.url(), 30);
        pairing::save_creds(&creds).unwrap();
        platform.respond_refresh_revoked();

        let state = fake_state(&nats, "pair_rev").await;
        let confirmed = state.pairing_confirmed.clone();
        install_state(state);
        let handshake =
            commands::new_handshake_state(creds, confirmed.clone(), "bridge_rev".into());
        let handle = spawn_credential_refresh(
            "pair_rev".to_string(),
            commands::new_reply_slots(),
            confirmed,
            handshake,
        );

        tokio::time::timeout(Duration::from_secs(5), handle)
            .await
            .expect("refresh task ends after revocation")
            .expect("refresh task not panicked");
        assert!(
            SUPERVISOR.state.lock().unwrap().is_none(),
            "bridge stopped itself"
        );
        assert_eq!(
            SUPERVISOR.last_error_code.lock().unwrap().as_deref(),
            Some("revoked")
        );
        assert!(pairing::load_creds().is_none());
        *SUPERVISOR.last_error_code.lock().unwrap() = None;
    }

    #[tokio::test]
    async fn credential_refresh_account_rejection_stops_without_deleting_pairing() {
        let _home = HomeGuard::new("remote-refresh-account-rejected");
        let platform = MockPlatform::start().await;
        let nats = FakeNats::start().await;
        sign_in(platform.url());
        let creds = test_creds("pair_account", nats.url(), 30);
        pairing::save_creds(&creds).unwrap();
        platform.push(
            "/client/v1/remote/auth/token",
            401,
            json!({"error":"unauthorized"}),
        );
        let state = fake_state(&nats, "pair_account").await;
        let confirmed = state.pairing_confirmed.clone();
        install_state(state);
        let handshake =
            commands::new_handshake_state(creds, confirmed.clone(), "bridge_account".into());
        let task = spawn_credential_refresh(
            "pair_account".into(),
            commands::new_reply_slots(),
            confirmed,
            handshake,
        );
        tokio::time::timeout(Duration::from_secs(5), task)
            .await
            .unwrap()
            .unwrap();
        assert!(SUPERVISOR.state.lock().unwrap().is_none());
        assert_eq!(
            SUPERVISOR.last_error_code.lock().unwrap().as_deref(),
            Some("account_authorization")
        );
        assert_eq!(pairing::load_creds().unwrap().pair_id, "pair_account");
        *SUPERVISOR.last_error_code.lock().unwrap() = None;
    }

    /// A loopback NATS endpoint that answers the CONNECT handshake with
    /// `-ERR 'Authorization Violation'`, i.e. what a broker sends when it
    /// rejects the credential the client presented.
    ///
    /// This is the missing fixture behind the refresh loop's
    /// `Err(AppError::RemoteAuthorization)` arm (`remote/transport.rs`): the
    /// in-process `FakeNats` authenticates nothing, and a real `nats-server`
    /// cannot be asked to reject one rotation and accept the next. The frame
    /// flow is the protocol's minimum: `INFO`, read until `CONNECT`, then the
    /// error. `async_nats`'s `ServerError::new` lowercases the text, so the
    /// canonical capitalisation the server uses is what has to be sent.
    struct NatsAuthRefuser {
        url: String,
        task: tokio::task::JoinHandle<()>,
    }

    impl NatsAuthRefuser {
        async fn start() -> Self {
            use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
            let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
                .await
                .expect("bind the refuser");
            let url = format!("nats://{}", listener.local_addr().expect("refuser address"));
            let task = tokio::spawn(async move {
                loop {
                    let Ok((stream, _)) = listener.accept().await else {
                        return;
                    };
                    tokio::spawn(async move {
                        let (read_half, mut write_half) = stream.into_split();
                        let info = concat!(
                            "INFO {\"server_id\":\"REFUSER\",\"server_name\":\"auth-refuser\",",
                            "\"version\":\"2.10.2\",\"host\":\"127.0.0.1\",\"port\":0,",
                            "\"max_payload\":1048576,\"proto\":1,\"headers\":true,",
                            "\"auth_required\":true,\"nonce\":\"dGVzdA==\"}\r\n"
                        );
                        if write_half.write_all(info.as_bytes()).await.is_err() {
                            return;
                        }
                        let mut reader = BufReader::new(read_half);
                        let mut line = String::new();
                        loop {
                            line.clear();
                            match reader.read_line(&mut line).await {
                                Ok(0) | Err(_) => return,
                                Ok(_) => {}
                            }
                            if line.starts_with("CONNECT") {
                                let _ = write_half
                                    .write_all(b"-ERR 'Authorization Violation'\r\n")
                                    .await;
                                let _ = write_half.flush().await;
                                return;
                            }
                        }
                    });
                }
            });
            NatsAuthRefuser { url, task }
        }
    }

    impl Drop for NatsAuthRefuser {
        fn drop(&mut self) {
            self.task.abort();
        }
    }

    /// A refreshed credential the broker rejects is terminal: the loop records
    /// `service_authorization` on the live bridge and returns instead of
    /// rotating the same invalid credential forever.
    ///
    /// The refresh request itself succeeds — the platform hands out a
    /// credential for the refused endpoint — which is exactly the production
    /// state after a server-side revocation the platform has not learned about.
    #[tokio::test]
    async fn credential_refresh_returns_when_the_broker_rejects_the_refreshed_credential() {
        let _home = HomeGuard::new("remote-refresh-auth-rejected");
        let platform = MockPlatform::start().await;
        let nats = FakeNats::start().await;
        let refuser = NatsAuthRefuser::start().await;
        sign_in(platform.url());
        let creds = test_creds("pair_auth_reject", nats.url(), 30);
        pairing::save_creds(&creds).unwrap();
        // The platform accepts the rotation; the broker it points at does not.
        platform.respond_refresh(&refuser.url);

        let state = fake_state(&nats, "pair_auth_reject").await;
        let confirmed = state.pairing_confirmed.clone();
        let health = state.nats_health.clone();
        install_state(state);
        let handshake =
            commands::new_handshake_state(creds, confirmed.clone(), "bridge_auth_reject".into());
        let task = spawn_credential_refresh(
            "pair_auth_reject".into(),
            commands::new_reply_slots(),
            confirmed,
            handshake,
        );

        tokio::time::timeout(Duration::from_secs(5), task)
            .await
            .expect("a rejected refreshed credential ends the refresh worker")
            .expect("the refresh worker did not panic");
        assert_eq!(
            SUPERVISOR.last_error_code.lock().unwrap().as_deref(),
            Some("service_authorization")
        );
        assert!(
            health.service_config_error.load(Ordering::Acquire),
            "the live bridge records that its own credential was refused"
        );
        assert!(
            SUPERVISOR.state.lock().unwrap().is_some(),
            "the worker reports the failure; stopping the bridge is the supervisor's"
        );
        *SUPERVISOR.last_error_code.lock().unwrap() = None;
        stop();
    }

    #[tokio::test]
    async fn credential_refresh_retries_on_transient_failures() {
        let _home = HomeGuard::new("remote-refresh-retry");
        let platform = MockPlatform::start().await;
        let nats = FakeNats::start().await;
        sign_in(platform.url());
        let creds = test_creds("pair_retry", nats.url(), 30);
        pairing::save_creds(&creds).unwrap();
        // Transient server error → retry; then refreshes pointing at a dead
        // broker → reconnect failure → retry (scripted twice so the loop is
        // observed completing a full failure iteration, not aborting mid-nap).
        platform.push(
            "/client/v1/remote/auth/token",
            500,
            json!({ "message": "busy" }),
        );
        platform.respond_refresh("nats://127.0.0.1:9");
        platform.respond_refresh("nats://127.0.0.1:9");

        let state = fake_state(&nats, "pair_retry").await;
        let confirmed = state.pairing_confirmed.clone();
        install_state(state);
        let handshake =
            commands::new_handshake_state(creds, confirmed.clone(), "bridge_retry".into());
        let handle = spawn_credential_refresh(
            "pair_retry".to_string(),
            commands::new_reply_slots(),
            confirmed,
            handshake,
        );

        // Both failure arms run and the loop keeps going: a third request
        // proves the dead-broker iteration ran its sleep+continue to the end.
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while platform.requests().len() < 3 {
            assert!(
                std::time::Instant::now() < deadline,
                "refresh never retried"
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        handle.abort();
        stop();
    }

    #[tokio::test]
    async fn credential_refresh_returns_when_the_world_moved_on() {
        let _home = HomeGuard::new("remote-refresh-guards");
        let nats = FakeNats::start().await;

        // No persisted credential → immediate return.
        let creds = test_creds("pair_none", nats.url(), 3600);
        let handshake = commands::new_handshake_state(
            creds.clone(),
            Arc::new(AtomicBool::new(true)),
            "bridge_none".into(),
        );
        let handle = spawn_credential_refresh(
            "pair_none".to_string(),
            commands::new_reply_slots(),
            Arc::new(AtomicBool::new(true)),
            handshake,
        );
        tokio::time::timeout(Duration::from_secs(5), handle)
            .await
            .expect("no-creds refresh returns")
            .unwrap();

        // A persisted credential for a DIFFERENT pairing → return.
        let other = test_creds("pair_other", nats.url(), 3600);
        pairing::save_creds(&other).unwrap();
        let handshake = commands::new_handshake_state(
            creds.clone(),
            Arc::new(AtomicBool::new(true)),
            "bridge_other".into(),
        );
        let handle = spawn_credential_refresh(
            "pair_none".to_string(),
            commands::new_reply_slots(),
            Arc::new(AtomicBool::new(true)),
            handshake,
        );
        tokio::time::timeout(Duration::from_secs(5), handle)
            .await
            .expect("mismatched-pairing refresh returns")
            .unwrap();
        pairing::clear_creds().unwrap();
    }

    #[tokio::test]
    async fn credential_refresh_aborts_when_state_generation_mismatches() {
        let _home = HomeGuard::new("remote-refresh-gen");
        let platform = MockPlatform::start().await;
        let nats = FakeNats::start().await;
        sign_in(platform.url());
        let creds = test_creds("pair_gen", nats.url(), 30);
        pairing::save_creds(&creds).unwrap();
        platform.respond_refresh(nats.url());

        // SUPERVISOR.state has the same pairing id but belongs to a newer bridge
        // generation → the old refresh worker must not overwrite it.
        let state = fake_state(&nats, "pair_gen").await;
        install_state(state);
        let handshake = commands::new_handshake_state(
            creds,
            Arc::new(AtomicBool::new(true)),
            "bridge_gen".into(),
        );
        let handle = spawn_credential_refresh(
            "pair_gen".to_string(),
            commands::new_reply_slots(),
            Arc::new(AtomicBool::new(true)),
            handshake,
        );
        tokio::time::timeout(Duration::from_secs(5), handle)
            .await
            .expect("mismatched-generation refresh returns")
            .unwrap();
        stop();
    }

    #[tokio::test]
    async fn credential_loop_does_not_compete_with_runtime_supervisor() {
        let _home = HomeGuard::new("remote-refresh-unhealthy");
        let platform = MockPlatform::start().await;
        let nats = FakeNats::start().await;
        sign_in(platform.url());
        // Far-future expiry: transport health belongs to RemoteSupervisor and
        // must not cause this credential-only loop to replace a generation.
        let creds = test_creds("pair_sick", nats.url(), 3600);
        pairing::save_creds(&creds).unwrap();

        let state = fake_state(&nats, "pair_sick").await;
        let confirmed = state.pairing_confirmed.clone();
        install_state(state);
        let handshake =
            commands::new_handshake_state(creds, confirmed.clone(), "bridge_sick".into());
        let handle = spawn_credential_refresh(
            "pair_sick".to_string(),
            commands::new_reply_slots(),
            confirmed,
            handshake,
        );
        // Killing the broker must not call the token endpoint from this task.
        nats.kill();
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(platform.requests().is_empty());
        handle.abort();
        stop();
    }

    #[tokio::test]
    async fn heartbeat_publishes_presence_and_catalog_snapshots() {
        let _home = HomeGuard::new("remote-heartbeat");
        init_store();
        let nats = FakeNats::start().await;
        let client = nats_connect_once(&nats).await;
        let pair = unique("pairhb");
        // The publisher serves the device in control; this fixture is it.
        SUPERVISOR.restore_active(Some(pair.clone()));

        // A thread (with a live run → streaming) and a user workspace.
        let session = unique("sesshb");
        let thread = crate::store::create_thread(crate::store::CreateThreadInput {
            mode: "chat".to_string(),
            title: Some("Heartbeat thread".to_string()),
            workspace_id: None,
            workspace_path: None,
            workspace_name: None,
            agent_session_id: Some(session.clone()),
        })
        .unwrap();
        crate::store::create_run(crate::store::CreateRunInput {
            id: None,
            thread_id: thread.id.clone(),
            trigger_message_id: None,
            model_provider: None,
            model_id: None,
        })
        .unwrap();
        let workspace_dir = std::env::temp_dir().join(unique("futureos-ws"));
        std::fs::create_dir_all(&workspace_dir).unwrap();
        crate::store::create_workspace(crate::store::CreateWorkspaceInput {
            name: Some("HB Workspace".to_string()),
            path: workspace_dir.to_string_lossy().to_string(),
            description: None,
            create_directory: None,
        })
        .unwrap();

        let mut tap = nats.tap();
        let handle = spawn_presence_heartbeat(client.clone(), pair.clone(), "bridge_hb".into());

        let presence = await_publish_matching(
            &mut tap,
            &format!("p.{pair}.presence"),
            Duration::from_secs(5),
            |published| published.json()["online"] == json!(true),
        )
        .await;
        assert_eq!(presence.json()["online"], json!(true));
        assert_eq!(presence.json()["pairId"], json!(pair));

        let sessions = await_publish(
            &mut tap,
            &format!("p.{pair}.state.sessions"),
            Duration::from_secs(5),
        )
        .await;
        let rows = sessions.json()["sessions"].as_array().unwrap().clone();
        let row = rows
            .iter()
            .find(|row| row["sessionId"] == json!(session))
            .unwrap();
        assert_eq!(row["streaming"], json!(true));
        assert_eq!(row["title"], json!("Heartbeat thread"));

        let workspaces = await_publish(
            &mut tap,
            &format!("p.{pair}.state.workspaces"),
            Duration::from_secs(5),
        )
        .await;
        let list = workspaces.json()["workspaces"].as_array().unwrap().clone();
        assert!(list.iter().any(|w| w["name"] == json!("HB Workspace")));

        // A catalog change is picked up by the signature check on a later tick.
        crate::store::rename_thread(crate::store::RenameThreadInput {
            thread_id: thread.id.clone(),
            title: "Renamed heartbeat".to_string(),
        })
        .unwrap();
        let mut saw_rename = false;
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while !saw_rename {
            let remaining = deadline.saturating_duration_since(std::time::Instant::now());
            assert!(!remaining.is_zero(), "rename never republished");
            let published = tokio::time::timeout(remaining, tap.recv())
                .await
                .expect("tap stays live")
                .expect("tap value");
            if published.subject == format!("p.{pair}.state.sessions")
                && published.json()["sessions"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|row| row["title"] == json!("Renamed heartbeat"))
            {
                saw_rename = true;
            }
        }

        // Publish failures (broker dead) are logged, not fatal.
        nats.kill();
        tokio::time::sleep(Duration::from_millis(100)).await;
        handle.abort();
        std::fs::remove_dir_all(&workspace_dir).ok();
    }

    /// One second of a streaming run, told to a client that declared
    /// `catalog_activity_v1`: the moved row in a few dozen bytes, never the
    /// whole session list.
    ///
    /// This is the catalog lane's entire reason to exist. A run rewrites its
    /// thread's activity timestamp about once a second, and before this lane
    /// that re-sent every session the host owns — measured at 94–173 MB per run
    /// on the three heaviest completed runs (a ~95 KB list, 232 rows, once a
    /// second), against the same runs' 0.26–1.11 MB live event lane.
    #[tokio::test]
    async fn a_declared_client_is_told_the_moved_row_not_the_whole_list() {
        let _home = HomeGuard::new("remote-catalog-activity");
        init_store();
        let nats = FakeNats::start().await;
        let client = nats_connect_once(&nats).await;
        let pair = unique("pairactivity");
        // The publisher serves the device in control; this fixture is it.
        SUPERVISOR.restore_active(Some(pair.clone()));
        // A directory with more than the one row that moves: the ratio this test
        // asserts is the whole point of the lane, and a single-conversation
        // fixture cannot show it (the packet has a fixed envelope either way).
        let mut target = String::new();
        for index in 0..20 {
            let thread = crate::store::create_thread(crate::store::CreateThreadInput {
                mode: "chat".to_string(),
                title: Some(format!("Streaming {index}")),
                workspace_id: None,
                workspace_path: None,
                workspace_name: None,
                agent_session_id: Some(unique("sessactivity")),
            })
            .expect("seed a conversation");
            if index == 0 {
                target = thread.id.clone();
            }
        }
        let target_session = crate::store::get_thread(&target)
            .expect("read the moving thread")
            .expect("the moving thread exists")
            .agent_session_id
            .expect("the fixture binds a session");

        let creds = pairing_creds(&pair);
        let _shared = shared_runtime(&creds, true, false);
        declare_features(&creds, &pair, &[CATALOG_ACTIVITY_FEATURE]);

        let handle =
            spawn_presence_heartbeat(client.clone(), pair.clone(), "bridge_activity".into());
        let mut tap = nats.tap();
        let sessions_subject = format!("p.{pair}.state.sessions");
        let activity_subject = format!("p.{pair}.state.activity");

        // The baseline is the whole list, as it has to be: it is what carries
        // the row set, the titles and the streaming flags.
        let baseline = await_publish(&mut tap, &sessions_subject, Duration::from_secs(5)).await;
        let baseline_bytes = baseline.payload.len();
        assert_eq!(
            baseline.json()["sessions"].as_array().map(Vec::len),
            Some(20)
        );
        assert!(
            baseline_bytes > 4_000,
            "the baseline must be a real snapshot, got {baseline_bytes} B"
        );

        // The writes start above every row's current activity, which is what a
        // run's own timestamps do (the row is the newest thing in the list as
        // soon as it speaks). Taken from the shipping recency vector rather than
        // from a clock: twenty conversations seeded in the same millisecond have
        // no meaningful order among themselves, so a fixture that leaned on one
        // would be asserting about SQLite's tie order.
        let mut at = crate::remote_host::catalog::sessions_catalog("")
            .expect("store readable")
            .activity
            .iter()
            .map(|(_, at)| *at)
            .max()
            .expect("the fixture seeded rows");

        // The first write lifts the row to the head of the list. The host's own
        // order is content — an ordinary single-machine list draws it — so that
        // one write is a whole list in *both* modes, and it is drained here so
        // the assertions below are about the steady state that follows.
        at += 1_000;
        crate::store::record_thread_message_activity(&target, at).expect("record activity");
        let overtake = await_publish(&mut tap, &sessions_subject, Duration::from_secs(5)).await;
        assert_eq!(
            overtake.json()["sessions"]
                .as_array()
                .and_then(|rows| rows.first())
                .and_then(|row| row["sessionId"].as_str()),
            Some(target_session.as_str()),
            "the write must move the row to the head of the list"
        );

        // Every write after it moves the value alone — exactly what the
        // observer's 1s throttle emits while text streams.
        at += 1_000;
        crate::store::record_thread_message_activity(&target, at).expect("record activity");

        let patch = await_publish(&mut tap, &activity_subject, Duration::from_secs(5)).await;
        let body = patch.json();
        assert_eq!(body["pairId"], json!(pair));
        let updates = body["activity"]
            .as_array()
            .expect("the packet carries the moved rows");
        assert_eq!(
            updates.len(),
            1,
            "only the row that moved may ride the packet: {body}"
        );
        assert_eq!(updates[0]["lastMessageAt"], json!(at));
        assert!(
            body["version"]["revision"].as_u64().unwrap_or(0) > 0,
            "a packet carries the revision its reader gates on: {body}"
        );
        // The saving, as bytes: a packet is the row that moved plus an
        // envelope, not the list.
        assert!(
            patch.payload.len() * 10 < baseline_bytes,
            "one moved row cost {} B against a {baseline_bytes} B snapshot",
            patch.payload.len()
        );

        // And the list itself was not re-sent. A second whole snapshot here is
        // precisely the traffic this capability removes, so its absence is
        // asserted rather than assumed.
        let quiet_until = std::time::Instant::now() + Duration::from_millis(300);
        loop {
            let remaining = quiet_until.saturating_duration_since(std::time::Instant::now());
            if remaining.is_zero() {
                break;
            }
            match tokio::time::timeout(remaining, tap.recv()).await {
                Ok(Ok(published)) => assert_ne!(
                    published.subject, sessions_subject,
                    "a declared client must not be re-sent the session list for activity"
                ),
                Ok(Err(_)) | Err(_) => break,
            }
        }

        handle.abort();
    }

    /// The other half of the same contract: a client that did *not* declare the
    /// capability keeps the whole snapshot on every change, because nothing else
    /// can tell it that a row moved. Silent degradation is the one outcome this
    /// lane may not have.
    #[tokio::test]
    async fn an_undeclared_client_still_receives_the_whole_list() {
        let _home = HomeGuard::new("remote-catalog-activity-legacy");
        init_store();
        let nats = FakeNats::start().await;
        let client = nats_connect_once(&nats).await;
        let pair = unique("pairlegacy");
        // The publisher serves the device in control; this fixture is it.
        SUPERVISOR.restore_active(Some(pair.clone()));
        let thread = crate::store::create_thread(crate::store::CreateThreadInput {
            mode: "chat".to_string(),
            title: Some("Streaming".to_string()),
            workspace_id: None,
            workspace_path: None,
            workspace_name: None,
            agent_session_id: Some(unique("sesslegacy")),
        })
        .expect("seed the streaming conversation");

        let creds = pairing_creds(&pair);
        let _shared = shared_runtime(&creds, true, false);
        // A declaration is authoritative and always written, so this is the
        // state an older client leaves behind (`build_transport` clears it, and
        // `secure_ready` never sets it).
        declare_features(&creds, &pair, &[]);

        let handle = spawn_presence_heartbeat(client.clone(), pair.clone(), "bridge_legacy".into());
        let mut tap = nats.tap();
        let sessions_subject = format!("p.{pair}.state.sessions");

        let first = await_publish(&mut tap, &sessions_subject, Duration::from_secs(5)).await;
        assert_eq!(first.json()["sessions"].as_array().map(Vec::len), Some(1));

        crate::store::record_thread_message_activity(&thread.id, 7_000).expect("record activity");

        // The whole list again, carrying the new timestamp: the only way an
        // older client can learn the row moved.
        let second = await_publish(&mut tap, &sessions_subject, Duration::from_secs(5)).await;
        assert_ne!(
            second.payload, first.payload,
            "the change must reach a client that cannot patch it"
        );
        assert_eq!(second.json()["sessions"][0]["lastMessageAt"], json!(7_000));

        handle.abort();
    }

    /// The idle-directory contract, which replaced the "re-send every unchanged
    /// snapshot every 20s" self-heal with a revision advertised on the presence
    /// heartbeat. Both halves are load-bearing: without the first an idle link
    /// still pays for a full snapshot, and without the second a dropped push is
    /// never noticed. A regression in either half breaks exactly one assertion
    /// here.
    #[tokio::test]
    async fn idle_catalog_advertises_a_revision_instead_of_resending() {
        let _home = HomeGuard::new("remote-idle-catalog");
        init_store();
        let nats = FakeNats::start().await;
        let client = nats_connect_once(&nats).await;
        let pair = unique("pairidle");
        // The publisher serves the device in control; this fixture is it.
        SUPERVISOR.restore_active(Some(pair.clone()));
        let session = unique("sessidle");
        // Exactly one thread and no runs: the snapshot signature is stable, so
        // any republication below is the timer this test exists to forbid.
        let thread = crate::store::create_thread(crate::store::CreateThreadInput {
            mode: "chat".to_string(),
            title: Some("Idle thread".to_string()),
            workspace_id: None,
            workspace_path: None,
            workspace_name: None,
            agent_session_id: Some(session.clone()),
        })
        .unwrap();

        let handle = spawn_presence_heartbeat(client.clone(), pair.clone(), "bridge_idle".into());
        let mut tap = nats.tap();
        await_publish(
            &mut tap,
            &format!("p.{pair}.state.sessions"),
            Duration::from_secs(5),
        )
        .await;

        // The heartbeat must carry the revision of the snapshot just published.
        // Draining until it appears makes the quiet window below a steady-state
        // measurement rather than a race with the first tick.
        let revision = await_publish_matching(
            &mut tap,
            &format!("p.{pair}.presence"),
            Duration::from_secs(5),
            |published| {
                published.json()["catalogVersion"]["sessions"]
                    .as_u64()
                    .unwrap_or(0)
                    > 0
            },
        )
        .await;
        let baseline = revision.json()["catalogVersion"]["sessions"]
            .as_u64()
            .unwrap();
        // The heartbeat is the *only* traffic an idle link now carries, so its
        // size is part of what this change promises. A regression that ships the
        // directory inside it would show up here rather than as a silent cost.
        let heartbeat_bytes = serde_json::to_vec(&revision.json()).unwrap().len();
        assert!(
            heartbeat_bytes < 512,
            "an idle heartbeat must stay small, got {heartbeat_bytes} B"
        );
        assert!(
            revision.json()["catalogVersion"]["workspaces"]
                .as_u64()
                .unwrap_or(0)
                > 0,
            "both domains share one revision envelope"
        );

        // Steady state: hundreds of catalog ticks (10ms in tests) with no change
        // must produce no second snapshot. The old timer's 20s tick had long
        // since fired by this point, so a reintroduced resend fails here.
        let quiet_until = std::time::Instant::now() + Duration::from_millis(400);
        loop {
            let remaining = quiet_until.saturating_duration_since(std::time::Instant::now());
            if remaining.is_zero() {
                break;
            }
            match tokio::time::timeout(remaining, tap.recv()).await {
                Ok(Ok(published)) => assert_ne!(
                    published.subject,
                    format!("p.{pair}.state.sessions"),
                    "an unchanged catalog must not be re-sent on a timer"
                ),
                // Closed tap or the quiet window elapsing ends the measurement.
                Ok(Err(_)) | Err(_) => break,
            }
        }

        // ...while a real change still moves the advertised revision, which is
        // the only thing a client needs to know it must pull.
        crate::store::rename_thread(crate::store::RenameThreadInput {
            thread_id: thread.id.clone(),
            title: "Renamed idle".to_string(),
        })
        .unwrap();
        let moved = await_publish_matching(
            &mut tap,
            &format!("p.{pair}.presence"),
            Duration::from_secs(5),
            |published| {
                published.json()["catalogVersion"]["sessions"]
                    .as_u64()
                    .unwrap_or(0)
                    > baseline
            },
        )
        .await;
        assert!(
            moved.json()["catalogVersion"]["sessions"].as_u64().unwrap() > baseline,
            "a catalog change must advance the advertised revision"
        );

        handle.abort();
    }

    #[tokio::test]
    async fn heartbeat_publish_failure_exits_for_generation_supervisor() {
        let _home = HomeGuard::new("remote-heartbeat-fail");
        init_store();
        let nats = FakeNats::start().await;
        let client = nats_connect_once(&nats).await;
        // Payloads beyond the broker's max_payload cap are refused client-side:
        // an oversized pair id inflates the presence/sessions payloads and an
        // oversized workspace name inflates the workspaces payload, so every
        // heartbeat publish fails deterministically — no broker kill timing.
        let pair = "p".repeat(9 * 1024 * 1024);
        SUPERVISOR.restore_active(Some(pair.clone()));
        let workspace_dir = std::env::temp_dir().join(unique("futureos-ws-hbf"));
        std::fs::create_dir_all(&workspace_dir).unwrap();
        crate::store::create_workspace(crate::store::CreateWorkspaceInput {
            name: Some("n".repeat(9 * 1024 * 1024)),
            path: workspace_dir.to_string_lossy().to_string(),
            description: None,
            create_directory: None,
        })
        .unwrap();

        let handle = spawn_presence_heartbeat(client.clone(), pair, "bridge_hbf".into());
        // A critical publisher failure ends the generation instead of logging
        // once per heartbeat forever.
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(handle.is_finished());
        std::fs::remove_dir_all(&workspace_dir).ok();
    }

    #[tokio::test]
    async fn web_server_serves_files_and_rejects_bad_requests() {
        let _home = HomeGuard::new("remote-web");
        // Point the handler at a fixture dir (the real web dir only ships
        // index.html; the content-type arms need .js/.css/other files).
        let dir = std::env::temp_dir().join(unique("futureos-web"));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("index.html"), "<html>hi</html>").unwrap();
        std::fs::write(dir.join("app.js"), "console.log(1)").unwrap();
        std::fs::write(dir.join("style.css"), "body{}").unwrap();
        std::fs::write(dir.join("logo.dat"), vec![0_u8; 8]).unwrap();

        async fn request(dir: &std::path::Path, raw: &str) -> String {
            let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
                .await
                .unwrap();
            let port = listener.local_addr().unwrap().port();
            let accept = tokio::spawn({
                let dir = dir.to_path_buf();
                async move {
                    let (mut stream, _) = listener.accept().await.unwrap();
                    handle_web_request(&mut stream, &dir).await;
                }
            });
            let mut client = tokio::net::TcpStream::connect(("127.0.0.1", port))
                .await
                .unwrap();
            use tokio::io::{AsyncReadExt, AsyncWriteExt};
            client.write_all(raw.as_bytes()).await.unwrap();
            let mut response = Vec::new();
            client.read_to_end(&mut response).await.unwrap();
            accept.await.unwrap();
            String::from_utf8_lossy(&response).to_string()
        }

        let response = request(&dir, "GET / HTTP/1.1\r\n\r\n").await;
        assert!(response.contains("200 OK"), "{response}");
        assert!(response.contains("text/html"), "{response}");
        assert!(response.contains("<html>hi</html>"), "{response}");

        let response = request(&dir, "GET /app.js HTTP/1.1\r\n\r\n").await;
        assert!(response.contains("application/javascript"), "{response}");

        let response = request(&dir, "GET /style.css HTTP/1.1\r\n\r\n").await;
        assert!(response.contains("text/css"), "{response}");

        let response = request(&dir, "GET /logo.dat HTTP/1.1\r\n\r\n").await;
        assert!(response.contains("application/octet-stream"), "{response}");

        let response = request(&dir, "GET /missing.txt HTTP/1.1\r\n\r\n").await;
        assert!(response.contains("404"), "{response}");

        // Release bundles do not have the source checkout. The root page is
        // embedded so it remains available when its on-disk directory is gone.
        let missing_dir = std::env::temp_dir().join(unique("futureos-web-missing"));
        let response = request(&missing_dir, "GET / HTTP/1.1\r\n\r\n").await;
        assert!(response.contains("200 OK"), "{response}");
        assert!(response.contains("Remote Control"), "{response}");

        let response = request(&dir, "GET /../secret HTTP/1.1\r\n\r\n").await;
        assert!(response.contains("403"), "{response}");

        for target in [
            r"\Windows\win.ini",
            r"\\server\share\file",
            "C:/Windows/win.ini",
            r"/C:\Windows\win.ini",
            "/file:stream",
        ] {
            let response = request(&dir, &format!("GET {target} HTTP/1.1\r\n\r\n")).await;
            assert!(response.contains("403"), "{target}: {response}");
        }

        // A request line without a path defaults to the index.
        let response = request(&dir, "GARBAGE\r\n\r\n").await;
        assert!(response.contains("200 OK"), "{response}");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn web_server_drops_silent_clients() {
        let _home = HomeGuard::new("remote-web-timeout");
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let port = listener.local_addr().unwrap().port();
        let dir = std::env::temp_dir().join(unique("futureos-webt"));
        std::fs::create_dir_all(&dir).unwrap();
        let accept = tokio::spawn({
            let dir = dir.clone();
            async move {
                let (mut stream, _) = listener.accept().await.unwrap();
                handle_web_request(&mut stream, &dir).await;
            }
        });
        let client = tokio::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .unwrap();
        // Never send a request: the read times out and the handler returns.
        tokio::time::timeout(Duration::from_secs(5), accept)
            .await
            .expect("handler returns on read timeout")
            .unwrap();
        drop(client);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn lan_ip_returns_an_ipv4_literal_or_none() {
        let _home = HomeGuard::new("remote-lan");
        // `lan_ip` may be None when the host has no routable probe address; when
        // it yields a literal it must be a valid IPv4 address.
        let ipv4 = lan_ip()
            .map(|ip| ip.parse::<std::net::Ipv4Addr>().is_ok())
            .unwrap_or(true);
        assert!(ipv4, "lan_ip yielded a non-IPv4 literal");
    }

    #[test]
    fn presence_payloads_are_built_from_the_store() {
        let _home = HomeGuard::new("remote-snapshots");
        init_store();

        // Empty store → empty but well-formed snapshots.
        let (payload, signature) = build_presence_snapshot("pair_x", "bridge_x");
        assert_eq!(payload["online"], json!(true));
        assert_eq!(payload["sessions"], json!([]));
        assert_eq!(payload["workspaces"], json!([]));
        assert_eq!(signature, "[][]");
        assert_eq!(payload, build_presence_payload("pair_x", "bridge_x"));

        let light = light_presence_payload("pair_x", "bridge_x");
        assert_eq!(light["online"], json!(true));
        assert!(light.get("sessions").is_none());

        // Threads without an agent session are skipped; workspaces are
        // filtered to user kind.
        let thread = crate::store::create_thread(crate::store::CreateThreadInput {
            mode: "chat".to_string(),
            title: Some("Snap".to_string()),
            workspace_id: None,
            workspace_path: None,
            workspace_name: None,
            agent_session_id: None,
        })
        .unwrap();
        let (payload, sig_without_session) =
            build_sessions_snapshot("pair_x").expect("store readable");
        assert_eq!(payload["sessions"], json!([]));
        // The full presence snapshot skips session-less threads the same way.
        let (payload, _) = build_presence_snapshot("pair_x", "bridge_x");
        assert_eq!(payload["sessions"], json!([]));
        crate::store::bind_thread_session_id(&thread.id, "sess-snap").unwrap();
        let (payload, sig_with_session) =
            build_sessions_snapshot("pair_x").expect("store readable");
        assert_eq!(payload["sessions"].as_array().unwrap().len(), 1);
        assert_ne!(sig_without_session, sig_with_session);

        // …and once the thread has a session, the full snapshot carries it.
        let (payload, signature) = build_presence_snapshot("pair_x", "bridge_x");
        let row = payload["sessions"]
            .as_array()
            .unwrap()
            .iter()
            .find(|row| row["sessionId"] == json!("sess-snap"))
            .expect("presence snapshot includes the session thread");
        assert_eq!(row["threadId"], json!(thread.id));
        assert_eq!(row["streaming"], json!(false));
        assert!(row["parentSessionId"].is_null());
        assert!(!signature.is_empty());

        // Lineage is carried by both initial presence and pushed snapshots;
        // changing only a parent must invalidate both publication signatures.
        crate::store::sync_thread_parent_session("sess-snap", "parent-snap").unwrap();
        let (payload, parent_signature) = build_presence_snapshot("pair_x", "bridge_x");
        assert_eq!(
            payload["sessions"][0]["parentSessionId"],
            json!("parent-snap")
        );
        assert_ne!(signature, parent_signature);
        let (payload, parent_signature) = build_sessions_snapshot("pair_x").unwrap();
        assert_eq!(
            payload["sessions"][0]["parentSessionId"],
            json!("parent-snap")
        );
        assert_ne!(sig_with_session, parent_signature);

        let workspace_dir = std::env::temp_dir().join(unique("futureos-ws2"));
        std::fs::create_dir_all(&workspace_dir).unwrap();
        crate::store::create_workspace(crate::store::CreateWorkspaceInput {
            name: Some("Snap WS".to_string()),
            path: workspace_dir.to_string_lossy().to_string(),
            description: None,
            create_directory: None,
        })
        .unwrap();
        let (payload, signature) = build_workspaces_snapshot().expect("store readable");
        assert!(payload["workspaces"]
            .as_array()
            .unwrap()
            .iter()
            .any(|w| w["name"] == json!("Snap WS")));
        assert!(!signature.is_empty());
        std::fs::remove_dir_all(&workspace_dir).ok();

        // The full presence snapshot carries the user-kind workspace too.
        let (payload, _) = build_presence_snapshot("pair_x", "bridge_x");
        assert!(payload["workspaces"]
            .as_array()
            .unwrap()
            .iter()
            .any(|w| w["name"] == json!("Snap WS")));
    }

    #[test]
    fn cap_event_data_only_truncates_beyond_the_budget() {
        let _home = HomeGuard::new("remote-cap");
        let small = cap_event_data("small");
        assert!(matches!(small, std::borrow::Cow::Borrowed(_)));
        let big_text = "x".repeat(MAX_EVENT_BYTES + 1);
        let big = cap_event_data(&big_text);
        assert!(big.contains("_truncated"));
        assert!(big.contains("full content is available via get_messages"));
    }

    #[test]
    fn oversized_live_tool_result_keeps_the_same_outcome_as_replay() {
        let data = json!({
            "type": "tool_end", "tool_id": "large-test", "tool_name": "shell",
            "exit_code": 1,
            "text": format!("{}\n[exit: 1]", "x".repeat(MAX_EVENT_BYTES + 1)),
        })
        .to_string();
        let live = build_event_body("s", "tool_end", &data, "r", 42, 1, "e", "", 42, 1);
        let live_data: serde_json::Value =
            serde_json::from_str(live["data"].as_str().unwrap()).unwrap();
        assert_eq!(live["idx"], 42);
        assert_eq!(live_data["tool_id"], "large-test");
        assert_eq!(live_data["tool_name"], "shell");
        assert_eq!(live_data["exit_code"], 1);
        assert!(live_data["text"].as_str().unwrap().ends_with("[exit: 1]"));
        assert!(serde_json::to_vec(&live).unwrap().len() < MAX_EVENT_BYTES);

        let replay = crate::remote_host::business::paginate_events(
            json!({"runId": "r", "events": [{"type": "tool_end", "idx": 42, "data": data}]}),
            0,
            100,
        );
        let replay_data: serde_json::Value =
            serde_json::from_str(replay["events"][0]["data"].as_str().unwrap()).unwrap();
        assert_eq!(live_data, replay_data);
    }

    /// The coalescing lane is the one a modern client opts into. It must still
    /// publish a burst (merged, never dropped) and it must end when the queue
    /// closes, so a generation swap does not leak the drain.
    #[tokio::test]
    async fn the_coalescing_lane_merges_a_burst_and_ends_on_a_closed_queue() {
        let _home = HomeGuard::new("remote-coalesce-drain");
        let nats = FakeNats::start().await;
        let client = nats_connect_once(&nats).await;
        let (tx, rx) = tokio::sync::mpsc::channel(EVENT_QUEUE_CAPACITY);
        let drain = spawn_secure_event_publisher(
            client.clone(),
            rx,
            secure::Transport::legacy_fixture(),
            Arc::new(AtomicBool::new(true)),
        );
        let mut tap = nats.tap();

        for (idx, text) in [(1, "a"), (2, "b")] {
            let body = build_event_body(
                "s1",
                "text_chunk",
                &json!({ "text": text }).to_string(),
                "r1",
                idx,
                1,
                "e",
                "",
                -1,
                idx,
            );
            tx.send(EventPublish {
                subject: "p.pair_coal.evt.s1".to_string(),
                payload: serde_json::to_vec(&body).unwrap(),
                status_subject: None,
            })
            .await
            .unwrap();
        }

        // Collect everything the lane emits for the burst. Whether the two
        // fragments merge is a timing decision; "no character lost or
        // duplicated" is the invariant either way.
        let deadline = std::time::Instant::now() + Duration::from_millis(600);
        let mut merged_text = String::new();
        let mut published = 0usize;
        loop {
            let remaining = deadline.saturating_duration_since(std::time::Instant::now());
            if remaining.is_zero() {
                break;
            }
            match tokio::time::timeout(remaining, tap.recv()).await {
                Ok(Ok(message)) if message.subject == "p.pair_coal.evt.s1" => {
                    published += 1;
                    let payload = message.json();
                    let data: serde_json::Value =
                        serde_json::from_str(payload["data"].as_str().unwrap()).unwrap();
                    merged_text.push_str(data["text"].as_str().unwrap());
                }
                Ok(Ok(_)) => {}
                _ => break,
            }
        }
        assert!(published >= 1, "the coalescing lane must publish the burst");
        assert_eq!(merged_text, "ab", "no fragment may be lost or duplicated");

        drop(tx);
        tokio::time::timeout(Duration::from_secs(2), drain)
            .await
            .expect("a closed queue must end the drain")
            .unwrap();
    }

    #[tokio::test]
    async fn start_once_returns_empty_when_not_requested() {
        let _home = HomeGuard::new("remote-start-not-requested");
        SUPERVISOR.start_requested.store(false, Ordering::Release);
        let result = start_once(true).await.expect("empty status");
        assert_eq!(result.phase, RemotePhase::Stopped);
    }

    #[tokio::test]
    async fn start_once_without_replace_returns_the_running_bridge() {
        let _home = HomeGuard::new("remote-start-no-replace");
        let nats = FakeNats::start().await;
        install_state(fake_state(&nats, "pair_no_replace").await);
        SUPERVISOR.start_requested.store(true, Ordering::Release);

        let result = start_once(false).await.expect("returns the current bridge");
        assert_eq!(result.pair_id, "pair_no_replace");
        assert!(matches!(result.phase, RemotePhase::Ready));
        stop();
        SUPERVISOR.start_requested.store(false, Ordering::Release);
    }

    #[tokio::test]
    async fn start_replaces_an_existing_generation() {
        let _home = HomeGuard::new("remote-start-replace");
        init_store();
        wait_for_web_port_free().await;
        let platform = MockPlatform::start().await;
        let nats = FakeNats::start().await;
        sign_in(platform.url());
        // A confirmed pairing: both starts refresh the persisted credential.
        let creds = test_creds("pair_replace", nats.url(), 3600);
        pairing::save_creds(&creds).unwrap();
        platform.respond_refresh(nats.url());
        platform.respond_refresh(nats.url());

        let first = start(RemoteStartInput {}).await.expect("first start");
        assert!(matches!(first.phase, RemotePhase::Ready));

        // A second start replaces (and aborts) the previous generation.
        let second = start(RemoteStartInput {}).await.expect("second start");
        assert!(matches!(second.phase, RemotePhase::Ready));

        stop();
        wait_for_web_port_free().await;
    }

    #[tokio::test]
    async fn handle_system_suspend_stops_the_running_generation() {
        let _home = HomeGuard::new("remote-suspend");
        let nats = FakeNats::start().await;
        install_state(fake_state(&nats, "pair_suspend").await);
        let mut tap = nats.tap();
        SUPERVISOR.start_requested.store(true, Ordering::Release);

        handle_system_suspend().await;
        assert_eq!(
            SUPERVISOR.last_error_code.lock().unwrap().as_deref(),
            Some("system_sleep")
        );
        assert!(
            SUPERVISOR.state.lock().unwrap().is_none(),
            "suspend stopped the bridge"
        );
        let offline = await_publish_matching(
            &mut tap,
            "p.pair_suspend.presence",
            Duration::from_secs(5),
            // The disconnect notice specifically: `SUPERVISOR.state` is shared,
            // so a parallel test can publish a differently-shaped notice here.
            |published| published.json()["disconnected"] == json!(true),
        )
        .await;
        assert_eq!(offline.json()["disconnected"], json!(true));
        assert_eq!(offline.json()["online"], false);
        assert_eq!(offline.json()["reason"], "system_sleep");

        // Not requested to run: suspend (and its disconnect notice) is a no-op.
        SUPERVISOR.start_requested.store(false, Ordering::Release);
        handle_system_suspend().await;
        *SUPERVISOR.last_error_code.lock().unwrap() = None;
    }

    #[tokio::test]
    async fn handle_system_resume_rotates_epoch_and_recovers() {
        let _home = HomeGuard::new("remote-resume");
        let nats = FakeNats::start().await;
        install_state(fake_state(&nats, "pair_resume").await);
        let _shared = shared_runtime(&test_creds("pair_resume", nats.url(), 3600), true, false);
        SUPERVISOR.start_requested.store(true, Ordering::Release);

        handle_system_resume();
        // The spawned recovery reuses the running bridge (Ready), so it takes
        // the Ok(_) arm after a scheduling turn.
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert_eq!(
            SUPERVISOR.last_error_code.lock().unwrap().as_deref(),
            Some("system_sleep")
        );

        stop();
        SUPERVISOR.start_requested.store(false, Ordering::Release);
        handle_system_resume();
        SUPERVISOR.bridge_shared.lock().unwrap().clear();
        *SUPERVISOR.last_error_code.lock().unwrap() = None;
    }

    #[tokio::test]
    async fn status_reports_refreshing_and_network_recovery() {
        let _home = HomeGuard::new("remote-status-recovery");
        let nats = FakeNats::start().await;
        install_state(fake_state(&nats, "pair_status_recovery").await);

        // A live generation mid credential refresh reports Refreshing.
        SUPERVISOR
            .credential_refreshing
            .store(true, Ordering::Release);
        let current = status();
        assert!(matches!(current.phase, RemotePhase::Refreshing));
        assert_eq!(current.reason, Some(RemoteFailureReason::CredentialExpired));
        SUPERVISOR
            .credential_refreshing
            .store(false, Ordering::Release);

        // A dead broker surfaces as Network with start-retry recovery details.
        SUPERVISOR.start_requested.store(true, Ordering::Release);
        nats.kill();
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        loop {
            let disconnected = {
                let guard = SUPERVISOR.state.lock().unwrap();
                guard.as_ref().unwrap().client.connection_state()
                    != async_nats::connection::State::Connected
            };
            if disconnected {
                break;
            }
            assert!(std::time::Instant::now() < deadline, "client never noticed");
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        SUPERVISOR.start_retry_attempts.store(3, Ordering::Release);
        SUPERVISOR.start_retry_since.store(12345, Ordering::Release);
        SUPERVISOR.start_retry_next_at.store(999, Ordering::Release);
        let current = status();
        assert!(matches!(current.phase, RemotePhase::Reconnecting));
        assert_eq!(current.reason, Some(RemoteFailureReason::Network));
        let recovery = current.recovery.expect("recovery progress");
        assert_eq!(recovery.attempt, 3);
        assert_eq!(recovery.since, 12345);
        assert_eq!(recovery.next_retry_at, Some(999));

        stop();
        SUPERVISOR.start_requested.store(false, Ordering::Release);
        SUPERVISOR.start_retry_attempts.store(0, Ordering::Release);
        SUPERVISOR.start_retry_since.store(0, Ordering::Release);
        SUPERVISOR.start_retry_next_at.store(0, Ordering::Release);
    }

    #[test]
    fn status_none_branch_maps_every_error_code() {
        let _home = HomeGuard::new("remote-status-none-codes");
        assert!(SUPERVISOR.state.lock().unwrap().is_none());

        let cases = [
            (
                "account_authorization",
                RemotePhase::Failed,
                Some(RemoteFailureReason::AccountAuthorization),
            ),
            (
                "service_authorization",
                RemotePhase::Failed,
                Some(RemoteFailureReason::ServiceAuthorization),
            ),
            (
                "service_config",
                RemotePhase::Failed,
                Some(RemoteFailureReason::ServiceAuthorization),
            ),
            (
                "reconnect_required",
                RemotePhase::Failed,
                Some(RemoteFailureReason::GenerationUnhealthy),
            ),
            (
                "system_sleep",
                RemotePhase::Failed,
                Some(RemoteFailureReason::SystemSleep),
            ),
            ("connecting", RemotePhase::Connecting, None),
            (
                "protocol",
                RemotePhase::Failed,
                Some(RemoteFailureReason::Protocol),
            ),
            (
                "server",
                RemotePhase::Reconnecting,
                Some(RemoteFailureReason::RemoteServer),
            ),
            (
                "unknown_code",
                RemotePhase::Failed,
                Some(RemoteFailureReason::Local),
            ),
        ];
        for (code, phase, reason) in cases {
            *SUPERVISOR.last_error_code.lock().unwrap() = Some(code.to_string());
            let current = status();
            assert_eq!(current.phase, phase, "code {code}");
            assert_eq!(current.reason, reason, "code {code}");
        }

        // The retry recovery block exposes the next-retry timestamp when set.
        *SUPERVISOR.last_error_code.lock().unwrap() = Some("network".to_string());
        SUPERVISOR.start_requested.store(true, Ordering::Release);
        SUPERVISOR
            .start_retry_running
            .store(true, Ordering::Release);
        SUPERVISOR
            .start_retry_next_at
            .store(1234, Ordering::Release);
        let current = status();
        assert_eq!(current.recovery.as_ref().unwrap().next_retry_at, Some(1234));

        *SUPERVISOR.last_error_code.lock().unwrap() = None;
        SUPERVISOR.start_requested.store(false, Ordering::Release);
        SUPERVISOR
            .start_retry_running
            .store(false, Ordering::Release);
        SUPERVISOR.start_retry_next_at.store(0, Ordering::Release);
    }

    #[tokio::test]
    async fn stopping_supervisor_cancels_owned_recovery_work() {
        let _home = HomeGuard::new("supervisor-cancel");
        SUPERVISOR.start_requested.store(true, Ordering::Release);
        let (entered_tx, entered_rx) = tokio::sync::oneshot::channel();
        let (held_tx, held_rx) = tokio::sync::oneshot::channel::<()>();
        SUPERVISOR.spawn(async move {
            let _held = held_tx;
            let _ = entered_tx.send(());
            std::future::pending::<()>().await;
        });
        entered_rx.await.unwrap();
        stop();
        assert!(tokio::time::timeout(Duration::from_secs(1), held_rx)
            .await
            .unwrap()
            .is_err());
        assert_eq!(status().phase, RemotePhase::Stopped);
    }

    #[tokio::test]
    async fn heartbeat_sessions_publish_failure_exits() {
        let _home = HomeGuard::new("remote-hb-sessions-fail");
        init_store();
        let nats = FakeNats::start().await;
        let client = nats_connect_once(&nats).await;
        let pair = unique("pairhbs");
        // The publisher serves the device in control; this fixture is it.
        SUPERVISOR.restore_active(Some(pair.clone()));
        // A thread title that alone exceeds the broker payload cap: presence
        // publishes (small pair id), then the sessions snapshot fails.
        crate::store::create_thread(crate::store::CreateThreadInput {
            mode: "chat".to_string(),
            title: Some("n".repeat(9 * 1024 * 1024)),
            workspace_id: None,
            workspace_path: None,
            workspace_name: None,
            agent_session_id: Some(unique("sesshbs")),
        })
        .unwrap();

        let handle = spawn_presence_heartbeat(client.clone(), pair, "bridge_hbs".into());
        tokio::time::timeout(Duration::from_secs(3), handle)
            .await
            .expect("catalog publish failure must stop heartbeat")
            .expect("heartbeat did not panic");
    }

    #[tokio::test]
    async fn heartbeat_workspaces_publish_failure_exits() {
        let _home = HomeGuard::new("remote-hb-workspaces-fail");
        init_store();
        let nats = FakeNats::start().await;
        let client = nats_connect_once(&nats).await;
        let pair = unique("pairhbw");
        // The publisher serves the device in control; this fixture is it.
        SUPERVISOR.restore_active(Some(pair.clone()));
        // A workspace name that alone exceeds the broker payload cap: presence
        // and the (empty) sessions snapshot publish, then workspaces fails.
        let workspace_dir = std::env::temp_dir().join(unique("futureos-ws-hbw"));
        std::fs::create_dir_all(&workspace_dir).unwrap();
        crate::store::create_workspace(crate::store::CreateWorkspaceInput {
            name: Some("n".repeat(9 * 1024 * 1024)),
            path: workspace_dir.to_string_lossy().to_string(),
            description: None,
            create_directory: None,
        })
        .unwrap();

        let handle = spawn_presence_heartbeat(client.clone(), pair, "bridge_hbw".into());
        tokio::time::timeout(Duration::from_secs(3), handle)
            .await
            .expect("catalog publish failure must stop heartbeat")
            .expect("heartbeat did not panic");
        std::fs::remove_dir_all(&workspace_dir).ok();
    }

    #[tokio::test]
    async fn credential_refresh_returns_when_health_is_terminal() {
        let _home = HomeGuard::new("remote-refresh-terminal");
        let nats = FakeNats::start().await;
        let creds = test_creds("pair_terminal", nats.url(), 30);
        pairing::save_creds(&creds).unwrap();

        let state = fake_state(&nats, "pair_terminal").await;
        let confirmed = state.pairing_confirmed.clone();
        install_state(state);
        // Mark the generation's health terminal before the refresh loops.
        {
            let guard = SUPERVISOR.state.lock().unwrap();
            guard
                .as_ref()
                .unwrap()
                .nats_health
                .service_config_error
                .store(true, Ordering::Release);
        }
        let handshake =
            commands::new_handshake_state(creds, confirmed.clone(), "bridge_terminal".into());
        let handle = spawn_credential_refresh(
            "pair_terminal".to_string(),
            commands::new_reply_slots(),
            confirmed,
            handshake,
        );
        tokio::time::timeout(Duration::from_secs(5), handle)
            .await
            .expect("terminal refresh returns")
            .expect("refresh task not panicked");
        stop();
    }

    #[tokio::test]
    async fn credential_refresh_retries_when_readiness_fails() {
        let _home = HomeGuard::new("remote-refresh-readiness");
        let platform = MockPlatform::start().await;
        let nats = FakeNats::start().await;
        let nats2 = FakeNats::start().await;
        sign_in(platform.url());
        // A pair id too large to publish a presence payload: the refreshed
        // generation connects but its readiness publish fails, so the loop
        // aborts the half-built generation and retries.
        let pair = "p".repeat(9 * 1024 * 1024);
        SUPERVISOR.restore_active(Some(pair.clone()));
        let creds = test_creds(&pair, nats.url(), 30);
        pairing::save_creds(&creds).unwrap();
        platform.respond_refresh(nats2.url());

        let state = fake_state(&nats, &pair).await;
        let confirmed = state.pairing_confirmed.clone();
        install_state(state);
        let handshake =
            commands::new_handshake_state(creds, confirmed.clone(), "bridge_ready".into());
        let handle = spawn_credential_refresh(
            pair.clone(),
            commands::new_reply_slots(),
            confirmed,
            handshake,
        );
        // The refresh loops (readiness keeps failing), so it never completes.
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(
            !handle.is_finished(),
            "refresh keeps retrying after readiness failure"
        );
        handle.abort();
        stop();
    }

    #[tokio::test]
    async fn start_once_without_replace_falls_through_when_inactive() {
        let _home = HomeGuard::new("remote-start-inactive");
        assert!(SUPERVISOR.state.lock().unwrap().is_none());
        SUPERVISOR.start_requested.store(true, Ordering::Release);
        // No running bridge → runtime_active is false, so the no-replace check
        // falls through, then establish fails (no platform) → network status.
        sign_in("http://127.0.0.1:9");
        let result = start_once(false).await.expect("network maps to status");
        assert_eq!(result.reason, Some(RemoteFailureReason::Network));
        SUPERVISOR.start_requested.store(false, Ordering::Release);
        *SUPERVISOR.last_error_code.lock().unwrap() = None;
    }

    #[tokio::test]
    async fn notify_mobile_unpair_publishes_offline_presence() {
        let _home = HomeGuard::new("remote-unpair-notice");
        let nats = FakeNats::start().await;
        install_state(fake_state(&nats, "pair_unpair_notice").await);
        let mut tap = nats.tap();

        notify_mobile_unpair().await;

        let published = await_publish_matching(
            &mut tap,
            "p.pair_unpair_notice.presence",
            Duration::from_secs(5),
            |published| published.json()["unpaired"] == json!(true),
        )
        .await;
        let body = published.json();
        assert_eq!(body["online"], json!(false));
        assert_eq!(body["unpaired"], json!(true));
        stop();
    }

    /// A wake-from-sleep recovery that is in flight must be presented as
    /// *reconnecting*, even though the stored error code is the same
    /// `system_sleep` that produced it. The phase is what the UI indicator reads:
    /// the identical code with no worker running is a finished sleep, reported as
    /// a failure attributed to sleep rather than as an in-flight reconnect.
    #[tokio::test]
    async fn status_presents_an_in_flight_sleep_recovery_as_reconnecting() {
        let _home = HomeGuard::new("remote-status-sleep");
        SUPERVISOR.start_requested.store(true, Ordering::Release);
        SUPERVISOR
            .resume_recovery_running
            .store(true, Ordering::Release);
        *SUPERVISOR.last_error_code.lock().unwrap() = Some("system_sleep".to_string());
        let reconnecting = status();
        assert!(
            matches!(reconnecting.phase, RemotePhase::Reconnecting),
            "{reconnecting:?}"
        );
        assert_eq!(reconnecting.reason, Some(RemoteFailureReason::SystemSleep));

        // The same error code with the recovery worker stopped is not an
        // in-flight reconnect; both keep the reason, because the cause is the
        // same sleep.
        SUPERVISOR
            .resume_recovery_running
            .store(false, Ordering::Release);
        let settled = status();
        assert!(matches!(settled.phase, RemotePhase::Failed), "{settled:?}");
        assert_eq!(settled.reason, Some(RemoteFailureReason::SystemSleep));

        SUPERVISOR.start_requested.store(false, Ordering::Release);
        *SUPERVISOR.last_error_code.lock().unwrap() = None;
    }

    /// `spawn_runtime_reconnect` is the recovery for a subscription task that
    /// died. In the test build it is deliberately a no-op: it must not record a
    /// failure attempt, because the tests that drive the supervisor own that
    /// state themselves.
    #[tokio::test]
    async fn spawning_a_runtime_reconnect_is_a_noop_under_test() {
        let _home = HomeGuard::new("remote-runtime-reconnect-noop");
        SUPERVISOR
            .runtime_reconnect_attempts
            .store(0, Ordering::Release);
        spawn_runtime_reconnect("pair_test".to_string());
        assert_eq!(
            SUPERVISOR
                .runtime_reconnect_attempts
                .load(Ordering::Acquire),
            0,
            "the test build must not arm the real reconnect worker"
        );
    }

    /// A `get_read_chunk` for a snapshot the desktop no longer holds is answered
    /// with the transport's own error code, which the phone already branches on.
    #[tokio::test]
    async fn an_expired_read_chunk_is_answered_with_the_transport_error() {
        let _home = HomeGuard::new("remote-read-chunk-expired");
        let sink = crate::remote::test_support::RecordingSink::default();
        host()
            .execute(
                future_remote_protocol::IncomingCmd {
                    cmd_type: "get_read_chunk".into(),
                    reply_id: unique("read"),
                    ..Default::default()
                },
                &sink,
            )
            .await;
        assert_eq!(sink.error_text(), "remote_read_expired");
    }
}
use super::SingleSlotMap;

/// Multi-device control: who drives the machine, and what a device that does
/// not may still do. These are the rules the product is built on, so they are
/// pinned here rather than inferred from the UI.
mod multi_device_tests {
    use super::runtime_tests::test_creds_for_multi_device as test_creds;
    use super::*;
    use crate::remote::test_support::FakeNats;

    /// Save two devices and make `first` the controller — the shape of a
    /// machine that was driven by one of them and then paired a second.
    pub(super) fn two_devices(
        first: &str,
        second: &str,
    ) -> (pairing::PairingCreds, pairing::PairingCreds) {
        let a = test_creds(first);
        let b = test_creds(second);
        pairing::save_creds(&a).unwrap();
        pairing::save_creds(&b).unwrap();
        pairing::set_active(&a.pair_id).unwrap();
        SUPERVISOR.restore_active(Some(a.pair_id.clone()));
        (a, b)
    }

    /// A device in standby keeps renewing *its own* credential.
    ///
    /// The renewal runner belongs to one slot, but it used to read the book's
    /// primary — the controlling device — so every standby slot's runner
    /// returned at once. The generation watch reads a finished renewal task as
    /// a dead critical task: it rebuilt that slot until the reconnect budget
    /// was gone, and then reported the machine as broken (RT001, switch off)
    /// the moment control moved to that device, while its bridge was in fact
    /// serving commands.
    #[tokio::test]
    async fn a_standby_slot_keeps_renewing_its_own_credential() {
        let _home = test_support::HomeGuard::new("remote-multi-standby-refresh");
        let (controller, standby) = two_devices("pair_renew_ctrl", "pair_renew_standby");
        assert!(
            !SUPERVISOR.in_control(&standby.pair_id),
            "the fixture must actually leave the second device in standby"
        );

        let nats = FakeNats::start().await;
        let confirmed = Arc::new(std::sync::atomic::AtomicBool::new(true));
        let mut state = test_support::fake_state(&nats, &standby.pair_id).await;
        state.pairing_confirmed = confirmed.clone();
        SUPERVISOR
            .state
            .lock()
            .unwrap()
            .insert(standby.pair_id.clone(), state);
        let renewal = spawn_credential_refresh(
            standby.pair_id.clone(),
            commands::ReplySlots::default(),
            confirmed,
            commands::new_handshake_state(
                standby.clone(),
                Arc::new(std::sync::atomic::AtomicBool::new(true)),
                "bridge_renew".to_string(),
            ),
        );

        // Long enough for the runner's own tick to have come and gone.
        tokio::time::sleep(std::time::Duration::from_millis(120)).await;
        let still_renewing = !renewal.is_finished();
        let control_kept = SUPERVISOR.in_control(&controller.pair_id);
        renewal.abort();
        stop();
        assert!(
            still_renewing,
            "the standby slot's renewal must keep running: it is what keeps that \
             device's bridge alive, and what stops the generation watch from \
             rebuilding a slot that is serving fine"
        );
        assert!(
            control_kept,
            "the standby device's renewal must not move control"
        );
    }

    /// Taking control must not make the machine report the *other* device's
    /// listener as broken.
    ///
    /// The browser listener is one per process and belongs to the slot that
    /// bound it. Reading a slot with no `web_task` as "the listener died" made
    /// the focused device — the one that just took over — spend the retry
    /// budget rebinding a port its owner still held, and then show `LC002` for
    /// a listener that was never down.
    #[tokio::test]
    async fn taking_over_does_not_report_the_other_devices_listener_as_dead() {
        let _home = test_support::HomeGuard::new("remote-multi-web-owner");
        let (controller, standby) = two_devices("pair_webowner_ctrl", "pair_webowner_standby");
        let nats = FakeNats::start().await;
        let mut owner = test_support::fake_state(&nats, &controller.pair_id).await;
        // The controller is the slot that bound the process-wide listener.
        owner.web_task = Some(SUPERVISOR.spawn_fixture_task(std::future::pending()));
        let standby_state = test_support::fake_state(&nats, &standby.pair_id).await;
        *SUPERVISOR.web_owner.lock().unwrap() = Some(controller.pair_id.clone());
        {
            let mut states = SUPERVISOR.state.lock().unwrap();
            states.insert(controller.pair_id.clone(), owner);
            states.insert(standby.pair_id.clone(), standby_state);
        }

        let moved = SUPERVISOR.claim_control(&standby.pair_id);
        assert_eq!(moved.as_deref(), Some(controller.pair_id.as_str()));
        let focused = status();
        // Even with the whole retry budget spent: the listener is the owner's,
        // its task is alive, and this device has nothing to report.
        SUPERVISOR
            .web_reconnect_attempts
            .store(MAX_WEB_RECONNECT_ATTEMPTS, Ordering::Release);
        let exhausted = status();
        stop();
        assert_eq!(
            focused.pair_id, standby.pair_id,
            "the takeover must be the focus"
        );
        assert_eq!(
            focused.warning_code, None,
            "a healthy listener must not be reported as missing"
        );
        assert_eq!(
            exhausted.warning_code, None,
            "a device that never held the listener must not take the blame for it"
        );
    }

    /// The fix must not hide a listener that really is down: the device that
    /// owns it is still the one that reports it.
    #[tokio::test]
    async fn the_listener_owner_still_reports_its_own_failure() {
        let _home = test_support::HomeGuard::new("remote-multi-web-owner-failed");
        let (controller, standby) = two_devices("pair_webfail_ctrl", "pair_webfail_standby");
        let nats = FakeNats::start().await;
        // Bound and then gone: the port is free and the process owed a listener.
        let mut owner = test_support::fake_state(&nats, &controller.pair_id).await;
        let dead = SUPERVISOR.spawn_fixture_task(std::future::pending::<()>());
        dead.abort();
        dead.clone().completed().await;
        owner.web_task = Some(dead);
        let standby_state = test_support::fake_state(&nats, &standby.pair_id).await;
        *SUPERVISOR.web_owner.lock().unwrap() = Some(controller.pair_id.clone());
        {
            let mut states = SUPERVISOR.state.lock().unwrap();
            states.insert(controller.pair_id.clone(), owner);
            states.insert(standby.pair_id.clone(), standby_state);
        }

        SUPERVISOR.claim_control(&controller.pair_id);
        SUPERVISOR
            .web_reconnect_attempts
            .store(MAX_WEB_RECONNECT_ATTEMPTS, Ordering::Release);
        let current = status();
        stop();
        assert_eq!(current.pair_id, controller.pair_id);
        assert_eq!(
            current.warning_code.as_deref(),
            Some("web_bind"),
            "the device that owns the listener must still report it missing"
        );
    }

    /// Taking control moves the pointer, and persists it: a restart must not
    /// hand the machine back to whoever held it before.
    #[test]
    fn takeover_moves_and_persists_control() {
        let _home = test_support::HomeGuard::new("remote-multi-takeover");
        let (controller, standby) = two_devices("pair_ctrl2", "pair_standby2");

        let previous = SUPERVISOR.claim_control(&standby.pair_id);
        assert_eq!(previous.as_deref(), Some(controller.pair_id.as_str()));
        assert!(SUPERVISOR.in_control(&standby.pair_id));
        assert!(!SUPERVISOR.in_control(&controller.pair_id));
        assert_eq!(
            pairing::active_slot().as_deref(),
            Some(standby.pair_id.as_str()),
            "the controlling device must survive a restart"
        );
    }

    /// A device with no runtime cannot take control: moving the pointer to it
    /// would tell the connected device to stand by for a bridge that is not
    /// there, leaving nobody driving the machine.
    #[tokio::test]
    async fn an_offline_device_cannot_take_control() {
        let _home = test_support::HomeGuard::new("remote-multi-offline-takeover");
        let (controller, standby) = two_devices("pair_ctrl5", "pair_standby5");

        // No runtime is installed for either device in this fixture; the
        // pointer must therefore stay where it was.
        let refused = super::takeover(&standby.pair_id).await;
        assert!(
            matches!(refused, Err(crate::AppError::Message(ref message)) if message == "device_offline"),
            "an offline device must be refused, not silently given the machine"
        );
        assert!(
            SUPERVISOR.in_control(&controller.pair_id),
            "the refusal must leave the existing controller in place"
        );
    }

    /// A device that reconnects is not a device that asked for the machine.
    /// This is the anti-ping-pong rule: if a reconnect reclaimed control, two
    /// devices would evict each other for as long as both keep retrying.
    #[test]
    fn reconnecting_restores_the_persisted_controller_and_nothing_else() {
        let _home = test_support::HomeGuard::new("remote-multi-reconnect");
        let (controller, standby) = two_devices("pair_ctrl3", "pair_standby3");

        SUPERVISOR.restore_active(None);
        SUPERVISOR.restore_active(pairing::active_slot());

        assert!(
            SUPERVISOR.in_control(&controller.pair_id),
            "the persisted controller keeps the machine across a restart"
        );
        assert!(!SUPERVISOR.in_control(&standby.pair_id));
    }

    /// Unpairing the controlling device must clear the pointer it named — and
    /// when that leaves a single device, that one controls, because a machine
    /// with one device has no arbitration to do.
    #[tokio::test]
    async fn unpairing_the_controller_clears_its_pointer() {
        let _home = test_support::HomeGuard::new("remote-multi-unpair");
        let (controller, standby) = two_devices("pair_ctrl4", "pair_standby4");
        SUPERVISOR.restore_active(Some(controller.pair_id.clone()));

        super::unpair_slot(&controller.pair_id).await.unwrap();

        assert!(pairing::load_slots()
            .iter()
            .all(|slot| slot.pair_id != controller.pair_id));
        assert_eq!(
            pairing::active_slot().as_deref(),
            Some(standby.pair_id.as_str()),
            "the only device left is the machine's controller"
        );
        assert!(SUPERVISOR.in_control(&standby.pair_id));
    }

    /// The same removal with two devices left is *not* a promotion: nobody was
    /// given the machine, so nobody may act as if they were.
    #[tokio::test]
    async fn unpairing_the_controller_of_three_leaves_nobody_in_control() {
        let _home = test_support::HomeGuard::new("remote-multi-unpair-three");
        let (controller, second) = two_devices("pair_ctrl5", "pair_second5");
        let third = test_creds("pair_third5");
        pairing::save_creds(&third).unwrap();
        SUPERVISOR.restore_active(Some(controller.pair_id.clone()));

        super::unpair_slot(&controller.pair_id).await.unwrap();

        assert_eq!(pairing::active_slot(), None);
        assert!(!SUPERVISOR.in_control(&second.pair_id));
        assert!(!SUPERVISOR.in_control(&third.pair_id));
    }

    /// The device list is what the Remote page shows, so it must describe every
    /// device — including the ones that are not running — and say which one is
    /// in control.
    #[test]
    fn the_device_list_names_every_paired_device_and_its_control() {
        let _home = test_support::HomeGuard::new("remote-multi-list");
        let (controller, standby) = two_devices("pair_ctrl6", "pair_standby6");
        *SUPERVISOR.last_error_code.lock().unwrap() = None;

        let current = status();
        assert_eq!(current.pair_id, controller.pair_id, "the controller leads");
        assert_eq!(current.devices.len(), 2);
        assert_eq!(
            current.devices[0].pair_id, controller.pair_id,
            "the controlling device is listed first"
        );
        assert_eq!(current.devices[0].control, DeviceControl::Active);
        assert_eq!(current.devices[1].pair_id, standby.pair_id);
        assert_eq!(current.devices[1].control, DeviceControl::Standby);
        assert!(
            current.devices.iter().all(|device| !device.connected),
            "nothing is running, so nothing may claim to be connected"
        );
    }
}

/// The answer a start or an add returns must describe the device that
/// operation produced.
///
/// This is not cosmetic: whatever pairs what those commands returned (the
/// host-bridge fixture does exactly that) would otherwise be pointed at another
/// device's pairing — a client that then talks to a subject the machine is not
/// serving, times out, and leaves its own runtime behind for the next fixture.
/// Turning the remote desktop on is not adding a connection.
///
/// The switch serves the devices the user paired; on a machine with none it is
/// simply on and empty. The other entry points (`future remote start`, the CLI's
/// device command, the headless desktop) still mint an invitation when nothing
/// is paired, because there the invitation *is* what was asked for — so the
/// difference has to live in the start, not in the caller's good intentions.
mod switch_tests {
    use super::*;
    use crate::remote::test_support::{init_store, sign_in, HomeGuard, MockPlatform};

    /// The switch on an empty machine: on, empty, and no invitation minted. The
    /// platform is a mock that *would* answer a pairing request, so a request
    /// reaching it is the failure the assertion names.
    #[tokio::test]
    async fn the_switch_leaves_an_empty_machine_empty_and_on() {
        let _home = HomeGuard::new("remote-switch-empty");
        init_store();
        let platform = MockPlatform::start().await;
        sign_in(platform.url());

        let status = super::supervisor::enable()
            .await
            .expect("an empty machine still answers with a status");

        assert!(
            matches!(status.phase, RemotePhase::Enabled),
            "the switch must read as on, not stopped or connecting: {status:?}"
        );
        assert!(status.reason.is_none(), "{status:?}");
        assert!(status.pairing_code.is_none(), "no invitation was asked for");
        assert!(
            status.devices.is_empty(),
            "and none was created: {status:?}"
        );
        assert!(
            platform.requests().is_empty(),
            "the switch must not touch the platform: {:?}",
            platform.requests()
        );
        assert!(
            pairing::load_slots().is_empty(),
            "nothing may be written to the pairing book"
        );
    }

    /// The same machine, asked for an invitation on purpose: the CLI and the
    /// headless desktop rely on a plain start still producing a code.
    #[tokio::test]
    async fn a_plain_start_still_offers_an_invitation_on_an_empty_machine() {
        let _home = HomeGuard::new("remote-start-empty");
        init_store();
        let platform = MockPlatform::start().await;
        sign_in(platform.url());

        // The platform refuses the invitation (no scripted answer), which is
        // enough: reaching it is what this test measures, and a refused mint is
        // reported as a status rather than panicking the start.
        let status = super::supervisor::start(RemoteStartInput {})
            .await
            .expect("a refused invitation is still a status");
        assert!(
            !platform.requests().is_empty(),
            "a plain start must ask for an invitation: {status:?}"
        );
    }
}

mod start_focus_tests {
    use super::runtime_tests::test_creds_for_multi_device as test_creds;
    use super::*;

    #[test]
    fn a_focused_status_names_that_device_even_when_another_one_controls() {
        let _home = test_support::HomeGuard::new("remote-focus-status");
        let controller = test_creds("pair_ctrl_focus");
        let newcomer = test_creds("pair_new_focus");
        pairing::save_creds(&controller).unwrap();
        pairing::save_creds(&newcomer).unwrap();
        pairing::set_active(&controller.pair_id).unwrap();
        SUPERVISOR.restore_active(Some(controller.pair_id.clone()));

        let focused = super::supervisor::status_focused(Some(&newcomer.pair_id));
        assert_eq!(
            focused.pair_id, newcomer.pair_id,
            "a start/add must answer about the device it produced"
        );
        assert_eq!(
            focused.devices[0].pair_id, controller.pair_id,
            "but the list still leads with whoever controls"
        );
        assert_eq!(focused.devices[0].control, DeviceControl::Active);

        // An unknown or absent focus falls back to the controller, so the
        // untouched status() is unchanged.
        assert_eq!(
            super::supervisor::status_focused(Some("pair_absent")).pair_id,
            controller.pair_id
        );
        assert_eq!(status().pair_id, controller.pair_id);
    }
}

/// The per-device runtime's own entry points: what a start, an add and a
/// removal do when the machine has no runtime yet, claims its only device, is
/// asked to keep an existing slot, or loses its controller. Each one is a
/// decision about *which* device answers a command, so they are pinned here
/// instead of inferred from the Remote page.
mod slot_tests {
    use super::runtime_tests::test_creds;
    use super::*;
    use crate::remote::test_support::{
        fake_state, init_store, sign_in, FakeNats, HomeGuard, MockPlatform,
    };
    use serde_json::json;
    use std::time::Duration;

    /// Install one fake bridge under `pair_id` and clear it again on drop.
    ///
    /// `install_fake_state` owns its own broker; these tests need the broker
    /// (or the state's security) in their own hands, so they install directly
    /// and restore in `Drop` — panic-safe, which is the `FLAKES.md` item 27
    /// requirement every fixture here shares.
    struct Slot {
        pair_id: String,
    }

    impl Slot {
        async fn install(broker: &FakeNats, pair_id: &str) -> Slot {
            let state = fake_state(broker, pair_id).await;
            Self::install_state(state)
        }

        /// The first install of a test: the state must be empty, so a leaked
        /// bridge from another test is a failure here instead of a mystery in
        /// whichever test it poisons next.
        fn install_state(state: crate::remote::RemoteState) -> Slot {
            assert!(
                SUPERVISOR.state.lock().unwrap().is_empty(),
                "a previous test leaked SUPERVISOR.state; fix that test"
            );
            Self::install_into(state)
        }

        /// A second device for a test that is about two of them; no emptiness
        /// check, because the first one is legitimately still there.
        fn install_into(state: crate::remote::RemoteState) -> Slot {
            let pair_id = state.pair_id.clone();
            SUPERVISOR
                .state
                .lock()
                .unwrap()
                .insert(pair_id.clone(), state);
            Slot { pair_id }
        }
    }

    impl Drop for Slot {
        fn drop(&mut self) {
            let taken = SUPERVISOR
                .state
                .lock()
                .unwrap_or_else(|poison| poison.into_inner())
                .remove(&self.pair_id);
            if let Some(state) = taken {
                crate::remote::supervisor::abort_generation(state);
            }
            SUPERVISOR.release_control(&self.pair_id);
        }
    }

    /// `add_device` mints a *keep-existing* invitation and connects only the
    /// device it just paired — the machine's running devices are not touched.
    /// Its answer names that device, because the invitation on screen is its
    /// QR code and the caller pairs whatever `pair_id` it returned.
    #[tokio::test]
    async fn add_device_connects_and_names_the_device_it_paired() {
        let _home = HomeGuard::new("remote-add-device");
        init_store();
        let platform = MockPlatform::start().await;
        let nats = FakeNats::start().await;
        sign_in(platform.url());
        let code = platform.respond_pair_code(nats.url());
        SUPERVISOR.start_requested.store(false, Ordering::Release);
        SUPERVISOR.restore_active(None);

        let status = crate::remote::supervisor::add_device()
            .await
            .expect("add device");

        assert!(matches!(status.phase, RemotePhase::Ready), "{status:?}");
        let shown = status
            .pairing_code
            .as_deref()
            .expect("the new device is still an invitation");
        assert!(
            shown.contains(&code),
            "the answer must carry the invitation that was just minted: {shown}"
        );
        assert!(
            SUPERVISOR.start_requested.load(Ordering::Acquire),
            "adding a device means the machine is running"
        );
        {
            let slots = SUPERVISOR.state.lock().unwrap();
            assert!(
                slots.contains_key(&status.pair_id),
                "the slot the command started is the one it answered about"
            );
            assert_eq!(slots.len(), 1, "only the new device was started");
        }
        assert!(
            pairing::load_slots().is_empty(),
            "an unconfirmed device is not in the book yet: it lives in the runtime"
        );
        let invitation = SUPERVISOR
            .state
            .lock()
            .unwrap()
            .get(&status.pair_id)
            .and_then(|state| state.pairing_code.clone())
            .expect("the runtime holds the invitation until the phone confirms");
        assert!(invitation.contains(&code));
        // The platform was asked to *keep* what was already paired.
        let add_request = platform
            .requests()
            .into_iter()
            .find(|(_, path, _)| path == "/client/v1/remote/pair/code")
            .expect("the pairing request");
        assert!(
            add_request.2.contains("keep_existing"),
            "an added device must not replace the machine's existing ones: {}",
            add_request.2
        );

        stop();
    }

    /// The device a request adds shows up in the machine's device list as a
    /// **standby** row carrying its invitation, while the device in control keeps
    /// the machine.
    ///
    /// This is the half of "add a device" a terminal cannot see for itself: the
    /// invitation is not a credential until a phone claims it, so it is not in the
    /// pairing book the CLI reads — it exists only in this process's device list,
    /// which is what the window, the phone and the request's own answer read.
    #[tokio::test]
    async fn an_added_device_is_listed_as_standby_with_its_invitation() {
        let _home = HomeGuard::new("remote-add-device-standby");
        init_store();
        let platform = MockPlatform::start().await;
        let nats = FakeNats::start().await;
        sign_in(platform.url());
        // The machine already has a device, and it is the one in control.
        let existing = test_creds("pair_control", nats.url(), 3600);
        pairing::save_creds(&existing).unwrap();
        platform.respond_refresh(nats.url());
        let code = platform.respond_pair_code(nats.url());
        SUPERVISOR.start_requested.store(false, Ordering::Release);
        // The device that was already there is the one in control — the state
        // adding a device must leave untouched.
        SUPERVISOR.restore_active(Some(existing.pair_id.clone()));

        let added = crate::remote::supervisor::add_device()
            .await
            .expect("add a device");
        assert_ne!(
            added.pair_id, existing.pair_id,
            "a new device, not a re-pair"
        );

        let listed = crate::remote::status();
        let row = listed
            .devices
            .iter()
            .find(|device| device.pair_id == added.pair_id)
            .unwrap_or_else(|| panic!("the added device must be listed: {listed:?}"));
        assert_eq!(row.control, DeviceControl::Standby, "{listed:?}");
        assert!(!row.connected, "an invitation is not a connection");
        assert!(
            row.pairing_code
                .as_deref()
                .is_some_and(|shown| shown.contains(&code)),
            "the row carries the invitation that was minted: {listed:?}"
        );
        // The device that was already there keeps the machine.
        let controlling = listed
            .devices
            .iter()
            .find(|device| device.control == DeviceControl::Active)
            .expect("one device is still in control");
        assert_eq!(controlling.pair_id, existing.pair_id, "{listed:?}");

        stop();
    }

    /// A machine with exactly one saved device and no controller claims it at
    /// start. There is no arbitration to do, and *not* claiming would make the
    /// device ask its own user to take the machine from nobody.
    #[tokio::test]
    async fn a_single_saved_device_claims_the_machine_at_start() {
        let _home = HomeGuard::new("remote-start-single-claim");
        init_store();
        let platform = MockPlatform::start().await;
        let nats = FakeNats::start().await;
        sign_in(platform.url());
        let creds = test_creds("pair_single_claim", nats.url(), 3600);
        pairing::save_creds(&creds).unwrap();
        // `save_creds` makes a lone device the controller, so the state this
        // test is about - a book that holds a device and no pointer - has to be
        // written down explicitly. That is exactly what `remove_slot` leaves on
        // disk between its write and `promote_a_lone_device`.
        let path = crate::store::future_dir()
            .unwrap()
            .join("remote_pairings.json");
        let mut book: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        book["active"] = serde_json::Value::Null;
        std::fs::write(&path, serde_json::to_vec(&book).unwrap()).unwrap();
        platform.respond_refresh(nats.url());
        SUPERVISOR.restore_active(None);
        assert_eq!(pairing::active_slot(), None, "nobody claimed it yet");

        let status = start(RemoteStartInput {}).await.expect("start");

        assert!(matches!(status.phase, RemotePhase::Ready), "{status:?}");
        assert_eq!(
            SUPERVISOR.active_pair().as_deref(),
            Some(creds.pair_id.as_str()),
            "the only device on the machine is the one driving it"
        );
        assert_eq!(
            pairing::active_slot().as_deref(),
            Some(creds.pair_id.as_str()),
            "and the claim is written down, so a restart keeps it"
        );

        stop();
    }

    /// Promotion after a removal is for the *unclaimed* case only. A pointer
    /// that already names a device is left exactly as the user left it —
    /// re-deciding it here would silently overrule them.
    #[test]
    fn promotion_leaves_a_claimed_pointer_alone() {
        let _home = test_support::HomeGuard::new("remote-promote-claimed");
        let (controller, standby) =
            super::multi_device_tests::two_devices("pair_promote_ctrl", "pair_promote_standby");
        SUPERVISOR.restore_active(Some(controller.pair_id.clone()));

        crate::remote::supervisor::promote_a_lone_device();

        assert_eq!(
            SUPERVISOR.active_pair().as_deref(),
            Some(controller.pair_id.as_str()),
            "a machine that already has a controller keeps it"
        );
        assert!(!SUPERVISOR.in_control(&standby.pair_id));
    }

    /// A second start that is explicitly *not* a replacement leaves a slot that
    /// is already installed alone: the device's own recovery decides when its
    /// generation is rebuilt, not an unrelated `start`.
    #[tokio::test]
    async fn a_start_without_replace_keeps_an_installed_slot() {
        let _home = HomeGuard::new("remote-start-keep-slot");
        init_store();
        let platform = MockPlatform::start().await;
        let nats = FakeNats::start().await;
        sign_in(platform.url());
        let creds = test_creds("pair_keep_slot", nats.url(), 3600);
        pairing::save_creds(&creds).unwrap();
        platform.respond_refresh(nats.url());
        SUPERVISOR.start_requested.store(true, Ordering::Release);

        let slot = Slot::install(&nats, &creds.pair_id).await;
        // The slot must look *unhealthy*, or the check before the loop returns
        // early with the running status and the per-slot guard is never
        // consulted. A finished critical task plus an exhausted crash-loop
        // budget is that shape: `slot_health` reports `Failed`, so the caller
        // goes on to try the slot and finds it already installed.
        SUPERVISOR
            .state
            .lock()
            .unwrap()
            .get(&slot.pair_id)
            .expect("installed")
            .cmd_task
            .abort();
        SUPERVISOR.runtime_reconnect_attempts.store(
            crate::remote::supervisor::MAX_RUNTIME_RECONNECT_ATTEMPTS,
            Ordering::Release,
        );
        wait_until("the slot to read as failed", Duration::from_secs(5), || {
            matches!(status().phase, RemotePhase::Failed)
        })
        .await;
        let before = SUPERVISOR
            .state
            .lock()
            .unwrap()
            .get(&slot.pair_id)
            .expect("installed")
            .generation_id;

        let status = super::start_once(false).await.expect("kept slot");

        assert_eq!(status.pair_id, creds.pair_id);
        assert_eq!(
            SUPERVISOR
                .state
                .lock()
                .unwrap()
                .get(&slot.pair_id)
                .expect("still installed")
                .generation_id,
            before,
            "the installed generation must not be rebuilt"
        );
        assert_eq!(
            SUPERVISOR
                .state
                .lock()
                .unwrap()
                .get(&slot.pair_id)
                .expect("still installed")
                .pairing_code,
            None,
            "the running device keeps its own runtime; the start did not take it over"
        );
        SUPERVISOR
            .runtime_reconnect_attempts
            .store(0, Ordering::Release);
        stop();
    }

    /// A machine whose only connected device was never given control still has
    /// *somebody* to publish to. `controlling_pair` is what every event lane
    /// reads, and `None` there would mean a working device receives nothing.
    #[tokio::test]
    async fn controlling_pair_falls_back_to_the_only_running_device() {
        let _home = test_support::HomeGuard::new("remote-controlling-pair");
        let nats = FakeNats::start().await;
        let slot = Slot::install(&nats, "pair_controlling_only").await;
        SUPERVISOR.restore_active(None);

        assert_eq!(
            SUPERVISOR.controlling_pair().as_deref(),
            Some(slot.pair_id.as_str()),
            "one connected device needs no pointer to be the machine's device"
        );

        // With two connected devices and no pointer there is nobody to pick.
        let second = Slot::install_into(fake_state(&nats, "pair_controlling_second").await);
        assert_eq!(
            SUPERVISOR.controlling_pair(),
            None,
            "choosing between two devices is the user's decision, not a default"
        );
        drop(second);
        drop(slot);
    }

    /// A client that is not connected makes the device `Failed` when nobody
    /// asked it to run — "reconnecting" is a statement about intent, and a
    /// stopped bridge that lost its socket is simply down.
    #[tokio::test]
    async fn a_disconnected_client_is_failed_when_no_start_was_requested() {
        let _home = test_support::HomeGuard::new("remote-status-disconnected");
        let nats = FakeNats::start().await;
        let slot = Slot::install(&nats, "pair_status_disconnected").await;
        SUPERVISOR.start_requested.store(false, Ordering::Release);
        let client = SUPERVISOR
            .state
            .lock()
            .unwrap()
            .get(&slot.pair_id)
            .expect("installed")
            .client
            .clone();
        nats.kill();
        tokio::time::timeout(Duration::from_secs(5), async {
            while client.connection_state() == async_nats::connection::State::Connected {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("the client must notice the dead broker");

        let current = status();

        assert_eq!(
            current.phase,
            RemotePhase::Failed,
            "a stopped bridge with no socket is failed, not reconnecting: {current:?}"
        );
        assert_eq!(current.reason, Some(RemoteFailureReason::Network));
    }

    /// Unpairing the machine notifies every connected device before the bridge
    /// goes away: the phone treats that packet as immediate offline state.
    #[tokio::test]
    async fn unpair_notifies_every_connected_device() {
        let _home = test_support::HomeGuard::new("remote-unpair-notices");
        init_store();
        let nats = FakeNats::start().await;
        let slot = Slot::install(&nats, "pair_unpair_notice").await;
        let mut tap = nats.tap();

        super::unpair().await.expect("unpair");

        let notice = test_support::await_publish_matching(
            &mut tap,
            &format!("p.{}.presence", slot.pair_id),
            Duration::from_secs(5),
            |published| published.json()["unpaired"] == json!(true),
        )
        .await;
        assert_eq!(notice.json()["online"], json!(false));
        assert_eq!(
            notice.json()["pairId"],
            json!(slot.pair_id),
            "the notice names the device it is about"
        );
        assert!(
            SUPERVISOR.state.lock().unwrap().is_empty(),
            "the bridge is stopped"
        );
    }

    /// Unpairing *one* device tells that device, by name.
    #[tokio::test]
    async fn unpairing_one_device_notifies_that_device_by_name() {
        let _home = test_support::HomeGuard::new("remote-unpair-slot-notice");
        init_store();
        let nats = FakeNats::start().await;
        let creds = test_creds("pair_unpair_slot_notice", nats.url(), 3600);
        pairing::save_creds(&creds).unwrap();
        pairing::set_active(&creds.pair_id).unwrap();
        let _slot = Slot::install(&nats, &creds.pair_id).await;
        SUPERVISOR.restore_active(Some(creds.pair_id.clone()));
        let mut tap = nats.tap();

        super::unpair_slot(&creds.pair_id)
            .await
            .expect("unpair slot");

        let notice = test_support::await_publish_matching(
            &mut tap,
            &format!("p.{}.presence", creds.pair_id),
            Duration::from_secs(5),
            |published| published.json()["unpaired"] == json!(true),
        )
        .await;
        assert_eq!(notice.json()["reason"], json!("unpaired"));
        assert_eq!(notice.json()["pairId"], json!(creds.pair_id));
        assert!(
            SUPERVISOR.state.lock().unwrap().is_empty(),
            "only the removed device's runtime is gone"
        );
    }

    /// Disconnecting one device stops serving *that* device and keeps its
    /// pairing: the row stays in the list, and the credential on disk is
    /// untouched. This is what separates "断开" from "删除" in the client list.
    #[tokio::test]
    async fn disconnecting_one_device_keeps_it_paired_and_notifies_it() {
        let _home = test_support::HomeGuard::new("remote-disconnect-slot");
        init_store();
        let nats = FakeNats::start().await;
        let creds = test_creds("pair_disconnect_slot", nats.url(), 3600);
        pairing::save_creds(&creds).unwrap();
        pairing::set_active(&creds.pair_id).unwrap();
        let _slot = Slot::install(&nats, &creds.pair_id).await;
        SUPERVISOR.restore_active(Some(creds.pair_id.clone()));
        SUPERVISOR.start_requested.store(true, Ordering::Release);
        let mut tap = nats.tap();

        let after = super::disconnect_device(&creds.pair_id)
            .await
            .expect("disconnect device");

        // The pairing survives: the credential is still on disk and the device
        // is still listed.
        assert!(
            pairing::load_slots()
                .iter()
                .any(|slot| slot.pair_id == creds.pair_id),
            "disconnect must not unpair the device"
        );
        let listed = after
            .devices
            .iter()
            .find(|device| device.pair_id == creds.pair_id);
        assert!(listed.is_some(), "the device stays in the list");
        assert!(
            !listed.unwrap().connected,
            "a disconnected device is not reported as connected"
        );
        // The runtime is gone, so nothing is being served for it any more.
        assert!(
            !SUPERVISOR
                .state
                .lock()
                .unwrap()
                .contains_key(&creds.pair_id),
            "the device's bridge is stopped"
        );
        assert!(
            !SUPERVISOR.in_control(&creds.pair_id),
            "control is released rather than silently kept"
        );

        let notice = test_support::await_publish_matching(
            &mut tap,
            &format!("p.{}.presence", creds.pair_id),
            Duration::from_secs(5),
            |published| published.json()["disconnected"] == json!(true),
        )
        .await;
        assert_eq!(notice.json()["reason"], json!("disconnected"));
        assert_eq!(
            notice.json()["unpaired"],
            json!(false),
            "the device is told it is disconnected, not unpaired"
        );
    }

    /// A device with no live bridge has nothing to disconnect; saying so beats a
    /// silent success that would hide a wrong id.
    #[tokio::test]
    async fn disconnecting_a_device_that_is_not_running_is_refused() {
        let _home = test_support::HomeGuard::new("remote-disconnect-offline");
        init_store();
        let refused = super::disconnect_device("pair_never_started").await;
        assert!(
            matches!(refused, Err(crate::AppError::Message(ref message)) if message == "device_offline"),
            "an offline device must be refused, not reported as disconnected"
        );
    }

    /// `stop_gracefully` outside a process shutdown delivers the notice on the
    /// process runtime, and `notify_mobile_disconnect` delivers it in place:
    /// both are ordinary paths that must not depend on the exit freeze.
    #[tokio::test]
    async fn a_graceful_stop_and_a_notice_call_reach_a_connected_device() {
        let _home = test_support::HomeGuard::new("remote-graceful-notice");
        init_store();
        let nats = FakeNats::start().await;
        let slot = Slot::install(&nats, "pair_graceful_notice").await;
        SUPERVISOR.start_requested.store(true, Ordering::Release);
        let mut tap = nats.tap();

        assert!(
            !crate::desktop_shutdown::is_stopping(),
            "not at process exit"
        );
        super::stop_gracefully("test_sleep").await;

        let notice = test_support::await_publish_matching(
            &mut tap,
            &format!("p.{}.presence", slot.pair_id),
            Duration::from_secs(5),
            |published| published.json()["disconnected"] == json!(true),
        )
        .await;
        assert_eq!(notice.json()["reason"], json!("test_sleep"));

        // `notify_mobile_disconnect` is the await-in-place sibling.
        let slot = Slot::install(&nats, "pair_notify_notice").await;
        let mut tap = nats.tap();
        super::notify_mobile_disconnect("power_off").await;
        let notice = test_support::await_publish_matching(
            &mut tap,
            &format!("p.{}.presence", slot.pair_id),
            Duration::from_secs(5),
            |published| published.json()["disconnected"] == json!(true),
        )
        .await;
        assert_eq!(notice.json()["reason"], json!("power_off"));
        drop(slot);
    }

    /// Poll `condition` until it holds, or fail with `what` after `timeout`.
    async fn wait_until(what: &str, timeout: Duration, mut condition: impl FnMut() -> bool) {
        let deadline = std::time::Instant::now() + timeout;
        while !condition() {
            assert!(
                std::time::Instant::now() < deadline,
                "{what} never happened"
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    /// A device whose transport cannot seal (a v2 pairing that has not
    /// handshaked yet) has nothing to say offline: the notice is skipped rather
    /// than published unsealed.
    #[tokio::test]
    async fn a_slot_without_an_active_secure_channel_announces_nothing() {
        let _home = test_support::HomeGuard::new("remote-inert-notice");
        let nats = FakeNats::start().await;
        let creds = test_creds("pair_inert_notice", nats.url(), 3600);
        let mut state = fake_state(&nats, &creds.pair_id).await;
        // A v2 identity with no channel yet: `seal` answers `None`.
        state.security = secure::new_transport(&creds);
        assert!(state.security.seal("p.presence", b"x").unwrap().is_none());
        let slot = Slot::install_state(state);
        let mut tap = nats.tap();

        crate::remote::supervisor::stop_slot(&slot.pair_id);

        test_support::assert_no_publish(
            &mut tap,
            &format!("p.{}.presence", slot.pair_id),
            Duration::from_millis(300),
        )
        .await;
        assert!(SUPERVISOR.state.lock().unwrap().is_empty());
    }
    /// A resume whose recovery attempt is refused with a *retryable* status
    /// re-arms the start retry: the recovery is not finished just because one
    /// attempt came back.
    ///
    /// The runtime installed here is terminal-healthy on purpose. Without it
    /// `start_once(false)` returns the "already running" status before it ever
    /// asks the platform (a resumed machine reports `Reconnecting`, which
    /// `runtime_active` treats as running) and the recovery arms are never
    /// reached — the same early return the shipping resume relies on.
    #[tokio::test]
    async fn a_resume_refused_with_a_retryable_status_arms_the_retry() {
        let _home = HomeGuard::new("remote-resume-retry");
        init_store();
        let platform = MockPlatform::start().await;
        let nats = FakeNats::start().await;
        sign_in(platform.url());
        platform.push(
            "/client/v1/remote/pair/code",
            503,
            json!({ "error": "busy", "message": "try later" }),
        );
        let state = fake_state(&nats, "pair_resume_retry").await;
        state
            .nats_health
            .service_config_error
            .store(true, Ordering::Release);
        let _slot = Slot::install_state(state);
        SUPERVISOR.start_requested.store(true, Ordering::Release);
        SUPERVISOR.suspended.store(true, Ordering::Release);
        *SUPERVISOR.last_error_code.lock().unwrap() = None;

        crate::remote::supervisor::handle_system_resume();

        wait_until(
            "the resume recovery to record a server failure",
            Duration::from_secs(10),
            || SUPERVISOR.last_error_code.lock().unwrap().as_deref() == Some("server"),
        )
        .await;
        assert!(
            !SUPERVISOR.resume_recovery_running.load(Ordering::Acquire),
            "the recovery budget is released once the attempt has an answer"
        );
        assert!(!SUPERVISOR.suspended.load(Ordering::Acquire));
        // The code is the proof of which arm ran: `start_failure` maps `server`
        // to `Reconnecting`, so the status it returned here was retryable and
        // the retry arm is the only one it can take. The *projection* afterwards
        // still reports the installed slot's own terminal health, which is why
        // it is not the thing asserted.
        assert_eq!(
            SUPERVISOR.start_retry_since.load(Ordering::Acquire),
            0,
            "the retry worker (a stub in this build) has not started"
        );
        SUPERVISOR.start_requested.store(false, Ordering::Release);
        *SUPERVISOR.last_error_code.lock().unwrap() = None;
    }

    /// A resume whose recovery fails for a *local*, uncategorized reason records
    /// `reconnect_required` instead: there is nothing to retry, and the page
    /// must not keep claiming the machine is coming back.
    #[tokio::test]
    async fn a_resume_with_a_local_fault_records_reconnect_required() {
        let _home = HomeGuard::new("remote-resume-local-fault");
        init_store();
        let platform = MockPlatform::start().await;
        let nats = FakeNats::start().await;
        sign_in(platform.url());
        let mut creds = test_creds("pair_resume_fault", nats.url(), 3600);
        // A seed that is not a seed: the local NKey cannot be rebuilt, which is
        // an `AppError::Message` and therefore unclassified.
        creds.nkey_seed = "not-a-seed".to_string();
        pairing::save_creds(&creds).unwrap();
        platform.respond_refresh(nats.url());
        let state = fake_state(&nats, &creds.pair_id).await;
        state
            .nats_health
            .service_config_error
            .store(true, Ordering::Release);
        let _slot = Slot::install_state(state);
        SUPERVISOR.start_requested.store(true, Ordering::Release);
        SUPERVISOR.suspended.store(true, Ordering::Release);
        *SUPERVISOR.last_error_code.lock().unwrap() = None;

        crate::remote::supervisor::handle_system_resume();

        wait_until(
            "the resume to record its failure",
            Duration::from_secs(10),
            || SUPERVISOR.last_error_code.lock().unwrap().as_deref() == Some("reconnect_required"),
        )
        .await;
        assert!(
            !SUPERVISOR.resume_recovery_running.load(Ordering::Acquire),
            "a failed recovery must not leave the resume flag set"
        );
        SUPERVISOR.start_requested.store(false, Ordering::Release);
        *SUPERVISOR.last_error_code.lock().unwrap() = None;
    }

    /// A start whose transport cannot be built (the peer refuses the secure
    /// handshake after `CONNECT` succeeded) is reported as the categorized
    /// failure it is — not as a panic and not as a half-installed slot.
    #[tokio::test]
    async fn a_start_whose_transport_cannot_be_built_reports_the_failure() {
        let _home = HomeGuard::new("remote-start-build-failure");
        init_store();
        let platform = MockPlatform::start().await;
        let nats = FakeNats::start().await;
        sign_in(platform.url());
        let creds = test_creds("pair_build_failure", nats.url(), 3600);
        pairing::save_creds(&creds).unwrap();
        platform.respond_refresh(nats.url());
        SUPERVISOR
            .hooks
            .inject_build_failure
            .store(1, Ordering::SeqCst);

        let status = start(RemoteStartInput {}).await.expect("categorized");

        assert_eq!(
            SUPERVISOR.hooks.inject_build_failure.load(Ordering::SeqCst),
            0,
            "the injected failure must be the one that was consumed"
        );
        assert!(
            matches!(status.phase, RemotePhase::Reconnecting),
            "a transport failure is retryable, not terminal: {status:?}"
        );
        assert_eq!(status.reason, Some(RemoteFailureReason::Network));
        assert!(
            SUPERVISOR.state.lock().unwrap().is_empty(),
            "a slot whose transport failed is never installed"
        );
        SUPERVISOR.start_requested.store(false, Ordering::Release);
    }

    /// The credential refresh's own rebuild can fail after the *new* credential
    /// connected. It must keep the running generation, clear the refresh flag,
    /// and wait for the next renewal instead of tearing the bridge down.
    #[tokio::test]
    async fn a_refresh_whose_transport_cannot_be_rebuilt_keeps_the_running_bridge() {
        let _home = HomeGuard::new("remote-refresh-build-failure");
        init_store();
        let platform = MockPlatform::start().await;
        let nats = FakeNats::start().await;
        sign_in(platform.url());
        // Already inside the renewal lead, so the worker does not sleep.
        let creds = test_creds("pair_refresh_build", nats.url(), 60);
        pairing::save_creds(&creds).unwrap();
        platform.respond_refresh(nats.url());
        let state = fake_state(&nats, &creds.pair_id).await;
        let pairing_confirmed = state.pairing_confirmed.clone();
        let generation = state.generation_id;
        let slot = Slot::install_state(state);
        let handshake = crate::remote::commands::new_handshake_state(
            creds.clone(),
            pairing_confirmed.clone(),
            format!("bridge_{}", crate::remote::test_support::unique("refresh")),
        );
        SUPERVISOR
            .hooks
            .inject_build_failure
            .store(1, Ordering::SeqCst);

        let task = transport::spawn_credential_refresh(
            slot.pair_id.clone(),
            crate::remote::commands::new_reply_slots(),
            pairing_confirmed,
            handshake,
        );

        wait_until(
            "the refresh to attempt its rebuild",
            Duration::from_secs(10),
            || SUPERVISOR.hooks.inject_build_failure.load(Ordering::SeqCst) == 0,
        )
        .await;
        wait_until(
            "the refresh flag to be cleared",
            Duration::from_secs(10),
            || !SUPERVISOR.credential_refreshing.load(Ordering::Acquire),
        )
        .await;
        assert_eq!(
            SUPERVISOR
                .state
                .lock()
                .unwrap()
                .get(&slot.pair_id)
                .expect("the generation is untouched")
                .generation_id,
            generation,
            "a failed rebuild must not replace the running generation"
        );
        assert!(
            platform
                .requests()
                .iter()
                .any(|(_, path, _)| path == "/client/v1/remote/auth/token"),
            "the failure is after the new credential was fetched"
        );
        task.abort();
    }
}
