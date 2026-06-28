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
    std::env::current_dir()
        .unwrap_or_else(|_| PathBuf::from("/tmp"))
        .to_string_lossy()
        .into()
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
