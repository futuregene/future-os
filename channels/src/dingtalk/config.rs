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
