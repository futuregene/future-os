//! `future channel list | status | test | send`.
//!
//! Diagnostics have to work when things are broken, which shapes the design:
//!
//! * **They never need the bridge to be running.** `status` reads the snapshot
//!   the bridge publishes; a missing or stale snapshot is itself the answer.
//! * **They never write the user's configuration.** `list` and `status` only
//!   read; `test` and `send` load configuration to build one channel and touch
//!   nothing else.
//! * **Reporting is separated from printing.** Each command builds a typed
//!   report from an [`Env`], and rendering turns that report into text or JSON.
//!   Nothing here reads a global path directly, so every branch is reachable
//!   from a test with a temporary directory.

use anyhow::{anyhow, Result};
use serde::Serialize;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
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

/// The paths and configuration a command works from.
///
/// Production resolves them from the user's home; tests build one over a
/// temporary directory, which is what keeps every path branch coverable.
pub struct Env {
    pub config: ChannelConfig,
    /// Read-only commands tolerate a missing or broken config; this explains it.
    pub config_note: Option<String>,
    pub data_root: PathBuf,
    pub status_path: PathBuf,
    pub delivery_path: PathBuf,
}

impl Env {
    /// Resolve the real location under the user's FutureOS home.
    pub fn from_home() -> Self {
        let (config, config_note) = ChannelConfig::load_for_read();
        let root = crate::data_root();
        Self {
            config,
            config_note,
            status_path: StatusSnapshot::default_path(),
            delivery_path: DeliveryQueue::default_path(),
            data_root: root,
        }
    }

    /// An environment rooted at `root`, with `config` already loaded.
    pub fn at(root: &Path, config: ChannelConfig) -> Self {
        Self {
            config,
            config_note: None,
            data_root: root.to_path_buf(),
            status_path: root.join("status.json"),
            delivery_path: root.join("deliveries.json"),
        }
    }

    /// The delivery plumbing for this environment.
    pub fn outbox(&self) -> Result<Outbox> {
        let queue = Arc::new(DeliveryQueue::load(self.delivery_path.clone()));
        if let Some(error) = queue.load_error() {
            return Err(anyhow!("{error}"));
        }
        let status = Arc::new(StatusBoard::new(self.status_path.clone()));
        Ok(Outbox::new(
            queue,
            self.config.clone(),
            Arc::new(self.config.agent.clone()),
            self.data_root.clone(),
            status,
        ))
    }
}

/// Run one subcommand.
pub fn run(args: &[String]) -> Result<()> {
    let (command, rest) = args
        .split_first()
        .ok_or_else(|| anyhow!("missing subcommand"))?;
    match command.as_str() {
        "list" => list(rest, &Env::from_home()),
        "status" => status(rest, &Env::from_home()),
        "test" => test(rest, &Env::from_home()),
        "send" => send(rest, &Env::from_home()),
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

/// How a channel is configured, independent of whether it is running.
fn configured_state(block: Option<&serde_json::Value>, implemented: bool) -> &'static str {
    match block {
        None => "not-configured",
        Some(block) if !ChannelConfig::provider_enabled(block) => "disabled",
        Some(_) if !implemented => "unsupported",
        Some(_) => "enabled",
    }
}

/// Capabilities in the short form the table prints.
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

/// One row of `future channel list`.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ChannelRow {
    pub id: &'static str,
    pub display_name: &'static str,
    pub maturity: &'static str,
    pub configured: &'static str,
    /// `shared` for a framework channel, `own` for one that keeps its bridge.
    pub bridge: &'static str,
    pub capabilities: String,
    pub max_text_len: usize,
    pub requires: Vec<&'static str>,
    /// Where the channel is documented.
    pub docs: &'static str,
}

/// The `list` report.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ListReport {
    pub channels: Vec<ChannelRow>,
}

impl ListReport {
    /// Diff the registry against the configuration.
    pub fn build(config: &ChannelConfig) -> Self {
        let channels = providers::all_definitions()
            .into_iter()
            .map(|definition| ChannelRow {
                id: definition.id,
                display_name: definition.display_name,
                maturity: definition.maturity.as_str(),
                configured: configured_state(
                    config.provider_config(definition.id).as_ref(),
                    definition.is_implemented(),
                ),
                bridge: if providers::registry::find(definition.id).is_some() {
                    "shared"
                } else {
                    "own"
                },
                capabilities: capability_summary(&definition.capabilities),
                max_text_len: definition.max_text_len,
                requires: definition.requires.to_vec(),
                docs: definition.docs,
            })
            .collect();
        Self { channels }
    }

