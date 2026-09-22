//! `/usage` — the session usage panel (context window, tokens, cost, quota).
//!
//! Two pure pieces, so the panel can be driven from whatever the caller has:
//!
//! * [`usage_from_state`] parses a `get_state` payload (and, for the fields it
//!   shares, a `get_session_stats` one) into a [`UsageView`]. It is written to
//!   *survive* an agent it does not know: every field is looked up under its
//!   camelCase **and** snake_case name, wrong types are ignored rather than
//!   defaulted-in-place-of-good-data, and a payload that is not an object, is
//!   `null`, or carries none of the recognised keys yields
//!   `available: false` instead of a screen of zeros. It never panics.
//! * [`render_usage`] turns that view into the panel rows — context bar, the
//!   per-category token/cost table, the per-model split, the session facts and
//!   the threshold warnings. Every row is exactly `width` visible columns wide
//!   for any width (including `0`), and the panel degrades by *dropping*
//!   columns (cost, then the bar) rather than by wrapping mid-row.
//!
//! Token accounting follows the agent, not a client-side guess: `input` is the
//! prompt size *including* the cached subset, so the table bills
//! `input − cache_read − cache_write` as plain input (row `Input (uncached)`)
//! and lists the two cache categories separately — the rows then add up to
//! `input + output`, which is the "Total" row. Cost is the agent's
//! authoritative `costCny`; the per-category figures are its estimates and are
//! shown as such (an unpriced model reports zeros and the panel says so instead
//! of printing `¥0.00`).
//!
//! **Two quantities that look alike are not the same thing**, so the panel
//! labels them apart: the `Context` row is the *bounded* occupancy of the
//! current window (the last request's prompt against the model's window, which
//! is what the compaction thresholds measure), while every counter under the
//! `Cumulative tokens (resent each call)` heading is a *session total* that
//! grows without bound because each request resends the whole prompt. The
//! `Avg input/query` row bridges the two: it is the cumulative input divided by
//! the agent's query count, i.e. the size a turn actually resends.

use serde_json::{Map, Value};

use crate::theme::{bold, fg, Chrome, Theme};
use crate::tui::Component;
use crate::utils::{truncate_to_width, visible_width, TruncateOptions};

// ─── Thresholds ────────────────────────────────────────────────────────────

/// Context occupancy that deserves a warning.
pub const CONTEXT_WARN_PERCENT: f64 = 80.0;
/// Context occupancy that means the next turn is at risk.
pub const CONTEXT_CRITICAL_PERCENT: f64 = 95.0;
/// Queue depth that deserves a warning (`1` is only reported, never warned).
pub const QUEUE_WARN_COUNT: i64 = 2;
/// Quota consumption that deserves a warning.
pub const QUOTA_WARN_PERCENT: f64 = 80.0;
/// Quota consumption that means the next request may be rejected.
pub const QUOTA_CRITICAL_PERCENT: f64 = 95.0;
/// Per-model rows the panel prints before collapsing the rest into a count.
pub const MAX_MODEL_ROWS: usize = 8;
/// Longest label column in the fixed token table: wide enough for
/// `Input (uncached)` and `Avg input/query` to read in full at ordinary widths.
const CATEGORY_LABEL_CAP: usize = 20;
/// Width at or above which the token rows also carry their cost column.
const COST_COLUMN_MIN_WIDTH: usize = 46;
/// Bar cells drawn at the widest layout.
const MAX_BAR_CELLS: usize = 28;
/// Width below which the context bar is replaced by its numbers.
const BAR_MIN_WIDTH: usize = 24;
/// Longest model column in the per-model section.
const MODEL_LABEL_CAP: usize = 40;

// ─── Value helpers (tolerant parsing) ──────────────────────────────────────

/// First present value among `keys` (camelCase first, snake_case second).
fn lookup<'a>(map: &'a Map<String, Value>, keys: &[&str]) -> Option<&'a Value> {
    keys.iter().find_map(|key| map.get(*key))
}

/// A finite number, also accepting a numeric string (`"12"`, `"3.5"`).
fn number(value: Option<&Value>) -> Option<f64> {
    let raw = match value? {
        Value::Number(number) => number.as_f64()?,
        Value::String(text) => text.trim().parse::<f64>().ok()?,
        _ => return None,
    };
    raw.is_finite().then_some(raw)
}

/// A whole number, truncating an integral float (`200000.0`).
fn integer(value: Option<&Value>) -> Option<i64> {
    let raw = number(value)?;
    if raw.abs() > i64::MAX as f64 {
        return None;
    }
    Some(raw as i64)
}

/// A non-negative whole number (token counts and counts of things).
fn count(value: Option<&Value>) -> Option<i64> {
    integer(value).filter(|number| *number >= 0)
}

/// A meaningful non-empty string.
fn text(value: Option<&Value>) -> Option<String> {
    let raw = value?.as_str()?.trim();
    if raw.is_empty() {
        None
    } else {
        Some(raw.to_string())
    }
}

/// A boolean, ignoring anything else.
fn boolean(value: Option<&Value>) -> Option<bool> {
    value?.as_bool()
}

fn object(value: Option<&Value>) -> Option<&Map<String, Value>> {
    value?.as_object()
}

fn array(value: Option<&Value>) -> Option<&Vec<Value>> {
    value?.as_array()
}

// ─── Formatting ────────────────────────────────────────────────────────────

/// Compact token count: `999`, `1.2K`, `12.3M`, `1.2B`, `3.4T`.
///
/// The scaled value is rounded *before* the unit is chosen, so nothing ever
/// renders as `1000.0K` — it promotes to `1M` instead.
pub fn format_tokens(tokens: i64) -> String {
    let negative = tokens < 0;
    let value = (tokens as f64).abs();
    let units = ["", "K", "M", "B", "T"];
    let mut unit = 0usize;
    let mut scaled = value;
    while scaled >= 1000.0 && unit + 1 < units.len() {
        scaled /= 1000.0;
        unit += 1;
    }
    if unit > 0 {
        // Round before choosing the unit, so a value just under the next
        // power of ten promotes (999_999 → 1M, never 1000K).
        let rounded = (scaled * 10.0).round() / 10.0;
        if rounded >= 1000.0 && unit + 1 < units.len() {
            unit += 1;
            scaled = rounded / 1000.0;
        } else {
            scaled = rounded;
        }
    }
    let body = if unit == 0 {
        format!("{scaled:.0}")
    } else {
        trimmed(scaled)
    };
    format!("{}{body}{}", if negative { "-" } else { "" }, units[unit])
}

/// One decimal, dropping a trailing `.0` (`1.2`, `2`, `1000`).
fn trimmed(value: f64) -> String {
    let rounded = (value * 10.0).round() / 10.0;
    if (rounded.fract()).abs() < f64::EPSILON {
        format!("{:.0}", rounded)
    } else {
        format!("{:.1}", rounded)
    }
}

/// RMB amount with two decimals (`¥0.00`, `-¥12.35`). Sub-cent amounts read
/// `<¥0.01` (a `¥0.00` there would claim the request was free), and a
/// non-finite value renders `—`.
pub fn format_cny(amount: f64) -> String {
    if !amount.is_finite() {
        return "—".to_string();
    }
    if amount == 0.0 {
        return "¥0.00".to_string();
    }
    if amount.abs() < 0.005 {
        return if amount < 0.0 {
            ">-¥0.01".to_string()
        } else {
            "<¥0.01".to_string()
        };
    }
    format!("¥{:.2}", amount)
}

/// Percentage with one decimal (`0.0%`, `62.5%`), clamped at zero.
pub fn format_percent(percent: f64) -> String {
    if !percent.is_finite() {
        return "0.0%".to_string();
    }
    format!("{:.1}%", percent.max(0.0))
}

/// A context-occupancy bar of `cells` characters, clamped to `0..=100%`.
pub fn context_bar(percent: f64, cells: usize) -> String {
    if cells == 0 {
        return String::new();
    }
    let ratio = if percent.is_finite() {
        (percent / 100.0).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let filled = ((ratio * cells as f64).round() as usize).min(cells);
    format!("{}{}", "█".repeat(filled), "░".repeat(cells - filled))
}

// ─── Data model ────────────────────────────────────────────────────────────

/// Token counters, exactly as the agent reports them: `input` includes the
/// cached subset.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct UsageTokens {
    pub input: i64,
    pub output: i64,
    pub cache_read: i64,
    pub cache_write: i64,
}

impl UsageTokens {
    /// `input + output` — the total the panel's rows add up to.
    pub fn total(&self) -> i64 {
        self.input.saturating_add(self.output)
    }

