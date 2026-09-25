//! End-to-end byte verification of the lean remote lane on the real
//! cryptography: a real v2 pairing (Noise handshake), per-record AEAD, the real
//! bridge, and a client that actually subscribes and decrypts.
//!
//! The lane tests next door (`remote::tests`) observe the `FakeNats` tap while
//! the state carries `Transport::legacy_fixture()` — that is plaintext *before*
//! encryption, so their byte counts are a model of the wire, not the wire. This
//! module instead wires the production path: one `secure::Transport` shared by
//! the command loop and the event drain (`build_transport`), a phone-side
//! `Channel` obtained through the real `secure_pair` handshake, and a phone
//! subscription whose every message is counted on receipt and again after
//! `Channel::open`. Each decrypted record is asserted to be exactly a sealed
//! record (`wire == plaintext + HEADER + TAG`), so a plaintext passthrough
//! cannot pass as an E2EE measurement.
//!
//! Two client states are measured over the same data: a client that never
//! declares `lean_events_v1` (the pre-change lane, asserted byte-identical to
//! the source) and one that declares it. Both `get_session_entries` shapes are
//! measured the same way: the full read and the `before`-paged read the phone
//! actually uses.
//!
//! The real-traffic measurement lives in [`measure_real_e2ee_bytes`] (the
//! in-process fake broker) and [`measure_real_broker_bytes`] (a real
//! `nats-server`), both ignored by default and driven by
//! `scripts/measure/verify-e2e-bytes.py`, which dumps one real run's journal
//! and that session's real history page from the live agent.
//!
//! [`Broker`] is the one difference between the two: the fake keeps its
//! in-process subscription table, while the real broker answers a PING/PONG
//! flush on the subscriber's own connection (the server processes the SUB
//! first, so the PONG proves the subscription is live). When a real broker's
//! monitoring endpoint is supplied, the harness also records the server's own
//! `/connz` byte counters for the phone connection across the measurement
//! window, so the broker's accounting can be compared with the bytes the
//! phone actually decrypted.

// `mock_agent_lock` is deliberately held across awaits for the whole bridge
// conversation (the same fixture-serialization pattern as
// `commands::bridge_tests`): the scripted agent is process-global, so releasing
// it between commands would let a sibling test's one-shot response be consumed
// by this session.
#![allow(clippy::await_holding_lock)]

use std::borrow::Cow;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::Duration;

use futures::StreamExt;
use serde_json::{json, Value};

use super::commands::{new_reply_slots, HandshakeState};
use super::protocol::PairingCreds;
use super::publisher::MAX_EVENT_BYTES;
use super::test_support::{
    ensure_mock_agent, init_store, jwt, mock_agent_lock, now_secs, secure_pair, unique, FakeNats,
    HomeGuard,
};
use super::transport::build_transport;
use super::{publish_event, secure, stop, DropCounters, NatsHealth, RemoteState, SUPERVISOR};
use crate::remote_host::business::truncated_event_data;
use crate::remote_host::lean;

/// The `before` cursor the mobile client opens a conversation with.
const NEWEST_PAGE_CURSOR: i64 = 9_007_199_254_740_991;

/// One journal event, exactly the fields the replay path publishes.
#[derive(Clone, Debug)]
struct JournalEvent {
    event_type: String,
    data: String,
    idx: i64,
    epoch: i64,
    timestamp: String,
    session_idx: i64,
    run_sequence: i64,
}

fn journal_event(event_type: &str, data: Value, idx: i64) -> JournalEvent {
    JournalEvent {
        event_type: event_type.to_string(),
        data: data.to_string(),
        idx,
        epoch: 0,
        timestamp: String::new(),
        session_idx: -1,
        run_sequence: 0,
    }
}

/// Parse a `run_events` dump: one JSON object per line, with the event's own
/// `data` as an (embedded, stringified) JSON document — the shape
/// `scripts/verify-e2e-bytes.py` writes.
fn parse_journal(text: &str) -> Vec<JournalEvent> {
    text.lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            let raw: Value = serde_json::from_str(line).expect("journal line is JSON");
            JournalEvent {
                event_type: raw["event_type"].as_str().unwrap_or_default().to_string(),
                data: raw["data"].as_str().unwrap_or("{}").to_string(),
                idx: raw["idx"].as_i64().unwrap_or(0),
                epoch: raw["epoch"].as_i64().unwrap_or(0),
                timestamp: raw["timestamp"].as_str().unwrap_or_default().to_string(),
                session_idx: raw["session_idx"].as_i64().unwrap_or(-1),
                run_sequence: raw["run_sequence"].as_i64().unwrap_or(0),
            }
        })
        .collect()
}

/// What the shipping code will publish for one event, given the declaration in
/// force: `None` when the lean feed drops it. Computed through the production
/// functions themselves (`lean_event_data`, the payload cap), never re-derived.
///
/// It doubles as the replay's pacing: only events that will actually be
/// enqueued are counted as in flight, so the bounded event queue can never
/// overflow (a queue drop would silently shrink the measurement).
fn expected_lane_data(event: &JournalEvent, lean_on: bool) -> Option<String> {
    let data: Cow<'_, str> = if lean_on {
        lean::lean_event_data(&event.event_type, &event.data)?
    } else {
        Cow::Borrowed(event.data.as_str())
    };
    Some(if data.len() > MAX_EVENT_BYTES {
        truncated_event_data(&data)
    } else {
        data.into_owned()
    })
}

