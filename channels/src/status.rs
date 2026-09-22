//! Channel runtime status, written by the bridge and read by the CLI.
//!
//! The bridge runs as its own process (`future channel`), so `future channel
//! status` cannot ask it anything in memory — and must still work when the
//! bridge is not running at all, which is exactly when status matters most. The
//! bridge therefore publishes a small JSON snapshot it rewrites as things
//! change, and the CLI reads that file. A missing file means "not running",
//! which is a truthful answer rather than an error.

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

/// How often the snapshot is flushed to disk at most.
const FLUSH_INTERVAL: Duration = Duration::from_secs(2);

/// One channel's state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChannelState {
    /// Configured but `enabled` is false.
    Disabled,
    /// Starting, or reconnecting after a failure.
    Starting,
    /// Running and connected.
    Running,
    /// Repeatedly failing; the last error explains why.
    Error,
    /// Configured, but this build cannot run it.
    Unsupported,
}

impl ChannelState {
    pub fn as_str(&self) -> &'static str {
        match self {
            ChannelState::Disabled => "disabled",
            ChannelState::Starting => "starting",
            ChannelState::Running => "running",
            ChannelState::Error => "error",
            ChannelState::Unsupported => "unsupported",
        }
    }
}

/// One channel's published status.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ChannelStatus {
    pub state: Option<ChannelState>,
    /// Seconds since the channel first started, when it is running.
    pub uptime_secs: Option<u64>,
    pub last_error: Option<String>,
    /// Unix seconds of the last inbound message handled.
    pub last_inbound_unix: Option<i64>,
    /// Unix seconds of the last outbound message sent.
    pub last_outbound_unix: Option<i64>,
    pub inbound_count: u64,
    pub outbound_count: u64,
    /// Messages rejected as duplicates.
    pub duplicate_count: u64,
    /// Messages dropped because the conversation mailbox was full.
    pub rejected_count: u64,
    /// Turns superseded by a newer message.
    pub superseded_count: u64,
}

/// The whole published snapshot.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct StatusSnapshot {
    /// Unix seconds of the last write.
    pub updated_unix: Option<i64>,
    /// Process id of the bridge that wrote it.
    pub pid: Option<u32>,
    pub channels: BTreeMap<String, ChannelStatus>,
}

impl StatusSnapshot {
    /// Default location: `~/.future/channels/status.json`.
    pub fn default_path() -> PathBuf {
        crate::config::home_dir()
            .join(".future")
            .join("channels")
            .join("status.json")
    }

    /// Read a snapshot, treating a missing or unreadable file as empty.
    ///
    /// A corrupt file is reported through `updated_unix == None` rather than
    /// failing the command: status is diagnostics, not a data source.
    pub fn load(path: &Path) -> Self {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|content| serde_json::from_str(&content).ok())
            .unwrap_or_default()
    }

    /// Whether the writer is probably still alive.
    pub fn is_fresh(&self, max_age: Duration) -> bool {
        match self.updated_unix {
            Some(updated) => {
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|elapsed| elapsed.as_secs() as i64)
                    .unwrap_or(0);
                now.saturating_sub(updated) <= max_age.as_secs() as i64
            }
            None => false,
        }
    }
}

/// In-process counters the bridge updates as it handles traffic.
#[derive(Debug, Default)]
pub struct Counters {
    inbound: u64,
    outbound: u64,
    duplicates: u64,
    rejected: u64,
    superseded: u64,
}

impl Counters {
    pub fn snapshot(&self) -> (u64, u64, u64, u64, u64) {
        (
            self.inbound,
            self.outbound,
            self.duplicates,
            self.rejected,
            self.superseded,
        )
    }
}

/// The bridge's status publisher.
pub struct StatusBoard {
    path: PathBuf,
    inner: Mutex<Inner>,
}

struct Inner {
    channels: BTreeMap<String, ChannelStatus>,
    counters: BTreeMap<String, Counters>,
    dirty: bool,
    last_flush: std::time::Instant,
}

