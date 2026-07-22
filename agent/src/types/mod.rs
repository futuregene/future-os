//! Core type definitions — 1:1 compatible with Go pkg/types/types.go

use serde::ser::{SerializeStruct, Serializer};
use serde::{de, de::MapAccess, de::SeqAccess, Deserialize, Deserializer, Serialize};
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
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub tool_args: String,
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
            name: String::new(),
            tool_args: String::new(),
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
    #[serde(default, skip_serializing_if = "String::is_empty")]
    #[serde(rename = "tool_args")]
    pub tool_args: String,
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

/// A local file attached to a prompt (GUI). Files are referenced by their
/// original absolute path — never copied — and read on demand by the agent's
/// tools. Images additionally carry base64 for an image_url block.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Attachment {
    #[serde(default)]
    pub path: String,
    /// "image" | "file".
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub name: String,
    /// Optional cached-thumbnail path (images only). Not model-facing; carried
    /// into the user entry's meta so the GUI can render the chip after reload.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thumbnail: Option<String>,
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
    /// Local filesystem path after the image is saved to disk (set by GUI).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file_path: Option<String>,
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
    /// Cost of this request as reported by the upstream API (Future platform
    /// returns `credit_cost` as a decimal string, e.g. "0.00019072").
    /// Parsed as f64 for accumulation; absent / null → None.
    #[serde(
        rename = "credit_cost",
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "deserialize_credit_cost"
    )]
    pub credit_cost: Option<f64>,
}

/// Deserialize `credit_cost` which may be a string ("0.00019") or a number.
fn deserialize_credit_cost<'de, D>(deserializer: D) -> Result<Option<f64>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de;
    struct CreditCostVisitor;
    impl<'de> de::Visitor<'de> for CreditCostVisitor {
        type Value = Option<f64>;
        fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
            f.write_str("a number or string representing credit cost")
        }
        fn visit_some<D2>(self, deserializer: D2) -> Result<Self::Value, D2::Error>
        where
            D2: de::Deserializer<'de>,
        {
            deserializer.deserialize_any(self)
        }
        fn visit_none<E>(self) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            Ok(None)
        }
        fn visit_f64<E>(self, v: f64) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            Ok(Some(v))
        }
        fn visit_i64<E>(self, v: i64) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            Ok(Some(v as f64))
        }
        fn visit_u64<E>(self, v: u64) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            Ok(Some(v as f64))
        }
        fn visit_str<E>(self, v: &str) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            v.parse::<f64>()
                .map(Some)
                .map_err(|_| de::Error::custom("invalid float"))
        }
        fn visit_bool<E>(self, _v: bool) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            Ok(None)
        }
    }
    deserializer.deserialize_option(CreditCostVisitor)
}

// ─── StreamEvent ────────────────────────────────────────────────────────────

/// StreamEvent matches Go's types.StreamEvent exactly.
/// JSON field names are camelCase as specified in Go struct tags.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
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
    /// Tool-call array index from the streaming SSE delta chunk.  Used to route
    /// `toolcall_delta` events to the correct tool-call accumulator when the
    /// model streams multiple tool calls in parallel.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub tc_index: usize,
}

fn is_zero(n: &usize) -> bool {
    *n == 0
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
    /// If true, the model is hidden from model lists but still callable.
    #[serde(default)]
    pub hide: bool,
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

    /// Refresh only the API key at runtime, after an out-of-band credential
    /// change (FutureGene login/logout, custom-provider key edits). This leaves
    /// the base_url untouched — a login/logout changes the key, not the model's
    /// endpoint.
    fn set_api_key(&self, _api_key: &str) {}

    /// Update thinking level and budget at runtime (after set_thinking_level / cycle_thinking_level).
    fn update_thinking(&self, _level: &str, _budget: i32) {}
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
            name: self.name.clone(),
            tool_args: self.tool_args.clone(),
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
                name: m.name.clone(),
                tool_args: m.tool_args.clone(),
                metadata: None,
            }
        })
        .collect()
}

// Aliases for Go-style names (PascalCase conversion functions)
pub use convert_from_llm as ConvertFromLLM;
pub use convert_to_llm as ConvertToLLM;

#[cfg(test)]
mod tests {
    use super::*;

    // ─── ContentBlock construction ──────────────────────────────────────────

