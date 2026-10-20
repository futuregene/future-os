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

use super::commands::new_reply_slots;
use super::publisher::{CATALOG_ACTIVITY_FEATURE, EVENT_COALESCING_FEATURE, MAX_EVENT_BYTES};
use super::test_support::{
    declare_features, ensure_mock_agent, init_store, jwt, mock_agent_lock, now_secs, pairing_creds,
    secure_pair, unique, FakeNats, HomeGuard,
};
use super::transport::build_transport;
use super::{publish_event, secure, stop, DropCounters, NatsHealth, RemoteState, SUPERVISOR};
use crate::remote_host::business::truncated_event_data;
use crate::remote_host::lean;
use future_remote_server::PairingCreds;

/// The `before` cursor the mobile client opens a conversation with.
/// The phone's history page size, in user exchanges: `HISTORY_PAGE_USER_EXCHANGES`
/// in `mobile/src/remote/useTimelineController.ts` (#607 lowered it from 10).
/// Keep these in step — a page measured at another size is not the page a phone
/// reads.
const PHONE_HISTORY_PAGE_EXCHANGES: i64 = 3;
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
/// `scripts/measure/verify-e2e-bytes.py` writes.
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
    /// Sealed bytes of the request that produced this reply, so a fetch can be
    /// costed as the round trip the phone actually pays.
    request_wire_bytes: usize,
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
#[derive(Clone, Copy, Debug, Default)]
struct PhoneAccounting {
    in_messages: u64,
    in_bytes: u64,
    out_messages: u64,
    out_bytes: u64,
}

