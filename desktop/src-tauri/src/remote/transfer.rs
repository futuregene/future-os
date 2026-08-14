//! Pair-scoped file transfer over the reserved NATS `xfer.up/down` subjects.
//!
//! Control-plane commands create/seal transfers. File bytes stay binary on
//! NATS (never JSON/base64) and are written incrementally so the relay's 1 MiB
//! payload limit and the desktop's memory use remain bounded.

use futures::StreamExt;
use serde::Serialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::{HashMap, HashSet},
    fs::{File, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    sync::{LazyLock, Mutex},
    time::{Duration, SystemTime},
};

use crate::agent_bridge::AttachmentInput;

pub const MAX_FILE_BYTES: u64 = 10 * 1024 * 1024;
pub const MAX_MESSAGE_BYTES: u64 = 20 * 1024 * 1024;
pub const MAX_ATTACHMENTS: usize = 10;
pub const MAX_IMAGES: usize = 4;
const MOBILE_PREVIEW_MAX_EDGE: u32 = 1600;
pub const CHUNK_BYTES: u64 = 512 * 1024;
const TRANSFER_TTL: Duration = Duration::from_secs(30 * 60);

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UploadReference {
    pub upload_id: String,
}

#[derive(Debug, Clone)]
struct UploadRecord {
    path: PathBuf,
    name: String,
    transfer_name: String,
    kind: String,
    original_size: u64,
    transfer_size: u64,
    received: u64,
    next_chunk: u64,
    complete: bool,
    created_at: SystemTime,
}

#[derive(Debug, Clone)]
struct DownloadRecord {
    path: PathBuf,
    size: u64,
    created_at: SystemTime,
}

static UPLOADS: LazyLock<Mutex<HashMap<String, UploadRecord>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));
static DOWNLOADS: LazyLock<Mutex<HashMap<String, DownloadRecord>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UploadInitResult {
    upload_id: String,
    chunk_bytes: u64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UploadCompleteResult {
    upload_id: String,
    content_hash: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadInfo {
    transfer_id: String,
    name: String,
    mime_type: String,
    size: u64,
    content_hash: String,
    preview_kind: String,
    chunk_bytes: u64,
}

fn transfer_root() -> PathBuf {
    std::env::temp_dir()
        .join("futureos-attachments")
        .join("mobile")
}

/// Create a staging directory with owner-only permissions (0700 on unix).
/// Attachment bytes (uploaded `.part` files and downloaded previews) sit here
/// until claimed or TTL'd — on a multi-user machine the default `/tmp` mode
/// would expose them to any local user. Windows has no 0700 equivalent; the
/// per-user profile directory is relied on there, matching `config_io`.
fn ensure_private_dir(path: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let permissions = std::fs::Permissions::from_mode(0o700);
        std::fs::set_permissions(path, permissions)?;
    }
    #[cfg(not(unix))]
    {
        let _ = path;
    }
    Ok(())
}

fn new_transfer_id(prefix: &str) -> String {
    format!("{prefix}_{}", nkeys::KeyPair::new_user().public_key())
}

fn display_name(name: &str, fallback: &str) -> String {
    // Picker names are user-facing metadata, not paths. Preserve Unicode while
    // stripping either platform's path prefix and non-printing characters.
    let base = name.rsplit(['/', '\\']).next().unwrap_or(fallback);
    let cleaned: String = base.chars().filter(|c| !c.is_control()).collect();
    if cleaned.trim_matches('.').trim().is_empty() {
        fallback.to_string()
    } else {
        cleaned
    }
}

fn safe_disk_name(name: &str, fallback: &str) -> String {
    let base = display_name(name, fallback);
    let cleaned: String = base
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | ' '))
        .collect();
    if cleaned.trim_matches('.').trim().is_empty() {
        fallback.to_string()
    } else {
        cleaned
    }
}

fn prune_expired() {
    let expired = |created: SystemTime| created.elapsed().unwrap_or_default() > TRANSFER_TTL;
    let mut uploads = UPLOADS.lock().unwrap();
    uploads.retain(|_, item| {
        let keep = !expired(item.created_at);
        if !keep {
            let _ = std::fs::remove_file(&item.path);
        }
        keep
    });
    drop(uploads);
    let mut downloads = DOWNLOADS.lock().unwrap();
    downloads.retain(|_, item| {
        let keep = !expired(item.created_at);
        if !keep {
            let _ = std::fs::remove_file(&item.path);
        }
        keep
    });
}

pub fn init_upload(
    name: &str,
    transfer_name: &str,
    _mime_type: &str,
    kind: &str,
    original_size: u64,
    transfer_size: u64,
) -> Result<UploadInitResult, crate::AppError> {
    prune_expired();
    if original_size == 0 || original_size > MAX_FILE_BYTES {
        return Err(
            format!("Original file exceeds the 10 MiB limit ({original_size} bytes).").into(),
        );
    }
    if transfer_size == 0 || transfer_size > MAX_FILE_BYTES {
        return Err(
            format!("Transfer file exceeds the 10 MiB limit ({transfer_size} bytes).").into(),
        );
    }
    if !matches!(kind, "image" | "file") {
        return Err("Unsupported attachment kind.".to_string().into());
    }
    let upload_id = new_transfer_id("upload");
    let dir = transfer_root().join("upload");
    ensure_private_dir(&dir)?;
    let path = dir.join(format!("{upload_id}.part"));
    File::create(&path)?;
    UPLOADS.lock().unwrap().insert(
        upload_id.clone(),
        UploadRecord {
            path,
            name: display_name(name, "attachment"),
            transfer_name: safe_disk_name(
                if transfer_name.trim().is_empty() {
                    name
                } else {
                    transfer_name
                },
                "attachment",
            ),
            kind: kind.to_string(),
            original_size,
            transfer_size,
            received: 0,
            next_chunk: 0,
            complete: false,
            created_at: SystemTime::now(),
        },
    );
    Ok(UploadInitResult {
        upload_id,
        chunk_bytes: CHUNK_BYTES,
    })
}

pub fn complete_upload(upload_id: &str) -> Result<UploadCompleteResult, crate::AppError> {
    let mut uploads = UPLOADS.lock().unwrap();
    let item = uploads
        .get_mut(upload_id)
        .ok_or_else(|| crate::AppError::Message("Upload expired or does not exist.".to_string()))?;
    if item.received != item.transfer_size {
        return Err(format!(
            "Upload is incomplete ({} of {} bytes).",
            item.received, item.transfer_size
        )
        .into());
    }
    let content_hash = sha256_file(&item.path)?;
    item.complete = true;
    Ok(UploadCompleteResult {
        upload_id: upload_id.to_string(),
        content_hash,
    })
}

pub fn cancel_upload(upload_id: &str) -> Result<(), crate::AppError> {
    if let Some(item) = UPLOADS.lock().unwrap().remove(upload_id) {
        let _ = std::fs::remove_file(item.path);
    }
    Ok(())
}

