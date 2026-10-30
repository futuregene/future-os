//! Versioned, host-independent failure and degradation policy.
use serde::{Deserialize, Serialize};

pub const RESILIENCE_POLICY_VERSION: u32 = 1;
pub const MAX_MATERIAL_BYTES: usize = 1400;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FailureKind {
    Authentication,
    Quota,
    Forbidden,
    RateLimited,
    Transient,
    Timeout,
    OutputLimit,
    InvalidOutput,
    ModelUnavailable,
    BudgetReserved,
    CircuitOpen,
    Cancelled,
    Unknown,
}
impl FailureKind {
    /// Normalize the legacy Agent error boundary once; scheduling uses this enum.
    pub fn classify(message: &str) -> Self {
        let lower = message.to_lowercase();
        if message.contains("[PROVIDER_QUOTA]")
            || lower.contains("insufficient_quota")
            || lower.contains("quota_exceeded")
            || lower.contains("billing_hard_limit_reached")
            || lower.contains("http 402")
        {
            Self::Quota
        } else if lower.contains("http 401") {
            Self::Authentication
        } else if lower.contains("http 403") {
            Self::Forbidden
        } else if lower.contains("http 429") {
            Self::RateLimited
        } else if message.contains("[OUTPUT_LIMIT]") || lower.contains("finish_length") {
            Self::OutputLimit
        } else if [
            "[UPSTREAM_DISCONNECTED]",
            "[RESPONSE_TIMEOUT]",
            "HTTP 500",
            "HTTP 502",
            "HTTP 503",
            "HTTP 504",
        ]
        .iter()
        .any(|v| message.contains(v))
        {
            Self::Transient
        } else if lower.contains("timed out") || lower.contains("timeout") {
            Self::Timeout
        } else if lower.contains("cancelled") || lower.contains("fusion stopped") {
            Self::Cancelled
        } else {
            Self::Unknown
        }
    }
    pub fn provider_wide(self) -> bool {
        matches!(self, Self::Authentication | Self::Quota)
    }
}