/// One message a phone received on `p.{pair}.evt.{session}`: wire bytes as they
/// arrived, decrypted body bytes, and the parsed body.
struct ReceivedEvent {
    wire_bytes: usize,
    plaintext_bytes: usize,
    body: Value,
}

#[derive(Default)]
struct LaneCapture {
    received: Vec<ReceivedEvent>,
}

impl LaneCapture {
    fn wire_bytes(&self) -> usize {
        self.received.iter().map(|event| event.wire_bytes).sum()
    }

    fn plaintext_bytes(&self) -> usize {
        self.received
            .iter()
            .map(|event| event.plaintext_bytes)
            .sum()
    }

    fn data_bytes(&self) -> usize {
        self.received
            .iter()
            .map(|event| event.body["data"].as_str().map_or(0, str::len))
            .sum()
    }

    fn summary(&self) -> Value {
        json!({
            "messages": self.received.len(),
            "wireBytes": self.wire_bytes(),
            "plaintextBytes": self.plaintext_bytes(),
            "dataBytes": self.data_bytes(),
        })
    }
}

/// A decrypted command reply: the sealed bytes received, the plaintext bytes
/// recovered, and the decoded body.
struct ReplyCapture {
    wire_bytes: usize,
    plaintext_bytes: usize,
    body: Value,
}

impl ReplyCapture {
    fn summary(&self) -> Value {
        json!({
            "wireBytes": self.wire_bytes,
            "plaintextBytes": self.plaintext_bytes,
            "entries": self.body["data"]["entries"].as_array().map(Vec::len).unwrap_or(0),
        })
    }
}

/// What the shipping code will send for one history page.
struct HistoryCapture {
    full: ReplyCapture,
    paged: ReplyCapture,
}

/// A real broker's cumulative counters for the phone's connection
/// (`nats-server` `/connz`), from the server's point of view: `in_*` is what
/// the server read *from* the phone (the sealed commands it sent), `out_*` is
/// what it wrote *to* it (the sealed records it received). Both count message
/// payload bytes only; the probe in the report moves them by exactly the
/// payload size. A window delta on `out_*` is therefore directly comparable
/// with the payload bytes the harness received and decrypted.
#[derive(Clone, Copy, Debug)]
struct PhoneAccounting {
    in_messages: u64,
    in_bytes: u64,
    out_messages: u64,
    out_bytes: u64,
}

/// The broker both connections run on: the in-process fake for the
/// self-contained tests, or a real `nats-server` for the real-broker replay.
enum Broker {
    Fake(FakeNats),
    Real { url: String },
}

impl Broker {
    fn url(&self) -> &str {
        match self {
            Broker::Fake(nats) => nats.url(),
            Broker::Real { url } => url,
        }
    }

    /// A real broker URL from the environment; the measurement script starts
    /// the server (`nats-server -a 127.0.0.1 -p <free>`) and passes its URL.
    fn real_from_env() -> Self {
        Broker::Real {
            url: std::env::var("VERIFY_E2E_NATS_URL").expect(
                "VERIFY_E2E_NATS_URL must point at a running nats-server \
                 (scripts/measure/verify-e2e-bytes.py --broker real starts one)",
            ),
        }
    }

    /// Return only once `client`'s subscription to `pattern` is live on the
    /// broker, so the replay cannot publish into the gap between SUB and
    /// registration.
    /// Returns the number of payload bytes the probe put on the connection, so
    /// the caller can subtract harness traffic from the broker's accounting.
    async fn await_subscription(&self, client: &async_nats::Client, pattern: &str) -> u64 {
        match self {
            // The fake exposes its subscription table directly.
            Broker::Fake(nats) => {
                nats.wait_for_sub(pattern, Duration::from_secs(5)).await;
                0
            }
            // `Client::flush` only proves the bytes left this process, not that
            // the server processed them — and a publish that overtakes the SUB
            // is dropped, never retried. A publish/subscribe echo on the same
            // connection is a real round trip: the server handles one
            // connection's commands in order, so once the echo of a publish
            // sent *after* the SUB arrives, the subscription is registered.
            // (This is what already made the fixture flake ~1 run in 5 before
            // it replaced the flush.)
            Broker::Real { .. } => {
                let probe_subject = format!("{pattern}.verify-sub-probe");
                let payload = "verify-sub-probe";
                let mut probe = client
                    .subscribe(probe_subject.clone())
                    .await
                    .expect("probe subscribe");
                client
                    .publish(probe_subject.clone(), payload.into())
                    .await
                    .expect("probe publish");
                let echo = tokio::time::timeout(Duration::from_secs(5), probe.next())
                    .await
                    .expect("timed out waiting for the subscription probe echo")
                    .expect("probe subscription ended");
                assert_eq!(echo.subject.as_str(), probe_subject);
                probe.unsubscribe().await.expect("probe unsubscribe");
                payload.len() as u64
            }
        }
    }
}

/// Connect a client the way the harness needs it: named, so the broker's
/// monitoring view identifies each side.
async fn connect(url: &str, name: &str) -> async_nats::Client {
    async_nats::ConnectOptions::new()
        .name(name)
        .connect(url)
        .await
        .expect("connect to the broker")
}

