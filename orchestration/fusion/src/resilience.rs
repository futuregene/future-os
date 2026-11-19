//! Versioned, host-independent failure and degradation policy.
use serde::{Deserialize, Serialize};

pub const RESILIENCE_POLICY_VERSION: u32 = 3;
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
    IdleTimeout,
    PanelTimeout,
    OutputLimit,
    InvalidOutput,
    ModelUnavailable,
    BudgetReserved,
    CircuitOpen,
    Cancelled,
    Persistence,
    Unknown,
}
impl FailureKind {
    /// Normalize the legacy Agent error boundary once; scheduling uses this enum.
    pub fn classify(message: &str) -> Self {
        let lower = message.to_lowercase();
        if lower.contains("persistencedegraded")
            || lower.contains("persistence worker")
            || lower.contains("persistence unavailable")
            || lower.contains("persistence failure")
            || lower.contains("event storage unavailable")
            || lower.contains("failed to spawn session")
        {
            Self::Persistence
        } else if lower.contains("agent service is stopping")
            || lower.contains("host background task admission is closed")
            || lower.contains("agent is shutting down")
        {
            Self::Cancelled
        } else if message.contains("[IDLE_TIMEOUT]")
            || (message.contains("[RESPONSE_TIMEOUT]") && lower.contains("idle_timeout"))
        {
            Self::IdleTimeout
        } else if message.contains("[PROVIDER_QUOTA]")
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
            "[RESPONSE_UNCONFIRMED]",
            "[MODEL_RESPONSE_ERROR] model_response_error",
            "[MODEL_RESPONSE_ERROR] finish_error",
            "HTTP 500",
            "HTTP 502",
            "HTTP 503",
            "HTTP 504",
        ]
        .iter()
        .any(|v| message.contains(v))
            || [
                "tls handshake eof",
                "connection reset",
                "connection refused",
                "error sending request",
                "unexpected eof",
                "grpc unavailable",
            ]
            .iter()
            .any(|v| lower.contains(v))
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
        let preview = public_preview(text, MAX_MATERIAL_BYTES);
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
/// Keep both public reasoning and the answer often placed at the end. Never
/// forward private reasoning or pretend a shortened report is complete.
fn public_preview(text: &str, max_bytes: usize) -> String {
    if text.len() <= max_bytes {
        return text.to_owned();
    }
    let marker = "\n[... public report shortened ...]\n";
    let room = max_bytes.saturating_sub(marker.len());
    let head = bounded_text(text, room * 2 / 3);
    let mut start = text.len().saturating_sub(room / 3);
    while !text.is_char_boundary(start) {
        start += 1;
    }
    format!("{head}{marker}{}", &text[start..])
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
    if config.inactivity_timeout_secs().is_some() {
        return 0;
    }
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
) -> anyhow::Result<Option<u64>> {
    if config.inactivity_timeout_secs().is_some() {
        return Ok(None);
    }
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
    Ok(Some(available.min(cap.saturating_mul(1000))))
}

impl crate::Consultation {
    /// The tool envelope used by every client and the outer Agent.
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
        if let Some(report) = &self.report {
            value["coverage"] = serde_json::json!({"analyses":report.analysis_participants.len(),"reviews":report.review_participants.len(),"skipped_phases":report.skipped_phases});
            let reasons: std::collections::BTreeSet<_> =
                report.degradations.iter().map(|d| &d.code).collect();
            if !reasons.is_empty() {
                value["degradations"] = serde_json::to_value(reasons)?;
            }
        }
        value["has_conclusion"] = serde_json::json!(self.result.is_some());
        if self.result.is_some() && matches!(self.status.as_str(), "delivering" | "completed") {
            value["continuation"] = serde_json::json!({
                "mode": "apply_conclusion",
                "guidance": crate::FOLLOW_THROUGH_POLICY,
            });
        }
        if self.result.is_none() {
            if let Some(report) = &self.report {
                value["available_results"] = serde_json::to_value(&report.available_results)?;
            }
            value["message"] = serde_json::json!("No valid Fusion conclusion. Available public participant reports, if any, are retained below as unverified attributed material, not consensus. Reuse applicable work and address only consequential unresolved gaps; do not automatically repeat this consultation. Full records remain in discussion details.");
        }
        Ok(serde_json::to_string(&value)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recorded_transport_storage_and_shutdown_errors_keep_their_origin() {
        for text in [
            "client error (Connect): tls handshake eof",
            "[MODEL_RESPONSE_ERROR] model_response_error: EngineCore encountered an issue",
            "[RESPONSE_UNCONFIRMED] finish_unknown",
        ] {
            assert_eq!(
                FailureKind::classify(text),
                FailureKind::Transient,
                "{text}"
            );
        }
        assert_eq!(
            FailureKind::classify("[RESPONSE_TIMEOUT] idle_timeout"),
            FailureKind::IdleTimeout
        );
        assert_eq!(
            FailureKind::classify(
                "failed to spawn session persistence worker: Resource temporarily unavailable"
            ),
            FailureKind::Persistence
        );
        assert_eq!(
            FailureKind::classify(
                "Agent readiness could not be verified: Agent service is stopping"
            ),
            FailureKind::Cancelled
        );
        assert_eq!(
            FailureKind::classify("[CONTENT_FILTER]"),
            FailureKind::Unknown
        );
    }

    #[test]
    fn a_failed_synthesis_delivers_attributed_public_material_without_a_conclusion() {
        let text = format!("{}\nFINAL_ANSWER: 6", "public derivation. ".repeat(200));
        let preview = public_preview(&text, MAX_MATERIAL_BYTES);
        assert!(preview.len() <= MAX_MATERIAL_BYTES);
        assert!(preview.starts_with("public derivation"));
        assert!(preview.ends_with("FINAL_ANSWER: 6"));
        let run: crate::Consultation = serde_json::from_value(serde_json::json!({
            "id":"discussion", "parent":{"database_id":"db", "session_id":"s", "run_id":"r", "tool_call_id":"t"},
            "input":{"question":"Q", "goal":"G", "background":"B"},
            "workspace":"", "snapshot_root":"", "status":"failed", "phase":"synthesizing", "completeness":"none",
            "result":null, "error":"moderator failed", "created_at":0, "updated_at":1,
            "report": {"version":RESILIENCE_POLICY_VERSION,"strategy":"single_advisor", "completed_phases":["analyzing"], "skipped_phases":[],
                "analysis_participants":["P3"],"review_participants":[],"degradations":[],
                "available_results":[{"step_id":"step", "phase":"analyzing", "participant":"P3", "model":"model", "text":preview, "truncated":true}]}
        })).unwrap();
        let output: serde_json::Value = serde_json::from_str(&run.tool_output().unwrap()).unwrap();
        assert_eq!(output["status"], "failed");
        assert_eq!(output["has_conclusion"], false);
        assert!(output["conclusion"].is_null());
        assert_eq!(output["available_results"][0]["participant"], "P3");
        assert!(output["available_results"][0]["text"]
            .as_str()
            .unwrap()
            .ends_with("FINAL_ANSWER: 6"));
    }
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
    fn active_discussions_have_no_wall_deadline_or_synthesis_time_reserve() {
        let c = crate::Configuration::default();
        for phase in [
            "preflight",
            "analyzing",
            "issue_mapping",
            "reviewing",
            "synthesizing",
        ] {
            assert_eq!(phase_timeout_ms(&c, phase, 0).unwrap(), None);
            assert_eq!(phase_timeout_ms(&c, phase, 86_400_000).unwrap(), None);
        }
        assert_eq!(summary_reserve_ms(&c), 0);
        assert_eq!(
            FailureKind::classify("[IDLE_TIMEOUT] no content"),
            FailureKind::IdleTimeout
        );
    }

    #[test]
    fn historical_deadline_policy_is_still_interpretable() {
        let c = crate::Configuration {
            version: 2,
            step_timeout_secs: 600,
            run_timeout_secs: 2700,
            idle_timeout_secs: None,
            ..Default::default()
        };
        let elapsed = c.run_timeout_secs * 1000 - summary_reserve_ms(&c);
        assert!(phase_timeout_ms(&c, "reviewing", elapsed).is_err());
        assert!(
            phase_timeout_ms(&c, "synthesizing", elapsed)
                .unwrap()
                .unwrap()
                > 0
        );
        assert_eq!(phase_timeout_ms(&c, "analyzing", 0).unwrap(), Some(600_000));
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
