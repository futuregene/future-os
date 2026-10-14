//! Service inputs captured after the host has selected its process environment.

use anyhow::{Context, Result};
use std::path::PathBuf;

/// Agent settings that do not belong to argument parsing or process control.
///
/// The data root is captured from the host environment. Until path injection is
/// complete, a host must keep that environment immutable for the service's
/// lifetime; this API never changes HOME, FUTURE_HOME or the current directory.
#[derive(Clone, Debug)]
pub struct AgentConfig {
    pub verbose: bool,
    pub grpc_addr: Option<String>,
    pub(crate) future_home: PathBuf,
}

impl Default for AgentConfig {
    fn default() -> Self {
        Self {
            verbose: false,
            grpc_addr: None,
            future_home: crate::utils::future_home(),
        }
    }
}

impl AgentConfig {
    pub fn future_home(&self) -> &std::path::Path {
        &self.future_home
    }

    pub(super) fn validate_environment(&self) -> Result<()> {
        let current = crate::utils::future_home();
        if current != self.future_home {
            anyhow::bail!(
                "FutureOS home changed after Agent configuration: {} -> {}",
                self.future_home.display(),
                current.display()
            );
        }
        Ok(())
    }
}

pub(crate) fn parse_tcp_bind(addr: &str) -> Result<(String, u16)> {
    let address = if addr.starts_with(':') && !addr[1..].contains(':') {
        format!("127.0.0.1{addr}")
    } else if !addr.contains(':') {
        format!("127.0.0.1:{addr}")
    } else {
        addr.to_owned()
    };
    let socket: std::net::SocketAddr = address.parse().with_context(|| {
        format!("invalid --grpc-addr {addr:?}: expected IP:port (bracket IPv6 addresses)")
    })?;
    Ok((socket.ip().to_string(), socket.port()))
}
