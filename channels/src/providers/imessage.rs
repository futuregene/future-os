//! iMessage on macOS, through Messages.app and the local message database.
//!
//! There is no iMessage API: sending goes through AppleScript and receiving
//! reads the local Messages database. Both are macOS-only, so the platform
//! pieces are compiled behind `cfg(target_os = "macos")` and every entry
//! point reports a clear unsupported error elsewhere rather than pretending
//! to exist.
//!
//! * **Outbound** — `osascript` driving Messages.app. Every interpolated
//!   value must be escaped: a message body is attacker-controlled text, and
//!   an unescaped quote would let it become AppleScript source.
//! * **Inbound** — poll `~/Library/Messages/chat.db` read-only via the
//!   system `sqlite3`, converting Apple's epoch (2001-01-01, nanoseconds)
//!   into Unix milliseconds. Full Disk Access is required, and a missing
//!   permission must produce an actionable error, not silence.
//! * **Addressing** — direct conversations only, keyed on the sender's
//!   handle (phone number or email); a sender allowlist gates them.

use anyhow::{anyhow, Result};
use async_trait::async_trait;
use serde::Deserialize;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::bridge::{ConversationRef, ProviderCtx};
// These three are only reached from the macOS-gated implementation, so importing
// them unconditionally is an unused-import error on every other platform.
#[cfg(target_os = "macos")]
use crate::bridge::{ChatKind, Inbound, SenderRef};
use crate::providers::traits::{
    Capabilities, ChannelDefinition, ChannelSender, Maturity, Provider,
};
use crate::transport::LengthUnit;

pub static DEFINITION: ChannelDefinition = ChannelDefinition {
    id: "imessage",
    display_name: "iMessage (macOS)",
    description: "macOS Messages.app: AppleScript send, local database receive.",
    docs: "docs/guide/channels-imessage.md",
    maturity: Maturity::Preview,
    capabilities: Capabilities {
        receive: true,
        send: true,
        edit: false,
        threads: false,
        typing: false,
        reactions: false,
        media_in: false,
        media_out: false,
        mention_gate: false,
    },
    max_text_len: 20000,
    length_unit: LengthUnit::Chars,
    config_example: r#"{
  "enabled": true,
  "recipients": [],
  "sender_allowlist": [],
  "poll_seconds": 5,
  "db_path": ""
}"#,
    requires: &[
        "macOS",
        "Full Disk Access for the terminal running the bridge",
    ],
};

/// Milliseconds between the Unix epoch and Apple's reference date
/// (2001-01-01T00:00:00Z): 978307200 seconds.
pub(crate) const APPLE_EPOCH_OFFSET_MS: i64 = 978_307_200_000;

/// Convert a chat.db timestamp into Unix milliseconds.
///
/// Modern rows store nanoseconds since the Apple epoch; rows written by very
/// old macOS versions store plain seconds. The two ranges are orders of
/// magnitude apart (2e18 vs 2e9), so the magnitude of the value is the
/// reliable discriminator.
pub(crate) fn apple_ts_to_unix_ms(raw: i64) -> i64 {
    if raw.abs() >= 1_000_000_000_000_000 {
        raw / 1_000_000 + APPLE_EPOCH_OFFSET_MS
    } else {
        raw * 1_000 + APPLE_EPOCH_OFFSET_MS
    }
}

/// Escape a string for interpolation into an AppleScript double-quoted
/// literal.
///
/// A message body is attacker-controlled: without escaping, a `"` would
/// close the literal and let the rest of the text run as AppleScript source,
/// and a `\` would escape the closing quote itself.
pub(crate) fn applescript_escape(raw: &str) -> String {
    let mut escaped = String::with_capacity(raw.len() + 8);
    for ch in raw.chars() {
        match ch {
            '\\' => escaped.push_str("\\\\"),
            '"' => escaped.push_str("\\\""),
            _ => escaped.push(ch),
        }
    }
    escaped
}

