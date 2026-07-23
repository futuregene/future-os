//! Auth module - reads API credentials from ~/.future/agent/auth.json or ~/.future/agent-app/auth.json
//! Mirrors the Go internal/auth package.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Auth entry for a single provider
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AuthEntry {
    #[serde(rename = "type")]
    pub entry_type: String,
    pub key: String,
    #[serde(rename = "baseUrl", default)]
    pub base_url: Option<String>,
}

/// Auth store holding all provider credentials
#[derive(Debug, Clone)]
pub struct AuthStore {
    entries: HashMap<String, AuthEntry>,
}

impl AuthStore {
    /// Load auth from standard paths
    pub fn load() -> Self {
        let paths = vec![
            dirs::home_dir()
                .map(|h| h.join(".future/agent-app/auth.json"))
                .unwrap_or_default(),
            dirs::home_dir()
                .map(|h| h.join(".future/agent/auth.json"))
                .unwrap_or_default(),
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
    fn from_json(data: &str) -> Result<Self, String> {
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

    /// Get API key for a provider (case-insensitive, prefix match).
    ///
    /// Resolution order:
    /// 1. Exact key match in the HashMap ("deepseek-v4-pro").
    /// 2. Case-insensitive exact match.
    /// 3. Prefix match — prefers the **longest** matching entry name
    ///    (most specific).  If two entries match and have the same
    ///    length, alphabetical order breaks the tie.  This avoids
    ///    non-deterministic resolution when multiple entries share a
    ///    prefix (e.g. "deepseek-v4-flash" and "deepseek-v4-pro" both
    ///    match query "deepseek").
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

        // Prefix match: pick the **longest** matching entry name
        // (most specific), with alphabetical tie-break.
        let mut best: Option<(&String, &AuthEntry)> = None;
        for (name, entry) in &self.entries {
            if entry.key.is_empty() {
                continue;
            }
            let name_lower = name.to_lowercase();
            if name_lower.starts_with(&provider_lower) || provider_lower.starts_with(&name_lower) {
                match best {
                    None => best = Some((name, entry)),
                    Some((best_name, _)) => {
                        if name.len() > best_name.len()
                            || (name.len() == best_name.len() && name.as_str() < best_name.as_str())
                        {
                            best = Some((name, entry));
                        }
                    }
                }
            }
        }
        if let Some((_, entry)) = best {
            return Some(entry.key.clone());
        }

        None
    }

    /// Get base URL for a provider (case-insensitive, prefix match).
    pub fn base_url(&self, provider: &str) -> Option<String> {
        let provider_lower = provider.to_lowercase();
        for (name, entry) in &self.entries {
            let name_lower = name.to_lowercase();
            if name_lower == provider_lower || provider_lower.starts_with(&name_lower) {
                if let Some(ref url) = entry.base_url {
                    if !url.is_empty() {
                        return Some(url.trim_end_matches('/').to_string());
                    }
                }
            }
        }
        None
    }

    /// Get the first available API key as default
    pub fn default_key(&self) -> Option<String> {
        for entry in self.entries.values() {
            if !entry.key.is_empty() {
                return Some(entry.key.clone());
            }
        }
        None
    }
}

// Simple dirs helper
mod dirs {
    use std::path::PathBuf;

    pub fn home_dir() -> Option<PathBuf> {
        std::env::var_os("HOME")
            .or_else(|| std::env::var_os("USERPROFILE"))
            .map(PathBuf::from)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_store(json: &str) -> AuthStore {
        AuthStore::from_json(json).unwrap()
    }

    #[test]
    fn prefix_match_is_deterministic_preferring_longer_name() {
        let store2 = make_store(
            r#"{
                "deepseek":       {"type": "api_key", "key": "short-key"},
                "deepseek-v4-pro": {"type": "api_key", "key": "long-key"}
            }"#,
        );

        // Query "deepseek" → exact match for "deepseek" wins, get "short-key"
        assert_eq!(store2.get("deepseek"), Some("short-key".to_string()));

        // Query "deepseek-" → no exact match, both prefix-match.
        // "deepseek-v4-pro" (16 chars) is longer than "deepseek" (8 chars).
        assert_eq!(store2.get("deepseek-"), Some("long-key".to_string()));
    }

    #[test]
    fn prefix_match_prefers_longer_name_over_shorter() {
        // Simulate the ambiguous case: multiple entries sharing a prefix.
        // Run many iterations to catch any HashMap-ordering non-determinism.
        for _ in 0..100 {
            let store = make_store(
                r#"{
                    "openai":      {"type": "api_key", "key": "generic"},
                    "openai-gpt-5": {"type": "api_key", "key": "specific"}
                }"#,
            );
            // "openai" → exact match for "openai" wins
            assert_eq!(store.get("openai"), Some("generic".to_string()));
            // "openai-" → prefix: "openai-gpt-5" (12) > "openai" (6), so "specific"
            assert_eq!(store.get("openai-"), Some("specific".to_string()));
        }
    }

    #[test]
    fn exact_match_takes_priority_over_prefix() {
        let store = make_store(
            r#"{
                "deepseek":       {"type": "api_key", "key": "generic"},
                "deepseek-v4":    {"type": "api_key", "key": "specific"}
            }"#,
        );
        // Exact key match
        assert_eq!(store.get("deepseek"), Some("generic".to_string()));
        // Exact case-insensitive: "DeepSeek" vs "deepseek"
        assert_eq!(store.get("DeepSeek"), Some("generic".to_string()));
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
    fn prefix_match_prefers_longer_name_when_query_is_shorter() {
        let store = make_store(
            r#"{
                "deepseek-v4-pro": {"type": "api_key", "key": "pro-key"},
                "deepseek": {"type": "api_key", "key": "base-key"}
            }"#,
        );
        // "deepseek" exact match → base-key
        assert_eq!(store.get("deepseek"), Some("base-key".to_string()));
        // "deepseek-" no exact, both prefix match → prefer longer "deepseek-v4-pro"
        assert_eq!(store.get("deepseek-"), Some("pro-key".to_string()));
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
    fn base_url_case_insensitive_prefix() {
        let store = make_store(
            r#"{
                "azure-openai": {"type": "api_key", "key": "key", "baseUrl": "https://my.openai.azure.com/"}
            }"#,
        );
        assert_eq!(
            store.base_url("Azure-OpenAI-eus"),
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
    fn default_key_returns_first_non_empty() {
        let store = make_store(
            r#"{
                "a": {"type": "api_key", "key": ""},
                "b": {"type": "api_key", "key": "valid-key"}
            }"#,
        );
        assert!(store.default_key().is_some());
        assert_ne!(store.default_key().as_deref(), Some(""));
    }

    #[test]
    fn default_key_empty_store_returns_none() {
        let store = make_store(r#"{}"#);
        assert_eq!(store.default_key(), None);
    }
}
