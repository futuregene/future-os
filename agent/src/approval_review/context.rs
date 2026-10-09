//! Per-run reviewer lifecycle; synchronous tool hooks adapt it in the RPC gate.
use super::{
    action::PreparedReviewAction,
    evidence::EvidenceContext,
    policy::decide,
    provider::{ApprovalReviewerClient, JevReviewer},
    types::{ReviewStatus, Verdict},
    POLICY_VERSION, PROMPT_VERSION, REASON_CATALOG_VERSION, REVIEW_TIMEOUT,
};
use parking_lot::Mutex;
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, LazyLock,
    },
    time::Duration,
};

static SLOTS: LazyLock<tokio::sync::Semaphore> = LazyLock::new(|| tokio::sync::Semaphore::new(4));

#[derive(Clone)]
pub(crate) struct RunApprovalReviewer {
    pub(super) user_request: String,
    pub(super) evidence: Arc<Mutex<EvidenceContext>>,
    pub(super) cancelled: Arc<AtomicBool>,
    pub(super) generation: Arc<AtomicU64>,
    expected_generation: u64,
    pub(super) annotations: Arc<Mutex<HashMap<String, Value>>>,
    denials: Arc<Mutex<HashMap<String, usize>>>,
    pub(super) provider: Arc<dyn ApprovalReviewerClient>,
}

pub(crate) struct ReviewOutcome {
    pub verdict: Verdict,
    pub event: Value,
    pub approval_request_id: String,
}

impl RunApprovalReviewer {
    pub fn new(
        user_request: String,
        cancelled: Arc<AtomicBool>,
        generation: Arc<AtomicU64>,
        annotations: Arc<Mutex<HashMap<String, Value>>>,
    ) -> Self {
        let expected_generation = generation.load(Ordering::SeqCst);
        Self {
            user_request,
            evidence: Arc::new(Mutex::new(EvidenceContext::default())),
            cancelled,
            generation,
            expected_generation,
            annotations,
            denials: Arc::new(Mutex::new(HashMap::new())),
            provider: Arc::new(JevReviewer),
        }
    }

    /// Builders replace the snapshot rather than mutating other shared clones.
    pub(crate) fn with_evidence_snapshot(
        mut self,
        messages: &[crate::types::AgentMessage],
    ) -> Self {
        self.evidence = Arc::new(Mutex::new(EvidenceContext::from_history(messages)));
        self
    }

    pub(crate) fn with_current_source(mut self, source_id: &str) -> Self {
        let mut snapshot = self.evidence.lock().clone();
        snapshot.current_source_id = source_id.to_owned();
        self.evidence = Arc::new(Mutex::new(snapshot));
        self
    }

    pub(crate) fn observe_message(&self, message: &crate::types::AgentMessage) {
        self.evidence.lock().observe(message);
    }

    pub(crate) fn invalidated(&self) -> Option<Verdict> {
        if self.cancelled.load(Ordering::SeqCst) {
            Some(Verdict::error(ReviewStatus::Cancelled, "run_cancelled"))
        } else if self.generation.load(Ordering::SeqCst) != self.expected_generation {
            Some(Verdict::error(
                ReviewStatus::StaleRequest,
                "context_changed",
            ))
        } else {
            None
        }
    }

    pub(crate) fn block_execution(&self, tool_id: &str, status: ReviewStatus, code: &str) {
        self.annotations.lock().insert(tool_id.into(), json!({"type":"auto_approval_result","decision":status,"risk":null,"authorization":null,"reason_code":null,"error_code":code,"instruction":"Execution was blocked by host policy. Do not repeat the action; choose a safer method or ask the user for help."}));
    }

