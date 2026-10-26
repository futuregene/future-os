//! Real-network acceptance for the **desktop download tier**.
//!
//! [`super::real_network`] proves the remote lane works across a process
//! boundary, but it sidesteps `download_prepare`: the file is registered with
//! `register_download_for_test`, so the host's size/type policy — which is the
//! whole subject of the desktop tier — never runs. And
//! [`crate::remote_peer::transfer`]'s tier tests all share one process and one
//! in-process broker, so "the desktop saved a 12.5 MB file from another machine"
//! had never been observed.
//!
//! This module makes exactly that observation, with both ends in separate OS
//! processes and a real `nats-server` between them:
//!
//! * **real:** the production host bridge (`remote::install_host` → the shipping
//!   supervisor/command loop/transfer loop), the real `download_prepare`
//!   command and therefore the real tier check, the real chunk pull, the real
//!   client-side hash check, real files on disk and the desktop's own
//!   `write_to_path` save, over real TCP and a real broker;
//! * **stubbed:** the pairing platform HTTP hop is bypassed the same way
//!   `real_network` bypasses it (the invitation is minted locally), and the host
//!   side has no agent conversation because a download needs none;
//! * **separate process:** the host is one child process, the client another, so
//!   the numbers and refusals cross a socket rather than a function call.
//!
//! The client calls `crate::remote_peer::transfer::download_for_desktop` — the
//! function the Desktop's `remote_peer_download_file` command calls — and saves
//! with `write_to_path`, also the command's own function. What is *not* covered
//! is the Tauri shell around it (window, plugin, the panel that picks the
//! destination); the code the button reaches is the code this drives.
//!
//! Run it with the commands in `docs/testing/remote-download-tiers-live-2026-10-26.md`
//! §1; it is `#[ignore]`d because no broker exists in CI. The orchestrator can
//! also drive the **old-host** arm (`DLTIERS_EXTERNAL_HOST=1`), where the host
//! half is an older build started separately — that run is what proves the
//! fallback to `original` is real rather than unit-tested.

#![cfg(test)]

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use serde_json::{json, Value};

use super::test_support::{init_store, jwt, now_secs, HomeGuard};
use super::verify_e2e::{install_host, Broker};
use crate::remote_peer::{creds, runtime, transfer};

/// 12.5 MiB — the installer that started all of this, byte for byte the size the
/// reported defect used (`12 * 1024 * 1024 + 512 * 1024`).
const APK_BYTES: u64 = 12 * 1024 * 1024 + 512 * 1024;
/// Over the desktop's 512 MiB ceiling, under nothing else.
const HUGE_BYTES: u64 = 600 * 1024 * 1024;
/// A file the phone's rules *do* allow, so an old host reaches its own variant
/// refusal instead of stopping at the size check.
const SMALL_BYTES: u64 = 1024 * 1024;
/// A type the phone's reader refuses at any size (`application/octet-stream`),
/// so the desktop tier's *second* lifted rule is observable on its own.
const BIN_BYTES: u64 = 1024 * 1024;
/// One byte over the desktop ceiling: the boundary, not just "a big file".
const JUST_OVER_BYTES: u64 = 512 * 1024 * 1024 + 1;
/// A mid-sized file: the desktop tier exists for files far larger than the
/// phone's ceiling, and 12.5 MiB alone would not show that it does.
const LARGE_BYTES: u64 = 64 * 1024 * 1024;

const TEST_NAME: &str = "remote::download_tiers_network::desktop_download_tiers_over_the_network";

