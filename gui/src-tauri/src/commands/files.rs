//! Local filesystem Tauri commands: opening paths in the OS, previewing text
//! files, exporting artifacts, and persisting pasted images.

use std::{
    ffi::OsString,
    fs::File,
    io::{BufReader, Read},
    path::{Path, PathBuf},
};

/// Resolve `path` to an absolute, symlink/`..`-collapsed form even when the
/// target doesn't exist yet (e.g. an export destination): canonicalize the
/// nearest existing ancestor, then re-append the missing tail.
fn best_effort_canonical(path: &Path) -> PathBuf {
    if let Ok(canonical) = path.canonicalize() {
        return canonical;
    }
    let mut existing = path;
    let mut tail: Vec<OsString> = Vec::new();
    while !existing.exists() {
        match (existing.file_name(), existing.parent()) {
            (Some(name), Some(parent)) => {
                tail.push(name.to_os_string());
                existing = parent;
            }
            _ => return path.to_path_buf(),
        }
    }
    let mut base = existing
        .canonicalize()
        .unwrap_or_else(|_| existing.to_path_buf());
    for name in tail.into_iter().rev() {
        base.push(name);
    }
    base
}

/// Reject file access to FutureOS's own credential files. These commands are
/// reachable from the webview, which renders agent-produced markdown/artifacts —
/// without this guard an XSS could read `auth.json` and exfiltrate API keys.
///
/// This is a **denylist of credential files only** (`auth.json` / `models.json`
/// in the agent config dirs). Everything else — attachment images, workspace
/// files, the app DB, session history, settings, run logs, and any user-chosen
/// file elsewhere — is allowed: the webview already reaches all of that through
/// typed store/session commands, so the raw bytes aren't a new exposure. The
/// only thing it never legitimately needs as raw bytes is the credentials.
fn ensure_path_allowed(path: &Path) -> Result<(), crate::AppError> {
    let resolved = best_effort_canonical(path);
    if let Some(home) = crate::home_dir() {
        if let Ok(future_dir) = PathBuf::from(home).join(".future").canonicalize() {
            if is_protected_credential(&future_dir, &resolved) {
                return Err("Refusing to access a protected FutureOS credential file."
                    .to_string()
                    .into());
            }
        }
    }
    Ok(())
}

/// True when `resolved` (already canonical) is a FutureOS credential file:
/// `auth.json` / `models.json` sitting directly in an agent config dir
/// (`~/.future/agent/` or `~/.future/agent-app/`). Scoped to those dirs so a
/// user's own file that merely shares the name (e.g. a workspace `models.json`)
/// stays readable.
fn is_protected_credential(future_dir: &Path, resolved: &Path) -> bool {
    let is_credential_name = resolved
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name == "auth.json" || name == "models.json");
    if !is_credential_name {
        return false;
    }
    ["agent", "agent-app"].iter().any(|dir| {
        let cred_dir = future_dir.join(dir);
        let cred_dir = cred_dir.canonicalize().unwrap_or(cred_dir);
        resolved.parent() == Some(cred_dir.as_path())
    })
}

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
pub struct ResolvedPreviewLink {
    /// Absolute path the link resolves to, used for the OS-open action.
    path: String,
    /// File name (last path component).
    name: String,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AttachmentInfo {
    is_dir: bool,
    size: u64,
    is_binary: bool,
}

const MAX_ATTACHMENT_IMAGE_BYTES: u64 = 25 * 1024 * 1024;

/// Inspect a local file for attachment classification. The webview can't read
/// arbitrary paths, so directory + binary detection must happen here in Rust.
#[tauri::command]
pub fn inspect_attachment(path: String) -> Result<AttachmentInfo, crate::AppError> {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return Err("path cannot be empty.".to_string().into());
    }
    ensure_path_allowed(Path::new(trimmed))?;
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

/// Fully decode a user-selected image before it enters the composer. Extension
/// classification alone is insufficient: a corrupt or renamed file would be
/// shown as attached but later skipped by the multimodal request builder.
#[tauri::command]
pub fn validate_image_attachment(path: String) -> Result<(), crate::AppError> {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return Err("path cannot be empty.".to_string().into());
    }
    ensure_path_allowed(Path::new(trimmed))?;
    let size = std::fs::metadata(trimmed)?.len();
    if size > MAX_ATTACHMENT_IMAGE_BYTES {
        return Err(format!(
            "Image is too large ({size} bytes; limit {MAX_ATTACHMENT_IMAGE_BYTES})."
        )
        .into());
    }
    let file = File::open(trimmed)?;
    let mut reader = image::ImageReader::new(BufReader::new(file))
        .with_guessed_format()
        .map_err(|error| format!("unreadable image: {error}"))?;
    let mut limits = image::Limits::default();
    limits.max_alloc = Some(512 * 1024 * 1024);
    reader.limits(limits);
    reader
        .decode()
        .map_err(|error| format!("undecodable image: {error}"))?;
    Ok(())
}