/// The real bridge + a paired phone, both on `broker`.
///
/// Field order matters for teardown: dropping `E2e` runs its `Drop` (which
/// resets the process-global declaration and stops the bridge) before the
/// `HomeGuard` restores `HOME`.
struct E2e {
    _home: HomeGuard,
    broker: Broker,
    pair_id: String,
    phone: async_nats::Client,
    channel: future_remote_crypto::Channel,
    /// Every payload byte the harness received and decrypted on the phone, and
    /// the message count — the harness's own side of the broker accounting.
    phone_rx_bytes: usize,
    phone_rx_messages: usize,
    /// Traffic this *harness* put on the phone connection that is not lane
    /// traffic: the subscription-readiness probe. It is counted by the broker
    /// and must be subtracted before comparing, or the comparison is off by
    /// exactly the probe's size (measured: 2 probes x 1 message x 16 bytes).
    probe_bytes: u64,
    probe_messages: u64,
}

impl Drop for E2e {
    fn drop(&mut self) {
        // Capabilities are process-global (single `AtomicBool`/`Arc<AtomicBool>`
        // by design, see `remote_host::lean`); a leaked "on" would silently
        // reshape every later test in this process.
        lean::set_enabled(false);
        stop();
    }
}

fn v2_creds(broker: &Broker) -> PairingCreds {
    PairingCreds {
        handshake_version: 2,
        secure: Some(secure::PairingIdentity::new(now_secs() + 600).unwrap()),
        pair_id: format!("pair_{}", unique("e2e")),
        desktop_id: format!("desktop_{}", unique("e2e")),
        nkey_seed: nkeys::KeyPair::new_user().seed().unwrap().to_string(),
        user_jwt: jwt(now_secs() + 3600),
        nats_url: broker.url().to_string(),
        nats_ws_url: broker.url().replace("nats://", "ws://"),
        jwt_expires_at: now_secs() + 3600,
    }
}

/// The QR invitation a phone scans, built the way `remote_host::pairing` builds
/// it for the real app.
fn invitation(creds: &PairingCreds) -> String {
    let identity = creds.secure.as_ref().expect("v2 identity");
    let mut url = reqwest::Url::parse("futureos://remote/pair").expect("constant URL");
    url.query_pairs_mut()
        .append_pair("v", "2")
        .append_pair("code", "verify-e2e-code")
        .append_pair("desktopId", &creds.desktop_id)
        .append_pair("desktopKey", "UTESTKEY")
        .append_pair("secureKey", &identity.public_key)
        .append_pair("secret", identity.secret.as_deref().expect("secret"));
    url.to_string()
}

impl E2e {
    async fn on_broker(broker: Broker, label: &str) -> Self {
        let home = HomeGuard::new(label);
        init_store();
        ensure_mock_agent();
        let creds = v2_creds(&broker);
        let pair_id = creds.pair_id.clone();
        let bridge_client = connect(broker.url(), &format!("verify-bridge-{pair_id}")).await;
        let bridge_instance_id = format!("bridge_{}", unique("e2e"));
        let handshake = HandshakeState::new(
            creds.clone(),
            Arc::new(AtomicBool::new(true)),
            bridge_instance_id.clone(),
        );
        // The production wiring, unmodified: one secure transport shared by the
        // command loop and the event drain, a real ordered event queue, a real
        // command subscription. `build_transport` also resets the capability
        // flags, which is what makes the first replay the pre-change baseline.
        let mut tasks = build_transport(&bridge_client, &pair_id, &handshake, new_reply_slots())
            .await
            .expect("bridge transport");
        let previous = SUPERVISOR.state.lock().unwrap().replace(RemoteState {
            security: handshake.secure.clone(),
            generation_id: 1,
            client: bridge_client,
            nats_health: Arc::new(NatsHealth::default()),
            nats_url: broker.url().to_string(),
            pair_id: pair_id.clone(),
            desktop_id: creds.desktop_id.clone(),
            desktop_public_key: "UTESTPUBKEY".to_string(),
            bridge_instance_id,
            event_tx: tasks.event_tx,
            drop_counters: Arc::new(DropCounters::new()),
            event_task: tasks.event_task,
            cmd_task: tasks.cmd_task,
            transfer_task: tasks.transfer_task,
            heartbeat_task: tasks.heartbeat_task,
            refresh_task: tokio::spawn(std::future::pending()),
            web_task: None,
            web_url: None,
            web_lan_url: None,
            pairing_code: None,
            pairing_code_expires_at: None,
            pairing_confirmed: Arc::new(AtomicBool::new(true)),
        });
        assert!(
            previous.is_none(),
            "a previous test leaked SUPERVISOR.state"
        );
        // The readiness candidates are now the serving tasks; without this the
        // `CandidateTasks` drop would abort every one of them (that is exactly
        // what it is for during a failed readiness).
        tasks.candidate_tasks.installed();

        // The phone: a separate broker connection running the real handshake.
        let phone = connect(broker.url(), &format!("verify-phone-{pair_id}")).await;
        let channel = secure_pair(&phone, &invitation(&creds), &pair_id).await;
        E2e {
            _home: home,
            broker,
            pair_id,
            phone,
            channel,
            phone_rx_bytes: 0,
            phone_rx_messages: 0,
            probe_bytes: 0,
            probe_messages: 0,
        }
    }

