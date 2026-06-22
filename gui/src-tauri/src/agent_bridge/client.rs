//! Construction of gRPC `RpcCommand`s and the agent endpoint, plus image
//! attachment encoding. This is the thin request-building layer; orchestration
//! and event handling live in the parent module.

use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};
use std::{
    fs,
    path::Path,
    time::{SystemTime, UNIX_EPOCH},
};

use crate::agent_proto::{image_content, ImageContent, RpcCommand};

pub(super) fn agent_endpoint() -> String {
    let raw =
        std::env::var("FUTURE_AGENT_GRPC_ADDR").unwrap_or_else(|_| "127.0.0.1:50051".to_string());
    if raw.starts_with("http://") || raw.starts_with("https://") {
        raw
    } else {
        format!("http://{raw}")
    }
}

pub(super) fn get_state_command(session_id: String) -> RpcCommand {
    base_command("get_state", session_id)
}

pub(super) fn new_session_command(session_id: String, cwd: String) -> RpcCommand {
    RpcCommand {
        cwd,
        ..base_command("new_session", session_id)
    }
}

pub(super) fn set_model_command(model_id: String, session_id: String) -> RpcCommand {
    RpcCommand {
        model_id,
        ..base_command("set_model", session_id)
    }
}

pub(super) fn set_thinking_level_command(level: String, session_id: String) -> RpcCommand {
    RpcCommand {
        level,
        ..base_command("set_thinking_level", session_id)
    }
}

pub(super) fn set_permission_level_command(level: String, session_id: String) -> RpcCommand {
    RpcCommand {
        level,
        ..base_command("set_permission_level", session_id)
    }
}

pub(super) fn prompt_command(
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

pub(super) fn approval_decision_command(
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

pub(super) fn base_command(command_type: &str, session_id: String) -> RpcCommand {
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

fn command_id() -> String {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or_default();
    format!("gui_{millis}")
}