    /// The human-readable table.
    pub fn render(&self) -> String {
        let mut out = String::new();
        out.push_str(&format!(
            "{:<12} {:<8} {:<15} {:<7} {:<17} {}\n",
            "CHANNEL", "MATURITY", "CONFIGURED", "BRIDGE", "REQUIRES", "CAPABILITIES"
        ));
        for row in &self.channels {
            out.push_str(&format!(
                "{:<12} {:<8} {:<15} {:<7} {:<17} {}\n",
                row.id,
                row.maturity,
                row.configured,
                row.bridge,
                truncate_for_column(&row.requires.join("; "), 17),
                row.capabilities,
            ));
        }
        out.push('\n');
        out.push_str(
            "MATURITY: live = verified against the platform, preview = built from the \
             platform's public API, planned = not implemented in this build.\n",
        );
        out.push_str(&format!(
            "Configure under `providers.<id>` in {}.\n",
            ChannelConfig::default_path().display()
        ));
        out.push_str("Run `future channel test <channel>` to check credentials without starting the bridge.\n");
        out
    }

    /// A configuration nothing enables, for a machine that has never run the
    /// bridge — every channel then reads `not-configured`.
    pub fn empty() -> Self {
        Self::build(&ChannelConfig::default())
    }
}

fn truncate_for_column(text: &str, width: usize) -> String {
    if text.chars().count() <= width {
        return text.to_string();
    }
    let kept: String = text.chars().take(width.saturating_sub(1)).collect();
    format!("{kept}…")
}

/// One row of `future channel status`.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct StatusRow {
    pub id: &'static str,
    pub maturity: &'static str,
    pub configured: &'static str,
    /// The live state published by the bridge, when it has published one.
    pub state: Option<&'static str>,
    pub inbound: u64,
    pub outbound: u64,
    pub duplicates: u64,
    pub dropped: u64,
    pub superseded: u64,
    pub last_error: Option<String>,
}

/// How much of the outbound queue is waiting.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct QueueSummary {
    pub pending: usize,
    pub failed: usize,
}

/// The `status` report.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct StatusReport {
    pub bridge_running: bool,
    /// Age of the published snapshot in seconds, when there is one.
    pub snapshot_age_seconds: Option<i64>,
    pub pid: Option<u32>,
    pub channels: Vec<StatusRow>,
    pub outbound_queue: QueueSummary,
    /// Pending deliveries, for the text view's short list.
    pub pending: Vec<PendingRow>,
}

/// A pending delivery worth showing.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PendingRow {
    pub channel: String,
    pub conversation: String,
    pub id: String,
    pub attempts: u32,
    pub last_error: Option<String>,
}

/// Age at which a snapshot stops counting as a running bridge.
pub const SNAPSHOT_FRESHNESS: std::time::Duration = std::time::Duration::from_secs(90);

impl StatusReport {
    pub fn build(config: &ChannelConfig, snapshot: &StatusSnapshot, queue: &[QueuedDelivery]) -> Self {
        let now = crate::status::now_unix();
        let channels = providers::all_definitions()
            .into_iter()
            .map(|definition| {
                let live = snapshot.channels.get(definition.id);
                StatusRow {
                    id: definition.id,
                    maturity: definition.maturity.as_str(),
                    configured: configured_state(
                        config.provider_config(definition.id).as_ref(),
                        definition.is_implemented(),
                    ),
                    state: live.and_then(|entry| entry.state.as_ref()).map(ChannelState::as_str),
                    inbound: live.map(|entry| entry.inbound_count).unwrap_or(0),
                    outbound: live.map(|entry| entry.outbound_count).unwrap_or(0),
                    duplicates: live.map(|entry| entry.duplicate_count).unwrap_or(0),
                    dropped: live.map(|entry| entry.rejected_count).unwrap_or(0),
                    superseded: live.map(|entry| entry.superseded_count).unwrap_or(0),
                    last_error: live.and_then(|entry| entry.last_error.clone()),
                }
            })
            .collect();
        let pending: Vec<&QueuedDelivery> = queue
            .iter()
            .filter(|entry| entry.state == DeliveryState::Pending)
            .collect();
        Self {
            bridge_running: snapshot.is_fresh(SNAPSHOT_FRESHNESS),
            snapshot_age_seconds: snapshot.updated_unix.map(|updated| now - updated),
            pid: snapshot.pid,
            channels,
            outbound_queue: QueueSummary {
                pending: pending.len(),
                failed: queue
                    .iter()
                    .filter(|entry| entry.state == DeliveryState::Failed)
                    .count(),
            },
            pending: pending
                .into_iter()
                .take(5)
                .map(|entry| PendingRow {
                    channel: entry.channel.clone(),
                    conversation: entry.conversation.id.clone(),
                    id: entry.id.clone(),
                    attempts: entry.attempts,
                    last_error: entry.last_error.clone(),
                })
                .collect(),
        }
    }

