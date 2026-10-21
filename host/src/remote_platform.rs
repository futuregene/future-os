//! Shared host-side HTTP adaptation for both Remote roles and relay control.
//! Application TLS setup and error presentation stay outside the role libraries.
use serde_json::Value;
const REQUEST_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(20);

/// The HTTP client every Remote role and relay-control call uses.
///
/// A self-hosted relay commonly serves its API from a certificate issued by a
/// CA the platform store does not know (`--relay-ca` / `FUTURE_RELAY_CA` names
/// it). That is the same operator-supplied trust addition the NATS connector
/// makes in `remote::trust`: it lands here too, or the very first call —
/// registering the room, claiming the invitation — dies in the handshake while
/// the NATS connection would have worked.
pub(crate) fn http_client() -> Result<reqwest::Client, crate::Error> {
    crate::install_rustls_provider();
    let mut builder = reqwest::Client::builder().timeout(REQUEST_TIMEOUT);
    for certificate in relay_trust_anchors()? {
        // `add_root_certificate` *adds* to the verified set (rustls-platform-
        // verifier's `new_with_extra_roots`): verification stays on, and a
        // certificate that does not chain to this anchor gains nothing.
        builder = builder.add_root_certificate(certificate);
    }
    builder
        .build()
        .map_err(|error| crate::Error::Message(format!("Failed to create HTTP client: {error}")))
}

/// The extra root certificates for this launch's relay, or none when the
/// operator configured the platform (or a public CA) and needs no addition.
///
/// A file that cannot be read is an error rather than a silent fallback: the
/// alternative is a handshake failure several layers down that says nothing
/// about the `--relay-ca` path that caused it.
fn relay_trust_anchors() -> Result<Vec<reqwest::Certificate>, crate::Error> {
    let Some(path) = crate::remote::relay::ca_path() else {
        return Ok(Vec::new());
    };
    let pem = std::fs::read(&path).map_err(|error| {
        crate::Error::Message(format!(
            "Cannot read the relay CA {}: {error}",
            path.display()
        ))
    })?;
    // `from_pem_bundle`, not `from_pem`: the single-certificate form keeps the
    // bytes for the rustls backend to check later, so a bundle holding several
    // roots (a CA chain) would be truncated to its first.
    let certificates = reqwest::Certificate::from_pem_bundle(&pem).map_err(|error| {
        crate::Error::Message(format!(
            "{} is not a PEM certificate: {error}",
            path.display()
        ))
    })?;
    if certificates.is_empty() {
        return Err(crate::Error::Message(format!(
            "{} holds no certificate.",
            path.display()
        )));
    }
    Ok(certificates)
}

/// Build the error for a failed `.send()`: a [`crate::Error::RemoteTransport`] when no HTTP
/// response was possible (so the UI can say "check your network"), a plain
/// [`crate::Error::Message`] otherwise. The original reqwest detail is preserved in both for
/// logs.
pub(crate) fn transport_or_message(action: &str, error: reqwest::Error) -> crate::Error {
    let message = format!("Failed to {action}: {error}");
    if is_transport_error(&error) {
        crate::Error::RemoteTransport(message)
    } else {
        crate::Error::Message(message)
    }
}

/// A send failure that never produced an HTTP response — i.e. the network path
/// itself failed (offline, DNS, connection refused, timeout). Status / body /
/// decode errors mean we *did* reach the server, so they aren't transport
/// failures.
pub(crate) fn is_transport_error(error: &reqwest::Error) -> bool {
    error.is_connect() || error.is_timeout() || error.is_request() || error.is_redirect()
}

pub(crate) async fn parse_response<T: serde::de::DeserializeOwned>(
    response: reqwest::Response,
    action: &str,
) -> Result<T, crate::Error> {
    if !response.status().is_success() {
        return Err(response_error(response, action).await);
    }
    response.json::<T>().await.map_err(|error| {
        crate::Error::Message(format!("Failed to parse {action} response: {error}"))
    })
}

