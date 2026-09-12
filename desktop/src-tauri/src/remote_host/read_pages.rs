//! Lossless transport pages for oversized history/replay results. Snapshots are
//! immutable and scoped to the authenticated bridge/session/run, never refetched
//! from a moving Agent projection while a phone is paging them.
use crate::remote::{protocol::IncomingCmd, services::ReplySink};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use futures::future::BoxFuture;
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    sync::{LazyLock, Mutex},
    time::{Duration, Instant},
};

const PAGE_THRESHOLD: usize = 512 * 1024;
const CHUNK_BYTES: usize = 192 * 1024;
const MAX_SNAPSHOT_BYTES: usize = 16 * 1024 * 1024;
const MAX_CACHE_BYTES: usize = 32 * 1024 * 1024;
const TTL: Duration = Duration::from_secs(120);

#[derive(Clone, PartialEq, Eq)]
struct Owner {
    bridge: String,
    session: String,
    run: String,
}
impl From<&IncomingCmd> for Owner {
    fn from(cmd: &IncomingCmd) -> Self {
        Self {
            bridge: cmd.bridge_instance_id.clone(),
            session: cmd.session_id.clone(),
            run: cmd.run_id.clone(),
        }
    }
}
struct Snapshot {
    owner: Owner,
    bytes: Vec<u8>,
    created: Instant,
}
#[derive(Default)]
struct Cache {
    snapshots: HashMap<String, Snapshot>,
}
impl Cache {
    fn prune(&mut self) {
        self.snapshots
            .retain(|_, value| value.created.elapsed() < TTL);
    }
    fn insert(&mut self, owner: Owner, bytes: Vec<u8>) -> Result<Value, String> {
        if bytes.len() > MAX_SNAPSHOT_BYTES {
            return Err("remote_read_too_large".into());
        }
        self.prune();
        while self
            .snapshots
            .values()
            .map(|snapshot| snapshot.bytes.len())
            .sum::<usize>()
            + bytes.len()
            > MAX_CACHE_BYTES
        {
            let oldest = self
                .snapshots
                .iter()
                .min_by_key(|(_, value)| value.created)
                .map(|(key, _)| key.clone());
            if let Some(key) = oldest {
                self.snapshots.remove(&key);
            } else {
                break;
            }
        }
        let id = format!("read_{}", nkeys::KeyPair::new_user().public_key());
        let snapshot = Snapshot {
            owner,
            bytes,
            created: Instant::now(),
        };
        let page = chunk(&id, &snapshot, 0)?;
        self.snapshots.insert(id, snapshot);
        Ok(page)
    }
    fn read(&mut self, cmd: &IncomingCmd) -> Result<Value, String> {
        self.prune();
        let snapshot = self
            .snapshots
            .get(&cmd.reply_id)
            .ok_or("remote_read_expired")?;
        if snapshot.owner != Owner::from(cmd) {
            return Err("remote_read_owner_mismatch".into());
        }
        let offset = usize::try_from(cmd.offset).map_err(|_| "remote_read_invalid_offset")?;
        chunk(&cmd.reply_id, snapshot, offset)
    }
}
fn chunk(id: &str, snapshot: &Snapshot, offset: usize) -> Result<Value, String> {
    if offset >= snapshot.bytes.len() || !offset.is_multiple_of(CHUNK_BYTES) {
        return Err("remote_read_invalid_offset".into());
    }
    let end = offset.saturating_add(CHUNK_BYTES).min(snapshot.bytes.len());
    Ok(json!({"readChunk": {
        "id": id, "offset": offset, "nextOffset": end,
        "totalBytes": snapshot.bytes.len(),
        "data": URL_SAFE_NO_PAD.encode(&snapshot.bytes[offset..end])
    }}))
}
static CACHE: LazyLock<Mutex<Cache>> = LazyLock::new(Default::default);
pub(super) fn prune() {
    CACHE.lock().unwrap().prune();
}
pub(super) fn clear() {
    CACHE.lock().unwrap().snapshots.clear();
}
pub(super) fn read(cmd: &IncomingCmd) -> Result<Value, String> {
    CACHE.lock().unwrap().read(cmd)
}

