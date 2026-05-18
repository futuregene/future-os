//! Core type definitions — 1:1 compatible with Go pkg/types/types.go

use serde::ser::{SerializeStruct, Serializer};
use serde::{de, de::MapAccess, de::SeqAccess, Deserialize, Deserializer, Serialize};
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use anyhow;

// ─── ContentBlock (polymorphic) ───────────────────────────────────────────────

/// ContentBlock is a polymorphic content type matching Go's ContentBlock interface.
/// Serializes exactly as Go does:
/// - TextBlock:    `{"type":"text","text":"..."}`
/// - ImageBlock:   `{"type":"image_url","image_url":{"url":"data:...;base64,..."}}`
/// - ToolResultBlock: `{"type":"tool_result","tool_call_id":"...","content":"..."}`
#[derive(Debug, Clone)]
pub enum ContentBlock {
    Text {
        text: String,
    },
    Image {
        image_url: ImageUrlData,
    },
    ToolResult {
        tool_call_id: String,
        content: String,
        is_error: bool,
    },
}

#[derive(Debug, Clone, Default)]
pub struct ImageUrlData {
    pub url: Option<String>,
}

impl<'de> Deserialize<'de> for ImageUrlData {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Raw {
            Object { url: Option<String> },
            String(String),
        }
        let raw = Raw::deserialize(deserializer)?;
        match raw {
            Raw::Object { url } => Ok(ImageUrlData { url }),
            Raw::String(s) => Ok(ImageUrlData { url: Some(s) }),
        }
    }
}

impl Serialize for ImageUrlData {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        if let Some(ref url) = self.url {
            let mut s = serializer.serialize_struct("ImageUrlData", 1)?;
            s.serialize_field("url", url)?;
            s.end()
        } else {
            serializer.serialize_struct("ImageUrlData", 0)?.end()
        }
    }
}

impl ContentBlock {
    pub fn text(text: impl Into<String>) -> Self {
        ContentBlock::Text { text: text.into() }
    }
    pub fn image(url: impl Into<String>) -> Self {
        ContentBlock::Image {
            image_url: ImageUrlData {
                url: Some(url.into()),
            },
        }
    }
    pub fn tool_result(
        tool_call_id: impl Into<String>,
        content: impl Into<String>,
        is_error: bool,
    ) -> Self {
        ContentBlock::ToolResult {
            tool_call_id: tool_call_id.into(),
            content: content.into(),
            is_error,
        }
    }
}

impl<'de> Deserialize<'de> for ContentBlock {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "type",
            "text",
            "image_url",
            "tool_call_id",
            "content",
            "is_error",
        ];
        deserializer.deserialize_struct("ContentBlock", FIELDS, ContentBlockVisitor)
    }
}

struct ContentBlockVisitor;

impl<'de> de::Visitor<'de> for ContentBlockVisitor {
    type Value = ContentBlock;
    fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "a ContentBlock object")
    }
    fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
    where
        A: MapAccess<'de>,
    {
        let mut typ: Option<String> = None;
        let mut text: Option<String> = None;
        let mut image_url: Option<ImageUrlData> = None;
        let mut tool_call_id: Option<String> = None;
        let mut content: Option<String> = None;
        let mut is_error: Option<bool> = None;

        while let Some(k) = map.next_key::<String>()? {
            match k.as_str() {
                "type" => {
                    typ = Some(map.next_value()?);
                }
                "text" => {
                    text = Some(map.next_value()?);
                }
                "image_url" => {
                    image_url = Some(map.next_value()?);
                }
                "tool_call_id" => {
                    tool_call_id = Some(map.next_value()?);
                }
                "content" => {
                    content = Some(map.next_value()?);
                }
                "is_error" => {
                    is_error = Some(map.next_value()?);
                }
                _ => {
                    let _: serde_json::Value = map.next_value()?;
                }
            }
        }

        match typ.unwrap_or_default().as_str() {
            "text" => {
                let t = text.unwrap_or_default();
                Ok(ContentBlock::Text { text: t })
            }
            "image_url" => Ok(ContentBlock::Image {
                image_url: image_url.unwrap_or_default(),
            }),
            "tool_result" => Ok(ContentBlock::ToolResult {
                tool_call_id: tool_call_id.unwrap_or_default(),
                content: content.unwrap_or_default(),
                is_error: is_error.unwrap_or(false),
            }),
            _ => {
                // Fallback: treat as text
                let t = text.unwrap_or_default();
                Ok(ContentBlock::Text { text: t })
            }
        }
    }

    fn visit_seq<A>(self, _seq: A) -> Result<Self::Value, A::Error>
    where
        A: SeqAccess<'de>,
    {
        Err(de::Error::invalid_type(de::Unexpected::Seq, &self))
    }
}

