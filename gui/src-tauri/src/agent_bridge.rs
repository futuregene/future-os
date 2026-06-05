use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::Path,
    time::{SystemTime, UNIX_EPOCH},
};
use tokio::time::{timeout, Duration};

use crate::{
    agent_proto::{image_content, FutureAgentClient, ImageContent, RpcCommand, StreamRequest},
    store,
};

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
    let endpoint = agent_endpoint();
    let session_id = session_id
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| thread_id.clone());
    let cwd = workspace_path_for_thread(&thread_id)?;
    let prior_user_message_count = prior_user_message_count(&thread_id)?;
    let force_reset_session = prior_user_message_count == 0;

    let mut command_client = FutureAgentClient::connect(endpoint.clone())
        .await
        .map_err(|error| format!("Unable to connect to Future Agent at {endpoint}: {error}"))?;
    ensure_agent_session(&mut command_client, &session_id, &cwd, force_reset_session).await?;

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
    let mut sequence = 0_i64;

    let result = timeout(Duration::from_secs(600), async {
        while let Some(event) = stream
            .message()
            .await
            .map_err(|error| format!("Future Agent event stream failed: {error}"))?
        {
            persist_run_event(run_id, &event.r#type, &event.data, sequence);
            sequence += 1;

            match event.r#type.as_str() {
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
        Ok::<(), String>(())
    })
    .await;

    match result {
        Ok(Ok(())) => {
            if content.trim().is_empty() && !saw_agent_end {
                Err("Future Agent finished without returning any text.".to_string())
            } else {
                Ok(content)
            }
        }
        Ok(Err(error)) => Err(error),
        Err(_) => {
            persist_run_event(
                run_id,
                "timeout",
                r#"{"error":"Future Agent response timed out."}"#,
                sequence,
            );
            Err("Future Agent response timed out.".to_string())
        }
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
        "approval_decision" => {
            if let Err(error) = store::update_run_status(store::UpdateRunStatusInput {
                run_id: run_id.to_string(),
                status: "running".to_string(),
                error_message: None,
            }) {
                eprintln!("FutureOS run approval decision status update failed: {error}");
            }
        }
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

    if let Err(error) = store::ensure_approval_request(store::EnsureApprovalRequestInput {
        approval_request_id: Some(approval_request_id),
        run_id: run_id.to_string(),
        tool_call_id,
        kind: value_string(value, &["kind"]).unwrap_or_else(|| "tool".to_string()),
        title: value_string(value, &["title"]).unwrap_or_else(|| format!("Approve `{tool_name}`")),
        summary: value_string(value, &["summary"]),
        risk_level: value_string(value, &["risk_level", "riskLevel"]),
        requested_action,
    }) {
        eprintln!("FutureOS approval persistence failed: {error}");
    }
    if let Err(error) = store::update_run_status(store::UpdateRunStatusInput {
        run_id: run_id.to_string(),
        status: "waiting_approval".to_string(),
        error_message: None,
    }) {
        eprintln!("FutureOS run approval status update failed: {error}");
    }
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

    if let Err(error) = store::complete_tool_call(store::CompleteToolCallInput {
        run_id: run_id.to_string(),
        tool_call_id,
        name: tool_name,
        status: if error.as_deref().unwrap_or_default().is_empty() {
            "completed".to_string()
        } else {
            "failed".to_string()
        },
        output_kind: if error.as_deref().unwrap_or_default().is_empty() {
            "text".to_string()
        } else {
            "error".to_string()
        },
        output_content: error.or(output_content),
    }) {
        eprintln!("FutureOS tool output persistence failed: {error}");
    }
}

fn persist_artifact(run_id: &str, value: &serde_json::Value) {
    let title = value_string(value, &["title", "name"]).unwrap_or_else(|| "Artifact".to_string());
    let artifact_type = value_string(value, &["type", "artifact_type", "artifactType"])
        .unwrap_or_else(|| "document".to_string());
    let path = value_string(value, &["path", "file_path", "filePath"]);
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
