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
        cancel(desktop_id, &info.transfer_id).await;
        return Err(crate::AppError::Message(format!(
            "remote_download_too_large:{}",
            MAX_DOWNLOAD_BYTES
        )));
    }
    if info.chunk_bytes == 0 {
        // A zero would make the chunk count a division by zero, and a host that
        // declares one cannot be asked for its file in any indexed way.
        cancel(desktop_id, &info.transfer_id).await;
        return Err(crate::AppError::Message(
            "remote_download_bad_chunk_size".into(),
        ));
    }
    match fetch(desktop_id, &info).await {
        Ok(bytes) => Ok(DownloadedFile {
            bytes,
            name: info.name,
        }),
        Err(error) => {
            // Release the host's record rather than leaving it to expire: the
            // host holds a prepared copy, and a failed attempt should not keep
            // one alive on the other machine.
            cancel(desktop_id, &info.transfer_id).await;
            Err(error)
        }
    }
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

async fn cancel(desktop_id: &str, transfer_id: &str) {
    // Best effort by design: the host expires unreferenced transfers on its own,
    // so a failure here leaks memory on the other machine for a while rather
    // than failing an operation the user already saw succeed or fail.
    let _ = super::runtime::request(
        desktop_id,
        json!({ "type": "download_cancel", "transferId": transfer_id }),
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
/// The join itself lives in the runtime, which owns both the inbox and the
/// connection; this is the retry wrapper around it.
/// Whether a failed pull is worth another attempt.
///
/// Split out because it is the one decision here with a failure mode that is not
/// self-evident: retrying a host's *refusal* delays a definite error by two more
/// round trips, while failing to retry a transport fault turns a lost packet
/// into a failed download.
pub(crate) fn retryable(error: &crate::AppError) -> bool {
    matches!(error, crate::AppError::RemoteTransport(_))
}

async fn pull_chunk(
    desktop_id: &str,
    transfer_id: &str,
    index: u64,
) -> Result<Vec<u8>, crate::AppError> {
    // The *first* transport failure, kept because it is the cause. A dropped
    // connection makes every later attempt report "not connected", and reporting
    // that symptom instead of "the request timed out" sends a support session
    // looking at pairing when the fault was the network.
    let mut first: Option<crate::AppError> = None;
    for _ in 0..CHUNK_ATTEMPTS {
        match super::runtime::pull_chunk(desktop_id, transfer_id, index).await {
            Ok(bytes) => return Ok(bytes),
            Err(error) => {
                if !retryable(&error) {
                    return Err(first.unwrap_or(error));
                }
                first.get_or_insert(error);
            }
        }
    }
    Err(first.unwrap_or_else(|| crate::AppError::Message("remote_download_chunk_failed".into())))
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
