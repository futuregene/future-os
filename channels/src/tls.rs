//! TLS config helpers that use the platform's native certificate verifier
//! (macOS Security framework, Windows Schannel, etc.) instead of only the
//! Mozilla root store bundled with webpki-roots.  Feishu and DingTalk use
//! Chinese CA certificates that may not be in the Mozilla trust store.

use rustls_platform_verifier::BuilderVerifierExt;
use std::sync::Arc;

/// Build a `reqwest::Client` that trusts the platform certificate store.
pub fn http_client() -> reqwest::Client {
    http_client_builder()
        .build()
        .expect("build reqwest client with platform verifier")
}

/// Return a pre-configured `reqwest::ClientBuilder` with the platform
/// certificate verifier.  Callers can add timeouts, proxies, etc. before
/// calling `.build()`.
pub fn http_client_builder() -> reqwest::ClientBuilder {
    let tls = platform_tls_config();
    reqwest::Client::builder().use_preconfigured_tls(tls)
}

/// Build a `tokio_tungstenite::Connector` that trusts the platform
/// certificate store.  Pass to `connect_async_tls_with_config`.
pub fn ws_connector() -> tokio_tungstenite::Connector {
    let tls = Arc::new(platform_tls_config());
    tokio_tungstenite::Connector::Rustls(tls)
}

/// Create a `rustls::ClientConfig` that verifies certificates using the
/// platform's native trust store (macOS Security framework, Windows
/// Schannel, Android cert store, etc.).
fn platform_tls_config() -> rustls::ClientConfig {
    rustls::ClientConfig::builder()
        .with_platform_verifier()
        .with_no_client_auth()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Install the process-level rustls provider (production does this in
    /// `run()`); idempotent — a second install returns Err we ignore.
    fn ensure_provider() {
        let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
    }

    #[test]
    fn http_client_builds() {
        ensure_provider();
        let client = http_client();
        // A built client is usable for requests (no panic, correct type).
        let req = client.get("http://127.0.0.1/").build();
        assert!(req.is_ok());
    }

    #[test]
    fn http_client_builder_builds() {
        ensure_provider();
        let builder = http_client_builder();
        let client = builder.build().expect("build client");
        let req = client.post("http://127.0.0.1/").build();
        assert!(req.is_ok());
    }

    #[test]
    fn ws_connector_is_rustls() {
        ensure_provider();
        let connector = ws_connector();
        assert!(matches!(connector, tokio_tungstenite::Connector::Rustls(_)));
    }

    #[test]
    fn platform_tls_config_builds() {
        ensure_provider();
        // Directly exercises the platform verifier builder path.
        let _config = platform_tls_config();
    }
}
