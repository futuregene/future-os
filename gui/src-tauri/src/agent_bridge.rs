use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock},
    time::{SystemTime, UNIX_EPOCH},
};
use tokio::time::{timeout, Duration};

use crate::{
    agent_proto::{image_content, FutureAgentClient, ImageContent, RpcCommand, StreamRequest},
    git_review, store,
};

const AGENT_EVENT_STREAM_TIMEOUT_SECS: u64 = 600;
static ACTIVE_AGENT_PROMPTS: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentPromptResponse {
    content: String,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentModelOption {
    id: String,
    label: String,
    provider: String,
    #[serde(default)]
    supports_images: bool,
    #[serde(default)]
    thinking_level: Option<String>,
    #[serde(default)]
    context_window: Option<i32>,
    #[serde(default)]
    is_default: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AgentModelsResponse {
    models: Vec<AgentModelOption>,
}

pub async fn list_agent_models() -> Result<Vec<AgentModelOption>, String> {
    let endpoint = agent_endpoint();
    let mut client = FutureAgentClient::connect(endpoint.clone())
        .await
        .map_err(|error| format!("Unable to connect to Future Agent at {endpoint}: {error}"))?;
    let response = client
        .execute_command(base_command("list_models", String::new()))
        .await
        .map_err(|error| format!("Unable to load Future Agent models: {error}"))?
        .into_inner();

    if !response.success {
        return Err(if response.error.is_empty() {
            "Future Agent rejected the model list request.".to_string()
        } else {
            response.error
        });
    }

    let parsed = serde_json::from_str::<AgentModelsResponse>(&response.data)
        .map_err(|error| format!("Future Agent returned invalid model data: {error}"))?;
    Ok(parsed.models)
}

pub async fn agent_prompt(
    message: String,
    image_paths: Option<Vec<String>>,
    thread_id: String,
    session_id: Option<String>,
    run_id: Option<String>,
    model_id: Option<String>,
    thinking_level: Option<String>,
) -> Result<AgentPromptResponse, String> {
    let result = agent_prompt_inner(
        message,
        image_paths,
        thread_id,
        session_id,
        run_id.clone(),
        model_id,
        thinking_level,
    )
    .await;

    if let Err(error) = &result {
        mark_run_failed_if_active(run_id.as_deref(), error);
    }

    result
}

async fn agent_prompt_inner(
    message: String,
    image_paths: Option<Vec<String>>,
    thread_id: String,
    session_id: Option<String>,
    run_id: Option<String>,
    model_id: Option<String>,
    thinking_level: Option<String>,
) -> Result<AgentPromptResponse, String> {
    let endpoint = agent_endpoint();
    let session_id = session_id
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| thread_id.clone());
    let _prompt_guard = PromptSessionGuard::acquire(&session_id)?;
    let cwd = workspace_path_for_thread(&thread_id)?;
    let prior_user_message_count = prior_user_message_count(&thread_id)?;
    let force_reset_session = prior_user_message_count == 0;

    let mut command_client = FutureAgentClient::connect(endpoint.clone())
        .await
        .map_err(|error| format!("Unable to connect to Future Agent at {endpoint}: {error}"))?;
    ensure_agent_session(&mut command_client, &session_id, &cwd, force_reset_session).await?;
    set_agent_permission_level(&mut command_client, &session_id, "workspace").await?;

    let mut event_client = FutureAgentClient::connect(endpoint.clone())
        .await
        .map_err(|error| format!("Unable to connect to Future Agent at {endpoint}: {error}"))?;
    let mut event_stream = event_client
        .stream_events(StreamRequest {
            event_types: vec![],
            session_id: session_id.clone(),
        })
        .await
        .map_err(|error| format!("Unable to subscribe to Future Agent events: {error}"))?
        .into_inner();

    if let Some(model_id) = model_id.filter(|value| !value.trim().is_empty()) {
        let response = command_client
            .execute_command(set_model_command(model_id, session_id.clone()))
            .await
            .map_err(|error| format!("Unable to set Future Agent model: {error}"))?
            .into_inner();

        if !response.success {
            return Err(if response.error.is_empty() {
                "Future Agent rejected the model selection.".to_string()
            } else {
                response.error
            });
        }
    }

    if let Some(thinking_level) = thinking_level.filter(|value| !value.trim().is_empty()) {
        let response = command_client
            .execute_command(set_thinking_level_command(
                thinking_level,
                session_id.clone(),
            ))
            .await
            .map_err(|error| format!("Unable to set Future Agent thinking level: {error}"))?
            .into_inner();

        if !response.success {
            return Err(if response.error.is_empty() {
                "Future Agent rejected the thinking level selection.".to_string()
            } else {
                response.error
            });
        }
    }

    let response = command_client
        .execute_command(prompt_command(
            message,
            session_id,
            image_paths.unwrap_or_default(),
        )?)
        .await
        .map_err(|error| format!("Unable to send prompt to Future Agent: {error}"))?
        .into_inner();

    if !response.success {
        return Err(if response.error.is_empty() {
            "Future Agent rejected the prompt.".to_string()
        } else {
            response.error
        });
    }

    let content = collect_agent_response(&mut event_stream, run_id.as_deref()).await?;
    Ok(AgentPromptResponse { content })
}

struct PromptSessionGuard {
    session_id: String,
}

impl PromptSessionGuard {
    fn acquire(session_id: &str) -> Result<Self, String> {
        let active = ACTIVE_AGENT_PROMPTS.get_or_init(|| Mutex::new(HashSet::new()));
        let mut guard = active
            .lock()
            .map_err(|_| "Unable to lock active Agent prompt registry.".to_string())?;
        if !guard.insert(session_id.to_string()) {
            return Err("Future Agent is already running for this session.".to_string());
        }
        Ok(Self {
            session_id: session_id.to_string(),
        })
    }
}

impl Drop for PromptSessionGuard {
    fn drop(&mut self) {
        if let Some(active) = ACTIVE_AGENT_PROMPTS.get() {
            if let Ok(mut guard) = active.lock() {
                guard.remove(&self.session_id);
            }
        }
    }
}

fn mark_run_failed_if_active(run_id: Option<&str>, error: &str) {
    let Some(run_id) = run_id else {
        return;
    };
    let Ok(Some(run)) = store::get_run(run_id) else {
        return;
    };
    if matches!(run.status.as_str(), "completed" | "failed" | "cancelled") {
        return;
    }
    let error_type = classify_run_error(error);
    if let Err(update_error) = store::update_run_status(store::UpdateRunStatusInput {
        run_id: run_id.to_string(),
        status: "failed".to_string(),
        error_message: Some(error.to_string()),
        error_type: Some(error_type.to_string()),
    }) {
        eprintln!("FutureOS run failure status update failed: {update_error}");
    }
}

/// Classify a raw error message string into a structured category that the UI
/// can use to render targeted recovery hints. Returns one of:
/// `stream_disconnected`, `command_failed`, `model_failed`, `abort_requested`,
/// `timeout`, or `unknown`.
///
/// The classification is intentionally conservative: when in doubt we return
/// `unknown` so the UI shows the generic error message without a misleading
/// category. Keep the patterns ordered from most-specific to most-generic; an
/// abort-induced timeout, for example, must be classified as `abort_requested`
/// rather than `timeout`.
pub(crate) fn classify_run_error(error: &str) -> &'static str {
    let lower = error.to_lowercase();

    // User-initiated abort wins over every other category, including timeouts
    // that may be reported as a side effect of cancellation.
    if lower.contains("interrupted")
        || lower.contains("aborted")
        || lower.contains("terminated by user")
        || lower.contains("cancelled")
        || lower.contains("canceled")
    {
        return "abort_requested";
    }

    if lower.contains("timed out") || lower.contains("timeout") {
        return "timeout";
    }

    // gRPC / transport / streaming layer failures.
    if lower.contains("unable to connect to future agent")
        || lower.contains("transport error")
        || lower.contains("broken pipe")
        || lower.contains("connection")
        || lower.contains("stream")
        || lower.contains("eof")
    {
        return "stream_disconnected";
    }

    // LLM / model-side failures. Check before generic `command failed` because
    // some providers report rate limit errors that mention "request" but are
    // model errors, not bash failures.
    if lower.contains("model")
        || lower.contains("llm")
        || lower.contains("provider")
        || lower.contains("api key")
        || lower.contains("rate limit")
        || lower.contains("unauthorized")
        || lower.contains("forbidden")
        || lower.contains("openai")
        || lower.contains("anthropic")
    {
        return "model_failed";
    }

    // Tool / shell command failures. We anchor on bash/exit-code phrasing the
    // agent itself emits to avoid mis-classifying generic "command" mentions.
    if lower.contains("bash command")
        || lower.contains("exit code")
        || lower.contains("failed to run bash")
        || lower.contains("tool execution")
    {
        return "command_failed";
    }

    "unknown"
}

