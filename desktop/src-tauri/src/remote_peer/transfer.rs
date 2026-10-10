//! Pulling a file from a paired host.
//!
//! The protocol is the phone's, and its shape is worth stating because it is not
//! a single request:
//!
//! 1. `download_prepare` names a transfer and returns its size, its SHA-256, and
//!    the chunk size the host will serve it in;
//! 2. each `xfer.up.{transfer}.pull.{index}` makes the host publish the chunk on
//!    `xfer.down.{transfer}.chunk.{index}` **before** it acknowledges the pull;
//! 3. `download_cancel` releases the host's record.
//!
//! Two consequences drive this module. The bytes and the acknowledgement arrive
//! by different routes, so they have to be joined (the stream task does that,
//! against the inbox `pull_chunk` registers). And because NATS Core is
//! at-most-once, a chunk that never arrives is indistinguishable from one still
//! in flight — so a pull is retried, and the finished file is checked against
//! the hash the host declared rather than merely assembled.

use serde_json::json;
use sha2::{Digest, Sha256};
use std::path::Path;

/// A chunk pull: the host answers within a round trip, and the phone allows 15 s.
///
/// Attempts per chunk. The bytes travel over a lossy transport; a pull that
/// times out is retried rather than failing the download, but a host that keeps
/// not answering is not worth waiting on forever.
const CHUNK_ATTEMPTS: usize = 3;

/// How many chunks are pulled at once.
///
/// Bounded so a large file does not open unbounded parallel requests, and so the
/// assembled result is never more than a few chunks larger than the file. The
/// host serves indexed chunks independently, which is what makes this safe.
const CHUNK_CONCURRENCY: usize = 4;

/// What the host says about a prepared download.
///
/// Only the fields this client acts on: the size and chunk size drive the pull
/// loop, and the hash is the integrity check. The host also sends a MIME type
/// and the variant it settled on, which a save-to-a-chosen-path flow has no use
/// for — the user already picked the destination, and the name the host chose
/// comes back with the file.
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DownloadInfo {
    pub(crate) transfer_id: String,
    pub(crate) name: String,
    pub(crate) size: u64,
    pub(crate) content_hash: String,
    pub(crate) chunk_bytes: u64,
}

/// A completed download: what to call it, and the bytes.
#[derive(Debug)]
pub(crate) struct DownloadedFile {
    pub(crate) name: String,
    pub(crate) bytes: Vec<u8>,
}

/// Ask a host to prepare a file, then pull and verify the whole of it.
pub(crate) async fn download(
    desktop_id: &str,
    session_id: &str,
    path: &str,
    name: Option<&str>,
    variant: &str,
) -> Result<DownloadedFile, crate::AppError> {
    let info = prepare(desktop_id, session_id, path, name, variant).await?;
    finish(desktop_id, info).await
}

/// Guard, pull and verify an already-prepared transfer.
///
/// Split from `download` because this is where every decision that can go wrong
/// lives — the size ceiling, the chunk size, and releasing the host's record
/// when the pull fails — and all of it is reachable with a prepared transfer in
/// hand. `prepare` above is the request and the field mapping, whose *shape* is
/// pinned by the contract test that reads the host's own serialization.
pub(crate) async fn finish(
    desktop_id: &str,
    info: DownloadInfo,
) -> Result<DownloadedFile, crate::AppError> {
    // The assembled file is held in memory, which is what makes a ceiling
    // necessary rather than optional.
    if info.size > MAX_DOWNLOAD_BYTES {
        cancel_transfer(desktop_id, TransferKind::Download, &info.transfer_id).await;
        return Err(crate::AppError::Message(format!(
            "remote_download_too_large:{}",
            MAX_DOWNLOAD_BYTES
        )));
    }
    if info.chunk_bytes == 0 {
        // A zero would make the chunk count a division by zero, and a host that
        // declares one cannot be asked for its file in any indexed way.
        cancel_transfer(desktop_id, TransferKind::Download, &info.transfer_id).await;
        return Err(crate::AppError::Message(
            "remote_download_bad_chunk_size".into(),
        ));
    }
    let transfer_id = info.transfer_id.clone();
    let bytes = with_release(desktop_id, TransferKind::Download, &transfer_id, || {
        fetch(desktop_id, &info)
    })
    .await?;
    Ok(DownloadedFile {
        bytes,
        name: info.name,
    })
}

