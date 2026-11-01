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
    // The macOS update worker, which production reaches by re-invoking the app itself with
    // `--app-update-macos-worker`. This target is a process too, so a scenario below can hand
    // the *real* worker to the preparation chain instead of a fixture script standing in for
    // it — and the worker is exactly the code that cannot be reached from a libtest body (it
    // announces `READY` on stdout and blocks on stdin, which there is the runner's own).
    #[cfg(all(feature = "test-support", target_os = "macos"))]
    {
        let args: Vec<String> = std::env::args().skip(1).collect();
        if args.first().map(String::as_str) == Some("--app-update-macos-worker") {
            let outcome = futureos_lib::test_support::run_update_worker_for_tests(&args[1..]);
            if let Err(error) = &outcome {
                eprintln!("gui_main_thread_drive: update worker refused: {error}");
            }
            // The real entry point maps the result the same way (`helper_result`): 0 installed
            // or cancelled, non-zero for a refusal the parent must report.
            std::process::exit(if outcome.is_ok() { 0 } else { 1 });
        }
    }

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

    // The update-preparation chain runs here: after the data root is isolated (it writes this
    // process's own transaction under it) and before the app starts. See
    // `exercise_update_preparation` for why it can live nowhere else.
    #[cfg(all(feature = "test-support", target_os = "macos"))]
    if !is_companion {
        if let Some(root) = ISOLATED.get() {
            exercise_update_preparation(root);
            // After the fixture-helper chain: the same chain with *this* process as the
            // worker, which is where the worker's own body runs, plus the two worker arms
            // that need a contrarian candidate (a cancel with no `COMMIT`, and a probe that
            // outlives its deadline).
            exercise_update_installation(root);
            exercise_update_probe_timeout(root);
        }
    }

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

/// The update-preparation chain (`commands::update_macos`), end to end, reported rather than
/// asserted — this target's contract is that it never adds a red to a suite whose subject is
/// not its own code (see the module docs).
///
/// **Why here and nowhere else.** The chain's successful half — the bundle's executable
/// identity, both `probe` calls, `sync_tree`, the hand-off to the privileged helper — was
/// written as a libtest case and **withdrawn**: `probe` gives each candidate executable a 10 s
/// budget, and this host does not create that process within 10 s while a few hundred
/// instrumented libtest cases are doing the same (measured 2 of 4 rounds, both fixtures tried;
/// `coverage/2026-11/tauri/wave3.md` §4). The budget is a property of production, not of the
/// host, so it stays — the case moves to a process with no parallel suite instead. This one.
///
/// Two things it needs that a test target does not have, both supplied from outside and
/// neither of them a change to the path under test:
///
/// * a **cooperating helper** — preparation hands the bundle to `current_exe()` with
///   `--app-update-macos-worker` and requires `READY` back, and here `current_exe()` is this
///   driver, so the helper is a fixture script speaking the same three lines;
/// * a **payload and a target** — a real ad-hoc-signed `.app` in a temporary directory, tarred
///   the way an update carries it, because the signature and identity steps are code under
///   test too.
fn exercise_update_preparation(root: &std::path::Path) {
    // Beside each other on purpose: "preparation left nothing behind" must be a question about
    // the case directory, and a payload tree inside it would answer for it.
    let case = root.join("update-case");
    let payload = root.join("update-payload");
    let helper = root.join("update-helper");
    let built = build_update_payload(&payload, "driver-fixture");
    let mut checks: Vec<(&str, bool, String)> = Vec::new();

    let bundle = match built {
        Ok(bytes) => {
            checks.push((
                "the fixture bundle is ad-hoc signed and tarred",
                true,
                format!("{} bytes", bytes.len()),
            ));
            Some(bytes)
        }
        Err(error) => {
            checks.push(("the fixture bundle is ad-hoc signed and tarred", false, error));
            None
        }
    };

    if let Some(bytes) = bundle {
        match write_helper(&helper) {
            Err(error) => checks.push(("the fixture helper speaks the worker protocol", false, error)),
            Ok(()) => {
                checks.push((
                    "the fixture helper speaks the worker protocol",
                    true,
                    helper.display().to_string(),
                ));
                let target = case.join("FutureOS.app");
                // Preparation stages *beside* the target, so the directory it would be
                // installed into has to exist — the same precondition a real Applications
                // folder provides.
                if let Err(error) = std::fs::create_dir_all(&case) {
                    checks.push((
                        "the case directory exists for the staging directory to sit in",
                        false,
                        error.to_string(),
                    ));
                }
                let outcome = futureos_lib::test_support::prepare_update_bundle_for_tests(
                    "9.9.9", &bytes, target, &helper,
                );
                let reported = match &outcome {
                    Ok(completion) => completion.clone(),
                    Err(error) => error.clone(),
                };
                // Derived, not the string: the chain's verdict is the helper's completion, and
                // a refusal names where the chain stopped.
                let completion: Option<serde_json::Value> =
                    outcome.as_ref().ok().and_then(|text| serde_json::from_str(text).ok());
                checks.push((
                    "the chain reaches the helper and the helper's completion comes back",
                    completion.as_ref().is_some_and(|value| value["ok"] == true),
                    reported.clone(),
                ));
                checks.push((
                    "the completion says the bundle was restored, so commit removed the staging",
                    completion
                        .as_ref()
                        .is_some_and(|value| value["restored"] == true),
                    reported,
                ));
                let leftovers = directory_entries(&case);
                checks.push((
                    "preparation left nothing beside the bundle",
                    leftovers.is_empty(),
                    format!("{leftovers:?}"),
                ));

                // The cancel half, on the same bundle: prepared and then dropped, which must
                // close the helper and take the staging directory with it. In the field that
                // directory sits in `/Applications`.
                let cancelled = futureos_lib::test_support::prepare_and_cancel_update_for_tests(
                    "9.9.8",
                    &bytes,
                    case.join("FutureOS.app"),
                    &helper,
                );
                let after_cancel = directory_entries(&case);
                checks.push((
                    "a prepared update that is cancelled leaves no staging directory either",
                    cancelled.is_ok() && after_cancel.is_empty(),
                    format!(
                        "{}; {after_cancel:?}",
                        match &cancelled {
                            Ok(()) => "prepared then dropped".to_string(),
                            Err(error) => error.clone(),
                        }
                    ),
                ));
            }
        }
    }

    let failed: Vec<&str> = checks
        .iter()
        .filter(|(_, ok, _)| !ok)
        .map(|(name, _, detail)| {
            eprintln!("gui_main_thread_drive: update preparation MISMATCH: {name} ({detail})");
            *name
        })
        .collect();
    eprintln!(
        "gui_main_thread_drive: update preparation ran end to end against a temporary bundle \
         ({}/{}) derived values match{}",
        checks.len() - failed.len(),
        checks.len(),
        if failed.is_empty() {
            String::new()
        } else {
            format!("; failed: {}", failed.join(", "))
        }
    );
    // Under the driver's isolated root, so `finish` removes it; stated so nobody adds a
    // second cleanup that would then race it.
    let _ = std::fs::remove_dir_all(&payload);
}

