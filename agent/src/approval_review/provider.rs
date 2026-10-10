use super::{
    budget,
    prompt::{build_jev_request, AUTHORIZATIONS, REASONS, RISKS},
    types::{Assessment, ProviderAssessment},
    PROBABILITY_SUM_TOLERANCE, REVIEW_TIMEOUT,
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, HashMap},
    sync::LazyLock,
    time::Duration,
};

static CLIENT: LazyLock<reqwest::Client> = LazyLock::new(|| {
    reqwest::Client::builder()
        .timeout(REVIEW_TIMEOUT)
        .build()
        .expect("System One HTTP client")
});
#[derive(Deserialize)]
struct Response {
    answers: HashMap<String, Choice>,
    #[serde(default)]
    model: Option<String>,
    #[serde(default, alias = "id")]
    request_id: Option<String>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Choice {
    #[serde(rename = "type")]
    #[serde(default)]
    kind: Option<String>,
    #[serde(default)]
    choice: Option<String>,
    probabilities: BTreeMap<String, f64>,
    #[serde(default)]
    confidence: Option<f64>,
}
pub(super) fn decode(value: Value) -> Result<ProviderAssessment, &'static str> {
    let response: Response = serde_json::from_value(value).map_err(|_| "invalid_response")?;
    if response
        .model
        .as_ref()
        .is_some_and(|model| model.is_empty())
        || response.answers.len() != 3
    {
        return Err("invalid_response");
    }
    let mut confidence = BTreeMap::new();
    let mut probabilities = serde_json::Map::new();
    let mut selected = Vec::new();
    for (name, options) in [
        ("risk", RISKS),
        ("authorization", AUTHORIZATIONS),
        ("reason_code", REASONS),
    ] {
        let answer = response.answers.get(name).ok_or("missing_choice")?;
        if answer.kind.as_deref().is_some_and(|kind| kind != "choice")
            || answer.probabilities.len() != options.len()
        {
            return Err("invalid_choice");
        }
        if options
            .iter()
            .any(|(key, _)| !answer.probabilities.contains_key(*key))
            || answer
                .probabilities
                .values()
                .any(|p| !p.is_finite() || !(0.0..=1.0).contains(p))
            || (answer.probabilities.values().sum::<f64>() - 1.0).abs() > PROBABILITY_SUM_TOLERANCE
        {
            return Err("invalid_probabilities");
        }
        // Rounded gateway distributions can have tied maxima. A supplied
        // choice may select either maximum; otherwise use stable key ordering.
        // Risk/reason ties cannot meet the threshold; authorization can
        // pass through the sum of permitted outcomes even when labels tie.
        let (maximum, maximum_p) = answer
            .probabilities
            .iter()
            .max_by(|a, b| a.1.total_cmp(b.1))
            .ok_or("missing_choice")?;
        let choice = answer.choice.as_ref().unwrap_or(maximum);
        let p = *answer.probabilities.get(choice).ok_or("invalid_choice")?;
        if p < *maximum_p {
            tracing::warn!(
                question = name,
                selected = %choice,
                selected_probability = p,
                maximum_probability = *maximum_p,
                "automatic review: provider choice does not match maximum probability"
            );
            return Err("inconsistent_choice");
        }
        let c = answer.confidence.unwrap_or(p);
        if !c.is_finite() || !(0.0..=1.0).contains(&c) {
            return Err("invalid_confidence");
        }
        confidence.insert(name.into(), c.min(p));
        probabilities.insert(name.into(), json!(answer.probabilities));
        selected.push(choice.clone());
    }
    Ok(ProviderAssessment {
        reported: Assessment {
            risk: selected[0].clone(),
            authorization: selected[1].clone(),
            reason_code: selected[2].clone(),
        },
        confidence,
        probabilities: Value::Object(probabilities),
        model: response.model.unwrap_or_else(|| "jev".into()),
        request_id: response.request_id,
    })
}

#[async_trait::async_trait]
pub trait ApprovalReviewerClient: Send + Sync {
    async fn assess(
        &self,
        state: Value,
        request_id: &str,
    ) -> Result<ProviderAssessment, &'static str>;
}
pub(super) struct JevReviewer;
#[async_trait::async_trait]
impl ApprovalReviewerClient for JevReviewer {
    async fn assess(
        &self,
        state: Value,
        request_id: &str,
    ) -> Result<ProviderAssessment, &'static str> {
        let endpoint = crate::system_one::endpoint().ok_or("reviewer_not_configured")?;
        assess_at(&endpoint, state, request_id).await
    }
}

pub(super) async fn assess_at(
    endpoint: &crate::system_one::Endpoint,
    state: Value,
    request_id: &str,
) -> Result<ProviderAssessment, &'static str> {
    let request = build_jev_request(state, &endpoint.model);
    budget::validate_request(&request)?;
    for attempt in 0..=1 {
        let response = crate::system_one::post(&CLIENT, endpoint, &request, request_id).await;
        match response {
            Ok(response) if response.status().is_success() => {
                // Bound the body while streaming, not after allocating an untrusted response.
                use futures_util::StreamExt;
                let mut stream = response.bytes_stream();
                let mut bytes = Vec::new();
                while let Some(chunk) = stream.next().await {
                    let chunk = chunk.map_err(|_| "reviewer_connection")?;
                    if bytes.len() + chunk.len() > 64 * 1024 {
                        return Err("response_too_large");
                    }
                    bytes.extend_from_slice(&chunk);
                }
                return decode(serde_json::from_slice(&bytes).map_err(|_| "invalid_response")?);
            }
            Ok(response) => {
                let status = response.status();
                if attempt == 0 && (status.as_u16() == 429 || status.is_server_error()) {
                    tokio::time::sleep(Duration::from_millis(200 + rand::random::<u8>() as u64))
                        .await;
                    continue;
                }
                return Err(if status.as_u16() == 401 || status.as_u16() == 403 {
                    "reviewer_auth"
                } else {
                    "reviewer_http"
                });
            }
            Err(error) => {
                if attempt == 0 && !error.is_builder() && !error.is_timeout() {
                    continue;
                }
                return Err(if error.is_timeout() {
                    "reviewer_timeout"
                } else {
                    "reviewer_connection"
                });
            }
        }
    }
    Err("reviewer_http")
}