    #[test]
    fn content_block_text() {
        let b = ContentBlock::text("hello");
        match b {
            ContentBlock::Text { text } => assert_eq!(text, "hello"),
            _ => panic!("expected Text"),
        }
    }

    #[test]
    fn content_block_image() {
        let b = ContentBlock::image("data:image/png;base64,abc");
        match b {
            ContentBlock::Image { image_url } => {
                assert_eq!(image_url.url.as_deref(), Some("data:image/png;base64,abc"))
            }
            _ => panic!("expected Image"),
        }
    }

    #[test]
    fn content_block_tool_result() {
        let b = ContentBlock::tool_result("call_1", "output text", false);
        match b {
            ContentBlock::ToolResult {
                tool_call_id,
                content,
                is_error,
            } => {
                assert_eq!(tool_call_id, "call_1");
                assert_eq!(content, "output text");
                assert!(!is_error);
            }
            _ => panic!("expected ToolResult"),
        }
    }

    // ─── ContentBlock serde ────────────────────────────────────────────────

    #[test]
    fn serialize_text_block() {
        let b = ContentBlock::text("world");
        let json = serde_json::to_value(&b).unwrap();
        assert_eq!(json["type"], "text");
        assert_eq!(json["text"], "world");
    }

    #[test]
    fn serialize_image_block() {
        let b = ContentBlock::image("https://example.com/img.png");
        let json = serde_json::to_value(&b).unwrap();
        assert_eq!(json["type"], "image_url");
        assert_eq!(json["image_url"]["url"], "https://example.com/img.png");
    }

    #[test]
    fn serialize_tool_result_no_error() {
        let b = ContentBlock::tool_result("c1", "ok", false);
        let json = serde_json::to_value(&b).unwrap();
        assert_eq!(json["type"], "tool_result");
        assert_eq!(json["tool_call_id"], "c1");
        assert_eq!(json["content"], "ok");
        // is_error=false should NOT be serialized (skip if false)
        assert!(json.get("is_error").is_none());
    }

    #[test]
    fn serialize_tool_result_with_error() {
        let b = ContentBlock::tool_result("c1", "fail msg", true);
        let json = serde_json::to_value(&b).unwrap();
        assert_eq!(json["is_error"], true);
    }

    #[test]
    fn deserialize_text_block() {
        let json = r#"{"type":"text","text":"hello"}"#;
        let b: ContentBlock = serde_json::from_str(json).unwrap();
        match b {
            ContentBlock::Text { text } => assert_eq!(text, "hello"),
            _ => panic!("expected Text"),
        }
    }

    #[test]
    fn deserialize_image_block() {
        let json = r#"{"type":"image_url","image_url":{"url":"data:..."}}"#;
        let b: ContentBlock = serde_json::from_str(json).unwrap();
        match b {
            ContentBlock::Image { image_url } => {
                assert_eq!(image_url.url.as_deref(), Some("data:..."))
            }
            _ => panic!("expected Image"),
        }
    }

    #[test]
    fn deserialize_tool_result_block() {
        let json = r#"{"type":"tool_result","tool_call_id":"c1","content":"done","is_error":true}"#;
        let b: ContentBlock = serde_json::from_str(json).unwrap();
        match b {
            ContentBlock::ToolResult {
                tool_call_id,
                content,
                is_error,
            } => {
                assert_eq!(tool_call_id, "c1");
                assert_eq!(content, "done");
                assert!(is_error);
            }
            _ => panic!("expected ToolResult"),
        }
    }

    #[test]
    fn deserialize_unknown_type_falls_back_to_text() {
        let json = r#"{"type":"unknown_type","text":"fallback"}"#;
        let b: ContentBlock = serde_json::from_str(json).unwrap();
        match b {
            ContentBlock::Text { text } => assert_eq!(text, "fallback"),
            _ => panic!("expected Text fallback"),
        }
    }

    #[test]
    fn content_block_roundtrip_text() {
        let original = ContentBlock::text("roundtrip");
        let json = serde_json::to_string(&original).unwrap();
        let restored: ContentBlock = serde_json::from_str(&json).unwrap();
        match restored {
            ContentBlock::Text { text } => assert_eq!(text, "roundtrip"),
            _ => panic!("roundtrip failed"),
        }
    }

