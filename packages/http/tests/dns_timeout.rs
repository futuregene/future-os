//! What a nameserver that never answers looks like to the caller.
//!
//! The interesting half of a resolver failure is the timeout: a closed port
//! answers at once, so the "did not answer" path needs an address that swallows
//! the packet. TEST-NET-1 (`192.0.2.0/24`, RFC 5737) is reserved for exactly
//! that — documentation traffic, never routed — so the query times out instead
//! of reaching anyone.
//!
//! The budget is cut to one second in a purpose-built `resolv.conf`, because the
//! default is five seconds per attempt and two attempts: the point is the
//! message, not the wait.

use std::io::Write;

#[tokio::test]
async fn a_nameserver_that_never_answers_reports_the_timeout_with_the_query_type() {
    let directory = tempfile::tempdir().expect("tempdir");
    let path = directory.path().join("resolv.conf");
    let mut file = std::fs::File::create(&path).expect("resolv.conf");
    writeln!(file, "nameserver 192.0.2.1").expect("nameserver line");
    writeln!(file, "options timeout:1 attempts:1").expect("options line");
    drop(file);

    // Decided here, before the plan is read for the first time in this process.
    std::env::set_var("FUTURE_RESOLV_CONF", &path);
    std::env::remove_var("FUTURE_DNS_SERVER");
    assert!(
        future_http::dns::is_custom(),
        "a resolv.conf is a custom plan"
    );

    let error = future_http::dns::preflight("https://silent.invalid:8443")
        .await
        .expect_err("a blackholed nameserver cannot answer");

    // The message names the server, the record type and the deadline it gave
    // up after — the three things an operator needs to tell "no such name" from
    // "nothing is listening".
    assert!(
        error.contains("did not answer"),
        "unexpected message: {error}"
    );
    assert!(error.contains("192.0.2.1"), "the server is named: {error}");
    assert!(
        error.contains("A query"),
        "the record type is named: {error}"
    );
    assert!(error.contains("1s"), "the deadline is named: {error}");
}
