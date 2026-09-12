//! Step C: command subscription + routing. Mobile commands arrive on
//! `p.{pairId}.cmd.>`; reads go straight to the store, prompts go through
//! `agent_bridge::headless` so the persist/finalize contract is shared with
//! the rest of the backend.

use super::protocol::IncomingCmd;
use super::services::ReplySink;
#[cfg(test)]
use crate::remote_host::business::*;
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use flate2::{write::GzEncoder, Compression};
use futures::StreamExt;
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    io::Write,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, LazyLock, Mutex,
    },
    time::Duration,
};

static COMMAND_EPISODE: LazyLock<super::FailureEpisode> = LazyLock::new(Default::default);

const REMOTE_JSON_GZIP_THRESHOLD_BYTES: usize = 32 * 1024;
const REMOTE_JSON_GZIP_ENV: &str = "FUTURE_REMOTE_JSON_GZIP";

/// Large remote JSON replies can be sent as standard gzip without changing the
/// reply envelope: mobile recognizes the gzip magic bytes and otherwise parses
/// plain JSON. The capability is deliberately OFF by default. Once history is
/// fetched in small, lazy pages, gzip's bandwidth saving is outweighed on the
/// current React Native client by JS-side gunzip + JSON decode time (about one
/// second across a long thread in the measured emulator run). Keep the wire
/// path available for high-latency/low-bandwidth deployments; opt in with
/// `FUTURE_REMOTE_JSON_GZIP=1` (also accepts true/yes/on).
fn remote_json_gzip_enabled() -> bool {
    std::env::var(REMOTE_JSON_GZIP_ENV).is_ok_and(|value| {
        matches!(
            value.trim().to_ascii_lowercase().as_str(),
            "1" | "true" | "yes" | "on"
        )
    })
}

pub(super) struct CachedReply {
    request: Value,
    response: tokio::sync::Mutex<Option<Vec<u8>>>,
}

type ReplySlot = Arc<CachedReply>;

/// Command-id → in-flight/completed response cache (single-flight). Created
/// once per bridge start and SHARED across command loops: credential refresh
/// swaps the loop every JWT TTL, and a cache local to the loop would be wiped
/// on every swap — a client retrying right after a swap would re-execute a
/// command the old loop had already run (for `prompt`, a duplicated message).
pub(super) type ReplySlots = Arc<Mutex<HashMap<String, ReplySlot>>>;

#[derive(Clone)]
pub(super) struct HandshakeState {
    host: &'static dyn super::services::BusinessHost,
    creds: crate::remote::pairing::PairingCreds,
    access_epoch: Option<u64>,
    confirmed: Arc<AtomicBool>,
    active: Arc<AtomicBool>,
    bridge_instance_id: String,
    pending: Arc<Mutex<HashMap<String, PendingHandshake>>>,
}

#[derive(Clone)]
struct PendingHandshake {
    transcript: String,
    device_id: String,
    client_public_key: String,
    created_at: std::time::Instant,
}

