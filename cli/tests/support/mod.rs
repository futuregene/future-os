//! The fixture both `remote peers add` report tests need: **one** host bridge
//! that is serving and has no device bound yet, on its own home, mock platform
//! and fake broker.
//!
//! It lives in an integration target rather than in `commands/remote_peers/tests.rs`
//! because `add` shuts the peer runtime down before it returns — one pairing per
//! *process*, which is what a one-shot command deserves. The lib's test binary
//! already spends its one pairing on the text report
//! (`add_says_which_machine_it_paired_and_who_connects_it`), so each further arm
//! of the same command needs a binary of its own; this is the shared half.
//!
//! `Served` stands where `future_host::remote_peer::testing::fixture` does and
//! stops one step earlier, because that fixture *claims* the bridge for a device
//! and a second claim against the same bridge is answered `peer_mismatch`. `add`
//! is the claim, so the bridge has to be built without one.
//!
//! Two more properties of the fixture cost real time and are recorded where a
//! reader will hit them:
//!
//! * The runtime the bridge spawns on must outlive the test that installed it.
//!   `future_host::runtime::set` is a `OnceLock`, so a test-scoped handle stays
//!   installed and goes dead with the test (measured: three `headless::tests`
//!   discovery probes fail with "discovery timed out"). Hence the leaked,
//!   process-lifetime runtime below.
//! * The bridge's tasks need a `multi_thread` test runtime: they are spawned on
//!   that process-lifetime runtime, and the home guard's cleanup waits for them
//!   from the test thread — a current-thread runtime cannot poll the aborts while
//!   that thread is blocked (a five second deadline followed by a panic in a
//!   destructor).

#![allow(dead_code)] // each test binary uses a different part of the fixture

use serde_json::{json, Value};

/// A `future` invocation against the fixture's home, exactly as the lib tests
/// run one: the real dispatch, in this process, with captured output.
pub async fn run(args: &[&str]) -> (i32, String, String) {
    let args: Vec<String> = args.iter().map(|value| value.to_string()).collect();
    let (out, capture) = future_cli::Output::memory();
    let code = future_cli::dispatch(&args, &out).await;
    let stdout = String::from_utf8(capture.out.lock().unwrap().clone()).unwrap();
    let stderr = String::from_utf8(capture.err.lock().unwrap().clone()).unwrap();
    (code, stdout, stderr)
}

/// The runtime the host bridge spawns its supervisors and sockets on, installed
/// once per process and never dropped. See the module note.
fn host_runtime() -> tokio::runtime::Handle {
    static RUNTIME: std::sync::OnceLock<&'static tokio::runtime::Runtime> =
        std::sync::OnceLock::new();
    RUNTIME
        .get_or_init(|| {
            Box::leak(Box::new(
                tokio::runtime::Builder::new_multi_thread()
                    .worker_threads(2)
                    .enable_all()
                    .build()
                    .expect("host runtime"),
            ))
        })
        .handle()
        .clone()
}

/// A host bridge that is serving and has **no device bound yet**.
pub struct Served {
    home: future_host::remote::test_support::HomeGuard,
    platform: future_host::remote::test_support::MockPlatform,
    nats: future_host::remote::test_support::FakeNats,
    pair_id: String,
    host_invitation: String,
    claim_url: String,
    pub desktop_id: String,
}

impl Served {
    /// Build the bridge. `label` names the temporary home (and therefore the
    /// test), and must be unique across the binaries that share `/tmp`.
    pub async fn start(label: &str) -> Self {
        use future_host::remote::test_support::{HomeGuard, MockPlatform};
        use future_host::remote_peer::testing;

        let home = HomeGuard::new(label);
        // The host bridge spawns through the shared host runtime, which refuses
        // to run without an installed handle. This process is the entry point
        // here, so it installs one — exactly what `add` does for its own call.
        future_host::runtime::set(host_runtime());
        future_host::remote_peer::runtime::reset_for_test(&home).await;
        future_host::remote::test_support::init_store();
        let platform = MockPlatform::start().await;
        let nats = future_host::remote::test_support::FakeNats::start().await;
        let (pair_id, host_invitation) = testing::start_host(&platform, &nats).await;
        let desktop_id = reqwest::Url::parse(&host_invitation)
            .expect("the bridge shows a url")
            .query_pairs()
            .find(|(key, _)| key == "desktopId")
            .map(|(_, value)| value.into_owned())
            .expect("the invitation names the machine");
        let claim_url = format!("{}/client/v1/remote/pair/claim", platform.url());
        Self {
            home,
            platform,
            nats,
            pair_id,
            claim_url,
            host_invitation,
            desktop_id,
        }
    }

    /// The machine's own root, for the files this command leaves behind.
    pub fn future_home(&self) -> std::path::PathBuf {
        self.home.path().join(".future")
    }

    /// Script one claim the way the platform answers one — the grant, and the
    /// endpoint the credential refresh resolves — then hand back an invitation
    /// for it. Built from this bridge's own invitation, so the PSK and the Noise
    /// keys are the ones it will answer for.
    pub fn invitation(&self, tag: &str) -> String {
        self.platform.push(
            "/client/v1/remote/pair/claim",
            200,
            json!({
                "pair_id": self.pair_id,
                "user_jwt": future_host::remote::test_support::jwt(
                    future_host::remote::test_support::now_secs() + 3_600
                ),
                "refresh_token": tag,
                "nats_ws_url": self.nats.url().replace("nats://", "ws://"),
            }),
        );
        self.platform.respond_refresh(self.nats.url());
        future_host::remote_peer::testing::invitation_for(&self.host_invitation, &self.claim_url)
    }

    /// The peer the book on disk holds, as the next start will read it.
    pub fn paired_book(&self) -> Value {
        let path = self.future_home().join("remote_peers.json");
        serde_json::from_str(
            &std::fs::read_to_string(path).expect("the book was written by the pairing"),
        )
        .expect("the book is JSON")
    }

    /// Stop the bridge before the home guard drops: a bridge left running keeps
    /// the guard's cleanup waiting for its supervisor, and a task that cannot
    /// stop aborts the test process instead of failing one test.
    pub async fn finish(self) {
        future_host::remote_peer::testing::teardown(&self.pair_id).await;
    }
}

/// Hold the Desktop's own lock for as long as this lives, so `desktop_running()`
/// answers "yes" — the same file `instance::InstanceGuard` locks.
pub struct RunningDesktop {
    _file: std::fs::File,
}

impl RunningDesktop {
    pub fn claim(future_home: &std::path::Path) -> Self {
        let app = future_home.join("app");
        std::fs::create_dir_all(&app).unwrap();
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(app.join("desktop.lock"))
            .unwrap();
        fs2::FileExt::lock_exclusive(&file).unwrap();
        RunningDesktop { _file: file }
    }
}