/// The AppleScript that sends one message to one handle.
pub(crate) fn send_script(recipient: &str, text: &str) -> String {
    format!(
        "tell application \"Messages\"\n\
         set targetService to 1st account whose service type = iMessage\n\
         set targetBuddy to participant \"{recipient}\" of targetService\n\
         send \"{text}\" to targetBuddy\n\
         end tell",
        recipient = applescript_escape(recipient),
        text = applescript_escape(text),
    )
}

/// The SQL that lists messages newer than a watermark, oldest first.
///
/// A message without text (a reaction-only row, an attachment the bridge
/// cannot read) is filtered here rather than turned into an empty prompt.
pub(crate) fn poll_query(since_ms: i64) -> String {
    let since_apple_ns = (since_ms - APPLE_EPOCH_OFFSET_MS) * 1_000_000;
    format!(
        "SELECT m.ROWID, h.id, m.text, m.date, m.is_from_me \
         FROM message m \
         LEFT JOIN handle h ON m.handle_id = h.ROWID \
         WHERE m.date > {since_apple_ns} AND m.text IS NOT NULL \
         ORDER BY m.date ASC"
    )
}

/// One row of the poll query, as printed by `sqlite3 -separator '\t'`.
///
/// Fields: ROWID, handle, text, raw timestamp, is_from_me. The handle can be
/// empty (a message on a conversation whose handle row was deleted), and
/// `sqlite3` prints an empty field for NULL.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ChatRow {
    pub rowid: i64,
    pub handle: String,
    pub text: String,
    pub timestamp_ms: i64,
    pub is_from_me: bool,
}

pub(crate) fn parse_chat_row(line: &str) -> Option<ChatRow> {
    let fields: Vec<&str> = line.split('\t').collect();
    if fields.len() < 5 {
        return None;
    }
    let rowid: i64 = fields[0].trim().parse().ok()?;
    let handle = fields[1].trim().to_string();
    let text = fields[2].to_string();
    let raw_ts: i64 = fields[3].trim().parse().ok()?;
    let is_from_me = matches!(fields[4].trim(), "1");
    Some(ChatRow {
        rowid,
        handle,
        text,
        timestamp_ms: apple_ts_to_unix_ms(raw_ts),
        is_from_me,
    })
}

/// Parse the whole `sqlite3` output into rows, skipping lines that do not
/// decode (a text field containing a literal tab is split across extra
/// fields; re-joining the middle fields keeps the row parseable).
pub(crate) fn parse_chat_rows(output: &str) -> Vec<ChatRow> {
    output
        .lines()
        .filter_map(|line| {
            let line = line.trim_end_matches(['\r', '\n']);
            if line.is_empty() {
                return None;
            }
            parse_chat_row(line).or_else(|| parse_chat_row_wide(line))
        })
        .collect()
}

/// A row whose text contains tabs arrives with more than five fields; the
/// rowid, handle and the trailing timestamp/flag are still positional.
fn parse_chat_row_wide(line: &str) -> Option<ChatRow> {
    let fields: Vec<&str> = line.split('\t').collect();
    if fields.len() <= 5 {
        return None;
    }
    let rowid: i64 = fields[0].trim().parse().ok()?;
    let handle = fields[1].trim().to_string();
    let is_from_me = matches!(fields[fields.len() - 1].trim(), "1");
    let raw_ts: i64 = fields[fields.len() - 2].trim().parse().ok()?;
    let text = fields[2..fields.len() - 2].join("\t");
    Some(ChatRow {
        rowid,
        handle,
        text,
        timestamp_ms: apple_ts_to_unix_ms(raw_ts),
        is_from_me,
    })
}

/// The diagnostic printed when chat.db cannot be read.
pub(crate) fn permission_advice(db_path: &Path) -> String {
    format!(
        "cannot read the Messages database at {}: grant Full Disk Access to \
         the terminal running the bridge (System Settings → Privacy & \
         Security → Full Disk Access), then restart it",
        db_path.display()
    )
}

/// One channel's config block in `~/.future/channels/config.json`.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct IMessageConfig {
    pub enabled: bool,
    /// Handles the bot may write to proactively; empty means replies only.
    pub recipients: Vec<String>,
    /// Handles the bot answers; empty means every direct sender.
    pub sender_allowlist: Vec<String>,
    /// How often chat.db is polled.
    pub poll_seconds: u64,
    /// Override for the database location (tests, non-standard installs).
    /// Empty means `~/Library/Messages/chat.db`.
    pub db_path: String,
}

