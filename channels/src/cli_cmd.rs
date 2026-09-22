//! `future channel list | status | test | send`.
//!
//! Diagnostics have to work when things are broken, which means two rules:
//!
//! * **They never need the bridge to be running.** `status` reads the snapshot
//!   the bridge publishes; a missing or stale snapshot is itself the answer.
//! * **They never write the user's configuration.** `list` and `status` only
//!   read; `test` and `send` load configuration to build one channel and touch
//!   nothing else.

use anyhow::{anyhow, Result};
use serde_json::json;
use std::collections::BTreeMap;
use std::sync::Arc;

use crate::config::ChannelConfig;
use crate::delivery::{DeliveryQueue, DeliveryState, QueuedDelivery};
use crate::outbox::Outbox;
use crate::providers::{self, traits::Maturity};
use crate::status::{ChannelState, StatusBoard, StatusSnapshot};

/// Subcommands this module owns. `run` dispatches on these before starting the
/// bridge, so `future channel status` does not start listening.
pub const SUBCOMMANDS: &[&str] = &["list", "status", "test", "send"];

/// Whether `command` is a subcommand of this module rather than a bridge flag.
pub fn is_subcommand(command: &str) -> bool {
    SUBCOMMANDS.contains(&command)
}

/// Run one subcommand.
pub fn run(args: &[String]) -> Result<()> {
    let (command, rest) = args
        .split_first()
        .ok_or_else(|| anyhow!("missing subcommand"))?;
    match command.as_str() {
        "list" => list(rest),
        "status" => status(rest),
        "test" => test(rest),
        "send" => send(rest),
        other => Err(anyhow!(
            "unknown `future channel` subcommand `{other}`; expected one of {}",
            SUBCOMMANDS.join(", ")
        )),
    }
}

/// A tiny option reader: `--flag`, `--key value`, `--key=value`.
#[derive(Debug)]
struct Options {
    flags: Vec<String>,
    values: BTreeMap<String, String>,
    positional: Vec<String>,
}

impl Options {
    fn parse(args: &[String], valued: &[&str]) -> Result<Self> {
        let mut options = Options {
            flags: Vec::new(),
            values: BTreeMap::new(),
            positional: Vec::new(),
        };
        let mut index = 0;
        while index < args.len() {
            let argument = &args[index];
            match argument.strip_prefix("--") {
                Some(name) => {
                    if let Some((key, value)) = name.split_once('=') {
                        options.values.insert(key.to_string(), value.to_string());
                    } else if valued.contains(&name) {
                        let value = args
                            .get(index + 1)
                            .ok_or_else(|| anyhow!("--{name} needs a value"))?;
                        index += 1;
                        options.values.insert(name.to_string(), value.clone());
                    } else {
                        options.flags.push(name.to_string());
                    }
                }
                None => options.positional.push(argument.clone()),
            }
            index += 1;
        }
        Ok(options)
    }

    fn flag(&self, name: &str) -> bool {
        self.flags.iter().any(|flag| flag == name)
    }

    fn value(&self, name: &str) -> Option<&str> {
        self.values.get(name).map(String::as_str)
    }

    fn wants_json(&self) -> bool {
        self.flag("json") || self.value("format") == Some("json")
    }
}

/// Build the delivery plumbing from configuration without starting a bridge.
fn outbox() -> Result<Outbox> {
    let (config, note) = ChannelConfig::load_for_read();
    if let Some(note) = note {
        tracing::warn!("{note}");
    }
    let agent_cfg = Arc::new(config.agent.clone());
    let root = crate::data_root();
    let status = Arc::new(StatusBoard::new(root.join("status.json")));
    let queue = Arc::new(DeliveryQueue::load(DeliveryQueue::default_path()));
    if let Some(error) = queue.load_error() {
        return Err(anyhow!("{error}"));
    }
    Ok(Outbox::new(queue, config, agent_cfg, root, status))
}

/// How a channel is configured, independent of whether it is running.
fn configured_state(block: Option<&serde_json::Value>, implemented: bool) -> &'static str {
    match block {
        None => "not-configured",
        Some(block) if !ChannelConfig::provider_enabled(block) => "disabled",
        Some(_) if !implemented => "unsupported",
        Some(_) => "enabled",
    }
}

