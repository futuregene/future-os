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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AnthropicThinkingMode {
    #[default]
    Manual,
    Adaptive,
}

#[derive(Debug, Clone)]
pub struct AnthropicMessagesConfig {
    pub version: String,
    pub thinking_mode: AnthropicThinkingMode,
}

impl Default for AnthropicMessagesConfig {
    fn default() -> Self {
        Self {
            version: "2023-06-01".to_string(),
            thinking_mode: AnthropicThinkingMode::Manual,
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
                ProtocolConfig::AnthropicMessages(parse_anthropic_config(model)?)
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

    pub fn openai_chat_compatible(
        model_id: impl Into<String>,
        base_url: &str,
        api_key: &str,
        temperature: Option<f32>,
        max_output_tokens: Option<i32>,
    ) -> Self {
        Self {
            model_id: model_id.into(),
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

fn parse_anthropic_config(model: &crate::models::Model) -> Result<AnthropicMessagesConfig> {
    let configured = match model.compat.get("anthropicThinkingMode") {
        None | Some(Value::Null) => None,
        Some(Value::String(value)) => Some(value.as_str()),
        Some(_) => {
            bail!(
                "provider `{}` model `{}` compat.anthropicThinkingMode must be a string",
                model.provider,
                model.id
            )
        }
    };
    let thinking_mode = match configured {
        Some("manual") => AnthropicThinkingMode::Manual,
        Some("adaptive") => AnthropicThinkingMode::Adaptive,
        Some(other) => {
            bail!(
                "provider `{}` model `{}` has unsupported compat.anthropicThinkingMode `{other}`",
                model.provider,
                model.id
            )
        }
        None if anthropic_model_uses_adaptive_thinking(&model.id) => {
            AnthropicThinkingMode::Adaptive
        }
        None => AnthropicThinkingMode::Manual,
    };
    Ok(AnthropicMessagesConfig {
        thinking_mode,
        ..Default::default()
    })
}

fn anthropic_model_uses_adaptive_thinking(model_id: &str) -> bool {
    let id = model_id.to_ascii_lowercase().replace('.', "-");
    [
        "opus-4-6",
        "opus-4-7",
        "opus-4-8",
        "opus-5",
        "sonnet-4-6",
        "sonnet-5",
        "fable-5",
        "mythos-5",
        "opus-latest",
        "sonnet-latest",
    ]
    .iter()
    .any(|marker| id.contains(marker))
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
        "" => {
            // Preserve the legacy runtime auto-detect: a dashscope/aliyuncs
            // endpoint without an explicit thinkingFormat is Qwen.
            if model.base_url.contains("dashscope") || model.base_url.contains("aliyuncs") {
                ChatReasoningFormat::Qwen {
                    chat_template: false,
                }
            } else {
                ChatReasoningFormat::None
            }
        }
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

    #[test]
    fn chat_config_autodetects_qwen_from_dashscope_base_url() {
        let qwen = crate::models::Model {
            id: "qwen-flash".into(),
            provider: "custom".into(),
            api: "openai-completions".into(),
            base_url: "https://dashscope.aliyuncs.com/compatible-mode/v1".into(),
            reasoning: true,
            ..Default::default()
        };
        let target = ResolvedModelTarget::from_model(&qwen, "key".into(), None, None).unwrap();
        let ProtocolConfig::OpenAiChat(chat) = target.protocol else {
            panic!("chat protocol expected")
        };
        assert_eq!(
            chat.reasoning,
            ChatReasoningFormat::Qwen {
                chat_template: false
            }
        );

        // A non-dashscope base_url without an explicit thinkingFormat stays None.
        let plain = crate::models::Model {
            base_url: "https://api.example.test/v1".into(),
            ..qwen
        };
        let target = ResolvedModelTarget::from_model(&plain, "key".into(), None, None).unwrap();
        let ProtocolConfig::OpenAiChat(chat) = target.protocol else {
            panic!("chat protocol expected")
        };
        assert_eq!(chat.reasoning, ChatReasoningFormat::None);
    }

    #[test]
    fn anthropic_thinking_mode_uses_model_generation_with_explicit_override() {
        let mut model = crate::models::Model {
            id: "claude-opus-4-8".into(),
            provider: "anthropic".into(),
            api: "anthropic".into(),
            ..Default::default()
        };
        let target = ResolvedModelTarget::from_model(&model, "key".into(), None, None).unwrap();
        let ProtocolConfig::AnthropicMessages(config) = target.protocol else {
            panic!("Anthropic protocol expected")
        };
        assert_eq!(config.thinking_mode, AnthropicThinkingMode::Adaptive);

        model.compat.insert(
            "anthropicThinkingMode".into(),
            Value::String("manual".into()),
        );
        let target = ResolvedModelTarget::from_model(&model, "key".into(), None, None).unwrap();
        let ProtocolConfig::AnthropicMessages(config) = target.protocol else {
            panic!("Anthropic protocol expected")
        };
        assert_eq!(config.thinking_mode, AnthropicThinkingMode::Manual);
    }
}
