//! Shared in-process test doubles for the remote bridge and provider tests:
//!
//! - [`ensure_mock_agent`]: a scripted gRPC `FutureAgent` server (tonic),
//!   started once per test process on an ephemeral port. `FUTURE_AGENT_GRPC_ADDR`
//!   is pointed at it before the first `connect_agent` — the agent channel is a
//!   process-global `OnceCell`, so every test that may reach the agent must call
//!   this first.
//! - [`FakeNats`]: a minimal core-NATS line-protocol server good enough for
//!   `async_nats` connect / (queue_)subscribe / publish / request-reply.
//! - [`MockPlatform`]: a scripted HTTP/1.1 server for the pairing control plane
//!   (`/client/v1/remote/...`).
//!
//! Scripting discipline (the mock agent is process-global): per-session
//! behavior is keyed by the UNIQUE session ids each test generates, so parallel
//! tests never steal each other's scripts. Session-less commands (`set_auth`,
//! `upsert_provider`, ...) are only scripted by tests that hold the
//! `TEST_HOME_LOCK` (via `HomeGuard`), which serializes them.

use super::SingleSlotMap;
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt};

pub(crate) use crate::auth_store::test_support::HomeGuard;

/// Unique-per-process id fragment so parallel tests never share session ids,
/// pair ids, or transfer subjects.
pub(crate) fn unique(tag: &str) -> String {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    format!(
        "{tag}-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    )
}

/// Initialize the GUI store (SQLite schema + dirs) under the current
/// (HomeGuard-redirected) HOME.
pub(crate) fn init_store() {
    crate::store::initialize_app_store().expect("store init under test HOME");
}

/// Credentials for a pairing that exists only inside the test process.
///
/// The per-pairing capability slots are keyed by `pair_id`, so this is what
/// makes a `secure_ready` declaration land where the publisher reads it.
pub(crate) fn pairing_creds(pair_id: &str) -> future_remote_server::PairingCreds {
    let expires_at = now_secs() + 3600;
    future_remote_server::PairingCreds {
        handshake_version: 2,
        secure: Some(
            crate::remote::secure::PairingIdentity::new(now_secs() + 600)
                .expect("pairing identity"),
        ),
        pair_id: pair_id.to_string(),
        desktop_id: format!("desktop_{}", unique("creds")),
        nkey_seed: nkeys::KeyPair::new_user()
            .seed()
            .expect("user seed")
            .to_string(),
        user_jwt: jwt(expires_at),
        nats_url: "nats://127.0.0.1:1".to_string(),
        nats_ws_url: "ws://127.0.0.1:1".to_string(),
        jwt_expires_at: expires_at,
        control_url: None,
    }
}

/// Declare a capability set the way a client's `secure_ready` does.
///
/// Through the production applier rather than by writing the flag directly: the
/// flags live in a per-pairing slot that answers an unknown pairing with a
/// fresh, disabled one, so a harness that skipped this step would measure the
/// *undeclared* lane while asserting about the declared one. Caller must have
/// installed the pairing's runtime (`supervisor::shared_runtime`, as
/// `supervisor::start` does before `build_transport`).
/// Declare (or withdraw) the lean feed for one pairing's connection.
///
/// The declaration is per connection now that a machine serves several clients,
/// so a fixture that wants a lean host must say so on *that* host's pairing.
pub(crate) fn set_pair_lean(pair_id: &str, enabled: bool) {
    crate::remote::SUPERVISOR
        .lean_events(pair_id)
        .store(enabled, std::sync::atomic::Ordering::Release);
}

pub(crate) fn declare_features(
    creds: &future_remote_server::PairingCreds,
    pair_id: &str,
    features: &[&str],
) {
    let handshake = crate::remote::commands::new_handshake_state(
        creds.clone(),
        Arc::new(std::sync::atomic::AtomicBool::new(true)),
        unique("bridge"),
    );
    let declared: Vec<String> = features
        .iter()
        .map(|feature| (*feature).to_string())
        .collect();
    crate::remote::commands::apply_declared_features(&handshake, pair_id, &declared);
}

/// Seed the Desktop projection a client's `list_sessions` would have shown: a
/// chat thread bound to that agent session.
///
/// A remote prompt names a conversation the client *listed*, so a fixture that
/// invents a session id has to give it a thread too. The host refuses an id no
/// thread here owns rather than answering it by creating a conversation of its
/// own (which would silently re-home the message); a bare `unique("sess")`
/// therefore describes something no client can send.
pub(crate) fn thread_for_session(session: &str) -> String {
    crate::store::create_thread(crate::store::CreateThreadInput {
        mode: "chat".to_string(),
        title: Some("fixture".to_string()),
        workspace_id: None,
        workspace_path: Some("/tmp/fixture".to_string()),
        workspace_name: Some("fixture".to_string()),
        agent_session_id: Some(session.to_string()),
    })
    .expect("fixture thread")
    .id
}

/// A direct connection to the GUI store's database, for fixtures the store's
/// own write path cannot express: a row left behind by an out-of-band delete
/// (a `runs` row whose thread is gone) or one whose column holds a value the
/// record decoder refuses. Foreign keys are off so such a row can be written,
/// and the caller must already hold the HOME guard that serializes the pool.
pub(crate) fn raw_store_connection() -> rusqlite::Connection {
    let path = crate::store::app_data_path()
        .expect("store path under the test HOME")
        .db_path;
    let conn = rusqlite::Connection::open(path).expect("open the store database");
    conn.execute_batch("PRAGMA foreign_keys = OFF;")
        .expect("fixture rows may describe a dangling parent");
    conn
}

/// Sign the test HOME in to FutureGene against `platform_url` (a MockPlatform),
/// writing the `future` auth entry the platform-URL resolver reads.
pub(crate) fn sign_in(platform_url: &str) {
    crate::auth_store::set_future_login("test-key", &format!("{platform_url}/api"))
        .expect("write test auth.json");
}

/// A syntactically valid JWT whose payload carries `exp` (unix seconds).
pub(crate) fn jwt(expires_at: i64) -> String {
    let payload = URL_SAFE_NO_PAD.encode(json!({ "exp": expires_at }).to_string());
    format!("hdr.{payload}.sig")
}

/// A v2 pairing code embedding `exp` (unix seconds), as the platform issues.
pub(crate) fn pairing_code(expires_at: i64) -> String {
    URL_SAFE_NO_PAD.encode(json!({ "exp": expires_at }).to_string())
}

pub(crate) fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or_default()
}

/// Complete the real v2 handshake over the fake broker, using the same
/// invitation a phone scans. Used by runtime and malicious-relay tests.
pub(crate) async fn secure_pair(
    client: &async_nats::Client,
    invitation: &str,
    pair_id: &str,
) -> future_remote_crypto::Channel {
    let mut channel = open_secure_channel(client, invitation, pair_id).await;
    let subject = format!("p.{pair_id}.cmd.handshake");
    let wire = channel
        .seal(
            &subject,
            &serde_json::to_vec(&json!({"type":"secure_ready"})).unwrap(),
        )
        .unwrap();
    let context = future_remote_crypto::reply_context(&subject, &wire).unwrap();
    let response = client.request(subject, wire.into()).await.unwrap();
    let ready: Value =
        serde_json::from_slice(&channel.open(&context, &response.payload).unwrap()).unwrap();
    assert_eq!(ready["success"], true);
    channel
}

/// The two *proving* legs of that handshake, stopped before the phone declares
/// readiness: the desktop holds a candidate channel it has not activated yet.
/// A test that needs "paired but not ready" (or that wants to drive
/// `secure_ready` itself) starts here.
pub(crate) async fn open_secure_channel(
    client: &async_nats::Client,
    invitation: &str,
    pair_id: &str,
) -> future_remote_crypto::Channel {
    open_secure_channel_keeping_identity(client, invitation, pair_id)
        .await
        .0
}

/// [`open_secure_channel`], also handing back the client's static private key.
///
/// A reconnect is a *different* Noise pattern (IK) that proves the same
/// identity instead of redeeming the invitation again — and the invitation is
/// one-shot, so the key is the only way back. A test that wants to reconnect as
/// the same device needs it.
pub(crate) async fn open_secure_channel_keeping_identity(
    client: &async_nats::Client,
    invitation: &str,
    pair_id: &str,
) -> (future_remote_crypto::Channel, [u8; 32]) {
    let url = reqwest::Url::parse(invitation).unwrap();
    let fields: HashMap<_, _> = url
        .query_pairs()
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect();
    let secret: [u8; 32] = URL_SAFE_NO_PAD
        .decode(&fields["secret"])
        .unwrap()
        .try_into()
        .unwrap();
    let desktop_key = URL_SAFE_NO_PAD.decode(&fields["secureKey"]).unwrap();
    let (private, _) = future_remote_crypto::generate_identity().unwrap();
    let prologue = future_remote_crypto::prologue(pair_id, &fields["desktopId"]).unwrap();
    let mut handshake = future_remote_crypto::Handshake::new(
        future_remote_crypto::Pattern::Pair,
        true,
        &private,
        None,
        Some(&secret),
        &prologue,
    )
    .unwrap();
    let subject = format!("p.{pair_id}.cmd.handshake");
    let first = json!({ "type": "secure_open", "pairing": true, "message": URL_SAFE_NO_PAD.encode(handshake.write(b"").unwrap()) });
    let response = client
        .request(subject.clone(), serde_json::to_vec(&first).unwrap().into())
        .await
        .unwrap();
    let response: Value = serde_json::from_slice(&response.payload).unwrap();
    assert_eq!(response["success"], true, "{response}");
    handshake
        .read(
            &URL_SAFE_NO_PAD
                .decode(response["data"]["message"].as_str().unwrap())
                .unwrap(),
        )
        .unwrap();
    assert_eq!(handshake.remote_public_key().unwrap(), desktop_key);
    let last = json!({ "type": "secure_finish", "id": response["data"]["id"], "message": URL_SAFE_NO_PAD.encode(handshake.write(b"").unwrap()) });
    let response = client
        .request(subject, serde_json::to_vec(&last).unwrap().into())
        .await
        .unwrap();
    let response: Value = serde_json::from_slice(&response.payload).unwrap();
    assert_eq!(response["success"], true, "{response}");
    let mut channel = handshake.finish().unwrap();
    let encrypted = URL_SAFE_NO_PAD
        .decode(response["data"]["confirmation"].as_str().unwrap())
        .unwrap();
    let confirmed: Value =
        serde_json::from_slice(&channel.open("handshake-confirm", &encrypted).unwrap()).unwrap();
    assert_eq!(confirmed["pairId"], pair_id);
    assert_eq!(confirmed["confirmed"], true);
    let private: [u8; 32] = private.as_slice().try_into().expect("32-byte identity key");
    (channel, private)
}

