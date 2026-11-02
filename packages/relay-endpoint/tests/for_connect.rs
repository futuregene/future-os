//! `for_connect` on the plan that rewrites the URL.
//!
//! The interesting branch is the custom-resolver one: when this process reads a
//! nameserver itself, the relay URL is rewritten to an address and its
//! certificate is pinned to the name that was asked for. That decision is made
//! once per process (`future_http::dns::is_custom`), so it cannot be reached
//! from the crate's unit tests — every one of them runs on the system plan and
//! sees the "unchanged" early return.
//!
//! A stub nameserver answers the lookup here, so the assertions are about the
//! rewrite rather than about a network.

use std::net::UdpSocket;

/// A UDP nameserver that answers every `A` question with `127.0.0.1`.
///
/// `AAAA` gets an empty answer section, which is a name with no IPv6 address —
/// exactly what a v4-only relay looks like.
fn serve_nameserver() -> u16 {
    let socket = UdpSocket::bind("127.0.0.1:0").expect("bind the stub nameserver");
    let port = socket.local_addr().expect("stub address").port();
    std::thread::spawn(move || {
        let mut buffer = [0u8; 512];
        while let Ok((length, peer)) = socket.recv_from(&mut buffer) {
            let request = &buffer[..length];
            if length < 16 {
                continue;
            }
            let qtype = u16::from_be_bytes([request[length - 4], request[length - 3]]);
            let mut reply = Vec::with_capacity(length + 16);
            reply.extend_from_slice(&request[0..2]);
            reply.extend_from_slice(&[0x81, 0x80]);
            reply.extend_from_slice(&[0x00, 0x01]);
            reply.extend_from_slice(&if qtype == 1 { 1u16 } else { 0u16 }.to_be_bytes());
            reply.extend_from_slice(&[0, 0, 0, 0]);
            reply.extend_from_slice(&request[12..]);
            if qtype == 1 {
                reply.extend_from_slice(&[0xC0, 0x0C, 0x00, 0x01, 0x00, 0x01]);
                reply.extend_from_slice(&60u32.to_be_bytes());
                reply.extend_from_slice(&[0x00, 0x04, 127, 0, 0, 1]);
            }
            let _ = socket.send_to(&reply, peer);
        }
    });
    port
}

/// Point this process at the stub, and assert the plan really is custom.
fn use_custom_plan() {
    let port = serve_nameserver();
    std::env::set_var("FUTURE_DNS_SERVER", format!("127.0.0.1:{port}"));
    std::env::remove_var("FUTURE_RESOLV_CONF");
    assert!(
        future_http::dns::is_custom(),
        "an explicit nameserver must select the custom plan"
    );
}

#[tokio::test]
async fn a_hostname_is_rewritten_to_the_resolved_address() {
    use_custom_plan();
    let endpoint = future_relay_endpoint::for_connect("nats://relay.example:4222", None, true)
        .await
        .expect("a resolvable name is rewritten, not refused");
    // The port and scheme survive; the name does not.
    assert!(endpoint.url.starts_with("nats://"), "{}", endpoint.url);
    assert!(endpoint.url.contains("127.0.0.1:4222"), "{}", endpoint.url);
    assert!(!endpoint.url.contains("relay.example"), "{}", endpoint.url);
    // Plaintext keeps no TLS configuration: there is nothing to pin.
    assert!(
        endpoint.tls.is_none(),
        "plaintext must not pin a certificate"
    );
}

#[tokio::test]
async fn a_rewritten_endpoint_pins_the_certificate_to_the_name_that_was_asked_for() {
    use_custom_plan();
    let endpoint = future_relay_endpoint::for_connect("tls://relay.example:4222", None, false)
        .await
        .expect("the machine's own roots are enough to build the configuration");
    assert!(endpoint.url.contains("127.0.0.1:4222"), "{}", endpoint.url);
    assert!(
        endpoint.tls.is_some(),
        "a rewritten TLS endpoint must carry the pinned configuration"
    );
}

#[tokio::test]
async fn an_endpoint_that_needs_no_rewrite_is_left_alone() {
    use_custom_plan();
    // An address is already what the rewrite would produce.
    let literal = future_relay_endpoint::for_connect("nats://10.1.2.3:4222", None, false)
        .await
        .expect("a literal address needs no lookup");
    assert_eq!(literal.url, "nats://10.1.2.3:4222");
    assert!(literal.tls.is_none());
}

#[tokio::test]
async fn a_url_that_is_not_a_url_is_a_permanent_refusal() {
    use_custom_plan();
    let error = future_relay_endpoint::for_connect("relay without a scheme", None, true)
        .await
        .expect_err("a bad URL is a configuration mistake");
    assert!(!error.transient, "no retry repairs it: {error}");
    assert!(error.message.contains("invalid relay URL"), "{error}");
}

#[tokio::test]
async fn a_root_bundle_that_cannot_be_read_is_reported_rather_than_ignored() {
    use_custom_plan();
    let directory = tempfile::tempdir().expect("tempdir");
    let absent = directory.path().join("no-such-roots.pem");
    let error =
        future_relay_endpoint::for_connect("tls://relay.example:4222", Some(&absent), false)
            .await
            .expect_err("a named bundle that is not there is a configuration mistake");
    assert!(!error.transient, "no retry repairs it: {error}");
    assert!(
        error.message.contains("read relay root certificates"),
        "{error}"
    );
}
