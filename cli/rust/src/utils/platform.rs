//! Platform helpers — port of `cli/src/utils/platform.ts`.

use crate::constants::{auth_file, DEFAULT_PLATFORM_URL, FUTURE_AUTH_PROVIDER};
use crate::utils::string::trim_trailing_slash;

/// Resolve the Future Platform base URL with this priority:
///   1. Explicit override (e.g. `--url` CLI argument; empty string is ignored,
///      matching JS falsy semantics)
///   2. auth.json → `future.base_url` (strip a trailing `/api` or `/api/`)
///   3. `DEFAULT_PLATFORM_URL`
pub async fn get_platform_url(override_url: Option<&str>) -> String {
    // Priority 1: explicit override
    if let Some(override_url) = override_url {
        if !override_url.is_empty() {
            return trim_trailing_slash(override_url);
        }
    }

    // Priority 2: auth.json
    if let Ok(raw) = tokio::fs::read_to_string(auth_file()).await {
        if let Ok(auth) = serde_json::from_str::<serde_json::Value>(&raw) {
            if let Some(future) = auth.get(FUTURE_AUTH_PROVIDER).and_then(|v| v.as_object()) {
                if let Some(base_url) = future.get("base_url").and_then(|v| v.as_str()) {
                    if !base_url.is_empty() {
                        // `baseUrl.replace(/\/api\/?$/, "")` — strip a trailing
                        // "/api" or "/api/" (nothing else).
                        let stripped = base_url
                            .strip_suffix("/api/")
                            .or_else(|| base_url.strip_suffix("/api"))
                            .unwrap_or(base_url);
                        return trim_trailing_slash(stripped);
                    }
                }
            }
        }
    }

    // Priority 3: default
    DEFAULT_PLATFORM_URL.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn override_wins() {
        let dir = tempfile::tempdir().unwrap();
        let saved = std::env::var_os("HOME");
        std::env::set_var("HOME", dir.path());
        let explicit = get_platform_url(Some("https://example.com/")).await;
        // Empty override is falsy in JS — falls through to auth.json/default.
        let empty_override = get_platform_url(Some("")).await;
        match saved {
            Some(v) => std::env::set_var("HOME", v),
            None => std::env::remove_var("HOME"),
        }
        assert_eq!(explicit, "https://example.com");
        assert_eq!(empty_override, DEFAULT_PLATFORM_URL);
    }

    #[tokio::test]
    async fn default_when_no_auth_file() {
        // Point HOME at a temp dir so auth.json doesn't exist.
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path();
        let saved = std::env::var_os("HOME");
        std::env::set_var("HOME", home);
        let result = get_platform_url(None).await;
        match saved {
            Some(v) => std::env::set_var("HOME", v),
            None => std::env::remove_var("HOME"),
        }
        assert_eq!(result, DEFAULT_PLATFORM_URL);
    }
}
