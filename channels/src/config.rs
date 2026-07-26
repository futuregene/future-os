//! Unified channel configuration.
//! Reads from ~/.future/channels/config.json

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct ChannelConfig {
    #[serde(default)]
    pub agent: AgentConfig,
    #[serde(default)]
    pub feishu: Option<FeishuChannelConfig>,
    #[serde(default)]
    pub dingtalk: Option<DingtalkChannelConfig>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AgentConfig {
    #[serde(default = "default_grpc_addr")]
    pub grpc_addr: String,
    #[serde(default = "default_cwd")]
    pub cwd: String,
    /// Default model for channel sessions (e.g. "deepseek-v4-flash").
    /// If empty, the agent's boot-time default is used.
    #[serde(default = "default_model")]
    pub model: String,
    /// Default thinking level: "off", "minimal", "low", "medium", "high", "xhigh".
    #[serde(default = "default_thinking_level")]
    pub thinking_level: String,
    /// Default permission level: "all", "workspace", "none".
    #[serde(default = "default_permission_level")]
    pub permission_level: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct FeishuChannelConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub app_id: String,
    #[serde(default)]
    pub app_secret: String,
    #[serde(default = "default_domain")]
    pub domain: String,
    #[serde(default = "default_dm_policy")]
    pub dm_policy: String,
    #[serde(default)]
    pub dm_allowlist: Vec<String>,
    #[serde(default = "default_group_policy")]
    pub group_policy: String,
    #[serde(default)]
    pub group_allowlist: Vec<String>,
    #[serde(default = "default_true")]
    pub require_mention: bool,
    #[serde(default = "default_true")]
    pub streaming: bool,
    #[serde(default = "default_true")]
    pub resolve_sender_names: bool,
    #[serde(default = "default_max_image_mb")]
    pub max_image_mb: u64,
    #[serde(default)]
    pub typing_indicator: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct DingtalkChannelConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub client_id: String,
    #[serde(default)]
    pub client_secret: String,
    #[serde(default = "default_dingtalk_domain")]
    pub domain: String,
}

fn default_dingtalk_domain() -> String {
    "api.dingtalk.com".into()
}

impl Default for DingtalkChannelConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            client_id: String::new(),
            client_secret: String::new(),
            domain: default_dingtalk_domain(),
        }
    }
}

// ─── Defaults ──────────────────────────────────────────────────────────────

fn default_grpc_addr() -> String {
    "http://127.0.0.1:50051".into()
}
fn default_cwd() -> String {
    std::env::var("HOME").unwrap_or_else(|_| "/tmp".into())
}
fn default_model() -> String {
    "future/deepseek-v4-pro".into()
}
fn default_thinking_level() -> String {
    "xhigh".into()
}
fn default_permission_level() -> String {
    "all".into()
}
fn default_domain() -> String {
    "feishu".into()
}
fn default_dm_policy() -> String {
    "allowlist".into()
}
fn default_group_policy() -> String {
    "disabled".into()
}
fn default_true() -> bool {
    true
}
fn default_max_image_mb() -> u64 {
    10
}

// ─── Load / Save ───────────────────────────────────────────────────────────

impl ChannelConfig {
    pub fn default_path() -> PathBuf {
        home_dir()
            .join(".future")
            .join("channels")
            .join("config.json")
    }

    pub fn load() -> anyhow::Result<Self> {
        let path = Self::default_path();
        match std::fs::read_to_string(&path) {
            Ok(content) => {
                let config: Self = serde_json::from_str(&content)
                    .map_err(|e| anyhow::anyhow!("Failed to parse {}: {}", path.display(), e))?;
                Ok(config)
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                let config = Self::default();
                if let Some(parent) = path.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                std::fs::write(&path, serde_json::to_string_pretty(&config)?)?;
                anyhow::bail!(
                    "Default config written to {}. Edit it and restart.",
                    path.display()
                );
            }
            Err(e) => Err(anyhow::anyhow!(
                "Failed to read config at {}: {}",
                path.display(),
                e
            )),
        }
    }
}

impl Default for AgentConfig {
    fn default() -> Self {
        Self {
            grpc_addr: default_grpc_addr(),
            cwd: default_cwd(),
            model: default_model(),
            thinking_level: default_thinking_level(),
            permission_level: default_permission_level(),
        }
    }
}

impl Default for FeishuChannelConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            app_id: String::new(),
            app_secret: String::new(),
            domain: default_domain(),
            dm_policy: default_dm_policy(),
            dm_allowlist: vec![],
            group_policy: default_group_policy(),
            group_allowlist: vec![],
            require_mention: true,
            streaming: true,
            resolve_sender_names: true,
            max_image_mb: 10,
            typing_indicator: false,
        }
    }
}

fn home_dir() -> PathBuf {
    std::env::var("HOME")
        .ok()
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("~"))
}

#[cfg(test)]
mod tests {
    use super::*;

    // ─── AgentConfig defaults ────────────────────────────────────────────────

    #[test]
    fn agent_config_defaults() {
        let c = AgentConfig::default();
        assert_eq!(c.grpc_addr, "http://127.0.0.1:50051");
        assert_eq!(c.model, "future/deepseek-v4-pro");
        assert_eq!(c.thinking_level, "xhigh");
        assert_eq!(c.permission_level, "all");
        assert!(!c.cwd.is_empty());
    }

