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

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AttachmentInfo {
    is_dir: bool,
    size: u64,
    is_binary: bool,
}

/// Inspect a local file for attachment classification. The webview can't read
/// arbitrary paths, so directory + binary detection must happen here in Rust.
#[tauri::command]
pub fn inspect_attachment(path: String) -> Result<AttachmentInfo, crate::AppError> {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return Err("path cannot be empty.".to_string().into());
    }
    let meta = std::fs::metadata(trimmed)?;
    if meta.is_dir() {
        return Ok(AttachmentInfo {
            is_dir: true,
            size: 0,
            is_binary: false,
        });
    }
    let mut file = File::open(trimmed)?;
    let mut buffer = vec![0_u8; 4096];
    let read = file.read(&mut buffer)?;
    let sample = &buffer[..read];
    // Binary if it contains a NUL byte or >30% control chars (excluding tab/CR/LF).
    let control = sample
        .iter()
        .filter(|&&b| b == 0 || (b < 0x20 && b != b'\t' && b != b'\n' && b != b'\r'))
        .count();
    let is_binary = sample.contains(&0) || (read > 0 && control * 100 / read > 30);
    Ok(AttachmentInfo {
        is_dir: false,
        size: meta.len(),
        is_binary,
    })
}

/// Read a whole file as base64 (for client-side PDF text extraction). Errors if
/// the file exceeds `max_bytes` (default 25MB) — extraction targets must be small.
#[tauri::command]
pub fn read_file_base64(path: String, max_bytes: Option<u64>) -> Result<String, crate::AppError> {
    use base64::{engine::general_purpose::STANDARD, Engine as _};
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return Err("path cannot be empty.".to_string().into());
    }
    let meta = std::fs::metadata(trimmed)?;
    let limit = max_bytes.unwrap_or(25 * 1024 * 1024);
    if meta.len() > limit {
        return Err(format!("File too large ({} bytes; limit {}).", meta.len(), limit).into());
    }
    Ok(STANDARD.encode(std::fs::read(trimmed)?))
}

/// Persist a base64-encoded JPEG thumbnail under `<appCache>/thumbnails/<key>.jpg`
/// and return its absolute path (rendered in the webview via `convertFileSrc`).
#[tauri::command]
pub fn write_thumbnail(
    app: tauri::AppHandle,
    base64_jpeg: String,
    key: String,
) -> Result<String, crate::AppError> {
    use base64::{engine::general_purpose::STANDARD, Engine as _};
    use tauri::Manager;
    let safe_key: String = key
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
        .collect();
    if safe_key.is_empty() {
        return Err("invalid thumbnail key.".to_string().into());
    }
    let bytes = STANDARD
        .decode(base64_jpeg.as_bytes())
        .map_err(|error| format!("invalid thumbnail data: {error}"))?;
    let dir = app
        .path()
        .app_cache_dir()
        .map_err(|error| format!("cache dir unavailable: {error}"))?
        .join("thumbnails");
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(format!("{safe_key}.jpg"));
    std::fs::write(&path, &bytes)?;
    Ok(path.display().to_string())
}

/// Delete a pasted temp attachment after send. Guarded to only remove files
/// inside our own `<temp>/futureos-attachments/` subdir — never user originals.
#[tauri::command]
pub fn delete_temp_attachment(path: String) -> Result<(), crate::AppError> {
    let base = std::env::temp_dir().join("futureos-attachments");
    let target = std::path::Path::new(path.trim());
    let canon_target = target.canonicalize().ok();
    let canon_base = base.canonicalize().ok();
    match (canon_target, canon_base) {
        (Some(t), Some(b)) if t.starts_with(&b) && t.is_file() => {
            std::fs::remove_file(&t)?;
            Ok(())
        }
        _ => Err("Refusing to delete: not a FutureOS temp attachment."
            .to_string()
            .into()),
    }
}

