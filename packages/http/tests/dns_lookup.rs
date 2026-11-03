//! Hostname resolution through the public entry points.
//!
//! `dns::resolve` is what `tonic` reuses (it builds its own connector, so it
//! cannot go through `attach`), and `dns::preflight` is the check an endpoint
//! passes through before a native client connects. Both were reachable only
//! through code paths the unit tests could not enter, because `Plan::detect`
//! decides once per process and the crate's own tests build plans by hand.
//!
//! This is an integration test on purpose: it gets its **own process**, so the
//! detected plan is the one this environment really produces (no env override,
//! so `Plan::System`) rather than whatever a sibling unit test has already
//! fixed into the process-wide `OnceLock`.

/// The public entry point returns addresses on the requested port.
///
/// `localhost` is resolved from the hosts file by the system plan, so this
/// asserts the wiring — the plan is consulted, the addresses are returned with
/// the asked-for port — without depending on a network.
#[tokio::test]
async fn resolve_returns_addresses_on_the_requested_port() {
    let addresses = future_http::dns::resolve("localhost", 4_321)
        .await
        .expect("resolve localhost");
    assert!(!addresses.is_empty(), "no addresses for localhost");
    for address in &addresses {
        assert_eq!(
            address.port(),
            4_321,
            "the port must be the one asked for: {address}"
        );
    }
}

/// Nothing is added to `getaddrinfo`, so a preflight is a no-op.
///
/// That is the documented default: the system plan leaves every lookup exactly
/// as it was before this module existed.
#[tokio::test]
async fn preflight_is_a_no_op_on_the_system_plan() {
    assert!(
        !future_http::dns::is_custom(),
        "this environment should detect the system plan"
    );
    future_http::dns::preflight("https://example.test:8443")
        .await
        .expect("a system plan has nothing to check");
    future_http::dns::preflight("127.0.0.1:50051")
        .await
        .expect("so is a literal address");
}

/// An endpoint that is not TCP is not resolved at all.
///
/// `auto`, a unix socket path and a named pipe all reach a native client, and
/// none of them is a host name — preflighting one would be a lookup of the
/// string `auto`.
#[tokio::test]
async fn preflight_ignores_endpoints_that_are_not_tcp() {
    for endpoint in [
        "auto",
        "unix:///tmp/future-agent.sock",
        "npipe:////./pipe/future-agent",
        "/tmp/future-agent.sock",
    ] {
        future_http::dns::preflight(endpoint)
            .await
            .unwrap_or_else(|error| panic!("{endpoint} must be left alone: {error}"));
    }
}