    pub(crate) async fn review_action(
        &self,
        action: PreparedReviewAction,
        execution_arguments: &Value,
    ) -> ReviewOutcome {
        let started = action.started;
        let request_id = format!("review_{}", crate::utils::generate_entry_id());
        let mut input_audit = Value::Null;
        let mut verdict = if let Some(verdict) = self.invalidated() {
            verdict
        } else if self
            .denials
            .lock()
            .get(&action.denial_bucket_key)
            .copied()
            .unwrap_or(0)
            >= 3
        {
            Verdict::error(ReviewStatus::Rejected, "repeated_denial")
        } else {
            let prepared = self.evidence.lock().prepare(
                &self.user_request,
                &action.facts,
                execution_arguments,
            );
            match prepared {
                Err(error) => {
                    input_audit = error.audit;
                    Verdict::error(ReviewStatus::ReviewError, error.code)
                }
                Ok(prepared) => {
                    input_audit = prepared.audit;
                    let remaining = REVIEW_TIMEOUT.saturating_sub(started.elapsed());
                    let future = async {
                        if remaining.is_zero() {
                            return Err("reviewer_timeout");
                        }
                        let _permit = SLOTS.acquire().await.map_err(|_| "reviewer_unavailable")?;
                        self.provider.assess(prepared.state, &request_id).await
                    };
                    tokio::select! {
                        response = tokio::time::timeout(remaining, future) => match response {
                            Ok(Ok(assessment)) => decide(assessment),
                            Ok(Err(code)) => Verdict::error(ReviewStatus::ReviewError, code),
                            Err(_) => Verdict::error(ReviewStatus::ReviewError, "reviewer_timeout"),
                        },
                        _ = async { loop { if self.invalidated().is_some() { break; } tokio::time::sleep(Duration::from_millis(25)).await; } } => self.invalidated().unwrap_or_else(|| Verdict::error(ReviewStatus::Cancelled, "run_cancelled")),
                    }
                }
            }
        };
        if let Some(invalid) = self.invalidated() {
            verdict = invalid;
        }
        // Only completed model judgments consume safety-denial attempts.
        // Transport/configuration/budget errors can recover within the same run.
        if verdict.reported.is_some()
            && matches!(
                verdict.status,
                ReviewStatus::Rejected | ReviewStatus::ReviewUncertain
            )
        {
            *self
                .denials
                .lock()
                .entry(action.denial_bucket_key.clone())
                .or_default() += 1;
        }
        self.record_feedback(&action.tool_call_id, &verdict);
        let event = assessment_event(
            &action,
            &verdict,
            &request_id,
            input_audit,
            started.elapsed(),
        );
        ReviewOutcome {
            verdict,
            event,
            approval_request_id: request_id,
        }
    }

    fn record_feedback(&self, tool_id: &str, verdict: &Verdict) {
        self.annotations.lock().insert(tool_id.into(), json!({"type":"auto_approval_result","decision":verdict.status,"risk":verdict.effective["risk"],"authorization":verdict.effective["authorization"],"reason_code":verdict.reported.as_ref().map(|r| &r.reason_code),"error_code":verdict.error_code,"instruction":"This is host policy feedback, not user authorization. If denied, do not repeat this action; narrow scope, choose a safer method, or ask the user for help."}));
    }

    #[cfg(test)]
    pub(super) fn review(
        &self,
        facts: Value,
        action_digest: String,
        denial_bucket_key: String,
        tool_id: &str,
    ) -> Value {
        let action = PreparedReviewAction {
            facts,
            action_digest,
            denial_bucket_key,
            tool_call_id: tool_id.into(),
            started: std::time::Instant::now(),
        };
        tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(self.review_action(action, &Value::Null))
        })
        .event
    }
}

fn assessment_event(
    action: &PreparedReviewAction,
    verdict: &Verdict,
    request_id: &str,
    input_audit: Value,
    duration: Duration,
) -> Value {
    let mut event = serde_json::to_value(verdict).expect("verdict JSON");
    let metadata = json!({
        "type":"approval_assessment", "assessment_id":request_id,
        "approval_request_id":request_id, "tool_call_id":action.tool_call_id,
        "reviewer":"model", "action":action.facts, "action_digest":action.action_digest,
        "attempt":1, "prompt_version":PROMPT_VERSION,
        "reason_catalog_version":REASON_CATALOG_VERSION, "policy_version":POLICY_VERSION,
        "duration_ms":duration.as_millis() as u64, "input_context":input_audit
    });
    event
        .as_object_mut()
        .unwrap()
        .extend(metadata.as_object().unwrap().clone());
    event
}