pub fn claim_uploads(
    references: &[UploadReference],
    thread_id: &str,
) -> Result<Vec<AttachmentInput>, crate::AppError> {
    if references.len() > MAX_ATTACHMENTS {
        return Err(format!("A message can contain at most {MAX_ATTACHMENTS} attachments.").into());
    }
    let uploads = UPLOADS.lock().unwrap();
    let mut total = 0_u64;
    let mut images = 0_usize;
    let mut seen = HashSet::new();
    let mut claimed = Vec::with_capacity(references.len());
    for reference in references {
        if !seen.insert(reference.upload_id.clone()) {
            return Err("The same attachment cannot be added twice."
                .to_string()
                .into());
        }
        let item = uploads.get(&reference.upload_id).ok_or_else(|| {
            crate::AppError::Message("An uploaded attachment expired; upload it again.".to_string())
        })?;
        if !item.complete {
            return Err("An attachment is still uploading.".to_string().into());
        }
        total = total.saturating_add(item.original_size);
        if item.kind == "image" {
            images += 1;
        }
        claimed.push((reference.upload_id.clone(), item.clone()));
    }
    if total > MAX_MESSAGE_BYTES {
        return Err("Attachments exceed the 20 MiB per-message limit."
            .to_string()
            .into());
    }
    if images > MAX_IMAGES {
        return Err(format!("A message can contain at most {MAX_IMAGES} images.").into());
    }
    drop(uploads);

    // Image probing/decoding can involve disk I/O and substantial CPU. Keep it
    // outside the global upload-map lock so unrelated transfers remain live.
    for (_, item) in &claimed {
        if item.kind == "image" {
            validate_mobile_image(&item.path)?;
        }
    }

    let destination = crate::store::thread_images_dir(thread_id)?.join("origin");
    std::fs::create_dir_all(&destination)?;
    let mut targets = Vec::with_capacity(claimed.len());
    for (upload_id, item) in &claimed {
        let target = destination.join(format!(
            "{}_{}",
            upload_id,
            safe_disk_name(&item.transfer_name, "attachment")
        ));
        if let Err(error) = std::fs::copy(&item.path, &target) {
            for copied in &targets {
                let _ = std::fs::remove_file(copied);
            }
            return Err(error.into());
        }
        targets.push(target);
    }

    let mut result = Vec::with_capacity(claimed.len());
    for ((_, item), target) in claimed.iter().zip(targets.iter()) {
        let target_string = target.display().to_string();
        let thumbnail = if item.kind == "image" {
            crate::commands::generate_image_thumbnail(thread_id.to_string(), target_string.clone())
                .ok()
        } else {
            None
        };
        result.push(AttachmentInput {
            path: target_string,
            kind: item.kind.clone(),
            name: item.name.clone(),
            thumbnail,
        });
    }
    let mut uploads = UPLOADS.lock().unwrap();
    for (upload_id, item) in claimed {
        uploads.remove(&upload_id);
        let _ = std::fs::remove_file(item.path);
    }
    Ok(result)
}

pub fn rollback_claimed(attachments: &[AttachmentInput]) {
    for attachment in attachments {
        let _ = std::fs::remove_file(&attachment.path);
        if let Some(thumbnail) = &attachment.thumbnail {
            let _ = std::fs::remove_file(thumbnail);
        }
    }
}

fn validate_mobile_image(path: &Path) -> Result<(), crate::AppError> {
    // Mobile downsamples every image pick to 1600 px before upload, so no
    // dimension cap is enforced here; still decode once so a valid-looking
    // header cannot smuggle corrupt image bytes into the agent, but cap
    // allocation for the already-bounded image.
    let mut reader = image::ImageReader::open(path)?
        .with_guessed_format()
        .map_err(|error| format!("Unreadable image: {error}"))?;
    let mut limits = image::Limits::default();
    limits.max_alloc = Some(128 * 1024 * 1024);
    reader.limits(limits);
    reader
        .decode()
        .map_err(|error| format!("Undecodable image: {error}"))?;
    Ok(())
}

fn sha256_file(path: &Path) -> Result<String, crate::AppError> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = [0_u8; 64 * 1024];
    loop {
        let read = file.read(&mut buf)?;
        if read == 0 {
            break;
        }
        hasher.update(&buf[..read]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

#[cfg(test)]
fn attachment_is_in_session(entries: &Value, requested: &str) -> bool {
    session_attachment_name(entries, requested).is_some()
}

fn session_attachment_name(entries: &Value, requested: &str) -> Option<String> {
    entries
        .get("entries")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|entry| entry.get("meta"))
        .filter_map(|meta| meta.get("attachments"))
        .filter_map(Value::as_array)
        .flatten()
        .find(|attachment| attachment.get("path").and_then(Value::as_str) == Some(requested))
        .map(|attachment| {
            attachment
                .get("name")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned)
                .unwrap_or_else(|| {
                    Path::new(requested)
                        .file_name()
                        .and_then(|value| value.to_str())
                        .unwrap_or("attachment")
                        .to_string()
                })
        })
}

/// Resolve a markdown local-file link to a path on this machine: absolute
/// targets are used as-is; relative targets resolve against the session's
/// working directory (its thread workspace) — the directory the model writes
/// its links relative to. Returns the canonicalized path when it exists.
fn resolve_local_link(session_id: &str, requested_path: &str) -> Option<PathBuf> {
    let requested = Path::new(requested_path);
    if requested.is_absolute() {
        return requested.canonicalize().ok();
    }
    let thread = crate::store::find_thread_by_agent_session(session_id).ok()??;
    let cwd = crate::agent_bridge::workspace_path_for_thread(&thread.id).ok()?;
    Path::new(&cwd).join(requested).canonicalize().ok()
}

pub async fn prepare_download(
    session_id: &str,
    requested_path: &str,
) -> Result<DownloadInfo, crate::AppError> {
    prune_expired();
    let entries = crate::agent_bridge::get_session_entries(session_id.to_string()).await?;
    let (source, display_name) = match session_attachment_name(&entries, requested_path) {
        Some(name) => (Path::new(requested_path).canonicalize()?, name),
        // Not a session attachment: treat the request as a markdown local-file
        // link — absolute targets are used as-is, relative ones resolve against
        // the session's working directory. A paired phone is a trusted device
        // (it can already read any file via the agent), matching the desktop
        // UI, which opens model-written links against the local disk.
        None => {
            let source = resolve_local_link(session_id, requested_path).ok_or_else(|| {
                crate::AppError::Message(
                    "The requested file is not an attachment in this session.".to_string(),
                )
            })?;
            (source, requested_path.to_string())
        }
    };
    if !source.is_file() {
        return Err("The attachment is no longer available.".to_string().into());
    }
    let prepared = prepare_preview(&source, &display_name)?;
    // prepare_preview already bounds its output: text/markdown previews are
    // byte-copies of a size-checked source and image previews are re-encoded at
    // ≤1600px, so the preview can never exceed MAX_FILE_BYTES here.
    let size = std::fs::metadata(&prepared.path)?.len();
    let transfer_id = new_transfer_id("download");
    let content_hash = sha256_file(&prepared.path)?;
    let info = DownloadInfo {
        transfer_id: transfer_id.clone(),
        name: prepared.name.clone(),
        mime_type: prepared.mime_type.clone(),
        size,
        content_hash: content_hash.clone(),
        preview_kind: prepared.preview_kind.clone(),
        chunk_bytes: CHUNK_BYTES,
    };
    DOWNLOADS.lock().unwrap().insert(
        transfer_id,
        DownloadRecord {
            path: prepared.path,
            size,
            created_at: SystemTime::now(),
        },
    );
    Ok(info)
}

#[derive(Debug)]
struct PreparedPreview {
    path: PathBuf,
    name: String,
    mime_type: String,
    preview_kind: String,
}

fn is_animated_image(source: &Path) -> Result<bool, crate::AppError> {
    let mut file = File::open(source)?;
    let file_size = file.metadata()?.len();
    let mut header = vec![0_u8; 21.min(file_size as usize)];
    file.read_exact(&mut header)?;
    if header.starts_with(b"GIF87a") || header.starts_with(b"GIF89a") {
        return Ok(true);
    }
    if header.len() >= 21
        && &header[0..4] == b"RIFF"
        && &header[8..12] == b"WEBP"
        && &header[12..16] == b"VP8X"
    {
        return Ok(header[20] & 0x02 != 0);
    }
    if !header.starts_with(&[137, 80, 78, 71, 13, 10, 26, 10]) {
        return Ok(false);
    }
    file.seek(SeekFrom::Start(8))?;
    while file.stream_position()?.saturating_add(8) <= file_size {
        let mut header = [0_u8; 8];
        file.read_exact(&mut header)?;
        let length = u32::from_be_bytes([header[0], header[1], header[2], header[3]]) as u64;
        let kind = &header[4..8];
        if kind == b"acTL" {
            return Ok(true);
        }
        if kind == b"IDAT" || kind == b"IEND" {
            return Ok(false);
        }
        let next = file
            .stream_position()?
            .saturating_add(length)
            .saturating_add(4);
        if next > file_size {
            return Ok(false);
        }
        file.seek(SeekFrom::Start(next))?;
    }
    Ok(false)
}

