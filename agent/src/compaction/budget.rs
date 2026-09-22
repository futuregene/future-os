//! Request-aware bounds shared by automatic and manual S2 compaction.
use super::{estimate_text_tokens, ContextManager, PromptContext};
use crate::types::ToolDef;

pub const TARGET_HISTORY: u64 = 32_000;
/// Ceiling for the compacted history, before the capacity clamp
/// (`min(this, (limit - fixed) * 3/4)`). This is the budget the protected assistant
/// originals compete for: user originals are never dropped, so when they plus the retained
/// tail and the evidence slot no longer fit, the plan fails outright rather than truncating
/// them; assistant originals are demoted newest-first and counted in a retention note.
///
/// Measured against every session in a real Agent database (the largest had 634K tokens of
/// history), 64 000 left that session 281 tokens of headroom: it kept every original, but
/// any further growth would have started demoting assistant text — and the trigger change
/// in the same series lets sessions grow past that point now. 128 000 restores a real
/// margin, and yields to the capacity clamp on smaller windows (128K window: 82 176;
/// 262K window: 96 768).
pub const MAX_EXPANDED_HISTORY: u64 = 128_000;
pub const RECENT_HISTORY: u64 = 8_000;

/// The economic trigger: 80% of the declared window, with no absolute cap.
///
/// A cap above this point only made large windows compact early. On a 1M-token model it
/// held the trigger at 256K, so the session compacted with three quarters of its window
/// unused; the admission check below is what actually bounds a request, and it clamps the
/// effective trigger to `window - output reserve - margin` anyway. That reserve is
/// `models::capped_output_tokens` rather than the raw catalog value, so a model declaring an
/// output limit the size of its window cannot drag the trigger down to nothing.
///
/// Consequence worth knowing: the compaction that fires at this point summarises a much
/// larger live conversation. That request reuses the turn's prefix (so it is normally
/// cache-served), but a cold cache means paying for the whole prefix at once.
pub fn trigger_tokens(window: u64) -> u64 {
    window.saturating_mul(4) / 5
}

/// Includes system/tools, conservative message framing and image allowance.
/// This remains an estimate: provider context-limit recovery is still required.
pub fn set_request_budget(
    prompt: &mut PromptContext,
    system: &str,
    tools: &[ToolDef],
    max_output: i32,
) {
    let tool_text = serde_json::to_string(tools).unwrap_or_default();
    let fixed = estimate_text_tokens(system) as u64 + estimate_text_tokens(&tool_text) as u64;
    let messages = prompt
        .messages
        .iter()
        .map(super::semantic::projected_token_cost)
        .sum::<u64>();
    prompt.usage.fixed_input_tokens = fixed;
    prompt.usage.output_reserve_tokens = max_output.max(0) as u64;
    prompt.usage.estimated_input_tokens = fixed.saturating_add(messages);
}

impl ContextManager {
    pub(crate) fn input_limit(&self, prompt: &PromptContext) -> u64 {
        self.input_limit_for_usage(&prompt.usage)
    }

    pub(crate) fn input_limit_for_usage(&self, usage: &super::ContextUsage) -> u64 {
        let window = self.context_window.max(1) as u64;
        let margin = 2_048.min(window / 16);
        // The same bound the request path applies (`models::capped_output_tokens`):
        // a reserve cannot be allowed to consume the window it is reserving
        // space in. `set_request_budget` already receives a capped value, so this
        // normally changes nothing; it keeps the invariant true for any caller
        // that passes a raw declared limit, instead of turning it into
        // "no input room" and failing the request.
        let reserve = crate::models::capped_output_tokens(
            i32::try_from(usage.output_reserve_tokens).unwrap_or(i32::MAX),
            self.context_window,
        ) as u64;
        window.saturating_sub(reserve).saturating_sub(margin)
    }

    pub(crate) fn effective_trigger(&self, prompt: &PromptContext) -> u64 {
        let configured =
            (self.context_window.max(1) as u64).saturating_sub(self.reserve_tokens.max(0) as u64);
        configured.min(self.input_limit(prompt))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn trigger_follows_the_window_without_an_absolute_cap() {
        assert_eq!(trigger_tokens(1_000_000), 800_000);
        assert_eq!(trigger_tokens(640_000), 512_000);
        assert_eq!(trigger_tokens(320_000), 256_000);
        assert_eq!(trigger_tokens(200_000), 160_000);
        assert_eq!(trigger_tokens(128_000), 102_400);
        assert_eq!(trigger_tokens(32_000), 25_600);
    }
    #[test]
    fn overhead_and_output_reserve_are_counted() {
        let mut prompt = super::super::project_prompt_context(&[], None, None, 128_000);
        set_request_budget(&mut prompt, &"x".repeat(400), &[], 32_000);
        assert!(prompt.usage.fixed_input_tokens >= 100);
        let manager = ContextManager {
            enabled: true,
            reserve_tokens: 25_600,
            keep_recent_tokens: 8000,
            context_window: 128_000,
            model: "m".into(),
        };
        assert_eq!(manager.input_limit(&prompt), 93_952);
        assert_eq!(manager.effective_trigger(&prompt), 93_952);
    }

    /// The Kimi shape end to end: the catalog declares an output limit equal to
    /// the window, and both the request and the reserve go through
    /// `models::effective_max_tokens`. Every request of that session — the first
    /// one included — used to fail with "system/tools and output reservation
    /// leave no input room" before any provider was called.
    #[test]
    fn a_declared_output_limit_equal_to_the_window_still_admits_input() {
        let model = crate::models::Model {
            id: "kimi-k3".into(),
            context_window: 1_048_576,
            max_tokens: 1_048_576,
            reasoning: true,
            ..Default::default()
        };
        let mut prompt = super::super::project_prompt_context(&[], None, None, 1_048_576);
        set_request_budget(
            &mut prompt,
            &"s".repeat(120_000), // a realistic system prompt + tool definitions
            &[],
            crate::models::effective_max_tokens(&model),
        );
        assert_eq!(prompt.usage.output_reserve_tokens, 65_536);
        let (reserve_tokens, keep_recent_tokens) =
            super::super::context_token_budgets(model.context_window);
        let manager = ContextManager {
            enabled: true,
            reserve_tokens,
            keep_recent_tokens,
            context_window: model.context_window,
            model: model.id.clone(),
        };
        assert_eq!(manager.input_limit(&prompt), 980_992);
        // 80% of the window with no absolute cap: the reserve no longer drags
        // the trigger down to a fraction of the window.
        assert_eq!(manager.effective_trigger(&prompt), 838_860);
    }

    /// The invariant holds even for a caller that hands `set_request_budget` a
    /// raw declared limit instead of the capped one.
    #[test]
    fn a_raw_reserve_larger_than_the_window_cannot_consume_it() {
        let mut prompt = super::super::project_prompt_context(&[], None, None, 1_048_576);
        set_request_budget(&mut prompt, "system", &[], 1_048_576);
        assert_eq!(prompt.usage.output_reserve_tokens, 1_048_576);
        let (reserve_tokens, keep_recent_tokens) = super::super::context_token_budgets(1_048_576);
        let manager = ContextManager {
            enabled: true,
            reserve_tokens,
            keep_recent_tokens,
            context_window: 1_048_576,
            model: "m".into(),
        };
        assert_eq!(manager.input_limit(&prompt), 980_992);
    }
}