    /// The broker's own accounting for the phone's connection, read from the
    /// real server's monitoring endpoint (`/connz`). Only meaningful for a
    /// real broker; the fake has no such endpoint and the fields stay unused.
    async fn phone_accounting(&self, monitor: &str) -> Option<PhoneAccounting> {
        // reqwest is built without a provider in this crate (one is installed
        // per entry point, see `crate::install_rustls_provider`).
        crate::install_rustls_provider();
        let view: Value = reqwest::get(format!("{monitor}/connz?subs=1"))
            .await
            .expect("broker monitoring reachable")
            .json()
            .await
            .expect("connz is JSON");
        let name = format!("verify-phone-{}", self.pair_id);
        let conn = view["connections"]
            .as_array()?
            .iter()
            .find(|conn| conn["name"].as_str() == Some(name.as_str()))?;
        Some(PhoneAccounting {
            in_messages: conn["in_msgs"].as_u64().expect("in_msgs"),
            in_bytes: conn["in_bytes"].as_u64().expect("in_bytes"),
            out_messages: conn["out_msgs"].as_u64().expect("out_msgs"),
            out_bytes: conn["out_bytes"].as_u64().expect("out_bytes"),
        })
    }

    /// Send one sealed command on the real command lane and decrypt the sealed
    /// reply the bridge publishes to the request inbox.
    async fn sealed_command(&mut self, subject: &str, command: &Value) -> ReplyCapture {
        let wire = self
            .channel
            .seal(
                subject,
                &serde_json::to_vec(command).expect("command serializes"),
            )
            .expect("command seals");
        let reply = self
            .phone
            .request(subject.to_string(), wire.clone().into())
            .await
            .expect("bridge reply");
        // The reply binds the authenticated request, exactly as the bridge sealed
        // it (`secure::reply_context`).
        let context = future_remote_crypto::reply_context(subject, &wire).expect("reply context");
        // The wire carries a v2 record, not the JSON the bridge produced.
        assert!(
            reply.payload.starts_with(b"FRE2"),
            "the reply must be a sealed v2 record, got {:?}",
            &reply.payload[..reply.payload.len().min(8)]
        );
        // A NATS header would ride outside `payload` (and outside the byte
        // count): the lane uses none, which is what keeps received payload
        // bytes the whole per-message wire cost.
        assert!(
            reply.headers.is_none(),
            "the reply must not carry NATS headers"
        );
        let plaintext = self
            .channel
            .open(&context, &reply.payload)
            .expect("reply decrypts");
        let body = serde_json::from_slice(&plaintext).expect("reply is JSON");
        self.phone_rx_bytes += reply.payload.len();
        self.phone_rx_messages += 1;
        ReplyCapture {
            wire_bytes: reply.payload.len(),
            plaintext_bytes: plaintext.len(),
            body,
        }
    }

    /// Declare a capability set on `secure_ready`, as the phone does after the
    /// handshake confirmation. The bridge applies it before it replies, so
    /// awaiting the reply makes the new state observable.
    async fn declare(&mut self, features: &[&str]) {
        assert!(
            features.len() <= 1,
            "this harness flips one capability at a time"
        );
        let subject = format!("p.{}.cmd.ready", self.pair_id);
        let body = json!({ "type": "secure_ready", "features": features });
        let reply = self.sealed_command(&subject, &body).await;
        assert_eq!(reply.body["success"], json!(true), "{:?}", reply.body);
    }

    /// Replay one run's journal through the real `publish_event`, counting only
    /// what the phone receives and decrypts.
    async fn replay_live_lane(
        &mut self,
        session: &str,
        run: &str,
        journal: &[JournalEvent],
    ) -> LaneCapture {
        let subject = format!("p.{}.evt.{session}", self.pair_id);
        let mut subscription = self
            .phone
            .subscribe(subject.clone())
            .await
            .expect("subscribe to the live lane");
        // `subscribe` flushes the SUB, but the broker must have registered it
        // before the first publish or that publish would race the subscription.
        self.probe_bytes += self.broker.await_subscription(&self.phone, &subject).await;
        self.probe_messages += 1;

        let lean_on = lean::enabled();
        let mut capture = LaneCapture::default();
        let mut in_flight = 0usize;
        for event in journal {
            publish_event(
                session,
                &event.event_type,
                &event.data,
                run,
                event.idx,
                event.epoch,
                "",
                &event.timestamp,
                event.session_idx,
                event.run_sequence,
            );
            if expected_lane_data(event, lean_on).is_some() {
                in_flight += 1;
            }
            // Keep the bounded event queue from ever filling: drain what we
            // published before adding much more.
            while in_flight > 64 {
                capture
                    .received
                    .push(receive_event(&mut subscription, &mut self.channel).await);
                in_flight -= 1;
            }
        }
        while in_flight > 0 {
            capture
                .received
                .push(receive_event(&mut subscription, &mut self.channel).await);
            in_flight -= 1;
        }
        self.phone_rx_bytes += capture.wire_bytes();
        self.phone_rx_messages += capture.received.len();
        // Nothing may trail the replay: the lane is complete exactly when the
        // expected messages have arrived, and an extra one would mean the
        // measured message set is not the journal.
        assert!(
            tokio::time::timeout(Duration::from_millis(150), subscription.next())
                .await
                .is_err(),
            "unexpected extra message on {subject}"
        );
        capture
    }

    /// Read one session's history in both shipping shapes.
    async fn history(&mut self, session: &str) -> HistoryCapture {
        let subject = format!("p.{}.cmd.rpc", self.pair_id);
        let full = self
            .sealed_command(
                &subject,
                &json!({ "id": unique("hist"), "type": "get_session_entries", "sessionId": session }),
            )
            .await;
        let paged = self
            .sealed_command(
                &subject,
                &json!({
                    "id": unique("hist"),
                    "type": "get_session_entries",
                    "sessionId": session,
                    "before": NEWEST_PAGE_CURSOR,
                    "limit": 10,
                }),
            )
            .await;
        HistoryCapture { full, paged }
    }

