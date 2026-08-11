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
    std::fs::create_dir_all(&dir)?;
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
    let dimensions_reader = image::ImageReader::open(path)?
        .with_guessed_format()
        .map_err(|error| format!("Unreadable image: {error}"))?;
    let (width, height) = dimensions_reader
        .into_dimensions()
        .map_err(|error| format!("Undecodable image header: {error}"))?;
    if width.max(height) > 2000 {
        return Err("Image longest edge exceeds the 2000 px mobile limit."
            .to_string()
            .into());
    }

    // Still decode once so a valid-looking header cannot smuggle corrupt image
    // bytes into the agent, but cap allocation for the already-bounded image.
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

pub async fn prepare_download(
    session_id: &str,
    requested_path: &str,
) -> Result<DownloadInfo, crate::AppError> {
    prune_expired();
    let entries = crate::agent_bridge::get_session_entries(session_id.to_string()).await?;
    let display_name = session_attachment_name(&entries, requested_path).ok_or_else(|| {
        crate::AppError::Message(
            "The requested file is not an attachment in this session.".to_string(),
        )
    })?;
    let source = Path::new(requested_path).canonicalize()?;
    if !source.is_file() {
        return Err("The attachment is no longer available.".to_string().into());
    }
    let prepared = prepare_preview(&source, &display_name)?;
    let size = std::fs::metadata(&prepared.path)?.len();
    if size > MAX_FILE_BYTES {
        let _ = std::fs::remove_file(&prepared.path);
        return Err(
            "The mobile preview is still larger than 10 MiB; view it on desktop."
                .to_string()
                .into(),
        );
    }
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
    std::fs::create_dir_all(&dir)?;
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
        let resized = if image.width().max(image.height()) > 600 {
            image.resize(600, 600, image::imageops::FilterType::Lanczos3)
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
    let path = dir.join(format!(
        "{stamp}_{}",
        safe_disk_name(&original_name, "attachment")
    ));
    std::fs::copy(source, &path)?;
    let markdown = matches!(ext.as_str(), "md" | "markdown");
    Ok(PreparedPreview {
        path,
        name: original_name,
        mime_type: if markdown {
            "text/markdown".to_string()
        } else {
            "application/octet-stream".to_string()
        },
        preview_kind: if markdown { "markdown" } else { "file" }.to_string(),
    })
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

pub fn spawn_transfer_loop(
    client: async_nats::Client,
    pair_id: String,
    active: std::sync::Arc<std::sync::atomic::AtomicBool>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let subject = format!("p.{pair_id}.xfer.up.>");
        let queue = format!("bridge-transfer.{pair_id}");
        let mut sub = match client.queue_subscribe(subject.clone(), queue).await {
            Ok(sub) => sub,
            Err(error) => {
                eprintln!("remote: failed to subscribe to transfers {subject}: {error}");
                return;
            }
        };
        let mut cleanup = tokio::time::interval(Duration::from_secs(60));
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
                        if let Ok(bytes) = serde_json::to_vec(&body) {
                            let _ = client.publish(reply, bytes.into()).await;
                        }
                    }
                }
            }
        }
    })
}

fn write_upload_chunk(transfer_id: &str, index: u64, payload: &[u8]) -> Result<Value, String> {
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
    let mut file = OpenOptions::new()
        .append(true)
        .open(&item.path)
        .map_err(|error| error.to_string())?;
    file.write_all(payload).map_err(|error| error.to_string())?;
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
        attachment_is_in_session, display_name, is_animated_image, safe_disk_name,
        validate_mobile_image, MAX_FILE_BYTES,
    };
    use serde_json::json;

    #[test]
    fn sanitizes_upload_names() {
        assert_eq!(safe_disk_name("../../hello?.md", "x"), "hello.md");
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

    #[test]
    fn rejects_mobile_images_over_2000_pixels() {
        let dir = std::env::temp_dir().join(format!(
            "futureos-transfer-test-{}",
            nkeys::KeyPair::new_user().public_key()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let accepted = dir.join("accepted.png");
        image::DynamicImage::new_rgb8(2000, 1)
            .save(&accepted)
            .unwrap();
        assert!(validate_mobile_image(&accepted).is_ok());

        let rejected = dir.join("rejected.png");
        image::DynamicImage::new_rgb8(2001, 1)
            .save(&rejected)
            .unwrap();
        assert!(validate_mobile_image(&rejected).is_err());
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
}
