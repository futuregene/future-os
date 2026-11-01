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
    /// The effort values this access path accepts (`compat.reasoningEffort`).
    /// `None` sends a level verbatim, which is the historical contract.
    pub reasoning_effort: Option<ReasoningEffortVocabulary>,
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
    /// The effort values this access path accepts (`compat.reasoningEffort`).
    /// The Responses API has one fixed location for the value
    /// (`reasoning.effort`), so a `param` in the declaration is ignored here.
    pub reasoning_effort: Option<ReasoningEffortVocabulary>,
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
            reasoning_effort: None,
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

/// Position of a known level on the fixed display scale, weakest first.
/// Returns `None` for a wire value the product does not name.
pub fn thinking_level_rank(name: &str) -> Option<usize> {
    KNOWN_THINKING_LEVELS
        .iter()
        .position(|known| *known == name)
}

/// Normalize a configured level list into the fixed display order.
///
/// Trim, drop empties and the reserved `default`, dedupe, cap the count and
/// each name's length, then sort: known levels follow [`KNOWN_THINKING_LEVELS`]
/// (off → … → max), unknown wire values keep their declaration order after the
/// known ones. Declaration order is a set, not a display sequence — every
/// picker shows this list untouched, so sorting here is what keeps the order
/// identical regardless of how the configuration was written.
///
/// A dropped entry is logged against the provider/model so a typo is visible
/// without failing the whole model.
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
    // Fixed display order: known levels weakest → strongest, then unknown wire
    // values in the order the configuration listed them.
    let mut known: Vec<String> = Vec::new();
    let mut unknown: Vec<String> = Vec::new();
    for level in out {
        if thinking_level_rank(&level).is_some() {
            known.push(level);
        } else {
            unknown.push(level);
        }
    }
    known.sort_by_key(|level| thinking_level_rank(level).unwrap_or(usize::MAX));
    let out = known.into_iter().chain(unknown).collect();
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

/// The `reasoning_effort` values one access path accepts, declared as data.
///
/// Providers do not share one effort vocabulary: the OpenAI family takes
/// `minimal|low|medium|high`, DeepSeek V4.1 takes `low|high|xhigh|max` (or an
/// integer 1..=100), some gateways take only the integer and reject every
/// spelling, and a few accept several spellings that are not on our scale at
/// all. The product's thinking levels are deliberately the same for every model
/// — a per-model picker vocabulary would be a second list to keep in step with
/// the first — so a model that does not accept a level *by name* declares what
/// it does accept here, and [`Self::resolve`] maps our level onto it.
///
/// Declared as `compat.reasoningEffort` on the model, or on its provider (a
/// provider-level block is merged into every model that does not override the
/// key, exactly like `thinkingFormat`):
///
/// ```json
/// "compat": {
///   "reasoningEffort": {
///     "values": ["low", "high", "xhigh", "max"],
///     "integer": {"min": 1, "max": 100},
///     "param": "chat_template_kwargs.reasoning_effort"
///   }
/// }
/// ```
///
/// - `values` — accepted spellings, sent unchanged. A level listed here always
///   travels as itself, so a declaration that lists every level reproduces the
///   verbatim behaviour of a model without this block.
/// - `integer` — inclusive accepted integer range, for providers whose depth is
///   a number (DeepSeek V4's `1..=100`, a token budget, …).
/// - `param` — where the value goes, as a dotted path; default
///   `reasoning_effort`. It exists because a gateway may validate a *number*
///   only inside `chat_template_kwargs`: westlakehpc's DeepSeek V4 accepts the
///   spellings at the top level and an integer only under
///   `chat_template_kwargs`, and answers a top-level integer with
///   `500 json: cannot unmarshal number into Go struct field
///   GeneralOpenAIRequest.reasoning_effort of type string`.
///
/// At least one of `values`/`integer` is required, and an unknown key is an
/// error: a typo such as `"value"` would otherwise silently shrink the
/// vocabulary and put the provider's 400 back on the wire.
///
/// Mapping rules ([`Self::resolve`]); all monotone, so a deeper level never
/// maps to a shallower value:
///
/// 1. A level listed in `values` travels as itself.
/// 2. A level that is *not* listed travels as an integer when `integer` is
///    declared: the provider's range is a continuous depth axis and our seven
///    levels are evenly spaced points on it, so a `1..=100` range puts `medium`
///    (the middle of the scale) at `50` and `minimal` at `17`. This is the one
///    place we *do* pick a value the provider never named, so the assumption is
///    explicit: a spelling in `values` is expected to sit at its own scale
///    position on that axis, which is what makes the result monotone against
///    the spellings that are sent as themselves.
/// 3. Every other unlisted level snaps to the closest listed spelling on that
///    scale, ties going to the shallower one — the same rule as
///    [`clamp_thinking_level`]: a request that cannot be honored never spends
///    more than the user asked for. A level with no position on the scale
///    (`turbo`) is not expressible; it sends nothing (the provider default) and
///    [`report_unexpressible_levels`] names it at model resolution time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReasoningEffortVocabulary {
    values: Vec<String>,
    integer: Option<(i64, i64)>,
    param: String,
}

