use super::*;

#[cfg(test)]
mod watchdog_tests {
    use super::{plan_active_run_reconciliation, ActiveRunAction, WATCHDOG_ORPHAN_SECS};

    fn state(json: &str) -> serde_json::Value {
        serde_json::from_str(json).unwrap()
    }

    #[test]
    fn attaches_when_agent_still_running_this_run() {
        let action = plan_active_run_reconciliation(
            &state(r#"{"isStreaming": true, "activeRun": {"runId": "run-1"}}"#),
            "run-1",
            120,
        );
        assert_eq!(action, ActiveRunAction::Attach);
    }

    #[test]
    fn does_not_attach_for_a_different_active_run() {
        let action = plan_active_run_reconciliation(
            &state(r#"{"isStreaming": true, "activeRun": {"runId": "run-2"}}"#),
            "run-1",
            120,
        );
        assert_eq!(action, ActiveRunAction::Skip);
    }

    #[test]
    fn mirrors_durable_completed_marker() {
        let action = plan_active_run_reconciliation(
            &state(r#"{"requestedRun": {"status": "completed"}}"#),
            "run-1",
            120,
        );
        assert_eq!(
            action,
            ActiveRunAction::SettleTerminal {
                agent_state: "completed".to_string(),
                error: None,
            }
        );
    }

    #[test]
    fn mirrors_error_marker_with_its_message() {
        let action = plan_active_run_reconciliation(
            &state(r#"{"requestedRun": {"status": "error", "error": "boom"}}"#),
            "run-1",
            120,
        );
        assert_eq!(
            action,
            ActiveRunAction::SettleTerminal {
                agent_state: "error".to_string(),
                error: Some("boom".to_string()),
            }
        );
    }

    #[test]
    fn settles_interrupted_by_restart() {
        let action = plan_active_run_reconciliation(
            &state(r#"{"interruptedRun": {"runId": "run-1"}}"#),
            "run-1",
            120,
        );
        assert_eq!(action, ActiveRunAction::SettleInterrupted);
    }

    #[test]
    fn skips_young_runs_without_markers() {
        let action =
            plan_active_run_reconciliation(&state(r#"{"isStreaming": false}"#), "run-1", 60);
        assert_eq!(action, ActiveRunAction::Skip);
    }

    #[test]
    fn orphans_old_runs_without_markers() {
        let action = plan_active_run_reconciliation(
            &state(r#"{"isStreaming": false}"#),
            "run-1",
            WATCHDOG_ORPHAN_SECS,
        );
        assert_eq!(action, ActiveRunAction::SettleOrphaned);
    }

    #[test]
    fn skips_a_run_the_agent_has_queued() {
        let action = plan_active_run_reconciliation(
            &state(
                r#"{"isStreaming": true, "activeRun": {"runId": "run-0"},
                   "queuedRuns": [{"runId": "run-1", "queuePosition": 1}]}"#,
            ),
            "run-1",
            WATCHDOG_ORPHAN_SECS,
        );
        assert_eq!(
            action,
            ActiveRunAction::Skip,
            "a queued run is alive — never orphan-settle it"
        );
    }
}

#[cfg(test)]
mod events_paging_tests {
    use super::next_events_cursor;
    use future_rpc::payloads::EventsSincePayload;
    use serde_json::json;

    fn replay_page(value: serde_json::Value) -> EventsSincePayload {
        serde_json::from_value(value).expect("replay page")
    }

    #[test]
    fn stops_when_the_page_reports_no_tail() {
        // hasMore absent (legacy server) or false ends the loop.
        let page = replay_page(json!({"events": [{"idx": 3}]}));
        assert_eq!(next_events_cursor(&page, -1), None);
        let page = replay_page(json!({"events": [{"idx": 3}], "hasMore": false}));
        assert_eq!(next_events_cursor(&page, -1), None);
    }

    #[test]
    fn advances_to_the_last_event_idx_while_has_more() {
        let page = replay_page(json!({"events": [{"idx": 3}, {"idx": 7}], "hasMore": true}));
        assert_eq!(next_events_cursor(&page, -1), Some(7));
        assert_eq!(next_events_cursor(&page, 7), None); // idx must advance
    }

    #[test]
    fn malformed_has_more_pages_terminate_instead_of_looping() {
        // No events, no idx, or a non-advancing idx would re-request the same
        // cursor forever — the loop must bail.
        assert_eq!(
            next_events_cursor(&replay_page(json!({"hasMore": true})), -1),
            None
        );
        assert_eq!(
            next_events_cursor(&replay_page(json!({"events": [], "hasMore": true})), -1),
            None
        );
        assert_eq!(
            next_events_cursor(
                &replay_page(json!({"events": [{"idx": 5}], "hasMore": true})),
                5,
            ),
            None
        );
    }
}

#[cfg(test)]
mod wire_decode_tests {
    use future_rpc::proto;

    fn rpc_response(command: &str, data: &str) -> proto::RpcResponse {
        proto::RpcResponse {
            id: "req".to_string(),
            r#type: "response".to_string(),
            command: command.to_string(),
            success: true,
            data: data.to_string(),
            ..Default::default()
        }
    }

    /// The dual-write guarantee as seen from the GUI: a response carrying BOTH
    /// the typed payload and the JSON string decodes to exactly the JSON, so
    /// deep reads behave identically against old and new agents.
    #[test]
    fn typed_and_data_decode_to_the_same_value() {
        let data = r#"{"models":[{"id":"m","label":"M","provider":"p","supportsImages":false,"thinkingLevel":"off","contextWindow":1,"isDefault":true,"description":null,"descriptionEn":null,"recommended":false}],"defaultModel":"m","isScoped":false}"#;
        let payload = future_rpc::encode::response_payload("list_models", &data_value(data))
            .expect("list_models encodes");
        let mut resp = rpc_response("list_models", data);
        resp.payload = Some(payload);
        assert_eq!(
            future_rpc::decode::response_data(&resp),
            data_value(data),
            "typed decode must match the dual-written JSON"
        );
    }

    /// Old agent (no typed payload) still decodes through the JSON fallback.
    #[test]
    fn data_only_falls_back_to_json() {
        let data = r#"{"sessionId":"s1"}"#;
        let resp = rpc_response("get_state", data);
        assert_eq!(future_rpc::decode::response_data(&resp), data_value(data));
    }

    /// Event byte-stability during the migration window: while the agent
    /// dual-writes `data`, the canonical event payload is the original string
    /// verbatim — persistence and the NATS mirror must not drift.
    #[test]
    fn event_payload_prefers_dual_written_data() {
        let data = r#"{"type":"tool_end","tool_id":"c1","text":"ok"}"#.to_string();
        let payload = future_rpc::encode::event_payload("tool_end", &data);
        let event = proto::StreamEvent {
            r#type: "tool_end".to_string(),
            data: data.clone(),
            payload,
            ..Default::default()
        };
        assert_eq!(future_rpc::decode::event_data_json(&event), data);
    }

    /// Once `data` is retired, the typed payload reconstructs the canonical
    /// shape (the wire JSON minus the redundant injected `type` key).
    #[test]
    fn typed_event_reconstructs_without_data() {
        let data = r#"{"type":"tool_end","tool_id":"c1","text":"ok"}"#;
        let payload = future_rpc::encode::event_payload("tool_end", data).expect("encodes");
        let event = proto::StreamEvent {
            r#type: "tool_end".to_string(),
            data: String::new(),
            payload: Some(payload),
            ..Default::default()
        };
        let reconstructed: serde_json::Value =
            serde_json::from_str(&future_rpc::decode::event_data_json(&event)).unwrap();
        assert_eq!(
            reconstructed,
            serde_json::json!({ "text": "ok", "tool_id": "c1" })
        );
    }

    fn data_value(data: &str) -> serde_json::Value {
        serde_json::from_str(data).unwrap()
    }
}

#[cfg(test)]
mod bridge_tests {
    use super::test_support::{
        break_home, get_state_payload, mock_agent, restore_home, seed_run, seed_thread,
        seed_workspace, Reply, TestHome,
    };
    use super::*;

    // ── simple command wrappers ───────────────────────────────────────

    #[tokio::test]
    async fn session_read_wrappers_decode_or_default() {
        let mock = mock_agent();

        mock.push_data(
            "get_messages",
            serde_json::json!({"messages": [{"role": "user"}]}),
        );
        let value = get_session_messages("sess-1".to_string())
            .await
            .expect("messages");
        assert_eq!(value["messages"][0]["role"], "user");

        // Empty data payloads fall back to empty envelopes.
        mock.push("get_messages", Reply::Data(String::new()));
        let value = get_session_messages("sess-1".to_string())
            .await
            .expect("messages");
        assert_eq!(value, serde_json::json!({"messages": []}));

        mock.push_typed_data(
            "get_session_entries",
            serde_json::json!({"entries": [{
                "id":"e1","kind":"assistant","role":"assistant","createdAtMs":1000,"blocks":[{"kind":"text","text":"world"}]
            }]}),
        );
        let value = get_session_entries("sess-1".to_string())
            .await
            .expect("typed entries");
        assert_eq!(value["entries"][0]["id"], "e1");

        mock.push_typed_data(
            "get_session_entries",
            serde_json::json!({
                "entries": [{
                    "id":"e2","kind":"user","role":"user","createdAtMs":1000,"blocks":[{"kind":"text","text":"next"}]
                }],
                "hasMore": true,
                "nextOffset": 1
            }),
        );
        mock.push_typed_data(
            "get_session_entries",
            serde_json::json!({"entries": [{
                "id":"e3","kind":"assistant","role":"assistant","createdAtMs":1000,"blocks":[{"kind":"text","text":"done"}]
            }]}),
        );
        let value = get_session_entries("sess-1".to_string())
            .await
            .expect("paged entries");
        assert_eq!(value["entries"].as_array().expect("entries").len(), 2);
        let requests = mock.requests_of("get_session_entries");
        let last_two = &requests[requests.len() - 2..];
        assert_eq!(last_two[0].offset, Some(0));
        assert_eq!(last_two[1].offset, Some(1));

        mock.push("get_session_entries", Reply::Data(String::new()));
        assert!(get_session_entries("sess-1".to_string()).await.is_err());

        mock.push_data("get_state", serde_json::json!({"isStreaming": true}));
        let value = get_session_state("sess-1".to_string())
            .await
            .expect("state");
        assert_eq!(value["isStreaming"], true);
        mock.push_typed_data("get_state", get_state_payload("sess-1", true));
        let value = get_session_state("sess-1".to_string())
            .await
            .expect("typed state");
        assert_eq!(value["sessionId"], "sess-1");
        assert_eq!(value["thinkingLevel"], "medium");
        mock.push("get_state", Reply::Data(String::new()));
        let value = get_session_state("sess-1".to_string())
            .await
            .expect("state");
        assert_eq!(value, serde_json::json!({}));

        mock.push_data("list_models", serde_json::json!({"models": [{"id": "m"}]}));
        let value = get_available_models().await.expect("models");
        assert_eq!(value["models"][0]["id"], "m");
        mock.push_typed_data(
            "list_models",
            serde_json::json!({
                "models": [{
                    "id": "typed-model", "label": "Typed", "provider": "future",
                    "supportsImages": false, "thinkingLevel": "medium",
                    "contextWindow": 1000, "isDefault": true, "description": null,
                    "descriptionEn": null, "recommended": true
                }],
                "defaultModel": "future/typed-model",
                "isScoped": false,
                "builtinProviders": {}
            }),
        );
        let value = get_available_models().await.expect("typed models");
        assert_eq!(value["models"][0]["id"], "typed-model");
        mock.push("list_models", Reply::Data(String::new()));
        let value = get_available_models().await.expect("models");
        assert_eq!(value, serde_json::json!({"models": []}));
    }

    #[tokio::test]
    async fn session_read_wrappers_surface_failures() {
        let mock = mock_agent();

        mock.push("get_messages", Reply::Status(tonic::Code::Internal, "boom"));
        let error = get_session_messages("s".to_string())
            .await
            .expect_err("transport");
        assert!(error.to_string().contains("get_messages failed"), "{error}");
        mock.push("get_messages", Reply::Reject("bad".to_string()));
        let error = get_session_messages("s".to_string())
            .await
            .expect_err("reject");
        assert_eq!(error.to_string(), "bad");

        mock.push(
            "get_session_entries",
            Reply::Status(tonic::Code::Internal, "boom"),
        );
        let error = get_session_entries("s".to_string())
            .await
            .expect_err("transport");
        assert!(
            error.to_string().contains("get_session_entries failed"),
            "{error}"
        );
        mock.push("get_session_entries", Reply::Reject(String::new()));
        let error = get_session_entries("s".to_string())
            .await
            .expect_err("reject");
        assert_eq!(error.to_string(), "get_session_entries returned an error");

        mock.push("get_state", Reply::Status(tonic::Code::Internal, "boom"));
        let error = get_session_state("s".to_string())
            .await
            .expect_err("transport");
        assert!(error.to_string().contains("get_state failed"), "{error}");
        mock.push("get_state", Reply::Reject("bad".to_string()));
        let error = get_session_state("s".to_string())
            .await
            .expect_err("reject");
        assert_eq!(error.to_string(), "bad");

        mock.push("list_models", Reply::Status(tonic::Code::Internal, "boom"));
        let error = get_available_models().await.expect_err("transport");
        assert!(
            error.to_string().contains("get_available_models failed"),
            "{error}"
        );
        mock.push("list_models", Reply::Reject("bad".to_string()));
        let error = get_available_models().await.expect_err("reject");
        assert_eq!(error.to_string(), "bad");
    }

    #[tokio::test]
    async fn session_setter_wrappers() {
        let mock = mock_agent();

        mock.push("set_model", Reply::Data("{}".to_string()));
        set_session_model("sess-1".to_string(), "future/k3".to_string())
            .await
            .expect("set model");
        let request = &mock.requests_of("set_model")[0];
        assert_eq!(request.model_id, "future/k3");
        assert_eq!(request.session_id, "sess-1");
        mock.push("set_model", Reply::Status(tonic::Code::Internal, "boom"));
        let error = set_session_model("s".to_string(), "m".to_string())
            .await
            .expect_err("transport");
        assert!(error.to_string().contains("set_model failed"), "{error}");
        mock.push("set_model", Reply::Reject("bad".to_string()));
        let error = set_session_model("s".to_string(), "m".to_string())
            .await
            .expect_err("reject");
        assert_eq!(error.to_string(), "bad");

        mock.push("set_default_model", Reply::Data("{}".to_string()));
        set_default_model("future/k3".to_string())
            .await
            .expect("default");
        assert_eq!(
            mock.requests_of("set_default_model")[0].model_id,
            "future/k3"
        );
        mock.push(
            "set_default_model",
            Reply::Status(tonic::Code::Internal, "boom"),
        );
        let error = set_default_model("m".to_string())
            .await
            .expect_err("transport");
        assert!(
            error.to_string().contains("set_default_model failed"),
            "{error}"
        );
        mock.push("set_default_model", Reply::Reject("bad".to_string()));
        let error = set_default_model("m".to_string())
            .await
            .expect_err("reject");
        assert_eq!(error.to_string(), "bad");

        mock.push("set_thinking_level", Reply::Data("{}".to_string()));
        set_session_thinking_level("sess-1".to_string(), "high".to_string())
            .await
            .expect("thinking");
        assert_eq!(mock.requests_of("set_thinking_level")[0].level, "high");
        mock.push(
            "set_thinking_level",
            Reply::Status(tonic::Code::Internal, "boom"),
        );
        let error = set_session_thinking_level("s".to_string(), "l".to_string())
            .await
            .expect_err("transport");
        assert!(
            error.to_string().contains("set_thinking_level failed"),
            "{error}"
        );
        mock.push("set_thinking_level", Reply::Reject("bad".to_string()));
        let error = set_session_thinking_level("s".to_string(), "l".to_string())
            .await
            .expect_err("reject");
        assert_eq!(error.to_string(), "bad");
    }

    #[tokio::test]
    async fn rename_session_mirrors_into_the_store() {
        let home = TestHome::new("bridge-rename");
        let mock = mock_agent();
        let workspace = seed_workspace(home.path(), "ws");
        let thread = seed_thread(&workspace.id, Some("sess-1"));

        mock.push("set_session_name", Reply::Data("{}".to_string()));
        rename_session("sess-1".to_string(), "New Title".to_string())
            .await
            .expect("rename");
        assert_eq!(mock.requests_of("set_session_name")[0].name, "New Title");
        assert_eq!(
            crate::store::get_thread(&thread.id)
                .expect("thread")
                .expect("exists")
                .title,
            "New Title",
            "store mirror keeps the sidebar in sync"
        );

        // Unknown session: the agent call still succeeds, no mirror.
        mock.push("set_session_name", Reply::Data("{}".to_string()));
        rename_session("sess-unknown".to_string(), "T".to_string())
            .await
            .expect("rename");

        mock.push(
            "set_session_name",
            Reply::Status(tonic::Code::Internal, "boom"),
        );
        let error = rename_session("sess-1".to_string(), "T".to_string())
            .await
            .expect_err("transport");
        assert!(
            error.to_string().contains("set_session_name failed"),
            "{error}"
        );
        mock.push("set_session_name", Reply::Reject("bad".to_string()));
        let error = rename_session("sess-1".to_string(), "T".to_string())
            .await
            .expect_err("reject");
        assert_eq!(error.to_string(), "bad");
    }

    #[tokio::test]
    async fn reload_agent_credentials_tolerates_a_down_agent() {
        let mock = mock_agent();

        mock.push("reload_auth", Reply::Data("{}".to_string()));
        reload_agent_credentials().await.expect("reload");
        assert_eq!(mock.requests_of("reload_auth").len(), 1);

        // Unreachable agent (unparseable endpoint) → Ok.
        let prev = std::env::var("FUTURE_AGENT_GRPC_ADDR").expect("mock addr");
        std::env::set_var("FUTURE_AGENT_GRPC_ADDR", "http://[::1");
        reload_agent_credentials().await.expect("down is ok");
        std::env::set_var("FUTURE_AGENT_GRPC_ADDR", prev);

        // Transport failure surfaces via map_rpc_error; rejection via the body.
        mock.push("reload_auth", Reply::Status(tonic::Code::Internal, "boom"));
        let error = reload_agent_credentials().await.expect_err("transport");
        assert!(matches!(error, crate::AppError::Message(_)), "{error}");
        mock.push("reload_auth", Reply::Reject("bad".to_string()));
        let error = reload_agent_credentials().await.expect_err("reject");
        assert_eq!(error.to_string(), "bad");
    }

    #[tokio::test]
    async fn sync_future_models_variants() {
        let mock = mock_agent();

        mock.push_data(
            "sync_future_models",
            serde_json::json!({"synced": true, "modelCount": 7, "revision": 11}),
        );
        let result = sync_future_models().await.expect("sync");
        assert!(result.synced);
        assert_eq!(result.model_count, 7);
        assert_eq!(result.revision, 11);

        // Down agent → zeroed result.
        let prev = std::env::var("FUTURE_AGENT_GRPC_ADDR").expect("mock addr");
        std::env::set_var("FUTURE_AGENT_GRPC_ADDR", "http://[::1");
        let result = sync_future_models().await.expect("down is zeroed");
        std::env::set_var("FUTURE_AGENT_GRPC_ADDR", prev);
        assert!(!result.synced);
        assert_eq!(result.model_count, 0);
        assert_eq!(result.revision, 0);

        mock.push(
            "sync_future_models",
            Reply::Status(tonic::Code::Unavailable, "gone"),
        );
        let error = sync_future_models().await.expect_err("transport");
        assert!(
            matches!(error, crate::AppError::AgentUnavailable(_)),
            "{error}"
        );
        mock.push("sync_future_models", Reply::Reject("bad".to_string()));
        let error = sync_future_models().await.expect_err("reject");
        assert_eq!(error.to_string(), "bad");
        mock.push_data("sync_future_models", serde_json::json!({"synced": "yes"}));
        let error = sync_future_models().await.expect_err("invalid");
        assert!(error.to_string().contains("invalid sync result"), "{error}");
    }

    #[tokio::test]
    async fn run_snapshot_validates_identity_and_only_falls_back_for_explicit_unsupported() {
        let mock = mock_agent();
        let snapshot = serde_json::json!({"runSnapshot":true,"watermark":100,"events":[],
            "projection":{"runId":"r","cursor":100,"events":[{"type":"agent_start","idx":0}]}});
        mock.push_data("get_run_snapshot", snapshot.clone());
        assert_eq!(
            get_run_snapshot("s".into(), "r".into()).await.unwrap(),
            Some(snapshot.clone())
        );
        assert_eq!(mock.requests_of("get_run_snapshot")[0].run_id, "r");
        mock.push(
            "get_run_snapshot",
            Reply::Reject("unknown command: get_run_snapshot".into()),
        );
        assert!(get_run_snapshot("s".into(), "r".into())
            .await
            .unwrap()
            .is_none());
        mock.push(
            "get_run_snapshot",
            Reply::Reject("storage unreadable".into()),
        );
        assert!(get_run_snapshot("s".into(), "r".into()).await.is_err());
        mock.push(
            "get_run_snapshot",
            Reply::Status(tonic::Code::Unavailable, "offline"),
        );
        assert!(get_run_snapshot("s".into(), "r".into()).await.is_err());
        let mut wrong_run = snapshot.clone();
        wrong_run["projection"]["runId"] = serde_json::json!("other");
        mock.push_data("get_run_snapshot", wrong_run);
        assert!(get_run_snapshot("s".into(), "r".into()).await.is_err());
        let mut wrong_cursor = snapshot;
        wrong_cursor["watermark"] = serde_json::json!(101);
        mock.push_data("get_run_snapshot", wrong_cursor);
        assert!(get_run_snapshot("s".into(), "r".into()).await.is_err());
    }

    // ── get_events_since paging ───────────────────────────────────────

    #[tokio::test]
    async fn events_since_merges_pages_and_strips_has_more() {
        let mock = mock_agent();
        mock.push_data(
            "get_events_since",
            serde_json::json!({"events": [{"idx": 1}, {"idx": 2}], "hasMore": true}),
        );
        mock.push_data(
            "get_events_since",
            serde_json::json!({"events": [{"idx": 3}], "hasMore": false}),
        );
        let merged = get_events_since("sess-1".to_string(), "run-1".to_string(), -1)
            .await
            .expect("merged");
        let idxs: Vec<i64> = merged["events"]
            .as_array()
            .expect("array")
            .iter()
            .map(|event| event["idx"].as_i64().unwrap_or_default())
            .collect();
        assert_eq!(idxs, vec![1, 2, 3]);
        assert!(
            merged.get("hasMore").is_none(),
            "merged envelope drops hasMore"
        );

        let requests = mock.requests_of("get_events_since");
        assert_eq!(requests.len(), 2);
        assert_eq!(requests[0].since_idx, -1);
        assert_eq!(requests[0].max_events, EVENTS_PAGE_SIZE);
        assert_eq!(requests[1].since_idx, 2, "paging resumes at the last idx");
        assert_eq!(requests[0].run_id, "run-1");
    }

    #[tokio::test]
    async fn events_since_remote_page_is_bounded_and_preserves_has_more() {
        let mock = mock_agent();
        mock.push_typed_data(
            "get_events_since",
            serde_json::json!({"runId": "r", "events": [{"idx": 100}], "hasMore": true}),
        );
        let page = get_events_since_page("s".into(), "r".into(), 99, 100)
            .await
            .unwrap();
        assert_eq!(page["hasMore"], true);
        assert_eq!(page["events"][0]["idx"], 100);
        let requests = mock.requests_of("get_events_since");
        assert_eq!(
            requests.len(),
            1,
            "must not drain the remaining Agent pages"
        );
        assert_eq!(requests[0].max_events, 100);
        assert_eq!(requests[0].since_idx, 99);
        assert_eq!(requests[0].run_id, "r");
    }

    #[tokio::test]
    async fn events_since_payload_decodes_typed_pages_without_json_reparse() {
        let mock = mock_agent();
        mock.push_typed_data(
            "get_events_since",
            serde_json::json!({
                "runId": "run-typed",
                "events": [
                    {
                        "type": "thinking_delta",
                        "data": "{\"text\":\"reason\"}",
                        "runId": "run-typed",
                        "idx": 0
                    },
                    {
                        "type": "text_chunk",
                        "data": "{\"text\":\"answer\"}",
                        "runId": "run-typed",
                        "idx": 1
                    }
                ],
                "truncated": false,
                "hasMore": false
            }),
        );

        let replay =
            get_events_since_payload("session-typed".to_string(), "run-typed".to_string(), -1)
                .await
                .expect("typed replay");

        assert_eq!(replay.run_id, "run-typed");
        assert_eq!(replay.events.len(), 2);
        assert_eq!(replay.events[0].event_type, "thinking_delta");
        assert_eq!(replay.events[0].data, r#"{"text":"reason"}"#);
        assert_eq!(replay.events[1].event_type, "text_chunk");
        assert_eq!(replay.events[1].data, r#"{"text":"answer"}"#);
        assert!(!replay.has_more);
    }

    #[tokio::test]
    async fn events_since_payload_rejects_cross_run_response() {
        let mock = mock_agent();
        mock.push_typed_data(
            "get_events_since",
            serde_json::json!({
                "runId": "wrong-run",
                "events": [],
                "truncated": false
            }),
        );

        let error =
            get_events_since_payload("session-typed".to_string(), "expected-run".to_string(), -1)
                .await
                .expect_err("cross-run replay must fail");
        assert!(error.to_string().contains("wrong-run"), "{error}");
        assert!(error.to_string().contains("expected-run"), "{error}");
    }

    #[tokio::test]
    async fn events_since_empty_and_terminating_pages() {
        let mock = mock_agent();

        // Empty data payload → empty envelope.
        mock.push("get_events_since", Reply::Data(String::new()));
        let merged = get_events_since("s".to_string(), "r".to_string(), 0)
            .await
            .expect("empty");
        assert_eq!(
            merged,
            serde_json::json!({"runId": "r", "events": [], "truncated": false})
        );

        // A hasMore page whose idx does not advance terminates the loop.
        mock.push_data(
            "get_events_since",
            serde_json::json!({"events": [{"idx": 5}], "hasMore": true}),
        );
        let merged = get_events_since("s".to_string(), "r".to_string(), 5)
            .await
            .expect("terminates");
        assert_eq!(merged["events"].as_array().expect("array").len(), 1);

        // Transport failure and rejection.
        mock.push(
            "get_events_since",
            Reply::Status(tonic::Code::Internal, "boom"),
        );
        let error = get_events_since("s".to_string(), "r".to_string(), 0)
            .await
            .expect_err("transport");
        assert!(
            error.to_string().contains("get_events_since failed"),
            "{error}"
        );

        // OutOfRange (the agent rejecting its own oversized tail) gets the
        // tailored rebuild-and-restart message.
        mock.push(
            "get_events_since",
            Reply::Status(tonic::Code::OutOfRange, "too big"),
        );
        let error = get_events_since("s".to_string(), "r".to_string(), 0)
            .await
            .expect_err("out-of-range");
        assert!(
            error.to_string().contains("exceeded the 32 MiB gRPC cap"),
            "{error}"
        );
        mock.push("get_events_since", Reply::Reject("stale run".to_string()));
        let error = get_events_since("s".to_string(), "r".to_string(), 0)
            .await
            .expect_err("reject");
        assert_eq!(error.to_string(), "stale run");
    }

    // ── provision_agent_session ───────────────────────────────────────

    #[tokio::test]
    async fn provision_creates_and_records_the_agent_session() {
        let home = TestHome::new("bridge-provision");
        let mock = mock_agent();
        let workspace = seed_workspace(home.path(), "ws");
        let thread = seed_thread(&workspace.id, None);

        mock.push_data("new_session", serde_json::json!({"sessionId": "sess-prov"}));
        let session_id = provision_agent_session(
            &thread.id,
            Some("future/k3".to_string()),
            Some("high".to_string()),
        )
        .await
        .expect("provision");
        assert_eq!(session_id, "sess-prov");
        assert_eq!(
            crate::store::get_thread(&thread.id)
                .expect("thread")
                .expect("exists")
                .agent_session_id
                .as_deref(),
            Some("sess-prov")
        );
        let created = &mock.requests_of("new_session")[0];
        assert_eq!(created.cwd, workspace.path);
        assert_eq!(created.model_id, "future/k3");
        assert_eq!(created.level, "high");
        assert_eq!(
            mock.requests_of("set_permission_level")[0].level,
            "workspace"
        );
        assert_eq!(mock.requests_of("set_sandbox_policy").len(), 1);

        // Unknown thread → workspace path resolution fails first.
        let error = provision_agent_session("no-such-thread", None, None)
            .await
            .expect_err("missing thread");
        assert_eq!(error.to_string(), "Thread could not be loaded.");
    }

    // ── delete outbox ─────────────────────────────────────────────────

    fn enqueue_delete(home: &TestHome, session_id: &str) {
        // Distinct, increasing requested_at keeps delivery order deterministic.
        static SEQ: std::sync::atomic::AtomicI64 = std::sync::atomic::AtomicI64::new(1);
        let conn =
            rusqlite::Connection::open(home.path().join(".future/app/app.db")).expect("open db");
        conn.execute(
            "INSERT INTO agent_delete_outbox(session_id, requested_at, attempts) VALUES (?1, ?2, 0)",
            rusqlite::params![
                session_id,
                SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            ],
        )
        .expect("enqueue");
    }

    #[tokio::test]
    async fn delete_outbox_delivers_acknowledges_and_notes_failures() {
        let home = TestHome::new("bridge-outbox");
        let mock = mock_agent();

        // Nothing pending → no traffic.
        reconcile_delete_outbox().await;
        assert!(mock.requests().is_empty());

        // Successful delivery acknowledges the row.
        enqueue_delete(&home, "sess-del-ok");
        mock.push("delete_session", Reply::Data("{}".to_string()));
        reconcile_delete_outbox().await;
        assert!(
            !crate::store::is_agent_session_tombstoned("sess-del-ok").expect("query"),
            "delivered deletion is acknowledged"
        );

        // "session not found" counts as delivered (idempotent).
        enqueue_delete(&home, "sess-del-gone");
        mock.push(
            "delete_session",
            Reply::Reject("session not found: sess-del-gone".to_string()),
        );
        reconcile_delete_outbox().await;
        assert!(!crate::store::is_agent_session_tombstoned("sess-del-gone").expect("query"));

        // A real rejection is noted, not acknowledged.
        enqueue_delete(&home, "sess-del-busy");
        mock.push(
            "delete_session",
            Reply::Reject("session is running".to_string()),
        );
        reconcile_delete_outbox().await;
        assert!(crate::store::is_agent_session_tombstoned("sess-del-busy").expect("query"));

        // Transport failure is noted too. The still-pending busy row is
        // retried first (FIFO), so it gets a scripted reply as well.
        enqueue_delete(&home, "sess-del-down");
        mock.push(
            "delete_session",
            Reply::Reject("session is running".to_string()),
        );
        mock.push(
            "delete_session",
            Reply::Status(tonic::Code::Unavailable, "down"),
        );
        reconcile_delete_outbox().await;
        assert!(crate::store::is_agent_session_tombstoned("sess-del-down").expect("query"));
        assert!(crate::store::is_agent_session_tombstoned("sess-del-busy").expect("query"));

        // Store unreadable → silent return.
        let prev = break_home();
        reconcile_delete_outbox().await;
        restore_home(prev);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn delete_outbox_worker_loops_until_stopped() {
        let home = TestHome::new("bridge-outbox-worker");
        let mock = mock_agent();
        enqueue_delete(&home, "sess-worker");
        mock.push("delete_session", Reply::Data("{}".to_string()));
        std::env::set_var("FUTURE_TEST_OUTBOX_INTERVAL_MS", "20");

        spawn_delete_outbox_worker();
        // Wait for the worker to deliver the pending deletion.
        for _ in 0..100 {
            if !crate::store::is_agent_session_tombstoned("sess-worker").expect("query") {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        assert!(!crate::store::is_agent_session_tombstoned("sess-worker").expect("query"));
        TEST_OUTBOX_STOP.store(true, std::sync::atomic::Ordering::Relaxed);
        tokio::time::sleep(std::time::Duration::from_millis(60)).await;
        std::env::remove_var("FUTURE_TEST_OUTBOX_INTERVAL_MS");
        // With the shrink seam removed the default (5s) interval is used.
        assert_eq!(delete_outbox_interval(), std::time::Duration::from_secs(5));
    }

    // ── active run watchdog ───────────────────────────────────────────

    #[tokio::test]
    async fn watchdog_pass_skips_young_runs_and_reconciles_old_ones() {
        let home = TestHome::new("bridge-watchdog-pass");
        let mock = mock_agent();
        let workspace = seed_workspace(home.path(), "ws");
        let thread = seed_thread(&workspace.id, Some("sess-1"));
        let run = seed_run(&thread.id);

        // A young run (now) is inside the grace window → skipped silently.
        let now = run.created_at;
        active_run_watchdog_pass(now).await;
        assert!(mock.requests().is_empty());

        // Aged past the grace window: the agent has no marker and the run is
        // below the orphan age → Skip (row untouched), but the probe went out.
        mock.push_run_state(&run.id, serde_json::json!({"isStreaming": false}));
        let aged_now = run.created_at + (WATCHDOG_GRACE_SECS as i64 + 1) * 1000;
        active_run_watchdog_pass(aged_now).await;
        assert_eq!(mock.requests_of("get_state").len(), 1);
        assert_eq!(mock.requests_of("get_state")[0].run_id, run.id);
        assert_eq!(
            crate::store::get_run(&run.id)
                .expect("run")
                .expect("some")
                .status,
            "running"
        );

        // Past the orphan age with no marker → settled failed.
        mock.push_run_state(&run.id, serde_json::json!({"isStreaming": false}));
        let orphan_now = run.created_at + (WATCHDOG_ORPHAN_SECS as i64) * 1000;
        active_run_watchdog_pass(orphan_now).await;
        let record = crate::store::get_run(&run.id).expect("run").expect("some");
        assert_eq!(record.status, "failed");
        assert_eq!(record.error_type.as_deref(), Some("stream_interrupted"));

        // Reconcile transport error on an aged active run → logged, row
        // untouched (the watchdog continues instead of propagating).
        let run2 = seed_run(&thread.id);
        mock.push(
            &format!("get_state#{}", run2.id),
            Reply::Status(tonic::Code::Unavailable, "down"),
        );
        active_run_watchdog_pass(orphan_now).await;
        assert_eq!(
            crate::store::get_run(&run2.id)
                .expect("run")
                .expect("some")
                .status,
            "running",
            "transport error leaves the row untouched"
        );

        // Store unreadable → the pass returns immediately.
        let prev = break_home();
        active_run_watchdog_pass(orphan_now).await;
        restore_home(prev);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn watchdog_loop_ticks_and_stops() {
        let _home = TestHome::new("bridge-watchdog-loop");
        let mock = mock_agent();
        std::env::set_var("FUTURE_TEST_WATCHDOG_INTERVAL_MS", "20");
        spawn_active_run_watchdog();

        // Agent unreachable (unparseable endpoint): the tick continues quietly.
        let prev = std::env::var("FUTURE_AGENT_GRPC_ADDR").expect("mock addr");
        std::env::set_var("FUTURE_AGENT_GRPC_ADDR", "http://[::1");
        tokio::time::sleep(std::time::Duration::from_millis(80)).await;
        std::env::set_var("FUTURE_AGENT_GRPC_ADDR", prev);

        // Agent reachable: the pass runs (no active runs → no probe traffic,
        // but the tick exercises the full loop).
        tokio::time::sleep(std::time::Duration::from_millis(80)).await;
        TEST_WATCHDOG_STOP.store(true, std::sync::atomic::Ordering::Relaxed);
        tokio::time::sleep(std::time::Duration::from_millis(60)).await;
        std::env::remove_var("FUTURE_TEST_WATCHDOG_INTERVAL_MS");
        // With the shrink seam removed the default interval is used.
        assert_eq!(
            watchdog_interval(),
            std::time::Duration::from_secs(WATCHDOG_INTERVAL_SECS)
        );
        let _ = &mock;
    }

    #[tokio::test]
    async fn reconcile_active_run_once_variants() {
        let home = TestHome::new("bridge-reconcile-once");
        let mock = mock_agent();
        let workspace = seed_workspace(home.path(), "ws");
        let thread = seed_thread(&workspace.id, Some("sess-1"));
        let run = seed_run(&thread.id);
        let active = crate::store::ActiveRun {
            run_id: run.id.clone(),
            thread_id: thread.id.clone(),
            session_id: "sess-1".to_string(),
            created_at: run.created_at,
        };

        // Agent still streaming this run → observer ensured (attach action).
        mock.push_run_state(
            &run.id,
            serde_json::json!({"isStreaming": true, "activeRun": {"runId": run.id}}),
        );
        reconcile_active_run_once(&active, &run.id, 120)
            .await
            .expect("attach");

        // Durable terminal marker → mirrored onto the row.
        mock.push_run_state(
            &run.id,
            serde_json::json!({"requestedRun": {"status": "completed"}}),
        );
        reconcile_active_run_once(&active, &run.id, 120)
            .await
            .expect("settle");
        assert_eq!(
            crate::store::get_run(&run.id)
                .expect("run")
                .expect("some")
                .status,
            "completed"
        );

        // get_state rejected → row untouched, Ok.
        mock.push(
            &format!("get_state#{}", run.id),
            Reply::Reject("unknown session".to_string()),
        );
        reconcile_active_run_once(&active, &run.id, 120)
            .await
            .expect("rejected is ok");

        // Transport failure → Err (the watchdog logs it).
        mock.push(
            &format!("get_state#{}", run.id),
            Reply::Status(tonic::Code::Unavailable, "down"),
        );
        let error = reconcile_active_run_once(&active, &run.id, 120)
            .await
            .expect_err("transport");
        assert!(error.contains("get_run_state"), "{error}");

        // Interrupted-by-restart marker → the row is cancelled.
        let run_i = seed_run(&thread.id);
        let active_i = crate::store::ActiveRun {
            run_id: run_i.id.clone(),
            thread_id: thread.id.clone(),
            session_id: "sess-1".to_string(),
            created_at: run_i.created_at,
        };
        mock.push_run_state(
            &run_i.id,
            serde_json::json!({"interruptedRun": {"runId": run_i.id}}),
        );
        reconcile_active_run_once(&active_i, &run_i.id, 120)
            .await
            .expect("interrupted");
        assert_eq!(
            crate::store::get_run(&run_i.id)
                .expect("run")
                .expect("some")
                .status,
            "cancelled"
        );
    }
}

#[cfg(test)]
mod pipeline_tests {
    use super::test_support::{
        mock_agent, seed_run, seed_thread, seed_workspace, stream_event, MockAgentGuard, Reply,
        StreamScript, TestHome,
    };
    use super::*;

    struct PipelineFixture {
        _home: TestHome,
        mock: MockAgentGuard,
        workspace: crate::store::WorkspaceRecord,
        thread: crate::store::ThreadRecord,
        run: crate::store::RunRecord,
    }

    /// Thread + run with a fresh (unstored) agent session; the observer the
    /// pipeline spawns parks on the default plain Hang stream.
    fn pipeline_fixture(tag: &str, title: &str) -> PipelineFixture {
        let home = TestHome::new(tag);
        let mock = mock_agent();
        let workspace = seed_workspace(home.path(), "ws");
        let mut thread = seed_thread(&workspace.id, None);
        thread.title = title.to_string();
        crate::store::rename_thread(crate::store::RenameThreadInput {
            thread_id: thread.id.clone(),
            title: title.to_string(),
        })
        .expect("rename");
        let run = seed_run(&thread.id);
        PipelineFixture {
            _home: home,
            mock,
            workspace,
            thread,
            run,
        }
    }

    fn prompt_args(fixture: &PipelineFixture) -> (String, String, String) {
        (
            "hello from the test".to_string(),
            fixture.thread.id.clone(),
            fixture.run.id.clone(),
        )
    }

    #[cfg(feature = "gui")]
    #[tokio::test]
    async fn gui_prompt_notifies_acceptance_before_stream_completion() {
        let fixture = pipeline_fixture("gui-prompt-ack", "New Chat");
        fixture.mock.push_data(
            "new_session",
            serde_json::json!({"sessionId": "sess-gui-ack"}),
        );
        let (message, thread_id, run_id) = prompt_args(&fixture);
        let accepted = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let signal = accepted.clone();
        let channel = tauri::ipc::Channel::new(move |_| {
            signal.store(true, std::sync::atomic::Ordering::SeqCst);
            Ok(())
        });
        // The mock accepts prompts but its default stream remains pending.
        let result = tokio::time::timeout(
            std::time::Duration::from_secs(3),
            crate::commands::forward_prompt_acceptance(
                AgentPromptRequest {
                    message,
                    model_context: String::new(),
                    attachments: None,
                    thread_id,
                    session_id: None,
                    run_id: Some(run_id),
                    model_id: None,
                    thinking_level: None,
                },
                Some(channel),
            ),
        )
        .await;
        assert!(result.is_err(), "the response must still be streaming");
        assert!(
            accepted.load(std::sync::atomic::Ordering::SeqCst),
            "commands reached: {:?}",
            fixture
                .mock
                .requests()
                .iter()
                .map(|request| &request.r#type)
                .collect::<Vec<_>>()
        );
    }

    #[tokio::test]
    async fn agent_prompt_new_session_full_pipeline() {
        let fixture = pipeline_fixture("pipe-new", "New Chat");
        let (message, thread_id, run_id) = prompt_args(&fixture);

        fixture
            .mock
            .push_data("new_session", serde_json::json!({"sessionId": "sess-p1"}));
        fixture.mock.push_stream(StreamScript::Events(
            vec![
                stream_event(&run_id, 0, "text_chunk", r#"{"text":"hi there"}"#),
                stream_event(&run_id, 1, "agent_end", r#"{"reason":"complete"}"#),
            ],
            None,
        ));

        let response = agent_prompt_with_model_context(AgentPromptRequest {
            message: message.clone(),
            model_context: "Referenced FutureOS objects:\n1. file:utils/a.py".to_string(),
            attachments: None,
            thread_id: thread_id.clone(),
            session_id: None,
            run_id: Some(run_id.clone()),
            model_id: Some("future/k3".to_string()),
            thinking_level: Some("high".to_string()),
        })
        .await
        .expect("prompt");

        assert!(response.complete);
        assert_eq!(response.content, "hi there");
        assert_eq!(response.session_id, "sess-p1");

        // Session id persisted; thread auto-named from the first message.
        let thread = crate::store::get_thread(&thread_id)
            .expect("thread")
            .expect("exists");
        assert_eq!(thread.agent_session_id.as_deref(), Some("sess-p1"));
        assert_eq!(thread.title, message);
        assert_eq!(
            crate::store::get_run(&run_id)
                .expect("run")
                .expect("some")
                .status,
            "completed",
            "the backend settles the run row"
        );

        // A freshly created session receives the caller's model + thinking.
        let new_session = &fixture.mock.requests_of("new_session")[0];
        assert_eq!(new_session.cwd, fixture.workspace.path);
        assert_eq!(
            fixture.mock.requests_of("set_model")[0].model_id,
            "future/k3"
        );
        assert_eq!(
            fixture.mock.requests_of("set_thinking_level")[0].level,
            "high"
        );
        let prompt = &fixture.mock.requests_of("prompt")[0];
        assert_eq!(prompt.message, message);
        assert_eq!(
            prompt.model_context,
            "Referenced FutureOS objects:\n1. file:utils/a.py"
        );
        assert_eq!(prompt.requested_run_id, run_id);
        assert_eq!(prompt.session_id, "sess-p1");
        // The observer was registered before the prompt reached the agent.
        assert!(
            observer::OBSERVERS
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .contains_key("sess-p1"),
            "session observer registered"
        );
    }

    #[tokio::test]
    async fn agent_prompt_existing_session_reuses_without_model_reapply() {
        let home = TestHome::new("pipe-existing");
        let mock = mock_agent();
        let workspace = seed_workspace(home.path(), "ws");
        let thread = seed_thread(&workspace.id, Some("sess-existing"));
        let run = seed_run(&thread.id);

        mock.push_data(
            "get_state",
            serde_json::json!({"sessionId": "sess-existing", "cwd": workspace.path}),
        );
        mock.push_stream(StreamScript::Events(
            vec![stream_event(
                &run.id,
                0,
                "agent_end",
                r#"{"reason":"incomplete"}"#,
            )],
            None,
        ));

        let response = agent_prompt(
            "follow-up".to_string(),
            Some(vec![AttachmentInput {
                path: "/tmp/a.txt".to_string(),
                kind: "file".to_string(),
                name: "a.txt".to_string(),
                thumbnail: None,
            }]),
            thread.id.clone(),
            None, // falls back to the thread's stored session id
            Some(run.id.clone()),
            Some("future/other".to_string()),
            None,
        )
        .await
        .expect("prompt");

        assert!(!response.complete, "incomplete agent_end is not clean");
        assert_eq!(response.session_id, "sess-existing");
        assert!(
            mock.requests_of("new_session").is_empty(),
            "the stored session was reused"
        );
        assert!(
            mock.requests_of("set_model").is_empty(),
            "an existing session keeps its authoritative model"
        );
        assert_eq!(
            mock.requests_of("prompt")[0].attachments.len(),
            1,
            "attachments forwarded"
        );
        let record = crate::store::get_run(&run.id).expect("run").expect("some");
        assert_eq!(
            record.status, "failed",
            "an interrupted stream fails the run"
        );
    }

    #[tokio::test]
    async fn agent_prompt_repairs_cwd_without_replacing_the_session() {
        let home = TestHome::new("pipe-cwd-repair");
        let mock = mock_agent();
        let workspace = seed_workspace(home.path(), "ws");
        let thread = seed_thread(&workspace.id, Some("sess-old"));
        let run = seed_run(&thread.id);

        // A fork/workspace transition may leave the session metadata pointed at
        // another cwd. The live session and its history remain authoritative.
        mock.push_data(
            "get_state",
            serde_json::json!({"sessionId": "sess-old", "cwd": "/moved/elsewhere"}),
        );
        mock.push_data("set_cwd", serde_json::json!({"cwd": workspace.path}));
        mock.push_stream(StreamScript::Events(
            vec![stream_event(
                &run.id,
                0,
                "agent_end",
                r#"{"reason":"complete"}"#,
            )],
            None,
        ));

        let response = agent_prompt(
            "hi".to_string(),
            None,
            thread.id.clone(),
            Some("sess-old".to_string()),
            Some(run.id.clone()),
            Some("future/k3".to_string()),
            None,
        )
        .await
        .expect("prompt");

        assert_eq!(response.session_id, "sess-old");
        assert_eq!(
            crate::store::get_thread(&thread.id)
                .expect("thread")
                .expect("exists")
                .agent_session_id
                .as_deref(),
            Some("sess-old")
        );
        assert!(mock.requests_of("new_session").is_empty());
        assert_eq!(mock.requests_of("set_cwd").len(), 1);
        // An existing session keeps its own authoritative model and context.
        assert!(mock.requests_of("set_model").is_empty());
    }

    #[tokio::test]
    async fn agent_prompt_missing_session_preserves_the_thread_binding() {
        let home = TestHome::new("pipe-missing-session");
        let mock = mock_agent();
        let workspace = seed_workspace(home.path(), "ws");
        let thread = seed_thread(&workspace.id, Some("sess-missing"));
        let run = seed_run(&thread.id);

        mock.push("get_state", Reply::Reject("session not found".to_string()));

        let error = agent_prompt(
            "keep my history".to_string(),
            None,
            thread.id.clone(),
            Some("sess-missing".to_string()),
            Some(run.id),
            Some("future/k3".to_string()),
            None,
        )
        .await
        .expect_err("a missing bound session must be explicit");

        assert!(error.to_string().contains("binding was preserved"));
        assert_eq!(
            crate::store::get_thread(&thread.id)
                .expect("thread")
                .expect("exists")
                .agent_session_id
                .as_deref(),
            Some("sess-missing")
        );
        assert!(mock.requests_of("new_session").is_empty());
        assert!(mock.requests_of("prompt").is_empty());
    }

    #[tokio::test]
    async fn agent_prompt_rejects_a_stale_client_session_identity() {
        let home = TestHome::new("pipe-stale-session");
        let mock = mock_agent();
        let workspace = seed_workspace(home.path(), "ws");
        let thread = seed_thread(&workspace.id, Some("sess-parent"));
        let run = seed_run(&thread.id);

        let error = agent_prompt(
            "must stay on parent".to_string(),
            None,
            thread.id.clone(),
            Some("sess-child".to_string()),
            Some(run.id),
            None,
            None,
        )
        .await
        .expect_err("stale client identity must not route the prompt");

        assert!(error.to_string().contains("session changed"));
        assert_eq!(
            crate::store::get_thread(&thread.id)
                .expect("thread")
                .expect("exists")
                .agent_session_id
                .as_deref(),
            Some("sess-parent")
        );
        assert!(mock.requests_of("new_session").is_empty());
        assert!(mock.requests_of("prompt").is_empty());
        assert!(mock
            .requests_of("get_state")
            .iter()
            .all(|request| request.session_id == "sess-parent"));
    }

    #[tokio::test]
    async fn agent_prompt_transport_and_rejection_failures_settle_the_run() {
        let fixture = pipeline_fixture("pipe-prompt-fail", "t");
        let (message, thread_id, run_id) = prompt_args(&fixture);
        fixture
            .mock
            .push_data("new_session", serde_json::json!({"sessionId": "sess-pf"}));

        fixture.mock.push(
            "prompt",
            Reply::Status(tonic::Code::Internal, "write failed"),
        );
        let error = agent_prompt(
            message.clone(),
            None,
            thread_id.clone(),
            None,
            Some(run_id.clone()),
            None,
            None,
        )
        .await
        .expect_err("transport");
        assert!(
            error
                .to_string()
                .contains("Unable to send prompt to Future Agent"),
            "{error}"
        );
        assert_eq!(
            crate::store::get_run(&run_id)
                .expect("run")
                .expect("some")
                .status,
            "failed"
        );

        // Release the first fixture before building the second: each fixture
        // holds the process-global TEST_HOME_LOCK + MOCK_LOCK guards, so two
        // live fixtures on one test thread would self-deadlock.
        drop(fixture);
        let fixture2 = pipeline_fixture("pipe-prompt-reject", "t");
        fixture2
            .mock
            .push_data("new_session", serde_json::json!({"sessionId": "sess-pr"}));
        fixture2
            .mock
            .push("prompt", Reply::Reject("busy".to_string()));
        let error = agent_prompt(
            message,
            None,
            fixture2.thread.id.clone(),
            None,
            Some(fixture2.run.id.clone()),
            None,
            None,
        )
        .await
        .expect_err("reject");
        assert_eq!(error.to_string(), "busy");
    }

    #[tokio::test]
    async fn agent_prompt_ack_must_carry_the_requested_run_id() {
        let fixture = pipeline_fixture("pipe-ack", "t");
        let (message, thread_id, run_id) = prompt_args(&fixture);
        fixture
            .mock
            .push_data("new_session", serde_json::json!({"sessionId": "sess-pa"}));

        // Ack without run_id.
        fixture
            .mock
            .push("prompt", Reply::Data(r#"{"ok":true}"#.to_string()));
        let error = agent_prompt(
            message.clone(),
            None,
            thread_id.clone(),
            None,
            Some(run_id.clone()),
            None,
            None,
        )
        .await
        .expect_err("missing run id");
        assert_eq!(
            error.to_string(),
            "Future Agent prompt acknowledgement omitted run_id."
        );

        // Ack with a DIFFERENT run id. Release the first fixture first: two
        // live fixtures on one test thread self-deadlock on the
        // process-global TEST_HOME_LOCK + MOCK_LOCK guards.
        drop(fixture);
        let fixture2 = pipeline_fixture("pipe-ack-mismatch", "t");
        fixture2
            .mock
            .push_data("new_session", serde_json::json!({"sessionId": "sess-pam"}));
        fixture2.mock.push(
            "prompt",
            Reply::Data(r#"{"run_id":"run-other"}"#.to_string()),
        );
        let error = agent_prompt(
            message,
            None,
            fixture2.thread.id.clone(),
            None,
            Some(fixture2.run.id.clone()),
            None,
            None,
        )
        .await
        .expect_err("mismatch");
        assert!(
            error.to_string().contains("adopted run id run-other"),
            "{error}"
        );
    }

    #[tokio::test]
    async fn agent_prompt_generates_a_run_id_when_absent() {
        let fixture = pipeline_fixture("pipe-gen-run", "t");
        fixture
            .mock
            .push_data("new_session", serde_json::json!({"sessionId": "sess-pg"}));
        // First attach closes with zero events: the collector treats a stream
        // that ends before a terminal event as a drop and reattaches. The
        // reattach then delivers an unclean agent_end → complete = false.
        fixture.mock.push_stream(StreamScript::Events(vec![], None));
        fixture.mock.push_stream(StreamScript::Events(
            vec![stream_event(
                "@attach",
                0,
                "agent_end",
                r#"{"reason":"incomplete"}"#,
            )],
            None,
        ));
        // No run_id: the pipeline generates one; the mock's default prompt
        // reply echoes the requested_run_id.
        let response = agent_prompt(
            "hi".to_string(),
            None,
            fixture.thread.id.clone(),
            None,
            None,
            None,
            None,
        )
        .await
        .expect("prompt");
        assert!(!response.complete, "an instantly-closed stream is a prefix");
        let prompt = &fixture.mock.requests_of("prompt")[0];
        assert!(
            prompt.requested_run_id.starts_with("run-"),
            "generated id: {}",
            prompt.requested_run_id
        );
    }

    #[tokio::test]
    async fn agent_prompt_rejects_when_the_run_already_has_a_collector() {
        let fixture = pipeline_fixture("pipe-lease", "t");
        let (message, thread_id, run_id) = prompt_args(&fixture);
        fixture
            .mock
            .push_data("new_session", serde_json::json!({"sessionId": "sess-pl"}));

        let lease = AGENT_REPLICAS.acquire(&run_id).expect("pre-acquire");
        let error = agent_prompt(
            message,
            None,
            thread_id,
            None,
            Some(run_id.clone()),
            None,
            None,
        )
        .await
        .expect_err("lease conflict");
        assert!(
            error.to_string().contains("already owns Agent run"),
            "{error}"
        );
        drop(lease);
    }

    #[tokio::test]
    async fn agent_prompt_run_gone_reconciles_from_the_journal() {
        let fixture = pipeline_fixture("pipe-rungone", "t");
        let (message, thread_id, run_id) = prompt_args(&fixture);
        fixture
            .mock
            .push_data("new_session", serde_json::json!({"sessionId": "sess-prg"}));
        fixture.mock.push_stream(StreamScript::AttachError(
            tonic::Code::FailedPrecondition,
            "no such run",
        ));
        // The journal holds a durable completed marker for the run.
        fixture.mock.push_run_state(
            &run_id,
            serde_json::json!({"requestedRun": {"status": "completed"}}),
        );
        let error = agent_prompt(
            message,
            None,
            thread_id,
            None,
            Some(run_id.clone()),
            None,
            None,
        )
        .await
        .expect_err("run gone");
        assert!(
            error
                .to_string()
                .contains("run ended before the stream attached"),
            "{error}"
        );
        assert_eq!(
            crate::store::get_run(&run_id)
                .expect("run")
                .expect("some")
                .status,
            "completed",
            "the journal marker settles the row"
        );
    }

    #[tokio::test]
    async fn agent_prompt_queued_run_attaches_later_and_stays_running() {
        let fixture = pipeline_fixture("pipe-queued", "t");
        let (message, thread_id, run_id) = prompt_args(&fixture);
        fixture
            .mock
            .push_data("new_session", serde_json::json!({"sessionId": "sess-pq"}));
        // The Agent accepted the prompt but queued it behind an older run:
        // attach fails with RunGone (a queued run has no execution epoch).
        fixture.mock.push_stream(StreamScript::AttachError(
            tonic::Code::FailedPrecondition,
            "run is not the active run",
        ));
        fixture.mock.push_run_state(
            &run_id,
            serde_json::json!({"queuedRuns": [{"runId": run_id, "queuePosition": 1}]}),
        );
        // The queued run's user entry is already durable on the Agent; the
        // pre-persistence replay fetches it through the typed entries page.
        fixture.mock.push_typed_data(
            "get_session_entries",
            serde_json::json!({
                "entries": [{
                    "id": "e-queued",
                    "kind": "user",
                    "role": "user",
                    "createdAtMs": 1000,
                    "runId": run_id,
                    "blocks": [{"kind": "text", "text": "hello from the test"}]
                }]
            }),
        );

        let response = agent_prompt(
            message,
            None,
            thread_id,
            None,
            Some(run_id.clone()),
            None,
            None,
        )
        .await
        .expect("queued prompt is not a failure");

        assert!(!response.complete, "a queued prompt ends its own stream");
        assert_eq!(response.termination_kind.as_deref(), Some("run_queued"));
        let run = crate::store::get_run(&run_id).expect("run").expect("some");
        assert_eq!(
            run.status, "running",
            "a queued run is alive — the observer settles it on start/finish"
        );
        assert!(
            run.error_message.is_none(),
            "no failure recorded: {:?}",
            run.error_message
        );
    }

    #[tokio::test]
    async fn agent_prompt_run_gone_with_a_failed_reconcile_reports_both() {
        let fixture = pipeline_fixture("pipe-rungone-fail", "t");
        let (message, thread_id, run_id) = prompt_args(&fixture);
        fixture
            .mock
            .push_data("new_session", serde_json::json!({"sessionId": "sess-prgf"}));
        fixture.mock.push_stream(StreamScript::AttachError(
            tonic::Code::NotFound,
            "unknown run",
        ));
        fixture.mock.push(
            &format!("get_state#{run_id}"),
            Reply::Status(tonic::Code::Unavailable, "agent restarting"),
        );
        let error = agent_prompt(
            message,
            None,
            thread_id,
            None,
            Some(run_id.clone()),
            None,
            None,
        )
        .await
        .expect_err("run gone");
        let message = error.to_string();
        assert!(message.contains("unknown run"), "{message}");
        assert!(
            message.contains("terminal reconciliation failed"),
            "{message}"
        );
    }

    #[tokio::test]
    async fn agent_prompt_stream_error_aborts_the_agent_run() {
        let fixture = pipeline_fixture("pipe-stream-err", "t");
        let (message, thread_id, run_id) = prompt_args(&fixture);
        fixture
            .mock
            .push_data("new_session", serde_json::json!({"sessionId": "sess-pse"}));
        fixture.mock.push_stream(StreamScript::Events(
            vec![stream_event(
                &run_id,
                0,
                "error",
                r#"{"error":"provider down"}"#,
            )],
            None,
        ));
        let error = agent_prompt(
            message,
            None,
            thread_id,
            None,
            Some(run_id.clone()),
            None,
            None,
        )
        .await
        .expect_err("stream error");
        assert_eq!(error.to_string(), "provider down");
        // The orphaned agent-side run is aborted best-effort.
        let aborts = fixture.mock.requests_of("abort");
        assert_eq!(aborts.len(), 1);
        assert_eq!(aborts[0].run_id, run_id);
        assert_eq!(aborts[0].session_id, "sess-pse");
        assert_eq!(
            crate::store::get_run(&run_id)
                .expect("run")
                .expect("some")
                .status,
            "failed"
        );
    }

    /// When the stream fails and the best-effort abort is itself rejected, the
    /// abort error is logged and the original stream error still surfaces.
    #[tokio::test]
    async fn agent_prompt_stream_error_logs_a_failed_abort() {
        let fixture = pipeline_fixture("pipe-stream-err-abort", "t");
        let (message, thread_id, run_id) = prompt_args(&fixture);
        fixture
            .mock
            .push_data("new_session", serde_json::json!({"sessionId": "sess-psea"}));
        fixture.mock.push_stream(StreamScript::Events(
            vec![stream_event(
                &run_id,
                0,
                "error",
                r#"{"error":"provider down"}"#,
            )],
            None,
        ));
        // The best-effort abort transport-fails — exercise the abort-failure log.
        fixture
            .mock
            .push("abort", Reply::Status(tonic::Code::Internal, "abort down"));

        let error = agent_prompt(
            message,
            None,
            thread_id,
            None,
            Some(run_id.clone()),
            None,
            None,
        )
        .await
        .expect_err("stream error");
        assert_eq!(error.to_string(), "provider down");
        assert_eq!(fixture.mock.requests_of("abort").len(), 1);
    }

    #[tokio::test]
    async fn agent_prompt_requires_a_real_thread() {
        let _home = TestHome::new("pipe-no-thread");
        let _mock = mock_agent();
        let error = agent_prompt(
            "hi".to_string(),
            None,
            "no-such-thread".to_string(),
            None,
            None,
            None,
            None,
        )
        .await
        .expect_err("missing thread");
        assert_eq!(error.to_string(), "Thread could not be loaded.");
    }

    // ── auto_name_thread ──────────────────────────────────────────────

    #[tokio::test]
    async fn auto_name_thread_variants() {
        let home = TestHome::new("pipe-autoname");
        let _mock = mock_agent();
        let workspace = seed_workspace(home.path(), "ws");

        // Missing thread: silent.
        auto_name_thread("no-such-thread", "hello");

        // Default-titled variants are renamed; the agent is told (fire-and-forget).
        // (rename_thread rejects empty titles, so the empty-titled row is made
        // through create_thread, which takes the caller's title verbatim.)
        let empty_titled = crate::store::create_thread(crate::store::CreateThreadInput {
            mode: "workspace".to_string(),
            title: Some(String::new()),
            workspace_id: Some(workspace.id.clone()),
            workspace_path: None,
            workspace_name: None,
            agent_session_id: Some("sess-an-empty".to_string()),
        })
        .expect("empty-titled thread");
        auto_name_thread(&empty_titled.id, "  a fresh question  ");
        assert_eq!(
            crate::store::get_thread(&empty_titled.id)
                .expect("thread")
                .expect("exists")
                .title,
            "a fresh question",
            "an empty title is auto-named"
        );
        for (tag, title) in [("zh", "新对话"), ("newchat", "New Chat")] {
            let thread = seed_thread(&workspace.id, Some(&format!("sess-an-{tag}")));
            crate::store::rename_thread(crate::store::RenameThreadInput {
                thread_id: thread.id.clone(),
                title: title.to_string(),
            })
            .expect("rename");
            auto_name_thread(&thread.id, "  a fresh question  ");
            assert_eq!(
                crate::store::get_thread(&thread.id)
                    .expect("thread")
                    .expect("exists")
                    .title,
                "a fresh question",
                "title {title:?} is auto-named"
            );
        }

        // Long messages truncate to 40 chars + ellipsis.
        let thread = seed_thread(&workspace.id, Some("sess-an-long"));
        crate::store::rename_thread(crate::store::RenameThreadInput {
            thread_id: thread.id.clone(),
            title: "New Chat".to_string(),
        })
        .expect("rename");
        let long = "x".repeat(50);
        auto_name_thread(&thread.id, &long);
        let titled = crate::store::get_thread(&thread.id)
            .expect("thread")
            .expect("exists")
            .title;
        assert!(titled.ends_with('…'), "truncated: {titled}");
        assert_eq!(titled.chars().count(), 41);

        // User-set titles are never overwritten.
        let thread = seed_thread(&workspace.id, Some("sess-an-custom"));
        auto_name_thread(&thread.id, "new message");
        assert_eq!(
            crate::store::get_thread(&thread.id)
                .expect("thread")
                .expect("exists")
                .title,
            "test thread"
        );

        // Blank messages never rename (empty-titled row via create_thread —
        // rename_thread rejects empty titles).
        let thread = crate::store::create_thread(crate::store::CreateThreadInput {
            mode: "workspace".to_string(),
            title: Some(String::new()),
            workspace_id: Some(workspace.id.clone()),
            workspace_path: None,
            workspace_name: None,
            agent_session_id: Some("sess-an-blank".to_string()),
        })
        .expect("empty-titled thread");
        auto_name_thread(&thread.id, "   ");
        assert_eq!(
            crate::store::get_thread(&thread.id)
                .expect("thread")
                .expect("exists")
                .title,
            ""
        );
    }

    /// The auto-name fire-and-forget agent rename silently skips the agent
    /// call when the agent is unreachable (the `if let Ok` else path).
    #[tokio::test]
    async fn auto_name_thread_survives_an_unreachable_agent() {
        let home = TestHome::new("pipe-autoname-down");
        let _mock = mock_agent();
        let workspace = seed_workspace(home.path(), "ws");
        let thread = seed_thread(&workspace.id, Some("sess-an-down"));
        crate::store::rename_thread(crate::store::RenameThreadInput {
            thread_id: thread.id.clone(),
            title: "New Chat".to_string(),
        })
        .expect("rename");

        let prev = std::env::var("FUTURE_AGENT_GRPC_ADDR").expect("mock addr");
        std::env::set_var("FUTURE_AGENT_GRPC_ADDR", "http://[::1");

        auto_name_thread(&thread.id, "hello");
        // Let the fire-and-forget rename task run against the dead endpoint.
        tokio::time::sleep(std::time::Duration::from_millis(150)).await;

        std::env::set_var("FUTURE_AGENT_GRPC_ADDR", prev);
        // The local rename still happened; only the agent propagation was
        // skipped.
        assert_eq!(
            crate::store::get_thread(&thread.id)
                .expect("thread")
                .expect("exists")
                .title,
            "hello"
        );
    }

    // ── crash recovery ────────────────────────────────────────────────

    fn mark_interrupted(run_id: &str) {
        crate::store::update_run_status_if_active(crate::store::UpdateRunStatusInput {
            run_id: run_id.to_string(),
            status: "cancelled".to_string(),
            error_message: Some("Interrupted because Future Agent restarted.".to_string()),
            error_type: Some("interrupted".to_string()),
        })
        .expect("mark interrupted");
    }

    #[tokio::test]
    async fn reconcile_interrupted_runs_edge_cases() {
        let _home = TestHome::new("pipe-reconcile-empty");
        let mock = mock_agent();

        // Empty list → no traffic.
        reconcile_interrupted_runs().await;
        assert!(mock.requests().is_empty());

        // Store unreadable → silent return.
        let prev = test_support::break_home();
        reconcile_interrupted_runs().await;
        test_support::restore_home(prev);
    }

    /// A crash-recovery pass over an interrupted run whose reanimation hits an
    /// unreachable agent logs the failure (rather than panicking) and keeps
    /// going.
    #[tokio::test]
    async fn reconcile_interrupted_runs_logs_a_reanimation_error() {
        let home = TestHome::new("pipe-reconcile-err");
        let _mock = mock_agent();
        let workspace = seed_workspace(home.path(), "ws");
        let thread = seed_thread(&workspace.id, Some("sess-re-err"));
        let run = seed_run(&thread.id);
        mark_interrupted(&run.id);

        // Agent unreachable → check_and_reanimate_run returns Err → logged.
        let prev = std::env::var("FUTURE_AGENT_GRPC_ADDR").expect("mock addr");
        std::env::set_var("FUTURE_AGENT_GRPC_ADDR", "http://[::1");
        reconcile_interrupted_runs().await;
        std::env::set_var("FUTURE_AGENT_GRPC_ADDR", prev);
    }

    #[tokio::test]
    async fn reanimate_still_streaming_run_attaches_an_observer() {
        let home = TestHome::new("pipe-reanimate");
        let mock = mock_agent();
        let workspace = seed_workspace(home.path(), "ws");
        let thread = seed_thread(&workspace.id, Some("sess-re"));
        let run = seed_run(&thread.id);
        mark_interrupted(&run.id);

        mock.push_run_state(
            &run.id,
            serde_json::json!({"isStreaming": true, "activeRun": {"runId": run.id}}),
        );
        reconcile_interrupted_runs().await;
        assert_eq!(
            crate::store::get_run(&run.id)
                .expect("run")
                .expect("some")
                .status,
            "running",
            "reanimated back to running"
        );
        assert!(
            observer::OBSERVERS
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .contains_key("sess-re"),
            "observer attached for the live run"
        );
    }

    #[tokio::test]
    async fn check_and_reanimate_run_variants() {
        let home = TestHome::new("pipe-check-variants");
        let mock = mock_agent();
        let workspace = seed_workspace(home.path(), "ws");
        let thread = seed_thread(&workspace.id, Some("sess-cv"));

        // Agent cannot resolve the session → leave interrupted, Ok.
        let run = seed_run(&thread.id);
        mark_interrupted(&run.id);
        mock.push(
            &format!("get_state#{}", run.id),
            Reply::Reject("unknown session".to_string()),
        );
        check_and_reanimate_run("sess-cv", &run.id, &thread.id)
            .await
            .expect("unresolved is ok");
        assert_eq!(
            crate::store::get_run(&run.id)
                .expect("run")
                .expect("some")
                .status,
            "cancelled"
        );

        // Streaming THIS run but the row is no longer interrupted → skip.
        let run2 = seed_run(&thread.id); // still "running" — never interrupted
        mock.push_run_state(
            &run2.id,
            serde_json::json!({"isStreaming": true, "activeRun": {"runId": run2.id}}),
        );
        check_and_reanimate_run("sess-cv", &run2.id, &thread.id)
            .await
            .expect("skip");
        assert!(
            !observer::OBSERVERS
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .contains_key("sess-cv"),
            "no observer for the skipped reanimation"
        );

        // Interrupted-by-restart marker → left cancelled.
        let run3 = seed_run(&thread.id);
        mark_interrupted(&run3.id);
        mock.push_run_state(
            &run3.id,
            serde_json::json!({"interruptedRun": {"runId": run3.id}}),
        );
        check_and_reanimate_run("sess-cv", &run3.id, &thread.id)
            .await
            .expect("interrupted");
        assert_eq!(
            crate::store::get_run(&run3.id)
                .expect("run")
                .expect("some")
                .status,
            "cancelled"
        );

        // Durable terminal marker → mirrored.
        let run4 = seed_run(&thread.id);
        mark_interrupted(&run4.id);
        mock.push_run_state(
            &run4.id,
            serde_json::json!({"requestedRun": {"status": "error", "error": "boom"}}),
        );
        check_and_reanimate_run("sess-cv", &run4.id, &thread.id)
            .await
            .expect("settle");
        let record = crate::store::get_run(&run4.id).expect("run").expect("some");
        assert_eq!(record.status, "failed");
        assert_eq!(record.error_message.as_deref(), Some("boom"));

        // No markers at all → conservatively left interrupted.
        let run5 = seed_run(&thread.id);
        mark_interrupted(&run5.id);
        mock.push_run_state(&run5.id, serde_json::json!({"isStreaming": false}));
        check_and_reanimate_run("sess-cv", &run5.id, &thread.id)
            .await
            .expect("leave");
        assert_eq!(
            crate::store::get_run(&run5.id)
                .expect("run")
                .expect("some")
                .status,
            "cancelled"
        );

        // Transport failure → Err.
        mock.push(
            &format!("get_state#{}", run5.id),
            Reply::Status(tonic::Code::Unavailable, "down"),
        );
        let error = check_and_reanimate_run("sess-cv", &run5.id, &thread.id)
            .await
            .expect_err("transport");
        assert!(error.contains("get_state"), "{error}");

        // Connect failure → Err.
        let prev = std::env::var("FUTURE_AGENT_GRPC_ADDR").expect("mock addr");
        std::env::set_var("FUTURE_AGENT_GRPC_ADDR", "http://[::1");
        let error = check_and_reanimate_run("sess-cv", &run5.id, &thread.id)
            .await
            .expect_err("connect");
        std::env::set_var("FUTURE_AGENT_GRPC_ADDR", prev);
        assert!(error.contains("connect"), "{error}");
    }

    #[tokio::test]
    async fn reconcile_run_gone_marker_precedence() {
        let home = TestHome::new("pipe-rungone-precedence");
        let mock = mock_agent();
        let workspace = seed_workspace(home.path(), "ws");
        let thread = seed_thread(&workspace.id, Some("sess-rgp"));

        // Still active agent-side (attach raced start_run) → left running.
        let run = seed_run(&thread.id);
        mock.push_run_state(&run.id, serde_json::json!({"activeRun": {"runId": run.id}}));
        reconcile_run_gone(&run.id, &run.id, "sess-rgp", &thread.id, "test")
            .await
            .expect("still active");
        assert_eq!(
            crate::store::get_run(&run.id)
                .expect("run")
                .expect("some")
                .status,
            "running"
        );

        // Interrupted marker → cancelled/interrupted.
        let run2 = seed_run(&thread.id);
        mock.push_run_state(
            &run2.id,
            serde_json::json!({"interruptedRun": {"runId": run2.id}}),
        );
        reconcile_run_gone(&run2.id, &run2.id, "sess-rgp", &thread.id, "test")
            .await
            .expect("interrupted");
        let record = crate::store::get_run(&run2.id).expect("run").expect("some");
        assert_eq!(record.status, "cancelled");
        assert_eq!(record.error_type.as_deref(), Some("interrupted"));

        // No marker at all → settled failed.
        let run3 = seed_run(&thread.id);
        mock.push_run_state(&run3.id, serde_json::json!({}));
        reconcile_run_gone(&run3.id, &run3.id, "sess-rgp", &thread.id, "vanished")
            .await
            .expect("failed");
        let record = crate::store::get_run(&run3.id).expect("run").expect("some");
        assert_eq!(record.status, "failed");
        assert!(
            record
                .error_message
                .as_deref()
                .unwrap_or_default()
                .contains("vanished"),
            "message: {:?}",
            record.error_message
        );

        // get_state itself failed → state treated as empty (no marker) → failed.
        let run4 = seed_run(&thread.id);
        mock.push(
            &format!("get_state#{}", run4.id),
            Reply::Reject("gone".to_string()),
        );
        reconcile_run_gone(&run4.id, &run4.id, "sess-rgp", &thread.id, "gone")
            .await
            .expect("failed");
        assert_eq!(
            crate::store::get_run(&run4.id)
                .expect("run")
                .expect("some")
                .status,
            "failed"
        );

        // Connect failure → Err.
        let prev = std::env::var("FUTURE_AGENT_GRPC_ADDR").expect("mock addr");
        std::env::set_var("FUTURE_AGENT_GRPC_ADDR", "http://[::1");
        let error = reconcile_run_gone(&run4.id, &run4.id, "sess-rgp", &thread.id, "test")
            .await
            .expect_err("connect");
        std::env::set_var("FUTURE_AGENT_GRPC_ADDR", prev);
        assert!(error.contains("reconcile connect"), "{error}");
    }

    #[test]
    fn settle_from_agent_terminal_maps_all_states() {
        let home = TestHome::new("pipe-settle");
        let workspace = seed_workspace(home.path(), "ws");
        let thread = seed_thread(&workspace.id, Some("sess-s"));

        let cases = [
            ("completed", "completed", None, None),
            (
                "cancelled",
                "cancelled",
                Some("cancelled"),
                Some("Run was cancelled."),
            ),
            (
                "error",
                "failed",
                Some("agent_error"),
                Some("Future Agent run failed."),
            ),
            (
                "mystery",
                "failed",
                Some("stream_interrupted"),
                Some("Future Agent response ended before a clean terminal."),
            ),
        ];
        for (agent_state, status, error_type, default_message) in cases {
            let run = seed_run(&thread.id);
            settle_from_agent_terminal(&run.id, agent_state, None).expect("settle");
            let record = crate::store::get_run(&run.id).expect("run").expect("some");
            assert_eq!(record.status, status, "agent state {agent_state}");
            assert_eq!(record.error_type.as_deref(), error_type);
            assert_eq!(record.error_message.as_deref(), default_message);
        }

        // The agent's own error message wins over the default.
        let run = seed_run(&thread.id);
        settle_from_agent_terminal(&run.id, "error", Some("provider exploded")).expect("settle");
        assert_eq!(
            crate::store::get_run(&run.id)
                .expect("run")
                .expect("some")
                .error_message
                .as_deref(),
            Some("provider exploded")
        );
    }

    // ── attach_remote_stream ──────────────────────────────────────────

    #[tokio::test]
    async fn attach_remote_stream_variants() {
        let home = TestHome::new("pipe-attach");
        let mock = mock_agent();
        let workspace = seed_workspace(home.path(), "ws");

        // Missing thread / missing session.
        let error = attach_remote_stream("no-such-thread")
            .await
            .expect_err("missing");
        assert_eq!(error, "Thread not found");
        let no_session = seed_thread(&workspace.id, None);
        let error = attach_remote_stream(&no_session.id)
            .await
            .expect_err("no session");
        assert_eq!(error, "Thread has no agent session");

        // An active local run short-circuits (no get_state round-trip).
        let thread = seed_thread(&workspace.id, Some("sess-at"));
        let active = seed_run(&thread.id);
        let run_id = attach_remote_stream(&thread.id).await.expect("attach");
        assert_eq!(run_id, active.id);
        assert!(mock.requests_of("get_state").is_empty());

        // Same for a run parked on an approval.
        let thread2 = seed_thread(&workspace.id, Some("sess-at2"));
        let parked = seed_run(&thread2.id);
        crate::store::update_run_status_if_active(crate::store::UpdateRunStatusInput {
            run_id: parked.id.clone(),
            status: "waiting_approval".to_string(),
            error_message: None,
            error_type: None,
        })
        .expect("park");
        let run_id = attach_remote_stream(&thread2.id).await.expect("attach");
        assert_eq!(run_id, parked.id);

        // The short-circuit paths above spawned observers for `sess-at` /
        // `sess-at2`; their async `get_state` probes would otherwise race the
        // scripted reply below (the mock's get_state queue is per command
        // type, not per session). Cancel them so the reply is deterministic.
        super::observer::cancel_all_observers();

        // No local run: the agent's active run gets a local row + observer.
        let thread3 = seed_thread(&workspace.id, Some("sess-at3"));
        mock.push_state_for_session(
            "sess-at3",
            Reply::Data(r#"{"activeRun": {"runId": "run-remote-1"}}"#.to_string()),
        );
        let run_id = attach_remote_stream(&thread3.id).await.expect("attach");
        assert_eq!(run_id, "run-remote-1");
        let row = crate::store::get_run("run-remote-1")
            .expect("run")
            .expect("some");
        assert_eq!(row.thread_id, thread3.id);

        // No active run agent-side → error.
        let thread4 = seed_thread(&workspace.id, Some("sess-at4"));
        mock.push_state_for_session(
            "sess-at4",
            Reply::Data(r#"{"isStreaming": false}"#.to_string()),
        );
        let error = attach_remote_stream(&thread4.id)
            .await
            .expect_err("no active");
        assert_eq!(error, "Agent session has no active canonical run");

        // Transport failure → error.
        let thread5 = seed_thread(&workspace.id, Some("sess-at5"));
        mock.push_state_for_session("sess-at5", Reply::Status(tonic::Code::Unavailable, "down"));
        let error = attach_remote_stream(&thread5.id)
            .await
            .expect_err("transport");
        assert!(error.contains("get_state"), "{error}");
    }

    // ── reconcile_thread_workspace ────────────────────────────────────

    #[tokio::test]
    async fn reconcile_thread_workspace_variants() {
        let home = TestHome::new("pipe-reconcile-ws");
        let _mock = mock_agent();
        let workspace = seed_workspace(home.path(), "ws");

        let error = reconcile_thread_workspace("sess-missing", "/tmp/x").expect_err("no thread");
        assert_eq!(error, "No thread found for this session");

        // Empty cwd is a no-op.
        let thread = seed_thread(&workspace.id, Some("sess-rw"));
        reconcile_thread_workspace("sess-rw", "   ").expect("empty ok");
        assert_eq!(
            crate::store::get_thread(&thread.id)
                .expect("thread")
                .expect("exists")
                .workspace_id,
            workspace.id
        );

        // Chat cwd → rename the chat thread's temporary workspace in place.
        // The rename only applies to threads created in chat mode (their
        // workspace row is the per-thread temporary one).
        let chat_thread = crate::store::create_thread(crate::store::CreateThreadInput {
            mode: "chat".to_string(),
            title: None,
            workspace_id: None,
            workspace_path: None,
            workspace_name: None,
            agent_session_id: Some("sess-rwc".to_string()),
        })
        .expect("chat thread");
        let chat_cwd = format!("{}/.future/workspaces/chat/sess-rwc", home.path().display());
        reconcile_thread_workspace("sess-rwc", &chat_cwd).expect("chat");
        let moved = crate::store::get_thread(&chat_thread.id)
            .expect("thread")
            .expect("exists");
        let moved_ws = crate::store::get_workspace(&moved.workspace_id)
            .expect("ws")
            .expect("exists");
        assert_eq!(moved_ws.path, chat_cwd);

        // Project cwd matching an existing workspace → move there.
        let target = seed_workspace(home.path(), "target");
        let thread2 = seed_thread(&workspace.id, Some("sess-rw2"));
        reconcile_thread_workspace("sess-rw2", &target.path).expect("move");
        assert_eq!(
            crate::store::get_thread(&thread2.id)
                .expect("thread")
                .expect("exists")
                .workspace_id,
            target.id
        );

        // Brand-new project cwd → a workspace is created, then moved to.
        let thread3 = seed_thread(&workspace.id, Some("sess-rw3"));
        let new_dir = home.path().join("brand-new");
        std::fs::create_dir_all(&new_dir).expect("mkdir");
        reconcile_thread_workspace("sess-rw3", &new_dir.display().to_string())
            .expect("create+move");
        let moved = crate::store::get_thread(&thread3.id)
            .expect("thread")
            .expect("exists");
        assert_ne!(moved.workspace_id, workspace.id);
        let created = crate::store::get_workspace(&moved.workspace_id)
            .expect("ws")
            .expect("exists");
        // Stored canonicalized, so the agent's spelling and every other
        // client's map to this one workspace (the temp HOME sits behind
        // macOS's `/var` → `/private/var` symlink), and in the ordinary
        // spelling — Windows' `\\?\` form must not leak into the row the GUI
        // renders and a shell is pointed at.
        let canonical = crate::store::strip_verbatim_prefix(new_dir.canonicalize().expect("canon"));
        assert_eq!(created.path, canonical.display().to_string());
        assert!(
            !created.path.starts_with(r"\\?\"),
            "the verbatim prefix must not leak into a stored workspace path: {}",
            created.path
        );
        assert_eq!(created.name, "brand-new");
    }
}