impl HandshakeState {
    pub(super) fn new(
        creds: crate::remote::pairing::PairingCreds,
        confirmed: Arc<AtomicBool>,
        bridge_instance_id: String,
    ) -> Self {
        Self {
            creds,
            access_epoch: None,
            host: super::host(),
            confirmed,
            active: Arc::new(AtomicBool::new(false)),
            bridge_instance_id,
            pending: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub(super) fn with_access(mut self, epoch: u64) -> Self {
        self.access_epoch = Some(epoch);
        self
    }

    fn access_current(&self) -> bool {
        self.access_epoch
            .is_none_or(|epoch| super::SUPERVISOR.access.current() == epoch)
    }

    pub(super) fn bridge_instance_id(&self) -> &str {
        &self.bridge_instance_id
    }

    pub(super) fn active_flag(&self) -> Arc<AtomicBool> {
        self.active.clone()
    }
}

pub(super) fn new_reply_slots() -> ReplySlots {
    Arc::new(Mutex::new(HashMap::new()))
}

/// How long a completed single-flight response stays cached for retrying
/// clients (matches the planned NATS duplicate window). Tests shrink it to
/// milliseconds so expiry can be observed without a ten-minute wait.
fn reply_slot_ttl() -> Duration {
    #[cfg(test)]
    const TTL: Duration = Duration::from_millis(500);
    #[cfg(not(test))]
    const TTL: Duration = Duration::from_secs(600);
    TTL
}

tokio::task_local! {
    static REPLY_CAPTURE: Arc<Mutex<Option<Vec<u8>>>>;
}

#[cfg(test)]
pub(super) async fn command_loop(
    client: async_nats::Client,
    pair_id: String,
    reply_slots: ReplySlots,
    handshake: HandshakeState,
) {
    command_loop_with_ready(client, pair_id, reply_slots, handshake, None).await;
}

pub(super) async fn command_loop_with_ready(
    client: async_nats::Client,
    pair_id: String,
    reply_slots: ReplySlots,
    handshake: HandshakeState,
    mut ready: Option<tokio::sync::oneshot::Sender<()>>,
) {
    let subject = format!("p.{pair_id}.cmd.>");
    let queue = format!("bridge.{pair_id}");
    let mut sub = match client.queue_subscribe(subject, queue).await {
        Ok(sub) => sub,
        Err(error) => {
            if let Some(line) = COMMAND_EPISODE.record("command_subscription", error) {
                eprintln!("{line}");
            }
            return;
        }
    };
    if let Some(line) = COMMAND_EPISODE.recovered() {
        eprintln!("{line}");
    }
    if let Some(sender) = ready.take() {
        let _ = sender.send(());
    }
    while let Some(msg) = sub.next().await {
        let client = client.clone();
        let reply_slots = reply_slots.clone();
        let handshake = handshake.clone();
        // Spawn per command: prevent a slow command from blocking others.
        tokio::spawn(async move {
            handle_command_singleflight(&client, msg, reply_slots, handshake).await;
        });
    }
    if let Some(line) =
        COMMAND_EPISODE.record("command_subscription", "subscription ended unexpectedly")
    {
        eprintln!("{line}");
    }
}

/// Merge concurrent/retried deliveries carrying the same command id. The first
/// delivery executes the command; followers wait for and receive the exact same
/// response bytes. Completed responses stay cached for ten minutes, matching
/// the planned NATS duplicate window, then expire without blocking unrelated ids.
async fn handle_command_singleflight(
    client: &async_nats::Client,
    msg: async_nats::Message,
    reply_slots: ReplySlots,
    handshake: HandshakeState,
) {
    if !handshake.access_current() {
        return;
    }
    let command_id = serde_json::from_slice::<IncomingCmd>(&msg.payload)
        .ok()
        .map(|cmd| cmd.id)
        .filter(|id| !id.is_empty());
    let Some(command_id) = command_id else {
        handle_command(client, msg, handshake).await;
        return;
    };

    let request: Value = serde_json::from_slice(&msg.payload).expect("command already decoded");
    let (slot, inserted) = {
        let mut slots = reply_slots.lock().unwrap();
        match slots.get(&command_id) {
            Some(slot) => (slot.clone(), false),
            None => {
                let slot = Arc::new(CachedReply {
                    request: request.clone(),
                    response: tokio::sync::Mutex::new(None),
                });
                slots.insert(command_id.clone(), slot.clone());
                (slot, true)
            }
        }
    };
    if slot.request != request {
        reply(
            client,
            &msg,
            false,
            Value::Null,
            Some("command_id_conflict"),
        )
        .await;
        return;
    }

    let mut cached = slot.response.lock().await;
    if !handshake.access_current() {
        return;
    }
    if let Some(payload) = cached.as_ref() {
        publish_reply_payload(client, &msg, payload.clone()).await;
        return;
    }

    let capture = Arc::new(Mutex::new(None));
    REPLY_CAPTURE
        .scope(capture.clone(), handle_command(client, msg, handshake))
        .await;
    *cached = capture.lock().unwrap().clone();
    if inserted {
        let slots = reply_slots.clone();
        let id = command_id.clone();
        let expected = slot.clone();
        tokio::spawn(async move {
            tokio::time::sleep(reply_slot_ttl()).await;
            let mut slots = slots.lock().unwrap();
            if slots
                .get(&id)
                .is_some_and(|current| Arc::ptr_eq(current, &expected))
            {
                slots.remove(&id);
            }
        });
    }
}

// SECURITY: NATS admits this bridge with a short-lived user JWT whose server-
// enforced ACL is scoped to this pair, and every non-handshake command is gated
// on a completed pairing handshake above. Only `approval_decision` additionally
// re-checks per-session ownership (that an approval belongs to the requesting
// session); the other handlers trust the pair-scoped ACL because the paired
// device is authorized to operate on all of this desktop's sessions.
async fn handle_command(
    client: &async_nats::Client,
    msg: async_nats::Message,
    handshake: HandshakeState,
) {
    if !handshake.access_current() {
        return;
    }
    let cmd: IncomingCmd = match serde_json::from_slice(&msg.payload) {
        Ok(cmd) => cmd,
        Err(e) => {
            reply(
                client,
                &msg,
                false,
                Value::Null,
                Some(&format!("Failed to parse command JSON: {e}")),
            )
            .await;
            return;
        }
    };

    let handshake_command = matches!(
        cmd.cmd_type.as_str(),
        "pair_handshake" | "pair_handshake_confirm"
    );
    if !handshake_command && !handshake.active.load(Ordering::Acquire) {
        reply(
            client,
            &msg,
            false,
            Value::Null,
            Some("pairing_handshake_required"),
        )
        .await;
        return;
    }

    if !handshake_command
        && !cmd.bridge_instance_id.is_empty()
        && cmd.bridge_instance_id != handshake.bridge_instance_id
    {
        reply(
            client,
            &msg,
            false,
            Value::Null,
            Some("remote_access_changed"),
        )
        .await;
        return;
    }
    match cmd.cmd_type.as_str() {
        "pair_handshake" => {
            handle_pair_handshake(client, &msg, &cmd, &handshake).await;
        }
        "pair_handshake_confirm" => {
            handle_pair_handshake_confirm(client, &msg, &cmd, &handshake).await;
        }
        // Presence is normally pushed every 20 seconds. A client that subscribes
        // after the latest heartbeat would otherwise look offline until the next
        // tick because core NATS subscriptions do not replay old messages.
        "get_presence" => {
            let pair_id = msg
                .subject
                .strip_prefix("p.")
                .and_then(|subject| subject.split('.').next())
                .unwrap_or_default();
            reply(
                client,
                &msg,
                true,
                super::build_presence_payload(pair_id, &handshake.bridge_instance_id),
                None,
            )
            .await;
        }
        // A phone-initiated unpair first reaches the desktop over the existing
        // authenticated command channel. Acknowledge before stopping the
        // bridge, then perform the destructive work in a separate task so the
        // requester can clear itself without waiting for token expiry.
        "unpair" => {
            reply(client, &msg, true, json!({}), None).await;
            tauri::async_runtime::spawn(async move {
                if !handshake.access_current() {
                    return;
                }
                if let Err(error) = super::unpair().await {
                    eprintln!("remote: phone-initiated unpair failed: {error}");
                }
            });
        }
        _ => {
            handshake
                .host
                .execute(cmd, &NatsReply { client, msg: &msg })
                .await;
        }
    }
}

const HANDSHAKE_PROTOCOL_VERSION: u32 = 1;

/// Inputs bound into the pairing handshake transcript. Every field is an
/// `&str`, so a positional call could silently transpose two of them and sign
/// a different transcript — named fields make that a compile error instead.
struct HandshakeTranscript<'a> {
    pair_id: &'a str,
    desktop_id: &'a str,
    desktop_public_key: &'a str,
    bridge_instance_id: &'a str,
    device_id: &'a str,
    client_public_key: &'a str,
    client_nonce: &'a str,
    desktop_nonce: &'a str,
}

fn handshake_transcript(parts: &HandshakeTranscript<'_>) -> String {
    [
        "futureos-remote-handshake-v1",
        parts.pair_id,
        parts.desktop_id,
        parts.desktop_public_key,
        parts.bridge_instance_id,
        parts.device_id,
        parts.client_public_key,
        parts.client_nonce,
        parts.desktop_nonce,
    ]
    .join("\n")
}

async fn handle_pair_handshake(
    client: &async_nats::Client,
    msg: &async_nats::Message,
    cmd: &IncomingCmd,
    state: &HandshakeState,
) {
    // The desktop key pair is derived once: a bad seed fails here, and a
    // successfully parsed pair always signs, so no later fallible step.
    let key_pair = match nkeys::KeyPair::from_seed(&state.creds.nkey_seed) {
        Ok(key_pair) => key_pair,
        Err(error) => {
            reply(client, msg, false, Value::Null, Some(&error.to_string())).await;
            return;
        }
    };
    let desktop_public_key = key_pair.public_key();
    // Validate BEFORE deactivating commands: a garbage handshake must not lock
    // the bridge (active=false gates every command) — only a well-formed
    // handshake from a party that holds the pair's identity may suspend the
    // current session while it re-authenticates.
    let valid = cmd.protocol_version == HANDSHAKE_PROTOCOL_VERSION
        && cmd.pair_id == state.creds.pair_id
        && cmd.expected_desktop_id == state.creds.desktop_id
        && cmd.expected_desktop_public_key == desktop_public_key
        && cmd.device_id.starts_with("dev_")
        && cmd.client_public_key.starts_with('U')
        && (16..=256).contains(&cmd.client_nonce.len());
    if !valid {
        reply(
            client,
            msg,
            false,
            Value::Null,
            Some("pairing_identity_mismatch"),
        )
        .await;
        return;
    }

    // A candidate challenge must not revoke the already authenticated peer.

    let desktop_nonce = nkeys::KeyPair::new_user().public_key();
    let transcript = handshake_transcript(&HandshakeTranscript {
        pair_id: &state.creds.pair_id,
        desktop_id: &state.creds.desktop_id,
        desktop_public_key: &desktop_public_key,
        bridge_instance_id: &state.bridge_instance_id,
        device_id: &cmd.device_id,
        client_public_key: &cmd.client_public_key,
        client_nonce: &cmd.client_nonce,
        desktop_nonce: &desktop_nonce,
    });
    let signature = URL_SAFE_NO_PAD.encode(
        key_pair
            .sign(transcript.as_bytes())
            .expect("a seed-derived KeyPair always signs"),
    );
    {
        let mut pending = state.pending.lock().unwrap();
        pending.retain(|_, value| value.created_at.elapsed() < Duration::from_secs(30));
        if pending.len() >= 32 {
            return;
        }
        pending.insert(
            desktop_nonce.clone(),
            PendingHandshake {
                transcript,
                created_at: std::time::Instant::now(),
                device_id: cmd.device_id.clone(),
                client_public_key: cmd.client_public_key.clone(),
            },
        );
    }
    reply(
        client,
        msg,
        true,
        json!({
            "protocolVersion": HANDSHAKE_PROTOCOL_VERSION,
            "pairId": state.creds.pair_id,
            "desktopId": state.creds.desktop_id,
            "desktopPublicKey": desktop_public_key,
            "bridgeInstanceId": state.bridge_instance_id,
            "deviceId": cmd.device_id,
            "clientPublicKey": cmd.client_public_key,
            "clientNonce": cmd.client_nonce,
            "desktopNonce": desktop_nonce,
            "desktopSignature": signature,
        }),
        None,
    )
    .await;
}

async fn handle_pair_handshake_confirm(
    client: &async_nats::Client,
    msg: &async_nats::Message,
    cmd: &IncomingCmd,
    state: &HandshakeState,
) {
    let pending = state.pending.lock().unwrap().remove(&cmd.desktop_nonce);
    let Some(pending) = pending else {
        reply(
            client,
            msg,
            false,
            Value::Null,
            Some("pairing_challenge_expired"),
        )
        .await;
        return;
    };
    if !state.access_current()
        || pending.created_at.elapsed() >= Duration::from_secs(30)
        || cmd.device_id != pending.device_id
    {
        reply(
            client,
            msg,
            false,
            Value::Null,
            Some("pairing_identity_mismatch"),
        )
        .await;
        return;
    }
    let signature = URL_SAFE_NO_PAD.decode(&cmd.client_signature).ok();
    let verified = signature
        .and_then(|signature| {
            nkeys::KeyPair::from_public_key(&pending.client_public_key)
                .ok()
                .map(|key| {
                    key.verify(pending.transcript.as_bytes(), &signature)
                        .is_ok()
                })
        })
        .unwrap_or(false);
    if !verified {
        reply(
            client,
            msg,
            false,
            Value::Null,
            Some("pairing_signature_invalid"),
        )
        .await;
        return;
    }
    let confirm = || -> Result<(), crate::AppError> {
        if !state.confirmed.load(Ordering::Acquire) {
            crate::remote::pairing::save_creds(&state.creds)?;
            state.confirmed.store(true, Ordering::Release);
        }
        state.active.store(true, Ordering::Release);
        Ok(())
    };
    let committed = match state.access_epoch {
        Some(epoch) => super::SUPERVISOR.access.commit(epoch, confirm),
        None => Some(confirm()),
    };
    match committed {
        None => return,
        Some(Err(error)) => {
            reply(client, msg, false, Value::Null, Some(&error.to_string())).await;
            return;
        }
        Some(Ok(())) => {}
    }
    reply(
        client,
        msg,
        true,
        json!({
            "confirmed": true,
            "pairId": state.creds.pair_id,
            "desktopId": state.creds.desktop_id,
            "bridgeInstanceId": state.bridge_instance_id,
            "deviceId": cmd.device_id,
            "desktopNonce": cmd.desktop_nonce,
            "features": ["file_transfer_v1", "file_download_v2", "approval_tier_v1", "continue_run_v1", "prompt_receipt_v1"],
            "presence": super::build_presence_payload(
                &state.creds.pair_id,
                &state.bridge_instance_id,
            ),
        }),
        None,
    )
    .await;
}

/// Send a unified request-reply response (in `RpcResponse` shape), and flush to ensure timely delivery.
async fn reply(
    client: &async_nats::Client,
    msg: &async_nats::Message,
    success: bool,
    data: Value,
    error: Option<&str>,
) {
    if msg.reply.is_none() {
        return;
    }
    let body = json!({
        "type": "response",
        "success": success,
        "data": data,
        "error": error,
    });
    // Plain JSON is the default. When FUTURE_REMOTE_JSON_GZIP explicitly opts
    // in, only large replies that actually shrink are gzip-compressed; mobile
    // recognizes standard gzip magic bytes without a negotiation round.
    let payload = encode_reply_payload(&body);
    let _ = REPLY_CAPTURE.try_with(|capture| {
        *capture.lock().unwrap() = Some(payload.clone());
    });
    publish_reply_payload(client, msg, payload).await;
}

fn encode_reply_payload(body: &Value) -> Vec<u8> {
    encode_reply_payload_with_gzip(body, remote_json_gzip_enabled())
}

fn encode_reply_payload_with_gzip(body: &Value, gzip_enabled: bool) -> Vec<u8> {
    let plain = serde_json::to_vec(body).expect("a response Value always serializes");
    if !gzip_enabled || plain.len() < REMOTE_JSON_GZIP_THRESHOLD_BYTES {
        return plain;
    }

    let mut encoder = GzEncoder::new(Vec::new(), Compression::fast());
    if encoder.write_all(&plain).is_err() {
        return plain;
    }
    let Ok(compressed) = encoder.finish() else {
        return plain;
    };
    if compressed.len() >= plain.len() {
        return plain;
    }
    compressed
}

async fn publish_reply_payload(
    client: &async_nats::Client,
    msg: &async_nats::Message,
    payload: Vec<u8>,
) {
    let Some(reply_subject) = msg.reply.clone() else {
        return;
    };
    if let Err(error) = client.publish(reply_subject, payload.into()).await {
        eprintln!("FutureOS: failed to publish remote command reply: {error}");
        return;
    }
    if let Err(error) = client.flush().await {
        eprintln!("FutureOS: failed to flush remote command reply: {error}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use flate2::read::GzDecoder;
    use std::io::Read;

    #[test]
    fn model_identity_keeps_raw_slashes_and_distinguishes_providers() {
        for provider in ["one", "two"] {
            assert_eq!(
                qualified_model_id("family/model", provider),
                Some(format!("{provider}/family/model"))
            );
            let qualified = format!("{provider}/family/model");
            assert_eq!(qualified_model_id(&qualified, provider), Some(qualified));
        }
        assert_eq!(
            qualified_model_id("openrouter/openrouter/auto", "openrouter").as_deref(),
            Some("openrouter/openrouter/auto")
        );
        assert_eq!(
            qualified_model_id("p/model", "").as_deref(),
            Some("p/model")
        );
        assert_eq!(qualified_model_id("", "p"), None);
    }

    #[test]
    fn remote_json_gzip_is_automatic_thresholded_and_standard() {
        let body = json!({ "entries": ["repeated history ".repeat(8_000)] });
        let plain = serde_json::to_vec(&body).unwrap();

        let encoded = encode_reply_payload_with_gzip(&body, true);
        assert!(encoded.starts_with(&[0x1f, 0x8b]), "expected standard gzip");
        assert!(
            encoded.len() < plain.len() / 4,
            "expected useful compression"
        );
        let mut decoder = GzDecoder::new(encoded.as_slice());
        let mut decoded = Vec::new();
        decoder.read_to_end(&mut decoded).unwrap();
        assert_eq!(decoded, plain);

        let small = json!({ "ok": true });
        let encoded_small = encode_reply_payload_with_gzip(&small, true);
        assert_eq!(encoded_small, serde_json::to_vec(&small).unwrap());

        let disabled = encode_reply_payload_with_gzip(&body, false);
        assert_eq!(disabled, plain, "gzip must remain opt-in");
    }

    #[test]
    fn handshake_transcript_binds_both_device_identities_and_nonces() {
        let transcript = handshake_transcript(&HandshakeTranscript {
            pair_id: "pair_1",
            desktop_id: "desktop_1",
            desktop_public_key: "UDESKTOP",
            bridge_instance_id: "bridge_1",
            device_id: "dev_1",
            client_public_key: "UCLIENT",
            client_nonce: "client_nonce",
            desktop_nonce: "desktop_nonce",
        });
        assert_eq!(
            transcript,
            "futureos-remote-handshake-v1\npair_1\ndesktop_1\nUDESKTOP\nbridge_1\ndev_1\nUCLIENT\nclient_nonce\ndesktop_nonce"
        );
        assert_ne!(
            transcript,
            handshake_transcript(&HandshakeTranscript {
                pair_id: "pair_1",
                desktop_id: "desktop_other",
                desktop_public_key: "UDESKTOP",
                bridge_instance_id: "bridge_1",
                device_id: "dev_1",
                client_public_key: "UCLIENT",
                client_nonce: "client_nonce",
                desktop_nonce: "desktop_nonce",
            })
        );
    }

    #[test]
    fn handshake_signature_rejects_tampered_transcript() {
        let desktop = nkeys::KeyPair::new_user();
        let transcript = handshake_transcript(&HandshakeTranscript {
            pair_id: "pair_1",
            desktop_id: "desktop_1",
            desktop_public_key: &desktop.public_key(),
            bridge_instance_id: "bridge_1",
            device_id: "dev_1",
            client_public_key: "UCLIENT",
            client_nonce: "client_nonce",
            desktop_nonce: "desktop_nonce",
        });
        let signature = desktop.sign(transcript.as_bytes()).unwrap();
        let verifier = nkeys::KeyPair::from_public_key(&desktop.public_key()).unwrap();
        assert!(verifier.verify(transcript.as_bytes(), &signature).is_ok());
        assert!(verifier
            .verify(format!("{transcript}_tampered").as_bytes(), &signature)
            .is_err());
    }

    fn text_message(text: &str) -> Value {
        json!({ "role": "assistant", "blocks": [{"kind":"text","text":text}] })
    }

    #[test]
    fn paginate_small_list_is_one_page() {
        let messages = vec![text_message("a"), text_message("b"), text_message("c")];
        let page = paginate_messages(messages, 0, 100);
        assert_eq!(page["messages"].as_array().unwrap().len(), 3);
        assert_eq!(page["offset"], 0);
        assert_eq!(page["nextOffset"], 3);
        assert_eq!(page["total"], 3);
        assert_eq!(page["hasMore"], false);
    }

    #[test]
    fn paginate_respects_limit_and_cursors() {
        let messages = vec![text_message("a"), text_message("b"), text_message("c")];
        let first = paginate_messages(messages.clone(), 0, 2);
        assert_eq!(first["messages"].as_array().unwrap().len(), 2);
        assert_eq!(first["nextOffset"], 2);
        assert_eq!(first["hasMore"], true);
        let second = paginate_messages(messages, 2, 2);
        assert_eq!(second["messages"].as_array().unwrap().len(), 1);
        assert_eq!(second["nextOffset"], 3);
        assert_eq!(second["hasMore"], false);
    }

    #[test]
    fn paginate_bounds_by_byte_budget() {
        // ~100KB messages; a 512KB budget fits ~5 of them, forcing a second page.
        let big = "x".repeat(100 * 1024);
        let messages: Vec<Value> = (0..6).map(|_| text_message(&big)).collect();
        let page = paginate_messages(messages, 0, 100);
        let arr = page["messages"].as_array().unwrap();
        let arr_len = arr.len();
        assert!(
            arr_len < 6,
            "expected byte budget to cap the page, got {arr_len}"
        );
        assert_eq!(page["hasMore"], true);
        // The page itself stays comfortably under the 1MB NATS payload cap.
        let size = serde_json::to_vec(&page).map(|b| b.len()).unwrap();
        assert!(size < 1024 * 1024, "page too large: {size}");
    }

    #[test]
    fn backward_history_budget_defers_complete_oldest_exchanges() {
        let big = "x".repeat(220 * 1024);
        let mut entries = Vec::new();
        for index in 0..3 {
            entries.push(json!({
                "id": format!("u{index}"),
                "role": "user",
                "blocks":[{"kind":"text","text":format!("q{index}")}]
            }));
            entries.push(json!({
                "id": format!("a{index}"),
                "role": "assistant",
                "blocks":[{"kind":"text","text":big.clone()}]
            }));
        }
        let page = prepare_backward_entries_page(
            "missing-session",
            json!({ "entries": entries, "hasMore": false, "nextOffset": 10 }),
        );
        let rows = page["entries"].as_array().unwrap();
        assert_eq!(rows.first().and_then(|row| row["id"].as_str()), Some("u1"));
        assert_eq!(page["nextOffset"], 12);
        assert_eq!(page["hasMore"], true);
        assert!(serde_json::to_vec(rows).unwrap().len() <= BACKWARD_HISTORY_PAGE_BYTES);
    }

    #[test]
    fn paginate_caps_and_includes_oversized_message() {
        // A message larger than the page budget is content-capped (cap < budget)
        // so it fits, and the page never exceeds the payload cap.
        let huge = "y".repeat(MESSAGES_PAGE_BYTES + 1024);
        let messages = vec![text_message(&huge), text_message("small")];
        let page = paginate_messages(messages, 0, 100);
        let arr = page["messages"].as_array().unwrap();
        assert!(!arr.is_empty());
        // The oversized message's content was truncated to the cap.
        let content = arr[0]["blocks"][0]["text"].as_str().unwrap();
        assert!(content.len() <= MESSAGE_CONTENT_CAP_BYTES + 128);
        let size = serde_json::to_vec(&page).map(|b| b.len()).unwrap();
        assert!(size < 1024 * 1024, "page too large: {size}");
    }

    #[test]
    fn paginate_caps_structured_tool_arguments_and_metadata() {
        let entries = vec![json!({
            "id":"a1","kind":"assistant","role":"assistant","runId":"r1","createdAtMs":1000,
            "blocks":[{"kind":"text","text":"done"},{"kind":"tool_call","toolCallId":"tc-1","name":"write","arguments":{"path":"/tmp/x","content":"x".repeat(700_000)}}],
            "metadata":{"blob":"y".repeat(700_000)}
        })];
        let page = paginate_items(entries, 0, 100, "entries");
        assert!(serde_json::to_vec(&page).unwrap().len() < 1024 * 1024);
        assert_eq!(page["entries"][0]["runId"], "r1");
        assert_eq!(page["entries"][0]["metadata"]["remoteTruncated"], true);
    }

    #[test]
    fn truncate_caps_string_content() {
        let mut message = text_message(&"z".repeat(MESSAGE_CONTENT_CAP_BYTES * 2));
        truncate_message_content(&mut message, MESSAGE_CONTENT_CAP_BYTES);
        let content = message["blocks"][0]["text"].as_str().unwrap();
        assert!(content.len() <= MESSAGE_CONTENT_CAP_BYTES + 128);
        assert!(content.ends_with('…'));
    }

    #[test]
    fn truncate_caps_text_blocks_and_keeps_others() {
        let mut message = json!({
            "role": "assistant",
            "blocks": [
                { "kind": "text", "text": "a".repeat(MESSAGE_CONTENT_CAP_BYTES * 2) },
                { "kind": "tool_use", "id": "t1", "name": "shell" },
            ]
        });
        truncate_message_content(&mut message, MESSAGE_CONTENT_CAP_BYTES);
        let blocks = message["blocks"].as_array().unwrap();
        // Tool block untouched.
        assert_eq!(blocks[1]["kind"], "tool_use");
        assert_eq!(blocks[1]["name"], "shell");
        // Text block truncated.
        let text = blocks[0]["text"].as_str().unwrap();
        assert!(text.len() <= MESSAGE_CONTENT_CAP_BYTES + 8);
    }

    #[test]
    fn truncate_leaves_small_messages_alone() {
        let mut message = text_message("small");
        truncate_message_content(&mut message, MESSAGE_CONTENT_CAP_BYTES);
        assert_eq!(message["blocks"][0]["text"], "small");
    }

    #[test]
    fn truncate_skips_messages_without_a_content_field() {
        // Oversized but with no `content` key → the let-else early-returns.
        let mut message = json!({
            "role": "assistant",
            "tool_use": "z".repeat(MESSAGE_CONTENT_CAP_BYTES * 2),
        });
        truncate_message_content(&mut message, MESSAGE_CONTENT_CAP_BYTES);
        assert!(message.get("blocks").is_none());
    }

    #[test]
    fn truncate_skips_non_text_blocks_and_fits_small_ones() {
        // A non-text block first (continue), then a small text block that fits
        // (the remaining-subtract path), then an oversized one.
        let mut message = json!({
            "role": "assistant",
            "blocks": [
                { "kind": "tool_use", "id": "t0", "name": "shell" },
                { "kind": "text", "text": "small" },
                { "kind": "text", "text": "z".repeat(MESSAGE_CONTENT_CAP_BYTES * 2) },
            ]
        });
        truncate_message_content(&mut message, MESSAGE_CONTENT_CAP_BYTES);
        let blocks = message["blocks"].as_array().unwrap();
        assert_eq!(blocks[0]["name"], "shell"); // untouched
        assert_eq!(blocks[1]["text"], "small"); // fits, untouched
        let text = blocks[2]["text"].as_str().unwrap();
        assert!(text.len() <= MESSAGE_CONTENT_CAP_BYTES + 8);
    }

    #[test]
    fn truncate_ignores_non_string_non_array_content() {
        // Oversized but content is a scalar → the `_ => {}` arm.
        let mut message = json!({
            "role": "assistant",
            "blocks": 42,
            "pad": "z".repeat(MESSAGE_CONTENT_CAP_BYTES * 2),
        });
        truncate_message_content(&mut message, MESSAGE_CONTENT_CAP_BYTES);
        assert_eq!(message["blocks"], 42);
    }

    #[test]
    fn truncate_skips_text_blocks_without_a_string_text() {
        // A block claiming `type: "text"` but carrying a non-string `text` is
        // left intact (the text-block match's `_ => {}` arm).
        let mut message = json!({
            "role": "assistant",
            "blocks": [
                { "kind": "text", "text": 42 },
                { "kind": "text", "text": "z".repeat(MESSAGE_CONTENT_CAP_BYTES * 2) },
            ]
        });
        truncate_message_content(&mut message, MESSAGE_CONTENT_CAP_BYTES);
        let blocks = message["blocks"].as_array().unwrap();
        assert_eq!(blocks[0]["text"], 42); // untouched
        let text = blocks[1]["text"].as_str().unwrap();
        assert!(text.len() <= MESSAGE_CONTENT_CAP_BYTES + 8);
    }

    #[test]
    fn byte_cut_is_char_boundary_safe() {
        let s = "中文内容"; // multi-byte chars
        let (end, truncated) = byte_cut(s, 4);
        assert!(s.is_char_boundary(end));
        assert!(truncated);
        let (end, truncated) = byte_cut(s, 1024);
        assert_eq!(end, s.len());
        assert!(!truncated);
    }

    #[test]
    fn paginate_events_pages_a_large_tail() {
        // A multi-MB replay tail must page instead of shipping as one reply.
        let big = "y".repeat(100 * 1024);
        let events: Vec<Value> = (0..6)
            .map(|i| json!({ "type": "text_chunk", "run_id": "run-1", "idx": i, "data": big }))
            .collect();
        let data = json!({ "runId": "run-1", "events": events });
        let first = paginate_events(data, 0, 100);
        let arr = first["events"].as_array().unwrap();
        let arr_len = arr.len();
        assert!(
            arr_len < 6,
            "byte budget should split the tail, got {arr_len}"
        );
        assert_eq!(first["runId"], "run-1");
        assert_eq!(first["hasMore"], true);
        let size = serde_json::to_vec(&first).map(|b| b.len()).unwrap();
        assert!(size < 1024 * 1024, "page too large: {size}");
    }

    #[test]
    fn paginate_events_carries_projection_on_first_page() {
        let events = vec![json!({ "type": "text_chunk", "run_id": "run-1", "idx": 0 })];
        let data = json!({
            "runId": "run-1",
            "events": events,
            "projection": { "run_id": "run-1", "cursor": 42, "events": [] },
        });
        let page = paginate_events(data, 0, 100);
        assert_eq!(page["projection"]["cursor"], 42);
        assert_eq!(page["events"].as_array().unwrap().len(), 1);
        assert_eq!(page["hasMore"], false);
    }

    #[test]
    fn paginate_events_carries_truncated_flag() {
        let events = vec![json!({ "type": "text_chunk", "run_id": "run-1", "idx": 0 })];
        let data = json!({
            "runId": "run-1",
            "events": events,
            "truncated": true,
        });
        let page = paginate_events(data, 0, 100);
        assert_eq!(page["truncated"], json!(true));
        assert_eq!(page["events"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn paginate_events_truncates_a_single_oversized_event_data() {
        // A single journal event larger than the relay payload cap (a multi-MB
        // tool result) must not page out whole — the "at least one item per
        // page" rule would otherwise ship it intact and the relay silently
        // drops it, failing every reconcile on that session (H2 residual).
        let huge = "z".repeat(3 * 1024 * 1024);
        let events =
            vec![json!({ "type": "tool_result", "run_id": "run-1", "idx": 0, "data": huge })];
        let data = json!({ "runId": "run-1", "events": events });
        let page = paginate_events(data, 0, 100);
        let arr = page["events"].as_array().unwrap();
        assert_eq!(arr.len(), 1, "the single event still pages through");
        let data_str = arr[0]["data"].as_str().unwrap();
        let parsed: Value = serde_json::from_str(data_str).unwrap();
        assert_eq!(
            parsed["_truncated"], true,
            "oversized event data must be swapped for the _truncated marker"
        );
        // The page must now fit well under the relay cap.
        let size = serde_json::to_vec(&page).map(|b| b.len()).unwrap();
        assert!(
            size < 1024 * 1024,
            "page too large after truncation: {size}"
        );
    }

    #[test]
    fn truncate_swaps_oversized_event_data_but_keeps_small() {
        let mut big = json!({ "type": "tool_result", "run_id": "r", "idx": 0, "data": "x".repeat(MESSAGE_CONTENT_CAP_BYTES + 10) });
        truncate_message_content(&mut big, MESSAGE_CONTENT_CAP_BYTES);
        let parsed: Value = serde_json::from_str(big["data"].as_str().unwrap()).unwrap();
        assert_eq!(parsed["_truncated"], true);

        let mut small = json!({ "type": "text_chunk", "run_id": "r", "idx": 1, "data": "ok" });
        truncate_message_content(&mut small, MESSAGE_CONTENT_CAP_BYTES);
        assert_eq!(small["data"], "ok");
    }
}

#[cfg(test)]
mod bridge_tests {
    #![allow(clippy::await_holding_lock)]
    use super::super::test_support::{
        await_publish, ensure_mock_agent, init_store, jwt, mock_agent_lock, nats_connect,
        nats_connect_once, now_secs, unique, FakeNats, HomeGuard,
    };
    use super::*;
    use crate::remote_host::files as transfer;
    use serde_json::json;
    use std::time::Duration;

    #[test]
    fn thread_title_derivation_matches_the_gui_draft() {
        // Whitespace-only / empty input falls back to the default chat title.
        assert_eq!(derive_thread_title(""), "New Chat");
        assert_eq!(derive_thread_title("  \n\t  "), "New Chat");
        // Whitespace collapses; 28 chars is the cut, ellipsized beyond it.
        assert_eq!(
            derive_thread_title("hello   there\nworld"),
            "hello there world"
        );
        let long = "abcdefghijklmnopqrstuvwxyz0123456789";
        assert_eq!(derive_thread_title(long), "abcdefghijklmnopqrstuvwxyz01...");
    }

    fn bridge_creds() -> crate::remote::pairing::PairingCreds {
        let key_pair = nkeys::KeyPair::new_user();
        crate::remote::pairing::PairingCreds {
            handshake_version: 1,
            pair_id: format!("pair_{}", unique("cmd")),
            desktop_id: format!("desktop_{}", unique("cmd")),
            nkey_seed: key_pair.seed().unwrap().to_string(),
            user_jwt: jwt(now_secs() + 3600),
            nats_url: "nats://127.0.0.1:1".to_string(),
            nats_ws_url: "ws://127.0.0.1:1".to_string(),
            jwt_expires_at: now_secs() + 3600,
        }
    }

    /// A running command loop against a fake NATS: returns the client handle a
    /// test drives, plus the pieces it may need to poke.
    struct Bridge {
        client: async_nats::Client,
        nats: FakeNats,
        pair_id: String,
        handshake: HandshakeState,
        loop_handle: tokio::task::JoinHandle<()>,
    }

    impl Bridge {
        async fn start() -> Self {
            Self::start_with_host(super::super::host()).await
        }
        async fn start_with_host(host: &'static dyn super::super::services::BusinessHost) -> Self {
            let nats = FakeNats::start().await;
            let client = nats_connect(&nats).await;
            let creds = bridge_creds();
            let pair_id = creds.pair_id.clone();
            let mut handshake = HandshakeState::new(
                creds,
                Arc::new(AtomicBool::new(false)),
                format!("bridge_{}", unique("cmd")),
            );
            handshake.host = host;
            let reply_slots = new_reply_slots();
            let loop_handle = tokio::spawn(command_loop(
                client.clone(),
                pair_id.clone(),
                reply_slots.clone(),
                handshake.clone(),
            ));
            nats.wait_for_sub(&format!("p.{pair_id}.cmd.>"), Duration::from_secs(5))
                .await;
            Bridge {
                client,
                nats,
                pair_id,
                handshake,
                loop_handle,
            }
        }

        /// Activate the bridge (as a completed handshake would).
        fn activate(&self) {
            self.handshake.active_flag().store(true, Ordering::Release);
        }

        /// Send a command and await its reply envelope.
        async fn call(&self, cmd: Value) -> Value {
            let subject = format!("p.{}.cmd.rpc", self.pair_id);
            let message = self
                .client
                .request(subject, serde_json::to_vec(&cmd).unwrap().into())
                .await
                .expect("bridge reply");
            serde_json::from_slice(&message.payload).expect("reply is JSON")
        }

        fn stop(self) {
            self.loop_handle.abort();
        }
    }

    #[tokio::test]
    async fn protocol_routes_to_a_substitute_host_and_rejects_a_stale_access_identity() {
        struct Echo;
        impl super::super::services::BusinessHost for Echo {
            fn execute<'a>(
                &'a self,
                cmd: IncomingCmd,
                sink: &'a dyn ReplySink,
            ) -> futures::future::BoxFuture<'a, ()> {
                Box::pin(async move {
                    sink.send(true, json!({"host": "substitute", "id": cmd.id}), None)
                        .await
                })
            }
        }
        static ECHO: Echo = Echo;
        let bridge = Bridge::start_with_host(&ECHO).await;
        bridge.activate();
        let response = bridge
            .call(json!({"id":"host-test", "type":"get_state"}))
            .await;
        assert_eq!(response["data"]["host"], "substitute");
        let response = bridge
            .call(json!({"id":"stale-test", "type":"prompt", "bridgeInstanceId":"old-access"}))
            .await;
        assert_eq!(response["error"], "remote_access_changed");
        bridge.stop();
    }

    fn handshake_cmd(
        creds: &crate::remote::pairing::PairingCreds,
        client_key: &nkeys::KeyPair,
    ) -> Value {
        json!({
            "id": unique("cmd"),
            "type": "pair_handshake",
            "protocolVersion": 1,
            "pairId": creds.pair_id,
            "deviceId": "dev_test",
            "clientPublicKey": client_key.public_key(),
            "clientNonce": "nonce-0123456789abcdef",
            "expectedDesktopId": creds.desktop_id,
            "expectedDesktopPublicKey": crate::remote::pairing::public_key(creds).unwrap(),
        })
    }

    /// A corrupted desktop NKey seed fails the handshake before any signing:
    /// the client gets a failure reply and the bridge stays inactive.
    #[tokio::test]
    async fn handshake_rejects_a_bad_desktop_seed() {
        let _home = HomeGuard::new("cmd-bad-seed");
        let nats = FakeNats::start().await;
        let client = nats_connect(&nats).await;
        let mut creds = bridge_creds();
        creds.nkey_seed = "not-a-valid-seed".to_string();
        let handshake = HandshakeState::new(
            creds,
            Arc::new(AtomicBool::new(false)),
            format!("bridge_{}", unique("cmd")),
        );
        let reply_subject = format!("rep_{}", unique("hs"));
        let mut tap = nats.tap();
        let msg = async_nats::Message {
            subject: "p.pair.cmd.pair_handshake".into(),
            reply: Some(reply_subject.clone().into()),
            payload: Vec::new().into(),
            headers: None,
            status: None,
            description: None,
            length: 0,
        };
        let cmd = IncomingCmd {
            cmd_type: "pair_handshake".to_string(),
            ..Default::default()
        };
        handle_pair_handshake(&client, &msg, &cmd, &handshake).await;
        let reply = await_publish(&mut tap, &reply_subject, Duration::from_secs(5)).await;
        assert_eq!(reply.json()["success"], json!(false));
        assert!(!handshake.active_flag().load(Ordering::Acquire));
    }

    #[tokio::test]
    async fn rejects_garbage_and_requires_handshake() {
        let _home = HomeGuard::new("cmd-gate");
        let bridge = Bridge::start().await;

        // Unparseable payload → error reply.
        let mut tap = bridge.nats.tap();
        let reply_subject = format!("rep-{}", unique("garbage"));
        bridge.nats.inject(
            &format!("p.{}.cmd.rpc", bridge.pair_id),
            Some(&reply_subject),
            b"{not json".to_vec(),
        );
        let reply = await_publish(&mut tap, &reply_subject, Duration::from_secs(5)).await;
        assert_eq!(reply.json()["success"], json!(false));
        assert!(reply.json()["error"]
            .as_str()
            .unwrap()
            .contains("Failed to parse command JSON"));

        // A well-formed non-handshake command before activation is refused.
        let reply = bridge
            .call(json!({ "id": unique("cmd"), "type": "list_sessions" }))
            .await;
        assert_eq!(reply["success"], json!(false));
        assert_eq!(reply["error"], json!("pairing_handshake_required"));

        // A command with no reply subject is processed but not answered.
        bridge.nats.inject(
            &format!("p.{}.cmd.rpc", bridge.pair_id),
            None,
            b"{ not json either".to_vec(),
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
        bridge.stop();
    }

    #[tokio::test]
    async fn handshake_roundtrip_activates_the_bridge() {
        let _home = HomeGuard::new("cmd-handshake");
        init_store();
        let bridge = Bridge::start().await;
        let client_key = nkeys::KeyPair::new_user();
        let creds = bridge.handshake.creds.clone();

        // Identity mismatch → refused.
        let mut bad = handshake_cmd(&creds, &client_key);
        bad["protocolVersion"] = json!(99);
        let reply = bridge.call(bad).await;
        assert_eq!(reply["error"], json!("pairing_identity_mismatch"));

        // Valid challenge → desktop nonce + signature.
        let reply = bridge.call(handshake_cmd(&creds, &client_key)).await;
        assert_eq!(reply["success"], json!(true), "got: {reply}");
        let data = &reply["data"];
        assert_eq!(data["pairId"], json!(creds.pair_id));
        let desktop_nonce = data["desktopNonce"].as_str().unwrap().to_string();
        // The desktop signature verifies against the transcript.
        let transcript = handshake_transcript(&HandshakeTranscript {
            pair_id: &creds.pair_id,
            desktop_id: &creds.desktop_id,
            desktop_public_key: &crate::remote::pairing::public_key(&creds).unwrap(),
            bridge_instance_id: &bridge.handshake.bridge_instance_id,
            device_id: "dev_test",
            client_public_key: &client_key.public_key(),
            client_nonce: "nonce-0123456789abcdef",
            desktop_nonce: &desktop_nonce,
        });
        let signature = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(data["desktopSignature"].as_str().unwrap())
            .unwrap();
        nkeys::KeyPair::from_public_key(&crate::remote::pairing::public_key(&creds).unwrap())
            .unwrap()
            .verify(transcript.as_bytes(), &signature)
            .unwrap();

        // Unknown challenge → expired.
        let reply = bridge
            .call(json!({
                "id": unique("cmd"),
                "type": "pair_handshake_confirm",
                "deviceId": "dev_test",
                "desktopNonce": "never-issued",
                "clientSignature": "x",
            }))
            .await;
        assert_eq!(reply["error"], json!("pairing_challenge_expired"));

        // A forged client signature → rejected.
        let forged = client_key.sign(b"a different transcript entirely").unwrap();
        let reply = bridge
            .call(json!({
                "id": unique("cmd"),
                "type": "pair_handshake_confirm",
                "deviceId": "dev_test",
                "desktopNonce": desktop_nonce,
                "clientSignature": base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(forged),
            }))
            .await;
        assert_eq!(reply["error"], json!("pairing_signature_invalid"));

        // A fresh challenge + correctly signed transcript activates the bridge
        // and persists the credential.
        let reply = bridge.call(handshake_cmd(&creds, &client_key)).await;
        let desktop_nonce = reply["data"]["desktopNonce"].as_str().unwrap().to_string();
        let transcript = handshake_transcript(&HandshakeTranscript {
            pair_id: &creds.pair_id,
            desktop_id: &creds.desktop_id,
            desktop_public_key: &crate::remote::pairing::public_key(&creds).unwrap(),
            bridge_instance_id: &bridge.handshake.bridge_instance_id,
            device_id: "dev_test",
            client_public_key: &client_key.public_key(),
            client_nonce: "nonce-0123456789abcdef",
            desktop_nonce: &desktop_nonce,
        });
        let signature = client_key.sign(transcript.as_bytes()).unwrap();
        let reply = bridge
            .call(json!({
                "id": unique("cmd"),
                "type": "pair_handshake_confirm",
                "deviceId": "dev_test",
                "desktopNonce": desktop_nonce,
                "clientSignature": base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(signature),
            }))
            .await;
        assert_eq!(reply["success"], json!(true), "got: {reply}");
        assert_eq!(reply["data"]["confirmed"], json!(true));
        assert_eq!(
            reply["data"]["features"],
            json!([
                "file_transfer_v1",
                "file_download_v2",
                "approval_tier_v1",
                "continue_run_v1",
                "prompt_receipt_v1"
            ])
        );
        assert!(bridge.handshake.active_flag().load(Ordering::Acquire));
        assert!(crate::remote::pairing::load_creds().is_some());

        // Now ordinary commands pass the gate.
        let reply = bridge
            .call(json!({ "id": unique("cmd"), "type": "list_sessions" }))
            .await;
        assert_eq!(reply["success"], json!(true));

        // A candidate handshake preserves the authenticated session and does
        // not invalidate another outstanding challenge.
        let reply = bridge.call(handshake_cmd(&creds, &client_key)).await;
        let desktop_nonce = reply["data"]["desktopNonce"].as_str().unwrap().to_string();
        assert!(bridge.handshake.active_flag().load(Ordering::Acquire));
        let overlapping = bridge.call(handshake_cmd(&creds, &client_key)).await;
        assert_eq!(overlapping["success"], json!(true));
        let live = bridge
            .call(json!({ "id": unique("cmd"), "type": "list_sessions" }))
            .await;
        assert_eq!(live["success"], json!(true));
        let transcript = handshake_transcript(&HandshakeTranscript {
            pair_id: &creds.pair_id,
            desktop_id: &creds.desktop_id,
            desktop_public_key: &crate::remote::pairing::public_key(&creds).unwrap(),
            bridge_instance_id: &bridge.handshake.bridge_instance_id,
            device_id: "dev_test",
            client_public_key: &client_key.public_key(),
            client_nonce: "nonce-0123456789abcdef",
            desktop_nonce: &desktop_nonce,
        });
        let signature = client_key.sign(transcript.as_bytes()).unwrap();
        let reply = bridge
            .call(json!({
                "id": unique("cmd"),
                "type": "pair_handshake_confirm",
                "deviceId": "dev_test",
                "desktopNonce": desktop_nonce,
                "clientSignature": base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(signature),
            }))
            .await;
        assert_eq!(reply["success"], json!(true), "got: {reply}");
        assert!(bridge.handshake.active_flag().load(Ordering::Acquire));
        bridge.stop();
    }

    #[tokio::test]
    async fn handshake_confirm_rejects_a_device_mismatch() {
        let _home = HomeGuard::new("cmd-device-mismatch");
        let bridge = Bridge::start().await;
        let client_key = nkeys::KeyPair::new_user();
        let creds = bridge.handshake.creds.clone();
        let reply = bridge.call(handshake_cmd(&creds, &client_key)).await;
        let desktop_nonce = reply["data"]["desktopNonce"].as_str().unwrap().to_string();
        let reply = bridge
            .call(json!({
                "id": unique("cmd"),
                "type": "pair_handshake_confirm",
                "deviceId": "dev_other",
                "desktopNonce": desktop_nonce,
                "clientSignature": "eA",
            }))
            .await;
        assert_eq!(reply["error"], json!("pairing_identity_mismatch"));
        bridge.stop();
    }

    #[tokio::test]
    async fn handshake_confirm_reports_credential_save_failures() {
        let home = HomeGuard::new("cmd-save-fail");
        let bridge = Bridge::start().await;
        let client_key = nkeys::KeyPair::new_user();
        let creds = bridge.handshake.creds.clone();
        let reply = bridge.call(handshake_cmd(&creds, &client_key)).await;
        let desktop_nonce = reply["data"]["desktopNonce"].as_str().unwrap().to_string();
        let transcript = handshake_transcript(&HandshakeTranscript {
            pair_id: &creds.pair_id,
            desktop_id: &creds.desktop_id,
            desktop_public_key: &crate::remote::pairing::public_key(&creds).unwrap(),
            bridge_instance_id: &bridge.handshake.bridge_instance_id,
            device_id: "dev_test",
            client_public_key: &client_key.public_key(),
            client_nonce: "nonce-0123456789abcdef",
            desktop_nonce: &desktop_nonce,
        });
        let signature = client_key.sign(transcript.as_bytes()).unwrap();
        // No HOME → the credential persist fails and the error surfaces.
        std::env::remove_var("HOME");
        std::env::remove_var("USERPROFILE");
        let reply = bridge
            .call(json!({
                "id": unique("cmd"),
                "type": "pair_handshake_confirm",
                "deviceId": "dev_test",
                "desktopNonce": desktop_nonce,
                "clientSignature": base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(signature),
            }))
            .await;
        assert_eq!(reply["success"], json!(false));
        drop(home);
        bridge.stop();
    }

    /// Activated bridge with a store and mock agent behind it.
    async fn active_bridge(label: &str) -> (HomeGuard, Bridge) {
        let home = HomeGuard::new(label);
        init_store();
        ensure_mock_agent();
        let bridge = Bridge::start().await;
        bridge.activate();
        (home, bridge)
    }

    #[tokio::test]
    async fn presence_and_catalog_commands() {
        let _lock = mock_agent_lock();
        let (_home, bridge) = active_bridge("cmd-presence").await;

        let reply = bridge
            .call(json!({ "id": unique("cmd"), "type": "get_presence" }))
            .await;
        assert_eq!(reply["success"], json!(true));
        assert_eq!(reply["data"]["online"], json!(true));
        assert_eq!(reply["data"]["pairId"], json!(bridge.pair_id));

        // Empty store → empty session/workspace lists.
        let reply = bridge
            .call(json!({ "id": unique("cmd"), "type": "list_sessions" }))
            .await;
        assert_eq!(reply["data"]["sessions"], json!([]));
        let reply = bridge
            .call(json!({ "id": unique("cmd"), "type": "list_workspaces" }))
            .await;
        assert_eq!(reply["success"], json!(true));

        // A thread with an agent session shows up, streaming while its run is
        // active.
        let session = unique("sess");
        let thread = crate::store::create_thread(crate::store::CreateThreadInput {
            mode: "chat".to_string(),
            title: Some("From remote".to_string()),
            workspace_id: None,
            workspace_path: None,
            workspace_name: None,
            agent_session_id: Some(session.clone()),
        })
        .unwrap();
        crate::store::create_run(crate::store::CreateRunInput {
            id: None,
            thread_id: thread.id.clone(),
            trigger_message_id: None,
            model_provider: None,
            model_id: None,
        })
        .unwrap();
        let reply = bridge
            .call(json!({ "id": unique("cmd"), "type": "list_sessions" }))
            .await;
        let sessions = reply["data"]["sessions"].as_array().unwrap();
        let row = sessions
            .iter()
            .find(|row| row["sessionId"] == json!(session))
            .expect("thread listed");
        assert_eq!(row["title"], json!("From remote"));
        assert_eq!(row["streaming"], json!(true));
        assert!(row["parentSessionId"].is_null());
        crate::store::sync_thread_parent_session(&session, "parent-session").unwrap();
        let reply = bridge
            .call(json!({ "id": unique("cmd"), "type": "list_sessions" }))
            .await;
        assert_eq!(
            reply["data"]["sessions"][0]["parentSessionId"],
            json!("parent-session")
        );

        bridge.stop();
    }

    #[tokio::test]
    async fn catalog_commands_report_store_failures() {
        let _lock = mock_agent_lock();
        // No init_store and no .future dir → the DB connect fails.
        let _home = HomeGuard::new("cmd-store-down");
        ensure_mock_agent();
        let bridge = Bridge::start().await;
        bridge.activate();

        let reply = bridge
            .call(json!({ "id": unique("cmd"), "type": "list_sessions" }))
            .await;
        assert_eq!(reply["success"], json!(false));
        let reply = bridge
            .call(json!({ "id": unique("cmd"), "type": "list_workspaces" }))
            .await;
        assert_eq!(reply["success"], json!(false));
        // Settings handlers report the same store failure.
        let reply = bridge
            .call(json!({ "id": unique("cmd"), "type": "get_settings" }))
            .await;
        assert_eq!(reply["success"], json!(false));
        let reply = bridge
            .call(json!({ "id": unique("cmd"), "type": "set_approval_tier", "tier": "sandbox" }))
            .await;
        assert_eq!(reply["success"], json!(false));
        bridge.stop();
    }

    #[tokio::test]
    async fn history_commands_page_and_fail() {
        let _lock = mock_agent_lock();
        let (_home, bridge) = active_bridge("cmd-history").await;
        let agent = ensure_mock_agent();
        agent.clear_scripts();
        let session = unique("sess");
        let thread = crate::store::create_thread(crate::store::CreateThreadInput {
            mode: "chat".to_string(),
            title: Some("History".to_string()),
            workspace_id: None,
            workspace_path: None,
            workspace_name: None,
            agent_session_id: Some(session.clone()),
        })
        .unwrap();
        let history_run = crate::store::create_run(crate::store::CreateRunInput {
            id: Some(unique("run-history")),
            thread_id: thread.id,
            trigger_message_id: None,
            model_provider: None,
            model_id: None,
        })
        .unwrap();
        crate::store::update_run_status_if_active(crate::store::UpdateRunStatusInput {
            run_id: history_run.id.clone(),
            status: "failed".to_string(),
            error_message: None,
            error_type: None,
        })
        .unwrap();

        let reply = bridge
            .call(json!({ "id": unique("cmd"), "type": "get_messages", "sessionId": session }))
            .await;
        assert_eq!(reply["success"], json!(true));
        assert_eq!(reply["data"]["total"], json!(2));
        assert_eq!(reply["data"]["hasMore"], json!(false));

        // An explicit positive limit pages instead of using the default.
        let reply = bridge
            .call(json!({ "id": unique("cmd"), "type": "get_messages", "sessionId": session, "limit": 1 }))
            .await;
        assert_eq!(reply["success"], json!(true));
        assert_eq!(reply["data"]["messages"].as_array().unwrap().len(), 1);
        assert_eq!(reply["data"]["hasMore"], json!(true));

        // Agent failure → the remote-specific error text.
        agent.script_for(
            "get_messages",
            &session,
            false,
            json!(null),
            "agent exploded",
        );
        let reply = bridge
            .call(json!({ "id": unique("cmd"), "type": "get_messages", "sessionId": session }))
            .await;
        assert_eq!(reply["success"], json!(false));
        assert!(reply["error"]
            .as_str()
            .unwrap()
            .contains("conversation history is unavailable"));

        // Entries page through too.
        agent.set_session_entries(
            &session,
            json!({ "entries": [{
                "id": "persisted-assistant",
                "kind": "assistant",
                "role": "assistant",
                "blocks":[{"kind":"text","text":"partial"}],
                "createdAtMs":1000,"runId":history_run.id,"run":{"status":"failed","durationMs":1000,"error":"synthetic failure"}
            }] }),
        );
        let reply = bridge
            .call(
                json!({ "id": unique("cmd"), "type": "get_session_entries", "sessionId": session }),
            )
            .await;
        assert_eq!(reply["success"], json!(true));
        assert_eq!(reply["data"]["entries"].as_array().unwrap().len(), 1);
        assert_eq!(
            reply["data"]["entries"][0]["run"]["status"],
            json!("failed")
        );
        assert!(reply["data"]["entries"][0]["run"]["durationMs"].is_number());

        // Mobile's tail request is forwarded as an Agent backward cursor; the
        // desktop must not expand it back into a full-history forward loop.
        let reply = bridge
            .call(json!({
                    "id": unique("cmd"),
                    "type": "get_session_entries",
                    "sessionId": session,
                    "before": i64::MAX,
                    "limit": 10
            }))
            .await;
        assert_eq!(reply["success"], json!(true));
        assert_eq!(reply["data"]["entries"].as_array().unwrap().len(), 1);

        // Entries honor an explicit positive limit too.
        let reply = bridge
            .call(json!({ "id": unique("cmd"), "type": "get_session_entries", "sessionId": session, "limit": 5 }))
            .await;
        assert_eq!(reply["success"], json!(true));
        assert_eq!(reply["data"]["entries"].as_array().unwrap().len(), 1);

        // Production sidecar responses carry typed payload only (`data` is
        // empty); mobile history must still flow through the remote bridge.
        agent.script_typed_for(
            "get_session_entries",
            &session,
            json!({"entries": [{
                "id": "typed-mobile-entry",
                "role": "assistant",
                "kind":"assistant","createdAtMs":1000,"blocks":[{"kind":"text","text":"typed mobile history"}]
            }]}),
        );
        let reply = bridge
            .call(
                json!({ "id": unique("cmd"), "type": "get_session_entries", "sessionId": session }),
            )
            .await;
        assert_eq!(reply["success"], json!(true));
        assert_eq!(
            reply["data"]["entries"][0]["blocks"][0]["text"],
            json!("typed mobile history")
        );

        agent.script_for("get_session_entries", &session, false, json!(null), "nope");
        let reply = bridge
            .call(
                json!({ "id": unique("cmd"), "type": "get_session_entries", "sessionId": session }),
            )
            .await;
        assert_eq!(reply["success"], json!(false));

        // Events backfill (success and failure).
        let reply = bridge
            .call(json!({ "id": unique("cmd"), "type": "get_events_since", "sessionId": session, "runId": "run-1", "sinceIdx": -1 }))
            .await;
        assert_eq!(reply["success"], json!(true));
        assert_eq!(reply["data"]["events"], json!([]));
        // A positive limit uses the caller's page size (limit > 0 branch).
        let reply = bridge
            .call(json!({ "id": unique("cmd"), "type": "get_events_since", "sessionId": session, "runId": "run-1", "sinceIdx": -1, "limit": 5 }))
            .await;
        assert_eq!(reply["success"], json!(true));
        assert_eq!(reply["data"]["events"], json!([]));
        agent.script_for(
            "get_events_since",
            &session,
            false,
            json!(null),
            "stale run",
        );
        let reply = bridge
            .call(json!({ "id": unique("cmd"), "type": "get_events_since", "sessionId": session, "runId": "run-1", "sinceIdx": -1 }))
            .await;
        assert_eq!(reply["success"], json!(false));

        bridge.stop();
    }

    #[tokio::test]
    async fn replay_continuations_keep_agent_has_more_and_stop_at_the_pinned_watermark() {
        let _lock = mock_agent_lock();
        let (_home, bridge) = active_bridge("bounded-replay").await;
        let agent = ensure_mock_agent();
        agent.clear_scripts();
        let session = unique("session");
        let events = |start: i64, end: i64| -> Value {
            json!((start..=end)
                .map(|idx| json!({"type": "text_chunk", "idx": idx, "data": "{}"}))
                .collect::<Vec<_>>())
        };
        agent.script_typed_for(
            "get_events_since",
            &session,
            json!({
                "runId": "r", "events": events(0, 4), "hasMore": false
            }),
        );
        let first = bridge
            .call(json!({
                "id": unique("cmd"), "type": "get_events_since", "sessionId": session,
                "runId": "r", "sinceIdx": -1, "limit": 2
            }))
            .await;
        assert_eq!(first["success"], true);
        assert_eq!(first["data"]["watermark"], 4);
        assert_eq!(first["data"]["nextSinceIdx"], 1);
        assert_eq!(first["data"]["hasMore"], true);

        agent.script_typed_for(
            "get_events_since",
            &session,
            json!({
                "runId": "r", "events": events(2, 3), "hasMore": true
            }),
        );
        let middle = bridge
            .call(json!({
                "id": unique("cmd"), "type": "get_events_since", "sessionId": session,
                "runId": "r", "sinceIdx": 1, "limit": 2, "replayUntilIdx": 4
            }))
            .await;
        assert_eq!(middle["success"], true);
        assert_eq!(middle["data"]["watermark"], 4);
        assert_eq!(middle["data"]["nextSinceIdx"], 3);
        assert_eq!(
            middle["data"]["hasMore"], true,
            "Agent still has another page"
        );

        // The task kept generating, but this replay must finish at its
        // original watermark rather than chasing the growing tail forever.
        agent.script_typed_for(
            "get_events_since",
            &session,
            json!({
                "runId": "r", "events": events(4, 5), "hasMore": true
            }),
        );
        let last = bridge
            .call(json!({
                "id": unique("cmd"), "type": "get_events_since", "sessionId": session,
                "runId": "r", "sinceIdx": 3, "limit": 2, "replayUntilIdx": 4
            }))
            .await;
        assert_eq!(last["success"], true);
        assert_eq!(last["data"]["watermark"], 4);
        assert_eq!(last["data"]["nextSinceIdx"], 4);
        assert_eq!(last["data"]["hasMore"], false);
        assert_eq!(last["data"]["events"].as_array().unwrap().len(), 1);
        bridge.stop();
    }

    #[tokio::test]
    async fn transfer_control_commands() {
        let _lock = mock_agent_lock();
        let (_home, bridge) = active_bridge("cmd-transfer").await;
        let agent = ensure_mock_agent();
        agent.clear_scripts();

        // upload_init validation + success.
        let reply = bridge
            .call(json!({ "id": unique("cmd"), "type": "upload_init", "name": "a.txt", "kind": "file", "originalSize": 0, "transferSize": 0 }))
            .await;
        assert_eq!(reply["success"], json!(false));
        let reply = bridge
            .call(json!({ "id": unique("cmd"), "type": "upload_init", "name": "a.txt", "kind": "file", "originalSize": 4, "transferSize": 4 }))
            .await;
        assert_eq!(reply["success"], json!(true));
        let upload_id = reply["data"]["uploadId"].as_str().unwrap().to_string();

        // upload_complete on an incomplete upload → error; after the bytes, ok.
        let reply = bridge
            .call(
                json!({ "id": unique("cmd"), "type": "upload_complete", "transferId": upload_id }),
            )
            .await;
        assert_eq!(reply["success"], json!(false));
        transfer::write_upload_chunk(&upload_id, 0, b"data").unwrap();
        let reply = bridge
            .call(
                json!({ "id": unique("cmd"), "type": "upload_complete", "transferId": upload_id }),
            )
            .await;
        assert_eq!(reply["success"], json!(true), "got: {reply}");
        assert_eq!(reply["data"]["contentHash"].as_str().unwrap().len(), 64);

        // upload_cancel always succeeds.
        let reply = bridge
            .call(json!({ "id": unique("cmd"), "type": "upload_cancel", "transferId": upload_id }))
            .await;
        assert_eq!(reply["success"], json!(true));

        // download_prepare failure (unknown attachment) and success.
        let session = unique("sess");
        agent.set_session_entries(&session, json!({ "entries": [] }));
        let reply = bridge
            .call(json!({ "id": unique("cmd"), "type": "download_prepare", "sessionId": session, "filePath": "/tmp/none.txt" }))
            .await;
        assert_eq!(reply["success"], json!(false));

        let dir = std::env::temp_dir().join(unique("futureos-cmd-dl"));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("take.txt");
        std::fs::write(&file, b"take me").unwrap();
        agent.set_session_entries(
            &session,
            json!({"entries":[{"metadata":{"attachments":[{"path": file.to_string_lossy()}]}}]}),
        );
        let reply = bridge
            .call(json!({ "id": unique("cmd"), "type": "download_prepare", "sessionId": session, "filePath": file.to_string_lossy() }))
            .await;
        assert_eq!(reply["success"], json!(true), "got: {reply}");
        let transfer_id = reply["data"]["transferId"].as_str().unwrap().to_string();

        let reply = bridge
            .call(json!({ "id": unique("cmd"), "type": "download_cancel", "transferId": transfer_id }))
            .await;
        assert_eq!(reply["success"], json!(true));

        std::fs::remove_dir_all(dir).ok();
        bridge.stop();
    }

    #[tokio::test]
    async fn prompt_creates_threads_and_rejects_busy_sessions() {
        let _lock = mock_agent_lock();
        let (_home, bridge) = active_bridge("cmd-prompt").await;

        // Workspace mode without a workspace id → validation error.
        let reply = bridge
            .call(json!({ "id": unique("cmd"), "type": "prompt", "message": "hi", "mode": "workspace" }))
            .await;
        assert_eq!(reply["success"], json!(false));
        assert!(reply["error"]
            .as_str()
            .unwrap()
            .contains("Select a workspace"));

        // Chat mode with an empty session id → lazy thread + agent session.
        let prompt_id = unique("cmd");
        let reply = bridge
            .call(json!({ "id": prompt_id, "type": "prompt", "message": "hello there", "modelId": "m1", "providerId": "p1", "level": "high" }))
            .await;
        assert_eq!(reply["success"], json!(true), "got: {reply}");
        let session = reply["data"]["sessionId"].as_str().unwrap().to_string();
        assert!(session.starts_with("mock-session-"));
        let thread_id = reply["data"]["threadId"].as_str().unwrap().to_string();
        // The run the ack carried settles as failed: the mock agent's event
        // stream ends without agent_end.
        let run_id = reply["data"]["runId"].as_str().unwrap().to_string();
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        loop {
            let run = crate::store::get_run(&run_id).unwrap().expect("run row");
            if run.status != "running" {
                assert_eq!(run.status, "failed");
                break;
            }
            assert!(std::time::Instant::now() < deadline, "run never settled");
            tokio::time::sleep(Duration::from_millis(20)).await;
        }

        // The run id is a durable receipt keyed by the mobile command id. It
        // remains queryable after the in-memory single-flight cache expires,
        // and resending the prompt returns the same ack without a second run.
        let receipt = bridge
            .call(
                json!({ "id": unique("cmd"), "type": "get_prompt_receipt", "promptId": prompt_id }),
            )
            .await;
        assert_eq!(receipt["data"]["runId"], json!(run_id));
        tokio::time::sleep(Duration::from_millis(650)).await;
        let retried = bridge
            .call(json!({ "id": prompt_id, "type": "prompt", "message": "hello there" }))
            .await;
        assert_eq!(retried["data"]["runId"], json!(run_id));

        // A follow-up prompt on the idle session reuses the thread.
        let reply = bridge
            .call(json!({ "id": unique("cmd"), "type": "prompt", "sessionId": session, "message": "again" }))
            .await;
        assert_eq!(reply["success"], json!(true), "got: {reply}");
        assert_eq!(reply["data"]["threadId"], json!(thread_id));

        // While a run for the session is active, another prompt is refused.
        crate::store::create_run(crate::store::CreateRunInput {
            id: None,
            thread_id: thread_id.clone(),
            trigger_message_id: None,
            model_provider: None,
            model_id: None,
        })
        .unwrap();
        let reply = bridge
            .call(json!({ "id": unique("cmd"), "type": "prompt", "sessionId": session, "message": "too soon" }))
            .await;
        assert_eq!(reply["success"], json!(false));
        assert!(reply["error"].as_str().unwrap().contains("still running"));

        bridge.stop();
    }

    #[tokio::test]
    async fn prompt_workspace_mode_and_prepare_failures() {
        let _lock = mock_agent_lock();
        let (_home, bridge) = active_bridge("cmd-prompt-ws").await;
        let agent = ensure_mock_agent();
        agent.clear_scripts();

        // Workspace mode with a real workspace id → thread bound to it.
        let workspace_dir = std::env::temp_dir().join(unique("futureos-ws-prompt"));
        std::fs::create_dir_all(&workspace_dir).unwrap();
        let workspace = crate::store::create_workspace(crate::store::CreateWorkspaceInput {
            name: Some("Prompt WS".to_string()),
            path: workspace_dir.to_string_lossy().to_string(),
            description: None,
            create_directory: None,
        })
        .unwrap();
        let reply = bridge
            .call(json!({
                "id": unique("cmd"), "type": "prompt", "message": "workspace hello",
                "mode": "workspace", "workspaceId": workspace.id,
            }))
            .await;
        assert_eq!(reply["success"], json!(true), "got: {reply}");
        let ws_thread = reply["data"]["threadId"].as_str().unwrap().to_string();
        assert_eq!(
            crate::store::list_threads()
                .unwrap()
                .iter()
                .find(|t| t.id == ws_thread)
                .map(|t| t.mode.as_str()),
            Some("workspace")
        );

        // Agent-side session provisioning failure → the just-created orphan
        // thread is removed and the error surfaces.
        agent.script(
            "new_session",
            false,
            json!(null),
            "agent rejected the session",
        );
        let before = crate::store::list_threads().unwrap().len();
        let reply = bridge
            .call(json!({ "id": unique("cmd"), "type": "prompt", "message": "fail provisioning" }))
            .await;
        assert_eq!(reply["success"], json!(false));
        assert!(reply["error"].as_str().unwrap().contains("rejected"));
        assert_eq!(crate::store::list_threads().unwrap().len(), before);

        // Prepare failure after attachments were claimed → the claimed copies
        // roll back (no leaked files under the thread's image dir).
        let reply = bridge
            .call(json!({ "id": unique("cmd"), "type": "upload_init", "name": "note.txt", "kind": "file", "originalSize": 3, "transferSize": 3 }))
            .await;
        assert_eq!(reply["success"], json!(true), "got: {reply}");
        let upload_id = reply["data"]["uploadId"].as_str().unwrap().to_string();
        transfer::write_upload_chunk(&upload_id, 0, b"hey").unwrap();
        transfer::complete_upload(&upload_id).unwrap();
        INJECT_PREPARE_FAILURE.store(true, Ordering::Relaxed);
        let reply = bridge
            .call(json!({
                "id": unique("cmd"), "type": "prompt", "message": "with attachment",
                "attachments": [{ "uploadId": upload_id }],
            }))
            .await;
        assert_eq!(reply["success"], json!(false));
        assert!(reply["error"]
            .as_str()
            .unwrap()
            .contains("injected prepare failure"));
        let thread = crate::store::list_threads()
            .unwrap()
            .into_iter()
            .find(|t| t.title == "with attachment")
            .expect("thread for the failed prepare");
        let origin = crate::store::thread_images_dir(&thread.id)
            .unwrap()
            .join("origin");
        let leaked = origin
            .read_dir()
            .map(|mut entries| entries.next().is_some())
            .unwrap_or(false);
        assert!(!leaked, "claimed attachment copies must roll back");

        std::fs::remove_dir_all(&workspace_dir).ok();
        bridge.stop();
    }

    #[tokio::test]
    async fn session_control_commands() {
        let _lock = mock_agent_lock();
        let (_home, bridge) = active_bridge("cmd-session-ctl").await;
        let agent = ensure_mock_agent();
        agent.clear_scripts();
        let session = unique("sess");

        // abort: success and agent failure.
        let reply = bridge
            .call(json!({ "id": unique("cmd"), "type": "abort", "sessionId": session }))
            .await;
        assert_eq!(reply["success"], json!(true), "got: {reply}");
        agent.script_for("abort", &session, false, json!(null), "cannot abort");
        let reply = bridge
            .call(json!({ "id": unique("cmd"), "type": "abort", "sessionId": session }))
            .await;
        assert_eq!(reply["success"], json!(false));

        // get_state.
        let reply = bridge
            .call(json!({ "id": unique("cmd"), "type": "get_state", "sessionId": session }))
            .await;
        assert_eq!(reply["success"], json!(true));
        agent.script_for("get_state", &session, false, json!(null), "gone");
        let reply = bridge
            .call(json!({ "id": unique("cmd"), "type": "get_state", "sessionId": session }))
            .await;
        assert_eq!(reply["success"], json!(false));

        // list_models / get_available_models share a handler.
        for cmd_type in ["list_models", "get_available_models"] {
            let reply = bridge
                .call(json!({ "id": unique("cmd"), "type": cmd_type }))
                .await;
            assert_eq!(reply["success"], json!(true), "{cmd_type}: {reply}");
        }
        agent.script("list_models", false, json!(null), "no models");
        let reply = bridge
            .call(json!({ "id": unique("cmd"), "type": "list_models" }))
            .await;
        assert_eq!(reply["success"], json!(false));

        // set_model / set_thinking_level / set_session_name (success + failure).
        let reply = bridge
            .call(json!({ "id": unique("cmd"), "type": "set_model", "sessionId": session, "modelId": "m1", "providerId": "p1" }))
            .await;
        assert_eq!(reply["success"], json!(true));
        assert!(agent.served("set_model", &session));
        agent.script_for("set_model", &session, false, json!(null), "bad model");
        let reply = bridge
            .call(json!({ "id": unique("cmd"), "type": "set_model", "sessionId": session, "modelId": "m1" }))
            .await;
        assert_eq!(reply["success"], json!(false));

        let reply = bridge
            .call(json!({ "id": unique("cmd"), "type": "set_thinking_level", "sessionId": session, "level": "high" }))
            .await;
        assert_eq!(reply["success"], json!(true));
        agent.script_for(
            "set_thinking_level",
            &session,
            false,
            json!(null),
            "bad level",
        );
        let reply = bridge
            .call(json!({ "id": unique("cmd"), "type": "set_thinking_level", "sessionId": session, "level": "high" }))
            .await;
        assert_eq!(reply["success"], json!(false));

        let reply = bridge
            .call(json!({ "id": unique("cmd"), "type": "set_session_name", "sessionId": session, "name": "Renamed" }))
            .await;
        assert_eq!(reply["success"], json!(true));
        agent.script_for(
            "set_session_name",
            &session,
            false,
            json!(null),
            "no rename",
        );
        let reply = bridge
            .call(json!({ "id": unique("cmd"), "type": "set_session_name", "sessionId": session, "name": "Renamed" }))
            .await;
        assert_eq!(reply["success"], json!(false));

        // A rename that resolves to a GUI thread emits remote activity
        // (the Ok(Some) find_thread_by_agent_session branch).
        let thread = crate::store::create_thread(crate::store::CreateThreadInput {
            mode: "chat".to_string(),
            title: Some("Phone".to_string()),
            workspace_id: None,
            workspace_path: None,
            workspace_name: None,
            agent_session_id: Some(session.clone()),
        })
        .unwrap();
        let reply = bridge
            .call(json!({ "id": unique("cmd"), "type": "set_session_name", "sessionId": session, "name": "Renamed" }))
            .await;
        assert_eq!(reply["success"], json!(true));

        // set_session_pinned: success and unknown-thread failure.
        let reply = bridge
            .call(json!({ "id": unique("cmd"), "type": "set_session_pinned", "threadId": thread.id, "pinned": true }))
            .await;
        assert_eq!(reply["success"], json!(true), "got: {reply}");
        let reply = bridge
            .call(json!({ "id": unique("cmd"), "type": "set_session_pinned", "threadId": "missing-thread", "pinned": false }))
            .await;
        assert_eq!(reply["success"], json!(false));

        // delete_session: missing id, success, and unknown-thread failure.
        let reply = bridge
            .call(json!({ "id": unique("cmd"), "type": "delete_session" }))
            .await;
        assert_eq!(reply["success"], json!(false));
        assert!(reply["error"]
            .as_str()
            .unwrap()
            .contains("missing thread_id"));
        let reply = bridge
            .call(json!({ "id": unique("cmd"), "type": "delete_session", "threadId": thread.id }))
            .await;
        assert_eq!(reply["success"], json!(true));
        let reply = bridge
            .call(json!({ "id": unique("cmd"), "type": "delete_session", "threadId": "missing-thread" }))
            .await;
        assert_eq!(reply["success"], json!(false));

        // Unknown command type.
        let reply = bridge
            .call(json!({ "id": unique("cmd"), "type": "teleport", "sessionId": session }))
            .await;
        assert_eq!(reply["success"], json!(false));
        assert!(reply["error"]
            .as_str()
            .unwrap()
            .contains("Unsupported command"));

        bridge.stop();
    }

    #[tokio::test]
    async fn delete_workspace_command_cascades_and_validates() {
        let _lock = mock_agent_lock();
        let (_home, bridge) = active_bridge("cmd-delete-ws").await;

        // A missing id is a malformed request, not a deletion.
        let reply = bridge
            .call(json!({ "id": unique("cmd"), "type": "delete_workspace" }))
            .await;
        assert_eq!(reply["success"], json!(false));
        assert!(reply["error"]
            .as_str()
            .unwrap()
            .contains("missing workspace_id"));

        // A workspace and the threads inside it go together; the user's own
        // directory on disk is never touched.
        let workspace_dir = std::env::temp_dir().join(unique("futureos-ws-delete"));
        std::fs::create_dir_all(&workspace_dir).unwrap();
        let workspace = crate::store::create_workspace(crate::store::CreateWorkspaceInput {
            name: Some("Phone Delete WS".to_string()),
            path: workspace_dir.to_string_lossy().to_string(),
            description: None,
            create_directory: None,
        })
        .unwrap();
        crate::store::create_thread(crate::store::CreateThreadInput {
            mode: "workspace".to_string(),
            title: Some("In workspace".to_string()),
            workspace_id: Some(workspace.id.clone()),
            workspace_path: None,
            workspace_name: None,
            agent_session_id: None,
        })
        .unwrap();
        assert_eq!(crate::store::list_workspaces().unwrap().len(), 1);
        assert_eq!(crate::store::list_threads().unwrap().len(), 1);

        let reply = bridge
            .call(json!({ "id": unique("cmd"), "type": "delete_workspace", "workspaceId": workspace.id.clone() }))
            .await;
        assert_eq!(reply["success"], json!(true), "got: {reply}");
        assert!(crate::store::list_workspaces().unwrap().is_empty());
        assert!(crate::store::list_threads().unwrap().is_empty());
        assert!(
            workspace_dir.exists(),
            "user files survive a workspace delete"
        );

        // The workspace is gone, so a repeat delete is an error.
        let reply = bridge
            .call(json!({ "id": unique("cmd"), "type": "delete_workspace", "workspaceId": workspace.id }))
            .await;
        assert_eq!(reply["success"], json!(false));

        bridge.stop();
    }

    #[tokio::test]
    async fn settings_commands_read_and_update() {
        let _lock = mock_agent_lock();
        let (_home, bridge) = active_bridge("cmd-settings").await;

        let reply = bridge
            .call(json!({ "id": unique("cmd"), "type": "get_settings" }))
            .await;
        assert_eq!(reply["success"], json!(true), "got: {reply}");
        assert_eq!(reply["data"]["approvalTier"], json!("off"));
        assert_eq!(
            reply["data"]["sandboxAvailable"],
            json!(cfg!(target_os = "macos"))
        );

        let reply = bridge
            .call(json!({ "id": unique("cmd"), "type": "set_approval_tier", "tier": "sandbox" }))
            .await;
        assert_eq!(reply["success"], json!(true), "got: {reply}");
        #[cfg(target_os = "macos")]
        assert_eq!(reply["data"]["approvalTier"], json!("sandbox"));
        #[cfg(not(target_os = "macos"))]
        assert_eq!(reply["data"]["approvalTier"], json!("manual"));

        // A non-sandbox tier takes the else branch (no host probe at all).
        let reply = bridge
            .call(json!({ "id": unique("cmd"), "type": "set_approval_tier", "tier": "manual" }))
            .await;
        assert_eq!(reply["success"], json!(true), "got: {reply}");
        assert_eq!(reply["data"]["approvalTier"], json!("manual"));

        bridge.stop();
    }

    #[tokio::test]
    async fn continue_run_resumes_a_failed_run_and_rejects_busy_sessions() {
        let _lock = mock_agent_lock();
        let (_home, bridge) = active_bridge("cmd-continue").await;
        let agent = ensure_mock_agent();
        let session = unique("sess");

        // A thread bound to the session + a failed run with terminal events.
        let thread = crate::store::create_thread(crate::store::CreateThreadInput {
            mode: "chat".to_string(),
            title: Some("Continue".to_string()),
            workspace_id: None,
            workspace_path: None,
            workspace_name: None,
            agent_session_id: Some(session.clone()),
        })
        .unwrap();
        let run = crate::store::create_run(crate::store::CreateRunInput {
            id: Some("run-failed".to_string()),
            thread_id: thread.id.clone(),
            trigger_message_id: None,
            model_provider: None,
            model_id: None,
        })
        .unwrap();
        crate::store::append_run_event(crate::store::AppendRunEventInput {
            run_id: run.id.clone(),
            event_type: "tool_result".to_string(),
            payload: Some(r#"{"text":"did the thing"}"#.to_string()),
            sequence: 1,
        })
        .unwrap();
        crate::store::flush_run_event_log_for_test(&run.id);
        crate::store::update_run_status_if_active(crate::store::UpdateRunStatusInput {
            run_id: run.id.clone(),
            status: "failed".to_string(),
            error_message: None,
            error_type: None,
        })
        .unwrap();

        // Reject the Ok arm's spawned `run_prepared_prompt` at session
        // provisioning (new_session) so it fails BEFORE spawning a session
        // observer — the ack path (prepare_remote_prompt) is store-only and
        // unaffected. This keeps the shared mock script clean for later tests
        // (no zombie) without touching the process-global agent endpoint.
        agent.script("new_session", false, json!(null), "rejected");

        // Ok arm: continue the failed run → ack carries the fresh run ids.
        let command_id = unique("cmd");
        let reply = bridge
            .call(json!({ "id": command_id, "type": "continue_run", "sessionId": session, "runId": run.id }))
            .await;
        assert_eq!(reply["success"], json!(true), "got: {reply}");
        assert_eq!(reply["data"]["sessionId"], json!(session));
        assert_eq!(reply["data"]["threadId"], json!(thread.id));

        // The spawned pipeline settles the new run as failed (its session
        // provisioning is rejected) — poll until the run leaves the running state.
        let continued_run = reply["data"]["runId"].as_str().unwrap().to_string();

        // Let the in-memory single-flight cache expire. The same command must
        // still resolve to the persisted run receipt instead of creating a
        // second continuation.
        tokio::time::sleep(reply_slot_ttl() + Duration::from_millis(50)).await;
        let retry = bridge
            .call(json!({ "id": command_id, "type": "continue_run", "sessionId": session, "runId": run.id }))
            .await;
        assert_eq!(retry["success"], json!(true), "got: {retry}");
        assert_eq!(retry["data"]["runId"], json!(continued_run));

        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        loop {
            let row = crate::store::get_run(&continued_run)
                .unwrap()
                .expect("continued run");
            if row.status != "running" {
                assert_eq!(row.status, "failed");
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "continued run never settled"
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }

        bridge.stop();
    }

    #[tokio::test]
    async fn continue_run_rejects_a_busy_session() {
        let _lock = mock_agent_lock();
        let (_home, bridge) = active_bridge("cmd-continue-busy").await;
        let session = unique("sess");

        let thread = crate::store::create_thread(crate::store::CreateThreadInput {
            mode: "chat".to_string(),
            title: Some("Busy".to_string()),
            workspace_id: None,
            workspace_path: None,
            workspace_name: None,
            agent_session_id: Some(session.clone()),
        })
        .unwrap();
        let source = crate::store::create_run(crate::store::CreateRunInput {
            id: Some("run-failed-source".to_string()),
            thread_id: thread.id.clone(),
            trigger_message_id: None,
            model_provider: None,
            model_id: None,
        })
        .unwrap();
        crate::store::update_run_status_if_active(crate::store::UpdateRunStatusInput {
            run_id: source.id.clone(),
            status: "failed".to_string(),
            error_message: Some("boom".to_string()),
            error_type: Some("model_failed".to_string()),
        })
        .unwrap();
        // A separate active run on the session → the busy guard fires before
        // anything is persisted or spawned.
        crate::store::create_run(crate::store::CreateRunInput {
            id: Some("run-active".to_string()),
            thread_id: thread.id.clone(),
            trigger_message_id: None,
            model_provider: None,
            model_id: None,
        })
        .unwrap();

        let reply = bridge
            .call(json!({ "id": unique("cmd"), "type": "continue_run", "sessionId": session, "runId": source.id }))
            .await;
        assert_eq!(reply["success"], json!(false));
        assert!(reply["error"].as_str().unwrap().contains("still running"));

        bridge.stop();
    }

    #[tokio::test]
    async fn continue_run_rejects_missing_wrong_session_and_nonfailed_sources() {
        let _lock = mock_agent_lock();
        let (_home, bridge) = active_bridge("cmd-continue-source-guards").await;
        let session = unique("sess");
        let thread = crate::store::create_thread(crate::store::CreateThreadInput {
            mode: "chat".to_string(),
            title: Some("Guarded continuation".to_string()),
            workspace_id: None,
            workspace_path: None,
            workspace_name: None,
            agent_session_id: Some(session.clone()),
        })
        .unwrap();
        let completed = crate::store::create_run(crate::store::CreateRunInput {
            id: Some(unique("run-completed")),
            thread_id: thread.id.clone(),
            trigger_message_id: None,
            model_provider: None,
            model_id: None,
        })
        .unwrap();
        crate::store::update_run_status_if_active(crate::store::UpdateRunStatusInput {
            run_id: completed.id.clone(),
            status: "completed".to_string(),
            error_message: None,
            error_type: None,
        })
        .unwrap();

        let missing = bridge
            .call(json!({ "id": unique("cmd"), "type": "continue_run", "sessionId": session, "runId": "missing-run" }))
            .await;
        assert_eq!(missing["success"], json!(false));
        assert!(missing["error"]
            .as_str()
            .unwrap()
            .contains("no longer exists"));

        let nonfailed = bridge
            .call(json!({ "id": unique("cmd"), "type": "continue_run", "sessionId": session, "runId": completed.id }))
            .await;
        assert_eq!(nonfailed["success"], json!(false));
        assert!(nonfailed["error"].as_str().unwrap().contains("failed run"));

        let failed = crate::store::create_run(crate::store::CreateRunInput {
            id: Some(unique("run-failed")),
            thread_id: thread.id,
            trigger_message_id: None,
            model_provider: None,
            model_id: None,
        })
        .unwrap();
        crate::store::update_run_status_if_active(crate::store::UpdateRunStatusInput {
            run_id: failed.id.clone(),
            status: "failed".to_string(),
            error_message: Some("boom".to_string()),
            error_type: Some("model_failed".to_string()),
        })
        .unwrap();
        let wrong_session = bridge
            .call(json!({ "id": unique("cmd"), "type": "continue_run", "sessionId": "another-session", "runId": failed.id }))
            .await;
        assert_eq!(wrong_session["success"], json!(false));
        assert!(wrong_session["error"]
            .as_str()
            .unwrap()
            .contains("does not belong"));

        bridge.stop();
    }

    #[tokio::test]
    async fn build_continue_prompt_folds_recent_terminal_events() {
        let _lock = mock_agent_lock();
        let _home = HomeGuard::new("cmd-continue-prompt");
        init_store();
        let thread = crate::store::create_thread(crate::store::CreateThreadInput {
            mode: "chat".to_string(),
            title: Some("Prompt".to_string()),
            workspace_id: None,
            workspace_path: None,
            workspace_name: None,
            agent_session_id: None,
        })
        .unwrap();
        let run = crate::store::create_run(crate::store::CreateRunInput {
            id: None,
            thread_id: thread.id.clone(),
            trigger_message_id: None,
            model_provider: None,
            model_id: None,
        })
        .unwrap();
        // A non-terminal event is ignored; terminal events are folded newest-first.
        crate::store::append_run_event(crate::store::AppendRunEventInput {
            run_id: run.id.clone(),
            event_type: "text_chunk".to_string(),
            payload: Some("ignored".to_string()),
            sequence: 1,
        })
        .unwrap();
        crate::store::append_run_event(crate::store::AppendRunEventInput {
            run_id: run.id.clone(),
            event_type: "tool_result".to_string(),
            payload: Some("tool output".to_string()),
            sequence: 2,
        })
        .unwrap();
        crate::store::append_run_event(crate::store::AppendRunEventInput {
            run_id: run.id.clone(),
            event_type: "error".to_string(),
            payload: Some("boom".to_string()),
            sequence: 3,
        })
        .unwrap();
        crate::store::flush_run_event_log_for_test(&run.id);

        let prompt = build_continue_prompt(&run.id);
        assert!(prompt.contains("继续上一个任务。"), "{prompt}");
        assert!(prompt.contains("已执行内容摘要:"), "{prompt}");
        assert!(prompt.contains("error"), "{prompt}");
        assert!(prompt.contains("tool_result"), "{prompt}");
        assert!(
            !prompt.contains("text_chunk"),
            "non-terminal ignored: {prompt}"
        );

        // An unknown run (no events) still yields the default continue prompt.
        let empty = build_continue_prompt("no-such-run");
        assert_eq!(empty, "继续上一个任务。");
    }

    #[tokio::test]
    async fn approval_decision_ownership_and_outcomes() {
        let _lock = mock_agent_lock();
        let (_home, bridge) = active_bridge("cmd-approval").await;
        let agent = ensure_mock_agent();
        agent.clear_scripts();
        let session = unique("sess");

        // Unknown approval request.
        let reply = bridge
            .call(json!({ "id": unique("cmd"), "type": "approval_decision", "sessionId": session, "entryId": "nope", "mode": "approved" }))
            .await;
        assert_eq!(reply["success"], json!(false));
        assert!(reply["error"]
            .as_str()
            .unwrap()
            .contains("could not be loaded"));

        // A real approval owned by a DIFFERENT session → ownership mismatch.
        let thread = crate::store::create_thread(crate::store::CreateThreadInput {
            mode: "chat".to_string(),
            title: None,
            workspace_id: None,
            workspace_path: None,
            workspace_name: None,
            agent_session_id: Some(session.clone()),
        })
        .unwrap();
        let run = crate::store::create_run(crate::store::CreateRunInput {
            id: None,
            thread_id: thread.id.clone(),
            trigger_message_id: None,
            model_provider: None,
            model_id: None,
        })
        .unwrap();
        crate::store::ensure_approval_request(crate::store::EnsureApprovalRequestInput {
            approval_request_id: Some("appr-1".to_string()),
            run_id: run.id.clone(),
            tool_call_id: Some("tool-1".to_string()),
            kind: "command".to_string(),
            title: "Run ls".to_string(),
            summary: None,
            risk_level: None,
            requested_action: None,
            action_category: None,
            action_payload: None,
            sandbox_boundary: None,
            save_suggestion: None,
            reviewer: None,
        })
        .unwrap();

        let reply = bridge
            .call(json!({ "id": unique("cmd"), "type": "approval_decision", "sessionId": "someone-else", "entryId": "appr-1", "mode": "approved" }))
            .await;
        assert_eq!(reply["success"], json!(false));
        assert!(reply["error"].as_str().unwrap().contains("does not belong"));

        // Owning session decides → the agent is notified and the reply is ok.
        let reply = bridge
            .call(json!({ "id": unique("cmd"), "type": "approval_decision", "sessionId": session, "entryId": "appr-1", "mode": "approved" }))
            .await;
        assert_eq!(reply["success"], json!(true), "got: {reply}");
        assert!(agent.served("approval_decision", &session));
        let record = crate::store::get_approval_request("appr-1")
            .unwrap()
            .unwrap();
        assert_eq!(record.status, "approved");

        // An agent-side stale approval cancels locally but still replies ok.
        crate::store::ensure_approval_request(crate::store::EnsureApprovalRequestInput {
            approval_request_id: Some("appr-2".to_string()),
            run_id: run.id.clone(),
            tool_call_id: Some("tool-2".to_string()),
            kind: "command".to_string(),
            title: "Run pwd".to_string(),
            summary: None,
            risk_level: None,
            requested_action: None,
            action_category: None,
            action_payload: None,
            sandbox_boundary: None,
            save_suggestion: None,
            reviewer: None,
        })
        .unwrap();
        agent.script_for(
            "approval_decision",
            &session,
            false,
            json!(null),
            "approval request is not pending",
        );
        let reply = bridge
            .call(json!({ "id": unique("cmd"), "type": "approval_decision", "sessionId": session, "entryId": "appr-2", "mode": "approved" }))
            .await;
        assert_eq!(reply["success"], json!(true), "got: {reply}");
        let record = crate::store::get_approval_request("appr-2")
            .unwrap()
            .unwrap();
        assert_eq!(record.status, "cancelled");

        // A genuine agent rejection surfaces as an error.
        crate::store::ensure_approval_request(crate::store::EnsureApprovalRequestInput {
            approval_request_id: Some("appr-3".to_string()),
            run_id: run.id.clone(),
            tool_call_id: Some("tool-3".to_string()),
            kind: "command".to_string(),
            title: "Run rm".to_string(),
            summary: None,
            risk_level: None,
            requested_action: None,
            action_category: None,
            action_payload: None,
            sandbox_boundary: None,
            save_suggestion: None,
            reviewer: None,
        })
        .unwrap();
        agent.script_for("approval_decision", &session, false, json!(null), "boom");
        let reply = bridge
            .call(json!({ "id": unique("cmd"), "type": "approval_decision", "sessionId": session, "entryId": "appr-3", "mode": "denied" }))
            .await;
        assert_eq!(reply["success"], json!(false));

        bridge.stop();
    }

    #[tokio::test]
    async fn duplicate_command_ids_get_one_execution_and_cached_replies() {
        let _lock = mock_agent_lock();
        let (_home, bridge) = active_bridge("cmd-singleflight").await;
        let agent = ensure_mock_agent();
        agent.clear_scripts();
        let session = unique("sess");

        let command_id = unique("cmdid");
        let cmd = || json!({ "id": command_id, "type": "get_state", "sessionId": session });
        // Two concurrent deliveries → a single agent call, identical replies.
        let (first, second) = tokio::join!(bridge.call(cmd()), bridge.call(cmd()));
        assert_eq!(first, second);
        let executions = agent
            .requests()
            .iter()
            .filter(|(command, sid)| command == "get_state" && sid == &session)
            .count();
        assert_eq!(executions, 1, "retried command must not re-execute");
        let conflict = bridge
            .call(json!({ "id": command_id, "type": "delete_session", "sessionId": session }))
            .await;
        assert_eq!(conflict["error"], json!("command_id_conflict"));

        // After the reply-slot TTL the entry expires and the command runs again.
        tokio::time::sleep(Duration::from_millis(650)).await;
        let third = bridge.call(cmd()).await;
        assert_eq!(third["success"], json!(true));
        let executions = agent
            .requests()
            .iter()
            .filter(|(command, sid)| command == "get_state" && sid == &session)
            .count();
        assert_eq!(executions, 2, "expired cache entries re-execute");

        // A cached reply replayed to a delivery WITHOUT a reply subject is
        // simply dropped (no publish, no panic).
        bridge.nats.inject(
            &format!("p.{}.cmd.rpc", bridge.pair_id),
            None,
            serde_json::to_vec(&cmd()).unwrap(),
        );
        tokio::time::sleep(Duration::from_millis(150)).await;
        bridge.stop();
    }

    #[tokio::test]
    async fn command_loop_exits_for_generation_supervisor_when_stream_ends() {
        let _home = HomeGuard::new("cmd-self-heal");
        let nats = FakeNats::start().await;
        let client = nats_connect_once(&nats).await;
        let creds = bridge_creds();
        let pair_id = creds.pair_id.clone();
        let handshake = HandshakeState::new(
            creds,
            Arc::new(AtomicBool::new(true)),
            format!("bridge_{}", unique("cmd")),
        );
        let handle = tokio::spawn(command_loop(
            client,
            pair_id.clone(),
            new_reply_slots(),
            handshake,
        ));
        nats.wait_for_sub(&format!("p.{pair_id}.cmd.>"), Duration::from_secs(5))
            .await;
        // A critical subscription is generation-local. The loop exits so the
        // single runtime supervisor can apply its 10-minute recovery budget.
        nats.kill();
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert!(
            handle.is_finished(),
            "supervisor must observe the dead task"
        );
    }

    #[tokio::test]
    async fn command_loop_subscribe_failure_logs_and_returns() {
        let nats = FakeNats::start().await;
        let client = nats_connect_once(&nats).await;
        // Sever the connection before the loop subscribes → `queue_subscribe`
        // fails and the loop returns without panicking (the error is logged).
        nats.kill();
        let creds = bridge_creds();
        let pair_id = creds.pair_id.clone();
        let handshake = HandshakeState::new(
            creds,
            Arc::new(AtomicBool::new(false)),
            format!("bridge_{}", unique("cmd")),
        );
        command_loop(client, pair_id, new_reply_slots(), handshake).await;
    }

    #[tokio::test]
    async fn unpair_command_acknowledges_and_spawns_cleanup() {
        let _lock = mock_agent_lock();
        let (_home, bridge) = active_bridge("cmd-unpair").await;
        let reply = bridge
            .call(json!({ "id": unique("cmd"), "type": "unpair" }))
            .await;
        assert_eq!(reply["success"], json!(true));
        // Give the spawned cleanup a moment to run (it is a no-op without a
        // persisted pairing).
        tokio::time::sleep(Duration::from_millis(50)).await;
        bridge.stop();
    }

    #[test]
    fn history_keeps_agent_outcome_without_desktop_inference() {
        let entry = json!({"id":"m","kind":"assistant","role":"assistant","runId":"run","createdAtMs":1000,"blocks":[],"run":{"status":"failed","error":"synthetic","durationMs":7}});
        let page = prepare_backward_entries_page(
            "unmirrored",
            json!({"entries":[entry.clone()],"hasMore":false,"nextOffset":0}),
        );
        assert_eq!(page["entries"][0], entry);
    }
}

struct NatsReply<'a> {
    client: &'a async_nats::Client,
    msg: &'a async_nats::Message,
}
impl ReplySink for NatsReply<'_> {
    fn send<'a>(
        &'a self,
        success: bool,
        data: Value,
        error: Option<String>,
    ) -> futures::future::BoxFuture<'a, ()> {
        Box::pin(async move { reply(self.client, self.msg, success, data, error.as_deref()).await })
    }
}
