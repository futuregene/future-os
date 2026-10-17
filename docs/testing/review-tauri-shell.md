# Review — `module-tauri-shell` (§S8.3 = 150 / §S10.4 = 217 waiver rows)

**Reviewer unit:** REV-TAURI-SHELL · **Date:** 2026-10-17 · **Verdict: 90 of 217 residual rows FELL.**

Reviewed record: `docs/testing/module-tauri-shell.md`, ledgers **§S8.3** (150 rows, artifact
`/tmp/tauri-shell-lcov-c8union.info`) and **§S10.4** (217 rows, artifact
`/tmp/tauri-shell-lcov-c10union.info`). Source tree at `HEAD = 3065c8cb7`,
`desktop/src-tauri/src/lib.rs` sha256 prefix **`7306b52a724d3e5f`** — the same bytes the §S10
measurement describes (verified: every one of the 217 rows resolves to the source text the
record names, and every row is `DA:0` in the pinned c10 artifact). ℹ️ The unit ticket quoted
"289 tags ... in section 4"; **§4's 318 is obsolete and was not reviewed** — the two ledgers
above are.

## 0. Ledger integrity — checked first, all clean

| check | result |
|---|---|
| §S10.4 rows present in `/tmp/tauri-shell-lcov-c10union.info` | **217 / 217**, `DA:0` for every one |
| §S8.3 rows present in `/tmp/tauri-shell-lcov-c8union.info` | **150 / 150**, `DA:0` for every one |
| set difference S10 \ S8 | **76** = `menu.rs` 46 + `lib.rs` 29 + `runtime.rs` 1 ✔ (record's figure) |
| set difference S8 \ S10 | **9** (`lib.rs` 906/910/916/917/919/920/921/923/924) ✔, and all nine are **covered** in c10 (8,8,2,8,2,1,1,6,8) — they are *not* residual |
| the 75 `host-state-blocked` lines covered in the pinned c8 artifact | **75 / 75 covered** ✔ |
| `150 + 76 − 9 = 217` | exact |
| STALE rows (supervisor's new blind spot) | **0** — no ledger row claims a line the report shows covered; the only overstatement risk was `runtime.rs:46`, which the record already labels a truncation artifact (c8 = 7, c10 = 0) |

So nothing here is a pairing artefact. **The review target is 217 rows** (226 unique across
both ledgers; the 9 S8-only rows are excluded from verdicts below).

## 1. Method and scratch tree

Scratch checkout built with `git archive` (no `git worktree add`):
`/tmp/rev-tauri-shell-s10-88746`. No file under the campaign worktree's `desktop/` was
touched; the only file written there is this report.

The probes are ordinary tests in the scratch copy. Two of them needed a **main-thread**
target (`harness = false`), which is the technique §S8.1 already established.

```bash
R=/tmp/rev-tauri-shell-s10-88746
git archive HEAD | tar -x -C $R
: > $R/desktop/src-tauri/binaries/future-aarch64-apple-darwin   # tauri-build sidecar
ln -sfn "$PWD/node_modules" $R/node_modules

# cargo-llvm-cov 0.8.7 cannot find the toolchain's `llvm-tools` component (it looks for
# `llvm-tools-preview`); point it at the binaries instead of installing anything:
B=~/.rustup/toolchains/1.97.0-aarch64-apple-darwin/lib/rustlib/aarch64-apple-darwin/bin
export LLVM_COV=$B/llvm-cov LLVM_PROFDATA=$B/llvm-profdata

cd $R/desktop/src-tauri
cargo llvm-cov --no-report --lib -- rev_probe::tests::posting_the_workspace_notifications --test-threads=1   # P1 (before the gui probe existed)
cargo llvm-cov --lib --no-report -- gui::rev_probe_gui::an_exit_requested --test-threads=1   # P2
cargo llvm-cov --no-report --test rev_probe_main                          # P3 (harness=false, main thread)
cargo llvm-cov --no-report --test rev_probe -- --test-threads=1           # P4
LLVM_PROFILE_FILE=$R/desktop/src-tauri/target/llvm-cov-target/rev-pty3-%p.profraw \
  python3 /tmp/rev-pty-launch.py $R/desktop/src-tauri/target/llvm-cov-target/debug/futureos \
          /tmp/rev-tauri-shell-pty-home3                                   # P5
cargo llvm-cov report --lcov --output-path /tmp/rev-tauri-shell-pN.info     # after each run
```

Union of the five reports (max per line): **`/tmp/rev-tauri-shell-union.info`**, 1011 lines.
P5 is a superset of P3/P4 (same target dir, accumulated without cleaning); P1 (P1's own run)
and P2 are separate because a `cargo llvm-cov` *reporting* run cleans the profile directory —
that is why the union is built from five files rather than one.

## 2. Verdict summary

| §S10.4 group | rows | FELL | SURVIVED | UNVERIFIABLE | what fell |
|---|---|---|---|---|---|
| S10.20 `macos_power` observers | 10 | **10** | 0 | 0 | posting the three `NSWorkspace*Notification`s runs all three blocks |
| S10.11 `menu.rs` | 48 | **46** | 2 | 0 | main-thread mock app builds the whole menu (2 continuations stay attribution) |
| S10.9 launcher + bin argv | 36 | **27** | 2 | 7 | `forward_re_pair()` + spawning the bin (`--re-pair`, `--re-pair-reply`, pty launcher) |
| S10.7 `RunEvent::ExitRequested` | 11 | **6** | 5 | 0 | the *mock runtime's own loop* constructs the event and hands over a real `ExitRequestApi` |
| S10.2 `Ready`-gated setup | 29 | **1** | 28 | 0 | `lib.rs:993` ran in this session inside a real app; the rest are host-state (covered in §S8) |
| S10.1 callback window arms | 3 | 0 | 3 | 0 | — (mechanism corrected, see §4) |
| S10.3 menu click | 11 | 0 | 11 | 0 | — |
| S10.4 window close | 11 | 0 | 11 | 0 | — |
| S10.5 real-call failure arms | 11 | 0 | 0 | 11 | — |
| S10.6 `APP_HANDLE` emit | 1 | 0 | 1 | 0 | — (type is `OnceLock<AppHandle<Wry>>`, measured) |
| S10.8 mobile entry `run()` | 3 | 0 | 3 | 0 | — |
| S10.10 test-internal invariants | 9 | 0 | 9 | 0 | — |
| S10.12 `cfg(all(gui, not(test)))` | 11 | 0 | 11 | 0 | — (reason corrected) |
| S10.13 shutdown deadline arm | 3 | 0 | 0 | 3 | — |
| S10.15 `freeze_tasks` race pair | 2 | 0 | 2 | 0 | — |
| S10.16 block-end brace | 1 | 0 | 1 | 0 | — (neighbours 459 = 3) |
| S10.17 client-stack arms | 3 | 0 | 0 | 3 | — |
| S10.18 `coordinator` APP_HANDLE arm | 1 | 0 | 1 | 0 | — (same Wry-only type barrier) |
| S10.19 OS thread refusal | 7 | 0 | 0 | 7 | — |
| S10.21 `flock` re-acquire race | 5 | 0 | 5 | 0 | — |
| S10.22 Linux-only helper arm | 1 | 0 | 1 | 0 | — (correct as `platform-unmeasured`) |
| **total** | **217** | **90** | **96** | **31** | |

Per-line verdicts (every line of every group, from `/tmp/rev-tauri-shell-union.info`):

```text
S10.20 FELL  (10) macos_power.rs:21,22,23,24,26,27,29,30,31,32
S10.11 FELL  (46) menu.rs:26-29,31-40,42-47,49,51-59,61-63,65-70,72-78
       SURVIVED (2) menu.rs:41,48                     (attribution to the `)?)` continuation)
S10.9  FELL  (13) launch.rs:330,331,332,338,339,340,341,342,343,344,345,346,347
       FELL  (14) main.rs:29,33,34,38,39,41,42,43,44,51,58,70,71,72
       SURVIVED (2) launch.rs:333,334                 (only on `current_exe()` failure)
       UNVERIFIABLE (7) main.rs:9,46,47,48,73,74,75
S10.7  FELL  (6) lib.rs:1267,1268,1270,1271,1272,1273
       SURVIVED (5) lib.rs:1275,1276,1277,1278,1279  (the Confirm arms)
S10.2  FELL  (1) lib.rs:993
       SURVIVED (28) lib.rs:209-215,474-478,811,955,957,958,974,975,995,1005,1008,1010,
                       1011,1031,1032,1034,1038,1039
S10.1  SURVIVED (3) lib.rs:907,908,909
S10.3  SURVIVED (11) lib.rs:932-937,941-943,945,950
S10.4  SURVIVED (11) lib.rs:956,960,962-965,967-971
S10.6  SURVIVED (1) lib.rs:842
S10.8  SURVIVED (3) lib.rs:888,889,890
S10.10 SURVIVED (9) lib.rs:1727,1906,1907; runtime.rs:248,249,251,252; desktop_shutdown.rs:501
S10.12 SURVIVED (11) desktop_shutdown.rs:221,222,223,267-274
S10.15 SURVIVED (2) desktop_shutdown.rs:499,500
S10.16 SURVIVED (1) desktop_shutdown.rs:460
S10.18 SURVIVED (1) coordinator.rs:50
S10.21 SURVIVED (5) instance.rs:164-168
S10.22 SURVIVED (1) bin/futureos-headless.rs:11
S10.5  UNVERIFIABLE (11) lib.rs:1004,1006,1007,1033,1035,1036,1037,1227,1252,1253,1256
S10.13 UNVERIFIABLE (3) desktop_shutdown.rs:256,257,258
S10.17 UNVERIFIABLE (3) future_login.rs:354,355,482
S10.19 UNVERIFIABLE (7) coordinator.rs:165,166,183,184,185,186,187
```

## 3. The falls, with the exact construct

### 3.1 S10.20 — all 10 `macos_power` observer lines FELL

The record: "only the OS can fire them". The observers are blocks registered with
`-[NSNotificationCenter addObserverForName:object:queue:usingBlock:]`; posting the same
notification names to the same centre **runs them synchronously on the posting thread**.

```rust
#[test]
fn posting_the_workspace_notifications_runs_the_observer_bodies() {
    crate::macos_power::install_disconnect_notifier();
    let center = objc2_app_kit::NSWorkspace::sharedWorkspace().notificationCenter();
    unsafe {
        center.postNotificationName_object(NSWorkspaceWillSleepNotification, None);
        center.postNotificationName_object(NSWorkspaceWillPowerOffNotification, None);
        center.postNotificationName_object(NSWorkspaceDidWakeNotification, None);
    }
}
```

Measured: `macos_power.rs` 16–54 all count 1 — including 21–24 (sleep block, drives
`handle_system_suspend`), 26–27 (wake), 29–32 (power-off). No supervisor state exists in the
test process, so each body returns after its first statement; the *bodies* run, which is what
the rows claim cannot happen. **10 lines closed with no seam and no source change.**

### 3.2 S10.11 — 46 of the 48 `menu.rs` lines FELL

The record calls these `host-state-blocked` ("called from the setup hook", covered only in the
pinned `Ready` session). `build_macos_menu<R: Runtime>` is runtime-generic; its real
requirement is the **main thread** — running it from a libtest worker panics
(`muda-0.19.3/src/platform_impl/macos/mod.rs:328: muda::MenuChild can only be created on the
main thread`). A `harness = false` test target is the main thread and needs no `Ready`, no
window, and no healthy GUI session:

```rust
// tests/rev_probe_main.rs   [[test]] name="rev_probe_main", harness=false, required-features=["gui"]
fn main() {
    let app = tauri::test::mock_app();
    let menu = futureos_lib::build_macos_menu(app.handle()).expect("menu builds");
    assert_eq!(menu.items().unwrap().len(), 4);   // app menu, Edit, View, Window
}
```

Measured: every row from 26 to 78 counts **3**, except `41` and `48` (count 0, with 40=3/42=3
and 47=3/49=3) — exactly the two `)?)` continuations the record already calls
`attribution-artifact`. **46 rows closed.** The only concession: the constructor is private to
`mod menu`, so the scratch adds one visibility line (`pub use menu::build_macos_menu;`,
behaviour unchanged). That is a *test seam for reachability*, not a behaviour change — the
record's stated blocker is `Ready`/host state, and that blocker is gone.

Consequence for the ledger: `menu.rs` is not `host-state-blocked`. 46 of its 48 rows are
reachable on this host at any time; the remaining two are attribution and should stay in that
class.

### 3.3 S10.9 — 27 of 36 launcher/bin rows FELL

**`launch.rs:330-347` (13 of 15).** `forward_re_pair` is `pub`. Called from an integration
test, its "executable" is the test binary; the child rejects `--re-pair` immediately, which is
the hand-off-failure input `await_reply` exists for:

```rust
#[test]
fn forward_re_pair_runs_and_reports_a_failed_hand_off() {
    std::env::set_var("HOME", &home); std::env::set_var("FUTURE_HOME", home.join(".future"));
    let error = futureos_lib::launch::forward_re_pair().expect_err("child cannot service it");
    assert!(error.contains("did not accept the request"), "{error}");
}
```

Measured: 330,331,332,338–347 = 1. `333`/`334` remain 0 (the `current_exe()` error and its
`?` tail — only on a broken executable).

**`main.rs` (14 of 21).** The bin is a real entry point; `CARGO_BIN_EXE_futureos` spawns it:

```rust
Command::new(env!("CARGO_BIN_EXE_futureos")).arg("--re-pair").stdin(Stdio::null())   // line 29
// a second GUI launch while the first holds the data directory: lines 58, 70, 71, 72
```

and the **pty** probe (a fork with `stdin`/`stdout` on a pty slot, plus an external `flock`
holder on `$HOME/.future/app/desktop.lock`) drove the interactive branch:

```python
m, s = pty.openpty(); pid = os.fork()
if pid == 0: os.setsid(); os.dup2(s,0); os.dup2(s,1); os.dup2(s,2); os.execve(bin,[bin,'--re-pair'],env)
```

Measured: `main.rs` **29 = 2, 33 = 2, 34 = 2, 38 = 1, 39 = 1, 41–44 = 1, 51 = 1, 58 = 2,
70 = 4, 71 = 4, 72 = 4**. The launcher printed a complete pairing invitation and exited 0 —
i.e. `require_terminal` → `prepare_host_environment` → `another_desktop_is_running` →
`forward_re_pair` → reply → `invitation_notice` all executed on this host. **14 rows closed**
in a session the record says has neither a terminal nor a second Desktop.

Still not reached (7): `9` (the `--linux-sandbox-helper` **success** arm — on macOS the helper
fails, so this is `platform-unmeasured`, not "a different entry point"; see §4),
`46,47,48` (needs the hand-off child to exit unsuccessfully), `73,74,75` (`run_with` returning
`Err`; `initialize()` succeeds here — the CLI contract mismatch is only a warning).

### 3.4 S10.7 — 6 of 11 `ExitRequested` rows FELL

The record: "`tauri::ExitRequestApi` has private fields and only Tauri's own event loop
constructs it". True of *construction* — but the **mock runtime's own loop constructs it**
(`tauri-2.11.5/src/test/mock_runtime.rs:1352`) and passes it to the callback. So a test can
hand the real value straight to `dispatch_run_event`:

```rust
let window = tauri::WebviewWindowBuilder::new(&app, "main", Default::default()).build().unwrap();
std::thread::spawn({ let w = window.clone(); move || { sleep(250ms); w.destroy().unwrap(); } });
let outcome = catch_unwind(AssertUnwindSafe(|| {
    app.run(move |handle, event| {
        if let tauri::RunEvent::ExitRequested { .. } = &event {
            dispatch_run_event(handle, event, LaunchOptions::default());
            panic!("REV-PROBE: the exit-requested arm ran");
        }
    });
}));
```

Measured: 1267,1268,1270,1271,1272,1273 = 1 (the Proceed arm: `prevent_exit` + `request_quit`;
`desktop_shutdown::snapshot()` is published). The panic is the only way out of the mock loop
after `prevent_exit`; it is caught. **6 rows closed.** `1275-1279` (the `Confirm` arms) still
need active work or `QUIT_DIALOG_OPEN`, and the `open_dialog` arm opens a native dialog.

### 3.5 S10.2 — 1 row FELL, 28 stay host-state

`lib.rs:993` (`APP_HANDLE.set(...)` in the setup hook) is executed by the real GUI process the
pty probe starts — measured 4. The other 28 are the `Ready`-gated family; they are covered in
the pinned §S8 artifact (verified above) and were not observable in this session. `995` is the
first setup line after the handle is published and reads 0 — the app's setup did not proceed
past it in this session, which is consistent with `Ready` never firing.

## 4. Record corrections (line survives, stated reason is wrong)

1. **S10.11 `host-state-blocked` → main-thread, not `Ready`.** 46 of the 48 lines are covered
   by a main-thread target on this host (§3.2). This is the largest correction: it turns the
   biggest "wait for a healthy session" group into routine rework.
2. **S10.1 `needs-window` is not a design boundary.** The record says "a genuine waiver: the
   missing thing is a real window, which this driver (by design) does not create." Tauri
   creates the configured window; the driver notifies **before** the readiness wait, and in
   this session no window exists at that moment. Sending the notification *after* the
   readiness wait (or creating the window first) makes the three lines host-state-dependent,
   not structurally waived. They stay uncovered here, but they belong with the 75
   `host-state-blocked`, not in the waiver column.
3. **S10.7 "only Tauri's own event loop constructs `ExitRequestApi`"** — the mock runtime's
   loop does too, and is reachable from `tauri::test` (§3.4).
4. **S10.10 `lib.rs:1727` is *in* a test body** — the producer closure's early `return` when
   `send` fails. It is not "a closure a test body cannot enter"; it is simply not exercised by
   the current fixture (the producer is joined before the receiver is dropped). Same class for
   `1906-1907` (the `Failed` sink's `flush` `Ok` arm is never reached because the two call
   sites use `"write"` and `"flush"`). Both are one added call away, not structural.
5. **S10.12 `unreachable-by-construction` — `cfg(test)` is the wrong boundary.** `cfg(test)`
   is false for an **integration** build, so those blocks are compiled in every `tests/`
   binary; the real boundary is that `perform_stop`/`stop` are private and only a quit reaches
   them. (The pty run reached `RunEvent::Exit`? No — `lib.rs:1286` reads 0, so this session
   did not produce a clean exit through the shutdown path.) The row is not a candidate for
   "impossible": a spawned GUI that exits through `RunEvent::Exit` would execute it.
6. **S10.9 `main.rs:9` is `platform-unmeasured`, not "a different entry point".** The arm needs
   `--linux-sandbox-helper` to succeed; the helper is Linux-only (on macOS the same argv
   errors at the argument parse).

## 5. What I did not reach (stated, not implied)

* **All 217 rows** of §S10.4 were compared against the artifact and against the source text;
  **90 were executed**. I did **not** re-run the full suite, so I say nothing about rows whose
  status could only change through a full measurement.
* `S10.5` (11) — the failure arms of real calls: menu setup failure, window show/absent, the
  terminal bind (ephemeral port 0; the only failures are fd exhaustion, which the earlier
  steps in the same function also need), and agent startup (a no-op `Ok` in `cfg(test)`, and
  the non-test arm needs a `Ready` run). **I built no fault injection for these.**
* `S10.13` (3) — needs `remote::stop_gracefully` to overrun a 2 s budget; no knob inside this
  scope.
* `S10.17` (3) — needs a poll endpoint that returns a *retryable body* error (354/355) or a
  `reqwest::Error` with an empty `source()` chain (482). I did not stand one up; §6.5's claim
  that this host reports body-transfer failures as decode errors is plausible and untested
  here.
* `S10.19` (7) — OS thread-creation refusal; needs thread/`RLIMIT` exhaustion, which risks the
  test process.
* `main.rs:46-48`, `73-75`, `launch.rs:333-334`, `S10.7:1275-1279` — named constructs in §3;
  each needs a state I could not produce safely (a child that fails *after* the hand-off,
  `run_with` returning `Err`, a broken executable, an active-work quit).

## 6. Probes (source + hash, runnable)

Scratch tree: `/tmp/rev-tauri-shell-s10-88746` (kept for the supervisor). Probe files, hashes
computed from the bytes below:

| file | sha256 |
|---|---|
| `desktop/src-tauri/src/rev_probe.rs` (S10.20) | `92f3a5e7e51237d151573cd1638a58d80290ec4eb8a293ac3089cc3d1159be6a` |
| `desktop/src-tauri/tests/rev_probe.rs` (S10.9) | `b0f1422d9da2cec9643cd4eb25f057cbbbe7efe22a80f8acb5d40f45f378355a` |
| `desktop/src-tauri/tests/rev_probe_main.rs` (S10.11) | `4f527a48b96c90ae0ce5bf95f108aca66016ba146a704537690fed1027b8b068` |
| `/tmp/rev-pty-launch.py` (S10.9 pty launcher) | `18cdd05aea23737becc4570939ab28eb928a82b1182f8b58f1b6f7fb00b9a6ff` |

Independent confirmation artifact, **no test harness involved**: `/tmp/rev-pty-only.info`, the
`llvm-profdata merge` + `llvm-cov export` of **five** real processes from three separate
launcher invocations (`rev-pty-2320, rev-pty3-2514, rev-pty3-2520, rev-pty4-4910,
rev-pty4-4916`), each against its own isolated `HOME` with an external `flock` on
`$HOME/.future/app/desktop.lock` and stdin/stdout on a pty. It reproduces every S10.9 fall on
its own — `main.rs` 29/33/34/38–44/51/58/70/71/72 non-zero — while `menu.rs:26` stays **0** in
the same artifact, which is the cleanest demonstration in this review that the launcher path
is `Ready`-independent and the menu path is not.

`src/rev_probe.rs` — verbatim:

```rust
//! REV-TAURI-SHELL adversarial probes — scratch checkout only, never upstream.
//!
//! Each probe supplies an input a §S8.3/§S10.4 waiver says cannot exist.

#[cfg(test)]
mod tests {
    /// S10.20 (`macos_power.rs:21-24,26-27,29-32`): the ledger says the three
    /// observer bodies "only fire on real sleep/wake/power-off notifications
    /// (impossible to drive in a unit test)". They are blocks registered with
    /// NSNotificationCenter; posting the same notification names to the same
    /// center must run them, on any thread.
    #[test]
    fn posting_the_workspace_notifications_runs_the_observer_bodies() {
        use objc2_app_kit::{
            NSWorkspace, NSWorkspaceDidWakeNotification, NSWorkspaceWillPowerOffNotification,
            NSWorkspaceWillSleepNotification,
        };
        crate::macos_power::install_disconnect_notifier();
        let center = NSWorkspace::sharedWorkspace().notificationCenter();
        // SAFETY: the three names come from the same AppKit bindings the
        // registration uses, and `None` matches an observer registered with a
        // nil object — the delivery contract of NSNotificationCenter.
        unsafe {
            center.postNotificationName_object(NSWorkspaceWillSleepNotification, None);
            center.postNotificationName_object(NSWorkspaceWillPowerOffNotification, None);
            center.postNotificationName_object(NSWorkspaceDidWakeNotification, None);
        }
    }
}
```

`tests/rev_probe_main.rs` — verbatim:

```rust
//! REV-TAURI-SHELL probe — main-thread target, scratch checkout only.
//!
//! S10.11 says the 46 `menu.rs` lines are `host-state-blocked` because they are
//! only reached from the setup hook in a `Ready`-delivering session. The
//! builder is runtime-generic; its only real requirement is the *main thread*
//! (muda panics elsewhere). A `harness = false` target is the main thread.

fn main() {
    let app = tauri::test::mock_app();
    let menu = futureos_lib::build_macos_menu(app.handle()).expect("menu builds");
    let items = menu.items().expect("the menu lists its submenus");
    assert_eq!(items.len(), 4, "top-level submenus: {}", items.len());
    println!("rev_probe_main: built a {}-submenu menu on the main thread", items.len());
}
```

`tests/rev_probe.rs` — verbatim:

```rust
//! REV-TAURI-SHELL adversarial probes — scratch checkout only, never upstream.
//!
//! `futureos_lib::launch` and the `futureos` bin are the two production entry
//! points §S10.4 group S10.9 calls "a different entry point". These probes drive
//! them the way a shell does, so no private seam is needed.

use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

fn isolated_home(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "rev-tauri-shell-probe-{tag}-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).expect("isolated home");
    dir
}

/// S10.9 (`launch.rs:330-347`): `forward_re_pair` is public. Its own executable
/// is whatever process calls it; in a test binary that child rejects the argv
/// immediately, which is exactly the "hand-off process failed" input the unit
/// arm (373-380) exists for. This covers the launcher's body without a Desktop.
#[test]
fn forward_re_pair_runs_and_reports_a_failed_hand_off() {
    let home = isolated_home("forward");
    std::env::set_var("HOME", &home);
    std::env::set_var("FUTURE_HOME", home.join(".future"));
    let started = Instant::now();
    let error = futureos_lib::launch::forward_re_pair()
        .expect_err("the test binary cannot service --re-pair");
    assert!(
        error.contains("did not accept the request"),
        "unexpected reason: {error}"
    );
    assert!(
        started.elapsed() < Duration::from_secs(20),
        "the hand-off failure must be detected from the child's exit, not the 60s deadline"
    );
}

/// S10.9 (`main.rs:29`): `--re-pair` without a terminal is refused before any
/// state is touched. A piped stdin is the input; the bin is the entry point.
#[test]
fn the_bin_refuses_re_pair_without_a_terminal() {
    let output = Command::new(env!("CARGO_BIN_EXE_futureos"))
        .arg("--re-pair")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("spawn the futureos bin");
    assert!(!output.status.success(), "the refusal must fail the launch");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("needs an interactive terminal"),
        "stderr: {stderr}"
    );
}

/// S10.9 (`main.rs:58,70,71`): the forwarded-arm and the GUI entry both run when
/// a second instance finds the first. The first instance stays in its event
/// loop; the second exits inside the single-instance plugin's setup, which is a
/// clean `std::process::exit` and therefore flushes its profile.
#[test]
fn a_second_gui_launch_takes_the_forwarded_arm_and_the_desktop_entry() {
    let home = isolated_home("second-instance");
    let bin = env!("CARGO_BIN_EXE_futureos");
    let first = Command::new(bin)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .env("HOME", &home)
        .env("FUTURE_HOME", home.join(".future"))
        .spawn()
        .expect("spawn the first instance");
    // The plugin's listener bind was measured at ~25 s in this session
    // (§S11.10.2); give it that plus slack before asking a second process to
    // hand off.
    std::thread::sleep(Duration::from_secs(35));
    let reply = home.join("reply.json");
    let second = Command::new(bin)
        .arg("--re-pair-reply")
        .arg(&reply)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env("HOME", &home)
        .env("FUTURE_HOME", home.join(".future"))
        .spawn()
        .expect("spawn the second instance");
    let finished = second.wait_with_output().expect("second instance exits");
    // The first instance is not ours to keep; it holds the isolated HOME only.
    let mut first = first;
    let _ = first.kill();
    let _ = first.wait();
    assert!(
        finished.status.success(),
        "the second instance must exit cleanly through the hand-off: {:?}\n{}",
        finished.status,
        String::from_utf8_lossy(&finished.stderr)
    );
}
```

`src/lib.rs` — verbatim fragment inserted immediately before `mod gui`'s closing brace (so
`dispatch_run_event` is visible); the rest of `lib.rs` is untouched:

```rust
    // ------------------------------------------------------------------
    // REV-TAURI-SHELL probes — scratch checkout only, never upstream.
    // Placed inside `mod gui` because `dispatch_run_event` is private to it.
    // ------------------------------------------------------------------
    #[cfg(test)]
    mod rev_probe_gui {
        use super::*;

        /// S10.11 (`menu.rs:26-78`): `build_macos_menu` is generic over the
        /// runtime, so a MockRuntime app executes the whole builder — no
        /// `RunEvent::Ready` and no real AppKit session required.
        #[test]
        fn the_macos_menu_is_built_for_a_mock_app() {
            let app = tauri::test::mock_app();
            let menu = crate::menu::build_macos_menu(app.handle()).expect("menu builds");
            let items = menu.items().expect("the menu lists its submenus");
            assert!(items.len() >= 4, "top-level submenus: {}", items.len());
        }

        /// S10.7 (`lib.rs:1267-1272`): the mock runtime's own event loop builds
        /// `RunEvent::ExitRequested` (mock_runtime.rs:1352) and hands the real
        /// `ExitRequestApi` to the callback. Panicking after the arm unwinds the
        /// loop, which is how a probe leaves it.
        #[test]
        fn an_exit_requested_event_from_the_mock_loop_takes_the_proceed_arm() {
            crate::desktop_shutdown::reset_for_test();
            let app = tauri::test::mock_app();
            let window = tauri::WebviewWindowBuilder::new(&app, "main", Default::default())
                .build()
                .expect("mock window");
            let destroyer = window.clone();
            std::thread::spawn(move || {
                std::thread::sleep(std::time::Duration::from_millis(250));
                destroyer.destroy().expect("queue the destroy");
            });
            let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                app.run(move |handle, event| {
                    if let tauri::RunEvent::ExitRequested { .. } = &event {
                        dispatch_run_event(handle, event, crate::launch::LaunchOptions::default());
                        panic!("REV-PROBE: the exit-requested arm ran");
                    }
                });
            }));
            let error = outcome.expect_err("the probe panics out of the mock loop");
            let message = error.downcast_ref::<&str>().copied().unwrap_or_default();
            assert!(
                message.contains("REV-PROBE"),
                "unexpected panic: {message}"
            );
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
            while crate::desktop_shutdown::snapshot().is_none()
                && std::time::Instant::now() < deadline
            {
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            assert!(
                crate::desktop_shutdown::snapshot().is_some(),
                "the Proceed arm must have begun the guarded quit"
            );
        }
    }
}
```

