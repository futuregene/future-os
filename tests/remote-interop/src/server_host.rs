use crate::CHUNK_BYTES;
use future_remote_protocol::IncomingCmd;
use future_remote_server::{
    services::*,
    supervisor::{PublisherTiming, RuntimeOptions, TaskExecutor},
    Error, PairingCreds,
};
use futures::future::BoxFuture;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, HashMap},
    sync::{
        atomic::{AtomicU64, Ordering},
        Condvar, Mutex,
    },
    time::Duration,
};

struct Upload {
    size: u64,
    parts: BTreeMap<u64, Vec<u8>>,
}
#[derive(Default)]
struct ChunkGateState {
    armed: bool,
    released: bool,
    waiting: bool,
    reads: u64,
    finished: u64,
}
#[derive(Default)]
struct ChunkGate {
    state: Mutex<ChunkGateState>,
    changed: Condvar,
    entered: tokio::sync::Notify,
    active: AtomicU64,
    maximum_active: AtomicU64,
}
struct ChunkRead<'a>(&'a ChunkGate);
impl Drop for ChunkRead<'_> {
    fn drop(&mut self) {
        self.0.state.lock().unwrap().finished += 1;
        self.0.active.fetch_sub(1, Ordering::SeqCst);
    }
}
impl ChunkGate {
    fn enter(&self, delay: Duration) -> Result<ChunkRead<'_>, Error> {
        let mut state = self.state.lock().unwrap();
        state.reads += 1;
        let active = self.active.fetch_add(1, Ordering::SeqCst) + 1;
        self.maximum_active.fetch_max(active, Ordering::SeqCst);
        if !delay.is_zero() {
            self.entered.notify_one();
            let (next, _) =
                tokio::task::block_in_place(|| self.changed.wait_timeout(state, delay).unwrap());
            state = next;
        }
        if state.armed {
            state.waiting = true;
            // Signal the actual held read, rather than requiring the test's
            // timer/IO driver to progress while a worker is synchronously held.
            self.entered.notify_one();
            // This models native IO that ignores Client cancellation. Tell Tokio
            // the worker is blocking so the shared Rust fixture runtime can
            // continue driving the Client, timers and harness control messages.
            let (mut next, timeout) = tokio::task::block_in_place(|| {
                self.changed
                    .wait_timeout_while(state, Duration::from_secs(20), |state| !state.released)
                    .unwrap()
            });
            next.waiting = false;
            next.armed = false;
            if timeout.timed_out() && !next.released {
                return Err(Error::Message("fixture chunk gate timed out".into()));
            }
        }
        Ok(ChunkRead(self))
    }
    fn release(&self) {
        self.state.lock().unwrap().released = true;
        self.changed.notify_all();
    }
}
pub struct MemoryServerHost {
    creds: Mutex<Option<PairingCreds>>,
    active: Mutex<Option<String>>,
    counts: Mutex<BTreeMap<String, usize>>,
    uploads: Mutex<HashMap<String, Upload>>,
    downloads: Mutex<HashMap<String, Vec<u8>>>,
    sequence: AtomicU64,
    file_bytes: Vec<u8>,
    catalog_label: String,
    chunk_bytes: usize,
    chunk_delay: Duration,
    upload_chunks: AtomicU64,
    chunk_gate: ChunkGate,
    /// One immutable paged read, as the desktop's own paging transport keeps it:
    /// a snapshot fixed by the first reply, served by index afterwards.
    paged: Mutex<Option<PagedRead>>,
    /// How that snapshot's chunks are answered.
    page_encoding: Mutex<PageEncoding>,
}

/// A paged read the fixture serves, and how its chunks are carried.
struct PagedRead {
    id: String,
    bytes: Vec<u8>,
}

