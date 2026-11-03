//! Structured, component-separated Rust diagnostics. Entry points install one
//! subscriber; an owned file guard is opened only after acquiring instance ownership.
mod file;
mod format;

use std::{
    collections::HashMap,
    io::{self, Write},
    path::{Path, PathBuf},
    sync::{Arc, OnceLock, RwLock},
};
use tracing::Metadata;
use tracing_appender::non_blocking::{NonBlocking, WorkerGuard};
use tracing_subscriber::{fmt::MakeWriter, prelude::*, EnvFilter};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Component {
    Desktop,
    Host,
    Agent,
    Channel,
    Tui,
    Cli,
}

impl Component {
    pub fn name(self) -> &'static str {
        match self {
            Self::Desktop => "desktop",
            Self::Host => "host",
            Self::Agent => "agent",
            Self::Channel => "channel",
            Self::Tui => "tui",
            Self::Cli => "cli",
        }
    }

    pub fn path(self, root: &Path) -> PathBuf {
        let directory = match self {
            Self::Desktop | Self::Host => "app",
            Self::Agent => "agent",
            Self::Channel => "channels",
            Self::Tui => "tui",
            Self::Cli => "cli",
        };
        root.join(directory)
            .join("logs")
            .join(format!("{}.log", self.name()))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Profile {
    Normal,
    Detailed,
}

impl Profile {
    /// Distribution identity, independent of Cargo optimisation/debug assertions.
    pub fn for_version(version: &str) -> Self {
        if version.starts_with('0') {
            Self::Detailed
        } else {
            Self::Normal
        }
    }

    pub fn filter(self) -> String {
        let level = match self {
            Self::Normal => "info",
            Self::Detailed => "debug",
        };
        // Unrelated libraries default to WARN, including HTTP/TLS/NATS chatter.
        let mut filter = String::from("warn");
        for target in [
            "futureos_lib",
            "future_host",
            "future_agent",
            "future_channel",
            "future_tui",
            "future_cli",
            "future_logging",
            "future_remote",
        ] {
            filter.push_str(&format!(",{target}={level}"));
        }
        filter
    }
}

struct State {
    primary: Component,
    profile: Profile,
    version: String,
    instance: String,
    sinks: RwLock<HashMap<Component, Arc<NonBlocking>>>,
}

impl State {
    fn component(&self, metadata: &Metadata<'_>) -> Component {
        // Shared Agent-owned helpers can explicitly attribute App database
        // work to the Host. An explicit first-party target is authoritative.
        if metadata.target().starts_with("future_host")
            || metadata.target().starts_with("futureos_lib")
        {
            return self.primary;
        }
        let agent = metadata.target().starts_with("future_agent::")
            || metadata
                .module_path()
                .is_some_and(|path| path.starts_with("future_agent::"));
        // An attached client must not open or write the external Agent's file.
        if agent
            && self
                .sinks
                .read()
                .unwrap_or_else(|e| e.into_inner())
                .contains_key(&Component::Agent)
        {
            Component::Agent
        } else {
            self.primary
        }
    }
}

static STATE: OnceLock<Arc<State>> = OnceLock::new();

/// Prepare filtering/console output before runtime threads. No directories or
/// files are opened here. File failures must never prevent application startup.
pub fn prepare(component: Component, version: &str, console: bool, verbose: bool) {
    STATE.get_or_init(|| {
        let profile = if verbose {
            Profile::Detailed
        } else {
            Profile::for_version(version)
        };
        let state = Arc::new(State {
            primary: component,
            profile,
            version: version.to_owned(),
            instance: format!(
                "{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_nanos()
            ),
            sinks: RwLock::new(HashMap::new()),
        });
        let filter =
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(profile.filter()));
        let terminal = console.then(|| {
            tracing_subscriber::fmt::layer()
                .with_target(true)
                .with_writer(SafeStderr)
        });
        let files = tracing_subscriber::fmt::layer()
            .json()
            .event_format(format::EventFormat::new(state.clone()))
            .with_writer(Router(state.clone()));
        // A subscriber installed by an embedding caller remains authoritative.
        let _ = tracing_subscriber::registry()
            .with(filter)
            .with(terminal)
            .with(files)
            .try_init();
        state
    });
}

pub fn primary_component() -> Option<Component> {
    STATE.get().map(|state| state.primary)
}