pub(crate) async fn response_error(response: reqwest::Response, action: &str) -> crate::Error {
    let status = response.status();
    let body = response.json::<Value>().await.ok();
    // The platform error body is `{error: <machine code>, message: <human text>}`.
    let code = body
        .as_ref()
        .and_then(|value| value.get("error"))
        .and_then(Value::as_str)
        .map(str::to_string)
        .filter(|code| !code.trim().is_empty());
    let message = body
        .as_ref()
        .and_then(|value| value.get("message"))
        .and_then(Value::as_str)
        .filter(|message| !message.trim().is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| format!("Failed to {action} (HTTP {})", status.as_u16()));
    // Error bodies can reflect credentials. Keep logs to the action/status;
    // the typed UI error still retains the platform code and message.
    eprintln!("remote: {action} failed: HTTP {}", status.as_u16());
    crate::Error::Remote {
        status: status.as_u16(),
        code,
        message,
    }
}

pub(crate) fn jwt_expiry(jwt: &str) -> Result<i64, crate::Error> {
    future_remote_protocol::jwt::expiry(jwt)
        .map_err(|error| crate::Error::Message(error.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    /// A private CA, in the PEM form `--relay-ca` / `FUTURE_RELAY_CA` documents.
    const CA_PEM: &str = "-----BEGIN CERTIFICATE-----
MIIDFTCCAf2gAwIBAgIUTEvO33LtadHeNu1yxgdZj9T1iHQwDQYJKoZIhvcNAQEL
BQAwITEfMB0GA1UEAwwWRnV0dXJlT1MgVGVzdCBSZWxheSBDQTAeFw0yNjEwMjEw
MzEzMjJaFw0zNjEwMTgwMzEzMjJaMCExHzAdBgNVBAMMFkZ1dHVyZU9TIFRlc3Qg
UmVsYXkgQ0EwggEiMA0GCSqGSIb3DQEBAQUAA4IBDwAwggEKAoIBAQCqvmA/0r5n
+ZDGd49z3mi2Z+tyR9Bj58SNKlfqcZ2Np7BG7iSQs4S1ZyG9KpfcSNqAvJM3gQA6
Htvw+yGq8t5e8JcE3Y+MR9IfIvcgTSnPMLKqEWWVxjqHTLOJfPq78mrp3ZB55N7V
PDj9qo4zULzoaVtHYvP75GtKf/JltdQqII5Q3svtJmRCSekfUxq0fGrH8UIQcgev
gRXLvIYfXpSoqDHPZ24b56KAgGyBhQjtOrLwJVdW511zNuzjMLFvvLUrBch3NKCM
EdQf1u5RT8RVN/td/kgWeCBmjo1yk0JKGUwyc5gpUR/wwEOqgPlHars3aqfj/aXl
yrR+nqOEkB1FAgMBAAGjRTBDMBIGA1UdEwEB/wQIMAYBAf8CAQAwDgYDVR0PAQH/
BAQDAgEGMB0GA1UdDgQWBBRx6l42EjPEgKioptlJmx9qU9GTpDANBgkqhkiG9w0B
AQsFAAOCAQEAOmBLvAGCm7FFlatfK/ExkrRwjskq/vDRz0jX+TXpNHxP07qCyMEl
E8MjBumRldrMu275KOYaHYNPrvw4g3nh7CjUMuNex2ZOgh4zJfVXzbBRAXAsRpB3
TR43qAAWKEdTIxm9g5wf24/Erxff0H7i5oqSMMje9qNOKJo5A8gOMVJD6hNUBbK+
w0zmAljSyw1h5s0DcdUtLoijhid6e5kX4IIHtaXN+ks6NRcVEYbIbGBotGdzdLkS
SXsquBplkYC/VEidMjL484xKpigZ+/QU4qPacaL8AhMlz1Ezsl2rO9VT1vBF0Tlk
H1m4S5azuWY05DlH2ERn+waDBeA5WNt+UQ==
-----END CERTIFICATE-----
";

    /// The leaf that CA issued for `127.0.0.1` / `localhost`, and its key, as
    /// base64 DER. Embedded so the test needs no certificate tooling and gets
    /// the same bytes on every machine; the key belongs to a listener that
    /// exists only inside this process.
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

    const BODY: &str = r#"{"ok":true}"#;

    fn decode_fixture(text: &str) -> Vec<u8> {
        let compact: String = text.chars().filter(|c| !c.is_whitespace()).collect();
        base64::engine::general_purpose::STANDARD
            .decode(compact)
            .expect("fixture DER")
    }

    /// A TLS listener on loopback that answers every request with [`BODY`].
    async fn serve_tls(acceptor: tokio_rustls::TlsAcceptor) -> (u16, tokio::task::JoinHandle<()>) {
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .expect("bind the test HTTPS listener");
        let port = listener.local_addr().expect("listener address").port();
        let task = tokio::spawn(async move {
            loop {
                let Ok((tcp, _)) = listener.accept().await else {
                    return;
                };
                let acceptor = acceptor.clone();
                tokio::spawn(async move {
                    // A client that does not trust the CA fails here, which is
                    // exactly what the second half of the test asserts from its
                    // own side.
                    let Ok(mut tls) = acceptor.accept(tcp).await else {
                        return;
                    };
                    let mut request = [0u8; 2048];
                    let _ = tls.read(&mut request).await;
                    let response = format!(
                        "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\n\
                         content-length: {}\r\nconnection: close\r\n\r\n{BODY}",
                        BODY.len()
                    );
                    let _ = tls.write_all(response.as_bytes()).await;
                    let _ = tls.shutdown().await;
                });
            }
        });
        (port, task)
    }

    /// Every layer of a failure, so an assertion can name what actually went
    /// wrong instead of just "the request failed".
    fn error_chain(error: &dyn std::error::Error) -> String {
        let mut text = error.to_string();
        let mut source = error.source();
        while let Some(next) = source {
            text.push_str(" | ");
            text.push_str(&next.to_string());
            source = next.source();
        }
        text
    }

    /// The relay CA is really used, not merely accepted: one client reaches an
    /// HTTPS endpoint whose chain ends at a private root this platform does not
    /// know, and the same request fails on that certificate when the CA is not
    /// configured. Both directions are asserted, so a client that ignored the
    /// CA would fail the first half and one that disabled verification would
    /// fail the second.
    #[tokio::test]
    // The guard is a serialization token over a process-global environment
    // variable, not a lock over data another task reads; nothing can make
    // progress on `FUTURE_RELAY_CA` until this test releases it anyway.
    #[allow(clippy::await_holding_lock)]
    async fn the_relay_ca_is_trusted_and_without_it_the_handshake_fails() {
        // `FUTURE_RELAY_CA` is process-global, like HOME: serialize with the
        // other tests that swap it.
        let _lock = crate::TEST_HOME_LOCK
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        crate::install_rustls_provider();

        let certificate =
            rustls::pki_types::CertificateDer::from(decode_fixture(SERVER_CERT_DER_B64));
        let key = rustls::pki_types::PrivateKeyDer::Pkcs8(
            rustls::pki_types::PrivatePkcs8KeyDer::from(decode_fixture(SERVER_KEY_DER_B64)),
        );
        let config = rustls::ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(vec![certificate], key)
            .expect("server TLS configuration");
        let (port, server) =
            serve_tls(tokio_rustls::TlsAcceptor::from(std::sync::Arc::new(config))).await;
        let url = format!("https://127.0.0.1:{port}/v1/health");

        let dir = std::env::temp_dir().join(format!("futureos-relay-ca-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("create the temp directory");
        let ca_path = dir.join("ca.pem");
        std::fs::write(&ca_path, CA_PEM).expect("write the CA");

        let previous = std::env::var_os("FUTURE_RELAY_CA");
        std::env::set_var("FUTURE_RELAY_CA", &ca_path);

        let trusting = http_client().expect("an HTTP client with the relay CA");
        let response = trusting
            .get(&url)
            .send()
            .await
            .expect("the relay CA makes the handshake succeed");
        assert_eq!(response.status(), reqwest::StatusCode::OK);
        assert_eq!(response.text().await.expect("the body"), BODY);

        std::env::remove_var("FUTURE_RELAY_CA");
        let untrusting = http_client().expect("an HTTP client without a relay CA");
        let error = untrusting
            .get(&url)
            .send()
            .await
            .expect_err("a private root is not in the platform store");
        let chain = error_chain(&error);
        assert!(
            chain.to_lowercase().contains("certificate"),
            "expected a certificate failure, got: {chain}"
        );

        match previous {
            Some(value) => std::env::set_var("FUTURE_RELAY_CA", value),
            None => std::env::remove_var("FUTURE_RELAY_CA"),
        }
        server.abort();
        let _ = std::fs::remove_dir_all(&dir);
    }
}
