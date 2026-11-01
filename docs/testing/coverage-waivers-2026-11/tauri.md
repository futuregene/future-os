# tauri (`desktop/src-tauri`) — 2026-11 coverage waivers

**Metric A** — a line counts as uncovered iff its LCOV record is `DA:<line>,0`. The llvm-cov
summary's `Lines` column is not the acceptance number.

**Lane** — `desktop/src-tauri` (crate `futureos`). Two legs, both `--features test-support`:
`cargo llvm-cov -p futureos --lib` (the default `gui` feature) and the `harness = false`
main-thread driver `cargo llvm-cov -p futureos --test gui_main_thread_drive`. The lib is a
single target, so this is the whole crate's unit-test surface — there are no integration
binaries beyond the driver.

**Last measurement**: `test/tauri-cov2` (waves 6-8), rustc **1.99.1** (`rust-toolchain.toml`),
`ulimit -n 8192`, raw profiles deleted first (`--no-report` accumulates). Artifacts
(gitignored) in `coverage/2026-11/tauri/`: `wave10-lcov.info` (the numbers quoted below; wave 9's
`wave9.md` was written against the pre-`a8669c8b3` base and re-measured here),
`wave10-lib.log` / `wave10-driver.log` / `wave10-report.log`, the run script `wave10.sh`, the
per-wave records `wave6.md` (waves 6-7) and `wave9.md` (waves 8-9), the classification
`classification.md`, and the diff tools `diff_lane.py` / `da0_lane.py` / `da0_lines.py` /
`da0_file.py`.

`da0_file.py` takes **one** file by its whole record path; `da0_lines.py` matches a suffix, which
silently unions `desktop/src-tauri/src/lib.rs` with `host/src/lib.rs` and `packages/*/src/lib.rs`
(546 lines against the real 48). Quote `da0_file.py` for a per-file row.

```bash
# One measurement, both legs, fresh profiles (the script does the rm for you).
coverage/2026-11/tauri/wave10.sh
python3 coverage/2026-11/tauri/da0_lane.py  coverage/2026-11/tauri/wave10-lcov.info           # lane total
python3 coverage/2026-11/tauri/da0_file.py  coverage/2026-11/tauri/wave10-lcov.info \
        desktop/src-tauri/src/lib.rs                                                         # per line, one file
cargo test -p futureos --lib                                  # gate: 421 passed, ulimit -n 8192
cargo test -p futureos --test gui_main_thread_drive --features test-support
```

## 1. Where the lane stands

| | `DA:0` | note |
|---|---:|---|
| baseline (wave 1, `05a260c6f`) | **1427** | `coverage/2026-11/rust/tauri-lcov.info` |
| wave 5 (`963`) | 963 | preparation chain moved into the driver |
| wave 6 | **864** | the real update worker end to end (−99) |
| wave 7 | **819** | worker failure arms, `confirm`, `update_process` refusals (−45) |
| wave 9 (first measurement) | 639 | the transfer commands, the host-event sink, the two open update items (−180), on the pre-`a8669c8b3` base |
| wave 10 | **638** | the same tests after `a8669c8b3` landed under them; per-file identical except the interleaving line below |

The lane is **not** at per-line 0 and does not claim to be. What it does claim: every remaining
line in the files this table covers is either registered below or named as a work item in §4;
nothing in §2 is claimed as "covered".

| file | `DA:0` | disposition |
|---|---:|---|
| `src/commands/update.rs` | 194 | work items — the IPC wrappers this lane has not reached yet |
| `src/agent_supervisor.rs` | 88 | work items — the watchdog loop body and the quit dialogs |
| `src/lib.rs` | 47 | §2.6 registered (real handle / mock-always-Ok / startup) + work items |
| `src/terminal/server.rs` | 56 | known ±1 fluctuation (§2.5) + interleaving-dependent test arms |
| `src/main.rs`, `src/terminal/pty.rs`, … | 429 | work items (see `classification.md` for the wave-1 bucket split) |
| `src/commands/history_transfer.rs` | 2 | 1 measured-unreachable command arm (§2.7) + 1 scaffolding arm |
| `src/commands/update_journal.rs` | 12 | 2 real items (§4.2) + attribution (§2.4) |
| `src/commands/update_process.rs` | **0** | closed in wave 9 (§4.1) |
| `src/commands/update_macos.rs` | 5 | 3 registered (§2.1) + 2 attribution (§2.4) |
| `src/commands/update_adapters.rs` | 15 | platform facades the worker leg reaches only on another OS |