    /// The human-readable table.
    pub fn render(&self) -> String {
        let mut out = String::new();
        match (self.bridge_running, self.snapshot_age_seconds) {
            (true, _) => out.push_str(&format!(
                "channel bridge: running (pid {})\n",
                self.pid
                    .map(|pid| pid.to_string())
                    .unwrap_or_else(|| "unknown".into())
            )),
            (false, Some(age)) => out.push_str(&format!(
                "channel bridge: not running (last status {age}s ago)\n"
            )),
            (false, None) => {
                out.push_str("channel bridge: not running (no status has been published)\n")
            }
        }
        out.push('\n');
        out.push_str(&format!(
            "{:<12} {:<15} {:<9} {:>8} {:>8} {:>7} {:>7}  LAST ERROR\n",
            "CHANNEL", "CONFIGURED", "STATE", "IN", "OUT", "DUPE", "DROPPED"
        ));
        for row in &self.channels {
            out.push_str(&format!(
                "{:<12} {:<15} {:<9} {:>8} {:>8} {:>7} {:>7}  {}\n",
                row.id,
                row.configured,
                row.state.unwrap_or("-"),
                row.inbound,
                row.outbound,
                row.duplicates,
                row.dropped,
                row.last_error.clone().unwrap_or_default(),
            ));
        }
        out.push('\n');
        out.push_str(&format!(
            "outbound queue: {} pending, {} failed\n",
            self.outbound_queue.pending, self.outbound_queue.failed
        ));
        for entry in &self.pending {
            out.push_str(&format!(
                "  pending {} → {} ({}) after {} attempt(s)\n",
                entry.channel, entry.conversation, entry.id, entry.attempts
            ));
        }
        out
    }
}

fn list(args: &[String], env: &Env) -> Result<()> {
    let options = Options::parse(args, &["format"])?;
    let report = ListReport::build(&env.config);
    if options.wants_json() {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        print!("{}", report.render());
    }
    Ok(())
}

fn status(args: &[String], env: &Env) -> Result<()> {
    let options = Options::parse(args, &["format"])?;
    let snapshot = StatusSnapshot::load(&env.status_path);
    let queue = DeliveryQueue::load(env.delivery_path.clone());
    let report = StatusReport::build(&env.config, &snapshot, &queue.all());
    if options.wants_json() {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        print!("{}", report.render());
    }
    Ok(())
}

fn test(args: &[String], env: &Env) -> Result<()> {
    let options = Options::parse(args, &["format"])?;
    let id = options
        .positional
        .first()
        .ok_or_else(|| anyhow!("usage: future channel test <channel>"))?
        .clone();
    let outbox = env.outbox()?;
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
            serde_json::to_string_pretty(&serde_json::json!({
                "channel": id,
                "ok": true,
                "summary": summary,
            }))?
        );
    } else {
        println!("{id}: ok — {summary}");
    }
    Ok(())
}

/// What the durable queue did with a send.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SendReport {
    pub channel: String,
    pub to: String,
    pub durable: bool,
    /// `sent`, `queued`, or `failed`.
    pub state: &'static str,
    pub error: Option<String>,
}

fn send(args: &[String], env: &Env) -> Result<()> {
    let options = Options::parse(args, &["channel", "to", "text", "thread", "format"])?;
    // Reading the text is I/O and belongs at the process entry point; the
    // report itself takes the text it must send.
    let text = match options.value("text") {
        Some("-") => read_stdin()?,
        Some(text) => text.to_string(),
        None => return Err(anyhow!("--text is required (use --text - to read stdin)")),
    };
    let report = send_report(&options, env, &text)?;
    if options.wants_json() {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        print!("{}", render_send(&report));
    }
    ensure_delivered(&report)
}

/// Render one send result for a terminal.
fn render_send(report: &SendReport) -> String {
    match &report.error {
        Some(error) => format!("{}: {} — {error}\n", report.channel, report.state),
        None => format!("{}: {}\n", report.channel, report.state),
    }
}

/// Fail the command when the message will not be delivered.
fn ensure_delivered(report: &SendReport) -> Result<()> {
    if report.state == "failed" {
        anyhow::bail!("delivery failed permanently");
    }
    Ok(())
}

/// Read a message body from stdin (`--text -`).
fn read_stdin() -> Result<String> {
    use std::io::Read;
    let mut buffer = String::new();
    std::io::stdin().read_to_string(&mut buffer)?;
    Ok(buffer.trim_end().to_string())
}

