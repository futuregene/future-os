# tauri (`desktop/src-tauri`) — 2026-11 coverage waivers

**Metric A** — a line counts as uncovered iff its LCOV record is `DA:<line>,0`. The llvm-cov
summary's `Lines` column is not the acceptance number.

**Lane** — `desktop/src-tauri` (crate `futureos`). Two legs, both `--features test-support`:
`cargo llvm-cov -p futureos --lib` (the default `gui` feature) and the `harness = false`
main-thread driver `cargo llvm-cov -p futureos --test gui_main_thread_drive`. The lib is a
single target, so this is the whole crate's unit-test surface — there are no integration
binaries beyond the driver.

**Last measurement**: `test/tauri-cov2` (waves 6-7), rustc **1.99.1** (`rust-toolchain.toml`),
`ulimit -n 8192`, raw profiles deleted first (`--no-report` accumulates). Artifacts
(gitignored) in `coverage/2026-11/tauri/`: `wave7-lcov.info` (the numbers quoted below),
`wave7-lib.log` / `wave7-driver.log` / `wave7-report.log`, the run script `wave7.sh`, the
per-wave record `wave6.md`, the classification `classification.md`, and the diff tools
`diff_lane.py` / `da0_lane.py` / `da0_lines.py`.

```bash
# One measurement, both legs, fresh profiles (the script does the rm for you).
coverage/2026-11/tauri/wave7.sh
python3 coverage/2026-11/tauri/da0_lane.py  coverage/2026-11/tauri/wave7-lcov.info            # lane total
python3 coverage/2026-11/tauri/da0_lines.py coverage/2026-11/tauri/wave7-lcov.info \
        src/commands/update_macos.rs desktop/src-tauri                                          # per-line
cargo test -p futureos --lib                                  # gate: 403 passed, ulimit -n 8192
cargo test -p futureos --test gui_main_thread_drive --features test-support
```

## 1. Where the lane stands

| | `DA:0` | note |
|---|---:|---|
| baseline (wave 1, `05a260c6f`) | **1427** | `coverage/2026-11/rust/tauri-lcov.info` |
| wave 5 (`963`) | 963 | preparation chain moved into the driver |
| wave 6 | **864** | the real update worker end to end (−99) |
| wave 7 | **819** | worker failure arms, `confirm`, `update_process` refusals (−45) |

The lane is **not** at per-line 0 and does not claim to be. What it does claim: every remaining
line in the files this table covers is either registered below or named as a work item in §4;
nothing in §2 is claimed as "covered".

| file | `DA:0` | disposition |
|---|---:|---|
| `src/commands/update_macos.rs` | 9 | 3 registered (§2.1) + 1 OPEN (§4) + 2 attribution (§2.4) — rest is the worker, now covered |
| `src/commands/update_process.rs` | 2 | 1 OPEN (§4.1) |
| `src/commands/update_journal.rs` | 41 | work items (§4.2) — `latest()`/`confirm_startup()` arms, reachable with fixtures |
| `src/commands/update.rs` | 194 | work items — the IPC wrappers this lane has not reached yet |
| `src/terminal/server.rs` | 56 | known ±1 fluctuation (§2.5) + interleaving-dependent test arms |
| `src/agent_supervisor.rs` | 88 | work items |
| other files | 429 | work items (see `classification.md` for the wave-1 bucket split) |

## 2. Registered waivers

Codes used by this lane (the other lanes' `W*` codes are listed in their own files):

- `T1` — **production-only entry**: the unit derives its input from `current_exe()` being inside
  a built `.app`. A test process is never that, and no seam may change the production path to
  make it so.
- `T4` — **attribution artifact**: the statement demonstrably ran (its neighbours read `DA>0`,
  and its observable effect is asserted elsewhere), but this line's own record stays `0`.
- `T5` — test code whose body only runs when an assertion fails.

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

## 3. Products of the coverage work (not waivers)

Two production seams were required and are gated, behaviour unchanged. Both are the same 1:1
split the coverage campaign approved for this lane (README §"已批准的三处 tauri 例外"):

| seam | why | gate |
|---|---|---|
| `probe` → `pub(super)` | its 10 s deadline can only be *expired* by a candidate that outlives it (+10 s, unaffordable beside 400 instrumented libtests) | `#[cfg(any(test, feature = "test-support"))]` facade in `commands::update` |
| `Prepared::prepare_with_helper` / `confirm_at` | the chain and the confirmation derive their bundle path from `current_exe()`; injecting it is what makes every step behind them observable | same |

## 4. Work items (NOT waivers — no evidence yet, so no row above)

### 4.1 `update_process.rs:288-289` — `OPEN`

`agent_lease`'s "another Agent started before installation" arm: `try_lock_exclusive()` fails
because a **live holder** exists. What is missing: a process that holds the flock while the
worker runs. `fs2` is a regular dependency of this crate but not a dev-dependency, so the driver
cannot lock it today; adding `fs2` to `[dev-dependencies]` (same version, already in the
lockfile) plus one driver scenario would close it. Deliberately not registered as a waiver: the
arm is reachable, it is simply not written yet.

### 4.2 `update_journal.rs` (41 lines)

`latest()`'s permission/`EAGAIN` arms (196-202), the "interrupted after shutdown began" branch
(212-217), and `confirm_startup`'s whole body (233-266: observer-lock races, `validate_startup`,
`cleanup_confirmed`). All reachable with fixtures modelled on the module's existing libtests —
work, not waivers.

### 4.3 The rest of the lane

`update.rs` (194) is the IPC wrapper layer this campaign has only started on; `lib.rs` (115),
`agent_supervisor.rs` (88) and `history_transfer.rs` (72) are next by size. See
`classification.md` for the wave-1 class split (R reachable / E error arms / P platform-bound /
C `cfg(not(test))` / A attribution / T test code) over the original 1427.

## 5. How the wave 6-7 numbers were produced and falsified

Every scenario added in these rounds was made red before it was trusted, and each failure named
the derived value it was built on (`coverage/2026-11/tauri/wave6.md` §4):

| scenario | falsification | result |
|---|---|---|
| worker install (driver) | worker dispatch disabled | `0/6`, first line is the child's own marker |
| probe timeout | fast candidate | `after 0.2s` instead of 10 s |
| persist failure | directory left readable | `ok: true` from a worker that succeeded |
| agent-lease failure | `agent` made a directory | `ok: true`, journal `pending-validation` |
| `confirm` | staging kept | "staging left behind" |

One check had to be rewritten because it passed for the wrong reason: the persist-failure
scenario first failed on a *missing record*, not on the directory permission (wave6.md §4.3).