## 2. Registered waivers

Codes used by this lane (the other lanes' `W*` codes are listed in their own files):

- `T1` — **production-only entry**: the unit derives its input from `current_exe()` being inside
  a built `.app`. A test process is never that, and no seam may change the production path to
  make it so.
- `T4` — **attribution artifact**: the statement demonstrably ran (its neighbours read `DA>0`,
  and its observable effect is asserted elsewhere), but this line's own record stays `0`.
- `T5` — test code whose body only runs when an assertion fails.
- `T6` — **environment-bound arm**: the error the line handles cannot be produced in a test
  process on the platforms CI runs (the syscall returns a *different* errno, or the mocks
  always succeed). Coupled with a positive control stating what *is* produced.

### 2.1 `src/commands/update_macos.rs` — `T1`

| line | code | mechanism (citation) | positive control | pinned by |
|---|---|---|---|---|
| 86-88 | `T1` | `Prepared::new` → `Self::prepare_at(bytes, operation, bundle()?)`, and `bundle()` is `tauri_plugin_updater::extract_path_from_executable(&current_exe())` (`update_macos.rs:11-17`): from a test binary it refuses, which is why the chain behind it takes the path as a parameter (`prepare_at`, `update_macos.rs:74-80`) | run the driver from a staged `.app` whose `current_exe()` is inside it — every line behind `prepare_at` then runs, and already does in the libtests | `prepared_case`-based libtests (`a_payload_must_hold_exactly_one_application_bundle` …) cover `prepare_at`; `production_hands_the_prepared_bundle_to_this_executable` pins the only thing `new` adds, `bundle()` |
| 187 | `T1` | `None => update_helper_program()?` — the production hand-off target; every test path passes `Some(helper)` (`prepare_with`, `update_macos.rs:118-146`), and the `None` caller is `Prepared::new` (line 86) | same `.app` host as above | `production_hands_the_prepared_bundle_to_this_executable`; deliberately **not** written as `helper.map_or_else(...)`, which would mark the line covered while the production arm never ran (wave5 §5.1) |
| 376-378 | `T1` | `confirm(id)` = `confirm_at(&bundle()?, id)`: the production wrapper, whose path again comes from `current_exe()` | a confirmed installation inside a real `.app` | `confirm_at` itself is covered by the driver's confirm step (10/10 check "a confirmed installation removes the staging directory it kept for rollback"); the wrapper adds only `bundle()` |

### 2.4 `src/commands/update_macos.rs` — `T4`

| line | code | mechanism | positive control | pinned by |
|---|---|---|---|---|
| 285 | `T4` | the `)?);` of `drop(Prepared::prepare_with_helper(...)?)` in `prepare_and_drop` — llvm attributes the multi-line call's closing line to the `?` *error* path, which cannot run on success | make `prepare_with_helper` fail after the operation begins | `prepare_and_cancel_update_for_tests` ran green in the driver (wave5 §4.2: "a prepared update that is cancelled leaves no staging directory either") — the call executed, its effect is asserted |
| 387 | `T4` | the closing brace of `confirm_at` | — | same as above: the driver's confirm step fails if this body does not run |

### 2.5 `src/terminal/server.rs` — the known ±1

`DA:0` here moves by one line between identical runs (56 ↔ 57). The two real-shell end-to-end
fixtures answer a shell's cursor query and read frames on an **interleaving**: which match arms
and which DSR branch execute depends on how the shell splits its output across reads. Measured
both directions (`wave5-lcov.info` 56 → `wave6-lcov.info` 57 → `wave7-lcov.info` 56) with no
change to the arms in between; wave5 §5.2 recorded the same family earlier.

This is **not** registered as a waiver — the lines are reachable — but a round that quotes this
file's number should quote the run it came from, not the field.

`src/lib.rs:808` belongs to the same family and was measured on both sides: the drain loop's
100 ms idle arm read `DA>0` in `wave8-lcov.info` (47 lines, no test had ever started the thread)
and `0` in `wave9-lcov.info` (48), because wave 9's event case is the first to start it. Nothing
in the arm changed — and `wave10-lcov.info` has it back at `DA>0` (47), which is what a ±1 looks
like.

### 2.6 `src/lib.rs` — `T1` / `T6`

`present_host_event` is the only sink `future_host::events::attach` registers, so the case list in
`gui::host_event_tests` covers all eleven variants and asserts the one that has a payload-free side
effect (the runtime revision only advances inside `emit_thread_runtime_updated`). The remaining
lines in the file:

| line | code | class | mechanism | positive control |
|---|---|---|---|---|
| 133 | `activate_main_window(app)` in the `ControlClaimed` arm | `T1` | reads the process-global `APP_HANDLE`, whose only writer is `run_with`'s setup; the handle has no reset, so a test may not set it | `activate_main_window` itself is covered both ways (`activate_main_window_restores_shows_and_focuses_the_main_window`, `…tolerates_a_missing_main_window`) |
| 526, 817 | `handle.emit(…)` inside the `APP_HANDLE` arms | `T1` | same global | `emit_tasks_updated` / `emit_runtime_updates_on` bodies are covered through their `_on(handle, …)` siblings; `the_emit_entry_points_are_noops_before_a_window_exists` pins the `None` half |
| 168, 171, 200 | `unminimize` / `show` / `set_focus` error arms | `T6` | the mock runtime answers `Ok(())` to all three (`tauri-2.11.5/src/test/mock_runtime.rs`) | `activate_main_window_restores_shows_and_focuses_the_main_window` runs the `Ok` half on the same window |
| 863-865, 892-948, 972-1000, 1249, 1260, 1293-1294 | `run()` and `run_with()`'s startup, menu, close and setup-failure paths | work items | need the real Wry event loop; `run_with` also writes `APP_HANDLE` | — |
| 1751, 2827 | test bodies | `T5` | their own `return` / `panic!` arms | — |

### 2.7 `src/commands/history_transfer.rs:69` — `T1`-adjacent, **measured**

`begin_session_history_transfer`'s `#[tauri::command]` line carries the async wrapper's
argument-deserialization error arm. It was measured rather than assumed: the command's arguments
are `WebviewWindow`, `thread_id: String`, `before: Option<i64>`, `limit: i64`,
`history_source: Option<String>`, and
`the_transfer_command_refuses_a_raw_bytes_payload` sends an `InvokeBody::Raw` request precisely
because `CommandItem::deserialize_option` rejects a raw payload
(`tauri-2.12.5/src/ipc/command.rs:139-151`). The request *is* refused — the assertion pins the
`bytes payload` message so a "command not found" could not pass for it — and the record lands on
the **signature** line: the first JSON argument (`threadId`) fails first. The attribute line can
therefore only be reached with a body that satisfies `threadId`, `before` and `limit` and still
fails, and no such body exists (`Option` accepts a missing key; a raw payload fails one argument
earlier). Same family as the `bootstall_builtin_skills` note in `commands/skills.rs`.

## 3. Products of the coverage work (not waivers)

Two production seams were required and are gated, behaviour unchanged. Both are the same 1:1
split the coverage campaign approved for this lane (README §"已批准的三处 tauri 例外"):

| seam | why | gate |
|---|---|---|
| `probe` → `pub(super)` | its 10 s deadline can only be *expired* by a candidate that outlives it (+10 s, unaffordable beside 400 instrumented libtests) | `#[cfg(any(test, feature = "test-support"))]` facade in `commands::update` |
| `Prepared::prepare_with_helper` / `confirm_at` | the chain and the confirmation derive their bundle path from `current_exe()`; injecting it is what makes every step behind them observable | same |

## 4. Work items

### 4.1 `update_process.rs:288-289` — **closed in wave 9**

`agent_lease`'s "another Agent started before installation" arm: `try_lock_exclusive()` fails
because a **live holder** exists. **Closed in wave 9** —
`the_agent_lease_refuses_while_another_agent_holds_the_instance_lock`. No child process is
needed and no dependency had to move: `flock(2)` locks belong to the open file description, so
two descriptors on one file *in one process* are two holders, and `fs2` is already a normal
dependency of this crate (a test target sees those too — the note that it was missing from
`[dev-dependencies]` was wrong, and adding it would have been redundant). The same call succeeds
once the holder unlocks, which is what makes the arm the lock rather than the fixture.

### 4.2 `update_journal.rs` — 12 lines left, 5 of them attribution

Closed in wave 9: the `latest()` permission arm (196-200) via an operation directory that cannot
be written into; its sibling `Err(..)` arm (202) via an `observer.lock` that is a **directory**
(`EISDIR`, not `PermissionDenied`); the "interrupted after shutdown began" branch (212-215) via a
seeded record at stage `installing`; `confirm_startup`'s lease refusal (233) with the transaction
lock held by the case, its observer refusal (252-253) with `observer.lock` held, and its body
(237-266) through to `installed`.

What is left:

| line | class | mechanism |
|---|---|---|
| 170-173 | `T6` | `acquire_transaction_lease`'s **non-`WouldBlock`** `flock` error arm. `transaction_lock()` opens the file read+write (so no `EBADF`), on a regular filesystem (so no `ENOLCK`/`EOPNOTSUPP`); the mock-free way to reach it does not exist on the CI filesystems. Positive control: the `WouldBlock` arm on the same call is covered by every `Operation::begin` collision test |
| 249-250 | `T6` | `raw_os_error() == Some(32)` in `confirm_startup`: Windows' `ERROR_SHARING_VIOLATION`. On Unix errno 32 is `EPIPE`, which `open` never returns. Positive control: the other two arms of the same `match` run in `latest_hands_back_an_undecided_record…` and `latest_refuses_an_observer_lock…` |
| 217, 261-263, 266 | `T4` | closing braces and the multi-line `host_environment::report(&format!(…))` in the cleanup-failed branch: line 260 (`DA=1`) is the statement, 261-263 are its continuation |
| 403 | `T4` | the `)?;` of `acknowledge_repair`'s `OpenOptions::open(…)`, attributed to the error path of a call that succeeded |

### 4.3 The rest of the lane

`update.rs` (194) is the IPC wrapper layer this campaign has only started on;
`agent_supervisor.rs` (88) is next by size, then the startup paths in `lib.rs` (§2.6).
`history_transfer.rs` (72) and `update_process.rs` (2) are closed. See `classification.md` for
the wave-1 class split (R reachable / E error arms / P platform-bound / C `cfg(not(test))` /
A attribution / T test code) over the original 1427.

## 5. How the wave 6-9 numbers were produced and falsified

Every scenario added in these rounds was made red before it was trusted, and each failure named
the derived value it was built on (`coverage/2026-11/tauri/wave6.md` §4,
`coverage/2026-11/tauri/wave9.md` §4):

| scenario | falsification | result |
|---|---|---|
| worker install (driver) | worker dispatch disabled | `0/6`, first line is the child's own marker |
| probe timeout | fast candidate | `after 0.2s` instead of 10 s |
| persist failure | directory left readable | `ok: true` from a worker that succeeded |
| agent-lease failure | `agent` made a directory | `ok: true`, journal `pending-validation` |
| `confirm` | staging kept | "staging left behind" |
| Agent lease held (wave 9) | `try_lock_exclusive` removed | the live-holder case goes red |
| journal observer/confirm arms (wave 9) | each guard broken in turn (5 mutations) | each case goes red |
| transfer commands (wave 9) | owner check dropped, error code renamed, publish skipped | each case goes red |
| host-event routing (wave 9) | the runtime-update arm dropped | the revision stops advancing |

One check had to be rewritten because it passed for the wrong reason: the persist-failure
scenario first failed on a *missing record*, not on the directory permission (wave6.md §4.3).
Wave 9's "another window cannot read it" case was the same shape — both mock windows were labelled
`main`, so the ownership assertion could never fail — and went red as soon as the second window
was given its own label (wave9.md §4).