    /// Input tokens that were *not* served from cache (never negative, even if
    /// a provider reports a cache read larger than the prompt).
    pub fn uncached_input(&self) -> i64 {
        self.input
            .saturating_sub(self.cache_read)
            .saturating_sub(self.cache_write)
            .max(0)
    }

    /// Cache tokens (read + write).
    pub fn cache_total(&self) -> i64 {
        self.cache_read.saturating_add(self.cache_write)
    }

    /// `true` when every counter is zero (a session that has not run yet).
    pub fn is_empty(&self) -> bool {
        self.total() == 0 && self.cache_total() == 0
    }
}

/// Session cost in RMB: the agent's authoritative total plus its per-category
/// estimates (which are zero for a model with no prices on file).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct UsageCost {
    pub total_cny: f64,
    pub input_cny: f64,
    pub output_cny: f64,
    pub cache_read_cny: f64,
    pub cache_write_cny: f64,
}

impl UsageCost {
    /// Sum of the per-category estimates (not necessarily the total: a
    /// provider that bills itself reports one figure with no breakdown).
    pub fn breakdown_total(&self) -> f64 {
        self.input_cny + self.output_cny + self.cache_read_cny + self.cache_write_cny
    }

    /// Whether the model has prices on file — i.e. whether a cost column is
    /// meaningful.
    pub fn is_priced(&self) -> bool {
        self.breakdown_total() > 0.0 || self.total_cny > 0.0
    }
}

/// One row of the per-model split.
#[derive(Debug, Clone, PartialEq)]
pub struct ModelUsage {
    pub model: String,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cost_cny: f64,
}

impl ModelUsage {
    /// `input + output`.
    pub fn total_tokens(&self) -> i64 {
        self.input_tokens.saturating_add(self.output_tokens)
    }
}

/// What a quota counts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuotaUnit {
    Cny,
    Tokens,
    Requests,
}

impl QuotaUnit {
    /// Format `amount` in this unit (`¥12.34`, `12.3K`, `12`).
    pub fn format(self, amount: f64) -> String {
        match self {
            QuotaUnit::Cny => format_cny(amount),
            QuotaUnit::Tokens => format_tokens(amount.round() as i64),
            QuotaUnit::Requests => format!("{:.0}", amount),
        }
    }

    /// Parse the payload's `unit` string; unknown/absent means money, which is
    /// what every current quota is.
    pub fn parse(raw: Option<&str>) -> Self {
        match raw.map(str::trim).map(str::to_ascii_lowercase).as_deref() {
            Some("tokens") => QuotaUnit::Tokens,
            Some("requests" | "calls") => QuotaUnit::Requests,
            _ => QuotaUnit::Cny,
        }
    }
}

/// A spending/usage quota for the current period.
#[derive(Debug, Clone, PartialEq)]
pub struct UsageQuota {
    pub used: f64,
    pub limit: f64,
    pub unit: QuotaUnit,
    pub period: Option<String>,
    pub resets_at: Option<String>,
}

impl UsageQuota {
    /// Consumption in percent; `0.0` for a missing/zero limit.
    pub fn percent(&self) -> f64 {
        if self.limit <= 0.0 {
            return 0.0;
        }
        (self.used / self.limit) * 100.0
    }

    /// `true` when the quota is spent (the next request may be rejected).
    pub fn is_exhausted(&self) -> bool {
        self.limit > 0.0 && self.used >= self.limit
    }
}

/// The parsed `/usage` panel state. `available == false` means the payload was
/// not a `get_state`-shaped object at all — the panel then says so instead of
/// showing a wall of zeros.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct UsageView {
    pub available: bool,
    pub model: String,
    pub session_id: Option<String>,
    pub session_name: Option<String>,
    pub context_tokens: i64,
    pub context_window: i64,
    pub context_percent: f64,
    pub image_support: bool,
    pub query_count: i64,
    pub queued_count: i64,
    pub is_streaming: bool,
    pub is_compacting: bool,
    pub thinking_level: String,
    pub permission_level: Option<String>,
    pub tokens: UsageTokens,
    pub cost: UsageCost,
    pub by_model: Vec<ModelUsage>,
    pub quota: Option<UsageQuota>,
}

impl UsageView {
    /// Resolved context occupancy: the reported percentage when the payload
    /// carried one, else derived from the token counts.
    pub fn percent(&self) -> f64 {
        if self.context_percent > 0.0 {
            return self.context_percent;
        }
        if self.context_window > 0 && self.context_tokens > 0 {
            return (self.context_tokens as f64 / self.context_window as f64) * 100.0;
        }
        0.0
    }

    /// Occupancy at or above [`CONTEXT_WARN_PERCENT`] (and a known window).
    pub fn is_context_warning(&self) -> bool {
        self.context_window > 0 && self.percent() >= CONTEXT_WARN_PERCENT
    }

    /// Occupancy at or above [`CONTEXT_CRITICAL_PERCENT`].
    pub fn is_context_critical(&self) -> bool {
        self.context_window > 0 && self.percent() >= CONTEXT_CRITICAL_PERCENT
    }

    /// `true` when runs are waiting for the current one.
    pub fn has_queue(&self) -> bool {
        self.queued_count > 0
    }

    /// Cumulative input per query — the size a turn actually resends, and so
    /// the figure that is comparable to the context occupancy.
    ///
    /// The denominator is the agent's `queryCount`, which counts *user
    /// messages* (prompts and follow-ups), not API calls: an agentic turn may
    /// call the model several times, and `get_state` carries no call counter
    /// (`get_session_stats` counts user messages too). The row is therefore
    /// labelled `query`, not `call` — the average is per turn and can exceed
    /// one turn's prompt. The numerator is the *whole* input including the
    /// cached subset, because the full prompt is what each turn re-reads.
    ///
    /// `None` when nothing has been asked yet, so the panel drops the row
    /// instead of dividing by zero.
    pub fn avg_input_per_query(&self) -> Option<i64> {
        (self.query_count > 0).then(|| self.tokens.input / self.query_count)
    }

    /// Human-readable attention lines: `!!` critical, `!` warning, `·` info.
    /// Empty for a healthy, fully-priced, idle session.
    pub fn warnings(&self) -> Vec<String> {
        let mut lines = Vec::new();
        if self.is_context_critical() {
            lines.push(format!(
                "!! context {} full — compaction is due",
                format_percent(self.percent())
            ));
        } else if self.is_context_warning() {
            lines.push(format!("! context {} used", format_percent(self.percent())));
        }
        if self.is_compacting {
            lines.push("· compacting the context".to_string());
        }
        if self.queued_count > 0 {
            let plural = if self.queued_count == 1 {
                "run"
            } else {
                "runs"
            };
            let prefix = if self.queued_count >= QUEUE_WARN_COUNT {
                "!"
            } else {
                "·"
            };
            lines.push(format!("{prefix} {} {plural} queued", self.queued_count));
        }
        if let Some(quota) = &self.quota {
            let percent = quota.percent();
            if quota.is_exhausted() {
                lines.push(format!(
                    "!! quota exhausted ({}/{})",
                    quota.unit.format(quota.used),
                    quota.unit.format(quota.limit)
                ));
            } else if percent >= QUOTA_CRITICAL_PERCENT {
                lines.push(format!(
                    "!! quota {} used ({}/{})",
                    format_percent(percent),
                    quota.unit.format(quota.used),
                    quota.unit.format(quota.limit)
                ));
            } else if percent >= QUOTA_WARN_PERCENT {
                lines.push(format!(
                    "! quota {} used ({}/{})",
                    format_percent(percent),
                    quota.unit.format(quota.used),
                    quota.unit.format(quota.limit)
                ));
            }
        }
        if self.available && !self.cost.is_priced() && !self.tokens.is_empty() {
            lines.push("· no prices on file — tokens only".to_string());
        }
        lines
    }
}

// ─── Parsing ───────────────────────────────────────────────────────────────