fn prepare_preview(
    source: &Path,
    requested_display_name: &str,
) -> Result<PreparedPreview, crate::AppError> {
    let ext = source
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    let original_name = display_name(requested_display_name, "attachment");
    let dir = transfer_root().join("download");
    ensure_private_dir(&dir)?;
    let stamp = new_transfer_id("preview");

    if is_animated_image(source)? {
        return Err(
            "Animated image preview is not supported on mobile; view it on desktop."
                .to_string()
                .into(),
        );
    }
    if matches!(ext.as_str(), "jpg" | "jpeg" | "bmp" | "png" | "webp") {
        let mut reader = image::ImageReader::open(source)?
            .with_guessed_format()
            .map_err(|error| format!("Unreadable image: {error}"))?;
        let mut limits = image::Limits::default();
        limits.max_alloc = Some(512 * 1024 * 1024);
        reader.limits(limits);
        let image = reader
            .decode()
            .map_err(|error| format!("Undecodable image: {error}"))?;
        let resized = if image.width().max(image.height()) > MOBILE_PREVIEW_MAX_EDGE {
            image.resize(
                MOBILE_PREVIEW_MAX_EDGE,
                MOBILE_PREVIEW_MAX_EDGE,
                image::imageops::FilterType::Lanczos3,
            )
        } else {
            image
        };
        if matches!(ext.as_str(), "jpg" | "jpeg" | "bmp") {
            let path = dir.join(format!("{stamp}.jpg"));
            let file = File::create(&path)?;
            let encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(file, 65);
            image::ImageEncoder::write_image(
                encoder,
                resized.to_rgb8().as_raw(),
                resized.width(),
                resized.height(),
                image::ExtendedColorType::Rgb8,
            )
            .map_err(|error| format!("JPEG preview encode failed: {error}"))?;
            return Ok(PreparedPreview {
                path,
                name: format!(
                    "{}.jpg",
                    Path::new(&original_name)
                        .file_stem()
                        .and_then(|v| v.to_str())
                        .unwrap_or("image")
                ),
                mime_type: "image/jpeg".to_string(),
                preview_kind: "image".to_string(),
            });
        }
        let path = dir.join(format!("{stamp}.png"));
        resized
            .save_with_format(&path, image::ImageFormat::Png)
            .map_err(|error| format!("PNG preview encode failed: {error}"))?;
        return Ok(PreparedPreview {
            path,
            name: format!(
                "{}.png",
                Path::new(&original_name)
                    .file_stem()
                    .and_then(|v| v.to_str())
                    .unwrap_or("image")
            ),
            mime_type: "image/png".to_string(),
            preview_kind: "image".to_string(),
        });
    }

    let size = std::fs::metadata(source)?.len();
    if size > MAX_FILE_BYTES {
        return Err("The file is larger than 10 MiB; view it on desktop."
            .to_string()
            .into());
    }
    let markdown = matches!(ext.as_str(), "md" | "markdown");
    if !is_plain_utf8_text(source)? {
        return Err(
            "This file type cannot be previewed on mobile; view it on desktop."
                .to_string()
                .into(),
        );
    }
    let path = dir.join(format!(
        "{stamp}_{}",
        safe_disk_name(&original_name, "attachment")
    ));
    std::fs::copy(source, &path)?;
    Ok(PreparedPreview {
        path,
        name: original_name,
        mime_type: if markdown {
            "text/markdown"
        } else {
            "text/plain"
        }
        .to_string(),
        preview_kind: if markdown { "markdown" } else { "text" }.to_string(),
    })
}

fn is_plain_utf8_text(path: &Path) -> Result<bool, crate::AppError> {
    let bytes = std::fs::read(path)?;
    let Ok(text) = std::str::from_utf8(&bytes) else {
        return Ok(false);
    };
    Ok(text
        .chars()
        .all(|c| !c.is_control() || matches!(c, '\n' | '\r' | '\t')))
}

pub fn cancel_download(transfer_id: &str) {
    if let Some(item) = DOWNLOADS.lock().unwrap().remove(transfer_id) {
        let _ = std::fs::remove_file(item.path);
    }
}

pub fn clear_all() {
    for (_, item) in UPLOADS.lock().unwrap().drain() {
        let _ = std::fs::remove_file(item.path);
    }
    for (_, item) in DOWNLOADS.lock().unwrap().drain() {
        let _ = std::fs::remove_file(item.path);
    }
}

/// First resubscribe delay after a failed subscribe / ended stream (doubles up
/// to 30s). Tests shrink it so the self-heal path runs without real waits.
fn resubscribe_backoff() -> Duration {
    #[cfg(test)]
    const BACKOFF: Duration = Duration::from_millis(10);
    #[cfg(not(test))]
    const BACKOFF: Duration = Duration::from_secs(1);
    BACKOFF
}

/// Periodic expiry sweep cadence inside the transfer loop. Tests shrink it so
/// the sweep path runs without a one-minute wait.
fn cleanup_tick() -> Duration {
    #[cfg(test)]
    const TICK: Duration = Duration::from_millis(20);
    #[cfg(not(test))]
    const TICK: Duration = Duration::from_secs(60);
    TICK
}

pub fn spawn_transfer_loop(
    client: async_nats::Client,
    pair_id: String,
    active: std::sync::Arc<std::sync::atomic::AtomicBool>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let subject = format!("p.{pair_id}.xfer.up.>");
        let queue = format!("bridge-transfer.{pair_id}");
        // Self-heal like command_loop: a dead transfer loop times out every
        // chunk pull until the next generation swap.
        let mut backoff = resubscribe_backoff();
        loop {
            let mut sub = match client.queue_subscribe(subject.clone(), queue.clone()).await {
                Ok(sub) => sub,
                Err(error) => {
                    eprintln!("remote: failed to subscribe to transfers {subject}: {error}; retrying in {backoff:?}");
                    tokio::time::sleep(backoff).await;
                    backoff = backoff.saturating_mul(2).min(Duration::from_secs(30));
                    continue;
                }
            };
            backoff = resubscribe_backoff();
            let mut cleanup = tokio::time::interval(cleanup_tick());
            loop {
                tokio::select! {
                    _ = cleanup.tick() => prune_expired(),
                    next = sub.next() => {
                        let Some(message) = next else { break };
                        if !active.load(std::sync::atomic::Ordering::Acquire) {
                            continue;
                        }
                        let suffix = message
                            .subject
                            .strip_prefix(&format!("p.{pair_id}.xfer.up."))
                            .unwrap_or_default();
                        let parts: Vec<&str> = suffix.split('.').collect();
                        let response = match parts.as_slice() {
                            [transfer_id, "chunk", index] => index
                                .parse::<u64>()
                                .map_err(|_| "Invalid chunk index.".to_string())
                                .and_then(|index| write_upload_chunk(transfer_id, index, &message.payload)),
                            [transfer_id, "pull", index] => index
                                .parse::<u64>()
                                .map_err(|_| "Invalid chunk index.".to_string())
                                .and_then(|index| {
                                    publish_download_chunk(&client, &pair_id, transfer_id, index)
                                        .map(|_| json!({ "published": true, "index": index }))
                                        .map_err(|error| error.to_string())
                                }),
                            _ => Err("Unsupported transfer operation.".to_string()),
                        };
                        if let Some(reply) = message.reply {
                            let body = match response {
                                Ok(data) => json!({ "success": true, "data": data }),
                                Err(error) => json!({ "success": false, "error": error }),
                            };
                            // A Value always serializes.
                            let bytes = serde_json::to_vec(&body)
                                .expect("a transfer reply Value always serializes");
                            let _ = client.publish(reply, bytes.into()).await;
                        }
                    }
                }
            }
            eprintln!(
                "remote: transfer subscription ended unexpectedly; resubscribing in {backoff:?}"
            );
            tokio::time::sleep(backoff).await;
            backoff = backoff.saturating_mul(2).min(Duration::from_secs(30));
        }
    })
}