    /// Serve one session's entries from the scripted agent, exactly as the
    /// existing history tests do.
    fn set_session_entries(&self, session: &str, entries: &Value) {
        let agent = ensure_mock_agent();
        agent.clear_scripts();
        agent.set_session_entries(session, entries.clone());
    }
}

async fn receive_event(
    subscription: &mut async_nats::Subscriber,
    channel: &mut future_remote_crypto::Channel,
) -> ReceivedEvent {
    let message = tokio::time::timeout(Duration::from_secs(30), subscription.next())
        .await
        .expect("timed out waiting for a live-lane message")
        .expect("live-lane subscription ended");
    // The wire carries a v2 record, not the JSON the bridge produced: this is
    // the check that separates a received-bytes measurement from a tap taken
    // before encryption.
    assert!(
        message.payload.starts_with(b"FRE2"),
        "received bytes must be a sealed v2 record, got {:?}",
        &message.payload[..message.payload.len().min(8)]
    );
    // The lane must not smuggle a NATS header block alongside the sealed
    // record: `message.payload` (and so every byte counted below) would then
    // miss the header bytes that really crossed the wire.
    assert!(
        message.headers.is_none(),
        "live-lane records must not carry NATS headers"
    );
    let plaintext = channel
        .open(message.subject.as_str(), &message.payload)
        .expect("live-lane record decrypts");
    let body = serde_json::from_slice(&plaintext).expect("live-lane body is JSON");
    ReceivedEvent {
        wire_bytes: message.payload.len(),
        plaintext_bytes: plaintext.len(),
        body,
    }
}

/// Assert the lane carried exactly the expected events, and that every received
/// record really was a sealed one.
fn assert_lane(capture: &LaneCapture, journal: &[JournalEvent], lean_on: bool, what: &str) {
    let expected: Vec<(&JournalEvent, String)> = journal
        .iter()
        .filter_map(|event| expected_lane_data(event, lean_on).map(|data| (event, data)))
        .collect();
    assert_eq!(
        capture.received.len(),
        expected.len(),
        "{what}: message count"
    );
    for (received, (source, data)) in capture.received.iter().zip(expected) {
        let at = format!("{what}: {}#{}", source.event_type, source.idx);
        assert_eq!(
            received.body["type"].as_str(),
            Some(source.event_type.as_str()),
            "{at}"
        );
        assert_eq!(received.body["idx"].as_i64(), Some(source.idx), "{at}");
        assert_eq!(
            received.body["data"].as_str(),
            Some(data.as_str()),
            "{at}: data"
        );
        // A real AEAD record: the wire is the plaintext plus the fixed
        // header/tag. A plaintext passthrough (or a pre-encryption tap) cannot
        // satisfy this, which is what makes the byte count a received-bytes
        // count rather than a model of one.
        assert_eq!(
            received.wire_bytes,
            received.plaintext_bytes
                + future_remote_crypto::HEADER_LEN
                + future_remote_crypto::TAG_LEN,
            "{at}: received bytes must be a sealed record"
        );
    }
}

fn fixture_journal() -> Vec<JournalEvent> {
    vec![
        journal_event(
            "thinking_delta",
            json!({"text": "private reasoning no phone renders", "block_id": "b1"}),
            1,
        ),
        journal_event("text_chunk", json!({"text": "Hello "}), 2),
        journal_event("text_chunk", json!({"text": "world"}), 3),
        journal_event(
            "tool_delta",
            json!({"text": "{\"path\":", "tool_id": "c1"}),
            4,
        ),
        journal_event(
            "tool_start",
            json!({"tool_id": "c1", "tool_name": "read", "tool_args": {"path": "/tmp/a"}}),
            5,
        ),
        journal_event(
            "tool_end",
            json!({
                "tool_id": "c1", "tool_name": "read", "text": "file body\n[exit: 3]",
                "exit_code": 3, "is_soft_fail": false, "target_path": "/tmp/a",
            }),
            6,
        ),
        journal_event(
            "usage",
            json!({"input_tokens": 100, "output_tokens": 50}),
            7,
        ),
    ]
}

/// A history page with all three payloads the lean history trim removes: a
/// reasoning body, a tool-result body, and tool-call arguments beyond the four
/// keys a tool row's target can come from.
fn fixture_entries() -> Value {
    json!({ "entries": [
        {
            "id": "u1", "kind": "user", "role": "user", "createdAtMs": 1,
            "blocks": [{"kind": "text", "text": "question"}],
        },
        {
            "id": "a1", "kind": "assistant", "role": "assistant", "createdAtMs": 2,
            "blocks": [
                {"kind": "reasoning", "text": "private reasoning body ".repeat(200)},
                {"kind": "text", "text": "visible answer"},
                {"kind": "tool_call", "name": "read", "toolCallId": "c1",
                 "arguments": {"path": "/tmp/a", "offset": 5, "limit": 10,
                               "content": "x".repeat(4000)}},
            ],
        },
        {
            "id": "t1", "kind": "tool", "role": "tool", "createdAtMs": 3,
            "blocks": [
                {"kind": "tool_result", "toolCallId": "c1", "text": "tool output body ".repeat(300)},
            ],
        },
    ] })
}

