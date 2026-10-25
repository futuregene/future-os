//! Spawning a child process with a deadline, draining both pipes while it runs.
//!
//! The clipboard reader (`paste`) is the one caller: `pbpaste`, `xclip` and the
//! macOS screenshot path are all "run a program, take its output, and never let
//! it wedge the terminal". The machinery lives here because it is a set of
//! defects waiting to be reintroduced by a second copy:
//!
//! * **Drain while waiting.** Reading only after the child exits deadlocks it on
//!   a full pipe once its output passes the OS buffer (~64 KiB) — a large
//!   clipboard is exactly that case.
//! * **Kill *and* reap.** A child that outlives the deadline must not become a
//!   zombie for the rest of the session.
//! * **Never inherit stdin.** The child shares our terminal, so an inherited
//!   raw-mode stdin would swallow the reader's keystrokes.
//! * **Cap the capture.** A runaway child must not be able to make the terminal
//!   allocate without bound.

use std::io::Read;
use std::path::Path;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::time::{Duration, Instant};

/// Prefix of the error raised when the deadline expired and the child had to be
/// killed. Callers may re-word it; the prefix is what identifies the cause.
pub const TIMEOUT_MARKER: &str = "timed out";

/// Output past this is a bug in the child, not a clipboard. Sized well above
/// any legitimate capture (16 MiB ≈ 12 MiB of base64 image) so the cap is a
/// guard rather than a limit someone hits.
const MAX_CAPTURED_BYTES: usize = 16 * 1024 * 1024;

/// Spawn `program args…` with `stdin` closed and wait for it, at most `timeout`.
/// Returns `(exit code, stdout, stderr)`; the code is `-1` for a child killed by
/// a signal.
pub(crate) fn spawn_with_timeout(
    program: &Path,
    args: &[String],
    timeout: Duration,
) -> Result<(i32, String, String), String> {
    let child = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|err| format!("failed to spawn {}: {err}", program.display()))?;
    wait_for(child, timeout)
}

/// Poll `child` until it exits *and* both pipes are closed, or `timeout`
/// expires. Output captured before a kill is discarded: a timeout is a failure,
/// not a partial success.
fn wait_for(mut child: Child, timeout: Duration) -> Result<(i32, String, String), String> {
    let mut pipes = CapturedPipes::start(&mut child);
    let deadline = Instant::now() + timeout;
    let mut code = None;
    loop {
        if let Some(error) = pipes.pump() {
            terminate(&mut child);
            return Err(error);
        }
        if code.is_none() {
            code = child
                .try_wait()
                .expect("a live child can always be polled")
                .map(|status| status_code(&status));
        }
        // Only the *pipes* can still be holding output once the child is gone.
        if let Some(code) = code.filter(|_| pipes.closed()) {
            return Ok(pipes.streams(code));
        }
        if Instant::now() >= deadline {
            terminate(&mut child);
            // A child that exited but left a descendant holding the pipe open is
            // not a timeout, and calling its (discarded) output "cut short"
            // would invent a result.
            return Err(if code.is_some() {
                format!(
                    "child exited but a descendant kept its output pipe open for {}s",
                    timeout.as_secs()
                )
            } else {
                format!("{TIMEOUT_MARKER} after {}s", timeout.as_secs())
            });
        }
        if let Some(message) = pipes.next_message(Duration::from_millis(2)) {
            pipes.accept(message);
        }
    }
}

/// Which of a child's two pipes a captured chunk came from.
#[derive(Clone, Copy)]
enum Stream {
    Stdout,
    Stderr,
}

/// One message from a pipe-draining thread to the waiter.
enum PipeMessage {
    /// Bytes just read from one stream.
    Data(Stream, Vec<u8>),
    /// That stream is closed; no further bytes will follow for it.
    Eof(Stream),
}

/// Both pipes, with one reader thread each, and what has arrived so far.
struct CapturedPipes {
    stdout: Vec<u8>,
    stderr: Vec<u8>,
    stdout_eof: bool,
    stderr_eof: bool,
    inbox: Receiver<PipeMessage>,
    readers: Vec<std::thread::JoinHandle<()>>,
}

impl CapturedPipes {
    fn start(child: &mut Child) -> Self {
        let (tx, inbox) = channel();
        let mut readers = Vec::new();
        if let Some(pipe) = child.stdout.take() {
            readers.push(drain(pipe, Stream::Stdout, tx.clone()));
        }
        if let Some(pipe) = child.stderr.take() {
            readers.push(drain(pipe, Stream::Stderr, tx.clone()));
        }
        Self {
            stdout: Vec::new(),
            stderr: Vec::new(),
            stdout_eof: false,
            stderr_eof: false,
            inbox,
            readers,
        }
    }

    /// Wait (bounded) for the next message.
    fn next_message(&self, timeout: Duration) -> Option<PipeMessage> {
        self.inbox.recv_timeout(timeout).ok()
    }

