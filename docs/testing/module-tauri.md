# Module: `desktop-tauri` (`desktop/src-tauri`)

Status: **not at target**. Measured line coverage **94.2945 % (46771/49601)**
with **2830 uncovered lines in 104 files** (the gate's metric — see §2). This
document is a *progress ledger plus plan*, not a completed waiver ledger: rows
marked `OPEN` are explicitly **not waived** and the acceptance gate is expected
to fail on them. Only rows that name a waiver category have been checked
against the code.

Owner: `todo_78a3eaaa0a28` (goal `cov-100-multidim`). Write scope:
`desktop/src-tauri/`, `docs/testing/module-tauri.md`.

## 1. Task identity and the exact measurement

The crate is **not a workspace member** (`desktop/src-tauri/Cargo.toml` declares
its own empty `[workspace]`), so it is measured by its own run from inside the
directory.

```powershell
# (0) tauri-build aborts without the externalBin placeholders, and they are
#     gitignored build artifacts — absent in a fresh worktree.
make desktop-sidecar-placeholder            # creates binaries/future-<triple>.exe

cd desktop/src-tauri
$env:CARGO_TARGET_DIR = "target/cov-tauri"  # private dir: several workers share this checkout
cargo llvm-cov --json --output-path D:/future-os/.worktrees/cov100/coverage/llvm-cov-tauri.json
```

Gate (run from the goal cwd):

```powershell
python .future/cov100/verify.py crate desktop-tauri 99.99 coverage/llvm-cov-tauri.json
python .future/cov100/verify.py uncovered desktop-tauri coverage/llvm-cov-tauri.json
```

Toolchain notes that cost time to discover and are not obvious from the repo:

- `cargo llvm-cov --target-dir DIR` is **rejected** by cargo-llvm-cov 0.9.1
  (`error: invalid option '--target-dir'`). Set `CARGO_TARGET_DIR` instead.
- `.cargo/config.toml` sets `[target.x86_64-pc-windows-msvc] rustflags`; the
  instrumented build correctly carries both the config flags
  (`-C target-feature=+crt-static …`) and `-C instrument-coverage` — the two do
  not conflict.
- A run whose *first* build in a fresh target dir happens in `--no-report` mode
  produced a report with **zero** coverage (only build-script counters in the
  profdata). Prefer the plain `cargo llvm-cov` invocation, which builds, runs
  and reports in one pass; if you must split, sanity-check the resulting
  percentage before trusting it.

## 2. Measurement, before and after this segment

From the real `coverage/llvm-cov-tauri.json` (llvm-cov `DA:`/region records;
`verify.py` numbers, which are the ones the gate uses):

| | covered / measured | lines % | uncovered lines | files with uncovered lines |
|---|---|---|---|---|
| start of task (instrumented build, suite red) | 46290 / 49231 | 94.0261 | 2941 | — (round-1 report was supplanted) |
| after this segment | 46771 / 49601 | 94.2945 | 2830 | 104 |
| delta | +481 covered | +0.27 pt | −111 | |

`verify.py` prints two different uncovered counts, both computed from the same
report — know which one you are reading:

- `verify.py crate …` (and therefore the gate) uses
  `uncovered_count = summary.count - summary.covered`, summed over files:
  **2830 lines in 104 files**. This is the number the waiver gate reconciles.
- `verify.py uncovered …` prints `uncovered_lines()`, the count of region
  entry records with a zero counter: **3540 lines in 122 files**. Useful for
  locating dead arms, but *not* the gate's metric.

Test suite: **1285 passed, 0 failed, 3 ignored** (1281 lib + 3 bin + 1
integration). At the start of the task the same suite had **4 failures on
Windows** (§6.1).

Largest single-file movement: `src/remote_host/sync_measurement.rs`
**0/178 → 480/548 (87.6 %)**.

## 3. Files changed

| file | change |
|---|---|
| `src/remote_host/sync_measurement.rs` | new `#[cfg(test)] mod tests` — 13 tests over the probe handler |
| `src/terminal/cwd.rs` | `real_directory_validates_and_is_canonicalized` re-pointed at the real contract + verbatim assertion |
| `src/terminal/server.rs` | e2e: answer the shell's DSR query, send CR (not LF) for Enter, home-fallback assertion uses the ordinary spelling |
| `src/store/workspaces.rs` | `create_workspace_expands_a_tilde_path` compares against the stored (non-verbatim) spelling |
| `src/agent_bridge/tests.rs` | `reconcile_thread_workspace_variants` same |

New tests (all in `sync_measurement.rs`, all asserting behaviour, none
assertion-free):

`serves_the_probe_shell_bundle_and_samples`,
`an_unknown_route_is_plain_not_found`, `a_foreign_host_header_is_rejected`,
`a_client_that_disconnects_early_is_not_an_error`,
`an_oversized_header_block_is_rejected`,
`a_declared_body_over_the_bound_is_rejected`, `a_truncated_body_is_rejected`,
`storing_a_result_requires_the_probe_header`,
`a_probe_result_is_stored_verbatim`, `rpc_rejects_anything_but_the_read_commands`,
`rpc_rejects_a_command_outside_the_sample_scope`,
`rpc_rejects_a_malformed_body`, `the_capture_sink_records_the_reply_envelope`.

They are driven over a **real loopback socket** against the real `handle`, not
against a helper, because the guards being tested (same-origin, `host`, size
bounds, read-only command allowlist) live in the header/body parsing itself.

## 4. Dimension matrix rows for `desktop-tauri`

For merging into `docs/testing/dimension-matrix.md`. `N/A` is stated with a
reason, never invented.

| module | dimension | evidence |
|---|---|---|
| desktop-tauri | boundary | `sync_measurement`: header block > 16 KiB rejected, declared body > 256 KiB rejected before any body byte is read, body truncated below `content-length`, empty sample list, empty `run_id` (matches any run); `store::util`: missing path / plain file / aliased dir counted once; `terminal::ticket`: capacity prune at the expiry boundary |
| desktop-tauri | error-path | `sync_measurement`: foreign `host`, foreign `origin`, missing probe header, malformed JSON (serde position propagated, not swallowed), non-read-only command, command outside sample scope, EOF before headers; `terminal::cwd`: `Missing`/`NotADirectory`/`CWD_INVALID` codes; `terminal::server`: 401 without the process secret, 404/TICKET after removal |
| desktop-tauri | concurrency | `terminal::server::end_to_end`: real listener + real WebSocket + real shell process, session create → stream → resize → delete; single-use connect tickets (replay yields a different ticket); `store::util::unpoison` recovers a poisoned mutex; `count_files_under` canonical-dedup; `store` uses one `Mutex`-guarded connection under parallel libtest threads |
| desktop-tauri | property | `sync_measurement`: `Vec<Sample>` encode → socket → decode round-trip; `Capture` reply envelope invariants for both success and failure; `store::util::strip_verbatim_prefix` is an exact identity on ordinary POSIX paths (no-op property) |
| desktop-tauri | platform-cfg | Windows verbatim prefix: `strip_verbatim_prefix` spellings (`\\?\C:\…`, `\\?\UNC\…`), and three assertions that a stored/resolved path cwd **never** starts with `\\?\`; `terminal::server` e2e encodes the Windows shell contract (answer `ESC[6n`, CR for Enter); `terminal::shell::windows_tests` covers PATH/extension/known-location resolution; `store::util` has a `#[cfg(unix)]` symlink-walk test |
| desktop-tauri | serialization | `Sample` JSON round-trip through the probe; `IncomingCmd` camelCase decode with `#[serde(default)]`; `remote::commands::encode_reply_payload_with_gzip` used by the live `/rpc` path; store schema apply/migrate tests | 

## 5. Uncovered-line ledger

Counts below are the **gate's** metric (`uncovered_count`,
`summary.count - summary.covered`), so this table reconciles with
`verify.py crate desktop-tauri`. Category keywords: `unreachable-by-construction`,
`unreachable-in-this-environment`, `attribution-artifact`,
`platform-unmeasured`.

### 5.1 Waived (checked against the code)

| file | uncovered | category | reason |
|---|---|---|---|
| `desktop/src-tauri/src/windows_power.rs` (waived in §5.1 — listed for size only) | 42 | `unreachable-in-this-environment` | Win32 window-procedure subclassing. Every branch either calls a real `DefWindowProcW`/`CallWindowProcW` needing a live HWND from a real Tauri window, or mutates the process-global `remote::SUPERVISOR` (suspend cancels the generation) — calling it from a parallel libtest thread would corrupt every other remote test in the same process. |
| `desktop/src-tauri/src/remote_host/sync_measurement.rs` | 68 (≈28 of them the ignored harness) | `unreachable-in-this-environment` | The module is `#[cfg(test)]`-only. `serve_real_snapshot` is `#[ignore]`d by design: it needs an isolated DB snapshot (`MEASUREMENT_ISOLATED_HOME`), a real Agent gRPC endpoint and a real browser, driven by `scripts/measure-sync-browser.py`. The handler itself is covered by the 13 tests above; the rest of the uncovered lines are the live-store `/rpc` success path (reachable — listed under OPEN). |
| `desktop/src-tauri/src/macos_power.rs`, `desktop/src-tauri/src/menu.rs` | n/a (absent from the report) | `platform-unmeasured` | `#[cfg(all(feature = "gui", target_os = "macos"))]` — not compiled in a Windows measurement. |
| `desktop/src-tauri/src/linux_power.rs` | n/a (absent) | `platform-unmeasured` | `#[cfg(all(feature = "gui", target_os = "linux"))]`. |

### 5.2 OPEN — not waived, work item for the next segment

Ordered by uncovered line count (top 22 of 104 files, gate metric). Each of
these is believed reachable from a test on this platform; that belief is the
work, not the finding.

| file | uncovered |
|---|---|
| `desktop/src-tauri/src/lib.rs` | 235 |
| `desktop/src-tauri/src/remote/publisher/coalesce.rs` | 206 |
| `desktop/src-tauri/src/headless/mod.rs` | 203 |
| `desktop/src-tauri/src/remote/supervisor/start.rs` | 178 |
| `desktop/src-tauri/src/terminal/server.rs` | 155 |
| `desktop/src-tauri/src/agent_supervisor.rs` | 137 |
| `desktop/src-tauri/src/headless/agent.rs` | 112 |
| `desktop/src-tauri/src/terminal/session.rs` | 104 |
| `desktop/src-tauri/src/terminal/manager.rs` | 98 |
| `desktop/src-tauri/src/scheduler/mod.rs` | 95 |
| `desktop/src-tauri/src/commands/runs.rs` | 69 |
| `desktop/src-tauri/src/remote_host/sync_measurement.rs` (live `/rpc` remainder; ignored harness waived in §5.1) | 68 |
| `desktop/src-tauri/src/commands/update.rs` | 63 |
| `desktop/src-tauri/src/remote/commands.rs` | 45 |
| `desktop/src-tauri/src/windows_power.rs` | 42 |
| `desktop/src-tauri/src/commands/files.rs` | 42 |
| `desktop/src-tauri/src/remote/secure.rs` | 41 |
| `desktop/src-tauri/src/terminal/cwd.rs` | 38 |
| `desktop/src-tauri/src/remote_host/business/settings.rs` | 38 |
| `desktop/src-tauri/src/shadow_review/snapshot.rs` | 36 |
| `desktop/src-tauri/src/remote/transport.rs` | 35 |
| `desktop/src-tauri/src/commands/skills.rs` | 34 |

The remaining ~82 files carry 1–32 uncovered lines each; the gate run prints
the full list of what it considers unwaiwed. Rough shape of the long tail:
`agent_bridge/*` observer and query arms, `remote/*` transport/secure error
arms, `commands/*` Tauri IPC argument-validation arms, `store/*` migration and
cascade arms.

Of the entries above, only `windows_power.rs` and the ignored-harness part of
`sync_measurement.rs` are *waived* (§5.1). Everything else in this table is
open work.

### 5.3 Blind spot: 18 source files are absent from the Windows measurement

`llvm-cov` can only report what it compiled, so a local 100 % would only ever
prove the *current* platform and feature set. Files under `desktop/src-tauri`
that appear in no report at all:

| file(s) | why absent | category |
|---|---|---|
| `src/linux_power.rs`, `src/macos_power.rs`, `src/menu.rs` | `cfg(all(feature = "gui", target_os = …))` | `platform-unmeasured` |
| `src/no_gui.rs` | `#[cfg(not(feature = "gui"))]` | feature-unmeasured — the measurement is default features (`gui`) |
| `src/bin/futureos-headless.rs` | `required-features = ["headless"]` | feature-unmeasured |
| `src/agent_bridge/tests.rs`, `src/agent_providers/tests.rs`, `src/remote/tests.rs` | test-only modules; the report contains no `tests.rs` file | not measured (test code) |
| `src/agent_bridge/mod.rs`, `src/commands/mod.rs`, `src/terminal/mod.rs`, `src/remote/services.rs` | declarations/trait signatures only — no executable regions | `unreachable-by-construction` |
| `src/store/schema.rs` (395 lines), `src/store/records.rs` (278 lines), `src/agent_proto.rs`, `src/shadow_review.rs` | SQL/macro/`include!` constants, 0 `fn` | `unreachable-by-construction` |
| `build.rs` | build script, outside the crate's coverage mapping | `unreachable-by-construction` |

**Consequence for the goal:** `desktop/src-tauri` has two build configurations
and only the `gui` one has been measured. The headless/non-GUI configuration
(`cargo llvm-cov --no-default-features`, as CI's desktop-backend test job runs)
needs its own run before any claim of "100 %" for this module is meaningful.

## 6. Findings

### 6.1 The suite was red on Windows — 4 failures, all fixed

`cargo llvm-cov` on Windows reported `1281 passed; 4 failed`. CI runs
`desktop/src-tauri` tests **only on `ubuntu-latest`** (`Rust tests (Linux)`
matrix); the Windows job runs `cargo check` only. These failures had therefore
never been seen.

| test | verdict | fix |
|---|---|---|
| `store::workspaces::tests::create_workspace_expands_a_tilde_path` | **test wrong** | asserted `Path::canonicalize`, which on Windows is the extended-length `\\?\C:\…` spelling; the store deliberately stores `strip_verbatim_prefix(canonicalize(..))`. Now asserts the stored spelling *and* that no `\\?\` leaks. |
| `terminal::cwd::tests::real_directory_validates_and_is_canonicalized` | **test wrong** | same; `validate_dir` strips the prefix on purpose because cmd.exe/PowerShell cannot resolve relative paths from a verbatim cwd. |
| `agent_bridge::tests::pipeline_tests::reconcile_thread_workspace_variants` | **test wrong** | same, for the workspace row created by `reconcile_thread_workspace`. |
| `terminal::server::end_to_end::a_real_shell_streams_over_the_loopback_transport` | **test wrong** | the client never behaved like a terminal: it ignored the shell's Device Status Report (`ESC[6n`), so PSReadLine blocked before drawing a prompt, and it sent LF for Enter where a console expects CR, so the command sat in the buffer as a `>>` continuation line. The test now answers the DSR and sends CR. |

None of the four needed a production change, and no existing assertion was
weakened — the three path tests gained an assertion. The lesson worth carrying:
on Windows the crate's canonical-path contract is "ordinary spelling", and any
new test that compares against a raw `canonicalize()` will be red here.

### 6.2 Checked and *not* a bug

`git_review::canonical_or_raw` (and its test at `src/git_review.rs:669`) does
**not** strip the verbatim prefix, unlike `store::util::strip_verbatim_prefix`.
Inspected: every caller (`git_review.rs:235,263-264`; `agent_bridge/persist.rs:382-383`)
uses the result as a *comparison key* with both sides passed through the same
function, so the two spellings cancel and nothing user-visible is affected.
Left alone deliberately; recorded here so the next reader does not "fix" it.

## 7. Known gaps and what to check next

1. Write the `OPEN` ledger above into real work: the tail is 2830 uncovered
   lines across 104 files (gate metric) and needs a file-by-file campaign, not
   a waive sweep.
2. Measure the `--no-default-features` (headless) configuration — the current
   number says nothing about it, and `no_gui.rs` plus
   `bin/futureos-headless.rs` are invisible here.
3. `desktop/src-tauri` has no CI test job on Windows; the four fixes in §6.1 are
   unprotected against regression there. Either wire the crate's tests into a
   Windows job, or add these four to `WINDOWS_RED_BASELINE` in
   `.future/cov100/verify.py` so `verify.py windows` re-runs them forever. That
   list currently covers only `future-channel` and `future-tui`, even though
   `desktop/src-tauri` produced four of its own Windows-red tests.
4. The 3 `#[ignore]`d tests and the `weak`/`skipped` audits for this module are
   reported in the shared `docs/testing/disabled-test-audit.md` /
   `weak-test-audit.md`, which are outside this task's write scope; this
   module's only ignored test is the documented `serve_real_snapshot`.
5. `verify.py blindspot desktop-tauri` cannot see a per-module report: it loads
   `coverage/llvm-cov-full.json` unconditionally instead of the report passed to
   the other subcommands, so it reported all 160 source files as "absent". The
   blind-spot table in §5.3 was computed against
   `coverage/llvm-cov-tauri.json` by hand.
