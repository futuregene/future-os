use anyhow::{anyhow, bail, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
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
    /// Echo the assistant turn's `reasoning_content` back in history
    /// (`compat.requiresReasoningContentOnAssistantMessages`).
    ///
    /// `thinkingFormat: "deepseek"` implies it: DeepSeek's thinking mode
    /// requires the reasoning of a previous assistant turn on the follow-up
    /// request and answers a tool-call round trip without it with HTTP 400
    /// ("The `reasoning_content` in the thinking mode must be passed back to
    /// the API"). An explicit compat boolean still wins over the implication.
    pub replay_assistant_reasoning: bool,
    pub max_tokens_field: ChatMaxTokensField,
    pub tool_stream: bool,
}

#[derive(Debug, Clone)]
pub struct OpenAiResponsesConfig {
    pub store: bool,
    pub include_encrypted_reasoning: bool,
    pub supports_reasoning_summary: bool,
    pub reasoning_context: Option<String>,
    pub reasoning_mode: Option<String>,
    pub prompt_cache_options: Option<Value>,
}

impl Default for OpenAiResponsesConfig {
    fn default() -> Self {
        Self {
            store: false,
            include_encrypted_reasoning: true,
            supports_reasoning_summary: true,
            reasoning_context: None,
            reasoning_mode: None,
            prompt_cache_options: None,
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

/// The reserved level meaning "send no thinking/effort parameter at all".
///
/// It is implicit: every model offers it, it is never stored in a configured
/// list, and it is always shown first.
pub const DEFAULT_THINKING_LEVEL: &str = "default";

/// Level names the product ships translations for.
///
/// This is not a wire vocabulary: a model's `thinkingLevels` may contain any
/// other string and it is sent verbatim. The list exists for pickers' fallback
/// text, CLI validation and the custom-provider checkbox UI.
pub const KNOWN_THINKING_LEVELS: [&str; 7] =
    ["off", "minimal", "low", "medium", "high", "xhigh", "max"];

/// Maximum configured levels per model, and maximum length of one level name.
pub const MAX_THINKING_LEVELS: usize = 16;
pub const MAX_THINKING_LEVEL_LEN: usize = 32;

#[derive(Debug, Clone, Default)]
pub struct ReasoningCapabilities {
    /// True when the model declares at least one level other than `off`.
    /// Derived from `Model::thinking_levels`; `Model::reasoning` is no longer
    /// consulted anywhere.
    pub supported: bool,
}

/// Normalize a configured level list: trim, drop empties and the reserved
/// `default`, dedupe preserving declared order, cap the count and each name's
/// length. A dropped entry is logged against the provider/model so a typo is
/// visible without failing the whole model.
pub fn normalize_thinking_levels(
    provider: &str,
    model: &str,
    declared: impl IntoIterator<Item = String>,
) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut dropped_reserved = false;
    let mut truncated = false;
    for raw in declared {
        let level = raw.trim().to_string();
        if level.is_empty() {
            continue;
        }
        if level == DEFAULT_THINKING_LEVEL {
            dropped_reserved = true;
            continue;
        }
        if level.chars().count() > MAX_THINKING_LEVEL_LEN {
            truncated = true;
            continue;
        }
        if out.iter().any(|known| known == &level) {
            continue;
        }
        if out.len() >= MAX_THINKING_LEVELS {
            truncated = true;
            break;
        }
        out.push(level);
    }
    if dropped_reserved {
        tracing::warn!(
            provider,
            model,
            "[models] thinkingLevels must not list the reserved `default`; it is implicit and always first"
        );
    }
    if truncated {
        tracing::warn!(
            provider,
            model,
            "[models] ignoring thinkingLevels entries beyond {MAX_THINKING_LEVELS} levels / {MAX_THINKING_LEVEL_LEN} chars"
        );
    }
    out
}

/// Clamp `level` onto a model's effective list (which always contains
/// `default`). A level the model does not offer becomes `default`: the value is
/// inert there, and falling back to the provider default is the only option
/// that cannot silently spend more than the user sees.
pub fn clamp_thinking_level(level: &str, allowed: &[String]) -> String {
    let level = level.trim();
    if level.is_empty() || level == DEFAULT_THINKING_LEVEL {
        return DEFAULT_THINKING_LEVEL.to_string();
    }
    if allowed.iter().any(|name| name == level) {
        return level.to_string();
    }
    DEFAULT_THINKING_LEVEL.to_string()
}

/// The value to put on the wire for a level.
///
/// `default` sends nothing at all; `off` sends nothing here (each adapter turns
/// it into its protocol's explicit "disabled" spelling, when it has one); any
/// other value is the model's wire value and travels verbatim.
pub fn effort_value(level: &str) -> Option<&str> {
    let level = level.trim();
    if level.is_empty() || level == DEFAULT_THINKING_LEVEL || level == "off" {
        None
    } else {
        Some(level)
    }
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
    Paused,
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
            Self::Paused => "pause_turn",
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
                ProtocolConfig::OpenAiResponses(parse_responses_config(model)?)
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
                    // Derived from the declared wire values, never from the
                    // legacy `reasoning` flag (which is kept for older clients
                    // but has no runtime meaning).
                    supported: model
                        .thinking_levels
                        .iter()
                        .any(|level| level.as_str() != "off"),
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

fn parse_responses_config(model: &crate::models::Model) -> Result<OpenAiResponsesConfig> {
    let optional_string = |key: &str| -> Result<Option<String>> {
        match model.compat.get(key) {
            None | Some(Value::Null) => Ok(None),
            Some(Value::String(value)) => Ok(Some(value.clone())),
            Some(_) => bail!(
                "provider `{}` model `{}` compat.{key} must be a string",
                model.provider,
                model.id
            ),
        }
    };
    let optional_boolean = |key: &str| -> Result<Option<bool>> {
        match model.compat.get(key) {
            None | Some(Value::Null) => Ok(None),
            Some(Value::Bool(value)) => Ok(Some(*value)),
            Some(_) => bail!(
                "provider `{}` model `{}` compat.{key} must be a boolean",
                model.provider,
                model.id
            ),
        }
    };

    let reasoning_context = optional_string("responsesReasoningContext")?;
    if let Some(value) = reasoning_context.as_deref() {
        if !matches!(value, "auto" | "current_turn" | "all_turns") {
            bail!(
                "provider `{}` model `{}` has unsupported compat.responsesReasoningContext `{value}`",
                model.provider,
                model.id
            );
        }
    }
    let reasoning_mode = optional_string("responsesReasoningMode")?;
    if let Some(value) = reasoning_mode.as_deref() {
        if !matches!(value, "standard" | "pro") {
            bail!(
                "provider `{}` model `{}` has unsupported compat.responsesReasoningMode `{value}`",
                model.provider,
                model.id
            );
        }
    }
    let prompt_cache_options = match model.compat.get("responsesPromptCacheOptions") {
        None | Some(Value::Null) => None,
        Some(Value::Object(value)) => Some(Value::Object(value.clone())),
        Some(_) => bail!(
            "provider `{}` model `{}` compat.responsesPromptCacheOptions must be an object",
            model.provider,
            model.id
        ),
    };

    Ok(OpenAiResponsesConfig {
        supports_reasoning_summary: optional_boolean("supportsReasoningSummary")?.unwrap_or(true),
        reasoning_context,
        reasoning_mode,
        prompt_cache_options,
        ..Default::default()
    })
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
    // The `deepseek` wire format's thinking mode requires the assistant turn's
    // `reasoning_content` to be echoed back on the follow-up request; without
    // it the API refuses the whole turn with HTTP 400 ("The `reasoning_content`
    // in the thinking mode must be passed back to the API"). That is a
    // property of the format, not of one registry entry, so the format alone
    // enables the replay — an explicit compat value still overrides it.
    let replay_assistant_reasoning = match model
        .compat
        .get("requiresReasoningContentOnAssistantMessages")
    {
        None | Some(Value::Null) => matches!(reasoning, ChatReasoningFormat::DeepSeek),
        Some(Value::Bool(value)) => *value,
        Some(_) => bail!(
            "provider `{}` model `{}` compat.requiresReasoningContentOnAssistantMessages must be a boolean",
            model.provider,
            model.id
        ),
    };

    Ok(OpenAiChatConfig {
        reasoning,
        replay_assistant_reasoning,
        max_tokens_field,
        tool_stream: boolean("toolStream")? || model.provider == "zai",
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn expect_chat(protocol: ProtocolConfig) -> OpenAiChatConfig {
        match protocol {
            ProtocolConfig::OpenAiChat(chat) => chat,
            other => panic!("chat protocol expected, got {other:?}"),
        }
    }

    fn expect_anthropic(protocol: ProtocolConfig) -> AnthropicMessagesConfig {
        match protocol {
            ProtocolConfig::AnthropicMessages(config) => config,
            other => panic!("Anthropic protocol expected, got {other:?}"),
        }
    }

    fn expect_responses(protocol: ProtocolConfig) -> OpenAiResponsesConfig {
        match protocol {
            ProtocolConfig::OpenAiResponses(config) => config,
            other => panic!("Responses protocol expected, got {other:?}"),
        }
    }

    #[test]
    fn protocol_canonical_names() {
        assert_eq!(
            ApiProtocol::OpenAiChatCompletions.canonical_name(),
            "openai-completions"
        );
        assert_eq!(
            ApiProtocol::OpenAiResponses.canonical_name(),
            "openai-responses"
        );
        assert_eq!(ApiProtocol::AnthropicMessages.canonical_name(), "anthropic");
    }

    #[test]
    fn max_tokens_field_keys() {
        assert_eq!(ChatMaxTokensField::MaxTokens.key(), "max_tokens");
        assert_eq!(
            ChatMaxTokensField::MaxCompletionTokens.key(),
            "max_completion_tokens"
        );
    }

    #[test]
    fn finish_reason_as_str_covers_every_variant() {
        assert_eq!(FinishReason::Stop.as_str(), "stop");
        assert_eq!(FinishReason::ToolCalls.as_str(), "tool_calls");
        assert_eq!(FinishReason::Length.as_str(), "length");
        assert_eq!(FinishReason::ContentFilter.as_str(), "content_filter");
        assert_eq!(FinishReason::Refusal.as_str(), "refusal");
        assert_eq!(FinishReason::Cancelled.as_str(), "cancelled");
        assert_eq!(FinishReason::Incomplete.as_str(), "truncated");
        assert_eq!(FinishReason::Error.as_str(), "error");
        assert_eq!(FinishReason::Unknown("custom".into()).as_str(), "custom");
    }

    #[test]
    fn from_model_rejects_unknown_protocol() {
        let model = crate::models::Model {
            id: "m".into(),
            provider: "p".into(),
            api: "bogus".into(),
            ..Default::default()
        };
        let error = ResolvedModelTarget::from_model(&model, "key".into(), None, None).unwrap_err();
        assert!(error.to_string().contains("unsupported model API protocol"));
    }

    #[test]
    fn from_model_resolves_responses_protocol() {
        let model = crate::models::Model {
            id: "m".into(),
            provider: "p".into(),
            api: "responses".into(),
            ..Default::default()
        };
        let target = ResolvedModelTarget::from_model(&model, "key".into(), None, None).unwrap();
        assert!(matches!(
            target.protocol,
            ProtocolConfig::OpenAiResponses(_)
        ));
    }

    #[test]
    fn responses_config_reads_latest_optional_capabilities() {
        let mut model = crate::models::Model {
            id: "gpt-5.6-sol".into(),
            provider: "openai".into(),
            api: "openai-responses".into(),
            ..Default::default()
        };
        model
            .compat
            .insert("responsesReasoningContext".into(), json!("all_turns"));
        model
            .compat
            .insert("responsesReasoningMode".into(), json!("pro"));
        model.compat.insert(
            "responsesPromptCacheOptions".into(),
            json!({"mode": "explicit", "ttl": "30m"}),
        );
        model
            .compat
            .insert("supportsReasoningSummary".into(), json!(false));

        let target = ResolvedModelTarget::from_model(&model, "key".into(), None, None).unwrap();
        let config = expect_responses(target.protocol);
        assert_eq!(config.reasoning_context.as_deref(), Some("all_turns"));
        assert_eq!(config.reasoning_mode.as_deref(), Some("pro"));
        assert_eq!(
            config.prompt_cache_options,
            Some(json!({"mode": "explicit", "ttl": "30m"}))
        );
        assert!(!config.supports_reasoning_summary);
    }

    #[test]
    fn responses_config_rejects_every_malformed_compat_value() {
        let mut model = crate::models::Model {
            id: "gpt-5.6-sol".into(),
            provider: "openai".into(),
            api: "openai-responses".into(),
            ..Default::default()
        };
        // Key, rejected value, and the fragment of the message that has to name
        // both the offending compat key and its expected JSON type.
        let cases: Vec<(&str, Value, &str)> = vec![
            (
                "responsesReasoningContext",
                Value::Bool(true),
                "compat.responsesReasoningContext must be a string",
            ),
            (
                "responsesReasoningMode",
                json!(7),
                "compat.responsesReasoningMode must be a string",
            ),
            (
                "supportsReasoningSummary",
                json!("yes"),
                "compat.supportsReasoningSummary must be a boolean",
            ),
            (
                "responsesReasoningContext",
                json!("sometimes"),
                "unsupported compat.responsesReasoningContext `sometimes`",
            ),
            (
                "responsesReasoningMode",
                json!("turbo"),
                "unsupported compat.responsesReasoningMode `turbo`",
            ),
            (
                "responsesPromptCacheOptions",
                json!("explicit"),
                "compat.responsesPromptCacheOptions must be an object",
            ),
        ];
        for (key, value, needle) in cases {
            model.compat.insert(key.into(), value);
            let error = ResolvedModelTarget::from_model(&model, "key".into(), None, None)
                .expect_err("a malformed compat value must fail the model, not be ignored");
            let message = error.to_string();
            assert!(message.contains(needle), "{key}: {message}");
            // The operator has to be able to find the offending model from the
            // message alone — several providers ship the same compat key.
            assert!(
                message.contains("openai") && message.contains("gpt-5.6-sol"),
                "{key} error does not name the model: {message}"
            );
            model.compat.remove(key);
        }
        // ... and with every bad value removed the same model resolves again.
        let target = ResolvedModelTarget::from_model(&model, "key".into(), None, None).unwrap();
        assert_eq!(expect_responses(target.protocol).reasoning_context, None);
    }

    #[test]
    fn finish_reason_wire_names_are_stable_and_distinct() {
        let named = [
            (FinishReason::Stop, "stop"),
            (FinishReason::ToolCalls, "tool_calls"),
            (FinishReason::Length, "length"),
            (FinishReason::ContentFilter, "content_filter"),
            (FinishReason::Refusal, "refusal"),
            (FinishReason::Cancelled, "cancelled"),
            (FinishReason::Paused, "pause_turn"),
            (FinishReason::Incomplete, "truncated"),
            (FinishReason::Error, "error"),
        ];
        let mut seen = std::collections::HashSet::new();
        for (reason, wire) in named {
            assert_eq!(reason.as_str(), wire, "{reason:?} changed its wire name");
            assert!(seen.insert(wire), "{wire} is claimed by two finish reasons");
        }
        // A provider-specific reason survives verbatim rather than collapsing
        // into one of the known names.
        assert_eq!(
            FinishReason::Unknown("max_tokens".into()).as_str(),
            "max_tokens"
        );
    }

    #[test]
    fn anthropic_thinking_mode_rejects_non_string_value() {
        let mut model = crate::models::Model {
            id: "m".into(),
            provider: "p".into(),
            api: "anthropic".into(),
            ..Default::default()
        };
        model
            .compat
            .insert("anthropicThinkingMode".into(), Value::Bool(true));
        let error = ResolvedModelTarget::from_model(&model, "key".into(), None, None).unwrap_err();
        assert!(error.to_string().contains("must be a string"));
    }

    #[test]
    fn anthropic_thinking_mode_adaptive_and_unknown_values() {
        let mut model = crate::models::Model {
            id: "m".into(),
            provider: "p".into(),
            api: "anthropic".into(),
            ..Default::default()
        };
        model.compat.insert(
            "anthropicThinkingMode".into(),
            Value::String("adaptive".into()),
        );
        let target = ResolvedModelTarget::from_model(&model, "key".into(), None, None).unwrap();
        assert_eq!(
            expect_anthropic(target.protocol).thinking_mode,
            AnthropicThinkingMode::Adaptive
        );

        model.compat.insert(
            "anthropicThinkingMode".into(),
            Value::String("bogus".into()),
        );
        let error = ResolvedModelTarget::from_model(&model, "key".into(), None, None).unwrap_err();
        assert!(error
            .to_string()
            .contains("unsupported compat.anthropicThinkingMode"));
    }

    #[test]
    fn chat_config_rejects_non_string_thinking_format() {
        let mut model = crate::models::Model {
            id: "m".into(),
            provider: "p".into(),
            api: "openai-completions".into(),
            ..Default::default()
        };
        model
            .compat
            .insert("thinkingFormat".into(), Value::Bool(true));
        let error = ResolvedModelTarget::from_model(&model, "key".into(), None, None).unwrap_err();
        assert!(error
            .to_string()
            .contains("thinkingFormat must be a string"));
    }

    #[test]
    fn chat_config_rejects_non_boolean_flag() {
        let mut model = crate::models::Model {
            id: "m".into(),
            provider: "p".into(),
            api: "openai-completions".into(),
            ..Default::default()
        };
        model.compat.insert(
            "requiresReasoningContentOnAssistantMessages".into(),
            Value::String("yes".into()),
        );
        let error = ResolvedModelTarget::from_model(&model, "key".into(), None, None).unwrap_err();
        assert!(error
            .to_string()
            .contains("requiresReasoningContentOnAssistantMessages must be a boolean"));
    }

    #[test]
    fn chat_config_qwen_chat_template_and_other_formats() {
        let chat = |compat: &str| -> OpenAiChatConfig {
            let mut model = crate::models::Model {
                id: "m".into(),
                provider: "p".into(),
                api: "openai-completions".into(),
                ..Default::default()
            };
            model
                .compat
                .insert("thinkingFormat".into(), Value::String(compat.into()));
            expect_chat(
                ResolvedModelTarget::from_model(&model, "key".into(), None, None)
                    .unwrap()
                    .protocol,
            )
        };
        assert_eq!(
            chat("qwen-chat-template").reasoning,
            ChatReasoningFormat::Qwen {
                chat_template: true
            }
        );
        assert_eq!(chat("zai").reasoning, ChatReasoningFormat::Zai);
        assert_eq!(
            chat("reasoning-split").reasoning,
            ChatReasoningFormat::ReasoningSplit
        );

        let mut model = crate::models::Model {
            id: "m".into(),
            provider: "p".into(),
            api: "openai-completions".into(),
            ..Default::default()
        };
        model
            .compat
            .insert("thinkingFormat".into(), Value::String("bogus".into()));
        let error = ResolvedModelTarget::from_model(&model, "key".into(), None, None).unwrap_err();
        assert!(error
            .to_string()
            .contains("unsupported compat.thinkingFormat"));
    }

    #[test]
    fn chat_config_rejects_unknown_max_tokens_field() {
        let mut model = crate::models::Model {
            id: "m".into(),
            provider: "p".into(),
            api: "openai-completions".into(),
            ..Default::default()
        };
        model
            .compat
            .insert("maxTokensField".into(), Value::String("bogus".into()));
        let error = ResolvedModelTarget::from_model(&model, "key".into(), None, None).unwrap_err();
        assert!(error
            .to_string()
            .contains("unsupported compat.maxTokensField"));
    }

    #[test]
    #[should_panic(expected = "chat protocol expected")]
    fn expect_chat_rejects_other_protocols() {
        expect_chat(ProtocolConfig::AnthropicMessages(
            AnthropicMessagesConfig::default(),
        ));
    }

    #[test]
    #[should_panic(expected = "Anthropic protocol expected")]
    fn expect_anthropic_rejects_other_protocols() {
        expect_anthropic(ProtocolConfig::OpenAiChat(OpenAiChatConfig::default()));
    }

    #[test]
    #[should_panic(expected = "Responses protocol expected")]
    fn expect_responses_rejects_other_protocols() {
        expect_responses(ProtocolConfig::OpenAiChat(OpenAiChatConfig::default()));
    }

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
        let chat = expect_chat(target.protocol);
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
        let chat = expect_chat(target.protocol);
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
        let chat = expect_chat(target.protocol);
        assert_eq!(chat.reasoning, ChatReasoningFormat::None);
    }

    /// `thinkingFormat: "deepseek"` is the wire contract that the assistant
    /// turn's `reasoning_content` must be echoed back: in thinking mode a
    /// follow-up without it is refused with HTTP 400 ("The `reasoning_content`
    /// in the thinking mode must be passed back to the API"). The Future
    /// registry entries for `deepseek-flash` / `deepseek-v4-pro` carry only
    /// `thinkingFormat` — no `requiresReasoningContentOnAssistantMessages` —
    /// so the format itself has to imply the replay.
    #[test]
    fn deepseek_thinking_format_implies_assistant_reasoning_replay() {
        let config = |compat: &[(&str, Value)]| -> OpenAiChatConfig {
            let mut model = crate::models::Model {
                id: "deepseek-flash".into(),
                provider: "future".into(),
                api: "openai-completions".into(),
                base_url: "https://future-os.cn/api/v1".into(),
                reasoning: true,
                ..Default::default()
            };
            for (key, value) in compat {
                model.compat.insert((*key).into(), value.clone());
            }
            expect_chat(
                ResolvedModelTarget::from_model(&model, "key".into(), None, None)
                    .unwrap()
                    .protocol,
            )
        };

        let deepseek = config(&[("thinkingFormat", json!("deepseek"))]);
        assert_eq!(deepseek.reasoning, ChatReasoningFormat::DeepSeek);
        assert!(
            deepseek.replay_assistant_reasoning,
            "a deepseek-format model must replay the assistant's reasoning_content"
        );

        // Only the deepseek wire format implies it.
        assert!(!config(&[("thinkingFormat", json!("zai"))]).replay_assistant_reasoning);
        assert!(!config(&[("thinkingFormat", json!("qwen"))]).replay_assistant_reasoning);

        // An explicit compat value still wins over what the format implies.
        assert!(
            !config(&[
                ("thinkingFormat", json!("deepseek")),
                ("requiresReasoningContentOnAssistantMessages", json!(false)),
            ])
            .replay_assistant_reasoning
        );
        assert!(
            config(&[
                ("thinkingFormat", json!("qwen")),
                ("requiresReasoningContentOnAssistantMessages", json!(true)),
            ])
            .replay_assistant_reasoning
        );
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
        let config = expect_anthropic(target.protocol);
        assert_eq!(config.thinking_mode, AnthropicThinkingMode::Adaptive);

        model.compat.insert(
            "anthropicThinkingMode".into(),
            Value::String("manual".into()),
        );
        let target = ResolvedModelTarget::from_model(&model, "key".into(), None, None).unwrap();
        let config = expect_anthropic(target.protocol);
        assert_eq!(config.thinking_mode, AnthropicThinkingMode::Manual);
    }
}
