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
    /// Apply the saved continuation policy with authoritative outcome checks.
    pub resume_interrupted_on_start: bool,
    pub(crate) future_home: PathBuf,
}

impl Default for AgentConfig {
    fn default() -> Self {
        Self {
            verbose: false,
            grpc_addr: None,
            resume_interrupted_on_start: true,
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

#[cfg(test)]
mod tests {
    use super::*;

    /// HOME is captured once, at configuration time, and the service refuses to
    /// run if it moved afterwards: a later `HOME` change would make every path
    /// this instance reports disagree with where its data actually is.
    #[test]
    fn validate_environment_rejects_a_home_that_changed_after_configuration() {
        // Captured against the ambient HOME, before the isolated one below.
        let configured = AgentConfig::default();
        let _isolated = crate::test_support::TestHome::new();

        let error = configured
            .validate_environment()
            .expect_err("a moved FutureOS home must be refused");
        let message = error.to_string();
        assert!(
            message.contains("FutureOS home changed"),
            "the refusal names the problem: {message}"
        );

        // A config captured against the *current* home validates.
        let matching = AgentConfig::default();
        matching
            .validate_environment()
            .expect("an unchanged home validates");
        assert_eq!(matching.future_home(), crate::utils::future_home());
    }
}