pub async fn notify_agent_approval_decision(
    approval: &store::ApprovalRequestRecord,
    input: &store::DecideApprovalRequestInput,
) -> Result<(), String> {
    let thread = store::get_thread(&approval.thread_id)?
        .ok_or_else(|| "Approval thread could not be loaded.".to_string())?;
    let endpoint = agent_endpoint();
    let mut client = FutureAgentClient::connect(endpoint.clone())
        .await
        .map_err(|error| format!("Unable to connect to Future Agent at {endpoint}: {error}"))?;
    let response = client
        .execute_command(approval_decision_command(
            approval.id.clone(),
            input.status.clone(),
            input.decision_note.clone().unwrap_or_default(),
            thread.agent_session_id.unwrap_or(thread.id),
        ))
        .await
        .map_err(|error| format!("Unable to send approval decision to Future Agent: {error}"))?
        .into_inner();

    if response.success {
        Ok(())
    } else if response.error.is_empty() {
        Err("Future Agent rejected the approval decision.".to_string())
    } else {
        Err(response.error)
    }
}

pub async fn abort_agent_thread(thread_id: &str) -> Result<(), String> {
    let thread =
        store::get_thread(thread_id)?.ok_or_else(|| "Thread could not be loaded.".to_string())?;
    let endpoint = agent_endpoint();
    let mut client = FutureAgentClient::connect(endpoint.clone())
        .await
        .map_err(|error| format!("Unable to connect to Future Agent at {endpoint}: {error}"))?;
    let response = client
        .execute_command(base_command(
            "abort",
            thread.agent_session_id.unwrap_or(thread.id),
        ))
        .await
        .map_err(|error| format!("Unable to abort Future Agent run: {error}"))?
        .into_inner();

    if response.success {
        Ok(())
    } else if response.error.is_empty() {
        Err("Future Agent rejected the abort request.".to_string())
    } else {
        Err(response.error)
    }
}

