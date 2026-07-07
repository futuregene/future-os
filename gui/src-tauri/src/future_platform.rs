//! Resolving the FutureGene platform / model-API base URLs from `auth.json`.
//!
//! This is an auth/platform concern shared across modules — the Providers page
//! (`agent_providers`), device-flow login (`future_login`), skill catalogue
//! fetches (`skills`), and the environment-switch debug commands (`commands::debug`)
//! all resolve the same platform root. It lived in `agent_providers` before, which
//! made those unrelated modules depend on the Providers page; it belongs here.

use serde_json::Value;

use crate::auth_store::FUTURE_PROVIDER_ID;

/// Future platform root (no `/api`); auth/account endpoints hang off this and
/// the model API base is derived as `{platform}/api/v1`.
pub(crate) const DEFAULT_FUTURE_PLATFORM_URL: &str = "https://future-os.cn";

/// Resolve the Future **platform** root (no `/api`), mirroring the CLI's
/// `getPlatformUrl()` precedence (see `cli/src/utils/platform.ts`):
///   1. `future.platform_base_url`
///   2. `future.base_url` with a trailing `/api` stripped (the CLI writes
///      `base_url = {platform}/api`)
///   3. [`DEFAULT_FUTURE_PLATFORM_URL`]
///
/// Auth/account endpoints live here (`{platform}/client/v1/...`); the model API
/// base is [`resolve_future_base_url`].
pub(crate) fn resolve_future_platform_url(auth: &Value) -> String {
    let Some(future) = auth.get(FUTURE_PROVIDER_ID) else {
        return DEFAULT_FUTURE_PLATFORM_URL.to_string();
    };
    if let Some(platform_url) = future.get("platform_base_url").and_then(Value::as_str) {
        return platform_url.trim_end_matches('/').to_string();
    }
    if let Some(base_url) = future.get("base_url").and_then(Value::as_str) {
        let trimmed = base_url.trim_end_matches('/');
        let platform = trimmed.strip_suffix("/api").unwrap_or(trimmed);
        return platform.trim_end_matches('/').to_string();
    }
    DEFAULT_FUTURE_PLATFORM_URL.to_string()
}

/// Resolve the FutureGene **model API** base URL: `{platform}/api/v1`. This is
/// what the Providers page shows and what model calls use.
pub(crate) fn resolve_future_base_url(auth: &Value) -> String {
    format!("{}/api/v1", resolve_future_platform_url(auth))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn platform_url_defaults_when_absent() {
        assert_eq!(
            resolve_future_platform_url(&json!({})),
            DEFAULT_FUTURE_PLATFORM_URL
        );
        assert_eq!(
            resolve_future_base_url(&json!({})),
            format!("{DEFAULT_FUTURE_PLATFORM_URL}/api/v1")
        );
    }

    #[test]
    fn platform_url_strips_trailing_api_from_base_url() {
        // The CLI writes `base_url = {platform}/api`; the platform is that minus /api.
        let auth = json!({ "future": { "base_url": "https://future-os.cn/api" } });
        assert_eq!(resolve_future_platform_url(&auth), "https://future-os.cn");
        assert_eq!(
            resolve_future_base_url(&auth),
            "https://future-os.cn/api/v1"
        );

        let trailing = json!({ "future": { "base_url": "https://future-os.cn/api/" } });
        assert_eq!(
            resolve_future_platform_url(&trailing),
            "https://future-os.cn"
        );
    }

    #[test]
    fn platform_url_prefers_platform_base_url() {
        let auth = json!({ "future": { "platform_base_url": "https://staging.example.com/" } });
        assert_eq!(
            resolve_future_platform_url(&auth),
            "https://staging.example.com"
        );
        assert_eq!(
            resolve_future_base_url(&auth),
            "https://staging.example.com/api/v1"
        );
    }

    #[test]
    fn base_url_without_api_suffix_is_used_as_platform() {
        // A bare host (no /api) is treated as the platform root verbatim.
        let auth = json!({ "future": { "base_url": "https://custom.example.com" } });
        assert_eq!(
            resolve_future_platform_url(&auth),
            "https://custom.example.com"
        );
        assert_eq!(
            resolve_future_base_url(&auth),
            "https://custom.example.com/api/v1"
        );
    }
}