pub(crate) fn write_upload_chunk(
    transfer_id: &str,
    index: u64,
    payload: &[u8],
) -> Result<Value, String> {
    if payload.len() as u64 > CHUNK_BYTES {
        return Err("Chunk exceeds the 512 KiB limit.".to_string());
    }
    let mut uploads = UPLOADS.lock().unwrap();
    let item = uploads
        .get_mut(transfer_id)
        .ok_or_else(|| "Upload expired or does not exist.".to_string())?;
    if item.complete {
        return Err("Upload is already complete.".to_string());
    }
    if index < item.next_chunk {
        return Ok(json!({ "index": index, "received": item.received, "duplicate": true }));
    }
    if index != item.next_chunk {
        return Err(format!(
            "Expected chunk {}, received {index}.",
            item.next_chunk
        ));
    }
    if item.received.saturating_add(payload.len() as u64) > item.transfer_size {
        return Err("Chunk exceeds the declared transfer size.".to_string());
    }
    OpenOptions::new()
        .append(true)
        .open(&item.path)
        .and_then(|mut file| file.write_all(payload))
        .map_err(|error| error.to_string())?;
    item.received += payload.len() as u64;
    item.next_chunk += 1;
    Ok(json!({ "index": index, "received": item.received }))
}

fn publish_download_chunk(
    client: &async_nats::Client,
    pair_id: &str,
    transfer_id: &str,
    index: u64,
) -> Result<(), crate::AppError> {
    let item = DOWNLOADS
        .lock()
        .unwrap()
        .get(transfer_id)
        .cloned()
        .ok_or_else(|| {
            crate::AppError::Message("Download expired or does not exist.".to_string())
        })?;
    let offset = index.saturating_mul(CHUNK_BYTES);
    if offset >= item.size {
        return Err("Chunk index is outside the file.".to_string().into());
    }
    let length = CHUNK_BYTES.min(item.size - offset) as usize;
    let mut file = File::open(item.path)?;
    file.seek(SeekFrom::Start(offset))?;
    let mut bytes = vec![0_u8; length];
    file.read_exact(&mut bytes)?;
    let subject = format!("p.{pair_id}.xfer.down.{transfer_id}.chunk.{index}");
    let client = client.clone();
    tokio::spawn(async move {
        let _ = client.publish(subject, bytes.into()).await;
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        attachment_is_in_session, display_name, ensure_private_dir, is_animated_image,
        prepare_preview, safe_disk_name, validate_mobile_image, MAX_FILE_BYTES,
    };
    use serde_json::json;

    #[test]
    fn sanitizes_upload_names() {
        assert_eq!(safe_disk_name("../../hello?.md", "x"), "hello.md");
    }

    #[test]
    fn name_fallbacks_cover_names_that_clean_to_nothing() {
        // Dots/control characters only → the display name falls back…
        assert_eq!(display_name("...", "x"), "x");
        assert_eq!(display_name("a/\u{7}", "x"), "x");
        // …and a name that survives display but loses every ASCII-safe
        // character falls back at the disk-name layer.
        assert_eq!(safe_disk_name("日本語", "x"), "x");
    }

    #[test]
    fn preserves_unicode_display_names_without_path_prefixes() {
        assert_eq!(display_name("../../报告 终稿.md", "x"), "报告 终稿.md");
        assert_eq!(display_name(r"C:\\temp\\照片📷.jpg", "x"), "照片📷.jpg");
    }

    #[test]
    fn finds_only_session_attachment_paths() {
        let entries = json!({"entries":[{"meta":{"attachments":[{"path":"/tmp/a.md"}]}}]});
        assert!(attachment_is_in_session(&entries, "/tmp/a.md"));
        assert!(!attachment_is_in_session(&entries, "/tmp/secret"));
    }

    #[test]
    fn limits_match_remote_contract() {
        assert_eq!(MAX_FILE_BYTES, 10 * 1024 * 1024);
    }

    #[cfg(unix)]
    #[test]
    fn staging_dir_is_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!(
            "futureos-transfer-mode-test-{}",
            nkeys::KeyPair::new_user().public_key()
        ));
        ensure_private_dir(&dir).unwrap();
        let mode = std::fs::metadata(&dir).unwrap().permissions().mode();
        // Group/other bits must be cleared; owner rwx may be stricter.
        assert_eq!(mode & 0o077, 0, "group/other access not cleared: {mode:o}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn accepts_mobile_images_over_2000_pixels() {
        let dir = std::env::temp_dir().join(format!(
            "futureos-transfer-test-{}",
            nkeys::KeyPair::new_user().public_key()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        // Oversized images are no longer rejected: current clients downsample
        // to 1600 px before upload, and the agent resizes anything above its
        // own cap before model submission.
        let oversized = dir.join("oversized.png");
        image::DynamicImage::new_rgb8(2001, 1)
            .save(&oversized)
            .unwrap();
        assert!(validate_mobile_image(&oversized).is_ok());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn detects_apng_and_animated_webp_headers() {
        let dir = std::env::temp_dir().join(format!(
            "futureos-animation-test-{}",
            nkeys::KeyPair::new_user().public_key()
        ));
        std::fs::create_dir_all(&dir).unwrap();

        let apng = dir.join("animated.png");
        let mut apng_bytes = vec![137, 80, 78, 71, 13, 10, 26, 10];
        apng_bytes.extend_from_slice(&[0, 0, 0, 0]);
        apng_bytes.extend_from_slice(b"acTL");
        apng_bytes.extend_from_slice(&[0, 0, 0, 0]);
        std::fs::write(&apng, apng_bytes).unwrap();
        assert!(is_animated_image(&apng).unwrap());

        let webp = dir.join("animated.webp");
        let mut webp_bytes = [0_u8; 21];
        webp_bytes[0..4].copy_from_slice(b"RIFF");
        webp_bytes[8..12].copy_from_slice(b"WEBP");
        webp_bytes[12..16].copy_from_slice(b"VP8X");
        webp_bytes[20] = 0x02;
        std::fs::write(&webp, webp_bytes).unwrap();
        assert!(is_animated_image(&webp).unwrap());

        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn prepares_only_plain_text_and_markdown_as_non_image_previews() {
        let dir = std::env::temp_dir().join(format!(
            "futureos-preview-test-{}",
            nkeys::KeyPair::new_user().public_key()
        ));
        std::fs::create_dir_all(&dir).unwrap();

        let text = dir.join("notes.txt");
        std::fs::write(&text, "hello\nworld").unwrap();
        let text_preview = prepare_preview(&text, "notes.txt").unwrap();
        assert_eq!(text_preview.preview_kind, "text");
        assert_eq!(text_preview.mime_type, "text/plain");
        std::fs::remove_file(text_preview.path).unwrap();

        let markdown = dir.join("notes.md");
        std::fs::write(&markdown, "# Hello").unwrap();
        let markdown_preview = prepare_preview(&markdown, "notes.md").unwrap();
        assert_eq!(markdown_preview.preview_kind, "markdown");
        assert_eq!(markdown_preview.mime_type, "text/markdown");
        std::fs::remove_file(markdown_preview.path).unwrap();

        let pdf = dir.join("document.pdf");
        std::fs::write(&pdf, b"%PDF-1.7\0binary").unwrap();
        let error =
            prepare_preview(&pdf, "document.pdf").expect_err("PDF should not get a mobile preview");
        assert!(error.to_string().contains("view it on desktop"));
        std::fs::remove_dir_all(dir).unwrap();
    }
}

#[cfg(test)]
mod flow_tests {
    #![allow(clippy::await_holding_lock)]
    use super::super::test_support::{
        assert_no_publish, await_publish, ensure_mock_agent, mock_agent_lock, nats_connect_once,
        unique, FakeNats, HomeGuard,
    };
    use super::*;
    use serde_json::json;
    use std::sync::{atomic::Ordering, Arc};
    use std::time::Duration;

    fn tiny_png() -> Vec<u8> {
        let dir = std::env::temp_dir().join(unique("futureos-png-src"));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("tiny.png");
        image::DynamicImage::new_rgb8(1, 1).save(&path).unwrap();
        let bytes = std::fs::read(&path).unwrap();
        std::fs::remove_dir_all(dir).unwrap();
        bytes
    }

    fn init_file_upload(name: &str, size: u64) -> String {
        init_upload(name, "", "application/octet-stream", "file", size, size)
            .unwrap()
            .upload_id
    }

    #[test]
    fn upload_lifecycle_end_to_end() {
        let _home = HomeGuard::new("xfer-lifecycle");
        let upload_id = init_file_upload("dir/hello.txt", 5);
        assert!(upload_id.starts_with("upload_"));

        // Out-of-order and oversized chunks are rejected; duplicates are idempotent.
        let gap = write_upload_chunk(&upload_id, 1, b"!").unwrap_err();
        assert!(gap.contains("Expected chunk 0"));
        let oversized = vec![0_u8; (CHUNK_BYTES + 1) as usize];
        assert!(write_upload_chunk(&upload_id, 0, &oversized)
            .unwrap_err()
            .contains("512 KiB"));
        let first = write_upload_chunk(&upload_id, 0, b"hel").unwrap();
        assert_eq!(first["received"], json!(3));
        let duplicate = write_upload_chunk(&upload_id, 0, b"hel").unwrap();
        assert_eq!(duplicate["duplicate"], json!(true));
        write_upload_chunk(&upload_id, 1, b"lo").unwrap();
        let overflow = write_upload_chunk(&upload_id, 2, b"!!").unwrap_err();
        assert!(overflow.contains("declared transfer size"));

        let complete = complete_upload(&upload_id).unwrap();
        assert_eq!(complete.upload_id, upload_id);
        assert_eq!(complete.content_hash.len(), 64);
        // Completing twice fails the size check (the upload is done receiving).
        let again = write_upload_chunk(&upload_id, 2, b"!!").unwrap_err();
        assert!(again.contains("already complete"));

        // Claim moves the bytes into the thread's attachment dir.
        let references = vec![UploadReference {
            upload_id: upload_id.clone(),
        }];
        let claimed = claim_uploads(&references, "thread-life").unwrap();
        assert_eq!(claimed.len(), 1);
        assert_eq!(claimed[0].kind, "file");
        assert_eq!(claimed[0].name, "hello.txt");
        assert_eq!(std::fs::read(&claimed[0].path).unwrap(), b"hello");
        assert!(claimed[0].thumbnail.is_none());
        // The upload record and staging file are gone after the claim.
        assert!(complete_upload(&upload_id).is_err());
        assert!(UPLOADS.lock().unwrap().get(&upload_id).is_none());

        rollback_claimed(&claimed);
        assert!(!Path::new(&claimed[0].path).exists());
    }

    #[test]
    fn init_upload_prefers_a_non_empty_transfer_name_for_the_disk_name() {
        let _home = HomeGuard::new("xfer-transfer-name");
        let result =
            init_upload("报告.txt", "My Transfer?.txt", "text/plain", "file", 4, 4).unwrap();
        let uploads = UPLOADS.lock().unwrap();
        let record = uploads.get(&result.upload_id).expect("upload record");
        assert_eq!(record.name, "报告.txt");
        assert_eq!(record.transfer_name, "My Transfer.txt");
        drop(uploads);
        cancel_upload(&result.upload_id).unwrap();
    }

    #[test]
    fn init_upload_validates_sizes_and_kind() {
        let _home = HomeGuard::new("xfer-init");
        assert!(init_upload("a", "", "m", "file", 0, 1).is_err());
        assert!(init_upload("a", "", "m", "file", MAX_FILE_BYTES + 1, 1).is_err());
        assert!(init_upload("a", "", "m", "file", 1, 0).is_err());
        assert!(init_upload("a", "", "m", "file", 1, MAX_FILE_BYTES + 1).is_err());
        assert!(init_upload("a", "", "m", "video", 1, 1).is_err());
    }

    #[test]
    fn upload_chunk_and_complete_error_paths() {
        let _home = HomeGuard::new("xfer-errors");
        assert!(write_upload_chunk("missing", 0, b"x")
            .unwrap_err()
            .contains("expired or does not exist"));
        assert!(complete_upload("missing").is_err());
        // cancel on a missing id is a no-op success.
        cancel_upload("missing").unwrap();

        let upload_id = init_file_upload("partial.bin", 10);
        // Not fully received → completion refused.
        write_upload_chunk(&upload_id, 0, b"abc").unwrap();
        let incomplete = complete_upload(&upload_id).unwrap_err();
        assert!(incomplete.to_string().contains("incomplete"));

        // A staging file that vanished mid-upload surfaces as a chunk error.
        let path = UPLOADS
            .lock()
            .unwrap()
            .get(&upload_id)
            .unwrap()
            .path
            .clone();
        std::fs::remove_file(&path).unwrap();
        assert!(write_upload_chunk(&upload_id, 1, b"def").is_err());

        cancel_upload(&upload_id).unwrap();
        assert!(UPLOADS.lock().unwrap().get(&upload_id).is_none());
    }

    #[test]
    fn complete_upload_fails_when_staging_file_is_gone() {
        let _home = HomeGuard::new("xfer-hash-missing");
        let upload_id = init_file_upload("gone.bin", 3);
        write_upload_chunk(&upload_id, 0, b"abc").unwrap();
        let path = UPLOADS
            .lock()
            .unwrap()
            .get(&upload_id)
            .unwrap()
            .path
            .clone();
        std::fs::remove_file(&path).unwrap();
        assert!(complete_upload(&upload_id).is_err());
    }

    #[test]
    fn claim_uploads_enforces_message_limits() {
        let _home = HomeGuard::new("xfer-limits");
        let make = |size: u64| {
            let id = init_upload("f.bin", "", "m", "file", size, 1)
                .unwrap()
                .upload_id;
            write_upload_chunk(&id, 0, b"x").unwrap();
            complete_upload(&id).unwrap();
            UploadReference { upload_id: id }
        };

        // More than MAX_ATTACHMENTS references.
        let too_many: Vec<UploadReference> = (0..=MAX_ATTACHMENTS)
            .map(|_| UploadReference {
                upload_id: unique("ref"),
            })
            .collect();
        assert!(claim_uploads(&too_many, "thread-x").is_err());

        // Duplicate reference.
        let one = make(1);
        assert!(claim_uploads(&[one.clone(), one], "thread-x").is_err());

        // Unknown reference.
        assert!(claim_uploads(
            &[UploadReference {
                upload_id: "nope".to_string()
            }],
            "thread-x"
        )
        .is_err());

        // Still uploading (not complete).
        let pending = init_file_upload("pending.bin", 10);
        assert!(claim_uploads(
            &[UploadReference {
                upload_id: pending.clone()
            }],
            "thread-x"
        )
        .is_err());
        cancel_upload(&pending).unwrap();

        // Combined size over the 20 MiB message limit.
        let big = vec![
            make(9 * 1024 * 1024),
            make(9 * 1024 * 1024),
            make(9 * 1024 * 1024),
        ];
        let error = claim_uploads(&big, "thread-x").unwrap_err();
        assert!(error.to_string().contains("20 MiB"));
    }

    #[test]
    fn claim_uploads_enforces_image_rules() {
        let _home = HomeGuard::new("xfer-images");
        let png = tiny_png();
        let make_image = || {
            let id = init_upload(
                "pic.png",
                "",
                "image/png",
                "image",
                png.len() as u64,
                png.len() as u64,
            )
            .unwrap()
            .upload_id;
            write_upload_chunk(&id, 0, &png).unwrap();
            complete_upload(&id).unwrap();
            UploadReference { upload_id: id }
        };

        // Five images exceed the four-image cap.
        let five: Vec<UploadReference> = (0..5).map(|_| make_image()).collect();
        let error = claim_uploads(&five, "thread-img").unwrap_err();
        assert!(error.to_string().contains("at most 4 images"));

        // A non-image payload claiming to be an image fails validation.
        let bogus = init_upload("pic.png", "", "image/png", "image", 4, 4)
            .unwrap()
            .upload_id;
        write_upload_chunk(&bogus, 0, b"nope").unwrap();
        complete_upload(&bogus).unwrap();
        let error = claim_uploads(
            &[UploadReference {
                upload_id: bogus.clone(),
            }],
            "thread-img",
        )
        .unwrap_err();
        assert!(
            error.to_string().contains("Unreadable image")
                || error.to_string().contains("Undecodable")
        );
        cancel_upload(&bogus).ok();

        // Four valid images claim fine and get thumbnails.
        let four: Vec<UploadReference> = (0..4).map(|_| make_image()).collect();
        let claimed = claim_uploads(&four, "thread-img").unwrap();
        assert_eq!(claimed.len(), 4);
        assert!(claimed.iter().all(|a| a.kind == "image"));
        rollback_claimed(&claimed);
    }

    #[test]
    fn claim_uploads_rolls_back_copies_on_copy_failure() {
        let _home = HomeGuard::new("xfer-rollback");
        let first = init_file_upload("one.txt", 1);
        write_upload_chunk(&first, 0, b"1").unwrap();
        complete_upload(&first).unwrap();
        let second = init_file_upload("two.txt", 1);
        write_upload_chunk(&second, 0, b"2").unwrap();
        complete_upload(&second).unwrap();

        // Pre-create a DIRECTORY where the second attachment's file must land,
        // so its copy fails and the already-copied first file is rolled back.
        let origin = crate::store::thread_images_dir("thread-rb")
            .unwrap()
            .join("origin");
        std::fs::create_dir_all(&origin).unwrap();
        let blocker = origin.join(format!("{second}_two.txt"));
        std::fs::create_dir_all(&blocker).unwrap();

        let references = vec![
            UploadReference {
                upload_id: first.clone(),
            },
            UploadReference {
                upload_id: second.clone(),
            },
        ];
        assert!(claim_uploads(&references, "thread-rb").is_err());
        // The first copy was removed again; nothing leaks into the thread dir.
        assert!(!origin.join(format!("{first}_one.txt")).exists());
        std::fs::remove_dir_all(&blocker).unwrap();
    }

    #[test]
    fn prune_expired_removes_stale_records_and_files() {
        let _home = HomeGuard::new("xfer-prune");
        let upload_id = init_file_upload("stale.bin", 1);
        let upload_path = UPLOADS
            .lock()
            .unwrap()
            .get(&upload_id)
            .unwrap()
            .path
            .clone();
        let download_path = transfer_root().join("download").join("stale.txt");
        std::fs::create_dir_all(download_path.parent().unwrap()).unwrap();
        std::fs::write(&download_path, b"x").unwrap();
        DOWNLOADS.lock().unwrap().insert(
            "download_stale".to_string(),
            DownloadRecord {
                path: download_path.clone(),
                size: 1,
                created_at: SystemTime::now(),
            },
        );
        // Age both records past the TTL.
        UPLOADS
            .lock()
            .unwrap()
            .get_mut(&upload_id)
            .unwrap()
            .created_at = SystemTime::now() - (TRANSFER_TTL + Duration::from_secs(60));
        DOWNLOADS
            .lock()
            .unwrap()
            .get_mut("download_stale")
            .unwrap()
            .created_at = SystemTime::now() - (TRANSFER_TTL + Duration::from_secs(60));

        prune_expired();
        assert!(UPLOADS.lock().unwrap().get(&upload_id).is_none());
        assert!(DOWNLOADS.lock().unwrap().get("download_stale").is_none());
        assert!(!upload_path.exists());
        assert!(!download_path.exists());

        // A fresh record survives the sweep.
        let fresh = init_file_upload("fresh.bin", 1);
        prune_expired();
        assert!(UPLOADS.lock().unwrap().get(&fresh).is_some());
        cancel_upload(&fresh).unwrap();
    }

    #[test]
    fn clear_all_drains_both_maps() {
        let _home = HomeGuard::new("xfer-clear");
        let upload_id = init_file_upload("a.bin", 1);
        let upload_path = UPLOADS
            .lock()
            .unwrap()
            .get(&upload_id)
            .unwrap()
            .path
            .clone();
        let download_path = transfer_root().join("download").join("b.txt");
        std::fs::create_dir_all(download_path.parent().unwrap()).unwrap();
        std::fs::write(&download_path, b"x").unwrap();
        DOWNLOADS.lock().unwrap().insert(
            "download_b".to_string(),
            DownloadRecord {
                path: download_path.clone(),
                size: 1,
                created_at: SystemTime::now(),
            },
        );
        clear_all();
        assert!(UPLOADS.lock().unwrap().is_empty());
        assert!(DOWNLOADS.lock().unwrap().is_empty());
        assert!(!upload_path.exists());
        assert!(!download_path.exists());
        // Idempotent on empty maps.
        clear_all();
    }

    #[test]
    fn session_attachment_name_falls_back_to_the_path_file_name() {
        let entries = json!({"entries":[{"meta":{"attachments":[{"path":"/tmp/no-name.md"}]}}]});
        assert_eq!(
            session_attachment_name(&entries, "/tmp/no-name.md"),
            Some("no-name.md".to_string())
        );
        let named =
            json!({"entries":[{"meta":{"attachments":[{"path":"/tmp/x","name":"Pretty.txt"}]}}]});
        assert_eq!(
            session_attachment_name(&named, "/tmp/x"),
            Some("Pretty.txt".to_string())
        );
        assert_eq!(session_attachment_name(&json!({}), "/tmp/x"), None);
    }

    #[tokio::test]
    async fn prepare_download_flows() {
        let _lock = mock_agent_lock();
        let _home = HomeGuard::new("xfer-download");
        let agent = ensure_mock_agent();
        let session = unique("sess");

        // Not an attachment of the session.
        agent.set_session_entries(&session, json!({ "entries": [] }));
        let error = prepare_download(&session, "/tmp/whatever.txt")
            .await
            .unwrap_err();
        assert!(error.to_string().contains("not an attachment"));

        // Attachment row exists but the file is gone.
        let dir = std::env::temp_dir().join(unique("futureos-dl"));
        std::fs::create_dir_all(&dir).unwrap();
        let gone = dir.join("gone.txt");
        agent.set_session_entries(
            &session,
            json!({"entries":[{"meta":{"attachments":[{"path": gone.to_string_lossy(),"name":"gone.txt"}]}}]}),
        );
        assert!(prepare_download(&session, &gone.to_string_lossy())
            .await
            .is_err());

        // An attachment path that still exists but is a directory, not a file.
        agent.set_session_entries(
            &session,
            json!({"entries":[{"meta":{"attachments":[{"path": dir.to_string_lossy(),"name":"adir"}]}}]}),
        );
        let error = prepare_download(&session, &dir.to_string_lossy())
            .await
            .unwrap_err();
        assert!(error.to_string().contains("no longer available"));

        // A real text file gets a preview + a download record.
        let notes = dir.join("notes.txt");
        std::fs::write(&notes, b"hello download").unwrap();
        agent.set_session_entries(
            &session,
            json!({"entries":[{"meta":{"attachments":[{"path": notes.to_string_lossy(),"name":"notes.txt"}]}}]}),
        );
        let info = prepare_download(&session, &notes.to_string_lossy())
            .await
            .unwrap();
        assert_eq!(info.name, "notes.txt");
        assert_eq!(info.mime_type, "text/plain");
        assert_eq!(info.preview_kind, "text");
        assert_eq!(info.size, 14);
        assert_eq!(info.chunk_bytes, CHUNK_BYTES);
        assert_eq!(info.content_hash.len(), 64);
        assert!(DOWNLOADS.lock().unwrap().contains_key(&info.transfer_id));
        cancel_download(&info.transfer_id);
        assert!(!DOWNLOADS.lock().unwrap().contains_key(&info.transfer_id));
        cancel_download("never-existed");

        std::fs::remove_dir_all(dir).unwrap();
    }

    #[tokio::test]
    async fn prepare_download_resolves_workspace_relative_links() {
        // Schema-applied fake HOME: the pooled connection is dropped back so
        // the store calls below observe the tables.
        let _home = crate::store::test_schema_home("xfer-link-resolve");
        let agent = ensure_mock_agent();
        let session = unique("sess");
        agent.set_session_entries(&session, json!({ "entries": [] }));

        // A chat thread bound to the session; its chat workspace is the
        // working directory relative links resolve against.
        let thread = crate::store::create_thread(crate::store::CreateThreadInput {
            mode: "chat".to_string(),
            title: None,
            workspace_id: None,
            workspace_path: None,
            workspace_name: None,
            agent_session_id: Some(session.clone()),
        })
        .expect("create thread");
        let cwd = crate::agent_bridge::workspace_path_for_thread(&thread.id).expect("cwd");
        std::fs::create_dir_all(&cwd).unwrap();
        std::fs::write(Path::new(&cwd).join("poem.txt"), b"a quiet poem").unwrap();

        let info = prepare_download(&session, "poem.txt")
            .await
            .expect("resolve link");
        assert_eq!(info.name, "poem.txt");
        assert_eq!(info.mime_type, "text/plain");
        assert_eq!(info.preview_kind, "text");
        assert_eq!(info.size, 12);

        // A relative link with no matching file in the workspace still fails.
        assert!(prepare_download(&session, "missing.txt").await.is_err());
    }

    #[test]
    fn animated_image_detection_edge_cases() {
        let dir = std::env::temp_dir().join(unique("futureos-anim"));
        std::fs::create_dir_all(&dir).unwrap();

        // GIF magic (both versions) is animated.
        let gif = dir.join("a.gif");
        std::fs::write(&gif, b"GIF89a...").unwrap();
        assert!(is_animated_image(&gif).unwrap());

        // Static WebP (VP8X without the animation flag) is not.
        let webp = dir.join("static.webp");
        let mut bytes = vec![0_u8; 21];
        bytes[0..4].copy_from_slice(b"RIFF");
        bytes[8..12].copy_from_slice(b"WEBP");
        bytes[12..16].copy_from_slice(b"VP8X");
        std::fs::write(&webp, &bytes).unwrap();
        assert!(!is_animated_image(&webp).unwrap());

        // A short, non-image file is not animated.
        let tiny = dir.join("tiny.bin");
        std::fs::write(&tiny, b"12345").unwrap();
        assert!(!is_animated_image(&tiny).unwrap());

        // PNG whose first chunk is IDAT (no acTL) is static.
        let png = dir.join("static.png");
        let mut png_bytes = vec![137, 80, 78, 71, 13, 10, 26, 10];
        png_bytes.extend_from_slice(&[0, 0, 0, 0]);
        png_bytes.extend_from_slice(b"IDAT");
        png_bytes.extend_from_slice(&[0, 0, 0, 0]);
        std::fs::write(&png, &png_bytes).unwrap();
        assert!(!is_animated_image(&png).unwrap());

        // PNG with a chunk length running past EOF is treated as static.
        let truncated = dir.join("truncated.png");
        let mut truncated_bytes = vec![137, 80, 78, 71, 13, 10, 26, 10];
        truncated_bytes.extend_from_slice(&[0x7F, 0xFF, 0xFF, 0xFF]);
        truncated_bytes.extend_from_slice(b"tEXt");
        truncated_bytes.extend_from_slice(&[0, 0]);
        std::fs::write(&truncated, &truncated_bytes).unwrap();
        assert!(!is_animated_image(&truncated).unwrap());

        // PNG whose chunks are walked to the end with no acTL/IDAT/IEND.
        let exhausted = dir.join("exhausted.png");
        let mut exhausted_bytes = vec![137, 80, 78, 71, 13, 10, 26, 10];
        exhausted_bytes.extend_from_slice(&[0, 0, 0, 0]);
        exhausted_bytes.extend_from_slice(b"tEXt");
        exhausted_bytes.extend_from_slice(&[0, 0, 0, 0]);
        std::fs::write(&exhausted, &exhausted_bytes).unwrap();
        assert!(!is_animated_image(&exhausted).unwrap());

        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn prepare_preview_image_variants() {
        let dir = std::env::temp_dir().join(unique("futureos-imgprev"));
        std::fs::create_dir_all(&dir).unwrap();

        // JPEG input → quality-reduced JPEG preview.
        let jpg = dir.join("photo.jpg");
        image::DynamicImage::new_rgb8(100, 50).save(&jpg).unwrap();
        let preview = prepare_preview(&jpg, "My Photo.jpg").unwrap();
        assert_eq!(preview.mime_type, "image/jpeg");
        assert_eq!(preview.name, "My Photo.jpg");
        assert_eq!(preview.preview_kind, "image");
        std::fs::remove_file(preview.path).unwrap();

        // Large PNG → resized PNG preview.
        let png = dir.join("big.png");
        image::DynamicImage::new_rgb8(3200, 800).save(&png).unwrap();
        let preview = prepare_preview(&png, "big.png").unwrap();
        assert_eq!(preview.mime_type, "image/png");
        let decoded = image::ImageReader::open(&preview.path)
            .unwrap()
            .with_guessed_format()
            .unwrap()
            .decode()
            .unwrap();
        assert_eq!(decoded.width(), MOBILE_PREVIEW_MAX_EDGE);
        assert_eq!(decoded.height(), 400);
        std::fs::remove_file(preview.path).unwrap();

        // Corrupt bytes under an image extension → undecodable.
        let corrupt = dir.join("broken.png");
        std::fs::write(&corrupt, b"not really a png").unwrap();
        let error = prepare_preview(&corrupt, "broken.png").unwrap_err();
        assert!(
            error.to_string().contains("Unreadable") || error.to_string().contains("Undecodable")
        );

        // Animated GIF → refused.
        let gif = dir.join("dance.gif");
        std::fs::write(&gif, b"GIF89a animated").unwrap();
        let error = prepare_preview(&gif, "dance.gif").unwrap_err();
        assert!(error.to_string().contains("Animated"));

        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn prepare_preview_rejects_oversized_and_binary_files() {
        let dir = std::env::temp_dir().join(unique("futureos-bigprev"));
        std::fs::create_dir_all(&dir).unwrap();

        let big = dir.join("huge.txt");
        std::fs::write(&big, vec![b'x'; (MAX_FILE_BYTES + 1) as usize]).unwrap();
        let error = prepare_preview(&big, "huge.txt").unwrap_err();
        assert!(error.to_string().contains("10 MiB"));

        // Invalid UTF-8 is not previewable as text.
        let binary = dir.join("data.bin");
        std::fs::write(&binary, [0xFF, 0xFE, 0x00, 0x01]).unwrap();
        assert!(!is_plain_utf8_text(&binary).unwrap());
        let error = prepare_preview(&binary, "data.bin").unwrap_err();
        assert!(error.to_string().contains("cannot be previewed"));

        // Control characters other than whitespace are not plain text.
        let control = dir.join("ctrl.txt");
        std::fs::write(&control, b"hello\x07world").unwrap();
        assert!(!is_plain_utf8_text(&control).unwrap());
        assert!(is_plain_utf8_text(&dir.join("..").join("does-not-exist")).is_err());

        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn validate_mobile_image_decode_failures() {
        let dir = std::env::temp_dir().join(unique("futureos-mobileimg"));
        std::fs::create_dir_all(&dir).unwrap();

        // Missing file → open error.
        assert!(validate_mobile_image(&dir.join("nope.png")).is_err());

        // Garbage → unreadable format.
        let garbage = dir.join("garbage.png");
        std::fs::write(&garbage, b"garbage bytes").unwrap();
        assert!(validate_mobile_image(&garbage).is_err());

        // A PNG header whose body is truncated passes dimension sniffing but
        // fails the full decode.
        let mut truncated = image::DynamicImage::new_rgb8(10, 10)
            .save(dir.join("full.png"))
            .map(|_| std::fs::read(dir.join("full.png")).unwrap())
            .unwrap();
        truncated.truncate(40);
        let truncated_path = dir.join("truncated.png");
        std::fs::write(&truncated_path, &truncated).unwrap();
        assert!(validate_mobile_image(&truncated_path).is_err());

        std::fs::remove_dir_all(dir).unwrap();
    }

    #[tokio::test]
    async fn transfer_loop_handles_chunks_pulls_and_reconnects() {
        let _home = HomeGuard::new("xfer-loop");
        let nats = FakeNats::start().await;
        let client = nats_connect_once(&nats).await;
        let pair = unique("pair");
        let active = Arc::new(std::sync::atomic::AtomicBool::new(true));
        let loop_handle = spawn_transfer_loop(client.clone(), pair.clone(), active.clone());
        nats.wait_for_sub(&format!("p.{pair}.xfer.up.>"), Duration::from_secs(5))
            .await;

        // While the handshake gate is closed, transfer messages are skipped
        // (no reply is published).
        active.store(false, Ordering::Release);
        let skipped_reply = format!("rep-{}", unique("skip"));
        let mut tap = nats.tap();
        nats.inject(
            &format!("p.{pair}.xfer.up.skip.chunk.0"),
            Some(&skipped_reply),
            b"xx".to_vec(),
        );
        assert_no_publish(&mut tap, &skipped_reply, Duration::from_millis(200)).await;
        active.store(true, Ordering::Release);

        // Upload chunks flow through the loop into the staging file.
        let upload_id = init_file_upload("loop.bin", 4);
        let chunk_reply = format!("rep-{}", unique("chunk"));
        nats.inject(
            &format!("p.{pair}.xfer.up.{upload_id}.chunk.0"),
            Some(&chunk_reply),
            b"loop".to_vec(),
        );
        let reply = await_publish(&mut tap, &chunk_reply, Duration::from_secs(5)).await;
        assert_eq!(reply.json()["success"], json!(true));
        assert_eq!(reply.json()["data"]["received"], json!(4));

        // Malformed chunk indexes and unknown operations get error replies.
        let bad_index_reply = format!("rep-{}", unique("badidx"));
        nats.inject(
            &format!("p.{pair}.xfer.up.{upload_id}.chunk.abc"),
            Some(&bad_index_reply),
            b"x".to_vec(),
        );
        let reply = await_publish(&mut tap, &bad_index_reply, Duration::from_secs(5)).await;
        assert_eq!(reply.json()["success"], json!(false));
        assert!(reply.json()["error"]
            .as_str()
            .unwrap()
            .contains("Invalid chunk index"));

        let bogus_reply = format!("rep-{}", unique("bogus"));
        nats.inject(
            &format!("p.{pair}.xfer.up.{upload_id}.bogus.0"),
            Some(&bogus_reply),
            b"x".to_vec(),
        );
        let reply = await_publish(&mut tap, &bogus_reply, Duration::from_secs(5)).await;
        assert!(reply.json()["error"]
            .as_str()
            .unwrap()
            .contains("Unsupported transfer operation"));

        // Download pulls publish file bytes on the down subject.
        let download_path = transfer_root().join("download").join("pull.txt");
        std::fs::create_dir_all(download_path.parent().unwrap()).unwrap();
        std::fs::write(&download_path, b"download-bytes").unwrap();
        DOWNLOADS.lock().unwrap().insert(
            "download_loop".to_string(),
            DownloadRecord {
                path: download_path.clone(),
                size: 14,
                created_at: SystemTime::now(),
            },
        );
        let pull_reply = format!("rep-{}", unique("pull"));
        nats.inject(
            &format!("p.{pair}.xfer.up.download_loop.pull.0"),
            Some(&pull_reply),
            Vec::new(),
        );
        let reply = await_publish(&mut tap, &pull_reply, Duration::from_secs(5)).await;
        assert_eq!(reply.json()["success"], json!(true));
        assert_eq!(reply.json()["data"]["published"], json!(true));
        let chunk = await_publish(
            &mut tap,
            &format!("p.{pair}.xfer.down.download_loop.chunk.0"),
            Duration::from_secs(5),
        )
        .await;
        assert_eq!(chunk.payload, b"download-bytes");

        // Out-of-range and unknown pulls get error replies.
        let range_reply = format!("rep-{}", unique("range"));
        nats.inject(
            &format!("p.{pair}.xfer.up.download_loop.pull.9"),
            Some(&range_reply),
            Vec::new(),
        );
        let reply = await_publish(&mut tap, &range_reply, Duration::from_secs(5)).await;
        assert!(reply.json()["error"]
            .as_str()
            .unwrap()
            .contains("outside the file"));

        let missing_reply = format!("rep-{}", unique("missing"));
        nats.inject(
            &format!("p.{pair}.xfer.up.download_nope.pull.0"),
            Some(&missing_reply),
            Vec::new(),
        );
        let reply = await_publish(&mut tap, &missing_reply, Duration::from_secs(5)).await;
        assert!(reply.json()["error"]
            .as_str()
            .unwrap()
            .contains("expired or does not exist"));

        // A transfer request without a reply subject is processed but never
        // answered (the reply-match `None` arm).
        nats.inject(&format!("p.{pair}.xfer.up.ghost.bogus.0"), None, Vec::new());
        tokio::time::sleep(Duration::from_millis(100)).await;

        // Losing the server ends the subscription stream; the loop logs and
        // resubscribes (failing while the server stays down) instead of dying.
        nats.kill();
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert!(!loop_handle.is_finished(), "transfer loop must self-heal");
        loop_handle.abort();
        cancel_upload(&upload_id).unwrap();
        std::fs::remove_file(&download_path).ok();
    }
}
