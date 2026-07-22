/// DingTalk-specific channel configuration.
#[derive(Debug, Clone)]
pub struct DingtalkConfig {
    pub client_id: String,
    pub client_secret: String,
    /// API domain (default: api.dingtalk.com)
    pub domain: String,
}

impl DingtalkConfig {
    pub fn api_domain(&self) -> &str {
        &self.domain
    }
}

/// Convert from ChannelConfig's dingtalk section.
impl From<&crate::config::FeishuChannelConfig> for DingtalkConfig {
    fn from(cfg: &crate::config::FeishuChannelConfig) -> Self {
        Self {
            client_id: cfg.app_id.clone(),
            client_secret: cfg.app_secret.clone(),
            domain: cfg.domain.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dingtalk_config_api_domain() {
        let cfg = DingtalkConfig {
            client_id: "id".to_string(),
            client_secret: "secret".to_string(),
            domain: "api.dingtalk.com".to_string(),
        };
        assert_eq!(cfg.api_domain(), "api.dingtalk.com");
    }

    #[test]
    fn dingtalk_config_custom_domain() {
        let cfg = DingtalkConfig {
            client_id: "id".to_string(),
            client_secret: "secret".to_string(),
            domain: "custom.example.com".to_string(),
        };
        assert_eq!(cfg.api_domain(), "custom.example.com");
    }

    #[test]
    fn from_feishu_config_converts() {
        let fc = crate::config::FeishuChannelConfig {
            enabled: true,
            app_id: "ding_id".to_string(),
            app_secret: "ding_secret".to_string(),
            domain: "api.dingtalk.com".to_string(),
            ..Default::default()
        };
        let dc = DingtalkConfig::from(&fc);
        assert_eq!(dc.client_id, "ding_id");
        assert_eq!(dc.client_secret, "ding_secret");
        assert_eq!(dc.domain, "api.dingtalk.com");
    }
}
