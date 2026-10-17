# future-cli `mcp` — coverage, waivers and test-quality record

Goal `cov-100-multidim` (round 2) · module **future-cli**, the MCP surface only:
`cli/src/commands/mcp_servers.rs` and `cli/src/commands/mcp_client.rs` · branch
`test/cov100-r2` (worktree `.worktrees/cov100-r2`) · host **macOS aarch64**,
rustc `1.97.0` pinned by `rust-toolchain.toml` · run 2026-10-16 (final
measurement 18:56) · session
`20261016-175258-ff156a127c5e4557aedbc91f1d9ee6ce` · baseline revision
`8449acf6`.

**Scope note.** This record covers those two files and nothing else.
[docs/testing/module-cli.md](module-cli.md) covers the rest of the crate (it is
the round-1 record, measured on a Windows host at revision
`20260926-013544`; its numbers are not comparable with these). A peer worker
owns every other file under `cli/` and is editing them concurrently, including
`cli/src/commands/mcp.rs` (the *remote* MCP helper — a different thing) and
`cli/tests/`.

**Acceptance contract — the three items and their evidence:**

| item | how it is satisfied | where |
|---|---|---|
| `zero-uncovered-lines` | every uncovered line is a *result* under the campaign's 口径 A: **0** lines are left unclassified. 4 remain and each is registered here with a category **and** a mechanism — three are `attribution-artifact` implicit-else spans with their covered neighbours named, one is `unreachable-by-construction` with its invariant. The other 256 of the 260 baseline lines are now executed by a test (the four converted in this turn are listed in §6.2). | §2, §6 |
| `cargo-test-cli-green` | `cargo llvm-cov -p future-cli --no-fail-fast --lcov … -j 4` → **exit 0** at the original measurement (`1159 passed; 0 failed` lib, plus 2/16/2/1), and green at the **S12 re-derivation** too (`1166 passed; 0 failed` lib, plus 2/24/2/1) — no test target failed, so the command produced its own report. At S12 a peer's signal killed the invocation *during report generation* after the suite had finished green, so that report was rendered from the run's own profraw (`cargo llvm-cov report`, exit 0, §1). | §1, §2 |
| `module-cli-mcp-doc-updated` | this file: run identity + exact command (§1), before/after counts (§2), one row per residual line with category and mechanism (§6), and the scope hand-off to `module-cli.md` (this note). | this file |

## 1. Measurement (exact commands)

```sh
cd /Users/geilige/future-os/.worktrees/cov100-r2
ulimit -n 10240
export PATH=/Users/geilige/.rustup/toolchains/1.97.0-aarch64-apple-darwin/bin:$PATH
export CARGO_HOME=/Users/geilige/.cargo RUSTUP_HOME=/Users/geilige/.rustup
export HOME=$PWD/target/test-home          # the real ~/.future must not leak in
export CARGO_TARGET_DIR=target/cov-cli-a  # private per worker, never shared
mkdir -p "$HOME"

# one atomic invocation: build + run every test target + emit the per-line locator
cargo llvm-cov -p future-cli --no-fail-fast --lcov --output-path /tmp/clamcp-final-lcov.info -j 4

python3 .future/cov100/covlines.py files  /tmp/clamcp-final-lcov.info | grep mcp
python3 .future/cov100/covlines.py total  /tmp/clamcp-final-lcov.info \
        --under .worktrees/cov100-r2/cli/src/commands/mcp_servers.rs
python3 .future/cov100/covlines.py total  /tmp/clamcp-final-lcov.info \
        --under .worktrees/cov100-r2/cli/src/commands/mcp_client.rs
python3 .future/cov100/covlines.py detail /tmp/clamcp-final-lcov.info \
        --under .worktrees/cov100-r2/cli/src/commands/mcp_servers.rs \
        --under .worktrees/cov100-r2/cli/src/commands/mcp_client.rs
```

`--under` needs the worktree-relative prefix (`covlines.py` strips only
`/Users/geilige/future-os/`), otherwise it silently selects nothing and reports
`covered=0 measured=0` — a `total` of zero is a path bug, not a full file.
`covlines.py` now *fails* on that shape (`exit 3`, plus 3 on an
all-zero counter report), so a nonzero tool exit means the number is invalid
whatever it printed; the final run here exits **0** on both scopes.

### Re-derivation at S12 (REWORK-CLI, 2026-10-17)

`review-cli.md` §6.2 proved this record's `mcp_client.rs` row **stale-paired**:
the row pins the md5 of the revision it measured (`6f2a1ba6…` = `fd018191`), and
`08108be6` has since rewritten exactly the construct it names. The two files were
therefore re-measured at the revision this record now describes — `cli/src` and
`cli/tests` as committed in **`7197e39fe`**, host macOS/aarch64, rustc 1.97.0 —
with the module record's own command, into a private target directory:

