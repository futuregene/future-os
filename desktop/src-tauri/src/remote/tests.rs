use super::*;

#[cfg(test)]
mod contract_tests {
    use super::*;

    #[test]
    fn drop_log_rate_limits_episodes() {
        let counters = DropCounters::new();

        // Episode progress is reported only at power-of-two attempts.
        for i in 1..=10 {
            let line = counters.record_drop("queue full", "tool_delta", "s1", 1_000 + i);
            assert_eq!(
                line.is_some(),
                i.is_power_of_two(),
                "unexpected reporting decision for drop {i}"
            );
        }
        assert_eq!(counters.reports.load(Ordering::Relaxed), 4);
    }

    #[test]
    fn drop_log_reports_recovery_and_resets() {
        let counters = DropCounters::new();

        counters.record_drop("NATS not connected", "tool_delta", "s1", 1_000);
        let line = counters
            .report_recovery()
            .expect("recovery after an active episode reports");
        assert!(line.contains("dropped 1 events"), "got: {line}");

        // No episode active: recovery is silent and counters stay zero.
        assert!(counters.report_recovery().is_none());

        // The next episode starts fresh: the first drop logs again.
        assert!(counters
            .record_drop("queue full", "tool_delta", "s1", 2_000)
            .is_some());
    }

    #[test]
    fn nats_v2_event_keeps_every_v1_field_unchanged() {
        let body = build_event_body(
            "session-1",
            "text_chunk",
            r#"{"text":"hi"}"#,
            "run-1",
            7,
            2,
            "evt-1",
            "2026-08-02T00:00:00Z",
            -1,
            11,
        );
        assert_eq!(body["type"], "text_chunk");
        assert_eq!(body["data"], r#"{"text":"hi"}"#);
        assert_eq!(body["runId"], "run-1");
        assert_eq!(body["idx"], 7);
        assert_eq!(body["schemaVersion"], 2);
        assert_eq!(body["sessionId"], "session-1");
        assert_eq!(body["epoch"], 2);
        assert_eq!(body["eventId"], "evt-1");
        assert_eq!(body["runSequence"], 11);
    }

    #[test]
    fn nats_session_event_has_independent_cursor() {
        let body = build_event_body(
            "session-1",
            "model_changed",
            "{}",
            "",
            -1,
            0,
            "session-1:session:4",
            "2026-08-02T00:00:00Z",
            4,
            -1,
        );
        assert_eq!(body["runId"], "");
        assert_eq!(body["sessionIdx"], 4);
        assert_eq!(body["runSequence"], -1);
    }

    #[test]
    fn failure_episode_same_category_non_power_of_two_attempt_is_silent() {
        let category = "test_episode_silent";
        let episode = FailureEpisode(Mutex::new(FailureEpisodeState::default()));
        // Attempt 1 (fresh category) and attempt 2 (power of two) both report;
        // attempt 3 is not a power of two, so it falls through to the silent None.
        assert!(episode.record(category, "first").is_some());
        assert!(episode.record(category, "second").is_some());
        assert!(episode.record(category, "third").is_none());
    }

    #[test]
    fn failure_episode_category_switch_after_quota_exhaustion_reports_nothing() {
        // Exhaust the process-wide log quota for a fresh category, then start a
        // new episode with that category: the first `record` resets the episode
        // state but `permit_failure_log` refuses the line, so it returns None.
        let category = "test_episode_switch_quota";
        for _ in 0..MAX_FAILURE_LOGS_PER_CATEGORY {
            assert!(permit_failure_log(category));
        }
        assert!(!permit_failure_log(category));

        let episode = FailureEpisode(Mutex::new(FailureEpisodeState::default()));
        assert!(episode.record(category, "boom").is_none());
    }

    #[test]
    fn support_codes_cover_every_category() {
        assert_eq!(support_code_for_category("network"), "NW001");
        assert_eq!(support_code_for_category("credential_network"), "NW001");
        assert_eq!(support_code_for_category("remote_server"), "SV001");
        assert_eq!(support_code_for_category("service_authorization"), "AU001");
        assert_eq!(support_code_for_category("account_authorization"), "AU003");
        assert_eq!(support_code_for_category("credential_expired"), "AU002");
        assert_eq!(support_code_for_category("credential_connect"), "AU002");
        assert_eq!(support_code_for_category("slow_consumer"), "RT002");
        assert_eq!(support_code_for_category("command_subscription"), "RT003");
        assert_eq!(support_code_for_category("transfer_subscription"), "RT004");
        assert_eq!(support_code_for_category("event_publish"), "RT005");
        assert_eq!(support_code_for_category("heartbeat_publish"), "RT006");
        assert_eq!(support_code_for_category("state_publish"), "RT006");
        assert_eq!(support_code_for_category("web_bind"), "LC002");
        assert_eq!(support_code_for_category("revoked"), "PA001");
        assert_eq!(support_code_for_category("local"), "LC001");
        assert_eq!(support_code_for_category("mystery"), "LC999");
    }

    #[test]
    fn nats_health_handles_remaining_event_variants() {
        let health = NatsHealth::default();

        // A routine drop: self-healing reconnect, no failure episode.
        health.handle_event(&async_nats::Event::Disconnected);
        assert!(!health.needs_reconnect());

        // Slow-consumer and generic server errors each open an episode.
        health.handle_event(&async_nats::Event::SlowConsumer(7));
        health.handle_event(&async_nats::Event::ServerError(
            async_nats::ServerError::SlowConsumer(8),
        ));
        health.handle_event(&async_nats::Event::ServerError(
            async_nats::ServerError::Other("permissions".to_string()),
        ));

        // Remaining event shapes fall through the wildcard arm harmlessly.
        health.handle_event(&async_nats::Event::LameDuckMode);
        health.handle_event(&async_nats::Event::Draining);
    }

    #[test]
    fn runtime_active_classifies_every_phase() {
        for phase in [
            RemotePhase::Connecting,
            RemotePhase::Ready,
            RemotePhase::Reconnecting,
            RemotePhase::Refreshing,
        ] {
            let status = RemoteStatus {
                phase,
                reason: None,
                ..empty()
            };
            assert!(runtime_active(&status), "{phase:?} should be active");
        }
        for phase in [
            RemotePhase::Stopped,
            RemotePhase::Failed,
            RemotePhase::Revoked,
        ] {
            let status = RemoteStatus {
                phase,
                reason: None,
                ..empty()
            };
            assert!(!runtime_active(&status), "{phase:?} should be inactive");
        }
    }

    #[test]
    fn classify_nats_connect_error_covers_server_parse() {
        assert!(matches!(
            classify_nats_connect_error(
                async_nats::ConnectErrorKind::ServerParse,
                "bad url".to_string()
            ),
            crate::AppError::Message(_)
        ));
    }

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
        await_publish, await_publish_matching, init_store, jwt, nats_connect, nats_connect_once,
        now_secs, sign_in, unique, FakeNats, HomeGuard, MockPlatform,
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

    fn test_creds(pair_id: &str, nats_url: &str, expires_in: i64) -> pairing::PairingCreds {
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
        }
    }

