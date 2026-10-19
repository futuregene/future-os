//! Migrated controller retry/timeout/watch policy tests, without application state.
use super::{is_request_timeout, should_retry_connect, visibility_change};

/// The decision itself, every combination — the three conditions are each
/// load-bearing, and a fixture cannot produce a rejected credential or a
/// user-stopped host on demand.
#[test]
fn connect_retry_policy_is_transient_and_not_stopped_and_not_already_retrying() {
    let transient = crate::Error::RemoteTransport("Failed to connect to NATS: x".into());
    let permanent = crate::Error::RemoteAuthorization("Failed to connect to NATS: x".into());
    let other_fault = crate::Error::Message("no such host".into());

    // The one case that retries.
    assert!(should_retry_connect(&transient, false, false));
    // A refused credential never does: the pairing, not the network, is wrong.
    assert!(!should_retry_connect(&permanent, false, false));
    // Anything that is not a transport fault is not the network either.
    assert!(!should_retry_connect(&other_fault, false, false));
    // A host the user disconnected stays disconnected.
    assert!(!should_retry_connect(&transient, true, false));
    // The loop's own `resume` comes through here; it must not spawn a second loop.
    assert!(!should_retry_connect(&transient, false, true));
}

/// The Noise handshake leg's two failures land on opposite sides of the retry
/// decision, and that is the whole point of `classify_connect_error` existing
/// for the socket leg: a handshake that **timed out** is the network, a
/// handshake the host **refused** is the pairing.
///
/// Pinned rather than inferred (the todo for this section asked for exactly
/// that). The three messages are the ones the code really produces
/// (`session::handshake`'s `exchange`): the outer budget's own timeout, the
/// `async-nats` inner one, and the refusal that carries the host's `detail`.
#[test]
fn handshake_failures_are_retried_only_when_they_are_timeouts() {
    let outer_timeout = crate::Error::RemoteTransport("Remote handshake timed out".into());
    let inner_timeout = crate::Error::RemoteTransport(
        "Remote handshake failed: request timed out: deadline has elapsed".into(),
    );
    let refused = crate::Error::Message("remote_secure_channel_invalid (invitation)".into());

    // A handshake that timed out is transient: the host may have been busy.
    assert!(
        should_retry_connect(&outer_timeout, false, false),
        "an expired handshake budget is the network, not the pairing"
    );
    assert!(
        should_retry_connect(&inner_timeout, false, false),
        "the library's inner timeout is the same kind of failure"
    );
    // A handshake the host refused is not: the invitation, the pin or the
    // credential is wrong, and retrying would repeat a refusal forever while
    // hiding the re-pair the user has to do.
    assert!(
        !should_retry_connect(&refused, false, false),
        "a refused handshake must not be retried"
    );
    // And neither is retried once the user has stopped the host.
    assert!(!should_retry_connect(&outer_timeout, true, false));
}

/// `is_request_timeout` answers for *request* legs only, which is what its
/// callers rely on.
///
/// The handshake rows are the reason this test exists: the Noise leg's inner
/// timeout contains the request phrase verbatim, so a bare substring match said
/// "yes" for a handshake. Nothing calls it with one today (handshake errors come
/// out of `connect_with_emitter`, not from a live connection's request path), and
/// this pins that a future caller cannot get the wrong answer either.
#[test]
fn request_timeouts_are_request_legs_only() {
    let transport = |message: &str| crate::Error::RemoteTransport(message.to_string());

    // The two request-leg shapes: yes.
    assert!(is_request_timeout(&transport("Remote request timed out")));
    assert!(is_request_timeout(&transport(
        "Remote request failed: request timed out: deadline has elapsed"
    )));
    // The handshake leg's inner timeout carries the same phrase: no.
    assert!(!is_request_timeout(&transport(
        "Remote handshake failed: request timed out: deadline has elapsed"
    )));
    // The socket-connect leg: no (there is no session to probe).
    assert!(!is_request_timeout(&transport(
        "Failed to connect to NATS: timed out"
    )));
    // Anything that is not a transport fault: no.
    assert!(!is_request_timeout(&crate::Error::Message(
        "request timed out".into()
    )));
}

/// Once every visibility sender is gone the lane can never move again, so the
/// future must **park** rather than return a lane or spin. That is the state a
/// test reset leaves behind (it replaces the runtime map holding the senders),
/// and a loop that kept re-polling there would burn a core for nothing.
#[tokio::test(start_paused = true)]
async fn a_closed_visibility_watch_parks_instead_of_yielding_a_lane() {
    let (sender, mut receiver) = tokio::sync::watch::channel(None::<String>);
    drop(sender);

    let outcome = tokio::time::timeout(
        std::time::Duration::from_millis(150),
        visibility_change(&mut receiver),
    )
    .await;

    assert!(
        outcome.is_err(),
        "a closed watch must park forever, not hand back a lane: {outcome:?}"
    );
}