fn assert_history_undeclared(capture: &ReplyCapture, entries: &Value, what: &str) {
    assert_eq!(
        capture.body["success"],
        json!(true),
        "{what}: {:?}",
        capture.body
    );
    let source = entries["entries"].as_array().expect("fixture entries");
    let page = capture.body["data"]["entries"]
        .as_array()
        .expect("reply entries");
    assert_eq!(page.len(), source.len(), "{what}: entry count");
    for (returned, original) in page.iter().zip(source) {
        for block in original["blocks"].as_array().into_iter().flatten() {
            let returned_block = returned["blocks"]
                .as_array()
                .into_iter()
                .flatten()
                .find(|candidate| {
                    candidate["kind"] == block["kind"]
                        && candidate.get("toolCallId") == block.get("toolCallId")
                })
                .expect("block survives");
            match block["kind"].as_str().unwrap_or_default() {
                "reasoning" => assert_eq!(
                    returned_block["text"], block["text"],
                    "{what}: reasoning body reaches an undeclared client"
                ),
                "tool_result" => assert_eq!(
                    returned_block["text"], block["text"],
                    "{what}: tool output reaches an undeclared client"
                ),
                "tool_call" => assert_eq!(
                    returned_block["arguments"], block["arguments"],
                    "{what}: every argument reaches an undeclared client"
                ),
                _ => {}
            }
        }
    }
    // Sealed reply, like every other record on the channel.
    assert_eq!(
        capture.wire_bytes,
        capture.plaintext_bytes + future_remote_crypto::HEADER_LEN + future_remote_crypto::TAG_LEN,
        "{what}: reply must be a sealed record"
    );
}

/// The declared state keeps the page's structure (entry/block identity and
/// count) and drops only the three unread bodies.
fn assert_history_lean(capture: &ReplyCapture, full: &ReplyCapture, entries: &Value, what: &str) {
    assert_eq!(
        capture.body["success"],
        json!(true),
        "{what}: {:?}",
        capture.body
    );
    let source = entries["entries"].as_array().expect("fixture entries");
    let page = capture.body["data"]["entries"]
        .as_array()
        .expect("reply entries");
    assert_eq!(page.len(), source.len(), "{what}: entries keep their count");
    let mut trimmed = 0usize;
    for (returned, original) in page.iter().zip(source) {
        assert_eq!(returned["id"], original["id"], "{what}: entry identity");
        for block in original["blocks"].as_array().into_iter().flatten() {
            let returned_block = returned["blocks"]
                .as_array()
                .into_iter()
                .flatten()
                .find(|candidate| {
                    candidate["kind"] == block["kind"]
                        && candidate.get("toolCallId") == block.get("toolCallId")
                })
                .expect("block survives");
            match block["kind"].as_str().unwrap_or_default() {
                "reasoning" | "tool_result" => {
                    assert!(
                        returned_block.get("text").is_none(),
                        "{what}: {} body must be dropped, not blanked",
                        block["kind"]
                    );
                    trimmed += 1;
                }
                "tool_call" => {
                    let arguments = returned_block["arguments"]
                        .as_object()
                        .expect("arguments object");
                    assert!(
                        arguments.keys().all(|key| {
                            ["command", "path", "file_path", "filePath"].contains(&key.as_str())
                        }),
                        "{what}: only target keys survive, got {arguments:?}"
                    );
                    trimmed += 1;
                }
                _ => {}
            }
        }
    }
    assert!(trimmed > 0, "{what}: the fixture must exercise the trim");
    assert!(
        capture.plaintext_bytes < full.plaintext_bytes,
        "{what}: the lean page must be smaller (lean {} >= full {})",
        capture.plaintext_bytes,
        full.plaintext_bytes
    );
    assert_eq!(
        capture.wire_bytes,
        capture.plaintext_bytes + future_remote_crypto::HEADER_LEN + future_remote_crypto::TAG_LEN,
        "{what}: reply must be a sealed record"
    );
}

/// The self-contained proof, run by `cargo test --lib verify_e2e` on the
/// in-process fake broker: real handshake, real bridge, real subscription,
/// both declaration states, both history shapes. The same body runs on a real
/// broker in `verify_e2e_real_broker_round_trip`; the real-traffic numbers come
/// from the ignored measurements below.
#[tokio::test]
async fn verify_e2e_lean_lane_real_crypto_round_trip() {
    run_fixture(Broker::Fake(FakeNats::start().await), "verify-e2e-fixture").await;
}

/// The same proof against a real `nats-server`: this is where "the E2EE
/// handshake and every lane invariant hold on a real broker" is pinned. Run by
/// `scripts/measure/verify-e2e-bytes.py --broker real`.
#[tokio::test]
#[ignore = "real broker: needs VERIFY_E2E_NATS_URL (scripts/measure/verify-e2e-bytes.py --broker real)"]
async fn verify_e2e_real_broker_round_trip() {
    run_fixture(Broker::real_from_env(), "verify-real-broker-fixture").await;
}

