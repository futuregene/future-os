//! future-rpc — the wire contract between FutureAgent and its clients.
//!
//! This crate is the single owner of:
//! - the generated proto code (`proto` module, from `proto/future.proto`),
//!   emitting both tonic server and client modules;
//! - (incoming batches) the payload structs and the encode/decode layer for
//!   the typed `RpcResponse.payload` / `StreamEvent.payload` oneofs and the
//!   transitional JSON-`data` fallback.
//!
//! Consumers: `future-agent` (server side), `future-channel` and the GUI
//! Tauri backend (client side). Dependency direction is strictly one-way:
//! this crate depends only on tonic/prost/serde.

pub mod proto {
    include!("generated/proto.rs");
}

#[cfg(test)]
mod tests {
    use super::proto::{RpcResponse, StreamEvent};
    use prost::Message;

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
}