/// Owns the worker through shutdown. Removing the sink precedes flushing it,
/// so a retired embedded Agent does not leave a writer holding its old file.
pub struct FileGuard {
    state: Arc<State>,
    component: Component,
    sink: Arc<NonBlocking>,
    _worker: WorkerGuard,
}

impl std::fmt::Debug for FileGuard {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("FileGuard")
            .field("component", &self.component)
            .finish_non_exhaustive()
    }
}

impl Drop for FileGuard {
    fn drop(&mut self) {
        let dropped_lines = self.sink.error_counter().dropped_lines();
        if dropped_lines > 0 {
            // Direct emergency output: reporting a full queue through itself
            // could lose this counter too. Never panic on an unavailable tty.
            let _ = writeln!(
                SafeStderr,
                "FutureOS: {} log queue dropped {dropped_lines} lines",
                self.component.name()
            );
        }
        let mut sinks = self.state.sinks.write().unwrap_or_else(|e| e.into_inner());
        if sinks
            .get(&self.component)
            .is_some_and(|sink| Arc::ptr_eq(sink, &self.sink))
        {
            sinks.remove(&self.component);
        }
    }
}

/// Register the owned component's JSONL file. `max_lines` preserves the Agent
/// CLI's explicit line budget; zero uses only the profile's bounded byte budget.
pub fn open(component: Component, path: &Path, max_lines: usize) -> io::Result<FileGuard> {
    let state = STATE
        .get()
        .ok_or_else(|| io::Error::other("logging is not prepared"))?
        .clone();
    open_with_state(state, component, path, max_lines)
}

fn open_with_state(
    state: Arc<State>,
    component: Component,
    path: &Path,
    max_lines: usize,
) -> io::Result<FileGuard> {
    let mut sinks = state.sinks.write().unwrap_or_else(|e| e.into_inner());
    if sinks.contains_key(&component) {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "component log is already open",
        ));
    }
    let mut actual_path = path.to_path_buf();
    let file = match file::RollingFile::open(path, state.profile, max_lines) {
        Err(error)
            if error.kind() == io::ErrorKind::WouldBlock
                || error.raw_os_error() == fs2::lock_contended_error().raw_os_error() =>
        {
            // Several TUI/CLI/Channel processes may coexist. They never rotate
            // another process's active file; an overlapping owner gets its own.
            let stem = path.file_stem().unwrap_or_default().to_string_lossy();
            actual_path.set_file_name(format!("{stem}.{}.log", std::process::id()));
            file::RollingFile::open(&actual_path, state.profile, max_lines)?
        }
        result => result?,
    };
    let (sink, worker) = tracing_appender::non_blocking::NonBlockingBuilder::default()
        .buffered_lines_limit(4096)
        .thread_name(&format!("{}-log", component.name()))
        .finish(file);
    let sink = Arc::new(sink);
    sinks.insert(component, sink.clone());
    drop(sinks);
    let guard = FileGuard {
        state,
        component,
        sink,
        _worker: worker,
    };
    if component == Component::Agent {
        tracing::info!(target: "future_agent::logging", path = %actual_path.display(),
            profile = ?guard.state.profile, event = "logging_started");
    } else {
        tracing::info!(component = component.name(), path = %actual_path.display(),
            profile = ?guard.state.profile, event = "logging_started");
    }
    Ok(guard)
}

#[derive(Clone)]
struct Router(Arc<State>);
impl<'a> MakeWriter<'a> for Router {
    type Writer = Box<dyn Write + 'a>;
    fn make_writer(&'a self) -> Self::Writer {
        Box::new(io::sink())
    }
    fn make_writer_for(&'a self, metadata: &Metadata<'_>) -> Self::Writer {
        let component = self.0.component(metadata);
        match self
            .0
            .sinks
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .get(&component)
        {
            Some(sink) => Box::new((**sink).clone()),
            None => Box::new(io::sink()),
        }
    }
}

#[derive(Clone, Copy)]
struct SafeStderr;
impl Write for SafeStderr {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let _ = io::stderr().write_all(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        let _ = io::stderr().flush();
        Ok(())
    }
}
impl<'a> MakeWriter<'a> for SafeStderr {
    type Writer = Self;
    fn make_writer(&'a self) -> Self {
        *self
    }
}

#[cfg(test)]
mod tests;