/// Reconnect to a desktop this device has already paired with.
///
/// The invitation is one-shot, so coming back is the *reconnect* pattern (IK):
/// the client proves the static key the desktop pinned during pairing, and the
/// desktop proves its own. `desktop_public` is that pinned key — read from the
/// pairing's `secure` identity, exactly as the real client stores it.
pub(crate) async fn reconnect_secure_channel(
    client: &async_nats::Client,
    pair_id: &str,
    desktop_id: &str,
    desktop_public: &str,
    private: &[u8; 32],
) -> future_remote_crypto::Channel {
    let prologue = future_remote_crypto::prologue(pair_id, desktop_id).unwrap();
    let desktop_key = URL_SAFE_NO_PAD.decode(desktop_public.as_bytes()).unwrap();
    let mut handshake = future_remote_crypto::Handshake::new(
        future_remote_crypto::Pattern::Reconnect,
        true,
        private,
        Some(&desktop_key),
        None,
        &prologue,
    )
    .expect("reconnect handshake");
    let subject = format!("p.{pair_id}.cmd.handshake");
    let first = json!({
        "type": "secure_open",
        "pairing": false,
        "message": URL_SAFE_NO_PAD.encode(handshake.write(b"").unwrap()),
    });
    let response = client
        .request(subject.clone(), serde_json::to_vec(&first).unwrap().into())
        .await
        .unwrap();
    let response: Value = serde_json::from_slice(&response.payload).unwrap();
    assert_eq!(response["success"], true, "reconnect refused: {response}");
    handshake
        .read(
            &URL_SAFE_NO_PAD
                .decode(response["data"]["message"].as_str().unwrap())
                .unwrap(),
        )
        .unwrap();
    let mut channel = handshake.finish().unwrap();
    let encrypted = URL_SAFE_NO_PAD
        .decode(response["data"]["confirmation"].as_str().unwrap())
        .unwrap();
    let confirmed: Value =
        serde_json::from_slice(&channel.open("handshake-confirm", &encrypted).unwrap()).unwrap();
    assert_eq!(confirmed["pairId"], pair_id);
    channel
}

// ── Reply sink ──────────────────────────────────────────────────────────────

/// Records every reply a command handler pushed, in order.
///
/// Handler-level tests drive `remote_host::business::execute` and
/// `PagedReply` directly instead of through NATS, so they need the handler's
/// *decision* (success flag, payload, error text) rather than a wire frame.
/// Keeping the replies in a list makes a multi-reply handler observable: a
/// handler that answers twice, or answers after `return`ing, cannot hide.
#[derive(Default)]
pub(crate) struct RecordingSink {
    replies: Mutex<Vec<(bool, Value, Option<String>)>>,
}

impl RecordingSink {
    pub(crate) fn len(&self) -> usize {
        self.replies.lock().unwrap().len()
    }

    /// The single reply this handler sent. Panics on zero or many so a test
    /// cannot silently pass on the wrong shape.
    pub(crate) fn only(&self) -> (bool, Value, Option<String>) {
        let replies = self.replies.lock().unwrap();
        assert_eq!(replies.len(), 1, "expected exactly one reply: {replies:?}");
        replies[0].clone()
    }

    /// The last reply, whatever came before it.
    pub(crate) fn last(&self) -> (bool, Value, Option<String>) {
        self.replies
            .lock()
            .unwrap()
            .last()
            .cloned()
            .expect("a reply")
    }

    // The `Result`-returning handlers are asserted through these two views, so
    // a test reads the same way whether the handler replied once or twice.
    pub(crate) fn ok_data(&self) -> Value {
        let (success, data, error) = self.last();
        assert!(success, "handler failed: {error:?}");
        data
    }

    pub(crate) fn error_text(&self) -> String {
        let (success, _, error) = self.last();
        assert!(!success, "handler unexpectedly succeeded");
        error.expect("a failed reply carries the reason")
    }
}

impl future_remote_server::services::ReplySink for RecordingSink {
    fn send<'a>(
        &'a self,
        success: bool,
        data: Value,
        error: Option<String>,
    ) -> futures::future::BoxFuture<'a, ()> {
        Box::pin(async move {
            self.replies.lock().unwrap().push((success, data, error));
        })
    }
}

// ── Mock gRPC FutureAgent ───────────────────────────────────────────────────

/// Serialize every test (across families) that scripts or drives the
/// process-global mock agent. The mock's one-shot script queue and the
/// persistent facade maps are process-global, so tests that mutate them must
/// never interleave — a remote test's leftover one-shot response would
/// otherwise be consumed by a concurrently running `commands` test (and vice
/// versa). The `commands::agent_mock` facade re-exports this same lock, so both
/// families agree on a single serialization point.
static MOCK_SER: Mutex<()> = Mutex::new(());

/// The one way to take [`MOCK_SER`] — and it must be taken **before** any
/// `HomeGuard` (`auth_store::test_support`) in the same test.
///
/// Both locks are process-global, and a test that takes them the other way round
/// deadlocks the whole binary against a test that takes them this way. Measured
/// 2026-10-17: `cargo test --lib -- remote_host::business::settings::tests
/// remote::commands::bridge_tests` hung for the full 300 s budget with every
/// thread at 0 % CPU: HOME held on one side waiting for MOCK, MOCK held on the
/// other side waiting for HOME (`settings.rs` was the inverted site).
/// `tests::no_body_takes_the_home_guard_before_the_mock_lock` enforces the order.
pub(crate) fn mock_agent_lock() -> MutexGuard<'static, ()> {
    MOCK_SER.lock().unwrap_or_else(|poison| poison.into_inner())
}

#[derive(Clone)]
struct ScriptedResponse {
    success: bool,
    data: String,
    error: String,
    typed: bool,
}

#[derive(Default)]
struct MockAgentState {
    /// (command, session_id) request log, for assertions.
    requests: Vec<(String, String)>,
    /// Last few full requests, so a test can assert the fields a forwarded
    /// read carried (run id, tool-call id) — the (command, session) log above
    /// answers "was this served", not "with what".
    full_requests: Vec<crate::agent_proto::RpcCommand>,
    /// One-shot scripted responses keyed by `command` or `command:session_id`.
    scripts: HashMap<String, VecDeque<ScriptedResponse>>,
    /// `get_session_entries` payloads keyed by session id.
    session_entries: HashMap<String, Value>,
    /// Session id → cwd recorded at `new_session` (so `get_state` answers
    /// consistently and the caller doesn't recreate the session).
    session_cwds: HashMap<String, String>,
    session_counter: u64,
    /// Persistent per-command canned answers installed wholesale by the
    /// `commands::agent_mock` facade (success payload / error message).
    /// Checked after one-shot scripts, before `default_answer`.
    persistent_data: HashMap<String, String>,
    persistent_errors: HashMap<String, String>,
    /// When true, every command except the connect-time health check
    /// (`list_streaming_sessions`) fails with `Code::Unavailable`.
    down: bool,
    /// Commands that fail with `Code::Unavailable` regardless of `down` —
    /// includes the health check itself, for RPC-error-arm tests.
    persistent_transport_fail: HashSet<String>,
    /// Complete the next run-scoped stream with `agent_end`. Session observers
    /// use an empty run id and do not consume this one-shot seam.
    complete_next_run_stream: bool,
}

/// Handle to the process-global mock agent.
#[derive(Clone)]
pub(crate) struct MockAgent {
    state: Arc<Mutex<MockAgentState>>,
}

impl MockAgent {
    pub(crate) fn complete_next_run_stream(&self) {
        self.state.lock().unwrap().complete_next_run_stream = true;
    }

    /// Enqueue a one-shot response for `command` (any session).
    pub(crate) fn script(&self, command: &str, success: bool, data: Value, error: &str) {
        self.script_for(command, "", success, data, error);
    }

    /// Enqueue a typed-only response addressed to one session.
    pub(crate) fn script_typed_for(&self, command: &str, session_id: &str, data: Value) {
        let key = format!("{command}:{session_id}");
        self.state
            .lock()
            .unwrap()
            .scripts
            .entry(key)
            .or_default()
            .push_back(ScriptedResponse {
                success: true,
                data: data.to_string(),
                error: String::new(),
                typed: true,
            });
    }

    /// Enqueue a one-shot response for `command` addressed to `session_id`.
    pub(crate) fn script_for(
        &self,
        command: &str,
        session_id: &str,
        success: bool,
        data: Value,
        error: &str,
    ) {
        let key = format!("{command}:{session_id}");
        self.state
            .lock()
            .unwrap()
            .scripts
            .entry(key)
            .or_default()
            .push_back(ScriptedResponse {
                success,
                data: data.to_string(),
                error: error.to_string(),
                typed: false,
            });
    }

    /// Set the `get_session_entries` payload for one session.
    pub(crate) fn set_session_entries(&self, session_id: &str, mut entries: Value) {
        // Keep convenient attachment-focused fixtures wire-valid now that the
        // desktop consumes the typed SessionEntry contract.
        for (index, entry) in entries
            .get_mut("entries")
            .and_then(Value::as_array_mut)
            .into_iter()
            .flatten()
            .enumerate()
        {
            let Some(object) = entry.as_object_mut() else {
                continue;
            };
            object
                .entry("id")
                .or_insert_with(|| Value::String(format!("mock-entry-{index}")));
            object
                .entry("role")
                .or_insert_with(|| Value::String("user".to_string()));
            let kind = object
                .get("role")
                .cloned()
                .unwrap_or_else(|| Value::String("user".to_string()));
            object.entry("kind").or_insert(kind);
            object
                .entry("createdAtMs")
                .or_insert_with(|| Value::Number(1_777_257_600_000_i64.into()));
            object
                .entry("blocks")
                .or_insert_with(|| Value::Array(Vec::new()));
        }
        self.state
            .lock()
            .unwrap()
            .session_entries
            .insert(session_id.to_string(), entries);
    }

    /// Snapshot of (command, session_id) pairs the mock has served.
    pub(crate) fn requests(&self) -> Vec<(String, String)> {
        self.state.lock().unwrap().requests.clone()
    }

    /// Install the persistent script used by the `commands::agent_mock`
    /// facade: canned per-command success payloads / error messages plus a
    /// `down` flag that makes readiness and business commands fail with
    /// `Code::Unavailable`, except the legacy `list_streaming_sessions` probe.
    /// Replaces the previous persistent script.
    pub(crate) fn set_persistent_script(
        &self,
        data: HashMap<String, String>,
        errors: HashMap<String, String>,
        down: bool,
        transport_fail: std::collections::HashSet<String>,
    ) {
        let mut state = self.state.lock().unwrap();
        state.persistent_data = data;
        state.persistent_errors = errors;
        state.down = down;
        state.persistent_transport_fail = transport_fail;
    }

    /// Drop every queued one-shot script. Callers holding [`mock_agent_lock`]
    /// clear stale scripts before scripting their own responses so a previous
    /// test family's leftovers can never leak in.
    pub(crate) fn clear_scripts(&self) {
        self.state.lock().unwrap().scripts.clear();
    }

    /// True when the mock served at least one `command` for `session_id`.
    pub(crate) fn served(&self, command: &str, session_id: &str) -> bool {
        self.requests()
            .iter()
            .any(|(c, s)| c == command && s == session_id)
    }

    /// Forget the request log, so a test can assert what the code under test
    /// asks for *from here on*. The log is process-global and the mock is
    /// shared, so a "was this command served" assertion needs a fresh start.
    pub(crate) fn clear_requests(&self) {
        let mut state = self.state.lock().unwrap();
        state.requests.clear();
        state.full_requests.clear();
    }