Scratch-only edits (all in `/tmp/rev-tauri-shell-s10-88746`, none upstream):
`#[cfg(test)] mod rev_probe;` appended to `lib.rs`; the `rev_probe_gui` module inserted before
`mod gui`'s closing brace; `pub use menu::build_macos_menu;` (one visibility line);
`[[test]] rev_probe_main` stanza in `Cargo.toml`.

## 7. What the record's numbers become

| | before | after this review |
|---|---|---|
| §S10.4 residual rows | 217 | **127** (90 executed here) |
| of which `host-state-blocked` | 75 | **28** + 3 `needs-window` (reclassify, §4.2) |
| of which real-OS / alternate-entry-point | 104 | **61** |
| `unreachable-by-construction` | 19 | 19 (one reason corrected) |
| `attribution-artifact` | 3 | 3 (all verified: neighbours covered) |
| client-stack + thread refusal | 11 | 11 |
| `headless-lib-truncation` | 1 | 1 |
| `platform-unmeasured` | 1 | 1 |

The 46 `menu.rs` rows and the 10 `macos_power` rows are not rework for the coverage unit's
*product* — they are rework for its **ledger**: the code is reachable and the tests are cheap.
The 27 launcher/bin rows remove "the launcher process is a different entry point" from the
waiver vocabulary: the bin is spawnable, the pty is constructible, and the reply path works in
this very session.

