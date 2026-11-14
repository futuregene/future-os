//! The `skipTlsVerify` acceptance test: a real self-signed HTTPS endpoint.
//!
//! The unit tests around it check that the flag travels (models.json → registry
//! → route); this one checks what the flag is *for*. A certificate no platform
//! store trusts makes the same request fail under verification and succeed once
//! the provider says to skip it — which is the entire promise of the feature,
//! and the one thing a field-plumbing test cannot show.
//!
//! The endpoint is the real thing: TLS terminated by `rustls` with an embedded
//! self-signed leaf (the same fixture bytes the host's relay tests use, so no
//! certificate tooling is needed and every machine gets identical bytes), then a
//! minimal HTTP/1.1 server that answers one Chat Completions stream. The client
//! side is the production path — `Client` built from a resolved target, which is
//! what a session uses.
use super::schema::{ModelStreamEvent, ProtocolConfig};
use super::tests::{canonical_request, protocol_target};
use super::*;
use crate::types::LLMProvider;
use base64::Engine as _;
use futures::StreamExt;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// Header + body of one Chat Completions stream: a single content delta, a
/// finish reason, then the `[DONE]` sentinel.
const CHAT_SSE: &str = concat!(
    "data: {\"choices\":[{\"delta\":{\"content\":\"pong\"}}]}\n\n",
    "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n",
    "data: [DONE]\n\n",
);

/// A self-signed leaf for `127.0.0.1` / `localhost` and its private key, as
/// base64 DER. Same fixture as `host/src/remote_platform.rs`: the key belongs to
/// a listener that exists only inside the test process.
const SERVER_CERT_DER_B64: &str = "\
MIIDVjCCAj6gAwIBAgIUfWhQyJhGIy7CfdR/scf6VgeI0gcwDQYJKoZIhvcNAQEL\
BQAwITEfMB0GA1UEAwwWRnV0dXJlT1MgVGVzdCBSZWxheSBDQTAeFw0yNjEwMjEw\
MzEzMjJaFw0yNzExMjIwMzEzMjJaMBQxEjAQBgNVBAMMCTEyNy4wLjAuMTCCASIw\
DQYJKoZIhvcNAQEBBQADggEPADCCAQoCggEBAKRGAp5OqIybHqcTDxacss3lPQMy\
lAeELA+VI9TNNfbZHx+2VaxbfcxQbfRgBhoI2wTXds2HjJWkbzFwP2FTFZKFN533\
4urTjRG4lWX/hkEW5+g8E2uUkNMTSEU/l+O98g0j6IAQDh3R3bk9FegxfUfxwxVa\
So537/OLfOPI/0HlnosCVwyMKLcEmS3OZjycjCLeKr4TJgpiL9oDypVQkPTHOdUh\
WnqJH1v6conrmCxLThhNQz2KXSy1abN/YjUwBC9kXYqrwpsu72hxr//I1RB/T94z\
yGsO+/MrzNyI5Lp3a5g1sMUr1DCUFnECKBCxsZLyqows+mstfBD1wvuF7J0CAwEA\
AaOBkjCBjzAaBgNVHREEEzARhwR/AAABgglsb2NhbGhvc3QwEwYDVR0lBAwwCgYI\
KwYBBQUHAwEwDAYDVR0TAQH/BAIwADAOBgNVHQ8BAf8EBAMCBaAwHQYDVR0OBBYE\
FMhXPj8jSXn4sRVM6nS3jjTHwiw2MB8GA1UdIwQYMBaAFHHqXjYSM8SAqKim2Umb\
H2pT0ZOkMA0GCSqGSIb3DQEBCwUAA4IBAQACwJifIZxSbhiBI39x5Vv+OPx30zkd\
UmUOWYP1haYjD0B/im4xco1NWIInWG+0F8SypBjjw8yDVP6Cdomj8eElPwz56j9m\
H238g9unH7rSYg74pcw/QktnzP3mpusdEuKhjBxAblm/xtqdeEcZQYzsoOMHKuLU\
QRPk6+gnHltJLq7PLLMBPKWgKpoxF8X7O1TaNtArkdKuOgbl5JIFeVSqj6V0kZwK\
57bDywTV6Yhn6OMu13tSLSRYTHmItG3dIECRkxeQUpd4VUTdSEXMGIimj5p3wwae\
8Mk0zY1cyeBMDM9m7Peh7nlXWDe4yrFq+h9dwFpsP7LiaKPlNEtBOG8y";
const SERVER_KEY_DER_B64: &str = "\
MIIEvQIBADANBgkqhkiG9w0BAQEFAASCBKcwggSjAgEAAoIBAQCkRgKeTqiMmx6n\
Ew8WnLLN5T0DMpQHhCwPlSPUzTX22R8ftlWsW33MUG30YAYaCNsE13bNh4yVpG8x\
cD9hUxWShTed9+Lq040RuJVl/4ZBFufoPBNrlJDTE0hFP5fjvfINI+iAEA4d0d25\
PRXoMX1H8cMVWkqOd+/zi3zjyP9B5Z6LAlcMjCi3BJktzmY8nIwi3iq+EyYKYi/a\
A8qVUJD0xznVIVp6iR9b+nKJ65gsS04YTUM9il0stWmzf2I1MAQvZF2Kq8KbLu9o\
ca//yNUQf0/eM8hrDvvzK8zciOS6d2uYNbDFK9QwlBZxAigQsbGS8qqMLPprLXwQ\
9cL7heydAgMBAAECggEATLLfcBibu6KFak6MwesB1P5OQVrxIJ1GkTP6GdaCdrGj\
RYQ1dhb1olp/LBVVY/x0fks3jfjW29t9iyC0hvVq4hFPCXN+wFNrzzVWk9v9TBCs\
0JKUIqun/tVG7gYmHdBG3nnAiWJ1pNii5GUwAExzCQ9JHYQdwW0gXJ7o/dWP0Kzs\
mxly/Y1VobQTEQkpWXR4vRnlVLQVKxmU5hk2phrMHw34G1/PL7KONS4LfYFoC37e\
3T3qHYPeXUFtVwrdh0YoeycFC65TPVQnPCoauGGCmMne3gTkninjaWc/ixSQO3Z/\
RddqAhNr8q4YmAbRII9NBXN5IhDQQ9qKHcW5XVb9BQKBgQDO97CzDkxZkqGWvp02\
4PqPTjPA6GjZMBpPZi8fV2CcLbtriCLPj2nLQYREvteXhoHbGZPrDhij7DB2V2Ja\
oc4vbKbMYWp8EcGIRIGZWOFjcFzUMdjRbD6YBtUS0I96vtQJSRDgNvsq7t65Cy38\
pu1kFiJ5Sy7YgDM72lizAT8oZwKBgQDLMPsqeQwA6bD8N58QiAi9oINn+wz7S+wQ\
rayfIh2eEhDblWNzvVX6V/w1pQVQqeD0mKHOn413nhKpg9n+f0OMFIZueh7S5qID\
hTW2tA3P09FVP3Hsybvr7FEHncGEJwlSMV7tm4Fv/RRWLTgUF9lSgmf6H7huIRE1\
Su/DEv7wWwKBgEDIo1TI6vk2vsHS1J8y+PsWfXiqxvcE6Zdk4ucqkKESZ5IXQN47\
g+ANl/t0yUMy5T7R2M1A0lGtQtYoMC8yarnJhsR+0bgYTr36w55ehd58c3IwUC/t\
dAJeUR/cjKzdQr3NUyz7yM79ilXdu/1TuwcTLqNM91LpfiAEDrOpdj2NAoGAD0df\
m5H+lRL9e/myvEt+XMP1HZyDHWJYHejxUgX+tCQnoFtbXxidnnV/QYlnHOmpQLs5\
b5AP0y7PuG9xtPFc4uuoMCZ+qt0s0ZbyswpW/4mt8/sRVfzVZKggb7ItGwLUcJnF\
yZgn44PkQ5ai4N/qN4oWx3gbJizWXmuEG+cRxIECgYEAjFQ4Y/30NQ8tJIKWdDjI\
E8DcQHFxtw1rp9rGj9oXdbrVvIK/mimcH7cOYFuf6jp6dTrq0nTWZFCxS0idji4f\
RKaaeMNbYRqPDeBxUW80Kk1KTCUqhyqev2ZtSDKkcplguYPcAdOH5JEqYwZmNcU4\
op9Y9tWQWw2IDo18bVpjmN8=";

