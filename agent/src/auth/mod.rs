//! Auth module - reads API credentials from ~/.future/agent/auth.json or ~/.future/agent-app/auth.json
//! Mirrors the Go internal/auth package.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Auth entry for a single provider
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AuthEntry {
    #[serde(rename = "type")]
    pub entry_type: String,
    #[serde(default)]
    pub key: String,
    #[serde(rename = "base_url", alias = "baseUrl", default)]
    pub base_url: Option<String>,
}

/// Auth store holding all provider credentials
#[derive(Debug, Clone, Default)]
pub struct AuthStore {
    entries: HashMap<String, AuthEntry>,
}

impl AuthStore {
    /// Load auth from standard paths
    pub fn load() -> Self {
        let home = crate::utils::home_dir();
        let paths = vec![
            home.join(".future/agent-app/auth.json"),
            home.join(".future/agent/auth.json"),
        ];

        for path in paths {
            if path.exists() {
                if let Ok(contents) = std::fs::read_to_string(&path) {
                    if let Ok(store) = Self::from_json(&contents) {
                        return store;
                    }
                }
            }
        }

        Self {
            entries: HashMap::new(),
        }
    }

    /// Parse auth from JSON string
    pub(crate) fn from_json(data: &str) -> Result<Self, String> {
        let raw: HashMap<String, serde_json::Value> =
            serde_json::from_str(data).map_err(|e| e.to_string())?;

        let mut entries = HashMap::new();
        for (name, value) in raw {
            if let Ok(entry) = serde_json::from_value::<AuthEntry>(value.clone()) {
                entries.insert(name, entry);
            }
        }

        Ok(Self { entries })
    }

    /// Get the API key for exactly one provider.
    ///
    /// Credentials are provider-owned authority. Model ids, provider-name
    /// prefixes, and an account-wide "default" must never borrow another
    /// provider's secret. Case-insensitive equality is retained only for
    /// legacy auth files written before provider ids were normalized.
    pub fn get(&self, provider: &str) -> Option<String> {
        let provider_lower = provider.to_lowercase();

        // Exact match first
        if let Some(entry) = self.entries.get(provider) {
            if !entry.key.is_empty() {
                return Some(entry.key.clone());
            }
        }

        // Case-insensitive exact match
        for (name, entry) in &self.entries {
            if name.to_lowercase() == provider_lower && !entry.key.is_empty() {
                return Some(entry.key.clone());
            }
        }

        None
    }

    /// Get the base URL for exactly one provider.
    pub fn base_url(&self, provider: &str) -> Option<String> {
        let provider_lower = provider.to_lowercase();
        for (name, entry) in &self.entries {
            let name_lower = name.to_lowercase();
            if name_lower == provider_lower {
                if let Some(ref url) = entry.base_url {
                    if !url.is_empty() {
                        return Some(url.trim_end_matches('/').to_string());
                    }
                }
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_store(json: &str) -> AuthStore {
        AuthStore::from_json(json).unwrap()
    }

    #[test]
    fn provider_lookup_never_uses_prefix_matches() {
        let store2 = make_store(
            r#"{
                "deepseek":       {"type": "api_key", "key": "short-key"},
                "deepseek-v4-pro": {"type": "api_key", "key": "long-key"}
            }"#,
        );

        assert_eq!(store2.get("deepseek"), Some("short-key".to_string()));
        assert_eq!(store2.get("deepseek-"), None);
    }

    #[test]
    fn exact_match_is_case_insensitive_only() {
        let store = make_store(
            r#"{
                "deepseek":       {"type": "api_key", "key": "generic"},
                "deepseek-v4":    {"type": "api_key", "key": "specific"}
            }"#,
        );
        assert_eq!(store.get("deepseek"), Some("generic".to_string()));
        assert_eq!(store.get("DeepSeek"), Some("generic".to_string()));
        assert_eq!(store.get("deepseek-v4-pro"), None);
    }

    #[test]
    fn case_insensitive_exact_ignores_prefix() {
        let store = make_store(
            r#"{
                "DeepSeek-V4-Pro": {"type": "api_key", "key": "pro"}
            }"#,
        );
        assert_eq!(store.get("deepseek-v4-pro"), Some("pro".to_string()));
    }

