//! Experiment transport for direct model calls (A/B/C/M probe requests).
//!
//! Reads the local Agent registry so credentials stay in the user's own config:
//! nothing is printed, logged or copied. Requests outside the experiment
//! allowlist are refused, and there is no fallback model — a rejected call must
//! fail loudly rather than silently switch models mid-experiment.
//!
//! Protocol: one JSON object on stdin —
//!   {"model":"future/…","body":{…chat-completions request…}}
//!   {"mode":"metadata","model":"future/…"}   → model id/window/cost only
//! Raw upstream SSE is streamed to stdout.
//!
//! The request bound is 8 MB: a Codex-style strategy reads the whole live
//! history, which reaches ~4 MB of JSON on the experiment's late stages. The
//! response bound is unchanged.

use anyhow::{Context, Result};
use futures::StreamExt;
use std::io::{Read, Write};

const ALLOWED: [&str; 2] = ["future/deepseek-flash", "future/glm-5.3-flash"];
// Raised from 8192: a strategy that summarizes the whole history needs more
// room to finish its summary before the response hits the output limit.
// The pinned DeepSeek registry declares 384K output tokens. A fidelity run
// must not silently substitute the older experiment's 64K ceiling; admission
// reserves the complete requested bound before each call.
const MAX_OUTPUT_TOKENS: i64 = 384_000;
const MAX_REQUEST_BYTES: usize = 8_000_000;
const MAX_RESPONSE_BYTES: usize = 8_000_000;

#[tokio::main]
async fn main() -> Result<()> {
    let mut input = String::new();
    std::io::stdin().read_to_string(&mut input)?;
    let request: serde_json::Value = serde_json::from_str(&input)?;
    let name = request["model"].as_str().context("model required")?;
    anyhow::ensure!(
        ALLOWED.contains(&name),
        "model outside the experiment allowlist"
    );
    let registry = future_agent::models::Registry::new();
    let (model, key) = registry
        .resolve_for_request(name)
        .context("configured model unavailable; fallback prohibited")?;
    let target =
        future_agent::llm::schema::ResolvedModelTarget::from_model(&model, key, None, Some(8192))?;
    anyhow::ensure!(
        matches!(
            target.protocol,
            future_agent::llm::schema::ProtocolConfig::OpenAiChat(_)
        ),
        "chat protocol required"
    );
    if request["mode"] == "metadata" {
        println!(
            "{}",
            serde_json::json!({
                "id": model.id,
                "contextWindow": model.context_window,
                "maxTokens": model.max_tokens,
                "cost": {"input": model.cost.input, "output": model.cost.output},
            })
        );
        return Ok(());
    }
    let mut body = request["body"].clone();
    anyhow::ensure!(body.is_object(), "body required");
    body["model"] = serde_json::json!(target.model_id);
    body["stream"] = serde_json::json!(true);
    let limit = body["max_tokens"].as_i64().unwrap_or(0);
    anyhow::ensure!(
        (1..=MAX_OUTPUT_TOKENS).contains(&limit),
        "output limit invalid"
    );
    let encoded = serde_json::to_vec(&body)?;
    anyhow::ensure!(
        encoded.len() <= MAX_REQUEST_BYTES,
        "input byte bound exceeded"
    );
    let http = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(180))
        .build()?;
    let mut call = http
        .post(format!(
            "{}/chat/completions",
            target.route.base_url.trim_end_matches('/')
        ))
        .bearer_auth(target.route.api_key)
        .header("content-type", "application/json");
    for (key, value) in target.route.headers {
        call = call.header(key, value);
    }
    let response = call.body(encoded).send().await?;
    if !response.status().is_success() {
        // Include a bounded provider error body: without it a 400 is not
        // diagnosable in the ledger. Never echo credentials (the body is the
        // provider's own message).
        let status = response.status().as_u16();
        let body = response.text().await.unwrap_or_default();
        anyhow::bail!(
            "upstream HTTP {status}: {}",
            body.chars().take(600).collect::<String>()
        );
    }
    let mut stream = response.bytes_stream();
    let mut output = std::io::stdout();
    let mut count = 0;
    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        count += chunk.len();
        anyhow::ensure!(count <= MAX_RESPONSE_BYTES, "response bound exceeded");
        output.write_all(&chunk)?;
        output.flush()?;
    }
    Ok(())
}