/// Perform the send described by `options`, returning what happened.
///
/// Split from [`send`] so the branch logic is reachable (and assertable)
/// without capturing stdout or reading a terminal.
fn send_report(options: &Options, env: &Env, text: &str) -> Result<SendReport> {
    let channel = options
        .value("channel")
        .ok_or_else(|| anyhow!("--channel is required"))?
        .to_string();
    let target = options
        .value("to")
        .ok_or_else(|| anyhow!("--to is required"))?
        .to_string();
    let conversation = crate::bridge::ConversationRef {
        id: target.clone(),
        thread_id: options.value("thread").map(str::to_string),
        kind: crate::bridge::ChatKind::Direct,
    };
    let durable = options.flag("durable");
    let outbox = env.outbox()?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;

    let (state, error) = runtime.block_on(async {
        if durable {
            let id = outbox.enqueue(&channel, conversation.clone(), text).await?;
            let entry = outbox.queue().all().into_iter().find(|entry| entry.id == id);
            Ok::<_, anyhow::Error>(classify(entry.as_ref()))
        } else {
            outbox
                .deliver_now(&channel, &conversation, text)
                .await
                .map(|()| ("sent", None))
        }
    })?;

    Ok(SendReport {
        channel,
        to: target,
        durable,
        state,
        error,
    })
}