impl Serialize for ContentBlock {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self {
            ContentBlock::Text { text } => {
                let mut s = serializer.serialize_struct("ContentBlock", 2)?;
                s.serialize_field("type", "text")?;
                s.serialize_field("text", text)?;
                s.end()
            }
            ContentBlock::Image { image_url } => {
                let mut s = serializer.serialize_struct("ContentBlock", 2)?;
                s.serialize_field("type", "image_url")?;
                s.serialize_field("image_url", image_url)?;
                s.end()
            }
            ContentBlock::ToolResult {
                tool_call_id,
                content,
                is_error,
            } => {
                let mut s = serializer.serialize_struct("ContentBlock", 4)?;
                s.serialize_field("type", "tool_result")?;
                s.serialize_field("tool_call_id", tool_call_id)?;
                s.serialize_field("content", content)?;
                if *is_error {
                    s.serialize_field("is_error", is_error)?;
                }
                s.end()
            }
        }
    }
}

// ─── AgentMessage ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct AgentMessage {
    #[serde(rename = "role")]
    pub role: String,
    #[serde(default)]
    pub content: Vec<ContentBlock>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub thinking: String,
    #[serde(rename = "tool_calls", default, skip_serializing_if = "Vec::is_empty")]
    pub tool_calls: Vec<AgentToolCall>,
    #[serde(
        rename = "tool_call_id",
        default,
        skip_serializing_if = "String::is_empty"
    )]
    pub tool_call_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Map<String, serde_json::Value>>,
}