/// Parse a `get_state` payload into a [`UsageView`].
///
/// Tolerates a `get_session_stats` payload as well (`tokens` +
/// `cost` at the top level), missing fields, `null`s, wrong types and unknown
/// keys. A non-object payload — or one whose recognised fields are all unusable
/// — returns `available: false`.
pub fn usage_from_state(state: &Value) -> UsageView {
    let mut view = UsageView::default();
    let Some(map) = state.as_object() else {
        return view;
    };

    let usage = object(lookup(map, &["usage"]));
    let stats_tokens = object(lookup(map, &["tokens"]));
    let mut recognized = false;

    // ── usage / tokens objects ──
    if let Some(usage) = usage {
        recognized = true;
        view.tokens = UsageTokens {
            input: count(lookup(usage, &["inputTokens", "input_tokens", "input"])).unwrap_or(0),
            output: count(lookup(usage, &["outputTokens", "output_tokens", "output"])).unwrap_or(0),
            cache_read: count(lookup(
                usage,
                &["cacheReadTokens", "cache_read_tokens", "cacheRead"],
            ))
            .unwrap_or(0),
            cache_write: count(lookup(
                usage,
                &["cacheWriteTokens", "cache_write_tokens", "cacheWrite"],
            ))
            .unwrap_or(0),
        };
        view.cost = UsageCost {
            total_cny: number(lookup(usage, &["costCny", "cost_cny"])).unwrap_or(0.0),
            input_cny: number(lookup(usage, &["costInputCny", "cost_input_cny"])).unwrap_or(0.0),
            output_cny: number(lookup(usage, &["costOutputCny", "cost_output_cny"])).unwrap_or(0.0),
            cache_read_cny: number(lookup(usage, &["costCacheReadCny", "cost_cache_read_cny"]))
                .unwrap_or(0.0),
            cache_write_cny: number(lookup(
                usage,
                &["costCacheWriteCny", "cost_cache_write_cny"],
            ))
            .unwrap_or(0.0),
        };
    } else if let Some(tokens) = stats_tokens {
        // `get_session_stats` shape: {tokens: {...}, cost: 1.23}
        recognized = true;
        view.tokens = UsageTokens {
            input: count(lookup(tokens, &["input"])).unwrap_or(0),
            output: count(lookup(tokens, &["output"])).unwrap_or(0),
            cache_read: count(lookup(tokens, &["cacheRead", "cache_read"])).unwrap_or(0),
            cache_write: count(lookup(tokens, &["cacheWrite", "cache_write"])).unwrap_or(0),
        };
        if let Some(cost) = number(lookup(map, &["cost"])) {
            view.cost.total_cny = cost;
        } else if let Some(cost) = number(lookup(tokens, &["cost"])) {
            view.cost.total_cny = cost;
        }
    }

    // ── identity ──
    view.model = text(lookup(map, &["model"]))
        .or_else(|| usage.and_then(|usage| text(lookup(usage, &["model"]))))
        .unwrap_or_default();
    view.session_id = text(lookup(map, &["sessionId", "session_id"]));
    view.session_name = text(lookup(map, &["sessionName", "session_name"]));
    view.thinking_level =
        text(lookup(map, &["thinkingLevel", "thinking_level"])).unwrap_or_default();
    view.permission_level = text(lookup(map, &["permissionLevel", "permission_level"]));

    // ── context ──
    if let Some(tokens) = count(lookup(
        map,
        &[
            "contextTokens",
            "context_tokens",
            "lastPromptTokens",
            "last_prompt_tokens",
        ],
    )) {
        view.context_tokens = tokens;
        recognized = true;
    } else if usage.is_some() {
        // Older payloads only carried the prompt size.
        if let Some(tokens) = count(lookup(map, &["promptTokens", "prompt_tokens"])) {
            view.context_tokens = tokens;
        }
    }
    if let Some(window) = count(lookup(map, &["contextWindow", "context_window"])).or_else(|| {
        usage.and_then(|usage| count(lookup(usage, &["contextWindow", "context_window"])))
    }) {
        view.context_window = window;
        recognized = true;
    }
    if let Some(percent) = number(lookup(map, &["contextPercent", "context_percent"])) {
        view.context_percent = percent.max(0.0);
        recognized = true;
    }

    // ── session facts ──
    if let Some(query_count) = count(lookup(map, &["queryCount", "query_count"])) {
        view.query_count = query_count;
        recognized = true;
    }
    if let Some(queued) = count(lookup(map, &["queuedCount", "queued_count"])) {
        view.queued_count = queued;
        recognized = true;
    } else if let Some(runs) = array(lookup(map, &["queuedRuns", "queued_runs"])) {
        view.queued_count = runs.len() as i64;
        recognized = true;
    }
    if let Some(streaming) = boolean(lookup(map, &["isStreaming", "is_streaming"])) {
        view.is_streaming = streaming;
        recognized = true;
    }
    if let Some(compacting) = boolean(lookup(map, &["isCompacting", "is_compacting"])) {
        view.is_compacting = compacting;
        recognized = true;
    }
    if let Some(images) = boolean(lookup(
        map,
        &["imageSupport", "image_support", "supportsImages"],
    )) {
        view.image_support = images;
        recognized = true;
    }

    // ── per-model split (absent from today's agent; parsed when present) ──
    let by_model = array(lookup(map, &["byModel", "by_model", "modelBreakdown"]))
        .or_else(|| usage.and_then(|usage| array(lookup(usage, &["byModel", "by_model"]))))
        .or_else(|| array(lookup(map, &["models"])));
    if let Some(entries) = by_model {
        view.by_model = entries.iter().filter_map(model_usage_from_value).collect();
        if !view.by_model.is_empty() {
            recognized = true;
        }
    }

    // ── quota ──
    let quota = object(lookup(map, &["quota", "dailyQuota", "daily_quota"]));
    if let Some(quota) = quota {
        if let Some(limit) = number(lookup(quota, &["limit", "total"])) {
            recognized = true;
            view.quota = Some(UsageQuota {
                used: number(lookup(quota, &["used", "consumed"])).unwrap_or(0.0),
                limit,
                unit: QuotaUnit::parse(lookup(quota, &["unit"]).and_then(Value::as_str)),
                period: text(lookup(quota, &["period"])),
                resets_at: text(lookup(quota, &["resetsAt", "resets_at"])),
            });
        }
    }

    view.available = recognized;
    view
}

fn model_usage_from_value(value: &Value) -> Option<ModelUsage> {
    let map = value.as_object()?;
    let model = text(lookup(map, &["model", "id", "name"]))?;
    let nested = object(lookup(map, &["usage"]));
    let tokens = |keys: &[&str]| -> i64 {
        count(lookup(map, keys))
            .or_else(|| nested.and_then(|usage| count(lookup(usage, keys))))
            .unwrap_or(0)
    };
    Some(ModelUsage {
        model,
        input_tokens: tokens(&["inputTokens", "input_tokens", "input"]),
        output_tokens: tokens(&["outputTokens", "output_tokens", "output"]),
        cost_cny: number(lookup(map, &["costCny", "cost_cny", "cost"]))
            .or_else(|| nested.and_then(|usage| number(lookup(usage, &["costCny", "cost_cny"]))))
            .unwrap_or(0.0),
    })
}

// ─── Rendering ─────────────────────────────────────────────────────────────

/// Pad/truncate a row to exactly `width` visible columns.
fn fit_row(content: &str, width: usize) -> String {
    if width == 0 {
        return String::new();
    }
    let clipped = if visible_width(content) > width {
        truncate_to_width(content, width, &TruncateOptions::default())
    } else {
        content.to_string()
    };
    let padding = width.saturating_sub(visible_width(&clipped));
    if padding == 0 {
        clipped
    } else {
        format!("{clipped}{}", " ".repeat(padding))
    }
}

/// One `label  tokens  ¥cost` row; the cost column is dropped on narrow
/// terminals, and `always_cost` keeps it for the total row (the authoritative
/// figure is the one number that must never be hidden).
fn table_row(
    label: &str,
    tokens: i64,
    cost: Option<&str>,
    width: usize,
    always_cost: bool,
) -> String {
    table_row_capped(label, tokens, cost, width, always_cost, CATEGORY_LABEL_CAP)
}

/// [`table_row`] with an explicit label-column cap: the per-model section
/// needs room for a model id where the fixed category labels do not.
fn table_row_capped(
    label: &str,
    tokens: i64,
    cost: Option<&str>,
    width: usize,
    always_cost: bool,
    label_cap: usize,
) -> String {
    let wide = width >= COST_COLUMN_MIN_WIDTH;
    let show_cost = cost.is_some() && (always_cost || wide);
    // The label column is never narrower than the fixed category labels
    // ("Cache write"), never wider than `label_cap`, and always leaves room for
    // the number column and — when it is shown — the cost column.
    let reserved = if show_cost { 24 } else { 10 };
    let label_width = if wide {
        label_cap.min(width.saturating_sub(reserved)).max(12)
    } else {
        9
    };
    let token_width = if wide { 10 } else { 9 };
    let label = truncate_to_width(label, label_width, &TruncateOptions::default());
    let tokens = format_tokens(tokens);
    let mut row = format!("{label:<label_width$}{tokens:>token_width$}");
    if let Some(cost) = cost.filter(|_| show_cost) {
        let cost_width = visible_width(cost);
        debug_assert!(cost_width <= 14);
        // Right-align inside whatever room is left, capped at the table's
        // column width. When even that is too narrow for the amount, the row
        // keeps the label and the amount and drops the token count — a panel
        // 20 columns wide cannot show three columns, and the amount is the
        // number that must survive.
        let room = width.saturating_sub(label_width + token_width).min(14);
        if room >= cost_width {
            row.push_str(&format!("{cost:>room$}"));
        } else {
            row = format!("{label:<label_width$}{cost}");
        }
    }
    row
}

