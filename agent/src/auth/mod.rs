//! Auth module - reads API credentials from ~/.xihu/auth.json or ~/.xihu-app/auth.json
//! Mirrors the Go internal/auth package.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

/// Auth entry for a single provider
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AuthEntry {
    #[serde(rename = "type")]
    pub entry_type: String,
    pub key: String,
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
                .map(|h| h.join(".xihu-app/auth.json"))
                .unwrap_or_default(),
            dirs::home_dir()
                .map(|h| h.join(".xihu/auth.json"))
                .unwrap_or_default(),
        ];

        for path in paths {
            if path.exists() {
                if let Ok(contents) = std::fs::read_to_string(&path) {
                    if let Ok(store) = Self::from_json(&contents) {
                        eprintln!("Loaded auth from {:?}", path);
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

    /// Get API key for a provider (case-insensitive, prefix match)
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

        // Prefix match (e.g., "deepseek" matches "deepseek-v4-flash")
        for (name, entry) in &self.entries {
            if name.to_lowercase().starts_with(&provider_lower) && !entry.key.is_empty() {
                return Some(entry.key.clone());
            }
        }

        // Also check if provider starts with entry name
        for (name, entry) in &self.entries {
            if provider_lower.starts_with(&name.to_lowercase()) && !entry.key.is_empty() {
                return Some(entry.key.clone());
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