/// How one product level travels under a [`ReasoningEffortVocabulary`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EffortMapping {
    /// Nothing is sent (the provider's own default applies).
    None,
    /// The level's own spelling, which the model lists.
    Exact(String),
    /// An integer on the provider's declared range.
    Integer(i64),
    /// The closest listed spelling, chosen when the provider declares no
    /// integer range (reported by [`report_unexpressible_levels`]).
    Nearest(String),
}

impl ReasoningEffortVocabulary {
    /// Where the value goes when the declaration does not say.
    pub const DEFAULT_PARAM: &'static str = "reasoning_effort";
    /// Guard for the integer range so the scaling arithmetic cannot overflow.
    const INTEGER_BOUND: i64 = 1_000_000;

    /// Parse one `compat.reasoningEffort` object.
    pub fn parse(provider: &str, model: &str, value: &Value) -> Result<Self> {
        let invalid = |detail: String| -> anyhow::Error {
            anyhow!("provider `{provider}` model `{model}` compat.reasoningEffort {detail}")
        };
        let Some(object) = value.as_object() else {
            return Err(invalid(
                "must be an object such as {\"values\":[\"low\",\"high\"],\"integer\":{\"min\":1,\"max\":100}}"
                    .to_string(),
            ));
        };
        for key in object.keys() {
            if !matches!(key.as_str(), "values" | "integer" | "param") {
                return Err(invalid(format!(
                    "has unknown key `{key}`; expected values, integer, param"
                )));
            }
        }

        let mut values: Vec<String> = Vec::new();
        match object.get("values") {
            None | Some(Value::Null) => {}
            Some(Value::Array(list)) => {
                for entry in list {
                    let Some(name) = entry.as_str() else {
                        return Err(invalid("values must be an array of strings".to_string()));
                    };
                    let name = name.trim();
                    if name.is_empty() || name.chars().count() > MAX_THINKING_LEVEL_LEN {
                        return Err(invalid(format!(
                            "has an invalid value `{name}` (1..={MAX_THINKING_LEVEL_LEN} characters)"
                        )));
                    }
                    if !values.iter().any(|seen| seen == name) {
                        values.push(name.to_string());
                    }
                }
            }
            Some(_) => return Err(invalid("values must be an array of strings".to_string())),
        }

        let integer = match object.get("integer") {
            None | Some(Value::Null) => None,
            Some(Value::Object(range)) => {
                for key in range.keys() {
                    if !matches!(key.as_str(), "min" | "max") {
                        return Err(invalid(format!(
                            "integer has unknown key `{key}`; expected min, max"
                        )));
                    }
                }
                let bound = |key: &str| -> Result<i64> {
                    let Some(number) = range.get(key).and_then(Value::as_i64) else {
                        return Err(invalid(format!("integer.{key} must be an integer")));
                    };
                    if number.abs() > Self::INTEGER_BOUND {
                        return Err(invalid(format!(
                            "integer.{key} is out of range (±{})",
                            Self::INTEGER_BOUND
                        )));
                    }
                    Ok(number)
                };
                let (min, max) = (bound("min")?, bound("max")?);
                if min >= max {
                    return Err(invalid(
                        "integer.min must be lower than integer.max".to_string(),
                    ));
                }
                Some((min, max))
            }
            Some(_) => {
                return Err(invalid(
                    "integer must be an object such as {\"min\":1,\"max\":100}".to_string(),
                ))
            }
        };

        let param = match object.get("param") {
            None | Some(Value::Null) => Self::DEFAULT_PARAM.to_string(),
            Some(Value::String(param)) => {
                let param = param.trim();
                let valid = !param.is_empty()
                    && param.len() <= 64
                    && param.split('.').all(|segment| {
                        !segment.is_empty()
                            && segment
                                .chars()
                                .all(|c| c.is_ascii_alphanumeric() || c == '_')
                    });
                if !valid {
                    return Err(invalid(format!(
                        "param `{param}` must be a dotted path of identifiers, such as \
                         `reasoning_effort` or `chat_template_kwargs.reasoning_effort`"
                    )));
                }
                param.to_string()
            }
            Some(_) => return Err(invalid("param must be a string".to_string())),
        };

        if values.is_empty() && integer.is_none() {
            return Err(invalid(
                "declares neither values nor integer; at least one accepted form is required"
                    .to_string(),
            ));
        }
        Ok(Self {
            values,
            integer,
            param,
        })
    }