fn capability_summary(capabilities: &providers::Capabilities) -> String {
    let mut names = Vec::new();
    for (enabled, name) in [
        (capabilities.receive, "receive"),
        (capabilities.send, "send"),
        (capabilities.edit, "stream"),
        (capabilities.threads, "threads"),
        (capabilities.typing, "typing"),
        (capabilities.reactions, "reactions"),
        (capabilities.media_in, "media-in"),
        (capabilities.media_out, "media-out"),
    ] {
        if enabled {
            names.push(name);
        }
    }
    names.join(",")
}

fn list(args: &[String]) -> Result<()> {
    let options = Options::parse(args, &["format"])?;
    let (config, _note) = ChannelConfig::load_for_read();
    let rows: Vec<serde_json::Value> = providers::all_definitions()
        .into_iter()
        .map(|definition| {
            let block = config.provider_config(definition.id);
            json!({
                "id": definition.id,
                "displayName": definition.display_name,
                "maturity": definition.maturity.as_str(),
                "configured": configured_state(block.as_ref(), definition.is_implemented()),
                "bridge": if providers::registry::find(definition.id).is_some() { "shared" } else { "own" },
                "capabilities": capability_summary(&definition.capabilities),
                "maxTextLen": definition.max_text_len,
                "requires": definition.requires,
            })
        })
        .collect();

    if options.wants_json() {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({ "channels": rows }))?
        );
        return Ok(());
    }

    println!(
        "{:<12} {:<8} {:<15} {:<7} {:<16} CAPABILITIES",
        "CHANNEL", "MATURITY", "CONFIGURED", "BRIDGE", "REQUIRES"
    );
    for row in &rows {
        let requires = row["requires"]
            .as_array()
            .map(|items| {
                items
                    .iter()
                    .filter_map(|item| item.as_str())
                    .collect::<Vec<_>>()
                    .join("; ")
            })
            .unwrap_or_default();
        println!(
            "{:<12} {:<8} {:<15} {:<7} {:<16} {}",
            row["id"].as_str().unwrap_or(""),
            row["maturity"].as_str().unwrap_or(""),
            row["configured"].as_str().unwrap_or(""),
            row["bridge"].as_str().unwrap_or(""),
            truncate_for_column(&requires, 16),
            row["capabilities"].as_str().unwrap_or(""),
        );
    }
    println!();
    println!("MATURITY: live = verified against the platform, preview = built from the platform's public API, planned = not implemented in this build.");
    println!(
        "Configure under `providers.<id>` in {}.",
        crate::config::ChannelConfig::default_path().display()
    );
    println!(
        "Run `future channel test <channel>` to check credentials without starting the bridge."
    );
    Ok(())
}

fn truncate_for_column(text: &str, width: usize) -> String {
    if text.chars().count() <= width {
        return text.to_string();
    }
    let kept: String = text.chars().take(width.saturating_sub(1)).collect();
    format!("{kept}…")
}

fn status(args: &[String]) -> Result<()> {
    let options = Options::parse(args, &["format"])?;
    let (config, _note) = ChannelConfig::load_for_read();
    let snapshot = StatusSnapshot::load(&StatusSnapshot::default_path());
    let running = snapshot.is_fresh(std::time::Duration::from_secs(90));
    let queue = DeliveryQueue::load(DeliveryQueue::default_path());
    let pending = queue.pending();
    let failed: Vec<QueuedDelivery> = queue
        .all()
        .into_iter()
        .filter(|entry| entry.state == DeliveryState::Failed)
        .collect();

    let rows: Vec<serde_json::Value> = providers::all_definitions()
        .into_iter()
        .map(|definition| {
            let block = config.provider_config(definition.id);
            let live = snapshot.channels.get(definition.id);
            json!({
                "id": definition.id,
                "maturity": definition.maturity.as_str(),
                "configured": configured_state(block.as_ref(), definition.is_implemented()),
                "state": live.and_then(|entry| entry.state.clone()).map(|state| state.as_str().to_string()),
                "inbound": live.map(|entry| entry.inbound_count).unwrap_or(0),
                "outbound": live.map(|entry| entry.outbound_count).unwrap_or(0),
                "duplicates": live.map(|entry| entry.duplicate_count).unwrap_or(0),
                "dropped": live.map(|entry| entry.rejected_count).unwrap_or(0),
                "superseded": live.map(|entry| entry.superseded_count).unwrap_or(0),
                "lastError": live.and_then(|entry| entry.last_error.clone()),
            })
        })
        .collect();

    if options.wants_json() {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "bridgeRunning": running,
                "snapshotAgeSeconds": snapshot.updated_unix.map(|updated| crate::status::now_unix() - updated),
                "pid": snapshot.pid,
                "channels": rows,
                "outboundQueue": {
                    "pending": pending.len(),
                    "failed": failed.len(),
                },
            }))?
        );
        return Ok(());
    }

    if running {
        println!(
            "channel bridge: running (pid {})",
            snapshot
                .pid
                .map(|pid| pid.to_string())
                .unwrap_or_else(|| "unknown".into())
        );
    } else {
        match snapshot.updated_unix {
            Some(updated) => println!(
                "channel bridge: not running (last status {}s ago)",
                crate::status::now_unix() - updated
            ),
            None => println!("channel bridge: not running (no status has been published)"),
        }
    }
    println!();
    println!(
        "{:<12} {:<15} {:<9} {:>8} {:>8} {:>7} {:>7}  LAST ERROR",
        "CHANNEL", "CONFIGURED", "STATE", "IN", "OUT", "DUPE", "DROPPED"
    );
    for row in &rows {
        let state = row["state"].as_str().unwrap_or("-");
        println!(
            "{:<12} {:<15} {:<9} {:>8} {:>8} {:>7} {:>7}  {}",
            row["id"].as_str().unwrap_or(""),
            row["configured"].as_str().unwrap_or(""),
            state,
            row["inbound"],
            row["outbound"],
            row["duplicates"],
            row["dropped"],
            row["lastError"].as_str().unwrap_or(""),
        );
    }
    println!();
    println!(
        "outbound queue: {} pending, {} failed",
        pending.len(),
        failed.len()
    );
    if !pending.is_empty() {
        for entry in pending.iter().take(5) {
            println!(
                "  pending {} → {} ({}) after {} attempt(s)",
                entry.channel, entry.conversation.id, entry.id, entry.attempts
            );
        }
    }
    Ok(())
}

