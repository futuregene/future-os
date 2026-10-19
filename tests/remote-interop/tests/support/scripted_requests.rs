//! Migrated Client transport fault tests: real broker/records, scripted responder.
//! These do not replace the real Client/Server interoperability flows.
use future_remote_client::session;
use future_remote_interop::Broker;
use futures::StreamExt;
use serde_json::json;
use std::{sync::Arc, time::Duration};

/**
 * A host the test scripts, speaking the real protocol.
 *
 * It opens each request with the peer's channel and seals the reply against the
 * request, which is what the real host does — so the client's seal/open path is
 * exercised, not stubbed. The scripted responder chooses whether a reply comes
 * back at all and which attempt it belongs to. A healthy broker cannot produce
 * this loss on demand.
 *
 * It is also the only place a request is readable — the client seals it with a
 * key the test's peer never opens with (send and receive keys differ), so "the
 * retry re-sends the same command" can only be asserted from this side.
 */
struct ScriptedHost {
    /// One entry per request that arrived, decrypted and in order.
    requests: Arc<std::sync::Mutex<Vec<serde_json::Value>>>,
    /// Keeps the broker alive for the life of the host handle.
    _nats: Broker,
    task: tokio::task::JoinHandle<()>,
}

impl ScriptedHost {
    fn requests(&self) -> Vec<serde_json::Value> {
        self.requests.lock().unwrap().clone()
    }
    async fn shutdown(mut self) {
        self.task.abort();
        let _ = (&mut self.task).await;
    }
}
impl Drop for ScriptedHost {
    fn drop(&mut self) {
        self.task.abort();
    }
}

/// A peer and the scripted host across from it, with the first `drop_replies`
/// requests left unanswered (so the rest are answered, in order).
async fn scripted_peer(pair_id: &str, drop_replies: usize) -> (session::PeerSession, ScriptedHost) {
    let (client_private, _) = future_remote_crypto::generate_identity().expect("client identity");
    let (host_private, _) = future_remote_crypto::generate_identity().expect("host identity");
    let secret = [7_u8; 32];
    let prologue = future_remote_crypto::prologue(pair_id, "desktop_test").expect("prologue");
    let mut initiator = future_remote_crypto::Handshake::new(
        future_remote_crypto::Pattern::Pair,
        true,
        &client_private,
        None,
        Some(&secret),
        &prologue,
    )
    .expect("initiator");
    let mut responder = future_remote_crypto::Handshake::new(
        future_remote_crypto::Pattern::Pair,
        false,
        &host_private,
        None,
        Some(&secret),
        &prologue,
    )
    .expect("responder");
    // The XXpsk0 legs, the same three the real handshake runs.
    responder
        .read(&initiator.write(b"").expect("leg 1"))
        .expect("leg 1");
    initiator
        .read(&responder.write(b"").expect("leg 2"))
        .expect("leg 2");
    responder
        .read(&initiator.write(b"").expect("leg 3"))
        .expect("leg 3");
    let client_channel = initiator.finish().expect("client channel");
    let host_channel = responder.finish().expect("host channel");

    let nats = Broker::start().await.expect("owned broker");
    let subject = session::command_subject(pair_id, "list");
    let host_client = async_nats::connect(&nats.nats_url)
        .await
        .expect("fixture connection");
    let mut incoming = host_client
        .subscribe(subject.clone())
        .await
        .expect("host subscribes");
    host_client
        .flush()
        .await
        .expect("subscription reached broker");

    let requests: Arc<std::sync::Mutex<Vec<serde_json::Value>>> =
        Arc::new(std::sync::Mutex::new(Vec::new()));
    let seen = requests.clone();
    let host_channel = Arc::new(std::sync::Mutex::new(host_channel));
    let reply_channel = host_channel.clone();
    let client = async_nats::connect(&nats.nats_url)
        .await
        .expect("fixture connection");
    let task = tokio::spawn(async move {
        while let Some(message) = incoming.next().await {
            let opened = reply_channel
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .open(&message.subject, &message.payload);
            let Ok(opened) = opened else {
                continue;
            };
            let command: serde_json::Value = serde_json::from_slice(&opened).expect("command json");
            let answered = {
                let mut seen = seen.lock().unwrap();
                seen.push(command);
                seen.len()
            };
            if answered <= drop_replies {
                continue;
            }
            let context = future_remote_crypto::reply_context(&message.subject, &message.payload)
                .expect("reply context");
            let body = serde_json::to_vec(&json!({ "success": true, "data": { "sessions": [] } }))
                .expect("reply json");
            let wire = reply_channel
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .seal(&context, &body)
                .expect("seal reply");
            if let Some(reply) = &message.reply {
                let _ = host_client.publish(reply.clone(), wire.into()).await;
            }
        }
    });

    let peer = session::PeerSession::for_test(
        pair_id,
        client,
        Arc::new(std::sync::Mutex::new(client_channel)),
        "bridge_test",
    );
    (
        peer,
        ScriptedHost {
            requests,
            _nats: nats,
            task,
        },
    )
}