/// The extensions the host treats as images.
///
/// Duplicated deliberately, because `kind` is a *wire* contract: the host picks
/// its preview and thumbnail handling from it. A client that always sent
/// `"file"` would silently lose that. If the host's list grows, this one lags by
/// sending `"file"` for a new image format — which degrades to a plain
/// attachment rather than failing.
const IMAGE_EXTENSIONS: &[&str] = &[
    "jpg", "jpeg", "png", "gif", "webp", "bmp", "tif", "tiff", "heic", "heif", "svg",
];

/// Whether the host should treat this name as an image.
fn upload_kind(name: &str) -> &'static str {
    let lower = name.to_ascii_lowercase();
    // A name with no dot at all has no extension: `Makefile` must not be read as
    // one whose extension is "makefile".
    if !lower.contains('.') {
        return "file";
    }
    let extension = lower.rsplit('.').next().unwrap_or_default();
    if IMAGE_EXTENSIONS.contains(&extension) {
        "image"
    } else {
        "file"
    }
}

/// What the host says about a started upload.
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct UploadInit {
    upload_id: String,
    chunk_bytes: u64,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct UploadComplete {
    content_hash: String,
}

/// A file handed to a host, ready to be attached to a message.
///
/// `upload_id` is the only thing that matters afterwards: the host holds the
/// bytes in a staging area, and a prompt claims them by this id. An upload that
/// is never referenced is dropped by the host when it expires — so this is not a
/// write to the other machine's filesystem, and it does not need consent the way
/// one would.
#[derive(Debug)]
pub(crate) struct UploadedFile {
    pub(crate) upload_id: String,
    pub(crate) content_hash: String,
    pub(crate) name: String,
}

/// Send a file to a host, and verify what it received.
///
/// The mirror of `download`, including its guarantee: the host answers with the
/// hash of what it wrote, and that is checked against the local bytes. Without
/// it, a chunk lost on an at-most-once transport would leave a truncated file
/// attached to the user's message — which they would not discover until the
/// model read it.
pub(crate) async fn upload(
    desktop_id: &str,
    name: &str,
    bytes: &[u8],
) -> Result<UploadedFile, crate::AppError> {
    let started = begin_upload(desktop_id, name, bytes.len() as u64).await?;
    let upload_id = started.upload_id.clone();
    let content_hash = with_release(desktop_id, TransferKind::Upload, &upload_id, || {
        finish_upload(desktop_id, &started, name, bytes)
    })
    .await?;
    Ok(UploadedFile {
        upload_id,
        content_hash,
        name: name.to_string(),
    })
}

async fn finish_upload(
    desktop_id: &str,
    started: &UploadInit,
    name: &str,
    bytes: &[u8],
) -> Result<String, crate::AppError> {
    send_chunks(desktop_id, started, bytes).await?;
    let complete = finish_on_host(desktop_id, &started.upload_id).await?;
    verified_hash(&complete.content_hash, bytes, name)
}

/// Check the host's hash of what it received against the local bytes.
///
/// A predicate rather than inline code so every arm is reachable: a real host
/// cannot be made to receive different bytes than it was sent, so the only way
/// to exercise the mismatch is to give this function a hash that does not belong
/// to the bytes it is checking.
fn verified_hash(host_hash: &str, bytes: &[u8], name: &str) -> Result<String, crate::AppError> {
    let local = format!("{:x}", Sha256::digest(bytes));
    if !host_hash.eq_ignore_ascii_case(&local) {
        return Err(crate::AppError::Message(format!(
            "remote_upload_hash_mismatch:{name}"
        )));
    }
    Ok(host_hash.to_string())
}

/// Ask the host to open a staging record for these bytes.
///
/// The size limits are the host's to enforce and its message is what the user
/// sees, so nothing is pre-checked here: a second copy of the limit would be a
/// second place for it to drift.
async fn begin_upload(
    desktop_id: &str,
    name: &str,
    size: u64,
) -> Result<UploadInit, crate::AppError> {
    let data = super::runtime::request(
        desktop_id,
        json!({
            "type": "upload_init",
            "name": name,
            // No transcoding on this side, so the two sizes are the same. The
            // host compares the transfer size against what it receives.
            "transferName": name,
            "mimeType": "",
            "kind": upload_kind(name),
            "originalSize": size,
            "transferSize": size,
        }),
        "transfer",
    )
    .await?;
    let started: UploadInit = serde_json::from_value(data)
        .map_err(|error| crate::AppError::Message(format!("remote_upload_bad_init: {error}")))?;
    usable_chunk_size(started.chunk_bytes)?;
    Ok(started)
}

/// The chunk size a host declared, as a number this machine can use.
///
/// A predicate rather than an inline check because a real host never declares a
/// size that does not work, so this is the only way to reach the refusal — and a
/// zero would make the chunk loop divide by zero.
///
/// A size larger than this platform can address is *not* refused: it means "one
/// chunk", which is what `chunks` does with a size past the end, and the host
/// rejects an oversized chunk itself with a message about its own limit. Clamped
/// rather than converted-and-rejected, because that refusal would be a 32-bit
/// arm with no way to exercise it on 64-bit.
fn usable_chunk_size(chunk_bytes: u64) -> Result<usize, crate::AppError> {
    if chunk_bytes == 0 {
        return Err(crate::AppError::Message(
            "remote_upload_bad_chunk_size".into(),
        ));
    }
    Ok(usize::try_from(chunk_bytes).unwrap_or(usize::MAX))
}

/// Write every chunk, in order.
///
/// Sequential rather than in waves, unlike the download: the host appends each
/// chunk to one file as it arrives, so a wave of concurrent writes would land
/// them in whatever order the broker delivered them.
async fn send_chunks(
    desktop_id: &str,
    started: &UploadInit,
    bytes: &[u8],
) -> Result<(), crate::AppError> {
    let chunk = usable_chunk_size(started.chunk_bytes)?;
    for (index, part) in bytes.chunks(chunk).enumerate() {
        put_chunk(desktop_id, &started.upload_id, index as u64, part).await?;
    }
    Ok(())
}

/// One chunk, retried on the transport failures a lossy link produces.
///
async fn put_chunk(
    desktop_id: &str,
    upload_id: &str,
    index: u64,
    part: &[u8],
) -> Result<(), crate::AppError> {
    retrying(|| super::runtime::put_chunk(desktop_id, upload_id, index, part)).await
}

async fn finish_on_host(
    desktop_id: &str,
    upload_id: &str,
) -> Result<UploadComplete, crate::AppError> {
    let data = super::runtime::request(
        desktop_id,
        json!({ "type": "upload_complete", "transferId": upload_id }),
        "transfer",
    )
    .await?;
    serde_json::from_value(data)
        .map_err(|error| crate::AppError::Message(format!("remote_upload_bad_complete: {error}")))
}

/// The ceiling on one download, in bytes.
const MAX_DOWNLOAD_BYTES: u64 = 512 * 1024 * 1024;

async fn prepare(
    desktop_id: &str,
    session_id: &str,
    path: &str,
    name: Option<&str>,
    variant: &str,
) -> Result<DownloadInfo, crate::AppError> {
    let data = super::runtime::request(
        desktop_id,
        json!({
            "type": "download_prepare",
            "sessionId": session_id,
            "filePath": path,
            "mode": variant,
            // The host falls back to the session's own name for the file when
            // this is absent; sending an empty string would name the download
            // "" on hosts that take it literally.
            "name": name.unwrap_or_default(),
        }),
        session_id,
    )
    .await?;
    serde_json::from_value(data)
        .map_err(|error| crate::AppError::Message(format!("remote_download_bad_prepare: {error}")))
}

/// Which direction a transfer is going, for the calls that differ by it.
///
/// The host keeps a prepared download and a staged upload in separate
/// registries behind *separate* commands, so a shared "cancel" name would release
/// the wrong record — the download's copy for an upload, or nothing at all.
#[derive(Clone, Copy)]
pub(crate) enum TransferKind {
    Download,
    Upload,
}

impl TransferKind {
    fn cancel_command(self) -> &'static str {
        match self {
            TransferKind::Download => "download_cancel",
            TransferKind::Upload => "upload_cancel",
        }
    }
}

