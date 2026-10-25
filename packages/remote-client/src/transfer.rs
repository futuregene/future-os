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

use crate::{
    session::{PeerSession, COMMAND_TIMEOUT},
    Client,
};
pub use future_remote_protocol::files::DownloadInfo;
use future_remote_protocol::files::{UploadComplete, UploadInit};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{sync::Arc, time::Duration};

const CHUNK_ATTEMPTS: usize = 3;

const CHUNK_CONCURRENCY: usize = 4;

#[derive(Debug)]
pub struct DownloadedFile {
    pub name: String,
    pub bytes: Vec<u8>,
}

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

#[derive(Debug)]
pub struct UploadedFile {
    pub upload_id: String,
    pub content_hash: String,
    pub name: String,
}

/// Check the host's hash of what it received against the local bytes.
///
/// A predicate rather than inline code so every arm is reachable: a real host
/// cannot be made to receive different bytes than it was sent, so the only way
/// to exercise the mismatch is to give this function a hash that does not belong
/// to the bytes it is checking.
fn verified_hash(host_hash: &str, bytes: &[u8], name: &str) -> Result<String, crate::Error> {
    let local = format!("{:x}", Sha256::digest(bytes));
    if !host_hash.eq_ignore_ascii_case(&local) {
        return Err(crate::Error::Message(format!(
            "remote_upload_hash_mismatch:{name}"
        )));
    }
    Ok(host_hash.to_string())
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
fn usable_chunk_size(chunk_bytes: u64) -> Result<usize, crate::Error> {
    if chunk_bytes == 0 {
        return Err(crate::Error::Message("remote_upload_bad_chunk_size".into()));
    }
    Ok(usize::try_from(chunk_bytes).unwrap_or(usize::MAX))
}

const MAX_DOWNLOAD_BYTES: u64 = 512 * 1024 * 1024;

#[derive(Clone, Copy)]
pub enum TransferKind {
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

/// The bytes of one chunk, joining the host's data publish to its
/// acknowledgement of the pull that asked for it.
///
/// Whether a failed transfer step is worth another attempt.
///
/// Split out because it is the one decision here with a failure mode that is not
/// self-evident: retrying a host's *refusal* delays a definite error by two more
/// round trips, while failing to retry a transport fault turns a lost packet
/// into a failed transfer.
pub fn retryable(error: &crate::Error) -> bool {
    matches!(error, crate::Error::RemoteTransport(_))
}

/// A file operation is bound to the connection that began it. Automatic cleanup
/// and retries never reconnect a host the user disconnected or re-paired.
struct Transfer<'a> {
    client: &'a Client,
    desktop_id: &'a str,
    epoch: u64,
    session: Arc<PeerSession>,
}
impl<'a> Transfer<'a> {
    async fn new(client: &'a Client, desktop_id: &'a str) -> Result<Self, crate::Error> {
        let (epoch, session) = client.transfer_session(desktop_id).await?;
        Ok(Self {
            client,
            desktop_id,
            epoch,
            session,
        })
    }
    async fn current(&self) -> Result<(), crate::Error> {
        self.client
            .check_transfer(self.desktop_id, self.epoch, &self.session)
            .await
    }
    async fn request(&self, command: Value, lane: &str) -> Result<Value, crate::Error> {
        self.current().await?;
        self.client
            .request_for_transfer(self.desktop_id, self.epoch, command, lane)
            .await
    }
    async fn send_chunk(
        &self,
        transfer: &str,
        index: u64,
        bytes: &[u8],
    ) -> Result<(), crate::Error> {
        self.current().await?;
        let subject = self.session.transfer_chunk_subject(transfer, index);
        let result = self
            .client
            .while_running(self.session.request_bytes(&subject, bytes, COMMAND_TIMEOUT))
            .await;
        if let Err(error) = &result {
            self.client
                .record_transfer_failure(self.desktop_id, &self.session, error)
                .await;
        }
        result.map(|_| ())
    }
    async fn receive_chunk(&self, transfer: &str, index: u64) -> Result<Vec<u8>, crate::Error> {
        self.current().await?;
        let (sender, receiver) = tokio::sync::oneshot::channel();
        let _registration =
            self.client
                .register_transfer_chunk(self.desktop_id, transfer, index, sender);
        let subject = self.session.transfer_pull_subject(transfer, index);
        let mut command = json!({"id": self.client.transfer_command_id()});
        let result = self
            .client
            .while_running(async {
                self.current().await?;
                let acknowledgement = self
                    .session
                    .request(&subject, command.take(), COMMAND_TIMEOUT)
                    .await;
                if let Err(error) = &acknowledgement {
                    self.client
                        .record_transfer_failure(self.desktop_id, &self.session, error)
                        .await;
                }
                acknowledgement?;
                tokio::time::timeout(Duration::from_secs(15), receiver)
                    .await
                    .ok()
                    .and_then(Result::ok)
                    .ok_or_else(|| {
                        crate::Error::RemoteTransport("remote_download_chunk_missing".into())
                    })
            })
            .await;
        result
    }
    /// Ask a host to prepare a file, then pull and verify the whole of it.
    async fn download(
        &self,
        session_id: &str,
        path: &str,
        name: Option<&str>,
        variant: &str,
    ) -> Result<DownloadedFile, crate::Error> {
        let info = self.prepare(session_id, path, name, variant).await?;
        self.finish(info).await
    }
    /// Guard, pull and verify an already-prepared transfer.
    ///
    /// Split from `download` because this is where every decision that can go wrong
    /// lives — the size ceiling, the chunk size, and releasing the host's record
    /// when the pull fails — and all of it is reachable with a prepared transfer in
    /// hand. `prepare` above is the request and the field mapping, whose *shape* is
    /// pinned by the contract test that reads the host's own serialization.
    async fn finish(&self, info: DownloadInfo) -> Result<DownloadedFile, crate::Error> {
        if let Err(error) = validate_download(&info) {
            self.cancel_transfer(TransferKind::Download, &info.transfer_id)
                .await;
            return Err(error);
        }
        let transfer_id = info.transfer_id.clone();
        let bytes = self
            .with_release(TransferKind::Download, &transfer_id, || self.fetch(&info))
            .await?;
        Ok(DownloadedFile {
            bytes,
            name: info.name,
        })
    }
    /// Send a file to a host, and verify what it received.
    ///
    /// The mirror of `download`, including its guarantee: the host answers with the
    /// hash of what it wrote, and that is checked against the local bytes. Without
    /// it, a chunk lost on an at-most-once transport would leave a truncated file
    /// attached to the user's message — which they would not discover until the
    /// model read it.
    async fn upload(&self, name: &str, bytes: &[u8]) -> Result<UploadedFile, crate::Error> {
        let started = self.begin_upload(name, bytes.len() as u64).await?;
        let upload_id = started.upload_id.clone();
        let content_hash = self
            .with_release(TransferKind::Upload, &upload_id, || {
                self.finish_upload(&started, name, bytes)
            })
            .await?;
        Ok(UploadedFile {
            upload_id,
            content_hash,
            name: name.to_string(),
        })
    }
    async fn finish_upload(
        &self,
        started: &UploadInit,
        name: &str,
        bytes: &[u8],
    ) -> Result<String, crate::Error> {
        self.send_chunks(started, bytes).await?;
        let complete = self.finish_on_host(&started.upload_id).await?;
        self.current().await?;
        verified_hash(&complete.content_hash, bytes, name)
    }
    /// Ask the host to open a staging record for these bytes.
    ///
    /// The size limits are the host's to enforce and its message is what the user
    /// sees, so nothing is pre-checked here: a second copy of the limit would be a
    /// second place for it to drift.
    async fn begin_upload(&self, name: &str, size: u64) -> Result<UploadInit, crate::Error> {
        let data = self
            .request(
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
            .map_err(|error| crate::Error::Message(format!("remote_upload_bad_init: {error}")))?;
        usable_chunk_size(started.chunk_bytes)?;
        Ok(started)
    }
    /// Write every chunk, in order.
    ///
    /// Sequential rather than in waves, unlike the download: the host appends each
    /// chunk to one file as it arrives, so a wave of concurrent writes would land
    /// them in whatever order the broker delivered them.
    async fn send_chunks(&self, started: &UploadInit, bytes: &[u8]) -> Result<(), crate::Error> {
        let chunk = usable_chunk_size(started.chunk_bytes)?;
        for (index, part) in bytes.chunks(chunk).enumerate() {
            self.put_chunk(&started.upload_id, index as u64, part)
                .await?;
        }
        Ok(())
    }
    /// One chunk, retried on the transport failures a lossy link produces.
    ///
    async fn put_chunk(
        &self,
        upload_id: &str,
        index: u64,
        part: &[u8],
    ) -> Result<(), crate::Error> {
        retrying(|| self.send_chunk(upload_id, index, part)).await
    }
    async fn finish_on_host(&self, upload_id: &str) -> Result<UploadComplete, crate::Error> {
        let data = self
            .request(
                json!({ "type": "upload_complete", "transferId": upload_id }),
                "transfer",
            )
            .await?;
        serde_json::from_value(data)
            .map_err(|error| crate::Error::Message(format!("remote_upload_bad_complete: {error}")))
    }
    async fn prepare(
        &self,
        session_id: &str,
        path: &str,
        name: Option<&str>,
        variant: &str,
    ) -> Result<DownloadInfo, crate::Error> {
        let data = self
            .request(
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
            .map_err(|error| crate::Error::Message(format!("remote_download_bad_prepare: {error}")))
    }
    async fn cancel_transfer(&self, kind: TransferKind, transfer_id: &str) {
        // Best effort by design: the host expires unreferenced transfers on its own,
        // so a failure here leaks bytes on the other machine for a while rather than
        // failing an operation the user already saw succeed or fail.
        let _ = self
            .request(
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
    async fn fetch(&self, info: &DownloadInfo) -> Result<Vec<u8>, crate::Error> {
        validate_download(info)?;
        let chunks = info.size.div_ceil(info.chunk_bytes);
        let mut out: Vec<u8> = Vec::with_capacity(usize::try_from(info.size).unwrap_or(0));
        let mut start = 0u64;
        while start < chunks {
            let wave_end = (start + CHUNK_CONCURRENCY as u64).min(chunks);
            let wave = futures::future::try_join_all(
                (start..wave_end).map(|index| self.pull_chunk(&info.transfer_id, index)),
            )
            .await?;
            // Appended in index order, after the whole wave has arrived: writing as
            // each chunk lands would interleave them.
            for bytes in wave {
                append_chunk(&mut out, &bytes, info.size)?;
            }
            start = wave_end;
        }
        // One check for the whole file rather than one per chunk. A chunk that is
        // short is a chunk that is missing bytes, which shows up here; checking each
        // chunk as well would only report it sooner, and the only way to reach that
        // branch against a real host is for the host to lie about what it served,
        // so it would be a guard no test could exercise.
        if out.len() as u64 != info.size {
            return Err(crate::Error::Message(format!(
                "remote_download_size_mismatch:{}:{}",
                info.size,
                out.len()
            )));
        }
        let digest = format!("{:x}", Sha256::digest(&out));
        if !digest.eq_ignore_ascii_case(&info.content_hash) {
            return Err(crate::Error::Message(
                "remote_download_hash_mismatch".into(),
            ));
        }
        self.current().await?;
        Ok(out)
    }
    /// Run a transfer to completion, releasing the host's copy if it fails.
    ///
    /// Shared by both directions: whichever failed, the host is holding bytes nobody
    /// will claim, and leaving them means they live until its timer expires. One
    /// body rather than two, so the release cannot be remembered in one direction
    /// and forgotten in the other.
    async fn with_release<T, F, Fut>(
        &self,
        kind: TransferKind,
        transfer_id: &str,
        work: F,
    ) -> Result<T, crate::Error>
    where
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = Result<T, crate::Error>>,
    {
        match work().await {
            Ok(value) => Ok(value),
            Err(error) => {
                self.cancel_transfer(kind, transfer_id).await;
                Err(error)
            }
        }
    }
    /// The join itself lives in the runtime, which owns both the inbox and the
    /// connection; this is the retry wrapper around it.
    async fn pull_chunk(&self, transfer_id: &str, index: u64) -> Result<Vec<u8>, crate::Error> {
        retrying(|| self.receive_chunk(transfer_id, index)).await
    }
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
async fn retrying<T, F, Fut>(mut step: F) -> Result<T, crate::Error>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<T, crate::Error>>,
{
    let mut first: Option<crate::Error> = None;
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

impl Client {
    pub async fn download(
        &self,
        desktop: &str,
        session: &str,
        path: &str,
        name: Option<&str>,
        variant: &str,
    ) -> Result<DownloadedFile, crate::Error> {
        self.ensure_connected(desktop).await?;
        Transfer::new(self, desktop)
            .await?
            .download(session, path, name, variant)
            .await
    }
    pub async fn upload(
        &self,
        desktop: &str,
        name: &str,
        bytes: &[u8],
    ) -> Result<UploadedFile, crate::Error> {
        self.ensure_connected(desktop).await?;
        Transfer::new(self, desktop)
            .await?
            .upload(name, bytes)
            .await
    }
    pub async fn finish_download(
        &self,
        desktop: &str,
        info: DownloadInfo,
    ) -> Result<DownloadedFile, crate::Error> {
        // Keep validation ahead of connection lookup, as in the original API.
        if let Err(error) = validate_download(&info) {
            if let Ok(transfer) = Transfer::new(self, desktop).await {
                transfer
                    .cancel_transfer(TransferKind::Download, &info.transfer_id)
                    .await;
            }
            return Err(error);
        }
        Transfer::new(self, desktop).await?.finish(info).await
    }
    #[cfg(any(test, feature = "test-support"))]
    pub async fn fetch_download_for_test(
        &self,
        desktop: &str,
        info: &DownloadInfo,
    ) -> Result<Vec<u8>, crate::Error> {
        Transfer::new(self, desktop).await?.fetch(info).await
    }
    #[cfg(any(test, feature = "test-support"))]
    pub async fn begin_upload_for_test(
        &self,
        desktop: &str,
        name: &str,
        size: u64,
    ) -> Result<(), crate::Error> {
        self.ensure_connected(desktop).await?;
        Transfer::new(self, desktop)
            .await?
            .begin_upload(name, size)
            .await
            .map(|_| ())
    }
}
fn validate_download(info: &DownloadInfo) -> Result<(), crate::Error> {
    if info.size > MAX_DOWNLOAD_BYTES {
        return Err(crate::Error::Message(format!(
            "remote_download_too_large:{}",
            MAX_DOWNLOAD_BYTES
        )));
    }
    if info.chunk_bytes == 0 {
        return Err(crate::Error::Message(
            "remote_download_bad_chunk_size".into(),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{retrying, CHUNK_ATTEMPTS};

    /// A transport failure is exactly what the retry exists for.
    ///
    /// The step is a closure, so the decision can be driven without a broker:
    /// one lost chunk costs one more attempt and the transfer still completes.
    #[tokio::test]
    async fn a_transport_failure_costs_one_retry_and_the_transfer_completes() {
        let mut attempts = 0_usize;
        let result = retrying(|| {
            attempts += 1;
            let attempt = attempts;
            async move {
                if attempt == 1 {
                    // What a dropped NATS Core publish looks like: the chunk
                    // never reached the host, so nothing was done and re-sending
                    // it is idempotent.
                    Err(crate::Error::RemoteTransport("chunk lost in flight".into()))
                } else {
                    Ok(attempt)
                }
            }
        })
        .await;

        assert_eq!(result.expect("the second attempt succeeds"), 2);
        assert_eq!(
            attempts, 2,
            "a retryable failure must cost exactly one retry"
        );
    }

    /// A chunk the host never acknowledges fails the transfer, and it reports
    /// the **first** error.
    ///
    /// A dropped connection makes every later attempt report "not connected",
    /// and reporting that symptom instead of the timeout that caused it is what
    /// sends a support session looking at pairing instead of the network — so
    /// the first recorded error is the one that leaves the function, and the
    /// loop's last line can never be reached without one.
    #[tokio::test]
    async fn a_chunk_that_never_lands_reports_its_first_failure() {
        let mut attempts = 0_usize;
        let error = retrying(|| {
            attempts += 1;
            let attempt = attempts;
            async move {
                Err::<(), _>(crate::Error::RemoteTransport(format!(
                    "attempt {attempt} timed out"
                )))
            }
        })
        .await
        .expect_err("every attempt failed, so the chunk fails");

        assert_eq!(
            error.to_string(),
            "attempt 1 timed out",
            "the first error is the report"
        );
        assert_eq!(
            attempts, CHUNK_ATTEMPTS,
            "the whole attempt budget must be spent before the transfer gives up"
        );
    }
}

#[cfg(test)]
#[path = "transfer_unit_tests.rs"]
mod guard_tests;

fn append_chunk(out: &mut Vec<u8>, bytes: &[u8], declared: u64) -> Result<(), crate::Error> {
    if (out.len() as u64).saturating_add(bytes.len() as u64) > declared {
        return Err(crate::Error::Message(format!(
            "remote_download_size_mismatch:{}:{}",
            declared,
            out.len().saturating_add(bytes.len())
        )));
    }
    out.extend_from_slice(bytes);
    Ok(())
}