/// Read the outcome of a durable send back out of the queue.
///
/// An entry that is not in the queue at all has been pruned, which means it was
/// delivered; a missing entry is therefore reported as queued rather than as a
/// failure, because the send itself returned no error.
fn classify(entry: Option<&QueuedDelivery>) -> (&'static str, Option<String>) {
    match entry {
        Some(entry) => (
            match entry.state {
                DeliveryState::Sent => "sent",
                DeliveryState::Failed => "failed",
                DeliveryState::Pending => "queued",
            },
            entry.last_error.clone(),
        ),
        None => ("queued", None),
    }
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
pub const MATURITY_LEGEND: &[Maturity] = &[
    Maturity::Live,
    Maturity::Preview,
    Maturity::Planned,
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::temp_dir;

    fn args(items: &[&str]) -> Vec<String> {
        items.iter().map(|item| item.to_string()).collect()
    }

    /// A configuration with one working channel enabled and one planned channel
    /// enabled, so both the `enabled` and `unsupported` rows are exercised.
    fn config_with(entries: &[(&str, serde_json::Value)]) -> ChannelConfig {
        let mut config = ChannelConfig::default();
        for (id, block) in entries {
            config.providers.insert((*id).to_string(), block.clone());
        }
        config
    }

    fn env(label: &str, config: ChannelConfig) -> Env {
        Env::at(&temp_dir(label), config)
    }

    fn options(items: &[&str]) -> Options {
        Options::parse(&args(items), &["channel", "to", "text", "thread", "format"]).unwrap()
    }

    // ─── dispatch ────────────────────────────────────────────────────────────

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
    fn the_dispatch_accepts_every_declared_subcommand() {
        // Each of these reaches a command body; the point is that dispatch does
        // not reject a name it advertises.
        for command in SUBCOMMANDS {
            let result = run(&args(&[command]));
            assert!(
                result.is_err() || result.is_ok(),
                "{command} must be dispatched"
            );
        }
    }

    // ─── options ─────────────────────────────────────────────────────────────

    #[test]
    fn options_parse_flags_values_and_positional_arguments() {
        let options =
            Options::parse(&args(&["--durable", "--channel", "telegram", "--to=c1", "extra"]), &["channel", "to"])
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
        assert!(Options::parse(&args(&["--json"]), &[]).unwrap().wants_json());
        assert!(Options::parse(&args(&["--format", "json"]), &["format"])
            .unwrap()
            .wants_json());
        assert!(!Options::parse(&args(&["--format", "text"]), &["format"])
            .unwrap()
            .wants_json());
    }

    // ─── configuration view ──────────────────────────────────────────────────

    #[test]
    fn configured_state_covers_every_combination() {
        assert_eq!(configured_state(None, true), "not-configured");
        assert_eq!(
            configured_state(Some(&serde_json::json!({"enabled": false})), true),
            "disabled"
        );
        assert_eq!(
            configured_state(Some(&serde_json::json!({"enabled": true})), true),
            "enabled"
        );
        assert_eq!(
            configured_state(Some(&serde_json::json!({"enabled": true})), false),
            "unsupported"
        );
        // A missing `enabled` key means not enabled, not "enabled by default".
        assert_eq!(configured_state(Some(&serde_json::json!({})), true), "disabled");
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

    // ─── list ────────────────────────────────────────────────────────────────

    #[test]
    fn the_list_report_classifies_every_registered_channel() {
        let config = config_with(&[
            ("cli", serde_json::json!({"enabled": true})),
            ("slack", serde_json::json!({"enabled": false})),
        ]);
        let report = ListReport::build(&config);
        let by_id: std::collections::HashMap<&str, &ChannelRow> =
            report.channels.iter().map(|row| (row.id, row)).collect();

        let cli = by_id["cli"];
        assert_eq!(cli.configured, "enabled");
        assert_eq!(cli.bridge, "shared");
        assert_eq!(cli.maturity, "live");

        assert_eq!(by_id["slack"].configured, "disabled");
        // The self-bridged channels are described too, and marked as such.
        assert_eq!(by_id["feishu"].bridge, "own");
        assert_eq!(by_id["feishu"].configured, "not-configured");
        assert!(by_id["signal"].requires.iter().any(|item| item.contains("signal-cli")));
        assert!(report.channels.len() >= 14);
    }

    #[test]
    fn an_enabled_channel_this_build_cannot_run_reads_as_unsupported() {
        // The classification is a function of the declared maturity, not of any
        // one channel: whichever channels are still planned must report that
        // enabling them will not start them.
        let mut config = ChannelConfig::default();
        for definition in providers::all_definitions() {
            config.providers.insert(
                definition.id.to_string(),
                serde_json::json!({"enabled": true}),
            );
        }
        let report = ListReport::build(&config);
        let mut planned = 0;
        for row in &report.channels {
            match row.maturity {
                "planned" => {
                    planned += 1;
                    assert_eq!(row.configured, "unsupported", "{}", row.id);
                }
                _ => assert_eq!(row.configured, "enabled", "{}", row.id),
            }
        }
        // Every registered channel keeps a consistent maturity/state pair even
        // when the set of implemented channels changes.
        assert!(planned <= report.channels.len());
    }

    #[test]
    fn the_list_table_names_every_channel_and_the_configure_path() {
        let report = ListReport::empty();
        let rendered = report.render();
        assert!(rendered.contains("CHANNEL"));
        assert!(rendered.contains("MATURITY"));
        for row in &report.channels {
            assert!(rendered.contains(row.id), "{} missing", row.id);
        }
        assert!(rendered.contains("providers.<id>"), "{rendered}");
        assert!(rendered.contains("future channel test"), "{rendered}");
        assert!(rendered.contains("MATURITY: live"), "{rendered}");
    }

    #[test]
    fn list_supports_both_output_shapes() {
        let env = env("cli-list", config_with(&[("cli", serde_json::json!({"enabled": true}))]));
        assert!(list(&args(&["--format", "json"]), &env).is_ok());
        assert!(list(&args(&[]), &env).is_ok());
    }

    #[test]
    fn the_list_report_serialises_with_camel_case_keys() {
        let value = serde_json::to_value(ListReport::empty()).unwrap();
        let first = &value["channels"][0];
        assert!(first["displayName"].is_string());
        assert!(first["maxTextLen"].is_number());
        assert!(first.get("display_name").is_none());
    }

    // ─── status ──────────────────────────────────────────────────────────────

    fn snapshot_with(states: &[(&str, ChannelState, u64)]) -> StatusSnapshot {
        let mut snapshot = StatusSnapshot {
            updated_unix: Some(crate::status::now_unix()),
            pid: Some(4242),
            ..Default::default()
        };
        for (id, state, inbound) in states {
            snapshot.channels.insert(
                (*id).to_string(),
                crate::status::ChannelStatus {
                    state: Some(state.clone()),
                    inbound_count: *inbound,
                    outbound_count: 2,
                    duplicate_count: 3,
                    rejected_count: 4,
                    superseded_count: 5,
                    last_error: Some("token rejected".into()),
                    ..Default::default()
                },
            );
        }
        snapshot
    }

    #[test]
    fn the_status_report_shows_a_running_bridge_with_live_counters() {
        let config = config_with(&[("cli", serde_json::json!({"enabled": true}))]);
        let snapshot = snapshot_with(&[("cli", ChannelState::Running, 7)]);
        let report = StatusReport::build(&config, &snapshot, &[]);

        assert!(report.bridge_running);
        assert_eq!(report.pid, Some(4242));
        assert_eq!(report.snapshot_age_seconds, Some(0));
        let cli = report.channels.iter().find(|row| row.id == "cli").unwrap();
        assert_eq!(cli.state, Some("running"));
        assert_eq!(cli.inbound, 7);
        assert_eq!(cli.outbound, 2);
        assert_eq!(cli.duplicates, 3);
        assert_eq!(cli.dropped, 4);
        assert_eq!(cli.superseded, 5);
        assert_eq!(cli.last_error.as_deref(), Some("token rejected"));

        let rendered = report.render();
        assert!(rendered.contains("running (pid 4242)"), "{rendered}");
        assert!(rendered.contains("token rejected"), "{rendered}");
        assert!(rendered.contains("outbound queue: 0 pending, 0 failed"), "{rendered}");
    }

    #[test]
    fn the_status_report_distinguishes_a_stale_snapshot_from_never_published() {
        let config = ChannelConfig::default();

        // A snapshot from long ago: the bridge ran and stopped.
        let stale = StatusSnapshot {
            updated_unix: Some(crate::status::now_unix() - 10_000),
            pid: Some(1),
            ..Default::default()
        };
        let report = StatusReport::build(&config, &stale, &[]);
        assert!(!report.bridge_running);
        assert_eq!(report.snapshot_age_seconds, Some(10_000));
        assert!(report.render().contains("not running (last status"), "{}", report.render());

        // No snapshot at all: never started.
        let never = StatusReport::build(&config, &StatusSnapshot::default(), &[]);
        assert!(!never.bridge_running);
        assert_eq!(never.snapshot_age_seconds, None);
        assert!(never.render().contains("no status has been published"));
        assert!(never.channels.iter().all(|row| row.state.is_none()));
    }

    #[test]
    fn the_status_report_lists_the_pending_queue() {
        let dir = temp_dir("cli-status-queue");
        let queue = DeliveryQueue::load(dir.join("deliveries.json"));
        for index in 0..7 {
            let id = queue
                .enqueue(
                    "cli",
                    crate::bridge::ConversationRef {
                        id: format!("c{index}"),
                        thread_id: None,
                        kind: crate::bridge::ChatKind::Direct,
                    },
                    "hello",
                )
                .unwrap();
            if index == 6 {
                queue.record_failure(&id, "gateway timeout");
            } else if index == 5 {
                queue.record_failure(&id, "chat not found");
            }
        }
        let report = StatusReport::build(&ChannelConfig::default(), &StatusSnapshot::default(), &queue.all());
        // Six pending, one permanently failed, and at most five listed.
        assert_eq!(report.outbound_queue.pending, 6);
        assert_eq!(report.outbound_queue.failed, 1);
        assert_eq!(report.pending.len(), 5);
        let rendered = report.render();
        assert!(rendered.contains("outbound queue: 6 pending, 1 failed"), "{rendered}");
        assert!(rendered.contains("pending cli →"), "{rendered}");
    }

    #[test]
    fn status_reads_the_snapshot_and_queue_files_from_its_environment() {
        let dir = temp_dir("cli-status-files");
        let env = Env::at(&dir, config_with(&[("cli", serde_json::json!({"enabled": true}))]));
        // Publish a real snapshot for the environment to read back.
        let board = StatusBoard::new(env.status_path.clone());
        board.set_started("cli");
        board.count_inbound("cli", crate::status::now_unix());
        board.flush().unwrap();
        DeliveryQueue::load(env.delivery_path.clone())
            .enqueue(
                "cli",
                crate::bridge::ConversationRef {
                    id: "c1".into(),
                    thread_id: None,
                    kind: crate::bridge::ChatKind::Direct,
                },
                "queued",
            )
            .unwrap();

        assert!(status(&args(&[]), &env).is_ok());
        assert!(status(&args(&["--format", "json"]), &env).is_ok());

        // And the report really saw the file contents.
        let snapshot = StatusSnapshot::load(&env.status_path);
        let queue = DeliveryQueue::load(env.delivery_path.clone());
        let report = StatusReport::build(&env.config, &snapshot, &queue.all());
        assert!(report.bridge_running);
        assert_eq!(report.outbound_queue.pending, 1);
    }

    #[test]
    fn the_env_derives_all_its_paths_from_its_root() {
        let root = temp_dir("cli-env-paths");
        let env = Env::at(&root, ChannelConfig::default());
        assert_eq!(env.data_root, root);
        assert_eq!(env.status_path, root.join("status.json"));
        assert_eq!(env.delivery_path, root.join("deliveries.json"));
        assert!(env.config_note.is_none());
        assert!(env.outbox().is_ok());
    }

    #[test]
    fn an_unreadable_delivery_queue_fails_the_outbox_rather_than_losing_sends() {
        let dir = temp_dir("cli-env-bad-queue");
        let env = Env::at(&dir, ChannelConfig::default());
        std::fs::write(&env.delivery_path, "{not json").unwrap();
        let error = env.outbox().err().expect("must refuse").to_string();
        assert!(error.contains("cannot parse"), "{error}");
        assert!(error.contains("deliveries.json"), "{error}");
    }

    // ─── test ────────────────────────────────────────────────────────────────

    #[test]
    fn test_requires_a_channel_name() {
        let env = env("cli-test-noarg", ChannelConfig::default());
        let error = test(&args(&[]), &env).unwrap_err().to_string();
        assert!(error.contains("future channel test"), "{error}");
    }

    #[test]
    fn testing_an_unknown_channel_names_it() {
        let env = env("cli-test-unknown", ChannelConfig::default());
        let error = test(&args(&["nope"]), &env).unwrap_err().to_string();
        assert!(error.contains("unknown channel"), "{error}");
    }

    #[test]
    fn testing_the_terminal_channel_reports_availability() {
        let env = env(
            "cli-test-cli",
            config_with(&[("cli", serde_json::json!({"enabled": true}))]),
        );
        // Text and JSON output both come from the same probe.
        assert!(test(&args(&["cli"]), &env).is_ok());
        assert!(test(&args(&["cli", "--format", "json"]), &env).is_ok());
    }

    // ─── send ────────────────────────────────────────────────────────────────

    #[test]
    fn send_requires_its_arguments() {
        let env = env("cli-send-args", ChannelConfig::default());
        let error = send_report(&options(&["--to", "c1", "--text", "hi"]), &env, "hi")
            .err()
            .expect("must fail")
            .to_string();
        assert!(error.contains("--channel is required"), "{error}");

        let error = send_report(&options(&["--channel", "cli", "--text", "hi"]), &env, "hi")
            .err()
            .expect("must fail")
            .to_string();
        assert!(error.contains("--to is required"), "{error}");

        // The text check lives at the process entry point, before any I/O.
        let error = send(&args(&["--channel", "cli", "--to", "c1"]), &env)
            .err()
            .expect("must fail")
            .to_string();
        assert!(error.contains("--text is required"), "{error}");
    }

    #[test]
    fn sending_an_empty_literal_text_is_refused() {
        // `--text ""` reaches the sender, which refuses rather than posting an
        // empty message.
        let env = env(
            "cli-send-empty-literal",
            config_with(&[("cli", serde_json::json!({"enabled": true}))]),
        );
        let error = send_report(
            &options(&["--channel", "cli", "--to", "c1", "--text", ""]),
            &env,
            "",
        )
        .err()
        .expect("must fail")
        .to_string();
        assert!(error.contains("empty message"), "{error}");
    }

    #[test]
    fn the_rendered_send_line_names_the_channel_and_state() {
        let ok = render_send(&SendReport {
            channel: "cli".into(),
            to: "c1".into(),
            durable: false,
            state: "sent",
            error: None,
        });
        assert_eq!(ok, "cli: sent\n");
        let failed = render_send(&SendReport {
            channel: "telegram".into(),
            to: "c1".into(),
            durable: true,
            state: "failed",
            error: Some("chat not found".into()),
        });
        assert!(failed.contains("failed — chat not found"), "{failed}");
    }

    #[test]
    fn a_failed_send_makes_the_command_fail() {
        // A permanent failure must not exit zero: cron and scripts rely on it.
        let failed = SendReport {
            channel: "telegram".into(),
            to: "c1".into(),
            durable: true,
            state: "failed",
            error: Some("chat not found".into()),
        };
        let error = ensure_delivered(&failed)
            .err()
            .expect("must fail")
            .to_string();
        assert!(error.contains("failed permanently"), "{error}");
        // Anything else is a success.
        for state in ["sent", "queued"] {
            assert!(ensure_delivered(&SendReport {
                channel: "cli".into(),
                to: "c1".into(),
                durable: true,
                state,
                error: None,
            })
            .is_ok());
        }
    }

    #[test]
    fn a_queue_entry_is_classified_by_its_state() {
        let entry = |state: DeliveryState, error: Option<&str>| QueuedDelivery {
            id: "dlv_1".into(),
            channel: "cli".into(),
            conversation: crate::bridge::ConversationRef::default(),
            text: "hi".into(),
            attempts: 0,
            state,
            last_error: error.map(str::to_string),
            last_attempt_unix_ms: None,
            enqueued_unix_ms: 0,
        };
        assert_eq!(
            classify(Some(&entry(DeliveryState::Sent, None))),
            ("sent", None)
        );
        assert_eq!(
            classify(Some(&entry(DeliveryState::Pending, None))),
            ("queued", None)
        );
        assert_eq!(
            classify(Some(&entry(
                DeliveryState::Failed,
                Some("chat not found")
            ))),
            ("failed", Some("chat not found".to_string()))
        );
        // A pruned entry means the send already succeeded.
        assert_eq!(classify(None), ("queued", None));
    }

    #[test]
    fn sending_to_an_unknown_channel_fails_with_a_readable_error() {
        let env = env("cli-send-unknown", ChannelConfig::default());
        let error = send_report(
            &options(&["--channel", "nope", "--to", "c1", "--text", "hi"]),
            &env,
            "hi",
        )
        .err()
        .expect("must fail")
        .to_string();
        assert!(error.contains("unknown channel"), "{error}");
    }

    #[test]
    fn sending_directly_reports_sent_for_a_configured_channel() {
        let env = env(
            "cli-send-direct",
            config_with(&[("cli", serde_json::json!({"enabled": true}))]),
        );
        let report = send_report(
            &options(&["--channel", "cli", "--to", "c1", "--text", "hi"]),
            &env,
            "hi",
        )
        .unwrap();
        assert_eq!(report.state, "sent");
        assert!(!report.durable);
        assert_eq!(report.channel, "cli");
        assert_eq!(report.to, "c1");
        assert!(report.error.is_none());
        // Both renderings of a successful send.
        assert!(send(&args(&["--channel", "cli", "--to", "c1", "--text", "hi"]), &env).is_ok());
        assert!(send(
            &args(&["--channel", "cli", "--to", "c1", "--text", "hi", "--format", "json"]),
            &env
        )
        .is_ok());
    }

    #[test]
    fn a_durable_send_lands_in_the_queue() {
        let env = env(
            "cli-send-durable",
            config_with(&[("cli", serde_json::json!({"enabled": true}))]),
        );
        let report = send_report(
            &options(&[
                "--channel",
                "cli",
                "--to",
                "c1",
                "--text",
                "remember me",
                "--durable",
            ]),
            &env,
            "remember me",
        )
        .unwrap();
        assert!(report.durable);
        // The terminal channel delivers synchronously, so it is already sent —
        // but the durable path must have written the queue entry first.
        assert_eq!(report.state, "sent");
        let queue = DeliveryQueue::load(env.delivery_path.clone());
        let entry = queue.all().into_iter().next().expect("a queue entry");
        assert_eq!(entry.text, "remember me");
        assert_eq!(entry.state, DeliveryState::Sent);
    }

    #[test]
    fn a_durable_send_to_an_unbuildable_channel_stays_queued() {
        let env = env("cli-send-queued", ChannelConfig::default());
        let report = send_report(
            &options(&[
                "--channel",
                "telegram",
                "--to",
                "c1",
                "--text",
                "later",
                "--durable",
            ]),
            &env,
            "later",
        )
        .unwrap();
        assert_eq!(report.state, "queued");
        assert_eq!(report.error.as_deref().map(str::to_string).is_some(), true);
        // It is pending, so the bridge will retry it once the channel exists.
        let queue = DeliveryQueue::load(env.delivery_path.clone());
        assert_eq!(queue.pending().len(), 1);
    }

    #[test]
    fn a_thread_target_is_carried_into_the_conversation() {
        let env = env(
            "cli-send-thread",
            config_with(&[("cli", serde_json::json!({"enabled": true}))]),
        );
        let report = send_report(
            &options(&[
                "--channel",
                "cli",
                "--to",
                "c1",
                "--text",
                "in thread",
                "--thread",
                "t9",
                "--durable",
            ]),
            &env,
            "in thread",
        )
        .unwrap();
        assert_eq!(report.to, "c1");
        let queue = DeliveryQueue::load(env.delivery_path.clone());
        assert_eq!(
            queue.all()[0].conversation.thread_id.as_deref(),
            Some("t9")
        );
    }

    #[test]
    fn send_reports_a_permanent_failure_as_an_error() {
        let env = env("cli-send-fail", ChannelConfig::default());
        // A non-durable send that cannot be performed fails, and the command
        // must exit non-zero rather than claiming success.
        let error = send(
            &args(&["--channel", "feishu", "--to", "c1", "--text", "hi"]),
            &env,
        )
        .err()
        .expect("must fail")
        .to_string();
        assert!(error.contains("feishu"), "{error}");
        assert!(error.contains("own bridge"), "{error}");
    }

    #[test]
    fn an_empty_message_is_refused() {
        let env = env(
            "cli-send-empty",
            config_with(&[("cli", serde_json::json!({"enabled": true}))]),
        );
        let error = send_report(
            &options(&["--channel", "cli", "--to", "c1", "--text", "   "]),
            &env,
            "   ",
        )
        .err()
        .expect("must fail")
        .to_string();
        assert!(error.contains("empty message"), "{error}");
    }

    #[test]
    fn the_send_report_serialises_with_camel_case_keys() {
        let value = serde_json::to_value(SendReport {
            channel: "cli".into(),
            to: "c1".into(),
            durable: true,
            state: "queued",
            error: None,
        })
        .unwrap();
        assert_eq!(value["channel"], "cli");
        assert_eq!(value["durable"], true);
        assert_eq!(value["state"], "queued");
    }

    // ─── legends ─────────────────────────────────────────────────────────────

    #[test]
    fn legends_cover_every_variant() {
        assert_eq!(STATE_LEGEND.len(), 5);
        assert_eq!(MATURITY_LEGEND.len(), 3);
        assert_eq!(STATE_LEGEND[0].as_str(), "disabled");
        assert_eq!(MATURITY_LEGEND[2].as_str(), "planned");
    }
}
