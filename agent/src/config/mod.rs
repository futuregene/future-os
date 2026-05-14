//! Settings management — 1:1 compatible with Go internal/settings/

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;

/// Thinking budgets per level
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ThinkingBudgetsSettings {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub minimal: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub low: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub medium: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub high: Option<i32>,
}

/// Image settings
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageSettings {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auto_resize: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub block_images: Option<bool>,
}

/// Terminal settings
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TerminalSettings {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub show_images: Option<bool>,
    #[serde(default)]
    pub image_width_cells: i32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub clear_on_shrink: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub show_terminal_progress: Option<bool>,
}

/// Provider retry settings
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderRetrySettings {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_ms: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_retries: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_retry_delay_ms: Option<i32>,
}

/// Retry settings
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetrySettings {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    #[serde(default)]
    pub max_retries: i32,
    #[serde(default)]
    pub base_delay_ms: i32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_ms: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_retry_delay_ms: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<Box<RetrySettings>>,
}

/// Branch summary settings
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BranchSummarySettings {
    #[serde(default)]
    pub reserve_tokens: i32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skip_prompt: Option<bool>,
}

/// Markdown settings
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MarkdownSettings {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub code_block_indent: String,
}

/// Package source
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackageSource {
    pub name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub version: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub source: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub path: String,
}

/// Warning settings
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WarningSettings {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub anthropic_extra_usage: Option<bool>,
}

/// Main Settings struct — mirrors Go settings.Settings exactly
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Settings {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub default_provider: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub default_model: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub default_thinking_level: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub theme: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compaction_enabled: Option<bool>,
    #[serde(default)]
    pub compaction_reserve_tokens: i32,
    #[serde(default)]
    pub compaction_keep_recent_tokens: i32,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub shell_path: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub shell_command_prefix: String,
    #[serde(default)]
    pub max_turns: i32,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub system_prompt: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub extensions: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub skills: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub prompts: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enable_skill_commands: Option<bool>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub thinking_level: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thinking_budgets: Option<Box<ThinkingBudgetsSettings>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hide_thinking_block: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub images: Option<Box<ImageSettings>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub terminal: Option<Box<TerminalSettings>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry: Option<Box<RetrySettings>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch_summary: Option<Box<BranchSummarySettings>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quiet_startup: Option<bool>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub npm_command: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub collapse_changelog: Option<bool>,
    #[serde(default)]
    pub editor_padding_x: i32,
    #[serde(default)]
    pub autocomplete_max_visible: i32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub show_hardware_cursor: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub markdown: Option<Box<MarkdownSettings>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub warnings: Option<Box<WarningSettings>>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub session_dir: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub scoped_models: Vec<String>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub double_escape_action: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub tree_filter_mode: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub enabled_models: Vec<String>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub transport: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub steering_mode: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub follow_up_mode: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enable_install_telemetry: Option<bool>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub packages: Vec<PackageSource>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub themes: Vec<String>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub last_changelog_version: String,
}

impl Settings {
    pub fn load(path: &Path) -> Result<Self> {
        let data = fs::read_to_string(path).context("read settings")?;
        serde_json::from_str(&data).context("parse settings")
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).context("create settings dir")?;
        }
        let data = serde_json::to_string_pretty(self).context("marshal settings")?;
        fs::write(path, data).context("write settings")?;
        Ok(())
    }
}

/// LoadSettings reads a settings file, returns empty Settings if not found.
pub fn load_settings(path: &Path) -> Result<Settings> {
    if !path.exists() {
        return Ok(Settings::default());
    }
    Settings::load(path)
}

