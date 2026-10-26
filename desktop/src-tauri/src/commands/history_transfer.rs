//! Bounded, disposable UI transport snapshots. SQLite remains authoritative.
//! Every IPC body is small, including WebView's JSON fallback transport.
use std::{
    collections::HashMap,
    io::Write,
    sync::{Arc, LazyLock, Mutex},
    time::{Duration, Instant},
};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

const CHUNK_BYTES: usize = 64 * 1024;
const MAX_BYTES: usize = 16 * 1024 * 1024;
const TTL: Duration = Duration::from_secs(120);
static ACTIVE: LazyLock<Arc<Semaphore>> = LazyLock::new(|| Arc::new(Semaphore::new(2)));
static ADMISSION: LazyLock<Arc<Semaphore>> = LazyLock::new(|| Arc::new(Semaphore::new(34)));
static CACHE: LazyLock<Mutex<HashMap<String, Snapshot>>> = LazyLock::new(Default::default);

pub(super) struct Lease {
    _admission: OwnedSemaphorePermit,
    _active: OwnedSemaphorePermit,
}
struct Snapshot {
    owner: String,
    bytes: Vec<u8>,
    created: Instant,
    _lease: Lease,
}
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryTransfer {
    id: String,
    total_bytes: usize,
    chunk_bytes: usize,
}
struct BoundedJson(Vec<u8>);
impl Write for BoundedJson {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if self.0.len().saturating_add(bytes.len()) > MAX_BYTES {
            return Err(std::io::Error::other("history_materialization_too_large"));
        }
        self.0.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

pub(super) async fn admit() -> Result<Lease, crate::AppError> {
    // Pruning drops the permits with expired snapshots before new admission.
    CACHE
        .lock()
        .unwrap()
        .retain(|_, snapshot| snapshot.created.elapsed() < TTL);
    let admission = ADMISSION
        .clone()
        .try_acquire_owned()
        .map_err(|_| "history_transfer_queue_full")?;
    let active = tokio::time::timeout(Duration::from_secs(5), ACTIVE.clone().acquire_owned())
        .await
        .map_err(|_| "history_transfer_admission_timeout")?
        .map_err(|_| "history_transfer_unavailable")?;
    Ok(Lease {
        _admission: admission,
        _active: active,
    })
}

#[tauri::command]
pub async fn begin_session_history_transfer<R: tauri::Runtime>(
    window: tauri::WebviewWindow<R>,
    thread_id: String,
    before: Option<i64>,
    limit: i64,
    history_source: Option<String>,
) -> Result<HistoryTransfer, crate::AppError> {
    let lease = admit().await?;
    let value = future_host::application::threads::get_session_entries_page(
        thread_id,
        before,
        limit,
        history_source,
    )
    .await?;
    // A started job retains its lease even if the command future is canceled.
    let (bytes, lease) = tokio::task::spawn_blocking(move || {
        let mut bytes = BoundedJson(Vec::new());
        serde_json::to_writer(&mut bytes, &value)?;
        Ok::<_, crate::AppError>((bytes.0, lease))
    })
    .await
    .map_err(|error| format!("history transfer preparation failed: {error}"))??;
    Ok(publish(window.label().to_owned(), bytes, lease))
}

fn publish(owner: String, bytes: Vec<u8>, lease: Lease) -> HistoryTransfer {
    // Publish only to a live command owner. A canceled blocking result drops
    // its bytes and permits rather than leaving an unreachable snapshot.
    let id = format!("history_{}", nkeys::KeyPair::new_user().public_key());
    let total_bytes = bytes.len();
    CACHE.lock().unwrap().insert(
        id.clone(),
        Snapshot {
            owner,
            bytes,
            created: Instant::now(),
            _lease: lease,
        },
    );
    HistoryTransfer {
        id,
        total_bytes,
        chunk_bytes: CHUNK_BYTES,
    }
}

/// Small remote replies retain the existing command shape. Large read replies
/// return an adapter-owned descriptor, never the complete graph through IPC.
pub(super) async fn remote_reply(
    owner: String,
    value: serde_json::Value,
    lease: Lease,
) -> Result<serde_json::Value, crate::AppError> {
    enum Prepared {
        Inline(serde_json::Value),
        Transfer(Vec<u8>, Lease),
    }
    let prepared = tokio::task::spawn_blocking(move || {
        let mut bytes = BoundedJson(Vec::new());
        serde_json::to_writer(&mut bytes, &value)?;
        if bytes.0.len() <= 32 * 1024 {
            Ok::<_, crate::AppError>(Prepared::Inline(value))
        } else {
            drop(value);
            Ok(Prepared::Transfer(bytes.0, lease))
        }
    })
    .await
    .map_err(|error| format!("remote read transfer preparation failed: {error}"))??;
    Ok(match prepared {
        Prepared::Inline(value) => value,
        Prepared::Transfer(bytes, lease) => {
            serde_json::json!({"nativeReadTransfer":publish(owner, bytes, lease)})
        }
    })
}

fn chunk(owner: &str, id: &str, offset: usize) -> Result<Vec<u8>, crate::AppError> {
    let mut cache = CACHE.lock().unwrap();
    read_chunk(&mut cache, owner, id, offset)
}

fn read_chunk(
    cache: &mut HashMap<String, Snapshot>,
    owner: &str,
    id: &str,
    offset: usize,
) -> Result<Vec<u8>, crate::AppError> {
    cache.retain(|_, snapshot| snapshot.created.elapsed() < TTL);
    let snapshot = cache.get(id).ok_or("history_transfer_expired")?;
    if snapshot.owner != owner {
        return Err("history_transfer_owner_mismatch".into());
    }
    if offset >= snapshot.bytes.len() || !offset.is_multiple_of(CHUNK_BYTES) {
        return Err("history_transfer_invalid_offset".into());
    }
    let end = (offset + CHUNK_BYTES).min(snapshot.bytes.len());
    let bytes = snapshot.bytes[offset..end].to_vec();
    if end == snapshot.bytes.len() {
        cache.remove(id);
    }
    Ok(bytes)
}

#[tauri::command]
pub fn read_session_history_transfer<R: tauri::Runtime>(
    window: tauri::WebviewWindow<R>,
    transfer_id: String,
    offset: usize,
) -> Result<tauri::ipc::Response, crate::AppError> {
    Ok(tauri::ipc::Response::new(chunk(
        window.label(),
        &transfer_id,
        offset,
    )?))
}

#[tauri::command]
pub fn release_session_history_transfer<R: tauri::Runtime>(
    window: tauri::WebviewWindow<R>,
    transfer_id: String,
) -> Result<(), crate::AppError> {
    let mut cache = CACHE.lock().unwrap();
    if cache
        .get(&transfer_id)
        .is_some_and(|snapshot| snapshot.owner != window.label())
    {
        return Err("history_transfer_owner_mismatch".into());
    }
    cache.remove(&transfer_id);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snapshot(bytes: Vec<u8>) -> Snapshot {
        let admission = Arc::new(Semaphore::new(1)).try_acquire_owned().unwrap();
        let active = Arc::new(Semaphore::new(1)).try_acquire_owned().unwrap();
        Snapshot {
            owner: "window".into(),
            bytes,
            created: Instant::now(),
            _lease: Lease {
                _admission: admission,
                _active: active,
            },
        }
    }

    #[test]
    fn chunks_are_immutable_bounded_and_retired_on_completion() {
        let text = serde_json::json!({"text": "中🙂".repeat(30000)}).to_string();
        let original = text.into_bytes();
        let mut cache = HashMap::from([("id".into(), snapshot(original.clone()))]);
        assert!(read_chunk(&mut cache, "other-window", "id", 0).is_err());
        assert!(read_chunk(&mut cache, "window", "id", 1).is_err());
        assert!(read_chunk(&mut cache, "window", "id", usize::MAX).is_err());
        let mut result = Vec::new();
        for offset in (0..original.len()).step_by(CHUNK_BYTES) {
            let bytes = read_chunk(&mut cache, "window", "id", offset).unwrap();
            assert!(bytes.len() <= CHUNK_BYTES);
            result.extend(bytes);
        }
        assert_eq!(result, original);
        assert!(cache.is_empty());
        assert!(read_chunk(&mut cache, "window", "id", 0).is_err());
    }

    #[test]
    fn expired_snapshots_release_admission_without_sending_bytes() {
        let mut expired = snapshot(vec![1, 2]);
        expired.created = Instant::now() - TTL;
        let active = expired._lease._active.semaphore().clone();
        assert_eq!(active.available_permits(), 0);
        let mut cache = HashMap::from([("id".into(), expired)]);
        assert!(read_chunk(&mut cache, "window", "id", 0).is_err());
        assert!(cache.is_empty());
        assert_eq!(active.available_permits(), 1);
    }

    #[test]
    fn encoding_refuses_over_budget_before_appending_that_write() {
        let mut writer = BoundedJson(Vec::new());
        writer.write_all(&vec![b' '; MAX_BYTES]).unwrap();
        assert!(writer.write_all(b"x").is_err());
        assert_eq!(writer.0.len(), MAX_BYTES);
        let mut writer = BoundedJson(Vec::new());
        assert!(serde_json::to_writer(&mut writer, &"x".repeat(MAX_BYTES)).is_err());
        assert!(writer.0.len() <= MAX_BYTES);
    }

    #[tokio::test]
    async fn remote_replies_keep_small_values_inline_and_page_large_values() {
        let small = serde_json::json!({"entries":[],"hasMore":false});
        let lease = snapshot(Vec::new())._lease;
        assert_eq!(
            remote_reply("test-window".into(), small.clone(), lease)
                .await
                .unwrap(),
            small
        );
        let large = serde_json::json!({"events":[{"data":"中🙂".repeat(30000)}],"watermark":5});
        let lease = snapshot(Vec::new())._lease;
        let permits = lease._active.semaphore().clone();
        let response = remote_reply("test-window".into(), large.clone(), lease)
            .await
            .unwrap();
        let transfer = &response["nativeReadTransfer"];
        let id = transfer["id"].as_str().unwrap();
        let total = transfer["totalBytes"].as_u64().unwrap() as usize;
        assert_eq!(permits.available_permits(), 0);
        assert!(chunk("another-window", id, 0).is_err());
        let mut bytes = Vec::new();
        for offset in (0..total).step_by(CHUNK_BYTES) {
            bytes.extend(chunk("test-window", id, offset).unwrap());
        }
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&bytes).unwrap(),
            large
        );
        assert_eq!(permits.available_permits(), 1);
    }
}
