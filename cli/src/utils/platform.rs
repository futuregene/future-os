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
        let _guard = crate::test_env::lock_env().await;
        let _home = crate::test_env::EnvGuard::temp_home();
        let explicit = get_platform_url(Some("https://example.com/")).await;
        // Empty override is falsy in JS — falls through to auth.json/default.
        let empty_override = get_platform_url(Some("")).await;
        assert_eq!(explicit, "https://example.com");
        assert_eq!(empty_override, DEFAULT_PLATFORM_URL);
    }

    #[tokio::test]
    async fn default_when_no_auth_file() {
        let _guard = crate::test_env::lock_env().await;
        let _home = crate::test_env::EnvGuard::temp_home();
        let result = get_platform_url(None).await;
        assert_eq!(result, DEFAULT_PLATFORM_URL);
    }

    /// Write `~/.future/agent/auth.json` under the guard's temp HOME.
    async fn write_auth(home: &crate::test_env::EnvGuard, body: &str) {
        let path = auth_file();
        tokio::fs::create_dir_all(path.parent().expect("parent"))
            .await
            .expect("mkdir");
        tokio::fs::write(&path, body).await.expect("write auth.json");
        let _ = home;
    }

    #[tokio::test]
    async fn auth_file_base_url_wins_over_default() {
        let _guard = crate::test_env::lock_env().await;
        let home = crate::test_env::EnvGuard::temp_home();
        write_auth(&home, r#"{"future": {"base_url": "https://corp.example.com"}}"#).await;
        assert_eq!(get_platform_url(None).await, "https://corp.example.com");
    }

    #[tokio::test]
    async fn auth_file_base_url_api_suffix_stripped() {
        let _guard = crate::test_env::lock_env().await;
        let home = crate::test_env::EnvGuard::temp_home();
        // Both "/api" and "/api/" suffixes strip; plain trailing slash trims.
        write_auth(&home, r#"{"future": {"base_url": "https://a.example.com/api"}}"#).await;
        assert_eq!(get_platform_url(None).await, "https://a.example.com");
        write_auth(&home, r#"{"future": {"base_url": "https://b.example.com/api/"}}"#).await;
        assert_eq!(get_platform_url(None).await, "https://b.example.com");
        write_auth(&home, r#"{"future": {"base_url": "https://c.example.com/"}}"#).await;
        assert_eq!(get_platform_url(None).await, "https://c.example.com");
        // A non-suffix "/api" in the middle is preserved.
        write_auth(&home, r#"{"future": {"base_url": "https://d.example.com/api/v1"}}"#).await;
        assert_eq!(get_platform_url(None).await, "https://d.example.com/api/v1");
    }

    #[tokio::test]
    async fn auth_file_edge_cases_fall_through_to_default() {
        let _guard = crate::test_env::lock_env().await;
        let home = crate::test_env::EnvGuard::temp_home();
        // Invalid JSON.
        write_auth(&home, "not json").await;
        assert_eq!(get_platform_url(None).await, DEFAULT_PLATFORM_URL);
        // `future` key not an object.
        write_auth(&home, r#"{"future": "nope"}"#).await;
        assert_eq!(get_platform_url(None).await, DEFAULT_PLATFORM_URL);
        // `base_url` empty (JS-falsy).
        write_auth(&home, r#"{"future": {"base_url": ""}}"#).await;
        assert_eq!(get_platform_url(None).await, DEFAULT_PLATFORM_URL);
        // `base_url` not a string.
        write_auth(&home, r#"{"future": {"base_url": 42}}"#).await;
        assert_eq!(get_platform_url(None).await, DEFAULT_PLATFORM_URL);
        // Explicit override still wins over a valid auth.json.
        write_auth(&home, r#"{"future": {"base_url": "https://corp.example.com"}}"#).await;
        assert_eq!(
            get_platform_url(Some("https://override.example.com/")).await,
            "https://override.example.com"
        );
    }
}