async fn ensure_agent_session(
    client: &mut FutureAgentClient<tonic::transport::Channel>,
    session_id: &str,
    cwd: &str,
    force_reset: bool,
) -> Result<(), String> {
    if force_reset {
        return create_agent_session(client, session_id, cwd).await;
    }

    let response = client
        .execute_command(get_state_command(session_id.to_string()))
        .await
        .map_err(|error| format!("Unable to inspect Future Agent session: {error}"))?
        .into_inner();

    if response.success {
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(&response.data) {
            let active_session_id = value
                .get("sessionId")
                .and_then(|session_id| session_id.as_str())
                .unwrap_or_default();
            let active_cwd = value
                .get("cwd")
                .and_then(|cwd| cwd.as_str())
                .unwrap_or_default();

            if active_session_id == session_id && active_cwd == cwd {
                return Ok(());
            }
        }
    }

    create_agent_session(client, session_id, cwd).await
}

async fn create_agent_session(
    client: &mut FutureAgentClient<tonic::transport::Channel>,
    session_id: &str,
    cwd: &str,
) -> Result<(), String> {
    let response = client
        .execute_command(new_session_command(session_id.to_string(), cwd.to_string()))
        .await
        .map_err(|error| format!("Unable to create Future Agent session: {error}"))?
        .into_inner();

    if response.success {
        Ok(())
    } else if response.error.is_empty() {
        Err("Future Agent rejected the session initialization.".to_string())
    } else {
        Err(response.error)
    }
}

async fn set_agent_permission_level(
    client: &mut FutureAgentClient<tonic::transport::Channel>,
    session_id: &str,
    level: &str,
) -> Result<(), String> {
    let response = client
        .execute_command(set_permission_level_command(
            level.to_string(),
            session_id.to_string(),
        ))
        .await
        .map_err(|error| format!("Unable to set Future Agent permission level: {error}"))?
        .into_inner();

    if response.success {
        Ok(())
    } else if response.error.is_empty() {
        Err("Future Agent rejected the permission level selection.".to_string())
    } else {
        Err(response.error)
    }
}

fn prior_user_message_count(thread_id: &str) -> Result<usize, String> {
    let messages = store::list_messages(thread_id)?;
    let user_message_count = messages
        .iter()
        .filter(|message| message.role == "user")
        .count();
    Ok(user_message_count.saturating_sub(1))
}

fn workspace_path_for_thread(thread_id: &str) -> Result<String, String> {
    let thread =
        store::get_thread(thread_id)?.ok_or_else(|| "Thread could not be loaded.".to_string())?;
    let workspace = store::get_workspace(&thread.workspace_id)?
        .ok_or_else(|| "Thread workspace could not be loaded.".to_string())?;
    Ok(workspace.path)
}

