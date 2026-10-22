//! Narrow fixture seams; absent from production default features.
use super::*;
pub fn with_gzip<T>(enabled: bool, run: impl FnOnce() -> T) -> T {
    REPLY_GZIP.sync_scope(enabled, run)
}
pub fn reply_gzip_allowed() -> bool {
    super::reply_gzip_allowed()
}
pub fn encode_reply_payload(body: &Value) -> Vec<u8> {
    super::encode_reply_payload(body)
}
pub fn set_business(state: &mut HandshakeState, host: Arc<dyn BusinessHost>) {
    state.host = host;
}
pub fn pending_count(state: &HandshakeState) -> usize {
    state.pending.lock().unwrap().len()
}
pub fn insert_pending(
    state: &HandshakeState,
    nonce: String,
    transcript: String,
    device_id: String,
    client_public_key: String,
    created_at: std::time::Instant,
) {
    state.pending.lock().unwrap().insert(
        nonce,
        PendingHandshake {
            transcript,
            device_id,
            client_public_key,
            created_at,
        },
    );
}
/// Insert a slot for a delivery that was descheduled before executing it.
///
/// `ttl` is the connection's reply TTL: the slot has to expire on the same
/// schedule a real first delivery would have given it, or a fixture that waits
/// past the TTL would still be served from cache.
pub fn reply_ttl(state: &HandshakeState) -> std::time::Duration {
    state.reply_ttl
}
pub fn insert_reply_slot(
    slots: &ReplySlots,
    id: String,
    request: Value,
    ttl: std::time::Duration,
) -> Arc<tokio::sync::Mutex<Option<Vec<u8>>>> {
    let response = Arc::new(tokio::sync::Mutex::new(None));
    slots.0.lock().unwrap().insert(
        id,
        Arc::new(CachedReply {
            request,
            response: response.clone(),
            expires_at: std::time::Instant::now() + ttl,
        }),
    );
    response
}
pub(crate) fn record_client_identity(
    runtime: &dyn CommandRuntime,
    pair: &str,
    command: &IncomingCmd,
) {
    super::record_client_identity(runtime, pair, command);
}
pub async fn handle_command(
    client: &async_nats::Client,
    msg: async_nats::Message,
    state: HandshakeState,
) {
    super::handle_command(client, msg, state).await;
}
/// The destructive half of a phone-initiated unpair, after its acknowledgement.
///
/// Exposed so a test can call it under a *known* epoch instead of racing the
/// detached task the command spawns: the guard it carries is only observable if
/// the epoch it sees is decided before it starts.
pub async fn finish_phone_unpair(state: HandshakeState, pair_id: String) {
    super::finish_phone_unpair(state, pair_id).await;
}
pub async fn handle_command_singleflight(
    client: &async_nats::Client,
    msg: async_nats::Message,
    slots: ReplySlots,
    state: HandshakeState,
) {
    super::handle_command_singleflight(client, msg, slots, state).await;
}
pub async fn handle_pair_handshake(
    client: &async_nats::Client,
    msg: &async_nats::Message,
    command: &IncomingCmd,
    state: &HandshakeState,
) {
    super::handle_pair_handshake(client, msg, command, state).await;
}
pub async fn handle_pair_handshake_confirm(
    client: &async_nats::Client,
    msg: &async_nats::Message,
    command: &IncomingCmd,
    state: &HandshakeState,
) {
    super::handle_pair_handshake_confirm(client, msg, command, state).await;
}
pub async fn publish_reply_payload(
    client: &async_nats::Client,
    msg: &async_nats::Message,
    bytes: Vec<u8>,
) {
    super::publish_reply_payload(client, msg, bytes).await;
}
