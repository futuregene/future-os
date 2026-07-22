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
        if self.domain == "lark" {
            "https://open.larksuite.com/open-apis"
        } else {
            "https://open.feishu.cn/open-apis"
        }
    }

    pub fn api_domain(&self) -> &str {
        if self.domain == "lark" {
            "https://open.larksuite.com"
        } else {
            "https://open.feishu.cn"
        }
    }

    pub fn ws_base(&self) -> &str {
        if self.domain == "lark" {
            "wss://open.larksuite.com"
        } else {
            "wss://open.feishu.cn"
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::FeishuChannelConfig;

    fn make_config(domain: &str) -> FeishuConfig {
        FeishuConfig::from_channel_config(&FeishuChannelConfig {
            enabled: true,
            app_id: "test_app".to_string(),
            app_secret: "test_secret".to_string(),
            domain: domain.to_string(),
            dm_policy: "allowlist".to_string(),
            dm_allowlist: vec!["user1".to_string()],
            group_policy: "open".to_string(),
            group_allowlist: vec!["chat1".to_string()],
            require_mention: true,
            streaming: true,
            resolve_sender_names: false,
            max_image_mb: 5,
            typing_indicator: false,
        })
    }

    // ─── from_channel_config ────────────────────────────────────────────────

    #[test]
    fn from_channel_config_maps_all_fields() {
        let cfg = make_config("feishu");
        assert_eq!(cfg.app_id, "test_app");
        assert_eq!(cfg.app_secret, "test_secret");
        assert_eq!(cfg.domain, "feishu");
        assert_eq!(cfg.policy.dm_policy, "allowlist");
        assert_eq!(cfg.policy.dm_allowlist, vec!["user1"]);
        assert_eq!(cfg.policy.group_policy, "open");
        assert!(cfg.policy.require_mention);
        assert!(cfg.behavior.streaming);
        assert!(!cfg.behavior.resolve_sender_names);
        assert_eq!(cfg.behavior.max_image_mb, 5);
    }

    // ─── api_base / api_domain / ws_base ────────────────────────────────────

    #[test]
    fn feishu_domain_urls() {
        let cfg = make_config("feishu");
        assert_eq!(cfg.api_base(), "https://open.feishu.cn/open-apis");
        assert_eq!(cfg.api_domain(), "https://open.feishu.cn");
        assert_eq!(cfg.ws_base(), "wss://open.feishu.cn");
    }

    #[test]
    fn lark_domain_urls() {
        let cfg = make_config("lark");
        assert_eq!(cfg.api_base(), "https://open.larksuite.com/open-apis");
        assert_eq!(cfg.api_domain(), "https://open.larksuite.com");
        assert_eq!(cfg.ws_base(), "wss://open.larksuite.com");
    }

    #[test]
    fn other_domain_defaults_to_feishu() {
        let cfg = make_config("other");
        assert_eq!(cfg.api_base(), "https://open.feishu.cn/open-apis");
        assert_eq!(cfg.api_domain(), "https://open.feishu.cn");
        assert_eq!(cfg.ws_base(), "wss://open.feishu.cn");
    }

    #[test]
    fn api_base_includes_open_apis() {
        let cfg = make_config("feishu");
        assert!(cfg.api_base().ends_with("/open-apis"));
    }

    #[test]
    fn api_domain_does_not_include_open_apis() {
        let cfg = make_config("feishu");
        assert!(!cfg.api_domain().ends_with("/open-apis"));
        assert!(cfg.api_domain().ends_with("feishu.cn"));
    }
}