/// Hard ceiling for [`read_file_base64`]: the whole file is buffered in memory
/// (×1.33 as base64), so a caller-supplied limit must never be able to raise it.
const READ_BASE64_MAX_BYTES: u64 = 25 * 1024 * 1024;

/// Read a whole file as base64 (for client-side PDF text extraction). Errors if
/// the file exceeds `max_bytes` (default and cap 25MB) — extraction targets must
/// be small.
#[tauri::command]
pub fn read_file_base64(path: String, max_bytes: Option<u64>) -> Result<String, crate::AppError> {
    use base64::{engine::general_purpose::STANDARD, Engine as _};
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return Err("path cannot be empty.".to_string().into());
    }
    ensure_path_allowed(Path::new(trimmed))?;
    let meta = std::fs::metadata(trimmed)?;
    let limit = max_bytes
        .unwrap_or(READ_BASE64_MAX_BYTES)
        .clamp(1, READ_BASE64_MAX_BYTES);
    if meta.len() > limit {
        return Err(format!("File too large ({} bytes; limit {}).", meta.len(), limit).into());
    }
    Ok(STANDARD.encode(std::fs::read(trimmed)?))
}

/// A filesystem-safe, process-unique stamp (`<nanos>-<seq>`). The atomic seq
/// disambiguates the several attachments of one message, which are imported
/// concurrently and could otherwise collide on the same nanosecond.
fn unique_stamp() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|value| value.as_nanos())
        .unwrap_or(0);
    let seq = SEQ.fetch_add(1, Ordering::Relaxed);
    format!("{nanos}-{seq}")
}

/// Reduce an arbitrary string to a safe single path component (used for the
/// thread id, which becomes a directory name — guards against traversal).
fn safe_component(value: &str) -> String {
    value
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
        .collect()
}

/// Sanitize a display filename to safe chars while preserving the extension.
fn safe_file_name(name: &str) -> String {
    let base = std::path::Path::new(name)
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("image");
    let cleaned: String = base
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
        .collect();
    if cleaned.is_empty() {
        "image".to_string()
    } else {
        cleaned
    }
}

/// Decode an image and write a downscaled JPEG thumbnail under
/// `~/.future/app/images/<thread_id>/thumb/<stamp>.jpg`, returning its path.
/// Done entirely in Rust so the full-size image (up to tens of MB) never crosses
/// the IPC bridge to a webview canvas — only the tiny thumbnail is produced. The
/// decoder's allocation is capped to reject decompression bombs. Returns an error
/// that rejects the send while leaving the composer draft intact.
#[tauri::command]
pub fn generate_image_thumbnail(
    thread_id: String,
    source_path: String,
) -> Result<String, crate::AppError> {
    const MAX_EDGE: u32 = 256;

    let thread_id = safe_component(&thread_id);
    if thread_id.is_empty() {
        return Err("invalid thread id.".to_string().into());
    }
    let source = source_path.trim();
    // Never read a protected credential file as an image source — otherwise a
    // compromised webview could launder auth.json bytes out through the copy.
    ensure_path_allowed(Path::new(source))?;
    let size = std::fs::metadata(source)?.len();
    if size > MAX_ATTACHMENT_IMAGE_BYTES {
        return Err(format!(
            "Image is too large ({size} bytes; limit {MAX_ATTACHMENT_IMAGE_BYTES})."
        )
        .into());
    }
    let bytes = std::fs::read(source).map_err(|error| format!("unable to read image: {error}"))?;
    let mut reader = image::ImageReader::new(std::io::Cursor::new(&bytes))
        .with_guessed_format()
        .map_err(|error| format!("unreadable image: {error}"))?;
    let mut limits = image::Limits::default();
    limits.max_alloc = Some(512 * 1024 * 1024);
    reader.limits(limits);
    let img = reader
        .decode()
        .map_err(|error| format!("undecodable image: {error}"))?;
    let thumb = img.resize(MAX_EDGE, MAX_EDGE, image::imageops::FilterType::Lanczos3);

    let mut buf = Vec::new();
    let encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut buf, 70);
    image::ImageEncoder::write_image(
        encoder,
        thumb.to_rgb8().as_raw(),
        thumb.width(),
        thumb.height(),
        image::ExtendedColorType::Rgb8,
    )
    .map_err(|error| format!("thumbnail encode failed: {error}"))?;

    let dir = crate::store::thread_images_dir(&thread_id)?.join("thumb");
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(format!("{}.jpg", unique_stamp()));
    std::fs::write(&path, &buf)?;
    Ok(path.display().to_string())
}