impl StatusBoard {
    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            inner: Mutex::new(Inner {
                channels: BTreeMap::new(),
                counters: BTreeMap::new(),
                dirty: false,
                last_flush: std::time::Instant::now() - FLUSH_INTERVAL,
            }),
        }
    }

    /// Record a channel's state, with an optional explanation.
    ///
    /// A state change is published immediately: it is the part of the snapshot
    /// an operator reads to answer "is it up?", and a process that dies seconds
    /// after starting must still have said so. Counters stay on the periodic
    /// flush, because they change constantly.
    pub fn set_state(&self, channel: &str, state: ChannelState, error: Option<String>) {
        {
            let mut inner = self.lock();
            let entry = inner.channels.entry(channel.to_string()).or_default();
            entry.state = Some(state);
            entry.last_error = error;
            inner.dirty = true;
        }
        self.publish();
    }

    /// Record when a channel came up, for the uptime column.
    pub fn set_started(&self, channel: &str) {
        {
            let mut inner = self.lock();
            let entry = inner.channels.entry(channel.to_string()).or_default();
            entry.state = Some(ChannelState::Running);
            entry.last_error = None;
            entry.uptime_secs = Some(0);
            inner.dirty = true;
        }
        self.publish();
    }

    /// Write a transition out, reporting failure without failing the caller.
    fn publish(&self) {
        if let Err(error) = self.flush() {
            let message = format!("cannot publish a channel state change: {error}");
            tracing::debug!("{message}");
        }
    }

    /// Count one handled inbound message.
    pub fn count_inbound(&self, channel: &str, now_unix: i64) {
        let mut inner = self.lock();
        let counters = inner.counters.entry(channel.to_string()).or_default();
        counters.inbound += 1;
        let entry = inner.channels.entry(channel.to_string()).or_default();
        entry.last_inbound_unix = Some(now_unix);
        inner.dirty = true;
    }

    /// Count one delivered outbound message.
    pub fn count_outbound(&self, channel: &str, now_unix: i64) {
        let mut inner = self.lock();
        let counters = inner.counters.entry(channel.to_string()).or_default();
        counters.outbound += 1;
        let entry = inner.channels.entry(channel.to_string()).or_default();
        entry.last_outbound_unix = Some(now_unix);
        inner.dirty = true;
    }

    /// Count one suppressed duplicate.
    pub fn count_duplicate(&self, channel: &str) {
        let mut inner = self.lock();
        inner
            .counters
            .entry(channel.to_string())
            .or_default()
            .duplicates += 1;
        inner.dirty = true;
    }

    /// Count one message dropped by backpressure.
    pub fn count_rejected(&self, channel: &str) {
        let mut inner = self.lock();
        inner
            .counters
            .entry(channel.to_string())
            .or_default()
            .rejected += 1;
        inner.dirty = true;
    }

    /// Count one superseded turn.
    pub fn count_superseded(&self, channel: &str) {
        let mut inner = self.lock();
        inner
            .counters
            .entry(channel.to_string())
            .or_default()
            .superseded += 1;
        inner.dirty = true;
    }

    /// Write the snapshot if anything changed and the flush interval elapsed.
    pub fn flush_if_due(&self) -> Result<bool> {
        let snapshot = {
            let mut inner = self.lock();
            if !inner.dirty || inner.last_flush.elapsed() < FLUSH_INTERVAL {
                return Ok(false);
            }
            inner.dirty = false;
            inner.last_flush = std::time::Instant::now();
            Self::build(&inner)
        };
        self.write(&snapshot)?;
        Ok(true)
    }

    /// Write the snapshot unconditionally (startup, shutdown, transitions).
    pub fn flush(&self) -> Result<()> {
        let snapshot = {
            let mut inner = self.lock();
            inner.dirty = false;
            inner.last_flush = std::time::Instant::now();
            Self::build(&inner)
        };
        self.write(&snapshot)
    }

    /// Where the snapshot is published (diagnostics and tests).
    pub fn path(&self) -> &Path {
        &self.path
    }

    fn build(inner: &Inner) -> StatusSnapshot {
        let mut channels = inner.channels.clone();
        for (id, counters) in &inner.counters {
            let (inbound, outbound, duplicates, rejected, superseded) = counters.snapshot();
            let entry = channels.entry(id.clone()).or_default();
            entry.inbound_count = inbound;
            entry.outbound_count = outbound;
            entry.duplicate_count = duplicates;
            entry.rejected_count = rejected;
            entry.superseded_count = superseded;
        }
        StatusSnapshot {
            updated_unix: Some(now_unix()),
            pid: Some(std::process::id()),
            channels,
        }
    }

    fn write(&self, snapshot: &StatusSnapshot) -> Result<()> {
        let parent = self
            .path
            .parent()
            .filter(|path| !path.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        std::fs::create_dir_all(parent)?;
        // Write-then-rename: a reader never sees a half-written snapshot.
        let temporary = self.path.with_extension("json.tmp");
        std::fs::write(&temporary, serde_json::to_string_pretty(snapshot)?)?;
        std::fs::rename(&temporary, &self.path)?;
        Ok(())
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|error| error.into_inner())
    }
}

