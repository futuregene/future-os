use std::io::Write;
use std::time::{Duration, Instant};

/// The summary line the harness accepts as this target's `test result:`.
const MARKER: &str = "gui_main_thread_drive: driver completed";

/// The app's own log line for "the Ready arm ran": `agent_supervisor` prints it once the
/// embedded Agent the launch path starts is up. Waiting for it (rather than sleeping a
/// fixed time) is what makes this driver's coverage repeatable on a loaded host.
const READY_EVIDENCE: &str = "embedded Agent ready";

/// Bound on waiting for the plugin's listener before giving up and reporting it. Every other
/// phase is bounded too (`CHILD_BUDGET`, `READY_BUDGET`, `SETTLE`), so the
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

/// Hand this process a single-instance notification and report how the child ended.
///
/// The child is this same binary with `ROOT_ENV` pointing at our root, so it reaches the
/// same socket. It never gets as far as the app: `Builder::build` runs the plugin's `setup`,
/// which connects to our listener, writes `cwd\0\0argv`, and calls
/// `std::process::exit(0)` (`tauri-plugin-single-instance 2.4.3`,
/// `platform_impl/macos.rs:23-31` — the mechanism §S6.7.1 measured). Our listener thread then
/// delivers the message to the app's callback on the tauri runtime, which is what makes the
/// callback body (`activate_main_window`) execute on the main thread.
///
/// **Attribution.** A hand-off child exits in ~0.1 s (at most ~2.3 s on a loaded host,
/// §S11.11); a child that missed the listener becomes an app of its own and runs this
/// driver's primary path instead, which is `READY_BUDGET` + `SETTLE` — an order of
/// magnitude longer. [`COMPANION_LATENCY`] is that gap. This used to be attributed by a
/// reply file the callback wrote for the old `--re-pair` hand-off; with no forwarded command
/// left, the latency is the only observable, so it is reported with the same
/// `hand-off ok` / `hand-off FAILED` vocabulary (supervisor ruling of 2026-10-17, §S11.9.5)
/// a leg-reader greps for.
fn notify_self() {
    let exe = match std::env::current_exe() {
        Ok(exe) => exe,
        Err(error) => {
            eprintln!("gui_main_thread_drive: no current_exe for the notification child: {error}");
            return;
        }
    };
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

    if notification_round(&exe) {
        eprintln!(
            "gui_main_thread_drive: hand-off ok, first-try (the first spawn handed off and exited)"
        );
        return;
    }
    // Fail loudly (§S11.9.5(c)): a run whose child became an app of its own earns nothing
    // from the callback lines, so a leg like that must be discardable by grep instead of
    // read as if it were a complete measurement.
    eprintln!(
        "gui_main_thread_drive: hand-off FAILED (the child became an app of its own or          outlived {}s); socket={socket_label}",
        COMPANION_LATENCY.as_secs()
    );
    // One retry, and only while the listener is actually accepting: the failure is
    // intermittent (§S11.11), so a second spawn can turn a bad run into a good one. It
    // cannot mask a good one: a hand-off exits in ~0.1 s, and this branch is only reached
    // when the first child did not.
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
    if notification_round(&exe) {
        eprintln!(
            "gui_main_thread_drive: hand-off ok, retried (the second spawn handed off; the \
             first round was degraded)"
        );
        return;
    }
    eprintln!(
        "gui_main_thread_drive: hand-off FAILED after one retry (no hand-off in either round); \
         socket={socket_label}"
    );
}

/// One notification round: spawn one child and judge it by how long it took to exit.
///
/// Returns whether the child handed the request over — it exited within
/// [`COMPANION_LATENCY`] and successfully. The per-child line is printed either way, so the
/// reader sees the evidence behind the verdict, not just the verdict.
fn notification_round(exe: &std::path::Path) -> bool {
    let mut child = match std::process::Command::new(exe)
        .stdin(std::process::Stdio::null())
        .spawn()
    {
        Ok(child) => child,
        Err(error) => {
            eprintln!("gui_main_thread_drive: could not start the notification child: {error}");
            return false;
        }
    };
    let started = Instant::now();
    let handoff = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status.success() && started.elapsed() <= COMPANION_LATENCY,
            Ok(None) if started.elapsed() >= CHILD_BUDGET => {
                let _ = child.kill();
                let _ = child.wait();
                eprintln!(
                    "gui_main_thread_drive: notification child exceeded {}s and was killed \
                     (the hand-off did not happen)",
                    CHILD_BUDGET.as_secs()
                );
                break false;
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(50)),
            Err(error) => {
                eprintln!("gui_main_thread_drive: notification child could not be waited on: {error}");
                break false;
            }
        }
    };
    eprintln!(
        "gui_main_thread_drive: notification child exited after {:.2}s ({})",
        started.elapsed().as_secs_f32(),
        if handoff { "hand-off" } else { "no hand-off" }
    );
    handoff
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

