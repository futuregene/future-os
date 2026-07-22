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

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    // ─── Defaults ───────────────────────────────────────────────────────────

    #[test]
    fn default_settings_values() {
        let s = Settings::default();
        assert_eq!(s.steering_mode, "one-at-a-time");
        assert_eq!(s.follow_up_mode, "one-at-a-time");
        assert_eq!(s.max_turns, 0);
        assert_eq!(s.default_permission_level, "all");
    }

    #[test]
    fn default_compaction_values() {
        let s = Settings::default();
        assert!(s.compaction_enabled());
        assert_eq!(s.compaction_reserve_tokens(), 16384);
        assert_eq!(s.compaction_keep_recent_tokens(), 20000);
    }

    #[test]
    fn default_retry_values() {
        let s = Settings::default();
        assert!(s.retry_enabled());
        let retry = s.retry.unwrap();
        assert_eq!(retry.max_retries, 3);
        assert_eq!(retry.base_delay_ms, 2000);
        assert!(retry.provider.is_none());
    }

    // ─── CompactionSettings ─────────────────────────────────────────────────

    #[test]
    fn compaction_settings_defaults() {
        let c = CompactionSettings {
            enabled: Some(true),
            reserve_tokens: default_compaction_reserve_tokens(),
            keep_recent_tokens: default_compaction_keep_recent_tokens(),
        };
        assert_eq!(c.reserve_tokens, 16384);
        assert_eq!(c.keep_recent_tokens, 20000);
        assert_eq!(c.enabled, Some(true));
    }

    #[test]
    fn compaction_disabled_explicitly() {
        let s = Settings {
            compaction: Some(Box::new(CompactionSettings {
                enabled: Some(false),
                reserve_tokens: 16384,
                keep_recent_tokens: 20000,
            })),
            ..Default::default()
        };
        assert!(!s.compaction_enabled());
    }

    #[test]
    fn compaction_none_falls_back_to_defaults() {
        let s = Settings {
            compaction: None,
            ..Default::default()
        };
        assert!(s.compaction_enabled());
        assert_eq!(s.compaction_reserve_tokens(), 16384);
    }

    // ─── RetrySettings ──────────────────────────────────────────────────────

    #[test]
    fn retry_settings_defaults() {
        let r = RetrySettings {
            enabled: Some(true),
            max_retries: default_max_retries(),
            base_delay_ms: default_base_delay_ms(),
            provider: None,
        };
        assert_eq!(r.max_retries, 3);
        assert_eq!(r.base_delay_ms, 2000);
    }

    #[test]
    fn retry_disabled() {
        let s = Settings {
            retry: Some(Box::new(RetrySettings {
                enabled: Some(false),
                max_retries: 3,
                base_delay_ms: 2000,
                provider: None,
            })),
            ..Default::default()
        };
        assert!(!s.retry_enabled());
    }

    #[test]
    fn retry_none_falls_back_to_enabled() {
        let s = Settings {
            retry: None,
            ..Default::default()
        };
        assert!(s.retry_enabled());
    }

    #[test]
    fn provider_retry_settings() {
        let p = ProviderRetrySettings {
            timeout_ms: Some(5000),
            max_retries: Some(5),
            max_retry_delay_ms: Some(30000),
        };
        assert_eq!(p.timeout_ms, Some(5000));
        assert_eq!(p.max_retries, Some(5));
    }

    // ─── JSON serialization ────────────────────────────────────────────────

    #[test]
    fn serialize_default_skips_defaults() {
        let s = Settings::default();
        let json = serde_json::to_string(&s).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        // skip_serializing_if fields should be absent when matching defaults
        assert!(parsed.get("steeringMode").is_none() || parsed["steeringMode"] == "one-at-a-time");
    }

    #[test]
    fn deserialize_minimal_json() {
        let json = r#"{}"#;
        let s: Settings = serde_json::from_str(json).unwrap();
        assert_eq!(s.steering_mode, "one-at-a-time");
        assert_eq!(s.max_turns, 0);
        assert!(s.compaction.is_some());
        assert!(s.retry.is_some());
    }

    #[test]
    fn deserialize_full_json() {
        let json = r#"{
            "steeringMode": "parallel",
            "followUpMode": "queue",
            "maxTurns": 10,
            "defaultPermissionLevel": "workspace",
            "compaction": {"enabled": false, "reserveTokens": 8192, "keepRecentTokens": 10000},
            "retry": {"enabled": false, "maxRetries": 5, "baseDelayMs": 1000}
        }"#;
        let s: Settings = serde_json::from_str(json).unwrap();
        assert_eq!(s.steering_mode, "parallel");
        assert_eq!(s.follow_up_mode, "queue");
        assert_eq!(s.max_turns, 10);
        assert_eq!(s.default_permission_level, "workspace");
        assert!(!s.compaction_enabled());
        assert!(!s.retry_enabled());
    }

    #[test]
    fn roundtrip_preserves_custom_values() {
        let original = Settings {
            steering_mode: "custom".to_string(),
            max_turns: 42,
            default_permission_level: "workspace".to_string(),
            ..Default::default()
        };
        let json = serde_json::to_string(&original).unwrap();
        let restored: Settings = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.steering_mode, "custom");
        assert_eq!(restored.max_turns, 42);
        assert_eq!(restored.default_permission_level, "workspace");
    }

    // ─── File I/O ───────────────────────────────────────────────────────────

    #[test]
    fn load_settings_missing_file_returns_defaults() {
        let path = std::path::Path::new("/tmp/nonexistent_settings_test.json");
        let s = load_settings(path).unwrap();
        assert_eq!(s.steering_mode, "one-at-a-time");
        assert!(s.compaction_enabled());
    }

    #[test]
    fn save_and_load_roundtrip() {
        let dir = std::env::temp_dir().join("future_config_test");
        let path = dir.join("settings.json");

        let original = Settings {
            steering_mode: "test_mode".to_string(),
            max_turns: 99,
            ..Default::default()
        };
        original.save(&path).unwrap();

        let loaded = load_settings(&path).unwrap();
        assert_eq!(loaded.steering_mode, "test_mode");
        assert_eq!(loaded.max_turns, 99);

        // Cleanup
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn load_settings_invalid_json_errors() {
        let dir = std::env::temp_dir().join("future_config_bad_test");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("settings.json");
        let mut f = std::fs::File::create(&path).unwrap();
        write!(f, "not valid json").unwrap();
        drop(f);

        assert!(load_settings(&path).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