    // ─── FeishuChannelConfig defaults ────────────────────────────────────────

    #[test]
    fn feishu_config_defaults() {
        let c = FeishuChannelConfig::default();
        assert!(!c.enabled);
        assert!(c.app_id.is_empty());
        assert!(c.app_secret.is_empty());
        assert_eq!(c.domain, "feishu");
        assert_eq!(c.dm_policy, "allowlist");
        assert_eq!(c.group_policy, "disabled");
        assert!(c.require_mention);
        assert!(c.streaming);
        assert!(c.resolve_sender_names);
        assert_eq!(c.max_image_mb, 10);
        assert!(!c.typing_indicator);
    }

    // ─── DingtalkChannelConfig defaults ──────────────────────────────────────

    #[test]
    fn dingtalk_config_defaults() {
        let c = DingtalkChannelConfig::default();
        assert!(!c.enabled);
        assert!(c.client_id.is_empty());
        assert!(c.client_secret.is_empty());
        assert_eq!(c.domain, "api.dingtalk.com");
    }

    // ─── ChannelConfig defaults ──────────────────────────────────────────────

    #[test]
    fn channel_config_default() {
        let c = ChannelConfig::default();
        assert_eq!(c.agent.grpc_addr, "http://127.0.0.1:50051");
        assert!(c.feishu.is_none());
        assert!(c.dingtalk.is_none());
    }

    // ─── JSON deserialization ────────────────────────────────────────────────

    #[test]
    fn deserialize_empty_json() {
        let c: ChannelConfig = serde_json::from_str("{}").unwrap();
        assert_eq!(c.agent.grpc_addr, "http://127.0.0.1:50051");
        assert!(c.feishu.is_none());
        assert!(c.dingtalk.is_none());
    }

    #[test]
    fn deserialize_full_config() {
        let json = r#"{
            "agent": {
                "grpc_addr": "http://localhost:50051",
                "cwd": "/home/user",
                "model": "openai/gpt-4o",
                "thinking_level": "high",
                "permission_level": "workspace"
            },
            "feishu": {
                "enabled": true,
                "app_id": "cli_test",
                "app_secret": "secret_test",
                "domain": "feishu",
                "dm_policy": "open",
                "dm_allowlist": ["user1"],
                "group_policy": "open",
                "group_allowlist": ["chat1"],
                "require_mention": false,
                "streaming": false,
                "resolve_sender_names": false,
                "max_image_mb": 5,
                "typing_indicator": true
            },
            "dingtalk": {
                "enabled": true,
                "client_id": "ding_id",
                "client_secret": "ding_secret",
                "domain": "custom.dingtalk.com"
            }
        }"#;
        let c: ChannelConfig = serde_json::from_str(json).unwrap();
        assert_eq!(c.agent.model, "openai/gpt-4o");
        assert_eq!(c.agent.thinking_level, "high");
        let feishu = c.feishu.unwrap();
        assert!(feishu.enabled);
        assert_eq!(feishu.app_id, "cli_test");
        assert!(!feishu.require_mention);
        assert!(!feishu.streaming);
        assert_eq!(feishu.max_image_mb, 5);
        assert!(feishu.typing_indicator);
        let dingtalk = c.dingtalk.unwrap();
        assert!(dingtalk.enabled);
        assert_eq!(dingtalk.domain, "custom.dingtalk.com");
    }

    #[test]
    fn deserialize_partial_feishu() {
        let json = r#"{"feishu": {"enabled": true, "app_id": "test"}}"#;
        let c: ChannelConfig = serde_json::from_str(json).unwrap();
        let feishu = c.feishu.unwrap();
        assert!(feishu.enabled);
        assert_eq!(feishu.app_id, "test");
        assert!(feishu.app_secret.is_empty()); // default
        assert!(feishu.streaming); // default true
    }

    // ─── Roundtrip ───────────────────────────────────────────────────────────

    #[test]
    fn config_roundtrip() {
        let original = ChannelConfig {
            agent: AgentConfig {
                grpc_addr: "http://test:9999".into(),
                cwd: "/tmp".into(),
                model: "test/model".into(),
                thinking_level: "low".into(),
                permission_level: "none".into(),
            },
            feishu: Some(FeishuChannelConfig {
                enabled: true,
                app_id: "app1".into(),
                app_secret: "sec1".into(),
                ..Default::default()
            }),
            dingtalk: None,
        };
        let json = serde_json::to_string(&original).unwrap();
        let restored: ChannelConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.agent.grpc_addr, "http://test:9999");
        assert_eq!(restored.agent.model, "test/model");
        assert!(restored.feishu.as_ref().unwrap().enabled);
    }

    // ─── default_path ────────────────────────────────────────────────────────

    #[test]
    fn default_path_contains_channels() {
        let path = ChannelConfig::default_path();
        assert!(path.to_string_lossy().contains(".future"));
        assert!(path.to_string_lossy().contains("channels"));
        assert!(path.to_string_lossy().ends_with("config.json"));
    }
}
