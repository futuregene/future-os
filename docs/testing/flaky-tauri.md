# Flaky-test record — desktop backend (`desktop/src-tauri`, lib target)

Owns FLAKES.md item 27 for the cov100 round-2 campaign: the **destructive `PATH` leak in
`cli_tool.rs`**, plus the three defects this work found on the way — a silent run-truncation,
a wall-clock deadline in the probe, and **a defect in how the campaign invokes its own test
suites** (NEW-C), which turned out to be the mechanism behind most of the "load-sensitive
flake" evidence, this unit's included.

| item | shape | outcome |
| --- | --- | --- |
| **27** — `cli_tool.rs` leaks `PATH` on the failure path and fails ~25 other tests | destructive, order-dependent (one panic poisons every later process-spawning test) | **fixed**: a panic-safe `EnvGuard` in all four mutating tests; controlled arms show the cascade go `25 → 0` |
| **NEW-A** — a run dies silently, mid-test, with **exit status 0** and no `test result:` line | destructive, concurrency-dependent (machine-wide Tauri single-instance socket) | **found, mechanism named, not fixed** (fix is in `src/lib.rs`, not this unit's scope) — see §6 |
| **NEW-B** — `probe`'s 3 s deadline is a wall-clock budget a busy host can blow | wall-clock budget vs scheduler | **fixed** (§7.1): thread-local `#[cfg(test)]` deadline seam, the shape `cli/commands/doctor.rs` uses; production budget pinned by its own test; deterministic control and a sensitivity control both landed. **But the failures attributed to it do not reproduce once NEW-C is fixed** (§7.2) |
| **NEW-C** — the campaign's inner-loop invocation silently runs the tests in **parallel** | harness/measurement defect | **found, proved** (§9): argv and wall-time evidence; one token in the command fixes it. This is the mechanism behind the `127` failures and, most likely, behind the "load-sensitive" rates taken with that form |
| **NEW-D** — a `PATH` swap by one test is visible to every test running **concurrently** | destructive, concurrency-dependent (item 27's second face) | **named with rates** (§10): 48 of 49 failures under NEW-C's form are `exit status: 127`; 0 under the serialized form |

## 1. Run identity and environment

| | |
| --- | --- |
| unit / todo | `worker-tauri-headless`, `todo_16d1326ad286` (goal `goal_160bc247aa63`) |
| worktree / branch | `/Users/geilige/future-os/.worktrees/cov100-r2`, `test/cov100-r2`, campaign baseline `8449acf6` |
| host | macOS (darwin, arm64), 32 CPUs, shared with ~13 campaign workers; load average **7–34** while measuring |
| suite | `desktop/src-tauri` lib target (`futureos`), default (gui) features: **2074 tests** during the arm runs (12 ignored); the count drifts as peers land in-flight tests (a `--list` at 00:16 said 2068) |
| toolchain | `rustc 1.97.0-aarch64-apple-darwin` (repo `rust-toolchain.toml`) |
| measured source | `desktop/src-tauri/src/cli_tool.rs`, pre-fix shasum `e4f2538acbf980f35166eb0fd132ed6b7ee37d03` (the only file this unit changed besides this record) |

Every run below used this environment (the flake harness of `FLAKES.md` §2.3 — **no
instrumentation**, plain `cargo test`, so no lcov number belongs to this workstream):

```bash
cd /Users/geilige/future-os/.worktrees/cov100-r2/desktop/src-tauri
export PATH=/Users/geilige/.rustup/toolchains/1.97.0-aarch64-apple-darwin/bin:$PATH
export CARGO_HOME=/Users/geilige/.cargo RUSTUP_HOME=/Users/geilige/.rustup
export HOME=/tmp/flake27/home          # the real ~/.future must not leak into tests
mkdir -p "$HOME"
# judgement command (identical in every arm):
/opt/homebrew/bin/timeout -k 5 5400 cargo test --lib -j 4 -- --test-threads=1
```

Drivers and logs (scratch, gitignored or `/tmp`): `/tmp/flake27/run-suite.sh` (cargo),
`/tmp/flake27/run-binary.sh` (the built binary, no cargo lock), `/tmp/flake27/classify.py`
(per-run failure classification), `/tmp/flake27/inject.py` (the measurement injection),
`/tmp/flake27/concurrent-probe.sh` (the NEW-A control), logs `/tmp/flake27/*.log`.
`--test-threads=1` is used because the incident was serial: the leaked value must reach the
*next* test in the harness order (libtest runs one test at a time, in name order).

## 2. Item 27 — the mechanism

`cli_tool.rs` has four tests that replace the process-global `PATH` with a fixture
directory and restore it as their **last statement**. Pre-fix shasum `e4f2538a` (the
supervisor's function lines 441/469/499/531 are the `fn` lines; the mutation is 6–16 lines
below each):

| test | `set_var` | first assertion that can panic after it | reachable by a real failure? |
| --- | --- | --- | --- |
| `a_runnable_cli_on_path_is_discovered_and_probed` | 447 | 449 (`discover(...).expect(...)`) | yes — `discover` has a 3 s probe deadline the host load can exceed |
| `a_missing_capability_is_reported_with_the_cli_identity` | 475 | 478, then 482/485 | yes, same deadline |
| `a_cli_that_changes_during_discovery_is_rejected` | 515 | 518 (with `discover` at 517) | yes, same deadline |
| `prepare_path_prepends_the_discovered_cli` | 537 | 539, 540, 542 | yes, `prepare_path` resolves the CLI |

`cargo test` runs every test of one target in **one process**, so when an assertion in that
window fails the value survives the unwind, the fixture `TempDir` is dropped (the directory
is deleted), and **every later test in the binary that spawns a process** inherits a
`PATH` that resolves nothing: `git`, `mkfifo`, `dd`, `sh`-script bodies — hence
`Os { code: 2, kind: NotFound }` in modules that never touch `cli_tool`. This is the class
`FLAKES.md` §4c describes; the one thing §4c adds and this unit kept is that
**a `Mutex` is not a restore** (`crate::TEST_HOME_LOCK` serialises these tests but restores
nothing when a test panics).

## 3. Item 27 — measurement

Two arms, one variable: the **guard**. Both arms carry the *same* injected failure, so the
only difference is the fix. The injection (`assert_eq!(descriptor.version, "1.2.3")` →
`"INJECTED-arm"`) is a deliberately wrong expectation placed after the `set_var` and before
the restore — a forced panic in the leak window, not a weakened or removed assertion (the
test and every other assertion are unchanged, and the injection is removed from the final
tree — §5).

| run | source | command | outcome |
| --- | --- | --- | --- |
| `base-clean` | pristine, **no injection** (natural) | `cargo test --lib -j 4 -- --test-threads=1` | **completed**: 2015 passed / **47 failed** / 12 ignored, wall 1302 s, load 24→34. Of the 47, **25 are the `PATH` cascade** (`commands::review` 5, `commands::update::recovery` 1, `git_review` 19, all `NotFound`/`failed to spawn`); the other 22 are load-bound (`agent_host` 45 s child deadline ×13, `headless`/`skills_bootstrap` probe deadlines, one peer's in-flight test) |
| `armA-forced` | pristine **+ injection** | same | **truncated at 161 s** (NEW-A, §6) with **31** failing tests: the injected one, `a_silent…` (a `PATH`-dependent sibling), 4 `commands::remote_peer` (load), and **the same 25 cascade failures** |
| `armA-forced-2` | same | same | truncated at 159 s, 27 failing names (same families) |
| `armA-direct` | same, binary run directly | `$BIN --test-threads=1` | truncated at 163 s, `RAW_EXIT=0`, 31 failing |
| `armA-skip-leak` | same binary, injected test skipped | `$BIN --test-threads=1 --skip cli_tool::tests::a_runnable_cli_on_path_is_discovered_and_probed` | **completed**: 2029 passed / **32 failed** / 585 s — and it still shows **25 cascade failures**, because the *other three* unguarded sibling tests leak the same way when they panic. Independent confirmation that the leak is not a property of the injected test |
| **`armB-forced`** | **fix + the same injection** | identical to `armA-forced` | truncated at 201 s (NEW-A); **5 failing tests: the injected one + 4 `cli_tool` probes that hit the 3 s deadline (NEW-B, §7) — cascade 0**. All 26 `git_review`, all 13 `commands::review`, all 6 `commands::update::recovery` tests that died in arm A **passed**; `a_silent…` passed |
| `final-clean` | fix, **no injection** | same as `armA-forced` | truncated at 318 s (NEW-A); **0 failures in the whole covered region**, including all 13 `cli_tool` tests and every `git_review` test |

Supporting controls, each one command and its exact result:

* **The leak's symptom in a single test.** `$BIN --test-threads=1 cli_tool::tests::a_silent_cli_is_killed_and_reported_as_a_timeout` — normal `PATH`: `ok` in 3.01 s; `PATH=/tmp/flake27/emptydir`: **FAILED in 0.18 s** (its fixture script `sleep 30` cannot find `sleep`, so the probe reports "exited with" where the test asserts "timed out").
* **Propagation across the module order** (second, independent construction): a panic injected into `a_cli_that_changes` — the *2nd* `cli_tool` test — makes `$BIN --test-threads=1 cli_tool::` report **3 failed / 10 passed**: the injected test, `a_flooding` (its `dd` fixture), and **`a_silent`, five tests later**. Same rule, different site: a panic in the window leaks for the rest of the binary.
* **Post-fix, alone.** `cargo test --lib -j 4 -- --test-threads=1 cli_tool::` → **13 passed / 0 failed**; neighbourhood `git_review::` → **26 ok**, `commands::review::` → **13 ok**, `commands::update::recovery::` → **6 ok** (the `FLAKES.md` §3.4 "rate the neighbourhood" requirement).
* **Rates, stated honestly.** Item 27's own trigger is *probabilistic* (a load-induced panic inside the window: 5 `cli_tool` failures in the load-34 natural run, 0–4 in the arms at load 7–16), while the *cascade* it produces once it fires is deterministic: **25/25 → 0/25** across the two arms, plus 25 again in the skipped-injection run of the same unguarded binary. `armA`/`armB` each ran once under the identical command and injection; every full-run datum (complete or truncated) is listed above so the reader can see which region each count covers.

## 4. Item 27 — the fix

`desktop/src-tauri/src/cli_tool.rs`: a `#[cfg(test)] pub(crate) struct EnvGuard` whose
`Drop` puts the previous value back, used by all four tests in place of the
`let saved = …; set_var(…); … match saved { … }` triple:

```rust
#[cfg(test)]
#[must_use = "the previous value is restored when the guard is dropped; bind it to a variable"]
pub(crate) struct EnvGuard {
    key: &'static str,
    saved: Option<std::ffi::OsString>,
}

impl EnvGuard {
    /// Replaces the variable `key` and remembers what to put back on drop.
    pub(crate) fn set(key: &'static str, value: &std::ffi::OsStr) -> Self {
        let saved = std::env::var_os(key);
        std::env::set_var(key, value);
        Self { key, saved }
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        match self.saved.take() {
            Some(value) => std::env::set_var(self.key, value),
            None => std::env::remove_var(self.key),
        }
    }
}
```

Per test the diff is `let _path = EnvGuard::set("PATH", directory.as_os_str());` in place of
the two lines, and the trailing `match saved { … }`. Two properties that make it correct
here, both of which the adopting sites must keep:

* **Drop order.** The guard is declared *after* `root`/`directory`, so it drops **before**
  the fixture `TempDir` is removed; the restored value therefore points at a live
  directory again. It runs during unwinding, which is the whole point.
* **The lock stays.** `path_lock()` (`crate::TEST_HOME_LOCK`) still serialises the tests;
  the guard only makes the value panic-safe. A lock is not a restore (§2).

`EnvGuard` is `pub(crate)`, so any test module in the crate can adopt it as
`crate::cli_tool::EnvGuard`; promoting it to a shared test-support module is the natural
follow-up (it is why the type is not private to `mod tests`).

### 4.1 The pattern for the other 214 sites

`.future/cov100/global-leak-scan.py` reports 218 *shapes* ("mutates a global and can panic
before the last restore"; desktop 70, orchestration 38, tui 27, packages 6, cli 4, agent 2,
channels 1). Adoption rule, applied here:

1. The mutation and its restore become **one** statement: `let _g = EnvGuard::set(key, value);`.
   Nothing that can panic may sit between them.
2. **Keep the serialising lock** where it exists; the guard does not serialise.
3. **Check the declaration order** against anything the value points at (a `TempDir`, a
   socket path, a file fixture): the guard must drop first.
4. `HOME`, `USERPROFILE`, `FUTURE_HOME`, `PATH`, `WT_SESSION`, the cwd and signal
   dispositions are all the same shape; the scanner's per-file list is the work list, and
   this file is the reference for what "fixed" looks like.
5. Watch for the *composite* case: a helper that owns several globals (e.g. `HOME` +
   `PATH`) needs either one guard per key or a struct that holds both.

## 5. What the fix does **not** address

* The `cli_tool` tests' **3 s probe deadline** (`probe`, `Duration::from_secs(3)`) is still
  a load-sensitive assertion: at load 34 every `cli_tool` probe in `base-clean` failed with
  `CLI discovery timed out`. That is a separate, *non-destructive* flake (it fails only its
  own test), and the deadline is production behaviour — tightening or seaming it is a
  product decision, not this unit's. It is recorded here because it is the **trigger** that
  makes the leak fire in practice.
* The injection used for the arms is not in the tree: the final `cli_tool.rs` contains the
  guard and the original assertions (verified by shasum and by `grep INJECTED` = 0).
  Note a **campaign hygiene incident**: peers' "safety snapshot" commits ran while the
  measurement injections were on disk, so the branch history now contains
  `34cb6ce2` (00:45, `INJECTED-arm`), `a746c411` (01:10, the fix, clean) and `0b123cbc`
  (01:22, `INJECTED-arm2`) — two of them with a deliberately wrong expectation inside
  `cli_tool.rs`. This unit's final commit restores the guarded file, so the **tree** is
  correct; the history entries need the supervisor's awareness (not a rewrite — they are
  peers' commits and squash-merged PRs flatten them anyway). Any scratch edit to a tracked
  file during measurement is exposed to this; `git status` was clean of the injections at
  the end (`grep INJECTED desktop/src-tauri/src/cli_tool.rs` = 0).

## 6. NEW-A — a full-suite run can die silently with exit status 0 (mechanism named)

Five of my runs — `armA-forced`, `armA-forced-2`, `armA-direct`, `armB-forced`,
`final-clean` — ended **mid-line**, with **no `test result:` line**, and the harness
reported **success** (`EXIT=0`, `RAW_EXIT=0` for the direct run). All five stopped in the
same test, `gui::startup_paths_tests::the_desktop_registration_builds_for_any_runtime`
(the log's last line is that test's name, printed, with no result). The runs that
completed (`base-clean` 1302 s, `armA-skip-leak` 585 s) did so when no competing
app-building process was active.

Mechanism, read in the dependency and then reproduced:

* `tauri-plugin-single-instance 2.4.3`, `src/platform_impl/macos.rs:25-29`: the plugin's
  `setup` calls `notify_singleton(&socket)` and, on `Ok(_)` — i.e. **some other live process
  answered** — runs **`std::process::exit(0)`**. No error, no output: the run just stops.
* The socket is a **fixed, machine-wide** path: `socket_path()` (same file, lines 61-72) is
  `/tmp/{identifier}_si.sock`. The mock config the test builds with
  (`tauri::test::mock_context`, `tauri-2.11.5/src/test/mod.rs:116`) sets
  `identifier: Default::default()` — the empty string — so the mock app's socket is
  `/tmp/_si.sock`: shared by every process on this host that builds that app, **across
  worktrees**. (`ls -la /tmp/*_si.sock` shows `/tmp/_si.sock` alongside the production
  `com_futureos_desktop_si.sock` and per-pid `com_futureos_test_<pid>_si.sock`.)
* Control pair (same binary, same filter, seconds apart): `$BIN --test-threads=1
  gui::startup_paths_tests::` **alone** → `17 passed` + summary; the **same command started
  twice concurrently** → **both** processes truncated at the same test with `rc=0` and no
  summary (2/2).

Why this is worse than a red run and belongs in the campaign's rules: a truncated run
**reports success**, so a coverage/lcov run that collides with any other app-building
process silently measures less than it claims, and the `-j`/`--test-threads=1` flags do not
prevent it because the competing process is outside the runner. **Every suite result in this
campaign should be checked for a `test result:` line before its numbers are used**; the
same hazard applies to `cargo llvm-cov` runs.

Fix belongs to `src/lib.rs` (not this unit's write set): give the Tauri test app a
per-process identifier, or do not register the single-instance plugin for the mock app.
There is precedent in the crate itself — `the_desktop_registration_builds_for_any_runtime`
already calls `enable_macos_default_menu(false)` with the comment that "`muda` may only
build menus on the main thread — which no libtest thread is", i.e. an app *build* inside a
libtest process already switches parts of a real launch off by hand. Declared as the
successor todo.

## 7. NEW-B — the probe deadline masks a poisoned `PATH`

`probe` returns "CLI discovery timed out" when `Instant::now() >= deadline` **before** it
examines the child's status. A poisoned `PATH` therefore has two readings depending on
whether the child's failure is observed in time:

* measured, deadline not reached: `a_silent…` with `PATH=/tmp/flake27/emptydir` → **FAILED in 0.18 s**;
* measured, by construction: after a leak, `a_flooding` and `a_silent` fail (arm A),
  and the same two tests pass under the guard (arm B, and `final-clean`).

So in the load-34 natural run, where **all five** `cli_tool` failures carry
`CLI discovery timed out`, `a_silent` reading `ok` is consistent with a probe that expired
before the child's failure could be seen — i.e. with a poisoned `PATH` whose symptom the
deadline hid. Practical consequence for triage (stated as the inference it is): *which*
tests failed is **not** a reliable fingerprint of a poisoned `PATH`; the reliable fingerprint
is the `NotFound`/`failed to spawn` family in modules that spawn processes
(`git_review`, `commands::review`), which is exactly how item 27 was detected.

### 7.1 The fix (NEW-B): a thread-local deadline seam, and why it must not be a static

`desktop/src-tauri/src/cli_tool.rs` now separates the budget from the rule:

```rust
const PROBE_DEADLINE: Duration = Duration::from_secs(3);   // production, unchanged

fn probe_deadline() -> Option<Duration> {
    #[cfg(test)]
    if let Some(overridden) = PROBE_DEADLINE_OVERRIDE.with(|cell| *cell.borrow()) {
        return overridden;                                  // test seam only
    }
    Some(PROBE_DEADLINE)
}
```

and the poll loop asks for it once: `let deadline = probe_deadline().map(|budget| Instant::now() + budget);`
`None` means *no deadline*, so the loop can then only end on the child's own exit or on the
output limit — which is exactly what the capture assertions need.

Two design choices worth stating, because both were forced by something this file already
learned the hard way:

* **The override is a `thread_local`, not a process-wide static.** `doctor.rs` uses a
  `Mutex<Option<…>>` static, which is correct there because every test that arms it holds
  `test_env::lock_env()`. Here the four tests that swap `PATH` hold `TEST_HOME_LOCK` but the
  three that call `probe` directly do not, and §10 shows that a process-wide global plus
  concurrency is precisely this file's destructive failure mode. A thread-local cannot leak
  between tests even if the guard is skipped, because libtest gives each test its own thread
  (verified: §9's trace shows two tests on `ThreadId(2)` and `ThreadId(3)`).
* **`ProbeDeadlineGuard` restores on `Drop` as well**, so the single-threaded harness mode
  (`--test-threads=1` really serial does run every test on one thread) is covered too, and a
  panicking test cannot hand its budget to the next one.

Which tests take the seam, and which deliberately do not:

| test | seam | why |
| --- | --- | --- |
| `a_runnable_cli_on_path_is_discovered_and_probed`, `a_missing_capability…`, `a_cli_that_changes…`, `prepare_path_prepends_the_discovered_cli` (through `discover`) | `None` | all four assert what was **captured** from a *working* fixture; the deadline can only steal the reply |
| `a_flooding_cli_is_killed_at_the_output_limit` | `None` | it asserts the **output limit** kills the child; with no deadline, `dd`'s 200 KiB crosses the limit on its own (and a fixture that fails to run now reports `exited with status: 127` instead of a misleading timeout) |
| `a_failing_cli_is_reported_with_its_exit_status` | `None` | asserts the exit **status** is reported |
| `a_silent_cli_is_killed_and_reported_as_a_timeout` | **none — production budget** | this is the kill arm: it asserts both `timed out` and `elapsed >= 3 s`. Leaving it unarmed is what keeps the seam from changing production behaviour, and it *is* the negative control for `PROBE_DEADLINE` |

Two new tests, both in `cli_tool.rs`:

1. `the_production_probe_deadline_is_unchanged_without_an_override` — with nothing armed,
   `probe_deadline() == Some(PROBE_DEADLINE)` and `PROBE_DEADLINE == 3 s`; arming `None` and
   dropping the guard puts it back. This is the "the seam did not change what ships" pin.
2. `a_reply_that_arrives_after_the_deadline_is_captured_only_without_one` — the defect and the
   fix in one deterministic test, no load and no timing luck: a fixture that answers
   **correctly** after 4 s (`/bin/sleep 4`, deliberately not `sleep`, so a poisoned `PATH`
   cannot make this test answer a different question) is (a) killed and reported as `timed out`
   under the production budget, and (b) captured with the seam armed.

**The control is sensitive — proved, not assumed.** With `probe_deadline()` patched to ignore
the override (a one-line neutering in a scratch copy), both new tests fail:

```
a_reply_that_arrives_after_the_deadline_is_captured_only_without_one ... FAILED  (panicked at src/cli_tool.rs:742)
the_production_probe_deadline_is_unchanged_without_an_override .......... FAILED  (assertion `left == right` failed)
test result: FAILED. 0 passed; 2 failed
```

so they cannot pass unless the seam really moves the deadline. Reverted immediately; the
committed content has `probe_deadline()` returning the override.

### 7.2 The rate — and the honest finding that the flake does **not** reproduce once NEW-C is fixed

Driver `/tmp/tau-rate.sh` (4 concurrent `cargo test -j 8` processes per round, per the
campaign convention; load sampled at every round; burners are 32 `/bin/sh` busy loops with a
bounded lifetime, identified by pidfile and stopped with the batch). Every arm below is the
**same** `cli_tool::tests` filter (15 tests after the seam; 13 before) unless the slice is
named, and the two "form" columns are §9's issue.

| arm | form | load (1-min) | process runs | runs failed | failures |
| --- | --- | --- | --- | --- | --- |
| `before-serial` | serial | 3.4–3.8 | 12 | **0** | — |
| `before-load` | serial + 32 burners | 86 | 8 | **0** | — |
| `before-heavy` (190-test slice) | serial | 8–10 | 8 | **0** | — |
| `before-heavy-load` (190-test slice) | serial + 32 burners | 52 | 4 | **0** | — |
| `before-parallel` | **the campaign's inner-loop form** | 6–10 | 24 | **24** | `a_silent` 24/24, `a_flooding` 24/24, `a_runnable` 2, `a_missing…` 1 |
| `after-serial` | serial | 9–13 | 12 | **0** | — |
| `after-heavy` (192-test slice) | serial | 5.9 | 4 | **0** | — |
| `after-parallel` | the same form again | ~10 | 8 | **8** | `a_silent` 8/8, `a_flooding` 7/8 |

Read the `before-serial` / `before-load` / `before-heavy*` rows against the todo's premise:
with the tests actually serialized, **no arm failed**, not at load 52 with the 190-test slice
and not at load 86 with the light module. The failures only appear in the `*-parallel` rows,
where **48 of 49 failure messages are `exit status: 127`** (the child shell could not find
`sleep`/`dd`) and 1 is a timeout — so the class this todo names is, in the reproducible
channel, the `PATH` race of §10, not the deadline. The one earlier datum that showed the
timeout symptom in a run I believed serial (8 failures at load 40, `module-tauri-headless.md`
§7) cannot be re-checked for its form retroactively: that log has no argv echo, and §9 shows
how little separates the two forms. So the seam is landed as the hardening F-CLI-2's precedent
and the deterministic control justify — *not* as a measured rate improvement, which I could
not produce and will not claim.

Commands, all run from `desktop/src-tauri` with `HOME=/tmp/tau-probe-rate-home` and
`CARGO_HOME=/Users/geilige/.cargo`:

```bash
/tmp/tau-rate.sh --label before-serial    --rounds 3 --form serial
/tmp/tau-rate.sh --label before-load      --rounds 2 --form serial --burners 32
/tmp/tau-rate.sh --label before-heavy     --rounds 2 --form serial --filter "terminal:: cli_tool:: agent_supervisor:: skills_bootstrap:: agent_host:: headless::"
/tmp/tau-rate.sh --label before-heavy-load --rounds 1 --form serial --burners 32 --filter "… same six prefixes …"
/tmp/tau-rate.sh --label before-parallel  --rounds 6 --form parallel
```

## 9. NEW-C — the campaign's inner-loop command silently runs the tests in parallel

This is the finding that reframes the todo, and it is a defect in a *measurement
instruction*, not in any test.

`cargo test` splits its own arguments from the test binary's at the **first** `--`. The
campaign's inner-loop form puts a filter *before* a second `--`:

```bash
cargo test --lib -j 8 -- cli_tool::tests -- --test-threads=1      # ← the form used everywhere
```

so the second `--` and the flag after it are handed to the binary as ordinary arguments. Two
independent proofs, both taken on a scratch pair of 1.5 s tests that record their own start/end
to a file (libtest's output capture cannot hide the interleaving, and the argv is printed by
the test itself):

| invocation | argv the test binary actually received | two 1.5 s tests | serial? |
| --- | --- | --- | --- |
| `cargo test --lib -j 8 -- diag_order -- --test-threads=1` | `<bin> diag_order -- --test-threads=1` | **1.51 s** | **no** — both entered in the same millisecond on `ThreadId(2)`/`ThreadId(3)` |
| `cargo test --lib -j 8 -- --test-threads=1 diag_order` | `<bin> --test-threads=1 diag_order` | **3.01 s** | **yes** — `A enter`/`A exit` strictly before `B enter` |

libtest only stops parsing options at the first `--` it sees, so in the first row
`--test-threads=1` becomes a *test-name filter* and the harness runs at
`get_concurrency()` = the core count: **32 threads on this host**.

Consequences, in the order they matter for the campaign:

1. **Every flake rate taken with that form measured a 32-way-parallel suite, not the suite
   the documentation says it measured.** "`--test-threads=1` is MANDATORY: the lib suite
   deadlocks in parallel on this host" (`module-tauri-headless.md` §1) was, whenever a filter
   sat in that position, not in force at all.
2. **It inflates the effective load far beyond the stated one.** 4 concurrent `cargo test`
   processes become ~128 tests racing inside one host, each spawning children — which is what
   makes wall-clock budgets (NEW-B) fail, and what makes `PATH` swaps (NEW-D) visible to other
   tests. Both symptoms of "load sensitivity" trace back here.
3. It is **not** why the module passes at load 86 in §7.2: those runs used the corrected form,
   which is the only reason the flake disappeared at that load.

The fix is one token: put the filter *after* `--test-threads=1`, or drop the second `--`:

```bash
cargo test --lib -j 8 -- --test-threads=1 cli_tool::tests     # serial, verified
cargo test --lib -j 8 -- --test-threads=1 terminal:: cli_tool:: agent_supervisor:: skills_bootstrap:: agent_host:: headless::
```

A campaign-wide check is worth running: any recorded rate whose command has a filter between
`--` and `--` should be re-measured (or at least re-read as a parallel-form rate).

## 10. NEW-D — a `PATH` swap is visible to every test running *concurrently* (item 27's second face)

Item 27 was *"a panic skips the restore, and the leak outlives the test"*. Under NEW-C's form
there is a second face that no guard can close: `a_runnable_cli_on_path_is_discovered_and_probed`,
`a_missing_capability…`, `a_cli_that_changes…` and `prepare_path_prepends_the_discovered_cli`
each replace the process-global `PATH` with `<fixture>/bin` **while other tests are running**,
and any test that spawns a shell script in that window gets a `PATH` that resolves nothing.

Measured, `cli_tool::tests`, 4 concurrent processes per round, `-j 8`, same binary:

| form | rounds × processes | runs failed | failure messages |
| --- | --- | --- | --- |
| NEW-C's form (parallel) | 6 × 4 | **24 / 24** | `CLI version discovery exited with exit status: 127` ×48, `CLI discovery timed out` ×1 |
| corrected (serial) | 3 × 4, then 2 × 4 with 32 burners | **0 / 12** and **0 / 8** | — |

`127` is the shell's "command not found": the fixture in `a_silent…`/`a_flooding…` is
`#!/bin/sh` + `sleep 30`/`dd …`, so it ran and could not find its own command. That is the same
class as item 27, and it is why the *symptom set* of the parallel form differs from run to run
while the *mechanism* stays fixed.

What closes it: serialization (the corrected form). What does not: the `EnvGuard` of §4 — it
restores the value when the test ends, which is correct but cannot help a test that runs
*while* the value is swapped. This is the strongest argument for the campaign's own
`--test-threads=1` rule, and the reason NEW-C must be fixed at the invocation rather than
worked around inside tests.

## 8. What I could not do / residual uncertainty

* **I could not produce a rate that shows NEW-B's seam improving anything** — see §7.2. With
  the invocation corrected, `cli_tool::tests` is green at every load I could establish (0/12
  natural, 0/8 at load 86 with the light module, 0/8 and 0/4 at load 8–10 and 52 with the
  190-test slice), and the reproducible failure channel is NEW-D's `127`. The seam is landed
  because its mechanism is proved deterministically (§7.1) and because F-CLI-2 set the
  precedent; it is *not* claimed to fix the observed counts.
* **The one datum that still shows the deadline symptom in a possibly-serial run** is
  `module-tauri-headless.md` §7: 182 passed / 8 failed at load 40, all eight `CLI discovery
  timed out`. That log carries no argv echo, and §9 shows the two forms are indistinguishable
  from the outside, so I cannot classify it retroactively. Re-running that exact command today
  in the corrected form at load 52 gives 190/0. A check worth doing (not done here): instrument
  the run to print its own argv, then re-measure.
* **Two peer-owned reds, reported not touched** (`--all-targets` on this crate):
  `tests/gui_main_thread_drive.rs:230` fails `clippy::empty_line_after_doc_comments`
  (`-D warnings`), in the gui main-thread unit's new file. My own scope is clean:
  `cargo clippy --lib -- -D warnings` → **0**, `cargo fmt --check --manifest-path
  Cargo.toml` → **0** crate-wide, `cargo clippy --no-default-features --features headless
  --all-targets -- -D warnings` → **0**.
* **A hygiene note**: a peer's safety-snapshot commit (`8283e1be9`) swept this unit's *scratch*
  diagnostic (`probe_diag`/`PROBEDIAG`) into the tree while it was in flight. It was removed
  before this delivery, and the file's committed content is the seam only — but a snapshot that
  captures a scratch edit can land it, which is why the pre-delivery check is
  `grep -c PROBEDIAG src/cli_tool.rs` = 0.

* **No full-suite green run of the fixed tree.** NEW-A truncates any full-suite run that
  overlaps another app-building process, and this host has ~13 workers running this same
  crate. The strongest "after" datum is therefore `final-clean`: **0 failures in every test
  the truncated run did execute** (all 13 `cli_tool`, all 26 `git_review`, all
  `commands::review`/`update::recovery`) plus the filtered runs above (`cli_tool` 13/13,
  neighbourhood 45/45).
* **`armB-forced`'s four sibling failures carry no panic text**: libtest prints the failure
  detail only in its final summary, which NEW-A removed. They are attributed to the probe
  deadline by elimination (the guard restores `PATH`; the same four tests pass on the same
  tree in `final-clean` and when filtered). Named residual uncertainty.
* **The natural run's leaking test is not attributable.** In `base-clean` several `cli_tool`
  tests panic in the window under load; the controlled arms (identical injection, guard as
  the only variable) are exact, and the skipped-injection run shows the cascade is reachable
  from the sibling sites too.
* **No lcov before/after here**: `FLAKES.md` §2.3 scopes flake work to instrumented-free
  runs. The incident's coverage consequence (a voided lcov) is the motivation, not a number
  this record re-measures.
* Checks run on the final file: `rustfmt --edition 2021 --check src/cli_tool.rs` → rc 0;
  `cargo clippy --all-targets --manifest-path desktop/src-tauri/Cargo.toml -- -D warnings`
  → rc 0 with no `futureos` warning (a peer's in-flight `future-agent` warning is present in
  the shared target dir and is not this file's). The `headless` clippy flavour was not run
  (the change is test-only and identical under both feature sets).



---

# Part II — `todo_e9b13ef21a67` (unit F-TAURI)

**Ownership, stated so the two records are not confused:** everything above (sections 1–10)
belongs to `todo_16d1326ad286` / `worker-tauri-headless` (item 27, NEW-A…NEW-D) and is left
untouched. Sections 11+ are this unit's, for a different pair of items:
**FLAKES.md item 1** (the parallel deadlock of this lib suite) and **item 8** (the
transfer/download family's 500 ms test budget). Same host, same worktree, same harness rules.

## 11. Run identity and environment

| | |
| --- | --- |
| unit / todo | `F-TAURI`, `todo_e9b13ef21a67` (goal `goal_160bc247aa63`) |
| worktree / branch | `/Users/geilige/future-os/.worktrees/cov100-r2`, `test/cov100-r2` |
| host | macOS (darwin, arm64), 32 CPUs, shared with ~4 live campaign workers; **load average 4–15** while measuring (peers ran instrumented `cargo llvm-cov` legs the whole time) |
| suite | `desktop/src-tauri` lib target (`futureos`), default (gui) features: **2091 tests** |
| toolchain | `rustc 1.97.0-aarch64-apple-darwin` (repo `rust-toolchain.toml`) |
| measured files | `src/remote_host/business/settings.rs` (the fix), `src/remote/test_support.rs` (the guard) |

Environment for **every** run below (PLAN §2; `CARGO_TARGET_DIR` is the crate's own
`desktop/src-tauri/target`, which is private to this non-workspace crate and is *not* shared
with the workspace units):

```bash
cd /Users/geilige/future-os/.worktrees/cov100-r2/desktop/src-tauri
ulimit -n 10240
export PATH=/Users/geilige/.rustup/toolchains/1.97.0-aarch64-apple-darwin/bin:$PATH
export CARGO_HOME=/Users/geilige/.cargo RUSTUP_HOME=/Users/geilige/.rustup
export HOME=/Users/geilige/future-os/.worktrees/cov100-r2/target/test-home
mkdir -p "$HOME"
```

No instrumentation anywhere in this part (FLAKES.md §2.3 rule 3): plain `cargo test`, so no
lcov number belongs to this work.

## 12. Item 1 — the two process-global test locks, taken in the opposite order

### 12.1 Negative control, measured before touching anything

The exact invocation `GOTCHAS.md` records as "必挂" and the supervisor reproduced on
2026-10-16. Same command before and after:

```
.future/cov100/flake-rate.sh --label tauri-deadlock-before --runs 3 --timeout 330 -- \
    timeout -k 5 300 cargo test --lib -- \
    remote_host::business::settings::tests remote::commands::bridge_tests
```

| | result |
| --- | --- |
| **before** (tree = this unit's base revision, no source change) | **run 1/3 = `TIMEOUT` (`rc=124`)**; the run was stopped at the 300 s budget. The batch was then stopped after run 1 on purpose: the hang is deterministic (the supervisor reproduced the same command on 2026-10-16, `GOTCHAS.md` records it as 必挂, and §12.2's stack sample shows why), so a second and third 300 s datum would say nothing the first did not — and this host's launch faults (§14) make every extra run expensive. Log: `…/T/flake-tauri-deadlock-before/tauri-deadlock-before-1.log` — **68 tests started, 32 printed `has been running for over 60 seconds`, zero `test result:` lines**. |
| **after** (§12.3 fix only, same command, same 3 runs) | **3/3 pass**, each run **7.0 s** wall (3 runs in 21.8 s total): `test result: ok. 71 passed; 0 failed; 0 ignored` — log `…/T/flake-tauri-deadlock-after/tauri-deadlock-after-1.log`. |

The "before" run was *not* a slow build: the binary page (`Running unittests src/lib.rs
(target/debug/deps/futureos_lib-a38f137727fdbb37)`) appears at line 2 of the log and the sample
below shows every thread parked. `timeout -k 5` was used because `timeout` alone cannot end a
thread that is asleep on a mutex if the process ignores `SIGTERM`; the harness's own
`--timeout 330` is the outer backstop.

### 12.2 The mechanism, from a stack sample (not from reading the code)

While the "before" run was hung, `sample <pid> 2` (the mechanism the campaign notes endorse;
`lldb` attach hangs) gave the two sides of the cycle **by name and line** — this is stronger
evidence than the same conclusion read out of the source, because it shows which pair of tests
was actually holding the locks:

* **Holding `MOCK_SER` (via `mock_agent_lock`), waiting for `TEST_HOME_LOCK`:**
  `remote::commands::bridge_tests::an_untrimmed_page_is_marked_as_such` →
  `bridge_tests::active_bridge` (`src/remote/commands.rs:2202`) →
  `HomeGuard::new` (`src/auth_store.rs:169`) → `__psynch_mutexwait`.
  (Its `mock_agent_lock` frame is not on the stack because that call *returned*: the guard is a
  live local, which is exactly why a per-thread lock-order assertion would not see this.)
* **Holding the HOME guard, waiting for `MOCK_SER`:**
  `remote_host::business::settings::tests::a_catalogue_without_a_models_array_is_not_reported_as_all_hidden`
  (`src/remote_host/business/settings.rs:418`) → `mock_agent_lock`
  (`src/remote/test_support.rs:321`) → `__psynch_mutexwait`, with the same for
  `settings::tests::an_unreadable_store_is_reported_not_defaulted` (`settings.rs:382`).

70 threads in the sample; **every** non-worker thread was in one of those two waits and the
process sat at **0.0 % CPU** (the flake is a total deadlock, not a slow run). Two process-global
locks were involved:

| lock | owner | taken by |
| --- | --- | --- |
| `MOCK_SER` (`src/remote/test_support.rs:319-321`) | the mock gRPC `FutureAgent`'s script queue + facade maps | `mock_agent_lock()` (also re-exported as `commands::agent_mock::mock_agent_lock`) |
| `TEST_HOME_LOCK` (`src/auth_store.rs`, `HomeGuard::new`) | process-global `HOME` + `FUTURE_HOME` + the SQLite pool | `HomeGuard::new(label)` |

### 12.3 The fix

**One order, everywhere: `mock_agent_lock()` first, then `HomeGuard`.** Two sites in
`src/remote_host/business/settings.rs` had the reverse order; both are now MOCK-then-HOME:

| site | before | after |
| --- | --- | --- |
| `settings.rs::tests::an_unreadable_store_is_reported_not_defaulted` | `HomeGuard::new(…)` at :381, then `mock_agent_lock()` at :382 | `mock_agent_lock()` then `HomeGuard::new(…)` (+ a comment naming the discipline) |
| `settings.rs::tests::a_catalogue_without_a_models_array_is_not_reported_as_all_hidden` | `HomeGuard::new(…)` at :416, `init_store()`, then `mock_agent_lock()` at :418 | `mock_agent_lock()` first, then `HomeGuard::new(…)`, `init_store()` |

Neither body had a dependency on the old order (`init_store()`/`ensure_mock_agent()` take
neither lock, and they still run under the new `HOME`), so the assertions are unchanged — this
is a reordering, not a weakening.

### 12.4 How the sites were enumerated (the census, so it can be re-run)

A source-level call-graph census over the crate, because the two locks are also taken
*transitively* (the majority of both-lock bodies get one of them through a helper such as
`active_bridge`) and a grep for two tokens in one body would have missed the reverse case.

Algorithm (script kept at `/tmp/lockorder.py`; ~150 lines of Python, no dependencies):

1. **Per-function bodies** by brace matching over a copy of each `.rs` file with comments,
   strings, raw strings and char literals blanked to spaces (offsets preserved, so line numbers
   survive).
2. **Direct acquisition sites**: `HomeGuard::new` / `TEST_HOME_LOCK` for HOME,
   `mock_agent_lock(` for MOCK. (The token for MOCK keeps the `(`, so the `use …::{…,
   mock_agent_lock};` import inside a test body is not a call site.)
3. **Call-graph edges are restricted to names defined exactly once in the tree, and no method
   calls** (`(?<![.\w])` before the callee). Without that restriction the census is garbage:
   a bare `ok(`/`count(`/`map(` resolves to an unrelated helper and invents both `INVERT` rows
   (measured: the unrestricted form reported 3 inversions, one of them a phantom in
   `remote/verify_e2e.rs` produced by a helper named `ok`).
4. **Fixpoint** over "acquires HOME / MOCK directly or transitively", then per body the first
   acquisition event of each kind (direct token, else the line of a call to an acquiring helper)
   decides the order.

Result, on the fixed tree:

```
files scanned: 191
functions that acquire BOTH locks (transitively): 89
HOME-before-MOCK (inversion) functions: 0 in 0 files
```

Before the fix the same script reported exactly the two `settings.rs` rows of §12.3 — and
nothing else, which is what made this a two-line fix rather than a sweep of every lock mention in
the crate. For scale, `grep -rn` over `src/**/*.rs` counts **224 lines mentioning the mock lock /
`MockAgentGuard`** and **574 mentioning `HomeGuard`**, while the campaign's task text counts **579
acquisition sites across ~76 files**; those three numbers are all *mentions or sites of one lock*,
and none of them is the number that decides the fix. What decides it is that only **89 bodies hold
both locks** and only **2** of those took them in the wrong order. To re-run:

```bash
cd desktop/src-tauri && python3 /tmp/lockorder.py src      # prints INVERT rows, then the census with -v
```

Residual uncertainty of the census, stated rather than hidden: step 3 drops *ambiguous* names
(a helper defined twice), so a body that reaches a lock only through a duplicated name would be
missed. The upper bound with every name as a possible edge is 2815 "HOME-acquiring" names, which
is all false-positive noise (`open`, `write`, `insert`, …), so the bound is not usable as a
check; the empirical check below is what covers the residue.

### 12.5 The guard: the order is enforced by the suite, not by a comment

A comment cannot fail. `src/remote/test_support.rs` now carries two tests that read the crate's
own sources (`env!("CARGO_MANIFEST_DIR")/src`, 191 files) with the same brace/blanking logic as
the census:

* `no_body_takes_the_home_guard_before_the_mock_lock` — fails, listing `path: fn name`, if any
  body mentions `HomeGuard::new`/`TEST_HOME_LOCK` before its `mock_agent_lock(` call. The blanking
  is what keeps this honest: this module's own doc comments *name* both locks in prose.
* `the_lock_order_scan_flags_an_inverted_body` — the scan's own control: it must flag a body that
  really is inverted, and must **not** flag the correct order or the wrong order appearing inside
  a string literal. (A scan with no control is a scan that can silently never match.)

Reading the source rather than the runtime state is deliberate: the failure this prevents
*cannot* be reported from inside the test that is stuck (the stack sample of §12.1 is what it
costs to diagnose one), and a runtime `try_lock`-deadline assertion would risk turning a slow
host into a red suite — the opposite of what this unit is for.

**Scope of the guard, stated so it is not mistaken for more:** it sees the shape inside ONE body,
which is what both known inversions had. An inversion routed through a *new helper* (a body that
creates a `HomeGuard`, called before a `mock_agent_lock` in its caller) is invisible to it; that
shape is what §12.4's census enumerates, and it is currently 0/89. The test's doc comment says the
same thing at the site.

### 12.6 Suite-level verification

The acceptance for item 1 is not only the two-module negative control: the whole lib suite has to
finish under **default parallelism** (CI runs it that way; `--test-threads=1` is not available as
an escape). Same harness, 5 runs, whole target:


## 13. Item 8 — the chunk pull's test-only 500 ms budget

### 13.1 The mechanism, from the code path (before any run)

`desktop/src-tauri/src/remote_peer/runtime.rs` joins the two halves of a chunk
download: `pull_chunk` registers a `oneshot` waiter, sends the pull request, and then
waits for the host's *separate* byte publish to be handed to that waiter:

```rust
match tokio::time::timeout(chunk_pull_timeout(), receiver).await {   // :1831
    Ok(Ok(bytes)) => Ok(bytes),
    _ => { forget_chunk(...); Err(RemoteTransport("remote_download_chunk_missing")) }
}
```

and, before this change, that budget was **different under `cfg(test)`**:

```rust
fn chunk_pull_timeout() -> Duration {
    #[cfg(test)]      const TIMEOUT: Duration = Duration::from_millis(500);
    #[cfg(not(test))] const TIMEOUT: Duration = Duration::from_secs(15);
    TIMEOUT
}
```

The arm cannot distinguish "the host never published" from "the publish has not been
scheduled yet" — the code says so in its own comment — so under `cfg(test)` **the
suite's verdict on a correct download depended on host scheduling**: if the delivery
path (host reads the chunk, publishes to the broker, the client's stream task
receives, the waiter is resolved) took longer than 500 ms, a *served* chunk was
reported as a lost publish, the step was retried (`transfer.rs::retrying`, 3 attempts,
first error reported), and the download failed. That is the documented item 8: CI saw
`remote: network failure recovered [NW001] after 1 attempts and 2410ms` immediately
before `download: Message("peer_not_connected")` — 2.41 s for one pull request on the
CI runner, i.e. ~5× the test budget for the *other* half of the same pull.

The measured cost of a wall clock on this host is in §14: a single lib test has been
observed at 17–22 s here, and freshly built binaries have stalled minutes at
`_dyld_start` when `syspolicyd` saturated. A 500 ms budget is not a budget on this
machine.

### 13.2 The fix — the budget becomes an argument, not a build mode

`runtime.rs`:

* `pull_chunk(desktop_id, transfer_id, index)` → waits with `chunk_pull_timeout()`,
* **new** `pull_chunk_within(desktop_id, transfer_id, index, budget)` → the same body with the
  wait as a parameter (the request's own budget, `session::COMMAND_TIMEOUT` = 10 s, is
  unchanged and still pays for the host's disk read),
* `chunk_pull_timeout()` is now **one value, 15 s, in tests and in the app** — the
  production budget, so no test-only timing assumption reaches a passing assertion.

`transfer_tests.rs::a_chunk_that_never_arrives_gives_up` (the one test that must
*observe* the give-up arm) passes its own 200 ms to `pull_chunk_within` instead of the
whole suite running on a shortened budget. That call cannot be affected by the mistake
above: the stream task is stopped before the pull starts, so no chunk can arrive at any
load — the deadline is *always* reached rather than raced for. Its assertion is
unchanged (`remote_download_chunk_missing`), and it now also asserts the fixture really
had a chunk to serve (`info.size > 0 && info.chunk_bytes > 0`), so "the wait expired" is
not confused with "there was nothing to deliver".

Not taken, with the reason: a `#[cfg(test)]` *override* of the budget (F-CLI-2's
shape, used for `doctor`'s probe). The wait happens after an `await`, so on a
multi-thread test runtime the override would be read on whichever worker thread resumes
the task — a thread-local it is not; a process-global one would be visible to every
concurrently running test, i.e. it would re-create exactly the flake it is meant to
remove. An argument has no such window.

### 13.3 Rate: the two arms, same command, same `-j`, one load window

Both arms ran 2026-10-17 06:17–06:52 local (the `after` arm 06:17–06:35, the `before` arm
06:35–06:52), back to back, with the same 16 CPU burners alive throughout
(`timeout 2700 sh -c 'while :; do :; done'`), the same `HOME`, the same filter and the same
`--test-threads=32`. The binary is handed to the rig directly, so no `cargo` and no build lock is
involved and the only difference between the arms is which binary runs:

```
/tmp/xfer-ab/run-arm.sh <binary> <label> 10 32
  = timeout -k 5 600 <binary> --test-threads=32 -- remote_peer::transfer_tests commands::remote_peer::tests
  before: /tmp/xfer-ab/before/futureos_lib  sha256 2291d0b7…  (the 500 ms test budget)
  after : /tmp/xfer-ab/after/futureos_lib   sha256 0de1dc65…  (15 s, this unit's fix)
```

| arm | runs | red runs | red tests | **budget-class** signatures | 8022-fixture signatures | load |
| --- | --- | --- | --- | --- | --- | --- |
| **before** (500 ms) | 10 | 5 | 13 | **0** | 13 | 18–30 |
| **after** (15 s) | 10 | 2 | 5 | **0** | 5 | 18–30 |

The classification, run over each arm's logs — this grep is the decisive step, not the red count
(quoting is shell-safe: single quotes, so the inner double quote is literal):

```bash
grep -hoE 'remote_download_chunk_missing|peer_not_connected|remote_download_size_mismatch' \
    /tmp/xfer-ab/logs-<label>/<label>-*.log | sort | uniq -c      # budget-class: 0 in both arms
grep -hE 'the test web port never freed' \
    /tmp/xfer-ab/logs-<label>/<label>-*.log | wc -l               # 8022 fixture: 13 / 5
```

**Every failure in both arms is `remote_peer/testing.rs:30`** — `wait_for_web_port_free`'s
"the test web port never freed", i.e. the shared port 8022 (`testing.rs:19`) that two tests starting
a host bridge at once collide on. **Not one failure in either arm carries a budget-class message.**
The victims differ between runs (`two_downloads_in_flight_pull_independently` twice,
`a_multi_chunk_download_assembles_in_order`, `a_multi_chunk_upload_keeps_its_order`,
`a_download_round_trips_a_file_through_the_host`, `a_download_whose_source_shrank_is_refused`,
`a_declaration_that_disagrees_with_the_bytes_is_refused`, `a_chunk_write_without_a_connection_is_refused`,
`a_failed_download_writes_nothing`, `a_preview_lands_in_this_machines_cache_under_the_hosts_name`,
`list_reports_every_paired_host`, `pairing_through_the_command_connects_and_reports_failures` in the
`before` arm; five different names in the `after` arm), and that variance — not the budget — is what
makes the red counts 13 and 5. The family belongs to the shared-fixture workstream
(`todo_0aedf87f73f8`), so this record reports it rather than fixing it.

**The honest natural rate is therefore: 0 budget-class failures in 20 runs (10 per arm) at load
18–30 with `-j 32`.** Item 8 does not reproduce on this host under ambient load, which matches the
campaign's history for it (a CI-only occurrence: `#1094`, where the `NW001` line shows one pull
taking 2.41 s). Two consequences, stated rather than glossed:

* **A green `before` arm is not evidence that the fix was unnecessary.** It says the natural rate
  here is below 1 in 10 runs of this 56-test filter. The mechanism is still what §13.1 names, and the
  fix removes it by construction (there is no 500 ms budget left to miss) rather than by making a race
  less likely.
* **The measurement was then biased towards a hit on purpose** — §13.4 — because a rate of 0 for the
  arm that is supposed to fail proves nothing about reachability.

### 13.4 The positive control: starving the delivery path on purpose

The flake's mechanism is host scheduling (the delivery misses a window), so the control starves the
test process of CPU and changes nothing else:

```
/tmp/xfer-ab/run-arm-hostile.sh <binary> <label> 3 32
  = timeout -k 5 900 /usr/sbin/taskpolicy -b <binary> --test-threads=32 \
        remote_peer::transfer_tests commands::remote_peer::tests
```

`taskpolicy -b` places the process in macOS's **background QoS band**, so it yields to the 16
burners instead of competing with them: the same binaries, the same filters, the same `-j`.

| arm | runs | red runs | **budget-class** (`remote_download_chunk_missing`) | request/connection timeout | 8022 fixture | wall time per run |
| --- | --- | --- | --- | --- | --- | --- |
| **hostile-before** (500 ms, starved) | 3 | 0 | **0** | 0 | 0 | 460 s, 94 s, 94 s |
| **hostile-after** (15 s, starved) | 3 | 2 | **0** | 3 + 2 | 0 + 4 | 460 s, 439 s, 97 s |

The starvation is real and large: the first run of each arm took **460 s** for a filter that takes
92 s unstarved under the same load — **5.0×** — and the `before` arm still passed 56/56. The `after`
arm's two red runs failed with **request/connection timeouts**, not with the budget's signature, and
run 2 mixed in 8022 fixture reds; §13.5 reads that text.

**So the `before` binary does not produce a budget-class failure even when the whole suite is slowed
fivefold, and the value of the fix is not a measured rate.** That is a negative result, and it is
worth more than the rate would have been, because it corrects the recorded mechanism rather than
confirming it:

* The 500 ms budget guards the wait **after the host has acknowledged the pull** — the join between
  the host's byte publish and the client's stream task, both inside this one process, a handover of
  microseconds (§13.1). Starving the process slows *both ends and the handover together*, so 500 ms
  keeps being enough; to miss it the *handover alone* has to stall for half a second while everything
  else proceeds, which is a different and rarer event than "the machine is busy".
* The **CI signature points elsewhere**: `#1094` shows `network failure recovered [NW001] after 1
  attempts and 2410ms` and then `download: Message("peer_not_connected")`. A missed post-ack wait
  reports `remote_download_chunk_missing` (and it is the *first* error that is preserved by
  `transfer::retrying`, so it would surface as that, not as a connection loss). `peer_not_connected`
  is the *connection* being gone — i.e. the verdict of the **request** half of the same pull
  (`request_at`, budget `session::COMMAND_TIMEOUT`) or of the socket underneath it, which is a
  production budget of 10 s and identical in tests.

What this unit therefore claims, precisely: (a) the test-only 500 ms budget is **gone** — there is no
longer any `cfg(test)`-shortened wall clock on the transfer path at all, which I checked by grep
(`session.rs:31/33` `HANDSHAKE_TIMEOUT`/`COMMAND_TIMEOUT` are 10 s in both builds; the only remaining
`cfg(test)` durations in `remote_peer/**` and `remote/**` are the *reconnect backoff* at
`runtime.rs:153/157` and unrelated publisher ticks); (b) the arm it existed for is covered by a
deterministic test with an explicit budget instead; and (c) **the recorded mechanism of item 8 is not
confirmed by this measurement — "a 500 ms budget mistaken for a transport failure" is not what the
CI log's own signature says, and the place to look next is the request/connection half of the pull**
(`remote_peer/session.rs` request path and what drops the socket), not the post-ack wait.


**Result** (`.future/cov100/flake-rate.sh --label tauri-lib-after --runs 5 --timeout 900 -- timeout -k 5 880 cargo test --lib`,
stopped after 1 run because each run costs ~10 min on this host and the whole-suite *completion* is
what is being measured):

| run | outcome | wall | suite summary | failures |
| --- | --- | --- | --- | --- |
| 1/5 | **completed** (no hang, no `TIMEOUT`) | **583 s** | `2058 passed; 21 failed; 12 ignored; 0 filtered out` | the 21 named below, **no hang and no in-scope regression** |

Before the fix this command could not produce a summary at all (§12.1: 300 s timeout, 32 tests
`running for over 60 seconds`, 0 `test result:` lines). It now runs to a complete summary at
**load 4–15** with ~4 peer workers active.

**Every one of the 21 failures is a pre-existing, documented family — none is this change, and
none is a hang.** Grouped (the full list is in the log's final `failures:` block):

| count | tests | mechanism (and where the campaign already records it) |
| --- | --- | --- |
| 5 | `commands::remote_peer::tests::{commands_reject_an_unpaired_host, connect_all_through_the_command_reaches_the_paired_host, connect_disconnect_and_unpair_round_trip, labels_and_catalogues_flow_through_the_commands}`, `remote_peer::runtime_tests::a_command_that_is_not_an_object_is_sent_as_is` | the **shared test web port 8022**: `remote_peer/testing.rs:19` pins `WEB_PORT = 8022` and `wait_for_web_port_free()` (:23) waits 5 s for it, so two tests that start a host bridge at once collide (`web server bind failed [LC002] … 8022 … Address already in use` → `the test web port never freed`). Documented in PLAN §5.3 and in `module-tauri-shell.md:1489` as the known `8022` family. Both of the `commands::remote_peer` names pass **isolated** here (2/2, 3.6 s) — victims of concurrency, not code. Fix belongs with the shared-fixture workstream (`todo_0aedf87f73f8`), not here. |
| 1 | `remote::commands::bridge_tests::replay_continuations_…` | `DatabaseBusy — "database is locked"` from `store init under test HOME` (`remote/test_support.rs:42`), the documented shared-HOME/SQLite contention — the subject of `todo_0aedf87f73f8`. |
| 1 | `remote::commands::bridge_tests::tool_call_args_is_reachable_only_for_a_client_that_declared_lean` | a leaked **`lean_events_v1` process-global declaration** by another test in the same binary: `module-tauri-remote.md` §5.1 documents this exact failure and the rule "any test in this crate that declares a capability must withdraw it"; `module-tauri-shell.md:1482` records the same test as a cross-test state leak in `src/remote/**`. It is **green 3/3** in its own group on this tree (`cargo test --lib -- remote::commands::bridge_tests` → 65 passed each time), which is the same signature. Not this change: the two locks it is about are untouched by my edits. |
| 1 | `skills_bootstrap::tests::the_bootstrap_spawns_the_cli_through_the_shell_plugin` | a **concurrent `PATH` swap** — FLAKES item 27 / NEW-D: `docs/testing/flaky-tauri.md` §10 measures 24/24 red under the parallel form and 0/20 under the serialized one. Expected in *any* parallel run of this binary until the swaps are removed. |
| 1 | `tasks::tests::the_gui_loop_runs_a_queued_request_it_is_woken_for` | the appdata lane's known in-scope red (PLAN §5.3 names it for `tauri-appdata` segments 8–10). |
| 12 | `terminal::{pty,server,session,manager}::…` | the terminal lane's PTY/e2e family failing under parallel load (documented by `TAU-D`/`TAU-C` records; the suite's instrumented runs in `module-tauri-headless.md` carry them too). |

Attribution, stated plainly: **there is no "before" suite rate to compare against, and there
cannot be** — before this fix the same command never finished (it deadlocked), so the parallel
full-suite red set could not previously be observed at all. What the record can do instead is what
is above: every failure named, its mechanism given, its documented owner cited, and (for the two
`commands::remote_peer` names) shown to pass in isolation. The two files this unit changed are two
test bodies in `remote_host/business/settings.rs` and a source-scanning test; neither binds a port,
spawns a program, or touches the `lean`/store fixtures those failures live in.

### 13.5 What the starved runs actually broke — and it is **not** the budget

(This section was added after §13.4 and lands immediately before §14.)

The starved `after` run (15 s budget, and *still* the failing one) has readable failure text, and it
names a different mechanism from the one item 8 was filed under:

```
---- remote_peer::transfer_tests::a_multi_chunk_download_assembles_in_order stdout ----
thread '…a_multi_chunk_download_assembles_in_order' panicked at src/remote_peer/transfer_tests.rs:76:59:
fetch: RemoteTransport("Remote request failed: request timed out: deadline has elapsed")

---- remote_peer::transfer_tests::a_multi_chunk_upload_keeps_its_order stdout ----
thread '…a_multi_chunk_upload_keeps_its_order' panicked at src/remote_peer/transfer_tests.rs:895:10:
upload: RemoteTransport("Remote request failed: request timed out: deadline has elapsed")

---- commands::remote_peer::tests::the_visible_session_command_moves_the_hosts_live_lane stdout ----
thread '…the_visible_session_command_moves_the_hosts_live_lane' panicked at src/commands/remote_peer.rs:593:14:
connect: RemoteTransport("Failed to connect to NATS: timed out")
```

`Remote request failed: request timed out: deadline has elapsed` comes from the **request** half of
the pull (`remote_peer::session`'s request path, budget `session::COMMAND_TIMEOUT` = **10 s in both
builds**), not from the delivery wait — whose own failure is `remote_download_chunk_missing`. Greps
over that log find **no** budget-class text and **no** 8022 text (`request timed out|Failed to
connect to NATS` = 3, `the test web port never freed` = 0, `remote_download_chunk_missing` = 0). So
this is a third family in the transfer lane, and the one that is genuinely load-sensitive here: *the
fixture's in-process host cannot serve a request inside 10 s while the machine is saturated.*

Why that matters more than the rate: it is the family the CI log belongs to. `#1094`'s
`remote: network failure recovered [NW001] after 1 attempts and 2410ms` is a request that needed
2.41 s and a retry, and the `download: Message("peer_not_connected")` that follows is the connection
gone — the same half of the pull, one step further along. The post-ack wait this item's todo blamed
can produce neither string.

**Reading of the starved sets, with their size stated:** the two arms' starved runs disagree in the
direction *opposite* to the hypothesis — the *before* arm passed 3/3 (`0` red tests), the *after* arm
went red 2 of 3 with 9 red tests — and that difference is **not** attributable to the code: the
request budget and the NATS connect logic are byte-identical in the two binaries, and the greps in
§13.5/§14.2 confirm no `cfg(test)` shortening touches them. It is load variance inside a fixed 10 s
race — which is the point: the race is a *production* budget, so both arms sit under it equally and
neither the presence nor the absence of the test-only 500 ms budget moves it. The `after` arm's red
set is itself mixed (run 1: 3 request/connection timeouts and no 8022; run 2: 2 request/connection
timeouts and 4 × 8022), i.e. two different families, neither of them the budget's.

**Conclusions this record stands behind:**

1. The 500 ms test-only delivery budget **never fired**: `remote_download_chunk_missing` appears
   **0 times in 26 runs** (20 natural + 6 starved at 5.0× starvation). It is not the mechanism behind
   this lane's load sensitivity.
2. The load-sensitive family that *does* reproduce here is request/connection timeout under
   saturation (`Remote request failed: request timed out`, `Failed to connect to NATS: timed out`),
   on **both** binaries. Removing the 500 ms budget neither causes nor cures it.
3. Item 8 should therefore be closed as "test-only hazard removed, its arm made deterministic" —
   **not** as "the CI flake fixed". The next hunt is the 10 s request budget and the NATS connect
   path under load (e.g. whether that budget can be derived from progress instead of a wall clock),
   which is the same question the todo asked, one layer up. Recorded in §16.1 for the successor.

## 13.6 The request-class timeout, characterized — and the tolerance that failed its own measurement

*(Unit F-TAURI-3, `todo_a0ee2bdabdbf`. This section continues §13.5: that section found the
request/connection family; this one names *which step* is slow, decides the fix, and records the
fix that was implemented and **withdrawn** because it did not move the rate.)*

### 13.6.1 Method: a three-point diagnostic (temporary, reverted)

A request against a real peer has four legs that can be slow: the socket connect, the Noise
handshake, the command publish, and the reply. Each was instrumented at its own boundary in an
env-gated diagnostic (`FLAKE_DIAG=1`), and the diagnostic was **removed before shipping**
(`grep -rn 'FLAKE_DIAG' src/` → 0 matches; the four frozen binaries below are what carry it):

| where | what it printed |
| --- | --- |
| `remote/test_support.rs::deliver` (the fake broker) | `DIAG nats deliver subject=… targets=<n> conns=<n>` — how many subscribers a publish reached. `targets=0` is a **lost** request |
| `remote/commands.rs` command loop (the host) | `DIAG host recv subject=…` — that the in-process host received it |
| `remote_peer/session.rs::exchange` (the client) | `DIAG client request subject=… elapsed_ms=<n> outcome=<ok\|nats_err\|outer_timeout>` |

### 13.6.2 The measurement: nothing is lost; the in-process peer is simply starved

Four runs of the filter under the reproducing rig (`taskpolicy -b`, 40 burners, **two** concurrent
rigs, `-j 32`, load 60–75, `--nocapture`), 13 failing tests, 1093 broker deliveries:

| run | wall | failing tests | deliveries | `targets=0` | host received | requests >1 s |
| --- | --- | --- | --- | --- | --- | --- |
| 1-a | 415 s | 5 | 268 | **0** | 124 | 5 |
| 1-b | 370 s | 6 | 252 | **0** | 115 | 2 |
| 2-a | 115 s | 1 | 287 | **0** | 131 | 1 |
| 2-b | 116 s | 1 | 286 | **0** | 131 | 2 |

The two slow requests that mattered, correlated by subject (script `/tmp/xfer-ab/correlate.py`):

```
p.pair_1792193984.cmd.sess_unused   11275 ms  nats_err: request timed out: deadline has elapsed
     broker delivered it to 1 subscriber; the host received it
p.pair_1792194064.cmd.handshake     12331 ms  nats_err: request timed out: deadline has elapsed
     broker delivered it to 1 subscriber; the host received it; a SECOND delivery of the same
     subject arrives after the client's deadline → the reply came back, late
```

And in the same window one *successful* handshake took **8052 ms** — 80 % of the production budget,
for a peer in the same process. The failure taxonomy of those 13 tests:

| class | count | signature |
| --- | --- | --- |
| client **connect** leg | **6** | `connect: RemoteTransport("Failed to connect to NATS: timed out")` |
| client **request** leg | **2** | `Remote request failed: request timed out: deadline has elapsed` (one on the download command, one on the Noise handshake leg) |
| **8022** shared-port fixture | **5** | `remote_peer/testing.rs:30: the test web port never freed` (owner: `todo_0aedf87f73f8`) |

**So the mechanism is: nothing is lost, and the client's own wall-clock budgets fire because the
whole fixture — client, fake broker and host bridge on ONE current-thread runtime per test — is
deprived of CPU.** Two of the budgets that fire are not even the ones this module documents: the
socket-connect leg is bounded by **`async-nats`'s inherited 5 s default** (`ConnectOptions::
connection_timeout`), and the per-request budget by its inherited 10 s default
(`request_timeout`) — neither was ever set by this crate.

### 13.6.3 Kept: the production budgets are named, set, and pinned

| | before | after |
| --- | --- | --- |
| socket connect | inherited `async-nats` default (5 s), unnamed | `NATS_CONNECT_TIMEOUT` = 5 s, **set explicitly** on `ConnectOptions` |
| one request | inherited `async-nats` default (10 s) | `request_budget()` = `PHONE_REQUEST_TIMEOUT` = 10 s, **set explicitly** |
| handshake leg | `HANDSHAKE_TIMEOUT` = 10 s | unchanged (`= PHONE_REQUEST_TIMEOUT`) |

**Production behaviour is unchanged by construction** — the values are the ones already in force — so
the change is a contract fix: a dependency bump can no longer silently change how long the app waits
for a socket, and `production_budgets_are_the_phone_and_the_library` pins all three numbers plus the
wiring (`src/remote_peer/session.rs::tests`) so a seam that drifted could not hide.

### 13.6.4 Kept: the give-up arms own their budget

`runtime::put_chunk_within(…, budget)` joins `pull_chunk_within` (§13.2), and the two tests that
*observe* a dead socket pass 200 ms instead of inheriting the production 10 s. Measured, same
command, `--test-threads=1`, same host:

```
remote_peer::transfer_tests::{a_chunk_write_on_a_dead_socket_is_recorded,
                              a_chunk_write_on_a_dropped_broker_is_recorded}
  before (binary 0de1dc65…, production budget): 14.28 s
  after  (binary 2ef1108c…, its own 200 ms):     4.14 s      (3.4× faster, same assertions)
```

### 13.6.5 Withdrawn: a fixture tolerance (implemented, measured, reverted)

The FLAKES-shape fix for "a fixture is starved" is to give the *fixture* its own window (item 28's
sidebar-launch precedent). It was implemented — `cfg(test)` 60 s for the connect, request and
handshake legs, production untouched, with the pinning test above — and then measured against the
same rig and load:

| arm (same rig, `-j 32`, load 60–75) | runs | outcome |
| --- | --- | --- |
| pre-fix binary (0de1dc65…), measured this turn | 4 | **4 red**, filter finished in 115–415 s, 13 failing tests (8 temporal, 5 fixture) |
| tolerance binary (0708fd0e…, the withdrawn candidate) | 2 | **both hit the harness's 900 s cap** (`rc=124`) with tests still failing inside; the other two were stopped with the batch |

**The tolerance made the datum worse, not better**: starvation scales every wait, including the
tolerance, so at the only load where this failure is reachable the filter cannot finish inside the
cap — while at every load this host can otherwise produce, the family does not fire at all (§13.3:
0 budget-class failures in 26 natural runs). A benefit that cannot be measured here against a real
cost (12× longer worst case for every future run, and reds turning into timeouts) does not ship, so
the third and fourth call sites of the seam were reverted: `request_budget()`/`connect_budget()`/
`handshake_budget()` now return the production numbers, and the reasoning is in their doc comments so
the next reader does not re-invent the experiment.

### 13.6.6 The next step this measurement points at (successor's first check)

**Commit-state note — RESOLVED (checked 2026-10-17, after F-TAURI-4).** The withdrawal of §13.6.5
spanned three source files and was uncommitted when this section was written, while the supervisor's
07:56 commit had already captured the tolerance version. That is now settled: `git show
HEAD:desktop/src-tauri/src/remote_peer/session.rs | grep -c 'from_secs(60)'` → **0**, i.e. the
committed tree carries the withdrawal. The note is kept because the trap is general (a supervisor
commit can capture any in-flight state) and because the check is one command.

The CI signature is `NW001 … 2410ms` then `peer_not_connected`. In this crate a timed-out request is
treated as a transport failure, and `on_transport_failure` **drops the connection** — which is
exactly how a slow host becomes "not connected". So the lever is not the budget but the client's
*conclusion*: on a request timeout, probe whether the connection is actually dead (the socket, or a
ping round trip) and drop it only on evidence, instead of inferring death from one slow round trip.
That is the "derive the wait from a condition/progress" direction FLAKES.md asks for, it changes no
budget, and it is measurable against this rig (the `DIAG client request` lines already distinguish
"reply lost" from "reply late"). **It is implemented in §13.7** (F-TAURI-4): a request timeout over a
connection the NATS client still reports as `Connected` no longer drops the socket, with both arms
pinned by tests and the alive arm controlled against a pre-fix binary.

## 13.7 The fix: a timed-out request no longer condemns a live connection

*(Unit F-TAURI-4, `todo_75ff0be76e75`, continuing §13.6.6. The lever it names — a timeout dropping
 the connection — is now implemented, controlled, and measured.)*

### 13.7.1 The change (three pieces, all in `remote_peer`)

| piece | what it does |
| --- | --- |
| `session::PeerSession::is_connected()` | asks the NATS client (`connection_state() == Connected`): the only component that knows whether the socket under a request is still there |
| `runtime::is_request_timeout()` | matches the phrase **both request paths** produce (`"Remote request timed out"`, `"Remote request failed: request timed out: deadline has elapsed"`). Deliberately not `"timed out"` alone: `"Failed to connect to NATS: timed out"` is the *socket-connect* leg, where there is no session to probe |
| `runtime::on_transport_failure()` | a request timeout over a session the client still reports as `Connected` **keeps** the connection, its subscriptions and the transfer lane; the failure stays the command's own. Every other transport failure drops and retries exactly as before |

The connection is the thing with state: dropping it also ends the streams the transfer lane depends
on, which is why one slow round trip used to be able to take a peer out of service.

### 13.7.2 Both arms pinned, and the new arm is controlled

| arm | test | pre-fix | post-fix |
| --- | --- | --- | --- |
| transport **dead** → drop + retry | `a_chunk_write_on_a_dropped_broker_is_recorded` (broker killed — the only path that reaches `on_transport_failure` with a session still in the map) | passed | passes, and now **also** asserts the retry was started (`supervisor_count() > 0`) |
| transport **alive but slow** → keep | **new** `a_request_that_times_out_keeps_a_connection_that_is_still_up` | **FAILS** — `one slow command must not drop a live connection` (control binary `d9bf393e…`, probe disabled) | passes: still connected, **no** supervisor started, and a real download completes afterwards |

The dead-socket sibling (`close_socket_for_test`) *removes the session itself*, so it never reaches
the drop logic at all; the annotation at that test says so, and the retry assertion therefore lives
in the broker test. The new arm's timeout is induced **deterministically, without load**: a request to
a subject nobody subscribes to, which the in-process broker accepts and cannot answer, over a socket
that is fine.

Measured discrimination, both directions: a socket closed under the connection produces a timeout
**and** `is_connected() == false` (dropped, retried); a starved-but-live peer produces a timeout over
a `Connected` client (kept).

### 13.7.3 Rate: the rig says *no measurable change*, and that is the finding

Same rig and window as §13.6.2 — `taskpolicy -b`, 40 burners, **two concurrent rigs**, `-j 32`, load
53–62 — with the two binaries interleaved in that one window (`before-probe` `d9bf393e…` = current
tree with the probe disabled; `after-probe` `fca1053b…` = the fix):

| arm | run | wall | failing tests | excluding the control test | connect-leg | request-leg | 8022 |
| --- | --- | --- | --- | --- | --- | --- | --- |
| before | 1 | 816 s | 14 | **13** | 9 | 3 | 2 |
| before | 2 | 99 s | 1 | **0** (the only red *was* the control test) | 0 | 0 | 0 |
| after | 1 | 802 s | 10 | **10** | 7 | 2 | 0 |
| after | 2 | 99 s | 0 | **0** | 0 | 0 | 0 |

**So: the fix removes exactly the defect its control test names, and no more.** At the load where
this family is reachable the red set is dominated by tests whose *own commands* time out (the
fixture's client, fake broker and host bridge share one starved runtime), which no probe can fix; the
victims differ between runs (the same stochasticity §13.3/§13.5 recorded for this filter), so 13 vs 10
with n=2 per arm is **noise, not a rate**. No rate improvement is claimed. What is claimed is
positional and deterministic: the pre-fix binary fails the new test, the fixed one passes it, and the
next command on that connection still succeeds.

At normal load the affected lane is green: `remote_peer::` + `commands::remote_peer::tests` →
**200 passed / 0 failed** in 262 s (`-j 8`).

### 13.7.4 What this fixes for CI, and what it does not

* **Fixes:** the `#1094` shape — `NW001 … recovered after 1 attempts and 2410ms` followed by
  `download: Message("peer_not_connected")`. A host that is slow now stays connected, so the retry
  (`transfer::retrying`, 3 attempts) re-sends over the same socket instead of reporting a peer that
  is gone. That is a product behaviour change and it is the intended one.
* **Does not fix:** a fixture so starved that its own commands time out (no budget was changed — see
  §13.6.5 for why raising one was withdrawn), and the 8022 shared-port family (owner
  `todo_0aedf87f73f8`), which supplied 2 of the before-arm's reds.

## 13.8 The connect leg: a transient failure no longer waits for a user

*(Unit F-TAURI-5, `todo_071e60ce3281`. §13.7 fixed the *request* leg; this completes the pair.)*

### 13.8.1 What was wrong, restated from the code

`runtime::connect_with_emitter` recorded the reason and returned on **any** `session::connect`
failure. Nothing scheduled a retry — `on_transport_failure` only ever runs for a *live* connection
that dropped — so an unreachable socket (the 5 s `NATS_CONNECT_TIMEOUT` this crate now sets, a
refused connection, DNS) put the host in an error state that only a user action cleared, even though
the **pairing was still valid**. Measured as the largest single class of the family: 6 of 13 reds in
§13.6.2 were `Failed to connect to NATS: timed out`.

There was a *test* encoding the old policy — `a_failed_first_connect_does_not_start_retrying_on_its_own`,
"a user who watched Connect fail should see that failure, not a quiet retry" — so this section is a
deliberate policy change, not an accident, and the test was rewritten rather than deleted: **both**
halves now hold. The caller still gets the error (the Connect button still reports it); the runtime
also schedules the same bounded retry the drop path schedules (`reconnect_attempt_budget`), so the
host comes back without anyone pressing anything.

### 13.8.2 The change: two classifications, one decision

| piece | what it does |
| --- | --- |
| `session::classify_connect_error(kind, message)` | maps the library's own `ConnectErrorKind`: `Authentication`/`AuthorizationViolation` → `AppError::RemoteAuthorization`; everything else (`TimedOut`, `Io`, `Dns`, `MaxReconnects`, `Tls`, `ServerParse`) → `AppError::RemoteTransport`. By **kind**, not by text — several legs say "timed out", and matching the phrase is how the crate would start retrying a rejected credential |
| `runtime::should_retry_connect(error, user_stopped, already_retrying)` | retry iff transient **and not** user-stopped **and not** already retrying. Each condition is load-bearing: a refused credential can never be repaired by a retry (and retrying would hide the re-pair the user has to do); a host the user disconnected must stay disconnected; and the loop's own `resume` comes back through `connect_with_emitter`, so without the third check every failing attempt would spawn another loop |

The `AppError` variant already existed for exactly this distinction (`RemoteAuthorization`'s own doc:
"not a network outage, so retrying forever cannot repair it"), so no type was added — the connect
path simply starts using it.

### 13.8.3 Both arms pinned, and the alive arm is controlled

| arm | test | control (schedule disabled, `c826cbc0…`) | fixed (`c455972f…`) |
| --- | --- | --- | --- |
| transient failure → **schedule** a bounded retry | `a_transient_first_connect_failure_schedules_a_bounded_retry` (unreachable port) | **FAILS** — `a transient connect failure must schedule a retry, not wait for a user action` | passes: the error is still returned **and** `supervisor_count() > 0`, and the host still reads as not connected while the retry is pending |
| the decision table | `connect_retry_policy_is_transient_and_not_stopped_and_not_already_retrying` | (pure) | 5 of 5 combinations: transient→retry; authorization→no; other fault→no; user-stopped→no; already-retrying→no |
| refusal vs network | `session::tests::connect_failures_are_classified_by_their_kind` | (pure) | both authorization kinds → `RemoteAuthorization`; `TimedOut`/`Io`/`Dns`/`MaxReconnects` → `RemoteTransport`; the message is carried through unchanged |
| the scheduled retry does not **outlive the test** | assertion inside `a_transient_first_connect_failure_schedules_a_bounded_retry` (`supervisor_count() == 0` after `teardown()`) | **FAILS** with the abort disabled — `left: 1, right: 0` | passes |

The two pure tests exist because the fixture **cannot** produce a rejected credential or a
user-stopped host on demand (`FakeNats` advertises `auth_required: false`), and a test that cannot
fail in the arm it names is worse than no test.

### 13.8.4 Rate, stated before the number: on this host it cannot move

The affected tests assert that a connect **succeeds**. A bounded retry cannot make a peer answer
inside a test's own lifetime — it can only make the *app* recover later — so the test-level reds of
§13.6.2 ("connect timed out") are **not** expected to fall. What the rig is being asked, honestly, is
the **cost** question: does scheduling a retry on failure make a starved run worse (more load, more
nested attempts, longer walls)? A/B with both binaries in one window, same rig as §13.6.2
(`taskpolicy -b`, 40 burners, two concurrent rigs, `-j 32`):

| window | rig | load | before-connect (no retry) | after-connect (retry) |
| --- | --- | --- | --- | --- |
| 09:54 | 20 burners, 2 rigs, **2 runs each** | 27–40 | **0 red / 2 runs**, 90 s + 88 s | **0 red / 2 runs**, 89 s + 86 s |
| 09:47 | 40 burners, 2 rigs | 60–65 | both run 1s **capped at 900 s** | both run 1s **capped at 900 s** |
| 10:09 | 40 burners, 2 rigs | 78–85 | run 1 capped, 13 reds at the cap | run 1 capped, 17 reds at the cap |

Read honestly:

* **At a load where the filter can finish, both arms are green (4/4 runs)** and the wall times are
  within a second of each other — the retry schedule costs nothing measurable there.
* **At the load where this family actually reproduces (40 burners + two rigs), the rig cannot resolve
  a difference**: *both* arms are capped at 900 s, and a red count taken at a cap is a snapshot of
  how far each run had got, not a rate. That is the third time this filter has refused to yield a
  rate (§13.3, §13.5, §13.7.3), and it is now stated as a property of the rig: **above ~40 burners
  with two concurrent rigs, this filter cannot finish on this host, on any binary.**
* **The one number that did move was my own fix's fault, and the fix is the finding of this
  section.** While assembling the first A/B (09:47 window, before the leak was noticed) the
  *after* arm's mid-run red count was **18 against the before arm's 9**. Mechanism, found by
  reading rather than by statistics: a supervisor started by a *failed connect* is not tied to any
  test's lifetime, `teardown()` never aborted it, and the **test build's retry schedule runs on
  `delay_unit()` = 1 ms** — so each loop spends the rest of the binary's life re-attempting a
  connect (up to the 5 s budget each) against the in-process broker, i.e. a CPU burner per failed
  connect. That is FLAKES.md §4b's warning landing exactly as written: *a fix can introduce a
  flake, and the fix's own suite is the only place it shows*.
  The repair is in the fixture, not the policy: `teardown()` now calls
  `runtime::abort_supervisors_for_test()` (drains and aborts every loop), so no loop outlives the
  test that made one — including the pre-existing leak from a dropped connection, which had the
  same shape all along. Pinned **deterministically** rather than by a red count: the new test
  asserts `supervisor_count() == 0` *after* `teardown()`, and that assertion **fails on a control
  build with the abort disabled** (`left: 1, right: 0`). The 9→18 observation is recorded as
  *suggestive, confounded by load* (60 vs 83 in the third window), which is why the claim rests on
  the deterministic assertion instead.

**Lane verification, in the same section because it is the gate that matters:** `remote_peer::` on
the frozen fixed binary → **202 passed / 0 failed**, exit 0, 264 s (`/tmp/f5-final2.log`). An earlier
pass on the same code read 201/1, the single red being the 8022 fixture, which passes in isolation.

**Commit-state note (checked 2026-10-17 10:40, the §13.6.6 trap again).** The supervisor's mid-flight
commit captured the *policy* half of this section (`classify_connect_error`,
`should_retry_connect`'s real body) but **not** the leak fix or its pin — at the time of writing,
`git show HEAD:…/runtime.rs | grep -c abort_supervisors_for_test` → **0**, and the working tree
carries it in `runtime.rs` + `testing.rs` + the new assertion in `runtime_tests.rs` (+34 lines). The
dangerous direction was checked explicitly: HEAD does **not** contain the disabled-control body
(`let _ = (error, user_stopped, already_retrying)` → 0 matches in HEAD and 0 in the tree). A reviewer
must read the working tree, or the next supervisor commit's content, not `HEAD` alone.

### 13.8.5 What this fixes for CI, and what it does not

* **Fixes:** a host that was merely unreachable when the app asked (the majority class of the CI
  log's failures: `Failed to connect to NATS: timed out`) now gets the same bounded retry a dropped
  connection always got, instead of an error state nobody retries. Combined with §13.7, both legs of
  a request now distinguish "slow/unreachable" from "refused".
* **Does not fix:** the test-level rate (13.8.4 — a retry cannot help a test that needs the peer
  *now*), the 8022 shared-port family (owner `todo_0aedf87f73f8`), and the Noise *handshake* leg,
  which still has its own 10 s budget and its own timeout message; if a successor finds it flipping
  under load the answer is the same shape again (probe before concluding), not a bigger budget.
* **Uncertainty I am keeping:** a failed `connect_all` now starts one bounded loop **per unreachable
  host**. That is what the drop path already does, and the budget is finite, but it is a change in
  the multi-host case that no test here measures under load.

## 14. Item 8 — the launch fault that stopped the first attempt, and the workaround that fixed it

*(This section is the second attempt's record. The first attempt — the same unit, an earlier turn —
reported the arms as unrunnable; that report was correct at the time and is superseded here, with the
cause and the workaround, because the workaround is reusable and the failure mode is a trap.)*

The rate rig (frozen binaries, so the tree can be edited between arms without rebuilding the other
side; same filters, same `-j`, one load window):

```
/tmp/xfer-ab/run-arm.sh <binary> <label> <runs> 32      # runs the binary directly: no cargo, no build lock
  filters: remote_peer::transfer_tests  commands::remote_peer::tests
  before: /tmp/xfer-ab/before/futureos_lib  sha256 2291d0b7…  (the 500 ms test budget)
  after : /tmp/xfer-ab/after/futureos_lib   sha256 0de1dc65…  (this unit's fix)
  16 CPU burners (`timeout 2700 sh -c 'while :; do :; done'`, pid file /tmp/xfer-ab/burners.pid,
  stopped by this unit; load ~19 → ~45 while they ran)
```

### 14.1 The fault: a *new inode* must clear macOS code-signature policy, and killing the attempt restarts it

The first attempt's diagnosis is above: from 05:25 the `before` copy would not launch — `_dyld_start`,
112 K footprint, `STAT SN`, 0.0 % CPU, empty `sample` call graph — while `syspolicyd` sat at
900–1080 % CPU re-verifying the ~208 MB test binaries that four workers were rebuilding. Two further
measurements this turn pin the mechanism and give the workaround:

| probe | result |
| --- | --- |
| `/tmp/xfer-ab/after/futureos_lib --list` (the copy that had *already* launched once, at 05:39) | **rc 0, instant**, 2099 tests |
| `cp /tmp/xfer-ab/after/futureos_lib /tmp/xfer-ab/t1/fresh_after` then run it — **identical bytes, new inode** | **rc 124 at a 60 s cap**, no output |
| `…/before/futureos_lib --list` under a `timeout 240` (one of five killed attempts since 05:41) | **rc 124**, 0 lines |
| **the same binary, one uninterrupted `timeout 1500` attempt** | **rc 0 after 315 s**, 2093 tests listed |

So the gate is per **inode**, every freshly created file must be verified once, verification takes
**~5 minutes** while `syspolicyd` is saturated — and **a `timeout` that kills the attempt throws the
verification away and the next attempt starts from zero**, which is why five short attempts in an hour
made no progress while one long one succeeded. Two consequences for the campaign, both general:

* **Never wrap a first launch of a new binary in a short timeout**, and do not read "it did not start
  in 60 s" as "it cannot start". Give it one uninterrupted attempt (the 315 s figure is this host's
  number; the pre-flight `--list` is cheap enough to spend it on).
* **A frozen copy is worth the wait**: once launched, that inode starts instantly forever, which is
  what makes the direct-binary arms (`run-arm.sh`) possible at all — no `cargo`, no build lock, no
  interference with peers.

The clamp on the CLI tool itself (`timeout -k 5 600` per run inside `run-arm.sh`) is unaffected: by
the time the arms run, both inodes are verified, and a run that exceeds 600 s is a hang worth
recording. The *pre-flight* is what must not be killed.

* **The `after` binary was verified in this window:** `remote_peer::transfer_tests` → **39 passed,
  0 failed, 60.9 s** at `--test-threads=8`, load 15. The rewritten negative test
  (`a_chunk_that_never_arrives_gives_up`, now 200 ms via `pull_chunk_within`) passes with the same
  assertion, and the fixture assertion added in §13.2 holds.
* **The `before` arm could not be run, and the reason is not the test.** From 05:25 local the host
  stopped launching this crate's freshly built test binaries: every attempt sat at `_dyld_start`
  with a 112 K footprint, `STAT SN`, **0.0 % CPU**, and an *empty* `sample` call graph (no thread
  ever reached `main`) — for the frozen `before` copy, for the freshly built `after` binary, and for
  three peer `--test-threads=1` runs of the same binary in this worktree (15+ minutes each). During
  the same window `/usr/bin/true` launched in 4 ms, so exec itself was fine and the stall is
  specific to newly created large binaries. The daemons: `syspolicyd` at **900–1080 % CPU** and
  `XProtectService` at 94 % — macOS code-signature/XProtect verification re-checking every new
  208 MB test binary produced by ~4 concurrent workers, i.e. a queue this host could not drain.
  One probe launched after 16 s and listed all 2099 tests, so the block is a queue, not a refusal.
* **A second, smaller measurement obstacle in the same window:** `cargo test` in
  `desktop/src-tauri` blocked on `Blocking waiting for file lock on build directory` for >7 min
  behind a peer unit's `cargo test --manifest-path …/cov100-r2/desktop/src-tauri/Cargo.toml --lib
  -- --test-threads=1` (this crate's target dir is shared by convention for flake work, so a peer's
  ~15 min serial run serializes behind its own build lock).

**Independently corroborated.** Another unit hit the same fault in the same minutes and recorded
it with a detection recipe: `docs/testing/module-tauri-appdata.md` §S12.6a — no `rustc` in `ps`, a
target dir frozen at 5.3 MB, `syspolicyd` at 1072–1115 % CPU, other units' `cargo test` at 0 % for
8–13 minutes, and the same corollary this unit measured: a wedged peer's `cargo` holds
`desktop/src-tauri/target`'s lock, so every other unit's plain-target gate blocks on it. That file's
recipe (rustc present? target dir growing? syspolicyd pegged?) is the cheap first check for any
stall of this kind; §14.1 adds the step that file did not have — **give the first launch of a new
binary one *uninterrupted* attempt (~5 min here) rather than several short ones**, because a killed
attempt restarts the verification.

### 14.2 Consequence: the arms ran after all, and the number they give is the opposite of the hypothesis

With the workaround, the rate runs were obtained (§13.3 natural: 10 + 10; §13.4 starved: 3 + 3). The
summary:

* **`remote_download_chunk_missing` — the budget's own failure signature — appears 0 times in 26 runs**
  (20 natural + 6 starved), and the other budget-class strings (`peer_not_connected`,
  `remote_download_size_mismatch`) likewise.
* In the **natural** arms, all 18 reds are the **8022 shared-port fixture** (`remote_peer/testing.rs:30`),
  a different mechanism owned by the shared-fixture workstream.
* In the **starved** arms the reds split by cause and neither cause is the budget: the `before` arm was
  green 3/3, and the `after` arm's two red runs carry **5 request/connection-timeout signatures** and
  **4 × 8022** (§13.5 reads that text — and it is the CI log's own family).

So item 8's *rate* is measured and its value is **zero on both sides**, which the todo's own text
anticipated ("if the before arm is green in >=10 runs, that IS the answer"). The substantive result of
this turn is the **correction**: starving the delivery path fivefold does not make the post-ack 500 ms
wait fire, and what starvation *does* break is the request/connection half — which is also what the CI
log's signature says. This unit therefore claims the fix as *removal of a test-only wall clock* (with a
grep-verified statement that no `cfg(test)`-shortened budget remains on the transfer path) and
**explicitly does not claim** that it fixes the CI flake the item was filed for; §13.5 says where to
look instead.


## 15. Item 1 — gates, and the status of both items

### 15.1 Formatting and the affected tests

```
cd desktop/src-tauri
rustfmt --edition 2021 --check src/remote/test_support.rs \
    src/remote_host/business/settings.rs src/remote_peer/runtime.rs src/remote_peer/transfer_tests.rs
```

One diff, in this unit's own new test (`assert!(scanned > 100, …)` wanted wrapping); fixed with
`rustfmt --edition 2021 src/remote/test_support.rs` (per file, PLAN §2b.5 — a crate-wide `cargo fmt`
was not run). All four files are clean now.

The affected tests, run together at `--test-threads=8` (`127 passed; 0 failed; 0 ignored` in
**65.3 s**, rc 0):

```
target/debug/deps/futureos_lib-a38f137727fdbb37 --test-threads=8 -- \
    remote_peer::transfer_tests remote_host::business::settings::tests \
    remote::test_support::tests remote::commands::bridge_tests
```

Serialized, the acceptance pair of §12.6 is unchanged: `cargo test --lib -- --test-threads=1
remote_host::business::settings::tests` → **6 passed**, and `… --test-threads=1
remote::commands::bridge_tests` → **65 passed** (`--test-threads=1` immediately after the one `--`,
FLAKES §8c).

### 15.2 Clippy — green in **both** flavours (2026-10-17, second attempt)

```
cd desktop/src-tauri
cargo clippy --all-targets -- -D warnings                                    → rc 0
cargo clippy --all-targets --no-default-features --features headless -- -D warnings → rc 0
```

Both runs finished in ~7 s against the campaign's shared target dir
(`Finished \`dev\` profile … in 6.63s`), each with **0 diagnostics**; logs
`/tmp/clippy-alltargets-flake.log` and `/tmp/clippy-headless-flake.log` (the first also carries an
earlier `--lib --tests --benches` run, also rc 0).

**How the two obstacles of the first attempt were cleared**, since both are reusable:

* *The shared build lock.* A peer's `cargo test --manifest-path …/desktop/src-tauri/Cargo.toml --lib
  -- --test-threads=1` holds `desktop/src-tauri/target/.cargo-lock` for its whole run, so a gate that
  gives up after a short wait reports nothing. The fix is not a private target dir (that was attempt
  2 last turn, and it wedged on the same host fault while compiling dependencies) but to **let cargo
  wait**: launch it in the background with **no kill-timeout** and read the result when it lands.
  Once the peer's run ended, the gate completed in seconds from the warm cache.
* *The launch/verification fault.* Not an issue for clippy: `-D warnings` only needs `rustc`/
  `clippy-driver`, which are pre-existing, already-verified binaries. What is affected is *running a
  freshly linked test binary* — see §14.1.

## 16. What I could not do, and what the next reader should check

0. **The request-class timeout is characterized *and* the connection-drop defect is fixed (§13.6,
   §13.7).** §13.6 characterizes: nothing is lost (0 of 1093 broker deliveries reached no subscriber;
the host received every slow request; replies arrive late) — the fixture's client, fake broker and
host bridge share one starved current-thread runtime, so the client's own budgets fire. §13.7 fixes
the consequence that mattered: a request timeout over a live connection no longer drops the socket
(which is how a slow host became `peer_not_connected`, the CI `#1094` shape), with both arms pinned
and the alive arm controlled against a pre-fix binary.
   **§13.8 then fixed the connect leg too** (F-TAURI-5): a transient connect failure schedules the same
bounded retry a dropped connection always got, and a refused credential does not
(`classify_connect_error` by the library's kind + `should_retry_connect`), with the retry loop now
aborted by `teardown()` so the test build's 1 ms schedule cannot leak into later tests.
   **Still open, stated rather than hidden:** (a) **the rig cannot yield a rate for this family at
all** — at a completable load both arms are green (4/4), and above ~40 burners with two rigs *every*
binary caps at 900 s (§13.8.4); so no before/after *rate* exists for §13.7/§13.8, only controls and
wall-time parity; (b) the 8022 fixture is the dominant reproducible failure of this filter (owner
`todo_0aedf87f73f8`); (c) the **Noise handshake leg** still has its own 10 s budget and its own
timeout text — same shape, not yet examined; (d) a failed `connect_all` now starts one bounded loop
*per unreachable host*, unmeasured under load (multi-host case).

1. **Item 8's recorded mechanism is contradicted by the measurement, and the real one is now
   reproduced (§13.4/§13.5).** Rates: `remote_download_chunk_missing` appears **0 times in 26 runs**
   (20 natural, 6 starved at 5.0×). What starvation *does* break is the request/connection half —
   `Remote request failed: request timed out: deadline has elapsed` and `Failed to connect to NATS:
   timed out`, on **both** binaries, i.e. a **production** 10 s budget (`session::COMMAND_TIMEOUT`),
   not the test-only one. That is the family the CI log belongs to (`NW001 … 2410ms` then
   `peer_not_connected`). The next check: what makes that request path or the socket under it exceed
   10 s on a saturated host — and whether that budget can be derived from **progress** rather than
   wall-clock, which is the same question the item asked, one layer up. The obvious lever, since the
   failing tests are fixture-driven (an in-process host bridge over the fake broker), is the
   fixture/host path itself rather than any client budget. Do not read §13 as "the CI flake is
   fixed": it is "the test-only short budget is gone, and it was not the cause".
2. **The 21-test parallel red set of §12.6 is unmeasured against a "before" tree, and cannot be
   measured**: the command did not complete before the fix. If the closing sweep needs a green
   full-suite run, the families to clear are the five listed in §12.6 — they are all owned by other
   units or by the shared-fixture workstream, and one of them (`skills_bootstrap` / NEW-D) is
   *expected* in any parallel run.
3. **`cargo clippy --all-targets -- -D warnings` is green in both flavours (§15.2)** — closed by this
   attempt, with the two obstacles (a peer's build lock; the launch/verification fault) recorded
   there because both recur.
4. **The two-lock census is static (§12.4).** It sees call edges only for names defined once; a
   helper defined twice could hide an inversion from it. The runtime cross-check is the suite
   itself: the fix's own full-suite parallel run completed (§12.6), and the enforcement test
   (§12.5) fails the build on the direct shape.
5. **`HomeGuard::new` runs `crate::runtime::cancel_test_tasks()` while the HOME lock is held**
   (`auth_store.rs:169`). Taking `mock_agent_lock()` first (§12.3) means that cancellation now
   happens with every other mock-driving test excluded, which is strictly safer — but it is worth a
   successor's eye, because it is the one *side effect* of the reorder and it was not previously
   inside the mock lock's critical section.
6. **The 8022 fixture family dominates any parallel run of this filter** (18 of 18 reds here, and 5
   of the 21 in §12.6): `remote_peer/testing.rs:19` pins `WEB_PORT = 8022` with a 5 s
   `wait_for_web_port_free`. It is owned by `todo_0aedf87f73f8`, but it is the reason a transfer-lane
   rate cannot be read cleanly: **any** budget or timing experiment on this filter has to be
   classified through the greps of §13.3, not by the pass/fail of the run.


## 17. Status of this unit's two items

| item | shape | outcome |
| --- | --- | --- |
| **1** — the lib suite deadlocks in parallel (two process-global test locks taken in opposite orders) | order/topology-dependent, total deadlock (0 % CPU, no panic, no summary) | **fixed.** Negative control `TIMEOUT` (300 s) → **3/3 pass in 7.0 s each**; census **2 inversions → 0** over 191 files / 89 both-lock bodies; the order is enforced by a source-scanning test with its own control; the full suite now runs to a summary in parallel (583 s, `2058 passed / 21 failed`, §12.6) and the two groups pass serialized (6 and 65 tests). |
| **8** — a 500 ms **test-only** budget on the chunk pull | arm made deterministic; the recorded mechanism **not confirmed by measurement** | **landed + rate measured (zero on both sides).** The budget is an argument now, production's 15 s is what the suite runs on, and the give-up arm has its own deterministic test (200 ms, nothing can arrive). Rates (§13.3/§13.4): **0 budget-class failures** in 20 natural runs (10 per arm, `-j 32`, load 18–30) and in 3 runs of the *before* binary starved **5.0×** (`taskpolicy -b`); the 18 reds that did occur are all the 8022 fixture. Grep-verified: no `cfg(test)`-shortened budget remains on the transfer path. **The CI signature is the request/connection half, and §13.5 reproduces it here** (`Remote request failed: request timed out`, `Failed to connect to NATS: timed out`, on *both* binaries, 0 × `remote_download_chunk_missing`) — see §16.1 for the next check. |
| **F-TAURI-4** (§13.7) — a request timeout **drops a live connection**, so a slow host becomes `peer_not_connected` (CI `#1094`) | product behaviour, order/topology of *state*, not timing | **fixed, controlled, and the rig says "no rate change".** `on_transport_failure` now asks the client (`is_connected()`) before condemning a connection on a *request timeout* (`is_request_timeout`, matched on "request timed out" so the connect leg is excluded); everything else drops and retries as before. Both arms pinned: the dead-transport arm (`a_chunk_write_on_a_dropped_broker_is_recorded`, + a retry assertion) and the **new** live-but-slow arm (`a_request_that_times_out_keeps_a_connection_that_is_still_up`), whose control **fails on the pre-fix binary** (`one slow command must not drop a live connection`) and passes after. A/B on the §13.6 rig (load 53–62): before 13 real reds / after 10 in run 1, both arms 0 in run 2 — **dominated by fixture starvation, so no rate is claimed**; at normal load the lane is **200/200 green**. |
| **F-TAURI-5** (§13.8) — a **transient connect failure** left the host in an error state only a user could clear | policy: transient vs permanent, not timing | **fixed, both arms controlled.** `session::classify_connect_error` maps `ConnectFailureKind::Authentication|AuthorizationViolation` → `RemoteAuthorization` (permanent) and the rest → `RemoteTransport`; `runtime::should_retry_connect` schedules the bounded retry iff transient **and** not user-stopped **and** not already retrying. Caller still gets the error; the host now recovers on its own. Controls: the scheduling test **fails** with the schedule disabled (`a transient connect failure must schedule a retry…`), the decision table is 5/5, and the classification test covers both arms. **A defect the fix introduced was found and fixed in the same pass**: the scheduled loop outlived its test (test-build schedule = 1 ms), so `teardown()` now aborts every loop — pinned deterministically, and the pin **fails** with the abort disabled (`left: 1, right: 0`). Rate: none available (§13.8.4 — 4/4 green at a completable load, both arms capped at the reproducing load). |
| **gates** | — | `cargo fmt --check` clean on the three touched files; `cargo clippy --all-targets -- -D warnings` **rc 0** in **both** flavours (gui + headless); affected lane 200/200 (§13.7.3). |
| **F-TAURI-3** (§13.6) — the same lane's **request-class** timeout: which leg is slow | wall-clock budget vs a starved in-process fixture | **characterized; the budget change was withdrawn on evidence.** Diagnostic (3 boundaries, reverted): **0 of 1093** broker deliveries reached no subscriber, the host received every slow request, one reply arrived *after* the deadline, one *successful* handshake took 8.05 s of the 10 s budget. Taxonomy of 13 reds in 4 runs (load 60–75): 6 connect-leg, 2 request-leg, 5 the 8022 fixture. **Kept:** the production budgets named and set explicitly (`NATS_CONNECT_TIMEOUT` 5 s, `request_budget` 10 s — both previously *inherited* `async-nats` defaults, so production behaviour is unchanged), a pinning test, and `put_chunk_within` (two observing tests **14.28 s → 4.14 s**). **Withdrawn:** a `cfg(test)` 60 s fixture tolerance — it turned the reproducing rig's reds into **900 s timeouts** (§13.6.5). Next step named: a timeout currently *drops the connection*, which is how a slow host becomes `peer_not_connected` (§13.6.6). |

Artifacts this record's measurements rest on (all under `/tmp`, none committed):

| path | what |
| --- | --- |
| `/tmp/sample-hang-93092.txt` | the `sample` of the hung negative-control run (§12.2), 70 threads |
| `$TMPDIR/flake-tauri-deadlock-{before,after}/` | the negative control's logs (`tauri-deadlock-before-1.log` = the 300 s hang; `tauri-deadlock-after-1.log` = 71 passed in 7.03 s) |
| `$TMPDIR/flake-tauri-lib-after/tauri-lib-after-1.log` | the full parallel suite run (§12.6), incl. the final 21-name `failures:` block |
| `/tmp/lockorder.py` | the two-lock census (§12.4) — `python3 /tmp/lockorder.py src`, `-v` for the full table |
| `/tmp/xfer-ab/{before,after}/futureos_lib` | the two frozen binaries of §13/§14 (`2291d0b7…` = 500 ms budget, `0de1dc65…` = this fix) |
| `/tmp/xfer-ab/run-arm.sh`, `run-arm-hostile.sh`, `burn.sh` | the rate rigs of §13.3 (direct binary, no cargo) and §13.4 (`taskpolicy -b`), and the burner batch |
| `/tmp/xfer-ab/logs-{after,before}/` | the natural arms' 20 run logs + `failing.txt`/`panic.txt` summaries (§13.3) |
| `/tmp/xfer-ab/diag-logs/diagB-*.log` | the **F-TAURI-3** diagnostic runs (4, load 60–75) carrying `DIAG nats deliver … targets=`, `DIAG host recv`, `DIAG client request … elapsed_ms=`; `correlate.py` is the script that pairs them |
| `/tmp/xfer-ab/correlate.py`, `/tmp/xfer-ab/panics.py` | the correlation and per-test-panic extractors of §13.6 |
| `/tmp/xfer-ab/diag/futureos_lib` (`c6a4b1d8…`-era diag build), `/tmp/xfer-ab/fix/futureos_lib` (`0708fd0e…`, the **withdrawn** tolerance), `target/debug/deps/futureos_lib-a38f137727fdbb37` (`2ef1108c…`, the final tree) | the three binaries of §13.6 — pre-fix, tolerance, final |
| `/tmp/fix-verify.log`, `/tmp/fix2-verify.log` | the affected-test runs (`125 passed` and `12 passed` respectively) |
| `/tmp/xfer-ab/{before-probe,after-probe}/futureos_lib` | the §13.7 A/B pair: `d9bf393e…` = current tree with the probe **disabled** (also the positive control for the new test), `fca1053b…` = the fix |
| `/tmp/xfer-ab/probe-logs/` | the §13.7 rig logs (4 runs, load 53–62) — `before-probe-{1,2}.log`, `after-probe-{1,2}.log`, `driver.log` |
| `/tmp/f4-verify.log` | the todo's own `--verify` command: `cargo test --lib -- remote_peer::` → **200 passed, exit 0** |
| `/tmp/clippy-f4.log`, `/tmp/clippy-f4-headless.log` | the two clippy gates of §17, each `…_EXIT=0` |
| `/tmp/xfer-ab/{before-connect2,after-connect2}/futureos_lib` | the §13.8 A/B pair — `81307733…` = the fix with the retry schedule disabled (control), `9b3820b6…` = the fix. Both carry the teardown abort, so the only difference is the schedule |
| `/tmp/xfer-ab/connect-logs{2,3}/` | the §13.8.4 windows: `2` = 20 burners (both arms green, 4/4), `3` = 40 burners (both arms capped) |
| `/tmp/xfer-ab/connect-logs/` | the **pre-leak-fix** A/B (9 vs 18 reds at the cap) that exposed the retry-loop leak — kept because that comparison is the evidence for §13.8.4's third bullet |
| `/tmp/f5-lane.log` | `remote_peer::` on the fixed tree (first pass): **201 passed / 1 failed** — the 8022 fixture (`testing.rs:30`), which passes in isolation |
| `/tmp/f5-final2.log` | `remote_peer::` on the frozen fixed binary (second pass): **202 passed / 0 failed**, exit 0, 264 s. Run from the frozen copy because the *freshly linked* binary stalled at launch for 9.5 min at `_dyld_start` with 0.0 % CPU and no output — the §14.1 per-inode code-signature queue, met again; the frozen copy is the same build (`9b3820b6…`) and starts instantly |
| `/tmp/clippy-f5.log` | `CLIPPY_GUI=0`, `CLIPPY_HEADLESS=0` |
| `/tmp/xfer-ab/logs-hostile-{before,after}/` | the starved arms' logs (§13.4) |
| `/tmp/preflight-before-long.log` (`rc=0 in 315s`) | the uninterrupted launch that made the `before` arm runnable at all (§14.1) |
| `/tmp/clippy-alltargets-flake.log`, `/tmp/clippy-headless-flake.log` | the two clippy gates, each `…_EXIT=0` (§15.2) |
| `/tmp/gate-tests2.log`, `/tmp/bridge-{1,2,3}.log` | the affected-test gate (127 passed) and the bridge group 3× parallel (65 passed each) |

If the frozen copies in `/tmp` are gone by the time a successor runs, the `before` binary is
repairable from git: this unit's item-8 edits were swept into commit `a80887d12` (a peer's
supervisor commit), so `git show a80887d12^:desktop/src-tauri/src/remote_peer/runtime.rs` and the
same for `transfer_tests.rs` give the pre-fix text, and §13.2 states the entire diff (one signature
split, one contract restored to 15 s, one test's trigger). The `after` binary is whatever
`cargo test --lib --no-run` produces on the current tree — and per §14.1 its **first launch must be
given one uninterrupted attempt** (or, cheaper, copy it to a path already launched once: the same
inode, so no re-verification).

Handoff in one line per item: **item 1 is done with before/after evidence and an enforcement test;
item 8's test-only budget is gone and its rates are measured (0/20 natural, 0/6 starved) — with the
finding that the recorded mechanism does not match the CI log's signature, so the request/connection
half of the pull is where the CI flake should be hunted next (§16.1); F-TAURI-3 characterized that half
(§13.6) and shipped the contract fix and the seam, while its budget change was withdrawn on evidence;
the gates are green.**
