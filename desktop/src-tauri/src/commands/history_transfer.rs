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
    // `SERIAL` is this module's serialization token for process-global semaphores: every case
    // holds it for the whole body, so the guard really does span the awaits in the async cases.
    // That is safe and intended here — a `#[tokio::test]` body awaits inside its own private
    // current-thread runtime with no second task on it, so no task can be blocked on this lock
    // on the same thread.
    #![allow(clippy::await_holding_lock)]

    use super::*;

    /// The admission limits are process-global statics, so a case that saturates one of them
    /// is only meaningful while no other case of this module holds a permit. Every case here
    /// takes this lock; nothing outside the module touches these semaphores.
    static SERIAL: Mutex<()> = Mutex::new(());

    fn serial() -> std::sync::MutexGuard<'static, ()> {
        SERIAL.lock().unwrap_or_else(|poison| poison.into_inner())
    }

    /// A real Tauri mock window: the commands take one, and its label — not a caller-supplied
    /// string — is the ownership key of a published snapshot.
    fn mock_window(
        label: &str,
    ) -> (
        tauri::App<tauri::test::MockRuntime>,
        tauri::WebviewWindow<tauri::test::MockRuntime>,
    ) {
        let app = tauri::test::mock_builder()
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .expect("build mock app");
        let window = tauri::WebviewWindowBuilder::new(&app, label, Default::default())
            .build()
            .expect("build webview");
        (app, window)
    }

    /// The bytes one command reply carried, which is what the frontend receives instead of a
    /// whole history graph.
    fn reply_bytes(response: tauri::ipc::Response) -> Vec<u8> {
        use tauri::ipc::IpcResponse;
        match response.body().expect("a body is always produced") {
            tauri::ipc::InvokeResponseBody::Raw(bytes) => bytes,
            // The adapter answers with `Response::new(Vec<u8>)`, so this arm is scaffolding
            // that cannot run; it exists because the enum has two variants.
            tauri::ipc::InvokeResponseBody::Json(text) => text.into_bytes(),
        }
    }

    /// The async wrapper's argument-deserialization error arm is attributed to the
    /// `#[tauri::command]` attribute line, not to the signature. This command's *last*
    /// argument is an `Option<String>`, which accepts a missing JSON key
    /// (`CommandItem::deserialize_option` visits `none`), so the only body it refuses is a
    /// **raw bytes** payload — measured here, not assumed: `deserialize_option` returns an
    /// error for `InvokeBody::Raw` before the key is looked up.
    #[test]
    fn the_transfer_command_refuses_a_raw_bytes_payload() {
        use tauri::ipc::{CallbackFn, InvokeBody};
        use tauri::test::{get_ipc_response, INVOKE_KEY};
        use tauri::webview::InvokeRequest;
        let app = tauri::test::mock_builder()
            .invoke_handler(tauri::generate_handler![begin_session_history_transfer])
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .expect("build mock app");
        let window = tauri::WebviewWindowBuilder::new(&app, "main", Default::default())
            .build()
            .expect("build webview");
        let response = get_ipc_response(
            &window,
            InvokeRequest {
                cmd: "begin_session_history_transfer".into(),
                callback: CallbackFn(0),
                error: CallbackFn(1),
                url: "tauri://localhost".parse().expect("url"),
                body: InvokeBody::Raw(Vec::new()),
                headers: Default::default(),
                invoke_key: INVOKE_KEY.to_string(),
            },
        );
        let error = response.expect_err("a raw payload is not a history request");
        assert!(
            error.as_str().unwrap_or_default().contains("bytes payload"),
            "the refusal must come from argument deserialization, not from an unregistered \
             command: {error}"
        );
    }

    /// The refusal message of a result whose success type has no `Debug` (`Response`, `Lease`,
    /// `HistoryTransfer`): `expect_err` cannot be used on those, and the message is the
    /// contract the frontend matches on. A call that was *accepted* yields `""`, so the
    /// assertion below still fails and shows the difference.
    fn refusal<T>(result: Result<T, crate::AppError>) -> String {
        result
            .err()
            .map(|error| error.to_string())
            .unwrap_or_default()
    }

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
        let _serial = serial();
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
        let _serial = serial();
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
        // `serde_json` flushes the writer it is handed, and the adapter's writer must accept
        // that without weakening the cap: a flush is not a write.
        let before = writer.0.len();
        writer.flush().unwrap();
        assert_eq!(writer.0.len(), before, "flush appended nothing");
    }

    #[tokio::test]
    async fn remote_replies_keep_small_values_inline_and_page_large_values() {
        let _serial = serial();
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

    /// Admission is two limits, not one: `ADMISSION` bounds how many transfers may exist at
    /// once and `ACTIVE` how many may *encode* at once. A lease holds one permit of each for
    /// exactly as long as the snapshot lives, so both are measured before and after.
    #[tokio::test]
    async fn a_lease_holds_one_active_and_one_queue_permit_until_it_is_dropped() {
        let _serial = serial();
        let active = ACTIVE.clone().available_permits();
        let admission = ADMISSION.clone().available_permits();
        let lease = admit().await.expect("an idle process admits one transfer");
        assert_eq!(ACTIVE.clone().available_permits(), active - 1);
        assert_eq!(ADMISSION.clone().available_permits(), admission - 1);
        drop(lease);
        assert_eq!(ACTIVE.clone().available_permits(), active);
        assert_eq!(ADMISSION.clone().available_permits(), admission);
    }

    /// A full queue is refused *before* any history is read: the refusal is the queue, not a
    /// failed query that happens to look like one.
    #[tokio::test]
    async fn a_full_queue_is_refused_without_touching_the_history() {
        let _serial = serial();
        let held: Vec<_> =
            std::iter::from_fn(|| ADMISSION.clone().try_acquire_owned().ok()).collect();
        assert_eq!(
            ADMISSION.clone().available_permits(),
            0,
            "the case must own every admission permit to mean anything"
        );
        assert_eq!(refusal(admit().await), "history_transfer_queue_full");
        drop(held);
        admit()
            .await
            .expect("the queue recovers when the permits are released");
    }

    /// Both encoding slots busy is a *timeout*, not a queue refusal: the transfer was admitted
    /// and only the encoder was busy. Paused time makes the five-second deadline immediate, so
    /// the arm costs nothing to measure.
    #[tokio::test(start_paused = true)]
    async fn waiting_for_an_encoding_slot_expires_after_its_deadline() {
        let _serial = serial();
        let held: Vec<_> = (0..2)
            .map(|index| {
                ACTIVE
                    .clone()
                    .try_acquire_owned()
                    .unwrap_or_else(|_| panic!("active permit {index} must be free"))
            })
            .collect();
        assert_eq!(refusal(admit().await), "history_transfer_admission_timeout");
        drop(held);
    }

    /// The read surface a webview actually calls: chunk by chunk, addressed by offset, readable
    /// only by the window the snapshot was published to, and gone once fully delivered.
    #[tokio::test]
    async fn the_read_command_pages_a_snapshot_for_its_own_window_only() {
        let _serial = serial();
        let (_app, window) = mock_window("main");
        let payload = serde_json::json!({"entries": [], "text": "中🙂".repeat(12000)});
        let bytes = serde_json::to_vec(&payload).unwrap();
        assert!(bytes.len() > CHUNK_BYTES, "the case needs two chunks");
        let transfer = publish(
            window.label().to_owned(),
            bytes.clone(),
            admit().await.expect("one transfer is admitted"),
        );
        assert_eq!(transfer.total_bytes, bytes.len());
        assert_eq!(transfer.chunk_bytes, CHUNK_BYTES);

        let first = reply_bytes(
            read_session_history_transfer(window.clone(), transfer.id.clone(), 0)
                .expect("the first chunk is served"),
        );
        assert_eq!(first, bytes[..CHUNK_BYTES].to_vec());
        let rest = reply_bytes(
            read_session_history_transfer(window.clone(), transfer.id.clone(), CHUNK_BYTES)
                .expect("the last chunk is served"),
        );
        assert_eq!(rest, bytes[CHUNK_BYTES..].to_vec());
        assert_eq!(
            refusal(read_session_history_transfer(
                window.clone(),
                transfer.id.clone(),
                0
            )),
            "history_transfer_expired"
        );

        // A second snapshot: unaligned offsets are refused, and another window never sees it.
        let transfer = publish(
            window.label().to_owned(),
            bytes,
            admit().await.expect("a transfer is admitted"),
        );
        assert_eq!(
            refusal(read_session_history_transfer(
                window.clone(),
                transfer.id.clone(),
                1
            )),
            "history_transfer_invalid_offset"
        );
        let (_other_app, other) = mock_window("other-window");
        assert_eq!(
            refusal(read_session_history_transfer(other, transfer.id.clone(), 0)),
            "history_transfer_owner_mismatch"
        );
        release_session_history_transfer(window, transfer.id).expect("the owner releases it");
    }

    /// Releasing is scoped to the owning window too, and releasing is how a page abandonment
    /// gives its permits back instead of waiting out the TTL.
    #[tokio::test]
    async fn the_release_command_drops_only_the_callers_own_snapshot() {
        let _serial = serial();
        let (_app, window) = mock_window("main");
        let foreign = publish(
            "another-window".into(),
            b"payload".to_vec(),
            admit().await.expect("a transfer is admitted"),
        );
        assert_eq!(
            refusal(release_session_history_transfer(
                window.clone(),
                foreign.id.clone()
            )),
            "history_transfer_owner_mismatch"
        );
        assert!(
            chunk(window.label(), &foreign.id, 0).is_err(),
            "the refused release must not have removed it for its owner"
        );
        // Retired here rather than left for the TTL: a cached snapshot holds an encoding permit
        // for as long as it lives, and this module's later cases measure those permits.
        chunk("another-window", &foreign.id, 0).expect("its owner may still take it");

        let active = ACTIVE.clone().available_permits();
        let mine = publish(
            window.label().to_owned(),
            b"payload".to_vec(),
            admit().await.expect("a transfer is admitted"),
        );
        release_session_history_transfer(window.clone(), mine.id.clone())
            .expect("the owner releases");
        assert_eq!(ACTIVE.clone().available_permits(), active);
        assert_eq!(
            refusal(read_session_history_transfer(window, mine.id, 0)),
            "history_transfer_expired"
        );
    }

    /// The command the history panel calls: it reads one page through the host, publishes the
    /// encoded snapshot, and hands back only its descriptor — never the graph through IPC.
    #[tokio::test]
    async fn beginning_a_transfer_pages_the_history_and_publishes_a_descriptor() {
        let _serial = serial();
        let _home = crate::auth_store::test_support::HomeGuard::new("history-transfer-begin");
        crate::store::initialize_app_store().expect("init store");
        let thread = crate::store::create_thread(crate::store::CreateThreadInput {
            mode: "chat".into(),
            title: Some("Chat".into()),
            workspace_id: None,
            workspace_path: None,
            workspace_name: None,
            agent_session_id: None,
        })
        .expect("create chat thread");
        let (_app, window) = mock_window("main");

        let transfer = begin_session_history_transfer(window.clone(), thread.id, None, 20, None)
            .await
            .expect("a page of history is published");
        let mut bytes = Vec::new();
        for offset in (0..transfer.total_bytes).step_by(CHUNK_BYTES) {
            bytes.extend(reply_bytes(
                read_session_history_transfer(window.clone(), transfer.id.clone(), offset)
                    .expect("each chunk is served in order"),
            ));
        }
        let page: serde_json::Value = serde_json::from_slice(&bytes).expect("the snapshot is JSON");
        assert_eq!(
            page["entries"],
            serde_json::json!([]),
            "a chat thread has no agent history"
        );
        assert_eq!(page["hasMore"], false);

        // A failed page refuses *and* returns its permits: a thread that does not exist must
        // not consume one of the two encoding slots.
        let active = ACTIVE.clone().available_permits();
        let refused = refusal(
            begin_session_history_transfer(window, "missing-thread".into(), None, 20, None).await,
        );
        assert!(
            refused.contains("Thread not found"),
            "the refusal names the thread, got {refused}"
        );
        assert_eq!(
            ACTIVE.clone().available_permits(),
            active,
            "a refused transfer holds no encoding slot"
        );
    }
}