impl Default for IMessageConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            recipients: Vec::new(),
            sender_allowlist: Vec::new(),
            poll_seconds: 5,
            db_path: String::new(),
        }
    }
}

impl IMessageConfig {
    /// Where the Messages database lives.
    pub(crate) fn db_path(&self) -> PathBuf {
        if !self.db_path.is_empty() {
            return PathBuf::from(&self.db_path);
        }
        default_db_path()
    }
}

fn default_db_path() -> PathBuf {
    crate::config::home_dir()
        .join("Library")
        .join("Messages")
        .join("chat.db")
}

/// Whether this build can touch Messages.app at all.
pub(crate) fn platform_supported() -> bool {
    cfg!(target_os = "macos")
}

fn unsupported_error() -> anyhow::Error {
    anyhow!("the imessage channel is only supported on macOS")
}

/// The provider itself.
pub struct IMessage;

impl IMessage {
    fn config(ctx: &ProviderCtx) -> Result<IMessageConfig> {
        ctx.config()
    }
}

#[async_trait]
impl Provider for IMessage {
    fn definition(&self) -> &'static ChannelDefinition {
        &DEFINITION
    }

    fn sender(&self, ctx: &ProviderCtx) -> Result<Arc<dyn ChannelSender>> {
        if !platform_supported() {
            return Err(unsupported_error());
        }
        let config = Self::config(ctx)?;
        let sender = IMessageSender {
            config: config.clone(),
        };
        Ok(Arc::new(sender))
    }

    async fn run(&self, ctx: ProviderCtx) -> Result<()> {
        if !platform_supported() {
            return Err(unsupported_error());
        }
        let config = Self::config(&ctx)?;
        let sender = self.sender(&ctx)?;
        #[cfg(target_os = "macos")]
        {
            macos::run_polling(&ctx, &config, sender).await
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = (&ctx, &config, sender);
            Err(unsupported_error())
        }
    }

    async fn probe(&self, ctx: &ProviderCtx) -> Result<String> {
        if !platform_supported() {
            return Err(unsupported_error());
        }
        let config = Self::config(ctx)?;
        #[cfg(target_os = "macos")]
        {
            return macos::probe(&config).await;
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = config;
            Err(unsupported_error())
        }
    }
}

/// The outbound half: AppleScript against Messages.app.
pub(crate) struct IMessageSender {
    config: IMessageConfig,
}

#[async_trait]
impl ChannelSender for IMessageSender {
    fn definition(&self) -> &'static ChannelDefinition {
        &DEFINITION
    }

    async fn send_text(
        &self,
        conversation: &ConversationRef,
        text: &str,
    ) -> Result<Option<String>> {
        if !platform_supported() {
            return Err(unsupported_error());
        }
        #[cfg(target_os = "macos")]
        {
            macos::send(&conversation.id, text).await?;
            return Ok(None);
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = (&self.config, conversation, text);
            Err(unsupported_error())
        }
    }
}

pub fn provider() -> Box<dyn Provider> {
    Box::new(IMessage)
}

/// The macOS-only transport: `osascript` out, `sqlite3` in.
///
/// Both go through the system binaries rather than a library so the bridge
/// inherits the user's own permissions and never links against private
/// frameworks.
#[cfg(target_os = "macos")]
mod macos {
    use super::*;
    use std::sync::atomic::{AtomicI64, Ordering};
    use std::sync::Mutex as StdMutex;
    use std::time::Duration;