/// Report a scenario's derived values in the vocabulary a leg reader greps for
/// (`... MISMATCH: <name> (<detail>)`), and never fail this target — its contract is that it
/// adds no red to a suite whose subject is not its own code.
#[cfg(all(feature = "test-support", target_os = "macos"))]
fn report_checks(step: &str, summary: &str, checks: &[(&str, bool, String)]) {
    let failed: Vec<&str> = checks
        .iter()
        .filter(|(_, ok, _)| !ok)
        .map(|(name, _, detail)| {
            eprintln!("gui_main_thread_drive: {step} MISMATCH: {name} ({detail})");
            *name
        })
        .collect();
    eprintln!(
        "gui_main_thread_drive: {step} {summary} ({}/{}) derived values match{}",
        checks.len() - failed.len(),
        checks.len(),
        if failed.is_empty() {
            String::new()
        } else {
            format!("; failed: {}", failed.join(", "))
        }
    );
}

/// The **real** worker, end to end: this process is the helper.
///
/// `exercise_update_preparation` hands the chain a fixture script, which is the right shape
/// for the parent's half — its subject is preparation. What such a script can never be is
/// `commands::update_macos::worker` itself: `READY` on stdout, `COMMIT` on stdin, the Agent
/// lease, the atomic bundle exchange and the journal write. That is a whole separate-process
/// protocol, and a libtest body's stdin and stdout *are* the runner's.
///
/// So the chain runs again with `current_exe()` as the helper — this target's `main`
/// dispatches `--app-update-macos-worker` to the production facade — and both halves of the
/// protocol are then real: `Prepared`/`PreparedProcess` on this side, `worker` on the child's.
/// A committed install *swaps* the two bundles, so what is asserted afterwards is derived from
/// both trees and from the record the worker wrote, not from the completion line alone.
#[cfg(all(feature = "test-support", target_os = "macos"))]
fn exercise_update_installation(root: &std::path::Path) {
    const INSTALLED: &str = "old-9.9.8";
    const CANDIDATE: &str = "new-9.9.9";
    let case = root.join("update-install");
    let payload = root.join("update-install-payload");
    let _ = std::fs::remove_dir_all(&case);
    let _ = std::fs::remove_dir_all(&payload);
    let mut checks: Vec<(&str, bool, String)> = Vec::new();
    let target = case.join("FutureOS.app");

    let staged = (|| -> Result<Vec<u8>, String> {
        write_installed_bundle(&target, INSTALLED)?;
        build_update_payload(&payload, CANDIDATE)
    })();
    let bytes = match staged {
        Ok(bytes) => Some(bytes),
        Err(error) => {
            checks.push((
                "the installed bundle and the incoming payload are staged",
                false,
                error,
            ));
            None
        }
    };

    if let Some(bytes) = bytes {
        let helper = std::env::current_exe().expect("this driver's own path");
        let outcome = futureos_lib::test_support::prepare_update_bundle_for_tests(
            "9.9.9",
            &bytes,
            target.clone(),
            &helper,
        );
        let reported = match &outcome {
            Ok(completion) => completion.clone(),
            Err(error) => error.clone(),
        };
        let completion: Option<serde_json::Value> = outcome
            .as_ref()
            .ok()
            .and_then(|text| serde_json::from_str(text).ok());
        checks.push((
            "the real worker announces READY, takes COMMIT and completes the install",
            completion.as_ref().is_some_and(|value| value["ok"] == true),
            reported.clone(),
        ));
        checks.push((
            "a successful install reports no restore, so the old bundle stays staged",
            completion
                .as_ref()
                .is_some_and(|value| value["restored"] == false),
            reported,
        ));

        let installed = bundle_version(&target);
        let staging: Vec<String> = directory_entries(&case)
            .into_iter()
            .filter(|name| name.starts_with(".futureos-"))
            .collect();
        let retired = staging
            .first()
            .map(|name| bundle_version(&case.join(name).join("candidate.app")))
            .unwrap_or_default();
        checks.push((
            "the installed bundle now carries the candidate",
            installed == CANDIDATE,
            format!("{installed:?}"),
        ));
        checks.push((
            "the bundle it replaced is retained beside it, for rollback",
            retired == INSTALLED,
            format!("{retired:?} in {staging:?}"),
        ));

        // The worker's own journal write, read back through the record the parent began: the
        // completion line is the worker's claim, this is the durable trace a restart reads.
        let id = staging
            .first()
            .map(|name| name.trim_start_matches(".futureos-").to_string())
            .unwrap_or_default();
        let record: Option<serde_json::Value> = std::env::var_os("FUTURE_HOME")
            .map(|home| {
                std::path::PathBuf::from(home)
                    .join("app/updates")
                    .join(&id)
                    .join("result.json")
            })
            .and_then(|path| std::fs::read(path).ok())
            .and_then(|bytes| serde_json::from_slice(&bytes).ok());
        checks.push((
            "the worker records the install as awaiting validation",
            record.as_ref().is_some_and(|value| {
                value["stage"] == "awaiting-validation"
                    && value["outcome"] == "pending-validation"
                    && value["restartSafe"] == true
            }),
            format!(
                "stage {:?}, outcome {:?}",
                record.as_ref().map(|value| value["stage"].clone()),
                record.as_ref().map(|value| value["outcome"].clone())
            ),
        ));
    }

    // The worker's cancel arm: `COMMIT` never arrives, so nothing may be replaced and the
    // staging directory must go. Driven in the worker's own process, because that is who the
    // parent protocol talks to.
    {
        let cancel_case = root.join("update-install-cancel");
        let _ = std::fs::remove_dir_all(&cancel_case);
        let target = cancel_case.join("FutureOS.app");
        let id = "update-driver-cancel";
        let staging = cancel_case.join(format!(".futureos-{id}"));
        let record = cancel_case
            .join("worker-home/app/updates")
            .join(id)
            .join("result.json");
        let prepared = (|| -> Result<(), String> {
            write_installed_bundle(&target, INSTALLED)?;
            std::fs::create_dir_all(staging.join("candidate.app"))
                .map_err(|e| e.to_string())?;
            std::fs::create_dir_all(record.parent().expect("a record directory"))
                .map_err(|e| e.to_string())
        })();
        let outcome = match prepared {
            Ok(()) => drive_update_worker_cancel(&target, id, &record),
            Err(error) => Err(error),
        };
        let kept = bundle_version(&target);
        checks.push((
            "a worker whose COMMIT never arrives replaces nothing and unstages itself",
            outcome.is_ok() && !staging.exists() && kept == INSTALLED,
            format!(
                "{}; version {kept:?}; staging {}",
                outcome.err().unwrap_or_else(|| "clean exit".to_string()),
                if staging.exists() {
                    "left behind"
                } else {
                    "gone"
                }
            ),
        ));
    }

    report_checks(
        "update installation",
        "ran the real worker against temporary bundles",
        &checks,
    );
    let _ = std::fs::remove_dir_all(&payload);
}

