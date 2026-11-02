//! One-shot endpoint probe for the custom-provider editor.
//!
//! The editor wants to offer a "detect" button: try each thinking level against
//! the provider the user is *editing* (nothing is saved yet) and keep the ones
//! the endpoint accepts. Doing that here — instead of in the GUI — is what makes
//! the answer true: the request is built by the same adapters, from the same
//! model shape, that a real call uses, so a level reported as accepted is one
//! whose outbound parameter really is accepted.
//!
//! Two details make the probe answer the right question:
//!
//! * The calibration call (`default`) sends *no* thinking parameter. If it
//!   fails, the endpoint/key/model id is wrong and no level is worth testing —
//!   the caller reports that failure instead of a half-filled level list.
//! * A request that the provider answered at all is an accepted level. Only
//!   transport failures, non-2xx statuses and error frames count as rejection,
//!   because the probe cancels the stream as soon as it sees the first frame:
//!   the level is the subject, not the model's prose.

use super::schema::{ModelRequest, ModelStreamEvent, ResolvedModelTarget};
use super::Client;
use crate::types::{AgentMessage, LLMProvider};
use anyhow::{anyhow, Result};
use futures::StreamExt;
use std::time::Duration;

/// The prompt the probe sends. Deliberately trivial: the model has to answer,
/// but its answer is discarded.
const PROBE_PROMPT: &str = "ping";

/// Token ceiling for one probe request on the chat-completions / responses
/// APIs. Small on purpose — the probe cancels the stream at the first frame, so
/// this only bounds what a chatty provider can bill before that.
const PROBE_MAX_OUTPUT_TOKENS: i32 = 1024;

/// Anthropic takes a thinking *budget* instead: the adapter clamps it below
/// `max_tokens`, and a budget under 1024 disables thinking entirely (which would
/// make every level look accepted for the wrong reason). 4096 leaves room for
/// the clamped budget plus an answer.
const PROBE_ANTHROPIC_MAX_OUTPUT_TOKENS: i32 = 4096;

/// Context window reported to the adapters. Large enough that nothing clamps;
/// the probe sends one short prompt.
const PROBE_CONTEXT_WINDOW: i32 = 200_000;

/// How long one probe waits for the provider's first stream frame. A provider
/// that accepts the request but never speaks is a failure, not a hanging RPC:
/// the shared per-command deadline is sized above this one so the call always
/// reports the probe's own reason first.
pub const PROBE_FIRST_FRAME_TIMEOUT: Duration = Duration::from_secs(30);

/// The reserved level meaning "send no thinking/effort parameter at all".
pub use super::schema::DEFAULT_THINKING_LEVEL;

/// What the caller asks one probe to do: one level, against the provider/model
/// the caller is editing.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ThinkingLevelProbeSpec {
    /// Wire protocol: `openai-completions`, `openai-responses` or `anthropic`.
    pub api: String,
    pub base_url: String,
    pub api_key: String,
    /// Provider / model id, used only to inherit an already-saved model's wire
    /// settings (`compat`, `headers`) that the editor cannot express.
    pub provider_id: String,
    pub model_id: String,
    /// The level to test; empty means the calibration call.
    pub level: String,
}

/// The wire carrier is the proto message; field-for-field, nothing derived, so
/// the mapping lives next to the spec it fills.
impl From<future_rpc::proto::ThinkingLevelProbe> for ThinkingLevelProbeSpec {
    fn from(probe: future_rpc::proto::ThinkingLevelProbe) -> Self {
        Self {
            api: probe.api,
            base_url: probe.base_url,
            api_key: probe.api_key,
            provider_id: probe.provider_id,
            model_id: probe.model_id,
            level: probe.level,
        }
    }
}

/// One probe's verdict.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ThinkingLevelProbeResult {
    /// `default` for the calibration call, else the requested level.
    pub level: String,
    /// True when the provider accepted the request carrying this level.
    pub ok: bool,
    /// The provider's own message when it did not. Absent when `ok`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// The model shape a probe sends with.