    /// A RemoteState wired to a live fake NATS server, with placeholder tasks
    /// (pending forever) for the loops this test doesn't drive.
    async fn fake_state(nats: &FakeNats, pair_id: &str) -> RemoteState {
        let client = nats_connect(nats).await;
        let (event_tx, event_rx) = tokio::sync::mpsc::channel(EVENT_QUEUE_CAPACITY);
        let event_task = spawn_event_publisher(client.clone(), event_rx);
        RemoteState {
            security: secure::Transport::legacy_fixture(),
            generation_id: 1,
            client,
            nats_health: Arc::new(NatsHealth::default()),
            nats_url: nats.url().to_string(),
            pair_id: pair_id.to_string(),
            desktop_id: format!("desktop_{}", unique("rt")),
            desktop_public_key: "UPUBKEY".to_string(),
            bridge_instance_id: format!("bridge_{}", unique("rt")),
            event_tx,
            drop_counters: Arc::new(DropCounters::new()),
            event_task,
            cmd_task: tokio::spawn(std::future::pending()),
            transfer_task: tokio::spawn(std::future::pending()),
            heartbeat_task: tokio::spawn(std::future::pending()),
            refresh_task: tokio::spawn(std::future::pending()),
            web_task: None,
            web_url: None,
            web_lan_url: None,
            pairing_code: None,
            pairing_code_expires_at: None,
            pairing_confirmed: Arc::new(AtomicBool::new(true)),
        }
    }

