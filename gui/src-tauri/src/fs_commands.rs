//! Local filesystem Tauri commands: opening paths in the OS, previewing text
//! files, exporting artifacts, and persisting pasted images.

use std::{fs::File, io::Read, process::Command};

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TextFilePreview {
    content: String,
    size: u64,
    truncated: bool,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SavedAttachment {
    path: String,
    name: String,
}

#[tauri::command]
pub fn open_path(path: String) -> Result<(), String> {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return Err("path cannot be empty.".to_string());
    }

    open_path_with_system(trimmed)
}

#[tauri::command]
pub fn read_text_file_preview(
    path: String,
    max_bytes: Option<usize>,
) -> Result<TextFilePreview, String> {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return Err("path cannot be empty.".to_string());
    }

    let limit = max_bytes.unwrap_or(200 * 1024).clamp(1, 1024 * 1024);
    let mut file = File::open(trimmed).map_err(|error| error.to_string())?;
    let size = file.metadata().map_err(|error| error.to_string())?.len();
    let mut buffer = vec![0_u8; limit.saturating_add(1)];
    let read = file.read(&mut buffer).map_err(|error| error.to_string())?;
    let truncated = read > limit || size > limit as u64;
    buffer.truncate(read.min(limit));

    Ok(TextFilePreview {
        content: String::from_utf8_lossy(&buffer).to_string(),
        size,
        truncated,
    })
}

#[tauri::command]
pub fn export_artifact_file(
    destination_path: String,
    source_path: Option<String>,
    content: Option<String>,
) -> Result<(), String> {
    let destination = destination_path.trim();
    if destination.is_empty() {
        return Err("destinationPath cannot be empty.".to_string());
    }

    if let Some(content) = content {
        std::fs::write(destination, content).map_err(|error| error.to_string())?;
        return Ok(());
    }

    let source = source_path
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "sourcePath or content is required.".to_string())?;
    std::fs::copy(source, destination).map_err(|error| error.to_string())?;
    Ok(())
}

/// Persist pasted image bytes to a temp file so the path can be attached and
/// later read by the multimodal agent. Pasted/dropped clipboard images have no
/// filesystem path of their own.
#[tauri::command]
pub fn save_pasted_image(
    bytes: Vec<u8>,
    extension: Option<String>,
) -> Result<SavedAttachment, String> {
    if bytes.is_empty() {
        return Err("Pasted image is empty.".to_string());
    }
    let ext = extension
        .map(|value| value.trim().trim_start_matches('.').to_ascii_lowercase())
        .filter(|value| !value.is_empty() && value.chars().all(|c| c.is_ascii_alphanumeric()))
        .unwrap_or_else(|| "png".to_string());

    let dir = std::env::temp_dir().join("futureos-attachments");
    std::fs::create_dir_all(&dir).map_err(|error| error.to_string())?;

    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|value| value.as_nanos())
        .unwrap_or(0);
    let name = format!("pasted-{nanos}.{ext}");
    let path = dir.join(&name);
    std::fs::write(&path, &bytes).map_err(|error| error.to_string())?;

    Ok(SavedAttachment {
        path: path.display().to_string(),
        name,
    })
}

#[cfg(target_os = "macos")]
fn open_path_with_system(path: &str) -> Result<(), String> {
    Command::new("open")
        .arg(path)
        .status()
        .map_err(|error| error.to_string())
        .and_then(|status| {
            status
                .success()
                .then_some(())
                .ok_or_else(|| format!("open exited with status {status}"))
        })
}

#[cfg(target_os = "windows")]
fn open_path_with_system(path: &str) -> Result<(), String> {
    Command::new("cmd")
        .args(["/C", "start", "", path])
        .status()
        .map_err(|error| error.to_string())
        .and_then(|status| {
            status
                .success()
                .then_some(())
                .ok_or_else(|| format!("start exited with status {status}"))
        })
}

#[cfg(all(unix, not(target_os = "macos")))]
fn open_path_with_system(path: &str) -> Result<(), String> {
    Command::new("xdg-open")
        .arg(path)
        .status()
        .map_err(|error| error.to_string())
        .and_then(|status| {
            status
                .success()
                .then_some(())
                .ok_or_else(|| format!("xdg-open exited with status {status}"))
        })
}