fn decode_fixture(text: &str) -> Vec<u8> {
    let compact: String = text.chars().filter(|c| !c.is_whitespace()).collect();
    base64::engine::general_purpose::STANDARD
        .decode(compact)
        .expect("fixture DER")
}

/// A self-signed HTTPS endpoint that answers every request with [`CHAT_SSE`].
///
/// Returns the `https://` base URL. The listener runs on its own thread with its
/// own runtime: the caller's `stream_model` future must be polled on the test's
/// runtime, so the server cannot share it.
fn self_signed_https_endpoint() -> String {
    crate::test_support::ensure_fd_headroom();
    crate::http::install_tls_provider();

    let certificate = rustls::pki_types::CertificateDer::from(decode_fixture(SERVER_CERT_DER_B64));
    let key = rustls::pki_types::PrivateKeyDer::Pkcs8(rustls::pki_types::PrivatePkcs8KeyDer::from(
        decode_fixture(SERVER_KEY_DER_B64),
    ));
    let config = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(vec![certificate], key)
        .expect("server TLS configuration");
    let acceptor = tokio_rustls::TlsAcceptor::from(std::sync::Arc::new(config));

    let listener =
        std::net::TcpListener::bind("127.0.0.1:0").expect("bind the test HTTPS listener");
    listener
        .set_nonblocking(true)
        .expect("nonblocking listener");
    let port = listener.local_addr().expect("listener address").port();

    std::thread::spawn(move || {
        let Ok(runtime) = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        else {
            return;
        };
        // `from_std` registers the socket with the reactor, so it must run
        // inside the runtime it will be polled on.
        runtime.block_on(async move {
            let Ok(listener) = tokio::net::TcpListener::from_std(listener) else {
                return;
            };
            loop {
                let Ok((tcp, _)) = listener.accept().await else {
                    return;
                };
                let acceptor = acceptor.clone();
                tokio::spawn(async move {
                    // A handshake the client aborts (the verified case) is an
                    // ordinary outcome here, not a test failure.
                    let Ok(mut stream) = acceptor.accept(tcp).await else {
                        return;
                    };
                    // Drain the request head; the body is irrelevant to this
                    // test and the client half-closes nothing, so read until the
                    // blank line.
                    let mut request = Vec::new();
                    let mut buf = [0u8; 2048];
                    loop {
                        match stream.read(&mut buf).await {
                            Ok(0) | Err(_) => return,
                            Ok(read) => {
                                request.extend_from_slice(&buf[..read]);
                                if request.windows(4).any(|window| window == b"\r\n\r\n") {
                                    break;
                                }
                            }
                        }
                    }
                    let response = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{CHAT_SSE}",
                        CHAT_SSE.len()
                    );
                    let _ = stream.write_all(response.as_bytes()).await;
                    let _ = stream.flush().await;
                });
            }
        });
    });

    format!("https://127.0.0.1:{port}")
}