fn test(args: &[String]) -> Result<()> {
    let options = Options::parse(args, &["format"])?;
    let id = options
        .positional
        .first()
        .ok_or_else(|| anyhow!("usage: future channel test <channel>"))?
        .clone();
    let outbox = outbox()?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let summary = {
        let id = id.clone();
        runtime.block_on(async move {
            let (provider, ctx) = outbox.context(&id)?;
            provider.probe(&ctx).await
        })?
    };
    if options.wants_json() {
        println!(
            "{}",
            serde_json::to_string_pretty(
                &json!({ "channel": id, "ok": true, "summary": summary })
            )?
        );
    } else {
        println!("{id}: ok — {summary}");
    }
    Ok(())
}

fn send(args: &[String]) -> Result<()> {
    let options = Options::parse(args, &["channel", "to", "text", "thread", "format"])?;
    let channel = options
        .value("channel")
        .ok_or_else(|| anyhow!("--channel is required"))?
        .to_string();
    let target = options
        .value("to")
        .ok_or_else(|| anyhow!("--to is required"))?
        .to_string();
    let text = match options.value("text") {
        Some("-") => {
            use std::io::Read;
            let mut buffer = String::new();
            std::io::stdin().read_to_string(&mut buffer)?;
            buffer.trim_end().to_string()
        }
        Some(text) => text.to_string(),
        None => return Err(anyhow!("--text is required (use --text - to read stdin)")),
    };
    let conversation = crate::bridge::ConversationRef {
        id: target,
        thread_id: options.value("thread").map(str::to_string),
        kind: crate::bridge::ChatKind::Direct,
    };
    let durable = options.flag("durable");
    let outbox = outbox()?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;

    let (state, detail) = runtime.block_on(async {
        if durable {
            outbox
                .enqueue(&channel, conversation.clone(), &text)
                .await?;
            let entry = outbox
                .queue()
                .all()
                .into_iter()
                .find(|entry| entry.text == text && entry.channel == channel);
            Ok::<_, anyhow::Error>(match entry {
                Some(entry) => (
                    match entry.state {
                        DeliveryState::Sent => "sent",
                        DeliveryState::Failed => "failed",
                        DeliveryState::Pending => "queued",
                    },
                    entry.last_error,
                ),
                None => ("queued", None),
            })
        } else {
            outbox
                .deliver_now(&channel, &conversation, &text)
                .await
                .map(|()| ("sent", None))
        }
    })?;

    if options.wants_json() {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "channel": channel,
                "to": conversation.id,
                "durable": durable,
                "state": state,
                "error": detail,
            }))?
        );
    } else {
        match detail {
            Some(error) => println!("{channel}: {state} — {error}"),
            None => println!("{channel}: {state}"),
        }
    }
    if state == "failed" {
        anyhow::bail!("delivery failed permanently");
    }
    Ok(())
}