/// The `/usage` panel at `width`. Empty for `width == 0`; an unavailable view
/// renders a single explanatory row.
pub fn render_usage(usage: &UsageView, width: usize) -> Vec<String> {
    render_usage_with(usage, width, &Chrome::LEGACY)
}

/// [`render_usage`] against an explicit [`Chrome`] palette — the overlay path,
/// where `/theme` has supplied one.
fn render_usage_with(usage: &UsageView, width: usize, chrome: &Chrome) -> Vec<String> {
    if width == 0 {
        return Vec::new();
    }

    let model = if usage.model.is_empty() {
        "unknown model".to_string()
    } else {
        usage.model.clone()
    };
    let mut title = format!("Usage · {}", bold(&model));
    if let Some(name) = &usage.session_name {
        title.push_str(&format!("  {}", fg(chrome.muted, name)));
    }
    if usage.is_streaming {
        title.push_str(&format!("  {}", fg(chrome.info, "streaming")));
    }
    let mut rows = vec![title];

    if !usage.available {
        rows.push(fg(
            chrome.muted,
            "no usage data yet — run a prompt or switch to a live session",
        ));
        return rows.into_iter().map(|row| fit_row(&row, width)).collect();
    }

    // ── context ──
    let percent = usage.percent();
    let percent_text = fg(
        if usage.is_context_critical() {
            chrome.error
        } else if usage.is_context_warning() {
            chrome.warn
        } else {
            chrome.success
        },
        &format_percent(percent),
    );
    let context = if usage.context_window > 0 {
        if width >= BAR_MIN_WIDTH {
            let cells = (width.saturating_sub(24)).clamp(4, MAX_BAR_CELLS);
            format!(
                "Context  {} {percent_text}  {}/{}",
                context_bar(percent, cells),
                format_tokens(usage.context_tokens),
                format_tokens(usage.context_window)
            )
        } else {
            format!(
                "Context  {percent_text}  {}/{}",
                format_tokens(usage.context_tokens),
                format_tokens(usage.context_window)
            )
        }
    } else {
        format!(
            "Context  {} used · window unknown",
            format_tokens(usage.context_tokens)
        )
    };
    rows.push(context);

    // ── token / cost table ──
    // Every counter below is a session *total*: each request resends the whole
    // prompt, so they grow without bound and are not a level that can be read
    // against the context window above. The heading says so, and the average
    // row translates the totals into the per-turn size that *is* comparable.
    rows.push(fg(chrome.muted, "Cumulative tokens (resent each call)"));
    if let Some(per_query) = usage.avg_input_per_query() {
        rows.push(table_row("Avg input/query", per_query, None, width, false));
    }
    let priced = usage.cost.is_priced();
    let cost = |value: f64| -> Option<String> { priced.then(|| format_cny(value)) };
    rows.push(table_row(
        "Input (uncached)",
        usage.tokens.uncached_input(),
        cost(usage.cost.input_cny).as_deref(),
        width,
        false,
    ));
    rows.push(table_row(
        "Output",
        usage.tokens.output,
        cost(usage.cost.output_cny).as_deref(),
        width,
        false,
    ));
    rows.push(table_row(
        "Cache read",
        usage.tokens.cache_read,
        cost(usage.cost.cache_read_cny).as_deref(),
        width,
        false,
    ));
    rows.push(table_row(
        "Cache write",
        usage.tokens.cache_write,
        cost(usage.cost.cache_write_cny).as_deref(),
        width,
        false,
    ));
    // The agent's total is authoritative even when the model has no prices on
    // file, so it is always shown (unlike the per-category estimates).
    rows.push(fg(
        chrome.text,
        &bold(&table_row(
            "Total",
            usage.tokens.total(),
            Some(&format_cny(usage.cost.total_cny)),
            width,
            true,
        )),
    ));

    // ── per-model split ──
    if !usage.by_model.is_empty() {
        rows.push(fg(chrome.heading, &bold("By model")));
        for entry in usage.by_model.iter().take(MAX_MODEL_ROWS) {
            let cost = if entry.cost_cny > 0.0 {
                Some(format_cny(entry.cost_cny))
            } else {
                None
            };
            rows.push(table_row_capped(
                &format!("  {}", entry.model),
                entry.total_tokens(),
                cost.as_deref(),
                width,
                false,
                MODEL_LABEL_CAP,
            ));
        }
        let hidden = usage.by_model.len().saturating_sub(MAX_MODEL_ROWS);
        if hidden > 0 {
            rows.push(fg(chrome.muted, &format!("  … {hidden} more models")));
        }
    }

    // ── session facts ──
    let mut facts = vec![format!(
        "{} {}",
        usage.query_count,
        if usage.query_count == 1 {
            "query"
        } else {
            "queries"
        }
    )];
    if usage.context_window == 0 {
        facts.push("window unknown".to_string());
    }
    if !usage.thinking_level.is_empty() {
        facts.push(format!("thinking {}", usage.thinking_level));
    }
    facts.push(
        if usage.image_support {
            "images on"
        } else {
            "text only"
        }
        .to_string(),
    );
    if let Some(level) = &usage.permission_level {
        facts.push(format!("perms {level}"));
    }
    if usage.queued_count > 0 {
        facts.push(format!("{} queued", usage.queued_count));
    }
    if usage.is_compacting {
        facts.push("compacting".to_string());
    }
    rows.push(format!(
        "{} {}",
        fg(chrome.muted, "Session"),
        facts.join(" · ")
    ));

    // ── warnings ──
    for line in usage.warnings() {
        let color = if line.starts_with("!!") {
            chrome.error
        } else if line.starts_with('!') {
            chrome.warn
        } else {
            chrome.muted
        };
        rows.push(fg(color, &line));
    }

    rows.into_iter().map(|row| fit_row(&row, width)).collect()
}

// ─── Tests ─────────────────────────────────────────────────────────────────

// ─── Overlay adapter ───────────────────────────────────────────────────

/// A [`UsageView`] as a read-only overlay [`Component`] — the `/usage` panel.
///
/// Rendering delegates to [`render_usage`]; the panel consumes no input (the
/// app closes it on `escape`), so an overlay stack entry is enough.
pub struct UsageOverlay {
    view: UsageView,
    theme: Theme,
}

impl UsageOverlay {
    pub fn new(view: UsageView) -> Self {
        Self {
            view,
            theme: Theme::default(),
        }
    }

    /// Adopt a palette (`/theme`); the panel recolors on the next render.
    pub fn set_theme(&mut self, theme: &Theme) {
        self.theme = *theme;
    }

    /// The palette this panel paints with.
    pub fn theme(&self) -> Theme {
        self.theme
    }

    pub fn view(&self) -> &UsageView {
        &self.view
    }
}

impl Component for UsageOverlay {
    fn render(&mut self, width: usize) -> Vec<String> {
        render_usage_with(&self.view, width, &Chrome::from_theme(&self.theme))
    }

    fn invalidate(&mut self) {}

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}

#[cfg(test)]
mod overlay_tests {
    use super::*;
    use crate::tui::Component;

    #[test]
    fn usage_overlay_renders_the_panel_and_ignores_input() {
        let view = usage_from_state(&serde_json::json!({
            "model": "openai/gpt-4o",
            "contextTokens": 64000,
            "contextWindow": 128000,
            "usage": { "inputTokens": 10, "outputTokens": 20, "costCny": 0.5 }
        }));
        let mut overlay = UsageOverlay::new(view);
        assert_eq!(overlay.view().model, "openai/gpt-4o");
        let rows = overlay.render(76);
        assert!(!rows.is_empty());
        assert!(
            rows.iter()
                .any(|row| crate::utils::strip_ansi_codes(row).contains("64K")),
            "{rows:?}"
        );
        // Input is a no-op; the app owns `escape` for this overlay.
        overlay.handle_input("j");
        overlay.invalidate();
        assert!(overlay.as_any().downcast_ref::<UsageOverlay>().is_some());
        assert!(overlay
            .as_any_mut()
            .downcast_mut::<UsageOverlay>()
            .is_some());
    }