async fn run_fixture(broker: Broker, label: &str) {
    // Lock order matches the other command-family tests: mock-agent lock first,
    // then the HOME lock taken inside `E2e::on_broker`.
    let _agent_lock = mock_agent_lock();
    let mut e2e = E2e::on_broker(broker, label).await;
    let session = format!("sess_{}", unique("verify-e2e"));
    let journal = fixture_journal();

    // ── Undeclared: the pre-change lane, byte-identical to the journal ──
    assert!(!lean::enabled());
    let full = e2e.replay_live_lane(&session, "run-verify", &journal).await;
    assert_lane(&full, &journal, false, "undeclared");
    // Spot checks so a regression names the mechanism, not just a count.
    assert_eq!(full.received[0].body["type"], json!("thinking_delta"));
    let tool_end: Value =
        serde_json::from_str(full.received[5].body["data"].as_str().unwrap()).unwrap();
    assert_eq!(tool_end["text"], json!("file body\n[exit: 3]"));

    // ── Declared: streamed content drops, outcomes survive ──
    e2e.declare(&[lean::LEAN_EVENTS_FEATURE]).await;
    assert!(lean::enabled());
    let lean_capture = e2e.replay_live_lane(&session, "run-verify", &journal).await;
    assert_lane(&lean_capture, &journal, true, "declared");
    assert_eq!(lean_capture.received.len(), 5, "two content events drop");
    assert_eq!(lean_capture.received[0].body["type"], json!("text_chunk"));
    assert_eq!(lean_capture.received[0].body["idx"], json!(2));
    let tool_end: Value =
        serde_json::from_str(lean_capture.received[3].body["data"].as_str().unwrap()).unwrap();
    assert!(tool_end.get("text").is_none(), "captured output is dropped");
    assert_eq!(tool_end["exit_code"], json!(3), "outcome survives");
    assert!(
        lean_capture.wire_bytes() < full.wire_bytes(),
        "the lean lane must receive fewer bytes ({} vs {})",
        lean_capture.wire_bytes(),
        full.wire_bytes()
    );

    // ── Withdrawn: a later connection must not inherit the declaration ──
    e2e.declare(&[]).await;
    assert!(!lean::enabled());
    let withdrawn = e2e.replay_live_lane(&session, "run-verify", &journal).await;
    assert_eq!(
        withdrawn.received.len(),
        full.received.len(),
        "withdrawing the declaration must restore every message"
    );
    for (after, before) in withdrawn.received.iter().zip(&full.received) {
        assert_eq!(
            after.body, before.body,
            "withdrawn lane must be byte-identical"
        );
    }

    // ── History: both shapes, both states ──
    let entries = fixture_entries();
    e2e.set_session_entries(&session, &entries);
    let full_pages = e2e.history(&session).await;
    assert_history_undeclared(&full_pages.full, &entries, "undeclared full");
    assert_history_undeclared(&full_pages.paged, &entries, "undeclared paged");

    e2e.declare(&[lean::LEAN_EVENTS_FEATURE]).await;
    assert!(lean::enabled());
    let lean_pages = e2e.history(&session).await;
    assert_history_lean(
        &lean_pages.full,
        &full_pages.full,
        &entries,
        "declared full",
    );
    assert_history_lean(
        &lean_pages.paged,
        &full_pages.paged,
        &entries,
        "declared paged",
    );

    e2e.declare(&[]).await;
    assert!(!lean::enabled());
    let withdrawn_pages = e2e.history(&session).await;
    assert_history_undeclared(&withdrawn_pages.full, &entries, "withdrawn full");
    assert_history_undeclared(&withdrawn_pages.paged, &entries, "withdrawn paged");

    println!(
        "VERIFY_E2E_FIXTURE {}",
        json!({
            "live": { "undeclared": full.summary(), "declared": lean_capture.summary() },
            "history": {
                "undeclared": { "full": full_pages.full.plaintext_bytes,
                                "paged": full_pages.paged.plaintext_bytes },
                "declared": { "full": lean_pages.full.plaintext_bytes,
                              "paged": lean_pages.paged.plaintext_bytes },
            },
        })
    );
}

/// The real-traffic measurement on the in-process fake broker: one real run's
/// journal and one real session's history page, dumped by
/// `scripts/measure/verify-e2e-bytes.py` and replayed through the same harness
/// as the fixture test. Prints one `VERIFY_E2E_*` line per measured lane with
/// raw received/decrypted byte counts.
#[tokio::test]
#[ignore = "measurement: needs VERIFY_E2E_JOURNAL/SESSION/RUN/ENTRIES from scripts/measure/verify-e2e-bytes.py"]
async fn measure_real_e2ee_bytes() {
    run_real_traffic(Broker::Fake(FakeNats::start().await)).await;
}

/// The same real traffic over a real `nats-server` (started by
/// `scripts/measure/verify-e2e-bytes.py --broker real`). When the script also
/// passes the server's monitoring endpoint, this additionally asserts the
/// broker's own `/connz` byte accounting for the phone connection over the
/// measurement window against the harness's received-and-decrypted totals.
#[tokio::test]
#[ignore = "real broker: needs VERIFY_E2E_NATS_URL + the measurement inputs (scripts/measure/verify-e2e-bytes.py --broker real)"]
async fn measure_real_broker_bytes() {
    run_real_traffic(Broker::real_from_env()).await;
}

