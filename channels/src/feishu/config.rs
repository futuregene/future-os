//! Feishu-specific configuration types.
//! Converted from the unified crate::config::FeishuChannelConfig.

use crate::config::FeishuChannelConfig;

#[derive(Debug, Clone)]
pub struct FeishuConfig {
    pub app_id: String,
    pub app_secret: String,
    pub domain: String,
    pub policy: PolicyConfig,
    pub behavior: BehaviorConfig,
}

#[derive(Debug, Clone)]
pub struct PolicyConfig {
    pub dm_policy: String,
    pub dm_allowlist: Vec<String>,
    pub group_policy: String,
    pub group_allowlist: Vec<String>,
    pub require_mention: bool,
}

#[derive(Debug, Clone)]
pub struct BehaviorConfig {
    pub streaming: bool,
    pub resolve_sender_names: bool,
    pub max_image_mb: u64,
}

impl FeishuConfig {
    pub fn from_channel_config(cfg: &FeishuChannelConfig) -> Self {
        Self {
            app_id: cfg.app_id.clone(),
            app_secret: cfg.app_secret.clone(),
            domain: cfg.domain.clone(),
            policy: PolicyConfig {
                dm_policy: cfg.dm_policy.clone(),
                dm_allowlist: cfg.dm_allowlist.clone(),
                group_policy: cfg.group_policy.clone(),
                group_allowlist: cfg.group_allowlist.clone(),
                require_mention: cfg.require_mention,
            },
            behavior: BehaviorConfig {
                streaming: cfg.streaming,
                resolve_sender_names: cfg.resolve_sender_names,
                max_image_mb: cfg.max_image_mb,
            },
        }
    }

    pub fn api_base(&self) -> &str {
        if self.domain == "lark" { "https://open.larksuite.com/open-apis" } else { "https://open.feishu.cn/open-apis" }
    }

    pub fn api_domain(&self) -> &str {
        if self.domain == "lark" { "https://open.larksuite.com" } else { "https://open.feishu.cn" }
    }

    pub fn ws_base(&self) -> &str {
        if self.domain == "lark" { "wss://open.larksuite.com" } else { "wss://open.feishu.cn" }
    }
}
