use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Assessment {
    pub risk: String,
    pub authorization: String,
    pub reason_code: String,
}

#[derive(Debug, Clone)]
pub struct ProviderAssessment {
    pub reported: Assessment,
    pub confidence: BTreeMap<String, f64>,
    pub probabilities: Value,
    pub model: String,
    pub request_id: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewStatus {
    Approved,
    Rejected,
    ReviewUncertain,
    ReviewError,
    Cancelled,
    StaleRequest,
}
impl ReviewStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Approved => "approved",
            Self::Rejected => "rejected",
            Self::ReviewUncertain => "review_uncertain",
            Self::ReviewError => "review_error",
            Self::Cancelled => "cancelled",
            Self::StaleRequest => "stale_request",
        }
    }
}
#[cfg(test)]
impl PartialEq<&str> for ReviewStatus {
    fn eq(&self, other: &&str) -> bool {
        self.as_str() == *other
    }
}
#[derive(Debug, Clone, Serialize)]
pub struct Verdict {
    pub status: ReviewStatus,
    pub reported: Option<Assessment>,
    pub effective: Value,
    pub confidence: BTreeMap<String, f64>,
    pub probabilities: Value,
    pub model: Option<String>,
    pub provider_request_id: Option<String>,
    pub error_code: Option<String>,
}
impl Verdict {
    pub(super) fn error(status: ReviewStatus, code: &str) -> Self {
        Self {
            status,
            reported: None,
            effective: json!({"risk": null, "authorization": null}),
            confidence: BTreeMap::new(),
            probabilities: Value::Null,
            model: None,
            provider_request_id: None,
            error_code: Some(code.into()),
        }
    }
    pub fn approved(&self) -> bool {
        self.status == ReviewStatus::Approved
    }
}
