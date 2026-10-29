//! Fixed development policy. Freeze this value on each discussion so a future
//! settings UI can supply the same contract without changing the executor.
use crate::{MODERATOR_MODEL, PANEL_MODELS};
use anyhow::{ensure, Result};
use serde::{Deserialize, Serialize};

pub const GENERATION_POLICY_VERSION: u32 = 1;
pub const STEP_TIMEOUT_SECS: u64 = 300;
pub const RUN_TIMEOUT_SECS: u64 = 2700;
pub const MAX_STEP_OUTPUT_BYTES: usize = 128 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ModelConfig {
    pub model: String,
    pub thinking_level: String,
    pub max_output_tokens: i32,
    pub retry_max_output_tokens: i32,
}

impl ModelConfig {
    fn fixed(model: &str, level: &str) -> Self {
        Self {
            model: model.into(),
            thinking_level: level.into(),
            max_output_tokens: 16_384,
            retry_max_output_tokens: 32_768,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Configuration {
    pub version: u32,
    pub panel: [ModelConfig; 3],
    pub moderator: ModelConfig,
    pub step_timeout_secs: u64,
    pub run_timeout_secs: u64,
}

impl Default for Configuration {
    fn default() -> Self {
        Self {
            version: GENERATION_POLICY_VERSION,
            panel: [
                ModelConfig::fixed(PANEL_MODELS[0], "medium"),
                ModelConfig::fixed(PANEL_MODELS[1], "low"),
                ModelConfig::fixed(PANEL_MODELS[2], "medium"),
            ],
            moderator: ModelConfig::fixed(MODERATOR_MODEL, "medium"),
            step_timeout_secs: STEP_TIMEOUT_SECS,
            run_timeout_secs: RUN_TIMEOUT_SECS,
        }
    }
}

impl Configuration {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.version == GENERATION_POLICY_VERSION,
            "Unsupported Fusion generation policy"
        );
        ensure!(
            self.step_timeout_secs > 0 && self.run_timeout_secs >= self.step_timeout_secs,
            "Invalid Fusion time budget"
        );
        for model in self.panel.iter().chain([&self.moderator]) {
            ensure!(!model.model.is_empty(), "Missing Fusion model");
            ensure!(
                matches!(
                    model.thinking_level.as_str(),
                    "off" | "minimal" | "low" | "medium" | "high" | "xhigh" | "max"
                ),
                "Fusion requires an explicit thinking level"
            );
            ensure!(
                model.max_output_tokens > 0
                    && model.retry_max_output_tokens >= model.max_output_tokens,
                "Invalid Fusion output budget"
            );
        }
        Ok(())
    }
}

/// Actual per-attempt policy, retained even when the child never completes.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StepSettings {
    pub thinking_level: String,
    pub max_output_tokens: i32,
    pub timeout_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retry_reason: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_roles_have_explicit_thinking_and_larger_budgets() {
        let config = Configuration::default();
        config.validate().unwrap();
        for model in config.panel.iter().chain([&config.moderator]) {
            assert_ne!(model.thinking_level, "default");
            assert!(model.max_output_tokens > 4096);
            assert!(model.retry_max_output_tokens > model.max_output_tokens);
        }
        assert_eq!(config.panel[1].thinking_level, "low");
        // Four sequential phases, each with two attempts, still fit the outer
        // deadline, with time left for admission and delivery.
        assert!(config.run_timeout_secs > config.step_timeout_secs * 8);
    }
}
