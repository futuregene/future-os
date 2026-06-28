//! Settings management — settings.json format exactly.

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

/// Compaction settings
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompactionSettings {
    #[serde(default = "default_true", skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    #[serde(default = "default_compaction_reserve_tokens")]
    pub reserve_tokens: i32,
    #[serde(default = "default_compaction_keep_recent_tokens")]
    pub keep_recent_tokens: i32,
}

fn default_compaction_reserve_tokens() -> i32 {
    16384
}
fn default_compaction_keep_recent_tokens() -> i32 {
    20000
}

/// Image settings
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageSettings {
    #[serde(default = "default_true", skip_serializing_if = "Option::is_none")]
    pub auto_resize: Option<bool>,
    #[serde(default = "default_false", skip_serializing_if = "Option::is_none")]
    pub block_images: Option<bool>,
}

/// Terminal settings
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TerminalSettings {
    #[serde(default = "default_true", skip_serializing_if = "Option::is_none")]
    pub show_images: Option<bool>,
    #[serde(default = "default_image_width_cells")]
    pub image_width_cells: i32,
    #[serde(default = "default_false", skip_serializing_if = "Option::is_none")]
    pub clear_on_shrink: Option<bool>,
    #[serde(default = "default_false", skip_serializing_if = "Option::is_none")]
    pub show_terminal_progress: Option<bool>,
}

fn default_image_width_cells() -> i32 {
    60
}

/// Provider-specific retry settings
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderRetrySettings {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_ms: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_retries: Option<i32>,
    #[serde(
        default = "default_max_retry_delay_ms",
        skip_serializing_if = "Option::is_none"
    )]
    pub max_retry_delay_ms: Option<i32>,
}

/// Retry settings
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetrySettings {
    #[serde(default = "default_true", skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    #[serde(default = "default_max_retries")]
    pub max_retries: i32,
    #[serde(default = "default_base_delay_ms")]
    pub base_delay_ms: i32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<Box<ProviderRetrySettings>>,
}

fn default_max_retries() -> i32 {
    3
}
fn default_base_delay_ms() -> i32 {
    2000
}
fn default_max_retry_delay_ms() -> Option<i32> {
    Some(60000)
}

/// Branch summary settings
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BranchSummarySettings {
    #[serde(default = "default_branch_summary_reserve_tokens")]
    pub reserve_tokens: i32,
    #[serde(default = "default_false", skip_serializing_if = "Option::is_none")]
    pub skip_prompt: Option<bool>,
}

fn default_branch_summary_reserve_tokens() -> i32 {
    16384
}

/// Markdown settings
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MarkdownSettings {
    #[serde(
        default = "default_code_block_indent",
        skip_serializing_if = "String::is_empty"
    )]
    pub code_block_indent: String,
}

fn default_code_block_indent() -> String {
    "  ".to_string()
}

/// Package source — PackageSource: string or { source, extensions?, skills?, prompts?, themes? }
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum PackageSource {
    String(String),
    Object {
        source: String,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        extensions: Vec<String>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        skills: Vec<String>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        prompts: Vec<String>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        themes: Vec<String>,
    },
}

/// Warning settings
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WarningSettings {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub anthropic_extra_usage: Option<bool>,
}

