//! One PTY session: the child process, a bounded tail of its output, and the
//! set of attached viewers.
//!
//! The output model is deliberately small and transport-free (the same shape
//! opencode uses, adapted from UTF-16 strings to bytes):
//!
//! * `cursor` counts every output byte the session has ever produced;
//! * `buffer` retains the most recent [`BUFFER_LIMIT`] bytes, and
//!   `buffer_cursor` is the absolute offset of `buffer[0]`;
//! * an attach asks for the bytes after a cursor it already applied, so a view
//!   that reconnects after a reload resumes instead of re-rendering.
//!
//! Nothing here is persisted: the retained tail lives only for the lifetime of
//! the process, and the renderer keeps its own screen state.

use std::collections::HashMap;
use std::io::Read;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::time::{Duration, Instant};

use serde::Serialize;
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};

use super::protocol::{chunks, Meta};
use super::pty::{PtySession, SpawnRequest};

/// Retained output per session (opencode uses 2 MiB).
pub const BUFFER_LIMIT: usize = 2 * 1024 * 1024;

/// Output buffered for a viewer that has not `activate`d yet. A viewer that
/// falls further behind than this is cut loose with `SessionEvent::Lagged`
/// instead of growing memory without bound; it re-attaches and replays.
pub const SUBSCRIBER_LIMIT: usize = 1024 * 1024;

/// Longest we wait for the shell to report its exit status after the PTY
/// reaches EOF before recording the exit without a code.
const EXIT_WAIT: Duration = Duration::from_secs(2);

static NEXT_ID: AtomicU64 = AtomicU64::new(1);

/// Opaque, monotonically issued session id. Stable for the tab's lifetime and
/// never reused, so a stale client cannot address a different shell.
pub fn new_id() -> String {
    format!("term_{:08x}", NEXT_ID.fetch_add(1, Ordering::Relaxed))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    Running,
    Exited,
}

/// What the client sees about a session. Never contains output.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Info {
    pub id: String,
    pub thread_id: String,
    pub title: String,
    pub command: String,
    pub args: Vec<String>,
    pub cwd: String,
    pub status: Status,
    pub exit_code: Option<i32>,
    pub pid: Option<u32>,
    pub cols: u16,
    pub rows: u16,
}

/// What an attached viewer receives.
#[derive(Debug)]
pub enum SessionEvent {
    /// Raw PTY bytes in order.
    Data(Vec<u8>),
    /// The final control frame: the process exited. Delivered after every byte
    /// the session produced, so a viewer never has to guess whether more is
    /// coming.
    Exited(Box<Meta>),
    /// The viewer fell further behind than [`SUBSCRIBER_LIMIT`] and was
    /// detached. It must re-attach from its last applied cursor.
    Lagged,
}

struct Subscriber {
    tx: UnboundedSender<SessionEvent>,
    active: bool,
    pending: Vec<Vec<u8>>,
    pending_bytes: usize,
    end: Option<Box<Meta>>,
}

#[derive(Default)]
struct Inner {
    buffer: Vec<u8>,
    buffer_cursor: u64,
    cursor: u64,
    subscribers: HashMap<u64, Subscriber>,
    next_token: u64,
}

struct State {
    title: String,
    status: Status,
    exit_code: Option<i32>,
    cols: u16,
    rows: u16,
}

pub struct Session {
    info: Mutex<State>,
    inner: Mutex<Inner>,
    pty: Mutex<PtySession>,
    id: String,
    thread_id: String,
    command: String,
    args: Vec<String>,
    cwd: PathBuf,
}

/// A viewer's handle on a session: the replay it must apply before live output
/// starts, and the channel live output arrives on.
pub struct Attachment {
    /// Retained output after the requested cursor, already chunked to
    /// [`super::protocol::REPLAY_CHUNK`] frames.
    pub replay: Vec<Vec<u8>>,
    /// Absolute offset of the first replayed byte.
    pub start: u64,
    /// Absolute end offset to store and resume from.
    pub cursor: u64,
    /// Set when the process had already exited: the session replays its last
    /// screen and then ends, it does not stream.
    pub exit_code: Option<i32>,
    receiver: Option<UnboundedReceiver<SessionEvent>>,
    session: Arc<Session>,
    token: u64,
}