impl AgentMessage {
    pub fn text(&self) -> String {
        self.content
            .iter()
            .filter_map(|b| match b {
                ContentBlock::Text { text } => Some(text.clone()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("")
    }
    pub fn add_text(&mut self, text: impl Into<String>) {
        self.content.push(ContentBlock::text(text));
    }
    pub fn add_image(&mut self, mime_type: String, data: String) {
        let url = format!("data:{};base64,{}", mime_type, data);
        self.content.push(ContentBlock::image(url));
    }
    pub fn new_user(role: &str, content: serde_json::Value) -> Self {
        Self {
            role: role.to_string(),
            content: match content {
                serde_json::Value::Array(arr) => arr
                    .into_iter()
                    .filter_map(|v| match v {
                        serde_json::Value::Object(mut obj) => {
                            let typ = obj
                                .remove("type")
                                .map(|t| t.as_str().unwrap_or("text").to_string())
                                .unwrap_or_else(|| "text".to_string());
                            match typ.as_str() {
                                "text" => {
                                    let text = obj
                                        .remove("text")
                                        .map(|t| t.as_str().unwrap_or("").to_string())
                                        .unwrap_or_default();
                                    Some(ContentBlock::Text { text })
                                }
                                "image_url" => {
                                    let url_val = obj.remove("image_url");
                                    let url = if let Some(url_obj) = url_val {
                                        if let Some(url_str) = url_obj.get("url") {
                                            url_str.as_str().unwrap_or("").to_string()
                                        } else {
                                            String::new()
                                        }
                                    } else {
                                        String::new()
                                    };
                                    Some(ContentBlock::Image {
                                        image_url: crate::types::ImageUrlData { url: Some(url) },
                                    })
                                }
                                _ => Some(ContentBlock::Text {
                                    text: serde_json::to_string(&obj).unwrap_or_default(),
                                }),
                            }
                        }
                        _ => None,
                    })
                    .collect(),
                serde_json::Value::String(s) => vec![ContentBlock::text(s)],
                _ => vec![],
            },
            thinking: String::new(),
            tool_calls: vec![],
            tool_call_id: String::new(),
            metadata: None,
        }
    }
}

// ─── AgentToolCall ─────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentToolCall {
    pub id: String,
    pub name: String,
    pub args: serde_json::Value,
}

// ─── Message (LLM wire format) ─────────────────────────────────────────────

/// Message is the LLM API wire format, matching Go's types.Message exactly.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Message {
    #[serde(rename = "role")]
    pub role: String,
    /// content is None when absent (Go: null), Some(vec) when array.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<serde_json::Value>,
    #[serde(
        rename = "tool_calls",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub tool_calls: Option<Vec<ToolCall>>,
    #[serde(
        rename = "tool_call_id",
        default,
        skip_serializing_if = "String::is_empty"
    )]
    pub tool_call_id: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub name: String,
    #[serde(
        rename = "reasoning_content",
        default,
        skip_serializing_if = "String::is_empty"
    )]
    pub reasoning_content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    #[serde(rename = "type")]
    pub call_type: String,
    pub function: ToolCallFn,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCallFn {
    pub name: String,
    pub arguments: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TextContent {
    #[serde(rename = "type")]
    pub content_type: String,
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageContent {
    #[serde(rename = "type")]
    pub content_type: String,
    #[serde(rename = "mime_type", default, skip_serializing_if = "Option::is_none")]
    pub mime_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<ImageSource>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageSource {
    #[serde(rename = "type")]
    pub source_type: String,
    #[serde(rename = "media_type")]
    pub media_type: String,
    pub data: String,
}

// ─── Usage ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Usage {
    #[serde(rename = "prompt_tokens")]
    pub prompt_tokens: i64,
    #[serde(rename = "completion_tokens")]
    pub completion_tokens: i64,
    #[serde(rename = "total_tokens")]
    pub total_tokens: i64,
    #[serde(
        rename = "cache_read_tokens",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub cache_read_tokens: Option<i64>,
    #[serde(
        rename = "cache_write_tokens",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub cache_write_tokens: Option<i64>,
}

// ─── StreamEvent ────────────────────────────────────────────────────────────

/// StreamEvent matches Go's types.StreamEvent exactly.
/// JSON field names are camelCase as specified in Go struct tags.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StreamEvent {
    #[serde(rename = "type")]
    pub event_type: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub text: String,
    #[serde(rename = "toolCall", default, skip_serializing_if = "Option::is_none")]
    pub tool_call: Option<ToolCall>,
    #[serde(rename = "toolName", default, skip_serializing_if = "String::is_empty")]
    pub tool_name: String,
    #[serde(rename = "toolID", default, skip_serializing_if = "String::is_empty")]
    pub tool_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<Usage>,
    #[serde(
        rename = "stopReason",
        default,
        skip_serializing_if = "String::is_empty"
    )]
    pub stop_reason: String,
    #[serde(
        rename = "errorText",
        default,
        skip_serializing_if = "String::is_empty"
    )]
    pub error_text: String,
}

// ─── ToolDef / AgentTool ──────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDef {
    #[serde(rename = "type")]
    pub tool_type: String,
    pub function: FunctionDef,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FunctionDef {
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value,
}

/// Type alias for async tool handler functions.
pub type ToolHandler =
    fn(serde_json::Value) -> Pin<Box<dyn Future<Output = Result<String, anyhow::Error>> + Send>>;