/// Main Settings struct
///
/// JSON field names use camelCase .
/// Defaults match SettingsManager getter defaults.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub last_changelog_version: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub default_provider: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub default_model: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub default_thinking_level: String,
    #[serde(
        default = "default_transport",
        skip_serializing_if = "String::is_empty"
    )]
    pub transport: String,
    #[serde(
        default = "default_steering_mode",
        skip_serializing_if = "String::is_empty"
    )]
    pub steering_mode: String,
    #[serde(
        default = "default_follow_up_mode",
        skip_serializing_if = "String::is_empty"
    )]
    pub follow_up_mode: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub theme: String,
    #[serde(
        default = "default_compaction",
        skip_serializing_if = "Option::is_none"
    )]
    pub compaction: Option<Box<CompactionSettings>>,
    #[serde(
        default = "default_branch_summary",
        skip_serializing_if = "Option::is_none"
    )]
    pub branch_summary: Option<Box<BranchSummarySettings>>,
    #[serde(default = "default_retry", skip_serializing_if = "Option::is_none")]
    pub retry: Option<Box<RetrySettings>>,
    #[serde(default = "default_false", skip_serializing_if = "Option::is_none")]
    pub hide_thinking_block: Option<bool>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub shell_path: String,
    #[serde(default = "default_false", skip_serializing_if = "Option::is_none")]
    pub quiet_startup: Option<bool>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub shell_command_prefix: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub npm_command: Vec<String>,
    #[serde(default = "default_false", skip_serializing_if = "Option::is_none")]
    pub collapse_changelog: Option<bool>,
    #[serde(default = "default_true", skip_serializing_if = "Option::is_none")]
    pub enable_install_telemetry: Option<bool>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub packages: Vec<PackageSource>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub extensions: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub skills: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub prompts: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub themes: Vec<String>,
    #[serde(default = "default_true", skip_serializing_if = "Option::is_none")]
    pub enable_skill_commands: Option<bool>,
    #[serde(default = "default_terminal", skip_serializing_if = "Option::is_none")]
    pub terminal: Option<Box<TerminalSettings>>,
    #[serde(default = "default_images", skip_serializing_if = "Option::is_none")]
    pub images: Option<Box<ImageSettings>>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub enabled_models: Vec<String>,
    #[serde(
        default = "default_double_escape_action",
        skip_serializing_if = "String::is_empty"
    )]
    pub double_escape_action: String,
    #[serde(
        default = "default_tree_filter_mode",
        skip_serializing_if = "String::is_empty"
    )]
    pub tree_filter_mode: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thinking_budgets: Option<Box<ThinkingBudgetsSettings>>,
    #[serde(default)]
    pub editor_padding_x: i32,
    #[serde(default = "default_autocomplete_max_visible")]
    pub autocomplete_max_visible: i32,
    #[serde(
        default = "default_show_hardware_cursor",
        skip_serializing_if = "Option::is_none"
    )]
    pub show_hardware_cursor: Option<bool>,
    #[serde(default = "default_markdown", skip_serializing_if = "Option::is_none")]
    pub markdown: Option<Box<MarkdownSettings>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub warnings: Option<Box<WarningSettings>>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub session_dir: String,

    // ─── future-specific extensions (not in upstream) ─────────────────────────────
    #[serde(default = "default_max_turns")]
    pub max_turns: i32,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub system_prompt: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub thinking_level: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub scoped_models: Vec<String>,
}

// ─── Serde default functions (defaults) ──────────────────────

