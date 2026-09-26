mod anthropic;
mod openai_chat;
mod openai_responses;

use super::schema::{ApiProtocol, ModelRequest, ModelStreamEvent, ResolvedModelTarget};
use super::sse::SseFrame;
use anyhow::{anyhow, Result};
use serde_json::Value;
use std::any::Any;
use std::collections::HashMap;
use std::sync::Arc;

pub trait ProtocolAdapter: Send + Sync {
    fn protocol(&self) -> ApiProtocol;
    fn endpoint_path(&self) -> &'static str;
    fn build_body(&self, target: &ResolvedModelTarget, request: &ModelRequest) -> Result<Value>;
    fn new_stream_state(&self) -> Box<dyn Any + Send>;
    fn decode_frame(
        &self,
        frame: &SseFrame,
        state: &mut (dyn Any + Send),
    ) -> Result<Vec<ModelStreamEvent>>;
    fn finish_stream(&self, state: &mut (dyn Any + Send)) -> Result<Vec<ModelStreamEvent>>;

    /// True only after a wire-level terminator (or terminal provider error).
    /// A logical Finish can precede trailing usage, notably in Chat Completions.
    /// Adapters without an explicit terminator continue reading until HTTP EOF.
    fn is_stream_complete(&self, _state: &(dyn Any + Send)) -> bool {
        false
    }
}

#[derive(Clone)]
pub struct AdapterRegistry {
    adapters: HashMap<ApiProtocol, Arc<dyn ProtocolAdapter>>,
}

impl Default for AdapterRegistry {
    fn default() -> Self {
        let mut registry = Self {
            adapters: HashMap::new(),
        };
        registry.register(openai_chat::OpenAiChatAdapter);
        registry.register(openai_responses::OpenAiResponsesAdapter);
        registry.register(anthropic::AnthropicMessagesAdapter);
        registry
    }
}

impl AdapterRegistry {
    pub fn register(&mut self, adapter: impl ProtocolAdapter + 'static) {
        self.adapters.insert(adapter.protocol(), Arc::new(adapter));
    }

    pub fn get(&self, protocol: ApiProtocol) -> Result<Arc<dyn ProtocolAdapter>> {
        self.adapters
            .get(&protocol)
            .cloned()
            .ok_or_else(|| anyhow!("no adapter registered for {}", protocol.canonical_name()))
    }
}

/// Gateways encode token counts as JSON integers, floats, or numeric strings.
pub(super) fn token_count(value: &Value) -> Option<i64> {
    value.as_i64().or_else(|| {
        value
            .as_f64()
            .or_else(|| value.as_str()?.parse::<f64>().ok())
            .filter(|number| number.is_finite())
            .map(|number| number as i64)
    })
}

pub(super) fn namespaced_metadata(namespace: &str, value: Value) -> crate::types::ProviderMetadata {
    let mut metadata = crate::types::ProviderMetadata::new();
    metadata.insert(namespace.to_string(), value);
    metadata
}

pub(super) fn parse_json_arguments(value: &Value) -> Value {
    match value {
        Value::String(text) => serde_json::from_str(text).unwrap_or_else(|_| value.clone()),
        other => other.clone(),
    }
}