/// Which carrier the fixture answers a paged read with.
///
/// The two arms of the page-encoding measurement differ *only* here: everything
/// else — the client, the broker, the sealed channel, the snapshot, the chunk
/// size — is the same run, which is what makes the byte and wall-clock numbers
/// comparable.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PageEncoding {
    /// base64 inside the JSON reply: what a desktop that never negotiated
    /// `read_binary_v1` sends, byte for byte.
    Base64,
    /// The chunk's bytes behind the JSON header (`ReplySink::send_binary`), which
    /// is what the same host sends once the client declares the capability.
    Binary,
}

/// The desktop's own chunk size, so the client's offsets line up with the
/// snapshot it is paging.
const PAGE_CHUNK_BYTES: usize = 192 * 1024;
impl MemoryServerHost {
    pub fn new(creds: PairingCreds, file_bytes: Vec<u8>) -> Self {
        let active = creds.pair_id.clone();
        let catalog_label = creds.desktop_id.clone();
        Self {
            creds: Mutex::new(Some(creds)),
            active: Mutex::new(Some(active)),
            counts: Mutex::default(),
            uploads: Mutex::default(),
            downloads: Mutex::default(),
            sequence: AtomicU64::new(1),
            file_bytes,
            catalog_label,
            chunk_bytes: CHUNK_BYTES,
            chunk_delay: Duration::ZERO,
            upload_chunks: AtomicU64::new(0),
            chunk_gate: ChunkGate::default(),
            paged: Mutex::default(),
            page_encoding: Mutex::new(PageEncoding::Base64),
        }
    }
    pub fn with_chunk_bytes(mut self, chunk_bytes: usize) -> Self {
        self.chunk_bytes = chunk_bytes;
        self
    }
    pub fn with_chunk_delay(mut self, delay: Duration) -> Self {
        self.chunk_delay = delay;
        self
    }
    /// Serve `bytes` as one immutable paged read.
    ///
    /// The first `get_session_entries` pins it and answers offset 0; every later
    /// `get_read_chunk` for the same id answers its own offset, which is the
    /// desktop's own shape (`remote_host::read_pages`). A snapshot rather than a
    /// live read is the point: a page must not change under a client that is
    /// still collecting it.
    pub fn install_page(&self, bytes: Vec<u8>) {
        *self.paged.lock().unwrap() = Some(PagedRead {
            id: format!(
                "read_fixture_{}",
                self.sequence.fetch_add(1, Ordering::Relaxed)
            ),
            bytes,
        });
    }
    /// Which carrier a paged read's chunks are answered with.
    pub fn set_page_encoding(&self, encoding: PageEncoding) {
        *self.page_encoding.lock().unwrap() = encoding;
    }
    pub fn page_bytes(&self) -> Vec<u8> {
        self.paged
            .lock()
            .unwrap()
            .as_ref()
            .map(|page| page.bytes.clone())
            .unwrap_or_default()
    }
    /// The chunk the command asks for, when this fixture is serving a page.
    ///
    /// `None` means "not a paged read": the fixture then behaves exactly as it
    /// did before pages existed, which is what keeps the other interop tests
    /// unaffected by a page being installed.
    fn page_chunk(&self, command: &IncomingCmd) -> Option<Result<(Value, Vec<u8>, usize), String>> {
        let paged = self.paged.lock().unwrap();
        let page = paged.as_ref()?;
        let offset = match command.cmd_type.as_str() {
            "get_session_entries" => 0,
            "get_read_chunk" => {
                if command.reply_id != page.id {
                    return Some(Err("remote_read_expired".into()));
                }
                match usize::try_from(command.offset) {
                    Ok(offset) => offset,
                    Err(_) => return Some(Err("remote_read_invalid_offset".into())),
                }
            }
            _ => return None,
        };
        if offset >= page.bytes.len() || !offset.is_multiple_of(PAGE_CHUNK_BYTES) {
            return Some(Err("remote_read_invalid_offset".into()));
        }
        let end = offset
            .saturating_add(PAGE_CHUNK_BYTES)
            .min(page.bytes.len());
        Some(Ok((
            json!({ "readChunk": {
                "id": page.id, "offset": offset, "nextOffset": end,
                "totalBytes": page.bytes.len(),
            }}),
            page.bytes[offset..end].to_vec(),
            page.bytes.len(),
        )))
    }
    /// Answer one page chunk on the carrier this fixture is configured for.
    async fn send_page(&self, reply: &dyn ReplySink, header: Value, body: Vec<u8>) {
        let encoding = *self.page_encoding.lock().unwrap();
        match encoding {
            PageEncoding::Base64 => {
                // The pre-negotiation reply, produced by the same helper the
                // negotiated fallback uses, so the legacy arm is byte-identical
                // to what a host without the capability sends.
                let data = future_remote_server::reply::with_base64_page(header, &body);
                reply.send(true, data, None).await;
            }
            PageEncoding::Binary => reply.send_binary(header, body).await,
        }
    }
    pub fn download_ids(&self) -> Vec<String> {
        self.downloads.lock().unwrap().keys().cloned().collect()
    }
    pub fn metrics(&self) -> Value {
        let gate = self.chunk_gate.state.lock().unwrap();
        json!({ "executions": self.counts.lock().unwrap().clone(), "uploads": self.uploads.lock().unwrap().len(), "downloads": self.downloads.lock().unwrap().len(),
            "uploadChunks": self.upload_chunks.load(Ordering::SeqCst),
            "chunkWaiting": gate.waiting, "chunkReads": gate.reads, "chunkFinished": gate.finished, "chunkMaxActive": self.chunk_gate.maximum_active.load(Ordering::SeqCst),
            "identityBound": self.creds.lock().unwrap().as_ref().and_then(|creds| creds.secure.as_ref()).is_some_and(|secure| secure.peer_public_key.is_some() && secure.secret.is_none()) })
    }
    pub fn hold_next_chunk(&self) {
        let mut state = self.chunk_gate.state.lock().unwrap();
        assert!(
            !state.waiting && !state.armed,
            "fixture chunk gate already active"
        );
        state.armed = true;
        state.released = false;
    }
    pub async fn wait_for_held_chunk(&self) {
        self.chunk_gate.entered.notified().await;
    }
    pub fn release_chunks(&self) {
        self.chunk_gate.release();
    }
    fn execute_command(&self, command: &IncomingCmd) -> Result<Value, String> {
        *self
            .counts
            .lock()
            .unwrap()
            .entry(command.id.clone())
            .or_default() += 1;
        if command.message == "refuse" {
            return Err("fixture_refused".into());
        }
        match command.cmd_type.as_str() {
            "list_sessions" => Ok(self.sessions("").expect("fixture catalog").0),
            "upload_init" => {
                let id = format!("upload_{}", self.sequence.fetch_add(1, Ordering::Relaxed));
                self.uploads.lock().unwrap().insert(
                    id.clone(),
                    Upload {
                        size: command.transfer_size,
                        parts: BTreeMap::new(),
                    },
                );
                Ok(json!({"uploadId": id, "chunkBytes": self.chunk_bytes}))
            }
            "upload_complete" => {
                let uploads = self.uploads.lock().unwrap();
                let upload = uploads
                    .get(&command.transfer_id)
                    .ok_or("upload expired or does not exist")?;
                let bytes: Vec<u8> = upload.parts.values().flatten().copied().collect();
                if bytes.len() as u64 != upload.size {
                    return Err("upload_size_mismatch".into());
                }
                Ok(
                    json!({"uploadId": command.transfer_id, "contentHash": format!("{:x}", Sha256::digest(bytes))}),
                )
            }
            "upload_cancel" => {
                self.uploads.lock().unwrap().remove(&command.transfer_id);
                Ok(json!({}))
            }
            "download_prepare" => {
                if command.file_path != "fixture.bin" {
                    return Err("not an attachment".into());
                }
                let id = format!("download_{}", self.sequence.fetch_add(1, Ordering::Relaxed));
                self.downloads
                    .lock()
                    .unwrap()
                    .insert(id.clone(), self.file_bytes.clone());
                Ok(
                    json!({"transferId": id, "name": "fixture.bin", "mimeType": "application/octet-stream", "size": self.file_bytes.len(),
                    "contentHash": format!("{:x}", Sha256::digest(&self.file_bytes)), "previewKind": "file", "variant": command.mode, "chunkBytes": self.chunk_bytes}),
                )
            }
            "download_cancel" => {
                self.downloads.lock().unwrap().remove(&command.transfer_id);
                Ok(json!({}))
            }
            _ => Ok(json!({"echo": command.message, "kind": command.cmd_type})),
        }
    }
}
impl BusinessHost for MemoryServerHost {
    fn execute<'a>(&'a self, command: IncomingCmd, reply: &'a dyn ReplySink) -> BoxFuture<'a, ()> {
        Box::pin(async move {
            if let Some(chunk) = self.page_chunk(&command) {
                return match chunk {
                    Ok((header, body, _total)) => self.send_page(reply, header, body).await,
                    Err(error) => reply.send(false, Value::Null, Some(error)).await,
                };
            }
            match self.execute_command(&command) {
                Ok(data) => reply.send(true, data, None).await,
                Err(error) => reply.send(false, Value::Null, Some(error)).await,
            }
        })
    }
}
impl PairingHost for MemoryServerHost {
    fn load(&self) -> Option<PairingCreds> {
        self.creds.lock().unwrap().clone()
    }
    fn load_slots(&self) -> Vec<PairingCreds> {
        self.creds.lock().unwrap().clone().into_iter().collect()
    }
    fn active_slot(&self) -> Option<String> {
        self.active.lock().unwrap().clone()
    }
    fn set_active(&self, pair: &str) -> Result<(), Error> {
        *self.active.lock().unwrap() = Some(pair.into());
        Ok(())
    }
    fn remove_slot(&self, pair: &str) -> Result<(), Error> {
        let mut creds = self.creds.lock().unwrap();
        if creds.as_ref().is_some_and(|creds| creds.pair_id == pair) {
            *creds = None;
        }
        Ok(())
    }
    fn save(&self, creds: &PairingCreds) -> Result<(), Error> {
        *self.creds.lock().unwrap() = Some(creds.clone());
        Ok(())
    }
    fn clear(&self) -> Result<(), Error> {
        *self.creds.lock().unwrap() = None;
        Ok(())
    }
    fn invite(&self, _: bool, _: &[String]) -> BoxFuture<'_, Result<Invitation, Error>> {
        Box::pin(async { Err(Error::Message("fixture credentials are explicit".into())) })
    }
    fn refresh(&self, creds: PairingCreds) -> BoxFuture<'_, Result<PairingCreds, Error>> {
        Box::pin(async move { Ok(creds) })
    }
    fn queue_revoke(&self, _: &PairingCreds) -> Result<(), Error> {
        Ok(())
    }
    fn queue_revoke_invitation(&self, _: &str) -> Result<(), Error> {
        // Credentials are injected by the fixture; no platform invitation exists.
        Ok(())
    }
    fn retry_revokes(&self) -> BoxFuture<'_, Result<(), Error>> {
        Box::pin(async { Ok(()) })
    }
}
impl StateHost for MemoryServerHost {
    fn catalog_epoch(&self) -> String {
        "interop_catalog".into()
    }
    fn sessions(&self, _: &str) -> Option<(Value, String)> {
        Some((
            json!({"sessions": [{"sessionId":"interop_session","title":self.catalog_label}], "version": {"epoch": "interop_catalog", "revision": 0}}),
            self.catalog_label.clone(),
        ))
    }
    fn sessions_catalog(&self, pair: &str) -> Option<SessionsCatalog> {
        self.sessions(pair)
            .map(|(payload, signature)| SessionsCatalog {
                payload,
                structural: signature.clone(),
                signature,
                activity: vec![],
            })
    }
    fn workspaces(&self) -> Option<(Value, String)> {
        Some((
            json!({"workspaces": [], "version": {"epoch": "interop_catalog", "revision": 0}}),
            "empty".into(),
        ))
    }
    fn catalog_revisions(&self) -> (String, u64, u64) {
        ("interop_catalog".into(), 0, 0)
    }
    fn catalog_dirty(&self) -> bool {
        false
    }
    fn agent_available(&self) -> bool {
        true
    }
    fn monitor_agent(&self) -> BoxFuture<'_, ()> {
        Box::pin(std::future::pending())
    }
}
impl FileHost for MemoryServerHost {
    fn upload_chunk(&self, id: &str, index: u64, bytes: &[u8]) -> Result<Value, String> {
        if !self.chunk_delay.is_zero() {
            std::thread::sleep(self.chunk_delay);
        }
        let mut uploads = self.uploads.lock().unwrap();
        let upload = uploads
            .get_mut(id)
            .ok_or("upload expired or does not exist")?;
        if let Some(previous) = upload.parts.get(&index) {
            if previous != bytes {
                return Err("upload_chunk_conflict".into());
            }
        }
        upload.parts.insert(index, bytes.to_vec());
        self.upload_chunks.fetch_add(1, Ordering::SeqCst);
        Ok(json!({}))
    }
    fn download_chunk(&self, id: &str, index: u64) -> Result<Vec<u8>, Error> {
        if !self.chunk_delay.is_zero() {
            std::thread::sleep(self.chunk_delay);
        }
        let _read = self.chunk_gate.enter(self.chunk_delay)?;
        let downloads = self.downloads.lock().unwrap();
        let bytes = downloads
            .get(id)
            .ok_or_else(|| Error::Message("download expired or does not exist".into()))?;
        let start = usize::try_from(index)
            .ok()
            .and_then(|index| index.checked_mul(self.chunk_bytes))
            .filter(|start| *start < bytes.len())
            .ok_or_else(|| Error::Message("outside the file".into()))?;
        Ok(bytes[start..bytes.len().min(start + self.chunk_bytes)].to_vec())
    }
    fn prune_transfers(&self) {}
    fn clear_transfers(&self) {
        self.uploads.lock().unwrap().clear();
        self.downloads.lock().unwrap().clear();
    }
    fn clear_preview_cache(&self) {}
}
pub struct Executor;
impl TaskExecutor for Executor {
    fn spawn(&self, future: BoxFuture<'static, ()>) {
        tokio::spawn(future);
    }
    fn spawn_native(&self, job: Box<dyn FnOnce() + Send + 'static>) {
        tokio::task::spawn_blocking(job);
    }
    fn connection_options(&self) -> future_remote_server::connection::ConnectionOptions {
        future_remote_server::connection::ConnectionOptions::default().for_loopback_fixture()
    }
    fn options(&self) -> RuntimeOptions {
        RuntimeOptions {
            publisher: PublisherTiming::new(Duration::from_millis(100), Duration::from_millis(100))
                .unwrap(),
            automatic_recovery: false,
            revoke_cleanup: false,
            ..Default::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    // No broker or socket: isolate whether the synchronous host gate itself
    // prevents another task from observing/releasing it on the fixture runtime.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn held_chunk_gate_remains_observable_and_releasable() {
        let gate = Arc::new(ChunkGate::default());
        gate.state.lock().unwrap().armed = true;
        let reader = gate.clone();
        let read = tokio::spawn(async move {
            let _held = reader
                .enter(Duration::ZERO)
                .expect("gate released before its deadline");
        });
        let observed = tokio::time::timeout(Duration::from_secs(5), gate.entered.notified()).await;
        let held = {
            let state = gate.state.lock().unwrap();
            (state.waiting, state.reads, state.finished)
        };
        // Release before assertions, including an observation timeout. A failing
        // test must not leave the worker held for the gate's full timeout.
        gate.release();
        let completed = tokio::time::timeout(Duration::from_secs(1), read).await;
        observed.expect("held read remains observable from the test task");
        assert_eq!(held, (true, 1, 0));
        completed
            .expect("released read finishes promptly")
            .expect("reader did not panic");
        let state = gate.state.lock().unwrap();
        assert!(!state.waiting);
        assert!(!state.armed);
        assert_eq!(state.finished, 1);
    }
}
