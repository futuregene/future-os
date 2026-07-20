//! Settings management — reads/writes settings.json

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;

// ─── Compaction ────────────────────────────────────────────────────────────

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

// ─── Retry ─────────────────────────────────────────────────────────────────

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

// ─── Main Settings ─────────────────────────────────────────────────────────

/// Settings read from ~/.future/agent/settings.json
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
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
    #[serde(
        default = "default_compaction",
        skip_serializing_if = "Option::is_none"
    )]
    pub compaction: Option<Box<CompactionSettings>>,
    #[serde(default = "default_retry", skip_serializing_if = "Option::is_none")]
    pub retry: Option<Box<RetrySettings>>,
    /// Maximum LLM + tool turns per prompt (0 = unlimited).
    #[serde(default)]
    pub max_turns: i32,
    #[serde(
        default = "default_permission_level",
        skip_serializing_if = "String::is_empty"
    )]
    pub default_permission_level: String,
}

// ─── Defaults ──────────────────────────────────────────────────────────────

/// Default permission level for new installs: "all" (unrestricted) is the
/// deliberate product default — matches rpc::session::DEFAULT_PERMISSION_LEVEL.
/// Stricter levels ("workspace") are opt-in by editing settings.json.
fn default_permission_level() -> String {
    "all".to_string()
}
fn default_true() -> Option<bool> {
    Some(true)
}
fn default_steering_mode() -> String {
    "one-at-a-time".to_string()
}
fn default_follow_up_mode() -> String {
    "one-at-a-time".to_string()
}
fn default_compaction() -> Option<Box<CompactionSettings>> {
    Some(Box::new(CompactionSettings {
        enabled: Some(true),
        reserve_tokens: 16384,
        keep_recent_tokens: 20000,
    }))
}
fn default_retry() -> Option<Box<RetrySettings>> {
    Some(Box::new(RetrySettings {
        enabled: Some(true),
        max_retries: 3,
        base_delay_ms: 2000,
        provider: None,
    }))
}

// ─── Convenience accessors ─────────────────────────────────────────────────

impl Settings {
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

    pub fn save(&self, path: &Path) -> Result<()> {
        let json = serde_json::to_string_pretty(self).context("serialize settings")?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).context("create settings dir")?;
        }
        fs::write(path, json).context("write settings")?;
        Ok(())
    }
}

// ─── Load ──────────────────────────────────────────────────────────────────

/// LoadSettings reads a settings file, returns defaults if not found.
pub fn load_settings(path: &Path) -> Result<Settings> {
    if !path.exists() {
        return Ok(Settings::default());
    }
    let data = fs::read_to_string(path).context("read settings file")?;
    let settings: Settings = serde_json::from_str(&data).context("parse settings")?;
    Ok(settings)
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            steering_mode: default_steering_mode(),
            follow_up_mode: default_follow_up_mode(),
            compaction: default_compaction(),
            retry: default_retry(),
            max_turns: 0,
            default_permission_level: default_permission_level(),
        }
    }
}