impl Attachment {
    /// Hand the queued live events to the caller. Called only after the replay
    /// and its control frame have been written, so the ordering the client
    /// observes is replay → control frame → live output. The receiver is moved
    /// out so the stream loop can borrow the attachment for writes at the same
    /// time as it awaits events.
    pub fn activate(&mut self) -> Option<UnboundedReceiver<SessionEvent>> {
        self.session.activate(self.token);
        self.receiver.take()
    }

    /// Stop receiving. Idempotent, and safe to call after the session is gone.
    pub fn detach(&mut self) {
        self.session.detach(self.token);
    }

    pub fn write(&self, bytes: &[u8]) -> Result<(), String> {
        self.session.write(bytes)
    }

    #[cfg(test)]
    pub fn receiver(&mut self) -> &mut UnboundedReceiver<SessionEvent> {
        self.receiver
            .as_mut()
            .expect("the receiver is available until `activate`")
    }
}

impl Session {
    /// Spawn a shell and start draining its PTY.
    #[allow(clippy::too_many_arguments)]
    pub fn spawn(
        thread_id: String,
        title: String,
        program: PathBuf,
        args: Vec<String>,
        cwd: PathBuf,
        cols: u16,
        rows: u16,
    ) -> Result<Arc<Session>, String> {
        let id = new_id();
        let pty = PtySession::spawn(&SpawnRequest {
            program: &program,
            args: &args,
            cwd: &cwd,
            cols,
            rows,
        })?;
        let mut reader = pty.reader()?;

        let session = Arc::new(Session {
            info: Mutex::new(State {
                title,
                status: Status::Running,
                exit_code: None,
                cols,
                rows,
            }),
            inner: Mutex::new(Inner::default()),
            pty: Mutex::new(pty),
            id: id.clone(),
            thread_id,
            command: program.to_string_lossy().into_owned(),
            args,
            cwd,
        });

        // One blocking reader thread per session. It is the only writer of
        // output: buffer, cursor and every subscriber see bytes in PTY order.
        let weak: Weak<Session> = Arc::downgrade(&session);
        let spawned = std::thread::Builder::new()
            .name(format!("pty-{id}"))
            .spawn(move || {
                let mut chunk = [0_u8; 8192];
                loop {
                    match reader.read(&mut chunk) {
                        Ok(0) => break,
                        Ok(n) => match weak.upgrade() {
                            Some(session) => session.on_data(&chunk[..n]),
                            None => return,
                        },
                        Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                        Err(_) => break,
                    }
                }
                if let Some(session) = weak.upgrade() {
                    session.on_eof();
                }
            });
        if let Err(error) = spawned {
            return Err(format!("SPAWN_FAILED: reader thread: {error}"));
        }

        Ok(session)
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn thread_id(&self) -> &str {
        &self.thread_id
    }

    pub fn info(&self) -> Info {
        let state = self.info.lock().unwrap();
        Info {
            id: self.id.clone(),
            thread_id: self.thread_id.clone(),
            title: state.title.clone(),
            command: self.command.clone(),
            args: self.args.clone(),
            cwd: self.cwd.to_string_lossy().into_owned(),
            status: state.status,
            exit_code: state.exit_code,
            pid: self.pty.lock().unwrap().pid(),
            cols: state.cols,
            rows: state.rows,
        }
    }

    pub fn is_running(&self) -> bool {
        self.info.lock().unwrap().status == Status::Running
    }

    /// Retitle and/or resize. Resizing a dead session is not an error: the
    /// viewer may be re-fitting a stale tab.
    pub fn update(&self, title: Option<String>, size: Option<(u16, u16)>) -> Info {
        if let Some(title) = title {
            let trimmed = title.trim();
            if !trimmed.is_empty() {
                self.info.lock().unwrap().title = trimmed.chars().take(120).collect();
            }
        }
        if let Some((cols, rows)) = size {
            let cols = cols.clamp(1, 1000);
            let rows = rows.clamp(1, 1000);
            {
                let mut state = self.info.lock().unwrap();
                state.cols = cols;
                state.rows = rows;
            }
            if self.is_running() {
                let _ = self.pty.lock().unwrap().resize(cols, rows);
            }
        }
        self.info()
    }

    pub fn write(&self, bytes: &[u8]) -> Result<(), String> {
        if !self.is_running() {
            return Err("TERMINAL_CLOSED: the shell has exited".to_string());
        }
        self.pty
            .lock()
            .unwrap()
            .write(bytes)
            .map_err(|error| format!("PTY_IO_FAILED: write: {error}"))
    }

    /// Register a viewer. `cursor` semantics match opencode:
    /// `None` replays everything retained, `-1` tails from the current end,
    /// and `Some(n)` replays everything after `n`.
    pub fn attach(self: &Arc<Self>, cursor: Option<i64>) -> Attachment {
        let token;
        let (tx, rx) = unbounded_channel();
        // Read the process state *before* taking the output lock: the reader
        // thread and `on_eof` take these two locks in the opposite order, so
        // nesting them here would be a deadlock, not a race.
        let (already_exited, exit_code) = {
            let state = self.info.lock().unwrap();
            (state.status == Status::Exited, state.exit_code)
        };
        let (replay, start, end) = {
            let mut inner = self.inner.lock().unwrap();
            // Register *before* releasing the lock: output produced from here on
            // is either in the replay snapshot or in the queue, never lost.
            inner.next_token += 1;
            token = inner.next_token;
            inner.subscribers.insert(
                token,
                Subscriber {
                    tx,
                    active: false,
                    pending: Vec::new(),
                    pending_bytes: 0,
                    end: None,
                },
            );

            let end = inner.cursor;
            let from = match cursor {
                None => 0,
                Some(-1) => end,
                Some(value) => u64::try_from(value).unwrap_or(0),
            };
            let start = inner.buffer_cursor;
            let offset = from.saturating_sub(start) as usize;
            let replay_slice = if from >= end || offset >= inner.buffer.len() {
                &[][..]
            } else {
                &inner.buffer[offset..]
            };
            let replay_start = start + offset as u64;
            let replay: Vec<Vec<u8>> = chunks(replay_slice).iter().map(|c| c.to_vec()).collect();
            (replay, replay_start, end)
        };

        if already_exited {
            // Nothing more will arrive; the viewer gets the final screen and an
            // immediate end event instead of a socket that never speaks again.
            let meta = Box::new(Meta {
                cursor: end,
                start,
                exit_code,
            });
            let mut inner = self.inner.lock().unwrap();
            if let Some(subscriber) = inner.subscribers.remove(&token) {
                let _ = subscriber.tx.send(SessionEvent::Exited(meta));
            }
        }

        Attachment {
            replay,
            start,
            cursor: end,
            exit_code,
            receiver: Some(rx),
            session: Arc::clone(self),
            token,
        }
    }

    fn activate(&self, token: u64) {
        // Drain and send under one lock: releasing it between the two would let
        // fresh output overtake the buffered pre-activation bytes. The channel
        // is unbounded, so `send` here never blocks on the viewer.
        let mut inner = self.inner.lock().unwrap();
        let Some(subscriber) = inner.subscribers.get_mut(&token) else {
            return;
        };
        subscriber.active = true;
        for chunk in subscriber.pending.drain(..) {
            let _ = subscriber.tx.send(SessionEvent::Data(chunk));
        }
        subscriber.pending_bytes = 0;
        if let Some(end) = subscriber.end.take() {
            let _ = subscriber.tx.send(SessionEvent::Exited(end));
        }
    }

    fn detach(&self, token: u64) {
        let mut inner = self.inner.lock().unwrap();
        if let Some(mut subscriber) = inner.subscribers.remove(&token) {
            subscriber.pending.clear();
            subscriber.end = None;
        }
    }

    /// Append output and fan it out. Called only from the session's reader
    /// thread, so the byte order is exactly the PTY's.
    fn on_data(&self, bytes: &[u8]) {
        let mut inner = self.inner.lock().unwrap();
        inner.cursor += bytes.len() as u64;
        inner.buffer.extend_from_slice(bytes);
        if inner.buffer.len() > BUFFER_LIMIT {
            let excess = inner.buffer.len() - BUFFER_LIMIT;
            inner.buffer.drain(..excess);
            inner.buffer_cursor += excess as u64;
        }

        let mut lagged: Vec<u64> = Vec::new();
        for (token, subscriber) in inner.subscribers.iter_mut() {
            if subscriber.active {
                if subscriber
                    .tx
                    .send(SessionEvent::Data(bytes.to_vec()))
                    .is_err()
                {
                    lagged.push(*token);
                }
                continue;
            }
            subscriber.pending_bytes += bytes.len();
            if subscriber.pending_bytes > SUBSCRIBER_LIMIT {
                // The viewer is not keeping up on its own; drop it and let it
                // replay from its cursor rather than buffer without bound.
                lagged.push(*token);
                continue;
            }
            subscriber.pending.push(bytes.to_vec());
        }
        for token in lagged {
            if let Some(subscriber) = inner.subscribers.remove(&token) {
                let _ = subscriber.tx.send(SessionEvent::Lagged);
            }
        }
    }

    /// The PTY reached EOF: record the exit and tell every viewer exactly once.
    fn on_eof(&self) {
        let exit_code = {
            let deadline = Instant::now() + EXIT_WAIT;
            let mut code = None;
            loop {
                if let Some(status) = self.pty.lock().unwrap().try_wait() {
                    code = Some(status);
                    break;
                }
                if Instant::now() >= deadline {
                    break;
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            code
        };

        let meta = {
            let mut state = self.info.lock().unwrap();
            if state.status == Status::Exited {
                return;
            }
            state.status = Status::Exited;
            state.exit_code = exit_code;
            let inner = self.inner.lock().unwrap();
            Box::new(Meta {
                cursor: inner.cursor,
                start: inner.cursor,
                exit_code,
            })
        };

        let mut inner = self.inner.lock().unwrap();
        let subscribers: Vec<u64> = inner.subscribers.keys().copied().collect();
        for token in subscribers {
            let Some(subscriber) = inner.subscribers.get_mut(&token) else {
                continue;
            };
            if subscriber.active {
                let _ = subscriber.tx.send(SessionEvent::Exited(meta.clone()));
            } else {
                subscriber.end = Some(meta.clone());
            }
        }
    }

    /// Terminate the shell, wait (bounded) for the tree to die, and end every
    /// attachment. Removing the session from the registry is the caller's job.
    pub fn close(&self, grace: Duration) {
        {
            let mut pty = self.pty.lock().unwrap();
            pty.kill_tree(grace);
        }
        // Bound the guard: an `if` condition's temporary lives to the end of the
        // whole `if` in edition 2021, and `on_eof` locks the same mutex.
        let running = { self.info.lock().unwrap().status == Status::Running };
        if running {
            self.on_eof();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn spawn(cursor: u16) -> Arc<Session> {
        Session::spawn(
            "thread-1".to_string(),
            "Terminal 1".to_string(),
            PathBuf::from("/bin/sh"),
            vec!["-c".to_string(), "printf 'abc'; sleep 30".to_string()],
            std::env::temp_dir(),
            cursor,
            24,
        )
        .expect("spawn")
    }

    fn drain_until(
        events: &mut UnboundedReceiver<SessionEvent>,
        needle: &str,
        timeout: Duration,
    ) -> (Vec<u8>, Option<Box<Meta>>) {
        let deadline = Instant::now() + timeout;
        let mut bytes = Vec::new();
        let mut meta = None;
        while Instant::now() < deadline {
            match events.try_recv() {
                Ok(SessionEvent::Data(chunk)) => bytes.extend_from_slice(&chunk),
                Ok(SessionEvent::Exited(end)) => {
                    meta = Some(end);
                    break;
                }
                Ok(SessionEvent::Lagged) => break,
                Err(tokio::sync::mpsc::error::TryRecvError::Empty) => {
                    if !bytes.is_empty() && needle.is_empty() {
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(10));
                }
                Err(_) => break,
            }
        }
        (bytes, meta)
    }

    #[cfg(unix)]
    #[test]
    fn attach_replays_retained_output_and_reports_the_cursor() {
        let session = spawn(80);
        // Wait until the shell produced its output.
        let deadline = Instant::now() + Duration::from_secs(5);
        while session.inner.lock().unwrap().cursor < 3 && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        let mut attachment = session.attach(None);
        assert_eq!(attachment.start, 0);
        assert!(
            attachment.cursor >= 3,
            "cursor tracks every byte: {}",
            attachment.cursor
        );
        let replay: Vec<u8> = attachment.replay.concat();
        assert!(replay.starts_with(b"abc"), "replay: {replay:?}");
        attachment.detach();
        session.close(Duration::from_millis(200));
    }

    #[cfg(unix)]
    #[test]
    fn resume_after_a_cursor_replays_only_the_new_bytes() {
        let session = spawn(80);
        let deadline = Instant::now() + Duration::from_secs(5);
        while session.inner.lock().unwrap().cursor < 3 && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        // A viewer that already applied the first byte asks for the rest.
        let mut attachment = session.attach(Some(1));
        assert_eq!(
            attachment.start, 1,
            "replay starts where the client stopped"
        );
        let replay: Vec<u8> = attachment.replay.concat();
        assert_eq!(replay, b"bc", "replay must resume, not re-render");
        attachment.detach();
        session.close(Duration::from_millis(200));
    }

    #[cfg(unix)]
    #[test]
    fn tail_cursor_skips_history() {
        let session = spawn(80);
        let deadline = Instant::now() + Duration::from_secs(5);
        while session.inner.lock().unwrap().cursor < 3 && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        let attachment = session.attach(Some(-1));
        assert!(attachment.replay.is_empty(), "tail must not replay history");
        assert_eq!(attachment.start, attachment.cursor);
        session.close(Duration::from_millis(200));
    }

    #[cfg(unix)]
    #[test]
    fn a_cursor_older_than_the_buffer_reports_a_truncated_replay() {
        let session = spawn(80);
        let deadline = Instant::now() + Duration::from_secs(5);
        while session.inner.lock().unwrap().cursor < 3 && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        {
            let mut inner = session.inner.lock().unwrap();
            // Simulate trimming: pretend only the last byte is retained.
            let drop_to = inner.buffer.len() - 1;
            inner.buffer.drain(..drop_to);
            inner.buffer_cursor += drop_to as u64;
        }
        let attachment = session.attach(Some(0));
        assert!(
            attachment.start > 0,
            "a truncated replay must not claim to start at 0"
        );
        assert_eq!(attachment.replay.concat(), b"c");
        session.close(Duration::from_millis(200));
    }

    #[cfg(unix)]
    #[test]
    fn live_output_reaches_an_activated_viewer() {
        let session = Session::spawn(
            "thread-1".to_string(),
            "Terminal 1".to_string(),
            PathBuf::from("/bin/sh"),
            vec![
                "-c".to_string(),
                "printf 'first'; sleep 1; printf 'second'; sleep 30".to_string(),
            ],
            std::env::temp_dir(),
            80,
            24,
        )
        .expect("spawn");

        let deadline = Instant::now() + Duration::from_secs(5);
        while session.inner.lock().unwrap().cursor < 5 && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        let mut attachment = session.attach(Some(-1));
        let mut events = attachment.activate().expect("activated");
        let (bytes, _) = drain_until(&mut events, "second", Duration::from_secs(5));
        let text = String::from_utf8_lossy(&bytes).into_owned();
        assert!(text.contains("second"), "live output: {text:?}");
        attachment.detach();
        session.close(Duration::from_millis(200));
    }

    #[cfg(unix)]
    #[test]
    fn exit_is_delivered_after_the_last_output_byte() {
        let session = Session::spawn(
            "thread-1".to_string(),
            "Terminal 1".to_string(),
            PathBuf::from("/bin/sh"),
            vec!["-c".to_string(), "printf 'bye'; exit 3".to_string()],
            std::env::temp_dir(),
            80,
            24,
        )
        .expect("spawn");
        let mut attachment = session.attach(None);
        let mut events = attachment.activate().expect("activated");
        let (bytes, meta) = drain_until(&mut events, "", Duration::from_secs(10));
        let text = String::from_utf8_lossy(&bytes).into_owned();
        assert!(text.contains("bye"), "output before exit: {text:?}");
        let meta = meta.expect("an exit event must be delivered");
        assert_eq!(meta.exit_code, Some(3));
        assert!(!session.is_running());
        assert_eq!(session.info().exit_code, Some(3));
    }

    #[cfg(unix)]
    #[test]
    fn attaching_to_an_exited_session_replays_its_last_screen() {
        let session = Session::spawn(
            "thread-1".to_string(),
            "Terminal 1".to_string(),
            PathBuf::from("/bin/sh"),
            vec!["-c".to_string(), "printf 'gone'; exit 0".to_string()],
            std::env::temp_dir(),
            80,
            24,
        )
        .expect("spawn");
        let deadline = Instant::now() + Duration::from_secs(10);
        while session.is_running() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(!session.is_running(), "session must have exited");
        let mut attachment = session.attach(None);
        assert_eq!(attachment.exit_code, Some(0));
        let replay = String::from_utf8_lossy(&attachment.replay.concat()).into_owned();
        assert!(replay.contains("gone"), "final screen: {replay:?}");
        let receiver = attachment.receiver();
        let event = receiver.try_recv().expect("immediate end event");
        assert!(matches!(event, SessionEvent::Exited(_)));
    }

    #[cfg(unix)]
    #[test]
    fn detach_stops_delivery_without_touching_the_shell() {
        let session = spawn(80);
        let mut attachment = session.attach(Some(-1));
        let mut events = attachment.activate().expect("activated");
        attachment.detach();
        // A detached viewer must not receive further output.
        assert!(
            session.is_running(),
            "a detached viewer must not kill the shell"
        );
        session
            .write(b"echo still-here\n")
            .expect("write after detach");
        std::thread::sleep(Duration::from_millis(50));
        assert!(
            events.try_recv().is_err(),
            "no events may arrive after detach"
        );
        session.close(Duration::from_millis(200));
        assert!(!session.is_running());
    }

    #[cfg(unix)]
    #[test]
    fn a_lagging_inactive_viewer_is_cut_loose_instead_of_buffering_forever() {
        let session = spawn(80);
        let mut attachment = session.attach(None);
        // Simulate a viewer that never activates while output keeps arriving.
        {
            let mut inner = session.inner.lock().unwrap();
            let subscriber = inner
                .subscribers
                .get_mut(&attachment.token)
                .expect("subscriber");
            subscriber.pending_bytes = SUBSCRIBER_LIMIT + 1;
        }
        session.on_data(b"x");
        let events = attachment.receiver();
        let event = events.try_recv().expect("lagged event");
        assert!(matches!(event, SessionEvent::Lagged));
        session.close(Duration::from_millis(200));
    }

    #[cfg(unix)]
    #[test]
    fn write_to_an_exited_session_is_rejected() {
        let session = Session::spawn(
            "thread-1".to_string(),
            "Terminal 1".to_string(),
            PathBuf::from("/bin/sh"),
            vec!["-c".to_string(), "exit 0".to_string()],
            std::env::temp_dir(),
            80,
            24,
        )
        .expect("spawn");
        let deadline = Instant::now() + Duration::from_secs(5);
        while session.is_running() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
        }
        let error = session.write(b"ls\n").expect_err("must reject");
        assert!(error.starts_with("TERMINAL_CLOSED"), "{error}");
    }

    #[test]
    fn ids_are_unique_and_prefixed() {
        let a = new_id();
        let b = new_id();
        assert_ne!(a, b);
        assert!(a.starts_with("term_"));
        assert!(Path::new(&a).is_relative());
    }
}