/// The bundle that is already installed: no signature and no executable, because nothing
/// probes or verifies it — an install only *swaps* it, and `CFBundleVersion` is the marker the
/// swap is read back through.
fn write_installed_bundle(app: &std::path::Path, version: &str) -> Result<(), String> {
    std::fs::create_dir_all(app.join("Contents")).map_err(|e| e.to_string())?;
    std::fs::write(
        app.join("Contents/Info.plist"),
        format!(
            "<?xml version=\"1.0\"?><plist version=\"1.0\"><dict>\
             <key>CFBundleVersion</key><string>{version}</string></dict></plist>"
        ),
    )
    .map_err(|e| e.to_string())
}

/// A bundle's `CFBundleVersion`, read through `PlistBuddy` — the same tool, and the same call
/// shape, that preparation itself uses to read `CFBundleExecutable`. An unreadable bundle is
/// the empty string, which no expected marker equals.
fn bundle_version(app: &std::path::Path) -> String {
    let output = std::process::Command::new("/usr/libexec/PlistBuddy")
        .args(["-c", "Print :CFBundleVersion"])
        .arg(app.join("Contents/Info.plist"))
        .output();
    match output {
        Ok(output) if output.status.success() => {
            String::from_utf8_lossy(&output.stdout).trim().to_string()
        }
        _ => String::new(),
    }
}