#[derive(Debug)]
pub struct ModelFailure {
    pub kind: FailureKind,
    pub message: String,
}
impl std::fmt::Display for ModelFailure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.message.fmt(formatter)
    }
}
impl std::error::Error for ModelFailure {}
pub fn failure(kind: FailureKind, message: impl Into<String>) -> anyhow::Error {
    ModelFailure {
        kind,
        message: message.into(),
    }
    .into()
}
pub fn failure_kind(error: &anyhow::Error) -> FailureKind {
    error
        .downcast_ref::<ModelFailure>()
        .map(|e| e.kind)
        .unwrap_or_else(|| FailureKind::classify(&format!("{error:#}")))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Degradation {
    pub code: String,
    pub phase: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub participant: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<FailureKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AvailableResult {
    pub step_id: String,
    pub phase: String,
    pub participant: String,
    pub model: String,
    pub text: String,
    pub truncated: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionReport {
    pub version: u32,
    pub strategy: String,
    pub completed_phases: Vec<String>,
    pub skipped_phases: Vec<String>,
    pub analysis_participants: Vec<String>,
    pub review_participants: Vec<String>,
    pub degradations: Vec<Degradation>,
    /// Bounded PUBLIC answers only. Full answers remain in child transcripts.
    pub available_results: Vec<AvailableResult>,
}
impl Default for ExecutionReport {
    fn default() -> Self {
        Self {
            version: RESILIENCE_POLICY_VERSION,
            strategy: "standard".into(),
            completed_phases: vec![],
            skipped_phases: vec![],
            analysis_participants: vec![],
            review_participants: vec![],
            degradations: vec![],
            available_results: vec![],
        }
    }
}
impl ExecutionReport {
    pub fn note(
        &mut self,
        code: &str,
        phase: &str,
        participant: Option<&str>,
        kind: Option<FailureKind>,
    ) {
        if self.degradations.len() < 24
            && !self.degradations.iter().any(|v| {
                v.code == code && v.phase == phase && v.participant.as_deref() == participant
            })
        {
            self.degradations.push(Degradation {
                code: code.into(),
                phase: phase.into(),
                participant: participant.map(str::to_owned),
                kind,
                provider: None,
            });
        }
    }
    pub fn complete(&mut self, phase: &str) {
        if !self.completed_phases.iter().any(|v| v == phase) {
            self.completed_phases.push(phase.into());
        }
    }
    pub fn skip(&mut self, phase: &str) {
        if !self.skipped_phases.iter().any(|v| v == phase) {
            self.skipped_phases.push(phase.into());
        }
    }
    pub fn retain(&mut self, step: &crate::Step, text: &str) {
        if self.available_results.len() >= 10 {
            return;
        }
        let preview = bounded_text(text, MAX_MATERIAL_BYTES);
        self.available_results.push(AvailableResult {
            step_id: step.id.clone(),
            phase: step.phase.clone(),
            participant: step.participant.clone(),
            model: step.model.clone(),
            truncated: preview.len() < text.len(),
            text: preview,
        });
    }
}
pub fn bounded_text(text: &str, max_bytes: usize) -> String {
    let mut end = text.len().min(max_bytes);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    text[..end].to_owned()
}

/// Reserve one summary plus its possible format/output retry, bounded for short runs.
pub fn summary_reserve_ms(config: &crate::Configuration) -> u64 {
    config
        .step_timeout_secs
        .saturating_mul(2000)
        .min(config.run_timeout_secs.saturating_mul(1000) / 3)
        .max(1)
}
pub fn phase_timeout_ms(
    config: &crate::Configuration,
    phase: &str,
    elapsed_ms: u64,
) -> anyhow::Result<u64> {
    let remaining = config
        .run_timeout_secs
        .saturating_mul(1000)
        .saturating_sub(elapsed_ms);
    let available = if phase == "synthesizing" {
        remaining
    } else {
        remaining.saturating_sub(summary_reserve_ms(config))
    };
    if available == 0 {
        return Err(failure(
            FailureKind::BudgetReserved,
            "Fusion time reserved for final synthesis",
        ));
    }
    let cap = if phase == "preflight" {
        config.step_timeout_secs.min(120)
    } else {
        config.step_timeout_secs
    };
    Ok(available.min(cap.saturating_mul(1000)))
}

impl crate::Consultation {
    /// The one bounded tool envelope used by every client and the outer Agent.
    pub fn tool_output(&self) -> anyhow::Result<String> {
        let mut value = if let Some(result) = &self.result {
            serde_json::to_value(result)?
        } else {
            serde_json::json!({"fusion_run_id": self.id, "conclusion": null, "completeness": "none"})
        };
        value["status"] = serde_json::json!(if self.status == "delivering" {
            "completed"
        } else {
            &self.status
        });
        value["report"] = serde_json::to_value(&self.report)?;
        value["has_conclusion"] = serde_json::json!(self.result.is_some());
        if self.result.is_none() {
            value["message"] = serde_json::json!("No valid Fusion conclusion was produced. Preserved public materials are evidence, not a synthesized or verified recommendation. Do not automatically repeat the whole consultation.");
        }
        let mut output = serde_json::to_string(&value)?;
        // Keep evidence identities even when escaped public text exceeds the
        // wire bound. Full originals remain queryable by these step IDs.
        for limit in [700, 300, 0] {
            if output.len() <= crate::MAX_RESULT_BYTES {
                break;
            }
            if let Some(items) = value["report"]["available_results"].as_array_mut() {
                for item in items {
                    let text = item["text"].as_str().unwrap_or_default();
                    let preview = bounded_text(text, limit);
                    if preview.len() < text.len() {
                        item["truncated"] = serde_json::json!(true);
                    }
                    item["text"] = serde_json::json!(preview);
                }
            }
            output = serde_json::to_string(&value)?;
        }
        anyhow::ensure!(
            output.len() <= crate::MAX_RESULT_BYTES,
            "Fusion tool envelope exceeds bound"
        );
        Ok(output)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn quota_is_not_retried_as_a_rate_limit_and_forbidden_is_model_scoped() {
        assert_eq!(
            FailureKind::classify("HTTP 429 insufficient_quota"),
            FailureKind::Quota
        );
        assert_eq!(
            FailureKind::classify("HTTP 429 rate limited"),
            FailureKind::RateLimited
        );
        assert!(!FailureKind::classify("HTTP 403 restricted model").provider_wide());
        assert!(FailureKind::classify("HTTP 401 invalid key").provider_wide());
    }
    #[test]
    fn status_code_takes_precedence_over_generic_timeout_wording() {
        assert_eq!(
            FailureKind::classify("HTTP 503: upstream timeout"),
            FailureKind::Transient
        );
        assert_eq!(
            FailureKind::classify("[RESPONSE_TIMEOUT]"),
            FailureKind::Transient
        );
        assert_eq!(
            FailureKind::classify("Fusion model timed out"),
            FailureKind::Timeout
        );
    }
    #[test]
    fn summary_budget_is_not_available_to_earlier_phases() {
        let c = crate::Configuration::default();
        let elapsed = c.run_timeout_secs * 1000 - summary_reserve_ms(&c);
        assert!(phase_timeout_ms(&c, "reviewing", elapsed).is_err());
        assert!(phase_timeout_ms(&c, "synthesizing", elapsed).unwrap() > 0);
        assert!(phase_timeout_ms(&c, "synthesizing", c.run_timeout_secs * 1000).is_err());
    }
    #[test]
    fn public_previews_are_utf8_bounded_and_reasons_are_deduplicated() {
        assert_eq!(bounded_text("中文材料", 7), "中文");
        let mut report = ExecutionReport::default();
        report.note(
            "participant_failed",
            "analyzing",
            Some("P3"),
            Some(FailureKind::Timeout),
        );
        report.note(
            "participant_failed",
            "analyzing",
            Some("P3"),
            Some(FailureKind::Timeout),
        );
        assert_eq!(report.degradations.len(), 1);
    }
}
