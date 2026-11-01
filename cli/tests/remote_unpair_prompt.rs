//! Fixture: the unpair `[y/N]` prompt answered with one key, no Enter.
//!
//! `future remote devices remove 2` used to sit at `read_line`: a pressed `y`
//! did nothing until Enter followed it, and at zero CPU with nothing on disk the
//! command was indistinguishable from one that had hung. The fix is
//! `prompt::ask_yes_no`, and only a terminal reaches that path — which is
//! exactly what `cargo test` does not provide. Its unit tests can reach every
//! decision a key produces, but not that the mode is entered at all, that a key
//! arrives without a newline, or that the child goes on to do the work. So these
//! are spawned children with a pty of their own:
//!
//! * [`a_single_key_answers_without_enter`] — one `y`, no newline: the device is
//!   unpaired, the book on disk loses it, and the command exits 0.
//! * [`a_key_that_means_nothing_keeps_the_prompt_waiting`] — `x` then `y`: the
//!   stray key is not an answer, and the deciding one still is.
//! * [`enter_alone_is_a_no`] — Enter, no `y`: the documented default, and the
//!   book is untouched.
//! * [`enter_alone_cancels_the_all_question_too`] — the same answer to the
//!   `--all` question: neither device is unpaired, and the command says so.
//! * [`enter_alone_cancels_a_peer_removal_too`] — the same confirmation in front
//!   of the *other* book, `future remote peers remove`: two books, one prompt,
//!   and the default must be a no in both.
//!
//! The home each child gets holds a two-device book and no `auth.json`, so the
//! revoke is queued instead of delivered: nothing here touches the real
//! platform, a real relay, or the account this machine is signed into.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

/// Time for the prompt to appear and for the child to finish.
///
/// A budget, not an assertion about speed: the CLI resolves its data root and
/// opens a console before it can print anything, and the first run on a loaded
/// runner is the slow one. A child that never finishes fails the test at this
/// budget instead of wedging the suite.
const BUDGET: Duration = Duration::from_secs(60);

// ---------------------------------------------------------------------------
// The terminal each child gets
// ---------------------------------------------------------------------------

/// A child on a pty of its own, with a transcript of what it printed.
///
/// Unix drives `script(1)` (the pty the CLI needs); Windows drives a ConPTY,
/// because that is what a native console program sees as a terminal there.
#[cfg(unix)]
struct Terminal {
    child: std::process::Child,
    input: Option<std::process::ChildStdin>,
    transcript: Arc<Mutex<String>>,
    home: PathBuf,
}

#[cfg(unix)]
impl Terminal {
    fn spawn(home: PathBuf, args: &[&str]) -> Self {
        let exe = env!("CARGO_BIN_EXE_future");
        #[cfg(target_os = "linux")]
        let mut command = {
            // `script -c` takes one command line, so the arguments are quoted
            // the way a shell would quote them.
            let line = std::iter::once(exe)
                .chain(args.iter().copied())
                .map(|part| format!("'{}'", part.replace('\'', r"'\''")))
                .collect::<Vec<_>>()
                .join(" ");
            let mut command = std::process::Command::new("script");
            command.args(["-q", "-e", "-c", &line, "/dev/null"]);
            command
        };
        #[cfg(not(target_os = "linux"))]
        let mut command = {
            let mut command = std::process::Command::new("script");
            command.arg("-q").arg("/dev/null").arg(exe).args(args);
            command
        };
        isolate(&mut command, &home);
        let mut child = command
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .spawn()
            .expect("spawn the CLI under script(1)");
        let input = child.stdin.take();
        let transcript = Arc::new(Mutex::new(String::new()));
        let mut output = child.stdout.take().expect("the session's stdout");
        let sink = Arc::clone(&transcript);
        std::thread::spawn(move || {
            let mut buffer = [0u8; 4096];
            loop {
                match output.read(&mut buffer) {
                    Ok(0) | Err(_) => return,
                    Ok(read) => sink
                        .lock()
                        .expect("transcript")
                        .push_str(&String::from_utf8_lossy(&buffer[..read])),
                }
            }
        });
        Self {
            child,
            input,
            transcript,
            home,
        }
    }