/// The channel states a snapshot can publish, for the `status` legend.
pub const STATE_LEGEND: &[ChannelState] = &[
    ChannelState::Disabled,
    ChannelState::Starting,
    ChannelState::Running,
    ChannelState::Error,
    ChannelState::Unsupported,
];

/// Maturities, for the `list` legend.
pub const MATURITY_LEGEND: &[Maturity] = &[Maturity::Live, Maturity::Preview, Maturity::Planned];

#[cfg(test)]
mod tests {
    use super::*;

    fn args(items: &[&str]) -> Vec<String> {
        items.iter().map(|item| item.to_string()).collect()
    }

    #[test]
    fn subcommands_are_dispatched_and_unknown_ones_are_rejected() {
        assert!(is_subcommand("list"));
        assert!(is_subcommand("send"));
        assert!(!is_subcommand("--version"));
        let error = run(&args(&["frobnicate"])).unwrap_err().to_string();
        assert!(error.contains("unknown"), "{error}");
        assert!(error.contains("list"), "{error}");
    }

    #[test]
    fn options_parse_flags_values_and_positional_arguments() {
        let options = Options::parse(
            &args(&["--durable", "--channel", "telegram", "--to=c1", "extra"]),
            &["channel", "to"],
        )
        .unwrap();
        assert!(options.flag("durable"));
        assert_eq!(options.value("channel"), Some("telegram"));
        assert_eq!(options.value("to"), Some("c1"));
        assert_eq!(options.positional, vec!["extra".to_string()]);
    }

    #[test]
    fn a_valued_option_without_a_value_is_an_error() {
        let error = Options::parse(&args(&["--channel"]), &["channel"])
            .unwrap_err()
            .to_string();
        assert!(error.contains("--channel needs a value"), "{error}");
    }

    #[test]
    fn json_can_be_requested_two_ways() {
        let json = Options::parse(&args(&["--json"]), &[]).unwrap();
        assert!(json.wants_json());
        let format = Options::parse(&args(&["--format", "json"]), &["format"]).unwrap();
        assert!(format.wants_json());
        let text = Options::parse(&args(&["--format", "text"]), &["format"]).unwrap();
        assert!(!text.wants_json());
    }

    #[test]
    fn configured_state_covers_every_combination() {
        assert_eq!(configured_state(None, true), "not-configured");
        assert_eq!(
            configured_state(Some(&json!({"enabled": false})), true),
            "disabled"
        );
        assert_eq!(
            configured_state(Some(&json!({"enabled": true})), true),
            "enabled"
        );
        assert_eq!(
            configured_state(Some(&json!({"enabled": true})), false),
            "unsupported"
        );
        // A missing `enabled` key means not enabled, not "enabled by default".
        assert_eq!(configured_state(Some(&json!({})), true), "disabled");
    }

    #[test]
    fn capability_summaries_name_what_the_channel_can_do() {
        let rich = capability_summary(&providers::Capabilities::RICH);
        for expected in ["receive", "send", "stream", "threads", "reactions"] {
            assert!(rich.contains(expected), "{rich}");
        }
        let text = capability_summary(&providers::Capabilities::TEXT);
        assert!(text.contains("receive"));
        assert!(!text.contains("stream"));
    }

    #[test]
    fn long_requirement_lists_are_truncated_in_the_table() {
        assert_eq!(truncate_for_column("short", 16), "short");
        let long = truncate_for_column("a very long requirement string", 10);
        assert_eq!(long.chars().count(), 10);
        assert!(long.ends_with('…'));
    }

    #[test]
    fn send_requires_its_arguments() {
        let error = send(&args(&["--to", "c1", "--text", "hi"]))
            .unwrap_err()
            .to_string();
        assert!(error.contains("--channel is required"), "{error}");
        let error = send(&args(&["--channel", "cli", "--text", "hi"]))
            .unwrap_err()
            .to_string();
        assert!(error.contains("--to is required"), "{error}");
        let error = send(&args(&["--channel", "cli", "--to", "c1"]))
            .unwrap_err()
            .to_string();
        assert!(error.contains("--text is required"), "{error}");
    }

    #[test]
    fn test_requires_a_channel_name() {
        let error = test(&args(&[])).unwrap_err().to_string();
        assert!(error.contains("future channel test"), "{error}");
    }

    #[test]
    fn legends_cover_every_variant() {
        assert_eq!(STATE_LEGEND.len(), 5);
        assert_eq!(MATURITY_LEGEND.len(), 3);
    }
}
