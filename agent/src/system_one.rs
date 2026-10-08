//! Shared Future System One transport; callers own their classification policy.
use crate::skill_reco::Endpoint;
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
