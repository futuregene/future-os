//! The `--re-pair` entry points, driven the way a shell drives them.
//!
//! The shell record called the launcher and the `futureos` bin "a different entry point" and
//! waived `main.rs`'s argv arms and `launch.rs::forward_re_pair` for that reason. They are
//! not different: the bin is spawnable (`CARGO_BIN_EXE_futureos`), the launcher's terminal
//! requirement is satisfiable with a pty, and the ownership check is satisfiable by holding
//! the same file lock the Desktop holds. `docs/testing/review-tauri-shell.md` §3.3 measured
//! all of this; `docs/testing/module-tauri-shell.md` §S12 records the landing.
//!
//! Three rules this file keeps:
//!
//! * **Isolated data root, always.** Every child gets `HOME`/`FUTURE_HOME` under this
//!   process's temp directory, so a real `~/.future` (locks, database, pairing) is never
//!   touched. The lock namespace follows the root, so a run here cannot collide with a real
//!   Desktop's `desktop.lock`.
//! * **No `RunEvent::Ready`.** Nothing below needs a window, a menu or a healthy GUI session
//!   — the arms are argv dispatch, the instance lock, and the single-instance hand-off the
//!   main-thread driver already proves works on this host.
//! * **Bounded.** Every wait has a deadline and every child is reaped, so a degraded host
//!   fails a test instead of hanging the suite.

#![cfg(feature = "gui")]

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

/// The graphical bin, built by `cargo test` for this package.
fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_futureos")
}

