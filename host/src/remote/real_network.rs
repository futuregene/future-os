//! Real-network, **two-process** exercise of the remote stack.
//!
//! Everything else in this crate meets a host bridge and its client inside one
//! process: the fake fixtures (and `verify_e2e`'s real-broker fixture) collapse
//! the two ends, so a defect in the *transport boundary* — a socket that a
//! process really owns, a broker that really relays, a peer that really
//! restarts — has nowhere to appear. This module runs the host bridge and the
//! client in two OS processes against a real `nats-server`, with a real
//! `future-agent` behind the host and a real file transfer in both directions.
//!
//! What is real and what is not, stated up front:
//!
//! * real: `nats-server` process, TCP, Noise v2 handshake and per-record AEAD,
//!   NKey signing on both ends, the production command loop / event publisher /
//!   transfer loop, process isolation of the two ends, and the `future-agent`
//!   process behind the host (a separate binary, its own HOME and database);
//! * stubbed: the *model vendor*. A local OpenAI-compatible endpoint answers
//!   the agent's requests (see `docs/testing/real-network.md`); the Agent, its
//!   gRPC contract, its run journal and its session persistence are the real
//!   ones;
//! * bypassed: the FutureOS platform's pair-code/claim endpoints. The host mints
//!   its v2 invitation locally (the same `PairingCreds` the GTT/claim endpoint
//!   would return). Pairing *policy* lives behind that HTTP call; the transport
//!   and the handshake it authorizes are the shipping ones.
//!
//! The environment (broker, Agent, model stub) and the exact commands are in
//! `docs/testing/real-network.md`; both tests are driven by `REALNET_*` env
//! vars and are `#[ignore]`d because neither process exists in CI.

#![cfg(test)]

use std::path::{Path, PathBuf};
use std::time::Duration;

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use serde_json::{json, Value};
use tokio::sync::mpsc;

use super::test_support::{init_store, jwt, now_secs, HomeGuard};
use super::verify_e2e::{install_host, Broker};
use crate::remote_peer::{creds, runtime, transfer};

/// The one test in this module: an orchestrator plus its two child roles.
///
/// The child processes run this same test name with `REALNET_ROLE` set, which
/// the body dispatches on before it can orchestrate anything (so there is no
/// recursion and no second kind of binary to build).
#[tokio::test]
#[ignore = "real network: needs nats-server + a future-agent (docs/testing/real-network.md)"]
async fn real_network_two_process_host_and_client() {
    match std::env::var("REALNET_ROLE").as_deref() {
        Ok("host") => {
            host_role().await;
            return;
        }
        Ok("client") => {
            client_role().await;
            return;
        }
        _ => {}
    }
    orchestrator();
}

/// Where the two children rendezvous.
fn run_dir() -> PathBuf {
    PathBuf::from(std::env::var("REALNET_DIR").expect("REALNET_DIR"))
}

fn nats_url() -> String {
    std::env::var("REALNET_NATS_URL").expect("REALNET_NATS_URL")
}

fn agent_addr() -> String {
    std::env::var("REALNET_AGENT_ADDR").expect("REALNET_AGENT_ADDR")
}

fn write_report(path: &Path, value: &Value) {
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, serde_json::to_vec_pretty(value).expect("report JSON"))
        .expect("write report");
    std::fs::rename(&tmp, path).expect("publish report");
}

fn read_report(path: &Path) -> Value {
    let raw = std::fs::read_to_string(path)
        .unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
    serde_json::from_str(&raw).unwrap_or_else(|error| panic!("parse {}: {error}", path.display()))
}

/// Spawn one child process running this test under `role`.
fn spawn_role(role: &str, log: &Path) -> std::process::Child {
    let exe = std::env::current_exe().expect("the test binary");
    let out = std::fs::File::create(log).expect("child log");
    let err = out.try_clone().expect("child log clone");
    let mut command = std::process::Command::new(exe);
    command
        .args([
            "--exact",
            "remote::real_network::real_network_two_process_host_and_client",
            "--ignored",
            "--nocapture",
            "--test-threads=1",
        ])
        .env("REALNET_ROLE", role)
        .env("REALNET_DIR", run_dir())
        .env("REALNET_NATS_URL", nats_url())
        .env("REALNET_AGENT_ADDR", agent_addr())
        .env("FUTURE_AGENT_GRPC_ADDR", agent_addr())
        .env("RUST_BACKTRACE", "1")
        .stdout(out)
        .stderr(err)
        .stdin(std::process::Stdio::null());
    command.spawn().expect("spawn role child")
}