fn agent_endpoint() -> String {
    let raw =
        std::env::var("FUTURE_AGENT_GRPC_ADDR").unwrap_or_else(|_| "127.0.0.1:50051".to_string());
    if raw.starts_with("http://") || raw.starts_with("https://") {
        raw
    } else {
        format!("http://{raw}")
    }
}

fn get_state_command(session_id: String) -> RpcCommand {
    base_command("get_state", session_id)
}

fn new_session_command(session_id: String, cwd: String) -> RpcCommand {
    RpcCommand {
        cwd,
        ..base_command("new_session", session_id)
    }
}

fn set_model_command(model_id: String, session_id: String) -> RpcCommand {
    RpcCommand {
        model_id,
        ..base_command("set_model", session_id)
    }
}

fn set_thinking_level_command(level: String, session_id: String) -> RpcCommand {
    RpcCommand {
        level,
        ..base_command("set_thinking_level", session_id)
    }
}

fn set_permission_level_command(level: String, session_id: String) -> RpcCommand {
    RpcCommand {
        level,
        ..base_command("set_permission_level", session_id)
    }
}

fn prompt_command(
    message: String,
    session_id: String,
    image_paths: Vec<String>,
) -> Result<RpcCommand, String> {
    Ok(RpcCommand {
        message,
        images: encode_image_paths(image_paths)?,
        ..base_command("prompt", session_id)
    })
}

fn approval_decision_command(
    approval_request_id: String,
    status: String,
    note: String,
    session_id: String,
) -> RpcCommand {
    RpcCommand {
        message: note,
        mode: status,
        entry_id: approval_request_id,
        ..base_command("approval_decision", session_id)
    }
}

fn base_command(command_type: &str, session_id: String) -> RpcCommand {
    RpcCommand {
        id: command_id(),
        r#type: command_type.to_string(),
        message: String::new(),
        images: vec![],
        streaming_behavior: String::new(),
        parent_session: String::new(),
        provider: String::new(),
        model_id: String::new(),
        level: String::new(),
        mode: String::new(),
        custom_instructions: String::new(),
        enabled: false,
        command: String::new(),
        session_path: String::new(),
        session_id,
        entry_id: String::new(),
        name: String::new(),
        output_path: String::new(),
        cwd: String::new(),
        system_prompt: String::new(),
        tools: vec![],
        no_tools: false,
        ephemeral: false,
        enabled_models: vec![],
    }
}

fn encode_image_paths(paths: Vec<String>) -> Result<Vec<ImageContent>, String> {
    paths
        .into_iter()
        .map(|path| {
            let mime_type = image_mime_type(&path)
                .ok_or_else(|| format!("Unsupported image attachment type: {path}"))?;
            let bytes =
                fs::read(&path).map_err(|error| format!("Unable to read image {path}: {error}"))?;
            let data_url = format!("data:{mime_type};base64,{}", BASE64_STANDARD.encode(bytes));
            Ok(ImageContent {
                r#type: "image_base64".to_string(),
                file_path: path,
                content: Some(image_content::Content::Base64(data_url)),
            })
        })
        .collect()
}

fn image_mime_type(path: &str) -> Option<&'static str> {
    let extension = Path::new(path)
        .extension()
        .and_then(|value| value.to_str())?
        .to_ascii_lowercase();

    match extension.as_str() {
        "jpg" | "jpeg" => Some("image/jpeg"),
        "png" => Some("image/png"),
        "gif" => Some("image/gif"),
        "webp" => Some("image/webp"),
        "bmp" => Some("image/bmp"),
        "svg" => Some("image/svg+xml"),
        _ => None,
    }
}

