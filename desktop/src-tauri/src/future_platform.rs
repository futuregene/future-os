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
/// the model API base is derived as `{platform}/api/v1`. Production host — the
/// production and test environments differ only in the host; see the shared
/// contract in `rpc/proto/future.proto` ("Future Platform URL Resolution").
pub(crate) const DEFAULT_FUTURE_PLATFORM_URL: &str = "https://future-os.cn";

/// Selectable FutureGene environments. Mirrors the CLI's `auth login --url`
/// targets — production is the default platform, test is the staging host. Used
/// by the environment-switch debug commands and the startup channel policy.
pub(crate) const PRODUCTION_PLATFORM_URL: &str = DEFAULT_FUTURE_PLATFORM_URL;
pub(crate) const TEST_PLATFORM_URL: &str = "https://test.future-os.cn";

/// Resolve the Future **platform** root (no `/api`), following the shared
/// contract in `rpc/proto/future.proto` ("Future Platform URL Resolution") — keep
/// aligned with the agent implementation (`agent/src/models/future.rs`):
///   1. `future.base_url` with a trailing `/api` stripped (the storage format
///      every writer uses — the CLI's `saveAuth` and this app's login /
///      environment switch all write `base_url = {platform}/api`)
///   2. `future.platform_base_url` (legacy field)
///   3. [`DEFAULT_FUTURE_PLATFORM_URL`]
///
/// Auth/account endpoints live here (`{platform}/client/v1/...`); the model API
/// base is [`resolve_future_base_url`].
pub(crate) fn resolve_future_platform_url(auth: &Value) -> String {
    let Some(future) = auth.get(FUTURE_PROVIDER_ID) else {
        return DEFAULT_FUTURE_PLATFORM_URL.to_string();
    };
    if let Some(base_url) = future
        .get("base_url")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
    {
        let trimmed = base_url.trim_end_matches('/');
        let platform = trimmed.strip_suffix("/api").unwrap_or(trimmed);
        return platform.trim_end_matches('/').to_string();
    }
    if let Some(platform_url) = future
        .get("platform_base_url")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
    {
        return platform_url.trim_end_matches('/').to_string();
    }
    DEFAULT_FUTURE_PLATFORM_URL.to_string()
}

/// Resolve the FutureGene **model API** base URL: `{platform}/api/v1`. This is
/// what the Providers page shows and what model calls use.
pub(crate) fn resolve_future_base_url(auth: &Value) -> String {
    format!("{}/api/v1", resolve_future_platform_url(auth))
}

/// The platform root currently in effect, read fresh from `auth.json`. A
/// convenience for callers (device-flow login, skill catalogue fetches) that
/// only need the live platform URL and don't already hold a parsed `auth` value.
/// A read failure resolves to [`DEFAULT_FUTURE_PLATFORM_URL`] like an empty auth.
pub(crate) fn current_platform_url() -> String {
    let auth = Value::Object(crate::auth_store::read().unwrap_or_default());
    resolve_future_platform_url(&auth)
}

/// Apply the environment policy for this build channel at startup (called once
/// before the agent is spawned, so the agent reads the right `base_url`).
///
/// Deliberately a direct `auth_store` write (not the RPC-first path of audit
/// item 2): it runs BEFORE the agent exists, so there is no agent to ask — the
/// agent simply reads the result at boot.
///
/// Release builds are production-locked: if the resolved platform is anything
/// other than production (e.g. a stale test `base_url` from a prior dev build
/// sharing `~/.future`), pin it back to production. Fresh installs already
/// resolve to production by default, so this is a no-op for them.
///
/// Dev builds default to the test environment on first launch (no `future`
/// base_url chosen yet), but leave an explicit choice alone so a manual switch
/// sticks across restarts.
pub(crate) fn apply_channel_environment_default() -> Result<(), crate::AppError> {
    apply_channel_environment_default_for(crate::build_info::is_release())
}

