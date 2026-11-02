//! Minimal file/console integration driver, with no application, database or GUI.
//! cargo run -p future-logging --example capture -- /tmp/future-log-capture 0.0.2+test
use future_logging::{Component, Profile};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let root = std::path::PathBuf::from(args.next().ok_or("missing output directory")?);
    let version = args.next().ok_or("missing distribution version")?;
    future_logging::prepare(Component::Desktop, &version, false, false);
    let desktop = future_logging::open(Component::Desktop, &Component::Desktop.path(&root), 0)?;
    let agent = future_logging::open(Component::Agent, &Component::Agent.path(&root), 0)?;
    tracing::info!(target: "futureos_lib::lifecycle", event = "desktop_ready");
    tracing::debug!(target: "futureos_lib::commands::history_transfer", event = "history_transfer_created", total_bytes = 65536);
    tracing::info!(target: "future_agent::service", event = "agent_ready");
    tracing::debug!(target: "future_agent::session", event = "history_page_read_completed", entries = 10);
    tracing::warn!(target: "future_agent::session", event = "storage_fixture_warning");
    tracing::info!(target: "h2", event = "transport_chatter_should_be_filtered");
    drop(agent);
    drop(desktop);
    println!("{:?} {}", Profile::for_version(&version), root.display());
    Ok(())
}