async fn collect_agent_response(
    stream: &mut tonic::Streaming<crate::agent_proto::StreamEvent>,
    run_id: Option<&str>,
) -> Result<String, String> {
    let mut content = String::new();
    let mut saw_agent_end = false;
    let mut waiting_for_approval = false;
    let mut sequence = 0_i64;

    loop {
        let next_event = if waiting_for_approval {
            stream
                .message()
                .await
                .map_err(|error| format!("Future Agent event stream failed: {error}"))?
        } else {
            match timeout(
                Duration::from_secs(AGENT_EVENT_STREAM_TIMEOUT_SECS),
                stream.message(),
            )
            .await
            {
                Ok(result) => {
                    result.map_err(|error| format!("Future Agent event stream failed: {error}"))?
                }
                Err(_) => {
                    persist_run_event(
                        run_id,
                        "timeout",
                        r#"{"error":"Future Agent response timed out."}"#,
                        sequence,
                    );
                    return Err("Future Agent response timed out.".to_string());
                }
            }
        };

        let Some(event) = next_event else {
            break;
        };

        persist_run_event(run_id, &event.r#type, &event.data, sequence);
        sequence += 1;

        match event.r#type.as_str() {
            "approval_request" => {
                waiting_for_approval = true;
            }
            "approval_decision" => {
                waiting_for_approval = false;
            }
            "text_chunk" => {
                if let Some(text) = event_text(&event.data) {
                    content.push_str(&text);
                }
            }
            "agent_end" => {
                saw_agent_end = true;
                break;
            }
            "error" => {
                return Err(event_error(&event.data)
                    .unwrap_or_else(|| "Future Agent returned an error event.".to_string()));
            }
            _ => {}
        }
    }

    if content.trim().is_empty() && !saw_agent_end {
        Err("Future Agent finished without returning any text.".to_string())
    } else {
        Ok(content)
    }
}

fn persist_run_event(run_id: Option<&str>, event_type: &str, payload: &str, sequence: i64) {
    let Some(run_id) = run_id else {
        return;
    };

    if let Err(error) = store::append_run_event(store::AppendRunEventInput {
        run_id: run_id.to_string(),
        event_type: event_type.to_string(),
        payload: if payload.is_empty() {
            None
        } else {
            Some(payload.to_string())
        },
        sequence,
    }) {
        eprintln!("FutureOS run event persistence failed: {error}");
    }

    persist_agent_tool_projection(run_id, event_type, payload, sequence);
}

fn event_text(data: &str) -> Option<String> {
    serde_json::from_str::<serde_json::Value>(data)
        .ok()
        .and_then(|value| {
            value
                .get("text")
                .and_then(|text| text.as_str())
                .map(str::to_string)
        })
}

fn persist_agent_tool_projection(run_id: &str, event_type: &str, payload: &str, sequence: i64) {
    let Some(value) = event_value(payload) else {
        return;
    };

    match event_type {
        "approval_request" => persist_approval_request(run_id, &value),
        "approval_decision" => persist_approval_decision(run_id, &value),
        "tool_start" | "toolcall_start" => persist_tool_start(run_id, &value, sequence),
        "tool_end" | "tool_result" => persist_tool_end(run_id, &value, sequence),
        "artifact_created" | "artifact.created" => persist_artifact(run_id, &value),
        _ => {}
    }
}

fn persist_approval_request(run_id: &str, value: &serde_json::Value) {
    let Some(approval_request_id) =
        value_string(value, &["approval_request_id", "approvalRequestId"])
    else {
        return;
    };
    let Some(tool_call_id) = value_string(value, &["tool_id", "toolID", "tool_call_id"]) else {
        return;
    };
    let tool_name =
        value_string(value, &["tool_name", "toolName"]).unwrap_or_else(|| "tool".to_string());
    let requested_action = value
        .get("requested_action")
        .or_else(|| value.get("requestedAction"))
        .or_else(|| value.get("tool_args"))
        .map(compact_json);

    // P2: structured action and sandbox boundary, persisted as JSON strings.
    let action_value = value.get("action").or_else(|| value.get("actionPayload"));
    let action_payload = action_value.map(compact_json);
    let action_category = action_value
        .and_then(|action| action.get("category"))
        .and_then(|category| category.as_str())
        .map(|category| category.to_string());
    let sandbox_boundary = value
        .get("sandbox_boundary")
        .or_else(|| value.get("sandboxBoundary"))
        .map(compact_json);
    let reviewer = value_string(value, &["reviewer"]);

    if let Err(error) = store::ensure_approval_request(store::EnsureApprovalRequestInput {
        approval_request_id: Some(approval_request_id),
        run_id: run_id.to_string(),
        tool_call_id,
        kind: value_string(value, &["kind"]).unwrap_or_else(|| "tool".to_string()),
        title: value_string(value, &["title"]).unwrap_or_else(|| format!("Approve `{tool_name}`")),
        summary: value_string(value, &["summary"]),
        risk_level: value_string(value, &["risk_level", "riskLevel"]),
        requested_action,
        action_category,
        action_payload,
        sandbox_boundary,
        reviewer,
    }) {
        eprintln!("FutureOS approval persistence failed: {error}");
    }
    if let Err(error) = store::update_run_status(store::UpdateRunStatusInput {
        run_id: run_id.to_string(),
        status: "waiting_approval".to_string(),
        error_message: None,
        error_type: None,
    }) {
        eprintln!("FutureOS run approval status update failed: {error}");
    }
}