#[tokio::test]
#[ignore = "real network: needs nats-server (docs/testing/remote-download-tiers-live-2026-10-26.md)"]
async fn desktop_download_tiers_over_the_network() {
    match std::env::var("DLTIERS_ROLE").as_deref() {
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

fn run_dir() -> PathBuf {
    PathBuf::from(std::env::var("DLTIERS_DIR").expect("DLTIERS_DIR"))
}

fn nats_url() -> String {
    std::env::var("DLTIERS_NATS_URL").expect("DLTIERS_NATS_URL")
}

fn source_path(name: &str) -> PathBuf {
    run_dir().join("source").join(name)
}

fn write_report(path: &Path, value: &Value) {
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    std::fs::write(path, serde_json::to_vec_pretty(value).expect("report JSON"))
        .expect("write report");
}

fn read_report(path: &Path) -> Value {
    let raw = std::fs::read_to_string(path)
        .unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
    serde_json::from_str(&raw).unwrap_or_else(|error| panic!("parse {}: {error}", path.display()))
}

fn sha256(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    format!("{:x}", Sha256::digest(bytes))
}

fn sha256_file(path: &Path) -> String {
    sha256(&std::fs::read(path).expect("read the source file"))
}

/// Deterministic bytes, not a zero fill: a transfer that dropped or duplicated a
/// chunk has to show up in the hash, and an all-zero file would hide a whole
/// class of offset mistakes.
fn pseudo_random(len: u64) -> Vec<u8> {
    let mut state: u64 = 0x9E37_79B9_7F4A_7C15;
    let mut bytes = Vec::with_capacity(len as usize);
    for _ in 0..len {
        state = state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        bytes.push((state >> 33) as u8);
    }
    bytes
}

// ── Orchestrator ────────────────────────────────────────────────────────────

/// Parent process: prepare the sources, start the host, run the client, report.
fn orchestrator() {
    let dir = run_dir();
    std::fs::create_dir_all(&dir).expect("run dir");
    std::fs::create_dir_all(dir.join("source")).expect("source dir");
    let external = std::env::var("DLTIERS_EXTERNAL_HOST").as_deref() == Ok("1");
    // An external host writes its own `host.json`; deleting it here would race
    // the process this arm exists to talk to.
    let stale: &[&str] = if external {
        &["client.json", "client.done"]
    } else {
        &["host.json", "client.json", "client.done"]
    };
    for name in stale {
        let _ = std::fs::remove_file(dir.join(name));
    }

    let apk = source_path("app-debug.apk");
    let apk_bytes = pseudo_random(APK_BYTES);
    std::fs::write(&apk, &apk_bytes).expect("write the installer");

    let small = source_path("release-notes.txt");
    let mut notes = Vec::with_capacity(SMALL_BYTES as usize);
    while (notes.len() as u64) < SMALL_BYTES {
        notes.extend_from_slice(format!("release note line {}\n", notes.len()).as_bytes());
    }
    notes.truncate(SMALL_BYTES as usize);
    std::fs::write(&small, &notes).expect("write the text file");

    let binary = source_path("toolchain.bin");
    let bin_bytes = pseudo_random(BIN_BYTES);
    std::fs::write(&binary, &bin_bytes).expect("write the binary file");

    let edge = source_path("edge.bin");
    {
        use std::io::{Seek, Write};
        let mut file = std::fs::File::create(&edge).expect("create the boundary file");
        file.write_all(b"DLTIERS-EDGE").expect("marker");
        file.seek(std::io::SeekFrom::Start(JUST_OVER_BYTES - 4))
            .expect("seek to the boundary end");
        file.write_all(b"EDGE").expect("end marker");
    }

    let large = source_path("database-dump.bin");
    let large_bytes = pseudo_random(LARGE_BYTES);
    std::fs::write(&large, &large_bytes).expect("write the mid-sized file");

    // Sparse: 600 MiB of length with almost no blocks behind it. The host must
    // refuse it on its declared size, so its contents are never read.
    let huge = source_path("disk-image.img");
    {
        use std::io::{Seek, Write};
        let mut file = std::fs::File::create(&huge).expect("create the large file");
        file.write_all(b"DLTIERS-HUGE-MARKER").expect("marker");
        file.seek(std::io::SeekFrom::Start(HUGE_BYTES - 8))
            .expect("seek to the end");
        file.write_all(b"DLTIERS!").expect("end marker");
    }

    println!(
        "DLTIERS_SOURCE {}",
        json!({
            "apk": { "path": apk.display().to_string(), "bytes": APK_BYTES, "sha256": sha256(&apk_bytes) },
            "small": { "path": small.display().to_string(), "bytes": notes.len(), "sha256": sha256(&notes) },
            "huge": { "path": huge.display().to_string(), "bytes": std::fs::metadata(&huge).expect("huge").len() },
            "binary": { "path": binary.display().to_string(), "bytes": BIN_BYTES, "sha256": sha256(&bin_bytes) },
            "edge": { "path": edge.display().to_string(), "bytes": std::fs::metadata(&edge).expect("edge").len() },
            "large": { "path": large.display().to_string(), "bytes": LARGE_BYTES, "sha256": sha256(&large_bytes) },
        })
    );

    let mut host = if external {
        println!("DLTIERS_HOST_EXTERNAL waiting for host.json");
        None
    } else {
        Some(spawn_role("host", &dir.join("host-child.log")))
    };
    if !wait_for(&dir.join("host.json"), Duration::from_secs(90)) {
        if let Some(mut child) = host.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
        panic!("the host bridge never published an invitation (see host-child.log)");
    }
    println!(
        "DLTIERS_HOST_UP {}",
        std::fs::read_to_string(dir.join("host.json")).expect("host.json")
    );

    let client = spawn_role("client", &dir.join("client-child.log"));
    let code = wait_child(client, Duration::from_secs(240), "client");
    if dir.join("client.json").is_file() {
        println!("DLTIERS_CLIENT {}", read_report(&dir.join("client.json")));
    }
    // Release the host (the old build waits on this marker; so does ours).
    std::fs::write(dir.join("client.done"), b"done").expect("done marker");
    if let Some(child) = host.take() {
        let _ = wait_child(child, Duration::from_secs(60), "host");
    }
    assert!(
        dir.join("client.json").is_file(),
        "the client exited 0 without writing its report"
    );
    assert_eq!(code, 0, "the client process failed (see client-child.log)");
}

/// Spawn one child process running this test under `role`.
fn spawn_role(role: &str, log: &Path) -> std::process::Child {
    let exe = std::env::current_exe().expect("the test binary");
    let out = std::fs::File::create(log).expect("child log");
    let err = out.try_clone().expect("child log clone");
    std::process::Command::new(exe)
        .args([
            "--exact",
            TEST_NAME,
            "--ignored",
            "--nocapture",
            "--test-threads=1",
        ])
        .env("DLTIERS_ROLE", role)
        .env("DLTIERS_DIR", run_dir())
        .env("DLTIERS_NATS_URL", nats_url())
        .env("RUST_BACKTRACE", "1")
        .stdout(out)
        .stderr(err)
        .stdin(std::process::Stdio::null())
        .spawn()
        .expect("spawn role child")
}

fn wait_for(path: &Path, budget: Duration) -> bool {
    let deadline = Instant::now() + budget;
    while Instant::now() < deadline {
        if std::fs::metadata(path)
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
    let deadline = Instant::now() + budget;
    loop {
        if let Some(status) = child.try_wait().expect("child status") {
            return status.code().unwrap_or(-1);
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("{name} did not finish within {budget:?}");
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

// ── Host process ────────────────────────────────────────────────────────────

/// Host process: the production bridge on a real broker.
///
/// It prepares nothing: the client's own `download_prepare` reaches the real
/// host policy, and the source files sit on this machine's disk exactly as they
/// would on a served host.
async fn host_role() {
    let dir = run_dir();
    let host = install_host(Broker::Real { url: nats_url() }, "dltiers-host", false).await;
    write_report(
        &dir.join("host.json"),
        &json!({
            "pairId": host.pair_id,
            "desktopId": host.creds.desktop_id,
            "invitation": super::verify_e2e::invitation(&host.creds),
            "natsUrl": nats_url(),
            "hostPid": std::process::id(),
        }),
    );
    println!("DLTIERS_HOST_READY pair={}", host.pair_id);

    let done = dir.join("client.done");
    let deadline = Instant::now() + Duration::from_secs(300);
    while Instant::now() < deadline && !done.is_file() {
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

// ── Client process ──────────────────────────────────────────────────────────

async fn client_role() {
    let dir = run_dir();
    let _home = HomeGuard::new("dltiers-client");
    init_store();
    let host = read_report(&dir.join("host.json"));
    let invitation = host["invitation"].as_str().expect("invitation");
    let desktop_id = host["desktopId"].as_str().expect("desktopId").to_string();
    let pair_id = host["pairId"].as_str().expect("pairId");
    let desktop_key = url_field(invitation, "secureKey");
    let secret = url_field(invitation, "secret");

    let device = nkeys::KeyPair::new_user();
    let (private, public) = future_remote_crypto::generate_identity().expect("client identity");
    creds::upsert(creds::PeerCreds {
        pair_id: pair_id.to_string(),
        desktop_id: desktop_id.clone(),
        device_id: crate::device_identity::device_id().expect("device id"),
        nkey_seed: device.seed().expect("device seed").to_string(),
        user_jwt: jwt(now_secs() + 3_600),
        refresh_token: "dltiers-refresh".into(),
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

    let noop: runtime::Emitter = std::sync::Arc::new(|_| {});
    let connected = runtime::connect_with_emitter(&desktop_id, Some(noop))
        .await
        .expect("pair and connect over the real transport");
    println!(
        "DLTIERS_CONNECTED {}",
        json!({
            "desktopId": desktop_id,
            "bridgeInstanceId": connected.bridge_instance_id,
            "features": connected.features,
            "agentAvailable": connected.agent_available,
        })
    );

    let old_host = std::env::var("DLTIERS_OLD_HOST").as_deref() == Ok("1");
    let report = if old_host {
        old_host_checks(&desktop_id, &dir).await
    } else {
        new_host_checks(&desktop_id, &dir).await
    };
    write_report(
        &dir.join("client.json"),
        &json!({
            "scenario": if old_host { "old-host" } else { "new-host" },
            "natsUrl": nats_url(),
            "clientPid": std::process::id(),
            "checks": report,
        }),
    );
}

/// The three downloads this acceptance is about, against a host that has the
/// desktop tier.
async fn new_host_checks(desktop_id: &str, dir: &Path) -> Value {
    let apk = source_path("app-debug.apk");
    let apk_source_sha = sha256_file(&apk);

    // ── 1. The desktop save-as that used to be refused ──
    let started = Instant::now();
    let fetched = transfer::download_for_desktop(
        desktop_id,
        "sess_dltiers",
        &apk.display().to_string(),
        Some("app-debug.apk"),
    )
    .await
    .expect("a desktop save-as of a 12.5 MB installer");
    let elapsed = started.elapsed();
    let destination = dir.join("saved").join("app-debug.apk");
    std::fs::create_dir_all(destination.parent().expect("destination parent"))
        .expect("create the destination directory");
    // The Desktop command's own save step, then read it back off the disk: what
    // the user ends up with, not what the transport assembled.
    transfer::write_to_path(&destination.display().to_string(), &fetched.bytes)
        .expect("write the downloaded file");
    let saved = std::fs::read(&destination).expect("read the saved file back");
    assert_eq!(saved.len() as u64, APK_BYTES, "the saved size");
    let saved_sha = sha256(&saved);
    assert_eq!(
        saved_sha, apk_source_sha,
        "the saved file equals the source byte for byte"
    );
    assert_eq!(fetched.name, "app-debug.apk");

    // ── 2. A file over the desktop ceiling ──
    let huge = source_path("disk-image.img");
    let huge_error = transfer::download_for_desktop(
        desktop_id,
        "sess_dltiers",
        &huge.display().to_string(),
        Some("disk-image.img"),
    )
    .await
    .expect_err("a 600 MiB file is over the desktop ceiling")
    .to_string();
    assert!(
        huge_error.contains("The file is over the 512 MB desktop download limit."),
        "the desktop ceiling speaks about itself: {huge_error}"
    );

    // ── 3. The same installer through the phone's mode ──
    let phone_error = transfer::download(
        desktop_id,
        "sess_dltiers",
        &apk.display().to_string(),
        Some("app-debug.apk"),
        "original",
    )
    .await
    .expect_err("the phone's 10 MiB ceiling still refuses it")
    .to_string();
    assert_eq!(
        phone_error, "The file is over the 10 MB download limit.",
        "the phone's sentence is unchanged, word for word"
    );

    // ── 4. The phone's *type* rule is lifted too, not only its ceiling ──
    // A 1 MiB `.bin` is under every size limit; only the phone's reader rule can
    // refuse it, so this arm fails if the desktop tier kept the allow-list.
    let binary = source_path("toolchain.bin");
    let bin_source_sha = sha256_file(&binary);
    let bin_fetched = transfer::download_for_desktop(
        desktop_id,
        "sess_dltiers",
        &binary.display().to_string(),
        Some("toolchain.bin"),
    )
    .await
    .expect("the desktop tier has no type allow-list");
    assert_eq!(sha256(&bin_fetched.bytes), bin_source_sha);
    let bin_phone_error = transfer::download(
        desktop_id,
        "sess_dltiers",
        &binary.display().to_string(),
        Some("toolchain.bin"),
        "original",
    )
    .await
    .expect_err("the phone's reader still refuses that type")
    .to_string();
    assert_eq!(
        bin_phone_error,
        "This file type is not available on mobile; view it on desktop."
    );

    // ── 5. One byte over the desktop ceiling ──
    // 600 MiB proves "a big file is refused"; this proves *where* the ceiling
    // is, so a limit quietly lowered (to, say, 500 MB) cannot pass as the same
    // behaviour.
    let edge = source_path("edge.bin");
    let edge_error = transfer::download_for_desktop(
        desktop_id,
        "sess_dltiers",
        &edge.display().to_string(),
        Some("edge.bin"),
    )
    .await
    .expect_err("512 MiB + 1 byte is over the desktop ceiling")
    .to_string();
    assert!(
        edge_error.contains("512 MB desktop download limit"),
        "the boundary is the 512 MiB ceiling: {edge_error}"
    );

    // ── 6. A mid-sized file, so the tier is shown to work above 12.5 MiB ──
    let large = source_path("database-dump.bin");
    let large_source_sha = sha256_file(&large);
    let large_started = Instant::now();
    let large_fetched = transfer::download_for_desktop(
        desktop_id,
        "sess_dltiers",
        &large.display().to_string(),
        Some("database-dump.bin"),
    )
    .await
    .expect("a 64 MiB desktop save-as");
    let large_elapsed = large_started.elapsed();
    assert_eq!(sha256(&large_fetched.bytes), large_source_sha);

    // ── Observed, not asserted: what the *preview* control still says ──
    // Opening the same installer goes through the phone's preview tier by
    // design (a preview is decoded and cached), so it is refused with the
    // phone's sentence. Recorded here because a reader of the saved-as path
    // should know the two controls now differ for this file.
    let preview_error = transfer::download(
        desktop_id,
        "sess_dltiers",
        &apk.display().to_string(),
        Some("app-debug.apk"),
        "preview",
    )
    .await
    .map(|fetched| format!("served {} bytes", fetched.bytes.len()))
    .unwrap_or_else(|error| error.to_string());

    json!({
        "desktop12_5": {
            "path": apk.display().to_string(),
            "sourceBytes": APK_BYTES,
            "savedPath": destination.display().to_string(),
            "savedBytes": saved.len(),
            "chunks": APK_BYTES.div_ceil(crate::remote_host::files::CHUNK_BYTES),
            "transferredName": fetched.name,
            "milliseconds": elapsed.as_millis(),
            "sourceSha256": apk_source_sha,
            "savedSha256": saved_sha,
            "sha256Equal": true,
        },
        "desktopOverCeiling": {
            "path": huge.display().to_string(),
            "bytes": std::fs::metadata(&huge).expect("huge").len(),
            "error": huge_error,
        },
        "phoneSameFile": {
            "mode": "original",
            "error": phone_error,
        },
        "desktopTypeLifted": {
            "path": binary.display().to_string(),
            "bytes": bin_fetched.bytes.len(),
            "sha256Equal": true,
        },
        "phoneSameType": {
            "mode": "original",
            "error": bin_phone_error,
        },
        "desktopBoundary": {
            "path": edge.display().to_string(),
            "bytes": std::fs::metadata(&edge).expect("edge").len(),
            "error": edge_error,
        },
        "desktopPreviewSameFile": {
            "mode": "preview",
            "result": preview_error,
        },
        "desktop64": {
            "path": large.display().to_string(),
            "bytes": large_fetched.bytes.len(),
            "chunks": LARGE_BYTES.div_ceil(crate::remote_host::files::CHUNK_BYTES),
            "milliseconds": large_elapsed.as_millis(),
            "sha256Equal": true,
        },
    })
}

/// What a desktop does against a host built **before** the `download` variant.
async fn old_host_checks(desktop_id: &str, dir: &Path) -> Value {
    let small = source_path("release-notes.txt");
    let small_sha = sha256_file(&small);
    // The old host refuses the variant itself, only for a file its phone rules
    // accept; that refusal is what the fallback matches on.
    let variant_error = transfer::download(
        desktop_id,
        "sess_dltiers",
        &small.display().to_string(),
        Some("release-notes.txt"),
        "download",
    )
    .await
    .expect_err("an old host does not implement the desktop variant")
    .to_string();
    assert!(
        variant_error.contains("Unsupported download variant."),
        "the old host answers the matched refusal: {variant_error}"
    );

    let started = Instant::now();
    let fetched = transfer::download_for_desktop(
        desktop_id,
        "sess_dltiers",
        &small.display().to_string(),
        Some("release-notes.txt"),
    )
    .await
    .expect("the fallback to `original` completes the download");
    let elapsed = started.elapsed();
    let destination = dir.join("saved-old").join("release-notes.txt");
    std::fs::create_dir_all(destination.parent().expect("destination parent"))
        .expect("create the destination directory");
    transfer::write_to_path(&destination.display().to_string(), &fetched.bytes)
        .expect("write the downloaded file");
    let saved_sha = sha256_file(&destination);
    assert_eq!(saved_sha, small_sha, "the fallback saved the source bytes");

    // The installer the report was about: an old host judges size before the
    // variant, so it refuses with the phone's sentence — today's outcome.
    let apk = source_path("app-debug.apk");
    let apk_error = transfer::download_for_desktop(
        desktop_id,
        "sess_dltiers",
        &apk.display().to_string(),
        Some("app-debug.apk"),
    )
    .await
    .expect_err("an old host still applies the phone's ceiling")
    .to_string();
    assert_eq!(apk_error, "The file is over the 10 MB download limit.");

    json!({
        "variantRefusal": { "mode": "download", "error": variant_error },
        "fallback": {
            "path": small.display().to_string(),
            "savedPath": destination.display().to_string(),
            "bytes": fetched.bytes.len(),
            "milliseconds": elapsed.as_millis(),
            "sourceSha256": small_sha,
            "savedSha256": saved_sha,
            "sha256Equal": true,
        },
        "installerOnOldHost": {
            "path": apk.display().to_string(),
            "error": apk_error,
        },
    })
}

/// Read one query parameter out of the invitation URL.
fn url_field(invitation: &str, name: &str) -> String {
    reqwest::Url::parse(invitation)
        .expect("invitation url")
        .query_pairs()
        .find(|(key, _)| key == name)
        .map(|(_, value)| value.into_owned())
        .unwrap_or_else(|| panic!("invitation is missing {name}"))
}