///
/// `inherited` is the already-saved model with the same provider/id, when there
/// is one: the custom-provider editor cannot edit `compat` (Qwen's
/// `thinkingFormat`, a gateway's `reasoningEffort` vocabulary, a
/// `max_completion_tokens` field name) or custom `headers`, and a save keeps
/// them. Sending without them would test a different request than the one the
/// agent will make once the form is saved.
pub fn probe_model(
    spec: &ThinkingLevelProbeSpec,
    inherited: Option<&crate::models::Model>,
) -> crate::models::Model {
    let api = spec.api.trim().to_string();
    let mut model = crate::models::Model {
        id: spec.model_id.trim().to_string(),
        name: spec.model_id.trim().to_string(),
        provider: if spec.provider_id.trim().is_empty() {
            "probe".to_string()
        } else {
            spec.provider_id.trim().to_string()
        },
        max_tokens: if api == "anthropic" {
            PROBE_ANTHROPIC_MAX_OUTPUT_TOKENS
        } else {
            PROBE_MAX_OUTPUT_TOKENS
        },
        api,
        base_url: spec.base_url.trim().to_string(),
        input: vec!["text".to_string()],
        output: vec!["text".to_string()],
        context_window: PROBE_CONTEXT_WINDOW,
        ..Default::default()
    };
    if let Some(inherited) = inherited {
        model.compat = inherited.compat.clone();
        model.headers = inherited.headers.clone();
    }
    // Every level is declared so capabilities say "this model reasons": the
    // Anthropic adapter only requests thinking for a model whose declared
    // levels include a real one, and a probe that silently dropped the thinking
    // block would report every level accepted for the wrong reason.
    model.thinking_levels = super::schema::KNOWN_THINKING_LEVELS
        .iter()
        .map(|level| (*level).to_string())
        .collect();
    model
}

/// Run one probe against `model` and report whether the endpoint accepted it.
///
/// Blocking on purpose: RPC handlers run on blocking worker threads, and the
/// session's own model calls are driven the same way (see `ServerSession`'s
/// compaction path), so the caller needs no runtime of its own.
pub fn probe_thinking_level(
    model: &crate::models::Model,
    api_key: &str,
    level: &str,
) -> ThinkingLevelProbeResult {
    let level = if level.trim().is_empty() {
        DEFAULT_THINKING_LEVEL.to_string()
    } else {
        level.trim().to_string()
    };
    match probe_blocking(model, api_key, &level) {
        Ok(()) => ThinkingLevelProbeResult {
            level,
            ok: true,
            error: None,
        },
        Err(error) => ThinkingLevelProbeResult {
            level,
            ok: false,
            error: Some(format!("{error:#}")),
        },
    }
}