fn default_steering_mode() -> String {
    "one-at-a-time".to_string()
}
fn default_follow_up_mode() -> String {
    "one-at-a-time".to_string()
}
fn default_transport() -> String {
    "auto".to_string()
}
fn default_double_escape_action() -> String {
    "tree".to_string()
}
fn default_tree_filter_mode() -> String {
    "default".to_string()
}
fn default_autocomplete_max_visible() -> i32 {
    5
}
fn default_max_turns() -> i32 {
    50
}
fn default_true() -> Option<bool> {
    Some(true)
}
fn default_false() -> Option<bool> {
    Some(false)
}
fn default_show_hardware_cursor() -> Option<bool> {
    if std::env::var("PI_HARDWARE_CURSOR").is_ok_and(|v| v == "1") {
        Some(true)
    } else {
        Some(false)
    }
}
fn default_compaction() -> Option<Box<CompactionSettings>> {
    Some(Box::new(CompactionSettings {
        enabled: Some(true),
        reserve_tokens: 16384,
        keep_recent_tokens: 20000,
    }))
}
fn default_images() -> Option<Box<ImageSettings>> {
    Some(Box::new(ImageSettings {
        auto_resize: Some(true),
        block_images: Some(false),
    }))
}
fn default_terminal() -> Option<Box<TerminalSettings>> {
    Some(Box::new(TerminalSettings {
        show_images: Some(true),
        image_width_cells: 60,
        clear_on_shrink: Some(false),
        show_terminal_progress: Some(false),
    }))
}
fn default_retry() -> Option<Box<RetrySettings>> {
    Some(Box::new(RetrySettings {
        enabled: Some(true),
        max_retries: 3,
        base_delay_ms: 2000,
        provider: Some(Box::new(ProviderRetrySettings {
            timeout_ms: None,
            max_retries: None,
            max_retry_delay_ms: Some(60000),
        })),
    }))
}
fn default_branch_summary() -> Option<Box<BranchSummarySettings>> {
    Some(Box::new(BranchSummarySettings {
        reserve_tokens: 16384,
        skip_prompt: Some(false),
    }))
}
fn default_markdown() -> Option<Box<MarkdownSettings>> {
    Some(Box::new(MarkdownSettings {
        code_block_indent: "  ".to_string(),
    }))
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            last_changelog_version: String::new(),
            default_provider: String::new(),
            default_model: String::new(),
            default_thinking_level: String::new(),
            transport: default_transport(),
            steering_mode: default_steering_mode(),
            follow_up_mode: default_follow_up_mode(),
            theme: String::new(),
            compaction: default_compaction(),
            branch_summary: default_branch_summary(),
            retry: default_retry(),
            hide_thinking_block: default_false(),
            shell_path: String::new(),
            quiet_startup: default_false(),
            shell_command_prefix: String::new(),
            npm_command: vec![],
            collapse_changelog: default_false(),
            enable_install_telemetry: default_true(),
            packages: vec![],
            extensions: vec![],
            skills: vec![],
            prompts: vec![],
            themes: vec![],
            enable_skill_commands: default_true(),
            terminal: default_terminal(),
            images: default_images(),
            enabled_models: vec![],
            double_escape_action: default_double_escape_action(),
            tree_filter_mode: default_tree_filter_mode(),
            thinking_budgets: None,
            editor_padding_x: 0,
            autocomplete_max_visible: default_autocomplete_max_visible(),
            show_hardware_cursor: default_show_hardware_cursor(),
            markdown: default_markdown(),
            warnings: None,
            session_dir: String::new(),
            max_turns: default_max_turns(),
            system_prompt: String::new(),
            thinking_level: String::new(),
            scoped_models: vec![],
        }
    }
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

    // ─── Pi-compatible accessor helpers ───────────────────────────────────

    pub fn compaction_enabled(&self) -> bool {
        self.compaction
            .as_ref()
            .and_then(|c| c.enabled)
            .unwrap_or(true)
    }

    pub fn compaction_reserve_tokens(&self) -> i32 {
        self.compaction
            .as_ref()
            .map(|c| c.reserve_tokens)
            .unwrap_or(16384)
    }

    pub fn compaction_keep_recent_tokens(&self) -> i32 {
        self.compaction
            .as_ref()
            .map(|c| c.keep_recent_tokens)
            .unwrap_or(20000)
    }

    pub fn retry_enabled(&self) -> bool {
        self.retry.as_ref().and_then(|r| r.enabled).unwrap_or(true)
    }

    pub fn retry_max_retries(&self) -> i32 {
        self.retry.as_ref().map(|r| r.max_retries).unwrap_or(3)
    }

    pub fn retry_base_delay_ms(&self) -> i32 {
        self.retry.as_ref().map(|r| r.base_delay_ms).unwrap_or(2000)
    }

    pub fn terminal_show_images(&self) -> bool {
        self.terminal
            .as_ref()
            .and_then(|t| t.show_images)
            .unwrap_or(true)
    }

    pub fn terminal_image_width_cells(&self) -> i32 {
        self.terminal
            .as_ref()
            .map(|t| t.image_width_cells)
            .unwrap_or(60)
    }

    pub fn images_auto_resize(&self) -> bool {
        self.images
            .as_ref()
            .and_then(|i| i.auto_resize)
            .unwrap_or(true)
    }

    pub fn steering_mode_or_default(&self) -> String {
        if self.steering_mode.is_empty() {
            "one-at-a-time".to_string()
        } else {
            self.steering_mode.clone()
        }
    }

    pub fn follow_up_mode_or_default(&self) -> String {
        if self.follow_up_mode.is_empty() {
            "one-at-a-time".to_string()
        } else {
            self.follow_up_mode.clone()
        }
    }
}

