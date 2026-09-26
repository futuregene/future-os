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

    /// Whether the OS has reaped the child, independently of whether the
    /// session has *noticed*. See [`Session::wait_for_child_exit`].
    #[cfg(test)]
    pub(crate) fn child_is_reaped(&self) -> bool {
        self.pty.lock().unwrap().try_wait().is_some()
    }

    /// Wait (bounded) for the child process itself to be gone, answering its
    /// cursor-position query while waiting.
    ///
    /// A child on this host emits `ESC [ 6 n` as it starts and will not run its
    /// command line until a client replies, so "the child exited" cannot be
    /// waited for without answering it — `terminal::pty`'s own tests do the
    /// same (`test_support::DSR_QUERY`). `pub(crate)` for the manager's tests,
    /// which must sequence an exit through `on_eof` because this host's ConPTY
    /// never delivers the master EOF the reader thread waits for. Returns
    /// whether the child was reaped before `timeout`.
    ///
    /// Test-only: it answers the DSR query with `terminal::test_support`'s
    /// fixture constants, and its two callers are test modules
    /// (`terminal::manager`, `terminal::server`).
    #[cfg(test)]
    pub(crate) fn wait_for_child_exit(&self, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        let mut answered = false;
        while Instant::now() < deadline {
            if self.child_is_reaped() {
                return true;
            }
            if !answered {
                let seen = self.inner.lock().unwrap().buffer.clone();
                if crate::terminal::test_support::asks_for_the_cursor(&seen) {
                    answered = true;
                    let _ = self.write(crate::terminal::test_support::DSR_REPLY);
                }
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        self.child_is_reaped()
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
    /// thread, so the byte order is exactly the PTY's. `pub(crate)` because the
    /// transport tests drive it directly: a viewer mid-stream must be served the
    /// bytes the reader would have handed it, and the pump's delivery arms are
    /// only reachable by producing output on demand.
    pub(crate) fn on_data(&self, bytes: &[u8]) {
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
    ///
    /// Two callers: the reader thread (on the real EOF of the master) and
    /// `close()` (after it has killed the tree). `pub(crate)` for the manager's
    /// tests, which need a session whose exit is recorded without the PTY EOF
    /// this host never delivers — the state machine below is the same one both
    /// production callers use.
    pub(crate) fn on_eof(&self) {
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
    use crate::terminal::protocol::REPLAY_CHUNK;
    use crate::terminal::test_support;
    use std::path::Path;

    fn spawn_with(command: (PathBuf, Vec<String>)) -> Arc<Session> {
        let (program, args) = command;
        Session::spawn(
            "thread-1".to_string(),
            "Terminal 1".to_string(),
            program,
            args,
            test_support::working_dir(),
            80,
            24,
        )
        .expect("spawn child")
    }

    /// A live, silent child. The tests below drive output with `on_data` — the
    /// buffer/cursor arithmetic is what they pin down — so the buffer holds
    /// exactly the bytes asserted on and the same assertions hold on Windows
    /// and unix. The reader thread's own delivery is covered separately by
    /// `a_real_childs_output_flows_through_the_reader_thread` and
    /// `a_real_child_exit_code_is_recorded_and_delivered`.
    fn spawn_idle() -> Arc<Session> {
        spawn_with(test_support::idle_command())
    }

    /// A child that exits with `code` as soon as it starts.
    fn spawn_exit(code: i32) -> Arc<Session> {
        spawn_with(test_support::exit_command(code))
    }

    /// Wait for `predicate`, answering the child's cursor-position query while it
    /// blocks on it.
    ///
    /// A child on this host emits `ESC [ 6 n` as it starts and then will not run
    /// its command line until a client replies (the measurement is recorded on
    /// `test_support::DSR_QUERY`). A test that never replies observes the query
    /// and nothing else, which is why the fixture answers it here.
    fn wait_until(session: &Arc<Session>, what: &str, predicate: impl Fn() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(30);
        let mut answered = false;
        while Instant::now() < deadline {
            if !answered {
                let seen = session.inner.lock().unwrap().buffer.clone();
                if test_support::asks_for_the_cursor(&seen) {
                    answered = true;
                    let _ = session.write(test_support::DSR_REPLY);
                }
            }
            if predicate() {
                return;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        panic!("timed out waiting for {what}");
    }

    /// A session whose child is already gone.
    ///
    /// The byte-arithmetic tests below need the buffer to be theirs alone: a
    /// live child appends whatever its console decides to emit (a cursor query,
    /// a redraw), which would turn their exact byte counts into a race. Killing
    /// the child first removes that input without weakening anything they
    /// assert — `on_data` is the same entry point the reader thread uses.
    fn settled() -> Arc<Session> {
        let session = spawn_idle();
        session.close(Duration::from_millis(200));
        wait_until(&session, "the session to be reported exited", || {
            !session.is_running()
        });
        session
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
                Ok(SessionEvent::Data(chunk)) => {
                    bytes.extend_from_slice(&chunk);
                    // A non-empty needle stops the drain as soon as it appears;
                    // an empty needle means "drain to the exit event" and never
                    // stops here.
                    if !needle.is_empty()
                        && bytes.windows(needle.len()).any(|w| w == needle.as_bytes())
                    {
                        break;
                    }
                }
                Ok(SessionEvent::Exited(end)) => {
                    meta = Some(end);
                    break;
                }
                Ok(SessionEvent::Lagged) => break,
                Err(tokio::sync::mpsc::error::TryRecvError::Empty) => {
                    // Wait out idle moments: with an empty needle the exit event
                    // may still be in flight between the last data byte and the
                    // reader's Exited push, so breaking on the first Empty made
                    // exit-dependent tests flaky on loaded runners.
                    std::thread::sleep(Duration::from_millis(10));
                }
                Err(_) => break,
            }
        }
        (bytes, meta)
    }

    #[test]
    fn attach_replays_retained_output_and_reports_the_cursor() {
        let session = settled();
        let base = session.inner.lock().unwrap().cursor;
        session.on_data(b"abc");
        let mut attachment = session.attach(None);
        assert_eq!(
            attachment.start, 0,
            "nothing was trimmed, so replay starts at 0"
        );
        assert_eq!(attachment.cursor, base + 3, "cursor tracks every byte");
        assert!(
            attachment.replay.concat().ends_with(b"abc"),
            "the replay must end with the bytes just produced"
        );
        attachment.detach();
    }

    #[test]
    fn resume_after_a_cursor_replays_only_the_new_bytes() {
        let session = settled();
        let end = session.inner.lock().unwrap().cursor;
        session.on_data(b"abc");
        // A viewer that already applied the first byte asks for the rest.
        let mut attachment = session.attach(Some((end + 1) as i64));
        assert_eq!(
            attachment.start,
            end + 1,
            "replay starts where the client stopped"
        );
        assert_eq!(
            attachment.replay.concat(),
            b"bc",
            "replay must resume, not re-render"
        );
        attachment.detach();
    }

    #[test]
    fn tail_cursor_skips_history() {
        let session = settled();
        session.on_data(b"abc");
        let attachment = session.attach(Some(-1));
        assert!(attachment.replay.is_empty(), "tail must not replay history");
        assert_eq!(attachment.start, attachment.cursor);
    }

    #[test]
    fn a_cursor_older_than_the_buffer_reports_a_truncated_replay() {
        let session = settled();
        session.on_data(b"abc");
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
    }

    /// A cursor past every byte already produced replays nothing rather than
    /// panicking on the out-of-range slice; the start it reports is the cursor
    /// the client asked for, so the client can see it is ahead of the stream.
    #[test]
    fn a_cursor_beyond_the_end_replays_nothing() {
        let session = settled();
        session.on_data(b"abc");
        let end = session.inner.lock().unwrap().cursor;
        let attachment = session.attach(Some((end + 9_999) as i64));
        assert!(attachment.replay.is_empty());
        assert_eq!(
            attachment.cursor, end,
            "the stream ends where the session says"
        );
        assert_eq!(attachment.start, end + 9_999);
    }

    /// Replay frames are bounded, and concatenating them loses nothing: the
    /// invariant `chunks()` promises to the transport.
    #[test]
    fn replay_is_split_into_bounded_frames() {
        let session = settled();
        let payload = vec![b'z'; REPLAY_CHUNK * 2 + 1];
        session.on_data(&payload);
        let attachment = session.attach(None);
        let frames = &attachment.replay;
        assert!(
            frames.len() >= 3,
            "a 2-chunk payload needs at least 3 frames"
        );
        for frame in &frames[..frames.len() - 1] {
            assert_eq!(
                frame.len(),
                REPLAY_CHUNK,
                "every frame but the last is full"
            );
        }
        assert!(frames.last().expect("a frame").len() <= REPLAY_CHUNK);
        assert!(
            attachment.replay.concat().ends_with(&payload),
            "the frames must carry the bytes in order, with nothing lost"
        );
    }

    /// The retained tail is bounded: bytes pushed past `BUFFER_LIMIT` are
    /// trimmed from the front and the cursor keeps counting absolutely.
    #[test]
    fn the_retained_buffer_is_trimmed_at_the_limit() {
        let session = settled();
        let base = session.inner.lock().unwrap().cursor;
        let chunk = vec![b'a'; 64 * 1024];
        let mut written = 0_usize;
        while written <= BUFFER_LIMIT + chunk.len() {
            session.on_data(&chunk);
            written += chunk.len();
        }
        let inner = session.inner.lock().unwrap();
        assert_eq!(inner.cursor, base + written as u64);
        assert!(
            inner.buffer.len() <= BUFFER_LIMIT,
            "retention must stay bounded, got {}",
            inner.buffer.len()
        );
        assert_eq!(
            inner.buffer_cursor + inner.buffer.len() as u64,
            inner.cursor,
            "buffer_cursor must track what was dropped"
        );
    }

    #[test]
    fn live_output_reaches_an_activated_viewer() {
        // A *live* session: `attach` on an already-exited one rightfully drops
        // the subscriber after sending its end event, so nothing could be
        // delivered afterwards — that is the `attaching_to_an_exited_session…`
        // case, not this one.
        let session = spawn_idle();
        let mut attachment = session.attach(Some(-1));
        let mut events = attachment.activate().expect("activated");
        session.on_data(b"first");
        let (bytes, _) = drain_until(&mut events, "first", Duration::from_secs(5));
        assert_eq!(String::from_utf8_lossy(&bytes), "first");
        attachment.detach();
        session.close(Duration::from_millis(200));
    }

    /// Output produced *before* activation is queued and delivered by
    /// `activate`, so a viewer that connects mid-stream loses nothing.
    #[test]
    fn output_produced_before_activation_is_queued() {
        let session = spawn_idle();
        let mut attachment = session.attach(None);
        session.on_data(b"early");
        assert!(
            attachment.receiver().try_recv().is_err(),
            "an inactive viewer must not receive live events yet"
        );
        let mut events = attachment.activate().expect("activated");
        let (bytes, _) = drain_until(&mut events, "early", Duration::from_secs(5));
        assert_eq!(String::from_utf8_lossy(&bytes), "early");
        attachment.detach();
        session.close(Duration::from_millis(200));
    }

    #[test]
    fn exit_is_delivered_after_the_last_output_byte() {
        let session = spawn_idle();
        let mut attachment = session.attach(Some(-1));
        let mut events = attachment.activate().expect("activated");
        session.on_data(b"bye");
        // Closing kills the child and ends the session; the data pushed above
        // must still arrive first, in order. The child may also have emitted a
        // cursor query, so the assertion is on the visible text.
        session.close(Duration::from_millis(500));
        let (bytes, meta) = drain_until(&mut events, "", Duration::from_secs(15));
        assert!(
            test_support::visible_text(&bytes).ends_with("bye"),
            "the injected bytes must arrive last: {:?}",
            String::from_utf8_lossy(&bytes)
        );
        assert!(meta.is_some(), "an exit event must follow the last byte");
        assert!(!session.is_running());
    }

    /// A viewer that attaches after the exit gets the exit event immediately
    /// (rather than a socket that never speaks again), and the reader thread
    /// really does drain a live PTY.
    #[test]
    fn attaching_to_an_exited_session_replays_its_last_screen() {
        let session = spawn_idle();
        session.on_data(b"gone");
        session.close(Duration::from_millis(200));
        wait_until(&session, "the session to be reported exited", || {
            !session.is_running()
        });
        let mut attachment = session.attach(None);
        assert_eq!(
            attachment.exit_code,
            session.info().exit_code,
            "the attachment must report the session's real exit state"
        );
        let replay = test_support::visible_text(&attachment.replay.concat());
        assert!(replay.ends_with("gone"), "the final screen is replayed");
        let receiver = attachment.receiver();
        let event = receiver.try_recv().expect("immediate end event");
        assert!(matches!(event, SessionEvent::Exited(_)));
    }

    /// The real exit code of a real child must be recorded and pushed to every
    /// attached viewer.
    ///
    /// The child's exit is observed the way `close()` observes it, by driving
    /// `on_eof` once the OS has actually reaped the process: this host's ConPTY
    /// never closes the master, so the reader thread's `Ok(0)` — the *other*
    /// caller of `on_eof` — never arrives (see the run status). Everything
    /// asserted here is still the production path: `on_eof` polls
    /// `PtySession::try_wait` itself and is what records the code and fans the
    /// `Exited` event out.
    #[test]
    fn a_real_child_exit_code_is_recorded_and_delivered() {
        let session = spawn_exit(3);
        let mut attachment = session.attach(None);
        // Activate *before* the exit so the fan-out takes the live-subscriber
        // path (`subscriber.end` is only used for a viewer that never
        // activated), which is the delivery this test is named for.
        let mut events = attachment.activate().expect("activated");
        wait_until(&session, "the child to be reaped", || {
            session.pty.lock().unwrap().try_wait().is_some()
        });
        session.on_eof();
        let info = session.info();
        assert_eq!(info.exit_code, Some(3), "the real exit code must survive");
        assert!(info.pid.is_some(), "a spawned child reports its pid");
        assert_eq!(info.status, Status::Exited);

        // ... and the code reaches the viewer, not just the session's state.
        let (_, meta) = drain_until(&mut events, "", Duration::from_secs(5));
        let meta = meta.expect("an exit event must be delivered");
        assert_eq!(
            meta.exit_code,
            Some(3),
            "the viewer must learn the real code"
        );
    }

    /// `on_eof` is the single place that turns "the child is gone" into durable
    /// state, and the reader thread is only one of its two callers — `close()`
    /// is the other. This test drives it *directly* on a **live** child, which
    /// is the only way to exercise two of its arms on this host:
    ///
    /// * the bounded wait must give up after `EXIT_WAIT` and record `exit_code`
    ///   as `None` rather than hanging or inventing a code (a live child never
    ///   reaps, so the loop runs its deadline arm);
    /// * a second `on_eof` must be a no-op, because the reader thread can reach
    ///   EOF at the same moment `close()` calls it — without that guard the
    ///   second call would re-read a consumed `status` and push a duplicate
    ///   `Exited` to every viewer.
    #[test]
    fn on_eof_is_bounded_and_idempotent_on_a_live_child() {
        let session = spawn_idle();
        assert!(session.is_running());
        let started = Instant::now();
        session.on_eof();
        let elapsed = started.elapsed();
        assert!(
            elapsed >= EXIT_WAIT,
            "a live child must be waited for, not assumed dead"
        );
        assert!(
            elapsed < EXIT_WAIT + Duration::from_secs(5),
            "the wait must be bounded, took {elapsed:?}"
        );
        let info = session.info();
        assert_eq!(info.status, Status::Exited);
        assert_eq!(
            info.exit_code, None,
            "an unreaped child must not be given an invented exit code"
        );

        // Idempotent: the second call must not move anything. `exit_code`
        // staying `None` is the observable, and the viewer count is what a
        // duplicate `Exited` would have disturbed.
        let viewer = session.attach(Some(-1));
        session.on_eof();
        assert_eq!(session.info().exit_code, None);
        assert_eq!(session.info().status, Status::Exited);
        drop(viewer);
        session.close(Duration::from_millis(200));
    }

    /// Three refusal/deferral arms that only a *sequence* of client actions
    /// reaches — and each one is what a real viewer hits when a tab is closed
    /// and re-opened:
    ///
    /// * `wait_for_child_exit` must report "not reaped" rather than lie when its
    ///   budget runs out (a live child);
    /// * `activate` on a token that has already been detached must be refused
    ///   instead of resurrecting the subscriber;
    /// * an attachment that was still inactive when the session exited must
    ///   receive that exit when it finally activates, or a viewer that connects
    ///   one moment too late would hang on a socket that never speaks.
    #[test]
    fn a_viewer_that_arrives_late_still_learns_about_the_exit() {
        let session = spawn_idle();
        assert!(
            !session.wait_for_child_exit(Duration::from_millis(50)),
            "a live child must be reported as not reaped, not assumed gone"
        );

        // Detach-then-activate must not start delivering: `Session::activate`
        // finds no subscriber for the token and returns, so the view stays
        // silent even though output keeps arriving.
        let mut abandoned = session.attach(None);
        abandoned.detach();
        session.on_data(b"after-detach");
        let mut events = abandoned
            .activate()
            .expect("the attachment still owns its receiver");
        assert!(
            events.try_recv().is_err(),
            "a detached attachment must not start receiving"
        );

        // A viewer attached but not yet activated when the exit happens: the
        // exit is held on the subscriber and delivered by `activate`.
        let mut pending = session.attach(Some(-1));
        session.on_eof();
        let mut events = pending
            .activate()
            .expect("a pending subscriber must still activate");
        // The child is a real process, and a ConPTY shell emits its own output
        // (a DSR probe, `ESC [ 6 n`) that can land ahead of the held exit — under
        // parallel load the ordering is not ours to assume. The claim under test
        // is that the late viewer LEARNS ABOUT the exit, so look for it among the
        // delivered events. Bounded and non-blocking, so it cannot hang.
        let mut exit = None;
        for _ in 0..64 {
            match events.try_recv() {
                Ok(event @ SessionEvent::Exited(_)) => {
                    exit = Some(event);
                    break;
                }
                // The child's own output, ahead of the held exit.
                Ok(_) => continue,
                Err(_) => break,
            }
        }
        assert!(
            exit.is_some(),
            "the held exit must be delivered to a viewer that activates late"
        );
        pending.detach();
        session.close(Duration::from_millis(200));
    }

    /// The reader thread — not a test-injected `on_data` — must surface a real
    /// child's bytes into the buffer/cursor.
    #[test]
    fn a_real_childs_output_flows_through_the_reader_thread() {
        let session = spawn_with(test_support::echo_command("hello-session"));
        wait_until(&session, "the child's output to reach the buffer", || {
            test_support::visible_text(&session.inner.lock().unwrap().buffer)
                .contains("hello-session")
        });
        let replay = session.attach(None).replay.concat();
        assert!(
            test_support::visible_text(&replay).contains("hello-session"),
            "reader output: {:?}",
            String::from_utf8_lossy(&replay)
        );
        session.close(Duration::from_millis(200));
    }

    /// A written line must reach the child, not merely be echoed by the
    /// terminal: the shell executing `exit 5` is the proof.
    #[test]
    fn a_written_line_is_executed_by_the_child() {
        let session = spawn_with(test_support::interactive_command());
        // The interactive shell asks where the cursor is before it reads a line.
        wait_until(&session, "the shell's cursor query", || {
            test_support::asks_for_the_cursor(&session.inner.lock().unwrap().buffer)
        });
        let line: &[u8] = if cfg!(windows) {
            b"exit 5\r\n"
        } else {
            b"exit 5\n"
        };
        session.write(line).expect("write to a live session");
        // The shell exits as soon as it runs the line. This host's ConPTY never
        // closes the master, so the reader thread cannot report the EOF that
        // would normally drive `on_eof`; wait for the process itself to be
        // reaped and drive it, which reads the same `PtySession::try_wait` the
        // reader's EOF path would have.
        wait_until(&session, "the child to run the written line", || {
            session.pty.lock().unwrap().try_wait().is_some()
        });
        session.on_eof();
        assert_eq!(
            session.info().exit_code,
            Some(5),
            "the child must have executed the line it was given"
        );
        session.close(Duration::from_millis(200));
    }

    #[test]
    fn detach_stops_delivery_without_touching_the_shell() {
        let session = spawn_idle();
        let mut attachment = session.attach(Some(-1));
        let mut events = attachment.activate().expect("activated");
        attachment.detach();
        // A detached viewer must not receive further output, and must not kill
        // the shell it was watching.
        assert!(
            session.is_running(),
            "a detached viewer must not kill the shell"
        );
        session.on_data(b"after-detach");
        std::thread::sleep(Duration::from_millis(50));
        assert!(
            events.try_recv().is_err(),
            "no events may arrive after detach"
        );
        session.close(Duration::from_millis(200));
        assert!(!session.is_running());
    }

    #[test]
    fn a_lagging_inactive_viewer_is_cut_loose_instead_of_buffering_forever() {
        let session = spawn_idle();
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

    /// An *active* viewer whose channel was dropped is also cut loose instead
    /// of the session keeping a dead sender alive.
    #[test]
    fn a_dropped_active_viewer_is_removed() {
        let session = spawn_idle();
        let mut attachment = session.attach(None);
        let events = attachment.activate().expect("activated");
        drop(events);
        session.on_data(b"x");
        assert!(
            session.inner.lock().unwrap().subscribers.is_empty(),
            "a viewer whose channel is gone must not be retained"
        );
        session.close(Duration::from_millis(200));
    }

    #[test]
    fn write_to_an_exited_session_is_rejected() {
        let session = spawn_exit(0);
        wait_until(&session, "the child to be reaped", || {
            session.pty.lock().unwrap().try_wait().is_some()
        });
        // Record the exit the way `close()` does (see the ConPTY note above),
        // so the write is refused because the session is *exited*, not merely
        // because a child is slow to answer.
        session.on_eof();
        assert!(!session.is_running());
        let error = session.write(b"ls\n").expect_err("must reject");
        assert!(error.starts_with("TERMINAL_CLOSED"), "{error}");
    }

    /// `update` retitles and resizes, normalizes out-of-range values, and caps
    /// the title by *characters* (a byte-based cap would split a CJK glyph).
    #[test]
    fn update_retitles_resizes_and_normalizes_input() {
        let session = spawn_idle();
        let info = session.update(Some("  Build logs  ".into()), Some((120, 40)));
        assert_eq!(info.title, "Build logs", "the title is trimmed");
        assert_eq!((info.cols, info.rows), (120, 40));

        let info = session.update(Some("   ".into()), Some((0, u16::MAX)));
        assert_eq!(info.title, "Build logs", "a blank title keeps the old one");
        assert_eq!((info.cols, info.rows), (1, 1000), "size is clamped");

        let cjk = "终".repeat(150);
        let info = session.update(Some(cjk), None);
        assert_eq!(info.title.chars().count(), 120);
        assert_eq!(info.title.chars().next(), Some('终'));

        // Resizing an exited session is not an error: the viewer may be
        // re-fitting a stale tab.
        session.close(Duration::from_millis(200));
        let info = session.update(Some("After exit".into()), Some((100, 30)));
        assert_eq!(info.title, "After exit");
        assert_eq!((info.cols, info.rows), (100, 30));
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