    /// Install the state; tests MUST clean up via `stop()` so the next
    /// serialized test starts clean. Poison-tolerant: one test's failure must
    /// not cascade into every later lock.
    fn install_state(state: RemoteState) {
        let previous = SUPERVISOR
            .state
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .replace(state);
        assert!(previous.is_none(), "previous test leaked SUPERVISOR.state");
    }

    #[test]
    fn nats_health_classifies_recoverable_and_terminal_events() {
        let _home = HomeGuard::new("remote-nats-health");
        let health = NatsHealth::default();
        health.handle_event(&async_nats::Event::ClientError(
            async_nats::ClientError::MaxReconnects,
        ));
        assert!(health.needs_reconnect());
        assert!(!health.is_terminal());

        health.handle_event(&async_nats::Event::Connected);
        assert!(!health.needs_reconnect());

        // async-nats wraps reconnect-time authorization errors in the client
        // event rather than forwarding the server event. This must trigger a
        // generation refresh instead of leaving an apparently live bridge in
        // an unbounded reconnect/log loop.
        health.handle_event(&async_nats::Event::ClientError(
            async_nats::ClientError::Other("authorization violation".to_string()),
        ));
        health.handle_event(&async_nats::Event::ClientError(
            async_nats::ClientError::Other("authorization violation".to_string()),
        ));
        assert!(health.needs_reconnect());
        assert!(!health.is_terminal());
        assert!(health
            .authorization_rejection_logged
            .load(Ordering::Acquire));

        health.handle_event(&async_nats::Event::Connected);
        assert!(!health.needs_reconnect());
        assert!(!health
            .authorization_rejection_logged
            .load(Ordering::Acquire));

        health.handle_event(&async_nats::Event::ClientError(
            async_nats::ClientError::Other("IO error".to_string()),
        ));
        health.handle_event(&async_nats::Event::ClientError(
            async_nats::ClientError::Other("IO error".to_string()),
        ));
        assert!(!health.needs_reconnect());

        health.handle_event(&async_nats::Event::Connected);

        health
            .credential_expires_at
            .store(unix_timestamp() + 3600, Ordering::Release);
        // The same rejection on the generation created by the reactive
        // refresh is terminal service authorization.
        health
            .credential_was_refreshed
            .store(true, Ordering::Release);
        health.handle_event(&async_nats::Event::ServerError(
            async_nats::ServerError::AuthorizationViolation,
        ));
        health.handle_event(&async_nats::Event::Connected);
        assert!(
            health.is_terminal(),
            "reconnect must not hide an auth failure"
        );
    }

    #[test]
    fn power_events_preserve_intent_and_only_resume_rebuilds_a_generation() {
        assert_eq!(
            power_transition(PowerEvent::Suspend, true),
            PowerTransition {
                stop_generation: true,
                start_generation: false,
                rotate_epoch: false,
            }
        );
        assert_eq!(
            power_transition(PowerEvent::Resume, true),
            PowerTransition {
                stop_generation: false,
                start_generation: true,
                rotate_epoch: true,
            }
        );
        for event in [PowerEvent::Suspend, PowerEvent::Resume] {
            assert_eq!(
                power_transition(event, false),
                PowerTransition {
                    stop_generation: false,
                    start_generation: false,
                    rotate_epoch: false,
                }
            );
        }
    }