    /// Apply everything that has already arrived. `Some(error)` when either
    /// stream passed the capture cap.
    fn pump(&mut self) -> Option<String> {
        while let Ok(message) = self.inbox.try_recv() {
            self.accept(message);
        }
        if self.stdout.len() + self.stderr.len() > MAX_CAPTURED_BYTES {
            return Some(self.overflow_error());
        }
        None
    }

    /// Have both streams reached EOF?
    fn closed(&self) -> bool {
        self.stdout_eof && self.stderr_eof
    }

    /// Finish: join the readers and hand back the captured text.
    fn streams(mut self, code: i32) -> (i32, String, String) {
        while let Some(reader) = self.readers.pop() {
            let _ = reader.join();
        }
        (code, decode(self.stdout), decode(self.stderr))
    }

    /// Add one chunk to the stream it came from.
    fn accept(&mut self, message: PipeMessage) {
        match message {
            PipeMessage::Data(Stream::Stdout, chunk) => self.stdout.extend_from_slice(&chunk),
            PipeMessage::Data(Stream::Stderr, chunk) => self.stderr.extend_from_slice(&chunk),
            PipeMessage::Eof(Stream::Stdout) => self.stdout_eof = true,
            PipeMessage::Eof(Stream::Stderr) => self.stderr_eof = true,
        }
    }

    fn overflow_error(&self) -> String {
        format!(
            "child wrote more than the {MAX_CAPTURED_BYTES}-byte capture limit ({} bytes \
             captured); killed it instead of buffering the rest",
            self.stdout.len() + self.stderr.len()
        )
    }
}

/// One reader thread per pipe: `read` blocks, so the waiter cannot poll.
fn drain(
    mut pipe: impl Read + Send + 'static,
    stream: Stream,
    tx: Sender<PipeMessage>,
) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        let mut buffer = [0u8; 8192];
        loop {
            match pipe.read(&mut buffer) {
                Ok(0) => break,
                Ok(read) => {
                    if tx
                        .send(PipeMessage::Data(stream, buffer[..read].to_vec()))
                        .is_err()
                    {
                        return;
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(_) => break,
            }
        }
        let _ = tx.send(PipeMessage::Eof(stream));
    })
}

/// Kill a child and reap it.
fn terminate(child: &mut Child) {
    let _ = child.kill();
    let _ = child.wait();
}

/// Exit code, or `-1` when the child was killed by a signal (Unix) or the OS
/// reports none.
fn status_code(status: &ExitStatus) -> i32 {
    status.code().unwrap_or(-1)
}

/// Lossy UTF-8 for captured output: one broken byte must not discard the rest.
fn decode(bytes: Vec<u8>) -> String {
    String::from_utf8_lossy(&bytes).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sh_args(script: &str) -> Vec<String> {
        vec!["-c".to_string(), script.to_string()]
    }

    #[cfg(unix)]
    #[test]
    fn captures_stdout_and_the_exit_code() {
        let (code, stdout, stderr) = spawn_with_timeout(
            Path::new("sh"),
            &sh_args("printf 'hello'"),
            Duration::from_secs(10),
        )
        .expect("run sh");
        assert_eq!((code, stdout.as_str(), stderr.as_str()), (0, "hello", ""));
    }

    #[cfg(unix)]
    #[test]
    fn kills_and_reports_a_child_that_outlives_the_deadline() {
        let started = Instant::now();
        let error = spawn_with_timeout(
            Path::new("sh"),
            &sh_args("sleep 30"),
            Duration::from_millis(120),
        )
        .expect_err("must time out");
        assert!(error.starts_with(TIMEOUT_MARKER), "{error}");
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "sooner than the child would exit"
        );
    }

    /// The bug this machinery exists for: a child whose output passes the pipe
    /// buffer must not be killed as a "timeout" while it is happily writing.
    #[cfg(unix)]
    #[test]
    fn drains_output_larger_than_the_pipe_buffer() {
        let (code, stdout, _) = spawn_with_timeout(
            Path::new("sh"),
            &sh_args("head -c 400000 /dev/zero | tr '\\0' 'x'"),
            Duration::from_secs(20),
        )
        .expect("run sh");
        assert_eq!(code, 0);
        assert_eq!(stdout.len(), 400_000);
    }

    #[cfg(unix)]
    #[test]
    fn a_signal_death_reports_minus_one() {
        let (code, _, _) = spawn_with_timeout(
            Path::new("sh"),
            &sh_args("kill -TERM $$"),
            Duration::from_secs(10),
        )
        .expect("run sh");
        assert_eq!(code, -1);
    }

    #[test]
    fn a_missing_program_is_an_error_not_a_panic() {
        let error = spawn_with_timeout(
            Path::new("/nonexistent/program/for/the/test"),
            &[],
            Duration::from_secs(1),
        )
        .expect_err("must fail");
        assert!(error.contains("failed to spawn"), "{error}");
    }
}