/// AgentTool wraps a tool definition with a handler function.
/// Handler is not serialized (matches Go's function pointer field).
#[derive(Clone)]
pub struct AgentTool {
    pub def: ToolDef,
    pub handler: ToolHandler,
    pub guidelines: Vec<String>,
}

impl std::fmt::Debug for AgentTool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AgentTool")
            .field("def", &self.def)
            .field("handler", &"<fn>")
            .field("guidelines", &self.guidelines)
            .finish()
    }
}

// ─── ToolCallResult ────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct ToolCallResult {
    pub result: String,
    pub is_error: bool,
}

// ─── AgentConfig ───────────────────────────────────────────────────────────

#[derive(Clone)]
#[allow(clippy::type_complexity)]
pub struct AgentConfig {
    pub system_prompt: String,
    pub max_turns: i32,
    pub thinking_budget: i32,
    pub max_retries: i32,
    pub transform_context: Option<Arc<dyn Fn(Vec<Message>, String) -> Vec<Message> + Send + Sync>>,
    pub stop_condition: Option<Arc<dyn Fn(Vec<Message>, &str) -> bool + Send + Sync>>,
    pub before_tool_call:
        Option<Arc<dyn Fn(&str, &str, &serde_json::Value) -> Option<ToolCallResult> + Send + Sync>>,
    pub prepare_tool_call:
        Option<Arc<dyn Fn(&str, &serde_json::Value) -> serde_json::Value + Send + Sync>>,
    pub finalize_tool_call: Option<
        Arc<dyn Fn(&str, String, anyhow::Error) -> (String, Option<anyhow::Error>) + Send + Sync>,
    >,
    pub after_tool_call: Option<
        Arc<
            dyn Fn(&str, &str, &serde_json::Value, String, anyhow::Error) -> Option<ToolCallResult>
                + Send
                + Sync,
        >,
    >,
    pub tools_execution_mode: String,
}

// ─── Model ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Model {
    pub id: String,
    pub name: String,
    pub provider: String,
    pub api: String,
    #[serde(rename = "baseUrl")]
    pub base_url: String,
    #[serde(skip)]
    pub api_key: String,
    #[serde(rename = "contextWindow")]
    pub context_window: i64,
    #[serde(rename = "maxTokens")]
    pub max_tokens: i64,
    pub reasoning: bool,
    #[serde(rename = "input", default, skip_serializing_if = "Vec::is_empty")]
    pub input_types: Vec<String>,
    #[serde(default)]
    pub cost: ModelCost,
    #[serde(
        rename = "thinkingLevelMap",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub thinking_level_map: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub headers: Option<serde_json::Map<String, serde_json::Value>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compat: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ModelCost {
    #[serde(rename = "input", default)]
    pub input: f64,
    #[serde(rename = "output", default)]
    pub output: f64,
    #[serde(rename = "cacheRead", default)]
    pub cache_read: f64,
    #[serde(rename = "cacheWrite", default)]
    pub cache_write: f64,
}

// ─── LLMProvider trait ─────────────────────────────────────────────────────

#[async_trait::async_trait]
pub trait LLMProvider: Send + Sync {
    async fn stream_chat(
        &self,
        model: String,
        messages: Vec<Message>,
        tools: Vec<ToolDef>,
        system_prompt: String,
    ) -> anyhow::Result<tokio_stream::wrappers::ReceiverStream<StreamEvent>>;

    /// Update compat/thinking settings at runtime (e.g. after set_model).
    fn update_compat(
        &self,
        _thinking_format: &str,
        _supports_reasoning_effort: bool,
        _requires_reasoning_on_assistant: bool,
        _thinking_level_map: HashMap<String, String>,
    ) {
    }

    /// Update the endpoint (base_url + api_key) at runtime for model switching.
    fn update_endpoint(&self, _base_url: &str, _api_key: &str) {}

    /// Update thinking level and budget at runtime (after set_thinking_level / cycle_thinking_level).
    fn update_thinking(&self, _level: &str, _budget: i32) {}

