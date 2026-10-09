//! Shared Future System One transport; callers own their classification policy.
use serde_json::Value;

pub(crate) fn blocking_post(
    client: &reqwest::blocking::Client,
    endpoint: &Endpoint,
    request: &Value,
) -> Result<reqwest::blocking::Response, reqwest::Error> {
    client
        .post(&endpoint.url)
        .bearer_auth(&endpoint.key)
        .json(request)
        .send()
}

pub(crate) async fn post(
    client: &reqwest::Client,
    endpoint: &Endpoint,
    request: &Value,
    request_id: &str,
) -> Result<reqwest::Response, reqwest::Error> {
    client
        .post(&endpoint.url)
        .bearer_auth(&endpoint.key)
        .header("Idempotency-Key", request_id)
        .json(request)
        .send()
        .await
}

pub(crate) fn choice(instructions: &str, options: &[(&str, &str)]) -> Value {
    serde_json::json!({"type": "choice", "instructions": instructions,
        "criteria": options.iter().map(|(k,v)| ((*k).to_string(), Value::String((*v).to_string())))
            .collect::<serde_json::Map<String, Value>>()})
}

/// Model id the gateway resolves (`jev` → `typesafe/jev-1.13-…`).
pub(crate) const JEV_MODEL: &str = "jev";
/// Fallback origin when the Future provider has no `base_url` configured.
const DEFAULT_FUTURE_BASE: &str = "https://future-os.cn/api";
/// The provider whose credential authenticates the call.
const FUTURE_PROVIDER: &str = "future";
/// Where a call goes and what authenticates it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Endpoint {
    pub(crate) url: String,
    pub(crate) key: String,
    pub(crate) model: String,
}

/// Resolve the endpoint from the Future account: its credential authenticates
/// the call and its base URL hosts the gateway. Jev is served by the Future
/// provider and nowhere else, so there is deliberately no separate credential
/// to configure.
///
/// `None` means no account credential is available; each caller owns its
/// behavior for that configuration (skip recommendation or deny approval).
pub(crate) fn endpoint() -> Option<Endpoint> {
    resolve(&crate::auth::AuthStore::load())
}

/// The resolution itself, over a given credential store. Split out so the tests
/// can exercise it without touching the process-global HOME (which `load()` reads
/// and which other tests read concurrently).
pub(crate) fn resolve(auth: &crate::auth::AuthStore) -> Option<Endpoint> {
    let key = auth.get(FUTURE_PROVIDER)?;
    let base = auth
        .base_url(FUTURE_PROVIDER)
        .unwrap_or_else(|| DEFAULT_FUTURE_BASE.to_string());
    Some(Endpoint {
        url: systemone_url(&base),
        key,
        model: JEV_MODEL.to_string(),
    })
}

/// `{base}/v1/systemone`, tolerating a base with or without a trailing slash or
/// an already-appended `/v1`.
pub(crate) fn systemone_url(base: &str) -> String {
    let trimmed = base.trim().trim_end_matches('/');
    if trimmed.ends_with("/v1/systemone") {
        return trimmed.to_string();
    }
    let origin = trimmed.strip_suffix("/v1").unwrap_or(trimmed);
    format!("{origin}/v1/systemone")
}

#[cfg(test)]
mod tests {
    use super::*;
    /// The gateway's URL is derived from the Future provider's base URL, which
    /// may or may not already carry a path suffix.
    #[test]
    fn the_systemone_url_is_derived_from_whatever_base_is_configured() {
        for base in [
            "https://future-os.cn/api",
            "https://future-os.cn/api/",
            "https://future-os.cn/api/v1",
            "https://future-os.cn/api/v1/systemone",
        ] {
            assert_eq!(
                systemone_url(base),
                "https://future-os.cn/api/v1/systemone",
                "base {base}"
            );
        }
        // A bare origin is fine too, and a staging host keeps its own path.
        assert_eq!(
            systemone_url("https://staging.example.com"),
            "https://staging.example.com/v1/systemone"
        );
        assert_eq!(
            systemone_url("https://staging.example.com/gw"),
            "https://staging.example.com/gw/v1/systemone"
        );
    }

    /// The credential store a signed-in install has, parsed from JSON so the test
    /// needs no HOME (and so cannot interfere with any other test).
    fn store(json: &str) -> crate::auth::AuthStore {
        crate::auth::AuthStore::from_json(json).expect("parses")
    }

    /// With no credential there is nothing to call: the feature is off, which the
    /// caller treats the same as "no recommendation".
    #[test]
    fn without_a_credential_the_feature_is_off() {
        assert!(
            resolve(&store("{}")).is_none(),
            "an install with no Future entry has no Jev endpoint"
        );
        // Another provider's key must not be borrowed for this one.
        assert!(resolve(&store(r#"{"openai":{"type":"api_key","key":"sk-x"}}"#)).is_none());
        // A Future entry with no key is the same as none.
        assert!(resolve(&store(r#"{"future":{"type":"api_key","key":""}}"#)).is_none());
    }

    /// A call goes to the Future account's gateway, authenticated by that
    /// account's credential — Jev has no separate key to configure. The base URL
    /// follows the provider entry, with the documented origin as the fallback.
    #[test]
    fn the_endpoint_is_the_future_accounts_gateway() {
        let with_base = resolve(&store(
            r#"{"future":{"type":"api_key","key":"acct-key","base_url":"https://future-os.cn/api"}}"#,
        ))
        .expect("the account credential is used");
        assert_eq!(with_base.key, "acct-key");
        assert_eq!(with_base.url, "https://future-os.cn/api/v1/systemone");
        assert_eq!(with_base.model, "jev");

        // No base URL configured: still usable, at the default origin.
        let no_base =
            resolve(&store(r#"{"future":{"type":"api_key","key":"acct-key"}}"#)).expect("resolves");
        assert_eq!(no_base.url, "https://future-os.cn/api/v1/systemone");

        // A non-production gateway keeps its own path.
        let staging = resolve(&store(
            r#"{"future":{"type":"api_key","key":"k","base_url":"https://staging.example.com/gw"}}"#,
        ))
        .expect("resolves");
        assert_eq!(staging.url, "https://staging.example.com/gw/v1/systemone");
    }
}