pub(super) fn data_url(url: &str) -> Option<(&str, &str)> {
    let data = url.strip_prefix("data:")?;
    let (header, payload) = data.split_once(',')?;
    let media_type = header.strip_suffix(";base64")?;
    Some((media_type, payload))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_registry_resolves_all_three_protocols() {
        let registry = AdapterRegistry::default();
        for protocol in [
            ApiProtocol::OpenAiChatCompletions,
            ApiProtocol::OpenAiResponses,
            ApiProtocol::AnthropicMessages,
        ] {
            assert_eq!(registry.get(protocol).unwrap().protocol(), protocol);
        }
    }

    /// The default must be "not complete": an adapter without a wire-level
    /// terminator has to keep reading until HTTP EOF, or a trailing usage frame
    /// (Chat Completions sends one after the logical finish) would be dropped.
    #[test]
    fn an_adapter_without_a_terminator_never_reports_the_stream_complete() {
        struct NoTerminator;

        impl ProtocolAdapter for NoTerminator {
            fn protocol(&self) -> ApiProtocol {
                ApiProtocol::OpenAiChatCompletions
            }
            fn endpoint_path(&self) -> &'static str {
                "/v1/chat/completions"
            }
            fn build_body(&self, _: &ResolvedModelTarget, _: &ModelRequest) -> Result<Value> {
                Ok(Value::Null)
            }
            fn new_stream_state(&self) -> Box<dyn Any + Send> {
                Box::new(())
            }
            fn decode_frame(
                &self,
                _: &SseFrame,
                _: &mut (dyn Any + Send),
            ) -> Result<Vec<ModelStreamEvent>> {
                Ok(vec![])
            }
            fn finish_stream(&self, _: &mut (dyn Any + Send)) -> Result<Vec<ModelStreamEvent>> {
                Ok(vec![])
            }
        }

        let adapter: Arc<dyn ProtocolAdapter> = Arc::new(NoTerminator);
        assert!(!adapter.is_stream_complete(&()));
        // The stub exists to satisfy the trait, so drive every method it
        // implements: if one of them stopped matching the trait contract (e.g.
        // `decode_frame` started reporting completion), the default
        // `is_stream_complete` assertion above would no longer mean what it says.
        assert_eq!(adapter.protocol(), ApiProtocol::OpenAiChatCompletions);
        assert_eq!(adapter.endpoint_path(), "/v1/chat/completions");
        let target =
            ResolvedModelTarget::openai_chat_compatible("m", "https://x.test/v1", "k", None, None);
        let request = ModelRequest {
            model: "m".to_string(),
            system_prompt: String::new(),
            messages: Vec::new(),
            tools: Vec::new(),
        };
        assert_eq!(
            adapter.build_body(&target, &request).unwrap(),
            Value::Null,
            "the stub builds no body"
        );
        let mut state = adapter.new_stream_state();
        assert!(adapter
            .decode_frame(
                &SseFrame {
                    event: None,
                    data: String::new(),
                },
                state.as_mut(),
            )
            .unwrap()
            .is_empty());
        // `finish_stream` must stay side-effect free and empty: a terminator-less
        // adapter reports nothing at EOF.
        assert!(adapter.finish_stream(state.as_mut()).unwrap().is_empty());
        assert!(adapter.finish_stream(state.as_mut()).unwrap().is_empty());
    }

    /// `register` is public, so a registry can legitimately be missing a
    /// protocol; the lookup error must name it rather than fail silently.
    #[test]
    fn an_unregistered_protocol_is_named_in_the_lookup_error() {
        let mut registry = AdapterRegistry::default();
        registry.adapters.clear();
        let err = registry
            .get(ApiProtocol::OpenAiResponses)
            .err()
            .expect("the empty registry must not resolve a protocol");
        assert_eq!(
            err.to_string(),
            "no adapter registered for openai-responses"
        );
    }

    /// Gateways disagree on how a token count is encoded on the wire; the
    /// helper has to accept all three shapes and reject what is not a number.
    #[test]
    fn token_count_accepts_integer_float_and_numeric_string_encodings() {
        assert_eq!(token_count(&serde_json::json!(7)), Some(7));
        assert_eq!(token_count(&serde_json::json!(-3)), Some(-3));
        assert_eq!(token_count(&serde_json::json!(0)), Some(0));
        // Truncation, never rounding: a report of 7.9 tokens is 7 tokens.
        assert_eq!(token_count(&serde_json::json!(7.9)), Some(7));
        assert_eq!(token_count(&serde_json::json!(-7.9)), Some(-7));
        assert_eq!(token_count(&serde_json::json!("12")), Some(12));
        assert_eq!(token_count(&serde_json::json!("12.5")), Some(12));
        // Not a number, or not finite: absent rather than a bogus 0 / i64::MAX.
        assert_eq!(token_count(&serde_json::json!(" 12 ")), None);
        assert_eq!(token_count(&serde_json::json!("twelve")), None);
        assert_eq!(token_count(&serde_json::json!("NaN")), None);
        assert_eq!(token_count(&serde_json::json!("inf")), None);
        assert_eq!(token_count(&serde_json::json!("-inf")), None);
        assert_eq!(token_count(&serde_json::json!(true)), None);
        assert_eq!(token_count(&serde_json::json!(null)), None);
        assert_eq!(token_count(&serde_json::json!([1, 2])), None);
        assert_eq!(token_count(&serde_json::json!({"n": 1})), None);
    }

    #[test]
    fn data_url_parses_base64_media_urls_and_rejects_everything_else() {
        assert_eq!(
            data_url("data:image/png;base64,AAAA"),
            Some(("image/png", "AAAA"))
        );
        // The payload is taken verbatim, so a comma inside it stays payload.
        assert_eq!(
            data_url("data:text/plain;base64,AA,BB"),
            Some(("text/plain", "AA,BB"))
        );
        assert_eq!(data_url("data:;base64,AAAA"), Some(("", "AAAA")));
        // No `;base64` marker, or nothing at all after the comma: not readable.
        assert_eq!(data_url("data:image/png,AAAA"), None);
        assert_eq!(data_url("data:image/png;base64"), None);
        assert_eq!(data_url("data:image/png;base64,"), Some(("image/png", "")));
        assert_eq!(data_url("https://example.test/a.png"), None);
        assert_eq!(data_url("image/png;base64,AAAA"), None);
        assert_eq!(data_url(""), None);
        // The scheme is matched literally, not case-insensitively.
        assert_eq!(data_url("DATA:image/png;base64,AAAA"), None);
    }
}
