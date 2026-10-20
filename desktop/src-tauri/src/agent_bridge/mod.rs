//! Desktop entry adapters. All Agent application services live in future-host.
mod client;
#[cfg(feature = "gui")]
pub(crate) use client::get_agent_readiness;
pub(crate) use client::install_channel_runtime;
#[cfg(all(feature = "gui", not(test)))]
pub(crate) use client::raw_agent_addr;
pub use future_host::agent_bridge::*;
#[cfg(feature = "gui")]
mod presentation;
#[cfg(test)]
pub(crate) mod test_support;
#[cfg(feature = "gui")]
pub(crate) use presentation::forward_settings_event;
#[cfg(test)]
mod presence;
#[cfg(test)]
mod tests;