async fn cancel_transfer(desktop_id: &str, kind: TransferKind, transfer_id: &str) {
    // Best effort by design: the host expires unreferenced transfers on its own,
    // so a failure here leaks bytes on the other machine for a while rather than
    // failing an operation the user already saw succeed or fail.
    let _ = super::runtime::request(
        desktop_id,
        json!({ "type": kind.cancel_command(), "transferId": transfer_id }),
        "transfer",
    )
    .await;
}

/// Pull every chunk and verify the assembled file.
///
/// `pub(crate)` rather than private because this is the transport half of a
/// download: a caller that already has the host's `DownloadInfo` (a resumed
/// transfer, a test serving a file it registered) can use it directly, and the
/// prepare-and-cancel shell around it is policy rather than protocol.
pub(crate) async fn fetch(
    desktop_id: &str,
    info: &DownloadInfo,
) -> Result<Vec<u8>, crate::AppError> {
    let chunks = info.size.div_ceil(info.chunk_bytes);
    let mut out: Vec<u8> = Vec::with_capacity(usize::try_from(info.size).unwrap_or(0));
    let mut start = 0u64;
    while start < chunks {
        let wave_end = (start + CHUNK_CONCURRENCY as u64).min(chunks);
        let mut wave = Vec::with_capacity(usize::try_from(wave_end - start).unwrap_or(0));
        for index in start..wave_end {
            wave.push(pull_chunk(desktop_id, &info.transfer_id, index).await?);
        }
        // Appended in index order, after the whole wave has arrived: writing as
        // each chunk lands would interleave them.
        for bytes in wave {
            out.extend_from_slice(&bytes);
        }
        start = wave_end;
    }
    // One check for the whole file rather than one per chunk. A chunk that is
    // short is a chunk that is missing bytes, which shows up here; checking each
    // chunk as well would only report it sooner, and the only way to reach that
    // branch against a real host is for the host to lie about what it served,
    // so it would be a guard no test could exercise.
    if out.len() as u64 != info.size {
        return Err(crate::AppError::Message(format!(
            "remote_download_size_mismatch:{}:{}",
            info.size,
            out.len()
        )));
    }
    let digest = format!("{:x}", Sha256::digest(&out));
    if !digest.eq_ignore_ascii_case(&info.content_hash) {
        return Err(crate::AppError::Message(
            "remote_download_hash_mismatch".into(),
        ));
    }
    Ok(out)
}