/**
 * The failure the retry exists for: the host ran the command, published the
 * reply, and the reply never arrived. The caller must never see it — and the
 * host must see the *same* command again, not a new one.
 *
 * Identity is what makes this idempotent, not luck: the host keys its reply
 * cache on the command id (`remote::commands`' `ReplySlots`, shared across
 * credential refreshes) and refuses a reused id whose body differs, so a
 * re-send that changed either would be a second execution.
 */
#[tokio::test]
async fn a_lost_reply_is_retried_under_the_same_command_identity() {
    let (peer, host) = scripted_peer("pair_retry", 1).await;

    let data = peer
        .request(
            &peer.command_subject("list"),
            json!({ "id": "cmd-retry", "type": "list_sessions" }),
            Duration::from_millis(300),
        )
        .await
        .expect("the retry must hide a reply that was lost once");
    assert_eq!(data["sessions"], json!([]));

    let requests = host.requests();
    assert_eq!(requests.len(), 2, "two attempts must reach the host");
    assert_eq!(
        requests[0], requests[1],
        "the retry re-sends the same command, id included"
    );
    assert_eq!(requests[0]["id"], json!("cmd-retry"));
    peer.close_socket().await.expect("client drained");
    host.shutdown().await;
}

/**
 * The retry is bounded, and what surfaces is the failure that exhausted it —
 * the phone's `throw lastError`, not the first transport error.
 *
 * The host never answers, so the failure is a *timeout*, which is the one the
 * caller must be able to tell apart from a business error: three attempts, then
 * the transport failure.
 */
#[tokio::test]
async fn a_command_that_keeps_losing_its_reply_is_abandoned_after_three_attempts() {
    let (peer, host) = scripted_peer("pair_retry_exhausted", usize::MAX).await;

    let error = peer
        .request(
            &peer.command_subject("list"),
            json!({ "id": "cmd-give-up", "type": "list_sessions" }),
            Duration::from_millis(200),
        )
        .await
        .expect_err("a reply that never arrives must surface, not hang or succeed");

    assert!(
        error.to_string().contains("timed out"),
        "the caller must see the transport failure itself: {error}"
    );
    let requests = host.requests();
    assert_eq!(
        requests.len(),
        3,
        "three attempts in total, not three retries on top of the first"
    );
    assert_eq!(
        requests[0], requests[2],
        "the abandoned attempts were the same command, not three different ones"
    );
    peer.close_socket().await.expect("client drained");
    host.shutdown().await;
}

/**
 * A write is not retried even the first time — the failure a read shrugs off
 * must reach the caller here, because a re-send the host cannot recognise as
 * one would apply the write twice.
 */
#[tokio::test]
async fn a_write_command_is_not_retried() {
    let (peer, host) = scripted_peer("pair_retry_write", usize::MAX).await;

    let error = peer
        .request(
            &peer.command_subject("list"),
            json!({
                "id": "cmd-write",
                "type": "set_task_enabled",
                "taskId": "no_such_task",
                "enabled": true,
            }),
            Duration::from_millis(200),
        )
        .await
        .expect_err("an unretried write surfaces its failure at once");

    assert!(
        error.to_string().contains("timed out"),
        "the transport failure must reach the caller: {error}"
    );
    let requests = host.requests();
    assert_eq!(
        requests.len(),
        1,
        "a command outside the whitelist gets exactly one attempt"
    );
    assert_eq!(requests[0]["id"], json!("cmd-write"));
    peer.close_socket().await.expect("client drained");
    host.shutdown().await;
}