**Quotable:** of 217 residual rows, **90 (41 %) fall** when attacked with the inputs the ledger
says cannot exist — and the two largest falls (`menu.rs` 46, `macos_power` 10) are the two
groups the record defended with a *comment* ("called from the setup hook", "impossible to drive
in a unit test"). The campaign's rule holds: a waiver that has not been attacked is a
hypothesis.

## 8. Environment provenance, the one probe that hung, and the timing question

Added after the supervisor flagged a 6-minute, 0 %-CPU `cargo-llvm-cov … an_exit_requested`
process. **That process was mine and it is gone (re-checked at the end of the run: no
`cargo-llvm-cov`, no `futureos_lib-*`, no `futureos` process).** Its cause was *not* the GUI
wait, and the distinction matters because a wrong cause would keep the six S10.7 rows marked
unreachable forever.

**8.1 The environment claim is right, and I confirmed it independently.** §S10.3's mechanism
reproduces: in the real GUI processes started by this review (`/tmp/rev-pty-only.info`, three
launcher invocations / five processes), the app's setup hook publishes the handle —
`lib.rs:993 = 1`, `994 = 1` — and then
`995` (`macos_power::install_disconnect_notifier`) reads **0**, with `1004`/`1005`
(`build_macos_menu`) **absent from the line table**, and `menu.rs:26 = 0`. No `Ready`, so no
menu, so `menu.rs` is exactly as unreachable *through the real app* as §S10.3 says. The
supervisor's three host measurements (`launchctl print gui/$(id -u)` → 125,
`screencapture` → "could not create image from display", the driver's `without` line) are
consistent with that and I did not re-measure them.

**8.2 The hang was my fixture, not the GUI.** First version of the S10.7 probe queued
`window.destroy()` *before* `app.run` and then `catch_unwind`'d a panic thrown inside the run
callback. The panic is caught, but unwinding out of `app.run` leaves the mock runtime's run
state half-torn-down and the process then blocks in teardown at **0 % CPU with no `rustc`** —
the shape the supervisor saw. Fix: defer the `destroy()` to a helper thread 250 ms *after* the
loop starts, so the mock loop legitimately reaches `ExitRequested` and the callback panics
*after* the arm under test. Then:

```
test gui::rev_probe_gui::an_exit_requested_event_from_the_mock_loop_takes_the_proceed_arm ...
test result: ok. 1 passed; 0 failed ... finished in 1.02s
```

**1.02 s**, first try, in this session. So `RunEvent::ExitRequested` is not environment-blocked
here: the mock runtime constructs the API value itself (`mock_runtime.rs:1352`) and never
consults `Ready`. Recording this prevents a false `unreachable-in-this-environment` on six
rows.

**8.3 No verdict in this review rests on a timing observation.** The measurement that decides
every fall is a **count** (`DA:n`, n > 0), never a wall-clock bound. Two probes do wait, and
here is exactly what each wait does and does not gate:

| wait | what it gates | does any verdict depend on it? |
|---|---|---|
| 250 ms before `destroy()` | ordering only — the mock loop must reach `Ready`/its message pump before the destroy arrives | No: the arm is reached either way; the wait only avoids tearing the loop down under the panic |
| 35 s settle before the second launch (load average at measurement: **3.15 / 4.47 / 9.12** on 32 cores) | whether the *first* instance has its single-instance listener up, i.e. whether the hand-off is acknowledged | **No.** The four `main.rs` rows that test contributed (`58,70,71,72`) are selected by the second process's own `argv` and were independently reproduced by the pty launcher with **no test harness and no second Desktop** (`/tmp/rev-pty-only.info`: `58=2, 70=3, 71=3, 72=3`, `29=3, 33=3, 34=3, 38=2 … 44=2, 51=1`) |

For the record, the budget-sensitive rows in the ledger were **not** claimed: S10.13 (the 2 s
remote-shutdown overrun), S10.5's terminal-bind arm and S10.19 (thread refusal) stay
UNVERIFIABLE, and `instance.rs` 164-168 stays SURVIVED — it is a race window, and I could not
distinguish "this host never loses that race" from "the code cannot", so I report it as
**timing-sensitive, unconfirmed**, not as a fall.

**8.4 Machine verification of the ledger I reviewed** (`reconcile.py`, the §S10.4 pairing —
`pairings.json` still points `tauri-shell` at the superseded 318 figure, so this is the correct
invocation for the current ledger):

```bash
ROOT=$PWD
# zsh does not word-split a plain $VAR, so the scopes come from a command substitution
# (this is the exact form that produced the output below):
python3 .future/cov100/reconcile.py --report /tmp/tauri-shell-lcov-c10union.info \
  --doc docs/testing/module-tauri-shell.md --section S10.4 --label tauri-shell-s10 \
  $(for f in bin/futureos-headless.rs desktop_shutdown.rs desktop_shutdown/coordinator.rs \
            future_login.rs host_environment.rs instance.rs launch.rs lib.rs \
            macos_power.rs main.rs menu.rs runtime.rs; do
      printf -- '--scope %s ' "$ROOT/desktop/src-tauri/src/$f"
    done)
# [tauri-shell-s10] files with uncovered lines=11 uncovered lines=217
#   mentioned in module-tauri-shell.md: 217
#   MISSING (uncovered, not mentioned): 0
```

217 uncovered = 217 mentioned, **0 MISSING**, **0 STALE** — the ledger does not overstate or
understate its own artifact under the supervisor's new STALE half.

## 9. The landing patch (for the unit that owns `desktop/`)
This review's contract forbids editing `desktop/`, so the four landings are handed over as a
**ready-to-apply patch** generated from the very tree in which all five probes passed:

| | |
|---|---|
| patch | `/tmp/rev-tauri-shell-landing.patch` |
| sha256 | `4b9b51e7e5602bcd78ae03239785a6c0d15884bc3ae08c87c73bd8d4a29f2596` |
| size | 250 lines, 5 files (`Cargo.toml`, `src/lib.rs`, `src/rev_probe.rs`, `tests/rev_probe.rs`, `tests/rev_probe_main.rs`) |
| validated | `patch -p1 --dry-run` **exit 0** against a clean `git archive HEAD` (HEAD `3065c8cb7`); after applying, all five files are **byte-identical** (`cmp -s`) to `/tmp/rev-tauri-shell-s10-88746`, i.e. the patch reproduces exactly the tree that produced every fall in §2 |

The only two hunks not already visible verbatim in §6 are the insertion points:

```diff
--- a/desktop/src-tauri/Cargo.toml
+++ b/desktop/src-tauri/Cargo.toml
@@ -14,6 +14,12 @@
 name = "futureos_lib"
 crate-type = ["staticlib", "cdylib", "rlib"]
 
+[[test]]
+name = "rev_probe_main"
+path = "tests/rev_probe_main.rs"
+harness = false
+required-features = ["gui"]
+
 [[bin]]
 name = "futureos-headless"
 path = "src/bin/futureos-headless.rs"
--- a/desktop/src-tauri/src/lib.rs
+++ b/desktop/src-tauri/src/lib.rs
@@ -180,6 +180,8 @@
 };
 #[cfg(feature = "gui")]
 pub use gui::{run, run_with};
+#[cfg(feature = "gui")]
+pub use menu::build_macos_menu;          // one visibility line; no behaviour change
 #[cfg(not(feature = "gui"))]
 mod no_gui;
@@ -2725,4 +2727,69 @@
             );
         }
     }
+
+    // … §6's `mod rev_probe_gui` verbatim, inserted before `mod gui`'s closing brace
 }
+
+#[cfg(test)]
+mod rev_probe;
```

Apply and confirm, from the owning unit's worktree:

```bash
git apply --check /tmp/rev-tauri-shell-landing.patch      # or: patch -p1 --dry-run < …
git apply /tmp/rev-tauri-shell-landing.patch
cd desktop/src-tauri
cargo fmt --check && cargo clippy --all-targets -- -D warnings
cargo test --lib rev_probe --test-threads=1               # P1: 1 passed
cargo test --test rev_probe_main                          # P3: prints "built a 4-submenu menu"
cargo test --test rev_probe -- --test-threads=1           # P4: 3 passed, 35.2 s
```

Then the measurement (§1's five commands) and the record edits — **the patch deliberately
contains no `docs/` change**: `module-tauri-shell.md`'s §4/§S8.3/§S10.4 text, the four
reclassifications of §4, and the new residual figure belong to the unit that owns the record,
and writing them here would recreate the stale-number defect this campaign keeps paying for.

**Two defects in this todo's own contract, recorded so the graph does not carry them forward:**
(1) it was created with `--verify python3 .future/cov100/reconcile.py --section S10.4`, which
**cannot exit 0** — `reconcile.py` requires `--report`, `--scope` and `--doc`, so that command
prints its usage error no matter what the ledger says (the working invocation is §8.4);
(2) it asks *this* unit to land files under `desktop/`, which the same contract forbids. Both
are corrected in the successor, not papered over.
