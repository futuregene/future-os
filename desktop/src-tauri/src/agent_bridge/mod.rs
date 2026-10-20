//! Desktop entry adapters. All Agent application services live in future-host.
mod client;
pub use client::connect_agent;
#[cfg(test)]
pub(crate) use client::reset_test_connection;
pub(crate) use client::{install_channel_runtime, raw_agent_addr};
pub use future_host::agent_bridge::*;
#[cfg(test)]
pub(crate) mod test_support;
#[cfg(test)]
pub(crate) use test_support::get_state_payload;
#[cfg(feature = "gui")]
mod presentation;
#[cfg(feature = "gui")]
pub(crate) use presentation::forward_settings_event;
#[cfg(test)]
mod presence;
#[cfg(test)]
mod tests;