/// Channel-policy core, split out so the release and dev branches are both
/// reachable in tests regardless of the compile-time `FUTURE_VERSION`.
fn apply_channel_environment_default_for(is_release: bool) -> Result<(), crate::AppError> {
    let auth = Value::Object(crate::auth_store::read()?);

    if is_release {
        let platform = resolve_future_platform_url(&auth);
        if platform != PRODUCTION_PLATFORM_URL {
            crate::auth_store::set_future_base_url(&format!("{PRODUCTION_PLATFORM_URL}/api"))?;
        }
        return Ok(());
    }

    let has_explicit_env = auth
        .get(FUTURE_PROVIDER_ID)
        .map(|future| future.get("base_url").is_some() || future.get("platform_base_url").is_some())
        .unwrap_or(false);
    if !has_explicit_env {
        crate::auth_store::set_future_base_url(&format!("{TEST_PLATFORM_URL}/api"))?;
    }
    Ok(())
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
    fn base_url_wins_over_platform_base_url() {
        // Same precedence as the agent (rpc/proto/future.proto contract): the stored
        // `base_url` beats a stale `platform_base_url`.
        let auth = json!({ "future": {
            "base_url": "https://future-os.cn/api",
            "platform_base_url": "https://stale.example.com",
        } });
        assert_eq!(resolve_future_platform_url(&auth), "https://future-os.cn");
        assert_eq!(
            resolve_future_base_url(&auth),
            "https://future-os.cn/api/v1"
        );
    }

    #[test]
    fn platform_base_url_is_the_legacy_fallback() {
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

    #[test]
    fn future_entry_with_no_url_falls_back_to_default() {
        // A `future` entry with neither base_url nor platform_base_url (and a
        // whitespace-only base_url that is filtered out) resolves to the default.
        assert_eq!(
            resolve_future_platform_url(&json!({ "future": {} })),
            DEFAULT_FUTURE_PLATFORM_URL
        );
        assert_eq!(
            resolve_future_platform_url(&json!({ "future": { "base_url": "   " } })),
            DEFAULT_FUTURE_PLATFORM_URL
        );
    }

    #[test]
    fn current_platform_url_reads_auth_or_defaults() {
        let _home = crate::auth_store::test_support::HomeGuard::new("fp-current");
        // No auth.json → empty map → default platform.
        assert_eq!(current_platform_url(), DEFAULT_FUTURE_PLATFORM_URL);
    }

    #[test]
    fn dev_channel_defaults_to_test_env_when_unset() {
        let _home = crate::auth_store::test_support::HomeGuard::new("fp-dev-default");
        apply_channel_environment_default_for(false).unwrap();
        let auth = crate::auth_store::read().unwrap();
        assert_eq!(
            auth["future"]["base_url"],
            json!(format!("{TEST_PLATFORM_URL}/api"))
        );
    }

    #[test]
    fn dev_channel_leaves_explicit_env_alone() {
        let _home = crate::auth_store::test_support::HomeGuard::new("fp-dev-explicit");
        crate::auth_store::set_future_base_url("https://custom.example.com/api").unwrap();
        apply_channel_environment_default_for(false).unwrap();
        let auth = crate::auth_store::read().unwrap();
        assert_eq!(
            auth["future"]["base_url"],
            json!("https://custom.example.com/api")
        );
    }

    #[test]
    fn release_channel_pins_non_production_back_to_production() {
        let _home = crate::auth_store::test_support::HomeGuard::new("fp-release-pin");
        crate::auth_store::set_future_base_url(&format!("{TEST_PLATFORM_URL}/api")).unwrap();
        apply_channel_environment_default_for(true).unwrap();
        let auth = crate::auth_store::read().unwrap();
        assert_eq!(
            auth["future"]["base_url"],
            json!(format!("{PRODUCTION_PLATFORM_URL}/api"))
        );
    }

    #[test]
    fn release_channel_is_noop_when_already_production() {
        let _home = crate::auth_store::test_support::HomeGuard::new("fp-release-noop");
        crate::auth_store::set_future_base_url(&format!("{PRODUCTION_PLATFORM_URL}/api")).unwrap();
        apply_channel_environment_default_for(true).unwrap();
        let auth = crate::auth_store::read().unwrap();
        assert_eq!(
            auth["future"]["base_url"],
            json!(format!("{PRODUCTION_PLATFORM_URL}/api"))
        );
    }

    #[test]
    fn public_wrapper_wires_the_compile_time_channel() {
        // The test build is a dev channel (`VERSION` starts with 0), so the
        // public entry point must behave exactly like the dev branch.
        let _home = crate::auth_store::test_support::HomeGuard::new("fp-public");
        apply_channel_environment_default().unwrap();
        let auth = crate::auth_store::read().unwrap();
        assert_eq!(
            auth["future"]["base_url"],
            json!(format!("{TEST_PLATFORM_URL}/api"))
        );
    }
}
