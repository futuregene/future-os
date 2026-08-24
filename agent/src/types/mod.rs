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
    Reasoning {
        text: String,
        provider_metadata: ProviderMetadata,
    },
    ToolCall {
        id: String,
        name: String,
        args: serde_json::Value,
        provider_metadata: ProviderMetadata,
    },
    ToolResult {
        tool_call_id: String,
        content: String,
        is_error: bool,
    },
}

/// Opaque, namespaced protocol state that must survive a model round trip.
/// Known adapters validate their own namespace (`openai`, `anthropic`); unknown
/// namespaces are retained so future adapters can preserve state losslessly.
pub type ProviderMetadata = serde_json::Map<String, serde_json::Value>;

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
    pub fn reasoning(text: impl Into<String>, provider_metadata: ProviderMetadata) -> Self {
        ContentBlock::Reasoning {
            text: text.into(),
            provider_metadata,
        }
    }
    pub fn tool_call(
        id: impl Into<String>,
        name: impl Into<String>,
        args: serde_json::Value,
        provider_metadata: ProviderMetadata,
    ) -> Self {
        ContentBlock::ToolCall {
            id: id.into(),
            name: name.into(),
            args,
            provider_metadata,
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

    /// Test-only variant accessors: a refutable `let ... else { panic!() }`
    /// in a test leaves the dead else arm as an uncovered line; Option +
    /// unwrap keeps the panic in std where it belongs.
    #[cfg(test)]
    fn as_text(&self) -> Option<&str> {
        match self {
            ContentBlock::Text { text } => Some(text),
            _ => None,
        }
    }

    #[cfg(test)]
    fn as_image(&self) -> Option<&ImageUrlData> {
        match self {
            ContentBlock::Image { image_url } => Some(image_url),
            _ => None,
        }
    }

    #[cfg(test)]
    fn as_tool_result(&self) -> Option<(&str, &str, bool)> {
        match self {
            ContentBlock::ToolResult {
                tool_call_id,
                content,
                is_error,
            } => Some((tool_call_id, content, *is_error)),
            _ => None,
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
            "id",
            "name",
            "args",
            "provider_metadata",
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
        let mut id: Option<String> = None;
        let mut name: Option<String> = None;
        let mut args: Option<serde_json::Value> = None;
        let mut provider_metadata: Option<ProviderMetadata> = None;

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
                "id" => id = Some(map.next_value()?),
                "name" => name = Some(map.next_value()?),
                "args" => args = Some(map.next_value()?),
                "provider_metadata" => provider_metadata = Some(map.next_value()?),
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
            "reasoning" => Ok(ContentBlock::Reasoning {
                text: text.unwrap_or_default(),
                provider_metadata: provider_metadata.unwrap_or_default(),
            }),
            "tool_call" => Ok(ContentBlock::ToolCall {
                id: id.unwrap_or_default(),
                name: name.unwrap_or_default(),
                args: args.unwrap_or(serde_json::Value::Null),
                provider_metadata: provider_metadata.unwrap_or_default(),
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
            ContentBlock::Reasoning {
                text,
                provider_metadata,
            } => {
                let mut s = serializer.serialize_struct("ContentBlock", 3)?;
                s.serialize_field("type", "reasoning")?;
                s.serialize_field("text", text)?;
                if !provider_metadata.is_empty() {
                    s.serialize_field("provider_metadata", provider_metadata)?;
                }
                s.end()
            }
            ContentBlock::ToolCall {
                id,
                name,
                args,
                provider_metadata,
            } => {
                let mut s = serializer.serialize_struct("ContentBlock", 5)?;
                s.serialize_field("type", "tool_call")?;
                s.serialize_field("id", id)?;
                s.serialize_field("name", name)?;
                s.serialize_field("args", args)?;
                if !provider_metadata.is_empty() {
                    s.serialize_field("provider_metadata", provider_metadata)?;
                }
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

#[derive(Debug, Clone, Default)]
pub struct AgentMessage {
    pub role: String,
    pub content: Vec<ContentBlock>,
    pub name: String,
    pub tool_args: String,
    pub metadata: Option<serde_json::Map<String, serde_json::Value>>,
}

impl Serialize for AgentMessage {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let content = self.model_content();
        let mut state = serializer.serialize_struct("AgentMessage", 3)?;
        state.serialize_field("role", &self.role)?;
        state.serialize_field("content", &content)?;
        if let Some(metadata) = &self.metadata {
            state.serialize_field("metadata", metadata)?;
        }
        state.end()
    }
}

impl AgentMessage {
    /// The canonical, ordered content used for persistence and protocol
    /// lowering. `content` is the single source of truth; legacy JSONL side
    /// fields are normalized into it at deserialization time.
    pub fn model_content(&self) -> Vec<ContentBlock> {
        self.content.clone()
    }

    /// Concatenated reasoning text across all `Reasoning` content blocks.
    pub fn reasoning_text(&self) -> String {
        self.content
            .iter()
            .filter_map(|block| match block {
                ContentBlock::Reasoning { text, .. } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("")
    }

    /// Reconstructed tool calls from `ToolCall` content blocks.
    pub fn tool_calls(&self) -> Vec<AgentToolCall> {
        self.content
            .iter()
            .filter_map(|block| match block {
                ContentBlock::ToolCall {
                    id,
                    name,
                    args,
                    provider_metadata,
                } => Some(AgentToolCall {
                    id: id.clone(),
                    name: name.clone(),
                    args: args.clone(),
                    provider_metadata: provider_metadata.clone(),
                }),
                _ => None,
            })
            .collect()
    }

    /// The `tool_call_id` of the first `ToolResult` block (tool messages).
    pub fn tool_call_id(&self) -> String {
        self.content
            .iter()
            .find_map(|block| match block {
                ContentBlock::ToolResult { tool_call_id, .. } => Some(tool_call_id.clone()),
                _ => None,
            })
            .unwrap_or_default()
    }

    /// True when the message carries at least one tool-call block.
    pub fn has_tool_calls(&self) -> bool {
        self.content
            .iter()
            .any(|block| matches!(block, ContentBlock::ToolCall { .. }))
    }
}

impl<'de> Deserialize<'de> for AgentMessage {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize, Default)]
        #[serde(default)]
        struct StoredMessage {
            role: String,
            content: Vec<ContentBlock>,
            thinking: String,
            tool_calls: Vec<AgentToolCall>,
            tool_call_id: String,
            name: String,
            tool_args: String,
            metadata: Option<serde_json::Map<String, serde_json::Value>>,
        }

        let mut stored = StoredMessage::deserialize(deserializer)?;
        let has_reasoning = stored
            .content
            .iter()
            .any(|block| matches!(block, ContentBlock::Reasoning { .. }));
        if !stored.thinking.is_empty() && !has_reasoning {
            stored.content.insert(
                0,
                ContentBlock::reasoning(stored.thinking.clone(), ProviderMetadata::new()),
            );
        }
        let has_tool_calls = stored
            .content
            .iter()
            .any(|block| matches!(block, ContentBlock::ToolCall { .. }));
        if !has_tool_calls {
            stored.content.extend(stored.tool_calls.iter().map(|call| {
                ContentBlock::tool_call(
                    call.id.clone(),
                    call.name.clone(),
                    call.args.clone(),
                    call.provider_metadata.clone(),
                )
            }));
        }
        if stored.role == "tool"
            && !stored
                .content
                .iter()
                .any(|block| matches!(block, ContentBlock::ToolResult { .. }))
        {
            let text = stored
                .content
                .iter()
                .filter_map(|block| match block {
                    ContentBlock::Text { text } => Some(text.as_str()),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join("");
            stored
                .content
                .retain(|block| !matches!(block, ContentBlock::Text { .. }));
            stored.content.push(ContentBlock::tool_result(
                stored.tool_call_id.clone(),
                text,
                false,
            ));
        }

        Ok(AgentMessage {
            role: stored.role,
            content: stored.content,
            name: stored.name,
            tool_args: stored.tool_args,
            metadata: stored.metadata,
        })
    }
}

impl AgentMessage {
    /// Internal-only metadata key that binds an in-memory message to its
    /// authoritative session-journal entry. It is stripped before `meta` is
    /// serialized, so it never becomes user/provider-visible data.
    pub const JOURNAL_ENTRY_ID_KEY: &'static str = "_future_journal_entry_id";

    pub fn journal_entry_id(&self) -> Option<&str> {
        self.metadata
            .as_ref()
            .and_then(|metadata| metadata.get(Self::JOURNAL_ENTRY_ID_KEY))
            .and_then(serde_json::Value::as_str)
            .filter(|id| !id.is_empty())
    }

    pub fn ensure_journal_entry_id(&mut self) -> String {
        if let Some(id) = self.journal_entry_id() {
            return id.to_string();
        }
        let id = crate::utils::generate_entry_id();
        self.metadata
            .get_or_insert_with(serde_json::Map::new)
            .insert(
                Self::JOURNAL_ENTRY_ID_KEY.to_string(),
                serde_json::Value::String(id.clone()),
            );
        id
    }

    pub fn text(&self) -> String {
        self.content
            .iter()
            .filter_map(|b| match b {
                ContentBlock::Text { text } => Some(text.clone()),
                ContentBlock::ToolResult { content, .. } => Some(content.clone()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("")
    }

    /// The user's visible text: only the FIRST text block. Later text blocks
    /// are agent-injected context (attachment manifest, file paths) that
    /// `build_user_message` appends for the model — they must never reach a
    /// message bubble. Mirrors `get_session_entries`, which renders user
    /// entries from the first text block only.
    pub fn display_text(&self) -> String {
        self.content
            .iter()
            .find_map(|b| match b {
                ContentBlock::Text { text } => Some(text.clone()),
                _ => None,
            })
            .unwrap_or_default()
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
    #[serde(default, skip_serializing_if = "serde_json::Map::is_empty")]
    pub provider_metadata: ProviderMetadata,
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

/// A local file attached to a prompt (GUI). The Agent preserves the absolute
/// path supplied by the caller and reads it on demand instead of copying it.
/// Images are read from this path when a run starts and converted to an
/// image_url block only for the model request; their bytes are not queued.
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
    #[serde(
        rename = "reasoning_tokens",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub reasoning_tokens: Option<i64>,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_metadata: Option<ProviderMetadata>,
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
    async fn stream_model(
        &self,
        request: crate::llm::schema::ModelRequest,
    ) -> anyhow::Result<tokio_stream::wrappers::ReceiverStream<crate::llm::schema::ModelStreamEvent>>;

    /// Refresh only the API key at runtime, after an out-of-band credential
    /// change (FutureGene login/logout, custom-provider key edits). This leaves
    /// the base_url untouched — a login/logout changes the key, not the model's
    /// endpoint.
    fn set_api_key(&self, _api_key: &str) {}

    /// Refresh the provider endpoint in the same shared runtime client. Run
    /// snapshots clone the provider Arc, so this also keeps already-accepted
    /// queued work aligned with a committed provider edit.
    fn set_base_url(&self, _base_url: &str) {}

    /// Update thinking level and budget at runtime (after set_thinking_level / cycle_thinking_level).
    fn update_thinking(&self, _level: &str, _budget: i32) {}
}

// ─── Message ↔ AgentMessage conversion ────────────────────────────────────

impl AgentMessage {
    pub fn to_llm(&self) -> Message {
        let model_content = self.model_content();
        let blocks: Vec<serde_json::Value> = model_content
            .iter()
            .filter_map(|block| match block {
                ContentBlock::Text { .. } | ContentBlock::Image { .. } => {
                    Some(serde_json::to_value(block).unwrap_or(serde_json::Value::Null))
                }
                ContentBlock::ToolResult { content, .. } => {
                    Some(serde_json::json!({"type": "text", "text": content}))
                }
                ContentBlock::Reasoning { .. } | ContentBlock::ToolCall { .. } => None,
            })
            .collect();
        let content = (!blocks.is_empty()).then_some(serde_json::Value::Array(blocks));

        let canonical_tool_calls: Vec<ToolCall> = model_content
            .iter()
            .filter_map(|block| match block {
                ContentBlock::ToolCall { id, name, args, .. } => Some(ToolCall {
                    id: id.clone(),
                    call_type: "function".to_string(),
                    function: ToolCallFn {
                        name: name.clone(),
                        arguments: match args {
                            serde_json::Value::String(value) => {
                                serde_json::Value::String(value.clone())
                            }
                            other => serde_json::Value::String(
                                serde_json::to_string(other).unwrap_or_default(),
                            ),
                        },
                    },
                }),
                _ => None,
            })
            .collect();
        let tool_calls = if canonical_tool_calls.is_empty() {
            None
        } else {
            Some(canonical_tool_calls)
        };
        let reasoning_content = model_content
            .iter()
            .filter_map(|block| match block {
                ContentBlock::Reasoning { text, .. } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("");
        let tool_result_id = model_content.iter().find_map(|block| match block {
            ContentBlock::ToolResult { tool_call_id, .. } => Some(tool_call_id.clone()),
            _ => None,
        });

        Message {
            role: self.role.clone(),
            content,
            tool_calls,
            tool_call_id: tool_result_id.unwrap_or_default(),
            name: self.name.clone(),
            tool_args: self.tool_args.clone(),
            reasoning_content,
        }
    }
}

pub fn convert_to_llm(msgs: &[AgentMessage]) -> Vec<Message> {
    msgs.iter()
        // Drop empty assistant messages (no visible content and no tool_calls)
        // before they reach the model. A crash/interrupt can leave a
        // reasoning-only assistant entry in the journal; on resume the API
        // rejects it with "content or tool_calls must be set". Filtering here —
        // at the single AgentMessage → Message boundary every send path and
        // every provider format flows through — keeps the model-bound context
        // clean without touching display or the journal.
        .filter(|m| {
            !(m.role == "assistant"
                && m.content
                    .iter()
                    .all(|block| matches!(block, ContentBlock::Reasoning { .. })))
        })
        .map(|m| m.to_llm())
        .collect()
}

pub fn convert_from_llm(msgs: Vec<Message>) -> Vec<AgentMessage> {
    msgs.into_iter()
        .map(|m| {
            let mut content = if let Some(c) = m.content {
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

            if !m.reasoning_content.is_empty() {
                content.insert(
                    0,
                    ContentBlock::reasoning(m.reasoning_content, ProviderMetadata::new()),
                );
            }
            if let Some(tcs) = m.tool_calls {
                for tc in tcs {
                    content.push(ContentBlock::tool_call(
                        tc.id,
                        tc.function.name,
                        tc.function.arguments,
                        ProviderMetadata::new(),
                    ));
                }
            }
            if m.role == "tool" {
                let text = content
                    .iter()
                    .filter_map(|block| match block {
                        ContentBlock::Text { text } => Some(text.as_str()),
                        _ => None,
                    })
                    .collect::<Vec<_>>()
                    .join("");
                content.retain(|block| !matches!(block, ContentBlock::Text { .. }));
                content.push(ContentBlock::tool_result(m.tool_call_id, text, false));
            }

            AgentMessage {
                role: m.role,
                content,
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
        assert_eq!(b.as_text().unwrap(), "hello");
    }

    #[test]
    fn content_block_image() {
        let b = ContentBlock::image("data:image/png;base64,abc");
        let image_url = b.as_image().unwrap();
        assert_eq!(image_url.url.as_deref(), Some("data:image/png;base64,abc"));
    }

    #[test]
    fn content_block_tool_result() {
        let b = ContentBlock::tool_result("call_1", "output text", false);
        let (tool_call_id, content, is_error) = b.as_tool_result().unwrap();
        assert_eq!(tool_call_id, "call_1");
        assert_eq!(content, "output text");
        assert!(!is_error);
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
        assert_eq!(b.as_text().unwrap(), "hello");
    }

    #[test]
    fn deserialize_image_block() {
        let json = r#"{"type":"image_url","image_url":{"url":"data:..."}}"#;
        let b: ContentBlock = serde_json::from_str(json).unwrap();
        assert_eq!(b.as_image().unwrap().url.as_deref(), Some("data:..."));
    }

    #[test]
    fn deserialize_tool_result_block() {
        let json = r#"{"type":"tool_result","tool_call_id":"c1","content":"done","is_error":true}"#;
        let b: ContentBlock = serde_json::from_str(json).unwrap();
        let (tool_call_id, content, is_error) = b.as_tool_result().unwrap();
        assert_eq!(tool_call_id, "c1");
        assert_eq!(content, "done");
        assert!(is_error);
    }

    #[test]
    fn deserialize_unknown_type_falls_back_to_text() {
        let json = r#"{"type":"unknown_type","text":"fallback"}"#;
        let b: ContentBlock = serde_json::from_str(json).unwrap();
        assert_eq!(b.as_text().unwrap(), "fallback");
    }

    #[test]
    fn content_block_roundtrip_text() {
        let original = ContentBlock::text("roundtrip");
        let json = serde_json::to_string(&original).unwrap();
        let restored: ContentBlock = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.as_text().unwrap(), "roundtrip");
    }

    #[test]
    fn content_block_roundtrip_tool_result() {
        let original = ContentBlock::tool_result("c2", "result", true);
        let json = serde_json::to_string(&original).unwrap();
        let restored: ContentBlock = serde_json::from_str(&json).unwrap();
        let (tool_call_id, content, is_error) = restored.as_tool_result().unwrap();
        assert_eq!(tool_call_id, "c2");
        assert_eq!(content, "result");
        assert!(is_error);
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

    #[test]
    fn image_url_data_serialize_empty() {
        let d = ImageUrlData { url: None };
        let json = serde_json::to_value(&d).unwrap();
        assert!(json.is_object());
    }

    // ─── deserialize_credit_cost additional visitors ────────────────────────

    #[test]
    fn usage_credit_cost_as_i64() {
        let json = r#"{"prompt_tokens":0,"completion_tokens":0,"total_tokens":0,"credit_cost":0}"#;
        let u: Usage = serde_json::from_str(json).unwrap();
        assert_eq!(u.credit_cost, Some(0.0));
    }

    #[test]
    fn usage_credit_cost_as_bool_returns_none() {
        let json =
            r#"{"prompt_tokens":0,"completion_tokens":0,"total_tokens":0,"credit_cost":true}"#;
        let u: Usage = serde_json::from_str(json).unwrap();
        assert!(u.credit_cost.is_none());
    }

    #[test]
    fn usage_credit_cost_negative_integer_uses_i64_visitor() {
        let json = r#"{"prompt_tokens":0,"completion_tokens":0,"total_tokens":0,"credit_cost":-5}"#;
        let u: Usage = serde_json::from_str(json).unwrap();
        assert_eq!(u.credit_cost, Some(-5.0));
    }

    #[test]
    fn usage_credit_cost_wrong_type_reports_expecting_message() {
        // An array payload hits no visitor arm → serde renders `expecting`.
        let json =
            r#"{"prompt_tokens":0,"completion_tokens":0,"total_tokens":0,"credit_cost":[1]}"#;
        let err = serde_json::from_str::<Usage>(json).unwrap_err();
        assert!(
            err.to_string().contains("representing credit cost"),
            "{err}"
        );
    }

    #[test]
    fn convert_from_llm_unpacks_image_url_object_form() {
        let msgs = vec![Message {
            role: "user".to_string(),
            content: Some(serde_json::json!([
                {"type": "image_url", "image_url": {"url": "data:obj-form"}}
            ])),
            ..Default::default()
        }];
        let converted = convert_from_llm(msgs);
        let image_url = converted[0].content[0].as_image().unwrap();
        assert_eq!(image_url.url.as_deref(), Some("data:obj-form"));
    }

    #[test]
    fn content_block_accessors_return_none_on_mismatch() {
        let text = ContentBlock::text("x");
        assert!(text.as_image().is_none());
        assert!(text.as_tool_result().is_none());
        let image = ContentBlock::image("data:image/png;base64,a");
        assert!(image.as_text().is_none());
        assert!(image.as_tool_result().is_none());
        let result = ContentBlock::tool_result("c", "out", false);
        assert!(result.as_text().is_none());
        assert!(result.as_image().is_none());
    }

    // ─── ContentBlock visitor ──────────────────────────────────────────────

    #[test]
    fn content_block_visit_seq_errors() {
        let json = r#"["not", "an", "object"]"#;
        let result: Result<ContentBlock, _> = serde_json::from_str(json);
        assert!(result.is_err());
    }

    // ─── AgentTool / ToolCallFn ────────────────────────────────────────────

    #[test]
    fn agent_tool_call_args_string_preserved() {
        let msg = AgentMessage {
            role: "assistant".to_string(),
            content: vec![ContentBlock::tool_call(
                "c1",
                "shell",
                serde_json::json!("string-args"),
                ProviderMetadata::new(),
            )],
            ..Default::default()
        };
        let llm = msg.to_llm();
        let tcs = llm.tool_calls.unwrap();
        assert_eq!(tcs[0].function.arguments, serde_json::json!("string-args"));
    }

    // ─── ImageContent / ImageSource ────────────────────────────────────────

    #[test]
    fn image_content_serialization() {
        let ic = ImageContent {
            content_type: "image".to_string(),
            mime_type: Some("image/png".to_string()),
            data: Some("base64data".to_string()),
            source: None,
            file_path: Some("/tmp/img.png".to_string()),
        };
        let json = serde_json::to_value(&ic).unwrap();
        assert_eq!(json["type"], "image");
        assert_eq!(json["mime_type"], "image/png");
        assert_eq!(json["file_path"], "/tmp/img.png");
    }

    #[test]
    fn image_source_serialization() {
        let src = ImageSource {
            source_type: "base64".to_string(),
            media_type: "image/png".to_string(),
            data: "encoded".to_string(),
        };
        let json = serde_json::to_value(&src).unwrap();
        assert_eq!(json["type"], "base64");
        assert_eq!(json["media_type"], "image/png");
    }

    // ─── ToolCallResult ────────────────────────────────────────────────────

    #[test]
    fn tool_call_result_debug() {
        let r = ToolCallResult {
            result: "output".to_string(),
            is_error: false,
        };
        let debug = format!("{r:?}");
        assert!(debug.contains("output"));
    }

    // ─── AgentConfig default ───────────────────────────────────────────────

    #[test]
    fn agent_config_default_values() {
        let c = AgentConfig {
            system_prompt: "prompt".to_string(),
            max_turns: 10,
            thinking_budget: 0,
            max_retries: 3,
            stop_condition: None,
            before_tool_call: None,
            prepare_tool_call: None,
            finalize_tool_call: None,
            after_tool_call: None,
            tools_execution_mode: "parallel".to_string(),
        };
        assert_eq!(c.max_turns, 10);
        assert_eq!(c.max_retries, 3);
        assert_eq!(c.tools_execution_mode, "parallel");
    }

    // ─── Model serialization extras ────────────────────────────────────────

    #[test]
    fn model_with_headers_and_compat() {
        let json = r#"{
            "id": "custom-model",
            "name": "Custom",
            "provider": "custom",
            "api": "openai",
            "baseUrl": "https://api.example.com",
            "contextWindow": 32000,
            "maxTokens": 8192,
            "reasoning": true,
            "hide": true,
            "headers": {"X-Custom": "value"},
            "compat": {"force_json": true}
        }"#;
        let m: Model = serde_json::from_str(json).unwrap();
        assert!(m.hide);
        assert!(m.reasoning);
        assert!(m.headers.is_some());
        assert!(m.compat.is_some());
    }

    // ─── TextContent ───────────────────────────────────────────────────────

    #[test]
    fn text_content_serialization() {
        let tc = TextContent {
            content_type: "text".to_string(),
            text: "hello".to_string(),
        };
        let json = serde_json::to_value(&tc).unwrap();
        assert_eq!(json["type"], "text");
        assert_eq!(json["text"], "hello");
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
    fn agent_message_display_text_is_first_block_only() {
        // build_user_message appends an attachment-manifest text block after
        // the typed message; display_text must exclude it (the manifest must
        // never reach a message bubble).
        let mut msg = AgentMessage::default();
        msg.add_text("识别");
        msg.add_text("\n\nUser attachment metadata follows as a JSON array: [{\"kind\":\"file\"}]");
        assert_eq!(msg.display_text(), "识别");
        assert!(msg.text().contains("attachment metadata"));
    }

    #[test]
    fn agent_message_display_text_empty_without_text_blocks() {
        let mut msg = AgentMessage::default();
        msg.content.push(ContentBlock::image("data:..."));
        assert_eq!(msg.display_text(), "");
    }

    #[test]
    fn agent_message_add_image() {
        let mut msg = AgentMessage::default();
        msg.add_image("image/png".to_string(), "aGVsbG8=".to_string());
        assert_eq!(msg.content.len(), 1);
        let image_url = msg.content[0].as_image().unwrap();
        assert!(image_url
            .url
            .as_ref()
            .unwrap()
            .starts_with("data:image/png;base64,aGVsbG8="));
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
            content: vec![ContentBlock::tool_call(
                "call_1",
                "shell",
                serde_json::json!({"command": "ls"}),
                ProviderMetadata::new(),
            )],
            ..Default::default()
        };
        let json = serde_json::to_value(&msg).unwrap();
        assert_eq!(json["content"][0]["type"], "tool_call");
        assert_eq!(json["content"][0]["name"], "shell");
        assert!(json.get("tool_calls").is_none());
    }

    #[test]
    fn legacy_agent_message_normalizes_to_canonical_blocks() {
        let message: AgentMessage = serde_json::from_value(serde_json::json!({
            "role": "assistant",
            "content": [{"type": "text", "text": "answer"}],
            "thinking": "summary",
            "tool_calls": [{"id": "call_1", "name": "shell", "args": {"command": "ls"}}]
        }))
        .unwrap();
        assert!(matches!(message.content[0], ContentBlock::Reasoning { .. }));
        assert!(matches!(message.content[2], ContentBlock::ToolCall { .. }));

        let stored = serde_json::to_value(message).unwrap();
        assert!(stored.get("thinking").is_none());
        assert!(stored.get("tool_calls").is_none());
        assert_eq!(stored["content"][0]["type"], "reasoning");
        assert_eq!(stored["content"][2]["type"], "tool_call");
    }

    #[test]
    fn provider_metadata_roundtrips_on_reasoning_and_tool_call() {
        let mut openai = ProviderMetadata::new();
        openai.insert(
            "openai".into(),
            serde_json::json!({"id": "rs_1", "encrypted_content": "cipher"}),
        );
        let message = AgentMessage {
            role: "assistant".into(),
            content: vec![
                ContentBlock::reasoning("summary", openai.clone()),
                ContentBlock::tool_call("call_1", "read", serde_json::json!({}), openai),
            ],
            ..Default::default()
        };
        let stored = serde_json::to_string(&message).unwrap();
        let loaded: AgentMessage = serde_json::from_str(&stored).unwrap();
        assert!(matches!(
            &loaded.content[0],
            ContentBlock::Reasoning { provider_metadata, .. }
                if provider_metadata["openai"]["encrypted_content"] == "cipher"
        ));
        assert!(matches!(
            &loaded.content[1],
            ContentBlock::ToolCall { provider_metadata, .. }
                if provider_metadata["openai"]["id"] == "rs_1"
        ));
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
            content: vec![ContentBlock::tool_call(
                "c1",
                "shell",
                serde_json::json!({"command": "ls"}),
                ProviderMetadata::new(),
            )],
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
    fn convert_to_llm_skips_empty_assistant() {
        // Regression: a crash mid-turn can leave a reasoning-only assistant
        // entry (no content, no tool_calls). convert_to_llm must drop it so the
        // API never sees "content or tool_calls must be set" on resume.
        let msgs = vec![
            AgentMessage {
                role: "assistant".to_string(),
                content: vec![ContentBlock::reasoning(
                    "thinking...",
                    ProviderMetadata::new(),
                )],
                ..Default::default()
            },
            AgentMessage {
                role: "user".to_string(),
                content: vec![ContentBlock::text("hi")],
                ..Default::default()
            },
        ];
        let llm_msgs = convert_to_llm(&msgs);
        assert_eq!(llm_msgs.len(), 1);
        assert_eq!(llm_msgs[0].role, "user");
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
        assert_eq!(agent_msgs[0].tool_calls().len(), 1);
        assert_eq!(agent_msgs[0].tool_calls()[0].name, "read");
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
        assert_eq!(agent_msgs[0].reasoning_text(), "thinking...");
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

    // ─── coverage batch: deserialization arms ──────────────────────────────

    #[test]
    fn content_block_ignores_unknown_fields() {
        // The `_ => next_value` skip arm in the custom ContentBlock visitor.
        let block: ContentBlock =
            serde_json::from_str(r#"{"type":"text","text":"ok","unexpected":{"nested":1}}"#)
                .unwrap();
        assert_eq!(block.as_text().unwrap(), "ok");
    }

    #[test]
    fn new_user_parses_content_block_variants() {
        // image_url object form, with and without a nested url key.
        let msg = AgentMessage::new_user(
            "user",
            serde_json::json!([
                {"type": "text", "text": "look"},
                {"type": "image_url", "image_url": {"url": "data:image/png;base64,a"}},
                {"type": "image_url", "image_url": "data:image/png;base64,b"},
                {"type": "image_url"},
                {"type": "custom", "payload": 1},
                42,
                "plain-string-entry"
            ]),
        );
        let images: Vec<_> = msg
            .content
            .iter()
            .filter(|b| matches!(b, ContentBlock::Image { .. }))
            .collect();
        assert_eq!(images.len(), 3, "all image_url variants become images");
        let image_url = images[0].as_image().unwrap();
        assert_eq!(image_url.url.as_deref(), Some("data:image/png;base64,a"));
        // The bare-string form is not unpacked by this conversion path.
        let image_url = images[1].as_image().unwrap();
        assert_eq!(image_url.url.as_deref(), Some(""));
        // Unknown object types fall back to their JSON text form.
        assert!(msg
            .content
            .iter()
            .any(|b| matches!(b, ContentBlock::Text { text } if text.contains("payload"))));
        // Non-object entries are dropped.
        assert!(!msg
            .content
            .iter()
            .any(|b| matches!(b, ContentBlock::Text { text } if text == "42")));

        // Plain string content becomes a single text block.
        let msg = AgentMessage::new_user("user", serde_json::json!("just text"));
        assert_eq!(msg.text(), "just text");
        // Empty string content yields no visible text.
        let msg = AgentMessage::new_user("user", serde_json::json!(""));
        assert!(msg.text().is_empty());
    }

    #[test]
    fn usage_credit_cost_accepts_string_number_and_absent() {
        let usage: Usage = serde_json::from_str(
            r#"{"prompt_tokens":1,"completion_tokens":2,"total_tokens":3,"credit_cost":"0.00019"}"#,
        )
        .unwrap();
        assert_eq!(usage.credit_cost, Some(0.00019));

        let usage: Usage = serde_json::from_str(
            r#"{"prompt_tokens":1,"completion_tokens":2,"total_tokens":3,"credit_cost":0.5}"#,
        )
        .unwrap();
        assert_eq!(usage.credit_cost, Some(0.5));

        let usage: Usage =
            serde_json::from_str(r#"{"prompt_tokens":1,"completion_tokens":2,"total_tokens":3}"#)
                .unwrap();
        assert_eq!(usage.credit_cost, None);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn provider_default_key_and_thinking_setters_are_noops() {
        struct MinimalProvider;
        #[async_trait::async_trait]
        impl LLMProvider for MinimalProvider {
            async fn stream_model(
                &self,
                _request: crate::llm::schema::ModelRequest,
            ) -> anyhow::Result<
                tokio_stream::wrappers::ReceiverStream<crate::llm::schema::ModelStreamEvent>,
            > {
                let (_tx, rx) = tokio::sync::mpsc::channel(1);
                Ok(tokio_stream::wrappers::ReceiverStream::new(rx))
            }
        }
        let provider = MinimalProvider;
        provider.set_api_key("ignored");
        provider.update_thinking("high", 1234);
        // The canonical model stream implementation is callable too.
        let mut stream = provider
            .stream_model(crate::llm::schema::ModelRequest {
                model: "m".into(),
                system_prompt: String::new(),
                messages: vec![],
                tools: vec![],
            })
            .await
            .unwrap();
        use tokio_stream::StreamExt;
        assert!(stream.next().await.is_none());
    }

    #[test]
    fn usage_credit_cost_integer_and_null_forms() {
        let usage: Usage = serde_json::from_str(
            r#"{"prompt_tokens":1,"completion_tokens":2,"total_tokens":3,"credit_cost":2}"#,
        )
        .unwrap();
        assert_eq!(usage.credit_cost, Some(2.0));
        let usage: Usage = serde_json::from_str(
            r#"{"prompt_tokens":1,"completion_tokens":2,"total_tokens":3,"credit_cost":null}"#,
        )
        .unwrap();
        assert_eq!(usage.credit_cost, None);
    }

    #[test]
    fn convert_from_llm_content_block_variants() {
        let messages = vec![
            Message {
                role: "assistant".to_string(),
                content: Some(serde_json::json!([
                    {"type": "text", "text": "t"},
                    {"type": "image_url", "image_url": "data:image/png;base64,x"},
                    {"type": "image_url", "image_url": 42},
                    {"type": "unknown"},
                    "plain-entry"
                ])),
                ..Default::default()
            },
            Message {
                role: "user".to_string(),
                content: Some(serde_json::json!("string content")),
                ..Default::default()
            },
            Message {
                role: "user".to_string(),
                content: Some(serde_json::json!("")),
                ..Default::default()
            },
            Message {
                role: "user".to_string(),
                content: None,
                ..Default::default()
            },
        ];
        let converted = convert_from_llm(messages);
        assert_eq!(converted.len(), 4);
        // The string-form image_url keeps its URL in this direction.
        let images: Vec<_> = converted[0]
            .content
            .iter()
            .filter(|b| matches!(b, ContentBlock::Image { .. }))
            .collect();
        assert_eq!(images.len(), 2);
        let image_url = images[0].as_image().unwrap();
        assert_eq!(image_url.url.as_deref(), Some("data:image/png;base64,x"));
        let image_url = images[1].as_image().unwrap();
        assert!(image_url.url.is_none());
        assert_eq!(converted[1].text(), "string content");
        assert!(converted[2].content.is_empty());
        assert!(converted[3].content.is_empty());
    }

    #[test]
    fn agent_tool_debug_redacts_handler() {
        let tool = AgentTool {
            def: ToolDef {
                tool_type: "function".to_string(),
                function: FunctionDef {
                    name: "t".to_string(),
                    description: "d".to_string(),
                    parameters: serde_json::json!({}),
                },
            },
            handler: |_: serde_json::Value| Box::pin(async { Ok("ok".to_string()) }),
            guidelines: vec![],
        };
        let debug = format!("{tool:?}");
        assert!(debug.contains("<fn>"));
        assert!(debug.contains("\"t\""));
        // The handler itself is callable.
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let result = runtime.block_on((tool.handler)(serde_json::json!({})));
        assert_eq!(result.unwrap(), "ok");
    }
}
