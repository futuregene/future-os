//! The configured-nameserver plan, in its own process.
//!
//! `Plan::detect` decides once per process and caches the answer, so the custom
//! path cannot be entered from the crate's unit tests without fixing that
//! decision for every test in the binary. A separate integration test file gets
//! its own process: it points the resolver at a closed loopback port, which
//! makes the plan custom and every query fail fast instead of hanging.
//!
//! What is asserted is the contract the guide gives: a non-TCP endpoint is left
//! alone (preflighting one would look up the string `auto`), and a TCP endpoint
//! that the nameserver cannot answer fails with that name in the message rather
//! than a bare "error sending request".

use std::time::Duration;

/// A closed port on loopback: the query fails at once instead of waiting.
const CLOSED_NAMESERVER: &str = "127.0.0.1:5399";

#[tokio::test]
async fn a_configured_nameserver_switches_the_plan_and_leaves_non_tcp_endpoints_alone() {
    // Must be set before the plan is decided for this process — every call
    // below is the first one in this binary.
    std::env::set_var("FUTURE_DNS_SERVER", CLOSED_NAMESERVER);
    std::env::remove_var("FUTURE_RESOLV_CONF");

    assert!(
        future_http::dns::is_custom(),
        "an explicit nameserver must select the custom plan"
    );

    // Not a host name, so nothing is looked up: `auto`, a unix socket, a named
    // pipe, and a path with no host at all.
    for endpoint in [
        "auto",
        "AUTO",
        "unix:///tmp/future-agent.sock",
        "npipe:////./pipe/future-agent",
        "/tmp/future-agent.sock",
        "",
    ] {
        future_http::dns::preflight(endpoint)
            .await
            .unwrap_or_else(|error| panic!("{endpoint} must not be resolved: {error}"));
    }
}

#[tokio::test]
async fn a_tcp_endpoint_the_nameserver_cannot_answer_names_the_host() {
    std::env::set_var("FUTURE_DNS_SERVER", CLOSED_NAMESERVER);
    std::env::remove_var("FUTURE_RESOLV_CONF");

    // A bounded wait: the failure is a closed port, not a timeout we sit on.
    let attempt = tokio::time::timeout(
        Duration::from_secs(20),
        future_http::dns::preflight("https://unreachable.invalid:8443"),
    )
    .await
    .expect("the closed nameserver must answer, not hang");

    let error = attempt.expect_err("nothing can resolve against a closed nameserver");
    assert!(
        error.contains("unreachable.invalid"),
        "the message names the host it could not resolve: {error}"
    );
}