    /// Run one AppleScript, returning stderr on failure.
    pub(super) async fn run_osascript(script: &str) -> Result<String> {
        let output = tokio::process::Command::new("osascript")
            .arg("-e")
            .arg(script)
            .output()
            .await
            .map_err(|error| anyhow!("cannot launch osascript: {error}"))?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            anyhow::bail!("Messages.app refused the send: {stderr}");
        }
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    }

    pub(super) async fn send(recipient: &str, text: &str) -> Result<()> {
        let script = send_script(recipient, text);
        run_osascript(&script).await?;
        Ok(())
    }

    /// Query the database read-only. `mode=ro` keeps the bridge from ever
    /// writing to the user's message history, even by accident.
    pub(super) async fn query_db(db_path: &Path, sql: &str) -> Result<String> {
        if !db_path.exists() {
            anyhow::bail!(permission_advice(db_path));
        }
        let output = tokio::process::Command::new("sqlite3")
            .arg("-readonly")
            .arg("-separator")
            .arg("\t")
            .arg(db_path)
            .arg(sql)
            .output()
            .await
            .map_err(|error| anyhow!("cannot launch sqlite3: {error}"))?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            // "unable to open database file" is the permission failure; give
            // the actionable version, not sqlite's phrasing.
            if stderr.contains("unable to open") || stderr.contains("authorization") {
                anyhow::bail!(permission_advice(db_path));
            }
            anyhow::bail!("reading the Messages database failed: {}", stderr.trim());
        }
        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    }

    /// Prove both halves work: read one row from chat.db, and confirm
    /// Messages.app answers AppleScript.
    pub(super) async fn probe(config: &IMessageConfig) -> Result<String> {
        let db_path = config.db_path();
        query_db(&db_path, "SELECT COUNT(*) FROM message").await?;
        let version = run_osascript("tell application \"Messages\" to get version").await?;
        let summary = format!(
            "Messages.app {version}, database readable at {}",
            db_path.display()
        );
        Ok(summary)
    }

    /// The poll loop: query newer messages, hand each one to the bridge,
    /// sleep until the next interval.
    pub(super) async fn run_polling(
        ctx: &ProviderCtx,
        config: &IMessageConfig,
        sender: Arc<dyn ChannelSender>,
    ) -> Result<()> {
        let db_path = config.db_path();
        // The watermark starts at "now" so enabling the channel does not
        // replay the user's whole history into the agent.
        let watermark = Arc::new(AtomicI64::new(crate::bridge::dedup::now_ms()));
        let interval = Duration::from_secs(config.poll_seconds.max(1));
        let allowlist: StdMutex<Vec<String>> = StdMutex::new(config.sender_allowlist.clone());
        ctx.mark_running();
        loop {
            tokio::select! {
                _ = ctx.shutdown().notified() => return Ok(()),
                _ = tokio::time::sleep(interval) => {
                    let since = watermark.load(Ordering::SeqCst);
                    let output = match query_db(&db_path, &poll_query(since)).await {
                        Ok(output) => output,
                        Err(error) => {
                            ctx.mark_failed(&error.to_string());
                            // A permission failure will not heal in one poll;
                            // keep looping so a granted permission is picked up.
                            continue;
                        }
                    };
                    let mut newest = since;
                    for row in parse_chat_rows(&output) {
                        newest = newest.max(row.timestamp_ms);
                        if row.is_from_me {
                            // Our own sends must never come back as prompts.
                            continue;
                        }
                        if row.handle.is_empty() {
                            continue;
                        }
                        let allowed = {
                            let list = allowlist.lock().unwrap_or_else(|e| e.into_inner());
                            let empty = list.is_empty();
                            empty || list.iter().any(|h| h == &row.handle)
                        };
                        if !allowed {
                            continue;
                        }
                        let inbound = Inbound {
                            message_id: format!("imessage-{}", row.rowid),
                            sender: SenderRef {
                                id: row.handle.clone(),
                                display: None,
                            },
                            conversation: ConversationRef {
                                id: row.handle.clone(),
                                thread_id: None,
                                kind: ChatKind::Direct,
                            },
                            text: row.text,
                            media: Vec::new(),
                            addressed_to_bot: true,
                            created_at_ms: Some(row.timestamp_ms),
                            raw: None,
                        };
                        let outcome = ctx.handle(inbound, sender.clone()).await;
                        tracing::debug!(?outcome, "imessage handled");
                    }
                    watermark.store(newest, Ordering::SeqCst);
                }
            }
        }
    }
}

#[cfg(test)]
#[path = "imessage_tests.rs"]
mod tests;