    /// Type these bytes at the terminal, and nothing else: no newline, which is
    /// the whole point of the fixture.
    fn press(&mut self, keys: &[u8]) {
        if let Some(input) = self.input.as_mut() {
            input.write_all(keys).expect("type at the terminal");
            input.flush().expect("flush the keystrokes");
        }
    }

    /// Wait for the child to end; `true` if it exited successfully.
    fn success(&mut self) -> bool {
        let deadline = Instant::now() + BUDGET;
        loop {
            match self.child.try_wait() {
                Ok(Some(status)) => return status.success(),
                Ok(None) if Instant::now() >= deadline => {
                    let _ = self.child.kill();
                    let _ = self.child.wait();
                    panic!("the CLI never finished; transcript:\n{}", self.text());
                }
                Ok(None) => std::thread::sleep(Duration::from_millis(50)),
                Err(error) => panic!("cannot wait for the child: {error}"),
            }
        }
    }

    fn text(&self) -> String {
        self.transcript.lock().expect("transcript").clone()
    }

    /// Wait until `needle` has been printed, up to `budget`.
    fn wait_for(&self, needle: &str, budget: Duration) -> bool {
        let deadline = Instant::now() + budget;
        while Instant::now() < deadline {
            if self.text().contains(needle) {
                return true;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        self.text().contains(needle)
    }
}

#[cfg(unix)]
impl Drop for Terminal {
    fn drop(&mut self) {
        if matches!(self.child.try_wait(), Ok(None)) {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
        let _ = std::fs::remove_dir_all(&self.home);
    }
}

#[cfg(windows)]
struct Terminal {
    child: Box<dyn portable_pty::Child + Send + Sync>,
    input: Arc<Mutex<Box<dyn Write + Send>>>,
    _master: Box<dyn portable_pty::MasterPty + Send>,
    transcript: Arc<Mutex<String>>,
    home: PathBuf,
}

#[cfg(windows)]
impl Terminal {
    fn spawn(home: PathBuf, args: &[&str]) -> Self {
        use portable_pty::{native_pty_system, CommandBuilder, PtySize};
        let pair = native_pty_system()
            .openpty(PtySize {
                rows: 40,
                cols: 120,
                pixel_width: 0,
                pixel_height: 0,
            })
            .expect("create an owned ConPTY");
        let mut command = CommandBuilder::new(env!("CARGO_BIN_EXE_future"));
        command.args(args);
        isolated(&mut command, &home);
        let child = pair
            .slave
            .spawn_command(command)
            .expect("spawn the CLI on ConPTY");
        drop(pair.slave);
        let mut output = pair.master.try_clone_reader().expect("ConPTY reader");
        let input = Arc::new(Mutex::new(
            pair.master.take_writer().expect("ConPTY writer"),
        ));
        let transcript = Arc::new(Mutex::new(String::new()));
        let sink = Arc::clone(&transcript);
        let terminal = Arc::clone(&input);
        std::thread::spawn(move || {
            let mut buffer = [0u8; 4096];
            let mut cursor_query = 0;
            const CURSOR_QUERY: &[u8] = b"\x1b[6n";
            loop {
                match output.read(&mut buffer) {
                    Ok(0) | Err(_) => return,
                    Ok(read) => {
                        sink.lock()
                            .expect("transcript")
                            .push_str(&String::from_utf8_lossy(&buffer[..read]));
                        // ConPTY asks the terminal for its initial cursor
                        // position before the child runs, and a plain pipe is
                        // not an emulator: answer it, byte by byte, so the
                        // session is not waiting on us when the prompt appears.
                        for &byte in &buffer[..read] {
                            if byte == CURSOR_QUERY[cursor_query] {
                                cursor_query += 1;
                            } else {
                                cursor_query = usize::from(byte == CURSOR_QUERY[0]);
                            }
                            if cursor_query == CURSOR_QUERY.len() {
                                let mut terminal = terminal.lock().expect("ConPTY writer");
                                terminal
                                    .write_all(b"\x1b[1;1R")
                                    .expect("answer the ConPTY cursor query");
                                terminal.flush().expect("flush the cursor answer");
                                cursor_query = 0;
                            }
                        }
                    }
                }
            }
        });
        Self {
            child,
            input,
            _master: pair.master,
            transcript,
            home,
        }
    }

    fn press(&mut self, keys: &[u8]) {
        let mut terminal = self.input.lock().expect("ConPTY writer");
        terminal.write_all(keys).expect("type at the terminal");
        terminal.flush().expect("flush the keystrokes");
    }

    fn success(&mut self) -> bool {
        let deadline = Instant::now() + BUDGET;
        loop {
            match self.child.try_wait() {
                Ok(Some(status)) => return status.success(),
                Ok(None) if Instant::now() >= deadline => {
                    let _ = self.child.kill();
                    let _ = self.child.wait();
                    panic!("the CLI never finished; transcript:\n{}", self.text());
                }
                Ok(None) => std::thread::sleep(Duration::from_millis(50)),
                Err(error) => panic!("cannot wait for the child: {error}"),
            }
        }
    }

    fn text(&self) -> String {
        self.transcript.lock().expect("transcript").clone()
    }

    fn wait_for(&self, needle: &str, budget: Duration) -> bool {
        let deadline = Instant::now() + budget;
        while Instant::now() < deadline {
            if self.text().contains(needle) {
                return true;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        self.text().contains(needle)
    }
}

#[cfg(windows)]
impl Drop for Terminal {
    fn drop(&mut self) {
        if matches!(self.child.try_wait(), Ok(None)) {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
        let _ = std::fs::remove_dir_all(&self.home);
    }
}

/// The environment every child runs in: its own data root and nothing else from
/// this machine (no `auth.json`, so no revoke reaches the real platform).
#[cfg(unix)]
fn isolate(command: &mut std::process::Command, home: &Path) {
    command.env_clear();
    for key in [
        "PATH",
        "SHELL",
        "TMPDIR",
        "TEMP",
        "TMP",
        "SystemRoot",
        "LANG",
        "LLVM_PROFILE_FILE",
        "RUST_BACKTRACE",
    ] {
        if let Some(value) = std::env::var_os(key) {
            command.env(key, value);
        }
    }
    command
        .env("HOME", home)
        .env("USERPROFILE", home)
        .env("FUTURE_HOME", home.join(".future"));
}

#[cfg(windows)]
fn isolated(command: &mut portable_pty::CommandBuilder, home: &Path) {
    command.env_clear();
    for key in [
        "PATH",
        "SystemRoot",
        "TEMP",
        "TMP",
        "LANG",
        "LLVM_PROFILE_FILE",
        "RUST_BACKTRACE",
    ] {
        if let Some(value) = std::env::var_os(key) {
            command.env(key, value);
        }
    }
    command.env("HOME", home);
    command.env("USERPROFILE", home);
    command.env("FUTURE_HOME", home.join(".future"));
}

// ---------------------------------------------------------------------------
// The pairing book each child is given
// ---------------------------------------------------------------------------

const BOOK: &str = "remote_pairings.json";

/// The other book: pairings where **this** machine is driven by another one.
/// Its own file, its own commands (`future remote peers`), the same prompt.
const PEERS: &str = "remote_peers.json";

/// Which device the fixture asks to remove — the row `future remote devices`
/// prints second, so the command reads exactly as it does for a user.
const SECOND: &str = "024E1F1C08A644579F7ED64B27558187";
const FIRST: &str = "8677790D603941ADB6057EC8ECC35EC1";

/// A home whose book holds the two devices, one of them in control.
fn isolated_home(tag: &str) -> PathBuf {
    let home = std::env::temp_dir().join(format!("fos-unpair-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    let future = home.join(".future");
    std::fs::create_dir_all(&future).expect("create the isolated data root");
    let book = json!({
        "version": 2,
        "active": FIRST,
        "slots": [
            {
                "handshakeVersion": 2,
                "pairId": FIRST,
                "desktopId": "desk-fixture",
                "nkeySeed": "SUAAAA",
                "userJwt": "jwt-a",
                "natsUrl": "nats://a",
            },
            {
                "handshakeVersion": 2,
                "pairId": SECOND,
                "desktopId": "desk-fixture",
                "nkeySeed": "SUBBBB",
                "userJwt": "jwt-b",
                "natsUrl": "nats://b",
            },
        ],
    });
    std::fs::write(
        future.join(BOOK),
        serde_json::to_vec(&book).expect("encode the pairing book"),
    )
    .expect("write the pairing book");
    home
}

fn paired_devices(home: &Path) -> Vec<String> {
    let text = std::fs::read_to_string(home.join(".future").join(BOOK))
        .expect("the book must still be on disk");
    let book: Value = serde_json::from_str(&text).expect("the book is JSON");
    book["slots"]
        .as_array()
        .expect("slots is an array")
        .iter()
        .map(|slot| {
            slot["pairId"]
                .as_str()
                .expect("every slot names its device")
                .to_string()
        })
        .collect()
}

/// A home whose `remote_peers.json` holds two remote desktops this machine is
/// paired with — the other book, read by `future remote peers`.
fn peers_home(tag: &str) -> PathBuf {
    let home = std::env::temp_dir().join(format!("fos-peers-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    let future = home.join(".future");
    std::fs::create_dir_all(&future).expect("create the isolated data root");
    // Only what the command reads before it asks: the label, and the machine it
    // names. A peer row without a `desktopId` is refused by the loader, so this
    // fixture is the smallest book that is *valid*, not the smallest that parses.
    let book = json!({
        "peers": [
            { "pairId": "pair_a", "desktopId": "desktop_AAAA", "name": "work" },
            { "pairId": "pair_b", "desktopId": "desktop_BBBB" },
        ],
    });
    std::fs::write(
        future.join(PEERS),
        serde_json::to_vec(&book).expect("encode the peer book"),
    )
    .expect("write the peer book");
    home
}

fn paired_desktops(home: &Path) -> Vec<String> {
    let text = std::fs::read_to_string(home.join(".future").join(PEERS))
        .expect("the peer book must still be on disk");
    let book: Value = serde_json::from_str(&text).expect("the peer book is JSON");
    book["peers"]
        .as_array()
        .expect("peers is an array")
        .iter()
        .map(|peer| {
            peer["desktopId"]
                .as_str()
                .expect("every peer names its machine")
                .to_string()
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// One `y` and no newline: the answer the user pressed, which used to be a hang.
#[test]
fn a_single_key_answers_without_enter() {
    let home = isolated_home("pressed-y");
    let mut terminal = Terminal::spawn(home.clone(), &["remote", "devices", "remove", "2"]);
    assert!(
        terminal.wait_for("[y/N]", BUDGET),
        "the prompt must appear before anything is typed; transcript:\n{}",
        terminal.text()
    );
    terminal.press(b"y");
    assert!(
        terminal.success(),
        "one key must be the whole answer; transcript:\n{}",
        terminal.text()
    );
    // Child exit and the background terminal reader are independent. ConPTY
    // may still be delivering the final output after try_wait reports success.
    assert!(
        terminal.wait_for("1 device remains paired.", BUDGET),
        "the terminal must deliver the completion output: {}",
        terminal.text()
    );
    let transcript = terminal.text();
    assert!(
        transcript.contains("Unpaired 024E1F1C08A644579F7ED64B27558187"),
        "transcript:\n{transcript}"
    );
    assert!(
        transcript.contains("1 device remains paired."),
        "transcript:\n{transcript}"
    );
    // The answer was acted on, not merely accepted: the book lost that device
    // and kept the other one.
    assert_eq!(paired_devices(&home), vec![FIRST.to_string()]);
}

/// A key that means nothing is not an answer: the prompt keeps waiting for one
/// that does, so a stray keystroke cannot unpair anything.
#[test]
fn a_key_that_means_nothing_keeps_the_prompt_waiting() {
    let home = isolated_home("stray-key");
    let mut terminal = Terminal::spawn(home.clone(), &["remote", "devices", "remove", "2"]);
    assert!(
        terminal.wait_for("[y/N]", BUDGET),
        "the prompt must appear; transcript:\n{}",
        terminal.text()
    );
    terminal.press(b"x");
    terminal.press(b"y");
    assert!(
        terminal.success(),
        "the deciding key must still answer; transcript:\n{}",
        terminal.text()
    );
    assert_eq!(paired_devices(&home), vec![FIRST.to_string()]);
}

/// Enter alone takes the `[y/N]` default, and nothing is unpaired.
#[test]
fn enter_alone_is_a_no() {
    let home = isolated_home("pressed-enter");
    let mut terminal = Terminal::spawn(home.clone(), &["remote", "devices", "remove", "2"]);
    assert!(
        terminal.wait_for("[y/N]", BUDGET),
        "the prompt must appear; transcript:\n{}",
        terminal.text()
    );
    terminal.press(b"\r");
    assert!(
        terminal.success(),
        "a cancelled removal is not a failure; transcript:\n{}",
        terminal.text()
    );
    let transcript = terminal.text();
    assert!(
        transcript.contains("Cancelled. Nothing was changed."),
        "transcript:\n{transcript}"
    );
    assert_eq!(
        paired_devices(&home),
        vec![FIRST.to_string(), SECOND.to_string()],
        "no device may be unpaired by a default answer"
    );
}

/// The `--all` question takes the same answer and leaves the same evidence: one
/// Enter unpairs nothing, and the sentence says exactly that rather than
/// reporting an empty book.
#[test]
fn enter_alone_cancels_the_all_question_too() {
    let home = isolated_home("pressed-enter-all");
    let mut terminal = Terminal::spawn(home.clone(), &["remote", "devices", "remove", "--all"]);
    assert!(
        terminal.wait_for("[y/N]", BUDGET),
        "the prompt must appear; transcript:\n{}",
        terminal.text()
    );
    assert!(
        terminal.text().contains("Unpair all 2 devices"),
        "the question names its scope: {}",
        terminal.text()
    );
    terminal.press(b"\r");
    assert!(
        terminal.success(),
        "a cancelled `--all` is not a failure; transcript:\n{}",
        terminal.text()
    );
    let transcript = terminal.text();
    assert!(
        transcript.contains("Cancelled. Nothing was changed."),
        "transcript:\n{transcript}"
    );
    assert_eq!(
        paired_devices(&home),
        vec![FIRST.to_string(), SECOND.to_string()],
        "no device may be unpaired by a default answer"
    );
}

/// The same confirmation guards the other book.
///
/// `future remote peers remove` unpairs a *remote desktop* — the machines this
/// one can drive — instead of a device driving this one, and it asks its own
/// question first. That question is the prompt's second caller, so the default
/// answer has to be a no here too, on the same one-key rule: a stray Enter must
/// not drop a pairing that took two machines to make.
#[test]
fn enter_alone_cancels_a_peer_removal_too() {
    let home = peers_home("pressed-enter-peer");
    let mut terminal = Terminal::spawn(home.clone(), &["remote", "peers", "remove", "1"]);
    assert!(
        terminal.wait_for("[y/N]", BUDGET),
        "the prompt must appear before anything is typed; transcript:\n{}",
        terminal.text()
    );
    assert!(
        terminal.text().contains("Remove the pairing with work"),
        "the question names the pairing it would drop: {}",
        terminal.text()
    );
    terminal.press(b"\r");
    assert!(
        terminal.success(),
        "a cancelled peer removal is not a failure; transcript:\n{}",
        terminal.text()
    );
    let transcript = terminal.text();
    assert!(
        transcript.contains("Cancelled. Nothing was changed."),
        "transcript:\n{transcript}"
    );
    assert_eq!(
        paired_desktops(&home),
        vec!["desktop_AAAA".to_string(), "desktop_BBBB".to_string()],
        "no pairing may be dropped by a default answer"
    );
}
