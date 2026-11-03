//! The lookup failure a caller may retry, in a process of its own.
//!
//! `for_connect` decides once per process whether this build resolves names by
//! itself, so the failure below cannot share a binary with the tests that point
//! at an answering stub: whichever test read the plan first would decide for all
//! of them. This file points at a closed loopback port, which makes the lookup
//! fail immediately rather than after a timeout.

use std::net::UdpSocket;

#[tokio::test]
async fn a_nameserver_that_does_not_answer_is_a_transient_refusal() {
    let closed = UdpSocket::bind("127.0.0.1:0").expect("reserve a port");
    let port = closed.local_addr().expect("port").port();
    drop(closed);
    std::env::set_var("FUTURE_DNS_SERVER", format!("127.0.0.1:{port}"));
    std::env::remove_var("FUTURE_RESOLV_CONF");

    let attempt = tokio::time::timeout(
        std::time::Duration::from_secs(20),
        future_relay_endpoint::for_connect("nats://relay.example:4222", None, true),
    )
    .await
    .expect("the closed port must answer, not hang");
    let error = attempt.expect_err("nothing can resolve against a closed nameserver");

    // A lookup that did not answer is what a retry is for; a bad URL or a bad
    // bundle is not, and the flag is what the caller branches on.
    assert!(
        error.transient,
        "a lookup that failed is worth retrying: {error}"
    );
    assert!(
        error.message.contains("Failed to resolve the relay"),
        "{error}"
    );
    assert!(
        error.message.contains("relay.example"),
        "the name it could not resolve is named: {error}"
    );
}
