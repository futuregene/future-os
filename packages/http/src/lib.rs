//! HTTP clients with the process TLS provider initialized before construction.
//!
//! Reqwest 0.13's `rustls-no-provider` requires explicit initialization, even
//! when Ring is the only compiled provider. Keep this at client construction
//! so library consumers and standalone tests do not depend on entry-point order.

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

#[cfg(feature = "blocking")]
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
