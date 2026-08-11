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

/// Base URL for DingTalk Open API endpoints: `https://{domain}`, or the
/// domain verbatim when it is already a full URL (self-hosted gateways and
/// test mocks).
pub(crate) fn base_url(domain: &str) -> String {
    if domain.starts_with("http://") || domain.starts_with("https://") {
        domain.trim_end_matches('/').to_string()
    } else {
        format!("https://{}", domain)
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

    #[test]
    fn base_url_adds_https_to_bare_domains() {
        assert_eq!(base_url("api.dingtalk.com"), "https://api.dingtalk.com");
    }

    #[test]
    fn base_url_passes_full_urls_through() {
        assert_eq!(base_url("http://127.0.0.1:8080"), "http://127.0.0.1:8080");
        assert_eq!(
            base_url("https://gw.example.com/"),
            "https://gw.example.com"
        );
    }
}