pub(super) struct PagedReply<'a> {
    sink: &'a dyn ReplySink,
    owner: Owner,
    enabled: bool,
}
impl<'a> PagedReply<'a> {
    pub fn new(cmd: &IncomingCmd, sink: &'a dyn ReplySink) -> Self {
        Self {
            sink,
            owner: Owner::from(cmd),
            enabled: cmd.chunked_read
                && matches!(
                    cmd.cmd_type.as_str(),
                    "get_session_entries" | "get_events_since" | "get_messages"
                ),
        }
    }
}
impl ReplySink for PagedReply<'_> {
    fn send<'a>(&'a self, success: bool, data: Value, error: Option<String>) -> BoxFuture<'a, ()> {
        Box::pin(async move {
            if success && self.enabled {
                let bytes = serde_json::to_vec(&data).expect("response Value serializes");
                if bytes.len() > PAGE_THRESHOLD {
                    let page = CACHE.lock().unwrap().insert(self.owner.clone(), bytes);
                    match page {
                        Ok(page) => self.sink.send(true, page, None).await,
                        Err(error) => self.sink.send(false, Value::Null, Some(error)).await,
                    }
                    return;
                }
            }
            self.sink.send(success, data, error).await;
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn command() -> IncomingCmd {
        IncomingCmd {
            bridge_instance_id: "bridge".into(),
            session_id: "session".into(),
            run_id: "run".into(),
            ..Default::default()
        }
    }

    #[test]
    fn oversized_projection_roundtrips_losslessly_with_small_immutable_pages() {
        let mut cache = Cache::default();
        let mut cmd = command();
        let value = json!({"projection": {"cursor": 123, "events": [{"data": "中文\\\"".repeat(400_000)}]}, "events": []});
        let bytes = serde_json::to_vec(&value).unwrap();
        let first = cache.insert(Owner::from(&cmd), bytes.clone()).unwrap();
        cmd.reply_id = first["readChunk"]["id"].as_str().unwrap().into();
        let mut page = first;
        let mut restored = Vec::new();
        loop {
            assert!(
                serde_json::to_vec(&json!({"success": true, "data": page}))
                    .unwrap()
                    .len()
                    < PAGE_THRESHOLD
            );
            let part = &page["readChunk"];
            restored.extend(
                URL_SAFE_NO_PAD
                    .decode(part["data"].as_str().unwrap())
                    .unwrap(),
            );
            let next = part["nextOffset"].as_i64().unwrap();
            if next as usize == bytes.len() {
                break;
            }
            cmd.offset = next;
            page = cache.read(&cmd).unwrap();
        }
        assert_eq!(restored, bytes);
        assert_eq!(serde_json::from_slice::<Value>(&restored).unwrap(), value);
    }

    #[test]
    fn snapshot_owner_offset_expiry_and_capacity_are_bounded() {
        let mut cache = Cache::default();
        let mut cmd = command();
        let page = cache
            .insert(Owner::from(&cmd), vec![0; MAX_SNAPSHOT_BYTES])
            .unwrap();
        cmd.reply_id = page["readChunk"]["id"].as_str().unwrap().into();
        cmd.offset = 1;
        assert_eq!(cache.read(&cmd).unwrap_err(), "remote_read_invalid_offset");
        cmd.offset = -1;
        assert_eq!(cache.read(&cmd).unwrap_err(), "remote_read_invalid_offset");
        cmd.offset = 0;
        cmd.bridge_instance_id = "other".into();
        assert_eq!(cache.read(&cmd).unwrap_err(), "remote_read_owner_mismatch");
        cmd.bridge_instance_id = "bridge".into();
        cache.snapshots.get_mut(&cmd.reply_id).unwrap().created = Instant::now() - TTL;
        assert_eq!(cache.read(&cmd).unwrap_err(), "remote_read_expired");
        for _ in 0..3 {
            cache
                .insert(Owner::from(&cmd), vec![0; MAX_SNAPSHOT_BYTES])
                .unwrap();
        }
        assert_eq!(cache.snapshots.len(), 2);
        assert_eq!(
            cache
                .insert(Owner::from(&cmd), vec![0; MAX_SNAPSHOT_BYTES + 1])
                .unwrap_err(),
            "remote_read_too_large"
        );
    }
}