fn chat_target(base_url: &str, skip_tls_verify: bool) -> schema::ResolvedModelTarget {
    let mut target = protocol_target(base_url, ProtocolConfig::OpenAiChat(Default::default()));
    target.route.skip_tls_verify = skip_tls_verify;
    target
}

/// The client under test sends once and returns the first non-error event, or
/// the error it got instead. Bounded so a hung connection fails the test rather
/// than the suite.
async fn first_outcome(
    base_url: &str,
    skip_tls_verify: bool,
) -> Result<Option<ModelStreamEvent>, String> {
    let client = Client::from_target(chat_target(base_url, skip_tls_verify));
    let mut stream = match tokio::time::timeout(
        Duration::from_secs(20),
        client.stream_model(canonical_request()),
    )
    .await
    {
        Ok(Ok(stream)) => stream,
        Ok(Err(error)) => return Err(format!("{error:#}")),
        Err(_) => return Err("stream_model timed out".to_string()),
    };
    match tokio::time::timeout(Duration::from_secs(20), stream.next()).await {
        Ok(Some(ModelStreamEvent::Error { message })) => Err(message),
        Ok(other) => Ok(other),
        Err(_) => Err("no stream event within the deadline".to_string()),
    }
}

/// The feature, end to end: the *same* self-signed endpoint is unreachable with
/// verification on and answers with it off.
///
/// Both halves matter. Only the success half would pass if the flag silently
/// disabled TLS altogether (a plain-HTTP fallback, say), and only the failure
/// half would pass if the flag did nothing. Together they pin the behaviour:
/// the handshake still happens, it is just not verified.
#[tokio::test]
async fn a_self_signed_endpoint_fails_verification_and_succeeds_when_skipped() {
    let base_url = self_signed_https_endpoint();

    let verified = first_outcome(&base_url, false).await;
    let error = verified.expect_err("verification must reject a self-signed certificate");
    assert!(
        error.contains("certificate") || error.contains("CERTIFICATE") || error.contains("tls"),
        "expected a certificate/verification failure, got: {error}"
    );

    let skipped = first_outcome(&base_url, true)
        .await
        .expect("the request must reach the endpoint when verification is skipped");
    assert!(
        matches!(
            skipped,
            Some(ModelStreamEvent::TextDelta { .. })
                | Some(ModelStreamEvent::TextStart { .. })
                | Some(ModelStreamEvent::Finish { .. })
        ),
        "expected a streamed answer from the endpoint, got: {skipped:?}"
    );
}

/// The decision is read from each *route*, not latched onto the client: an
/// unverified request must not make the next verified one succeed.
///
/// This is what catches the tempting shortcut of flipping a process-wide or
/// per-client "accept invalid certs" switch the first time a provider asks for
/// it — the flag would then leak into every other provider the same client
/// serves.
#[tokio::test]
async fn an_unverified_request_does_not_relax_the_next_verified_one() {
    let base_url = self_signed_https_endpoint();

    // The unverified request goes first, so any latch would already be set by
    // the time the verified one runs.
    let skipped = first_outcome(&base_url, true)
        .await
        .expect("the unverified request must reach the endpoint");
    assert!(
        !matches!(skipped, Some(ModelStreamEvent::Error { .. })),
        "expected an answer, got: {skipped:?}"
    );

    let verified = first_outcome(&base_url, false).await;
    let error =
        verified.expect_err("a verified target must still reject the same self-signed endpoint");
    assert!(
        error.contains("certificate") || error.contains("CERTIFICATE") || error.contains("tls"),
        "expected a certificate/verification failure, got: {error}"
    );
}