fn persist_approval_decision(run_id: &str, value: &serde_json::Value) {
    let Some(approval_request_id) =
        value_string(value, &["approval_request_id", "approvalRequestId"])
    else {
        return;
    };
    let status = value_string(value, &["status"]).unwrap_or_else(|| "cancelled".to_string());
    let note = value_string(value, &["note"]);

    if let Err(error) = store::decide_approval_request(store::DecideApprovalRequestInput {
        approval_request_id,
        status: status.clone(),
        decision_note: note,
    }) {
        eprintln!("FutureOS approval decision persistence failed: {error}");
    }

    if status == "cancelled" {
        if let Err(error) = update_run_status_if_active(
            run_id,
            "cancelled",
            Some("Approval request was cancelled.".to_string()),
        ) {
            eprintln!("FutureOS run approval cancellation status update failed: {error}");
        }
        return;
    }

    if let Err(error) = update_run_status_if_active(run_id, "running", None) {
        eprintln!("FutureOS run approval decision status update failed: {error}");
    }
}

fn update_run_status_if_active(
    run_id: &str,
    status: &str,
    error_message: Option<String>,
) -> Result<(), String> {
    let Some(run) = store::get_run(run_id)? else {
        return Ok(());
    };
    if matches!(run.status.as_str(), "completed" | "failed" | "cancelled") {
        return Ok(());
    }
    store::update_run_status(store::UpdateRunStatusInput {
        run_id: run_id.to_string(),
        status: status.to_string(),
        error_message,
        error_type: None,
    })?;
    Ok(())
}

fn persist_tool_start(run_id: &str, value: &serde_json::Value, sequence: i64) {
    let tool_name =
        value_string(value, &["tool_name", "toolName"]).unwrap_or_else(|| "tool".to_string());
    let tool_call_id = value_string(value, &["tool_id", "toolID", "tool_call_id"])
        .unwrap_or_else(|| format!("{run_id}_tool_{sequence}"));
    let input = value
        .get("tool_args")
        .or_else(|| value.get("toolArgs"))
        .or_else(|| value.get("arguments"))
        .map(compact_json);

    if let Err(error) = store::upsert_tool_call(store::UpsertToolCallInput {
        run_id: run_id.to_string(),
        tool_call_id: tool_call_id.clone(),
        name: tool_name.clone(),
        kind: "agent_tool".to_string(),
        input: input.clone(),
        status: "running".to_string(),
    }) {
        eprintln!("FutureOS tool call persistence failed: {error}");
    }

    if let Some((change_type, path)) = review_shape_for_tool(&tool_name, value) {
        if let Err(error) = store::ensure_review_change(store::EnsureReviewChangeInput {
            run_id: run_id.to_string(),
            tool_call_id,
            title: format!("Review `{tool_name}` changes"),
            summary: Some(format!("Agent requested `{tool_name}`.")),
            path,
            change_type,
        }) {
            eprintln!("FutureOS review persistence failed: {error}");
        }
    }
}

fn persist_tool_end(run_id: &str, value: &serde_json::Value, sequence: i64) {
    let tool_name =
        value_string(value, &["tool_name", "toolName"]).unwrap_or_else(|| "tool".to_string());
    let tool_call_id = value_string(value, &["tool_id", "toolID", "tool_call_id"])
        .unwrap_or_else(|| format!("{run_id}_tool_{sequence}"));
    let error = value_string(value, &["error", "errorText"]);
    let output_content =
        value_string(value, &["text", "result"]).or_else(|| value.get("output").map(compact_json));
    let status = if error.as_deref().unwrap_or_default().is_empty() {
        "completed".to_string()
    } else {
        "failed".to_string()
    };
    let output_kind = if status == "completed" {
        "text".to_string()
    } else {
        "error".to_string()
    };
    let final_output = error.or(output_content);

    if status == "completed" {
        persist_written_file_artifact(run_id, &tool_name, final_output.as_deref());
    }

    if let Err(error) = store::complete_tool_call(store::CompleteToolCallInput {
        run_id: run_id.to_string(),
        tool_call_id,
        name: tool_name,
        status,
        output_kind,
        output_content: final_output,
    }) {
        eprintln!("FutureOS tool output persistence failed: {error}");
    }
}

