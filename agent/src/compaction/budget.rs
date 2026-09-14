//! Request-aware bounds shared by automatic and manual S2 compaction.
use super::{estimate_text_tokens, ContextManager, PromptContext};
use crate::types::ToolDef;

pub const TRIGGER_CAP: u64 = 256_000;
pub const TARGET_HISTORY: u64 = 32_000;
pub const MAX_EXPANDED_HISTORY: u64 = 64_000;
pub const RECENT_HISTORY: u64 = 8_000;

/// 80% of the declared window or 256K, whichever is reached first.
pub fn trigger_tokens(window: u64) -> u64 {
    (window.saturating_mul(4) / 5).min(TRIGGER_CAP)
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
        window
            .saturating_sub(usage.output_reserve_tokens)
            .saturating_sub(margin)
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
    fn window_percentage_and_absolute_cap_are_independent_of_target() {
        assert_eq!(trigger_tokens(1_000_000), 256_000);
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
}