    #[test]
    fn content_block_roundtrip_tool_result() {
        let original = ContentBlock::tool_result("c2", "result", true);
        let json = serde_json::to_string(&original).unwrap();
        let restored: ContentBlock = serde_json::from_str(&json).unwrap();
        match restored {
            ContentBlock::ToolResult {
                tool_call_id,
                content,
                is_error,
            } => {
                assert_eq!(tool_call_id, "c2");
                assert_eq!(content, "result");
                assert!(is_error);
            }
            _ => panic!("roundtrip failed"),
        }
    }

    // ─── ImageUrlData ──────────────────────────────────────────────────────

    #[test]
    fn image_url_data_from_object() {
        let json = r#"{"url":"data:image/png;base64,xyz"}"#;
        let d: ImageUrlData = serde_json::from_str(json).unwrap();
        assert_eq!(d.url.as_deref(), Some("data:image/png;base64,xyz"));
    }

    #[test]
    fn image_url_data_from_string() {
        let json = r#""data:image/png;base64,xyz""#;
        let d: ImageUrlData = serde_json::from_str(json).unwrap();
        assert_eq!(d.url.as_deref(), Some("data:image/png;base64,xyz"));
    }

    #[test]
    fn image_url_data_empty() {
        let d = ImageUrlData::default();
        assert!(d.url.is_none());
    }

    // ─── AgentMessage ──────────────────────────────────────────────────────

    #[test]
    fn agent_message_text() {
        let mut msg = AgentMessage::default();
        msg.add_text("hello ");
        msg.add_text("world");
        assert_eq!(msg.text(), "hello world");
    }

    #[test]
    fn agent_message_text_skips_non_text_blocks() {
        let mut msg = AgentMessage::default();
        msg.add_text("before");
        msg.content.push(ContentBlock::image("data:..."));
        msg.add_text("after");
        assert_eq!(msg.text(), "beforeafter");
    }

    #[test]
    fn agent_message_add_image() {
        let mut msg = AgentMessage::default();
        msg.add_image("image/png".to_string(), "aGVsbG8=".to_string());
        assert_eq!(msg.content.len(), 1);
        match &msg.content[0] {
            ContentBlock::Image { image_url } => {
                assert!(image_url
                    .url
                    .as_ref()
                    .unwrap()
                    .starts_with("data:image/png;base64,aGVsbG8="));
            }
            _ => panic!("expected Image"),
        }
    }

    #[test]
    fn agent_message_new_user_string_content() {
        let msg = AgentMessage::new_user("user", serde_json::json!("hello"));
        assert_eq!(msg.role, "user");
        assert_eq!(msg.content.len(), 1);
        assert_eq!(msg.text(), "hello");
    }

    #[test]
    fn agent_message_new_user_array_content() {
        let content = serde_json::json!([
            {"type": "text", "text": "first"},
            {"type": "text", "text": " second"},
        ]);
        let msg = AgentMessage::new_user("user", content);
        assert_eq!(msg.text(), "first second");
    }

    #[test]
    fn agent_message_new_user_with_image() {
        let content = serde_json::json!([
            {"type": "text", "text": "look at this"},
            {"type": "image_url", "image_url": {"url": "data:image/png;base64,abc"}},
        ]);
        let msg = AgentMessage::new_user("user", content);
        assert_eq!(msg.content.len(), 2);
        assert_eq!(msg.text(), "look at this");
    }

    #[test]
    fn agent_message_new_user_empty_content() {
        let msg = AgentMessage::new_user("user", serde_json::json!(null));
        assert!(msg.content.is_empty());
    }

    // ─── AgentMessage serde ────────────────────────────────────────────────

    #[test]
    fn agent_message_serialize_omits_empty_fields() {
        let msg = AgentMessage {
            role: "user".to_string(),
            content: vec![ContentBlock::text("hi")],
            ..Default::default()
        };
        let json = serde_json::to_value(&msg).unwrap();
        assert!(json.get("thinking").is_none());
        assert!(json.get("tool_calls").is_none());
        assert!(json.get("tool_call_id").is_none());
        assert!(json.get("name").is_none());
    }

    #[test]
    fn agent_message_serialize_with_tool_calls() {
        let msg = AgentMessage {
            role: "assistant".to_string(),
            content: vec![],
            tool_calls: vec![AgentToolCall {
                id: "call_1".to_string(),
                name: "shell".to_string(),
                args: serde_json::json!({"command": "ls"}),
            }],
            ..Default::default()
        };
        let json = serde_json::to_value(&msg).unwrap();
        assert_eq!(json["tool_calls"][0]["name"], "shell");
    }