fn probe_blocking(model: &crate::models::Model, api_key: &str, level: &str) -> Result<()> {
    if model.id.is_empty() {
        return Err(anyhow!("model id is empty"));
    }
    if model.base_url.is_empty() {
        return Err(anyhow!("base URL is empty"));
    }
    let mut target =
        ResolvedModelTarget::from_model(model, api_key.to_string(), None, Some(model.max_tokens))?;
    // Exactly what a session sets for the level, so a level that passes here
    // cannot be refused later by the budget/effort split.
    target.generation.thinking_level = level.to_string();
    target.generation.thinking_budget = crate::rpc::thinking_budget_for(level);

    let client = Client::from_target(target);
    let request = ModelRequest {
        model: model.id.clone(),
        system_prompt: String::new(),
        messages: vec![AgentMessage::new_user(
            "user",
            serde_json::json!(PROBE_PROMPT),
        )],
        tools: Vec::new(),
    };
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| anyhow!("probe runtime unavailable: {error}"))?;
    runtime.block_on(async move {
        let mut stream = client.stream_model(request).await?;
        // The first frame decides. A provider that accepted the request and
        // started answering took the thinking parameter with it; anything else
        // (an error frame, silence past the deadline) is a rejection.
        match tokio::time::timeout(PROBE_FIRST_FRAME_TIMEOUT, stream.next()).await {
            Ok(Some(ModelStreamEvent::Error { message })) => Err(anyhow!("{message}")),
            Ok(Some(_)) | Ok(None) => Ok(()),
            Err(_) => Err(anyhow!(
                "no response within {}s",
                PROBE_FIRST_FRAME_TIMEOUT.as_secs()
            )),
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::sync::{Arc, Mutex};

    /// A one-connection-per-request mock endpoint: it records the request body
    /// and answers with whatever the test queued for it.
    struct MockEndpoint {
        base_url: String,
        bodies: Arc<Mutex<Vec<serde_json::Value>>>,
    }

    impl MockEndpoint {
        /// `respond` receives each request body (parsed) and returns
        /// `(status, content_type, body)`.
        fn start(
            respond: impl Fn(&serde_json::Value) -> (u16, &'static str, String) + Send + 'static,
        ) -> Self {
            crate::test_support::ensure_fd_headroom();
            let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
            let port = listener.local_addr().unwrap().port();
            let bodies = Arc::new(Mutex::new(Vec::new()));
            let captured = bodies.clone();
            std::thread::spawn(move || {
                for _ in 0..32 {
                    let Ok((mut stream, _)) = listener.accept() else {
                        return;
                    };
                    let mut buf = Vec::new();
                    let mut chunk = [0u8; 8192];
                    let mut header_end = None;
                    while header_end.is_none() {
                        match stream.read(&mut chunk) {
                            Ok(0) | Err(_) => break,
                            Ok(n) => {
                                buf.extend_from_slice(&chunk[..n]);
                                header_end = buf
                                    .windows(4)
                                    .position(|window| window == b"\r\n\r\n")
                                    .map(|at| at + 4);
                            }
                        }
                    }
                    let Some(header_end) = header_end else {
                        continue;
                    };
                    let headers = String::from_utf8_lossy(&buf[..header_end]).to_string();
                    let length: usize = headers
                        .lines()
                        .find_map(|line| {
                            line.to_ascii_lowercase()
                                .strip_prefix("content-length:")
                                .and_then(|value| value.trim().parse().ok())
                        })
                        .unwrap_or(0);
                    while buf.len() < header_end + length {
                        match stream.read(&mut chunk) {
                            Ok(0) | Err(_) => break,
                            Ok(n) => buf.extend_from_slice(&chunk[..n]),
                        }
                    }
                    let body = String::from_utf8_lossy(&buf[header_end..]).to_string();
                    let parsed = serde_json::from_str::<serde_json::Value>(&body)
                        .unwrap_or(serde_json::Value::Null);
                    captured.lock().unwrap().push(parsed.clone());
                    let (status, content_type, payload) = respond(&parsed);
                    let _ = stream.write_all(
                        format!(
                            "HTTP/1.1 {status} X\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{payload}",
                            payload.len()
                        )
                        .as_bytes(),
                    );
                    let _ = stream.flush();
                }
            });
            Self {
                base_url: format!("http://127.0.0.1:{port}"),
                bodies,
            }
        }

        fn bodies(&self) -> Vec<serde_json::Value> {
            self.bodies.lock().unwrap().clone()
        }
    }

    fn spec(api: &str, base_url: &str, level: &str) -> ThinkingLevelProbeSpec {
        ThinkingLevelProbeSpec {
            api: api.to_string(),
            base_url: base_url.to_string(),
            api_key: "sk-test".to_string(),
            provider_id: "acme".to_string(),
            model_id: "qwen".to_string(),
            level: level.to_string(),
        }
    }

    /// A minimal Chat Completions stream: one content delta, then the
    /// terminator.
    const CHAT_SSE: &str = concat!(
        "data: {\"choices\":[{\"delta\":{\"content\":\"pong\"}}]}\n\n",
        "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n",
        "data: [DONE]\n\n",
    );

    #[test]
    fn the_calibration_level_sends_no_thinking_parameter() {
        let endpoint = MockEndpoint::start(|_| (200, "text/event-stream", CHAT_SSE.to_string()));
        let model = probe_model(&spec("openai-completions", &endpoint.base_url, ""), None);
        // An empty level *is* the calibration call, and it is reported under its
        // own name so the GUI can tell it apart from a real level.
        let result = probe_thinking_level(&model, "sk-test", "");
        assert!(result.ok, "{result:?}");
        assert_eq!(result.level, DEFAULT_THINKING_LEVEL);

        let body = endpoint.bodies()[0].clone();
        assert_eq!(body["model"], "qwen");
        assert!(body.get("reasoning_effort").is_none(), "{body}");
        assert!(body.get("thinking").is_none(), "{body}");
        assert!(body.get("enable_thinking").is_none(), "{body}");
    }

    #[test]
    fn a_level_travels_as_the_reasoning_effort_value() {
        let endpoint = MockEndpoint::start(|_| (200, "text/event-stream", CHAT_SSE.to_string()));
        let model = probe_model(
            &spec("openai-completions", &endpoint.base_url, "high"),
            None,
        );
        let result = probe_thinking_level(&model, "sk-test", "high");
        assert!(result.ok, "{result:?}");
        assert_eq!(endpoint.bodies()[0]["reasoning_effort"], "high");
    }

    /// A provider that refuses the level answers 400; that is an answer about
    /// the level, not a broken probe — `ok:false` with the provider's message.
    #[test]
    fn a_rejected_level_reports_the_provider_message() {
        let endpoint = MockEndpoint::start(|_| {
            (
                400,
                "application/json",
                r#"{"error":{"message":"Unsupported value: 'reasoning_effort' does not support 'high'"}}"#
                    .to_string(),
            )
        });
        let model = probe_model(
            &spec("openai-completions", &endpoint.base_url, "high"),
            None,
        );
        let result = probe_thinking_level(&model, "sk-test", "high");
        assert!(!result.ok);
        assert!(
            result
                .error
                .as_deref()
                .unwrap()
                .contains("does not support 'high'"),
            "{result:?}"
        );
    }

    /// The calibration call failing is how "the endpoint / key / model id is
    /// wrong" surfaces, so the message must name the reason rather than read as
    /// a thinking-level verdict.
    #[test]
    fn a_failed_calibration_call_reports_the_http_failure() {
        let endpoint = MockEndpoint::start(|_| {
            (
                401,
                "application/json",
                r#"{"error":{"message":"invalid api key"}}"#.to_string(),
            )
        });
        let model = probe_model(&spec("openai-completions", &endpoint.base_url, ""), None);
        let result = probe_thinking_level(&model, "bad-key", DEFAULT_THINKING_LEVEL);
        assert!(!result.ok);
        let error = result.error.unwrap();
        assert!(error.contains("401"), "{error}");
        assert!(error.contains("invalid api key"), "{error}");
    }

    /// An inherited `compat` is what the saved model will actually send, so the
    /// probe has to send it too: here the level must arrive under the declared
    /// parameter, not at the default one.
    #[test]
    fn an_inherited_compat_shapes_the_request() {
        let endpoint = MockEndpoint::start(|_| (200, "text/event-stream", CHAT_SSE.to_string()));
        let mut inherited = crate::models::Model {
            id: "qwen".into(),
            provider: "acme".into(),
            ..Default::default()
        };
        inherited.compat.insert(
            "reasoningEffort".into(),
            serde_json::json!({"values": ["low", "high", "xhigh", "max"], "param": "chat_template_kwargs.reasoning_effort"}),
        );
        let model = probe_model(
            &spec("openai-completions", &endpoint.base_url, "high"),
            Some(&inherited),
        );
        let result = probe_thinking_level(&model, "sk-test", "high");
        assert!(result.ok, "{result:?}");
        let body = endpoint.bodies()[0].clone();
        assert_eq!(
            body["chat_template_kwargs"]["reasoning_effort"], "high",
            "{body}"
        );
        assert!(body.get("reasoning_effort").is_none(), "{body}");
    }

    #[test]
    fn anthropic_probes_send_a_thinking_budget() {
        let endpoint = MockEndpoint::start(|_| {
            (
                200,
                "text/event-stream",
                concat!(
                    "event: content_block_delta\n",
                    "data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"pong\"}}\n\n",
                    "event: message_stop\n",
                    "data: {\"type\":\"message_stop\"}\n\n",
                )
                .to_string(),
            )
        });
        let model = probe_model(&spec("anthropic", &endpoint.base_url, "low"), None);
        let result = probe_thinking_level(&model, "sk-test", "low");
        assert!(result.ok, "{result:?}");
        let body = endpoint.bodies()[0].clone();
        assert_eq!(body["thinking"]["type"], "enabled", "{body}");
        assert!(
            body["thinking"]["budget_tokens"].as_i64().unwrap() >= 1024,
            "{body}"
        );
    }

    /// An unknown protocol is rejected before any request goes out, so a typo
    /// in the form cannot look like "the endpoint refused this level".
    #[test]
    fn an_unknown_api_is_reported_without_a_request() {
        let endpoint = MockEndpoint::start(|_| (200, "text/event-stream", CHAT_SSE.to_string()));
        let model = probe_model(&spec("not-a-protocol", &endpoint.base_url, "low"), None);
        let result = probe_thinking_level(&model, "sk-test", "low");
        assert!(!result.ok);
        assert!(
            result.error.as_deref().unwrap().contains("not-a-protocol"),
            "{result:?}"
        );
        assert!(endpoint.bodies().is_empty());
    }

    #[test]
    fn a_stream_error_frame_is_a_rejection() {
        let endpoint = MockEndpoint::start(|_| {
            (
                200,
                "text/event-stream",
                "data: {\"error\":{\"message\":\"unsupported reasoning effort\"}}\n\n".to_string(),
            )
        });
        let model = probe_model(&spec("openai-completions", &endpoint.base_url, "max"), None);
        let result = probe_thinking_level(&model, "sk-test", "max");
        assert!(!result.ok, "{result:?}");
        assert!(
            result
                .error
                .as_deref()
                .unwrap()
                .contains("unsupported reasoning effort"),
            "{result:?}"
        );
    }

    #[test]
    fn the_wire_probe_maps_field_for_field_onto_the_spec() {
        let spec: ThinkingLevelProbeSpec = future_rpc::proto::ThinkingLevelProbe {
            api: "anthropic".to_string(),
            base_url: "https://api.example.test".to_string(),
            api_key: "sk-key".to_string(),
            provider_id: "acme".to_string(),
            model_id: "claude-ish".to_string(),
            level: "max".to_string(),
        }
        .into();
        assert_eq!(
            spec,
            ThinkingLevelProbeSpec {
                api: "anthropic".to_string(),
                base_url: "https://api.example.test".to_string(),
                api_key: "sk-key".to_string(),
                provider_id: "acme".to_string(),
                model_id: "claude-ish".to_string(),
                level: "max".to_string(),
            }
        );
    }

    #[test]
    fn a_probe_model_declares_the_known_levels_and_bounds_its_output() {
        let model = probe_model(&spec("anthropic", "https://example.test", "low"), None);
        assert_eq!(
            model.thinking_levels.len(),
            super::super::schema::KNOWN_THINKING_LEVELS.len()
        );
        assert_eq!(model.max_tokens, PROBE_ANTHROPIC_MAX_OUTPUT_TOKENS);
        let chat = probe_model(
            &spec("openai-completions", "https://example.test", "low"),
            None,
        );
        assert_eq!(chat.max_tokens, PROBE_MAX_OUTPUT_TOKENS);
        assert_eq!(chat.input, vec!["text".to_string()]);
    }

    #[test]
    fn a_model_without_base_url_or_id_is_rejected() {
        let mut model = probe_model(&spec("openai-completions", "", "low"), None);
        assert!(!probe_thinking_level(&model, "sk-test", "low").ok);
        model.base_url = "https://example.test".into();
        model.id.clear();
        let result = probe_thinking_level(&model, "sk-test", "low");
        assert!(!result.ok);
        assert!(
            result.error.as_deref().unwrap().contains("model id"),
            "{result:?}"
        );
    }
}