#[tauri::command]
pub fn open_path(path: String) -> Result<(), crate::AppError> {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return Err("path cannot be empty.".to_string().into());
    }

    open_path_with_system(trimmed)
}

/// Open an http(s) URL in the user's default browser. The scheme is restricted
/// to http/https so this can't be used to launch arbitrary local handlers
/// (`file:`, custom app schemes, …) via a crafted url.
#[tauri::command]
pub fn open_external_url(url: String) -> Result<(), crate::AppError> {
    let trimmed = url.trim();
    if !(trimmed.starts_with("http://") || trimmed.starts_with("https://")) {
        return Err("Only http(s) URLs can be opened.".to_string().into());
    }

    open_path_with_system(trimmed)
}

#[tauri::command]
pub fn read_text_file_preview(
    path: String,
    max_bytes: Option<usize>,
) -> Result<TextFilePreview, crate::AppError> {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return Err("path cannot be empty.".to_string().into());
    }

    let limit = max_bytes.unwrap_or(200 * 1024).clamp(1, 1024 * 1024);
    let mut file = File::open(trimmed)?;
    let size = file.metadata()?.len();
    let mut buffer = vec![0_u8; limit.saturating_add(1)];
    let read = file.read(&mut buffer)?;
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
) -> Result<(), crate::AppError> {
    let destination = destination_path.trim();
    if destination.is_empty() {
        return Err("destinationPath cannot be empty.".to_string().into());
    }

    if let Some(content) = content {
        std::fs::write(destination, content)?;
        return Ok(());
    }

    let source = source_path
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "sourcePath or content is required.".to_string())?;
    std::fs::copy(source, destination)?;
    Ok(())
}

/// Persist pasted image bytes to a temp file so the path can be attached and
/// later read by the multimodal agent. Pasted/dropped clipboard images have no
/// filesystem path of their own.
#[tauri::command]
pub fn save_pasted_image(
    bytes: Vec<u8>,
    extension: Option<String>,
) -> Result<SavedAttachment, crate::AppError> {
    if bytes.is_empty() {
        return Err("Pasted image is empty.".to_string().into());
    }
    let ext = extension
        .map(|value| value.trim().trim_start_matches('.').to_ascii_lowercase())
        .filter(|value| !value.is_empty() && value.chars().all(|c| c.is_ascii_alphanumeric()))
        .unwrap_or_else(|| "png".to_string());

    let dir = std::env::temp_dir().join("futureos-attachments");
    std::fs::create_dir_all(&dir)?;

    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|value| value.as_nanos())
        .unwrap_or(0);
    let name = format!("pasted-{nanos}.{ext}");
    let path = dir.join(&name);
    std::fs::write(&path, &bytes)?;

    Ok(SavedAttachment {
        path: path.display().to_string(),
        name,
    })
}

#[cfg(target_os = "macos")]
fn open_path_with_system(path: &str) -> Result<(), crate::AppError> {
    Command::new("open")
        .arg(path)
        .status()
        .map_err(crate::AppError::from)
        .and_then(|status| {
            status
                .success()
                .then_some(())
                .ok_or_else(|| format!("open exited with status {status}").into())
        })
}

#[cfg(target_os = "windows")]
fn open_path_with_system(path: &str) -> Result<(), crate::AppError> {
    Command::new("cmd")
        .args(["/C", "start", "", path])
        .status()
        .map_err(crate::AppError::from)
        .and_then(|status| {
            status
                .success()
                .then_some(())
                .ok_or_else(|| format!("start exited with status {status}").into())
        })
}

#[cfg(all(unix, not(target_os = "macos")))]
fn open_path_with_system(path: &str) -> Result<(), crate::AppError> {
    Command::new("xdg-open")
        .arg(path)
        .status()
        .map_err(crate::AppError::from)
        .and_then(|status| {
            status
                .success()
                .then_some(())
                .ok_or_else(|| format!("xdg-open exited with status {status}").into())
        })
}