    /// The accepted spellings, in declaration order.
    pub fn values(&self) -> &[String] {
        &self.values
    }

    /// The accepted inclusive integer range, when one is declared.
    pub fn integer(&self) -> Option<(i64, i64)> {
        self.integer
    }

    /// The dotted path the value is written to.
    pub fn param(&self) -> &str {
        &self.param
    }

    /// Whether `value` is one of the accepted wire values.
    pub fn accepts(&self, value: &Value) -> bool {
        match value {
            Value::String(value) => self.values.iter().any(|accepted| accepted == value),
            Value::Number(number) => match (number.as_i64(), self.integer) {
                (Some(number), Some((min, max))) => number >= min && number <= max,
                _ => false,
            },
            _ => false,
        }
    }

    /// How a product level travels under this vocabulary.
    pub fn mapping(&self, level: &str) -> EffortMapping {
        let level = level.trim();
        if level.is_empty() || level == DEFAULT_THINKING_LEVEL || level == "off" {
            return EffortMapping::None;
        }
        if self.values.iter().any(|value| value == level) {
            return EffortMapping::Exact(level.to_string());
        }
        if let (Some(rank), Some((min, max))) = (thinking_level_rank(level), self.integer) {
            return EffortMapping::Integer(integer_for_rank(rank, min, max));
        }
        match self.nearest(thinking_level_rank(level)) {
            Some(value) => EffortMapping::Nearest(value),
            None => EffortMapping::None,
        }
    }

    /// Map a product level onto this vocabulary; `None` sends nothing.
    pub fn resolve(&self, level: &str) -> Option<Value> {
        match self.mapping(level) {
            EffortMapping::None => None,
            EffortMapping::Exact(value) | EffortMapping::Nearest(value) => {
                Some(Value::String(value))
            }
            EffortMapping::Integer(value) => Some(Value::from(value)),
        }
    }

    /// The accepted values as one human-readable phrase, for errors and logs.
    pub fn describe(&self) -> String {
        let mut parts = self.values.clone();
        if let Some((min, max)) = self.integer {
            parts.push(format!("an integer {min}..={max}"));
        }
        match parts.len() {
            0 => "nothing".to_string(),
            1 => parts.remove(0),
            _ => {
                let last = parts.pop().unwrap_or_default();
                format!("{} or {last}", parts.join(", "))
            }
        }
    }

    /// The closest listed spelling on the product scale; ties go shallower.
    fn nearest(&self, rank: Option<usize>) -> Option<String> {
        let rank = rank?;
        let mut best: Option<(usize, usize, &str)> = None;
        for value in &self.values {
            let Some(other) = thinking_level_rank(value) else {
                continue;
            };
            let distance = other.abs_diff(rank);
            let better = match best {
                None => true,
                Some((best_distance, best_rank, _)) => {
                    distance < best_distance || (distance == best_distance && other < best_rank)
                }
            };
            if better {
                best = Some((distance, other, value));
            }
        }
        best.map(|(_, _, value)| value.to_string())
    }
}

/// The integer a level travels as, spreading our seven-point scale evenly over
/// the provider's range: `off` sits at `min`, `max` at `max`, and `medium` — the
/// middle of the scale — at the middle of the range (`50` for `1..=100`).
fn integer_for_rank(rank: usize, min: i64, max: i64) -> i64 {
    let steps = (KNOWN_THINKING_LEVELS.len() - 1) as i64;
    min + (max - min) * rank as i64 / steps
}