/// Copy an ephemeral pasted-image original into
/// `~/.future/app/images/<thread_id>/origin/<stamp>_<name>` and return the new
/// path. Conversations don't save attachments into the workspace/project dir,
/// so the durable copy lives here (persistent, in the asset-protocol scope)
/// instead of the temp dir, which the OS may purge.
#[tauri::command]
pub fn import_ephemeral_image(
    thread_id: String,
    source_path: String,
    name: String,
) -> Result<String, crate::AppError> {
    let thread_id = safe_component(&thread_id);
    if thread_id.is_empty() {
        return Err("invalid thread id.".to_string().into());
    }
    let source = source_path.trim();
    if source.is_empty() {
        return Err("sourcePath cannot be empty.".to_string().into());
    }
    // Guard the source: a copy-out of a protected credential file (e.g.
    // auth.json) would otherwise defeat `ensure_path_allowed` — the copy lands
    // under a non-credential name/dir and becomes readable via read_file_base64.
    ensure_path_allowed(Path::new(source))?;
    let dir = crate::store::thread_images_dir(&thread_id)?.join("origin");
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(format!("{}_{}", unique_stamp(), safe_file_name(&name)));
    std::fs::copy(source, &path)?;
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

    ensure_path_allowed(Path::new(trimmed))?;
    open_path_with_system(trimmed)
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DirEntry {
    /// Last path component (display name).
    name: String,
    /// Absolute path to this entry.
    path: String,
    is_dir: bool,
    /// Byte size for files; 0 for directories.
    size: u64,
    /// Last-modified time as Unix epoch millis, or None if unavailable.
    modified: Option<u64>,
}

/// List a single directory level (no recursion) for the file-tree panel. The
/// tree lazy-loads each level by calling this on expand. Entries are sorted
/// directories-first, then case-insensitively by name. An individual entry that
/// can't be stat'd is skipped rather than failing the whole listing, and
/// symlinks are reported by their own metadata (not followed) so a symlink cycle
/// can't turn one directory read into an unbounded walk. `~/.future` internals
/// stay blocked via `ensure_path_allowed`.
#[tauri::command]
pub fn list_directory(path: String) -> Result<Vec<DirEntry>, crate::AppError> {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return Err("path cannot be empty.".to_string().into());
    }
    ensure_path_allowed(Path::new(trimmed))?;

    let mut entries: Vec<DirEntry> = Vec::new();
    for entry in std::fs::read_dir(trimmed)? {
        let Ok(entry) = entry else { continue };
        let Ok(meta) = entry.metadata() else { continue };
        let is_dir = meta.is_dir();
        let modified = meta
            .modified()
            .ok()
            .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|elapsed| elapsed.as_millis() as u64);
        entries.push(DirEntry {
            name: entry.file_name().to_string_lossy().to_string(),
            path: entry.path().to_string_lossy().to_string(),
            is_dir,
            size: if is_dir { 0 } else { meta.len() },
            modified,
        });
    }
    entries.sort_by(|a, b| {
        b.is_dir
            .cmp(&a.is_dir)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    Ok(entries)
}

/// Open an http(s) or mailto URL in the user's default handler. The scheme is
/// restricted to http/https/mailto so this can't be used to launch arbitrary
/// local handlers (`file:`, custom app schemes, …) via a crafted url.
#[tauri::command]
pub fn open_external_url(url: String) -> Result<(), crate::AppError> {
    let trimmed = url.trim();
    if !(trimmed.starts_with("http://")
        || trimmed.starts_with("https://")
        || trimmed.starts_with("mailto:"))
    {
        return Err("Only http(s) or mailto URLs can be opened."
            .to_string()
            .into());
    }

    open_path_with_system(trimmed)
}

/// Resolve a markdown link target encountered while previewing a local file into
/// an absolute path. `base_file` is the absolute path of the file being
/// previewed; a relative `target` resolves against that file's parent directory,
/// an absolute `target` is returned as-is. Pure path arithmetic — no filesystem
/// access — mirroring `resolve_file_reference` but anchored to the previewed
/// file's directory instead of a workspace root, so relative links in a previewed
/// document point at siblings on disk rather than at the workspace root.
#[tauri::command]
pub fn resolve_preview_link_path(
    base_file: String,
    target: String,
) -> Result<ResolvedPreviewLink, crate::AppError> {
    let target = target.trim();
    if target.is_empty() {
        return Err("target cannot be empty.".to_string().into());
    }

    let target_path = Path::new(target);
    let absolute = if target_path.is_absolute() {
        target_path.to_path_buf()
    } else {
        let base = Path::new(base_file.trim());
        base.parent()
            .unwrap_or_else(|| Path::new(""))
            .join(target_path)
    };

    let name = absolute
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_string();

    Ok(ResolvedPreviewLink {
        path: absolute.to_string_lossy().into_owned(),
        name,
    })
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

    ensure_path_allowed(Path::new(trimmed))?;
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
    ensure_path_allowed(Path::new(destination))?;

    if let Some(content) = content {
        std::fs::write(destination, content)?;
        return Ok(());
    }

    let source = source_path
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "sourcePath or content is required.".to_string())?;
    // The source must pass the same `~/.future` guard as the destination —
    // otherwise a copy-out defeats the guard (copy auth.json somewhere
    // readable, then preview it).
    ensure_path_allowed(Path::new(source))?;
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
    if bytes.len() as u64 > MAX_ATTACHMENT_IMAGE_BYTES {
        return Err(format!(
            "Pasted image is too large ({} bytes; limit {}).",
            bytes.len(),
            MAX_ATTACHMENT_IMAGE_BYTES
        )
        .into());
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

/// Hand the path/URL to the OS default handler via the `open` crate
/// (`open`/`xdg-open`/ShellExecuteW). Never route through `cmd /C start`:
/// cmd re-parses the argument, so `&`/`^`/`%VAR%` in an agent-produced path
/// would be interpreted — an injection vector, not just a broken open.
fn open_path_with_system(path: &str) -> Result<(), crate::AppError> {
    open::that(path).or_else(|_| {
        // macOS may not have a default handler for extensionless / dotfiles
        // (e.g. `.env`).  Fall back to `open -t` which forces the default
        // text editor.
        #[cfg(target_os = "macos")]
        {
            let status = std::process::Command::new("open")
                .arg("-t")
                .arg(path)
                .status()
                .map_err(|e| format!("Failed to open: {e}"))?;
            if status.success() {
                Ok(())
            } else {
                Err(format!("Failed to open {path}: open -t exited with {status}").into())
            }
        }
        #[cfg(not(target_os = "macos"))]
        Err(format!("Failed to open: {path}").into())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    /// Fresh, canonicalized fake `~/.future` root for one test.
    fn future_root(name: &str) -> PathBuf {
        let base =
            std::env::temp_dir().join(format!("futureos_files_{}_{name}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(&base).unwrap();
        base.canonicalize().unwrap()
    }

    fn write_under(root: &Path, rel: &str) -> PathBuf {
        let path = root.join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, b"x").unwrap();
        best_effort_canonical(&path)
    }

    #[test]
    fn credential_files_are_protected() {
        let future_dir = future_root("block_creds");
        for rel in [
            "agent/auth.json",
            "agent/models.json",
            "agent-app/auth.json",
        ] {
            let cred = write_under(&future_dir, rel);
            assert!(
                is_protected_credential(&future_dir, &cred),
                "{rel} must be blocked"
            );
        }
    }

    #[test]
    fn content_and_config_are_readable() {
        let future_dir = future_root("allow_content");
        // Attachment images, the app DB, session history, settings, workspace
        // files — none are credentials, so all stay readable.
        for rel in [
            "app/images/thread_x/origin/1-pic.png",
            "app/images/thread_x/thumb/2.jpg",
            "app/app.db",
            "agent/sessions/s.jsonl",
            "agent/settings.json",
            "workspaces/chat/thread_x/长诗.md",
            // Same filename as a credential but not in an agent config dir.
            "workspaces/chat/thread_x/models.json",
        ] {
            let file = write_under(&future_dir, rel);
            assert!(
                !is_protected_credential(&future_dir, &file),
                "{rel} must be readable"
            );
        }
    }

    #[test]
    fn preview_link_resolves_relative_against_base_file_dir() {
        let resolved =
            resolve_preview_link_path("/docs/guide/index.md".into(), "../assets/logo.png".into())
                .unwrap();
        assert_eq!(resolved.path, "/docs/guide/../assets/logo.png");
        assert_eq!(resolved.name, "logo.png");
    }

    #[test]
    fn preview_link_keeps_absolute_target() {
        let resolved =
            resolve_preview_link_path("/docs/guide/index.md".into(), "/etc/notes.md".into())
                .unwrap();
        assert_eq!(resolved.path, "/etc/notes.md");
        assert_eq!(resolved.name, "notes.md");
    }

    #[test]
    fn preview_link_rejects_empty_target() {
        assert!(resolve_preview_link_path("/docs/index.md".into(), "  ".into()).is_err());
    }

    #[test]
    fn image_validation_accepts_decodable_image_and_rejects_fake_image() {
        let root = future_root("validate_image");
        let valid = root.join("valid.png");
        image::RgbImage::from_pixel(2, 2, image::Rgb([1, 2, 3]))
            .save(&valid)
            .unwrap();
        assert!(validate_image_attachment(valid.display().to_string()).is_ok());

        let invalid = root.join("invalid.png");
        fs::write(&invalid, b"not an image").unwrap();
        assert!(validate_image_attachment(invalid.display().to_string()).is_err());
    }
}
