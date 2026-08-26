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
/// without this guard an XSS could read the credential files and exfiltrate
/// API keys, remote bridge identity, or IM channel secrets.
///
/// This is a **denylist of credential files only** (`auth.json` / `models.json`
/// in the agent config dirs, `remote_pairing.json` at the `~/.future` root, and
/// `channels/config.json`). Everything else — attachment images, workspace
/// files, the app DB, session history, settings, run logs, and any user-chosen
/// file elsewhere — is allowed: the webview already reaches all of that through
/// typed store/session commands, so the raw bytes aren't a new exposure. The
/// only thing it never legitimately needs as raw bytes is the credentials.
pub(crate) fn ensure_path_allowed(path: &Path) -> Result<(), crate::AppError> {
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
/// `auth.json` / `models.json` directly in an agent config dir
/// (`~/.future/agent/` or `~/.future/agent-app/`), `remote_pairing.json` at the
/// `~/.future` root, or `channels/config.json`. Scoped so a user's own file that
/// merely shares the name (e.g. a workspace `models.json`) stays readable.
fn is_protected_credential(future_dir: &Path, resolved: &Path) -> bool {
    let name = resolved
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default();

    // Remote bridge identity (nkey_seed + user_jwt) sits directly under ~/.future.
    if name == "remote_pairing.json" && resolved.parent() == Some(future_dir) {
        return true;
    }

    // IM channel secrets live in ~/.future/channels/config.json.
    if name == "config.json" {
        let channels_dir = future_dir.join("channels");
        let channels_dir = channels_dir.canonicalize().unwrap_or(channels_dir);
        if resolved.parent() == Some(channels_dir.as_path()) {
            return true;
        }
    }

    if name != "auth.json" && name != "models.json" {
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
    valid_utf8: bool,
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

const IMAGE_PREVIEW_MAX_BYTES: u64 = 25 * 1024 * 1024;

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImagePreviewAsset {
    path: String,
    version: String,
}

/// Validate one local image path and expose exactly that canonical file through
/// Tauri's asset protocol. The WebView then reads the bytes directly instead of
/// receiving a 1.33x Base64 string over JSON IPC.
#[tauri::command]
pub fn prepare_image_preview(
    app: tauri::AppHandle,
    path: String,
) -> Result<ImagePreviewAsset, crate::AppError> {
    prepare_image_preview_with(&app, &path)
}

fn prepare_image_preview_with<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    path: &str,
) -> Result<ImagePreviewAsset, crate::AppError> {
    use tauri::Manager;

    let (canonical, size) = validate_image_preview_path(path)?;
    app.asset_protocol_scope()
        .allow_file(&canonical)
        .map_err(|error| format!("failed to authorize image preview path: {error}"))?;
    Ok(ImagePreviewAsset {
        path: canonical.display().to_string(),
        // The asset URL is path-based. Give every preparation a fresh version so
        // reopening a changed file never reuses an old WebView cache entry.
        version: format!("{}-{size}", unique_stamp()),
    })
}

fn validate_image_preview_path(path: &str) -> Result<(PathBuf, u64), crate::AppError> {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return Err("path cannot be empty.".to_string().into());
    }
    ensure_path_allowed(Path::new(trimmed))?;
    let canonical = Path::new(trimmed).canonicalize()?;
    let meta = std::fs::metadata(&canonical)?;
    if !meta.is_file() {
        return Err("image preview path must be a file.".to_string().into());
    }
    if meta.len() > IMAGE_PREVIEW_MAX_BYTES {
        return Err(format!(
            "File too large ({} bytes; limit {}).",
            meta.len(),
            IMAGE_PREVIEW_MAX_BYTES
        )
        .into());
    }
    Ok((canonical, meta.len()))
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
    // under a non-credential name/dir and becomes readable via the asset protocol.
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
    open_path_with(&path, open_path_with_system)
}

/// Validation + opener, with the OS layer injectable so the happy path and
/// the OS-error path are testable without launching a GUI app.
fn open_path_with(
    path: &str,
    opener: impl Fn(&str) -> Result<(), crate::AppError>,
) -> Result<(), crate::AppError> {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return Err("path cannot be empty.".to_string().into());
    }

    ensure_path_allowed(Path::new(trimmed))?;
    opener(trimmed)
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
    open_external_url_with(&url, open_path_with_system)
}

