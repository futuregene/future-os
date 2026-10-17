//! **The main-thread driver** (TAU-C8): a `harness = false` test target.
//!
//! Why this target exists — and why it is a *test target*, not a `src/bin/` file:
//! the shell's biggest residual family (`menu.rs`'s `build_macos_menu`, `run_with`'s own
//! body, the `.setup` hook, the window-geometry calls) is unreachable from a libtest body
//! because `muda`/AppKit require the process's **main thread** and libtest runs every test
//! body on a spawned thread (`docs/testing/module-tauri-shell.md` §6.2, measured).
//! `harness = false` gives this file a real `fn main()`, i.e. the main thread, while
//! staying a test target: `cargo test` runs it, so `cargo llvm-cov` instruments it and its
//! profile merges like any other target's, and it is never bundled into the app.
//!
//! How it works:
//!
//! 1. **Isolate first.** `HOME`/`FUTURE_HOME` are redirected to a temp directory before
//!    anything else, so the real `~/.future` (locks, database, logs, the user's agent) is
//!    never touched. That also makes the single-instance socket unique:
//!    `instance::gui_identifier` appends the data root's id whenever `FUTURE_HOME` is
//!    overridden (`lib.rs`'s identifier logic), so this process cannot collide with a real
//!    Desktop on `/tmp/<identifier>_si.sock`.
//! 2. **Run the real entry point** on the main thread: `futureos_lib::run_with`, the same
//!    function `src/main.rs` calls.
//! 3. **Exit through `libc::exit`.** LLVM writes the profile from an `atexit` handler, so
//!    the process must end normally — a `SIGKILL`ed app records nothing (measured; see the
//!    record's Appendix S7). `std::process::exit` runs the `atexit` chain, which is why the
//!    watchdog thread below can end the run from the main thread's event loop.
//! 4. **Print the summary marker** `gui_main_thread_drive: driver completed` as the last
//!    thing before exiting. A `harness = false` target prints no `test result:` line, and
//!    the campaign's harness treats a target without one as truncated; this marker is that
//!    target's summary (see `.future/cov100/measure-final.sh`).
//!
//! Bounded, and it never fails the suite:
//!
//! * it **always** hands the app one single-instance notification once the plugin's listener
//!   is up — that is the coverage this target exists for, and it does not depend on the app
//!   reaching `Ready` (the plugin delivers the callback on its own thread, measured §S9.2);
//! * it waits a **short, tightening** window for the app's `Ready` state before notifying, so
//!   that on a host which delivers `Ready` the callback finds the window (its `unminimize`/
//!   `show`/`set_focus` arms) and the `.setup` hook (menu, dock icon, geometry) has run;
//! * a hard deadline ends the process regardless, so a hang cannot hold the suite;
//! * if `run_with` returns — on a host where the real Tauri/Wry runtime cannot start (a
//!   Linux CI box without a display, for instance) it returns `Err` — the driver reports
//!   that on stderr and exits **0**: the platform's own behaviour is the result, and this
//!   target must not add a red to a job whose subject is not this unit's code.
//!
//! Flavour-sensitive: it needs the `gui` feature (`run_with` only exists there — the
//! headless build exports a different surface), and the `[[test]]` stanza declares
//! `required-features = ["gui"]` so the headless leg skips it instead of failing to build.

use std::io::Write;
use std::time::{Duration, Instant};

/// The summary line the harness accepts as this target's `test result:`.
const MARKER: &str = "gui_main_thread_drive: driver completed";

/// The app's own log line for "the Ready arm ran": `agent_supervisor` prints it once the
/// embedded Agent the launch path starts is up. Waiting for it (rather than sleeping a
/// fixed time) is what makes this driver's coverage repeatable on a loaded host.
const READY_EVIDENCE: &str = "embedded Agent ready";

/// Bound on waiting for the plugin's listener before giving up and reporting it. Every other
/// phase is bounded too (`CHILD_BUDGET`, the reply wait, `READY_BUDGET`, `SETTLE`), so the
/// target's total cost has a ceiling of roughly this plus half a minute.
const LISTENER_BUDGET: Duration = Duration::from_secs(30);