    #[test]
    fn failure_logs_are_bounded_across_alternating_episodes() {
        // The per-episode counter alone is insufficient: alternating category
        // names used to restart it indefinitely. This unique test category
        // exercises the process-level, 24-hour quota.
        let category = "test_alternating_failure_quota";
        for _ in 0..MAX_FAILURE_LOGS_PER_CATEGORY {
            assert!(permit_failure_log(category));
        }
        assert!(!permit_failure_log(category));
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
        state.web_task = Some(tokio::spawn(std::future::pending()));
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
            state.cmd_task = tokio::spawn(async {});
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
        // only after its own reconnect budget is exhausted.
        {
            let mut guard = SUPERVISOR.state.lock().unwrap();
            let state = guard.as_mut().unwrap();
            state.web_task = None;
            state.cmd_task = tokio::spawn(std::future::pending());
        }
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

        crate::remote_host::lean::set_enabled(true);
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
        crate::remote_host::lean::set_enabled(false);

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
        assert_eq!(body["sessionId"], json!(null), "sessionId is dropped");
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
            state.drop_counters.dropping.store(false, Ordering::Relaxed);
            state.drop_counters.dropped.store(0, Ordering::Relaxed);
            state.drop_counters.reports.store(0, Ordering::Relaxed);
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

    #[test]
    fn catalog_lane_keeps_status_and_approvals_but_not_token_traffic() {
        for event in [
            "agent_start",
            "agent_end",
            "approval_request",
            "approval_decision",
            "error",
            "session_name_changed",
            "provider_config_changed",
            "model_visibility_changed",
            "app_settings_changed",
            "skills_changed",
            "run_snapshot",
        ] {
            assert!(is_catalog_event(event), "{event}");
        }
        for event in [
            "text_chunk",
            "thinking_delta",
            "tool_delta",
            "toolcall_delta",
            "tool_end",
        ] {
            assert!(!is_catalog_event(event), "{event}");
        }
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
        *READINESS_PAUSE.lock().unwrap() = Some((entered_tx, release_rx));
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
        assert!(SUPERVISOR.bridge_shared.lock().unwrap().is_none());
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

    #[test]
    fn expired_rotated_credential_is_refreshable_not_a_configuration_failure() {
        let health = NatsHealth::default();
        health
            .credential_was_refreshed
            .store(true, Ordering::Release);
        health
            .credential_expires_at
            .store(unix_timestamp().saturating_sub(1), Ordering::Release);
        health.mark_authorization_rejected();
        assert!(health.needs_reconnect());
        assert!(!health.is_terminal());
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
        let error = establish().await.unwrap_err();
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
    fn only_transient_remote_start_failures_arm_background_recovery() {
        let network = RemoteStatus {
            phase: RemotePhase::Reconnecting,
            reason: Some(RemoteFailureReason::Network),
            ..empty()
        };
        let server = RemoteStatus {
            phase: RemotePhase::Reconnecting,
            reason: Some(RemoteFailureReason::RemoteServer),
            ..empty()
        };
        let revoked = RemoteStatus {
            phase: RemotePhase::Revoked,
            reason: Some(RemoteFailureReason::CredentialRevoked),
            ..empty()
        };
        assert!(retryable_start_status(&network));
        assert!(retryable_start_status(&server));
        assert!(!retryable_start_status(&revoked));
        assert!(!retryable_start_status(&RemoteStatus {
            phase: RemotePhase::Ready,
            reason: Some(RemoteFailureReason::Network),
            ..empty()
        }));
    }

    #[test]
    fn initial_nats_connect_errors_distinguish_authorization_from_network() {
        for kind in [
            async_nats::ConnectErrorKind::Authentication,
            async_nats::ConnectErrorKind::AuthorizationViolation,
        ] {
            assert!(matches!(
                classify_nats_connect_error(kind, "rejected".to_string()),
                crate::AppError::RemoteAuthorization(_)
            ));
        }
        for kind in [
            async_nats::ConnectErrorKind::Dns,
            async_nats::ConnectErrorKind::TimedOut,
            async_nats::ConnectErrorKind::Io,
        ] {
            assert!(matches!(
                classify_nats_connect_error(kind, "offline".to_string()),
                crate::AppError::RemoteTransport(_)
            ));
        }
    }

    #[test]
    fn desktop_reconnect_backoff_matches_the_shared_policy() {
        assert_eq!(reconnect_delay(0, 0.5), Duration::from_secs(1));
        assert_eq!(reconnect_delay(1, 0.5), Duration::from_secs(2));
        assert_eq!(reconnect_delay(2, 0.5), Duration::from_secs(4));
        assert_eq!(reconnect_delay(4, 0.5), Duration::from_secs(16));
        assert_eq!(reconnect_delay(5, 0.5), Duration::from_secs(30));
        assert!(reconnect_delay(20, 1.0) <= Duration::from_secs(30));
    }

    #[test]
    fn bridge_shared_state_survives_generation_swaps_but_rotates_epoch() {
        let _home = HomeGuard::new("remote-shared-generation");
        *SUPERVISOR.bridge_shared.lock().unwrap() = None;
        let creds = test_creds("pair_shared", "nats://127.0.0.1:4222", 3600);
        let first = shared_runtime(&creds, true, false);
        let same_credential_epoch = shared_runtime(&creds, true, false);
        assert!(Arc::ptr_eq(
            &first.reply_slots,
            &same_credential_epoch.reply_slots
        ));
        assert!(Arc::ptr_eq(
            &first.pairing_confirmed,
            &same_credential_epoch.pairing_confirmed
        ));
        assert_eq!(
            first.bridge_instance_id,
            same_credential_epoch.bridge_instance_id
        );

        let rebuilt = shared_runtime(&creds, true, true);
        assert!(Arc::ptr_eq(&first.reply_slots, &rebuilt.reply_slots));
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
        assert!(!Arc::ptr_eq(&first.reply_slots, &reissued.reply_slots));
        assert!(!Arc::ptr_eq(
            &first.pairing_confirmed,
            &reissued.pairing_confirmed
        ));
        assert!(!reissued.pairing_confirmed.load(Ordering::Acquire));
        assert_ne!(first.bridge_instance_id, reissued.bridge_instance_id);
        *SUPERVISOR.bridge_shared.lock().unwrap() = None;
    }

    #[test]
    fn runtime_failure_budget_uses_a_ten_minute_window() {
        let _home = HomeGuard::new("remote-runtime-budget");
        SUPERVISOR
            .runtime_failure_window_started
            .store(0, Ordering::Release);
        SUPERVISOR
            .runtime_reconnect_attempts
            .store(0, Ordering::Release);
        assert_eq!(record_runtime_failure(1_000), 1);
        assert_eq!(record_runtime_failure(2_000), 2);
        assert_eq!(record_runtime_failure(3_000), 3);
        assert_eq!(record_runtime_failure(4_000), 4);
        assert_eq!(
            record_runtime_failure(1_000 + RUNTIME_FAILURE_WINDOW_MS + 1),
            1
        );
        SUPERVISOR
            .runtime_failure_window_started
            .store(0, Ordering::Release);
        SUPERVISOR
            .runtime_reconnect_attempts
            .store(0, Ordering::Release);
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
        let handshake = commands::HandshakeState::new(
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
        let handshake = commands::HandshakeState::new(
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
            commands::HandshakeState::new(creds, confirmed.clone(), "bridge_rev".into());
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
            commands::HandshakeState::new(creds, confirmed.clone(), "bridge_account".into());
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
            commands::HandshakeState::new(creds, confirmed.clone(), "bridge_retry".into());
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
        let handshake = commands::HandshakeState::new(
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
        let handshake = commands::HandshakeState::new(
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
        let handshake = commands::HandshakeState::new(
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
            commands::HandshakeState::new(creds, confirmed.clone(), "bridge_sick".into());
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
        *SUPERVISOR.bridge_shared.lock().unwrap() = None;
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
            commands::HandshakeState::new(creds, confirmed.clone(), "bridge_terminal".into());
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
        let creds = test_creds(&pair, nats.url(), 30);
        pairing::save_creds(&creds).unwrap();
        platform.respond_refresh(nats2.url());

        let state = fake_state(&nats, &pair).await;
        let confirmed = state.pairing_confirmed.clone();
        install_state(state);
        let handshake =
            commands::HandshakeState::new(creds, confirmed.clone(), "bridge_ready".into());
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
    /// died. In the test build it is deliberately a no-op: it must not arm the
    /// singleton flag or record a failure attempt, because the tests that drive
    /// the supervisor own that state themselves.
    #[tokio::test]
    async fn spawning_a_runtime_reconnect_is_a_noop_under_test() {
        let _home = HomeGuard::new("remote-runtime-reconnect-noop");
        SUPERVISOR
            .runtime_reconnect_running
            .store(false, Ordering::Release);
        SUPERVISOR
            .runtime_reconnect_attempts
            .store(0, Ordering::Release);
        spawn_runtime_reconnect();
        assert!(
            !SUPERVISOR.runtime_reconnect_running.load(Ordering::Acquire),
            "the test build must not arm the real reconnect worker"
        );
        assert_eq!(
            SUPERVISOR
                .runtime_reconnect_attempts
                .load(Ordering::Acquire),
            0
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
                crate::remote::protocol::IncomingCmd {
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
