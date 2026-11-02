//! `dns::tcp_resolver` in a process whose resolver plan is *custom*.
//!
//! One test per process on purpose. `future_http::dns`'s plan is a process-wide
//! `OnceLock` (`packages/http/src/dns/mod.rs:141`): the first `plan()` call in a
//! process fixes `System` or `Custom` for every later caller, and it also prints
//! the `[dns] plan=…` announcement once. So a lib test cannot make
//! `tcp_resolver()` answer `Some` — by the time it runs, some sibling has
//! already built a client — and setting `FUTURE_DNS_SERVER` there would either
//! be ignored (plan already initialised) or change every other test's plan.
//! An integration target is a process of its own, so the variable set at the
//! top of the test is the one the plan reads.
//!
//! What this pins: the CLI hands tonic a resolver exactly when the plan is
//! custom, and that resolver answers through the configured nameserver.
//! `commands` reach `tcp_resolver()` on every TCP call (`rpc.rs:126`, `:912`),
//! so a `None` where a `Some` belongs is not a cosmetic difference — it is the
//! gRPC endpoint resolving through `getaddrinfo` while HTTP resolves through the
//! nameserver the operator configured.

/// A configured nameserver attaches the gRPC connector's resolver, and the
/// resolver it attaches works.
///
/// `FUTURE_DNS_SERVER=127.0.0.1:9` is never queried: the endpoint resolved here
/// is an IP literal, which `resolve_configured` answers without a lookup
/// (`packages/http/src/dns/mod.rs:250`) — so this test needs no nameserver, no
/// network, and no Agent.
#[tokio::test]
async fn a_configured_nameserver_attaches_the_grpc_resolver() {
    // Before the first `plan()` call in this process, which is this test's own.
    std::env::set_var("FUTURE_DNS_SERVER", "127.0.0.1:9");
    // A file-only override would outrank nothing here, but a stray
    // `FUTURE_RESOLV_CONF` from the developer's shell must not decide the plan.
    std::env::remove_var("FUTURE_RESOLV_CONF");

    let resolver = future_cli::dns::tcp_resolver()
        .expect("a custom plan is exactly the case the gRPC connector needs a resolver for");

    // The nameserver is configured and the endpoint above is an IP literal, so
    // nothing here touches the wire.
    let addresses = resolver("127.0.0.1".to_string(), 7000)
        .await
        .expect("an IP literal resolves without asking the nameserver");
    assert_eq!(
        addresses,
        vec!["127.0.0.1:7000".parse().expect("parse")],
        "the resolver answers the endpoint it was handed, not a placeholder"
    );
}