/// How long the app's **own readiness** gets to appear before the notification is sent anyway.
///
/// `Ready` is what runs the `.setup` hook (menu, dock icon, window show/geometry) and creates
/// the window the callback touches, so waiting for it is what makes those lines coverable at
/// all. It is deliberately **tight** — the target runs in every suite invocation — and it is
/// **opportunistic**: `Ready` is emitted when tao's app delegate receives
/// `applicationDidFinishLaunching:` (`tao` `platform_impl/macos/app_state.rs` →
/// `StartCause::Init` → `tauri-runtime-wry` `RunEvent::Ready`), and on a host where that
/// message does not arrive this wait can never succeed however long it is: measured 4/4 runs
/// on 2026-10-16 20:00–20:12 with the app's own terminal server still not accepting (see
/// §S10.3) while readiness had been reached in ~2.5 s earlier the same evening. So the yield
/// that depends on `Ready` is *host-state dependent* and the notification is sent regardless.
///
/// Override with `FUTUREOS_DRIVE_READY_BUDGET_SECS` when a measurement wants a longer window.
const READY_BUDGET: Duration = Duration::from_secs(15);

/// Environment override for [`READY_BUDGET`], in seconds.
const READY_BUDGET_ENV: &str = "FUTUREOS_DRIVE_READY_BUDGET_SECS";

/// How long the callback's reply file gets to appear after the notification.
const REPLY_BUDGET: Duration = Duration::from_secs(6);

/// [`READY_BUDGET`], with the environment override applied.
fn ready_budget() -> Duration {
    match std::env::var(READY_BUDGET_ENV)
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
    {
        Some(secs) => Duration::from_secs(secs),
        None => READY_BUDGET,
    }
}

/// After the Ready evidence appears: let the emitters, the window geometry and the
/// terminal server run before the profile is flushed.
const SETTLE: Duration = Duration::from_secs(2);

/// Set on the primary process to publish the data root a **notification child** must share.
///
/// The child has to reuse the primary's root, not make its own: the single-instance socket
/// is `/tmp/<identifier>_si.sock` and `instance::gui_identifier` derives the identifier from
/// the data root, so a child with a different root would claim its *own* socket instead of
/// notifying this process — and the callback body this target exists to cover would never
/// run.
const ROOT_ENV: &str = "FUTUREOS_DRIVE_ROOT";

/// How long a notification child may take to hand the request over and exit. It exits
/// inside the plugin's `setup` (`std::process::exit(0)`), so this is a generous bound.
const CHILD_BUDGET: Duration = Duration::from_secs(20);

/// The latency that distinguishes a hand-off from its absence.
///
/// A child that reached the plugin's `setup` exits in ~0.1 s (0.16 s measured in the
/// Segment-10 measurement) and at most ~2.3 s on a loaded host (§S11.11). A child that
/// *missed* the listener does not hand off at all: it becomes an app of its own and runs
/// this driver's primary path instead, which is `READY_BUDGET` + `SETTLE` — 17.10/17.16 s
/// measured in §S11.8, 17.08/17.14 s in §S11.10. The gap is an order of magnitude wide,
/// which is what makes the one retry below self-checking (§S11.9.5(a)).
const COMPANION_LATENCY: Duration = Duration::from_secs(5);

/// Run one notification child and report it.
fn spawn_child(
    label: &str,
    command: &mut std::process::Command,
) -> Option<(String, std::process::Child)> {
    match command.spawn() {
        Ok(child) => Some((label.to_string(), child)),
        Err(error) => {
            eprintln!("gui_main_thread_drive: could not start the {label} child: {error}");
            None
        }
    }
}

/// The isolated data root, published for the exit path: the driver removes it **only when it
/// created it**. A notification child shares the primary's root (`ROOT_ENV`) and must never
/// delete it out from under the primary.
static ISOLATED: std::sync::OnceLock<std::path::PathBuf> = std::sync::OnceLock::new();
static OWNS_ROOT: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

