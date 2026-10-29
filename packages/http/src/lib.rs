//! HTTP clients with the process TLS provider initialized before construction.
//!
//! Reqwest 0.13's `rustls-no-provider` requires explicit initialization, even
//! when Ring is the only compiled provider. Keep this at client construction
//! so library consumers and standalone tests do not depend on entry-point order.
//!
//! This lives in its own crate because the provider choice has to be made once
//! for the whole tree: the Agent needs it, the application host needs it, and
//! the TUI's update check needs it without linking either of them. A caller
//! that installed the provider by hand would be re-deciding Ring versus another
//! backend, and the two decisions would drift apart silently.

/// Install Ring unless an entry point already selected a process provider.
pub fn install_tls_provider() {
    let _ = rustls::crypto::ring::default_provider().install_default();
}

/// Start an HTTP client builder. Callers retain their own trust, proxy,
/// authentication and timeout settings.
pub fn client_builder() -> reqwest::ClientBuilder {
    install_tls_provider();
    reqwest::Client::builder()
}

/// Construct a client with Reqwest's default settings.
pub fn client() -> reqwest::Client {
    client_builder().build().expect("initialize HTTP client")
}

/// Send a one-off GET with the same TLS initialization as a reusable client.
pub async fn get(url: impl reqwest::IntoUrl) -> reqwest::Result<reqwest::Response> {
    client().get(url).send().await
}

pub mod blocking {
    /// Start a blocking HTTP client builder with the process provider ready.
    pub fn client_builder() -> reqwest::blocking::ClientBuilder {
        super::install_tls_provider();
        reqwest::blocking::Client::builder()
    }

    /// Construct a blocking client with Reqwest's default settings.
    pub fn client() -> reqwest::blocking::Client {
        client_builder()
            .build()
            .expect("initialize blocking HTTP client")
    }
}
