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
}
