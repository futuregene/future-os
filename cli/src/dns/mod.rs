//! The CLI's side of hostname resolution.
//!
//! The resolver itself lives in `future_agent::http::dns`: it is attached to
//! the shared HTTP client builder, so the Agent, the host and the CLI all
//! resolve the same way. What stays here is the part that is not `reqwest` —
//! `tonic`'s connector for a TCP Agent endpoint — plus the names the CLI's call
//! sites already use.

use std::sync::Arc;

pub use future_agent::http::dns::{describe, preflight, resolve};
pub use future_rpc::transport::{ResolveFuture, TcpNameResolver};

/// [`future_agent::http::client_builder`], named for the CLI's call sites.
pub fn client_builder() -> reqwest::ClientBuilder {
    future_agent::http::client_builder()
}

/// [`future_agent::http::client`], named for the CLI's call sites.
pub fn client() -> reqwest::Client {
    future_agent::http::client()
}

/// The resolver for TCP gRPC endpoints, or `None` when the system's own
/// resolver is in use.
pub fn tcp_resolver() -> Option<TcpNameResolver> {
    if !future_agent::http::dns::is_custom() {
        return None;
    }
    Some(Arc::new(|host: String, port: u16| -> ResolveFuture {
        Box::pin(async move { resolve(&host, port).await })
    }))
}