    // ─── Usage deserialization ─────────────────────────────────────────────

    #[test]
    fn usage_from_json() {
        let json = r#"{"prompt_tokens":100,"completion_tokens":50,"total_tokens":150}"#;
        let u: Usage = serde_json::from_str(json).unwrap();
        assert_eq!(u.prompt_tokens, 100);
        assert_eq!(u.completion_tokens, 50);
        assert_eq!(u.total_tokens, 150);
        assert!(u.cache_read_tokens.is_none());
    }

    #[test]
    fn usage_with_cache_tokens() {
        let json = r#"{
            "prompt_tokens":100,"completion_tokens":50,"total_tokens":150,
            "cache_read_tokens":80,"cache_write_tokens":20
        }"#;
        let u: Usage = serde_json::from_str(json).unwrap();
        assert_eq!(u.cache_read_tokens, Some(80));
        assert_eq!(u.cache_write_tokens, Some(20));
    }

    #[test]
    fn usage_credit_cost_as_string() {
        let json =
            r#"{"prompt_tokens":0,"completion_tokens":0,"total_tokens":0,"credit_cost":"0.00019"}"#;
        let u: Usage = serde_json::from_str(json).unwrap();
        assert_eq!(u.credit_cost, Some(0.00019));
    }

    #[test]
    fn usage_credit_cost_as_number() {
        let json =
            r#"{"prompt_tokens":0,"completion_tokens":0,"total_tokens":0,"credit_cost":0.00025}"#;
        let u: Usage = serde_json::from_str(json).unwrap();
        assert_eq!(u.credit_cost, Some(0.00025));
    }

    #[test]
    fn usage_credit_cost_absent() {
        let json = r#"{"prompt_tokens":0,"completion_tokens":0,"total_tokens":0}"#;
        let u: Usage = serde_json::from_str(json).unwrap();
        assert!(u.credit_cost.is_none());
    }

    // ─── StreamEvent ───────────────────────────────────────────────────────

    #[test]
    fn stream_event_serialization() {
        let event = StreamEvent {
            event_type: "text_chunk".to_string(),
            text: "hello".to_string(),
            ..Default::default()
        };
        let json = serde_json::to_value(&event).unwrap();
        assert_eq!(json["type"], "text_chunk");
        assert_eq!(json["text"], "hello");
    }

    #[test]
    fn stream_event_deserialization() {
        let json = r#"{"type":"tool_start","toolName":"shell","toolID":"call_1"}"#;
        let e: StreamEvent = serde_json::from_str(json).unwrap();
        assert_eq!(e.event_type, "tool_start");
        assert_eq!(e.tool_name, "shell");
        assert_eq!(e.tool_id, "call_1");
    }

    #[test]
    fn stream_event_camel_case_fields() {
        let event = StreamEvent {
            event_type: "agent_end".to_string(),
            stop_reason: "max_tokens".to_string(),
            error_text: "some error".to_string(),
            ..Default::default()
        };
        let json = serde_json::to_value(&event).unwrap();
        assert_eq!(json["stopReason"], "max_tokens");
        assert_eq!(json["errorText"], "some error");
    }

    // ─── Message ↔ AgentMessage conversion ─────────────────────────────────

    #[test]
    fn agent_message_to_llm_text_only() {
        let msg = AgentMessage {
            role: "user".to_string(),
            content: vec![ContentBlock::text("hello")],
            ..Default::default()
        };
        let llm = msg.to_llm();
        assert_eq!(llm.role, "user");
        assert!(llm.content.is_some());
        assert!(llm.tool_calls.is_none());
    }

    #[test]
    fn agent_message_to_llm_with_tool_calls() {
        let msg = AgentMessage {
            role: "assistant".to_string(),
            content: vec![],
            tool_calls: vec![AgentToolCall {
                id: "c1".to_string(),
                name: "shell".to_string(),
                args: serde_json::json!({"command": "ls"}),
            }],
            ..Default::default()
        };
        let llm = msg.to_llm();
        let tcs = llm.tool_calls.unwrap();
        assert_eq!(tcs.len(), 1);
        assert_eq!(tcs[0].id, "c1");
        assert_eq!(tcs[0].function.name, "shell");
    }

    #[test]
    fn agent_message_to_llm_empty_content() {
        let msg = AgentMessage {
            role: "assistant".to_string(),
            content: vec![],
            ..Default::default()
        };
        let llm = msg.to_llm();
        assert!(llm.content.is_none());
    }

    #[test]
    fn convert_to_llm_and_back() {
        let original = vec![AgentMessage {
            role: "user".to_string(),
            content: vec![ContentBlock::text("test")],
            ..Default::default()
        }];
        let llm_msgs = convert_to_llm(&original);
        assert_eq!(llm_msgs.len(), 1);
        let back = convert_from_llm(llm_msgs);
        assert_eq!(back.len(), 1);
        assert_eq!(back[0].role, "user");
        assert_eq!(back[0].text(), "test");
    }

    #[test]
    fn convert_from_llm_with_tool_calls() {
        let msgs = vec![Message {
            role: "assistant".to_string(),
            content: None,
            tool_calls: Some(vec![ToolCall {
                id: "c1".to_string(),
                call_type: "function".to_string(),
                function: ToolCallFn {
                    name: "read".to_string(),
                    arguments: serde_json::json!({"path": "/tmp"}),
                },
            }]),
            ..Default::default()
        }];
        let agent_msgs = convert_from_llm(msgs);
        assert_eq!(agent_msgs[0].tool_calls.len(), 1);
        assert_eq!(agent_msgs[0].tool_calls[0].name, "read");
    }

    #[test]
    fn convert_from_llm_preserves_reasoning_content() {
        let msgs = vec![Message {
            role: "assistant".to_string(),
            content: None,
            reasoning_content: "thinking...".to_string(),
            ..Default::default()
        }];
        let agent_msgs = convert_from_llm(msgs);
        assert_eq!(agent_msgs[0].thinking, "thinking...");
    }

    #[test]
    fn convert_from_llm_string_content() {
        let msgs = vec![Message {
            role: "assistant".to_string(),
            content: Some(serde_json::json!("plain text")),
            ..Default::default()
        }];
        let agent_msgs = convert_from_llm(msgs);
        assert_eq!(agent_msgs[0].text(), "plain text");
    }

    // ─── Attachment ────────────────────────────────────────────────────────

    #[test]
    fn attachment_serialization() {
        let att = Attachment {
            path: "/tmp/file.pdf".to_string(),
            kind: "file".to_string(),
            name: "file.pdf".to_string(),
            thumbnail: None,
        };
        let json = serde_json::to_value(&att).unwrap();
        assert_eq!(json["path"], "/tmp/file.pdf");
        assert_eq!(json["kind"], "file");
        assert!(json.get("thumbnail").is_none());
    }

    #[test]
    fn attachment_with_thumbnail() {
        let att = Attachment {
            path: "/tmp/img.png".to_string(),
            kind: "image".to_string(),
            name: "img.png".to_string(),
            thumbnail: Some("/tmp/thumb.png".to_string()),
        };
        let json = serde_json::to_value(&att).unwrap();
        assert_eq!(json["thumbnail"], "/tmp/thumb.png");
    }

    // ─── Model / ModelCost ─────────────────────────────────────────────────

    #[test]
    fn model_deserialization() {
        let json = r#"{
            "id": "gpt-4o",
            "name": "GPT-4o",
            "provider": "openai",
            "api": "openai",
            "baseUrl": "https://api.openai.com",
            "contextWindow": 128000,
            "maxTokens": 4096,
            "reasoning": false
        }"#;
        let m: Model = serde_json::from_str(json).unwrap();
        assert_eq!(m.id, "gpt-4o");
        assert_eq!(m.context_window, 128000);
        assert!(!m.reasoning);
    }

    #[test]
    fn model_cost_defaults() {
        let c = ModelCost::default();
        assert_eq!(c.input, 0.0);
        assert_eq!(c.output, 0.0);
    }

    // ─── ToolDef / FunctionDef ─────────────────────────────────────────────

    #[test]
    fn tool_def_serialization() {
        let tool = ToolDef {
            tool_type: "function".to_string(),
            function: FunctionDef {
                name: "shell".to_string(),
                description: "Run a command".to_string(),
                parameters: serde_json::json!({"type": "object"}),
            },
        };
        let json = serde_json::to_value(&tool).unwrap();
        assert_eq!(json["type"], "function");
        assert_eq!(json["function"]["name"], "shell");
    }
}
