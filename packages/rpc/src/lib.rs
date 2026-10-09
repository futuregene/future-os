//! future-rpc — the wire contract between FutureAgent and its clients.
//!
//! This crate is the single owner of:
//! - the generated proto code (`proto` module, from `packages/rpc/proto/future.proto`),
//!   emitting both tonic server and client modules;
//! - (incoming batches) the payload structs and the encode/decode layer for
//!   the typed `RpcResponse.payload` / `StreamEvent.payload` oneofs and the
//!   transitional JSON-`data` fallback.
//!
//! Consumers: `future-agent` (server side), `future-channel` and the desktop
//! Tauri backend (client side). Dependency direction is strictly one-way:
//! this crate also owns the cross-client local transport so every frontend
//! discovers the same per-user Agent endpoint.

pub mod command_policy;
pub mod home;
pub mod shell_result;
pub mod transport;

pub mod proto {
    // The typed-payload oneofs deliberately mix large variants (a full
    // SessionState) with small ack messages; the size spread is inherent to a
    // wire oneof, so silence the lint here rather than boxing generated code.
    #![allow(clippy::large_enum_variant)]
    include!("generated/proto.rs");
}

pub mod event_payloads;
pub mod message;
pub mod payloads;
pub mod payloads_ext;

pub mod decode;
pub mod encode;
pub mod events;

#[cfg(test)]
mod parity;

#[cfg(test)]
mod tests {
    use super::proto::{response_payload, PromptAck, RpcResponse, SessionState, StreamEvent};
    use prost::Message;

    /// Extract the GetState variant, panicking on anything else. The panic
    /// arm stays covered by `expect_get_state_rejects_other_kinds`.
    fn expect_get_state(kind: response_payload::Kind) -> SessionState {
        match kind {
            response_payload::Kind::GetState(state) => state,
            other => panic!("unexpected payload kind: {other:?}"),
        }
    }

    #[test]
    #[should_panic(expected = "unexpected payload kind")]
    fn expect_get_state_rejects_other_kinds() {
        expect_get_state(response_payload::Kind::Prompt(PromptAck::default()));
    }

    /// Smoke test: generated types round-trip through the wire encoding.
    #[test]
    fn rpc_response_roundtrip() {
        let resp = RpcResponse {
            id: "req-1".to_string(),
            r#type: "response".to_string(),
            command: "get_state".to_string(),
            success: true,
            data: r#"{"sessionId":"s1"}"#.to_string(),
            error: String::new(),
            error_code: String::new(),
            error_data: String::new(),
            payload: None,
        };
        let bytes = resp.encode_to_vec();
        let decoded = RpcResponse::decode(bytes.as_slice()).unwrap();
        assert_eq!(resp, decoded);
    }

    #[test]
    fn stream_event_roundtrip() {
        let event = StreamEvent {
            r#type: "text_chunk".to_string(),
            data: r#"{"text":"hi"}"#.to_string(),
            run_id: "run-1".to_string(),
            idx: 3,
            ..Default::default()
        };
        let bytes = event.encode_to_vec();
        let decoded = StreamEvent::decode(bytes.as_slice()).unwrap();
        assert_eq!(event, decoded);
    }

    /// Typed payload oneofs round-trip through the wire encoding alongside the
    /// dual-written JSON `data` string.
    #[test]
    fn typed_response_payload_roundtrip() {
        use super::proto::{
            event_payload, response_payload, EventPayload, ResponsePayload, SessionState, ToolEnd,
        };

        let resp = RpcResponse {
            id: "req-1".to_string(),
            r#type: "response".to_string(),
            command: "get_state".to_string(),
            success: true,
            data: r#"{"sessionId":"s1"}"#.to_string(),
            error: String::new(),
            error_code: String::new(),
            error_data: String::new(),
            payload: Some(ResponsePayload {
                kind: Some(response_payload::Kind::GetState(SessionState {
                    session_id: Some("s1".to_string()),
                    model: "m".to_string(),
                    ..Default::default()
                })),
            }),
        };
        let decoded = RpcResponse::decode(resp.encode_to_vec().as_slice()).unwrap();
        assert_eq!(resp, decoded);
        let kind = decoded.payload.unwrap().kind.unwrap();
        let state = expect_get_state(kind);
        assert_eq!(state.session_id.as_deref(), Some("s1"));

        let event = StreamEvent {
            r#type: "tool_end".to_string(),
            data: r#"{"tool_id":"t1"}"#.to_string(),
            payload: Some(EventPayload {
                kind: Some(event_payload::Kind::ToolEnd(ToolEnd {
                    tool_id: "t1".to_string(),
                    text: "ok".to_string(),
                    exit_code: Some(1),
                    is_soft_fail: Some(true),
                    ..Default::default()
                })),
            }),
            ..Default::default()
        };
        let decoded = StreamEvent::decode(event.encode_to_vec().as_slice()).unwrap();
        assert_eq!(event, decoded);
    }
}

#[cfg(test)]
mod automatic_approval_wire_tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn typed_assessment_round_trips_without_json_fallback() {
        let value = json!({"assessment_id":"a","approval_request_id":"a","tool_call_id":"t","reviewer":"model","status":"approved","reported":{"risk":"low","authorization":"high","reason_code":"routine_bounded_action"},"effective":{"risk":"low","authorization":"high"},"confidence":{"risk":0.95,"authorization":0.96,"reason_code":0.99},"probabilities":{"risk":{"low":0.95}},"model":"jev","provider_request_id":"provider","error_code":null,"action":{"command":"pwd"},"action_digest":"sha256:fixture","input_context":{"source_ids":["user-entry"],"budget":{"estimated_state_tokens":600}},"attempt":1,"prompt_version":1,"reason_catalog_version":1,"policy_version":1,"duration_ms":50});
        let payload = encode::event_payload("approval_assessment", &value.to_string())
            .expect("typed payload");
        let event = proto::StreamEvent {
            r#type: "approval_assessment".into(),
            payload: Some(payload),
            ..Default::default()
        };
        let decoded = decode::event_data(&event);
        assert_eq!(decoded["action"], value["action"]);
        assert_eq!(decoded["confidence"], value["confidence"]);
        assert_eq!(decoded["reported"], value["reported"]);
        assert_eq!(decoded["status"], "approved");
        assert_eq!(decoded["action_digest"], value["action_digest"]);
        assert_eq!(decoded["input_context"], value["input_context"]);
    }
}