async fn run_real_traffic(broker: Broker) {
    let journal_path = std::env::var("VERIFY_E2E_JOURNAL").expect("VERIFY_E2E_JOURNAL");
    let session = std::env::var("VERIFY_E2E_SESSION").expect("VERIFY_E2E_SESSION");
    let run = std::env::var("VERIFY_E2E_RUN").unwrap_or_default();
    let entries_path = std::env::var("VERIFY_E2E_ENTRIES").expect("VERIFY_E2E_ENTRIES");
    let journal = parse_journal(&std::fs::read_to_string(journal_path).expect("journal readable"));
    let entries: Value =
        serde_json::from_str(&std::fs::read_to_string(entries_path).expect("entries readable"))
            .expect("entries JSON");
    assert!(!journal.is_empty(), "the journal dump must not be empty");

    let _agent_lock = mock_agent_lock();
    let mut e2e = E2e::on_broker(broker, "verify-e2e-measure").await;
    // The broker's view of the phone's connection is a cumulative counter, so
    // take a baseline before the measured traffic: the window delta is then
    // exactly the measured lanes (the handshake and setup fall outside it).
    let monitor = std::env::var("VERIFY_E2E_NATS_MONITOR").ok();
    let before = match &monitor {
        Some(url) => Some(e2e.phone_accounting(url).await.expect("phone connection")),
        None => None,
    };
    // Serve the real session's page through the scripted agent (the bridge's
    // agent link is a process-global mock in this test binary). The dump is the
    // bare entries array the agent replied with; the mock speaks the desktop's
    // `{ "entries": [...] }` page shape.
    e2e.set_session_entries(&session, &json!({ "entries": entries }));

    // ── Undeclared: the pre-change lane over real traffic ──
    assert!(
        !lean::enabled(),
        "build_transport must reset the declaration"
    );
    let live_full = e2e.replay_live_lane(&session, &run, &journal).await;
    assert_lane(&live_full, &journal, false, "undeclared");
    let history_full = e2e.history(&session).await;
    let history_undeclared = json!({
        "full": history_full.full.summary(),
        "paged": history_full.paged.summary(),
    });
    assert_eq!(
        history_full.full.body["success"],
        json!(true),
        "full read failed: {}",
        history_full.full.body
    );

    // ── Declared: the lean lane over the same traffic ──
    e2e.declare(&[lean::LEAN_EVENTS_FEATURE]).await;
    assert!(lean::enabled());
    let live_lean = e2e.replay_live_lane(&session, &run, &journal).await;
    assert_lane(&live_lean, &journal, true, "declared");
    let history_lean = e2e.history(&session).await;
    assert_eq!(
        history_lean.full.body["success"],
        json!(true),
        "lean full read failed: {}",
        history_lean.full.body
    );

    // Both history shapes keep their structure. The full page is bounded by the
    // 100-entry limit; the paged page is bounded by the 512 KiB budget, which
    // fits *more* lean exchanges per round trip — so the count may only grow.
    let count = |capture: &ReplyCapture| {
        capture.body["data"]["entries"]
            .as_array()
            .map(Vec::len)
            .unwrap_or(0)
    };
    assert!(
        count(&history_lean.full) >= count(&history_full.full),
        "the lean full page must not carry fewer entries ({} vs {})",
        count(&history_lean.full),
        count(&history_full.full)
    );
    assert!(
        history_lean.full.plaintext_bytes <= history_full.full.plaintext_bytes,
        "the lean full page must not be larger: {} vs {}",
        history_lean.full.plaintext_bytes,
        history_full.full.plaintext_bytes
    );
    assert!(
        count(&history_lean.paged) >= count(&history_full.paged),
        "the lean paged page must not carry fewer entries ({} vs {})",
        count(&history_lean.paged),
        count(&history_full.paged)
    );

    println!(
        "VERIFY_E2E_LIVE {}",
        json!({
            "session": session,
            "run": run,
            "journalEvents": journal.len(),
            "undeclared": live_full.summary(),
            "declared": live_lean.summary(),
        })
    );
    println!(
        "VERIFY_E2E_HISTORY {}",
        json!({
            "session": session,
            "undeclared": history_undeclared,
            "declared": {
                "full": history_lean.full.summary(),
                "paged": history_lean.paged.summary(),
            },
        })
    );
    if let (Some(before), Some(url)) = (before, monitor.as_deref()) {
        let after = e2e
            .phone_accounting(url)
            .await
            .expect("phone connection after the measurement");
        // The broker's own count of what it relayed to the phone (`out_*`),
        // against what the harness received and decrypted.
        //
        // The comparison subtracts the harness's own probe traffic rather than
        // allowing slack, so it stays an exact equality: the probe subscription
        // is registered on the phone connection and the broker counts its echo,
        // but the probe is not lane traffic and the harness does not count it.
        // (`replay_live_lane` runs twice per measurement, so this is exactly
        // 2 messages and 2 x `"verify-sub-probe".len()` = 32 bytes.)
        let relayed = after.out_bytes - before.out_bytes - e2e.probe_bytes;
        let decrypted = e2e.phone_rx_bytes as u64;
        assert_eq!(
            relayed, decrypted,
            "the broker must relay exactly the bytes the phone decrypted, \
             once its own probe traffic ({} B) is excluded",
            e2e.probe_bytes
        );
        let relayed_messages = after.out_messages - before.out_messages - e2e.probe_messages;
        assert_eq!(
            relayed_messages, e2e.phone_rx_messages as u64,
            "the broker must relay exactly the messages the phone received, \
             once its own probe traffic ({} msgs) is excluded",
            e2e.probe_messages
        );
        // The event subject is what every per-message protocol frame carries,
        // so its length is what the framing arithmetic in the report needs.
        let evt_subject = format!("p.{}.evt.{session}", e2e.pair_id);
        println!(
            "VERIFY_E2E_NATS_ACCOUNTING {}",
            json!({
                "receivedMessages": e2e.phone_rx_messages,
                "receivedBytes": e2e.phone_rx_bytes,
                "serverOutMessages": after.out_messages - before.out_messages,
                "serverOutBytes": after.out_bytes - before.out_bytes,
                "sentMessages": after.in_messages - before.in_messages,
                "sentBytes": after.in_bytes - before.in_bytes,
                "evtSubject": evt_subject,
                "evtSubjectBytes": evt_subject.len(),
            })
        );
    }
}