```sh
cd /Users/geilige/future-os/.worktrees/cov100-r2
ulimit -n 10240
export PATH=/Users/geilige/.rustup/toolchains/1.97.0-aarch64-apple-darwin/bin:$PATH
export CARGO_HOME=/Users/geilige/.cargo RUSTUP_HOME=/Users/geilige/.rustup
export HOME=$PWD/target/test-home
export CARGO_TARGET_DIR=target/cov-cli-r3
mkdir -p "$HOME"
find target/cov-cli-r3 -name '*.profraw' -delete
cargo llvm-cov -p future-cli --no-fail-fast --lcov \
    --output-path /tmp/cli-r2s4-final-lcov.info -j 4
# the invocation was killed by a peer's signal AFTER the suite finished green, so
# the report was rendered from that run's own profraw (the fallback above):
cargo llvm-cov report --lcov --output-path /tmp/cli-r2s4-final-lcov.info

python3 .future/cov100/covlines.py total  /tmp/cli-r2s4-final-lcov.info \
        --under .worktrees/cov100-r2/cli/src/commands/mcp_servers.rs   # rc 0
python3 .future/cov100/covlines.py total  /tmp/cli-r2s4-final-lcov.info \
        --under .worktrees/cov100-r2/cli/src/commands/mcp_client.rs    # rc 0
python3 .future/cov100/covlines.py detail /tmp/cli-r2s4-final-lcov.info \
        --under .worktrees/cov100-r2/cli/src/commands/mcp_servers.rs \
        --under .worktrees/cov100-r2/cli/src/commands/mcp_client.rs    # rc 0
```

Suite state of that run: `future-cli` lib **1166** passed / 0 failed, main 2,
`cli/tests/bin.rs` 24, `event_wire_contract.rs` 2, `version_discovery.rs` 1 —
the same four residual lines, on the revision this record now pins.

Every measurement in this record is from the single atomic command above. One
earlier run of it had a *peer-owned* timing test fail, in which case
`cargo llvm-cov` writes no report while the profraw is still on disk:

```sh
cargo llvm-cov report --lcov --output-path /tmp/<out>.info
```

The final run was clean (`exit 0`), so no fallback was needed for the numbers
below. Seen failures of that class are recorded in §7.2.