    #[test]
    fn usage_overlay_theme_round_trips_and_recolors_the_panel() {
        let view = usage_from_state(&serde_json::json!({
            "model": "openai/gpt-4o",
            "sessionName": "Refactor",
            "isStreaming": true,
            "contextTokens": 64000,
            "contextWindow": 128000,
            "usage": { "inputTokens": 10, "outputTokens": 20, "costCny": 0.5 }
        }));
        let mut overlay = UsageOverlay::new(view.clone());

        // The default palette is byte-identical to the `render_usage` entry
        // point, which the byte-level tests above pin.
        let default_rows = overlay.render(76);
        assert_eq!(default_rows, render_usage(&view, 76));
        assert_eq!(overlay.theme(), Theme::default());

        let light = crate::themes::theme_by_id("light").expect("light is in the catalog");
        overlay.set_theme(&light);
        assert_eq!(overlay.theme(), light);
        let themed = overlay.render(76);
        assert_ne!(
            themed, default_rows,
            "a light palette must recolor the panel"
        );
        // The annotation text follows the palette (light `dim`), not `C.dim_gray`.
        let annotation = format!("\x1b[38;5;{}m", light.dim);
        assert!(themed[0].contains(&annotation), "title: {:?}", themed[0]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::utils::strip_ansi_codes;
    use serde_json::json;

    fn plain(row: &str) -> String {
        strip_ansi_codes(row)
    }

    fn plain_rows(rows: &[String]) -> Vec<String> {
        rows.iter().map(|row| plain(row)).collect()
    }

    fn rendered(usage: &UsageView, width: usize) -> String {
        plain_rows(&render_usage(usage, width)).join("\n")
    }

    /// A complete, camelCase `get_state` payload.
    fn full_state() -> Value {
        json!({
            "model": "deepseek-v4-pro",
            "sessionId": "sess-1",
            "sessionName": "Refactor",
            "thinkingLevel": "high",
            "permissionLevel": "ask",
            "isStreaming": true,
            "isCompacting": false,
            "imageSupport": true,
            "queryCount": 7,
            "queuedCount": 2,
            "contextTokens": 64000,
            "contextWindow": 128000,
            "contextPercent": 50.0,
            "usage": {
                "inputTokens": 100000,
                "outputTokens": 20000,
                "cacheReadTokens": 30000,
                "cacheWriteTokens": 5000,
                "costCny": 1.234,
                "costInputCny": 0.5,
                "costOutputCny": 0.6,
                "costCacheReadCny": 0.1,
                "costCacheWriteCny": 0.034
            },
            "byModel": [
                {"model": "deepseek-v4-pro", "inputTokens": 90000, "outputTokens": 18000, "costCny": 1.1},
                {"model": "deepseek-v4-flash", "inputTokens": 10000, "outputTokens": 2000, "costCny": 0.134}
            ],
            "quota": {"used": 12.5, "limit": 100.0, "unit": "cny", "period": "month", "resetsAt": "2026-10-01"}
        })
    }

    // ─── Formatting ────────────────────────────────────────────────────────

    #[test]
    fn format_tokens_uses_compact_units() {
        assert_eq!(format_tokens(0), "0");
        assert_eq!(format_tokens(1), "1");
        assert_eq!(format_tokens(999), "999");
        assert_eq!(format_tokens(1000), "1K");
        assert_eq!(format_tokens(1234), "1.2K");
        assert_eq!(format_tokens(12_345), "12.3K");
        assert_eq!(format_tokens(999_000), "999K");
        assert_eq!(format_tokens(999_999), "1M", "rounds up into the next unit");
        assert_eq!(format_tokens(1_000_000), "1M");
        assert_eq!(format_tokens(1_234_567), "1.2M");
        assert_eq!(format_tokens(1_000_000_000), "1B");
        assert_eq!(format_tokens(1_000_000_000_000), "1T");
        assert_eq!(format_tokens(9_999_999_999_999), "10T");
    }

    #[test]
    fn format_tokens_handles_negatives_without_panicking() {
        assert_eq!(format_tokens(-1), "-1");
        assert_eq!(format_tokens(-1234), "-1.2K");
        assert_eq!(format_tokens(i64::MIN), "-9223372T");
        assert_eq!(format_tokens(i64::MAX), "9223372T");
    }

    #[test]
    fn format_cny_marks_sub_cent_amounts_and_non_finite_values() {
        assert_eq!(format_cny(0.0), "¥0.00");
        assert_eq!(format_cny(0.0001), "<¥0.01");
        assert_eq!(format_cny(-0.0001), ">-¥0.01");
        assert_eq!(format_cny(1.234), "¥1.23");
        assert_eq!(format_cny(12.3456), "¥12.35");
        assert_eq!(format_cny(-12.3456), "¥-12.35");
        assert_eq!(format_cny(1_234.5), "¥1234.50");
        assert_eq!(format_cny(f64::NAN), "—");
        assert_eq!(format_cny(f64::INFINITY), "—");
    }

    #[test]
    fn format_percent_and_bar_clamp_out_of_range_input() {
        assert_eq!(format_percent(62.54), "62.5%");
        assert_eq!(format_percent(0.0), "0.0%");
        assert_eq!(format_percent(-5.0), "0.0%");
        assert_eq!(format_percent(f64::NAN), "0.0%");
        assert_eq!(context_bar(50.0, 4), "██░░");
        assert_eq!(context_bar(100.0, 4), "████");
        assert_eq!(context_bar(150.0, 4), "████");
        assert_eq!(context_bar(-10.0, 4), "░░░░");
        assert_eq!(context_bar(f64::NAN, 4), "░░░░");
        assert_eq!(context_bar(50.0, 0), "");
    }

    #[test]
    fn quota_units_format_and_parse() {
        assert_eq!(QuotaUnit::parse(Some("CNY")), QuotaUnit::Cny);
        assert_eq!(QuotaUnit::parse(Some(" tokens ")), QuotaUnit::Tokens);
        assert_eq!(QuotaUnit::parse(Some("requests")), QuotaUnit::Requests);
        assert_eq!(QuotaUnit::parse(Some("calls")), QuotaUnit::Requests);
        assert_eq!(QuotaUnit::parse(Some("weird")), QuotaUnit::Cny);
        assert_eq!(QuotaUnit::parse(None), QuotaUnit::Cny);
        assert_eq!(QuotaUnit::Cny.format(12.5), "¥12.50");
        assert_eq!(QuotaUnit::Tokens.format(1200.0), "1.2K");
        assert_eq!(QuotaUnit::Requests.format(42.6), "43");
    }

    // ─── Parsing ───────────────────────────────────────────────────────────

    #[test]
    fn full_payload_maps_every_field() {
        let usage = usage_from_state(&full_state());
        assert!(usage.available);
        assert_eq!(usage.model, "deepseek-v4-pro");
        assert_eq!(usage.session_id.as_deref(), Some("sess-1"));
        assert_eq!(usage.session_name.as_deref(), Some("Refactor"));
        assert_eq!(usage.thinking_level, "high");
        assert_eq!(usage.permission_level.as_deref(), Some("ask"));
        assert!(usage.is_streaming);
        assert!(!usage.is_compacting);
        assert!(usage.image_support);
        assert_eq!(usage.query_count, 7);
        assert_eq!(usage.queued_count, 2);
        assert_eq!(usage.context_tokens, 64_000);
        assert_eq!(usage.context_window, 128_000);
        assert_eq!(usage.context_percent, 50.0);
        assert_eq!(
            usage.tokens,
            UsageTokens {
                input: 100_000,
                output: 20_000,
                cache_read: 30_000,
                cache_write: 5_000,
            }
        );
        assert_eq!(usage.tokens.total(), 120_000);
        assert_eq!(usage.tokens.uncached_input(), 65_000);
        assert_eq!(usage.tokens.cache_total(), 35_000);
        assert!(!usage.tokens.is_empty());
        assert_eq!(usage.cost.total_cny, 1.234);
        let breakdown = usage.cost.breakdown_total();
        assert!((breakdown - 1.234).abs() < 1e-9, "adds up: {breakdown}");
        assert!(usage.cost.is_priced());
        assert_eq!(usage.by_model.len(), 2);
        assert_eq!(usage.by_model[0].model, "deepseek-v4-pro");
        assert_eq!(usage.by_model[0].total_tokens(), 108_000);
        let quota = usage.quota.as_ref().unwrap();
        assert_eq!(quota.unit, QuotaUnit::Cny);
        assert_eq!(quota.percent(), 12.5);
        assert_eq!(quota.period.as_deref(), Some("month"));
        assert_eq!(quota.resets_at.as_deref(), Some("2026-10-01"));
        assert!(!quota.is_exhausted());
    }

    #[test]
    fn snake_case_payload_parses_too() {
        let state = json!({
            "model": "m",
            "session_id": "s",
            "session_name": "N",
            "thinking_level": "low",
            "context_tokens": 10,
            "context_window": 100,
            "image_support": true,
            "query_count": 3,
            "queued_count": 1,
            "is_streaming": true,
            "usage": {
                "input_tokens": 5,
                "output_tokens": 6,
                "cache_read_tokens": 1,
                "cache_write_tokens": 2,
                "cost_cny": 0.5
            }
        });
        let usage = usage_from_state(&state);
        assert!(usage.available);
        assert_eq!(usage.session_id.as_deref(), Some("s"));
        assert_eq!(usage.session_name.as_deref(), Some("N"));
        assert_eq!(usage.thinking_level, "low");
        assert_eq!(usage.context_tokens, 10);
        assert_eq!(usage.context_window, 100);
        assert!(usage.image_support);
        assert_eq!(usage.query_count, 3);
        assert_eq!(usage.queued_count, 1);
        assert!(usage.is_streaming);
        assert_eq!(usage.tokens.output, 6);
        assert_eq!(usage.cost.total_cny, 0.5);
    }

    #[test]
    fn session_stats_shape_parses_tokens_and_cost() {
        let state = json!({
            "sessionId": "s",
            "userMessages": 4,
            "tokens": {"input": 1000, "output": 200, "cacheRead": 50, "cacheWrite": 10, "total": 1200},
            "cost": 0.75
        });
        let usage = usage_from_state(&state);
        assert!(usage.available);
        assert_eq!(usage.tokens.input, 1000);
        assert_eq!(usage.tokens.cache_write, 10);
        assert_eq!(usage.cost.total_cny, 0.75);
        assert!(usage.cost.is_priced());
        assert_eq!(usage.percent(), 0.0, "no context window in this payload");
    }

    #[test]
    fn partial_payload_defaults_the_missing_half() {
        let state = json!({"usage": {"inputTokens": 10, "outputTokens": 0}});
        let usage = usage_from_state(&state);
        assert!(usage.available);
        assert_eq!(usage.tokens.input, 10);
        assert_eq!(usage.tokens.cache_read, 0);
        assert_eq!(usage.context_window, 0);
        assert_eq!(usage.model, "");
        assert!(usage.by_model.is_empty());
        assert!(usage.quota.is_none());
        assert_eq!(usage.percent(), 0.0);
    }

    #[test]
    fn percent_is_derived_when_the_payload_omits_it() {
        let state = json!({"contextTokens": 25, "contextWindow": 100});
        let usage = usage_from_state(&state);
        assert!(usage.available);
        assert_eq!(usage.percent(), 25.0);
        // An explicit zero still derives from the token counts.
        let state = json!({"contextTokens": 25, "contextWindow": 100, "contextPercent": 0.0});
        assert_eq!(usage_from_state(&state).percent(), 25.0);
        // A reported percentage wins over the derived one.
        let state = json!({"contextTokens": 1, "contextWindow": 100, "contextPercent": 42.5});
        assert_eq!(usage_from_state(&state).percent(), 42.5);
        // A negative percentage is clamped away.
        let state = json!({"contextWindow": 100, "contextPercent": -3.0});
        assert_eq!(usage_from_state(&state).percent(), 0.0);
    }

    #[test]
    fn numeric_strings_and_integral_floats_are_accepted() {
        let state = json!({
            "contextTokens": "1234",
            "contextWindow": 200000.0,
            "queryCount": "3",
            "usage": {"inputTokens": "100", "outputTokens": 2.0}
        });
        let usage = usage_from_state(&state);
        assert_eq!(usage.context_tokens, 1234);
        assert_eq!(usage.context_window, 200_000);
        assert_eq!(usage.query_count, 3);
        assert_eq!(usage.tokens.input, 100);
        assert_eq!(usage.tokens.output, 2);
    }

    #[test]
    fn negative_and_out_of_range_counts_are_rejected_not_wrapped() {
        let state = json!({
            "contextTokens": -5,
            "queryCount": -1,
            "usage": {"inputTokens": -100, "outputTokens": 1.0e30}
        });
        let usage = usage_from_state(&state);
        assert_eq!(usage.context_tokens, 0);
        assert_eq!(usage.query_count, 0);
        assert_eq!(usage.tokens.input, 0, "a negative count is not usable");
        assert_eq!(usage.tokens.output, 0, "1e30 does not fit an i64");
        assert_eq!(
            usage.tokens.uncached_input(),
            0,
            "cache larger than input stays at zero"
        );
    }

    #[test]
    fn malformed_payloads_never_panic_and_stay_unavailable() {
        for state in [
            json!(null),
            json!("nope"),
            json!(42),
            json!([1, 2, 3]),
            json!({}),
            json!({"usage": null}),
            json!({"usage": "not an object"}),
            json!({"tokens": []}),
            json!({"contextTokens": "abc", "contextWindow": {}, "queryCount": []}),
            json!({"quota": {"used": 1.0}}),
            json!({"byModel": "nope"}),
        ] {
            let usage = usage_from_state(&state);
            assert!(!usage.available, "unexpectedly available: {state}");
            assert_eq!(usage.tokens, UsageTokens::default());
            assert_eq!(usage.cost, UsageCost::default());
            assert!(usage.by_model.is_empty());
            assert!(usage.quota.is_none());
            // Rendering an unavailable view must still respect the width.
            for width in [1usize, 20, 120] {
                let rows = render_usage(&usage, width);
                assert_eq!(rows.len(), 2);
                for row in &rows {
                    assert_eq!(visible_width(row), width);
                }
            }
        }
    }

    #[test]
    fn a_null_bearing_payload_keeps_the_good_half() {
        let state = json!({
            "model": null,
            "contextTokens": 10,
            "contextWindow": null,
            "usage": null,
            "queuedRuns": null
        });
        let usage = usage_from_state(&state);
        assert!(usage.available, "the context token count is usable");
        assert_eq!(usage.context_tokens, 10);
        assert_eq!(usage.context_window, 0);
        assert_eq!(usage.model, "");
        assert_eq!(usage.queued_count, 0);
    }

    #[test]
    fn a_queued_run_array_supplies_the_queue_depth() {
        let state = json!({
            "contextWindow": 100,
            "queuedRuns": [{"runId": "r1"}, {"runId": "r2"}]
        });
        let usage = usage_from_state(&state);
        assert!(usage.available);
        assert_eq!(usage.queued_count, 2);
        assert!(usage.has_queue());
    }

    #[test]
    fn by_model_accepts_nested_usage_and_skips_unusable_entries() {
        let state = json!({
            "contextWindow": 10,
            "byModel": [
                {"model": "a", "usage": {"inputTokens": 10, "outputTokens": 5, "costCny": 0.5}},
                {"id": "b", "inputTokens": 1, "outputTokens": 1},
                {"name": "c", "inputTokens": 2, "outputTokens": 2, "cost": 0.1},
                {"inputTokens": 9},
                "junk",
                42
            ]
        });
        let usage = usage_from_state(&state);
        assert_eq!(usage.by_model.len(), 3);
        assert_eq!(usage.by_model[0].model, "a");
        assert_eq!(usage.by_model[0].total_tokens(), 15);
        assert_eq!(usage.by_model[0].cost_cny, 0.5);
        assert_eq!(usage.by_model[1].model, "b");
        assert_eq!(usage.by_model[2].model, "c");
        assert_eq!(usage.by_model[2].cost_cny, 0.1);
    }

    #[test]
    fn a_top_level_models_array_is_accepted_as_a_breakdown() {
        let state = json!({
            "contextWindow": 10,
            "models": [{"model": "x", "inputTokens": 1, "outputTokens": 2, "costCny": 0.1}]
        });
        let usage = usage_from_state(&state);
        assert_eq!(usage.by_model.len(), 1);
        assert_eq!(usage.by_model[0].model, "x");
    }

    #[test]
    fn quota_without_a_limit_is_ignored_and_exhaustion_is_detected() {
        let ignored = usage_from_state(&json!({"contextWindow": 1, "quota": {"used": 5.0}}));
        assert!(ignored.quota.is_none());

        let spent = usage_from_state(&json!({
            "contextWindow": 1,
            "quota": {"used": 100.0, "limit": 100.0, "unit": "tokens"}
        }));
        let quota = spent.quota.as_ref().unwrap();
        assert_eq!(quota.unit, QuotaUnit::Tokens);
        assert!(quota.is_exhausted());
        assert_eq!(quota.percent(), 100.0);
    }

    #[test]
    fn quota_with_a_zero_limit_reports_no_usage_rather_than_dividing_by_zero() {
        let quota = UsageQuota {
            used: 5.0,
            limit: 0.0,
            unit: QuotaUnit::Cny,
            period: None,
            resets_at: None,
        };
        assert_eq!(quota.percent(), 0.0);
        assert!(!quota.is_exhausted());
    }

    // ─── Warnings ──────────────────────────────────────────────────────────

    fn view_with_context(percent: f64, window: i64) -> UsageView {
        UsageView {
            available: true,
            context_percent: percent,
            context_window: window,
            context_tokens: (window as f64 * percent / 100.0) as i64,
            tokens: UsageTokens {
                input: 100,
                output: 10,
                ..Default::default()
            },
            cost: UsageCost {
                total_cny: 1.0,
                input_cny: 1.0,
                ..Default::default()
            },
            ..Default::default()
        }
    }

    #[test]
    fn the_cumulative_block_is_labelled_and_ordered_away_from_the_context_row() {
        let usage = usage_from_state(&full_state());
        let rows = plain_rows(&render_usage(&usage, 100));
        let position = |prefix: &str| {
            rows.iter()
                .position(|row| row.starts_with(prefix))
                .unwrap_or_else(|| panic!("no row starts with {prefix:?}: {rows:?}"))
        };
        // The bounded Context row first, then the unbounded block and its
        // per-turn average, then the individual counters.
        let context = position("Context");
        let heading = position("Cumulative tokens (resent each call)");
        let average = position("Avg input/query");
        let input = position("Input (uncached)");
        assert!(context < heading, "{rows:?}");
        assert!(heading < average, "{rows:?}");
        assert!(average < input, "{rows:?}");
        // The heading is an annotation (muted), not a counter in the table.
        let heading_row = render_usage(&usage, 100)
            .into_iter()
            .find(|row| row.contains("Cumulative tokens"))
            .expect("the heading row is rendered");
        assert!(
            heading_row.contains(&format!("\x1b[38;5;{}m", Chrome::LEGACY.muted)),
            "{heading_row:?}"
        );
        assert!(!heading_row.contains("¥"), "{heading_row:?}");
        let average_row = rows[average].clone();
        assert!(average_row.trim_end().ends_with("14.3K"), "{average_row}");
        // Naming is what separates the two quantities: neither the context row
        // nor the counters claim to be the other.
        let context_row = rows[context].clone();
        let input_row = rows[input].clone();
        assert!(context_row.starts_with("Context"), "{context_row}");
        assert!(!input_row.contains("Context"), "{input_row}");
    }

    #[test]
    fn the_average_input_row_divides_the_cumulative_input_by_the_query_count() {
        // 100 000 input over 7 queries; the cached subset is included because
        // each turn re-reads the whole prompt.
        let full = usage_from_state(&full_state());
        assert_eq!(full.avg_input_per_query(), Some(14_285));
        let odd = usage_from_state(&json!({
            "contextWindow": 100,
            "queryCount": 3,
            "usage": {"inputTokens": 10, "cacheReadTokens": 9}
        }));
        assert_eq!(odd.avg_input_per_query(), Some(3), "10 / 3 truncates to 3");
        // Queries but no tokens yet: an honest zero, not a dropped row.
        let idle = usage_from_state(&json!({"contextWindow": 100, "queryCount": 3}));
        assert_eq!(idle.avg_input_per_query(), Some(0));
    }

    #[test]
    fn a_session_without_queries_drops_the_average_row() {
        let usage = usage_from_state(&json!({
            "model": "m",
            "contextTokens": 10,
            "contextWindow": 100,
            "queryCount": 0,
            "usage": {"inputTokens": 5000, "outputTokens": 1}
        }));
        assert_eq!(usage.avg_input_per_query(), None, "nothing to average over");
        let text = rendered(&usage, 90);
        assert!(!text.contains("Avg input/query"), "{text}");
        // The cumulative heading and the counters are still shown.
        assert!(
            text.contains("Cumulative tokens (resent each call)"),
            "{text}"
        );
        assert!(text.contains("Input (uncached)"), "{text}");
        assert!(
            text.contains("5K"),
            "the input counter is still there: {text}"
        );
    }

    #[test]
    fn context_thresholds_are_inclusive_at_the_configured_percent() {
        assert!(!view_with_context(79.9, 1000).is_context_warning());
        assert!(view_with_context(80.0, 1000).is_context_warning());
        assert!(!view_with_context(80.0, 1000).is_context_critical());
        assert!(view_with_context(95.0, 1000).is_context_critical());
        assert!(view_with_context(140.0, 1000).is_context_critical());
        // An unknown window can never warn (nothing to measure against).
        assert!(!view_with_context(100.0, 0).is_context_warning());
        assert!(view_with_context(100.0, 0).warnings().is_empty());
    }

    #[test]
    fn warning_lines_rank_context_queue_quota_and_price_gaps() {
        let healthy = UsageView {
            available: true,
            tokens: UsageTokens {
                input: 10,
                ..Default::default()
            },
            cost: UsageCost {
                total_cny: 0.1,
                input_cny: 0.1,
                ..Default::default()
            },
            ..Default::default()
        };
        assert!(healthy.warnings().is_empty());

        let warning = view_with_context(82.0, 1000);
        assert!(warning.warnings()[0].starts_with("! context 82.0% used"));

        let critical = view_with_context(96.0, 1000);
        assert!(critical.warnings()[0].starts_with("!! context 96.0% full"));

        let mut queued = healthy.clone();
        queued.queued_count = 1;
        assert!(queued.warnings()[0].starts_with("· 1 run queued"));
        queued.queued_count = QUEUE_WARN_COUNT;
        assert!(queued.warnings()[0].starts_with("! 2 runs queued"));

        let mut compacting = healthy.clone();
        compacting.is_compacting = true;
        assert_eq!(
            compacting.warnings(),
            vec!["· compacting the context".to_string()]
        );

        let mut unpriced = healthy.clone();
        unpriced.cost = UsageCost::default();
        assert_eq!(
            unpriced.warnings(),
            vec!["· no prices on file — tokens only".to_string()]
        );
        // An empty session has nothing to price and says nothing.
        unpriced.tokens = UsageTokens::default();
        assert!(unpriced.warnings().is_empty());
    }

    #[test]
    fn quota_warnings_cover_warning_critical_and_exhausted() {
        let mut usage = view_with_context(10.0, 1000);
        let quota = |used: f64| UsageQuota {
            used,
            limit: 100.0,
            unit: QuotaUnit::Cny,
            period: Some("month".to_string()),
            resets_at: None,
        };
        usage.quota = Some(quota(50.0));
        assert!(usage.warnings().is_empty());
        usage.quota = Some(quota(80.0));
        assert!(usage.warnings()[0].starts_with("! quota 80.0% used"));
        usage.quota = Some(quota(95.0));
        assert!(usage.warnings()[0].starts_with("!! quota 95.0% used"));
        usage.quota = Some(quota(120.0));
        assert!(usage.warnings()[0].starts_with("!! quota exhausted"));
    }

    // ─── Rendering ─────────────────────────────────────────────────────────

    #[test]
    fn every_row_is_exactly_width_columns_for_every_width() {
        let usage = usage_from_state(&full_state());
        for width in [0usize, 1, 2, 3, 8, 16, 24, 40, 46, 60, 120, 240] {
            let rows = render_usage(&usage, width);
            if width == 0 {
                assert!(rows.is_empty());
                continue;
            }
            assert!(!rows.is_empty());
            for row in &rows {
                assert_eq!(visible_width(row), width, "width {width}: {:?}", plain(row));
            }
        }
    }

    #[test]
    fn extreme_widths_still_render_without_panicking() {
        let mut usage = usage_from_state(&full_state());
        usage.model = "a-very-long-model-identifier-that-overflows".to_string();
        for width in 1..=6 {
            let rows = render_usage(&usage, width);
            assert!(!rows.is_empty());
            for row in &rows {
                assert_eq!(visible_width(row), width);
            }
        }
        assert!(render_usage(&usage, 0).is_empty());
    }

    #[test]
    fn the_panel_shows_context_tokens_cost_and_session_facts() {
        let usage = usage_from_state(&full_state());
        let text = rendered(&usage, 100);
        assert!(text.contains("Usage · deepseek-v4-pro"), "{text}");
        assert!(text.contains("Refactor"));
        assert!(text.contains("streaming"));
        assert!(text.contains("Context"), "{text}");
        assert!(text.contains("50.0%"));
        assert!(text.contains("64K/128K"), "{text}");
        assert!(text.contains("█"), "the bar is drawn at this width: {text}");
        // The token block is labelled as a session total and the input row says
        // which slice of the prompt it counts.
        assert!(
            text.contains("Cumulative tokens (resent each call)"),
            "{text}"
        );
        assert!(text.contains("Avg input/query"), "{text}");
        assert!(text.contains("14.3K"), "100000 input / 7 queries: {text}");
        let input_row = plain_rows(&render_usage(&usage, 100))
            .into_iter()
            .find(|row| row.contains("65K"))
            .expect("the input counter is rendered");
        assert!(
            input_row.starts_with("Input (uncached)"),
            "the bare `Input` label is gone: {input_row}"
        );
        assert!(text.contains("65K"), "uncached input: {text}");
        assert!(text.contains("Output"), "{text}");
        assert!(text.contains("Cache read"));
        assert!(text.contains("Cache write"));
        assert!(text.contains("Total"), "{text}");
        assert!(text.contains("120K"), "input + output: {text}");
        assert!(text.contains("¥1.23"), "{text}");
        assert!(text.contains("By model"));
        assert!(text.contains("deepseek-v4-flash"));
        assert!(text.contains("7 queries"));
        assert!(text.contains("thinking high"));
        assert!(text.contains("images on"));
        assert!(text.contains("perms ask"));
        assert!(text.contains("2 queued"));
    }

    #[test]
    fn the_cost_column_disappears_on_narrow_terminals_but_the_total_stays() {
        let usage = usage_from_state(&full_state());
        let wide = rendered(&usage, 100);
        assert!(wide.contains("¥0.50"));
        assert!(wide.contains("█"), "the bar is drawn at this width: {wide}");
        assert!(wide.contains("¥1.23"));

        let medium = rendered(&usage, 30);
        assert!(
            !medium.contains("¥0.50"),
            "the cost column is hidden: {medium}"
        );
        assert!(medium.contains("¥1.23"), "the total stays: {medium}");
        assert!(medium.contains("50.0%"));

        let narrow = rendered(&usage, 22);
        assert!(!narrow.contains("█"), "no room for a bar: {narrow}");
        assert!(narrow.contains("50.0%"), "{narrow}");
        assert!(narrow.contains("¥1.23"), "{narrow}");
    }

    #[test]
    fn an_unavailable_view_says_so_instead_of_printing_zeros() {
        let usage = UsageView::default();
        let rows = plain_rows(&render_usage(&usage, 80));
        assert_eq!(rows.len(), 2);
        assert!(rows[0].contains("unknown model"), "{rows:?}");
        assert!(rows[1].contains("no usage data yet"), "{rows:?}");
        assert!(rows.iter().all(|row| !row.contains("0.0%")), "{rows:?}");
    }

    #[test]
    fn an_empty_session_renders_a_zeroed_panel() {
        let usage = usage_from_state(&json!({
            "model": "m",
            "contextTokens": 0,
            "contextWindow": 128000,
            "queryCount": 0,
            "usage": {"inputTokens": 0, "outputTokens": 0}
        }));
        assert!(usage.available);
        let text = rendered(&usage, 80);
        assert!(text.contains("0.0%"), "{text}");
        assert!(text.contains("0/128K"), "{text}");
        assert!(text.contains("0 queries"), "{text}");
        assert!(text.contains("¥0.00"), "{text}");
        assert!(!text.contains("By model"), "{text}");
        assert!(!text.contains("queue"), "{text}");
    }

    #[test]
    fn a_missing_window_renders_without_a_bar_and_without_a_warning() {
        let usage = usage_from_state(&json!({
            "model": "m",
            "contextTokens": 5000,
            "contextPercent": 99.0,
            "usage": {"inputTokens": 1, "outputTokens": 1}
        }));
        let text = rendered(&usage, 90);
        assert!(text.contains("window unknown"), "{text}");
        assert!(!text.contains("█"));
        // The session row states it too.
        assert_eq!(text.matches("window unknown").count(), 2, "{text}");
    }

    #[test]
    fn per_model_rows_are_capped_with_a_remainder_line() {
        let mut usage = usage_from_state(&full_state());
        usage.by_model = (0..MAX_MODEL_ROWS + 3)
            .map(|index| ModelUsage {
                model: format!("m{index}"),
                input_tokens: 1000,
                output_tokens: 0,
                cost_cny: 0.0,
            })
            .collect();
        let text = rendered(&usage, 90);
        assert!(text.contains("m0"));
        assert!(text.contains(&format!("m{}", MAX_MODEL_ROWS - 1)));
        assert!(!text.contains("m99"));
        assert!(text.contains("… 3 more models"), "{text}");
    }

    #[test]
    fn warning_lines_are_rendered_with_their_prefix() {
        let mut usage = usage_from_state(&full_state());
        usage.context_percent = 97.0;
        usage.queued_count = 4;
        let text = rendered(&usage, 100);
        assert!(text.contains("!! context 97.0% full"), "{text}");
        assert!(text.contains("! 4 runs queued"), "{text}");
    }

    #[test]
    fn an_unpriced_model_prints_tokens_without_inventing_a_breakdown() {
        let usage = usage_from_state(&json!({
            "model": "local",
            "contextTokens": 10,
            "contextWindow": 100,
            "usage": {"inputTokens": 50, "outputTokens": 5}
        }));
        let text = rendered(&usage, 100);
        assert!(text.contains("¥0.00"), "the authoritative total: {text}");
        assert!(text.contains("no prices on file"), "{text}");
    }

    #[test]
    fn a_streaming_session_is_marked_in_the_title_only_when_streaming() {
        let mut usage = usage_from_state(&full_state());
        assert!(rendered(&usage, 80).contains("streaming"));
        usage.is_streaming = false;
        assert!(!rendered(&usage, 80).contains("streaming"));
    }

    #[test]
    fn the_stats_shape_reads_a_top_level_cost() {
        let usage = usage_from_state(&json!({
            "tokens": {"input": 10, "output": 5},
            "cost": 1.25
        }));
        assert!(usage.available);
        assert_eq!(usage.tokens.total(), 15);
        assert_eq!(usage.cost.total_cny, 1.25);
    }

    #[test]
    fn the_stats_shape_falls_back_to_a_cost_beside_the_tokens() {
        let usage = usage_from_state(&json!({"tokens": {"input": 10, "cost": 0.5}}));
        assert_eq!(usage.cost.total_cny, 0.5);
    }

    #[test]
    fn an_older_payload_reports_the_prompt_size_as_the_context() {
        let usage = usage_from_state(&json!({
            "usage": {"inputTokens": 10},
            "promptTokens": 1234
        }));
        assert_eq!(usage.context_tokens, 1234);
        let text = rendered(&usage, 80);
        assert!(text.contains("1.2K"), "{text}");
    }

    #[test]
    fn blank_strings_are_not_identity() {
        let usage = usage_from_state(&json!({"model": "   ", "usage": {"inputTokens": 1}}));
        assert_eq!(usage.model, "");
        assert!(rendered(&usage, 60).contains("unknown model"));
    }

    #[test]
    fn fit_row_has_nothing_to_pad_at_width_zero() {
        assert_eq!(fit_row("anything", 0), "");
        assert_eq!(fit_row("", 4), "    ");
    }

    #[test]
    fn the_context_warning_band_uses_the_palette_warning_role() {
        // 85 % of the window: past the warning threshold, short of critical.
        let usage = usage_from_state(&json!({
            "model": "m",
            "contextTokens": 85000,
            "contextWindow": 100000
        }));
        assert!(usage.is_context_warning() && !usage.is_context_critical());
        let context_row = render_usage(&usage, 76)
            .into_iter()
            .find(|row| row.contains("Context"))
            .expect("the panel has a context row");
        assert!(
            context_row.contains(&format!("\x1b[38;5;{}m", Chrome::LEGACY.warn)),
            "{context_row:?}"
        );
    }

    #[test]
    fn a_single_query_and_a_running_compaction_are_reported() {
        let usage = usage_from_state(&json!({
            "model": "m",
            "queryCount": 1,
            "isCompacting": true,
            "contextTokens": 10,
            "contextWindow": 100
        }));
        let text = rendered(&usage, 100);
        assert!(text.contains("1 query"), "{text}");
        assert!(!text.contains("1 queries"), "{text}");
        assert!(text.contains("compacting"), "{text}");
    }
}
