//! Shared Fusion settings, validated before saving and frozen per discussion.
use crate::{MODERATOR_MODEL, PANEL_MODELS};
use anyhow::{ensure, Result};
use serde::{Deserialize, Serialize};

pub const GENERATION_POLICY_VERSION: u32 = 2;
pub const STEP_TIMEOUT_SECS: u64 = 600;
pub const RUN_TIMEOUT_SECS: u64 = 2700;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ModelConfig {
    pub model: String,
    pub thinking_level: String,
    /// Legacy metadata only. Zero means no Fusion output override.
    #[serde(default, skip_serializing)]
    pub max_output_tokens: i32,
    #[serde(default, skip_serializing)]
    pub retry_max_output_tokens: i32,
}

impl ModelConfig {
    fn fixed(model: &str, level: &str) -> Self {
        Self {
            model: model.into(),
            thinking_level: level.into(),
            max_output_tokens: 0,
            retry_max_output_tokens: 0,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Configuration {
    pub version: u32,
    pub panel: Vec<ModelConfig>,
    pub moderator: ModelConfig,
    pub step_timeout_secs: u64,
    pub run_timeout_secs: u64,
}

impl Default for Configuration {
    fn default() -> Self {
        Self {
            version: GENERATION_POLICY_VERSION,
            panel: vec![
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
    /// Upgrade the retired default in saved settings, never historical run
    /// snapshots or role thinking levels. Explicitly shorter total budgets and
    /// other custom timeouts keep their existing behavior.
    pub(crate) fn upgrade_legacy_timeout(&mut self) {
        if self.step_timeout_secs == 300 && self.run_timeout_secs >= STEP_TIMEOUT_SECS {
            self.step_timeout_secs = STEP_TIMEOUT_SECS;
        }
    }

    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.version == GENERATION_POLICY_VERSION,
            "Unsupported Fusion generation policy"
        );
        ensure!(
            self.step_timeout_secs > 0
                && self.step_timeout_secs <= 3600
                && self.run_timeout_secs >= self.step_timeout_secs
                && self.run_timeout_secs <= 86_400,
            "Invalid Fusion time budget"
        );
        ensure!(
            (2..=5).contains(&self.panel.len()),
            "Fusion requires 2 to 5 participants"
        );
        for model in self.panel.iter().chain([&self.moderator]) {
            ensure!(!model.model.trim().is_empty(), "Missing Fusion model");
            ensure!(
                !model.thinking_level.trim().is_empty() && model.thinking_level.len() <= 64,
                "Fusion requires a thinking level"
            );
        }
        Ok(())
    }
}

/// Actual per-attempt policy, retained even when the child never completes.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StepSettings {
    pub thinking_level: String,
    /// Generation ceiling; Agent adapts each turn to remaining context/repair space.
    /// Independent of the final report quota; zero occurs only in legacy records.
    #[serde(default)]
    pub max_output_tokens: i32,
    #[serde(default)]
    pub context_window: usize,
    #[serde(default)]
    pub max_public_bytes: usize,
    /// Same-session report repairs; investigation tools are disabled after zero.
    #[serde(default)]
    pub report_revision: u32,
    pub timeout_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retry_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Settings {
    pub enabled: bool,
    pub configuration: Configuration,
    pub revision: u64,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            enabled: true,
            configuration: Configuration::default(),
            revision: 0,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SettingsPatch {
    pub expected_revision: u64,
    pub enabled: Option<bool>,
    pub configuration: Option<Configuration>,
}

/// Terminal adapters share the same role/count rules as the settings pages.
/// `--thinking` applies to the most recently selected main/participant role.
pub fn settings_patch_from_args(
    current: &Settings,
    args: &[String],
) -> Result<Option<SettingsPatch>> {
    let mut patch = SettingsPatch {
        expected_revision: current.revision,
        enabled: None,
        configuration: None,
    };
    let mut config = current.configuration.clone();
    let mut changed = false;
    let mut panel_replaced = false;
    let mut role: Option<usize> = None;
    let mut main = false;
    let mut index = 0;
    while index < args.len() {
        let flag = args[index].as_str();
        if flag == "--json" {
            index += 1;
            continue;
        }
        if flag == "--enable" || flag == "--disable" {
            ensure!(patch.enabled.is_none(), "Choose --enable or --disable once");
            patch.enabled = Some(flag == "--enable");
        } else {
            let value = args
                .get(index + 1)
                .filter(|value| !value.starts_with("--"))
                .ok_or_else(|| anyhow::anyhow!("{flag} needs a value"))?;
            match flag {
                "--main" => {
                    config.moderator = ModelConfig::fixed(value, "default");
                    main = true;
                    role = None;
                }
                "--participant" => {
                    if !panel_replaced {
                        config.panel.clear();
                        panel_replaced = true;
                    }
                    config.panel.push(ModelConfig::fixed(value, "default"));
                    main = false;
                    role = Some(config.panel.len() - 1);
                }
                "--thinking" => {
                    if main {
                        config.moderator.thinking_level = value.clone();
                    } else if let Some(role) = role {
                        config.panel[role].thinking_level = value.clone();
                    } else {
                        anyhow::bail!("Select --main or --participant before --thinking");
                    }
                }
                _ => anyhow::bail!("Unknown Fusion settings option: {flag}"),
            }
            changed = true;
            index += 1;
        }
        index += 1;
    }
    if changed {
        config.validate()?;
        patch.configuration = Some(config);
    }
    Ok((changed || patch.enabled.is_some()).then_some(patch))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_roles_have_explicit_thinking_without_fusion_output_caps() {
        let config = Configuration::default();
        config.validate().unwrap();
        for model in config.panel.iter().chain([&config.moderator]) {
            assert_ne!(model.thinking_level, "default");
            assert_eq!(model.max_output_tokens, 0);
            assert_eq!(model.retry_max_output_tokens, 0);
        }
        assert_eq!(config.panel[1].thinking_level, "low");
        // Four discussion phases fit at the new per-step ceiling. Retries
        // still share the original run deadline and cannot extend it.
        assert_eq!(config.step_timeout_secs, 600);
        assert_eq!(config.run_timeout_secs, 2700);
        assert!(config.run_timeout_secs > config.step_timeout_secs * 4);
    }

    #[test]
    fn timeout_upgrade_preserves_custom_budgets_and_user_thinking() {
        let mut config = Configuration {
            step_timeout_secs: 900,
            ..Default::default()
        };
        config.moderator.thinking_level = "high".into();
        let before = config.clone();
        config.upgrade_legacy_timeout();
        assert_eq!(config, before);
        config.step_timeout_secs = 300;
        config.run_timeout_secs = 450;
        let before = config.clone();
        config.upgrade_legacy_timeout();
        assert_eq!(config, before);
    }

    #[test]
    fn settings_require_a_main_and_two_to_five_independent_roles() {
        for count in 0..=6 {
            let mut config = Configuration::default();
            config.panel = vec![config.moderator.clone(); count];
            assert_eq!(config.validate().is_ok(), (2..=5).contains(&count));
        }
        let mut config = Configuration::default();
        config.panel[0] = config.moderator.clone();
        config.panel[0].thinking_level = "low".into();
        config.validate().unwrap();
        assert_eq!(config.moderator.thinking_level, "medium");
        config.moderator.model = " ".into();
        assert!(config.validate().is_err());
    }

    #[test]
    fn terminal_flags_assign_thinking_to_each_role_without_overwriting_others() {
        let args = [
            "--main",
            "future/deepseek-flash",
            "--thinking",
            "high",
            "--participant",
            "future/deepseek-flash",
            "--thinking",
            "low",
            "--participant",
            "future/qwen3.7-plus",
            "--thinking",
            "medium",
        ]
        .map(str::to_string);
        let patch = settings_patch_from_args(&Settings::default(), &args)
            .unwrap()
            .unwrap();
        let config = patch.configuration.unwrap();
        assert_eq!(config.panel.len(), 2);
        assert_eq!(config.moderator.thinking_level, "high");
        assert_eq!(config.panel[0].thinking_level, "low");
        assert_eq!(config.panel[1].thinking_level, "medium");
        assert!(settings_patch_from_args(
            &Settings::default(),
            &["--thinking".into(), "high".into()]
        )
        .is_err());
        assert!(settings_patch_from_args(
            &Settings::default(),
            &["--participant".into(), "only-one".into()]
        )
        .is_err());
    }
}