Metric: per-line LCOV `DA:<n>,0` (口径 A). The `summary` block is never read.
Baseline for both files: `.future/cov100/baseline/ws-dirty-lcov.info`
(revision `8449acf6`; `git diff 8449acf6 -- <these two files>` is empty, so the
baseline line numbers are this file's line numbers).

## 2. Result

| file | uncovered before | measured before | lines % before | uncovered after | measured after | lines % after |
|---|---|---|---|---|---|---|
| `cli/src/commands/mcp_servers.rs` | 222 | 1 282 | 82.6833 % | **3** | 2 043 | **99.8532 %** |
| `cli/src/commands/mcp_client.rs` | 38 | 423 | 91.0165 % | **1** | 667 | **99.8501 %** |
| **total** | **260** | 1 705 | 84.7507 % | **4** | 2 710 | **99.8524 %** |

Denominator growth (+1 002 lines) is the new test code in the same two files: the
instrumented unit is the file, tests included. The "after" numbers and the line
numbers in §6 are from **`/tmp/cli-r2s4-final-lcov.info`**, the S12 measurement of
§1 whose two source files have md5 `05c5e3ccb3d12f7caf555a0a24168218`
(`mcp_servers.rs`) and **`005636fdb91e23d1d0e9292359e191ad`** (`mcp_client.rs`) —
the content this record describes. The original measurement pinned
`6f2a1ba6e217a2ab2038d6ea4f6c4cb6` for `mcp_client.rs`; `08108be6` rewrote part of
that file (23 lines) afterwards, which is what `review-cli.md` §6.2 caught: the
row cited a construct that no longer existed. The counts are unchanged (still
exactly one residual line in that file) — what changed is which line it is, and
the row now names the construct as it stands (§6). Both files are rustfmt-clean (`rustfmt --edition 2021 --check`,
run file-by-file — never `cargo fmt -p future-cli`, which rewrites peers'), and
`cargo clippy -p future-cli --all-targets -- -D warnings` exits **0**.

Every one of the 4 residual lines is classified in §6 — there is no unaccounted
line, and none of them is a production guard that was removed or weakened (§5).
Three are closing braces carrying a dead implicit-else/`?`-gap region, and one is
a `continue;` whose lookup cannot fail.

Suite state — this is `cargo-test-cli-green` for the crate, from the same atomic
run that produced the numbers (no `--skip`, no `#[ignore]`, no filter):

| target | tests | result |
|---|---|---|
| `future-cli` lib (this record's 42 + the rest of the crate) | 1 159 passed | **0 failed** |
| `future-cli` `bin.rs` / `event_wire_contract.rs` / main / version-discovery | 16 / 2 / 2 / 1 | ok |

**At the S12 re-derivation** (§1) the same row reads: lib **1 166** passed / 0
failed, and `bin.rs` **24** / `event_wire_contract.rs` 2 / main 2 /
version-discovery 1 — the crate grew by seven tests between the two measurements,
none of them in these two files.
| the whole command | `cargo llvm-cov … -j 4` | **exit 0**, report written by the run itself |

The fast scope check is `cargo llvm-cov -p future-cli --no-report -j 4 -- mcp` →
`97 passed; 0 failed` (96 before this turn's extra test).

## 3. The tests added (42), and what each group closes

All tests live **inline** in the two files (the dominant style for this crate's
commands), so the two files stay the whole unit of this record.

### 3.1 `cli/src/commands/mcp_servers.rs` — 31 tests

**Configuration file** (was 5 lines of `load_config`/`save_config`/`resolve`
plus every `add`/`remove`/`list` arm):

* `config_reads_an_absent_blank_and_unreadable_file` — the four states of
  `~/.future/mcp.json`: absent, whitespace-only, not JSON (names the path), and
  path that cannot be read as a file (a directory) → `could not read`.
* `save_config_creates_its_parent_and_reports_its_failures` — the parent
  directory is created when missing and renamed into place with no
  `mcp.json.tmp` left behind; a parent that cannot be created (a regular file in
  the way) reports `could not create`; a destination that cannot be replaced (a
  directory sits at `mcp.json`) reports `could not replace` **and cleans the
  temp file up**.
* `resolve_reports_a_disabled_server_and_names_the_alternatives` — the three
  failure texts: `'off' is disabled`, `unknown … configured: live, off`, and
  `none are configured` when the file is empty.
* `list_rejects_an_extra_argument_and_prints_the_document_on_request` — the
  `--json` document, field by field (`configPath`, `args` verbatim, `cwd`,
  `enabled` for both an enabled and a disabled server).
* `add_stores_the_arguments_environment_and_working_directory` — the written
  file is the contract (`--arg --mcp` kept dash and all, `--env KEY=VALUE` as
  strings, `--env EMPTY=` as the empty string, `--cwd`), the confirmation text,
  and the duplicate-name refusal that leaves the existing entry untouched.
* `add_rejects_an_unusable_name_and_a_malformed_environment_pair` — a name with
  a space, `--env TOKEN` (no `=`), `--env =secret` (empty key); nothing is
  written by any of the three.
* `remove_reports_an_unknown_server_and_deletes_a_known_one`.

**Argument parsing / dispatch** (was ~10 lines): `a_switch_with_a_value_is_rejected`
(`--json=1`), `the_timeout_and_frame_cap_boundaries_are_enforced` (a positive
`--timeout`, `0` refused; `--max-frame-bytes 0` refused, a real cap returned),
`a_missing_or_extra_positional_reports_the_usage` (both arities, exact text),
`an_unknown_subcommand_is_rejected_with_its_name` (`mcp("bogus")` — the arm
`lib.rs` cannot reach, because it gates on `is_mcp_command`),
`tools_and_call_reject_an_unknown_option_before_reading_any_config` (the
`?`-error path of `parse_flags` in both commands),
`call_validates_its_names_and_its_args_flag` (arity 1 and 3, `--args` that is
not JSON, `--args` that is JSON but not an object),
`status_takes_at_most_one_server_name`.

**Rendering helpers**: `content_blocks_without_a_detail_are_named` (an image
with neither `mimeType` nor `uri`; a block with no `type`; a `uri`-named
resource), `stderr_is_attached_to_a_failure_only_when_the_server_wrote_some`.

**Against a real spawned server** (unix; `/bin/sh` as an MCP server over real
pipes):

* `status_json_reports_every_server_with_its_state` — the `--json` form: one
  record per server in configuration order, `ok` with `toolCount`/`handshakeMs`/
  `totalMs`, and a `disabled` record with no `error` key.
* `status_reports_a_server_that_fails_after_a_successful_handshake` — the arm
  that made `status` a command rather than a list: the handshake succeeds, the
  server then stops answering, the verdict is `FAILED` with the server's own
  stderr (`JAVA_HOME is not set`) attached, and the command exits 1.
* `tools_filters_by_query_and_reports_the_totals` — `--query` matching the name
  and, separately, only the description; the header's `(n of total)`.
* `tools_json_page_carries_the_progress_through_the_list` — the `--json` page:
  `total`/`offset`/`limit`/`tools`/`nextOffset`, including the explicit `null`
  on the last page.
* `tools_names_the_next_offset_and_the_offset_that_is_past_the_end`.
* `describe_prints_the_description_and_the_schema`,
  `describe_json_prints_the_tool_verbatim`,
  `describe_requires_two_names_and_suggests_alternatives` (`describe` had **no**
  test at all: 41 production lines). Both arities, the "list what it has"
  fallback when nothing resembles the typo, and `Did you mean: full` when
  something does.
* `call_passes_a_json_args_object_to_the_tool` (over a server that echoes
  `path` back, proving the object reached the wire),
  `call_json_prints_the_raw_result_envelope`,
  `call_writes_a_short_result_to_out_and_says_where_it_went` (the `--out` write
  of a result that fits, plus the header line, plus the caller-visible file),
  `call_truncates_a_large_result_and_names_the_rerun_flag`,
  `call_keeps_the_servers_error_when_the_tool_is_in_the_list` (a protocol-level
  failure for a tool that *is* listed keeps the server's own words instead of
  being rewritten as a typo).
* `serve_drives_a_real_stdin_session` — `serve` reads the process's own stdin,
  which no in-process test can replace, so this re-runs **this same instrumented
  test binary** as a child with a piped stdin (the technique
  `commands::tools` uses for the same shape; the child's coverage merges with
  the parent's). Three children, one per scenario, each asserting on the child's
  real stdout/stderr:
  * *serves*: the ready ack (`{ok,ready,server,toolCount}`) first, then one
    answer per request, a tool-reported error preserved as `isError`, and
    exactly **one** `initialize` in `$MCP_START_LOG` for two calls — one cold
    start, which is the whole point of `serve`; the server's diagnostics appear
    on **stderr**, so stdout stays a clean JSON stream.
  * *fails*: the server answers `initialize` and then stops answering, so the
    ready check fails after a successful handshake and the server's last words
    are the error (the command's own failure path, on real stdin).
  * *shutdown*: a **broken** stdin — a directory, opened as a file, so its reads
    fail rather than reaching EOF — through the production `mcp_serve` wrapper:
    the request loop reports `stdin: <error>` and ends, and the session still
    closes with the diagnostics on stderr.
* `serve_reports_a_stdin_that_fails_to_read` — the loop's `Err` arm with a
  reader that fails at the first read (`FailingReader`), asserting the reported
  line and that the loop stopped rather than spinning.

### 3.2 `cli/src/commands/mcp_client.rs` — 11 tests

* `a_silent_server_times_out_with_the_budget_it_was_given` — a channel that
  stays open and never answers: the request fails at its own deadline and the
  message names `--timeout`. (The fixture holds the channel open with a read
  loop rather than a `sleep`, so the test can hang up and *await the task's end* —
  see §6.2.)
* `list_tools_gives_up_when_the_pages_never_end` — a server that answers every
  page with another cursor: `200` pages then an error naming the cap and the
  cause (`nextCursor`), instead of spinning.
* `an_empty_frame_is_skipped_and_an_unreadable_frame_is_reported` — a bare
  newline before the answer is not the answer; a frame that is not JSON is an
  error naming the method being awaited.
* `every_error_code_shape_is_described` — `describe_error` for a numeric code, a
  string code, no code, no message, with `data`, and with `data: null`.
* `call_tool_sends_the_name_and_arguments_it_was_given` — the `tools/call` arm
  of the in-memory fake plus the wire shape it asserts (`params.name`,
  `params.arguments`).
* `every_json_kind_is_named_in_the_environment_error` + 
  `a_non_string_environment_value_is_refused_before_spawning` — the six
  `json_kind` arms and the configuration error itself, and that it is raised
  **before** anything is spawned.
* `the_configured_environment_and_working_directory_reach_the_child` — a real
  child that records `$MCP_PROBE` and `pwd -P` to a file: the spec's `env` and
  `cwd` reach the process (`env` had never been exercised; `cwd` was never set).
* `shutdown_kills_a_server_that_ignores_its_closed_stdin` — a server that serves
  one handshake and then `sleep 30`: closing stdin is not enough, so the call
  waits out `SHUTDOWN_GRACE` and then kills it (asserted as a bounded window, not
  a sleep).
* `the_stderr_tail_keeps_the_last_lines_and_drops_the_oldest` — 45 stderr lines
  through a real pipe: the last 40 are kept, `stderr-06` is the oldest survivor
  and `stderr-01` is gone.
* `a_poisoned_stderr_tail_is_tolerated_by_the_drain_and_the_reader` — the
  `drain_stderr` arm that a *lock failure* takes. Nothing in this crate panics
  while holding that lock, so the state is produced deliberately: a thread holds
  the guard and unwinds, then the server is driven on (it writes one stderr line
  per `tools/call`) so lines keep arriving *after* the poison. Asserted: the tail
  kept the pre-poison line, three further calls succeed, and the caller-visible
  tail reads as empty rather than panicking (which is what a `future mcp status`
  would see). This is the arm the earlier measurement had waived — §6.2.

## 4. Dimensions (matrix rows for the MCP surface)

| dimension | evidence |
|---|---|
| boundary | `the_timeout_and_frame_cap_boundaries_are_enforced` (0 vs 1 second, 0 vs 1 byte); `tools_json_page_carries_the_progress_through_the_list` (`--limit 1` of 2, last page); `tools_names_the_next_offset_and_the_offset_that_is_past_the_end` (offset 9 of 2); `the_stderr_tail_keeps_the_last_lines_and_drops_the_oldest` (40 vs 45 lines); `a_missing_or_extra_positional_reports_the_usage` |
| error-path | `resolve_reports_a_disabled_server_and_names_the_alternatives`; `add_*`/`remove_*` refusals; `call_validates_its_names_and_its_args_flag`; `status_reports_a_server_that_fails_after_a_successful_handshake`; `call_keeps_the_servers_error_when_the_tool_is_in_the_list`; `serve_reports_a_stdin_that_fails_to_read`; the child-process `fails` and `shutdown` scenarios |
| concurrency | `serve_drives_a_real_stdin_session` (one long-lived session answering many requests in order, one cold start); `shutdown_kills_a_server_that_ignores_its_closed_stdin` (the grace period as a bound, not a sleep); `the_stderr_tail_…` (a background drain observed through a bounded poll) |
| property | `every_error_code_shape_is_described`; `every_json_kind_is_named_in_the_environment_error`; `suggest_tools`' rarity ordering (existing tests); `content_blocks_without_a_detail_are_named`; "a refused add/remove changes nothing" |
| platform-cfg | the process tests are `#[cfg(unix)]` by construction (they spawn `/bin/sh` as the server); they run here and on the POSIX CI leg — see §6 for what that means for a Windows run |
| serialization | the `mcp.json` round trip through `add`/`list --json`/`remove`; `status --json`; `tools --json`; `--args` reaching the server as an object; the MCP frame cap (`--max-frame-bytes`, `FUTURE_MCP_MAX_FRAME_BYTES`) |

## 5. Test-quality notes

* **No assertion was weakened and no production guard was removed.** `git diff`
  against `8449acf6` for these two files is additive except for one signature
  (below).
* **One production change, with its reason:** `mcp_serve` grew a reader
  parameter and became a two-line wrapper:

  ```rust
  async fn mcp_serve(args: &[String], out: &Output) -> Result<(), String> {
      mcp_serve_with_stdin(args, out, tokio::io::BufReader::new(tokio::io::stdin())).await
  }
  async fn mcp_serve_with_stdin<R: tokio::io::AsyncBufRead + Unpin>(args, out, reader: R) { … }
  ```

  The body is byte-for-byte the old body; production still passes the process's
  own stdin. It exists because the command's *own* paths — a stdin whose reads
  fail, and the diagnostics-and-shutdown after the request loop — are otherwise
  provably reachable but unreachable **by any test**: fd 0 belongs to the whole
  test binary. It is the same split the file already documents for
  `serve_requests`. No behaviour changed for a caller.
* Two `match … Ok(_) => panic!(…)` arms written during this segment were
  replaced by the idiomatic `.err().expect("…")` (identical assertion, no
  `Debug` bound on the `Ok` type, no never-executing line).
* **The fixtures now end.** The three test-side mock tasks used to be cancelled by
  the runtime mid-await, which left their span ends unexecuted; each is now
  awaited after the session is dropped, so the loop's end *is* executed. That is
  the fix the round-1 record asked for in `module-cli.md` §7.5 ("letting those
  mock loops terminate instead of aborting — it is cosmetic for coverage and real
  for hygiene"). The waits are bounded (5 s), so a task that never ends fails the
  test instead of hanging the suite.
* **An assertion message is not a test.** The poison test's first version passed
  `process.stderr_tail()` as a `{:?}` argument to `assert!`, which made line 1068
  a line no passing run can execute; it now asserts a `bool` and formats nothing.
  That is the same class as `module-cli.md` §6.1's `auth.rs:988` rows — fixed
  here rather than waived, because it is this record's own test.
* **`clippy -D warnings` is clean for the crate** (`cargo clippy -p future-cli
  --all-targets -- -D warnings` → exit 0). Getting there needed a real change,
  not an `#[allow]`: three tests held a `std::sync::MutexGuard` on the fixture's
  request log in the same scope as an `await`, and `clippy::await_holding_lock`
  flags that shape even when an explicit `drop` precedes the `await` (the lint is
  syntactic and does not see the `drop`). Those assertions now run against a
  `Vec` snapshot taken by a helper (`seen_messages`), so no guard can be live at
  an `await`. The assertions themselves are unchanged — the log is still checked
  entry by entry.
* 42 new tests; each asserts the behaviour of the lines it executes. The
  child-process test asserts on the **child's** output (the ack, each answer,
  the one cold start, the stderr hand-off), not on the parent's.
* The MCP surface's weak-test detector state: the new tests use `assert!`/
  `assert_eq!` on captured output, table entries or the file written, so no new
  entry is expected from `verify.py weak`.

## 6. Waiver ledger — the 4 residual lines

Each row is `path:line — category — mechanism`. The categories and their
requirements are [docs/testing/waiver-ledger.md](waiver-ledger.md)'s. These four
are the *only* uncovered lines in the two files: `covlines.py total` exits 0 on
both scopes and `detail` lists exactly these numbers.

| path:line | category | mechanism |
|---|---|---|
| `cli/src/commands/mcp_servers.rs:85` | `attribution-artifact` | The `}` closing `if let Some(parent) = path.parent() { … ?; }` in `save_config`: a zero-count region for the block's dead remainder (the body ends in `?`, which diverges) and for the `if let`'s implicit else. No input reaches it: `config_path()` is always `<FUTURE_HOME>/mcp.json`, which always has a parent. Evidence: 83-84 (the `create_dir_all` + `map_err`) are covered from both directions by `save_config_creates_its_parent_and_reports_its_failures` (a created directory, and `could not create`), and 86 onward is covered. At S12 the same neighbours read 83 = 9, 84 = 9, 86 = 8. |
| `cli/src/commands/mcp_servers.rs:412` | `attribution-artifact` | The `}` closing `if let Some(error) = report["error"].as_str()` inside the `_ =>` arm of `status`'s text report — the `if let`'s implicit else. Not reachable: every report whose `state` is neither `ok` nor `disabled` is pushed with an `error` key (both failure constructors set it), so the else never runs. Evidence: 407-411 are covered by `status_reports_a_server_that_fails_after_a_successful_handshake`, which asserts the `FAILED` line **and** the indented stderr line; 413-414 (the arm's and the `match`'s closing braces) are covered too, which is what pins the zero region to the `if let` rather than to the arm. At S12: 410 = 7, 411 = 7, 413 no record. |
| `cli/src/commands/mcp_client.rs:406` | `attribution-artifact` | The `}` closing the **new** `if let Some(drain) = stderr_drain` in `ServerProcess::start_with_frame_cap` — the `if let`'s implicit else, in the same place the old `if let Some(stderr) = child.stderr.take()` used to be. Not reachable: the handle is built by `child.stderr.take().map(\|stderr\| tokio::spawn(drain_stderr(stderr, tail)))` a few lines earlier, where `stderr(Stdio::piped())` is set unconditionally, so `drain` is always `Some`; the projection is `.map()` rather than `if let` because `08108be6` rewrote it (the reason this row is re-derived rather than re-pointed — `review-cli.md` §6.2: at the pinned md5, `:385` *was* that `if let`'s `}`; the residual moved one construct down). Evidence at S12: 404 = 4, 405 = 4, 407 = 4 — the clone, the `tokio::spawn(drain_stderr…)` and the statement after the block are all covered by every process test, and the tail they fill is asserted by `the_stderr_tail_keeps_the_last_lines_and_drops_the_oldest` and by the poisoned-tail test. |
| `cli/src/commands/mcp_servers.rs:342` | `unreachable-by-construction` | The `continue;` of `let Some(spec) = config.servers.get(name) else { … }` in `status`. Invariant: `targets` is built either from a single name that `resolve(&config, name)?` has already proved present, or from `config.servers.keys()` — in both cases every element is a key of `config.servers`, so the lookup cannot fail and the `else` block is dead. Mechanism: the two construction sites, both covered. It stays as the defensive half of a `let-else`; deleting it would make a line *stop existing*, which is what this ledger forbids — so it is registered instead. At S12 the same numbers read 341 = 9 (the `else` block is 342 = 0). |

### 6.1 Re-derivation at S12, and what the adversarial review changed here

`review-cli.md` reviewed these four rows (it is the fifth record's peer): **three
SURVIVED** — `mcp_servers.rs:85`, `:342`, `:412`, each with its neighbour counts
re-checked against the report the record pins — and **one was stale-paired**:
`mcp_client.rs:385`. That row is not re-pointed but **re-derived** (§6, first
row of the table above): `08108be6` rewrote the construct it named into a
`.map()`, so the residual is the `if let` one construct later, at `:406`, and
this record's md5 pin was for content HEAD no longer holds (`6f2a1ba6…` →
`005636fd…`, §2). `mcp_servers.rs`'s md5 is unchanged (`05c5e3cc…`), which is why
its three rows and their line numbers did not move.

Both self-checks are clean at S12:

```sh
$ python3 .future/cov100/covlines.py total /tmp/cli-r2s4-final-lcov.info \
      --under .worktrees/cov100-r2/cli/src/commands/mcp_servers.rs      # rc 0
covered=2040 measured=2043 uncovered=3 lines%=99.8532
$ python3 .future/cov100/covlines.py total /tmp/cli-r2s4-final-lcov.info \
      --under .worktrees/cov100-r2/cli/src/commands/mcp_client.rs       # rc 0
covered=666 measured=667 uncovered=1 lines%=99.8501
$ python3 .future/cov100/reconcile.py --report /tmp/cli-r2s4-final-lcov.info \
      --scope .worktrees/cov100-r2/cli/src/commands/mcp_client.rs \
      --scope .worktrees/cov100-r2/cli/src/commands/mcp_servers.rs \
      --doc docs/testing/module-cli-mcp.md --section "Waiver ledger" --label cli-mcp-s12
[cli-mcp-s12] files with uncovered lines=2 uncovered lines=4
  mentioned in module-cli-mcp.md: 4
  MISSING (uncovered, not mentioned): 0            # and no STALE block
```

The STALE half is *live* since the supervisor's `5da793b6` fix (it used to skip
a file whose uncovered set had become empty — the false clean AG-A reported), so
its silence here is a statement about the pairing, not a blind spot: both files
are measured by this report and every `path:line` row above matches the report's
uncovered set exactly.

### 6.2 Four lines that were waived last time and are executed now

Each of these was a row in the previous version of this ledger. They are gone
from it because a test executes them, not because a row was re-worded:

| was | now | what changed |
|---|---|---|
| `mcp_client.rs:525` — span end of the in-memory fake's task | covered | every test that uses the fake now calls `finish(server)`: it hangs up and awaits the task, so the loop reaches its EOF end. |
| `mcp_client.rs:707` — span end of `raw_server`'s task (`sleep(30 s)`) | covered | the sleep became a read-until-EOF loop and the callers `close(session, task).await`; holding the channel open is still what makes the "no answer came" arm distinguishable from EOF. |
| `mcp_client.rs:730` — span end of `endless_cursor_server`'s task | covered | same shape: the caller hangs up and awaits the task, so its 200 answered pages end at a real EOF. |
| `mcp_client.rs:455` — `drain_stderr`'s `if let Ok(mut tail) = tail.lock()` else (poisoned mutex) | covered | the new `a_poisoned_stderr_tail_is_tolerated_by_the_drain_and_the_reader` poisons the lock by unwinding while holding it, then drives the server on, so the drain really takes the `Err` arm while lines keep arriving. |

The previous ledger said the poison arm was "not reachable: nothing panics while
holding that lock, so no test can poison it". That was right about *production*
and wrong about *what a test can construct*: a test may poison the lock
deliberately, which turns the tolerant arm into a real contract (a poisoned
diagnostics lock must not kill the drain and block the child) instead of an
unreachable branch. `mem::forget`-style trickery is not involved; the guard is
held and a panic unwinds through it, which is exactly how a poisoned mutex
happens in practice.

Everything else in the two files executes: `python3 .future/cov100/covlines.py
detail` over this lcov lists exactly these 4 line numbers, and §2's `total` for
each file is their sum against the measured lines.

**On a Windows run.** Every process-level test in this record is inside
`#[cfg(unix)] mod` blocks (the fixture server is `/bin/sh`), the same way the
pre-existing tests in both files already were. The 4 rows above are unaffected
by the host — none of them is a statement — but a Windows lcov would show the
unix-gated blocks as uncovered, and those lines are the POSIX CI leg's to
measure (`platform-coverage.md` names the authoritative platform per host).

## 7. Findings worth keeping

### 7.1 A crate-wide `cargo fmt` reformatted the two files, so this record's numbers were re-measured

At 18:20:07 every file under `cli/src/commands/` (eight of them, including this
record's two and six owned by other workers) was rewritten in the same instant —
a crate-wide `cargo fmt`/`make lint`. Whose, exactly, is not visible from this
side: the campaign's tooling takes a "safety snapshot" commit per worker unit
(`7aea504b test(CLI-A): safety snapshot of the worker-cli-mcp coverage unit` is
this unit's), so a format-before-snapshot run and a peer's own `make lint` look
identical in the mtimes. Either way it reformatted this record's two files
(multi-line call arguments exploded onto separate lines: +16 instrumented lines
in `mcp_servers.rs`, +1 in `mcp_client.rs`), which invalidated the lcov taken
moments earlier. The numbers in §2 and the line
numbers in §6 are from a run **after** that reformat, and the record pins the md5
of both files so a reviewer can tell whether a later edit invalidates it again.
Formatting is semantically inert (`rustfmt --edition 2021 --check` passes on both
files, and the 96 scoped tests pass both before and after), but the *line numbers
of an uncovered-line census are not* — anyone landing a crate-wide `cargo fmt`
must expect every in-flight coverage record in the crate to need a re-measure.
Note that the four `mcp_servers.rs`-side and `mcp_client.rs` production lines in
§6 kept their line numbers across that reformat (it only touched the test region
below them). Per the campaign's later rule, this unit now formats **file by
file** (`rustfmt` on each of its two files, after `rustfmt --check` says it is
dirty) and never `cargo fmt -p future-cli`, which is the shape that rewrote the
six peer files above.

**A second re-pin, 2026-10-17.** The md5 pin did its job: `review-cli.md` §6.2
caught that `mcp_client.rs` no longer held the bytes the record described —
`08108be6` rewrote the `if let Some(stderr) = child.stderr.take()` block into a
`.map()`, 23 lines of the file. That is what §1's re-derivation and §6's
re-derived row are for; the *counts* were unmoved (still one residual line), so
the lesson is precise: **a pin that is not re-checked tells you a file changed
but not that a row became wrong**, and it was the reviewer, not this record, that
read the diff. The re-measurement is the same command against the same two
paths, so re-running it is cheap; do it whenever either md5 moves.

### 7.2 Peer-owned flakes seen mid-campaign — all green in the final run

The two tests below are in another worker's files (`browser_tools.rs`,
`chromium_page.rs`), were failing intermittently while that worker was rewriting
them, and are *not* failing any more. Listed as history, because they are what
the earlier version of §2 had to report as a red whole-crate signal:

* `browser::chromium::chromium_page::tests::create_page_times_out_when_target_never_discovered` —
  `assertion failed: started.elapsed().as_secs() >= 5` (`chromium_page.rs:780`),
  a wall-clock assertion on a timeout, seen once.
* `commands::browser_tools::tests::status_over_a_live_socket_reports_the_reachable_version` —
  panicked at `browser_tools.rs:2926` with a `status --json` document whose
  `reachable` was `true`, seen once.

Two consecutive whole-lib runs failed on two *different* tests of this class
while all of this record's tests passed, and both are green now: the final
measurement run is **`1159 passed; 0 failed`, exit 0**. The same is true of
the lint gate — `cargo clippy -p future-cli --all-targets -- -D warnings` failed
mid-campaign on `cli/src/commands/session_wait.rs:331`
(`clippy::field_reassign_with_default`) and now exits **0** for the whole crate,
with no diagnostic naming either file of this record.

### 7.3 What the MCP surface's coverage rests on

* **`cli/src/lib.rs` already covers part of this surface**, through
  `mcp_dispatch_config_and_unknown_subcommand` (`run(&["mcp", …])`): the empty
  `list`/`status` text, the unknown-server `resolve` error, `add` (basic),
  `status` against a nonexistent program (which is why the spawn-failure arm was
  the one `status` arm already covered) and `remove`. This record's tests name
  which of *their* arms is still missing rather than duplicating those.
  Nothing in this record is claimed on the strength of that test alone: each row
  in §3 names the test in this file that asserts the behaviour.
* **`serve` cannot be driven by an in-process test.** Not "hard": fd 0 is
  process-global and a test that reads inherited stdin can block on a TTY. The
  two honest ways to cover its production paths are a child process with a pipe
  (used here, following `commands::tools`) or a reader seam (added here for the
  paths a working pipe cannot reach). Both are documented in §3.1 and §5.
* **A directory opened as a file is a portable way to make a reader fail**
  (`read(2)` → `EISDIR`) — that is how the `serve` child exercises the request
  loop's stdin-error arm without a broken pipe or a synthetic reader.
* **A mutex is poisoned by a panic, and a test can stage that panic.** That is
  not a synthetic hook: it is the same sequence a real panicking reader would
  produce, and it is what turned the `drain_stderr` "else" from a waiver into a
  contract with an assertion (§6.1). A defensive `if let Ok(…)` arm whose state
  a test *can* produce is not `unreachable-by-construction`, and it should not be
  waived as if it were.

## 8. How a reviewer reproduces all of the above

```sh
cd /Users/geilige/future-os/.worktrees/cov100-r2
ulimit -n 10240
export PATH=/Users/geilige/.rustup/toolchains/1.97.0-aarch64-apple-darwin/bin:$PATH
export CARGO_HOME=/Users/geilige/.cargo RUSTUP_HOME=/Users/geilige/.rustup
export HOME=$PWD/target/test-home CARGO_TARGET_DIR=target/cov-cli-a

# the 42 new tests (and the pre-existing tests of both files), fast:
cargo llvm-cov -p future-cli --no-report -j 4 -- mcp          # 97 passed

# the numbers in §2 and the acceptance run (exit 0 == every target passed):
cargo llvm-cov -p future-cli --no-fail-fast --lcov --output-path /tmp/review-lcov.info -j 4
python3 .future/cov100/covlines.py total  /tmp/review-lcov.info \
        --under .worktrees/cov100-r2/cli/src/commands/mcp_servers.rs \
        --under .worktrees/cov100-r2/cli/src/commands/mcp_client.rs
python3 .future/cov100/covlines.py detail /tmp/review-lcov.info \
        --under .worktrees/cov100-r2/cli/src/commands/mcp_servers.rs \
        --under .worktrees/cov100-r2/cli/src/commands/mcp_client.rs

# the lint gate, exactly as CI runs it (expect exit 0):
cargo clippy -p future-cli --all-targets -j 4 -- -D warnings

# format check — per file, never `cargo fmt -p future-cli` (rewrites peers):
rustfmt --edition 2021 --check cli/src/commands/mcp_servers.rs cli/src/commands/mcp_client.rs

# the mechanical tripwire for §6 (expect MISSING 0 and no STALE block; run it
# WITH --section, or the earlier §2/§6.1 tables of this record are also matched):
python3 .future/cov100/reconcile.py --report /tmp/cli-r2s4-final-lcov.info \
        --scope .worktrees/cov100-r2/cli/src/commands/mcp_client.rs \
        --scope .worktrees/cov100-r2/cli/src/commands/mcp_servers.rs \
        --doc docs/testing/module-cli-mcp.md --section "Waiver ledger" \
        --label cli-mcp-s12
```

A nonzero exit from any `covlines.py` command invalidates the number it printed
(zeroed report, or `--under` that matched nothing) — check it, do not read past
it.

To falsify the child-process evidence specifically, watch what the child is
asserted on: `cargo llvm-cov -p future-cli --no-report -j 4 --
--exact commands::mcp_servers::tests::process::serve_drives_a_real_stdin_session
--nocapture` prints each child's real stdout/stderr on failure (the parent's
assert message embeds both).