fn finish(why: &str) -> ! {
    // Best effort, and before the marker: the marker is this target's summary line and must
    // stay the last thing it writes. The embedded Agent still holds its database open, which
    // is fine on every platform this crate supports (the app's own `HomeGuard` does the same
    // and documents why the pool is released first where it matters).
    if OWNS_ROOT.load(std::sync::atomic::Ordering::Relaxed) {
        if let Some(dir) = ISOLATED.get() {
            let _ = std::fs::remove_dir_all(dir);
        }
    }
    eprintln!("gui_main_thread_drive: {why}");
    println!("{MARKER}");
    let _ = std::io::stdout().flush();
    // `exit` (not `abort`): the LLVM profile writer is an `atexit` handler.
    std::process::exit(0);
}

/// Hand this process a single-instance notification, once per `argv` shape per round —
/// the two shapes the plugin's callback distinguishes — and report how each child ended.
/// At most two rounds: the second only after the first produced no reply (see below).
///
/// The child is this same binary with `ROOT_ENV` pointing at our root, so it reaches the
/// same socket. It never gets as far as the app: `Builder::build` runs the plugin's `setup`,
/// which connects to our listener, writes `cwd\0\0argv`, and calls
/// `std::process::exit(0)` (`tauri-plugin-single-instance 2.4.3`,
/// `platform_impl/macos.rs:23-31` — the mechanism §S6.7.1 measured). Our listener thread then
/// delivers the message to the app's callback on the tauri runtime, which is what makes the
/// callback body execute.
///
/// The reply file is the attribution: the callback's `RePair` arm spawns the re-pair pass
/// with `RePairDelivery::Reply(path)`, and that pass writes an answer even when it fails
/// (there are no pairing credentials in this root), so the file's appearance proves the
/// callback ran rather than merely that a child exited.
///
/// **Outcome reporting (supervisor ruling of 2026-10-17, §S11.9.5):** a round that produces
/// the reply prints `hand-off ok, first-try` (or `…ok, retried`), and a round that does not
/// prints a single greppable `hand-off FAILED (…); socket=…` line — because a run whose
/// children took the companion path earns nothing from the nine callback lines and must be
/// discardable by a reader who is not looking. One bounded retry follows, and only while
/// the listener still accepts; a second failure prints its own terminal line instead of
/// looping. These lines are a flag, not a fix: the underlying host state is §S11.1/§S11.3.
fn notify_self() {
    let exe = match std::env::current_exe() {
        Ok(exe) => exe,
        Err(error) => {
            eprintln!("gui_main_thread_drive: no current_exe for the notification child: {error}");
            return;
        }
    };
    let reply = std::env::temp_dir().join(format!(
        "futureos-gui-drive-reply-{}.json",
        std::process::id()
    ));
    if let Some(root) = std::env::var_os(ROOT_ENV) {
        let _ = std::fs::remove_dir_all(std::path::Path::new(&root).join("future/agent"));
    }
    // Computed once and reported on every failure line: the socket is the one thing a
    // reader cannot reconstruct from the child latencies, and a wrong/path-mismatched
    // socket is the first hypothesis for a hand-off that never reaches this process
    // (§S11.10.2 states the confound that made it unprovable there).
    let socket = own_socket();
    let socket_label = socket
        .as_deref()
        .map(|socket| socket.display().to_string())
        .unwrap_or_else(|| "<undetermined>".to_string());

    let (answered, companion) = notification_round(&exe, &reply);
    if answered {
        eprintln!(
            "gui_main_thread_drive: hand-off ok, first-try (the first spawn produced the reply)"
        );
        return;
    }
    // Fail loudly (§S11.9.5(c)): a run whose children took the companion path earns
    // nothing from the nine callback lines, so a leg like that must be discardable by
    // grep instead of read as if it were a complete measurement.
    eprintln!(
        "gui_main_thread_drive: hand-off FAILED ({}; no reply within {}s); socket={socket_label}",
        if companion {
            "companion path"
        } else {
            "children exited on the hand-off path but left no reply"
        },
        REPLY_BUDGET.as_secs()
    );
    // One retry, and only while the listener is actually accepting: the failure is
    // intermittent (§S11.11: the same driver failed three times, then passed with 2.24 s
    // children and a real reply), so a second spawn can turn a bad run into a good one.
    // It cannot mask a good one: a hand-off answers in ~0.1 s with the reply file, and
    // this branch is only reached when there is no reply after a whole round.
    match socket.as_deref() {
        Some(socket) if listener_is_up(socket) => {}
        _ => {
            eprintln!(
                "gui_main_thread_drive: hand-off NOT RETRIED (nothing is accepting on {socket_label})"
            );
            return;
        }
    }
    eprintln!("gui_main_thread_drive: retrying the notification once (spawn #2)");
    let (answered, companion) = notification_round(&exe, &reply);
    if answered {
        eprintln!(
            "gui_main_thread_drive: hand-off ok, retried (the second spawn produced the reply; \
             the first round was degraded)"
        );
        return;
    }
    eprintln!(
        "gui_main_thread_drive: hand-off FAILED after one retry ({}; no reply within {}s); \
         socket={socket_label}",
        if companion {
            "companion path"
        } else {
            "children exited on the hand-off path but left no reply"
        },
        REPLY_BUDGET.as_secs()
    );
}

