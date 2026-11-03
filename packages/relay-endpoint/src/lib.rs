//! Handing `async-nats` an address instead of a host name.
//!
//! `async-nats` resolves inside itself — `ToServerAddrs::socket_addrs()` is
//! `tokio::net::lookup_host` — and `ConnectOptions` has no resolver field, so on
//! a static musl build with no `/etc/resolv.conf` the relay is the one leg this
//! process cannot resolve even when every HTTP client can (`future_http::dns`).
//! Rather than persuade the library, resolve the name here and connect to the
//! address: the URL then carries the address, and certificate verification is
//! pinned to the name we actually asked for — because the URL's host is an
//! address at that point, and rustls would otherwise verify the relay's
//! certificate against it.
//!
//! Both relay clients use this: the client role (`future-remote-client`) and the
//! host role that serves a paired desktop (`future-remote-server`).
//!
//! Measured 2026-10-31 against the platform relay: a handshake with **no SNI**
//! (the address form sends none — RFC 6066 forbids a literal address in SNI) is
//! accepted and presents the `*.future-os.cn` certificate, which is what makes
//! this work at all.

use std::net::IpAddr;
use std::path::Path;
use std::sync::Arc;

use async_nats::rustls::client::danger::{
    HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier,
};
use async_nats::rustls::client::WebPkiServerVerifier;
use async_nats::rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use async_nats::rustls::{
    ClientConfig, DigitallySignedStruct, Error, RootCertStore, SignatureScheme,
};
use base64::Engine as _;

/// What to hand `ConnectOptions`: the URL to connect to, and the TLS
/// configuration that goes with it (only when the endpoint was rewritten).
#[derive(Debug)]
pub struct Endpoint {
    pub url: String,
    pub tls: Option<ClientConfig>,
}

/// Why an endpoint could not be prepared.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EndpointError {
    pub message: String,
    /// `true` for a failure the caller may retry (a lookup that did not answer);
    /// `false` for a configuration mistake, which no retry repairs.
    pub transient: bool,
}

impl EndpointError {
    fn permanent(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            transient: false,
        }
    }

    fn transient(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            transient: true,
        }
    }
}

impl std::fmt::Display for EndpointError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for EndpointError {}

/// Resolve the relay's host and return an address-bearing URL plus a TLS
/// configuration pinned to the original name.
///
/// Leaves the endpoint untouched when this process resolves through the system
/// (nothing to improve on, and a rewritten URL would change the certificate
/// name for no reason), when the URL already carries an address, or when the
/// relay is plaintext.
pub async fn for_connect(
    url: &str,
    roots: Option<&Path>,
    plaintext: bool,
) -> Result<Endpoint, EndpointError> {
    let unchanged = |url: &str| Endpoint {
        url: url.to_string(),
        tls: None,
    };
    if !future_http::dns::is_custom() {
        announce("system", url, None);
        return Ok(unchanged(url));
    }
    let parsed = url::Url::parse(url)
        .map_err(|error| EndpointError::permanent(format!("invalid relay URL: {error}")))?;
    let Some(host) = parsed.host_str().map(str::to_owned) else {
        announce("custom", url, None);
        return Ok(unchanged(url));
    };
    if host.parse::<IpAddr>().is_ok() {
        announce("custom", url, None);
        return Ok(unchanged(url));
    }
    let port = parsed.port().unwrap_or(4222);
    let addresses = future_http::dns::resolve(&host, port)
        .await
        .map_err(|error| {
            announce("custom", url, Some(&error.to_string()));
            EndpointError::transient(format!("Failed to resolve the relay {host:?}: {error}"))
        })?;
    let Some(address) = addresses.first().copied() else {
        announce("custom", url, Some("resolved to no address"));
        return Err(EndpointError::transient(format!(
            "the relay {host:?} resolved to no address"
        )));
    };
    let Some(rewritten) = address_url(url, address.ip()) else {
        announce("custom", url, None);
        return Ok(unchanged(url));
    };
    announce(
        "custom",
        url,
        Some(&format!("{address} (certificate pinned to {host})")),
    );
    let tls = if plaintext {
        None
    } else {
        Some(pinned_config(&host, roots)?)
    };
    Ok(Endpoint {
        url: rewritten,
        tls,
    })
}

