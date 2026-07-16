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