/// One notification round: both `argv` shapes, one spawn each, then the wait for the
/// callback's reply file.
///
/// Returns `(answered, companion_path)` — whether the reply appeared, and whether any
/// child outlived [`COMPANION_LATENCY`], which is the signature of a child that never
/// reached the plugin's `setup`. The per-child latency lines are printed either way, so
/// the reader sees the evidence behind both answers, not just the verdict.
fn notification_round(exe: &std::path::Path, reply: &std::path::Path) -> (bool, bool) {
    // A stale answer from a previous round would read as a hand-off that never happened.
    let _ = std::fs::remove_file(reply);

    // Shape 1: a bare `--re-pair`, which the callback reads as `reply: None`.
    let mut bare = std::process::Command::new(exe);
    bare.arg("--re-pair").stdin(std::process::Stdio::null());

    // Shape 2: the launcher's own hand-off, which reads as `reply: Some(path)`.
    let mut handed = futureos_lib::launch::notifier_command(exe, reply, false);

    // Both at once: the notifications are delivered to one listener thread and each child
    // exits as soon as it has written its message, so this costs one child's latency.
    let mut pending: Vec<(String, std::process::Child)> = Vec::new();
    for (label, command) in [
        ("bare --re-pair", &mut bare),
        ("--re-pair --re-pair-reply", &mut handed),
    ] {
        if let Some(child) = spawn_child(label, command) {
            pending.push(child);
        }
    }

    let started = Instant::now();
    let mut companion = false;
    while !pending.is_empty() {
        let mut index = 0;
        while index < pending.len() {
            match pending[index].1.try_wait() {
                Ok(Some(status)) => {
                    let (label, _) = pending.remove(index);
                    let elapsed = started.elapsed();
                    companion |= elapsed > COMPANION_LATENCY;
                    eprintln!(
                        "gui_main_thread_drive: {label} child exited {status} after {:.2}s",
                        elapsed.as_secs_f32()
                    );
                    continue;
                }
                Ok(None) => index += 1,
                Err(error) => {
                    let (label, _) = pending.remove(index);
                    eprintln!(
                        "gui_main_thread_drive: {label} child could not be waited on: {error}"
                    );
                }
            }
        }
        if !pending.is_empty() && started.elapsed() >= CHILD_BUDGET {
            for (label, mut child) in pending.drain(..) {
                let _ = child.kill();
                let _ = child.wait();
                companion = true;
                eprintln!(
                    "gui_main_thread_drive: {label} child exceeded {}s and was killed \
                     (the hand-off did not happen)",
                    CHILD_BUDGET.as_secs()
                );
            }
        }
        std::thread::sleep(Duration::from_millis(50));
    }

    // The callback runs on the app's runtime; give it a window to answer on the file.
    let deadline = Instant::now() + REPLY_BUDGET;
    while !reply.is_file() {
        if Instant::now() >= deadline {
            eprintln!(
                "gui_main_thread_drive: no reply file at {} within {}s (the callback's Reply \
                 arm did not finish)",
                reply.display(),
                REPLY_BUDGET.as_secs()
            );
            return (false, companion);
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    let answer = std::fs::read_to_string(reply).unwrap_or_default();
    eprintln!(
        "gui_main_thread_drive: the forwarded re-pair was answered on {}: {}",
        reply.display(),
        answer.trim()
    );
    let _ = std::fs::remove_file(reply);
    (true, companion)
}

/// The socket this process's single-instance plugin listens on, computed the way the plugin
/// computes it (`/tmp/<identifier>_si.sock`, `.`/`-` normalised to `_`).
///
/// The identifier is the app's own (`tauri.conf.json`'s `com.futureos.desktop`) with
/// `gui_identifier`'s data-root suffix when `FUTURE_HOME` is overridden — which is why a
/// **root of its own** also means a socket of its own, instead of the host-wide
/// `/tmp/_si.sock` an empty mock identifier yields (§S6.7.1).
fn own_socket() -> Option<std::path::PathBuf> {
    const BASE: &str = "com.futureos.desktop";
    let identifier = match future_rpc::home::future_home_override() {
        None => BASE.to_string(),
        Some(_) => {
            let root = future_rpc::home::future_home();
            let id = future_rpc::connection::data_root_id(&root).ok()?;
            format!("{BASE}.home{}", &id[..16])
        }
    };
    Some(std::path::PathBuf::from(format!(
        "/tmp/{}_si.sock",
        identifier.replace(['.', '-'], "_")
    )))
}

/// Whether the plugin's listener is accepting connections yet.
///
/// This, and not the app's own readiness, is what the notification step waits for: the
/// listener starts inside `Builder::build`, while `RunEvent::Ready` (which creates the
/// window the callback touches) fires a moment later and — under load — the embedded
/// Agent's startup can take tens of seconds after that. Gating on the listener keeps the
/// target's cost load-insensitive; gating on the Agent made it flaky (measured: readiness
/// in 2.5 s on an idle host, 30–40 s while several instrumented suites ran).
fn listener_is_up(socket: &std::path::Path) -> bool {
    std::os::unix::net::UnixStream::connect(socket).is_ok()
}

/// Build the macOS menu on this process's main thread, against a mock app.
///
/// **Why this belongs in a `harness = false` target.** `menu::build_macos_menu` is generic
/// over `tauri::Runtime`, so it needs no desktop, no window and no `RunEvent::Ready`; `muda`
/// only insists that each menu child is constructed on the **main thread** (it panics from a
/// libtest worker: `muda-0.19.3/src/platform_impl/macos/mod.rs:328`). This target's `main()`
/// *is* that thread, so the whole builder — both app-menu items, the four submenus, and every
/// predefined item — executes here. The shell record called these 46 lines
/// `host-state-blocked`; `docs/testing/review-tauri-shell.md` §3.2 falsified that, and
/// `docs/testing/module-tauri-shell.md` §S12.6 lands it.
///
/// It is a *report*, not a silent no-op: a menu that cannot be built here would silently
/// remove 46 lines from the measurement, so the failure is named on stderr. The driver still
/// exits 0 — this target never adds a red to a job whose subject is not this unit — but a
/// reader of the leg's log sees exactly why the yield is missing.
#[cfg(target_os = "macos")]
fn build_the_menu() {
    let app = tauri::test::mock_app();
    match futureos_lib::build_macos_menu(app.handle()) {
        Ok(menu) => match menu.items() {
            Ok(items) => eprintln!(
                "gui_main_thread_drive: menu built on the main thread ({} top-level submenus)",
                items.len()
            ),
            Err(error) => eprintln!(
                "gui_main_thread_drive: menu built but could not be read back ({error}); the \
                 builder's own lines still ran"
            ),
        },
        Err(error) => eprintln!(
            "gui_main_thread_drive: menu build FAILED ({error}); the menu.rs lines are NOT \
             covered by this run"
        ),
    }
}

fn main() {
    // A notification child (`ROOT_ENV` set) reuses the primary's root so it lands on the
    // same single-instance socket; a primary makes its own and is the one that removes it.
    // The same flag decides which of the two this process is: only a primary notifies, or
    // the children would notify each other forever.
    let companion_root = std::env::var_os(ROOT_ENV);
    let is_companion = companion_root.is_some();
    let (isolated, owns_root) = match companion_root {
        Some(root) => (std::path::PathBuf::from(root), false),
        None => (
            std::env::temp_dir().join(format!("futureos-gui-drive-{}", std::process::id())),
            true,
        ),
    };
    if let Err(error) = std::fs::create_dir_all(isolated.join("future")) {
        finish(&format!(
            "could not create {}/future: {error}",
            isolated.display()
        ));
    }
    // SAFETY: set before the runtime starts and before any thread reads them; the whole
    // point of this target is to run the app against its own data root.
    std::env::set_var("HOME", &isolated);
    std::env::set_var("FUTURE_HOME", isolated.join("future"));
    std::env::set_var(ROOT_ENV, &isolated);
    let app_log = isolated.join("future/app/logs/desktop.log");
    let _ = ISOLATED.set(isolated);
    OWNS_ROOT.store(owns_root, std::sync::atomic::Ordering::Relaxed);

    // The macOS menu, built on the main thread — the one thing `build_macos_menu` actually
    // needs, and the reason the shell record's `host-state-blocked` label was wrong for it
    // (`docs/testing/module-tauri-shell.md` §S12.6): `muda` panics when a child is created
    // off the main thread, but the builder is runtime-generic, so a mock app on this thread
    // executes the whole menu with no `RunEvent::Ready`, no window and no GUI session.
    #[cfg(target_os = "macos")]
    build_the_menu();

    // The watchdog owns the exit: the app's event loop does not return on its own, and the
    // profile must be written by a normal exit (see the module docs).
    std::thread::spawn(move || {
        let started = Instant::now();
        let socket = own_socket();
        let listener_up = match &socket {
            Some(socket) => {
                // Bounded wait for the listener the plugin starts inside `Builder::build`.
                while !listener_is_up(socket) {
                    if started.elapsed() >= LISTENER_BUDGET {
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(100));
                }
                listener_is_up(socket)
            }
            None => false,
        };
        // Wait for the app's **own readiness** before notifying: `Ready` is what runs the setup
        // hook (menu, dock icon, window show/geometry) and creates the window the callback
        // touches, so on a host that delivers it the callback's window arms run too. A
        // listener-gated variant that left earlier gained the callback's lines but LOST the
        // setup-hook coverage (measured, §S9.3). Tight and opportunistic by design — see
        // `READY_BUDGET`.
        let ready_deadline = Instant::now() + ready_budget();
        let mut ready = false;
        while Instant::now() < ready_deadline {
            if std::fs::read_to_string(&app_log).is_ok_and(|log| log.contains(READY_EVIDENCE)) {
                ready = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        if !ready && !is_companion {
            eprintln!(
                "gui_main_thread_drive: {READY_EVIDENCE:?} did not appear within {}s; \
                 notifying anyway (the callback's non-window arms do not need `Ready`, and its \
                 window arms plus the setup-hook coverage stay host-state dependent)",
                ready_budget().as_secs()
            );
        }
        // Both notification shapes, once — a plain `--re-pair` (`reply: None`) and the
        // launcher's own `--re-pair --re-pair-reply <path>` (`reply: Some(…)`). A companion
        // never does this: it is here to *be* a notification, or the children would notify
        // each other forever.
        if !is_companion {
            notify_self();
        }
        // Let the callback (delivered on the app's runtime) and the settle window finish,
        // then end the process — the profile is written on a normal exit.
        std::thread::sleep(SETTLE);
        let readiness = if ready { "with" } else { "without" };
        finish(&format!(
            "driver completed {readiness} {READY_EVIDENCE:?}; listener_up={listener_up} elapsed={:.1}s",
            started.elapsed().as_secs_f32()
        ));
    });

    // The real entry point, on this process's main thread.
    let outcome = futureos_lib::run_with(futureos_lib::launch::LaunchOptions::default());
    match outcome {
        Ok(()) => finish("driver completed after run_with returned"),
        Err(error) => finish(&format!(
            "run_with could not start the graphical runtime on this host ({error}); \
             skipped rather than failed"
        )),
    }
}