/// Drive the one IPC command whose *body* needs the real runtime.
///
/// `commands::files::prepare_image_preview` takes `tauri::AppHandle` — the concrete
/// `AppHandle<Wry>` — so the unit test beside it can only reach the generic seam behind it
/// (`prepare_image_preview_with::<R>`); `tauri::test::mock_app()` yields
/// `AppHandle<MockRuntime>`, a different type. This process has the real handle — the app's
/// `setup` hook publishes it — so the command's own six lines run here.
///
/// Gated on the `test-support` feature: the accessor and the re-export do not exist in a
/// default build, so the default build of this target compiles exactly as it did before.
///
/// Reported, not asserted: this driver must never add a red to a job whose subject is not
/// this unit (see the module docs), so each check prints its own verdict and the summary
/// line says how many matched — a reader of the leg's log sees the values, not just a pass.
#[cfg(all(feature = "test-support", feature = "gui"))]
fn exercise_image_preview() {
    use tauri::Manager;

    // `setup` runs inside `Builder::build`, so the handle appears before `Ready`; poll rather
    // than assume which of the two wins the race.
    let deadline = Instant::now() + Duration::from_secs(10);
    let handle = loop {
        if let Some(handle) = futureos_lib::test_support::app_handle() {
            break handle.clone();
        }
        if Instant::now() >= deadline {
            eprintln!(
                "gui_main_thread_drive: the app published no handle within 10s; \
                 prepare_image_preview is NOT covered by this run"
            );
            return;
        }
        std::thread::sleep(Duration::from_millis(50));
    };

    // Deliberately **outside** every scope `tauri.conf.json` grants up front
    // (`$APPCACHE`, `$APPDATA`, `$HOME/.future`, `$TEMP` — all of which resolve under this
    // process's isolated temp root). Anywhere under the isolated root would already be
    // readable, so the `allow_file` this command performs would be unobservable; measured
    // once with the fixtures under the isolated root: `is_allowed` said `true` for a sibling
    // that had never been prepared, which is what made the check below discriminating.
    // `/private/tmp` (not `std::env::temp_dir()`, i.e. not `$TEMP`) is the nearest one.
    let directory = std::path::Path::new("/private/tmp").join(format!(
        "futureos-drive-{}-preview",
        std::process::id()
    ));
    if let Err(error) = std::fs::create_dir_all(&directory) {
        eprintln!("gui_main_thread_drive: cannot create {}: {error}", directory.display());
        return;
    }
    let image = directory.join("pixel.png");
    let sibling = directory.join("sibling.png");
    // Content is irrelevant to this command (it validates the *path*, not the bytes).
    for path in [&image, &sibling] {
        if let Err(error) = std::fs::write(path, b"\x89PNG\r\n\x1a\npreview fixture") {
            eprintln!("gui_main_thread_drive: cannot write {}: {error}", path.display());
            return;
        }
    }

    let canonical = match image.canonicalize() {
        Ok(path) => path,
        Err(error) => {
            eprintln!("gui_main_thread_drive: cannot canonicalize the fixture: {error}");
            return;
        }
    };
    let first = match futureos_lib::test_support::prepare_image_preview(
        handle.clone(),
        image.display().to_string(),
    ) {
        Ok(asset) => asset,
        Err(error) => {
            eprintln!("gui_main_thread_drive: prepare_image_preview FAILED ({error})");
            return;
        }
    };
    let second = match futureos_lib::test_support::prepare_image_preview(
        handle.clone(),
        format!("  {}  ", image.display()),
    ) {
        Ok(asset) => asset,
        Err(error) => {
            eprintln!("gui_main_thread_drive: prepare_image_preview (retry) FAILED ({error})");
            return;
        }
    };

    // Every value below is derived from the arguments the command was given, so a wrapper
    // that dropped or mangled them changes one of them:
    //   * the returned path is the *canonical* form of the path passed in (padding included
    //     in the second call, so trimming is exercised too);
    //   * the version is re-derived per call (the WebView cache key), so two preparations of
    //     the same file must differ;
    //   * the asset scope holds the canonical file and nothing its sibling — that scope write
    //     is what makes a preview readable at all, and it only happens if the call reached it.
    let first: serde_json::Value = serde_json::to_value(&first).unwrap_or_default();
    let second: serde_json::Value = serde_json::to_value(&second).unwrap_or_default();
    let canonical_text = canonical.display().to_string();
    let checks: [(&str, bool, String); 3] = [
        (
            "path is the canonical file",
            first["path"].as_str() == Some(canonical_text.as_str()),
            format!("{} != {canonical_text}", first["path"]),
        ),
        (
            "the version is re-derived per call",
            second["version"].as_str().is_some_and(|version| {
                !version.is_empty() && Some(version) != first["version"].as_str()
            }),
            format!("{} then {}", first["version"], second["version"]),
        ),
        (
            "the asset scope holds the file and only it",
            handle.asset_protocol_scope().is_allowed(&canonical)
                && !handle
                    .asset_protocol_scope()
                    .is_allowed(directory.join("sibling.png")),
            format!("allowed={}", handle.asset_protocol_scope().is_allowed(&canonical)),
        ),
    ];
    let failed: Vec<&str> = checks
        .iter()
        .filter(|(_, ok, _)| !ok)
        .map(|(name, _, detail)| {
            eprintln!("gui_main_thread_drive: image preview MISMATCH: {name} ({detail})");
            *name
        })
        .collect();
    eprintln!(
        "gui_main_thread_drive: prepare_image_preview ran with the real Wry handle \
         ({}/{}) derived values match{}",
        checks.len() - failed.len(),
        checks.len(),
        if failed.is_empty() { String::new() } else { format!("; failed: {}", failed.join(", ")) }
    );
    // Not under the driver's isolated root, so `finish` will not remove it for us.
    let _ = std::fs::remove_dir_all(&directory);
}

pub fn main() {
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
        // One IPC wrapper's body needs the real (Wry) handle the app published; a companion
        // is here to *be* a notification and exits inside `Builder::build`, so it never has
        // one. See `exercise_image_preview`.
        #[cfg(all(feature = "test-support", feature = "gui"))]
        if !is_companion {
            exercise_image_preview();
        }
        // One notification, once. A companion never does this: it is here to *be* a
        // notification, or the children would notify each other forever.
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
    let outcome = futureos_lib::run_with();
    match outcome {
        Ok(()) => finish("driver completed after run_with returned"),
        Err(error) => finish(&format!(
            "run_with could not start the graphical runtime on this host ({error}); \
             skipped rather than failed"
        )),
    }
}