/// A worker child whose stdin is closed before `COMMIT`: it must announce `READY`, read the
/// end of input where the authorisation would have been, and undo its staging.
#[cfg(all(feature = "test-support", target_os = "macos"))]
fn drive_update_worker_cancel(
    target: &std::path::Path,
    id: &str,
    record: &std::path::Path,
) -> Result<(), String> {
    use std::io::BufRead;
    use std::process::{Command, Stdio};
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let mut child = Command::new(exe)
        .arg("--app-update-macos-worker")
        .arg(target)
        .arg(id)
        .arg(record)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| format!("cannot start the worker: {error}"))?;
    let mut line = String::new();
    std::io::BufReader::new(child.stdout.take().ok_or("the worker has no stdout")?)
        .read_line(&mut line)
        .map_err(|error| format!("cannot read the worker's first line: {error}"))?;
    if line.trim() != "READY" {
        let _ = child.kill();
        let _ = child.wait();
        return Err(format!("the worker announced {line:?}, not READY"));
    }
    // Closing stdin **is** the cancel: `await_commit` reads end of input and answers "not
    // committed", and the worker then removes the staging directory and exits normally.
    drop(child.stdin.take());
    let status = child.wait().map_err(|error| error.to_string())?;
    if !status.success() {
        return Err(format!("the cancelled worker exited with {status}"));
    }
    Ok(())
}

/// `probe`'s timeout arm: a candidate that outlives the deadline is killed and named.
///
/// Reaching it needs a program that really is slower than the 10 s budget, so this costs
/// +10 s — affordable in a driver with no parallel suite, and the reason the budget itself is
/// not widened in the libtest suite (it is production's; `wave3.md` §4).
#[cfg(all(feature = "test-support", target_os = "macos"))]
fn exercise_update_probe_timeout(root: &std::path::Path) {
    let directory = root.join("update-probe-timeout");
    let _ = std::fs::remove_dir_all(&directory);
    let mut checks: Vec<(&str, bool, String)> = Vec::new();
    let slow = directory.join("slow-candidate");
    let prepared = std::fs::create_dir_all(&directory)
        .and_then(|()| std::fs::write(&slow, "#!/bin/sh\nsleep 15\n"))
        .and_then(|()| {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&slow, std::fs::Permissions::from_mode(0o755))
        });
    match prepared {
        Err(error) => checks.push(("the contrarian candidate exists", false, error.to_string())),
        Ok(()) => {
            let started = Instant::now();
            let outcome = futureos_lib::test_support::probe_update_candidate_for_tests(&slow);
            let elapsed = started.elapsed();
            checks.push((
                "a candidate that outlives the probe deadline is killed and named a timeout",
                outcome
                    .as_ref()
                    .is_err_and(|error| error.contains("probe timed out")),
                format!(
                    "{:?} after {:.1}s",
                    outcome.err().unwrap_or_default(),
                    elapsed.as_secs_f64()
                ),
            ));
        }
    }
    report_checks(
        "update probe timeout",
        "expired the production deadline against a slow candidate",
        &checks,
    );
}

