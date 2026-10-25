//! Migrated Client transport fault tests: real broker/records, scripted responder.
//! These do not replace the real Client/Server interoperability flows.
use future_remote_client::{
    session::{self, ControlState, EventsSubscription},
    Client, Emitter, PeerEvent,
};
use future_remote_interop::Broker;
use future_remote_interop::MemoryClientHost;
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
    channel: Arc<std::sync::Mutex<future_remote_crypto::Channel>>,
    publisher: async_nats::Client,
}

impl ScriptedHost {
    fn requests(&self) -> Vec<serde_json::Value> {
        self.requests.lock().unwrap().clone()
    }
    async fn inject(&self, subject: &str, value: &serde_json::Value) {
        let wire = self
            .channel
            .lock()
            .unwrap()
            .seal(subject, &serde_json::to_vec(value).unwrap())
            .unwrap();
        self.publisher
            .publish(subject.to_owned(), wire.into())
            .await
            .expect("publish fixture event");
        self.publisher.flush().await.expect("event reached broker");
    }
    async fn shutdown(mut self) {
        self.publisher
            .drain()
            .await
            .expect("fixture publisher drained");
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
async fn scripted_peer(
    pair_id: &str,
    drop_replies: usize,
) -> (Arc<session::PeerSession>, ScriptedHost) {
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
    let publisher = host_client.clone();
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
        Arc::new(peer),
        ScriptedHost {
            requests,
            _nats: nats,
            task,
            channel: host_channel,
            publisher,
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
        .expect("the retry must hide a reply that was lost once")
        .into_data();
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

struct ClientGuard(Client);
impl Drop for ClientGuard {
    fn drop(&mut self) {
        self.0.stop_admission();
    }
}
fn recording_emitter() -> (Emitter, Arc<std::sync::Mutex<Vec<PeerEvent>>>) {
    let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
    let sink = seen.clone();
    (
        Arc::new(move |event| sink.lock().unwrap().push(event)),
        seen,
    )
}
async fn wait_for(what: &str, mut predicate: impl FnMut() -> bool) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    while !predicate() {
        assert!(tokio::time::Instant::now() < deadline, "{what}");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

#[tokio::test]
async fn control_arrives_from_presence_and_from_the_control_packet() {
    let pair_id = "pair_cov_control";
    let (session, host) = scripted_peer(pair_id, 0).await;
    let owner = ClientGuard(Client::new(Arc::new(MemoryClientHost::empty())));
    let (emitter, seen) = recording_emitter();
    let (_stream, _visible) = owner
        .0
        .spawn_fixture_stream(
            session.clone(),
            session.subscribe(None).await.expect("subscriptions"),
            "desktop_cov_control",
            emitter,
            future_remote_client::runtime::testing::BACKLOG_CAPACITY,
        )
        .await;

    // The subscription must exist before the first push is injected, or the
    // broker has nothing to deliver to and this passes without reading anything.
    session
        .client_for_test()
        .flush()
        .await
        .expect("subscription reached broker");

    host.inject(
        &format!("p.{pair_id}.presence"),
        &json!({
            "online": true,
            "control": "standby",
            "holder": { "pairId": "pair_phone", "name": "Phone" },
        }),
    )
    .await;
    wait_for("the standby tick never reached the session", || {
        session.control().state == Some(ControlState::Standby)
    })
    .await;
    assert_eq!(
        session.control().holder.and_then(|holder| holder.name),
        Some("Phone".to_string()),
        "the holder is what lets the window name who took it"
    );

    // The move back, announced ahead of any tick.
    host.inject(
        &format!("p.{pair_id}.state.control"),
        &json!({ "control": "active", "pairId": pair_id }),
    )
    .await;
    wait_for("the control packet never reached the session", || {
        session.control().state == Some(ControlState::Active)
    })
    .await;
    assert_eq!(
        session.control().holder,
        None,
        "a packet names no holder, so the displaced one must not survive it"
    );
    // And the window is told: a `control` push is what makes it re-read the
    // host's conversations the moment they become readable again.
    wait_for("the control packet was not forwarded to the UI", || {
        seen.lock()
            .unwrap()
            .iter()
            .any(|event| event.kind == "control")
    })
    .await;
    owner
        .0
        .shutdown(Duration::from_secs(1))
        .await
        .expect("stream owner stopped");
    if session.is_connected() {
        session.close_socket().await.expect("session drained");
    }
    host.shutdown().await;
}

#[tokio::test]
async fn a_push_without_a_session_id_recovers_it_from_the_subject() {
    let pair_id = "pair_cov_backfill";
    let (session, host) = scripted_peer(pair_id, 0).await;
    let owner = ClientGuard(Client::new(Arc::new(MemoryClientHost::empty())));
    let (emitter, seen) = recording_emitter();
    let (_stream, _visible) = owner
        .0
        .spawn_fixture_stream(
            session.clone(),
            session.subscribe(None).await.expect("subscriptions"),
            "desktop_cov_backfill",
            emitter,
            future_remote_client::runtime::testing::BACKLOG_CAPACITY,
        )
        .await;

    // `subscribe()` returning only means the command was sent; injecting before
    // the broker registers the `SUB` would race and pass without delivering.
    session
        .client_for_test()
        .flush()
        .await
        .expect("subscription reached broker");

    host.inject(
        &format!("p.{pair_id}.evt.sess_backfill"),
        &json!({ "type": "agent_text", "data": r#"{"text":"hi"}"# }),
    )
    .await;

    wait_for("the session-less push never reached the emitter", || {
        seen.lock()
            .unwrap()
            .iter()
            .any(|event| event.kind == "event")
    })
    .await;
    let events = seen.lock().unwrap().clone();
    let event = events
        .iter()
        .find(|event| event.kind == "event")
        .expect("the push is an event");
    assert_eq!(
        event.payload["sessionId"],
        json!("sess_backfill"),
        "the subject's conversation must be written back onto the payload"
    );
    // Only the session is added: the rest of the body is the host's own, so a
    // client that declared nothing still sees exactly what it always did.
    assert_eq!(event.payload["type"], json!("agent_text"));
    assert_eq!(event.payload["data"], json!(r#"{"text":"hi"}"#));
    owner
        .0
        .shutdown(Duration::from_secs(1))
        .await
        .expect("stream owner stopped");
    if session.is_connected() {
        session.close_socket().await.expect("session drained");
    }
    host.shutdown().await;
}

#[tokio::test]
async fn an_overfull_event_backlog_resumes_and_drops_the_newest() {
    let pair_id = "pair_cov_resumed";
    let (session, host) = scripted_peer(pair_id, 0).await;
    let owner = ClientGuard(Client::new(Arc::new(MemoryClientHost::empty())));
    let subscriptions = session.subscribe(None).await.expect("subscribe");
    // Observe a marker on the same receiving connection, published by the same
    // connection as the burst. A local flush/PONG alone does not establish when
    // another connection's publishes have reached this subscriber's queues.
    let barrier_subject = format!("p.{pair_id}.fixture.burst_ready");
    let mut barrier = session
        .client_for_test()
        .subscribe(barrier_subject.clone())
        .await
        .expect("subscribe burst barrier");
    session
        .client_for_test()
        .flush()
        .await
        .expect("event and barrier subscriptions reached broker");

    let capacity = 8usize;
    let total = capacity + 4;
    for idx in 0..total {
        host.inject(
            &format!("p.{pair_id}.evt.sess_resumed"),
            &json!({ "type": "agent_text", "data": "{}", "idx": idx }),
        )
        .await;
    }
    host.publisher
        .publish(barrier_subject.clone(), "burst_ready".into())
        .await
        .expect("publish marker after the complete burst");
    host.publisher.flush().await.expect("marker reached broker");
    let marker = tokio::time::timeout(Duration::from_secs(5), barrier.next())
        .await
        .expect("complete burst must reach the receiving connection")
        .expect("barrier subscription remains open");
    assert_eq!(marker.subject.as_str(), barrier_subject);
    assert_eq!(marker.payload.as_ref(), b"burst_ready");
    // NATS orders this publisher's messages on the receiving connection;
    // async-nats dispatches them to subscriber queues in that order. Receiving
    // the marker therefore establishes that all event frames are queued before
    // the actual Client stream starts. The marker is outside every role lane.
    drop(barrier);

    let (emitter, seen) = recording_emitter();
    let (_stream, _visible) = owner
        .0
        .spawn_fixture_stream(
            session.clone(),
            subscriptions,
            "desktop_cov_resumed",
            emitter,
            capacity,
        )
        .await;

    wait_for("the overflow never produced a resumed", || {
        seen.lock()
            .unwrap()
            .iter()
            .any(|event| event.kind == "resumed")
    })
    .await;
    // The resync is the *last* thing the loop does for the episode, so waiting
    // for it means every surviving push has already been emitted.
    let events = seen.lock().unwrap().clone();
    let resumed = events
        .iter()
        .filter(|event| event.kind == "resumed")
        .count();
    let delivered: Vec<&serde_json::Value> = events
        .iter()
        .filter(|event| event.kind == "event")
        .map(|event| &event.payload)
        .collect();
    assert_eq!(
        resumed, 1,
        "one episode of dropping is one resync, not one per lost push"
    );
    assert_eq!(
        delivered.len(),
        capacity,
        "the queue's ceiling is exactly what survives an overfull burst"
    );
    // The drop policy is newest-first, so the last push that arrives is the one
    // at the ceiling; the four injected past it never surface at all.
    assert_eq!(
        delivered.last().expect("the capped queue drained")["idx"],
        json!(total - 5),
        "the newest pushes are the ones dropped"
    );
    owner
        .0
        .shutdown(Duration::from_secs(1))
        .await
        .expect("stream owner stopped");
    if session.is_connected() {
        session.close_socket().await.expect("session drained");
    }
    host.shutdown().await;
}

#[tokio::test]
async fn a_failed_lane_move_keeps_the_installed_lane() {
    let pair_id = "pair_cov_lane";
    let (session, host) = scripted_peer(pair_id, 0).await;
    let owner = ClientGuard(Client::new(Arc::new(MemoryClientHost::empty())));

    let old_subject = "p.pair_cov.evt.old".to_string();
    let live_client = host.publisher.clone();
    let mut events = live_client
        .subscribe(old_subject.clone())
        .await
        .expect("subscribe the installed lane");
    live_client
        .flush()
        .await
        .expect("subscription reached broker");

    // Close the connection the lane would move on, and wait until it can no
    // longer accept a `SUB`: that is the state a failed re-subscribe reports.
    session
        .close_socket()
        .await
        .expect("drain the lane's client");
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        if session
            .client_for_test()
            .subscribe("p.pair_cov.probe")
            .await
            .is_err()
        {
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "a drained client must stop accepting subscriptions"
        );
        tokio::task::yield_now().await;
    }

    let lane = session.event_lane();
    let mut installed = EventsSubscription {
        subject: old_subject.clone(),
        narrow: false,
    };

    future_remote_client::runtime::testing::apply_visible_lane(
        &lane,
        &mut installed,
        &mut events,
        None,
        "desktop_cov_lane",
    )
    .await;

    assert_eq!(
        installed.subject, old_subject,
        "a failed move must leave the lane that was already installed"
    );
    // The stream is still there: the installed subscription still receives,
    // which a torn-down connection would not.
    host.inject(&old_subject, &json!({ "type": "agent_text", "data": "{}" }))
        .await;
    let delivered = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        futures::StreamExt::next(&mut events),
    )
    .await
    .expect("the installed lane must still deliver after the failed move")
    .expect("a message on the installed lane");
    assert_eq!(delivered.subject.as_str(), old_subject);
    owner
        .0
        .shutdown(Duration::from_secs(1))
        .await
        .expect("stream owner stopped");
    // This session was deliberately drained before the lane-move attempt.
    // ConnectionState is not a completion signal for async-nats' drain command;
    // issuing a second drain can fail after the command channel has closed.
    host.shutdown().await;
}