fn wait_for(path: &Path, budget: Duration) -> bool {
    let deadline = std::time::Instant::now() + budget;
    while std::time::Instant::now() < deadline {
        if path.is_file()
            && std::fs::metadata(path)
                .map(|m| m.len() > 0)
                .unwrap_or(false)
        {
            return true;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    false
}

fn wait_child(mut child: std::process::Child, budget: Duration, name: &str) -> i32 {
    let deadline = std::time::Instant::now() + budget;
    loop {
        if let Some(status) = child.try_wait().expect("child status") {
            return status.code().unwrap_or(-1);
        }
        if std::time::Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("{name} did not finish within {budget:?}");
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// Parent process: start the host, wait for its invitation, run the client,
/// then report both exits.
fn orchestrator() {
    let dir = run_dir();
    std::fs::create_dir_all(&dir).expect("run dir");
    for stale in ["host.json", "client.json", "client.done"] {
        let _ = std::fs::remove_file(dir.join(stale));
    }
    let logs = std::env::var("REALNET_LOG_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| dir.clone());
    std::fs::create_dir_all(&logs).expect("log dir");

    let mut host = spawn_role("host", &logs.join("host-child.log"));
    if !wait_for(&dir.join("host.json"), Duration::from_secs(45)) {
        let _ = host.kill();
        let _ = host.wait();
        panic!("the host bridge never published an invitation (see host-child.log)");
    }
    println!(
        "REALNET_HOST_UP {}",
        std::fs::read_to_string(dir.join("host.json")).expect("host.json")
    );

    let client = spawn_role("client", &logs.join("client-child.log"));
    let code = wait_child(client, Duration::from_secs(120), "client");
    let client_report = dir.join("client.json");
    if client_report.is_file() {
        println!("REALNET_CLIENT {}", read_report(&client_report));
    }
    if code != 0 {
        let _ = host.kill();
        let _ = host.wait();
        panic!("the client process failed with exit {code} (see client-child.log)");
    }
    // The host exits as soon as the client's done marker lands; waiting for it
    // proves the host stayed healthy through the whole exchange rather than
    // having died and been reported as a client failure.
    let _ = wait_child(host, Duration::from_secs(60), "host");
    assert!(
        client_report.is_file(),
        "the client exited 0 without writing its report"
    );
}

/// Host process: production bridge on a real broker, real Agent behind it.
async fn host_role() {
    let dir = run_dir();
    let broker = Broker::Real { url: nats_url() };
    let host = install_host(broker, "realnet-host", true).await;

    // A file the client will pull: the host registers the same record
    // `download_prepare` creates, on the real serving path.
    let source = dir.join("download-source.bin");
    let download_bytes = download_payload();
    std::fs::write(&source, &download_bytes).expect("write download source");
    let download = crate::remote_host::files::register_download_for_test(&source);
    // A real workspace for the prompt: the Agent refuses a cwd that does not
    // exist, and the phone's "new conversation in a workspace" path is the one
    // that names a directory the user already has.
    let workspace = dir.join("workspace");
    std::fs::create_dir_all(&workspace).expect("workspace dir");
    std::fs::write(workspace.join("README.md"), b"real-network workspace\n")
        .expect("workspace file");

    write_report(
        &dir.join("host.json"),
        &json!({
            "pairId": host.pair_id,
            "desktopId": host.creds.desktop_id,
            "invitation": super::verify_e2e::invitation(&host.creds),
            "download": download,
            "workspacePath": workspace.display().to_string(),
            "downloadSize": download_bytes.len(),
            "downloadSha256": sha256(&download_bytes),
            "agentAddr": agent_addr(),
            "natsUrl": nats_url(),
            "hostPid": std::process::id(),
        }),
    );
    println!("REALNET_HOST_READY pair={}", host.pair_id);

    // Stay up until the client says it is done (or the orchestrator gives up).
    let done = dir.join("client.done");
    let deadline = std::time::Instant::now() + Duration::from_secs(110);
    while std::time::Instant::now() < deadline && !done.is_file() {
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    // What the host actually received: the upload staging file carries the
    // client's marker if (and only if) the bytes crossed the wire.
    let mut upload_scan = json!({ "found": false });
    let upload_dir = std::env::temp_dir()
        .join("futureos-attachments")
        .join("mobile")
        .join("upload");
    if let Ok(entries) = std::fs::read_dir(&upload_dir) {
        for entry in entries.flatten() {
            if let Ok(bytes) = std::fs::read(entry.path()) {
                if let Ok(text) = String::from_utf8(bytes) {
                    if text.contains("REALNET-UPLOAD-") {
                        upload_scan = json!({
                            "found": true,
                            "path": entry.path().display().to_string(),
                            "marker": text.trim(),
                        });
                        break;
                    }
                }
            }
        }
    }
    write_report(
        &dir.join("host-report.json"),
        &json!({ "uploadScan": upload_scan }),
    );
    println!(
        "REALNET_HOST_REPORT {}",
        read_report(&dir.join("host-report.json"))
    );
}

/// The bytes the client must receive byte-for-byte.
fn download_payload() -> Vec<u8> {
    let mut bytes = Vec::new();
    for index in 0..40_000u32 {
        bytes.extend_from_slice(format!("REALNET-DOWNLOAD-{index:06}\n").as_bytes());
    }
    bytes
}

fn sha256(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    format!("{:x}", Sha256::digest(bytes))
}

/// 160 MiB of a fixed pattern: large enough that a kill during the pull cannot
/// be mistaken for a kill after it.
fn big_payload() -> Vec<u8> {
    let mut bytes = Vec::with_capacity(160 * 1024 * 1024);
    let mut index = 0u32;
    while bytes.len() < 160 * 1024 * 1024 {
        bytes.extend_from_slice(format!("REALNET-BIG-{index:010}\n").as_bytes());
        index += 1;
    }
    bytes
}

/// Client process: pair over the real transport, upload, download, prompt.
async fn client_role() {
    let dir = run_dir();
    let _home = HomeGuard::new("realnet-client");
    init_store();
    let host = read_report(&dir.join("host.json"));
    let invitation = host["invitation"].as_str().expect("invitation");
    let desktop_id = host["desktopId"].as_str().expect("desktopId");
    let pair_id = host["pairId"].as_str().expect("pairId");
    let desktop_key = url_field(invitation, "secureKey");
    let secret = url_field(invitation, "secret");

    let device = nkeys::KeyPair::new_user();
    let (private, public) = future_remote_crypto::generate_identity().expect("client identity");
    creds::upsert(creds::PeerCreds {
        pair_id: pair_id.to_string(),
        desktop_id: desktop_id.to_string(),
        device_id: crate::device_identity::device_id().expect("device id"),
        nkey_seed: device.seed().expect("device seed").to_string(),
        // The broker under test has no auth callout, so the JWT only has to be
        // a non-empty string the client signs the nonce against. It is still a
        // real NKey signature on the CONNECT.
        user_jwt: jwt(now_secs() + 3_600),
        refresh_token: "realnet-refresh".into(),
        nats_url: nats_url(),
        nats_ws_url: nats_url().replace("nats://", "ws://"),
        jwt_expires_at: now_secs() + 3_600,
        token_url: "http://127.0.0.1:1/client/v1/remote/auth/token".into(),
        secure: Some(creds::PeerIdentity {
            private_key: URL_SAFE_NO_PAD.encode(&private),
            public_key: URL_SAFE_NO_PAD.encode(&public),
            peer_public_key: Some(desktop_key.clone()),
            secret: Some(secret.clone()),
        }),
    })
    .expect("store peer credentials");

    let (events_tx, mut events_rx) = mpsc::unbounded_channel();
    let emitter: runtime::Emitter = std::sync::Arc::new(move |event| {
        let _ = events_tx.send(event);
    });
    let connected = runtime::connect_with_emitter(desktop_id, Some(emitter))
        .await
        .expect("pair and connect over the real transport");
    assert!(connected.connected, "the client reports a live connection");
    assert!(
        connected.agent_available,
        "the host's real Agent is reachable"
    );
    let bridge_instance = connected.bridge_instance_id.clone().expect("bridge id");
    println!(
        "REALNET_CLIENT_CONNECTED {}",
        json!({
            "desktopId": desktop_id,
            "bridgeInstanceId": bridge_instance,
            "features": connected.features,
            "agentAvailable": connected.agent_available,
        })
    );

    // ── Upload: client → host, through the real chunk lane ──
    // A multi-chunk file first: 1.2 MiB is three 512 KiB chunks, so the chunk
    // loop, the host's append path and the final hash all run more than once.
    let bulk_marker = format!("REALNET-UPLOAD-BULK-{}", now_secs());
    let mut bulk = bulk_marker.clone().into_bytes();
    bulk.resize(1_200_000, b'x');
    let bulk_upload = transfer::upload(desktop_id, "realnet-upload.bin", &bulk)
        .await
        .expect("multi-chunk upload across the real transfer lane");
    assert_eq!(
        bulk_upload.content_hash.to_ascii_lowercase(),
        sha256(&bulk),
        "the host's hash of the multi-chunk upload is the client's hash"
    );
    println!(
        "REALNET_CLIENT_UPLOAD_BULK {}",
        json!({
            "uploadId": bulk_upload.upload_id,
            "bytes": bulk.len(),
            "contentHash": bulk_upload.content_hash,
            "chunks": bulk.len().div_ceil(512 * 1024),
        })
    );

    let upload_marker = format!("REALNET-UPLOAD-{}", now_secs());
    let upload_bytes = upload_marker.clone().into_bytes();
    let uploaded = transfer::upload(desktop_id, "realnet-upload.txt", &upload_bytes)
        .await
        .expect("upload across the real transfer lane");
    assert_eq!(
        uploaded.content_hash.to_ascii_lowercase(),
        sha256(&upload_bytes),
        "the host's hash of what it received is the client's hash"
    );
    println!(
        "REALNET_CLIENT_UPLOAD {}",
        json!({
            "uploadId": uploaded.upload_id,
            "bytes": upload_bytes.len(),
            "contentHash": uploaded.content_hash,
            "marker": upload_marker,
        })
    );

    // ── Download: host → client, prepared by the host and pulled in chunks ──
    let info: transfer::DownloadInfo =
        serde_json::from_value(host["download"].clone()).expect("host download record");
    let fetched = transfer::finish(desktop_id, info)
        .await
        .expect("download across the real transfer lane");
    let expected = download_payload();
    assert_eq!(fetched.bytes, expected, "the download is byte-identical");
    println!(
        "REALNET_CLIENT_DOWNLOAD {}",
        json!({
            "name": fetched.name,
            "bytes": fetched.bytes.len(),
            "sha256": sha256(&fetched.bytes),
        })
    );

    // ── Prompt: client → host bridge → real Agent → model stub → events ──
    //
    // A phone's new conversation names an existing workspace: the Agent refuses
    // a cwd that is not there, and a chat thread's directory is created by the
    // GUI, not by the bridge.
    let workspace = runtime::request(
        desktop_id,
        json!({
            "type": "create_workspace",
            "path": host["workspacePath"].as_str().expect("workspacePath"),
            "name": "Real-net workspace",
        }),
        "list",
    )
    .await
    .expect("the host registers the workspace");
    let workspace_id = workspace["workspace"]["id"]
        .as_str()
        .expect("created workspace id")
        .to_string();
    println!(
        "REALNET_CLIENT_WORKSPACE {}",
        json!({ "workspaceId": workspace_id })
    );

    let prompt_marker = format!("REALNET-PROMPT-{}", now_secs());
    let ack = runtime::request(
        desktop_id,
        json!({
            "type": "prompt",
            "sessionId": "",
            "message": format!("say {prompt_marker}"),
            "mode": "workspace",
            "workspaceId": workspace_id,
            // The upload reference makes the host claim the staged bytes and
            // hand them to the Agent as this message's attachment.
            "attachments": [{ "uploadId": uploaded.upload_id }],
        }),
        "",
    )
    .await
    .expect("the host acknowledges the prompt");
    let session_id = ack["sessionId"]
        .as_str()
        .expect("ack sessionId")
        .to_string();
    let run_id = ack["runId"].as_str().expect("ack runId").to_string();
    println!(
        "REALNET_CLIENT_PROMPT_ACK {}",
        json!({ "sessionId": session_id, "runId": run_id, "bridgeInstanceId": ack["bridgeInstanceId"] })
    );

    // The answer comes back as sealed pushes on `p.<pair>.evt.<session>`,
    // decrypted by the client's own subscription.
    let mut seen = Vec::new();
    let mut chunks = 0usize;
    let mut answer = String::new();
    let terminal = tokio::time::timeout(Duration::from_secs(60), async {
        while let Some(event) = events_rx.recv().await {
            if event.kind != "event" {
                continue;
            }
            let kind = event.payload["type"]
                .as_str()
                .unwrap_or_default()
                .to_string();
            let data = event.payload["data"].clone();
            let text = data
                .as_str()
                .and_then(|raw| serde_json::from_str::<Value>(raw).ok())
                .and_then(|parsed| parsed["text"].as_str().map(str::to_owned))
                .or_else(|| data["text"].as_str().map(str::to_owned))
                .unwrap_or_default();
            if kind == "text_chunk" {
                chunks += 1;
                answer.push_str(&text);
            }
            seen.push(kind.clone());
            if kind == "agent_end" || kind == "error" {
                return kind;
            }
        }
        "stream ended".to_string()
    })
    .await
    .expect("the answer's terminal event within 60 s");
    assert_eq!(terminal, "agent_end", "the run must end cleanly");
    assert!(chunks > 0, "the phone received no text chunks");
    let echoed = answer.contains(&prompt_marker);
    println!(
        "REALNET_CLIENT_PROMPT {}",
        json!({
            "runId": run_id,
            "terminal": terminal,
            "chunks": chunks,
            "answerEchoed": echoed,
            "eventTypes": seen,
        })
    );

    write_report(
        &dir.join("client.json"),
        &json!({
            "connected": true,
            "bridgeInstanceId": bridge_instance,
            "agentAvailable": connected.agent_available,
            "upload": { "marker": upload_marker, "bytes": upload_bytes.len() },
            "bulkUpload": { "bytes": bulk.len(), "chunks": bulk.len().div_ceil(512 * 1024) },
            "download": { "bytes": fetched.bytes.len(), "sha256": sha256(&fetched.bytes) },
            "prompt": { "sessionId": session_id, "runId": run_id, "chunks": chunks, "answerEchoed": echoed },
        }),
    );
    std::fs::write(dir.join("client.done"), b"done").expect("done marker");
    runtime::disconnect(desktop_id).await;
}

/// Failure paths that only exist across a real network.
///
/// Three scenarios, driven by files the orchestrator and the children exchange
/// (each one is a command in `docs/testing/real-network.md`, not prose):
///
/// 1. the client's socket is closed under its open event stream, and the next
///    command has to reconnect before it can be answered;
/// 2. the broker is killed and restarted on the same port while the run is
///    live, and the client has to come back;
/// 3. the host process is replaced by a new process holding the *same* pairing
///    identity, and the client has to resume with `Noise_IK` (the invitation
///    PSK was spent by the first pairing).
#[tokio::test]
#[ignore = "real network: needs nats-server + a future-agent (docs/testing/real-network.md §5)"]
async fn real_network_failure_paths() {
    match std::env::var("REALNET_ROLE").as_deref() {
        Ok("host-restart") => {
            host_restart_role().await;
            return;
        }
        Ok("client-failure") => {
            client_failure_role().await;
            return;
        }
        _ => {}
    }
    failure_orchestrator();
}

fn spawn_failure_role(role: &str, extra: &[(&str, String)], log: &Path) -> std::process::Child {
    let exe = std::env::current_exe().expect("the test binary");
    let out = std::fs::File::create(log).expect("child log");
    let err = out.try_clone().expect("child log clone");
    let mut command = std::process::Command::new(exe);
    command
        .args([
            "--exact",
            "remote::real_network::real_network_failure_paths",
            "--ignored",
            "--nocapture",
            "--test-threads=1",
        ])
        .env("REALNET_ROLE", role)
        .env("REALNET_DIR", run_dir())
        .env("REALNET_NATS_URL", nats_url())
        .env("REALNET_AGENT_ADDR", agent_addr())
        .env("FUTURE_AGENT_GRPC_ADDR", agent_addr())
        .env("RUST_BACKTRACE", "1")
        .stdout(out)
        .stderr(err)
        .stdin(std::process::Stdio::null());
    for (key, value) in extra {
        command.env(key, value);
    }
    command.spawn().expect("spawn role child")
}

fn failure_orchestrator() {
    let dir = run_dir();
    std::fs::create_dir_all(&dir).expect("run dir");
    for stale in [
        "host.json",
        "host2.json",
        "host1-stopped",
        "client.json",
        "client.done",
        "phase-socket.json",
        "phase-broker.json",
        "host-creds.json",
        "ready-broker-kill",
        "ready-host-restart",
        "broker-back",
        "host2-back",
        "stop-host1",
    ] {
        let _ = std::fs::remove_file(dir.join(stale));
    }
    let logs = std::env::var("REALNET_LOG_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| dir.clone());
    std::fs::create_dir_all(&logs).expect("log dir");
    let creds_path = dir.join("host-creds.json");
    let creds = creds_path.display().to_string();

    let argv: Vec<String> =
        serde_json::from_str(&std::env::var("REALNET_NATS_ARGV").expect("REALNET_NATS_ARGV"))
            .expect("broker argv JSON");
    let broker_pid: i32 = std::env::var("REALNET_NATS_PID")
        .expect("REALNET_NATS_PID")
        .parse()
        .expect("broker pid");

    let mut host1 = spawn_failure_role(
        "host-restart",
        &[
            ("REALNET_HOST_PHASE", "1".into()),
            ("REALNET_HOST_CREDS", creds.clone()),
        ],
        &logs.join("failure-host1.log"),
    );
    if !wait_for(&dir.join("host.json"), Duration::from_secs(45)) {
        let _ = host1.kill();
        panic!("host 1 never published an invitation");
    }

    let client = spawn_failure_role(
        "client-failure",
        &[
            ("REALNET_NATS_PID", broker_pid.to_string()),
            (
                "REALNET_NATS_ARGV",
                serde_json::to_string(&argv).expect("argv JSON"),
            ),
        ],
        &logs.join("failure-client.log"),
    );

    // ── 2. broker killed during a transfer and restarted ──
    //
    // The *client* owns this kill: file-marker polling in the orchestrator adds
    // enough latency (a 50 ms poll plus process start) that a loopback transfer
    // finished first, which is what the first attempt measured. The client
    // signals the broker immediately after spawning the transfer task, then
    // brings it back on the same port.
    if !wait_for(&dir.join("phase-broker.json"), Duration::from_secs(90)) {
        let _ = host1.kill();
        panic!("the client never reported the broker-restart phase");
    }
    let phase_broker = read_report(&dir.join("phase-broker.json"));
    println!("REALNET_PHASE_BROKER {phase_broker}");
    let restarted_broker_pid = phase_broker["brokerPid"].as_i64().map(|pid| pid as i32);

    // ── 3. peer restarted with the same identity ──
    if !wait_for(&dir.join("ready-host-restart"), Duration::from_secs(60)) {
        panic!("the client never reached the host-restart phase");
    }
    std::fs::write(dir.join("stop-host1"), b"stop").expect("stop host 1");
    let host1_code = wait_child(host1, Duration::from_secs(30), "host 1");
    assert_eq!(host1_code, 0, "host 1 did not stop cleanly");
    let mut host2 = spawn_failure_role(
        "host-restart",
        &[
            ("REALNET_HOST_PHASE", "2".into()),
            ("REALNET_HOST_CREDS", creds.clone()),
        ],
        &logs.join("failure-host2.log"),
    );
    if !wait_for(&dir.join("host2.json"), Duration::from_secs(45)) {
        let _ = host2.kill();
        panic!("host 2 never came up on the persisted identity");
    }
    std::fs::write(dir.join("host2-back"), b"back").expect("host2-back marker");
    println!(
        "REALNET_HOST2_UP {}",
        std::fs::read_to_string(dir.join("host2.json")).unwrap_or_default()
    );

    let code = wait_child(client, Duration::from_secs(120), "failure client");
    if code != 0 {
        let _ = host2.kill();
        if let Some(pid) = restarted_broker_pid {
            let _ = std::process::Command::new("kill")
                .arg("-TERM")
                .arg(pid.to_string())
                .status();
        }
        panic!("the failure-path client exited {code}");
    }
    std::fs::write(dir.join("client.done"), b"done").expect("done marker");
    let _ = wait_child(host2, Duration::from_secs(30), "host 2");
    if let Some(pid) = restarted_broker_pid {
        let _ = std::process::Command::new("kill")
            .arg("-TERM")
            .arg(pid.to_string())
            .status();
    }
    println!("REALNET_FAILURES {}", read_report(&dir.join("client.json")));
}

/// Host process for the restart scenario: phase 1 mints and persists the
/// identity, phase 2 loads it.
async fn host_restart_role() {
    let dir = run_dir();
    let creds_path = PathBuf::from(std::env::var("REALNET_HOST_CREDS").expect("creds path"));
    let phase: u32 = std::env::var("REALNET_HOST_PHASE")
        .expect("host phase")
        .parse()
        .expect("host phase number");
    let broker = Broker::Real { url: nats_url() };
    let host = if creds_path.is_file() {
        let raw = std::fs::read_to_string(&creds_path).expect("read host creds");
        let creds = serde_json::from_str(&raw).expect("parse host creds");
        super::verify_e2e::install_host_with_creds(creds, broker, "realnet-failure-host2", true)
            .await
    } else {
        let host = install_host(broker, "realnet-failure-host1", true).await;
        write_report(
            &creds_path,
            &serde_json::to_value(&host.creds).expect("creds JSON"),
        );
        host
    };
    if phase == 1 {
        // A 160 MiB file is big enough that a broker killed tens of ms into the
        // pull is killed *during* the transfer, not after it.
        let big_path = dir.join("download-big.bin");
        let big = big_payload();
        std::fs::write(&big_path, &big).expect("write big download source");
        let download_big = crate::remote_host::files::register_download_for_test(&big_path);
        let download_big_retry = crate::remote_host::files::register_download_for_test(&big_path);
        write_report(
            &dir.join("host.json"),
            &json!({
                "pairId": host.pair_id,
                "desktopId": host.creds.desktop_id,
                "invitation": super::verify_e2e::invitation(&host.creds),
                "downloadBig": download_big,
                "downloadBigRetry": download_big_retry,
                "downloadBigSize": big.len(),
                "downloadBigSha256": sha256(&big),
                "hostPid": std::process::id(),
            }),
        );
        println!("REALNET_HOST1_READY pair={}", host.pair_id);
        let stop = dir.join("stop-host1");
        let deadline = std::time::Instant::now() + Duration::from_secs(90);
        while std::time::Instant::now() < deadline && !stop.is_file() {
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        // The pairing wrote the *paired* credential (the client's Noise static
        // key is now pinned). Copying the pre-pairing snapshot would leave
        // host 2 with `peer_public_key: None` and make every `Noise_IK`
        // reconnect answer `peer_unknown` — the exact failure this scenario
        // exists to catch.
        if let Some(saved) = crate::remote_host::pairing::load_creds() {
            write_report(
                &creds_path,
                &serde_json::to_value(&saved).expect("paired creds JSON"),
            );
        }
        std::fs::write(dir.join("host1-stopped"), b"stopped").expect("host1-stopped");
    } else {
        write_report(
            &dir.join("host2.json"),
            &json!({
                "pairId": host.pair_id,
                "desktopId": host.creds.desktop_id,
                "hostPid": std::process::id(),
            }),
        );
        println!("REALNET_HOST2_READY pair={}", host.pair_id);
        let done = dir.join("client.done");
        let deadline = std::time::Instant::now() + Duration::from_secs(90);
        while std::time::Instant::now() < deadline && !done.is_file() {
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }
}

/// Client process for the failure scenarios.
async fn client_failure_role() {
    let dir = run_dir();
    let _home = HomeGuard::new("realnet-failure-client");
    init_store();
    let host = read_report(&dir.join("host.json"));
    let invitation = host["invitation"].as_str().expect("invitation");
    let desktop_id = host["desktopId"].as_str().expect("desktopId");
    let pair_id = host["pairId"].as_str().expect("pairId");
    let desktop_key = url_field(invitation, "secureKey");
    let secret = url_field(invitation, "secret");

    let device = nkeys::KeyPair::new_user();
    let (private, public) = future_remote_crypto::generate_identity().expect("client identity");
    creds::upsert(creds::PeerCreds {
        pair_id: pair_id.to_string(),
        desktop_id: desktop_id.to_string(),
        device_id: crate::device_identity::device_id().expect("device id"),
        nkey_seed: device.seed().expect("device seed").to_string(),
        user_jwt: jwt(now_secs() + 3_600),
        refresh_token: "realnet-refresh".into(),
        nats_url: nats_url(),
        nats_ws_url: nats_url().replace("nats://", "ws://"),
        jwt_expires_at: now_secs() + 3_600,
        token_url: "http://127.0.0.1:1/client/v1/remote/auth/token".into(),
        secure: Some(creds::PeerIdentity {
            private_key: URL_SAFE_NO_PAD.encode(&private),
            public_key: URL_SAFE_NO_PAD.encode(&public),
            peer_public_key: Some(desktop_key),
            secret: Some(secret),
        }),
    })
    .expect("store peer credentials");
    let (events_tx, _events_rx) = mpsc::unbounded_channel();
    let emitter: runtime::Emitter = std::sync::Arc::new(move |event| {
        let _ = events_tx.send(event);
    });
    runtime::connect_with_emitter(desktop_id, Some(emitter))
        .await
        .expect("initial pair and connect");
    println!("REALNET_FAILURE_CONNECTED");

    // ── 1. socket closed under the open stream ──
    runtime::close_socket_for_test(desktop_id).await;
    let socket_recovered = runtime::request(desktop_id, json!({ "type": "list_sessions" }), "list")
        .await
        .is_ok();
    write_report(
        &dir.join("phase-socket.json"),
        &json!({ "socketClosedThenCommandSucceeded": socket_recovered }),
    );
    println!(
        "REALNET_PHASE_SOCKET {}",
        read_report(&dir.join("phase-socket.json"))
    );

    // ── 2. broker killed and restarted under us ──
    // Kill it *during* a transfer: the 160 MiB pull has 320 chunks, so a kill
    // tens of milliseconds in cannot land after the file has arrived.
    let big: transfer::DownloadInfo =
        serde_json::from_value(host["downloadBig"].clone()).expect("big download record");
    let retry_info: transfer::DownloadInfo =
        serde_json::from_value(host["downloadBigRetry"].clone()).expect("big retry record");
    let big_expected = host["downloadBigSha256"]
        .as_str()
        .expect("big sha")
        .to_string();
    let big_size = host["downloadBigSize"].as_u64().expect("big size");
    let big_id = desktop_id.to_string();
    let mut big_task = tokio::spawn(async move { transfer::finish(&big_id, big).await });
    // Let the task actually start and pull a few waves before the kill.
    tokio::time::sleep(Duration::from_millis(5)).await;
    // Kill the broker ourselves, with no rendezvous round trip: the parent's
    // 50 ms file poll was slower than the loopback pull, so the transfer kept
    // finishing before the kill landed. Only the broker this harness started
    // is signalled, by the pid the orchestrator passed in.
    let broker_pid: i32 = std::env::var("REALNET_NATS_PID")
        .expect("REALNET_NATS_PID")
        .parse()
        .expect("broker pid");
    let argv: Vec<String> =
        serde_json::from_str(&std::env::var("REALNET_NATS_ARGV").expect("REALNET_NATS_ARGV"))
            .expect("broker argv JSON");
    let killed = std::process::Command::new("kill")
        .arg("-TERM")
        .arg(broker_pid.to_string())
        .status()
        .expect("kill the broker");
    assert!(killed.success(), "could not signal the broker");
    tokio::time::sleep(Duration::from_millis(12_000)).await;
    let mut broker = std::process::Command::new(&argv[0])
        .args(&argv[1..])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .stdin(std::process::Stdio::null())
        .spawn()
        .expect("restart the broker");
    // The orchestrator owns this broker's lifetime and kills it by pid; this
    // waiter only reaps it so the restarted server cannot become a zombie.
    let broker_pid_now = broker.id() as i32;
    std::thread::spawn(move || {
        let _ = broker.wait();
    });
    let port = reqwest::Url::parse(&nats_url())
        .expect("broker URL")
        .port()
        .expect("broker port");
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    while std::time::Instant::now() < deadline
        && std::net::TcpStream::connect(("127.0.0.1", port)).is_err()
    {
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    println!("REALNET_BROKER_RESTARTED pid={broker_pid_now}");
    let mid_transfer = match tokio::time::timeout(Duration::from_secs(30), &mut big_task).await {
        Ok(Ok(Ok(file))) => json!({
            "completedBeforeKill": true,
            "bytes": file.bytes.len(),
        }),
        Ok(Ok(Err(error))) => json!({
            "completedBeforeKill": false,
            "error": error.to_string(),
        }),
        Ok(Err(error)) => json!({
            "completedBeforeKill": false,
            "joinError": error.to_string(),
        }),
        Err(_) => {
            big_task.abort();
            json!({ "completedBeforeKill": false, "timedOut": true })
        }
    };
    println!("REALNET_MID_TRANSFER {mid_transfer}");
    // The 12 s outage outlived the client's 10 s request budget on purpose, so
    // both ends have to reconnect before a retry can mean anything. A cheap
    // command is the liveness proof: the host bridge has to be serving again,
    // not merely the broker.
    let mut host_back = false;
    for _ in 0..240 {
        if runtime::request(desktop_id, json!({ "type": "list_sessions" }), "list")
            .await
            .is_ok()
        {
            host_back = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    println!("REALNET_HOST_BACK {}", json!({ "hostBack": host_back }));
    // The interrupted attempt may already have released its host record, so the
    // retry uses the second registration.
    let mut retried = None;
    for _ in 0..20 {
        match transfer::finish(desktop_id, retry_info.clone()).await {
            Ok(file) if file.bytes.len() as u64 == big_size => {
                assert_eq!(
                    sha256(&file.bytes),
                    big_expected,
                    "retry bytes are identical"
                );
                retried = Some(file.bytes.len());
                break;
            }
            Ok(file) => panic!(
                "retry returned {} bytes, expected {big_size}",
                file.bytes.len()
            ),
            Err(_) => tokio::time::sleep(Duration::from_millis(250)).await,
        }
    }
    let retried = retried.expect("the transfer retried after the broker came back");
    let during_outage =
        runtime::request(desktop_id, json!({ "type": "list_sessions" }), "list").await;
    let first = match &during_outage {
        Ok(_) => json!({ "ok": true }),
        Err(error) => json!({ "ok": false, "error": error.to_string() }),
    };
    // Let the supervisor/async-nats settle, then require a working command.
    let mut recovered = during_outage.is_ok();
    for _ in 0..40 {
        if recovered {
            break;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
        recovered = runtime::request(desktop_id, json!({ "type": "list_sessions" }), "list")
            .await
            .is_ok();
    }
    write_report(
        &dir.join("phase-broker.json"),
        &json!({
            "midTransfer": mid_transfer,
            "retriedBytes": retried,
            "brokerPid": broker_pid_now,
            "hostBack": host_back,
            "requestDuringOutage": first,
            "recovered": recovered
        }),
    );
    println!(
        "REALNET_PHASE_BROKER {}",
        read_report(&dir.join("phase-broker.json"))
    );
    assert!(
        recovered,
        "the client never came back after the broker restart"
    );
    assert!(
        host_back,
        "the host bridge never came back after the broker restart"
    );

    // ── 3. the peer is replaced, same identity ──
    // Close the client's socket as well: a link that merely lost its host can
    // wait for the same subscription to be served again, but a fresh connection
    // has to prove the pinned key with `Noise_IK` — the invitation PSK is gone.
    runtime::close_socket_for_test(desktop_id).await;
    std::fs::write(dir.join("ready-host-restart"), b"ready").expect("ready marker");
    let back = wait_for_async(&dir.join("host2-back"), Duration::from_secs(60)).await;
    assert!(back, "the replacement host never came back");
    let mut resumed = false;
    let mut last_error = String::new();
    for _ in 0..80 {
        match runtime::request(desktop_id, json!({ "type": "list_sessions" }), "list").await {
            Ok(_) => {
                resumed = true;
                break;
            }
            Err(error) => {
                last_error = error.to_string();
                // A live socket died with host 1; the retry loop has to notice
                // and run `Noise_IK` against the pinned key.
                let _ = runtime::ensure_connected(desktop_id).await;
                tokio::time::sleep(Duration::from_millis(250)).await;
            }
        }
    }
    assert!(
        resumed,
        "the client did not resume against the restarted host: {last_error}"
    );
    println!("REALNET_PHASE_HOST_RESTART {}", json!({ "resumed": true }));
    write_report(
        &dir.join("client.json"),
        &json!({
            "socketClosedThenCommandSucceeded": socket_recovered,
            "broker": read_report(&dir.join("phase-broker.json")),
            "hostRestartResumed": resumed,
        }),
    );
    std::fs::write(dir.join("client.done"), b"done").expect("done marker");
    runtime::disconnect(desktop_id).await;
}

async fn wait_for_async(path: &Path, budget: Duration) -> bool {
    let deadline = std::time::Instant::now() + budget;
    while std::time::Instant::now() < deadline {
        if path.is_file() {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    false
}

fn url_field(url: &str, name: &str) -> String {
    reqwest::Url::parse(url)
        .expect("invitation URL")
        .query_pairs()
        .find(|(key, _)| key == name)
        .map(|(_, value)| value.into_owned())
        .unwrap_or_else(|| panic!("invitation is missing {name}"))
}
