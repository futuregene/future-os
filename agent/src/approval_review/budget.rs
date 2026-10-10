use serde_json::{json, Value};

pub(super) const STATE_TARGET: usize = 6_000;
pub(super) const STATE_LIMIT: usize = 8_000;
pub(super) const QUESTIONS_LIMIT: usize = 4_000;
pub(super) const REQUEST_LIMIT: usize = 12_000;
pub(super) const PAIR_LIMIT: usize = 30_000; // Headroom below the upstream 32K pair limit.
pub(super) const TOTAL_LIMIT: usize = 62_000; // Headroom below the upstream 64K request limit.
pub(super) const ESTIMATOR: &str = "serialized_utf8_bytes_conservative_v1";

// Without a Jev tokenizer, charge every serialized UTF-8 byte as one estimated
// token, including JSON escaping/framing. This deliberately underuses capacity;
// it is not a measured tokenizer count or a guarantee about gateway-added text.
pub(super) fn estimate(value: &Value) -> usize {
    value.to_string().len()
}

pub(super) fn validate_request(request: &Value) -> Result<(), &'static str> {
    let state = estimate(&request["state"]);
    let questions = request["questions"].as_object().ok_or("invalid_request")?;
    let longest = questions.values().map(estimate).max().unwrap_or(0);
    let all_questions = estimate(&request["questions"]);
    if state > STATE_LIMIT
        || all_questions > QUESTIONS_LIMIT
        || estimate(request) > REQUEST_LIMIT
        || state.saturating_add(longest) > PAIR_LIMIT
        || state.saturating_add(all_questions) > TOTAL_LIMIT
    {
        Err("input_too_large")
    } else {
        Ok(())
    }
}

pub(super) fn audit(request: &Value) -> Value {
    let state = estimate(&request["state"]);
    let questions = estimate(&request["questions"]);
    let longest = request["questions"]
        .as_object()
        .map(|questions| questions.values().map(estimate).max().unwrap_or(0))
        .unwrap_or(0);
    json!({"estimator":ESTIMATOR,"estimated_state_tokens":state,
        "estimated_questions_tokens":questions,"estimated_request_tokens":estimate(request),
        "estimated_longest_pair_tokens":state.saturating_add(longest),
        "estimated_total_tokens":state.saturating_add(questions),
        "state_target":STATE_TARGET,"state_limit":STATE_LIMIT,"questions_limit":QUESTIONS_LIMIT,
        "request_limit":REQUEST_LIMIT,"pair_limit":PAIR_LIMIT,"total_limit":TOTAL_LIMIT})
}