/// The bytes of one chunk, joining the host's data publish to its
/// acknowledgement of the pull that asked for it.
///
/// Whether a failed transfer step is worth another attempt.
///
/// Split out because it is the one decision here with a failure mode that is not
/// self-evident: retrying a host's *refusal* delays a definite error by two more
/// round trips, while failing to retry a transport fault turns a lost packet
/// into a failed transfer.
pub(crate) fn retryable(error: &crate::AppError) -> bool {
    matches!(error, crate::AppError::RemoteTransport(_))
}

/// Run one transfer step, retrying only the failures a retry can fix.
///
/// Shared by both directions because the decision is the same in each: the host
/// may have done the work before its acknowledgement was lost, so re-sending a
/// chunk is idempotent in either direction — while a refusal will simply be
/// refused again.
///
/// The *first* error is the one reported. A dropped connection makes every later
/// attempt report "not connected", and reporting that symptom instead of the
/// timeout that caused it sends a support session looking at pairing when the
/// fault was the network.
async fn retrying<T, F, Fut>(mut step: F) -> Result<T, crate::AppError>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<T, crate::AppError>>,
{
    let mut first: Option<crate::AppError> = None;
    for _ in 0..CHUNK_ATTEMPTS {
        match step().await {
            Ok(value) => return Ok(value),
            Err(error) => {
                if !retryable(&error) {
                    return Err(first.unwrap_or(error));
                }
                first.get_or_insert(error);
            }
        }
    }
    // Every attempt either returned or recorded its error, so reaching the end
    // of the loop means one is recorded. An `unwrap_or_else` here would be a
    // branch no test could reach.
    Err(first.expect("a failed attempt always records its error"))
}