/// Read `compat.reasoningEffort` off a model.
pub fn parse_reasoning_effort_vocabulary(
    model: &crate::models::Model,
) -> Result<Option<ReasoningEffortVocabulary>> {
    match model.compat.get("reasoningEffort") {
        None | Some(Value::Null) => Ok(None),
        Some(value) => {
            ReasoningEffortVocabulary::parse(&model.provider, &model.id, value).map(Some)
        }
    }
}

/// The value a level travels as, honoring the model's declared vocabulary.
///
/// Without a vocabulary the level is the wire value, verbatim.
pub fn effort_wire_value(
    level: &str,
    vocabulary: Option<&ReasoningEffortVocabulary>,
) -> Option<Value> {
    match vocabulary {
        Some(vocabulary) => vocabulary.resolve(level),
        None => effort_value(level).map(|value| Value::String(value.to_string())),
    }
}

/// Write a level's wire value into a chat-completions request body, at the
/// path the model's vocabulary declares (top-level `reasoning_effort` unless
/// `compat.reasoningEffort.param` says otherwise).
pub fn apply_effort_param(
    body: &mut Value,
    level: &str,
    vocabulary: Option<&ReasoningEffortVocabulary>,
) {
    let Some(value) = effort_wire_value(level, vocabulary) else {
        return;
    };
    let param = vocabulary
        .map(|vocabulary| vocabulary.param())
        .unwrap_or(ReasoningEffortVocabulary::DEFAULT_PARAM);
    insert_dotted(body, param, value);
}

/// Insert `value` at a dotted path, merging into an existing object when one of
/// the parent segments already is one (`chat_template_kwargs` is also written by
/// the Qwen thinking format).
fn insert_dotted(body: &mut Value, path: &str, value: Value) {
    if !body.is_object() {
        *body = Value::Object(serde_json::Map::new());
    }
    let (head, rest) = match path.split_once('.') {
        Some((head, rest)) => (head, Some(rest)),
        None => (path, None),
    };
    let map = body
        .as_object_mut()
        .expect("insert_dotted made the body an object");
    match rest {
        None => {
            map.insert(head.to_string(), value);
        }
        Some(rest) => {
            let slot = map
                .entry(head.to_string())
                .or_insert_with(|| Value::Object(serde_json::Map::new()));
            insert_dotted(slot, rest, value);
        }
    }
}

/// Report every declared level a model's vocabulary cannot express, once per
/// `provider/model/level` and process.
///
/// It runs where a model is resolved for a request — which is also where a
/// switch or a reload lands, so a bad hand-written vocabulary is named before a
/// run pays for the provider's 400. The dedupe keeps a per-request resolution
/// from turning one broken entry into a log flood.
pub fn report_unexpressible_levels(
    model: &crate::models::Model,
    vocabulary: &ReasoningEffortVocabulary,
) {
    use std::collections::HashSet;
    use std::sync::{Mutex, OnceLock};

    static REPORTED: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
    for level in &model.thinking_levels {
        if !is_effort_level(level) {
            continue;
        }
        let (wire, advice) = match vocabulary.mapping(level) {
            // A listed spelling, and an integer on the declared axis, are both
            // what the declaration asks for.
            EffortMapping::Exact(_) | EffortMapping::Integer(_) => continue,
            EffortMapping::Nearest(wire) => (
                Some(wire),
                "it sends the closest accepted value; declare only levels the vocabulary \
                 reaches, or add an `integer` range",
            ),
            EffortMapping::None => (
                None,
                "it sends no effort parameter; declare it in `values` or on the product \
                 scale (off/minimal/low/medium/high/xhigh/max)",
            ),
        };
        let key = format!("{}/{}/{}", model.provider, model.id, level);
        let first = REPORTED
            .get_or_init(|| Mutex::new(HashSet::new()))
            .lock()
            .map(|mut reported| reported.insert(key))
            .unwrap_or(false);
        if !first {
            continue;
        }
        tracing::warn!(
            provider = %model.provider,
            model = %model.id,
            level = %level,
            wire = wire.as_deref().unwrap_or("(none)"),
            accepted = %vocabulary.describe(),
            "[models] thinking level has no exact spelling in compat.reasoningEffort; {advice}"
        );
    }
}