/// Current Unix time in seconds.
pub fn now_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn board(label: &str) -> (StatusBoard, PathBuf) {
        let dir = crate::test_support::temp_dir(label);
        let path = dir.join("status.json");
        (StatusBoard::new(path.clone()), path)
    }

    #[test]
    fn a_missing_snapshot_reads_as_empty() {
        let snapshot = StatusSnapshot::load(Path::new("/nonexistent/status.json"));
        assert!(snapshot.channels.is_empty());
        assert!(snapshot.updated_unix.is_none());
        assert!(!snapshot.is_fresh(Duration::from_secs(60)));
    }

    #[test]
    fn a_corrupt_snapshot_reads_as_empty_instead_of_failing() {
        let dir = crate::test_support::temp_dir("status-corrupt");
        let path = dir.join("status.json");
        std::fs::write(&path, "{not json").unwrap();
        assert!(StatusSnapshot::load(&path).channels.is_empty());
    }

    #[test]
    fn state_and_counters_round_trip_through_disk() {
        let (board, path) = board("status-round-trip");
        board.set_started("telegram");
        board.count_inbound("telegram", 1_700_000_000);
        board.count_outbound("telegram", 1_700_000_010);
        board.count_duplicate("telegram");
        board.count_rejected("slack");
        board.count_superseded("telegram");
        board.flush().unwrap();

        let snapshot = StatusSnapshot::load(&path);
        let telegram = snapshot.channels.get("telegram").expect("telegram");
        assert_eq!(telegram.state, Some(ChannelState::Running));
        assert_eq!(telegram.inbound_count, 1);
        assert_eq!(telegram.outbound_count, 1);
        assert_eq!(telegram.duplicate_count, 1);
        assert_eq!(telegram.superseded_count, 1);
        assert_eq!(telegram.rejected_count, 0);
        assert_eq!(telegram.last_inbound_unix, Some(1_700_000_000));
        assert_eq!(telegram.last_outbound_unix, Some(1_700_000_010));
        let slack = snapshot.channels.get("slack").expect("slack");
        assert_eq!(slack.rejected_count, 1);
        assert_eq!(slack.state, None);
    }

    #[test]
    fn an_error_state_keeps_the_explanation() {
        let (board, path) = board("status-error");
        board.set_state("qq", ChannelState::Error, Some("invalid token".into()));
        board.flush().unwrap();
        let snapshot = StatusSnapshot::load(&path);
        let entry = snapshot.channels.get("qq").unwrap();
        assert_eq!(entry.state, Some(ChannelState::Error));
        assert_eq!(entry.last_error.as_deref(), Some("invalid token"));
    }

    #[test]
    fn starting_again_clears_a_previous_error() {
        let (board, _path) = board("status-recover");
        board.set_state("qq", ChannelState::Error, Some("boom".into()));
        board.set_started("qq");
        let inner = board.lock();
        let entry = inner.channels.get("qq").unwrap();
        assert_eq!(entry.last_error, None);
        assert_eq!(entry.state, Some(ChannelState::Running));
    }

    #[test]
    fn periodic_flushes_are_rate_limited_but_forced_ones_are_not() {
        let (board, path) = board("status-flush");
        // A new board starts "due", so a counter update is written out by the
        // periodic check...
        board.count_inbound("telegram", 1);
        let flushed = board.flush_if_due().unwrap();
        assert!(flushed, "a due snapshot is written");
        assert!(path.exists());
        // ...and a second check inside the interval is a no-op.
        board.count_inbound("telegram", 2);
        assert!(!board.flush_if_due().unwrap(), "within the interval");
        // An explicit flush always writes.
        board.flush().unwrap();
    }

    #[test]
    fn a_clean_board_is_not_flushed_again() {
        let (board, _path) = board("status-clean");
        board.set_state("telegram", ChannelState::Running, None);
        // The state change published it, so nothing is left to flush.
        assert!(!board.flush_if_due().unwrap(), "nothing changed");
    }

    #[test]
    fn a_fresh_snapshot_is_reported_as_fresh() {
        let (board, path) = board("status-fresh");
        board.set_started("telegram");
        board.flush().unwrap();
        let snapshot = StatusSnapshot::load(&path);
        assert!(snapshot.is_fresh(Duration::from_secs(60)));
        assert!(!snapshot.is_fresh(Duration::ZERO) || snapshot.updated_unix.is_some());
    }

    #[test]
    fn writing_is_atomic_enough_to_leave_no_temp_file() {
        let (board, path) = board("status-atomic");
        board.flush().unwrap();
        assert!(path.exists());
        assert!(!path.with_extension("json.tmp").exists());
    }

    #[test]
    fn state_names_are_stable_strings() {
        assert_eq!(ChannelState::Disabled.as_str(), "disabled");
        assert_eq!(ChannelState::Starting.as_str(), "starting");
        assert_eq!(ChannelState::Running.as_str(), "running");
        assert_eq!(ChannelState::Error.as_str(), "error");
        assert_eq!(ChannelState::Unsupported.as_str(), "unsupported");
    }
}