fn persist_written_file_artifact(run_id: &str, tool_name: &str, output: Option<&str>) {
    if tool_name != "write" {
        return;
    }

    let Some(path) = output.and_then(written_path_from_tool_output) else {
        return;
    };
    match path_is_inside_run_workspace(run_id, &path) {
        Ok(true) => {}
        Ok(false) => return,
        Err(error) => {
            eprintln!("FutureOS write artifact workspace check failed: {error}");
            return;
        }
    }

    let path_ref = Path::new(&path);
    let title = path_ref
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.trim().is_empty())
        .unwrap_or("Written file")
        .to_string();
    let artifact_type = artifact_type_from_path(path_ref);

    if let Err(error) = store::ensure_artifact(store::EnsureArtifactInput {
        run_id: run_id.to_string(),
        title,
        artifact_type,
        path: Some(path),
        content: None,
        content_storage: Some("file".to_string()),
        summary: Some("Written by Agent.".to_string()),
    }) {
        eprintln!("FutureOS write artifact persistence failed: {error}");
    }
}

fn path_is_inside_run_workspace(run_id: &str, path: &str) -> Result<bool, String> {
    let run = store::get_run(run_id)?.ok_or_else(|| "Run could not be loaded.".to_string())?;
    let thread = store::get_thread(&run.thread_id)?
        .ok_or_else(|| "Thread could not be loaded.".to_string())?;
    let workspace = store::get_workspace(&thread.workspace_id)?
        .ok_or_else(|| "Workspace could not be loaded.".to_string())?;
    if git_review::is_git_workspace(Path::new(&workspace.path)) {
        return Ok(false);
    }

    let workspace_path = canonical_or_raw(&workspace.path);
    let candidate_path = canonical_or_raw(path);
    Ok(candidate_path.starts_with(workspace_path))
}

fn canonical_or_raw(path: &str) -> PathBuf {
    fs::canonicalize(path).unwrap_or_else(|_| PathBuf::from(path))
}

fn written_path_from_tool_output(output: &str) -> Option<String> {
    output
        .trim()
        .strip_prefix("Written to ")
        .map(str::trim)
        .filter(|path| !path.is_empty())
        .map(str::to_string)
}

fn persist_artifact(run_id: &str, value: &serde_json::Value) {
    let title = value_string(value, &["title", "name"]).unwrap_or_else(|| "Artifact".to_string());
    let artifact_type = value_string(value, &["type", "artifact_type", "artifactType"])
        .unwrap_or_else(|| "document".to_string());
    let path = value_string(value, &["path", "file_path", "filePath"]);
    match artifact_is_allowed_for_run(run_id, path.as_deref()) {
        Ok(true) => {}
        Ok(false) => return,
        Err(error) => {
            eprintln!("FutureOS artifact workspace check failed: {error}");
            return;
        }
    }
    let content = value_string(value, &["content", "text"]);
    let content_storage = value_string(value, &["content_storage", "contentStorage"])
        .or_else(|| path.as_ref().map(|_| "file".to_string()))
        .or_else(|| content.as_ref().map(|_| "inline".to_string()));
    let summary = value_string(value, &["summary", "description"]);
    if let Err(error) = store::ensure_artifact(store::EnsureArtifactInput {
        run_id: run_id.to_string(),
        title,
        artifact_type,
        path,
        content,
        content_storage,
        summary,
    }) {
        eprintln!("FutureOS artifact persistence failed: {error}");
    }
}

fn artifact_is_allowed_for_run(run_id: &str, path: Option<&str>) -> Result<bool, String> {
    let run = store::get_run(run_id)?.ok_or_else(|| "Run could not be loaded.".to_string())?;
    let thread = store::get_thread(&run.thread_id)?
        .ok_or_else(|| "Thread could not be loaded.".to_string())?;
    let workspace = store::get_workspace(&thread.workspace_id)?
        .ok_or_else(|| "Workspace could not be loaded.".to_string())?;
    if git_review::is_git_workspace(Path::new(&workspace.path)) {
        return Ok(false);
    }

    let Some(path) = path else {
        return Ok(true);
    };
    let workspace_path = canonical_or_raw(&workspace.path);
    let candidate_path = canonical_or_raw(path);
    Ok(candidate_path.starts_with(workspace_path))
}

fn event_value(payload: &str) -> Option<serde_json::Value> {
    serde_json::from_str::<serde_json::Value>(payload).ok()
}

fn value_string(value: &serde_json::Value, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|key| {
        value.get(*key).and_then(|field| {
            field
                .as_str()
                .map(str::to_string)
                .or_else(|| (!field.is_null()).then(|| compact_json(field)))
        })
    })
}