/// The broker both connections run on: the in-process fake for the
/// self-contained tests, or a real `nats-server` for the real-broker replay.
pub(super) enum Broker {
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
pub(super) async fn connect(url: &str, name: &str) -> async_nats::Client {
    async_nats::ConnectOptions::new()
        .name(name)
        .connect(url)
        .await
        .expect("connect to the broker")
}

/// Whether the connection on `pair_id` declared the lean feed.
///
/// The declaration is per connection (a machine serves several clients), so it
/// is read where it is recorded rather than from the old process-wide flag.
fn lean_declared(pair_id: &str) -> bool {
    super::SUPERVISOR
        .lean_events(pair_id)
        .load(std::sync::atomic::Ordering::Acquire)
}

/// The real bridge + a paired phone, both on `broker`.
///
/// Field order matters for teardown: dropping `E2e` runs its `Drop` (which
/// stops the bridge) before the `HomeGuard` restores `HOME`.
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

/// A host bridge on a real broker with **no in-process phone**.
///
/// [`E2e`] keeps both ends in one process. The real product does not: a phone
/// (or a desktop's client role) pairs from another process over the real
/// transport. This fixture installs exactly the same production state
/// [`E2e::on_broker`] installs — one secure transport shared by the command
/// loop and the event drain — and then gets out of the way, so a second process
/// can pair with `v2_creds`' invitation.
///
/// `real_agent` leaves `FUTURE_AGENT_GRPC_ADDR` alone (the caller has pointed
/// it at a running `future-agent`) instead of installing the scripted mock.
pub(super) struct InstalledHost {
    _home: HomeGuard,
    pub(super) pair_id: String,
    pub(super) creds: PairingCreds,
}

impl Drop for InstalledHost {
    fn drop(&mut self) {
        lean::set_enabled(false);
        stop();
    }
}

/// Start the production host bridge for a cross-process client.
pub(super) async fn install_host(broker: Broker, label: &str, real_agent: bool) -> InstalledHost {
    let creds = v2_creds(&broker);
    install_host_with_creds(creds, broker, label, real_agent).await
}

/// [`install_host`] with pairing credentials supplied by the caller: the shape
/// a restarted host needs, because a peer that keeps the same identity must
/// present the same Noise static key after the restart.
pub(super) async fn install_host_with_creds(
    creds: PairingCreds,
    broker: Broker,
    label: &str,
    real_agent: bool,
) -> InstalledHost {
    let home = HomeGuard::new(label);
    init_store();
    if !real_agent {
        ensure_mock_agent();
    }
    let slot = install_slot(creds, &broker, true).await;
    // The fixture is the machine's only device, so it is the controller; the
    // command gate is real and would otherwise refuse every command.
    SUPERVISOR.restore_active(Some(slot.pair_id.clone()));
    InstalledHost {
        _home: home,
        pair_id: slot.pair_id,
        creds: slot.creds,
    }
}

/// One device's host-side runtime, installed.
///
/// Split out of [`install_host_with_creds`] so a test can install *several*
/// devices on one machine: the process-global `HOME` and the store belong to
/// the caller (`HomeGuard` is exclusive), while a slot belongs to one pairing.
pub(super) struct InstalledSlot {
    pub(super) pair_id: String,
    pub(super) creds: PairingCreds,
}

/// Install the production bridge for one device on `broker`.
///
/// `confirmed` is the pairing's "this device has paired before" flag. A real
/// first connection has it `false`, and the first secure handshake that
/// completes then claims control of the machine; a test that wants a device to
/// behave as already-established passes `true`, and that device's handshakes
/// never move the pointer (which is how a reconnect behaves).
pub(super) async fn install_slot(
    creds: PairingCreds,
    broker: &Broker,
    confirmed: bool,
) -> InstalledSlot {
    let pair_id = creds.pair_id.clone();
    let _shared = super::supervisor::shared_runtime(&creds, true, false);
    let bridge_client = connect(broker.url(), &format!("realnet-host-{pair_id}")).await;
    let bridge_instance_id = format!("bridge_{}", unique("realnet-host"));
    let confirmed_flag = Arc::new(AtomicBool::new(confirmed));
    let handshake = crate::remote::commands::new_handshake_state(
        creds.clone(),
        confirmed_flag.clone(),
        bridge_instance_id.clone(),
    );
    let mut tasks = build_transport(&bridge_client, &pair_id, &handshake, new_reply_slots())
        .await
        .expect("host bridge transport");
    let previous = SUPERVISOR.state.lock().unwrap().insert(
        pair_id.clone(),
        RemoteState {
            security: handshake.secure.clone(),
            generation_id: 1,
            client: bridge_client,
            nats_health: Arc::new(NatsHealth::default()),
            nats_url: broker.url().to_string(),
            pair_id: pair_id.clone(),
            desktop_id: creds.desktop_id.clone(),
            desktop_public_key: "REALNETHOSTPUBKEY".to_string(),
            bridge_instance_id,
            event_tx: tasks.event_tx,
            drop_counters: Arc::new(DropCounters::new()),
            event_task: tasks.event_task,
            cmd_task: tasks.cmd_task,
            transfer_task: tasks.transfer_task,
            heartbeat_task: tasks.heartbeat_task,
            refresh_task: crate::remote::SUPERVISOR.spawn_fixture_task(std::future::pending()),
            web_task: None,
            web_url: None,
            web_lan_url: None,
            pairing_code: None,
            pairing_code_expires_at: None,
            pairing_confirmed: confirmed_flag,
        },
    );
    assert!(
        previous.is_none(),
        "a previous fixture leaked SUPERVISOR.state for {pair_id}"
    );
    tasks.candidate_tasks.installed();
    // The bridge publishes what the Agent stream observer hands it; without
    // this sink the phone would see commands answered and no events at all.
    crate::remote_host::attach_events();
    InstalledSlot { pair_id, creds }
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

pub(super) fn v2_creds(broker: &Broker) -> PairingCreds {
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
        control_url: None,
    }
}

/// The QR invitation a phone scans, built the way `remote_host::pairing` builds
/// it for the real app.
pub(super) fn invitation(creds: &PairingCreds) -> String {
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
        // Install the per-pairing shared runtime the way the supervisor does
        // before it builds the transport (`supervisor::start`). The live-lane
        // capability flags live there, and `coalesce_events()` answers an
        // unknown pairing with a *fresh, disabled* flag rather than an error —
        // so a harness that skips this step has its declarations land in a
        // throwaway flag and silently measures the undeclared lane.
        let _shared = super::supervisor::shared_runtime(&creds, true, false);
        let bridge_client = connect(broker.url(), &format!("verify-bridge-{pair_id}")).await;
        let bridge_instance_id = format!("bridge_{}", unique("e2e"));
        let handshake = crate::remote::commands::new_handshake_state(
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
        let previous = SUPERVISOR.state.lock().unwrap().insert(
            pair_id.clone(),
            RemoteState {
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
                refresh_task: crate::remote::SUPERVISOR.spawn_fixture_task(std::future::pending()),
                web_task: None,
                web_url: None,
                web_lan_url: None,
                pairing_code: None,
                pairing_code_expires_at: None,
                pairing_confirmed: Arc::new(AtomicBool::new(true)),
            },
        );
        assert!(
            previous.is_none(),
            "a previous test leaked SUPERVISOR.state"
        );
        SUPERVISOR.restore_active(Some(pair_id.clone()));
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
        let head = reply.payload[..reply.payload.len().min(8)].to_vec();
        assert!(
            reply.payload.starts_with(b"FRE2"),
            "the reply must be a sealed v2 record, got {head:?}"
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
            request_wire_bytes: wire.len(),
            body,
        }
    }

    /// Whether the drain task currently sees the client's coalescing
    /// declaration (the same flag `publish_event` feeds through).
    fn coalescing(&self) -> bool {
        super::SUPERVISOR
            .coalesce_events(&self.pair_id)
            .load(std::sync::atomic::Ordering::Relaxed)
    }

    /// Declare a capability set on `secure_ready`, as the phone does after the
    /// handshake confirmation. The bridge applies it before it replies, so
    /// awaiting the reply makes the new state observable.
    async fn declare(&mut self, features: &[&str]) {
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

        let lean_on = lean_declared(&self.pair_id);
        let mut capture = LaneCapture::default();
        // Flow control counts *source events*, not messages. With coalescing on,
        // one message can stand for many events (`coalescedCount`); decrementing
        // per message would leave the drain permanently "behind", so it would
        // wait a full merge window for nearly every event — a three-minute
        // measurement becomes hours. Off the coalesced lane every message is one
        // event, which is what the original accounting assumed.
        let mut outstanding = 0usize;
        for event in journal {
            publish_event(
                session,
                &event.event_type,
                &event.data,
                run,
                event.idx,
                event.epoch,
                // The id the agent would have assigned. An empty string here
                // would understate every body by the id's ~96 bytes, which is
                // how `eventId` came to look like a free field to drop.
                &format!("{session}:{run}:{}:{}", event.epoch, event.idx),
                &event.timestamp,
                event.session_idx,
                event.run_sequence,
            );
            if expected_lane_data(event, lean_on).is_some() {
                outstanding += 1;
            }
            // Keep the bounded event queue from ever filling: drain what we
            // published before adding much more.
            while outstanding > 64 {
                let received = receive_event(&mut subscription, &mut self.channel).await;
                outstanding = outstanding.saturating_sub(covered_source_events(&received));
                capture.received.push(received);
            }
        }
        while outstanding > 0 {
            let received = receive_event(&mut subscription, &mut self.channel).await;
            outstanding = outstanding.saturating_sub(covered_source_events(&received));
            capture.received.push(received);
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
                    "limit": PHONE_HISTORY_PAGE_EXCHANGES,
                }),
            )
            .await;
        HistoryCapture { full, paged }
    }

    /// What one lean shell row costs to open: the same envelope the other
    /// commands use, sealed on the real channel.
    async fn fetch_tool_call_args(
        &mut self,
        session: &str,
        run: &str,
        tool_call_id: &str,
    ) -> ReplyCapture {
        let subject = format!("p.{}.cmd.rpc", self.pair_id);
        self.sealed_command(
            &subject,
            &json!({
                "id": unique("args"),
                "type": "get_tool_call_args",
                "sessionId": session,
                "runId": run,
                "toolCallId": tool_call_id,
            }),
        )
        .await
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
    let head = message.payload[..message.payload.len().min(8)].to_vec();
    assert!(
        message.payload.starts_with(b"FRE2"),
        "received bytes must be a sealed v2 record, got {head:?}"
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
/// How many source events one received message stands for. A merged message
/// declares it (`coalescedCount`, written next to `idx` on the envelope); an
/// unmerged one stands for exactly itself.
fn covered_source_events(event: &ReceivedEvent) -> usize {
    event.body["coalescedCount"].as_u64().unwrap_or(1).max(1) as usize
}

/// The `data.text` a source event carried, or `None` when it has none.
fn data_text(data: &str) -> Option<String> {
    let value: Value = serde_json::from_str(data).ok()?;
    value.get("text")?.as_str().map(str::to_owned)
}

fn assert_lane(capture: &LaneCapture, journal: &[JournalEvent], lean_on: bool, what: &str) {
    let expected: Vec<(&JournalEvent, String)> = journal
        .iter()
        .filter_map(|event| expected_lane_data(event, lean_on).map(|data| (event, data)))
        .collect();
    // Walk the expected source events, consuming `coalescedCount` per message.
    // A merged message keeps the *newest* event of its group — envelope and `idx`
    // — and carries the whole group's text (the client reads it the same way:
    // `start = idx - coalescedCount + 1`). So "one message per event" is only
    // true off the coalesced lane, and the idx to check is the group's last.
    let mut next = 0usize;
    for received in &capture.received {
        let at = format!("{what}: message {next}");
        assert_eq!(
            received.wire_bytes,
            received.plaintext_bytes
                + future_remote_crypto::HEADER_LEN
                + future_remote_crypto::TAG_LEN,
            "{at}: received bytes must be a sealed record"
        );
        let covered = covered_source_events(received);
        let group = expected
            .get(next..next + covered)
            .unwrap_or_else(|| panic!("{at}: covers {covered} events, past the journal's end"));
        let newest = group[group.len() - 1].0;
        assert_eq!(
            received.body["type"].as_str(),
            Some(newest.event_type.as_str()),
            "{at}: {}#{}",
            newest.event_type,
            newest.idx
        );
        assert_eq!(
            received.body["idx"].as_i64(),
            Some(newest.idx),
            "{at}: a merged message keeps its group's newest idx"
        );
        if covered == 1 {
            assert_eq!(
                received.body["data"].as_str(),
                Some(group[0].1.as_str()),
                "{at}: data"
            );
        } else {
            let kinds: Vec<&str> = group
                .iter()
                .map(|(event, _)| event.event_type.as_str())
                .collect();
            assert!(
                kinds.windows(2).all(|pair| pair[0] == pair[1]),
                "{at}: a merge may only cover one stream, got {kinds:?}"
            );
            let body: Value = serde_json::from_str(received.body["data"].as_str().unwrap_or("{}"))
                .unwrap_or(Value::Null);
            let merged: String = group
                .iter()
                .filter_map(|(_, data)| data_text(data))
                .collect();
            assert_eq!(
                body.get("text").and_then(Value::as_str),
                Some(merged.as_str()),
                "{at}: a merged message carries its whole group's text"
            );
        }
        next += covered;
    }
    assert_eq!(next, expected.len(), "{what}: covered source events");
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
                    let kind = block["kind"].as_str().unwrap_or_default();
                    assert!(
                        returned_block.get("text").is_none(),
                        "{what}: {kind} body must be dropped, not blanked"
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
    assert!(!lean_declared(&e2e.pair_id));
    let full = e2e.replay_live_lane(&session, "run-verify", &journal).await;
    assert_lane(&full, &journal, false, "undeclared");
    // Spot checks so a regression names the mechanism, not just a count.
    assert_eq!(full.received[0].body["type"], json!("thinking_delta"));
    let tool_end: Value =
        serde_json::from_str(full.received[5].body["data"].as_str().unwrap()).unwrap();
    assert_eq!(tool_end["text"], json!("file body\n[exit: 3]"));

    // ── Declared: streamed content drops, outcomes survive ──
    e2e.declare(&[lean::LEAN_EVENTS_FEATURE]).await;
    assert!(lean_declared(&e2e.pair_id));
    let lean_capture = e2e.replay_live_lane(&session, "run-verify", &journal).await;
    assert_lane(&lean_capture, &journal, true, "declared");
    assert_eq!(lean_capture.received.len(), 5, "two content events drop");
    assert_eq!(lean_capture.received[0].body["type"], json!("text_chunk"));
    assert_eq!(lean_capture.received[0].body["idx"], json!(2));
    let tool_end: Value =
        serde_json::from_str(lean_capture.received[3].body["data"].as_str().unwrap()).unwrap();
    assert!(tool_end.get("text").is_none(), "captured output is dropped");
    assert_eq!(tool_end["exit_code"], json!(3), "outcome survives");
    let lean_bytes = lean_capture.wire_bytes();
    let full_bytes = full.wire_bytes();
    assert!(
        lean_bytes < full_bytes,
        "the lean lane must receive fewer bytes ({lean_bytes} vs {full_bytes})"
    );

    // ── Coalesced: fragments merge, and the merge is verifiable ──
    //
    // The capability has to be installed per pairing before the transport is
    // built (`E2e::on_broker` does what the supervisor does). Without it the
    // declaration lands in a throwaway flag and a measurement silently reports
    // the un-merged lane as "coalesced" — which is what happened once.
    e2e.declare(&[EVENT_COALESCING_FEATURE]).await;
    assert!(
        e2e.coalescing(),
        "the coalescing declaration must land in the flag the drain reads"
    );
    let coalesced = e2e.replay_live_lane(&session, "run-verify", &journal).await;
    assert_lane(&coalesced, &journal, false, "coalesced");
    let coalesced_messages = coalesced.received.len();
    let full_messages = full.received.len();
    assert!(
        coalesced_messages < full_messages,
        "the coalesced lane must merge fragments ({coalesced_messages} vs {full_messages} messages)"
    );
    e2e.declare(&[]).await;
    assert!(!e2e.coalescing());
    let uncoalesced = e2e.replay_live_lane(&session, "run-verify", &journal).await;
    for (after, before) in uncoalesced.received.iter().zip(&full.received) {
        assert_eq!(
            after.body, before.body,
            "withdrawing the capability must restore the per-event lane"
        );
    }

    // ── Withdrawn: a later connection must not inherit the declaration ──
    e2e.declare(&[]).await;
    assert!(!lean_declared(&e2e.pair_id));
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
    assert!(lean_declared(&e2e.pair_id));
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
    assert!(!lean_declared(&e2e.pair_id));
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

/// Every shell tool call in a served page: its identity plus the command bytes
/// that page carries for it (zero once the trim drops `arguments`).
fn shell_calls_in(capture: &ReplyCapture) -> Vec<(String, String, usize)> {
    capture.body["data"]["entries"]
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(|entry| {
            let run = entry["runId"].as_str().unwrap_or_default().to_string();
            entry["blocks"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(move |block| {
                    if block["kind"] != json!("tool_call") || block["name"] != json!("shell") {
                        return None;
                    }
                    let id = block["toolCallId"].as_str().unwrap_or_default().to_string();
                    let command = block["arguments"]["command"].as_str();
                    Some((run.clone(), id, command.map_or(0, str::len)))
                })
        })
        .collect()
}

/// The catalog lane's own measurement: what one second of a streaming run costs
/// the wire, for a client that declared `catalog_activity_v1` and for one that
/// did not, over a store the size of the local desktop's own.
///
/// The numbers are the *publisher's payload* bytes, measured off the broker tap
/// by the shipping `spawn_catalog_publisher` — the record layer adds a fixed
/// `HEADER_LEN + TAG_LEN` (44 B) per packet on top, which the script applies
/// (one snapshot or one packet is one record, so the overhead is per packet, not
/// per byte).
///
/// Driven by `scripts/measure/measure-catalog-activity.py`, which supplies the
/// row count and each real run's activity-write count — the observer's own 1 s
/// throttle replayed from that run's journal — and turns these per-write bytes
/// into a per-run total.
#[tokio::test]
#[ignore = "measurement: run through scripts/measure/measure-catalog-activity.py"]
async fn measure_catalog_activity_lane() {
    let rows: usize = std::env::var("CATALOG_ACTIVITY_ROWS")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(232);
    let writes: usize = std::env::var("CATALOG_ACTIVITY_WRITES")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(20);

    let declared = measure_catalog_writes(rows, writes, true, "catalog-activity-measure").await;
    // Separate HOME per mode, so the two runs cannot see each other's rows.
    // `HomeGuard` serializes on a process-global lock, which is why these are
    // awaited one after the other rather than together.
    let undeclared = measure_catalog_writes(rows, writes, false, "catalog-legacy-measure").await;

    let saving = 1.0
        - declared["payloadBytes"].as_f64().unwrap_or_default()
            / undeclared["payloadBytes"].as_f64().unwrap_or(1.0).max(1.0);
    println!(
        "CATALOG_ACTIVITY {}",
        json!({
            "rows": rows,
            "writes": writes,
            "declared": declared,
            "undeclared": undeclared,
            "payloadSaved": saving,
        })
    );
}

/// One mode of the measurement: seed `rows` conversations, then put `writes`
/// activity writes through the real publisher, counting what it published.
async fn measure_catalog_writes(
    rows: usize,
    writes: usize,
    declared_capability: bool,
    label: &str,
) -> Value {
    use super::spawn_presence_heartbeat;
    use super::test_support::{await_publish, nats_connect_once};

    let _home = HomeGuard::new(label);
    init_store();
    let mut target = String::new();
    for index in 0..rows {
        let thread = crate::store::create_thread(crate::store::CreateThreadInput {
            mode: "chat".to_string(),
            title: Some(format!("A conversation titled as a real one does {index}")),
            workspace_id: None,
            workspace_path: None,
            workspace_name: None,
            agent_session_id: Some(unique("sess-measure")),
        })
        .expect("seed a conversation");
        if index == 0 {
            target = thread.id.clone();
        }
    }

    let nats = FakeNats::start().await;
    let client = nats_connect_once(&nats).await;
    let pair = unique("pairmeasure");
    let creds = pairing_creds(&pair);
    let _shared = super::supervisor::shared_runtime(&creds, true, false);
    declare_features(
        &creds,
        &pair,
        if declared_capability {
            &[CATALOG_ACTIVITY_FEATURE]
        } else {
            &[]
        },
    );
    // The publisher serves the device in control; this measurement is that
    // device's lane.
    SUPERVISOR.restore_active(Some(pair.clone()));
    let handle = spawn_presence_heartbeat(client.clone(), pair.clone(), format!("bridge_{label}"));
    let mut tap = nats.tap();
    let sessions_subject = format!("p.{pair}.state.sessions");
    let activity_subject = format!("p.{pair}.state.activity");

    // The baseline is what a client pays once, when it connects — not per
    // second — so it is reported separately from the streaming cost.
    let baseline = await_publish(&mut tap, &sessions_subject, Duration::from_secs(5)).await;

    // Activity timestamps advance from above every row's current activity, which
    // is what a real run does: in this fixture that lifts the moving row to the
    // head of the list on the first write (the row's place *is* content — an
    // ordinary list draws the host's order), and leaves it there after. One
    // snapshot for the overtake is therefore part of the honest per-run shape,
    // not an artifact to hide.
    //
    // The base comes from the shipping recency vector rather than from a clock:
    // `rows` conversations seeded in the same millisecond have no meaningful
    // order among themselves, so a fixture that leaned on one would be measuring
    // SQLite's tie order instead of the lane.
    let mut started = crate::remote_host::catalog::sessions_catalog("")
        .expect("store readable")
        .activity
        .iter()
        .map(|(_, at)| *at)
        .max()
        .unwrap_or_default();

    let mut sessions_packets = 0usize;
    let mut sessions_bytes = 0usize;
    let mut activity_packets = 0usize;
    let mut activity_bytes = 0usize;
    for _ in 0..writes {
        started += 1_000;
        crate::store::record_thread_message_activity(&target, started).expect("record activity");
        // The test profile's catalog tick is 10 ms; this is several ticks wide,
        // so the count below is the publisher's decision and not a sampling
        // artifact.
        tokio::time::sleep(Duration::from_millis(60)).await;
        while let Ok(published) = tap.try_recv() {
            if published.subject == sessions_subject {
                sessions_packets += 1;
                sessions_bytes += published.payload.len();
            } else if published.subject == activity_subject {
                activity_packets += 1;
                activity_bytes += published.payload.len();
            }
        }
    }
    handle.abort();

    let payload_bytes = sessions_bytes + activity_bytes;
    json!({
        "declared": declared_capability,
        "rows": rows,
        "writes": writes,
        "baselineBytes": baseline.payload.len(),
        "sessionsPackets": sessions_packets,
        "sessionsPayloadBytes": sessions_bytes,
        "activityPackets": activity_packets,
        "activityPayloadBytes": activity_bytes,
        "payloadBytes": payload_bytes,
        "records": sessions_packets + activity_packets,
    })
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
    let monitor = std::env::var("VERIFY_E2E_NATS_MONITOR").ok();
    run_real_traffic_with(broker, &journal, &session, &run, &entries, monitor).await;
}

/// The measurement itself, with its inputs as arguments instead of env vars, so
/// a test can drive the same code over a fixture page. The env-gated drivers
/// above are the ones that read a *real* run's journal and history dump
/// (`scripts/measure/verify-e2e-bytes.py`); everything below them is this
/// function, and it is where the invariants of the measurement are asserted.
async fn run_real_traffic_with(
    broker: Broker,
    journal: &[JournalEvent],
    session: &str,
    run: &str,
    entries: &Value,
    monitor: Option<String>,
) {
    assert!(!journal.is_empty(), "the journal dump must not be empty");

    let _agent_lock = mock_agent_lock();
    let mut e2e = E2e::on_broker(broker, "verify-e2e-measure").await;
    // The broker's view of the phone's connection is a cumulative counter, so
    // take a baseline before the measured traffic: the window delta is then
    // exactly the measured lanes (the handshake and setup fall outside it).
    let before = match &monitor {
        Some(url) => Some(e2e.phone_accounting(url).await.expect("phone connection")),
        None => None,
    };
    // Serve the real session's page through the scripted agent (the bridge's
    // agent link is a process-global mock in this test binary). The dump is the
    // bare entries array the agent replied with; the mock speaks the desktop's
    // `{ "entries": [...] }` page shape.
    e2e.set_session_entries(session, &json!({ "entries": entries }));

    // ── Undeclared: the pre-change lane over real traffic ──
    assert!(
        !lean_declared(e2e.pair_id.as_str()),
        "build_transport must reset the declaration"
    );
    let live_full = e2e.replay_live_lane(session, run, journal).await;
    assert_lane(&live_full, journal, false, "undeclared");
    let history_full = e2e.history(session).await;
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
    assert!(lean_declared(&e2e.pair_id));
    let live_lean = e2e.replay_live_lane(session, run, journal).await;
    assert_lane(&live_lean, journal, true, "declared");
    let history_lean = e2e.history(session).await;
    assert_eq!(
        history_lean.full.body["success"],
        json!(true),
        "lean full read failed: {}",
        history_lean.full.body
    );

    // ── What the phone actually negotiates ──
    //
    // The mobile client declares all three features at once
    // (`mobile/src/remote/client.ts`: `event_coalescing_v1`, `reply_gzip_v1`,
    // `lean_events_v1`). Measuring lean alone answers "what does the trim remove
    // from the raw stream", not "what did the phone gain from it" — coalescing
    // was already shipped before the trim landed, so the phone's before/after is
    // coalesced-vs-coalesced+lean. Both are measured here.
    e2e.declare(&[EVENT_COALESCING_FEATURE]).await;
    // The declaration must actually reach the drain task's flag. Without this
    // the harness would keep measuring the undeclared lane while reporting a
    // "coalesced" number, which is how a wrong `98.5% marginal` reading was
    // produced once already: capabilities live behind a per-pairing lookup that
    // answers an unknown pairing with a fresh, disabled flag instead of an
    // error.
    assert!(
        e2e.coalescing(),
        "the coalescing declaration must land in the flag the drain reads"
    );
    let live_coalesced = e2e.replay_live_lane(session, run, journal).await;
    assert_lane(&live_coalesced, journal, false, "coalesced");
    let coalesced_messages = live_coalesced.received.len();
    let source_events = journal.len();
    assert!(
        coalesced_messages < source_events,
        "the coalesced lane must merge fragments ({coalesced_messages} messages for {source_events} source events)"
    );
    e2e.declare(&[EVENT_COALESCING_FEATURE, lean::LEAN_EVENTS_FEATURE])
        .await;
    assert!(lean_declared(&e2e.pair_id));
    let live_phone = e2e.replay_live_lane(session, run, journal).await;
    assert_lane(&live_phone, journal, true, "coalesced+lean");
    let phone_bytes = live_phone.wire_bytes();
    let coalesced_bytes = live_coalesced.wire_bytes();
    assert!(
        phone_bytes < coalesced_bytes || live_phone.received.len() <= live_coalesced.received.len(),
        "the phone's lane may not grow when lean is added ({phone_bytes} vs {coalesced_bytes})"
    );
    println!(
        "VERIFY_E2E_LIVE_PHONE {}",
        json!({
            "session": session,
            "journalEvents": journal.len(),
            "coalesced": live_coalesced.summary(),
            "coalescedLean": live_phone.summary(),
            "leanMarginal": 1.0 - (live_phone.wire_bytes() as f64
                / live_coalesced.wire_bytes().max(1) as f64),
        })
    );

    // Both history shapes keep their structure. The full read is bounded by the
    // 100-entry limit; the paged read carries the same requested exchanges for
    // either client, and it is the *undeclared* one that has to shed oldest
    // exchanges when they exceed the byte budget — so the lean page may hold
    // more of the very page that was asked for, never fewer.
    let count = |capture: &ReplyCapture| {
        capture.body["data"]["entries"]
            .as_array()
            .map(Vec::len)
            .unwrap_or(0)
    };
    let lean_full_entries = count(&history_lean.full);
    let full_entries = count(&history_full.full);
    assert!(
        lean_full_entries >= full_entries,
        "the lean full page must not carry fewer entries ({lean_full_entries} vs {full_entries})"
    );
    let lean_full_bytes = history_lean.full.plaintext_bytes;
    let undeclared_full_bytes = history_full.full.plaintext_bytes;
    assert!(
        lean_full_bytes <= undeclared_full_bytes,
        "the lean full page must not be larger: {lean_full_bytes} vs {undeclared_full_bytes}"
    );
    let lean_paged_entries = count(&history_lean.paged);
    let undeclared_paged_entries = count(&history_full.paged);
    assert!(
        lean_paged_entries >= undeclared_paged_entries,
        "the lean paged page must not carry fewer entries ({lean_paged_entries} vs {undeclared_paged_entries})"
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
        assert_broker_accounting_against(&e2e, before, url, session).await;
    }

    // ── The way back: what one opened shell row costs ──
    //
    // The trim drops a shell call's `arguments` whole and the phone asks the
    // command back when the row is opened, so the page is smaller either way
    // and the open question is whether the fetch gives the saving back:
    //
    //   lazy total = (page without the commands) + Σ(fetched) + k · overhead
    //   carried    = the same page with them
    //
    // measured here as real sealed bytes on the real channel: `overhead` is the
    // round trip minus the command it returns, and the tap count at which lazy
    // stops paying for itself is `carried bytes / (mean command + overhead)`.
    // The comparison runs on the *full* page, because that is the one page both
    // sides hold the same entries of (the 100-entry limit is the same). The
    // paged pages deliberately differ in entry count — that is the trim's other
    // effect — so their shell sets are not identity-comparable.
    let carried = shell_calls_in(&history_full.full);
    let trimmed = shell_calls_in(&history_lean.full);
    let command_bytes: usize = carried.iter().map(|(_, _, bytes)| bytes).sum();
    assert!(
        !carried.is_empty() && carried.iter().all(|(_, _, bytes)| *bytes > 0),
        "the undeclared page must carry the shell commands this measurement costs"
    );
    assert!(
        trimmed.iter().all(|(_, _, bytes)| *bytes == 0),
        "the lean page must carry none of them, or there is nothing to fetch back"
    );
    assert_eq!(
        carried
            .iter()
            .map(|(run, id, _)| (run, id))
            .collect::<Vec<_>>(),
        trimmed
            .iter()
            .map(|(run, id, _)| (run, id))
            .collect::<Vec<_>>(),
        "both pages must describe the same shell calls"
    );

    let (row_run, row_id, largest) = carried
        .iter()
        .max_by_key(|(_, _, bytes)| *bytes)
        .expect("carried is not empty")
        .clone();
    let fetched = e2e.fetch_tool_call_args(session, &row_run, &row_id).await;
    assert_eq!(
        fetched.body["success"],
        json!(true),
        "the lean fetch must succeed: {}",
        fetched.body
    );
    let returned = fetched.body["data"]["arguments"]["command"]
        .as_str()
        .map(str::len)
        .unwrap_or(0);
    assert_eq!(
        returned, largest,
        "the fetch must return the command the page dropped, whole"
    );
    let overhead = fetched.request_wire_bytes + fetched.wire_bytes - returned;
    let mean_command = command_bytes / carried.len();
    // The phone requests the *paged* page, and its commands are the ones the
    // saving is really about (they run longer than the newest 100 entries').
    // Its entry set differs by design, so only the properties that do not need
    // identity matching are asserted here: it carries commands, the lean page of
    // the same budget carries none.
    let paged_carried = shell_calls_in(&history_full.paged);
    let paged_commands: usize = paged_carried.iter().map(|(_, _, bytes)| bytes).sum();
    assert!(
        !paged_carried.is_empty() && paged_carried.iter().all(|(_, _, bytes)| *bytes > 0),
        "the undeclared paged page must carry the commands the phone would save"
    );
    assert!(
        shell_calls_in(&history_lean.paged)
            .iter()
            .all(|(_, _, bytes)| *bytes == 0),
        "the lean paged page must carry none of them"
    );
    println!(
        "VERIFY_E2E_LAZY_FETCH {}",
        json!({
            "session": session,
            "shellRows": carried.len(),
            "carriedCommandBytes": command_bytes,
            "largestCommandBytes": largest,
            "fetchRequestWireBytes": fetched.request_wire_bytes,
            "fetchReplyWireBytes": fetched.wire_bytes,
            "fetchOverheadBytes": overhead,
            "pagedShellRows": paged_carried.len(),
            "pagedCarriedCommandBytes": paged_commands,
            "pagedMeanCommandBytes": paged_commands / paged_carried.len(),
            "fullBreakEvenTaps": command_bytes / (mean_command + overhead),
            "pagedBreakEvenTaps": paged_commands
                / (paged_commands / paged_carried.len() + overhead),
        })
    );
}

/// The broker-accounting window: read the server's counters now, and compare the
/// delta since `before` with what this harness received and decrypted.
///
/// Split from [`assert_broker_accounting`] only so the reading and the
/// arithmetic can each be driven by an ordinary test: the reading is one HTTP
/// call against a monitoring endpoint (`/connz`), the arithmetic is a pure
/// function of two counter snapshots. A real `nats-server` supplies both; the
/// in-process broker supplies neither, so the state a *truthful* server reports
/// has to be scripted — and a scripted endpoint is only worth having if the
/// arithmetic it feeds is asserted on its own terms, which is what the two
/// tests next door do.
async fn assert_broker_accounting_against(
    e2e: &E2e,
    before: PhoneAccounting,
    monitor: &str,
    session: &str,
) {
    let after = e2e
        .phone_accounting(monitor)
        .await
        .expect("phone connection after the measurement");
    assert_broker_accounting(
        before,
        after,
        PhoneReceipt {
            bytes: e2e.phone_rx_bytes,
            messages: e2e.phone_rx_messages,
            probe_bytes: e2e.probe_bytes,
            probe_messages: e2e.probe_messages,
        },
        session,
        &e2e.pair_id,
    );
}

/// What the harness received, and how much of the broker's traffic for it was
/// the harness's own subscription probes.
#[derive(Clone, Copy, Debug)]
struct PhoneReceipt {
    bytes: usize,
    messages: usize,
    probe_bytes: u64,
    probe_messages: u64,
}

/// The window's arithmetic, with both sides as arguments.
///
/// Two subtractions matter and both are asserted, not merely reported:
///
/// * the counters are cumulative, so the window is `after - before`;
/// * the broker also relays the harness's own probe traffic, which is not lane
///   traffic the phone counts, so it is subtracted too rather than left as
///   slack.
///
/// The equality is exact because the sealed record is the only thing on the
/// lane: whatever the publisher wrote is what the phone received. A test that
/// drives this with a zero baseline and a zero probe would pass an
/// implementation that skipped both subtractions, so the ordinary test next
/// door supplies non-zero values for each and a lying broker that is one byte
/// and one message over.
fn assert_broker_accounting(
    before: PhoneAccounting,
    after: PhoneAccounting,
    receipt: PhoneReceipt,
    session: &str,
    pair_id: &str,
) {
    let relayed = after.out_bytes - before.out_bytes - receipt.probe_bytes;
    let decrypted = receipt.bytes as u64;
    assert_eq!(
        relayed, decrypted,
        "the broker must relay exactly the bytes the phone decrypted, \
         once its own probe traffic ({} B) is excluded",
        receipt.probe_bytes
    );
    let relayed_messages = after.out_messages - before.out_messages - receipt.probe_messages;
    assert_eq!(
        relayed_messages, receipt.messages as u64,
        "the broker must relay exactly the messages the phone received, \
         once its own probe traffic ({} msgs) is excluded",
        receipt.probe_messages
    );
    // The event subject is what every per-message protocol frame carries, so
    // its length is what the framing arithmetic in the report needs.
    let evt_subject = format!("p.{pair_id}.evt.{session}");
    println!(
        "VERIFY_E2E_NATS_ACCOUNTING {}",
        json!({
            "receivedMessages": receipt.messages,
            "receivedBytes": receipt.bytes,
            "serverOutMessages": after.out_messages - before.out_messages,
            "serverOutBytes": after.out_bytes - before.out_bytes,
            "sentMessages": after.in_messages - before.in_messages,
            "sentBytes": after.in_bytes - before.in_bytes,
            "evtSubject": evt_subject,
            "evtSubjectBytes": evt_subject.len(),
        })
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::EnvGuard;
    use futures::FutureExt;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    /// A page in the shape a real history dump has, carrying the one thing the
    /// lazy-fetch half of the measurement needs: shell rows whose commands the
    /// undeclared page carries and the lean page drops, with the call identity
    /// (`runId` + `toolCallId`) the on-open fetch is scoped by.
    fn traffic_entries() -> Value {
        json!([
            {
                "id": "u1", "kind": "user", "role": "user",
                "createdAtMs": 1, "runId": "run-verify",
                "blocks": [{"kind": "text", "text": "run it"}],
            },
            {
                "id": "a1", "kind": "assistant", "role": "assistant",
                "createdAtMs": 2, "runId": "run-verify",
                "blocks": [
                    {"kind": "reasoning", "text": "r".repeat(300)},
                    {"kind": "tool_call", "name": "shell", "toolCallId": "call-small",
                     "arguments": {"command": "ls"}},
                    {"kind": "tool_call", "name": "shell", "toolCallId": "call-big",
                     "arguments": {"command": "cargo test --all-targets -- --test-threads=1"}},
                    {"kind": "tool_result", "toolCallId": "call-big",
                     "text": "o".repeat(300), "isError": true},
                ],
            },
        ])
    }

    /// The journal dump parser: one JSON object per line, with the event's own
    /// `data` as an embedded, stringified document — exactly what
    /// `scripts/measure/verify-e2e-bytes.py` writes out of `run_events.payload`.
    /// Blank lines are skipped, and a field the agent did not send falls back to
    /// the value the schema means (`session_idx` of -1, an empty `data`), so a
    /// dump from an older agent still measures instead of panicking.
    #[test]
    fn a_journal_dump_parses_line_by_line_with_its_defaults() {
        let dump = concat!(
            r#"{"event_type":"text_chunk","data":"{\"text\":\"hi\"}","idx":3,"epoch":2,"timestamp":"t","session_idx":7,"run_sequence":9}"#,
            "\n\n",
            r#"{"event_type":"tool_end"}"#,
            "\n"
        );
        let journal = parse_journal(dump);
        assert_eq!(journal.len(), 2, "a blank line is not an event");
        assert_eq!(journal[0].event_type, "text_chunk");
        assert_eq!(journal[0].data, r#"{"text":"hi"}"#);
        assert_eq!(journal[0].idx, 3);
        assert_eq!(journal[0].epoch, 2);
        assert_eq!(journal[0].timestamp, "t");
        assert_eq!(journal[0].session_idx, 7);
        assert_eq!(journal[0].run_sequence, 9);
        assert_eq!(journal[1].event_type, "tool_end");
        assert_eq!(journal[1].data, "{}", "no data means an empty document");
        assert_eq!(journal[1].idx, 0);
        assert_eq!(journal[1].epoch, 0);
        assert_eq!(journal[1].session_idx, -1);
        assert_eq!(journal[1].timestamp, "");
        assert_eq!(journal[1].run_sequence, 0);
    }

    /// The measured body is what the publisher will really send: an event over
    /// the lane's cap is counted truncated, and a streamed type is counted as
    /// dropped rather than as an event the phone receives.
    #[test]
    fn an_oversized_event_is_measured_as_the_truncated_body() {
        let huge = "x".repeat(MAX_EVENT_BYTES + 5_000);
        let event = journal_event("text_chunk", json!({ "text": huge }), 1);
        let undeclared = expected_lane_data(&event, false).expect("kept undeclared");
        assert_ne!(
            undeclared, event.data,
            "the measured body is the cut one, not the source's"
        );
        let cut = undeclared.len();
        let source = event.data.len();
        assert!(cut < source, "the cut body is smaller: {cut} vs {source}");
        let lean = expected_lane_data(&event, true).expect("a text chunk is kept");
        assert_eq!(lean, undeclared, "the same cap applies on either lane");

        // A streamed type is not measured at all when the client declared it.
        let streamed = journal_event("thinking_delta", json!({ "text": huge }), 2);
        assert!(expected_lane_data(&streamed, true).is_none());
        assert!(expected_lane_data(&streamed, false).is_some());
    }

    /// The summary a measurement prints: sealed bytes, decrypted bytes, and how
    /// many entries the page held. A harness that reported the pre-seal size
    /// would understate every page by the record overhead it exists to measure.
    #[test]
    fn a_reply_summary_separates_the_wire_size_from_the_decrypted_one() {
        let capture = ReplyCapture {
            wire_bytes: 300,
            plaintext_bytes: 256,
            request_wire_bytes: 128,
            body: json!({ "data": { "entries": [{"id": "a"}, {"id": "b"}] } }),
        };
        assert_eq!(
            capture.summary(),
            json!({ "wireBytes": 300, "plaintextBytes": 256, "entries": 2 })
        );
        let empty = ReplyCapture {
            wire_bytes: 44,
            plaintext_bytes: 0,
            request_wire_bytes: 44,
            body: json!({ "data": null }),
        };
        assert_eq!(
            empty.summary()["entries"],
            json!(0),
            "a reply with no page has nothing to count"
        );
    }

    /// The real-broker URL is read from the environment the measurement script
    /// exports (`scripts/measure/verify-e2e-bytes.py --broker real`), and it is
    /// what both the URL accessor and the connect path use. The restore a
    /// driver does afterwards has two shapes — a value that was already there
    /// comes back, a var that was absent stays absent — and both are exercised
    /// here, because a measurement that leaves its inputs set would make the
    /// next run read the previous run's dump.
    #[test]
    fn a_real_broker_reads_its_url_from_the_environment() {
        // The same variable the driver tests set: serialized with them so a
        // concurrent writer cannot interleave, and restored by the closures
        // below so no value outlives this test (`FLAKES.md` item 27).
        let _env_lock = driver_env_lock();
        let original = std::env::var_os("VERIFY_E2E_NATS_URL");
        std::env::set_var("VERIFY_E2E_NATS_URL", "nats://127.0.0.1:14221");
        let previous = std::env::var_os("VERIFY_E2E_NATS_URL");

        std::env::set_var("VERIFY_E2E_NATS_URL", "nats://127.0.0.1:14222");
        let broker = Broker::real_from_env();
        assert_eq!(broker.url(), "nats://127.0.0.1:14222");

        for (value, expected) in [
            (previous.clone(), Some("nats://127.0.0.1:14221".to_string())),
            (None, None),
        ] {
            match value {
                Some(value) => std::env::set_var("VERIFY_E2E_NATS_URL", value),
                None => std::env::remove_var("VERIFY_E2E_NATS_URL"),
            }
            assert_eq!(
                std::env::var("VERIFY_E2E_NATS_URL").ok(),
                expected,
                "the restore puts back exactly what the run saw"
            );
        }
        match original {
            Some(value) => std::env::set_var("VERIFY_E2E_NATS_URL", value),
            None => std::env::remove_var("VERIFY_E2E_NATS_URL"),
        }
    }

    /// The readiness probe a real broker needs: subscribe, publish an echo on
    /// the *same* connection, and only then treat the subscription as live — a
    /// publish that overtook the SUB is dropped by the server, never retried, so
    /// without the probe the first measured message would be a race.
    #[tokio::test]
    async fn the_subscription_probe_round_trips_its_own_echo() {
        let nats = FakeNats::start().await;
        // The `Real` branch is the one under test, and it speaks the ordinary
        // NATS protocol through the same client the measurement uses: what it
        // needs is a broker that delivers a publish to the subscriber that made
        // it. (What a fake cannot stand in for is a *real server's* accounting,
        // which is read separately, over `/connz`.)
        let broker = Broker::Real {
            url: nats.url().to_string(),
        };
        let client = connect(broker.url(), "verify-probe-test").await;
        let bytes = broker
            .await_subscription(&client, "p.pair-x.evt.session")
            .await;
        assert_eq!(
            bytes,
            "verify-sub-probe".len() as u64,
            "the probe reports exactly what it put on the connection, so the \
             harness can subtract it from the broker's own accounting"
        );
    }

    /// The broker's accounting for the phone connection is read from its
    /// monitoring endpoint and found **by the name the harness gave that
    /// connection**. A `/connz` view without it has nothing to compare against
    /// and answers `None` — reporting zeroes instead would look like a
    /// perfectly quiet window and silently excuse any discrepancy.
    #[tokio::test]
    async fn the_phone_accounting_finds_the_connection_by_name() {
        let _agent_lock = mock_agent_lock();
        let e2e = E2e::on_broker(Broker::Fake(FakeNats::start().await), "verify-accounting").await;
        let phone = format!("verify-phone-{}", e2e.pair_id);
        let view = serve_connz(
            &json!({
                "connections": [
                    {"name": "verify-bridge-x", "in_msgs": 1, "in_bytes": 2,
                     "out_msgs": 3, "out_bytes": 4},
                    {"name": phone, "in_msgs": 11, "in_bytes": 12,
                     "out_msgs": 13, "out_bytes": 14},
                ]
            })
            .to_string(),
        )
        .await;
        let accounting = e2e
            .phone_accounting(&view)
            .await
            .expect("the phone's own connection is found, not the bridge's");
        assert_eq!(accounting.in_messages, 11);
        assert_eq!(accounting.in_bytes, 12);
        assert_eq!(accounting.out_messages, 13);
        assert_eq!(accounting.out_bytes, 14);

        let without_the_phone = serve_connz(
            &json!({"connections": [{"name": "someone-else", "in_msgs": 0}]}).to_string(),
        )
        .await;
        assert!(
            e2e.phone_accounting(&without_the_phone).await.is_none(),
            "no such connection is not a quiet one"
        );

        let not_a_list = serve_connz(&json!({"connections": {"name": phone}}).to_string()).await;
        assert!(
            e2e.phone_accounting(&not_a_list).await.is_none(),
            "a malformed view answers nothing rather than panicking"
        );
    }

    /// A replay longer than the bounded event queue has to drain while it
    /// publishes: an overflow drops events silently, which would shrink a
    /// measurement without failing it.
    #[tokio::test]
    async fn a_long_replay_drains_the_bounded_queue_while_it_publishes() {
        let _agent_lock = mock_agent_lock();
        let mut e2e =
            E2e::on_broker(Broker::Fake(FakeNats::start().await), "verify-long-replay").await;
        let session = format!("sess_{}", unique("long-replay"));
        // Every event is kept off the lean lane, so events in flight is the
        // journal's own length — 80 is past the queue's 64-event burst.
        let journal: Vec<JournalEvent> = (1..=80)
            .map(|idx| journal_event("text_chunk", json!({ "text": format!("chunk {idx}") }), idx))
            .collect();
        let capture = e2e.replay_live_lane(&session, "run-long", &journal).await;
        assert_lane(&capture, &journal, false, "long replay");
        assert_eq!(
            capture.received.len(),
            80,
            "the drain must not have dropped an event"
        );
        assert_eq!(e2e.phone_rx_messages, 80);
        assert_eq!(
            capture
                .received
                .last()
                .map(|event| event.body["idx"].clone()),
            Some(json!(80)),
            "the last source event arrives last"
        );
    }

    /// The real-traffic measurement body, over inputs this test owns. The
    /// ignored drivers read a live run's journal and one session's history page
    /// (that is what `scripts/measure/verify-e2e-bytes.py` exists for); what
    /// they *do* with it is this function, and this is where its claims run in
    /// the ordinary suite: the lean lane carries fewer bytes for the same
    /// events, the lean page never delivers fewer exchanges, and the one opened
    /// shell row gives back exactly the command the lean page dropped.
    #[tokio::test]
    async fn the_real_traffic_lane_holds_its_invariants_on_fixture_inputs() {
        run_real_traffic_with(
            Broker::Fake(FakeNats::start().await),
            &fixture_journal(),
            "sess_fixture_traffic",
            "run-verify",
            &traffic_entries(),
            None,
        )
        .await;
    }

    /// The catalog lane's measurement body, at this test's own row and write
    /// counts: what one second of a streaming run costs, for the client that
    /// declared the activity lane and for one that did not. The declaration is
    /// the whole difference, so the claim the measurement prints — declaring is
    /// cheaper than not declaring — is asserted rather than only reported.
    #[tokio::test]
    async fn the_catalog_measurement_publishes_activity_only_to_a_declared_client() {
        let declared = measure_catalog_writes(2, 2, true, "catalog-measure-declared").await;
        let undeclared = measure_catalog_writes(2, 2, false, "catalog-measure-undeclared").await;

        assert!(
            declared["activityPackets"].as_u64().unwrap_or_default() > 0,
            "a declared client receives the activity lane: {declared}"
        );
        assert_eq!(
            undeclared["activityPackets"],
            json!(0),
            "an undeclared client is not sent it: {undeclared}"
        );
        // The declared lane announces a change as an activity packet *instead
        // of* re-sending the whole catalogue, so it never sends more of them.
        assert!(
            declared["sessionsPackets"].as_u64().unwrap_or_default()
                <= undeclared["sessionsPackets"].as_u64().unwrap_or_default(),
            "the declared lane must not also re-send the catalogue snapshot: \
             {declared} vs {undeclared}"
        );
        assert!(
            declared["activityPayloadBytes"]
                .as_u64()
                .unwrap_or_default()
                > 0,
            "the activity packet carries the row it announces: {declared}"
        );
        assert_eq!(
            declared["records"],
            json!(
                declared["sessionsPackets"].as_u64().unwrap_or_default()
                    + declared["activityPackets"].as_u64().unwrap_or_default()
            ),
            "every published packet is one record on the wire"
        );
        assert_eq!(declared["rows"], json!(2));
        assert_eq!(declared["writes"], json!(2));
        assert!(declared["baselineBytes"].as_u64().unwrap_or_default() > 0);
    }

    /// One `/connz` answer on a loopback socket, plus the URL to ask it by.
    /// `nats-server`'s monitoring endpoint is the only source of the broker's
    /// own counters, and the harness has to read them the way it writes them.
    async fn serve_connz(body: &str) -> String {
        serve_connz_answers(body, 1).await
    }

    /// The same endpoint answering `answers` requests and then going away. The
    /// accounting window reads the counters twice (a baseline before the
    /// measured traffic, a second reading after it), so a fixture that drives
    /// both reads has to stay up for both.
    async fn serve_connz_answers(body: &str, answers: usize) -> String {
        let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
            .await
            .expect("bind loopback");
        let url = format!("http://{}", listener.local_addr().expect("addr"));
        let body = body.to_string();
        tokio::spawn(async move {
            for _ in 0..answers {
                let Ok((mut socket, _)) = listener.accept().await else {
                    return;
                };
                let mut request = [0u8; 2048];
                let _ = socket.read(&mut request).await;
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\
                     Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = socket.write_all(response.as_bytes()).await;
                let _ = socket.shutdown().await;
            }
        });
        url
    }

    // ── The `#[ignore]`d measurement drivers, run on the in-process broker ──
    //
    // The annotations stay: they name the external inputs a *real* run needs
    // (a `nats-server`, a live run's journal, a session's history page), and
    // they are the operator's instruments. What these tests add is the
    // observation that a driver's *body* is ordinary code: hand it those inputs
    // from a fixture and it must hold every invariant the measurement asserts.
    //
    // They are `#[test]`, not `#[tokio::test]`, on purpose. A `#[tokio::test]`
    // function is a plain blocking fn that builds its own runtime, so calling
    // one from inside a runtime panics ("Cannot start a runtime from within a
    // runtime"); the caller has to be the same kind of test libtest itself
    // would run. An external in-process broker is started on a runtime the
    // caller owns and keeps alive, because the driver cannot create it.

    /// Serializes the drivers' process-global inputs. `EnvGuard` is the
    /// *restore* (`FLAKES.md` item 4c: a lock is not a restore); this is only
    /// what stops two concurrent writers from interleaving.
    static DRIVER_ENV: std::sync::Mutex<()> = std::sync::Mutex::new(());

    pub(super) fn driver_env_lock() -> std::sync::MutexGuard<'static, ()> {
        DRIVER_ENV
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
    }

    /// The two on-disk shapes `scripts/measure/verify-e2e-bytes.py` writes,
    /// built from the same fixtures the in-module tests measure: a journal dump
    /// (one JSON object per line) and the bare entries array a history page
    /// was converted to.
    struct DriverInputs {
        _dir: tempfile::TempDir,
        /// Restored on drop, including while a driver panics mid-measurement.
        _guards: Vec<EnvGuard>,
    }

    impl DriverInputs {
        fn write(broker_url: &str) -> Self {
            let dir = tempfile::tempdir().expect("temp dir");
            let journal_path = dir.path().join("journal.jsonl");
            let mut dump = String::new();
            for event in fixture_journal() {
                dump.push_str(
                    &json!({
                        "event_type": event.event_type,
                        "data": event.data,
                        "idx": event.idx,
                        "epoch": event.epoch,
                        "timestamp": event.timestamp,
                        "session_idx": event.session_idx,
                        "run_sequence": event.run_sequence,
                    })
                    .to_string(),
                );
                dump.push('\n');
            }
            std::fs::write(&journal_path, dump).expect("write the journal dump");
            let entries_path = dir.path().join("entries.json");
            std::fs::write(&entries_path, traffic_entries().to_string())
                .expect("write the history dump");

            // `VERIFY_E2E_NATS_MONITOR` is deliberately left unset: the fake
            // broker has no `/connz`, and a scripted endpoint would have to
            // invent the counters the window compares against. The window is
            // asserted on its own terms by the tests below.
            let guards = vec![
                EnvGuard::set("VERIFY_E2E_JOURNAL", journal_path.as_os_str()),
                EnvGuard::set(
                    "VERIFY_E2E_SESSION",
                    std::ffi::OsStr::new("sess_fixture_traffic"),
                ),
                EnvGuard::set("VERIFY_E2E_RUN", std::ffi::OsStr::new("run-verify")),
                EnvGuard::set("VERIFY_E2E_ENTRIES", entries_path.as_os_str()),
                EnvGuard::set("VERIFY_E2E_NATS_URL", std::ffi::OsStr::new(broker_url)),
            ];
            Self {
                _dir: dir,
                _guards: guards,
            }
        }
    }

    /// An in-process broker on a runtime the caller keeps alive.
    fn broker_on_its_own_runtime() -> (tokio::runtime::Runtime, FakeNats) {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .enable_all()
            .build()
            .expect("broker runtime");
        let nats = runtime.block_on(FakeNats::start());
        (runtime, nats)
    }

    /// Both measurement drivers, through their own bodies, with the inputs the
    /// script would hand them.
    ///
    /// `measure_real_e2ee_bytes` starts its own broker; `measure_real_broker_bytes`
    /// takes one URL from `VERIFY_E2E_NATS_URL`, and the `Broker::Real` arm only
    /// needs an address — the in-process broker answers the probe publish that
    /// arm uses to prove the subscription is live. That is the same arm a real
    /// `nats-server` takes; what is *not* real here is the server's own
    /// accounting, which is why the window is asserted separately.
    #[test]
    fn the_ignored_measurement_drivers_run_their_own_bodies() {
        let _env_lock = driver_env_lock();
        let (broker_runtime, nats) = broker_on_its_own_runtime();
        let _inputs = DriverInputs::write(nats.url());

        super::measure_real_e2ee_bytes();
        super::measure_real_broker_bytes();

        drop(broker_runtime);
    }

    /// The real-broker fixture driver, through its own body, against the
    /// address `VERIFY_E2E_NATS_URL` names. Everything it asserts — the
    /// handshake, both declaration states, both history shapes — is the
    /// shipping behaviour; only the server under it is in-process.
    #[test]
    fn the_ignored_fixture_driver_runs_its_own_body() {
        let _env_lock = driver_env_lock();
        let (broker_runtime, nats) = broker_on_its_own_runtime();
        let _inputs = DriverInputs::write(nats.url());

        super::verify_e2e_real_broker_round_trip();

        drop(broker_runtime);
    }

    /// The catalog measurement, through its own body, at this test's own row
    /// and write counts (its two env inputs have in-code defaults — 232 rows,
    /// 20 writes — that would make the suite pay for a measurement).
    #[test]
    fn the_ignored_catalog_driver_runs_its_own_body() {
        let _env_lock = driver_env_lock();
        let _rows = EnvGuard::set("CATALOG_ACTIVITY_ROWS", std::ffi::OsStr::new("2"));
        let _writes = EnvGuard::set("CATALOG_ACTIVITY_WRITES", std::ffi::OsStr::new("2"));

        super::measure_catalog_activity_lane();
    }

    // ── The broker-accounting window, with both sides as arguments ──

    fn accounting(before: PhoneAccounting, out_bytes: u64, out_messages: u64) -> PhoneAccounting {
        PhoneAccounting {
            in_messages: before.in_messages + 2,
            in_bytes: before.in_bytes + 200,
            out_messages: before.out_messages + out_messages,
            out_bytes: before.out_bytes + out_bytes,
        }
    }

    /// A truthful broker, over a window that has a **non-zero baseline** and a
    /// **non-zero probe**: the equality only holds if both are subtracted, so
    /// an implementation that dropped either would fail here rather than pass
    /// for the want of a value to subtract.
    #[test]
    fn the_accounting_window_subtracts_the_baseline_and_the_probes() {
        let before = PhoneAccounting {
            in_messages: 3,
            in_bytes: 300,
            out_messages: 9,
            out_bytes: 4_096,
        };
        let after = accounting(before, 32 + 9_234, 2 + 55);
        super::assert_broker_accounting(
            before,
            after,
            PhoneReceipt {
                bytes: 9_234,
                messages: 55,
                probe_bytes: 32,
                probe_messages: 2,
            },
            "sess_window",
            "pair_window",
        );
    }

    /// One byte more than the phone decrypted is a broker that is accounting
    /// for something the client never received.
    #[test]
    #[should_panic(expected = "the broker must relay exactly the bytes the phone decrypted")]
    fn a_broker_one_byte_over_the_receipt_is_caught() {
        let before = PhoneAccounting::default();
        let after = accounting(before, 101, 5);
        super::assert_broker_accounting(
            before,
            after,
            PhoneReceipt {
                bytes: 100,
                messages: 5,
                probe_bytes: 0,
                probe_messages: 0,
            },
            "sess_window",
            "pair_window",
        );
    }

    /// Same for the message count: a relayed message the phone never saw is a
    /// lane the measurement is not accounting for.
    #[test]
    #[should_panic(expected = "the broker must relay exactly the messages the phone received")]
    fn a_broker_one_message_over_the_receipt_is_caught() {
        let before = PhoneAccounting::default();
        let after = accounting(before, 100, 6);
        super::assert_broker_accounting(
            before,
            after,
            PhoneReceipt {
                bytes: 100,
                messages: 5,
                probe_bytes: 0,
                probe_messages: 0,
            },
            "sess_window",
            "pair_window",
        );
    }

    /// The reading half: one `/connz` answer, parsed and handed to the same
    /// arithmetic. The endpoint is scripted — the in-process broker has no
    /// monitoring service — but the counters it serves are the ones a truthful
    /// server reports for this traffic: a non-zero baseline, the probe it
    /// relayed, and exactly what the phone decrypted. The two sides are
    /// independent only in the real-broker run; here the point is that the
    /// reading and the arithmetic compose, and that `Broker::Real`'s probe path
    /// (16 bytes and 1 message per replay) is what the subtraction removes.
    #[tokio::test]
    async fn the_accounting_window_reads_the_monitor_and_accepts_a_truthful_broker() {
        let _agent_lock = mock_agent_lock();
        let nats = FakeNats::start().await;
        // `Broker::Real` against the in-process broker: the only difference is
        // that this arm's subscription probe is a real publish/echo counted by
        // the server, which is what gives the window a non-zero probe to
        // subtract.
        let mut e2e = E2e::on_broker(
            Broker::Real {
                url: nats.url().to_string(),
            },
            "verify-window",
        )
        .await;
        let session = "sess_window";
        e2e.set_session_entries(session, &json!({ "entries": traffic_entries() }));
        e2e.replay_live_lane(session, "run-verify", &fixture_journal())
            .await;
        assert!(e2e.probe_bytes > 0, "the Real arm probes the subscription");

        let before = PhoneAccounting {
            in_messages: 3,
            in_bytes: 300,
            out_messages: 9,
            out_bytes: 4_096,
        };
        let after = PhoneAccounting {
            in_messages: before.in_messages + 2,
            in_bytes: before.in_bytes + 200,
            out_messages: before.out_messages + e2e.probe_messages + e2e.phone_rx_messages as u64,
            out_bytes: before.out_bytes + e2e.probe_bytes + e2e.phone_rx_bytes as u64,
        };
        let url = serve_connz(
            &json!({
                "connections": [{
                    "name": format!("verify-phone-{}", e2e.pair_id),
                    "in_msgs": after.in_messages,
                    "in_bytes": after.in_bytes,
                    "out_msgs": after.out_messages,
                    "out_bytes": after.out_bytes,
                }]
            })
            .to_string(),
        )
        .await;

        super::assert_broker_accounting_against(&e2e, before, &url, session).await;

        // And the same reading against a server that is one byte over: the
        // window has to reject it, or the reading is not feeding the assertion.
        let liar = PhoneAccounting {
            out_bytes: after.out_bytes + 1,
            ..after
        };
        let liar_url = serve_connz(
            &json!({
                "connections": [{
                    "name": format!("verify-phone-{}", e2e.pair_id),
                    "in_msgs": liar.in_messages,
                    "in_bytes": liar.in_bytes,
                    "out_msgs": liar.out_messages,
                    "out_bytes": liar.out_bytes,
                }]
            })
            .to_string(),
        )
        .await;
        let outcome = std::panic::AssertUnwindSafe(super::assert_broker_accounting_against(
            &e2e, before, &liar_url, session,
        ))
        .catch_unwind()
        .await;
        assert!(
            outcome.is_err(),
            "a monitor one byte over the receipt must be rejected"
        );
    }

    /// The baseline monitor reading runs before the measured traffic.
    ///
    /// The window's *second* reading cannot be scripted in advance — it reports
    /// the bytes the run itself produces — but the *baseline* reading has no
    /// such requirement: any answer for the phone's connection makes the
    /// `Some(url)` arm run, and the later comparison only needs `before` to be
    /// `Some`. This drives the shipping `run_real_traffic_with(.., Some(url))`
    /// against a loopback `/connz` that offers zeroed counters under every pair
    /// id the publisher could mint in this process.
    ///
    /// Reachability is the claim, so the zeroed monitor failing the receipt
    /// arithmetic is the expected outcome and is caught; the panic message is
    /// asserted to be that arithmetic (or the overflow its subtraction raises in
    /// a debug build), not the baseline's own `expect("phone connection")`,
    /// which would mean the connection was never found.
    #[tokio::test]
    async fn the_baseline_monitor_reading_runs_before_the_traffic() {
        let nats = FakeNats::start().await;
        let pid = std::process::id();
        // `E2e` mints the pair id at construction (`format!("pair_{}", unique("e2e"))`),
        // so the id cannot be known before the run; reserve the next counter
        // value and offer the ids that can follow it. `unique`'s counter is
        // process-global and the measured suite runs one test at a time, so the
        // pair id is the next value after the reservation; the wide window keeps
        // a margin for any helper that mints ids in between.
        let reserved = unique("baseline-monitor");
        let next: u64 = reserved
            .rsplit('-')
            .next()
            .and_then(|value| value.parse().ok())
            .expect("unique() ends in its counter");
        let rows: Vec<serde_json::Value> = (0..4096)
            .map(|offset| {
                json!({
                    "name": format!("verify-phone-pair_e2e-{pid}-{}", next + offset),
                    "in_msgs": 0u64,
                    "in_bytes": 0u64,
                    "out_msgs": 0u64,
                    "out_bytes": 0u64,
                })
            })
            .collect();
        let url = serve_connz_answers(&json!({ "connections": rows }).to_string(), 2).await;
        let journal = fixture_journal();
        let entries = traffic_entries();

        // NB: `run_real_traffic_with` takes `mock_agent_lock` itself; taking it
        // here first deadlocks the non-reentrant mutex.
        let outcome = std::panic::AssertUnwindSafe(super::run_real_traffic_with(
            Broker::Real {
                url: nats.url().to_string(),
            },
            &journal,
            "sess_baseline_monitor",
            "run-verify",
            &entries,
            Some(url),
        ))
        .catch_unwind()
        .await;
        let message = match outcome {
            Ok(()) => panic!("a zeroed monitor must not satisfy the receipt arithmetic"),
            Err(payload) => payload
                .downcast_ref::<String>()
                .cloned()
                .or_else(|| payload.downcast_ref::<&str>().map(|text| text.to_string()))
                .unwrap_or_default(),
        };
        assert!(
            message.contains("subtract with overflow")
                || message.contains("the broker must relay exactly"),
            "the baseline reading ran and the window rejected the monitor: {message}"
        );
    }
}

/// Two devices on one machine, on a real broker.
///
/// The three invariants this machine's arbitration promises exist *only* while
/// two devices are online at once — with one device connected, `superseded` and
/// "never reclaim by yourself" have nothing to arbitrate. On the in-process
/// `FakeNats` they hold vacuously: that fixture has a subscription table and
/// little else, and the ping-pong rule depends precisely on what it does not
/// model — real connection lifetimes, real delivery of a fire-and-forget
/// `state.control` publish, real per-connection ordering. So this module runs
/// the same production paths against a real `nats-server` when one is offered.
///
/// It is not a hard dependency: without `VERIFY_E2E_NATS_URL` the test skips
/// and prints why (see [`real_broker_or_skip`]). Start one with
/// `nats-server -a 127.0.0.1 -p <free>` and pass its URL.
///
/// What this still does **not** cover, and must not be reported as covered:
/// the platform's JWT issuance (a local broker does not check the platform's
/// tokens, so "does a *new* device get a valid credential" is untested here),
/// and two real devices across a network (NAT/relay paths, a phone losing
/// signal and coming back). Those need a deployment and two machines.
#[cfg(test)]
mod multi_device {
    use super::*;

    /// How long a "wait until X" is allowed to take. Generous enough for a
    /// loaded CI box, bounded so a regression fails instead of hanging.
    const DEADLINE: Duration = Duration::from_secs(20);

    /// The broker this machine can use, or `None` after saying why it skipped.
    fn real_broker_or_skip() -> Option<Broker> {
        // Measurement drivers temporarily replace this process-global input.
        // Read it under their lock, then release the lock before taking HOME.
        let _env_lock = super::tests::driver_env_lock();
        match std::env::var("VERIFY_E2E_NATS_URL") {
            Ok(url) if !url.trim().is_empty() => Some(Broker::Real { url }),
            _ => {
                eprintln!(
                    "SKIP verify_multi_device_real_broker: VERIFY_E2E_NATS_URL is not set, so \
                     there is no real broker to check multi-device arbitration on. Run \
                     `nats-server -a 127.0.0.1 -p <free>` and set VERIFY_E2E_NATS_URL=nats://127.0.0.1:<free> \
                     to run it. (The in-process FakeNats cannot stand in for this: it is the \
                     fixture whose thinness this test exists to get out from under.)"
                );
                None
            }
        }
    }

    /// One client: its own connection, its own secure channel to its own
    /// bridge, and a subscription to the control packet the machine sends when
    /// the device loses or gains the machine.
    struct DeviceLink {
        client: async_nats::Client,
        channel: future_remote_crypto::Channel,
        pair_id: String,
        control: async_nats::Subscriber,
        /// The client's Noise static key, kept so a reconnect can prove it is
        /// the same device instead of pairing again.
        private: [u8; 32],
    }

    impl DeviceLink {
        /// Connect and pair, exactly as a phone does: subscribe first (a
        /// supersede notice is fire-and-forget, so a subscription opened after
        /// the handshake would legitimately never see it), then run the
        /// production secure handshake.
        ///
        /// `previous` is the pairing identity of an earlier connection to the
        /// same device. With it, this is a **reconnect**: the invitation cannot
        /// be used twice, and a device coming back knows the desktop's static
        /// key, so it runs the real reconnect pattern instead of pairing again.
        async fn open(
            broker_url: &str,
            creds: &PairingCreds,
            name: &str,
            previous: Option<&DeviceLink>,
        ) -> Self {
            let pair_id = creds.pair_id.clone();
            let client = connect(broker_url, &format!("{name}-{pair_id}")).await;
            let control = client
                .subscribe(format!("p.{pair_id}.state.control"))
                .await
                .expect("subscribe to the control channel");
            // A real round trip: the SUB must be registered by the server before
            // anything is published to it, or the notice is silently lost and
            // the test would be asserting on a race rather than on the machine.
            let probe = format!("p.{pair_id}.state.control.verify-sub-probe");
            let mut echo = client.subscribe(probe.clone()).await.expect("probe sub");
            client
                .publish(probe.clone(), "probe".into())
                .await
                .expect("probe publish");
            tokio::time::timeout(Duration::from_secs(5), echo.next())
                .await
                .expect("the broker must echo the subscription probe")
                .expect("probe subscription ended");
            echo.unsubscribe().await.expect("probe unsubscribe");
            let (mut channel, private) = match previous {
                None => {
                    let (channel, private) =
                        super::super::test_support::open_secure_channel_keeping_identity(
                            &client,
                            &invitation(creds),
                            &pair_id,
                        )
                        .await;
                    (channel, private)
                }
                Some(previous) => (
                    super::super::test_support::reconnect_secure_channel(
                        &client,
                        &pair_id,
                        &creds.desktop_id,
                        &creds.secure.as_ref().expect("v2 identity").public_key,
                        &previous.private,
                    )
                    .await,
                    previous.private,
                ),
            };
            // Declare readiness sealed, as the real client does: this is also
            // what activates the desktop's channel for sealed traffic.
            let subject = format!("p.{pair_id}.cmd.ready");
            let ready = json!({ "type": "secure_ready" });
            let wire = channel
                .seal(
                    &subject,
                    &serde_json::to_vec(&ready).expect("ready serializes"),
                )
                .expect("ready seals");
            let reply = client
                .request(subject.clone(), wire.into())
                .await
                .expect("ready reply");
            let _ = reply;
            Self {
                client,
                channel,
                pair_id,
                control,
                private,
            }
        }

        /// Reconnect as the same device, without pairing again.
        async fn reconnect(&self, broker_url: &str, creds: &PairingCreds) -> Self {
            Self::open(broker_url, creds, "md-reconnect", Some(self)).await
        }

        /// One sealed business command, answered by the bridge. The error string
        /// is what the *wire* said, which is the point: a standby device must
        /// learn it from the machine, not from a flag this test can read.
        async fn command(&mut self, command: Value) -> Value {
            let subject = format!("p.{}.cmd.rpc", self.pair_id);
            let wire = self
                .channel
                .seal(
                    &subject,
                    &serde_json::to_vec(&command).expect("command serializes"),
                )
                .expect("command seals");
            let reply = self
                .client
                .request(subject.clone(), wire.clone().into())
                .await
                .expect("bridge reply");
            let context =
                future_remote_crypto::reply_context(&subject, &wire).expect("reply context");
            let plaintext = self
                .channel
                .open(&context, &reply.payload)
                .expect("reply decrypts");
            serde_json::from_slice(&plaintext).expect("reply is JSON")
        }

        /// Whether this device's business commands are accepted right now.
        async fn may_drive(&mut self, label: &str) -> bool {
            let reply = self
                .command(json!({ "id": unique(label), "type": "list_sessions" }))
                .await;
            if reply["success"] == json!(true) {
                true
            } else {
                assert_eq!(
                    reply["error"],
                    json!("superseded"),
                    "{label} was refused, but not as superseded: {reply}"
                );
                false
            }
        }

        /// Wait for a control packet saying `expected`.
        ///
        /// Packets already queued are examined rather than assumed absent: the
        /// machine also announces a device's *own* arrival on this channel, so
        /// an earlier `active` may be sitting in front of the `standby` this
        /// waits for.
        async fn await_control(&mut self, expected: &str) {
            let deadline = tokio::time::Instant::now() + DEADLINE;
            loop {
                let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
                let message = tokio::time::timeout(remaining, self.control.next())
                    .await
                    .unwrap_or_else(|_| {
                        panic!("timed out waiting for a `{expected}` control packet")
                    })
                    .expect("control subscription ended");
                let plaintext = self
                    .channel
                    .open(message.subject.as_str(), &message.payload)
                    .expect("control packet decrypts");
                let body: Value = serde_json::from_slice(&plaintext).expect("control is JSON");
                if body["control"] == json!(expected) {
                    return;
                }
            }
        }

        /// The control state named in the device's own `get_presence` answer —
        /// a second, independent path to the same fact (the heartbeat-driven
        /// payload, not the dedicated channel).
        async fn presence(&mut self) -> Value {
            let reply = self
                .command(json!({ "id": unique("presence"), "type": "get_presence" }))
                .await;
            assert_eq!(
                reply["success"],
                json!(true),
                "get_presence refused: {reply}"
            );
            reply["data"].clone()
        }

        async fn presence_control(&mut self) -> String {
            self.presence()
                .await
                .get("control")
                .and_then(Value::as_str)
                .map(str::to_string)
                .unwrap_or_else(|| panic!("no control field in presence"))
        }

        /// Declare what this device is, the way a real client does on
        /// `secure_ready`. The machine records it so a *standby* device can be
        /// told whom it would displace.
        async fn declare_identity(&mut self, name: &str, kind: &str) {
            let subject = format!("p.{}.cmd.ready", self.pair_id);
            let body = json!({
                "type": "secure_ready",
                "deviceName": name,
                "deviceKind": kind,
            });
            let wire = self
                .channel
                .seal(
                    &subject,
                    &serde_json::to_vec(&body).expect("ready serializes"),
                )
                .expect("ready seals");
            let reply = self
                .client
                .request(subject.clone(), wire.into())
                .await
                .expect("ready reply");
            let _ = reply;
        }

        /// Prove a subscription is registered on the broker before anything is
        /// published to it: a publish that overtakes the SUB is dropped, never
        /// retried, and the test would then be asserting on a race.
        async fn await_live(&self, subject: &str) {
            let probe = format!("{subject}.verify-sub-probe");
            let mut echo = self
                .client
                .subscribe(probe.clone())
                .await
                .expect("probe subscribe");
            self.client
                .publish(probe.clone(), "probe".into())
                .await
                .expect("probe publish");
            tokio::time::timeout(Duration::from_secs(5), echo.next())
                .await
                .expect("the broker must echo the subscription probe")
                .expect("probe subscription ended");
            echo.unsubscribe().await.expect("probe unsubscribe");
        }

        /// Whether any control packet arrives within `window`, without
        /// assuming which one. Used for "the machine said *nothing*".
        async fn control_pending(&mut self, window: Duration) -> bool {
            tokio::time::timeout(window, self.control.next())
                .await
                .map(|message| message.is_some())
                .unwrap_or(false)
        }

        /// Receive one event if one arrives within `window`.
        async fn event_within(
            &mut self,
            subscription: &mut async_nats::Subscriber,
            window: Duration,
        ) -> bool {
            tokio::time::timeout(window, subscription.next())
                .await
                .map(|message| message.is_some())
                .unwrap_or(false)
        }
    }

    /// Two devices installed on one machine, plus the machine's store.
    struct TwoDeviceMachine {
        _home: HomeGuard,
        a: InstalledSlot,
        b: InstalledSlot,
    }

    impl Drop for TwoDeviceMachine {
        fn drop(&mut self) {
            // The machine stops serving both devices, and the next fixture must
            // start from a machine with no devices and no controller.
            stop();
        }
    }

    async fn machine_with_two_devices(broker: &Broker, label: &str) -> TwoDeviceMachine {
        let home = HomeGuard::new(label);
        init_store();
        ensure_mock_agent();
        // Neither device has paired before, so each one's *first* secure
        // handshake is the act that puts it in front of the machine — the
        // production path a phone takes when it scans a code.
        let a = install_slot(v2_creds(broker), broker, false).await;
        let b = install_slot(v2_creds(broker), broker, false).await;
        // Two connected devices and no explicit choice: nobody drives. The
        // machine must not guess which one the user meant.
        SUPERVISOR.restore_active(None);
        TwoDeviceMachine { _home: home, a, b }
    }

    /// Poll until `check` holds, or fail naming what never happened.
    async fn until(label: &str, mut check: impl FnMut() -> bool) {
        let deadline = tokio::time::Instant::now() + DEADLINE;
        while tokio::time::Instant::now() < deadline {
            if check() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        panic!("timed out after {DEADLINE:?} waiting for: {label}");
    }

    /// Publish one live event for `session`, so a test's subscription and the
    /// event it waits for name the same lane.
    fn publish_one_event(session: &str, marker: &str) {
        super::publish_event(
            session,
            "thinking_delta",
            &json!({ "text": marker }).to_string(),
            "run-md",
            0,
            0,
            &format!("{session}:run-md:0:0"),
            "2026-10-18T00:00:00Z",
            0,
            0,
        );
    }

    /// The arbitration invariants, on a real broker, with two real bridges and
    /// two real client connections.
    ///
    /// The requested semantics: connecting does **not** displace anyone. A
    /// second device comes up in standby, is told who holds the machine, and its
    /// user decides; only an explicit `takeover` moves control. A device that
    /// lost the machine never takes it back by reconnecting.
    #[tokio::test]
    async fn a_second_device_is_told_who_holds_the_machine_and_only_takes_it_when_asked() {
        let Some(broker) = real_broker_or_skip() else {
            return;
        };
        let machine = machine_with_two_devices(&broker, "verify-multi-device").await;
        let broker_url = broker.url().to_string();

        // ── A connects: the machine is free, so A drives it ─────────────────
        let mut a = DeviceLink::open(&broker_url, &machine.a.creds, "md-a", None).await;
        a.declare_identity("A's phone", "mobile").await;
        until("A holds the machine after pairing", || {
            SUPERVISOR.in_control(&machine.a.pair_id)
        })
        .await;
        assert_eq!(a.presence_control().await, "active");
        assert!(
            a.presence().await["holder"].is_null(),
            "the controller is not displacing anybody"
        );

        // Live events reach the device that is driving.
        let a_evt_subject = format!("p.{}.evt.sess_md_a", machine.a.pair_id);
        let mut a_events = a
            .client
            .subscribe(a_evt_subject.clone())
            .await
            .expect("A subscribes to the live lane");
        a.await_live(&a_evt_subject).await;
        publish_one_event("sess_md_a", "to-a");
        assert!(
            a.event_within(&mut a_events, DEADLINE).await,
            "the device in control must receive live events"
        );

        // ── ① B connects ⇒ nobody is displaced ─────────────────────────────
        let mut b = DeviceLink::open(&broker_url, &machine.b.creds, "md-b", None).await;
        b.declare_identity("B's laptop", "desktop").await;
        // The handshake has been answered by now, so arbitration, if any, ran.
        assert!(
            SUPERVISOR.in_control(&machine.a.pair_id),
            "B's mere connection displaced A from the machine"
        );
        assert!(
            !SUPERVISOR.in_control(&machine.b.pair_id),
            "connecting must not be the same act as taking over"
        );
        // What B's user gets to see: standby, and who holds it.
        assert_eq!(b.presence_control().await, "standby");
        let holder = b.presence().await["holder"].clone();
        assert_eq!(
            holder["name"],
            json!("A's phone"),
            "the standby device must be told who it would displace: {holder}"
        );
        assert_eq!(holder["pairId"], json!(machine.a.pair_id));
        // B is refused over the wire, not by this process's opinion.
        assert!(!b.may_drive("b-before-consent").await);
        // A was told nothing and keeps driving: no notice, no break.
        assert!(a.may_drive("a-after-b-connected").await);
        assert_eq!(a.presence_control().await, "active");

        // Events still belong to A, and never reach B.
        let b_evt_subject = format!("p.{}.evt.sess_md_b", machine.b.pair_id);
        let mut b_events = b
            .client
            .subscribe(b_evt_subject.clone())
            .await
            .expect("B subscribes to its own lane");
        b.await_live(&b_evt_subject).await;
        publish_one_event("sess_md_a", "to-a-again");
        assert!(
            a.event_within(&mut a_events, DEADLINE).await,
            "the controller must keep receiving live events"
        );
        assert!(
            !b.event_within(&mut b_events, Duration::from_millis(300))
                .await,
            "a standby device must not receive the machine's events"
        );
        assert!(
            !a.control_pending(Duration::from_millis(300)).await,
            "the controller must not be told to stand by just because a device connected"
        );

        // ── ② B's user agrees ⇒ only then is A displaced ────────────────────
        let reply = b
            .command(json!({ "id": unique("takeover"), "type": "takeover" }))
            .await;
        assert_eq!(reply["success"], json!(true), "takeover refused: {reply}");
        assert_eq!(reply["data"]["control"], json!("active"));
        assert_eq!(
            reply["data"]["previous"],
            json!(machine.a.pair_id),
            "the takeover must name the device it displaced"
        );
        until("B holds the machine", || {
            SUPERVISOR.in_control(&machine.b.pair_id)
        })
        .await;
        // A learns it from the machine, on A's own channel, over the broker.
        a.await_control("standby").await;
        assert_eq!(a.presence_control().await, "standby");
        assert!(!a.may_drive("a-after-takeover").await);
        // …and events now follow B, not A.
        let a_gone = a
            .event_within(&mut a_events, Duration::from_millis(300))
            .await;
        publish_one_event("sess_md_b", "to-b");
        assert!(
            b.event_within(&mut b_events, DEADLINE).await,
            "the new controller must receive live events"
        );
        assert!(
            !a_gone,
            "the superseded device must not receive the controller's events"
        );
        assert!(b.may_drive("b-in-control").await);

        // ── ③ A reconnect cannot take it back by itself ─────────────────────
        //
        // The realistic trigger is a reconnect: A comes back up, replays its
        // handshake, and must not read "I am reconnecting" as "I want the
        // machine". Nothing here asks for control, so nothing may move.
        let mut a_again = a.reconnect(&broker_url, &machine.a.creds).await;
        // The old connection is gone; its replacement proved the same identity
        // over the reconnect pattern rather than redeeming the invitation.
        drop(a);
        assert!(
            SUPERVISOR.in_control(&machine.b.pair_id),
            "A's reconnect stole the machine back from B"
        );
        assert!(
            !SUPERVISOR.in_control(&machine.a.pair_id),
            "a reconnect must not put a device back in control"
        );
        assert!(
            !a_again.may_drive("a-reconnected").await,
            "the reconnected device was allowed to drive without asking"
        );
        // …and it is told whom it would displace, so its user can decide.
        assert_eq!(a_again.presence_control().await, "standby");
        assert_eq!(
            a_again.presence().await["holder"]["name"],
            json!("B's laptop"),
            "the reconnected device must be told who holds the machine now"
        );
        // B is untouched through all of it.
        assert!(b.may_drive("b-after-a-reconnect").await);
        assert_eq!(b.presence_control().await, "active");

        // ── …and A takes it back only because its user says so ─────────────
        let reply = a_again
            .command(json!({ "id": unique("takeover"), "type": "takeover" }))
            .await;
        assert_eq!(reply["success"], json!(true), "takeover refused: {reply}");
        assert_eq!(reply["data"]["previous"], json!(machine.b.pair_id));
        b.await_control("standby").await;
        assert!(!b.may_drive("b-displaced-again").await);

        drop(a_again);
        drop(b);
        drop(machine);
    }

    /// Both devices reconnect at the same instant, repeatedly.
    ///
    /// The single-device reconnect above proves one device does not reclaim.
    /// This is the race the rule actually has to survive: two devices coming
    /// back together, each re-deriving its own transport on a real broker,
    /// neither reading "I am reconnecting" as "I want the machine". A stub
    /// broker cannot expose it — there is no real reconnection to collide — and
    /// a settlement based on ordering rather than on an explicit request would
    /// show up here as control ping-ponging between rounds.
    ///
    /// Deterministic by construction: each round awaits both handshake replies
    /// (they are request/reply), then polls the pointer, and nothing sleeps for
    /// a fixed interval to "let things settle".
    #[tokio::test]
    async fn two_simultaneous_reconnects_leave_control_where_it_was() {
        let Some(broker) = real_broker_or_skip() else {
            return;
        };
        let machine = machine_with_two_devices(&broker, "verify-multi-reconnect-race").await;
        let broker_url = broker.url().to_string();

        let mut a = DeviceLink::open(&broker_url, &machine.a.creds, "race-a", None).await;
        until("A holds the machine after pairing", || {
            SUPERVISOR.in_control(&machine.a.pair_id)
        })
        .await;
        let mut b = DeviceLink::open(&broker_url, &machine.b.creds, "race-b", None).await;
        // B's connection did not displace A (that is the previous test's subject);
        // this one starts from a machine that is already settled.
        assert!(SUPERVISOR.in_control(&machine.a.pair_id));

        for round in 0..3 {
            // Both clients vanish at once and both come back at once. The join
            // is the whole point: their handshakes are genuinely in flight
            // together on the broker.
            let (a_next, b_next) = tokio::join!(
                a.reconnect(&broker_url, &machine.a.creds),
                b.reconnect(&broker_url, &machine.b.creds),
            );
            drop(a);
            drop(b);
            a = a_next;
            b = b_next;

            // The pointer must not have moved, and must not still be moving:
            // poll it rather than reading it once, so a claim that lands a tick
            // late is caught instead of raced with.
            let holder = machine.a.pair_id.clone();
            until(
                &format!("round {round}: the machine stays with the device that held it"),
                || SUPERVISOR.in_control(&holder) && !SUPERVISOR.in_control(&machine.b.pair_id),
            )
            .await;

            // …and that is what both devices experience, over the wire.
            assert!(
                a.may_drive(&format!("race-a-{round}")).await,
                "round {round}: the device that held the machine was refused after reconnecting"
            );
            assert!(
                !b.may_drive(&format!("race-b-{round}")).await,
                "round {round}: a device took the machine by reconnecting"
            );
            assert_eq!(b.presence_control().await, "standby");
            assert_eq!(a.presence_control().await, "active");
        }

        // The only thing that may move it still moves it: one explicit request.
        let reply = b
            .command(json!({ "id": unique("race-takeover"), "type": "takeover" }))
            .await;
        assert_eq!(reply["success"], json!(true), "takeover refused: {reply}");
        assert_eq!(reply["data"]["previous"], json!(machine.a.pair_id));
        until("B now holds the machine", || {
            SUPERVISOR.in_control(&machine.b.pair_id)
        })
        .await;
        a.await_control("standby").await;
        assert!(!a.may_drive("race-a-after-takeover").await);

        drop(a);
        drop(b);
        drop(machine);
    }
}
