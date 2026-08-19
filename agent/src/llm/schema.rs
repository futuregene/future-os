use anyhow::{anyhow, bail, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, HashMap};
use std::str::FromStr;

use crate::types::{AgentMessage, ProviderMetadata, ToolDef, Usage};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ApiProtocol {
    #[serde(rename = "openai-completions")]
    OpenAiChatCompletions,
    #[serde(rename = "openai-responses")]
    OpenAiResponses,
    #[serde(rename = "anthropic")]
    AnthropicMessages,
}

impl FromStr for ApiProtocol {
    type Err = anyhow::Error;

    fn from_str(value: &str) -> Result<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "" | "chat" | "openai" | "completions" | "openai-completions"
            | "openai-chat" | "openai-chat-completions" => Ok(Self::OpenAiChatCompletions),
            "responses" | "openai-responses" => Ok(Self::OpenAiResponses),
            "anthropic" | "anthropic-messages" => Ok(Self::AnthropicMessages),
            other => bail!(
                "unsupported model API protocol `{other}`; expected openai-completions, openai-responses, or anthropic"
            ),
        }
    }
}

impl ApiProtocol {
    pub fn canonical_name(self) -> &'static str {
        match self {
            Self::OpenAiChatCompletions => "openai-completions",
            Self::OpenAiResponses => "openai-responses",
            Self::AnthropicMessages => "anthropic",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthScheme {
    Bearer,
    AnthropicApiKey,
}

#[derive(Debug, Clone)]
pub struct ProviderRoute {
    pub provider_id: String,
    pub base_url: String,
    pub api_key: String,
    pub auth: AuthScheme,
    pub headers: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ChatMaxTokensField {
    #[default]
    MaxTokens,
    MaxCompletionTokens,
}

impl ChatMaxTokensField {
    pub fn key(self) -> &'static str {
        match self {
            Self::MaxTokens => "max_tokens",
            Self::MaxCompletionTokens => "max_completion_tokens",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum ChatReasoningFormat {
    #[default]
    None,
    ReasoningEffort,
    Qwen {
        chat_template: bool,
    },
    DeepSeek,
    Zai,
    ReasoningSplit,
}

#[derive(Debug, Clone, Default)]
pub struct OpenAiChatConfig {
    pub reasoning: ChatReasoningFormat,
    pub supports_reasoning_effort: bool,
    pub replay_assistant_reasoning: bool,
    pub max_tokens_field: ChatMaxTokensField,
    pub tool_stream: bool,
}

#[derive(Debug, Clone)]
pub struct OpenAiResponsesConfig {
    pub store: bool,
    pub include_encrypted_reasoning: bool,
}

impl Default for OpenAiResponsesConfig {
    fn default() -> Self {
        Self {
            store: false,
            include_encrypted_reasoning: true,
        }
    }
}

#[derive(Debug, Clone)]
pub struct AnthropicMessagesConfig {
    pub version: String,
}

impl Default for AnthropicMessagesConfig {
    fn default() -> Self {
        Self {
            version: "2023-06-01".to_string(),
        }
    }
}

#[derive(Debug, Clone)]
pub enum ProtocolConfig {
    OpenAiChat(OpenAiChatConfig),
    OpenAiResponses(OpenAiResponsesConfig),
    AnthropicMessages(AnthropicMessagesConfig),
}

impl ProtocolConfig {
    pub fn protocol(&self) -> ApiProtocol {
        match self {
            Self::OpenAiChat(_) => ApiProtocol::OpenAiChatCompletions,
            Self::OpenAiResponses(_) => ApiProtocol::OpenAiResponses,
            Self::AnthropicMessages(_) => ApiProtocol::AnthropicMessages,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct ReasoningCapabilities {
    pub supported: bool,
    pub levels: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, Default)]
pub struct ModelCapabilities {
    pub supports_text_input: bool,
    pub supports_image_input: bool,
    pub supports_tools: bool,
    pub supports_parallel_tools: bool,
    pub reasoning: ReasoningCapabilities,
    pub context_window: i32,
    pub max_output_tokens: i32,
}

#[derive(Debug, Clone, Default)]
pub struct GenerationConfig {
    pub temperature: Option<f32>,
    pub max_output_tokens: Option<i32>,
    pub thinking_level: String,
    pub thinking_budget: i32,
}

#[derive(Debug, Clone)]
pub struct ResolvedModelTarget {
    pub model_id: String,
    pub route: ProviderRoute,
    pub protocol: ProtocolConfig,
    pub capabilities: ModelCapabilities,
    pub generation: GenerationConfig,
}

#[derive(Debug, Clone)]
pub struct ModelRequest {
    pub model: String,
    pub system_prompt: String,
    pub messages: Vec<AgentMessage>,
    pub tools: Vec<ToolDef>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FinishReason {
    Stop,
    ToolCalls,
    Length,
    ContentFilter,
    Refusal,
    Cancelled,
    Incomplete,
    Error,
    Unknown(String),
}

impl FinishReason {
    pub fn as_str(&self) -> &str {
        match self {
            Self::Stop => "stop",
            Self::ToolCalls => "tool_calls",
            Self::Length => "length",
            Self::ContentFilter => "content_filter",
            Self::Refusal => "refusal",
            Self::Cancelled => "cancelled",
            // Existing UI/RPC consumers use `truncated` for a stream that
            // ended without a complete model response.
            Self::Incomplete => "truncated",
            Self::Error => "error",
            Self::Unknown(value) => value,
        }
    }
}

#[derive(Debug, Clone)]
pub enum ModelStreamEvent {
    TextStart {
        id: String,
    },
    TextDelta {
        id: String,
        text: String,
    },
    TextEnd {
        id: String,
    },
    ReasoningStart {
        id: String,
    },
    ReasoningDelta {
        id: String,
        text: String,
    },
    ReasoningEnd {
        id: String,
        provider_metadata: ProviderMetadata,
    },
    ToolInputStart {
        index: usize,
        id: String,
        name: String,
        /// Optional initial input carried by non-standard Chat streams.
        /// Native Responses/Anthropic streams normally send subsequent deltas.
        arguments: Option<Value>,
        provider_metadata: ProviderMetadata,
    },
    ToolInputDelta {
        index: usize,
        id: String,
        delta: String,
        snapshot: bool,
    },
    ToolInputEnd {
        index: usize,
        id: String,
        name: String,
        arguments: Value,
        provider_metadata: ProviderMetadata,
    },
    Usage(Usage),
    Finish {
        reason: FinishReason,
        usage: Option<Usage>,
    },
    Error {
        message: String,
    },
}

impl ModelStreamEvent {
    pub fn from_legacy(event: crate::types::StreamEvent) -> Vec<Self> {
        use crate::types::ProviderMetadata;
        match event.event_type.as_str() {
            "text_start" => vec![Self::TextStart { id: "text".into() }],
            "text" | "text_delta" => vec![Self::TextDelta {
                id: "text".into(),
                text: event.text,
            }],
            "text_end" => vec![Self::TextEnd { id: "text".into() }],
            "thinking_start" => vec![Self::ReasoningStart {
                id: "reasoning".into(),
            }],
            "thinking_delta" => vec![Self::ReasoningDelta {
                id: "reasoning".into(),
                text: event.text,
            }],
            "thinking_end" => vec![Self::ReasoningEnd {
                id: "reasoning".into(),
                provider_metadata: ProviderMetadata::new(),
            }],
            "toolcall_start" => vec![Self::ToolInputStart {
                index: event.tc_index,
                id: event.tool_id,
                name: event.tool_name,
                arguments: event.tool_call.map(|call| call.function.arguments),
                provider_metadata: ProviderMetadata::new(),
            }],
            "toolcall_delta" => vec![Self::ToolInputDelta {
                index: event.tc_index,
                id: event.tool_id,
                delta: event.text,
                snapshot: false,
            }],
            "tool_call" | "toolcall_end" => vec![Self::ToolInputEnd {
                index: event.tc_index,
                id: event.tool_id,
                name: event.tool_name,
                arguments: event
                    .tool_call
                    .map(|call| call.function.arguments)
                    .unwrap_or(Value::Null),
                provider_metadata: ProviderMetadata::new(),
            }],
            "usage" => event.usage.map(Self::Usage).into_iter().collect(),
            "stop" => vec![Self::Finish {
                reason: match event.stop_reason.as_str() {
                    "" | "stop" => FinishReason::Stop,
                    "tool_calls" => FinishReason::ToolCalls,
                    "length" => FinishReason::Length,
                    "content_filter" => FinishReason::ContentFilter,
                    "truncated" => FinishReason::Incomplete,
                    other => FinishReason::Unknown(other.to_string()),
                },
                usage: event.usage,
            }],
            "error" => vec![Self::Error {
                message: event.error_text,
            }],
            _ => Vec::new(),
        }
    }

    pub fn to_legacy(&self) -> crate::types::StreamEvent {
        use crate::types::{StreamEvent, ToolCall, ToolCallFn};
        match self {
            Self::TextStart { .. } => StreamEvent {
                event_type: "text_start".into(),
                ..Default::default()
            },
            Self::TextDelta { text, .. } => StreamEvent {
                event_type: "text_delta".into(),
                text: text.clone(),
                ..Default::default()
            },
            Self::TextEnd { .. } => StreamEvent {
                event_type: "text_end".into(),
                ..Default::default()
            },
            Self::ReasoningStart { .. } => StreamEvent {
                event_type: "thinking_start".into(),
                ..Default::default()
            },
            Self::ReasoningDelta { text, .. } => StreamEvent {
                event_type: "thinking_delta".into(),
                text: text.clone(),
                ..Default::default()
            },
            Self::ReasoningEnd { .. } => StreamEvent {
                event_type: "thinking_end".into(),
                ..Default::default()
            },
            Self::ToolInputStart {
                index,
                id,
                name,
                arguments,
                ..
            } => StreamEvent {
                event_type: "toolcall_start".into(),
                tool_id: id.clone(),
                tool_name: name.clone(),
                tc_index: *index,
                tool_call: Some(ToolCall {
                    id: id.clone(),
                    call_type: "function".into(),
                    function: ToolCallFn {
                        name: name.clone(),
                        arguments: arguments
                            .clone()
                            .unwrap_or_else(|| Value::String(String::new())),
                    },
                }),
                ..Default::default()
            },
            Self::ToolInputDelta {
                index,
                id,
                delta,
                snapshot,
            } => StreamEvent {
                event_type: "toolcall_delta".into(),
                tool_id: id.clone(),
                text: delta.clone(),
                tc_index: *index,
                payload: Some(serde_json::json!({"snapshot": snapshot})),
                ..Default::default()
            },
            Self::ToolInputEnd {
                index,
                id,
                name,
                arguments,
                ..
            } => StreamEvent {
                event_type: "toolcall_end".into(),
                tool_id: id.clone(),
                tool_name: name.clone(),
                tc_index: *index,
                tool_call: Some(ToolCall {
                    id: id.clone(),
                    call_type: "function".into(),
                    function: ToolCallFn {
                        name: name.clone(),
                        arguments: arguments.clone(),
                    },
                }),
                ..Default::default()
            },
            Self::Usage(usage) => StreamEvent {
                event_type: "usage".into(),
                usage: Some(usage.clone()),
                ..Default::default()
            },
            Self::Finish { reason, usage } => StreamEvent {
                event_type: "stop".into(),
                stop_reason: reason.as_str().to_string(),
                usage: usage.clone(),
                ..Default::default()
            },
            Self::Error { message } => StreamEvent {
                event_type: "error".into(),
                error_text: message.clone(),
                ..Default::default()
            },
        }
    }
}

impl ResolvedModelTarget {
    pub fn from_model(
        model: &crate::models::Model,
        api_key: String,
        temperature: Option<f32>,
        max_output_tokens: Option<i32>,
    ) -> Result<Self> {
        let api = ApiProtocol::from_str(&model.api).map_err(|error| {
            anyhow!(
                "provider `{}` model `{}`: {error}",
                model.provider,
                model.id
            )
        })?;
        let protocol = match api {
            ApiProtocol::OpenAiChatCompletions => {
                ProtocolConfig::OpenAiChat(parse_chat_config(model)?)
            }
            ApiProtocol::OpenAiResponses => {
                ProtocolConfig::OpenAiResponses(OpenAiResponsesConfig::default())
            }
            ApiProtocol::AnthropicMessages => {
                ProtocolConfig::AnthropicMessages(AnthropicMessagesConfig::default())
            }
        };
        let auth = match api {
            ApiProtocol::AnthropicMessages => AuthScheme::AnthropicApiKey,
            _ => AuthScheme::Bearer,
        };

        Ok(Self {
            model_id: model.id.clone(),
            route: ProviderRoute {
                provider_id: model.provider.clone(),
                base_url: model.base_url.clone(),
                api_key,
                auth,
                headers: model
                    .headers
                    .iter()
                    .map(|(key, value)| (key.clone(), value.clone()))
                    .collect(),
            },
            protocol,
            capabilities: ModelCapabilities {
                supports_text_input: model.input.iter().any(|kind| kind == "text"),
                supports_image_input: model.input.iter().any(|kind| kind == "image"),
                supports_tools: true,
                supports_parallel_tools: true,
                reasoning: ReasoningCapabilities {
                    supported: model.reasoning,
                    levels: model
                        .thinking_level_map
                        .iter()
                        .map(|(key, value)| (key.clone(), value.clone()))
                        .collect(),
                },
                context_window: model.context_window,
                max_output_tokens: model.max_tokens,
            },
            generation: GenerationConfig {
                temperature,
                max_output_tokens,
                ..Default::default()
            },
        })
    }

    pub fn legacy_chat(
        base_url: &str,
        api_key: &str,
        temperature: Option<f32>,
        max_output_tokens: Option<i32>,
    ) -> Self {
        Self {
            model_id: String::new(),
            route: ProviderRoute {
                provider_id: String::new(),
                base_url: base_url.to_string(),
                api_key: api_key.to_string(),
                auth: AuthScheme::Bearer,
                headers: BTreeMap::new(),
            },
            protocol: ProtocolConfig::OpenAiChat(OpenAiChatConfig::default()),
            capabilities: ModelCapabilities::default(),
            generation: GenerationConfig {
                temperature,
                max_output_tokens,
                ..Default::default()
            },
        }
    }
}

fn parse_chat_config(model: &crate::models::Model) -> Result<OpenAiChatConfig> {
    let string = |key: &str| -> Result<Option<&str>> {
        match model.compat.get(key) {
            None | Some(Value::Null) => Ok(None),
            Some(Value::String(value)) => Ok(Some(value)),
            Some(_) => bail!(
                "provider `{}` model `{}` compat.{key} must be a string",
                model.provider,
                model.id
            ),
        }
    };
    let boolean = |key: &str| -> Result<bool> {
        match model.compat.get(key) {
            None | Some(Value::Null) => Ok(false),
            Some(Value::Bool(value)) => Ok(*value),
            Some(_) => bail!(
                "provider `{}` model `{}` compat.{key} must be a boolean",
                model.provider,
                model.id
            ),
        }
    };

    let reasoning = match string("thinkingFormat")?.unwrap_or("") {
        "" => ChatReasoningFormat::None,
        "openai" | "openrouter" => ChatReasoningFormat::ReasoningEffort,
        "qwen" => ChatReasoningFormat::Qwen {
            chat_template: false,
        },
        "qwen-chat-template" => ChatReasoningFormat::Qwen {
            chat_template: true,
        },
        "deepseek" => ChatReasoningFormat::DeepSeek,
        "zai" => ChatReasoningFormat::Zai,
        "reasoning-split" => ChatReasoningFormat::ReasoningSplit,
        other => bail!(
            "provider `{}` model `{}` has unsupported compat.thinkingFormat `{other}`",
            model.provider,
            model.id
        ),
    };
    let max_tokens_field = match string("maxTokensField")?.unwrap_or("max_tokens") {
        "max_tokens" => ChatMaxTokensField::MaxTokens,
        "max_completion_tokens" => ChatMaxTokensField::MaxCompletionTokens,
        other => bail!(
            "provider `{}` model `{}` has unsupported compat.maxTokensField `{other}`",
            model.provider,
            model.id
        ),
    };

    Ok(OpenAiChatConfig {
        reasoning,
        supports_reasoning_effort: boolean("supportsReasoningEffort")?,
        replay_assistant_reasoning: boolean("requiresReasoningContentOnAssistantMessages")?,
        max_tokens_field,
        tool_stream: boolean("toolStream")? || model.provider == "zai",
    })
}

pub fn string_level_map(values: &HashMap<String, Value>) -> HashMap<String, String> {
    values
        .iter()
        .filter_map(|(key, value)| value.as_str().map(|value| (key.clone(), value.to_string())))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn protocol_aliases_normalize() {
        assert_eq!(
            ApiProtocol::from_str("chat").unwrap(),
            ApiProtocol::OpenAiChatCompletions
        );
        assert_eq!(
            ApiProtocol::from_str("responses").unwrap(),
            ApiProtocol::OpenAiResponses
        );
        assert_eq!(
            ApiProtocol::from_str("anthropic-messages").unwrap(),
            ApiProtocol::AnthropicMessages
        );
        assert!(ApiProtocol::from_str("unknown").is_err());
    }

    #[test]
    fn model_compat_resolves_to_typed_chat_config() {
        let mut model = crate::models::Model {
            id: "qwen".into(),
            provider: "dashscope".into(),
            api: "openai-completions".into(),
            base_url: "https://example.test/v1".into(),
            reasoning: true,
            input: vec!["text".into(), "image".into()],
            ..Default::default()
        };
        model
            .compat
            .insert("thinkingFormat".into(), Value::String("qwen".into()));
        model.compat.insert(
            "maxTokensField".into(),
            Value::String("max_completion_tokens".into()),
        );

        let target = ResolvedModelTarget::from_model(&model, "key".into(), None, None).unwrap();
        let ProtocolConfig::OpenAiChat(chat) = target.protocol else {
            panic!("chat protocol expected")
        };
        assert_eq!(
            chat.reasoning,
            ChatReasoningFormat::Qwen {
                chat_template: false
            }
        );
        assert_eq!(
            chat.max_tokens_field,
            ChatMaxTokensField::MaxCompletionTokens
        );
        assert!(target.capabilities.supports_image_input);
    }
}