/// MergeSettings performs a deep merge of two Settings structs.
/// override takes precedence over base. Slices from override replace base slices entirely.
pub fn merge_settings(base: &Settings, override_: &Settings) -> Settings {
    // Simple field-by-field merge: non-empty/zero override wins
    let mut result = base.clone();

    if !override_.default_provider.is_empty() {
        result.default_provider = override_.default_provider.clone();
    }
    if !override_.default_model.is_empty() {
        result.default_model = override_.default_model.clone();
    }
    if !override_.default_thinking_level.is_empty() {
        result.default_thinking_level = override_.default_thinking_level.clone();
    }
    if !override_.theme.is_empty() {
        result.theme = override_.theme.clone();
    }
    if override_.compaction_enabled.is_some() {
        result.compaction_enabled = override_.compaction_enabled;
    }
    if override_.compaction_reserve_tokens != 0 {
        result.compaction_reserve_tokens = override_.compaction_reserve_tokens;
    }
    if override_.compaction_keep_recent_tokens != 0 {
        result.compaction_keep_recent_tokens = override_.compaction_keep_recent_tokens;
    }
    if !override_.shell_path.is_empty() {
        result.shell_path = override_.shell_path.clone();
    }
    if !override_.shell_command_prefix.is_empty() {
        result.shell_command_prefix = override_.shell_command_prefix.clone();
    }
    if override_.max_turns != 0 {
        result.max_turns = override_.max_turns;
    }
    if !override_.system_prompt.is_empty() {
        result.system_prompt = override_.system_prompt.clone();
    }
    if !override_.extensions.is_empty() {
        result.extensions = override_.extensions.clone();
    }
    if !override_.skills.is_empty() {
        result.skills = override_.skills.clone();
    }
    if !override_.prompts.is_empty() {
        result.prompts = override_.prompts.clone();
    }
    if override_.enable_skill_commands.is_some() {
        result.enable_skill_commands = override_.enable_skill_commands;
    }
    if !override_.thinking_level.is_empty() {
        result.thinking_level = override_.thinking_level.clone();
    }
    if override_.thinking_budgets.is_some() {
        result.thinking_budgets = override_.thinking_budgets.clone();
    }
    if override_.hide_thinking_block.is_some() {
        result.hide_thinking_block = override_.hide_thinking_block;
    }
    if override_.images.is_some() {
        result.images = override_.images.clone();
    }
    if override_.terminal.is_some() {
        result.terminal = override_.terminal.clone();
    }
    if override_.retry.is_some() {
        result.retry = override_.retry.clone();
    }
    if override_.branch_summary.is_some() {
        result.branch_summary = override_.branch_summary.clone();
    }
    if override_.quiet_startup.is_some() {
        result.quiet_startup = override_.quiet_startup;
    }
    if !override_.npm_command.is_empty() {
        result.npm_command = override_.npm_command.clone();
    }
    if override_.collapse_changelog.is_some() {
        result.collapse_changelog = override_.collapse_changelog;
    }
    if override_.editor_padding_x != 0 {
        result.editor_padding_x = override_.editor_padding_x;
    }
    if override_.autocomplete_max_visible != 0 {
        result.autocomplete_max_visible = override_.autocomplete_max_visible;
    }
    if override_.show_hardware_cursor.is_some() {
        result.show_hardware_cursor = override_.show_hardware_cursor;
    }
    if override_.markdown.is_some() {
        result.markdown = override_.markdown.clone();
    }
    if override_.warnings.is_some() {
        result.warnings = override_.warnings.clone();
    }
    if !override_.session_dir.is_empty() {
        result.session_dir = override_.session_dir.clone();
    }
    if !override_.scoped_models.is_empty() {
        result.scoped_models = override_.scoped_models.clone();
    }
    if !override_.double_escape_action.is_empty() {
        result.double_escape_action = override_.double_escape_action.clone();
    }
    if !override_.tree_filter_mode.is_empty() {
        result.tree_filter_mode = override_.tree_filter_mode.clone();
    }
    if !override_.enabled_models.is_empty() {
        result.enabled_models = override_.enabled_models.clone();
    }
    if !override_.transport.is_empty() {
        result.transport = override_.transport.clone();
    }
    if !override_.steering_mode.is_empty() {
        result.steering_mode = override_.steering_mode.clone();
    }
    if !override_.follow_up_mode.is_empty() {
        result.follow_up_mode = override_.follow_up_mode.clone();
    }
    if override_.enable_install_telemetry.is_some() {
        result.enable_install_telemetry = override_.enable_install_telemetry;
    }
    if !override_.packages.is_empty() {
        result.packages = override_.packages.clone();
    }
    if !override_.themes.is_empty() {
        result.themes = override_.themes.clone();
    }
    if !override_.last_changelog_version.is_empty() {
        result.last_changelog_version = override_.last_changelog_version.clone();
    }

    result
}
