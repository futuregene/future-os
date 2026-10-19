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
}
struct ChunkRead<'a>(&'a ChunkGate);
impl Drop for ChunkRead<'_> {
    fn drop(&mut self) {
        self.0.state.lock().unwrap().finished += 1;
    }
}
impl ChunkGate {
    fn enter(&self) -> Result<ChunkRead<'_>, Error> {
        let mut state = self.state.lock().unwrap();
        state.reads += 1;
        if state.armed {
            state.waiting = true;
            // Model a synchronous host read that ignores Client cancellation.
            // The fixture uses two workers and always releases on owner drop.
            let (mut next, timeout) = self
                .changed
                .wait_timeout_while(state, Duration::from_secs(20), |state| !state.released)
                .unwrap();
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
    chunk_gate: ChunkGate,
}
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
            chunk_gate: ChunkGate::default(),
        }
    }
    pub fn download_ids(&self) -> Vec<String> {
        self.downloads.lock().unwrap().keys().cloned().collect()
    }
    pub fn metrics(&self) -> Value {
        let gate = self.chunk_gate.state.lock().unwrap();
        json!({ "executions": self.counts.lock().unwrap().clone(), "uploads": self.uploads.lock().unwrap().len(), "downloads": self.downloads.lock().unwrap().len(),
            "chunkWaiting": gate.waiting, "chunkReads": gate.reads, "chunkFinished": gate.finished,
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
                Ok(json!({"uploadId": id, "chunkBytes": CHUNK_BYTES}))
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
                    "contentHash": format!("{:x}", Sha256::digest(&self.file_bytes)), "previewKind": "file", "variant": command.mode, "chunkBytes": CHUNK_BYTES}),
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
    fn invite(&self, _: bool) -> BoxFuture<'_, Result<Invitation, Error>> {
        Box::pin(async { Err(Error::Message("fixture credentials are explicit".into())) })
    }
    fn refresh(&self, creds: PairingCreds) -> BoxFuture<'_, Result<PairingCreds, Error>> {
        Box::pin(async move { Ok(creds) })
    }
    fn queue_revoke(&self, _: &PairingCreds) -> Result<(), Error> {
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
        Ok(json!({}))
    }
    fn download_chunk(&self, id: &str, index: u64) -> Result<Vec<u8>, Error> {
        let _read = self.chunk_gate.enter()?;
        let downloads = self.downloads.lock().unwrap();
        let bytes = downloads
            .get(id)
            .ok_or_else(|| Error::Message("download expired or does not exist".into()))?;
        let start = usize::try_from(index)
            .ok()
            .and_then(|index| index.checked_mul(CHUNK_BYTES))
            .filter(|start| *start < bytes.len())
            .ok_or_else(|| Error::Message("outside the file".into()))?;
        Ok(bytes[start..bytes.len().min(start + CHUNK_BYTES)].to_vec())
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
            let _held = reader.enter().expect("gate released before its deadline");
        });
        let observed = tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                {
                    let state = gate.state.lock().unwrap();
                    if state.waiting {
                        assert_eq!(state.reads, 1);
                        assert_eq!(state.finished, 0);
                        break;
                    }
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await;
        // Release before assertions, including an observation timeout. A failing
        // test must not leave the worker held for the gate's full timeout.
        gate.release();
        let completed = tokio::time::timeout(Duration::from_secs(1), read).await;
        observed.expect("held read remains observable from the test task");
        completed
            .expect("released read finishes promptly")
            .expect("reader did not panic");
        let state = gate.state.lock().unwrap();
        assert!(!state.waiting);
        assert!(!state.armed);
        assert_eq!(state.finished, 1);
    }
}