/// Name the endpoint decision once per process, on stderr — the same reason the
/// resolver itself does (`[dns] plan=…`): the question that ends these
/// investigations is always "which address did this process actually dial".
fn announce(plan: &str, url: &str, note: Option<&str>) {
    use std::sync::OnceLock;
    static ANNOUNCED: OnceLock<()> = OnceLock::new();
    if ANNOUNCED.set(()).is_err() {
        return;
    }
    match note {
        Some(note) => eprintln!("[relay] plan={plan} url={url} -> {note}"),
        None => eprintln!("[relay] plan={plan} url={url} (unchanged)"),
    }
}

/// The same URL with its host replaced by `address`, keeping the scheme, port
/// and userinfo. `None` when `url` does not parse or has no host.
fn address_url(url: &str, address: IpAddr) -> Option<String> {
    let mut parsed = url::Url::parse(url).ok()?;
    parsed.host_str()?;
    // `Url::set_host` parses its argument like a URL host, where an IPv6
    // literal has to carry its brackets (without them the last group reads as
    // a port: `2001:db8::1` became `2001:4222`).
    let host = match address {
        IpAddr::V4(v4) => v4.to_string(),
        IpAddr::V6(v6) => format!("[{v6}]"),
    };
    parsed.set_host(Some(&host)).ok()?;
    Some(parsed.to_string())
}

/// A client configuration whose certificate verification is pinned to `host`.
///
/// The roots follow `async-nats`' own rule: the relay bundle when one was
/// supplied, the machine's store otherwise.
fn pinned_config(host: &str, roots: Option<&Path>) -> Result<ClientConfig, EndpointError> {
    // The verifier needs a crypto provider, and this path can run before any
    // HTTP client in the process was built.
    future_http::install_tls_provider();

    let mut store = RootCertStore::empty();
    match roots {
        Some(path) => {
            for certificate in pem_certificates(path)? {
                store.add(certificate).map_err(|error| {
                    EndpointError::permanent(format!("relay root certificate: {error}"))
                })?;
            }
        }
        None => {
            for certificate in rustls_native_certs::load_native_certs().certs {
                let _ = store.add(certificate);
            }
        }
    }
    let verifier = WebPkiServerVerifier::builder(Arc::new(store.clone()))
        .build()
        .map_err(|error| EndpointError::permanent(format!("relay TLS verifier: {error}")))?;
    let name = ServerName::try_from(host.to_owned())
        .map_err(|error| EndpointError::permanent(format!("relay host name: {error}")))?;
    let mut config = ClientConfig::builder()
        .with_root_certificates(store)
        .with_no_client_auth();
    config
        .dangerous()
        .set_certificate_verifier(Arc::new(PinnedName {
            inner: verifier,
            name,
        }));
    Ok(config)
}

/// Verifies the chain against the pinned name instead of the name the
/// connection carries.
struct PinnedName {
    inner: Arc<dyn ServerCertVerifier>,
    name: ServerName<'static>,
}

impl std::fmt::Debug for PinnedName {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PinnedName")
            .field("name", &self.name)
            .finish()
    }
}