    /// Update max_tokens field name (from compat.maxTokensField: "max_tokens" or "max_completion_tokens").
    fn update_max_tokens_field(&self, _field: &str) {}
}

// ─── Message ↔ AgentMessage conversion ────────────────────────────────────

impl AgentMessage {
    pub fn to_llm(&self) -> Message {
        let content = if self.content.is_empty() {
            None
        } else {
            let blocks: Vec<serde_json::Value> = self
                .content
                .iter()
                .map(|b| serde_json::to_value(b).unwrap_or(serde_json::Value::Null))
                .collect();
            Some(serde_json::Value::Array(blocks))
        };

        let tool_calls = if self.tool_calls.is_empty() {
            None
        } else {
            Some(
                self.tool_calls
                    .iter()
                    .map(|tc| ToolCall {
                        id: tc.id.clone(),
                        call_type: "function".to_string(),
                        function: ToolCallFn {
                            name: tc.name.clone(),
                            arguments: match &tc.args {
                                serde_json::Value::String(s) => {
                                    serde_json::Value::String(s.clone())
                                }
                                other => serde_json::Value::String(
                                    serde_json::to_string(other).unwrap_or_default(),
                                ),
                            },
                        },
                    })
                    .collect(),
            )
        };

        Message {
            role: self.role.clone(),
            content,
            tool_calls,
            tool_call_id: self.tool_call_id.clone(),
            name: String::new(),
            reasoning_content: self.thinking.clone(),
        }
    }
}

pub fn convert_to_llm(msgs: &[AgentMessage]) -> Vec<Message> {
    msgs.iter().map(|m| m.to_llm()).collect()
}

pub fn convert_from_llm(msgs: Vec<Message>) -> Vec<AgentMessage> {
    msgs.into_iter()
        .map(|m| {
            let content = if let Some(c) = m.content {
                match c {
                    serde_json::Value::Array(arr) => arr
                        .into_iter()
                        .filter_map(|v| {
                            let obj = match v {
                                serde_json::Value::Object(o) => o,
                                _ => return None,
                            };
                            let typ = obj.get("type")?.as_str()?.to_string();
                            match typ.as_str() {
                                "text" => {
                                    let text = obj.get("text")?.as_str()?.to_string();
                                    Some(ContentBlock::Text { text })
                                }
                                "image_url" => {
                                    let url_data = obj
                                        .get("image_url")
                                        .map(|v| match v {
                                            serde_json::Value::Object(o) => ImageUrlData {
                                                url: o
                                                    .get("url")
                                                    .and_then(|v| v.as_str().map(String::from)),
                                            },
                                            serde_json::Value::String(s) => ImageUrlData {
                                                url: Some(s.clone()),
                                            },
                                            _ => ImageUrlData { url: None },
                                        })
                                        .unwrap_or_default();
                                    Some(ContentBlock::Image {
                                        image_url: url_data,
                                    })
                                }
                                _ => None,
                            }
                        })
                        .collect(),
                    serde_json::Value::String(s) if !s.is_empty() => {
                        vec![ContentBlock::text(s)]
                    }
                    _ => vec![],
                }
            } else {
                vec![]
            };

            let tool_calls = m
                .tool_calls
                .map(|tcs| {
                    tcs.into_iter()
                        .map(|tc| AgentToolCall {
                            id: tc.id,
                            name: tc.function.name,
                            args: tc.function.arguments,
                        })
                        .collect()
                })
                .unwrap_or_default();

            AgentMessage {
                role: m.role,
                content,
                thinking: m.reasoning_content,
                tool_calls,
                tool_call_id: m.tool_call_id,
                metadata: None,
            }
        })
        .collect()
}

// Aliases for Go-style names (PascalCase conversion functions)
pub use convert_from_llm as ConvertFromLLM;
pub use convert_to_llm as ConvertToLLM;