fn compact_json(value: &serde_json::Value) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| value.to_string())
}

fn artifact_type_from_path(path: &Path) -> String {
    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();

    match extension.as_str() {
        "jpg" | "jpeg" | "png" | "gif" | "webp" | "svg" | "bmp" | "tif" | "tiff" => "image",
        "pdf" => "pdf",
        "doc" | "docx" | "md" | "rtf" | "txt" => "document",
        "csv" | "tsv" | "xls" | "xlsx" => "spreadsheet",
        "json" | "jsonl" | "parquet" | "sqlite" | "db" => "data",
        "py" | "rs" | "ts" | "tsx" | "js" | "jsx" | "go" | "java" | "c" | "cpp" | "h" | "hpp" => {
            "code"
        }
        _ => "file",
    }
    .to_string()
}

fn review_shape_for_tool(
    tool_name: &str,
    value: &serde_json::Value,
) -> Option<(String, Option<String>)> {
    if !matches!(tool_name, "write" | "edit") {
        return None;
    }

    let args = value
        .get("tool_args")
        .or_else(|| value.get("toolArgs"))
        .or_else(|| value.get("arguments"))?;
    let path = value_string(args, &["path", "file_path", "filePath"]);
    let change_type = if tool_name == "write" {
        "write"
    } else {
        "modify"
    };
    Some((change_type.to_string(), path))
}

fn event_error(data: &str) -> Option<String> {
    serde_json::from_str::<serde_json::Value>(data)
        .ok()
        .and_then(|value| {
            value
                .get("error")
                .or_else(|| value.get("message"))
                .and_then(|error| error.as_str())
                .map(str::to_string)
        })
}

fn command_id() -> String {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or_default();
    format!("gui_{millis}")
}

#[cfg(test)]
mod tests {
    use super::classify_run_error;

    #[test]
    fn test_classify_abort_requested() {
        assert_eq!(classify_run_error("Bash command interrupted by abort"), "abort_requested");
        assert_eq!(classify_run_error("Interrupted"), "abort_requested");
        assert_eq!(classify_run_error("Terminated by user."), "abort_requested");
        assert_eq!(classify_run_error("aborted"), "abort_requested");
        assert_eq!(classify_run_error("cancelled"), "abort_requested");
        assert_eq!(classify_run_error("canceled"), "abort_requested");
    }

    #[test]
    fn test_classify_timeout() {
        assert_eq!(classify_run_error("Bash command timed out after 60 seconds"), "timeout");
        assert_eq!(classify_run_error("timeout"), "timeout");
        assert_eq!(classify_run_error("Timed out"), "timeout");
    }

    #[test]
    fn test_classify_stream_disconnected() {
        assert_eq!(classify_run_error("Unable to connect to Future Agent at 127.0.0.1:50051"), "stream_disconnected");
        assert_eq!(classify_run_error("Transport error: broken pipe"), "stream_disconnected");
        assert_eq!(classify_run_error("connection closed"), "stream_disconnected");
        assert_eq!(classify_run_error("Stream error: unexpected EOF"), "stream_disconnected");
    }

    #[test]
    fn test_classify_model_failed() {
        assert_eq!(classify_run_error("Model returned error: unauthorized"), "model_failed");
        assert_eq!(classify_run_error("LLM provider failed: rate limit exceeded"), "model_failed");
        assert_eq!(classify_run_error("api key is invalid"), "model_failed");
        assert_eq!(classify_run_error("forbidden"), "model_failed");
        assert_eq!(classify_run_error("OpenAI API error"), "model_failed");
        assert_eq!(classify_run_error("Anthropic API error"), "model_failed");
    }

    #[test]
    fn test_classify_command_failed() {
        assert_eq!(classify_run_error("Bash command exited with code 1"), "command_failed");
        assert_eq!(classify_run_error("Failed to run bash command: no such file"), "command_failed");
        assert_eq!(classify_run_error("exit code: 127"), "command_failed");
    }

    #[test]
    fn test_classify_unknown() {
        assert_eq!(classify_run_error("Something unexpected happened"), "unknown");
        assert_eq!(classify_run_error(""), "unknown");
    }

    #[test]
    fn test_classify_abort_beats_timeout() {
        // Interrupt aborts take priority over timeout mentions
        assert_eq!(classify_run_error("Bash command interrupted: timed out"), "abort_requested");
        assert_eq!(classify_run_error("Timed out (user aborted)"), "abort_requested");
    }
}
