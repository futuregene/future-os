use super::budget::{validate_request, STATE_TARGET};
use super::prompt::{build_jev_request as request, AUTHORIZATIONS, REASONS, RISKS};
use super::provider::{assess_at, decode};
use super::*;
use parking_lot::Mutex;
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc,
    },
    time::Duration,
};
mod action;
mod evidence;
mod lifecycle;
mod policy;
mod provider;

fn action(
    tool: &str,
    tool_id: &str,
    arguments: &Value,
    shape: &Value,
    boundary: &Value,
    cwd: &str,
) -> (Value, String, String) {
    let action = prepare_review_action(tool, tool_id, arguments, shape, boundary, cwd);
    (action.facts, action.action_digest, action.denial_bucket_key)
}
fn assessment(risk: &str, authorization: &str, reason: &str) -> ProviderAssessment {
    ProviderAssessment {
        reported: Assessment {
            risk: risk.into(),
            authorization: authorization.into(),
            reason_code: reason.into(),
        },
        confidence: ["risk", "authorization", "reason_code"]
            .map(|key| (key.into(), 0.95))
            .into(),
        probabilities: json!({"authorization": AUTHORIZATIONS.iter().map(|(key,_)| (key.to_string(), json!(if *key == authorization {0.95} else {0.05/3.0}))).collect::<serde_json::Map<String,Value>>()}),
        model: "jev-fixture".into(),
        request_id: Some("provider-1".into()),
    }
}
fn response() -> Value {
    let mut answers = serde_json::Map::new();
    for (name, options) in [
        ("risk", RISKS),
        ("authorization", AUTHORIZATIONS),
        ("reason_code", REASONS),
    ] {
        answers.insert(name.into(),json!({"type":"choice","choice":options[0].0,"confidence":0.9,"probabilities":options.iter().enumerate().map(|(i,(key,_))| (key.to_string(),json!(if i==0 {1.0}else{0.0}))).collect::<serde_json::Map<String,Value>>()}));
    }
    json!({"model":"jev-fixture","answers":answers,"usage":{"input_tokens":123}})
}
fn prepared(ctx: &RunApprovalReviewer) -> super::evidence::PreparedInput {
    ctx.evidence
        .lock()
        .prepare(
            &ctx.user_request,
            &json!({"kind":"file_write","tool_call_id":"call"}),
            &Value::Null,
        )
        .unwrap()
}

struct Fake {
    calls: Arc<AtomicU64>,
    status: bool,
    hang: bool,
}
#[async_trait::async_trait]
impl ApprovalReviewerClient for Fake {
    async fn assess(&self, _: Value, _: &str) -> Result<ProviderAssessment, &'static str> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if self.hang {
            std::future::pending::<()>().await;
        }
        if self.status {
            Ok(assessment("medium", "high", "bounded_external_side_effect"))
        } else {
            Ok(assessment("high", "low", "broad_destructive_action"))
        }
    }
}
fn context(status: bool, hang: bool) -> (RunApprovalReviewer, Arc<AtomicU64>) {
    let calls = Arc::new(AtomicU64::new(0));
    let mut ctx = RunApprovalReviewer::new(
        "修改指定文件".into(),
        Arc::new(AtomicBool::new(false)),
        Arc::new(AtomicU64::new(0)),
        Arc::new(Mutex::new(HashMap::new())),
    );
    ctx.provider = Arc::new(Fake {
        calls: calls.clone(),
        status,
        hang,
    });
    (ctx, calls)
}