/// A fresh, this-process-only data root.
///
/// The name carries the pid so two concurrent `cargo test` runs cannot share a lock, a
/// database or a single-instance socket.
fn isolated_root(tag: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!("futureos-re-pair-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("future/app")).expect("isolated data root");
    root
}

/// Serialises the two tests that need the process-global `HOME`/`FUTURE_HOME`: `cargo test`
/// runs test bodies on worker threads, and an environment write is process-wide.
static ENV: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Point this process (and therefore every child it spawns with `.envs`) at `root`.
///
/// `forward_re_pair` resolves the answer file and the notifier from the caller's own
/// environment, so the caller's must already be the isolated one.
fn adopt(root: &Path) {
    std::env::set_var("HOME", root);
    std::env::set_var("FUTURE_HOME", root.join("future"));
    std::env::remove_var("FUTURE_AGENT_SOCKET");
}

/// The single-instance socket the app under `root` listens on, computed the way
/// `instance::gui_identifier` computes the plugin's identifier: the base id, plus
/// `.home<first 16 hex of the data root>` whenever `FUTURE_HOME` is overridden. `/tmp` and the
/// `_si.sock` suffix are `tauri-plugin-single-instance`'s own layout.
fn socket_for(root: &Path) -> Option<PathBuf> {
    const BASE: &str = "com.futureos.desktop";
    std::env::set_var("FUTURE_HOME", root.join("future"));
    let root_id = future_rpc::connection::data_root_id(&root.join("future")).ok()?;
    let identifier = format!("{BASE}.home{}", &root_id[..16]);
    Some(PathBuf::from(format!(
        "/tmp/{}_si.sock",
        identifier.replace(['.', '-'], "_")
    )))
}

fn accepts(socket: &Path) -> bool {
    std::os::unix::net::UnixStream::connect(socket).is_ok()
}

/// Wait until the first instance's listener accepts, or give up.
///
/// The bind is done by the single-instance plugin inside `Builder::build`, i.e. before
/// `RunEvent::Ready`; on this host it was measured anywhere from ~2 s to ~25 s under load,
/// which is why the budget is generous and the wait is a poll rather than a sleep.
fn wait_for_listener(socket: &Path, budget: Duration) -> bool {
    let deadline = Instant::now() + budget;
    while Instant::now() < deadline {
        if accepts(socket) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    false
}

/// Spawn the Desktop and own its lifetime: `Drop` sends `SIGTERM` and reaps it.
///
/// `SIGTERM` rather than `SIGKILL` on purpose: a killed process cannot run the app's own
/// cleanup, and the previous segment paid for that with leaked isolated roots.
struct Desktop(Child);

impl Desktop {
    fn start(root: &Path) -> Self {
        let child = Command::new(bin())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .env("HOME", root)
            .env("FUTURE_HOME", root.join("future"))
            .env_remove("FUTURE_AGENT_SOCKET")
            .spawn()
            .expect("spawn the Desktop");
        Desktop(child)
    }
}

impl Drop for Desktop {
    fn drop(&mut self) {
        // SAFETY: a plain `kill(2)` on a pid this struct owns.
        unsafe {
            libc::kill(self.0.id() as i32, libc::SIGTERM);
        }
        let deadline = Instant::now() + Duration::from_secs(20);
        while Instant::now() < deadline {
            if matches!(self.0.try_wait(), Ok(Some(_))) {
                return;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        // A Desktop that ignored SIGTERM is still not ours to leave running.
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// Hold the Desktop's own data-directory lock, exactly as a running Desktop does.
///
/// `launch::another_desktop_is_running` is defined by this lock (`instance.rs`), not by a
/// process table: `try_lock_exclusive` failing is what makes a launch answer "join the
/// running Desktop" instead of "become one".
fn hold_desktop_lock(root: &Path) -> std::fs::File {
    use fs2::FileExt;
    let path = root.join("future/app/desktop.lock");
    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&path)
        .expect("open the Desktop lock");
    file.try_lock_exclusive()
        .expect("hold the Desktop lock exclusively");
    file
}

/// Run the graphical bin with a **pty** on stdin/stdout/stderr, which is what
/// `launch::require_terminal` asks for.
///
/// `/usr/bin/script` is this platform's pty allocator: it opens a pseudo-terminal, runs the
/// command on it, and forwards both directions to its own stdio. No dependency, no unsafe
/// `forkpty`, and the child sees a real tty (`isatty(0) && isatty(1)`), which is the exact
/// predicate `require_terminal` reads.
fn run_on_a_pty(root: &Path, args: &[&str]) -> std::process::Output {
    // `script` allocates the pty, but the two implementations disagree about how the
    // command is passed: BSD (macOS) takes it as trailing argv (`script -q /dev/null cmd
    // args...`), while util-linux (the Linux runners) takes it as ONE `-c` string and would
    // otherwise read `--re-pair` as its own option ("unrecognized option '--re-pair'").
    // `-e` makes util-linux return the child's status instead of its own.
    let mut cmd = Command::new("/usr/bin/script");
    #[cfg(target_os = "macos")]
    {
        cmd.arg("-q").arg("/dev/null").arg(bin()).args(args);
    }
    #[cfg(not(target_os = "macos"))]
    {
        // `bin()` is `&'static str`, not a Path: it is `env!("CARGO_BIN_EXE_futureos")`.
        let mut line = bin().to_string();
        for a in args {
            line.push(' ');
            line.push_str(a);
        }
        cmd.arg("-q").arg("-e").arg("-c").arg(line).arg("/dev/null");
    }
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env("HOME", root)
        .env("FUTURE_HOME", root.join("future"))
        .env_remove("FUTURE_AGENT_SOCKET")
        .output()
        .expect("run the launcher on a pty")
}

/// `launch.rs::forward_re_pair` — the launcher's hand-off and its failure arm.
///
/// `forward_re_pair` spawns **its own executable** with `--re-pair --re-pair-reply <file>`.
/// Called from this test binary, that child is a libtest harness, which refuses the unknown
/// argv and exits non-zero. `await_reply` treats that as conclusive — a failed hand-off
/// process never delivered the request, so no answer is coming — and reports it instead of
/// waiting out `REPLY_TIMEOUT` (60 s). The assertion on the elapsed time is therefore part of
/// the contract, not decoration.
#[test]
fn forward_re_pair_reports_a_hand_off_the_child_refused() {
    let root = isolated_root("forward");
    let _env = ENV.lock().unwrap_or_else(|poison| poison.into_inner());
    adopt(&root);
    let started = Instant::now();
    let error = futureos_lib::launch::forward_re_pair(false)
        .expect_err("a libtest harness cannot service --re-pair");
    assert!(
        error.contains("did not accept the request"),
        "the failure must name the hand-off, not the timeout: {error}"
    );
    assert!(
        started.elapsed() < Duration::from_secs(45),
        "the failure must be detected from the child's exit, not the 60s deadline {:?}",
        started.elapsed()
    );
    let _ = std::fs::remove_dir_all(&root);
}

/// `main.rs:26-29` — a `--re-pair` without a terminal is refused before any state is touched.
///
/// Piped stdin is the input; the bin is the entry point, so this is the real `main`, not a
/// re-implementation of it.
#[test]
fn the_bin_refuses_re_pair_without_a_terminal() {
    let root = isolated_root("refusal");
    let output = Command::new(bin())
        .arg("--re-pair")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env("HOME", &root)
        .env("FUTURE_HOME", root.join("future"))
        .env_remove("FUTURE_AGENT_SOCKET")
        .output()
        .expect("spawn the futureos bin");
    assert!(!output.status.success(), "the refusal must fail the launch");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("needs an interactive terminal"),
        "stderr: {stderr}"
    );
    // Refused before any state was touched: the data directory was never resolved.
    assert!(
        !root.join("future/app/desktop.lock").exists(),
        "the refusal must precede every state write"
    );
    let _ = std::fs::remove_dir_all(&root);
}

/// `main.rs:33-44` — the whole forwarded arm, from a terminal, with a Desktop that owns the
/// data directory and answers.
///
/// This is the arm the record called unreachable ("a different entry point"): the launcher
/// checks the terminal, hydrates the host environment, finds the lock held, hands the request
/// to the running Desktop through the single-instance socket, waits for the answer and prints
/// the invitation notice. Every step is the production one — the only injected inputs are a
/// pty and a second process.
///
/// The answer is deliberately **not** asserted to be a link: an isolated root has no pairing
/// credentials, so the Desktop answers with the `AccountAuthorization` error, which
/// `invitation_notice` turns into "No pairing invitation was issued: …". Either shape proves
/// the reply reached the launcher, and asserting one would make this test depend on the
/// account state of whoever runs it.
///
/// macOS only, for a measured precondition: this test needs a REAL `futureos` process to
/// bind the single-instance socket and answer the hand-off, and on the headless Linux
/// runner the Desktop cannot start at all (tao panics with "Failed to initialize GTK").
/// The failure mode was `the Desktop never bound …_si.sock within 60s`, which the shell
/// record already tracks as §S11.10.2 - a late or absent listener bind, not a launcher
/// defect. Gated rather than turned into a skip, so the arm stays exercised where a
/// Desktop can actually run.
#[cfg(target_os = "macos")]
#[test]
fn the_pty_launcher_forwards_to_a_running_desktop_and_prints_the_answer() {
    let root = isolated_root("forwarded");
    // `socket_for` reads `FUTURE_HOME` through `future_rpc`, so it takes the same lock the
    // forward test does.
    let socket = {
        let _env = ENV.lock().unwrap_or_else(|poison| poison.into_inner());
        socket_for(&root).expect("the plugin identifier is derivable")
    };
    let desktop = Desktop::start(&root);
    if !wait_for_listener(&socket, Duration::from_secs(60)) {
        // The host state is the precondition the shell record already tracks (a late or
        // absent listener bind, §S11.10.2). Say so instead of asserting a lie; the one
        // thing this test must not do is read the wrong reason as a pass.
        panic!(
            "the Desktop never bound {} within 60s; this host cannot answer a hand-off \
             (see docs/testing/module-tauri-shell.md §S11.10.2)",
            socket.display()
        );
    }

    let output = run_on_a_pty(&root, &["--re-pair"]);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    drop(desktop);
    let _ = std::fs::remove_dir_all(&root);

    assert!(
        output.status.success(),
        "the launcher must exit 0 after printing the answer: {stdout}\n{stderr}"
    );
    assert!(
        stdout.contains("No pairing invitation was issued")
            || stdout.contains("PAIRING INVITATION"),
        "the invitation notice must be printed on the terminal: {stdout}\n{stderr}"
    );
}

/// `main.rs:46-48,51` — the same pty launch when **no** Desktop is listening.
///
/// The lock is held by this process (so the launcher correctly concludes that a Desktop owns
/// the directory), but nothing is bound to the plugin socket, so the hand-off child starts a
/// Desktop, cannot take the lock, and exits unsuccessfully. `forward_re_pair` reports that as
/// "did not accept the request", which `main` prints before returning `FAILURE` — the arm
/// that tells an operator a headless Desktop is holding the directory.
#[test]
fn the_pty_launcher_reports_a_hand_off_nobody_answered() {
    let root = isolated_root("unanswered");
    let _lock = hold_desktop_lock(&root);
    let started = Instant::now();
    let output = run_on_a_pty(&root, &["--re-pair"]);
    // A pty merges the child's stdout and stderr into one stream, which `script` forwards on
    // its own stdout — so both are read together here, and asserting on the split would only
    // test the pty.
    let transcript = format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let _ = std::fs::remove_dir_all(&root);

    assert!(
        !output.status.success(),
        "a failed hand-off must fail the launch: {transcript}"
    );
    assert!(
        transcript.contains("did not accept the request"),
        "the reason must name the refused hand-off: {transcript}"
    );
    assert!(
        started.elapsed() < Duration::from_secs(90),
        "the failure must be conclusive before the 60s reply timeout is spent twice: {:?}",
        started.elapsed()
    );
}