/// Validation + opener with the OS layer injectable (see [`open_path_with`]).
fn open_external_url_with(
    url: &str,
    opener: impl Fn(&str) -> Result<(), crate::AppError>,
) -> Result<(), crate::AppError> {
    let trimmed = url.trim();
    let normalized = trimmed.to_ascii_lowercase();
    if !(normalized.starts_with("http://")
        || normalized.starts_with("https://")
        || normalized.starts_with("mailto:"))
    {
        return Err("Only http(s) or mailto URLs can be opened."
            .to_string()
            .into());
    }

    opener(trimmed)
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

    let valid_utf8 = std::str::from_utf8(&buffer).is_ok();
    Ok(TextFilePreview {
        content: String::from_utf8_lossy(&buffer).to_string(),
        size,
        truncated,
        valid_utf8,
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

/// Lowercase base36 encoding of a u128 (compact filename component).
fn to_base36(mut n: u128) -> String {
    const DIGITS: &[u8; 36] = b"0123456789abcdefghijklmnopqrstuvwxyz";
    if n == 0 {
        return "0".to_string();
    }
    let mut buf = Vec::new();
    while n > 0 {
        buf.push(DIGITS[(n % 36) as usize]);
        n /= 36;
    }
    buf.reverse();
    String::from_utf8(buf).expect("base36 digits are valid ASCII")
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

    // Base36-encode the nanosecond timestamp: same uniqueness as the raw value
    // but ~12 chars instead of 19, so the chip label stays short enough to read.
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|value| value.as_nanos())
        .unwrap_or(0);
    let name = format!("pasted-{}.{ext}", to_base36(nanos));
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
#[cfg(target_os = "macos")]
fn open_path_with_system(path: &str) -> Result<(), crate::AppError> {
    open::that(path).or_else(|_| fallback_open_text(path))
}

/// The `open -t` fallback: spawn + classify, split so the status classifier is
/// pure (spawn itself is an OS seam).
#[cfg(target_os = "macos")]
fn fallback_open_text(path: &str) -> Result<(), crate::AppError> {
    let status = std::process::Command::new("open")
        .arg("-t")
        .arg(path)
        .status()
        .map_err(|e| format!("Failed to open: {e}"))?;
    open_t_status_to_result(status.success(), path)
}

/// Pure classifier for the `open -t` fallback's exit status.
#[cfg(target_os = "macos")]
fn open_t_status_to_result(success: bool, path: &str) -> Result<(), crate::AppError> {
    if success {
        Ok(())
    } else {
        Err(format!("Failed to open {path}: open -t exited with an error").into())
    }
}

#[cfg(not(target_os = "macos"))]
fn open_path_with_system(path: &str) -> Result<(), crate::AppError> {
    open::that(path).map_err(|_| format!("Failed to open: {path}").into())
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
            "remote_pairing.json",
            "channels/config.json",
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
            // config.json only counts as a credential under ~/.future/channels/.
            "workspaces/chat/thread_x/config.json",
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
        // Joined with the platform separator (e.g. `guide\../assets` on
        // Windows) — still a valid, equivalent path for the local OS.
        let expected = Path::new("/docs/guide").join("../assets/logo.png");
        assert_eq!(resolved.path, expected.to_string_lossy().into_owned());
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

    // ---- broader command coverage ----

    /// A sparse 26MB file (over the 25MB attachment cap) without materializing
    /// the bytes — `set_len` creates a hole that still reports the size.
    fn sparse_large(root: &Path, name: &str) -> PathBuf {
        let path = root.join(name);
        let file = fs::File::create(&path).unwrap();
        file.set_len(26 * 1024 * 1024).unwrap();
        path
    }

    #[test]
    fn best_effort_canonical_resolves_each_shape() {
        let root = future_root("canonical");
        let existing = write_under(&root, "a/b.txt");
        assert_eq!(best_effort_canonical(&existing), existing);

        // Missing tail re-appends under the canonicalized nearest ancestor.
        let missing_tail = root.join("a/c/d.txt");
        let resolved = best_effort_canonical(&missing_tail);
        assert!(resolved.ends_with("a/c/d.txt"));

        // A path whose ancestor never exists falls back to the input verbatim.
        let ghost = Path::new("/definitely/not/here/x/y/z.txt");
        assert_eq!(best_effort_canonical(ghost), ghost);
    }

    #[test]
    fn ensure_path_allowed_blocks_credentials_and_allows_others() {
        let home = crate::auth_store::test_support::HomeGuard::new("files_allowed");
        let home_dir = std::env::var("HOME").unwrap();
        let cred = write_under(Path::new(&home_dir), ".future/agent/auth.json");
        assert!(ensure_path_allowed(&cred).is_err());
        let ordinary = future_root("allowed_ordinary").join("app.db");
        fs::write(&ordinary, b"x").unwrap();
        assert!(ensure_path_allowed(&ordinary).is_ok());
        drop(home);
    }

    #[test]
    fn ensure_path_allowed_is_a_noop_without_a_home_dir() {
        let lock = crate::TEST_HOME_LOCK
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        // Other tests can intentionally leave HOME unset. Preserve its exact
        // prior state; USERPROFILE is a Windows-only fallback absent on
        // macOS/Linux.
        let old_home = std::env::var_os("HOME");
        std::env::remove_var("HOME");
        std::env::remove_var("USERPROFILE");

        let file = write_under(&future_root("no_home"), "ok.txt");
        assert!(ensure_path_allowed(&file).is_ok());

        if let Some(old_home) = old_home {
            std::env::set_var("HOME", old_home);
        } else {
            std::env::remove_var("HOME");
        }
        std::env::remove_var("USERPROFILE");
        drop(lock);
    }

    #[test]
    fn inspect_attachment_classifies_dir_text_and_binary() {
        let root = future_root("inspect");
        assert!(inspect_attachment("   ".into()).is_err());

        let dir = root.join("sub");
        fs::create_dir_all(&dir).unwrap();
        let info = inspect_attachment(dir.display().to_string()).unwrap();
        assert!(info.is_dir);

        let text = root.join("notes.txt");
        fs::write(&text, b"hello world\n").unwrap();
        let info = inspect_attachment(text.display().to_string()).unwrap();
        assert!(!info.is_dir && !info.is_binary);

        let binary = root.join("blob.bin");
        fs::write(&binary, [0u8, 1, 2, 3]).unwrap();
        let info = inspect_attachment(binary.display().to_string()).unwrap();
        assert!(info.is_binary);
    }

    #[test]
    fn validate_image_rejects_empty_and_oversized() {
        let root = future_root("validate_edges");
        assert!(validate_image_attachment("   ".into()).is_err());
        let big = sparse_large(&root, "big.png");
        assert!(validate_image_attachment(big.display().to_string()).is_err());
    }

    #[test]
    fn validate_image_preview_path_covers_edges() {
        let root = future_root("image_preview");
        assert!(validate_image_preview_path("   ").is_err());
        let big = sparse_large(&root, "big.bin");
        assert!(validate_image_preview_path(&big.display().to_string()).is_err());

        let small = root.join("small.png");
        fs::write(&small, b"hello").unwrap();
        let (canonical, size) = validate_image_preview_path(&small.display().to_string()).unwrap();
        assert_eq!(canonical, small.canonicalize().unwrap());
        assert_eq!(size, 5);
    }

    #[test]
    fn prepare_image_preview_authorizes_only_the_canonical_file() {
        use tauri::Manager;

        let root = future_root("image_preview_scope");
        let source = root.join("source.png");
        let other = root.join("other.png");
        fs::write(&source, b"image bytes").unwrap();
        fs::write(&other, b"other image bytes").unwrap();
        let app = tauri::test::mock_app();

        let asset =
            prepare_image_preview_with(app.handle(), &source.display().to_string()).unwrap();
        let refreshed =
            prepare_image_preview_with(app.handle(), &source.display().to_string()).unwrap();
        let canonical = source.canonicalize().unwrap();
        assert_eq!(asset.path, canonical.display().to_string());
        assert_ne!(asset.version, refreshed.version);
        assert!(app.asset_protocol_scope().is_allowed(&canonical));
        assert!(!app.asset_protocol_scope().is_allowed(other));
    }

    #[test]
    fn thumbnail_rejects_bad_thread_and_oversized_source() {
        let home = crate::auth_store::test_support::HomeGuard::new("files_thumb");
        assert!(generate_image_thumbnail("!!!".into(), "/x.png".into()).is_err());
        let root = future_root("thumb_src");
        let big = sparse_large(&root, "big.png");
        assert!(generate_image_thumbnail("thread_x".into(), big.display().to_string()).is_err());
        drop(home);
    }

    #[test]
    fn thumbnail_generates_a_jpeg_for_a_real_image() {
        let home = crate::auth_store::test_support::HomeGuard::new("files_thumb_ok");
        let root = future_root("thumb_src2");
        let src = root.join("in.png");
        image::RgbImage::from_pixel(8, 8, image::Rgb([7, 8, 9]))
            .save(&src)
            .unwrap();
        let thumb = generate_image_thumbnail("thread_x".into(), src.display().to_string()).unwrap();
        assert!(thumb.ends_with(".jpg"));
        assert!(Path::new(&thumb).is_file());
        drop(home);
    }

    #[test]
    fn import_ephemeral_image_covers_edges() {
        let home = crate::auth_store::test_support::HomeGuard::new("files_import");
        assert!(import_ephemeral_image("!!!".into(), "/x.png".into(), "x.png".into()).is_err());
        assert!(import_ephemeral_image("t".into(), "   ".into(), "x.png".into()).is_err());

        let root = future_root("import_src");
        let src = root.join("src.png");
        image::RgbImage::from_pixel(1, 1, image::Rgb([1, 1, 1]))
            .save(&src)
            .unwrap();
        let dest = import_ephemeral_image(
            "thread_x".into(),
            src.display().to_string(),
            "name.png".into(),
        )
        .unwrap();
        assert!(Path::new(&dest).is_file());
        drop(home);
    }

    #[test]
    fn delete_temp_attachment_refuses_outside_the_attachment_dir() {
        let root = future_root("delete_temp");
        let outside = write_under(&root, "keep.txt");
        assert!(delete_temp_attachment(outside.display().to_string()).is_err());
    }

    #[test]
    fn delete_temp_attachment_removes_an_attachment_file() {
        let base = std::env::temp_dir().join("futureos-attachments");
        fs::create_dir_all(&base).unwrap();
        let file = base.join(format!("test-{}.tmp", std::process::id()));
        fs::write(&file, b"x").unwrap();
        assert!(delete_temp_attachment(file.display().to_string()).is_ok());
        assert!(!file.exists());
    }

    #[test]
    fn open_path_and_url_validate_before_reaching_the_os() {
        assert!(open_path("   ".into()).is_err());
        assert!(open_external_url("file:///etc/passwd".into()).is_err());
        assert!(open_external_url("ftp://x".into()).is_err());
    }

    #[test]
    fn open_path_with_injects_the_opener_for_both_arms() {
        let _home = crate::auth_store::test_support::HomeGuard::new("files_open_seam");
        assert!(open_path_with("   ", |_| unreachable!()).is_err());
        assert!(open_path_with("/tmp/ok", |_| Ok(())).is_ok());
        assert!(open_path_with("/tmp/err", |_| Err("os failed".to_string().into())).is_err());
        // Credential files stay blocked even with an injected opener.
        let home = std::env::var("HOME").expect("test home");
        let agent_dir = std::path::Path::new(&home).join(".future/agent");
        std::fs::create_dir_all(&agent_dir).unwrap();
        std::fs::write(agent_dir.join("auth.json"), "{}").unwrap();
        let cred = agent_dir.join("auth.json");
        assert!(open_path_with(cred.to_str().unwrap(), |_| unreachable!()).is_err());
    }

    #[test]
    fn open_external_url_with_injects_the_opener_for_both_arms() {
        assert!(open_external_url_with("https://ok.example/", |_| Ok(())).is_ok());
        assert!(open_external_url_with("mailto:x@example.com", |_| Ok(())).is_ok());
        assert!(
            open_external_url_with("https://x", |_| Err("os failed".to_string().into())).is_err()
        );
        assert!(open_external_url_with("ftp://x", |_| unreachable!()).is_err());
        assert!(open_external_url_with("   ", |_| unreachable!()).is_err());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn open_t_status_classifier_maps_both_arms() {
        assert!(open_t_status_to_result(true, "x").is_ok());
        assert!(open_t_status_to_result(false, "x").is_err());
    }

    #[test]
    fn open_path_runs_the_real_os_fallback_chain() {
        let _home = crate::auth_store::test_support::HomeGuard::new("files_open_os");
        // A path whose PARENT does not exist: `open` and `open -t` both fail
        // fast without launching any GUI app, so the fallback chain runs
        // end-to-end and returns the OS error.
        let ghost = "/definitely-not-a-real-dir-cov100/file.txt";
        let err = open_path(ghost.to_string()).unwrap_err();
        assert!(!err.to_string().is_empty());
    }

    #[test]
    fn list_directory_rejects_empty_and_lists_entries() {
        assert!(list_directory("   ".into()).is_err());
        let root = future_root("list_dir");
        fs::write(root.join("b.txt"), b"x").unwrap();
        fs::create_dir_all(root.join("a_dir")).unwrap();
        let entries = list_directory(root.display().to_string()).unwrap();
        assert_eq!(entries.len(), 2);
        // Directories sort first.
        assert!(entries[0].is_dir);
    }

    #[test]
    fn read_text_file_preview_covers_edges() {
        assert!(read_text_file_preview("   ".into(), None).is_err());
        let root = future_root("preview");
        let file = root.join("long.txt");
        fs::write(&file, b"0123456789abcdef").unwrap();
        let preview = read_text_file_preview(file.display().to_string(), Some(4)).unwrap();
        assert!(preview.truncated);
        assert!(preview.valid_utf8);
        assert_eq!(preview.content, "0123");

        let invalid = root.join("invalid.json");
        fs::write(&invalid, [0xff, 0xfe]).unwrap();
        let preview = read_text_file_preview(invalid.display().to_string(), None).unwrap();
        assert!(!preview.valid_utf8);
    }

    #[test]
    fn export_artifact_file_covers_all_branches() {
        let root = future_root("export");
        assert!(export_artifact_file("   ".into(), None, None).is_err());

        // Content branch.
        let dest = root.join("out.md");
        assert!(
            export_artifact_file(dest.display().to_string(), None, Some("# hi".into())).is_ok()
        );
        assert_eq!(fs::read_to_string(&dest).unwrap(), "# hi");

        // Source copy branch.
        let src = root.join("src.md");
        fs::write(&src, b"src").unwrap();
        let dest2 = root.join("out2.md");
        assert!(export_artifact_file(
            dest2.display().to_string(),
            Some(src.display().to_string()),
            None
        )
        .is_ok());
        assert_eq!(fs::read_to_string(&dest2).unwrap(), "src");

        // Missing both source and content.
        assert!(export_artifact_file(dest.display().to_string(), None, None).is_err());
    }

    #[test]
    fn to_base36_handles_zero_and_positive() {
        assert_eq!(to_base36(0), "0");
        assert_eq!(to_base36(35), "z");
        assert_eq!(to_base36(36), "10");
    }

    #[test]
    fn save_pasted_image_covers_edges() {
        assert!(save_pasted_image(vec![], None).is_err());
        assert!(save_pasted_image(vec![0u8; 26 * 1024 * 1024], None).is_err());
        let saved = save_pasted_image(vec![1, 2, 3], Some("PNG".into())).unwrap();
        assert!(saved.name.ends_with(".png"));
        assert!(Path::new(&saved.path).is_file());
    }

    #[test]
    fn canonical_falls_back_for_a_path_with_no_existing_ancestor() {
        // A bare relative name that does not exist walks up to an empty path
        // (which has no parent), so the `_` arm returns the input unchanged.
        let ghost = Path::new("futureos_definitely_not_a_real_file_xyz");
        assert_eq!(best_effort_canonical(ghost), ghost);
    }

    #[test]
    fn safe_file_name_falls_back_to_image_when_everything_is_filtered() {
        assert_eq!(safe_file_name("你好"), "image");
        assert_eq!(safe_file_name("report.md"), "report.md");
    }
}