    #[test]
    fn get_returns_none_for_empty_key() {
        let store = make_store(
            r#"{
                "provider": {"type": "api_key", "key": ""}
            }"#,
        );
        assert_eq!(store.get("provider"), None);
    }

    #[test]
    fn get_returns_none_for_unknown_provider() {
        let store = make_store(
            r#"{
                "openai": {"type": "api_key", "key": "sk-123"}
            }"#,
        );
        assert_eq!(store.get("unknown"), None);
    }

    #[test]
    fn similar_provider_names_remain_isolated() {
        let store = make_store(
            r#"{
                "deepseek-v4-pro": {"type": "api_key", "key": "pro-key"},
                "deepseek": {"type": "api_key", "key": "base-key"}
            }"#,
        );
        assert_eq!(store.get("deepseek"), Some("base-key".to_string()));
        assert_eq!(store.get("deepseek-"), None);
    }

    #[test]
    fn base_url_exact_match() {
        let store = make_store(
            r#"{
                "openai": {"type": "api_key", "key": "sk-123", "baseUrl": "https://api.openai.com"}
            }"#,
        );
        assert_eq!(
            store.base_url("openai"),
            Some("https://api.openai.com".to_string())
        );
    }

    #[test]
    fn base_url_reads_the_current_snake_case_storage_field() {
        let store = make_store(
            r#"{
                "future": {"type": "api_key", "key": "k", "base_url": "https://future.example/api"}
            }"#,
        );
        assert_eq!(
            store.base_url("future"),
            Some("https://future.example/api".to_string())
        );
    }

    #[test]
    fn base_url_does_not_use_prefix_matches() {
        let store = make_store(
            r#"{
                "azure-openai": {"type": "api_key", "key": "key", "baseUrl": "https://my.openai.azure.com/"}
            }"#,
        );
        assert_eq!(store.base_url("Azure-OpenAI-eus"), None);
        assert_eq!(
            store.base_url("Azure-OpenAI"),
            Some("https://my.openai.azure.com".to_string())
        );
    }

    #[test]
    fn base_url_empty_returns_none() {
        let store = make_store(
            r#"{
                "provider": {"type": "api_key", "key": "key", "baseUrl": ""}
            }"#,
        );
        assert_eq!(store.base_url("provider"), None);
    }

    #[test]
    fn base_url_unknown_provider_returns_none() {
        let store = make_store(
            r#"{
                "openai": {"type": "api_key", "key": "sk-123"}
            }"#,
        );
        assert_eq!(store.base_url("unknown"), None);
    }

    #[test]
    fn base_url_matching_entry_without_url_returns_none() {
        // Name matches but the entry carries no baseUrl → falls through to None.
        let store = make_store(
            r#"{
                "openai": {"type": "api_key", "key": "sk-123"}
            }"#,
        );
        assert_eq!(store.base_url("openai"), None);
    }

    #[test]
    fn load_unreadable_auth_file_falls_through_to_empty() {
        let home = crate::test_support::TestHome::new();
        // A directory at the auth.json path: exists() is true but
        // read_to_string fails → load() skips it and returns an empty store.
        let auth_path = home.auth_path();
        std::fs::create_dir_all(&auth_path).unwrap();
        let store = AuthStore::load();
        assert!(store.get("future").is_none());
    }

    #[test]
    fn load_invalid_json_auth_file_falls_through_to_empty() {
        let home = crate::test_support::TestHome::new();
        // Readable but unparseable auth.json → from_json fails, load()
        // skips it and returns an empty store.
        let auth_path = home.auth_path();
        std::fs::create_dir_all(auth_path.parent().unwrap()).unwrap();
        std::fs::write(&auth_path, "not json").unwrap();
        let store = AuthStore::load();
        assert!(store.get("future").is_none());
    }
}