impl ServerCertVerifier for PinnedName {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        ocsp_response: &[u8],
        now: UnixTime,
    ) -> Result<ServerCertVerified, Error> {
        self.inner
            .verify_server_cert(end_entity, intermediates, &self.name, ocsp_response, now)
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        certificate: &CertificateDer<'_>,
        signature: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, Error> {
        self.inner
            .verify_tls12_signature(message, certificate, signature)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        certificate: &CertificateDer<'_>,
        signature: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, Error> {
        self.inner
            .verify_tls13_signature(message, certificate, signature)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.inner.supported_verify_schemes()
    }
}

/// Every certificate in a PEM file.
fn pem_certificates(path: &Path) -> Result<Vec<CertificateDer<'static>>, EndpointError> {
    let text = std::fs::read_to_string(path).map_err(|error| {
        EndpointError::permanent(format!("read relay root certificates {path:?}: {error}"))
    })?;
    let mut certificates = Vec::new();
    for block in text.split("-----BEGIN CERTIFICATE-----").skip(1) {
        let Some((body, _)) = block.split_once("-----END CERTIFICATE-----") else {
            continue;
        };
        let compact: String = body.split_whitespace().collect();
        let der = base64::engine::general_purpose::STANDARD
            .decode(compact.as_bytes())
            .map_err(|error| {
                EndpointError::permanent(format!("relay root certificate is not base64: {error}"))
            })?;
        certificates.push(CertificateDer::from(der));
    }
    if certificates.is_empty() {
        return Err(EndpointError::permanent(format!(
            "no certificates in the relay root bundle {path:?}"
        )));
    }
    Ok(certificates)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_address_url_keeps_scheme_port_and_the_name_it_replaced_is_gone() {
        for (url, address, port) in [
            ("tls://www.future-os.cn:4222", "198.51.100.7", 4222u16),
            ("tls://www.future-os.cn:8443", "198.51.100.7", 8443),
            ("tls://www.future-os.cn:4222", "2001:db8::1", 4222),
        ] {
            let rewritten = address_url(url, address.parse().unwrap()).unwrap();
            let parsed = url::Url::parse(&rewritten).unwrap();
            assert_eq!(parsed.scheme(), "tls", "{rewritten}");
            let host = parsed
                .host_str()
                .unwrap()
                .trim_start_matches('[')
                .trim_end_matches(']');
            assert_eq!(host, address, "{rewritten}");
            assert_eq!(parsed.port(), Some(port), "{rewritten}");
            assert!(!rewritten.contains("future-os.cn"), "{rewritten}");
        }
    }

    /// The whole point of the wrapper: verification is asked about the name we
    /// resolved, not the address the connection carries.
    #[test]
    fn the_pinned_verifier_checks_the_name_we_asked_for() {
        let seen = Arc::new(std::sync::Mutex::new(None));
        let recorder = Arc::new(RecordName { seen: seen.clone() });
        let pinned = PinnedName {
            inner: recorder,
            name: ServerName::try_from("www.future-os.cn").unwrap(),
        };
        let now = UnixTime::since_unix_epoch(std::time::Duration::ZERO);
        pinned
            .verify_server_cert(
                &CertificateDer::from(vec![0u8]),
                &[],
                &ServerName::try_from("198.51.100.7").unwrap(),
                &[],
                now,
            )
            .unwrap();
        assert_eq!(
            seen.lock().unwrap().as_deref(),
            Some("www.future-os.cn"),
            "the verifier must check the name we resolved, not the address"
        );
    }

    /// A PEM bundle is read the way the platform writes it, and an empty one is
    /// an error rather than a silently empty root store.
    #[test]
    fn a_pem_bundle_is_read_or_reported() {
        let directory = tempfile::tempdir().unwrap();
        let bundle = directory.path().join("roots.pem");
        std::fs::write(
            &bundle,
            "-----BEGIN CERTIFICATE-----\naGVsbG8=\n-----END CERTIFICATE-----\n",
        )
        .unwrap();
        assert_eq!(pem_certificates(&bundle).unwrap().len(), 1);
        std::fs::write(&bundle, "nothing here\n").unwrap();
        assert!(pem_certificates(&bundle).is_err());
    }

    struct RecordName {
        seen: Arc<std::sync::Mutex<Option<String>>>,
    }

    impl std::fmt::Debug for RecordName {
        fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            formatter.write_str("RecordName")
        }
    }

    impl ServerCertVerifier for RecordName {
        fn verify_server_cert(
            &self,
            _end_entity: &CertificateDer<'_>,
            _intermediates: &[CertificateDer<'_>],
            server_name: &ServerName<'_>,
            _ocsp_response: &[u8],
            _now: UnixTime,
        ) -> Result<ServerCertVerified, Error> {
            *self.seen.lock().unwrap() = Some(server_name.to_str().to_string());
            Ok(ServerCertVerified::assertion())
        }

        fn verify_tls12_signature(
            &self,
            _message: &[u8],
            _certificate: &CertificateDer<'_>,
            _signature: &DigitallySignedStruct,
        ) -> Result<HandshakeSignatureValid, Error> {
            Err(Error::General("unused".into()))
        }

        fn verify_tls13_signature(
            &self,
            _message: &[u8],
            _certificate: &CertificateDer<'_>,
            _signature: &DigitallySignedStruct,
        ) -> Result<HandshakeSignatureValid, Error> {
            Err(Error::General("unused".into()))
        }

        fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
            Vec::new()
        }
    }
}