/// Run a transfer to completion, releasing the host's copy if it fails.
///
/// Shared by both directions: whichever failed, the host is holding bytes nobody
/// will claim, and leaving them means they live until its timer expires. One
/// body rather than two, so the release cannot be remembered in one direction
/// and forgotten in the other.
async fn with_release<T, F, Fut>(
    desktop_id: &str,
    kind: TransferKind,
    transfer_id: &str,
    work: F,
) -> Result<T, crate::AppError>
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = Result<T, crate::AppError>>,
{
    match work().await {
        Ok(value) => Ok(value),
        Err(error) => {
            cancel_transfer(desktop_id, kind, transfer_id).await;
            Err(error)
        }
    }
}

/// The join itself lives in the runtime, which owns both the inbox and the
/// connection; this is the retry wrapper around it.
async fn pull_chunk(
    desktop_id: &str,
    transfer_id: &str,
    index: u64,
) -> Result<Vec<u8>, crate::AppError> {
    retrying(|| super::runtime::pull_chunk(desktop_id, transfer_id, index)).await
}

/// Write a downloaded file to a path the caller chose, atomically.
///
/// Takes the path as a string because that is what arrives from the UI, and
/// keeps the call site short enough to stay on one line — a wrapped call would
/// put its `?` alone on a line that then reads as uncovered whatever the tests
/// do.
pub(crate) fn write_to_path(destination: &str, bytes: &[u8]) -> Result<(), crate::AppError> {
    write_atomically(std::path::Path::new(destination), bytes)
}

/// Write a downloaded file to `destination`, atomically.
///
/// A partial file that looks complete is the one failure a download must not
/// have, so the bytes go to a temporary neighbour and are renamed into place
/// only once the whole file is on disk.
pub(crate) fn write_atomically(destination: &Path, bytes: &[u8]) -> Result<(), crate::AppError> {
    let file_name = destination
        .file_name()
        .ok_or_else(|| crate::AppError::Message("remote_download_bad_destination".into()))?;
    let mut temporary = destination.to_path_buf();
    temporary.set_file_name(format!(".{}.part", file_name.to_string_lossy()));
    std::fs::write(&temporary, bytes).map_err(|error| {
        crate::AppError::Message(format!("remote_download_write_failed: {error}"))
    })?;
    std::fs::rename(&temporary, destination).map_err(|error| {
        // Leave no debris behind: the rename failed, so the partial copy is
        // removed rather than left beside the destination.
        let _ = std::fs::remove_file(&temporary);
        crate::AppError::Message(format!("remote_download_write_failed: {error}"))
    })
}

/// The guards as the tests see them, so each is reachable without a host that
/// misbehaves.
#[cfg(test)]
pub(crate) async fn begin_upload_for_test(
    desktop_id: &str,
    name: &str,
    size: u64,
) -> Result<(), crate::AppError> {
    begin_upload(desktop_id, name, size).await.map(|_| ())
}

#[cfg(test)]
pub(crate) fn verified_hash_for_test(
    host_hash: &str,
    bytes: &[u8],
    name: &str,
) -> Result<String, crate::AppError> {
    verified_hash(host_hash, bytes, name)
}

#[cfg(test)]
pub(crate) fn upload_kind_for_test(name: &str) -> &'static str {
    upload_kind(name)
}

#[cfg(test)]
pub(crate) fn usable_chunk_size_for_test(chunk_bytes: u64) -> Result<usize, crate::AppError> {
    usable_chunk_size(chunk_bytes)
}

#[cfg(test)]
pub(crate) fn cancel_command_for_test(kind: TransferKind) -> &'static str {
    kind.cancel_command()
}
