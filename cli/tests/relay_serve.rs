//! What `future relay serve` tells an operator before it starts serving.
//!
//! The banner and the plaintext warning are the only place a reader learns
//! which deployment this process is trying to be, and they go to the process's
//! own streams, so the real binary is the only way to read them. Every case
//! here exits on its own — a configuration it refuses, or a port it cannot
//! bind — because `serve` otherwise runs for the life of the process.

use std::process::Command;

fn future(args: &[&str]) -> (Option<i32>, String, String) {
    let output = Command::new(env!("CARGO_BIN_EXE_future"))
        .args(args)
        .output()
        .expect("spawn future binary");
    (
        output.status.code(),
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

/// A `relay.json` `serve` will read, with a real account seed (`Relay::new`
/// refuses anything else) and no certificate.
fn relay_config(dir: &std::path::Path, listen: &str, public_url: &str) -> std::path::PathBuf {
    let seed = nkeys::KeyPair::new_account().seed().expect("account seed");
    let path = dir.join("relay.json");
    std::fs::write(
        &path,
        serde_json::json!({
            "listen": listen,
            "publicUrl": public_url,
            "natsUrl": "nats://127.0.0.1:4222",
            "natsWsUrl": "ws://127.0.0.1:9090",
            "accountSeed": seed,
        })
        .to_string(),
    )
    .expect("write the configuration");
    path
}

/// A bare `future relay` is a request to read the usage, not an error.
#[test]
fn a_bare_invocation_prints_the_usage() {
    for args in [
        vec!["relay"],
        vec!["relay", "--help"],
        vec!["relay", "help"],
    ] {
        let (code, stdout, stderr) = future(&args);
        assert_eq!(code, Some(0), "{args:?}: {stderr}");
        assert!(stdout.contains("future relay"), "{args:?}: {stdout}");
        assert!(stdout.contains("serve"), "{args:?}: {stdout}");
        assert!(stderr.is_empty(), "{args:?}: {stderr}");
    }
}

/// A relay has no default configuration file: one that guessed would serve
/// somebody else's rooms.
#[test]
fn serve_refuses_to_start_without_a_configuration() {
    let (code, _, stderr) = future(&["relay", "serve"]);
    assert_eq!(code, Some(1), "{stderr}");
    assert!(stderr.contains("serve needs --config FILE"), "{stderr}");
    assert!(
        stderr.contains("relay init"),
        "the way out is named: {stderr}"
    );

    let (code, _, stderr) = future(&["relay", "serve", "--config", "/nonexistent/relay.json"]);
    assert_eq!(code, Some(1), "{stderr}");
    assert!(stderr.contains("relay.json"), "{stderr}");
}

/// `https` with no certificate is the proxy deployment: the listener stays
/// plaintext on purpose, and the operator is told so — then the bind failure
/// names the reason, instead of the warning being the last word.
#[test]
fn an_https_url_without_a_certificate_is_announced_and_the_bind_failure_explains_it() {
    let dir = tempfile::tempdir().expect("temp dir");
    let holder = std::net::TcpListener::bind(("127.0.0.1", 0)).expect("bind a holder");
    let port = holder.local_addr().expect("address").port();
    let path = relay_config(
        dir.path(),
        &format!("127.0.0.1:{port}"),
        "https://relay.example.com",
    );

    let (code, stdout, stderr) = future(&[
        "relay",
        "serve",
        "--config",
        path.to_str().expect("utf-8 path"),
    ]);
    assert_eq!(code, Some(1), "a taken port is an error: {stderr}");

    // What it was about to serve: the address it could not take, the account
    // key phones will trust, and the public URL invitations will carry.
    assert!(
        stdout.contains(&format!(
            "future relay listening on http://127.0.0.1:{port}"
        )),
        "{stdout}"
    );
    assert!(
        stdout.contains("public https://relay.example.com"),
        "{stdout}"
    );
    assert!(stdout.contains("account A"), "{stdout}");
    assert!(stdout.contains("rooms 0"), "{stdout}");

    // And why that listener is plaintext despite the `https` public URL.
    assert!(stderr.contains("no certificate is configured"), "{stderr}");
    assert!(stderr.contains("TLS terminator"), "{stderr}");
    // The OS reason is the last line, asked of this host rather than hard-coded:
    // unix says "Address already in use", Windows "Only one usage of each socket
    // address (protocol/network address/port) is normally permitted". Binding
    // the held port again here produces the wording the child was given.
    let os_reason = std::net::TcpListener::bind(("127.0.0.1", port))
        .expect_err("the holder still owns the port")
        .to_string();
    let last = stderr
        .lines()
        .rev()
        .find(|line| !line.trim().is_empty())
        .unwrap_or_default();
    assert!(
        last.contains(&os_reason),
        "the OS reason is the last line: {last:?} vs {os_reason:?}"
    );
    drop(holder);
}

/// A listen address that is not an address is refused as a configuration
/// problem, before anything is bound.
#[test]
fn an_unreadable_listen_address_is_named() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = relay_config(dir.path(), "127.0.0.1:not-a-port", "http://127.0.0.1:8099");

    let (code, _, stderr) = future(&[
        "relay",
        "serve",
        "--config",
        path.to_str().expect("utf-8 path"),
    ]);
    assert_eq!(code, Some(1), "{stderr}");
    assert!(
        stderr.contains("listen address `127.0.0.1:not-a-port`"),
        "{stderr}"
    );
}