    /// The most recent full request for `command`, for assertions about the
    /// fields it carried (see [`MockAgentState::full_requests`]).
    pub(crate) fn last_full_request(
        &self,
        command: &str,
    ) -> Option<crate::agent_proto::RpcCommand> {
        self.state
            .lock()
            .unwrap()
            .full_requests
            .iter()
            .rev()
            .find(|request| request.r#type == command)
            .cloned()
    }
}

struct AgentService {
    state: Arc<Mutex<MockAgentState>>,
}

impl AgentService {
    fn answer(&self, cmd: crate::agent_proto::RpcCommand) -> crate::agent_proto::RpcResponse {
        let (success, data, error, typed) = {
            let mut state = self.state.lock().unwrap();
            state
                .requests
                .push((cmd.r#type.clone(), cmd.session_id.clone()));
            state.full_requests.push(cmd.clone());
            if state.full_requests.len() > 256 {
                state.full_requests.remove(0);
            }
            let key = format!("{}:{}", cmd.r#type, cmd.session_id);
            let scripted = state
                .scripts
                .get_mut(&key)
                .and_then(VecDeque::pop_front)
                .or_else(|| {
                    state
                        .scripts
                        .get_mut(cmd.r#type.as_str())
                        .and_then(VecDeque::pop_front)
                });
            match scripted {
                Some(response) => (
                    response.success,
                    response.data,
                    response.error,
                    response.typed,
                ),
                // The persistent facade (commands tests) — one-shot scripts
                // take precedence over it.
                None => {
                    if let Some(error) = state.persistent_errors.get(cmd.r#type.as_str()) {
                        (false, String::new(), error.clone(), false)
                    } else if let Some(data) = state.persistent_data.get(cmd.r#type.as_str()) {
                        (true, data.clone(), String::new(), false)
                    } else {
                        let (success, data, error) = default_answer(&cmd, &mut state);
                        (success, data, error, false)
                    }
                }
            }
        };
        let (data, payload) = if typed {
            let value = serde_json::from_str(&data).expect("scripted typed response JSON");
            let payload = future_rpc::encode::response_payload(&cmd.r#type, &value)
                .expect("scripted typed payload must match its command");
            (String::new(), Some(payload))
        } else {
            (data, None)
        };
        crate::agent_proto::RpcResponse {
            id: cmd.id.clone(),
            r#type: "response".to_string(),
            command: cmd.r#type.clone(),
            success,
            data,
            error,
            payload,
            ..Default::default()
        }
    }
}

fn ok(value: Value) -> (bool, String, String) {
    (true, value.to_string(), String::new())
}

/// Mirror the Agent's backward paging over a stored session.
///
/// A measurement that ignores `before`/`limit` hands the bridge a whole session,
/// and the bridge's 512 KiB budget then keeps *as many newest exchanges as fit* —
/// which is not the page the phone asks for. The phone sends `before` (its
/// backward cursor) plus `limit` user exchanges, so the page has to be selected
/// here the way `agent::session::history::paging::read_display_page` selects it: the newest
/// `limit` user entries ending at the cursor. Without this the harness overstates
/// a lean page several-fold (it fills the budget instead of stopping at the
/// requested exchanges).
fn paginate_backward(entries: Value, before: Option<i64>, limit: Option<i64>) -> Value {
    let mut stored = entries
        .get("entries")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let Some(before) = before else {
        return json!({ "entries": stored });
    };
    let total = stored.len() as i64;
    let end = before.clamp(0, total) as usize;
    let count = limit.unwrap_or(10).clamp(1, 100) as usize;
    let user_positions: Vec<usize> = stored
        .iter()
        .enumerate()
        .take(end)
        .filter(|(_, entry)| entry.get("role").and_then(Value::as_str) == Some("user"))
        .map(|(index, _)| index)
        .collect();
    let start = if user_positions.len() >= count {
        user_positions[user_positions.len() - count]
    } else {
        0
    };
    let page: Vec<Value> = stored.drain(start..end).collect();
    json!({
        "entries": page,
        "hasMore": start > 0,
        "nextOffset": start,
    })
}

fn default_answer(
    cmd: &crate::agent_proto::RpcCommand,
    state: &mut MockAgentState,
) -> (bool, String, String) {
    match cmd.r#type.as_str() {
        "get_session_presence" => ok(
            json!({"databaseId":"test-agent-database","restoreGeneration":"test-generation",
            "sessions":cmd.session_ids.iter().map(|id|json!({"sessionId":id,"status":"present"})).collect::<Vec<_>>()}),
        ),
        "delete_sessions" => ok(json!({"results": cmd.session_ids.iter().map(|session_id| {
            json!({"sessionId": session_id, "deleted": true,
                "error": "", "errorCode": "", "errorData": null})
        }).collect::<Vec<_>>()})),
        // A healthy mock Agent must satisfy the same readiness handshake as a
        // real Agent. Login commands now require this before requesting or
        // persisting a one-time credential.
        "get_agent_info" => ok(json!({
            "version": crate::build_info::VERSION,
            "agentInstanceId": "mock-agent",
            "skillsCount": 0,
        })),
        "get_agent_readiness" => ok(json!({
            "version": crate::build_info::VERSION,
            "agentInstanceId": "mock-agent",
            "protocolVersion": future_rpc::connection::AGENT_PROTOCOL_VERSION,
            "dataRootId": "0".repeat(64),
            "storageSchemaVersion": future_rpc::connection::AGENT_STORAGE_SCHEMA_VERSION,
        })),
        "list_streaming_sessions" => ok(json!({ "sessions": [] })),
        "probe_sandbox" => ok(json!({
            "available": cfg!(target_os = "macos"),
            "code": if cfg!(target_os = "macos") {
                "available"
            } else {
                "unsupported_platform"
            },
            "backend": if cfg!(target_os = "macos") {
                "macos_seatbelt"
            } else {
                "none"
            },
            "path": null,
            "version": null,
            "capabilities": null,
        })),
        "list_models" => ok(json!({
            "models": [],
            "defaultModel": "",
            "isScoped": false,
            "builtinProviders": {
                "deepseek": {
                    "name": "DeepSeek",
                    "modelCount": 3,
                    "baseUrl": "https://api.deepseek.com/v1",
                },
                "azure-openai-responses": {
                    "name": "Azure OpenAI Responses",
                    "modelCount": 1,
                    "baseUrl": "https://YOUR_RESOURCE.openai.azure.com/openai",
                },
                // Filtered out of the GUI catalog: FutureGene is presented
                // separately, and an empty id is invalid.
                "future": {
                    "name": "Future",
                    "modelCount": 9,
                    "baseUrl": "https://future-os.cn/api/v1",
                },
                "": { "name": "", "modelCount": 0, "baseUrl": "" },
                // A pre-display-name agent entry: the GUI falls back to the id.
                "noname": {
                    "name": "",
                    "modelCount": 1,
                    "baseUrl": "https://noname.example.com/v1",
                },
            },
        })),
        "list_providers" => ok(json!({
            "builtin": [
                {
                    "id": "future",
                    "name": "Future",
                    "baseUrl": "https://future-os.cn/api/v1",
                    "hasApiKey": false,
                    "modelCount": 9,
                    "requiresBaseUrl": false
                },
                {
                    "id": "deepseek",
                    "name": "DeepSeek",
                    "baseUrl": "https://api.deepseek.com/v1",
                    "hasApiKey": false,
                    "modelCount": 3,
                    "requiresBaseUrl": false
                },
                {
                    "id": "azure-openai-responses",
                    "name": "Azure OpenAI Responses",
                    "baseUrl": "https://YOUR_RESOURCE.openai.azure.com/openai",
                    "hasApiKey": false,
                    "modelCount": 1,
                    "requiresBaseUrl": true
                }
            ],
            "custom": []
        })),
        "get_messages" => ok(json!({
            "messages": [
                { "role": "user", "content": "hi" },
                { "role": "assistant", "content": "hello" },
            ],
        })),
        "get_session_entries" => {
            let entries = state
                .session_entries
                .get(&cmd.session_id)
                .cloned()
                .unwrap_or_else(|| json!({ "entries": [] }));
            ok(paginate_backward(entries, cmd.before, cmd.limit))
        }
        // The lean phone's way back to a shell call's dropped `arguments`.
        // Answered from the same recorded entries `get_session_entries` serves,
        // and scoped by run as well as id, so a mock cannot answer an identity
        // the real agent would refuse.
        "get_tool_call_args" => {
            let requested = cmd.tool_call_id.clone().unwrap_or_default();
            let found = state
                .session_entries
                .get(&cmd.session_id)
                .and_then(|entries| entries.get("entries"))
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter(|entry| entry.get("runId").and_then(Value::as_str) == Some(&cmd.run_id))
                .flat_map(|entry| entry["blocks"].as_array().into_iter().flatten())
                .find(|block| block.get("toolCallId").and_then(Value::as_str) == Some(&requested))
                .map(|block| (block.get("name"), block.get("arguments")));
            let (name, arguments) = match found {
                Some((name, arguments)) => (name.cloned(), arguments.cloned()),
                None => (None, None),
            };
            ok(json!({
                "toolCallId": requested,
                "name": name.unwrap_or(Value::Null),
                "arguments": arguments.unwrap_or(Value::Null),
            }))
        }
        "get_events_since" => ok(json!({ "events": [], "hasMore": false })),
        "get_state" => ok(json!({
            "sessionId": cmd.session_id,
            "cwd": state.session_cwds.get(&cmd.session_id).cloned().unwrap_or_default(),
            "isStreaming": false,
        })),
        "new_session" => {
            state.session_counter += 1;
            let session_id = format!("mock-session-{}", state.session_counter);
            state
                .session_cwds
                .insert(session_id.clone(), cmd.cwd.clone());
            ok(json!({ "sessionId": session_id }))
        }
        "prompt" => ok(json!({
            "run_id": cmd.requested_run_id,
            "accepted_state": "running",
        })),
        _ => ok(json!({})),
    }
}

#[tonic::async_trait]
impl crate::agent_proto::future_agent_server::FutureAgent for AgentService {
    async fn execute_command(
        &self,
        request: tonic::Request<crate::agent_proto::RpcCommand>,
    ) -> Result<tonic::Response<crate::agent_proto::RpcResponse>, tonic::Status> {
        let cmd = request.into_inner();
        // Down mode rejects readiness, so it cannot produce a verified client.
        // Preserve the legacy streaming-list probe for existing fixtures;
        // `persistent_transport_fail` may reject that command as well.
        let (down, transport_fail) = {
            let state = self.state.lock().unwrap();
            (
                state.down,
                state.persistent_transport_fail.contains(&cmd.r#type),
            )
        };
        if (down && cmd.r#type != "list_streaming_sessions") || transport_fail {
            return Err(tonic::Status::new(
                tonic::Code::Unavailable,
                "mock agent is down",
            ));
        }
        Ok(tonic::Response::new(self.answer(cmd)))
    }

    type StreamEventsStream = std::pin::Pin<
        Box<
            dyn futures::Stream<Item = Result<crate::agent_proto::StreamEvent, tonic::Status>>
                + Send,
        >,
    >;

    async fn stream_events(
        &self,
        request: tonic::Request<crate::agent_proto::StreamRequest>,
    ) -> Result<tonic::Response<Self::StreamEventsStream>, tonic::Status> {
        let request = request.into_inner();
        let complete = if request.run_id.is_empty() {
            false
        } else {
            let mut state = self.state.lock().unwrap();
            std::mem::take(&mut state.complete_next_run_stream)
        };
        if complete {
            let event = crate::agent_proto::StreamEvent {
                r#type: "agent_end".to_string(),
                data: r#"{"reason":"complete"}"#.to_string(),
                run_id: request.run_id,
                idx: 0,
                ..Default::default()
            };
            return Ok(tonic::Response::new(Box::pin(futures::stream::iter([Ok(
                event,
            )]))));
        }
        // An immediately-ending stream: prompt drivers observe the
        // stream-interrupted path (no agent_end), which is all the remote
        // tests need.
        Ok(tonic::Response::new(Box::pin(futures::stream::empty())))
    }
}

/// Accepted-connection stream for `serve_with_incoming` (avoids depending on
/// tokio-stream feature flags).
struct Incoming {
    listener: tokio::net::TcpListener,
}

impl futures::Stream for Incoming {
    type Item = Result<tokio::net::TcpStream, std::io::Error>;

    fn poll_next(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        self.listener
            .poll_accept(cx)
            .map(|result| Some(result.map(|(stream, _)| stream)))
    }
}

/// Start (once per process) the mock agent and point the GUI at it.
pub(crate) fn ensure_mock_agent() -> MockAgent {
    static MOCK: OnceLock<MockAgent> = OnceLock::new();
    static MOCK_ADDR: OnceLock<String> = OnceLock::new();
    let agent = MOCK
        .get_or_init(|| {
            let state = Arc::new(Mutex::new(MockAgentState::default()));
            let (port_tx, port_rx) = std::sync::mpsc::channel();
            let service_state = state.clone();
            std::thread::Builder::new()
                .name("mock-agent".to_string())
                .spawn(move || {
                    let runtime = tokio::runtime::Builder::new_multi_thread()
                        .worker_threads(1)
                        .enable_all()
                        .build()
                        .expect("mock agent runtime");
                    runtime.spawn(async move {
                        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
                            .await
                            .expect("bind mock agent");
                        port_tx
                            .send(listener.local_addr().expect("mock agent addr").port())
                            .expect("report mock agent port");
                        let server = tonic::transport::Server::builder()
                            .add_service(
                                crate::agent_proto::future_agent_server::FutureAgentServer::new(
                                    AgentService {
                                        state: service_state,
                                    },
                                ),
                            )
                            .serve_with_incoming(Incoming { listener });
                        // Serves until the test process exits; the outcome never
                        // materializes. `map` discards it without a block whose
                        // closing brace would count as an unreached line. Spawn on
                        // the ambient runtime so the outer `runtime` stays borrowable
                        // below (parking it keeps the server task alive).
                        tokio::spawn(futures::FutureExt::map(server, |_| ()));
                    });
                    // Park forever: dropping the runtime would kill the server task.
                    runtime.block_on(std::future::pending::<()>());
                })
                .expect("spawn mock agent thread");
            let port = port_rx.recv().expect("mock agent port");
            let addr = format!("127.0.0.1:{port}");
            let _ = MOCK_ADDR.set(addr);
            MockAgent { state }
        })
        .clone();
    // Re-assert the env var on every call so a test that briefly redirected
    // it (e.g. `commands::agent_mock::with_broken_endpoint`) is restored to
    // the live mock.
    if let Some(addr) = MOCK_ADDR.get() {
        std::env::set_var("FUTURE_AGENT_GRPC_ADDR", addr);
    }
    agent
}

// ── Fake NATS (core line protocol) ──────────────────────────────────────────

/// One published message observed by the fake server.
#[derive(Clone, Debug)]
pub(crate) struct Published {
    pub subject: String,
    #[allow(dead_code)] // populated for completeness; tests assert on `subject`
    pub reply: Option<String>,
    pub payload: Vec<u8>,
}

impl Published {
    pub(crate) fn json(&self) -> Value {
        serde_json::from_slice(&self.payload).expect("published payload is JSON")
    }
}

#[derive(Clone)]
struct NatsSub {
    pattern: String,
    queue: Option<String>,
    sid: String,
}

enum Out {
    Line(String),
    Msg {
        subject: String,
        sid: String,
        reply: Option<String>,
        payload: Vec<u8>,
    },
}

struct ConnHandle {
    subs: Vec<NatsSub>,
    tx: tokio::sync::mpsc::UnboundedSender<Out>,
}

#[derive(Default)]
struct NatsState {
    conns: HashMap<u64, ConnHandle>,
    next_id: u64,
}

/// A minimal in-process NATS server: INFO handshake, PING/PONG, SUB/UNSUB,
/// PUB with wildcard subject matching and queue-group single delivery.
pub(crate) struct FakeNats {
    url: String,
    state: Arc<Mutex<NatsState>>,
    tap: tokio::sync::broadcast::Sender<Published>,
    accept_task: tokio::task::JoinHandle<()>,
}

impl FakeNats {
    pub(crate) async fn start() -> Self {
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .expect("bind fake nats");
        let port = listener.local_addr().expect("fake nats addr").port();
        let state = Arc::new(Mutex::new(NatsState::default()));
        let (tap, _) = tokio::sync::broadcast::channel(256);
        let accept_state = state.clone();
        let accept_tap = tap.clone();
        let accept_task = tokio::spawn(async move {
            loop {
                if let Ok((stream, _)) = listener.accept().await {
                    let conn_id = {
                        let mut state = accept_state.lock().unwrap();
                        state.next_id += 1;
                        state.next_id
                    };
                    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
                    accept_state.lock().unwrap().conns.insert(
                        conn_id,
                        ConnHandle {
                            subs: Vec::new(),
                            tx,
                        },
                    );
                    tokio::spawn(serve_conn(
                        conn_id,
                        stream,
                        rx,
                        accept_state.clone(),
                        accept_tap.clone(),
                    ));
                }
            }
        });
        FakeNats {
            url: format!("nats://127.0.0.1:{port}"),
            state,
            tap,
            accept_task,
        }
    }

    pub(crate) fn url(&self) -> &str {
        &self.url
    }

    /// Observe every message published through the server (client or injected).
    pub(crate) fn tap(&self) -> tokio::sync::broadcast::Receiver<Published> {
        self.tap.subscribe()
    }

    /// Deliver a message as if published by a client (no sender needed).
    pub(crate) fn inject(&self, subject: &str, reply: Option<&str>, payload: Vec<u8>) {
        deliver(
            &self.state,
            &self.tap,
            subject,
            reply.map(str::to_string),
            payload,
        );
    }

    /// Block until some connection subscribes to `pattern` (or panic).
    pub(crate) async fn wait_for_sub(&self, pattern: &str, timeout: std::time::Duration) {
        let deadline = std::time::Instant::now() + timeout;
        loop {
            let found = self
                .state
                .lock()
                .unwrap()
                .conns
                .values()
                .any(|conn| conn.subs.iter().any(|sub| sub.pattern == pattern));
            if found {
                return;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "timed out waiting for a subscription to {pattern}"
            );
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
    }

    /// Sever every connection and stop accepting (drives client-side
    /// subscribe/stream failure paths).
    pub(crate) fn kill(self) {
        self.accept_task.abort();
        self.state.lock().unwrap().conns.clear();
    }
}

impl Drop for FakeNats {
    fn drop(&mut self) {
        self.accept_task.abort();
    }
}

/// Wait for a tapped publish on `subject`; panic after `timeout`.
pub(crate) async fn await_publish(
    rx: &mut tokio::sync::broadcast::Receiver<Published>,
    subject: &str,
    timeout: std::time::Duration,
) -> Published {
    let deadline = std::time::Instant::now() + timeout;
    loop {
        let remaining = deadline.saturating_duration_since(std::time::Instant::now());
        let outcome = tokio::time::timeout(remaining, rx.recv()).await;
        let published = outcome
            .ok()
            .and_then(Result::ok)
            .filter(|published| published.subject == subject);
        match published {
            Some(published) => return published,
            None if remaining.is_zero() => panic!("timed out waiting for a publish on {subject}"),
            None => continue,
        }
    }
}

/// Wait for a tapped publish on `subject` that also satisfies `matches`; panic
/// after `timeout`.
///
/// Use this when one subject carries several kinds of message: the presence
/// heartbeat keeps publishing `online: true` on the same subject that `stop()`
/// publishes `online: false` to, so matching on the subject alone can return the
/// heartbeat and race the assertion.
pub(crate) async fn await_publish_matching(
    rx: &mut tokio::sync::broadcast::Receiver<Published>,
    subject: &str,
    timeout: std::time::Duration,
    mut matches: impl FnMut(&Published) -> bool,
) -> Published {
    let deadline = std::time::Instant::now() + timeout;
    loop {
        let remaining = deadline.saturating_duration_since(std::time::Instant::now());
        let published = tokio::time::timeout(remaining, rx.recv())
            .await
            .ok()
            .and_then(Result::ok)
            .filter(|published| published.subject == subject && matches(published));
        match published {
            Some(published) => return published,
            None if remaining.is_zero() => {
                panic!("timed out waiting for a matching publish on {subject}")
            }
            None => continue,
        }
    }
}

/// Poll `done` until it returns true, panicking with `message` after `timeout`.
///
/// Prefer this over a fixed `sleep` + assert: an aborted read/write surfaces
/// much later on Windows than Linux's immediate refusal, so a fixed window
/// races the scheduler and fails intermittently on slower hosts.
pub(crate) async fn wait_until(
    message: &str,
    timeout: std::time::Duration,
    mut done: impl FnMut() -> bool,
) {
    let deadline = std::time::Instant::now() + timeout;
    loop {
        if done() {
            return;
        }
        if std::time::Instant::now() >= deadline {
            panic!("{message}");
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
}

/// Assert that no publish on `subject` is tapped within `window`.
pub(crate) async fn assert_no_publish(
    rx: &mut tokio::sync::broadcast::Receiver<Published>,
    subject: &str,
    window: std::time::Duration,
) {
    let deadline = std::time::Instant::now() + window;
    while let Ok(Ok(published)) = tokio::time::timeout(
        deadline.saturating_duration_since(std::time::Instant::now()),
        rx.recv(),
    )
    .await
    {
        assert_ne!(
            published.subject, subject,
            "unexpected publish on {subject}"
        );
    }
}

fn subject_matches(pattern: &str, subject: &str) -> bool {
    let pattern_tokens = pattern.split('.');
    let mut subject_tokens = subject.split('.');
    for pattern_token in pattern_tokens {
        if pattern_token == ">" {
            return true;
        }
        match subject_tokens.next() {
            Some(token) if pattern_token == "*" || pattern_token == token => {}
            _ => return false,
        }
    }
    subject_tokens.next().is_none()
}

fn deliver(
    state: &Arc<Mutex<NatsState>>,
    tap: &tokio::sync::broadcast::Sender<Published>,
    subject: &str,
    reply: Option<String>,
    payload: Vec<u8>,
) {
    let _ = tap.send(Published {
        subject: subject.to_string(),
        reply: reply.clone(),
        payload: payload.clone(),
    });
    let state = state.lock().unwrap();
    // Non-queue subscribers each receive a copy; queue groups receive exactly
    // one copy (first live member), keyed by (pattern, queue).
    let mut served_groups: Vec<(String, String)> = Vec::new();
    for conn in state.conns.values() {
        for sub in &conn.subs {
            if !subject_matches(&sub.pattern, subject) {
                continue;
            }
            if let Some(queue) = &sub.queue {
                let group = (sub.pattern.clone(), queue.clone());
                if served_groups.contains(&group) {
                    continue;
                }
                served_groups.push(group);
            }
            let _ = conn.tx.send(Out::Msg {
                subject: subject.to_string(),
                sid: sub.sid.clone(),
                reply: reply.clone(),
                payload: payload.clone(),
            });
        }
    }
}

/// Serialize one outbound frame; the caller treats any error as fatal.
async fn write_out(
    write_half: &mut tokio::net::tcp::OwnedWriteHalf,
    out: Out,
) -> Result<(), std::io::Error> {
    match out {
        Out::Line(line) => write_half.write_all(line.as_bytes()).await,
        Out::Msg {
            subject,
            sid,
            reply,
            payload,
        } => {
            let header = match &reply {
                Some(reply) => format!("MSG {subject} {sid} {reply} {}\r\n", payload.len()),
                None => format!("MSG {subject} {sid} {}\r\n", payload.len()),
            };
            let mut frame = header.into_bytes();
            frame.extend_from_slice(&payload);
            frame.extend_from_slice(b"\r\n");
            write_half.write_all(&frame).await
        }
    }
}

async fn serve_conn(
    conn_id: u64,
    stream: tokio::net::TcpStream,
    mut rx: tokio::sync::mpsc::UnboundedReceiver<Out>,
    state: Arc<Mutex<NatsState>>,
    tap: tokio::sync::broadcast::Sender<Published>,
) {
    let (read_half, mut write_half) = stream.into_split();
    let writer = tokio::spawn(async move {
        let mut alive = true;
        while alive {
            match rx.recv().await {
                Some(out) => alive = write_out(&mut write_half, out).await.is_ok(),
                None => alive = false,
            }
        }
    });

    let info = concat!(
        "INFO {\"server_id\":\"FAKE\",\"server_name\":\"fake-nats\",\"version\":\"2.10.2\",",
        "\"host\":\"127.0.0.1\",\"port\":0,\"max_payload\":8388608,\"client_id\":1,",
        "\"proto\":1,\"headers\":true,\"auth_required\":false,\"nonce\":\"dGVzdA==\"}\r\n"
    );
    state
        .lock()
        .unwrap()
        .conns
        .get(&conn_id)
        .expect("connection registered while its read loop runs")
        .tx
        .send(Out::Line(info.to_string()))
        .expect("fresh connection channel");

    let mut reader = tokio::io::BufReader::new(read_half);
    let mut line = Vec::new();
    loop {
        line.clear();
        let read = reader.read_until(b'\n', &mut line).await;
        match read {
            Ok(0) | Err(_) => break,
            Ok(_) => {}
        }
        let text = String::from_utf8_lossy(&line);
        let tokens: Vec<&str> = text.split_whitespace().collect();
        match tokens.first().copied() {
            Some("PING") => {
                // The connection entry lives until this read loop breaks, so
                // the lookup cannot fail here.
                let tx = state
                    .lock()
                    .unwrap()
                    .conns
                    .get(&conn_id)
                    .map(|conn| conn.tx.clone())
                    .expect("connection registered while its read loop runs");
                let _ = tx.send(Out::Line("PONG\r\n".to_string()));
            }
            Some("CONNECT") | Some("PONG") => {}
            Some("SUB") => {
                // SUB <subject> [queue] <sid>
                let sub = match tokens.as_slice() {
                    [_, subject, sid] => NatsSub {
                        pattern: subject.to_string(),
                        queue: None,
                        sid: sid.to_string(),
                    },
                    [_, subject, queue, sid] => NatsSub {
                        pattern: subject.to_string(),
                        queue: Some(queue.to_string()),
                        sid: sid.to_string(),
                    },
                    _ => continue,
                };
                state
                    .lock()
                    .unwrap()
                    .conns
                    .entry(conn_id)
                    .and_modify(|conn| conn.subs.push(sub));
            }
            Some("UNSUB") => {
                if let Some(sid) = tokens.get(1) {
                    state
                        .lock()
                        .unwrap()
                        .conns
                        .entry(conn_id)
                        .and_modify(|conn| conn.subs.retain(|sub| sub.sid != *sid));
                }
            }
            Some("PUB") => {
                // PUB <subject> [reply] <size>
                let (subject, reply, size) = match tokens.as_slice() {
                    [_, subject, size] => (*subject, None, size.parse::<usize>().unwrap_or(0)),
                    [_, subject, reply, size] => {
                        (*subject, Some(reply.to_string()), size.parse().unwrap_or(0))
                    }
                    _ => continue,
                };
                let mut payload = vec![0_u8; size];
                if reader.read_exact(&mut payload).await.is_err() {
                    break;
                }
                let mut crlf = [0_u8; 2];
                if reader.read_exact(&mut crlf).await.is_err() {
                    break;
                }
                deliver(&state, &tap, subject, reply, payload);
            }
            _ => {}
        }
    }
    state.lock().unwrap().conns.remove(&conn_id);
    writer.abort();
}

/// Connect an async-nats client to the fake server (no auth).
pub(crate) async fn nats_connect(nats: &FakeNats) -> async_nats::Client {
    async_nats::connect(nats.url())
        .await
        .expect("connect to fake nats")
}

/// Connect with reconnects disabled: when the server dies the connection
/// closes for good, so subscription streams end and publishes fail — the
/// shape the bridge's self-heal loops are built around. Note async-nats maps
/// `max_reconnects(0)` to `None` ("no limit"), so one allowed reconnect is
/// the least that still gives up: the first reconnect attempt trips the
/// MaxReconnects check and the client closes instead of buffering forever.
pub(crate) async fn nats_connect_once(nats: &FakeNats) -> async_nats::Client {
    async_nats::ConnectOptions::new()
        .max_reconnects(1)
        .connect(nats.url())
        .await
        .expect("connect to fake nats")
}

// ── Mock HTTP platform ──────────────────────────────────────────────────────

#[derive(Default)]
struct PlatformState {
    /// Scripted (status, body) responses keyed by path, served in order.
    scripts: HashMap<String, VecDeque<(u16, String)>>,
    /// (method, path, body) request log.
    requests: Vec<(String, String, String)>,
}

/// A scripted HTTP/1.1 server for the pairing control plane. Unscripted paths
/// answer 404 `{"error":"not_found"}`.
pub(crate) struct MockPlatform {
    url: String,
    state: Arc<Mutex<PlatformState>>,
    task: tokio::task::JoinHandle<()>,
}

impl MockPlatform {
    pub(crate) async fn start() -> Self {
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .expect("bind mock platform");
        let port = listener.local_addr().expect("mock platform addr").port();
        let state = Arc::new(Mutex::new(PlatformState::default()));
        let accept_state = state.clone();
        let task = tokio::spawn(async move {
            loop {
                if let Ok((stream, _)) = listener.accept().await {
                    tokio::spawn(serve_http(stream, accept_state.clone()));
                }
            }
        });
        MockPlatform {
            url: format!("http://127.0.0.1:{port}"),
            state,
            task,
        }
    }

    /// Platform root URL (no `/api`) for auth.json.
    pub(crate) fn url(&self) -> &str {
        &self.url
    }

    /// Enqueue a one-shot response for `path`.
    pub(crate) fn push(&self, path: &str, status: u16, body: Value) {
        self.state
            .lock()
            .unwrap()
            .scripts
            .entry(path.to_string())
            .or_default()
            .push_back((status, body.to_string()));
    }

    /// Snapshot of (method, path, body) requests served so far.
    pub(crate) fn requests(&self) -> Vec<(String, String, String)> {
        self.state.lock().unwrap().requests.clone()
    }

    /// Script a successful pair-code issuance pointing at `nats_url`.
    pub(crate) fn respond_pair_code(&self, nats_url: &str) -> String {
        self.respond_pair_code_for(&format!("pair_{}", unique("mock")), nats_url)
    }

    /// Script a pair-code issuance that reuses an existing `pair_id`, as the
    /// platform does when a desktop that is already known asks for another
    /// invitation (the previous one expired or was never scanned).
    pub(crate) fn respond_pair_code_for(&self, pair_id: &str, nats_url: &str) -> String {
        let code = pairing_code(now_secs() + 600);
        self.push(
            "/client/v1/remote/pair/code",
            200,
            json!({
                "pair_id": pair_id,
                "pairing_code": code,
                "user_jwt": jwt(now_secs() + 3600),
                "nats_url": nats_url,
                "nats_ws_url": nats_url.replace("nats://", "ws://"),
            }),
        );
        code
    }

    /// Script a successful bridge-JWT refresh pointing at `nats_url`.
    pub(crate) fn respond_refresh(&self, nats_url: &str) {
        self.push(
            "/client/v1/remote/auth/token",
            200,
            json!({
                "user_jwt": jwt(now_secs() + 3600),
                "nats_url": nats_url,
                "nats_ws_url": nats_url.replace("nats://", "ws://"),
            }),
        );
    }

    /// Script a revoked-credential refresh failure.
    pub(crate) fn respond_refresh_revoked(&self) {
        self.push(
            "/client/v1/remote/auth/token",
            401,
            json!({
                "error": "invalid_remote_credential",
                "message": "Remote credential is invalid or revoked.",
            }),
        );
    }
}

impl Drop for MockPlatform {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn serve_http(mut stream: tokio::net::TcpStream, state: Arc<Mutex<PlatformState>>) {
    let (read_half, mut write_half) = stream.split();
    let mut reader = tokio::io::BufReader::new(read_half);
    let mut request_line = String::new();
    if reader.read_line(&mut request_line).await.is_err() {
        return;
    }
    let parts: Vec<&str> = request_line.split_whitespace().collect();
    let (method, path) = match parts.as_slice() {
        [method, path, ..] => (method.to_string(), path.to_string()),
        _ => return,
    };
    let mut content_length = 0_usize;
    let mut header = String::new();
    loop {
        header.clear();
        match reader.read_line(&mut header).await {
            Ok(0) | Err(_) => return,
            Ok(_) => {}
        }
        let trimmed = header.trim();
        if trimmed.is_empty() {
            break;
        }
        if let Some(value) = trimmed
            .to_ascii_lowercase()
            .strip_prefix("content-length:")
            .and_then(|value| value.trim().parse::<usize>().ok())
        {
            content_length = value;
        }
    }
    let mut body = vec![0_u8; content_length];
    if reader.read_exact(&mut body).await.is_err() {
        return;
    }
    let body_text = String::from_utf8_lossy(&body).to_string();
    let (status, response_body) = {
        let mut state = state.lock().unwrap();
        state
            .requests
            .push((method.clone(), path.clone(), body_text));
        state
            .scripts
            .get_mut(&path)
            .and_then(VecDeque::pop_front)
            .unwrap_or((404, json!({ "error": "not_found" }).to_string()))
    };
    let reason = match status {
        200 => "OK",
        401 => "Unauthorized",
        404 => "Not Found",
        500 => "Internal Server Error",
        _ => "Status",
    };
    let response = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response_body}",
        response_body.len()
    );
    let _ = write_half.write_all(response.as_bytes()).await;
    let _ = stream.shutdown().await;
}

// ── A fake bridge in the process-global supervisor ──────────────────────────

/// A [`RemoteState`](crate::remote::RemoteState) wired to `nats`, with
/// placeholder tasks (pending forever) for the loops a test does not drive.
///
/// Kept here rather than in `tests::runtime_tests` because it is shared: the
/// in-module suite installs it directly, and [`install_fake_state`] installs it
/// for tests **outside** `src/remote/**` (the Tauri shell's remote-dependent
/// rows). One implementation, so the two fixtures cannot drift.
pub(in crate::remote) async fn fake_state(
    nats: &FakeNats,
    pair_id: &str,
) -> crate::remote::RemoteState {
    use crate::remote::{
        secure, spawn_event_publisher, DropCounters, NatsHealth, RemoteState, EVENT_QUEUE_CAPACITY,
    };

    let client = nats_connect(nats).await;
    let (event_tx, event_rx) = tokio::sync::mpsc::channel(EVENT_QUEUE_CAPACITY);
    let event_task = spawn_event_publisher(client.clone(), event_rx);
    RemoteState {
        security: secure::Transport::legacy_fixture(),
        generation_id: 1,
        client,
        nats_health: Arc::new(NatsHealth::default()),
        nats_url: nats.url().to_string(),
        pair_id: pair_id.to_string(),
        desktop_id: format!("desktop_{}", unique("rt")),
        desktop_public_key: "UPUBKEY".to_string(),
        bridge_instance_id: format!("bridge_{}", unique("rt")),
        event_tx,
        drop_counters: Arc::new(DropCounters::new()),
        event_task,
        cmd_task: crate::remote::SUPERVISOR.spawn_fixture_task(std::future::pending()),
        transfer_task: crate::remote::SUPERVISOR.spawn_fixture_task(std::future::pending()),
        heartbeat_task: crate::remote::SUPERVISOR.spawn_fixture_task(std::future::pending()),
        refresh_task: crate::remote::SUPERVISOR.spawn_fixture_task(std::future::pending()),
        web_task: None,
        web_url: None,
        web_lan_url: None,
        pairing_code: None,
        pairing_code_expires_at: None,
        pairing_confirmed: Arc::new(std::sync::atomic::AtomicBool::new(true)),
    }
}

/// A fake bridge installed into the process-global supervisor by
/// [`install_fake_state`], which clears it — the state and the broker behind
/// it — when dropped.
///
/// The restore is a `Drop` and not a lock for a measured reason: `FLAKES.md`
/// item 27 / §4c records a leaked process-global failing **27 unrelated tests**,
/// and a mutex that merely serialises global-state tests is not a restore. A
/// test that panics halfway through must still leave `SUPERVISOR.state` empty.
pub(crate) struct InstalledFakeState {
    /// The broker the installed bridge is connected to; held so the installer
    /// owns the whole lifetime. `Drop` takes it down with the state.
    broker: Option<FakeNats>,
}

impl Drop for InstalledFakeState {
    fn drop(&mut self) {
        // This runs while a panicking test unwinds, so it must not itself
        // panic: every lock below is poison-tolerant (an `assert!` that panics
        // while holding `SUPERVISOR.state` poisons it), and the teardown is the
        // synchronous task-abort path. In particular this is NOT `remote::stop()`:
        // that locks with `unwrap()` and may publish, either of which aborts the
        // process when a panic is already in flight.
        let installed = super::SUPERVISOR
            .state
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .take();
        if let Some(state) = installed {
            super::abort_generation(state);
        }
        // A test that drove `remote::start` left start intent behind (and, on a
        // successful start, a shared runtime); reset exactly what `stop()`
        // resets so the next test in this process starts from the same
        // baseline instead of inheriting a bridge that is half there.
        // Deliberately not reset: `last_error_code`, which `remote::stop()` also
        // keeps — a test that asserts on it must arrange its own baseline.
        super::SUPERVISOR
            .start_requested
            .store(false, std::sync::atomic::Ordering::Release);
        super::SUPERVISOR
            .invite_when_empty
            .store(true, std::sync::atomic::Ordering::Release);
        super::SUPERVISOR
            .suspended
            .store(false, std::sync::atomic::Ordering::Release);
        super::SUPERVISOR
            .bridge_shared
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .clear();
        // Control is process-global like the runtime it names: a test that
        // leaves a device in control would make the next one a standby.
        super::SUPERVISOR.restore_active(None);
        if let Some(broker) = self.broker.take() {
            broker.kill();
        }
    }
}

/// Install a complete fake remote bridge — its own in-process NATS broker,
/// a connected client, and the placeholder generation tasks — into the
/// process-global supervisor, for a test that lives **outside**
/// `src/remote/**`.
///
/// Why this exists: the Tauri shell's remote-dependent rows (`lib.rs`'s
/// auto-connect pass, `desktop_shutdown.rs`'s shutdown-deadline arm) need
/// `SUPERVISOR.state` to be `Some`, and both `RemoteState` and `SUPERVISOR` are
/// `pub(in crate::remote)` — a shell test can neither build nor install one.
/// This is the single supported route, so the shell does not replicate the
/// fixture.
///
/// Call it from inside a Tokio runtime context (the state owns spawned tasks
/// and the broker accepts connections on the caller's runtime) and hold the
/// returned guard for as long as the bridge is needed:
///
/// ```ignore
/// let _bridge = remote::test_support::install_fake_state("shell-row").await;
/// ```
///
/// The pair id is [`unique`]`(tag)`, so two tests can never share a subject.
/// Panics if a previous test leaked a bridge — the guard makes that a bug worth
/// failing on, not a state to inherit — and the guard, not the caller, is what
/// removes the bridge again.
pub(crate) async fn install_fake_state(tag: &str) -> InstalledFakeState {
    let broker = FakeNats::start().await;
    let state = fake_state(&broker, &unique(tag)).await;
    {
        let mut slots = super::SUPERVISOR
            .state
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        assert!(
            slots.is_empty(),
            "a previous test leaked SUPERVISOR.state; fix that test instead of \
             inheriting its bridge"
        );
        let pair_id = state.pair_id.clone();
        slots.insert(pair_id.clone(), state);
        drop(slots);
        // One device, so it is the controller — the same rule the runtime
        // applies. Without this the installed bridge would be a standby and
        // publish nothing, which looks exactly like a broken fixture.
        super::SUPERVISOR.restore_active(Some(pair_id));
    }
    InstalledFakeState {
        broker: Some(broker),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    /// The two process-global test locks have one legal acquisition order:
    /// `mock_agent_lock()` first, `HomeGuard` second.
    ///
    /// Both locks serialize process-global state (the mock agent's script queue
    /// here, `HOME`/the SQLite pool in `auth_store::test_support`), and a test
    /// that takes them in one order deadlocks against a test that takes them in
    /// the other: each holds what the other is waiting for. It is a *total*
    /// deadlock - every thread at 0 % CPU, no panic, no `test result:` line - and
    /// the harness can only report it as a timeout. Measured 2026-10-17 on this
    /// host: `cargo test --lib -- remote_host::business::settings::tests
    /// remote::commands::bridge_tests` hung for its full 300 s budget, 32 tests
    /// "running for over 60 seconds", and a stack sample showed
    /// `settings.rs` blocked in `mock_agent_lock` while holding the HOME guard and
    /// `commands::bridge_tests::active_bridge` (commands.rs) blocked in
    /// `HomeGuard::new` while holding MOCK.
    ///
    /// So the order is fixture contract, and this test enforces it by reading the
    /// crate's own sources: no function body may mention the HOME guard before its
    /// `mock_agent_lock(` call. Reading the source rather than the runtime state is
    /// deliberate - the failure this prevents cannot be reported from inside the
    /// test that is stuck, and a stack sample is what it costs to diagnose.
    ///
    /// **Scope of the check, stated so it is not mistaken for more:** it sees the
    /// shape within ONE body, which is what both known inversions had
    /// (`settings.rs`). An inversion routed through a *helper* (a body that builds
    /// a `HomeGuard`, called before a `mock_agent_lock` in its caller) is invisible
    /// here. The census that covered those walks the call graph over all 191 files
    /// of `src/`: 89 bodies take both locks, 2 were inverted, both in
    /// `remote_host/business/settings.rs` (recorded with the algorithm in
    /// docs/testing/flaky-tauri.md §1).
    #[test]
    fn no_body_takes_the_home_guard_before_the_mock_lock() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut violations = Vec::new();
        let mut scanned = 0_usize;
        for path in rust_sources(&root) {
            let source = std::fs::read_to_string(&path).expect("a crate source file readable");
            scanned += 1;
            for (name, body) in function_bodies(&source) {
                let home = ["HomeGuard::new", "TEST_HOME_LOCK"]
                    .iter()
                    .filter_map(|token| body.find(token))
                    .min();
                let mock = body.find("mock_agent_lock(");
                if let (Some(home), Some(mock)) = (home, mock) {
                    if home < mock {
                        violations.push(format!("{}: fn {name}", path.display()));
                    }
                }
            }
        }
        assert!(
            scanned > 100,
            "the scan must see the crate: {scanned} files"
        );
        assert!(
            violations.is_empty(),
            "these bodies take the HOME guard before the mock lock, which deadlocks \
             this test binary against `commands::bridge_tests` (take `mock_agent_lock()` \
             first; see this test's doc comment):\n  {}",
            violations.join("\n  ")
        );
    }

    /// Every `.rs` file under `dir`, depth-first. Sorted so a failure report is
    /// stable across runs and filesystems.
    fn rust_sources(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
        let mut out = Vec::new();
        let mut stack = vec![dir.to_path_buf()];
        while let Some(next) = stack.pop() {
            for entry in std::fs::read_dir(&next).expect("a readable directory") {
                let path = entry.expect("a directory entry").path();
                if path.is_dir() {
                    stack.push(path);
                } else if path.extension().is_some_and(|ext| ext == "rs") {
                    out.push(path);
                }
            }
        }
        out.sort();
        out
    }

    /// Each `fn` name with the text of its body, over a copy of `source` in which
    /// comments and string/char literals are blanked - so a doc comment that
    /// *names* a lock (this module is full of them) cannot be read as a call site.
    fn function_bodies(source: &str) -> Vec<(String, String)> {
        let text = blank_comments_and_literals(source);
        let mut out = Vec::new();
        let mut from = 0_usize;
        while let Some(rel) = text[from..].find("fn ") {
            let at = from + rel;
            from = at + 3;
            let Some(open_rel) = text[at..].find('{') else {
                break;
            };
            let open = at + open_rel;
            // A body-less declaration (`fn f();`) is not a site.
            if text[at..open].contains(';') {
                continue;
            }
            let mut depth = 0_usize;
            let mut close = text.len();
            for (index, ch) in text[open..].char_indices() {
                match ch {
                    '{' => depth += 1,
                    '}' => {
                        depth -= 1;
                        if depth == 0 {
                            close = open + index;
                            break;
                        }
                    }
                    _ => {}
                }
            }
            let name = text[at + 3..]
                .trim_start()
                .split(|c: char| !(c.is_alphanumeric() || c == '_'))
                .next()
                .unwrap_or_default()
                .to_string();
            out.push((name, text[at..close].to_string()));
        }
        out
    }

    /// `source` with `//`, `/* */`, `"…"`, `r#"…"#` and `'c'` replaced by spaces
    /// (newlines kept, so byte offsets and bodies stay aligned).
    fn blank_comments_and_literals(source: &str) -> String {
        let bytes = source.as_bytes();
        let mut out = source.to_string().into_bytes();
        let blank = |out: &mut Vec<u8>, range: std::ops::Range<usize>| {
            for byte in &mut out[range] {
                if *byte != b'\n' {
                    *byte = b' ';
                }
            }
        };
        let mut i = 0_usize;
        while i < bytes.len() {
            match bytes[i] {
                b'/' if bytes.get(i + 1) == Some(&b'/') => {
                    let end = source[i..].find('\n').map_or(bytes.len(), |off| i + off);
                    blank(&mut out, i..end);
                    i = end;
                }
                b'/' if bytes.get(i + 1) == Some(&b'*') => {
                    let mut end = i + 2;
                    let mut depth = 1_usize;
                    while end < bytes.len() && depth > 0 {
                        if bytes[end..].starts_with(b"/*") {
                            depth += 1;
                            end += 2;
                        } else if bytes[end..].starts_with(b"*/") {
                            depth -= 1;
                            end += 2;
                        } else {
                            end += 1;
                        }
                    }
                    blank(&mut out, i..end.min(bytes.len()));
                    i = end;
                }
                b'r' if matches!(bytes.get(i + 1), Some(b'#') | Some(b'"')) => {
                    let mut cursor = i + 1;
                    let mut hashes = 0_usize;
                    while bytes.get(cursor) == Some(&b'#') {
                        hashes += 1;
                        cursor += 1;
                    }
                    if bytes.get(cursor) == Some(&b'"') {
                        let close = format!("\"{}", "#".repeat(hashes));
                        let end = source[cursor + 1..]
                            .find(&close)
                            .map_or(bytes.len(), |off| cursor + 1 + off + close.len());
                        blank(&mut out, i..end);
                        i = end;
                    } else {
                        i += 1;
                    }
                }
                b'"' => {
                    let mut end = i + 1;
                    while end < bytes.len() {
                        match bytes[end] {
                            b'\\' => end += 2,
                            b'"' => {
                                end += 1;
                                break;
                            }
                            _ => end += 1,
                        }
                    }
                    blank(&mut out, i..end.min(bytes.len()));
                    i = end;
                }
                b'\'' => {
                    // A char literal (`'a'`, `'\n'`) is blanked; a lifetime (`'a`)
                    // is not a literal and stays as it is.
                    let rest = &source[i + 1..];
                    let len = rest
                        .char_indices()
                        .take(3)
                        .find(|(_, ch)| *ch == '\'')
                        .map(|(off, _)| off + 1);
                    match len {
                        Some(len) if len <= 4 => {
                            blank(&mut out, i..i + 1 + len);
                            i += 1 + len;
                        }
                        _ => i += 1,
                    }
                }
                _ => i += 1,
            }
        }
        String::from_utf8(out).expect("blanking only replaces bytes with spaces")
    }

    /// The scan's own control: it must flag the shape it exists to catch, and it
    /// must not be fooled by a doc comment that names the locks in the wrong
    /// order (which this module contains by construction).
    #[test]
    fn the_lock_order_scan_flags_an_inverted_body() {
        let inverted = "// mock_agent_lock() must come first\n\
             fn bad() {\n\
             let _home = crate::auth_store::test_support::HomeGuard::new(\"x\");\n\
             let _lock = mock_agent_lock();\n\
             }\n";
        let bodies = function_bodies(inverted);
        assert_eq!(bodies.len(), 1, "one body: {bodies:?}");
        let body = &bodies[0].1;
        assert!(
            body.find("HomeGuard::new") < body.find("mock_agent_lock("),
            "the scan must see HOME before MOCK in that body"
        );

        // The right order, and the wrong order *inside a doc comment or a
        // string*, must both stay unflagged.
        let fine = "fn good() {\n\
             let _lock = mock_agent_lock();\n\
             let _home = crate::auth_store::test_support::HomeGuard::new(\"x\");\n\
             }\n\
             fn commented() {\n\
             let _note = \"HomeGuard::new then mock_agent_lock\";\n\
             let _lock = mock_agent_lock();\n\
             }\n";
        for (name, body) in function_bodies(fine) {
            let home = ["HomeGuard::new", "TEST_HOME_LOCK"]
                .iter()
                .filter_map(|token| body.find(token))
                .min();
            let mock = body.find("mock_agent_lock(");
            assert!(
                home.is_none_or(|home| mock.is_none_or(|mock| home > mock)),
                "{name} must not look inverted: {body}"
            );
        }
    }

    /// A `fn` token with no `{` after it is not a call site.
    ///
    /// `function_bodies` finds the name and then the body's opening brace; the
    /// scan has to stop at the end of the text when there is no brace rather
    /// than slicing the rest of the file into one body. That body would then be
    /// attributed the wrong name and could hide (or invent) an inversion, which
    /// is the one failure this whole test exists to prevent. The crate's own
    /// sources contain no such token, so the branch is driven with one here.
    #[test]
    fn a_fn_without_a_body_is_not_a_call_site() {
        assert!(function_bodies("fn f").is_empty());
        assert!(
            function_bodies("fn declaration();").is_empty(),
            "a body-less declaration is not a site either"
        );
    }

    /// A nested block comment is one comment, not the outer two.
    ///
    /// Rust allows `/* /* */ */`, and the scanner has to skip the whole thing:
    /// stopping at the first `*/` would leave the tail of an inner comment
    /// looking like code, where a lock name could be read as a call site. The
    /// depth counter is what prevents that, and no comment in this crate is
    /// nested, so the input is crafted.
    #[test]
    fn a_nested_block_comment_is_blanked_whole() {
        let source = "a /* b /* c */ d */ e";
        let blanked = blank_comments_and_literals(source);

        assert_eq!(
            blanked.len(),
            source.len(),
            "blanking keeps offsets aligned"
        );
        assert!(
            !blanked.contains("/*") && !blanked.contains("*/"),
            "the whole nested comment must be gone, got {blanked:?}"
        );
        assert!(
            blanked.starts_with("a ") && blanked.ends_with(" e"),
            "only the comment may be blanked, got {blanked:?}"
        );
    }

    /// Read one CRLF-terminated line from a raw socket (with a deadline).
    async fn read_line(stream: &mut tokio::net::TcpStream) -> String {
        let mut bytes = Vec::new();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            let mut byte = [0_u8; 1];
            let remaining = deadline.saturating_duration_since(std::time::Instant::now());
            let read = tokio::time::timeout(remaining, stream.read(&mut byte))
                .await
                .expect("read deadline")
                .expect("read");
            if read == 0 || byte[0] == b'\n' {
                break;
            }
            bytes.push(byte[0]);
        }
        String::from_utf8_lossy(&bytes).to_string()
    }

    #[tokio::test]
    async fn fake_nats_tolerates_malformed_frames() {
        let nats = FakeNats::start().await;
        let addr = nats.url().trim_start_matches("nats://").to_string();
        let mut raw = tokio::net::TcpStream::connect(&addr).await.unwrap();
        // The INFO handshake arrives first; PING gets a PONG.
        assert!(read_line(&mut raw).await.starts_with("INFO"));
        raw.write_all(b"PING\r\n").await.unwrap();
        assert_eq!(read_line(&mut raw).await, "PONG\r");

        // Malformed control lines are skipped, not fatal.
        raw.write_all(b"SUB\r\n").await.unwrap(); // missing args
        raw.write_all(b"SUB only-subject\r\n").await.unwrap(); // missing sid
        raw.write_all(b"UNSUB\r\n").await.unwrap(); // no sid
        raw.write_all(b"HELLO there\r\n").await.unwrap(); // unknown verb
        raw.write_all(b"PUB\r\n").await.unwrap(); // malformed publish
        raw.write_all(b"PING\r\n").await.unwrap();
        assert_eq!(read_line(&mut raw).await, "PONG\r");
        drop(raw);

        // A payload truncated mid-body drops the connection.
        let mut raw = tokio::net::TcpStream::connect(&addr).await.unwrap();
        assert!(read_line(&mut raw).await.starts_with("INFO"));
        raw.write_all(b"PUB s 5\r\nab").await.unwrap();
        raw.shutdown().await.unwrap();
        drop(raw);
    }

    #[tokio::test]
    async fn fake_nats_drops_a_connection_with_a_truncated_payload() {
        let nats = FakeNats::start().await;
        let addr = nats.url().trim_start_matches("nats://").to_string();
        let mut raw = tokio::net::TcpStream::connect(&addr).await.unwrap();
        assert!(read_line(&mut raw).await.starts_with("INFO"));
        // Declare 5 payload bytes but send only 2, then half-close: the payload
        // read fails and the server must drop the connection.
        raw.write_all(b"PUB s 5\r\nab").await.unwrap();
        raw.shutdown().await.unwrap();
        drop(raw);
        // Wait until the server removes the half-read connection, making the
        // payload-read break under test deterministic.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            if nats.state.lock().unwrap().conns.is_empty() {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "truncated-payload connection never dropped"
            );
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
    }

    #[tokio::test]
    async fn fake_nats_drops_a_connection_missing_its_trailing_crlf() {
        let nats = FakeNats::start().await;
        let addr = nats.url().trim_start_matches("nats://").to_string();
        let mut raw = tokio::net::TcpStream::connect(&addr).await.unwrap();
        assert!(read_line(&mut raw).await.starts_with("INFO"));
        // Send a complete payload but omit the trailing CRLF, then half-close:
        // the server must drop the connection instead of hanging.
        raw.write_all(b"PUB s 2\r\nok").await.unwrap();
        raw.shutdown().await.unwrap();
        drop(raw);
        // Wait until the server removes the half-read connection (the trailing
        // CRLF read fails), making the break under test deterministic.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            if nats.state.lock().unwrap().conns.is_empty() {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "missing-CRLF connection never dropped"
            );
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
    }

    #[tokio::test]
    async fn fake_nats_delivers_a_queue_group_to_one_member_only() {
        let nats = FakeNats::start().await;
        let first = nats_connect(&nats).await;
        let second = nats_connect(&nats).await;
        use futures::StreamExt;
        let mut sub_a = first
            .queue_subscribe("q.subject".to_string(), "workers".to_string())
            .await
            .unwrap();
        let mut sub_b = second
            .queue_subscribe("q.subject".to_string(), "workers".to_string())
            .await
            .unwrap();
        // Both subscriptions must be live before the publish, or the dedup
        // branch under test never runs.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            let members = nats
                .state
                .lock()
                .unwrap()
                .conns
                .values()
                .flat_map(|conn| conn.subs.iter())
                .filter(|sub| sub.pattern == "q.subject")
                .count();
            if members == 2 {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "both queue subscriptions never registered"
            );
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
        // Both members share one (pattern, queue) group: each publish lands on
        // exactly one of them, never both.
        for _ in 0..2 {
            nats.inject("q.subject", None, b"x".to_vec());
        }
        let got_a = tokio::time::timeout(std::time::Duration::from_millis(200), sub_a.next())
            .await
            .ok()
            .flatten()
            .is_some();
        let got_b = tokio::time::timeout(std::time::Duration::from_millis(200), sub_b.next())
            .await
            .ok()
            .flatten()
            .is_some();
        assert!(got_a || got_b, "the group must receive the publish");
        assert_ne!(got_a, got_b, "only one group member may receive it");
    }

    #[tokio::test]
    #[should_panic(expected = "timed out waiting for a publish")]
    async fn await_publish_times_out_when_nothing_matches() {
        let nats = FakeNats::start().await;
        let mut tap = nats.tap();
        await_publish(
            &mut tap,
            "never.published",
            std::time::Duration::from_millis(30),
        )
        .await;
    }

    /// The matching variant must fail loudly too. It is the helper that decides
    /// whether a heartbeat or an offline notice was observed, so a typo in its
    /// predicate has to end the test rather than let it time out silently and
    /// pass on a stale value.
    #[tokio::test]
    #[should_panic(expected = "timed out waiting for a matching publish")]
    async fn await_publish_matching_times_out_when_no_publish_matches() {
        let nats = FakeNats::start().await;
        let mut tap = nats.tap();
        nats.inject("presence", None, b"{\"online\":true}".to_vec());
        // The subject matches but the predicate never does: the publish is
        // drained and the wait still has to end.
        await_publish_matching(
            &mut tap,
            "presence",
            std::time::Duration::from_millis(30),
            |published| published.json()["online"] == serde_json::json!(false),
        )
        .await;
    }

    /// `wait_until` is the fixture every "surface appears eventually" assertion
    /// rests on: if the condition never holds it must panic with its message
    /// rather than return and let the following assertion read a half-built
    /// state.
    #[tokio::test]
    #[should_panic(expected = "the surface never appeared")]
    async fn wait_until_panics_when_the_condition_never_holds() {
        wait_until(
            "the surface never appeared",
            std::time::Duration::from_millis(30),
            || false,
        )
        .await;
    }

    /// A non-object session entry is skipped instead of aborting the fixture:
    /// `set_session_entries` exists to make one session's history wire-valid,
    /// and a test that hands it a mixed list still wants the entries it could
    /// shape to be usable.
    #[tokio::test]
    async fn set_session_entries_shape_skips_non_object_entries() {
        let agent = ensure_mock_agent();
        let session = unique("sess-mixed-entries");
        agent.set_session_entries(
            &session,
            serde_json::json!({"entries": [{"role": "user"}, "not an object", 7]}),
        );
        let entries = agent.state.lock().unwrap().session_entries.clone();
        let shaped = entries.get(&session).expect("entry list stored");
        let shaped = shaped["entries"].as_array().expect("entries array");
        assert_eq!(shaped.len(), 3, "the list is kept as given: {shaped:?}");
        assert_eq!(shaped[0]["kind"], serde_json::json!("user"));
        assert_eq!(shaped[1], serde_json::json!("not an object"));
    }

    /// The mock's default answers are a contract, not filler: a healthy mock
    /// Agent has to satisfy the same readiness handshake as a real one, or
    /// every test that reaches the login/readiness path would fail against the
    /// double rather than against the code under test.
    #[test]
    fn the_mock_defaults_satisfy_the_agent_readiness_handshake() {
        let mut state = MockAgentState::default();
        let info = default_answer(
            &crate::agent_proto::RpcCommand {
                r#type: "get_agent_info".to_string(),
                ..Default::default()
            },
            &mut state,
        );
        assert!(info.0, "the readiness probe must succeed: {info:?}");
        let data: serde_json::Value = serde_json::from_str(&info.1).expect("JSON payload");
        assert_eq!(data["version"], crate::build_info::VERSION);
        assert_eq!(data["agentInstanceId"], "mock-agent");
        assert_eq!(data["skillsCount"], 0);

        let sandbox = default_answer(
            &crate::agent_proto::RpcCommand {
                r#type: "probe_sandbox".to_string(),
                ..Default::default()
            },
            &mut state,
        );
        assert!(sandbox.0, "an unsupported sandbox is not a failure");
        let data: serde_json::Value = serde_json::from_str(&sandbox.1).expect("JSON payload");
        // The mock must never claim a sandbox its own host does not have: the
        // desktop's approval tier follows this answer.
        assert_eq!(
            data["available"],
            serde_json::json!(cfg!(target_os = "macos"))
        );
        assert!(
            data["code"].as_str().is_some_and(|code| !code.is_empty()),
            "a probe reports a machine-readable reason: {data}"
        );
    }

    #[tokio::test]
    async fn assert_no_publish_fails_when_one_arrives() {
        let nats = FakeNats::start().await;
        let mut tap = nats.tap();
        nats.inject("some.subject", None, Vec::new());
        let result = tokio::spawn(async move {
            assert_no_publish(&mut tap, "some.subject", std::time::Duration::from_secs(1)).await;
        })
        .await
        .expect_err("a matching publish must fail the assertion");
        assert!(result.is_panic());
    }

    #[tokio::test]
    async fn mock_platform_handles_broken_requests() {
        // Install the crypto provider here rather than inheriting it from
        // whichever test happened to run first: without it, `Client::build`
        // panics and this test would pass or fail on test scheduling alone.
        crate::install_rustls_provider();
        let platform = MockPlatform::start().await;
        let addr = platform.url().trim_start_matches("http://").to_string();

        // Connect and close without sending anything.
        let raw = tokio::net::TcpStream::connect(&addr).await.unwrap();
        drop(raw);

        // Invalid UTF-8 in the request line fails the read outright.
        let mut raw = tokio::net::TcpStream::connect(&addr).await.unwrap();
        raw.write_all(b"\xff\xfe\r\n").await.unwrap();
        raw.shutdown().await.unwrap();
        drop(raw);

        // A request line without a path.
        let mut raw = tokio::net::TcpStream::connect(&addr).await.unwrap();
        raw.write_all(b"GARBAGE\r\n\r\n").await.unwrap();
        raw.shutdown().await.unwrap();
        drop(raw);

        // Headers that never terminate.
        let mut raw = tokio::net::TcpStream::connect(&addr).await.unwrap();
        raw.write_all(b"GET / HTTP/1.1\r\nHost: x\r\n")
            .await
            .unwrap();
        raw.shutdown().await.unwrap();
        drop(raw);

        // A body shorter than Content-Length.
        let mut raw = tokio::net::TcpStream::connect(&addr).await.unwrap();
        raw.write_all(b"POST /x HTTP/1.1\r\nContent-Length: 100\r\n\r\nshort")
            .await
            .unwrap();
        raw.shutdown().await.unwrap();
        drop(raw);

        // An unscripted path → 404; an unusual scripted status maps its name.
        platform.push("/redirect", 302, serde_json::json!({}));
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap();
        let response = client
            .get(format!("{}/redirect", platform.url()))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status().as_u16(), 302);
        let response = client
            .get(format!("{}/nowhere", platform.url()))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status().as_u16(), 404);
    }

    /// The mock's backward page is selected the way the Agent selects it: the
    /// newest `limit` *user* entries ending at the cursor, with everything after
    /// the oldest of them. A harness that ignored the limit would fill the byte
    /// budget instead and overstate every lean page it measured.
    #[test]
    fn paginate_backward_takes_the_newest_user_exchanges_up_to_the_limit() {
        let entries = json!({ "entries": [
            {"id": "u1", "role": "user", "blocks": []},
            {"id": "a1", "role": "assistant", "blocks": []},
            {"id": "u2", "role": "user", "blocks": []},
            {"id": "a2", "role": "assistant", "blocks": []},
            {"id": "u3", "role": "user", "blocks": []},
            {"id": "a3", "role": "assistant", "blocks": []},
        ] });

        // Two exchanges from the newest page: it starts at the second user row.
        let page = paginate_backward(entries.clone(), Some(6), Some(2));
        assert_eq!(
            page["entries"]
                .as_array()
                .unwrap()
                .iter()
                .map(|entry| entry["id"].as_str().unwrap())
                .collect::<Vec<_>>(),
            ["u2", "a2", "u3", "a3"],
            "the page starts at the second newest user row"
        );
        assert_eq!(page["hasMore"], json!(true));
        assert_eq!(page["nextOffset"], json!(2));

        // The whole history in one exchange: nothing more to scroll back to.
        let first = paginate_backward(entries.clone(), Some(6), Some(1));
        assert_eq!(first["nextOffset"], json!(4));
        let start_of_history = paginate_backward(entries, Some(2), Some(5));
        assert_eq!(start_of_history["hasMore"], json!(false));
        assert_eq!(start_of_history["nextOffset"], json!(0));

        // No cursor is the whole list: the first paint asks without `before`.
        let whole = paginate_backward(json!({ "entries": [] }), None, Some(3));
        assert_eq!(whole["entries"], json!([]));
    }

    // ── install_fake_state: the fixture the Tauri shell rows wait on ────────

    /// The install is a *live* bridge, not a stub that merely makes
    /// `state.is_some()` true, and `remote::start` — the call whose result the
    /// auto-connect pass matches on — is driveable from it.
    ///
    /// Outcome driven here: the **deferred** arm (`Ok(status)` with a phase
    /// other than `Ready`). In an isolated HOME there is no persisted pairing
    /// and no sign-in, so `establish` cannot mint an invitation and
    /// `remote::start` reports `Failed`/`account_authorization` instead of
    /// `Err`.
    #[tokio::test]
    async fn install_fake_state_installs_a_live_bridge_and_drives_the_deferred_arm() {
        let _home = HomeGuard::new("remote-install-fake-state");
        init_store();
        assert!(crate::remote::SUPERVISOR.state.lock().unwrap().is_empty());

        let installed = install_fake_state("shell-deferred").await;
        {
            let guard = crate::remote::SUPERVISOR.state.lock().unwrap();
            let state = guard.as_ref().expect("the fake bridge is installed");
            assert!(
                state.pair_id.starts_with("shell-deferred-"),
                "pair id must be unique per install, got {}",
                state.pair_id
            );
        }
        // A connected client, live tasks and no terminal health error: the
        // shell rows sit behind a *running* bridge, so the fixture has to be one.
        assert!(matches!(
            crate::remote::status().phase,
            crate::remote::RemotePhase::Ready
        ));

        let outcome = crate::remote::start(crate::remote::RemoteStartInput {}).await;
        match outcome {
            // The auto-connect pass's deferred arm.
            Ok(status) => {
                assert!(
                    !matches!(status.phase, crate::remote::RemotePhase::Ready),
                    "arm driven was Ready, not deferred: {status:?}"
                );
                assert_eq!(
                    status.reason,
                    Some(crate::remote::RemoteFailureReason::AccountAuthorization),
                    "arm driven was {status:?}"
                );
            }
            Err(error) => panic!("expected the deferred arm, got the failed arm: {error}"),
        }

        drop(installed);
        assert!(
            crate::remote::SUPERVISOR.state.lock().unwrap().is_empty(),
            "the guard must clear the bridge it installed"
        );
        assert!(!crate::remote::SUPERVISOR
            .start_requested
            .load(std::sync::atomic::Ordering::Acquire));
    }

    /// The second arm reachable without starting a real bridge: a persisted
    /// pairing whose stored NKey cannot be read fails *before* any HTTP or NATS
    /// call, and the error carries no category — so `remote::start` returns
    /// `Err`, which is exactly the arm the pass's `Err(error) => eprintln!`
    /// branch matches. (The `Ready` arm is the remaining one; it needs a full
    /// successful start — mock platform, broker and a real web listener — which
    /// is the route this fixture deliberately does not provide.)
    #[tokio::test]
    async fn install_fake_state_drives_the_failed_arm() {
        let _home = HomeGuard::new("remote-install-fake-state-failed");
        init_store();
        let mut creds = pairing_creds("pair_shell_failed");
        creds.nkey_seed = "this-is-not-a-seed".to_string();
        crate::remote::pairing::save_creds(&creds).expect("persist unusable pairing");

        let installed = install_fake_state("shell-failed").await;
        let outcome = crate::remote::start(crate::remote::RemoteStartInput {}).await;
        assert!(
            outcome.is_err(),
            "a local, uncategorized fault must reach the failed arm, got {outcome:?}"
        );
        drop(installed);
        assert!(crate::remote::SUPERVISOR.state.lock().unwrap().is_empty());
    }

    /// The restore must happen while a test unwinds, not only on the happy
    /// path: this is the destructive-leak shape (`FLAKES.md` item 27) the guard
    /// exists to rule out.
    #[test]
    fn install_fake_state_restores_when_the_test_panics() {
        let _home = HomeGuard::new("remote-install-fake-state-panic");
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("test runtime");

        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            runtime.block_on(async {
                let _installed = install_fake_state("shell-panic").await;
                assert!(crate::remote::SUPERVISOR.state.lock().unwrap().is_some());
                panic!("intentional: the guard must restore while unwinding");
            })
        }));

        assert!(outcome.is_err(), "the closure above panics by construction");
        assert!(
            crate::remote::SUPERVISOR.state.lock().unwrap().is_empty(),
            "a panicking test leaked the bridge into every later test"
        );
    }
}
