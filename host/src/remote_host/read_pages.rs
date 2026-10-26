//! Lossless transport pages for oversized history/replay results. Snapshots are
//! immutable and scoped to the authenticated bridge/session/run, never refetched
//! from a moving Agent projection while a phone is paging them.
use future_remote_protocol::IncomingCmd;
use future_remote_server::services::ReplySink;
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
    fn insert(&mut self, owner: Owner, bytes: Vec<u8>) -> Result<Page, String> {
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
    fn read(&mut self, cmd: &IncomingCmd) -> Result<Page, String> {
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
fn chunk(id: &str, snapshot: &Snapshot, offset: usize) -> Result<Page, String> {
    if offset >= snapshot.bytes.len() || !offset.is_multiple_of(CHUNK_BYTES) {
        return Err("remote_read_invalid_offset".into());
    }
    let end = offset.saturating_add(CHUNK_BYTES).min(snapshot.bytes.len());
    Ok(Page {
        // The address of the chunk, and the snapshot facts the reader pins from
        // the first reply. The chunk's bytes are *not* in here: whether they are
        // base64 inside this JSON or raw behind it is what the connection
        // negotiated, and that is the sink's decision, not the page's.
        header: json!({"readChunk": {
            "id": id, "offset": offset, "nextOffset": end,
            "totalBytes": snapshot.bytes.len(),
        }}),
        body: snapshot.bytes[offset..end].to_vec(),
    })
}
/// One page of a snapshot: the JSON that addresses it, and its bytes.
#[derive(Debug)]
pub(super) struct Page {
    pub(super) header: Value,
    pub(super) body: Vec<u8>,
}
static CACHE: LazyLock<Mutex<Cache>> = LazyLock::new(Default::default);
pub(super) fn prune() {
    CACHE.lock().unwrap().prune();
}
pub(super) fn clear() {
    CACHE.lock().unwrap().snapshots.clear();
}
pub(super) fn read(cmd: &IncomingCmd) -> Result<Page, String> {
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
                    "get_session_entries"
                        | "get_events_since"
                        | "get_messages"
                        | "list_session_files"
                        | "list_settings_models"
                        | "list_providers"
                        | "list_available_skills"
                        | "list_skills"
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
                        Ok(page) => self.sink.send_binary(page.header, page.body).await,
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
    use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
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
        cmd.reply_id = first.header["readChunk"]["id"].as_str().unwrap().into();
        let mut page = first;
        let mut restored = Vec::new();
        loop {
            assert!(
                serde_json::to_vec(&json!({"success": true, "data": page.header}))
                    .unwrap()
                    .len()
                    < PAGE_THRESHOLD
            );
            restored.extend_from_slice(&page.body);
            let next = page.header["readChunk"]["nextOffset"].as_i64().unwrap();
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
        cmd.reply_id = page.header["readChunk"]["id"].as_str().unwrap().into();
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

    /// Pagination is enabled by the *client's* declaration plus the command
    /// name, and by nothing else. A phone that never declared it must keep
    /// receiving plain replies for every one of these commands, or it would get
    /// a `readChunk` envelope it cannot decode.
    #[test]
    fn every_pageable_command_family_needs_both_the_declaration_and_its_name() {
        let sink = crate::remote::test_support::RecordingSink::default();
        for cmd_type in [
            "get_session_entries",
            "get_events_since",
            "get_messages",
            "list_session_files",
            "list_settings_models",
            "list_providers",
            "list_available_skills",
            "list_skills",
        ] {
            let cmd = IncomingCmd {
                cmd_type: cmd_type.into(),
                chunked_read: true,
                ..command()
            };
            assert!(PagedReply::new(&cmd, &sink).enabled, "{cmd_type}");
            let undeclared = IncomingCmd {
                cmd_type: cmd_type.into(),
                ..command()
            };
            assert!(!PagedReply::new(&undeclared, &sink).enabled, "{cmd_type}");
        }
        // A command outside the pageable set stays plain even when the client
        // declares the capability.
        let cmd = IncomingCmd {
            cmd_type: "get_desktop_settings".into(),
            chunked_read: true,
            ..command()
        };
        assert!(!PagedReply::new(&cmd, &sink).enabled);
    }

    /// A large reply is replaced by a snapshot page whose first chunk is what
    /// the client asked for. This is the `Ok` arm of the transport: without it
    /// the phone would receive an event too big for the broker payload limit.
    #[tokio::test]
    async fn a_large_reply_is_replaced_by_a_page_that_starts_at_offset_zero() {
        let inner = crate::remote::test_support::RecordingSink::default();
        let cmd = IncomingCmd {
            cmd_type: "list_providers".into(),
            chunked_read: true,
            ..command()
        };
        let payload = json!({ "blob": "x".repeat(PAGE_THRESHOLD + 1) });
        let bytes = serde_json::to_vec(&payload).unwrap();
        PagedReply::new(&cmd, &inner)
            .send(true, payload, None)
            .await;

        let (success, page, error) = inner.only();
        assert!(success, "{error:?}");
        let chunk = &page["readChunk"];
        assert!(
            !chunk["id"].as_str().unwrap().is_empty(),
            "a page is addressable by the client"
        );
        assert_eq!(chunk["totalBytes"], json!(bytes.len()));
        assert_eq!(chunk["offset"], json!(0));
        assert_eq!(chunk["nextOffset"], json!(CHUNK_BYTES));
        // The chunk is the raw reply bytes, so the client reassembles exactly
        // what the unpaged path would have sent.
        let decoded = URL_SAFE_NO_PAD
            .decode(chunk["data"].as_str().unwrap())
            .unwrap();
        assert_eq!(decoded, bytes[..CHUNK_BYTES]);
        clear();
    }

    /// The page reaches the sink with its bytes *apart* from its address, which
    /// is what lets one sink base64 them and another frame them. The address
    /// itself carries no `data` key: a JSON carrier is the sink's decision, so
    /// the page must not pre-decide it by putting an empty one there.
    #[tokio::test]
    async fn a_page_hands_its_bytes_to_the_sink_outside_the_json() {
        let sink = BinaryRecordingSink::default();
        let cmd = IncomingCmd {
            cmd_type: "list_providers".into(),
            chunked_read: true,
            ..command()
        };
        let payload = json!({ "blob": "x".repeat(PAGE_THRESHOLD + 1) });
        let bytes = serde_json::to_vec(&payload).unwrap();
        PagedReply::new(&cmd, &sink).send(true, payload, None).await;
        let (header, body) = sink.only();
        let chunk = &header["readChunk"];
        assert!(chunk.get("data").is_none(), "the carrier is the sink's");
        assert_eq!(chunk["offset"], json!(0));
        assert_eq!(chunk["nextOffset"], json!(CHUNK_BYTES));
        assert_eq!(body, bytes[..CHUNK_BYTES]);
        clear();
    }

    /// A sink that overrides nothing still produces the legacy page: the trait's
    /// default carrier is base64 inside the same JSON key, so an undeclared
    /// client sees no difference at all.
    #[tokio::test]
    async fn the_default_carrier_is_the_legacy_base64_page() {
        let header = json!({ "readChunk": {
            "id": "read_a", "offset": 0, "nextOffset": 3, "totalBytes": 3
        }});
        let inner = crate::remote::test_support::RecordingSink::default();
        inner.send_binary(header, vec![0xfb, 0xff, 0x00]).await;
        let (success, page, error) = inner.only();
        assert!(success, "{error:?}");
        assert_eq!(page["readChunk"]["data"], json!("-_8A"));
    }

    #[derive(Default)]
    struct BinaryRecordingSink {
        captured: Mutex<Option<(Value, Vec<u8>)>>,
    }
    impl BinaryRecordingSink {
        fn only(&self) -> (Value, Vec<u8>) {
            self.captured
                .lock()
                .unwrap()
                .take()
                .expect("one page was sent")
        }
    }
    impl ReplySink for BinaryRecordingSink {
        fn send<'a>(
            &'a self,
            _success: bool,
            _data: Value,
            _error: Option<String>,
        ) -> BoxFuture<'a, ()> {
            Box::pin(async move { panic!("a page must use the binary carrier") })
        }
        fn send_binary<'a>(&'a self, header: Value, body: Vec<u8>) -> BoxFuture<'a, ()> {
            Box::pin(async move {
                *self.captured.lock().unwrap() = Some((header, body));
            })
        }
    }

    /// A reply too large to snapshot is refused with an error rather than
    /// silently sent whole: a partial page the phone cannot detect would look
    /// like a complete answer.
    #[tokio::test]
    async fn a_reply_above_the_snapshot_bound_is_refused_not_truncated() {
        let inner = crate::remote::test_support::RecordingSink::default();
        let cmd = IncomingCmd {
            cmd_type: "list_skills".into(),
            chunked_read: true,
            ..command()
        };
        PagedReply::new(&cmd, &inner)
            .send(
                true,
                json!({ "blob": "x".repeat(MAX_SNAPSHOT_BYTES + 1) }),
                None,
            )
            .await;
        let (success, data, error) = inner.only();
        assert!(!success);
        assert_eq!(data, Value::Null);
        assert_eq!(error.as_deref(), Some("remote_read_too_large"));
    }

    /// A reply below the page threshold is passed through untouched even when
    /// the client declared the capability: paging a small answer would cost the
    /// phone a second round trip for nothing.
    #[tokio::test]
    async fn a_small_reply_is_not_paged_and_a_failure_is_not_either() {
        let inner = crate::remote::test_support::RecordingSink::default();
        let cmd = IncomingCmd {
            cmd_type: "list_providers".into(),
            chunked_read: true,
            ..command()
        };
        let paged = PagedReply::new(&cmd, &inner);
        paged.send(true, json!({ "providers": [] }), None).await;
        assert_eq!(inner.ok_data(), json!({ "providers": [] }));
        // A failure keeps the handler's own error text instead of being
        // replaced by a transport error.
        paged
            .send(false, Value::Null, Some("providers_unavailable".into()))
            .await;
        assert_eq!(inner.error_text(), "providers_unavailable");
        assert_eq!(inner.len(), 2);
    }
}