/// The `.app` tree a driver payload is built from, ad-hoc signed (the same signature a locally
/// built bundle gets) and returned as a gzip'd tar with one `FutureOS.app` entry.
///
/// The executable is a shell script, not a copied Mach-O: a re-signed copy of a system binary
/// has a signature macOS has never seen, so its first exec goes through `amfid`, and that is
/// the cost `probe`'s budget was being spent on in the libtest version of this case.
///
/// `version` is written to `CFBundleVersion` — **not** as a file of its own — because
/// `Contents/<anything>` that is not part of the standard layout stops `codesign --verify
/// --deep --strict` from accepting the bundle after a round trip through this crate's `tar`
/// (`code object is not signed at all … In subcomponent: …/Contents/version`, measured
/// 2026-11-01 with the bundle signed, verified, tarred and unpacked in one process; the same
/// bundle signs, round-trips through BSD `tar` and verifies). A marker inside `Info.plist`
/// survives it and is the field a real update bundle already carries.
fn build_update_payload(root: &std::path::Path, version: &str) -> Result<Vec<u8>, String> {
    let app = root.join("FutureOS.app");
    let macos = app.join("Contents/MacOS");
    std::fs::create_dir_all(&macos).map_err(|e| format!("create the fixture bundle: {e}"))?;
    let main = macos.join("future");
    std::fs::write(&main, "#!/bin/sh\nexit 0\n").map_err(|e| e.to_string())?;
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&main, std::fs::Permissions::from_mode(0o755))
        .map_err(|e| e.to_string())?;
    std::fs::write(
        app.join("Contents/Info.plist"),
        format!(
            "<?xml version=\"1.0\"?><plist version=\"1.0\"><dict>\
             <key>CFBundleExecutable</key><string>future</string>\
             <key>CFBundleIdentifier</key><string>com.futureos.driver</string>\
             <key>CFBundleVersion</key><string>{version}</string></dict></plist>"
        ),
    )
    .map_err(|e| e.to_string())?;
    let status = std::process::Command::new("/usr/bin/codesign")
        .args(["--force", "--deep", "--sign", "-"])
        .arg(&app)
        .status()
        .map_err(|e| format!("run codesign: {e}"))?;
    if !status.success() {
        return Err(format!("the fixture bundle could not be ad-hoc signed: {status}"));
    }
    let encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
    let mut archive = tar::Builder::new(encoder);
    archive
        .append_dir_all("FutureOS.app", &app)
        .map_err(|e| e.to_string())?;
    let encoder = archive.into_inner().map_err(|e| e.to_string())?;
    encoder.finish().map_err(|e| e.to_string())
}

/// The privileged helper, as far as preparation can tell it apart: `READY` on start, a
/// `COMMIT` line to authorize the install, one completion line back. The parent's side of this
/// — building the command, opening the worker log, requiring `READY`, reading the completion —
/// is the code under test.
///
/// `restored: true` is chosen, not incidental: `Prepared::commit` removes the staging directory
/// when the helper reports a restore, which is what makes "nothing left beside the bundle"
/// answerable after a *successful* chain.
fn write_helper(path: &std::path::Path) -> Result<(), String> {
    std::fs::write(
        path,
        "#!/bin/sh\n\
         printf 'READY\\n'\n\
         IFS= read -r request || exit 0\n\
         test \"$request\" = COMMIT || exit 1\n\
         printf '{\"ok\":true,\"restored\":true,\"diagnostic\":\"driver fixture\"}\\n'\n",
    )
    .map_err(|e| e.to_string())?;
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).map_err(|e| e.to_string())
}

/// The names in `directory`, sorted, so a mismatch names what is there instead of only that
/// something is. A missing directory reads as empty: preparation never has to create it.
fn directory_entries(directory: &std::path::Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(directory) else {
        return Vec::new();
    };
    let mut names: Vec<String> = entries
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}