/// True for a level that asks for effort (not the two inert spellings).
fn is_effort_level(level: &str) -> bool {
    !level.trim().is_empty() && level != DEFAULT_THINKING_LEVEL && level != "off"
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
        // A declared level the vocabulary cannot express travels as something
        // else (or not at all); name it here, where a switch, a reload or a
        // first request resolves the model, instead of letting the provider
        // refuse the request later.
        match &protocol {
            ProtocolConfig::OpenAiChat(chat) => {
                if let Some(vocabulary) = &chat.reasoning_effort {
                    report_unexpressible_levels(model, vocabulary);
                }
            }
            ProtocolConfig::OpenAiResponses(responses) => {
                if let Some(vocabulary) = &responses.reasoning_effort {
                    report_unexpressible_levels(model, vocabulary);
                }
            }
            ProtocolConfig::AnthropicMessages(_) => {}
        }
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
        reasoning_effort: parse_reasoning_effort_vocabulary(model)?,
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
        reasoning_effort: parse_reasoning_effort_vocabulary(model)?,
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

    fn vocab(value: Value) -> ReasoningEffortVocabulary {
        ReasoningEffortVocabulary::parse("p", "m", &value).unwrap()
    }

    /// The westlakehpc DeepSeek V4 declaration: the spellings it accepts, plus
    /// the integer range it accepts *only* under `chat_template_kwargs`.
    fn deepseek_v4_vocab() -> ReasoningEffortVocabulary {
        vocab(json!({
            "values": ["low", "high", "xhigh", "max"],
            "integer": {"min": 1, "max": 100},
            "param": "chat_template_kwargs.reasoning_effort"
        }))
    }

    #[test]
    fn vocabulary_maps_listed_levels_verbatim_and_the_gap_to_an_integer() {
        let vocabulary = deepseek_v4_vocab();
        assert_eq!(vocabulary.resolve("low"), Some(json!("low")));
        assert_eq!(vocabulary.resolve("high"), Some(json!("high")));
        assert_eq!(vocabulary.resolve("xhigh"), Some(json!("xhigh")));
        assert_eq!(vocabulary.resolve("max"), Some(json!("max")));
        // `medium` sits between `low` and `high`, so it takes the integer axis
        // at the middle of 1..=100 rather than inventing a spelling.
        assert_eq!(vocabulary.resolve("medium"), Some(json!(50)));
        // `default` sends nothing; `off` is the protocol toggle's business.
        assert_eq!(vocabulary.resolve("default"), None);
        assert_eq!(vocabulary.resolve("off"), None);
        // `minimal` sits below every accepted spelling; with a declared integer
        // axis it takes the point below `low` (17), which is monotone with it.
        assert_eq!(vocabulary.resolve("minimal"), Some(json!(17)));
    }

    #[test]
    fn vocabulary_mapping_is_monotone_across_the_whole_scale() {
        // Depth of one wire value on the vocabulary's own axis: a listed
        // spelling sits where our scale says it does (or, with an integer range
        // declared, on the same numeric axis the integers live on), and an
        // integer is itself.
        fn position(vocabulary: &ReasoningEffortVocabulary, mapping: EffortMapping) -> Option<i64> {
            match mapping {
                EffortMapping::None => None,
                EffortMapping::Integer(value) => Some(value),
                EffortMapping::Exact(name) | EffortMapping::Nearest(name) => {
                    let rank = thinking_level_rank(&name)? as i64;
                    Some(match vocabulary.integer() {
                        Some((min, max)) => integer_for_rank(rank as usize, min, max),
                        None => rank,
                    })
                }
            }
        }

        for vocabulary in [
            deepseek_v4_vocab(),
            vocab(json!({"values": ["minimal", "low", "medium", "high"]})),
            vocab(json!({"values": ["low", "high"], "integer": {"min": 1, "max": 10}})),
            vocab(json!({"integer": {"min": 0, "max": 7}})),
        ] {
            let mut previous: Option<i64> = None;
            for level in ["off", "minimal", "low", "medium", "high", "xhigh", "max"] {
                let Some(position) = position(&vocabulary, vocabulary.mapping(level)) else {
                    continue;
                };
                if let Some(previous) = previous {
                    assert!(
                        position >= previous,
                        "{} maps `{level}` below the previous level ({position} < {previous})",
                        vocabulary.describe()
                    );
                }
                previous = Some(position);
            }
        }
    }

    #[test]
    fn vocabulary_parse_rejects_a_declaration_it_cannot_honor() {
        let error = |value: Value| {
            ReasoningEffortVocabulary::parse("p", "m", &value)
                .unwrap_err()
                .to_string()
        };
        // A typo must not silently narrow the vocabulary: the provider's 400
        // would then be back on the wire.
        assert!(error(json!({"value": ["low"]})).contains("unknown key `value`"));
        assert!(error(json!({"values": ["low"], "integers": {"min": 1}})).contains("unknown key"));
        assert!(error(json!({"values": []})).contains("neither values nor integer"));
        assert!(error(json!({})).contains("neither values nor integer"));
        assert!(error(json!({"values": "low"})).contains("must be an array of strings"));
        assert!(error(json!({"values": [""]})).contains("invalid value"));
        assert!(
            error(json!({"values": ["low"], "integer": {"min": 100, "max": 1}}))
                .contains("lower than integer.max")
        );
        assert!(
            error(json!({"values": ["low"], "integer": {"min": "1", "max": 2}}))
                .contains("must be an integer")
        );
        assert!(
            error(json!({"values": ["low"], "integer": {"min": -5_000_000, "max": 5}}))
                .contains("out of range")
        );
        assert!(error(json!({"values": ["low"], "param": "chat template"})).contains("dotted path"));
        assert!(error(json!({"values": ["low"], "param": "a..b"})).contains("dotted path"));
        assert!(error(json!("low")).contains("must be an object"));
    }

    #[test]
    fn vocabulary_parse_defaults_the_param_and_reports_what_it_accepts() {
        let vocabulary = vocab(json!({"values": ["low", "high"]}));
        assert_eq!(vocabulary.param(), "reasoning_effort");
        assert_eq!(vocabulary.describe(), "low or high");

        let both = deepseek_v4_vocab();
        assert_eq!(
            both.describe(),
            "low, high, xhigh, max or an integer 1..=100"
        );

        // `integer` alone is a complete declaration.
        let numbers_only = vocab(json!({"integer": {"min": 0, "max": 7}}));
        assert_eq!(numbers_only.describe(), "an integer 0..=7");
        assert_eq!(numbers_only.resolve("max"), Some(json!(7)));
    }

    #[test]
    fn an_unlisted_level_uses_the_integer_axis_when_one_is_declared() {
        // `medium`, `xhigh` and `minimal` are all off the declared spellings, so
        // each takes the integer axis at its own point of the product scale.
        let vocabulary =
            vocab(json!({"values": ["low", "high"], "integer": {"min": 1, "max": 100}}));
        assert_eq!(vocabulary.mapping("medium"), EffortMapping::Integer(50));
        assert_eq!(vocabulary.mapping("xhigh"), EffortMapping::Integer(83));
        assert_eq!(vocabulary.mapping("minimal"), EffortMapping::Integer(17));

        // Without an integer range an unlisted level snaps to the nearest
        // spelling, ties going shallower.
        let names_only = vocab(json!({"values": ["low", "high"]}));
        assert_eq!(
            names_only.mapping("xhigh"),
            EffortMapping::Nearest("high".into())
        );
        assert_eq!(
            names_only.mapping("minimal"),
            EffortMapping::Nearest("low".into())
        );

        // A declaration that lists everything keeps verbatim behaviour.
        let verbatim = vocab(json!({"values": ["low", "medium", "high"]}));
        assert_eq!(
            verbatim.mapping("medium"),
            EffortMapping::Exact("medium".into())
        );

        // A level off our scale has no exact spelling and no position: nothing
        // is sent (the provider default), and the model reports it.
        assert_eq!(deepseek_v4_vocab().mapping("turbo"), EffortMapping::None);
    }

    #[test]
    fn effort_param_writes_at_the_declared_path_and_merges_objects() {
        let vocabulary = deepseek_v4_vocab();
        let mut body = json!({"model": "deepseek-flash"});
        apply_effort_param(&mut body, "medium", Some(&vocabulary));
        assert_eq!(body["chat_template_kwargs"]["reasoning_effort"], json!(50));

        // The Qwen chat-template format writes the same object; the effort must
        // join it rather than replace it.
        let mut body = json!({"chat_template_kwargs": {"enable_thinking": true}});
        apply_effort_param(&mut body, "max", Some(&vocabulary));
        assert_eq!(
            body,
            json!({"chat_template_kwargs": {
                "enable_thinking": true,
                "reasoning_effort": "max"
            }})
        );

        // No vocabulary: the historical top-level verbatim write.
        let mut body = json!({});
        apply_effort_param(&mut body, "medium", None);
        assert_eq!(body, json!({"reasoning_effort": "medium"}));

        // `default` writes nothing at all.
        let mut body = json!({});
        apply_effort_param(&mut body, DEFAULT_THINKING_LEVEL, Some(&vocabulary));
        assert_eq!(body, json!({}));
    }

    #[test]
    fn vocabulary_is_read_from_compat_and_reaches_the_chat_config() {
        let mut model = crate::models::Model {
            id: "deepseek-flash".into(),
            provider: "westlakehpc".into(),
            api: "openai-completions".into(),
            ..Default::default()
        };
        model.compat.insert(
            "reasoningEffort".into(),
            json!({"values": ["low", "high"], "integer": {"min": 1, "max": 100}}),
        );
        let target = ResolvedModelTarget::from_model(&model, "key".into(), None, None).unwrap();
        let chat = expect_chat(target.protocol);
        let vocabulary = chat.reasoning_effort.expect("declared vocabulary");
        assert_eq!(vocabulary.resolve("medium"), Some(json!(50)));

        // Absent or null means "the level is the wire value".
        model.compat.remove("reasoningEffort");
        let target = ResolvedModelTarget::from_model(&model, "key".into(), None, None).unwrap();
        assert!(expect_chat(target.protocol).reasoning_effort.is_none());

        // A malformed declaration fails model resolution, so the selection that
        // reproduces it names the model and the problem.
        model
            .compat
            .insert("reasoningEffort".into(), json!({"values": []}));
        let error = ResolvedModelTarget::from_model(&model, "key".into(), None, None).unwrap_err();
        assert!(error.to_string().contains("compat.reasoningEffort"));
    }

    #[test]
    fn normalize_thinking_levels_drops_overlong_names_and_caps_the_list() {
        // A name longer than MAX_THINKING_LEVEL_LEN is dropped rather than
        // shortened: a truncated spelling would not match anything the provider
        // accepts, so guessing is worse than leaving it out.
        let overlong = "x".repeat(MAX_THINKING_LEVEL_LEN + 1);
        let declared = vec![
            " high ".to_string(),
            overlong,
            "low".to_string(),
            "low".to_string(),
            String::new(),
            DEFAULT_THINKING_LEVEL.to_string(),
        ];
        assert_eq!(
            normalize_thinking_levels("p", "m", declared),
            vec!["low".to_string(), "high".to_string()]
        );

        // Past MAX_THINKING_LEVELS the tail is dropped: a picker with more than
        // that many entries is already unusable, and the provider's own list is
        // the authority for the rest.
        let declared: Vec<String> = (0..MAX_THINKING_LEVELS + 4)
            .map(|index| format!("wire-{index}"))
            .collect();
        let normalized = normalize_thinking_levels("p", "m", declared);
        assert_eq!(normalized.len(), MAX_THINKING_LEVELS);
        assert_eq!(normalized.first().map(String::as_str), Some("wire-0"));
        assert_eq!(
            normalized.last(),
            Some(&format!("wire-{}", MAX_THINKING_LEVELS - 1))
        );
    }

    #[test]
    fn normalize_thinking_levels_sorts_known_levels_ahead_of_wire_values() {
        // The picker shows this list untouched, so the known scale must come
        // first in its fixed order and unknown wire values keep the order the
        // configuration declared them in.
        let declared = vec![
            "turbo".to_string(),
            "high".to_string(),
            "minimal".to_string(),
            "zap".to_string(),
        ];
        assert_eq!(
            normalize_thinking_levels("p", "m", declared),
            vec![
                "minimal".to_string(),
                "high".to_string(),
                "turbo".to_string(),
                "zap".to_string(),
            ]
        );
    }

    #[test]
    fn vocabulary_parse_rejects_each_malformed_field_shape() {
        let error = |value: Value| {
            ReasoningEffortVocabulary::parse("p", "m", &value)
                .unwrap_err()
                .to_string()
        };
        // A non-string entry inside `values` (a number, not the whole field).
        assert!(error(json!({"values": ["low", 2]})).contains("must be an array of strings"));
        // `integer` accepts only min/max, and a non-object is refused with the
        // shape it wanted rather than defaulted.
        assert!(error(json!({"values": ["low"], "integer": {"step": 5}}))
            .contains("integer has unknown key `step`"));
        assert!(
            error(json!({"values": ["low"], "integer": 5})).contains("must be an object such as")
        );
        // A non-string `param` would silently write at some path the provider
        // never reads, so it is an error rather than a fallback.
        assert!(error(json!({"values": ["low"], "param": 5})).contains("param must be a string"));
    }

    #[test]
    fn vocabulary_lists_and_accepts_exactly_what_it_declared() {
        let vocabulary = deepseek_v4_vocab();
        assert_eq!(vocabulary.values(), ["low", "high", "xhigh", "max"]);
        // Listed spellings, and integers inside the declared range, are accepted.
        assert!(vocabulary.accepts(&json!("high")));
        assert!(!vocabulary.accepts(&json!("medium")));
        assert!(vocabulary.accepts(&json!(1)));
        assert!(vocabulary.accepts(&json!(100)));
        assert!(!vocabulary.accepts(&json!(0)));
        assert!(!vocabulary.accepts(&json!(101)));
        // A fractional number has no integer spelling on this wire.
        assert!(!vocabulary.accepts(&json!(1.5)));
        // Neither does a boolean or null.
        assert!(!vocabulary.accepts(&json!(true)));
        assert!(!vocabulary.accepts(&json!(null)));

        // Without a declared integer range a number is never accepted: the
        // absence of a range must not be read as an unbounded one.
        let spellings_only = vocab(json!({"values": ["low"]}));
        assert!(!spellings_only.accepts(&json!(1)));
    }

    #[test]
    fn effort_value_is_none_for_the_inert_spellings() {
        assert_eq!(effort_value(""), None);
        assert_eq!(effort_value("   "), None);
        assert_eq!(effort_value(DEFAULT_THINKING_LEVEL), None);
        assert_eq!(effort_value(" off "), None);
        assert_eq!(effort_value(" high "), Some("high"));
    }

    #[test]
    fn applying_an_effort_to_a_non_object_body_starts_a_fresh_object() {
        // Protocol builders hand in whatever the caller's body is; writing the
        // effort must replace a shape it cannot extend instead of panicking.
        let mut body = json!(["not", "an", "object"]);
        apply_effort_param(&mut body, "high", Some(&deepseek_v4_vocab()));
        assert_eq!(
            body,
            json!({"chat_template_kwargs": {"reasoning_effort": "high"}})
        );
    }

    #[test]
    fn nearest_ignores_listed_values_outside_the_product_scale() {
        // A hand-written wire value the product does not name must not win the
        // distance comparison: `low` still picks the closest known spelling.
        let vocabulary = vocab(json!({"values": ["turbo", "high"]}));
        assert_eq!(
            vocabulary.mapping("low"),
            EffortMapping::Nearest("high".into())
        );
    }

    #[test]
    fn report_names_a_level_no_vocabulary_spelling_reaches() {
        // An unknown configured level (a typo, or a spelling from another
        // provider) has no rank on the product scale, so there is no nearest
        // spelling: the report must take the "sends nothing" arm instead of
        // claiming a wire value the provider never accepts.
        let mut model = crate::models::Model {
            id: "report-none".into(),
            provider: "report".into(),
            ..Default::default()
        };
        model.thinking_levels = vec!["turbo".into()];
        let vocabulary = vocab(json!({"values": ["low"]}));
        assert_eq!(vocabulary.mapping("turbo"), EffortMapping::None);
        // Run the report with an enabled subscriber, so the event it raises is
        // actually constructed rather than dropped by the callsite guard. It is
        // idempotent per provider/model/level: the second resolution of the same
        // model must not log again.
        let _subscriber = tracing::subscriber::set_default(
            tracing_subscriber::fmt()
                .with_writer(std::io::sink)
                .with_max_level(tracing::Level::TRACE)
                .finish(),
        );
        report_unexpressible_levels(&model, &vocabulary);
        report_unexpressible_levels(&model, &vocabulary);
    }

    #[test]
    fn chat_config_reads_the_tool_stream_flag_and_rejects_a_non_boolean() {
        let mut model = crate::models::Model {
            id: "m".into(),
            provider: "p".into(),
            api: "openai-completions".into(),
            ..Default::default()
        };
        model.compat.insert("toolStream".into(), Value::Bool(true));
        let target = ResolvedModelTarget::from_model(&model, "key".into(), None, None).unwrap();
        assert!(expect_chat(target.protocol).tool_stream);

        model
            .compat
            .insert("toolStream".into(), Value::String("yes".into()));
        let error = ResolvedModelTarget::from_model(&model, "key".into(), None, None).unwrap_err();
        assert!(error.to_string().contains("toolStream must be a boolean"));
    }
}