/// LoadSettings reads a settings file, returns defaults if not found.
pub fn load_settings(path: &Path) -> Result<Settings> {
    if !path.exists() {
        return Ok(Settings::default());
    }
    Settings::load(path)
}

/// MergeSettings performs a deep merge of two Settings structs.
/// override takes precedence over base. Slices from override replace base slices entirely.
pub fn merge_settings(base: &Settings, override_: &Settings) -> Settings {
    let mut result = base.clone();

    if !override_.last_changelog_version.is_empty() {
        result.last_changelog_version = override_.last_changelog_version.clone();
    }
    if !override_.default_provider.is_empty() {
        result.default_provider = override_.default_provider.clone();
    }
    if !override_.default_model.is_empty() {
        result.default_model = override_.default_model.clone();
    }
    if !override_.default_thinking_level.is_empty() {
        result.default_thinking_level = override_.default_thinking_level.clone();
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
    if !override_.theme.is_empty() {
        result.theme = override_.theme.clone();
    }
    if override_.compaction.is_some() {
        result.compaction = override_.compaction.clone();
    }
    if override_.branch_summary.is_some() {
        result.branch_summary = override_.branch_summary.clone();
    }
    if override_.retry.is_some() {
        result.retry = override_.retry.clone();
    }
    if override_.hide_thinking_block.is_some() {
        result.hide_thinking_block = override_.hide_thinking_block;
    }
    if !override_.shell_path.is_empty() {
        result.shell_path = override_.shell_path.clone();
    }
    if override_.quiet_startup.is_some() {
        result.quiet_startup = override_.quiet_startup;
    }
    if !override_.shell_command_prefix.is_empty() {
        result.shell_command_prefix = override_.shell_command_prefix.clone();
    }
    if !override_.npm_command.is_empty() {
        result.npm_command = override_.npm_command.clone();
    }
    if override_.collapse_changelog.is_some() {
        result.collapse_changelog = override_.collapse_changelog;
    }
    if override_.enable_install_telemetry.is_some() {
        result.enable_install_telemetry = override_.enable_install_telemetry;
    }
    if !override_.packages.is_empty() {
        result.packages = override_.packages.clone();
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
    if !override_.themes.is_empty() {
        result.themes = override_.themes.clone();
    }
    if override_.enable_skill_commands.is_some() {
        result.enable_skill_commands = override_.enable_skill_commands;
    }
    if override_.terminal.is_some() {
        result.terminal = override_.terminal.clone();
    }
    if override_.images.is_some() {
        result.images = override_.images.clone();
    }
    if !override_.enabled_models.is_empty() {
        result.enabled_models = override_.enabled_models.clone();
    }
    if !override_.double_escape_action.is_empty() {
        result.double_escape_action = override_.double_escape_action.clone();
    }
    if !override_.tree_filter_mode.is_empty() {
        result.tree_filter_mode = override_.tree_filter_mode.clone();
    }
    if override_.thinking_budgets.is_some() {
        result.thinking_budgets = override_.thinking_budgets.clone();
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
    if override_.max_turns != 0 {
        result.max_turns = override_.max_turns;
    }
    if !override_.system_prompt.is_empty() {
        result.system_prompt = override_.system_prompt.clone();
    }
    if !override_.thinking_level.is_empty() {
        result.thinking_level = override_.thinking_level.clone();
    }
    if !override_.scoped_models.is_empty() {
        result.scoped_models = override_.scoped_models.clone();
    }

    result
}
