# future-cli — coverage, waivers and test-quality record

Goal `cov-100-multidim` (round 2) · module **future-cli** (`cli/`) · branch
`test/cov100-r2` (worktree `.worktrees/cov100-r2`) · baseline revision
`8449acf6` · host **macOS arm64 (Apple Silicon)**, rustc 1.97.0 pinned by
`rust-toolchain.toml` · session `20261016-175258-7e4ce8d75ffc4a8ba5c9bf8de548e77f`.

Scope of this record: **`cli/src` except `commands/mcp_servers.rs` and
`commands/mcp_client.rs`**, which a peer owns (its record is
`docs/testing/module-cli-mcp.md`) — plus `cli/tests` and this file.

Metric (round 2, 口径 A): per-line LCOV `DA:<line>,0`. Never the `summary`
block. The supervisor's baseline is
`.future/cov100/baseline/ws-dirty-lcov.info` (revision `8449acf6`, measured on
the main worktree, so its `SF:` paths are `/Users/geilige/future-os/cli/...`);
this segment's lcov is `/tmp/cli-final4-lcov.info` and its paths are
`/Users/geilige/future-os/.worktrees/cov100-r2/cli/...`, which is why the
`--under` argument differs between the two.

## 1. Run identity (exact commands)

```bash
cd /Users/geilige/future-os/.worktrees/cov100-r2
ulimit -n 10240
export PATH=/Users/geilige/.rustup/toolchains/1.97.0-aarch64-apple-darwin/bin:$PATH
export CARGO_HOME=/Users/geilige/.cargo RUSTUP_HOME=/Users/geilige/.rustup
export HOME=$PWD/target/test-home          # the real ~/.future must not leak in
export CARGO_TARGET_DIR=target/cov-cli-b   # private to this unit
mkdir -p "$HOME"
# build + run + report, one process (the atomic form the plan asks for)
find target/cov-cli-b -name '*.profraw' -delete   # one run, one profile
cargo llvm-cov -p future-cli --no-fail-fast --lcov \
    --output-path /tmp/cli-final4-lcov.info -j 4
# if a test target fails, cargo-llvm-cov writes no report; the profraw is still
# on disk, so render the same run instead of re-running it:
cargo llvm-cov report --lcov --output-path /tmp/cli-final4-lcov.info

# the work list and the residual total (note the worktree prefix in SF:)
python3 .future/cov100/covlines.py detail /tmp/cli-final4-lcov.info \
    --under .worktrees/cov100-r2/cli/src
python3 .future/cov100/covlines.py total  /tmp/cli-final4-lcov.info \
    --under .worktrees/cov100-r2/cli/src
```

Suite state for that run — **green**, which is the `cargo-test-cli-green` half of
the acceptance contract:

| target | tests | result |
|---|---|---|
| `future-cli` lib | 1139 | ok |
| `future-cli` main | 2 | ok |
| `cli/tests/bin.rs` | 16 | ok |
| `cli/tests/event_wire_contract.rs` | 2 | ok |
| `cli/tests/version_discovery.rs` | 1 | ok |

Segment 2's runs are the **same command**, with the report path as the only
difference, each after `find target/cov-cli-b -name '*.profraw' -delete`:

| run | command | report |
|---|---|---|
| S4 (intermediate) | `cargo llvm-cov -p future-cli --no-fail-fast --lcov --output-path /tmp/cli-r2s2-lcov.info -j 4` | `/tmp/cli-r2s2-lcov.info` |
| **S5 (final)** | `cargo llvm-cov -p future-cli --no-fail-fast --lcov --output-path /tmp/cli-r2s2-final-lcov.info -j 4` | `/tmp/cli-r2s2-final-lcov.info` |

S5's suite state: `future-cli` lib **1159** ok (1139 at S3, so +20 — exactly this
segment's new test functions), main 2 ok, `cli/tests/bin.rs` 16 ok,
`event_wire_contract.rs` 2 ok, `version_discovery.rs` 1 ok, and `exit=0` from the
whole invocation. S5 ran **after** the `rustfmt` normalisation of §6.6, so §4b's
line numbers are the ones a reviewer will see in that report.

### Segment 3 (mock-span hygiene) — the five runs behind §4c

Same environment block, same `CARGO_TARGET_DIR=target/cov-cli-r2`, each run
preceded by `find target/cov-cli-r2 -name '*.profraw' -delete`:

| run | command (report path is the only difference) | report |
|---|---|---|
| S6 | `cargo llvm-cov -p future-cli --no-fail-fast --lcov --output-path /tmp/cli-r2s3-lcov.info -j 4` | `/tmp/cli-r2s3-lcov.info` |
| S7 | `cargo llvm-cov -p future-cli --no-fail-fast --lcov --output-path /tmp/cli-r2s3-final-lcov.info -j 4` | `/tmp/cli-r2s3-final-lcov.info` |
| S8 (same revision as S7, re-run) | `cargo llvm-cov -p future-cli --no-fail-fast --lcov --output-path /tmp/cli-r2s3-final2-lcov.info -j 4` | `/tmp/cli-r2s3-final2-lcov.info` |
| **S9 (final revision)** | `cargo llvm-cov -p future-cli --no-fail-fast --lcov --output-path /tmp/cli-r2s3-final3-lcov.info -j 4` | `/tmp/cli-r2s3-final3-lcov.info` |
| **S10 (same revision, re-run)** | `cargo llvm-cov -p future-cli --no-fail-fast --lcov --output-path /tmp/cli-r2s3-final4-lcov.info -j 4` | `/tmp/cli-r2s3-final4-lcov.info` |

All five exited `0` (`LLVMCOV_EXIT=0`) with the same green suite state as S5
(lib 1159 / main 2 / 16 / 2 / 1, 0 failed). **Revision measured by S9/S10** (the
run identity the brief asks for): `cli/src` as committed by the supervisor's
snapshot `b80595c7` on branch `test/cov100-r2` (`git status --porcelain cli/src`
empty at measurement time), baseline `8449acf6`, host macOS/aarch64, the
environment block above. **A comment-only normalisation** of the
mocks made two of them say what the code does rather than what an earlier draft of
this pass did (the 404 arms the first attempt had were removed again for being
dead — §3.11), which moved `browser_tools.rs` by −3 lines and `safari_manager.rs`
by +1 from the S7 revision; **the doc's line numbers are S9's**, and S9/S10 are
the two runs on the final revision. In every pair, the residual differs by
`cdp_connection.rs:229` alone (§6.7, §6.11) — that pair is the second measurement
the ledger needs, not a re-run for a better number.

The gates for this pass (`cargo test`, `cargo clippy --all-targets -D warnings`,
`cargo fmt --check`, per-file `rustfmt`) are recorded with their exit codes in
§6.12; the per-line evidence commands used here are
`python3 .future/cov100/covlines.py {total,detail}` and the two scratch readers
`/tmp/covline.py` (one line's `DA` count across reports) and `/tmp/zerolines.py`
(the zero-hit lines inside a source range).

### Segment 4 (REWORK-CLI) — the two runs behind §4c

Same environment block, `CARGO_TARGET_DIR=target/cov-cli-r3` (private to this
unit), each run preceded by `find target/cov-cli-r3 -name '*.profraw' -delete`:

| run | command (report path is the only difference) | report | suite |
|---|---|---|---|
| **S11** | `cargo llvm-cov -p future-cli --no-fail-fast --lcov --output-path /tmp/cli-r2s4-lcov.info -j 4` | `/tmp/cli-r2s4-lcov.info` | exit 0 · lib 1163 / 2 / 24 / 2 / 1 |
| **S12** | the same command with `--output-path /tmp/cli-r2s4-final-lcov.info` | `/tmp/cli-r2s4-final-lcov.info` | suite green (1166 / 2 / 24 / 2 / 1), then the invocation was terminated by a peer's signal during report generation (`LLVMCOV_EXIT=143`) — the report was rendered from **that run's own** 41 profraw files (`cargo llvm-cov report --lcov --output-path …`, exit 0), which is §1's documented fallback |

**Revision measured by S11/S12**: `cli/src` + `cli/tests` as committed in
`7197e39fe` on branch `test/cov100-r2`, baseline `8449acf6`. (S11 measured the
same source minus the last test batch, whose uncommitted changes were what the
run built; the two are named separately in §4c because they differ by four
lines.) Note for a reviewer reproducing this: **this worktree is shared** — a
peer session's broad `git add` swept this unit's first batch of changes into
`03e7df445` and `30db8cdc4` before this unit committed them itself, so
`git log -- cli/tests/bin.rs` has three authors for one batch of tests and
`git blame` is not a reliable attribution here.

## 2. Result — measured, not estimated

| run | covered / measured (`cli/src`) | line % | uncovered | files with gaps |
|---|---|---|---|---|
| baseline `8449acf6` (supervisor) | 36 457 / 37 188 | 98.0343 % | **731** | 30 |
| baseline, excluding the peer's MCP files | — | — | **471** | 28 |
| segment 1, intermediate (S2, before the last batch) | 38 921 / 39 128 | 99.4710 % | **207** | 25 |
| segment 1, final (S3) | 39 128 / 39 308 | 99.5421 % | **180** | 24 |
| segment 1 final, excluding the peer's MCP files | — | — | **172** | 22 |
| segment 2, intermediate (S4, before the closers below) | 39 744 / 39 882 | 99.6540 % | **138** | 22 |
| **segment 2, final (S5 — `/tmp/cli-r2s2-final-lcov.info`)** | **39 781 / 39 914** | **99.6668 %** | **133** | 21 |
| **S5, excluding the peer's MCP files** (`mcp_servers.rs` 3, `mcp_client.rs` 1) | — | — | **129** | 19 |
| segment 3 (mock-span hygiene), intermediate (S6 — `/tmp/cli-r2s3-lcov.info`) | 39 980 / 40 107 | 99.6833 % | **127** | 20 |
| segment 3, pre-normalisation (S7 — `/tmp/cli-r2s3-final-lcov.info`) | 39 971 / 40 093 | 99.6957 % | **122** | 19 |
| segment 3, pre-normalisation, re-run (S8 — `/tmp/cli-r2s3-final2-lcov.info`) | 39 970 / 40 093 | 99.6932 % | **123** | 19 |
| **segment 3 FINAL (S9 — `/tmp/cli-r2s3-final3-lcov.info`)** | **39 970 / 40 093** | **99.6932 %** | **123** | 19 |
| **S9, excluding the peer's MCP files** | — | — | **119** | 17 |
| **segment 3 final, same revision measured again (S10 — `/tmp/cli-r2s3-final4-lcov.info`)** | **39 971 / 40 093** | **99.6957 %** | **122** | 19 |
| **S10, excluding the peer's MCP files** | — | — | **118** | 17 |
| segment 4 (REWORK-CLI), first measurement (S11 — `/tmp/cli-r2s4-lcov.info`) | 40 284 / 40 340 | 99.8612 % | **56** | 19 |
| **segment 4 FINAL (S12 — `/tmp/cli-r2s4-final-lcov.info`)** | **40 327 / 40 379** | **99.8712 %** | **52** | 19 |
| **S12, excluding the peer's MCP files** | — | — | **48** | 19 |

The last three rows are segment 4 (§3.12, §4c): the rework that ran the shipped
binary's `#[cfg(not(test))]` halves, the pty wizard and the mock platform. They
describe revision `7197e39fe` — **123 → 52 uncovered on `cli/src`, 119 → 48 in
scope** — 99.8712 % of 40 379 measured lines. The denominator grew by 286 lines
over segment 3 (mostly the new test code, which is measured too). S9/S10, above
them, still describe their own revision and are kept as history; the reviewer
measured 167 on *its* revision (`fc964fb6`, an upper bound with two caveats —
§4c).

The last four rows are two pairs on two revisions, and within **each** pair the
code and the test set are identical: the only difference is
`browser/chromium/cdp_connection.rs:229` (count 1 in S7 and S10, 0 in S8 and S9 —
§6.7/§6.11). That is why §4c gives both totals instead of a single number, and
why every "residual" figure there names the run it was read from. The `measured`
denominator is the same 40 093 in all four: the comment normalisation between the
pairs moved line *numbers*, not measured lines.

Reading the table: **342 of the 471 lines segment 1 opened with are executed by
tests now (73 %)**, and the in-scope residual is 129 of 471 (27 %). The denominator grew
by 2 120 lines because test code is measured too and this segment added ~1 500
lines of it — a *measured* line that a test executes is a numerator line, so
adding tests cannot flatter the percentage, but it does move the denominator.

Two comparisons matter and they are not the same number:

* **`cli/src` as a whole: 731 → 180 uncovered** (98.03 % → 99.54 %).
* **in scope (all but the two MCP files): 471 → 172.**

Seven files left the residual list entirely: `rpc.rs` (14 → 0),
`session_wait.rs` (12 → 0), `session_control.rs`, `session_reset.rs`,
`session_history.rs`, `browser/target.rs` (9 → 0), and
`browser/chromium/socket_http.rs` (10 → 1, a test-side span end).

## 3. Files changed and the tests that now run

All changes are inside the declared write set and are **tests plus this
record**; no production line was deleted, no guard weakened, no assertion
relaxed, nothing `#[ignore]`d. 48 tests were added (segments 1–2), 8 test-side
mock servers were tightened in segment 3 (§3.11), and segment 4 (REWORK-CLI)
added 6 tests in `cli/tests/bin.rs` and 3 in `cli/src` plus one **projection
seam** in `cli/src/version.rs` (§3.12 — the seam is the one production change of
the whole pass, and it is behaviour-preserving: `build_info()` now calls
`build_info_from(known(GIT_COMMIT), GIT_COMMIT_SHORT, GIT_DIRTY)` and the JSON it
returns is unchanged).

### 3.1 `cli/src/commands/session_transcript.rs` — 13 tests (111 → 5)

The whole *text* form of the three content modes was untested; `--json` was the
only form any test drove, which is why whole `format_text` / `format_counts`
branches were dark.

* `text_paths_render_compact_lists_and_an_empty_one_on_purpose` — a `--paths`
  call (from its structured arguments), a `--paths` result (from its free text),
  and a block with nothing path-shaped rendering `(none)` rather than an empty
  field. `compact_paths` had **no** execution at all before this.
* `text_renders_calls_results_and_the_error_marker` — the unnamed call falling
  back to the bare `tool-call` label, the parsed-arguments rendering, and the
  `(error)` marker that is the only place text mode shows a failed result.
* `transcript_counts_text_mode_renders_every_section_and_the_window` — one
  heading per non-empty distribution (entries/blocks/tools/runs/tokens), the two
  scalars, and the window; proves an empty section is omitted rather than
  printed empty.
* `transcript_runs_text_mode_renders_the_ledger` — the ledger's text form with a
  failed run's error line.
* `transcript_text_mode_reports_the_header_and_skipped_results` — the page
  header and the "N tool result(s) had no call in this window" note.
* `transcript_empty_page_ends_the_scan` — an empty page consumes nothing, so the
  cursor stays put (`nextCursor` 7 in, 7 out).
* `transcript_max_bytes_leaves_the_next_entry_for_the_following_window` — the
  byte budget stops *before* the entry that does not fit, so the next window can
  pick it up.
* `transcript_option_errors_name_the_flag_they_belong_to` — a flag-shaped value
  is a missing value; blank `--session`, an empty `--tool` item, the 64 MiB
  `--max-bytes` boundary (accepted at the limit, refused one past it).
* `session_payloads_are_grepped_and_truncated_inside_their_object` — `--grep`
  can drop a `session_info` payload and `--truncate` bounds the strings inside
  it without touching a string already at the limit.
* `path_collection_walks_arrays_and_stops_at_the_per_block_cap` — arrays are
  walked like objects; both shapes that fill the 100-path cap are stopped, and
  the free-text walker shares the cap.
* `truncation_recurses_through_arrays_and_leaves_scalars_alone`.
* `unselected_and_unmodelled_blocks_leave_no_trace` — a media block and an
  unselected result survive in neither the projection nor the text.
* `the_text_view_skips_block_kinds_it_cannot_render` — a projection from a newer
  agent degrades to the blocks this build can render.

### 3.2 `cli/src/commands/session.rs` — 8 tests (81 → 1)

* `set_reports_each_live_switch_failure_and_keeps_the_rest` — each live switch
  owns its own failure line (`tools`, `appendSystemPrompt`,
  `permissionLevel`, `contextFiles`, `autoCompaction`, `autoRetry`) while a
  successful switch stays in `updated`; the empty system prompt is the *restore*
  path (`(built-in restored)`) and is what goes on the wire.
* `set_text_labels_each_live_switch_it_applied` — the four `option_label` arms
  (`Sandbox`/`CtxFiles`/`AutoCompact`/`AutoRetry`) that only exist on the paths
  that apply those switches, asserted as the exact padded column.
* `session_dispatches_the_self_parsing_subcommands` — `transcript`, `status`,
  `reset`, `wait` (their own parsers), `new` (its own help), and the
  live-session actions reaching `abort_session` rather than the journal path.
* `the_fork_family_reports_json_when_asked` — `forks`/`fork`/`clone` `--json`,
  with the fork request carrying the entry id and target, plus the `clone` text
  form.
* `title_and_export_report_json` — the generator's model, whether the rename was
  applied, and where the export landed.
* `the_fork_family_rejects_unknown_and_valueless_options` — five cases, before
  any agent call.
* `export_refuses_an_agent_that_reported_no_path`.
* `a_refused_system_prompt_is_reported_by_its_own_label`.

### 3.3 `cli/src/commands/browser_tools.rs` — 6 tests (58 → 17)

Socket endpoints are attach-only, and nothing drove that path at all. Two new
test helpers (`serve_unix_json_version`, `socket_path`) make a listener on a
socket path a fixture like the existing TCP ones.

* `start_over_a_socket_endpoint_attaches_and_records_it` — reachable socket →
  `status: already_running`, the endpoint recorded for the next command, and
  both notes (the switch away from the previous endpoint, and the no-op).
* `start_over_a_dead_socket_refuses_to_invent_a_launch` — the refusal that names
  where the browser really belongs.
* `start_without_endpoint_or_port_uses_the_configured_endpoint` — the saved
  config decides when neither `--endpoint` nor `--port` is given.
* `a_socket_answering_garbage_reports_a_failed_probe` — "something is there and
  the probe failed", told apart from "nothing is listening".
* `a_socket_that_answers_only_the_slower_probe_points_at_a_retry` — the third
  diagnosis; each connection gets its own task so the 2 s probe arrives while
  the 1 s one is still waiting.
* `status_over_a_live_socket_reports_the_reachable_version` — `status` over a
  socket reports the endpoint as reachable with the announced version.

### 3.4 `cli/src/browser/chromium/socket_http.rs` — 5 tests (10 → 1)

`get`'s failure modes are each reported as themselves rather than as a timeout,
which is what makes a diagnostic actionable:

* `get_reports_a_peer_that_closed_without_answering` (empty close),
* `get_reports_an_under_run_body_as_incomplete` (body shorter than its own
  `Content-Length`, with the byte count of what did arrive),
* `get_reads_an_unframed_body_to_eof` (no framing headers → EOF ends the body),
* `get_reports_a_malformed_status_line` (a head that frames a body but has no
  parseable status line — the malformed-response arm is the only thing that can
  catch it, since the completeness check passes),
* `an_unterminated_chunk_asks_for_more_input` (chunk data arrived, its CRLF did
  not: "more coming", not "malformed").

### 3.5 `cli/src/commands/session_wait.rs`, `session_control.rs`,
`session_reset.rs`, `session_history.rs` — 6 tests

* `a_second_positional_and_an_unknown_option_are_reported` — a usage error goes
  to stderr with empty stdout.
* `json_mode_reports_the_run_as_a_document` — `--mode json` with `sessionId`,
  `runId`, `text` and the event list.
* `usage_and_missing_values_are_reported_per_action` — the usage line for each
  of the five actions, plus `--run --json` being a *missing value*, not a run id.
* `abort_and_cancel_report_json_when_asked` — both actions' `--json` pass-through.
* `keep_first_is_reported_and_a_report_that_kept_everything_says_unchanged` —
  the third selector form, and the "unchanged" arm (a report whose entry count
  did not fall is not a negative removal).
* `the_all_sessions_table_names_its_session_and_hints_continuations` — the
  `--all` table names each match's session (the entry id alone is only unique
  inside one session) and carries the same continuation hint as the single
  session table; an untruncated scan says nothing about widening it.

### 3.6 `cli/src/commands/workspace.rs` — 1 test (12 → 10)

`every_empty_store_shape_lists_as_empty_without_writing` — three ways of having
nothing to list (no store — pre-existing coverage; a store file the app created
before writing any schema; and a store with the table but no rows) all read as
"no workspaces", and listing a schema-less store does not create the schema.

### 3.7 `cli/src/rpc.rs` — 4 tests (14 → 0)

* `get_session_stats_round_trips_through_the_client` — `getSessionStats` is part
  of the client surface nothing in the CLI reads yet; this is the only place its
  request shape is pinned.
* `wait_for_idle_state_polls_the_streaming_flag_to_clear` — the post-run wait
  stops at the first idle answer (three reads for two polls).
* `an_empty_title_is_skipped_not_applied` — a model that answers with a blank
  title is skipped with the reason on stderr, and nothing is renamed.
* `a_verbose_run_reports_the_title_it_applied` — the `--verbose` trace of the
  extra write.

### 3.8 `cli/src/commands/configure.rs` — 3 tests (69 → 53)

* `a_persisted_model_is_repaired_before_it_reaches_the_validator` —
  `ModelInput::normalize`'s three repairs (missing window, missing output limit,
  output exceeding the window) and the no-op case.
* `a_persisted_model_is_read_from_either_limit_shape` — the flat keys this
  wizard writes and the nested `limit` object the agent's catalog uses; no
  usable id → not a model.
* `unusable_answers_are_refused_and_the_next_one_is_taken` — a blank base URL, a
  non-positive context window and a negative price are each refused once and
  then accepted, and the accepted values are the ones written.

### 3.9 `cli/src/browser/target.rs` — 2 tests (9 → 0)

* `every_target_shape_round_trips_through_as_str` — `as_str` is what error
  messages and the config round-trip echo back, so every target shape answers
  with exactly what it was given.
* `an_abstract_socket_says_which_platform_supports_it` — off Linux/Android the
  stub says which platform can rather than reporting the browser as missing.

### 3.10 Segment 2 — the OPEN rows segment 1 left, closed one fixture each (20 tests)

Every test below was written against a *named* fixture from segment 1's §4 (or
from the corrected classification, §6.8). None of them touches production code.

#### `cli/src/commands/session_status.rs` — 4 tests (195, 279, 281, 303-307, 347 → 0)

* `a_compacting_session_says_so_next_to_the_streaming_flag` — `isCompacting`
  adds `(compacting)` to a line that otherwise reads "no": the session looks
  idle while the agent rewrites its history, which is exactly the state a reader
  must not miss; not compacting keeps the plain line.
* `a_forked_session_names_its_parent` — a real parent id prints, an empty string
  (the payload's unset value) prints nothing, and a payload with **no
  `parentSessionId` member** (an older agent) takes the same silent path — that
  third case is what closed `281`, the `if let`'s else region; segment 1 had it
  as `attribution-artifact` because the sibling line (279) was OPEN.
* `an_interrupted_run_is_reported_distinctly_from_active_and_queued` — the
  interrupted run gets its own line with `activeRun` null, and a null
  `interruptedRun` prints nothing.
* `metrics_without_a_journal_error_read_as_healthy` — `--metrics` on a healthy
  session says `healthy`; an *empty* `eventJournalError` is not a failure
  ("degraded ()" would be a lie). The healthy arm was dark because every
  metrics fixture carried a non-empty error.
* `options_are_strict_and_require_a_session` (extended) — pins that `--` is
  rejected as an unknown option *before* the duplicate-option guard, which is
  the invariant behind §6.8's reclassification of `84`.

#### `cli/src/commands/task.rs` — 3 tests (149, 159-162, 173, 851-855 → 0)

* `output_says_when_a_deleted_runs_answer_was_capped` — a saved answer of
  exactly `RUN_OUTPUT_CHARS` on a run whose conversation was deleted names the
  cap (`(capped at 16000 characters, head…tail)`) and still prints the whole
  answer; one character short is *not* capped (the boundary, not "long-ish");
  and an empty saved answer is `(the run's conversation has no assistant answer
  yet)` rather than a blank line.
* `output_notes_a_conversation_a_later_run_reused` — the note names the run that
  reused the conversation, and the *later* run itself carries no note (the
  warning is about a mismatch, not about reuse).
* `edit_switches_session_retention_and_refuses_an_unknown_value` — `edit
  --session-retention delete` is stored and `show` reports "new per run, deleted
  after it"; an unknown value is refused by name and nothing is stored.

#### `cli/src/commands/workspace.rs` — 5 tests (290-293, 304, 310, 313, 327-328 → 0)

* `a_refused_filing_names_the_store_as_what_could_not_be_written` — the whole
  `note()` surface, including the two refusal wordings: a write failure must not
  advise "create the directory" (it exists), and a missing directory must.
* `filed_reports_a_vanished_workspace_and_an_unreadable_store` — a filing whose
  workspace id resolves to no row is refused ("disappeared"), and a store whose
  `workspaces` table cannot be read is refused rather than panicking.
* `filing_an_empty_or_root_cwd_says_nothing` — `""`, blanks, `/` and `\` all
  trim to "nothing to file under": `Ok(None)`, i.e. the same silent reading as
  "no desktop app", not an error about a change that did happen.
* `filing_a_chat_conversation_updates_its_scratch_workspace` — a session the app
  imported into a chat scratch directory follows it
  (`ChatWorkspaceUpdated`, no note, no workspace row invented).
* `filing_a_cwd_over_an_unreadable_store_is_refused_with_the_reason` — an
  `app.db` that is not a database: the refusal carries the store's own reason
  and the note about the store, not about the directory.

#### `cli/src/browser/chromium/` — 4 tests (39, 78, 79, 154-156, 233, 252, 866 → 0)

* `cdp_connection::connect_over_a_socket_endpoint_round_trips_a_command` — a
  real WebSocket handshake on a Unix socket path plus a `Browser.getVersion`
  round trip, so the socket branch is proved by the answer coming back over the
  socket (segment 1 recorded this as needing "a fixture serving a WebSocket
  handshake on a Unix socket path").
* `cdp_transport::connect_over_socket_handshakes_with_the_path_and_a_loopback_host`
  (extended) — a non-`ping` text frame is sent first: the reply that comes back
  is still `pong`, which is the assertion that the mock skipped the first frame
  (the `_ => continue` arm) instead of answering it.
* `chromium_endpoint::a_socket_endpoint_reporting_a_pathless_web_socket_url_is_refused`
  — `/json/version` over a socket answering `ws://127.0.0.1:9222` (no path) is
  refused, and the message echoes the offending value so it can be compared.
* `chromium_manager::endpoint_reachable_variants` (extended) — five
  unparseable endpoints (`""`, blanks, `127.0.0.1:9222`, `unix:`,
  `abstract:`) are not reachable and no probe is attempted.
* `chromium_session::a_page_without_a_session_is_listed_but_not_probed` — a page
  that is ordered but whose session is not in hand is listed, is **not** asked
  for `document.visibilityState` (no `Runtime.evaluate` is recorded), is not
  reported as visible, and with nothing visible the last page in order is the
  active fallback. One fixture, and it is also the assertion that decides
  `252`: without the guard the probe would run on the browser session and the
  page would come back "visible".

#### `cli/src/commands/configure.rs` — 3 tests (345-347, 483, 488 → 0)

* `an_existing_protocol_is_offered_by_number_or_verbatim_and_kept` — an existing
  `openai-responses` provider is offered `[2]`, `anthropic` `[3]`, and pressing
  Enter keeps the stored protocol end-to-end; a protocol the wizard does **not**
  know is offered verbatim (never silently rewritten to a built-in) and the same
  run is refused by the CLI's own validator — see §6.9.
* `an_unrecognised_model_disposition_is_refused_and_asked_again` — `x` at the
  disposition prompt is refused on stderr, the prompt repeats, and no model is
  deleted by a typo.
* `a_prompter_failure_at_the_model_disposition_propagates` — answers running out
  at that prompt abort the wizard (`483`, the `?` that segment 1 recorded as
  needing a failing read) and nothing is written.

#### `cli/src/commands/doctor.rs` — 1 test (140 → reclassified, §6.8)

* `login_with_an_empty_key_reads_as_not_logged_in` — asserts the accessor drops
  an empty key (`FutureAuthEntry::key == None`), that the check therefore reports
  "Not logged in", and that one character *is* a credential. The **fixture
  segment 1 named for `140` does not reach `140`** — see §6.8.

#### `cli/src/commands/session_transcript.rs` — 1 test extended (959 → 0)

* `text_renders_calls_results_and_the_error_marker` (extended) — the plain
  `tool-result: <text>` rendering (no `paths`, `isError` false) is now driven:
  segment 1 recorded `959` as "attribution-artifact **or OPEN**, and if a
  reviewer falsifies this the test to add is a plain tool-result rendering in
  text mode". The reviewer was right: it was OPEN, and the empty marker is now
  covered by an assertion that the plain form carries **no** `(error)` marker.

### 3.11 Segment 3 — the test-side mock spans of §7.4, closed where they are winnable

Segment 2's §7.4 named the honest fix for the mock-server rows: *let those loops
terminate instead of aborting*. That is what this pass did — no new test
function, but **8 mock servers in 3 files** now run to their own last line, and
each one asserts **which call it served** instead of replying to anything that
connects (7 of the 8 were ledger rows; the eighth,
`serve_json_version_connections`, was already covered and got the same shape so
its callers could assert the probe count):

| file | mock | what it now does |
|---|---|---|
| `commands/browser_tools.rs` | `a_socket_that_never_answers_points_at_the_foreground`'s holder | records the request on the connection it holds (`1` hit), holds it until a `oneshot` release instead of sleeping 5 s and `abort()`ing, and the test then `await`s the task — asserting it was the *bare reachability* probe (`seen[0].is_empty()`) that was held, not the `/json/version` one |
| `commands/browser_tools.rs` | `UnixJsonVersionMock` (was `serve_unix_json_version`, count-parameterised) | accept loop stopped by a signal (`break` = 10 hits), per-connection answer joined, request lines recorded; both callers assert every request was `GET /json/version HTTP/1.1` (4 for the double `start`, exactly 1 for `status`) |
| `commands/browser_tools.rs` | `JsonVersionMock` (was `serve_json_version_connections`, count-parameterised) | same shape for the fixed TCP port; the three callers assert **4**, **3** and **0** version probes — the 0 is a finding (§6.13) |
| `commands/browser_tools.rs` | the garbage-answer socket | `for _ in 0..2` loop now *runs out* and is `await`ed under a 10 s bound; the test asserts the two connections in order: `""` (bare connect) then `GET /json/version HTTP/1.1` (the probe handed non-HTTP) |
| `commands/browser_tools.rs` | the slow/retry socket | accept loop joins its per-connection tasks before returning (`connection.await` = 2 hits each); same two-request assertion, with the second one the probe answered late |
| `commands/browser_tools.rs` | the readiness-window endpoint | `select!`-stopped accept loop, inner answers joined; the test asserts the poll asked for `/json/version` and nothing else |
| `browser/safari/safari_manager.rs` | `serve_webdriver` → `WebdriverMock` | signal-stopped loop, joined answers, answers keyed on the request *path* instead of `contains("/session")`; the test asserts exactly one `POST /session`, at least one `GET /status`, and nothing else |
| `commands/auth.rs` | `auth_request_timeout_includes_response_body`'s server | the `future::pending::<()>()` tail is a `oneshot` release; the test `await`s the task after the body times out |

The mock-span rows this pass took **out** of the ledger — **10 lines across 7
mocks**, each the *span end* of a mock the old code `abort()`ed or a loop it
never let end. The second column carries the historical row (S5's numbering, as
in §4b — kept for the reviewer's `git diff`); the S9/S10 reports show the
replacement statements executed:

| mock | ledger row that is gone | now covered at | hits in S9 / S10 |
|---|---|---|---|
| the never-answers holder | `browser_tools.rs:2556,2557` | the holder's `released.await`, `drop(stream)`, closing `});` (2572–2574 at S9) | 1 each |
| `UnixJsonVersionMock` | `browser_tools.rs:2817` | its `break` (2774), joined answers (2799), loop end | 10 / 7 |
| the garbage-answer socket | `browser_tools.rs:2973` | the `for _ in 0..2` loop end, awaited | 2 |
| the slow/retry socket | `browser_tools.rs:3016` | the accept-loop end + `connection.await` | 2 |
| the readiness-window endpoint | `browser_tools.rs:3310,3328` | its `break` (3514) and joined answers (3542) | 2 / 1 |
| `serve_webdriver` → `WebdriverMock` | `safari_manager.rs:565,587` | its `break` (598) and `connection.await` (635) | 3 / 2 |
| the auth timeout server | `auth.rs:997` | the mock task's `released.await` (1001) and closing `});` (1002) | 1 each |

Two deliberate non-changes:

* `browser/chromium/socket_http.rs:233` — the `serve_and_close` accept guard.
  Removing the arm (the `.expect("accept")` shape `test_server.rs` and the mocks
  above use) would be *deleting a test-side line to improve a metric*, the
  inverse of this campaign's policy. It stays, with its category corrected
  (§4c), and the invariant is now the one `test_server.rs` writes down.
* `browser/chromium/cdp_transport.rs:255` and `commands/browser_tools.rs:232` are
  not mock servers: they are the closing brace of an `if let` whose body always
  `break`s / `return`s. No loop terminates into them, so they keep their
  `attribution-artifact` rows.

## 3.12 Segment 4 (REWORK-CLI) — the shipped binary's non-test halves

`review-cli.md` §0 fell 10 of §4c's 33 rows; §8.5 named the shared cause: 60 of
the 68 `unreachable-by-construction` lines rested on "in no `cfg(test)`
translation unit", which is a fact about the test build. §4c-i states the rule
this segment adopted and applied. The fixtures, all in `cli/tests/bin.rs`:

* **`--linux-sandbox-helper`** (1 test). The unified binary's hidden entry is
  dispatched by `main.rs` before any CLI runtime; the test runs it and asserts
  the platform's outcome (exit 1 + "unavailable on this platform" off Linux;
  exit 125 + the helper's own prefix on Linux, where the request "probe" is not
  decodable). Covers `main.rs:30-31`.
* **A mock Future platform over a hand-written HTTP/1.1 socket** (1 test +
  helpers). Two endpoints — `/client/v1/oauth/device/code` (with `interval: 0`,
  so the poll makes exactly one token request) and
  `/client/v1/oauth/device/token` — plus a record of the paths it answered, so a
  login that never consulted it cannot pass. `PATH` is pointed at an **empty
  directory**: the platform opener (`open` / `xdg-open`) then cannot be resolved,
  the CLI prints the verification URL instead, and **no browser window opens on
  the machine running the suite** — an assertion, not a hope
  (`auth_login_writes_the_key_itself_when_no_agent_is_running`). Covers
  `auth.rs:88-89`, the whole `save_auth` fallback (`484-486`) and, in the second
  test, its RPC branch (`471-483`) through the in-process `MockAgent` that
  already existed.
* **`script(1)` for a real console** (2 tests + `run_under_a_pty`). The custom
  wizard's API-key prompt calls `rpassword`, which reads the **console device**,
  not stdin — which is why the piped test can only reach the failure
  (`config_wizard_reports_the_missing_console_at_the_secret_prompt`, covering
  `configure.rs:45,46` and the `read_secret` `Err` path) and a pty is needed for
  the rest. Three details cost an iteration each and are commented at the
  helper: the command line differs (BSD/macOS `script -q /dev/null cmd args…`
  vs util-linux `script -q -e -c "cmd args" /dev/null`); `script(1)` forwards
  its own stdin EOF to the pty as an **EOT**, which the line discipline turns
  into `read() == 0`, so the answer pipe is held open by a thread until the
  child exits (`wait_with_output` would close it); and the wizard's echoed
  answers appear in stdout, so every assertion is `contains`, never equality.
  The answers drive **two** models (text-only and image-accepting), which is what
  makes both arms of the `modalities` projection run. Run twice — with a dead
  agent address (the CLI's own file writer, asserted by reading `models.json` and
  `auth.json`) and against `MockAgent` (the RPC path, asserted field by field on
  the recorded `upsert_provider`, with `models.json` asserted **absent**). Covers
  `configure.rs:45-54`, `819-851` (`provider_rpc`, all of it), `853-862`.
* **Two probe-budget tests** (1 in `browser_tools.rs`, 1 in `safari_manager.rs`).
  The fast launch tests set `ReadinessBudgetGuard` but their endpoint always
  answers first, so the loop's `probes >= max` break had never been taken;
  these run a launcher that exits immediately against an endpoint nothing serves
  and assert the poll is ended by the **budget, not the 10 s window** (elapsed
  asserted). Covers `browser_tools.rs:382` and `safari_manager.rs:196`.
* **One doctor test** (`a_probe_that_found_no_version_renders_the_bare_path`).
  `render_version_line`'s `NoVersion` arm had no test: the component-path test
  that names the fallback goes through a different renderer. Covers
  `doctor.rs:253`.
* **One unix integration test in `bin.rs`**
  (`browser_start_with_a_missing_launcher_waits_out_the_readiness_window_on_unix`).
  The non-test `drop(child)` exists in no test build; a launcher path that does
  not exist makes the spawn fail while still executing the drop, so no browser
  is launched. It asserts the **unix** outcome (exit 0, `status: "starting"`)
  rather than Windows' failure report, and says so — that difference is the
  product question §6.9 already records. Covers `browser_tools.rs:366-367`.
* **The projection seam in `cli/src/version.rs`.** `FUTURE_GIT_COMMIT` is
  injected by `build.rs` at compile time, so a build made in a checkout can only
  ever take one of the two arms of the build-identity contract — that is why
  `version.rs:96,99` was waived as "needs a source tree without `.git`", which
  `review-cli.md` then **measured** (`0 → 1` in a `.git`-less scratch tree). The
  honest fix is not a waiver for a reachable line: the projection is now
  `build_info_from(commit, commit_short, dirty)`, `build_info()` passes the
  injected values, and
  `a_build_without_a_checkout_reports_no_commit_and_unknown_dirtiness` asserts the
  no-checkout shape through the same function the binary uses. Both arms are
  measured in every build; the JSON `build_info()` returns is byte-identical
  (the shared `assert_identity_shape` assertion set is unchanged).

**77 of the 119 in-scope lines S9 carried are no longer uncovered at S12** —
`auth.rs` 25, `configure.rs` 41, `browser_tools.rs` 8, `main.rs` 1, `version.rs`
2. Six lines of S12's 48 are *new* since S9, and each is accounted for above: the
two braces of a test's port poll (`safari_manager.rs:940,941`, from a peer's test
edits after S9), `doctor.rs:325,357,1103` (CLI-B's new probe code) and
`utils/files.rs:161` (uncovered at S9 too, but mentioned by no row — the reviewer
found it). Of the `browser_tools.rs` eight, seven are the comment/attribute/brace
lines of the `#[cfg(not(windows))]` launch block: they were `attribution-artifact`
rows at S9's numbers and the block now sits at 350-368, where those lines carry
counts of their own (351 = 1, 358 = 1, 359 = 5) or no record at all. Nothing was
deleted to get there: the only production change is the `version.rs` seam, and
its behaviour is asserted identical on both shapes.

## 4. Waiver ledger — every residual line, with its category

> **This is segment 1's ledger (measurement S3, 180 residual lines).** The
> authoritative *final* ledger is §4b (measurement S5, 133 lines): four rows here
> were reclassified and every OPEN row below is either covered now or moved. The
> line numbers in this table are the ones S3 reported; §4b's are the ones S5
> reports.

Categories are the four of `docs/testing/waiver-ledger.md`. A row whose category
is **OPEN** is *not* waived: it is a line a test can still reach, with the
fixture or technique it needs written down. Nothing is left unaccounted for, and
the table is honest about which rows are not results yet.

| file:lines | count | category | mechanism |
|---|---|---|---|
| `commands/configure.rs:820-850` | 31 | `unreachable-by-construction` | `provider_rpc` is `#[cfg(not(test))]`: it exists in no `cfg(test)` translation unit, so no unit test can name it. The `future` binary's copy is reachable only by an interactive `future configure` against a real console — see §4's `read_secret` row for why an integration test cannot reach it either. |
| `commands/configure.rs:853-862` | 8 | `unreachable-by-construction` | the agent-RPC branch of `save_custom_provider`, same `#[cfg(not(test))]` shape. |
| `commands/configure.rs:45-53` | 9 | `unreachable-in-this-environment` | `StdioPrompter::read_line`'s success arm reads the *process* stdin and `read_secret` calls `rpassword`, which opens the console device; a test process has neither. The failure arms are covered (`cli/tests/bin.rs`). 46 is answerable in principle by an integration test that pipes stdin at the wizard (piped stdin answers `read_line`), which no test does yet. |
| `commands/configure.rs:1022-1026` | 5 | `attribution-artifact` | the `assert!(seeded.contains("future-key"), …, seeded.len(), …)` message arguments in `custom_provider_writes_models_and_secret_files`: a *passing* test cannot evaluate them. Evidence: 1019-1021 of the same assertion are covered. |
| `commands/configure.rs:345-347` | 3 | **OPEN** | the protocol default of an *existing* provider (`openai-responses`, `anthropic`, and an unrecognised current value). Fixture: the `editing_a_provider_keeps_existing_models` template with `api` seeded to each value and `""` answered at the protocol prompt. |
| `commands/configure.rs:483,488` | 2 | **OPEN** | the disposition prompt: `483` is the prompt's `?` span, `488` is the "k/e/d" re-prompt. A model edit script that answers `"x"` once closes both. |
| `commands/configure.rs:636,765` | 2 | `attribution-artifact` | the `}` closing the `if let Ok(number) = value.parse::<…>()` in the two numeric prompts: the refusal message below each is covered (636/765 have no counter of their own), and the accepted-value path is covered by `unusable_answers_are_refused_and_the_next_one_is_taken`. |
| `commands/auth.rs:89` | 1 | `unreachable-by-construction` | `#[cfg(not(test))] let auth_data = Value::Null;` — the test build takes the other arm (`load_auth_file`), so this line is not in the test translation unit. |
| `commands/auth.rs:465-488` | 24 | `unreachable-by-construction` | `#[cfg(not(test))] async fn save_auth` (the agent-RPC path and its "agent not running" fallback). Absent from every test binary; reachable only from the shipped binary with a live agent. |
| `commands/auth.rs:997` | 1 | `attribution-artifact` | inside a *test-side* mock HTTP task: the line follows `std::future::pending::<()>().await`, which never resolves, so the expression after it has no counter. Evidence: 975-996 of the same task are covered. |
| `commands/doctor.rs:173,183,186-189` | 6 | `platform-unmeasured` | the sandbox report is the *host probe's own answer*: the `Warn` arm needs `available=false`, `183` a probe that reports a version, and `186-189` (`cfg!(target_os = "linux") && !probe.available`) the Linux Bubblewrap hint. On this host the probe reports `available=true` with neither a path nor a version. Authority: the Linux CI run. |
| `commands/doctor.rs:197-201` | 5 | `unreachable-in-this-environment` | `check_sandbox`'s `Err` arm needs `platform_sandbox_probe_product()` to fail. The previous campaign tried to force it (`FUTURE_HOME` at a temp dir whose `windows-capabilities.json` is `{not json`) and the probe still answered `available=true`; nothing this host accepts reaches the arm. |
| `commands/doctor.rs:140` | 1 | **OPEN** | `check_login`'s *empty*-key arm (distinct from the missing-key arm, which is covered). Fixture: an `auth.json` whose `future` entry has `"key": ""`. |
| `commands/session.rs:1336` | 1 | `unreachable-by-construction` | `option_label`'s `_ => "Setting"`: every key `set_session_command` can push is one of the thirteen named arms (verified by enumerating the `applied.push((` sites), so no input reaches it. |
| `commands/session_transcript.rs:279` | 1 | `unreachable-by-construction` | `parse`'s `_ => unreachable!("flag list checked above")`: the loop's `matches!` guard rejects every other flag before the `match`. |
| `commands/session_transcript.rs:853` | 1 | `unreachable-by-construction` | `filter_entry`'s `_ => unreachable!("allowed kinds enumerated above")`: `allowed` is only ever true for the four kinds the arms below enumerate. |
| `commands/session_transcript.rs:337,560` | 2 | `attribution-artifact` | closing braces whose `?`/statement already returned: `337` ends `block_arguments`' `if !value.is_null()`, `560` ends `tally`'s `if let Some(ms) = entry["createdAtMs"]`. Evidence: 334-336 and 557-559 are covered. |
| `commands/session_transcript.rs:959` | 1 | `attribution-artifact` (or OPEN) | the empty-string literal of the `isError` conditional inside a `format!` argument list: 956-958 and 960-961 are covered by the error/non-error renderings, so the macro's expansion executes and only the folded literal's line carries no counter of its own. If a reviewer falsifies this, the test to add is a plain `tool-result: <text>` rendering in text mode. |
| `commands/session_status.rs:281` | 1 | `attribution-artifact` | the `}` of `if !parent.is_empty()`; its body (279) is OPEN, see below. |
| `commands/session_status.rs:84,195,279,303-307,347` | 8 | **OPEN** | six rendering arms that need one state fixture each: a duplicated `--` (84), `isCompacting: true` (195), a non-empty `parentSessionId` (279), a non-null `interruptedRun` (303-307), and a state read with no journal error (347, the "healthy" arm). Each is a `get_state` payload away. |
| `commands/task.rs:149-155,159-162,173` | 10 | **OPEN** | the fallback-answer notes: a saved run whose text is capped at `RUN_OUTPUT_CHARS`, a conversation a newer run also used, and a run with no assistant answer. Fixtures: three task-store states. |
| `commands/task.rs:851-855` | 5 | **OPEN** | `--session-retention delete` and the unknown-value error. Fixture: two parse calls (the `keep` arm is already covered). |
| `commands/workspace.rs:215,290-293,304,310,313,327-328` | 10 | **OPEN** | the desktop store's unhappy shapes: a missing `app.db` parent directory (215), a `SessionFiling::Refused` whose reason is not "does not exist" (290-293), a store that cannot be opened (304), a `file_session_for_cwd` that reports the app absent (310) or the chat workspace updated (313), and a `filed` workspace that disappeared or a DB error (327-328). |
| `commands/browser_tools.rs:302-303,310,316-318,320` | 7 | `attribution-artifact` | the attribute lines and comments around covered statements: the non-Windows launch (304-309) *is* covered, while the `#[cfg(not(windows))]` block's `{`, its comment, the `#[cfg(test)]`/`#[cfg(not(test))]` attribute lines and the trailing `}` carry no execution of their own. |
| `commands/browser_tools.rs:319` | 1 | `unreachable-by-construction` | `#[cfg(not(test))] drop(child)` — not in the test translation unit. |
| `commands/browser_tools.rs:536` | 1 | `unreachable-by-construction` | the second `parse` in `browser_status` falls through to `_ =>` only if the endpoint is not a socket, which the guard immediately above has already proved. |
| `commands/browser_tools.rs:232` | 1 | `attribution-artifact` | the `}` closing the `if let Ok(EndpointTarget::Socket(spec)) … return`, whose `return` is covered. |
| `commands/browser_tools.rs:2566-2567,3322,3340` | 4 | `attribution-artifact` | test-side mock servers: the `break`/`drop`/task ends of loops the test aborts once the assertion it needs has landed. Pre-existing (the previous campaign waived the same spans). |
| `commands/browser_tools.rs:2827,2983,3026` | 3 | `attribution-artifact` | the same shape in this batch's new socket fixtures: span ends of `tokio::spawn` closures and of an accept tail that cannot fail while the test owns the listener. |
| `browser/chromium/socket_http.rs:233` | 1 | `attribution-artifact` | the `return` of `serve_and_close`'s accept `else` arm — test-side, unreachable while the test owns the path. |
| `browser/chromium/cdp_connection.rs:154-156` | 3 | **OPEN** | `CdpConnection::connect_with` over a *socket* spec: needs a fixture serving a WebSocket handshake on a Unix socket path (the transport-level socket test exists in `cdp_transport`, so the server side is buildable). |
| `browser/chromium/cdp_transport.rs:233,250` | 2 | **OPEN** | the `_ => continue` arm of the socket transport's message loop and the `break text` of the test's own reply loop. |
| `browser/chromium/chromium_endpoint.rs:78,79` | 2 | **OPEN** | `ws_path_of` returning `None` for a socket endpoint whose reported `webSocketDebuggerUrl` has no path: one mock `/json/version` body away. |
| `browser/chromium/chromium_manager.rs:39` | 1 | **OPEN** | `endpoint_reachable` with an unparseable endpoint (`EndpointTarget::parse` fails). |
| `browser/chromium/chromium_manager.rs:234,238,241-242,245` | 5 | **OPEN**/`attribution-artifact` | the miss arm of the host-dependent `launcher_lookup_platform_discovery_runs` (`None =>`) plus its block ends: whether the platform scan finds a browser is the *host's* property, and the deterministic version of the same claim lives in `launcher_lookup_misses_when_the_roots_are_unset`. The `None` arm is reachable by clearing `LOCALAPPDATA`/`PROGRAMFILES` with the env lock held. |
| `browser/chromium/chromium_session.rs:252` | 1 | **OPEN** | `tabs` skipping a non-page or unattached target: a mocked target list with a `"browser"`-type target. |
| `browser/chromium/chromium_session.rs:866` | 1 | `attribution-artifact` | the `or_else(\|\| pages.last()…)` span end in the active-tab fallback chain; the surrounding expressions are covered. |
| `browser/discovery.rs:180` | 1 | `attribution-artifact` | the `}` of the host-dependent hit arm in `discovery_runs_platform_candidates`; the assertion inside it is covered when the host has a browser and the deterministic miss is covered separately. |
| `browser/safari/safari_manager.rs:209` | 1 | `unreachable-in-this-environment` | `safari_status`'s *outer* `tokio::time::timeout(2 s, …)` arm: the inner reqwest timeout is also 2 s and starts first, so on this host the inner one always resolves the future. Forcing the outer arm would need the inner deadline to be slower, which the code does not permit. |
| `browser/safari/safari_manager.rs:565,587` | 2 | `attribution-artifact` | test-side mock WebDriver server: the `break` of its accept loop and the closing `})` of its task, aborted by the test once `safari_start` has returned. |
| `browser/safari/safari_manager.rs:744` | 1 | `unreachable-by-construction` | a *test-side* `match` arm: `_ => None` for a non-`Webdriver` connection, kept so the match is total; the assertion two lines below requires the `Webdriver` variant, so the arm cannot be taken. |
| `browser/target.rs:108` | 0 | covered | the abstract-socket connect arm — `an_abstract_socket_says_which_platform_supports_it` reaches it, and the Linux/Android authority is the CI run. |
| `commands/tools.rs:891` | 1 | `attribution-artifact` | the `}` closing `if let Some(parent) = file_path.parent()` whose `map_err(…)?` already returned; next to the covered image-write path. |
| `commands/settings.rs:180` | 1 | `attribution-artifact` | the same shape in `write_document`: `create_dir_all` (178-179) is covered, the closing brace has no counter of its own. |
| `commands/skills.rs:564` | 1 | `attribution-artifact` | the `}` of the `else if let Some(v) = meta_rest.strip_prefix("version:")` branch; the branch's own return is covered by the `metadata: version:` case and its `else` by the JSON cases. |
| `main.rs:31` | 1 | `platform-unmeasured` | the `--linux-sandbox-helper` re-exec entry: it dispatches to the agent runtime before any CLI initialisation, and it belongs to the Linux sandbox path. Authority: the Linux CI run (no test drives it on either platform today — see §6.2). |
| `version.rs:96,99` | 2 | `unreachable-in-this-environment` | inside a test, the `None =>` arm for a build with no git metadata: the test binary is always built in a checkout, so `gitCommitShort` is present. Needs a source tree without `.git`. |

Machine totals for the (`S3`) measurement: `cli/src` residual **180**, of which
the two peer-owned MCP files hold 8, so **172 in scope**. Of those 172, ≈47 were
genuine waivers under the four categories and ≈106 were OPEN (a test each, with
the fixture named above). Segment 2 covered 41 of those OPEN rows and reclassified
four — the final ledger is §4b.

## 4b. Final ledger — measurement S5 (`/tmp/cli-r2s2-final-lcov.info`, superseded by §4c)

`cli/src` residual **133** (`python3 .future/cov100/covlines.py total … --under
.worktrees/cov100-r2/cli/src` → `covered=39781 measured=39914 uncovered=133
lines%=99.6668`, tool exit **0**). Four lines are the peer-owned MCP files, so
**129 are in scope, and every one of them is waived** — there is no OPEN row
left. Line numbers are S5's; §4's table explains the same rows at S3's numbers
where they moved (the `rustfmt` normalisation in §6.6 shifted `browser_tools.rs`
by −10).

| file:lines | count | category | mechanism |
|---|---|---|---|
| `browser/chromium/cdp_connection.rs:229` | 1 | `unreachable-in-this-environment` | the "closed while holding the pending lock" re-check: the window between the unlocked `is_connected()` (214) and this one contains **no await point**, so no fixture can open it deterministically; it is reached only when another thread's `disconnect()` lands inside it (measured count **2 in S3**, **0 in S4 and S5** — see §6.7). A production seam to force it is exactly what this campaign forbids adding. |
| `browser/chromium/cdp_transport.rs:255` | 1 | `attribution-artifact` | the `}` of the reply loop's `if let` (was 250): the body always `break`s, so the fall-through has no counter; 251-254 are covered by the same test. |
| `browser/chromium/chromium_manager.rs:244,248` | 2 | `attribution-artifact` | the block ends of the two covered `if let` arms inside `launcher_lookup_platform_discovery_runs` (was 234/238); the assertions inside them execute. |
| `browser/chromium/chromium_manager.rs:251,252,255` | 3 | `platform-unmeasured` | the `None =>` arm of the same test (was 241/242/245): macOS discovery is a fixed `/Applications` path list (no env seam, unlike Windows) and Chrome is installed here, so the miss arm needs a host without a browser — a bare container, i.e. the Linux CI image. The same claim *is* covered deterministically on Windows by `launcher_lookup_misses_when_the_roots_are_unset`. |
| `browser/chromium/socket_http.rs:233` | 1 | `attribution-artifact` | the test-side `serve_and_close` accept `else`'s `return` — unreachable while the test owns the socket path. |
| `browser/discovery.rs:180` | 1 | `attribution-artifact` | the `}` of the host-dependent hit arm in `discovery_runs_platform_candidates`; the assertion inside it is covered on this host and the deterministic miss is covered by `windows_discovery_misses_when_the_program_files_roots_are_unset`. |
| `browser/safari/safari_manager.rs:209` | 1 | `unreachable-in-this-environment` | `safari_status`'s *outer* `tokio::time::timeout(2 s, …)`: the inner reqwest timeout is also 2 s and starts first, so the outer one never wins. Forcing it needs the inner deadline slower, which the code does not permit. |
| `browser/safari/safari_manager.rs:565,587` | 2 | `attribution-artifact` | test-side mock WebDriver server: the accept loop's `break` and the task's closing `})`, aborted by the test once `safari_start` has returned. |
| `browser/safari/safari_manager.rs:744` | 1 | `unreachable-by-construction` | a *test-side* `match` arm (`_ => None` for a non-`Webdriver` connection) kept so the match is total; the assertion below it requires the `Webdriver` variant. |
| `commands/auth.rs:89` | 1 | `unreachable-by-construction` | `#[cfg(not(test))] let auth_data = Value::Null;` — the test build takes the `load_auth_file` arm, so the line is in no test translation unit. |
| `commands/auth.rs:465-488` | 24 | `unreachable-by-construction` | `#[cfg(not(test))] async fn save_auth` (agent-RPC path and its fallback): absent from every test binary, reachable only from the shipped binary with a live agent. |
| `commands/auth.rs:997` | 1 | `attribution-artifact` | test-side mock HTTP task: the line after `std::future::pending::<()>().await`, which never resolves, so the expression after it has no counter; 975-996 of the same task are covered. |
| `commands/browser_tools.rs:232` | 1 | `attribution-artifact` | the `}` closing `if let Ok(EndpointTarget::Socket(spec)) … return`, whose `return` is covered. |
| `commands/browser_tools.rs:302,303,310,316,317,318,320` | 7 | `attribution-artifact` | the `#[cfg(not(windows))]` block's `{`, its comment, the `#[cfg(test)]`/`#[cfg(not(test))]` attribute lines and the trailing `}`: the non-Windows launch (304-309) is covered, these carry no execution of their own. |
| `commands/browser_tools.rs:319` | 1 | `unreachable-by-construction` | `#[cfg(not(test))] drop(child)` — not in the test translation unit. |
| `commands/browser_tools.rs:536` | 1 | `unreachable-by-construction` | the second `parse` in `browser_status` reaches `_ =>` only if the endpoint is not a socket, which the guard immediately above has already proved. |
| `commands/browser_tools.rs:2556,2557,2817,2973,3016,3310,3328` | 7 | `attribution-artifact` | test-side mock servers (was 2566/2567/2827/2983/3026/3322/3340): span ends of `tokio::spawn` closures, `break`s and drops of loops the test aborts once the assertion it needs has landed. Pre-existing shape, and the honest way to close them is to let the loops end (§7.4). |
| `commands/configure.rs:45,46,49,50,51,52,53` | 7 | `unreachable-in-this-environment` | `StdioPrompter::read_line`'s success arm reads the *process* stdin and `read_secret` calls `rpassword`, which opens the console device; a test process has neither. The failure arms are covered (`cli/tests/bin.rs:main.rs`). |
| `commands/configure.rs:636,765` | 2 | `attribution-artifact` | the `}` closing `if let Ok(number) = value.parse::<…>()` in the two numeric prompts; the refusal below each is covered and so is the accepted-value path (`unusable_answers_are_refused_and_the_next_one_is_taken`). |
| `commands/configure.rs:820,822-833,835,837-846,848,850` | 26 | `unreachable-by-construction` | `#[cfg(not(test))] fn provider_rpc` — in no `cfg(test)` translation unit; reachable only from the shipped binary. |
| `commands/configure.rs:855-862` | 8 | `unreachable-by-construction` | the agent-RPC branch of `save_custom_provider`, same `#[cfg(not(test))]` shape. |
| `commands/configure.rs:1022-1026` | 5 | `attribution-artifact` | the message arguments of `assert!(seeded.contains("future-key"), …, seeded.len(), …)` in a *passing* test: a passing assertion cannot evaluate them. Evidence: 1019-1021 are covered. |
| `commands/doctor.rs:140` | 1 | `unreachable-by-construction` (**corrected**, §6.8) | `check_login`'s empty-key guard: `FutureAuthEntry::key` is only `Some(k)` for non-empty `k` (`auth.rs:530-534` filters), so the accessor cannot produce the value this guards against. Segment 1's named fixture (`"key": ""`) reaches the *missing-key* arm instead — verified by `login_with_an_empty_key_reads_as_not_logged_in`. |
| `commands/doctor.rs:173,183,186-189` | 6 | `platform-unmeasured` | the sandbox report is the host probe's own answer: `173` needs `available=false`, `183` a probe reporting a version, `186-189` the Linux Bubblewrap hint. On this host the probe reports `available=true` with neither path nor version. Authority: the Linux CI run. |
| `commands/doctor.rs:197-201` | 5 | `unreachable-in-this-environment` | `check_sandbox`'s `Err` arm needs `platform_sandbox_probe_product()` to fail; the previous campaign tried forcing it (`FUTURE_HOME` at a temp dir with a malformed `windows-capabilities.json`) and the probe still answered `available=true`. |
| `commands/session.rs:1336` | 1 | `unreachable-by-construction` | `option_label`'s `_ => "Setting"`: every key `set_session_command` can push is one of the thirteen named arms (enumerated by the `applied.push((` sites). |
| `commands/session_status.rs:84` | 1 | `unreachable-by-construction` (**corrected**, §6.8) | `parse`'s `duplicate option: --`: the `else if flag.starts_with("--")` arm above rejects `--` first (and every string equal to it starts with `--`), so the guard cannot see it. `options_are_strict_and_require_a_session` pins the earlier arm's answer. |
| `commands/session_transcript.rs:279` | 1 | `unreachable-by-construction` | `parse`'s `_ => unreachable!("flag list checked above")`: the loop's `matches!` guard rejects every other flag first. |
| `commands/session_transcript.rs:853` | 1 | `unreachable-by-construction` | `filter_entry`'s `_ => unreachable!("allowed kinds enumerated above")`: `allowed` is only true for the four enumerated kinds. |
| `commands/session_transcript.rs:337,560` | 2 | `attribution-artifact` | closing braces whose `?`/statement already returned: `337` ends `block_arguments`' `if !value.is_null()`, `560` ends `tally`'s `if let Some(ms) = entry["createdAtMs"]`. Evidence: 334-336 and 557-559 are covered. |
| `commands/settings.rs:180` | 1 | `attribution-artifact` | `write_document`'s `}` after the covered `create_dir_all` (178-179). |
| `commands/skills.rs:564` | 1 | `attribution-artifact` | the `}` of the `else if let Some(v) = meta_rest.strip_prefix("version:")` branch; its return is covered by the `metadata: version:` case and its `else` by the JSON cases. |
| `commands/tools.rs:891` | 1 | `attribution-artifact` | the `}` closing `if let Some(parent) = file_path.parent()` whose `map_err(…)?` already returned; next to the covered image-write path. |
| `commands/workspace.rs:215` | 1 | `attribution-artifact` (**corrected**, §6.8) | the `}` of `open_for_write`'s `if let Some(parent) = path.parent()`: the body (213-214, `create_dir_all`) is covered; segment 1 had it OPEN with "a missing `app.db` parent directory" as the fixture, but creating a missing parent *is* that line's body. |
| `main.rs:31` | 1 | `unreachable-in-this-environment` (**reclassified**, §6.8b) | the hidden `--linux-sandbox-helper` re-exec entry (`return run_agent(args)`). It is *not* measured by any platform today: the only tests that pass that flag are `agent/tests/linux_sandbox_smoke.rs`, which is `#![cfg(target_os = "linux")]`, is `#[ignore = "requires a native Linux host with a working system bwrap"]`, and spawns `CARGO_BIN_EXE_future-agent` — the *standalone* binary, so it never enters this CLI's `run_from_args`. Reaching it needs a test that spawns the **unified `future` binary** with the hidden flag, which cannot be made meaningful (or non-hanging) off a Linux+bwrap host. |
| `version.rs:96,99` | 2 | `unreachable-in-this-environment` | inside a test, the `None =>` arm for a build with no git metadata: the test binary is always built in a checkout, so `gitCommitShort` is present. Needs a source tree without `.git`. |
| `commands/mcp_client.rs:385`, `commands/mcp_servers.rs:85,342,412` | 4 | **peer-owned** | reported, not claimed: `docs/testing/module-cli-mcp.md` owns these files; they are excluded from the in-scope count above. |

**Totals: 129 in scope, all waived; 0 OPEN.** By category: 67
`unreachable-by-construction` (of which 34 are `#[cfg(not(test))]` production
surface that no test binary contains), 36 `attribution-artifact`, 9
`platform-unmeasured`, 17 `unreachable-in-this-environment` (the MCP peer's 4 are
outside the scope count). By line kind, roughly a quarter are *test-side* mock
spans, which is why §7.4 is the next useful check rather than a rewrite.

## 4c. Ledger — measurement S12 (`/tmp/cli-r2s4-final-lcov.info`)

**The revision this section describes.** `cli/src` and `cli/tests` as committed
in **`7197e39fe`** (`test(REWORK-CLI): the shipped binary's non-test halves,
measured`) on branch `test/cov100-r2`, baseline `8449acf6`, host **macOS
aarch64**, rustc 1.97.0 (pinned), the §1 environment block with
`CARGO_TARGET_DIR=target/cov-cli-r3`, suite **green** (lib 1166 / main 2 /
`cli/tests/bin.rs` 24 / `event_wire_contract.rs` 2 / `version_discovery.rs` 1,
0 failed). Line numbers below are **S12's**; every earlier number in this record
(S3…S10) describes an earlier revision and is kept only as history.

```
$ python3 .future/cov100/covlines.py total /tmp/cli-r2s4-final-lcov.info \
      --under .worktrees/cov100-r2/cli/src
covered=40327 measured=40379 uncovered=52 lines%=99.8712     # tool exit 0
```

**`cli/src`: 123 uncovered at S9 → 56 at S11 → 52 at S12** (99.8712 % of 40 379
measured lines). Four of the 52 are the peer-owned MCP files, so the **in-scope
residual is 48** (was 119 at S9).

Two measurements bracket this rewrite, and they are named because they are not
the same number:

| measurement | report | command | `cli/src` | in scope | what changed |
|---|---|---|---|---|---|
| **S11** | `/tmp/cli-r2s4-lcov.info` | `cargo llvm-cov -p future-cli --no-fail-fast --lcov --output-path /tmp/cli-r2s4-lcov.info -j 4` (exit 0) | 56 | 52 | first run after the shipped-binary tests (§3.12) landed |
| **S12** | `/tmp/cli-r2s4-final-lcov.info` | the same command with `--output-path /tmp/cli-r2s4-final-lcov.info`; the invocation was terminated by a peer's signal **after** the suite had finished green, so the report was rendered from that run's own profraw (`cargo llvm-cov report --lcov --output-path …`, exit 0) — the documented fallback of §1, not a second run | **52** | **48** | the four lines S11 still showed, each closed by a test: `configure.rs:833`, `doctor.rs:253`, `browser_tools.rs:382`, `safari_manager.rs:196` |

**On the reviewer's 167.** `review-cli.md` §6.3 measured `cli/src` at **167**
uncovered on its own revision (`fc964fb6`, scratch tree) as an **upper bound**:
that tree had no `.git` (which flips `version.rs`'s build-identity arm, +8 lines)
and one test failed there so its tail never ran. It is not comparable with the
numbers above, and the 119/118 headline of S9/S10 describes S9/S10 — this
section's 48 describes `7197e39fe`. The difference between the reviewer's 167 and
S9's 123 is the S9 → `fc964fb6` drift plus the lines that run never measured
(its `.git`-less tree and its one failing test — §6.3 there). Between S9 and S12
the four peer-owned lines are unchanged, and of the 119 in-scope lines **77 are no
longer uncovered** while **six entered** the ledger (2 braces of a test's poll
loop added by a peer after S9, 3 lines of CLI-B's new doctor code, and
`utils/files.rs:161`, uncovered at S9 too but mentioned by no row until the
reviewer found it) — the per-file arithmetic is in §3.12.

| file:lines (S12) | count | category | mechanism |
|---|---|---|---|
| `browser/chromium/cdp_connection.rs:229` | 1 | `unreachable-in-this-environment` | the "closed while holding the pending lock" re-check, between the unlocked `is_connected()` (228) and the locked one (230) — **both at 961 hits**, and nothing between them awaits, so only another thread's `disconnect()` landing inside the window reaches it. Measured 2/0/0/0/1/0/0/1/0/0 across S3…S12 and the reviewer's cross-run table (6 of 18 reports nonzero); the *neighbour counts* move too. Reading rule: 0-vs-nonzero only (§6.11), and S10 → S12 are three more measurements of the same claim. |
| `browser/chromium/cdp_transport.rs:255` | 1 | `attribution-artifact` | the `}` of the test's wait `if let`, whose body always `break`s: 253 = 1, 254 = 1, and 255 is the region llvm-cov attributes to the block's dead remainder. |
| `browser/chromium/chromium_manager.rs:244,248` | 2 | `attribution-artifact` | the block ends of the two `if let` bodies in `launcher_lookup_platform_discovery_runs`: every statement inside them is covered (239-242, 245-247 all ≥ 1). |
| `browser/chromium/chromium_manager.rs:251,252,255` | 3 | `platform-unmeasured` (b) | the `None =>` arm of the same test. macOS discovery is a fixed `/Applications` list with **no env seam** (unlike the Windows arm), so this needs a **host on which the browser scan misses** — the Linux CI image, or a container without Chrome. The Windows counterpart is covered by `launcher_lookup_misses_when_the_roots_are_unset`. |
| `browser/chromium/socket_http.rs:233` | 1 | `unreachable-in-this-environment` (**reclassified from `unreachable-by-construction`**, per `review-cli.md` §3.2.5) | the `return;` of the test-side `serve_and_close` accept guard (230-232 = 4, 235-236 covered). Its own text names the reaching failures: **EMFILE/ENFILE** on `accept` — i.e. a full file-descriptor table. That shape was *measured* `0 → 1` in this campaign (`channels/transport/webhook.rs` 149-151, `review-rust.md` §3.5) and the fixture needs `setrlimit(RLIMIT_NOFILE)` **and** its own process. This guard lives in the lib's `#[cfg(test)] mod tests`, whose one process also runs 1 166 other tests, so filling its table would make unrelated tests flake; the missing thing is named, and it is not "no input". |
| `browser/discovery.rs:180` | 1 | `attribution-artifact` | the `}` of the host-dependent hit arm in `discovery_runs_platform_candidates`: 179 = 1, 181 = 1, and the deterministic miss has its own Windows test. |
| `browser/safari/safari_manager.rs:267` | 1 | `unreachable-in-this-environment` | `safari_status`'s *outer* `tokio::time::timeout(2 s, …)`: the inner reqwest timeout is also 2 s and starts first, so the outer one never wins and the code offers no way to make the inner deadline slower. Evidence: 0 in every one of the reviewer's 18 cli reports (`review-cli.md` §7's cross-table, where the construct sits at 252 on that revision) and 0 again at S11/S12 here — that is what upgrades the mechanism from an argument to a measurement. |
| `browser/safari/safari_manager.rs:940,941` | 2 | `attribution-artifact` | the closing braces of the port-poll `if let` chain in `launch_then_become_reachable_reports_started`: 938 = 1, 939 = 1, 942 = 16 — the poll always leaves through `break port`, so the block's remainder has no counter. |
| `browser/safari/safari_manager.rs:960` | 1 | `unreachable-by-construction` | a *test-side* `match` arm (`_ => None` for a non-`Webdriver` connection) kept so the match is total; the assertions below it require the `Webdriver` variant. (744 at S5, 820 at S9, 960 at S12.) |
| `commands/browser_tools.rs:280` | 1 | `attribution-artifact` | the `}` closing `if let Ok(EndpointTarget::Socket(spec)) … return`, whose `return` is covered (278 = 4, 279 = 4, 281 = 14). (232 at S9, re-derived per the reviewer's §6.1 drift map.) |
| `commands/browser_tools.rs:599` | 1 | `unreachable-by-construction` | the second `EndpointTarget::parse` reaches `_ =>` only for a non-socket endpoint, which the `matches!` guard four lines above has already proved it is not — a pure string function, so the guard's proof transfers (597 = 2). (536 at S9.) |
| `commands/configure.rs:636,765` | 2 | `attribution-artifact` | the `}` closing the two `if let Ok(number) = value.parse::<…>()` numeric prompts; the refusal below each is covered (632-635 = 20/20/19/1, 637 = 1; 761-764 = 29/29/27/2, 766 = 2). |
| `commands/configure.rs:1022-1026` | 5 | `attribution-artifact` | the message arguments of a *passing* `assert!` — a passing assertion cannot evaluate them. 1018-1021 = 1/1/1 and the assertion itself is covered. |
| `commands/doctor.rs:140` | 1 | `unreachable-by-construction` | `check_login`'s empty-key guard: `FutureAuthEntry::key` is only `Some(k)` for non-empty `k` (`auth.rs:530-534` filters), so the check cannot be handed `Some("")`. |
| `commands/doctor.rs:173,183,186-189` | 6 | `platform-unmeasured` (b) | the host probe's own answer: `173` needs `available=false`, `183` a version string, `186-189` the Linux Bubblewrap hint. On this host the probe reports `available=true` with neither. Authority: the Linux CI run (re-check per §7.3). |
| `commands/doctor.rs:197-201` | 5 | `platform-unmeasured` (b) (**reclassified from `unreachable-in-this-environment`**, per `review-cli.md` §3.2.3) | `check_sandbox`'s `Err` arm. Read the producer: the **macOS arm of `platform_sandbox_probe_product()` has no failure path at all** (`agent/src/sandbox/mod.rs:1054`), and the only `?` in the function is the Windows one. So this is not a host state this platform cannot produce — it is another platform's code path, the exact case `waiver-ledger.md` records being reclassified before (`agent/src/rpc/commands/settings.rs`'s eight rows). |
| `commands/doctor.rs:325` | 1 | `unreachable-by-construction` | the `let-else` that reports "the probe's pipes are not piped": the `spawn` two statements above always sets `stdout(Stdio::piped())` **and** `stderr(Stdio::piped())`, so both `take()`s yield `Some`; a child with non-piped stdio cannot be constructed through this function. (New row: CLI-B's code, which §4c never mentioned — `review-cli.md` §6.3.) |
| `commands/doctor.rs:357` | 1 | `unreachable-by-construction` | the `NoVersion` arm for a failed `(out_task.await, err_task.await)`: a `JoinError` needs one of the two spawned drain tasks to panic or be aborted, and neither has a panic path nor is cancelled — the function holds both handles and awaits them (356 = 13 = both successes). (New row, same origin as 325.) |
| `commands/doctor.rs:1103` | 1 | `unreachable-by-construction` | a *test-side* `panic!` arm (`other => panic!` in `a_probe_that_cannot_be_started_is_reported_as_such`): the production invariant (a missing executable is `NotStarted`, not `NoVersion`) is what the test asserts, so the arm cannot be taken by a passing test (1099-1102 = 1 each, 253 = 1 — this rework's new test). (New row.) |
| `commands/session.rs:1336` | 1 | `unreachable-by-construction` | `option_label`'s `_ => "Setting"`: every key `set_session_command` can push is one of the thirteen named arms. |
| `commands/session_status.rs:84` | 1 | `unreachable-by-construction` | `parse`'s `duplicate option: --`: the `else if flag.starts_with("--")` arm above rejects `--` first, and every string equal to `--` starts with `--`. |
| `commands/session_transcript.rs:279` | 1 | `unreachable-by-construction` | `parse`'s `_ => unreachable!("flag list checked above")`. |
| `commands/session_transcript.rs:337,560` | 2 | `attribution-artifact` | closing braces whose `?`/statement already returned; 334-336 = 13/10/10 and 557-559 = 9/9/9. |
| `commands/session_transcript.rs:853` | 1 | `unreachable-by-construction` | `filter_entry`'s `_ => unreachable!("allowed kinds enumerated above")`. |
| `commands/settings.rs:180` | 1 | `attribution-artifact` | `write_document`'s `}` after the covered `create_dir_all` (178-179 = 6/6, 181 = 6). |
| `commands/skills.rs:564` | 1 | `attribution-artifact` | the `}` of the `else if let Some(v) = meta_rest.strip_prefix("version:")` branch (563 = 1, 565 = 4). |
| `commands/tools.rs:891` | 1 | `attribution-artifact` | the `}` closing `if let Some(parent) = file_path.parent()` whose `map_err(…)?` already returned (890 = 9, 892 = 8). |
| `commands/workspace.rs:215` | 1 | `attribution-artifact` | the `}` of `open_for_write`'s `if let Some(parent) = path.parent()`; the body (213-214 = 12/12, `create_dir_all`, the missing-parent fixture) is covered. |
| `utils/files.rs:161` | 1 | `unreachable-in-this-environment` | the `panic!` of a `let-else` in the **test** `which_finds_the_host_shell` (`#[cfg(unix)]`): reaching it needs a POSIX host whose `PATH` has no `sh`, which is the one thing that test exists to assert the opposite of (158-160 = 1 each, i.e. the arm above is the one taken). **This row was missing entirely** at S9 (`review-cli.md` §6.3: `utils/files.rs` was "a file §4c never mentions"). |
| `commands/mcp_client.rs:406`, `commands/mcp_servers.rs:85,342,412` | 4 | **peer-owned** | `docs/testing/module-cli-mcp.md` owns these files and re-derived all four at this same S12 measurement (its §6); excluded from the in-scope count. |

**Totals at S12: 48 in scope, all waived; 0 OPEN.** By category: 20
`attribution-artifact`, 14 `platform-unmeasured`, 10
`unreachable-by-construction`, 4 `unreachable-in-this-environment`. Against §4c's
S9 totals (68/25/9/17) the moves are: 67 lines became executed tests (§3.12),
three rows changed category (`socket_http.rs:233`, `doctor.rs:197-201`, and
`cdp_connection.rs:229`'s re-worded mechanism), one re-worded condition
(`chromium_manager.rs:251,252,255`), and five rows appeared that S9 never
mentioned (`utils/files.rs:161`, `doctor.rs:253` (covered by test),
`doctor.rs:325,357,1103`).

### 4c-i. The `#[cfg(not(test))]` family — the class rule, decided once

`review-cli.md` §8.5 is right that 60 of S9's 68 `unreachable-by-construction`
lines rested on "in no `cfg(test)` translation unit", which is a fact about the
**test build** and not an unreachability claim. The rule this record now applies,
stated once and used consistently:

> **A line that exists only in the non-test build is not
> `unreachable-by-construction`.** The shipped binary is an input. If a fixture
> can run it and reach the line, the line gets a test in `cli/tests/` — never a
> waiver. If no fixture here can reach it, it is registered
> `unreachable-in-this-environment` and the *missing fixture* is named.

Applied to the family's 60 lines, all of which are **gone from the ledger**:

| family member (S9 lines) | fixture that now runs it |
|---|---|
| `auth.rs:88-89` (`login`'s non-test binding) | `auth_login_against_an_unreachable_platform_reports_a_network_error` |
| `auth.rs:464-488` (`save_auth`) | `auth_login_writes_the_key_itself_when_no_agent_is_running` (file branch, 484-486) and `auth_login_saves_through_a_running_agent` (RPC branch, 471-483) |
| `configure.rs:45,46,49-54` (`StdioPrompter`) | `config_wizard_reports_the_missing_console_at_the_secret_prompt` (pipe → `read_line`) and the two pty tests (`read_secret`) |
| `configure.rs:819-851` (`provider_rpc`) | `config_wizard_saves_through_a_running_agent` (+ `configure.rs:833` by the two-model variant) |
| `configure.rs:853-862` (`save_custom_provider`'s agent branch) | the same two pty tests, one per branch (dead port → file writer, mock agent → RPC) |
| `browser_tools.rs:366-367` (`drop(child)`) | `browser_start_with_a_missing_launcher_waits_out_the_readiness_window_on_unix` |

**No line of §4c at S12 is a `#[cfg(not(test))]` line**, so the rule costs
nothing today; it is written down because it is what a future segment must apply
to the next such line instead of re-deriving it per row.

The three fixtures the class needed are reusable and are documented in §3.12:
a mock platform over a hand-written HTTP/1.1 socket, the in-process gRPC
`MockAgent` that already existed, and `script(1)` for a real console.

### 4c-ii. What happened to every row `review-cli.md` §0 fell

| reviewer's fall (S9 numbering) | lines | S12 disposition |
|---|---|---|
| `main.rs:31` — measured 0 → 1 | 1 | **tested**: `the_hidden_linux_sandbox_helper_entry_is_dispatched_by_the_binary` |
| `commands/auth.rs:89` — measured 0 → 1 | 1 | **tested**: the dead-platform login test |
| `commands/configure.rs:45,46` — measured 0 → 16 | 2 | **tested**: the piped wizard test |
| `commands/configure.rs:49-53` — measured 0 → 1 | 5 | **tested**: both pty wizard tests (the console device) |
| `commands/configure.rs:855-862` — measured 0 → 1 | 8 | **tested**: the pty tests, one per branch (`857-861` by the mock-agent variant) |
| `version.rs:96,99` — measured 0 → 1 | 2 | **executed in every build**: the projection seam of §3.12 plus `a_build_without_a_checkout_reports_no_commit_and_unknown_dirtiness`; `version.rs` has **0** residuals at S12 |
| `commands/auth.rs:465-488` — category | 24 | **tested**: both `save_auth` branches |
| `commands/configure.rs:820ff` — category | 26 | **tested**: `provider_rpc` through the mock agent |
| `commands/browser_tools.rs:319` (now 367) — category | 1 | **tested**: the unix launcher test |
| `browser/chromium/socket_http.rs:233` — category | 1 | **re-registered** `unreachable-in-this-environment` (a full fd table, own process) — the row above |
| `commands/doctor.rs:197-201` — category | 5 | **re-registered** `platform-unmeasured` (b), Windows — the row above |
| `browser/chromium/cdp_connection.rs:229` — survived, relabelled | 1 | **kept**, mechanism re-worded to "metric flake, read 0-vs-nonzero" (§6.11) |
| `browser/chromium/chromium_manager.rs:251,252,255` — survived, condition narrowed | 3 | **kept**, condition now names the **host state** (a browser scan that misses), not "the Linux CI run" |
| §6.1 drifted citations (`browser_tools.rs`, `safari_manager.rs`, `mcp_client.rs`) | — | **re-derived** at S12: `browser_tools.rs` 280 / 599, `safari_manager.rs` 267 / 940-941 / 960, `mcp_client.rs:406` (its own record) |
| §6.3 `MISSING` lines (`utils/files.rs:161`, `doctor.rs:253,325,357,1103`) | 5 | **registered** (161, 325, 357, 1103) or **tested** (253) — the rows above |

### 4c-iii. Mechanical self-check (and the correction `review-cli.md` §6.4 asked for)

S9's version of this paragraph promised that the STALE half could not fire,
because the tool skipped files with no uncovered line. The supervisor's fix
(`5da793b6`) removed that blind spot, so the sentence below replaces it: **the
STALE half is live, and a STALE `COVERED` row is a real disagreement to
investigate, not a tool artifact.** On S10 it fires on
`cdp_connection.rs:229` — the ±1 of §6.11, which is why the run named here is
S12 and every row above is read from it.

```
$ python3 .future/cov100/reconcile.py --report /tmp/cli-r2s4-final-lcov.info \
      --scope .worktrees/cov100-r2/cli/src --doc docs/testing/module-cli.md \
      --section "4c. Ledger" --label cli-r2s4
[cli-r2s4] files with uncovered lines=19 uncovered lines=52
  mentioned in module-cli.md: 52
  MISSING (uncovered, not mentioned): 0            # and no STALE block
```

`MISSING = 0` and `STALE = 0` at S12: every residual line of this report is
accounted for by a row above, and no row claims a line the report shows covered.
Run **without** `--section` the tool still reports STALE rows from §4 (S3) and
§4b (S5) — its documented "superseded ledger" behaviour, and those two sections
say so in their own headers.

## 5. Dimensions (matrix rows for future-cli)

| dimension | evidence |
|---|---|
| boundary | `session_transcript` — the 64 MiB `--max-bytes` limit accepted at the limit and refused one past it, the 100-path per-block cap in both walkers, a byte budget that stops before the entry that does not fit; `session_wait` — a second positional; `session_control` — five usage lines and a flag-shaped value; `browser_tools` — a socket path that exists vs one that does not |
| error-path | `socket_http` — four peer failures told apart (empty close, under-run body, unframed body, malformed status line); `session` — each live switch's own failure label, a refused system prompt, an agent that reported no export path; `session_control` — a decision the agent refuses; `rpc` — an unusable (empty) title |
| concurrency | `browser_tools` — a 1 s reachability probe vs the 2 s diagnostic probe on the same socket, each connection its own task; `session_wait` — the run event stream under `--mode json`; `rpc` — the post-run wait polling `isStreaming` until it clears |
| property | `session_transcript` — `--truncate` bounds strings *inside* tool arguments and leaves a string at the limit alone, the count sections are omitted when empty, `(none)` rather than an empty path list; `configure` — a persisted model is always repaired to a state the agent's validator accepts; `target` — every endpoint shape round-trips through `as_str` |
| platform-cfg | `browser_tools`/`socket_http`/`target` — the Unix-socket endpoint paths (attach-only, abstract sockets refusing with the platform that supports them), which on this host run for real rather than being skipped |
| serialization | `session` — the `--json` document of every fork-family command, `updated`/`failed` maps keyed by camelCase label; `session_wait` — the run document; `session_history` — the `--all` table naming each match's session |
| fixture fidelity (segment 3) | 8 test-side mock servers in `browser_tools` / `safari_manager` / `auth` now record the request line of every connection they answer and the tests assert those requests (`GET /json/version`, `GET /status`, exactly one `POST /session`) instead of assuming them; each is stopped by a signal and awaited, so the mock's own last lines execute (§3.11) |

## 6. Findings

### 6.1 `cargo fmt -p future-cli` rewrote two files a peer owns

I ran `cargo fmt -p future-cli` (the natural way to satisfy the pre-PR fmt
check) and it **rewrote `cli/src/commands/mcp_servers.rs` and
`cli/src/commands/mcp_client.rs`**, which are outside my write set and belong to
a concurrent peer. Evidence: rustfmt does not re-write a file whose formatted
output is identical (verified with a probe file: mtime unchanged after
`rustfmt`), yet both files' mtimes moved to the same instant as the run, while
their content is now rustfmt-clean (`cargo fmt -p future-cli -- --check` exits
0). The change is therefore layout-only — rustfmt output, no logic touched — but
it may collide with the peer's in-flight buffer.

Recommendation for the rest of the campaign: format per file
(`rustfmt --edition 2024 <my files>`) rather than per crate, and use
`cargo fmt -p <crate> -- --check` only as a check.

### 6.2 Three residual classes are *not* waivers

Per §4, ≈106 lines are OPEN: they are reachable, each needs one fixture, and
they are named so the next segment can start immediately. The cheapest large
ones are `session_status` (8 lines, one `get_state` payload each),
`task` (15 lines, three store states), `workspace` (10 lines, the store's
unhappy shapes) and `browser/chromium` (14 lines, mostly one mock response
each).
### 6.3 Flakes, and one failure of my own

Two failures were seen across four full runs of this batch, and neither is a
regression:

* `commands::doctor::tests::doctor_detects_agent_binary_not_running_as_issue`
  FAILED in one full run of the S2 batch, passed alone
  (`cargo llvm-cov … --lib -- commands::doctor::tests::doctor_detects_agent_binary_not_running_as_issue`
  → ok) and passed in the next two full runs. `doctor` inspects the *host* (is an
  agent running, which binary is on `PATH`), and eight workers share this machine
  with their own agents up. Same class as the documented `cli/tests/bin.rs`
  agent-instance-lock race (`.future/memory/testing-gotchas.md`); re-run, do not
  "fix".
* `status_over_a_live_socket_reports_the_reachable_version` — **mine**, in its
  first run: it asserted the `version` field was a string, and `browser_status`
  carries the whole `/json/version` payload there instead. Fixed to assert
  `version.Browser` against the mock body, which is the stronger claim anyway;
  the final run is green.

### 6.4 The private target directory was emptied mid-batch

Between two measurements `target/cov-cli-b` went from a full instrumented build
to 37 MB with the dependency `rmeta` files gone, which surfaces as
`extern location for <crate> does not exist` while cargo still believes those
units are fresh. The disk was not full (49 GiB free), so the likely cause is a
broad cleanup by another worker — the plan's own instruction is
`rm -rf target/cov-<unit>`, and `target/cov-*` is easy to over-match. It cost
one full rebuild (~13 min) and no correctness: the final numbers in §2 come from
the rebuilt, private directory.

### 6.5 Real bugs found (segment 1)

None this segment. One measured surprise worth recording: the CLI's
`get_session_stats` client method is not called by any command — the tests added
in §3.7 pin its request shape, but if the CLI is not going to read session stats
it is dead surface (its removal is a product decision, not a coverage one).

### 6.6 Segment 2: the crate's `cargo fmt` gate was red, and not for a style opinion

`cargo fmt --check --all` (the CI gate, pinned toolchain, `make lint-rust:251`)
reported **30 hunks** on this branch. 25 of them were in `cli/`, in six files —
`browser_tools.rs` 8, `configure.rs` 5, `session_transcript.rs` 5, `session.rs`
3, `task.rs` 2, `workspace.rs` 2 — and the *cause was a wrong `style_edition`*,
not a disagreement about layout:

* `cli/Cargo.toml` is **`edition = "2021"`**; `cargo fmt` therefore formats this
  crate in the 2021/2015 style (imports case-sensitive, `serde_json::{Map, Value,
  json}`, macro calls wrapped). Verified: with `rustfmt --edition 2021`,
  `origin/main`'s copies of these files are **clean** and the working tree's were
  **dirty** with exactly the hunk counts `cargo fmt -p future-cli -- --check`
  prints; with `--edition 2024` the verdicts invert.
* The working tree had been normalised to the *2024* style — including
  `origin/main`'s own lines: e.g. `use serde_json::{json, Map, Value};` became
  `{Map, Value, json}`, and wrapped `assert!`s became multi-line, in the
  snapshot commit `177ad042` ("safety snapshot … 12 file(s)", 81 deletions), and
  again in `session_transcript.rs`'s imports. That is the same failure mode as
  §6.1 (a whole-file reformat by someone else's tool) but with a *different*
  root cause: a `rustfmt`/`cargo fmt` invocation whose edition/style did not
  match the crate's.
* Repair done **per file, only mine**: `rustfmt --edition 2021 <the six files>`
  (never `cargo fmt -p future-cli`, which rewrites the peer's MCP files). After
  that, `cargo fmt -p future-cli -- --check` exits clean (`0` hunks on
  re-check). At the time of the repair, `cargo fmt --check --all` still flagged 5
  hunks in other units' files (`tui/src/terminal_posix.rs`, four under
  `agent/src/rpc/`); by the end of the segment those units had repaired theirs and
  only two hunks remained, in `agent/src/llm/stream_wait_tests.rs` and
  `packages/rpc/src/transport/unix_endpoint.rs` — **reported, not touched**, they
  are outside this unit's write set.

Consequence for anyone else formatting this crate: the per-file check that
matches CI is `rustfmt --edition 2021 --check <file>` (or `cargo fmt -p
future-cli -- --check`), **not** a bare `rustfmt <file>` (defaults to edition
2015) and **not** `rustfmt --edition 2024`.

### 6.7 The metric itself wobbles: a race-guard line measured 2, then 0, then 1

`browser/chromium/cdp_connection.rs:229` — the "connection closed while holding
the pending lock" re-check — has now been measured across **eight** runs of the
same suite with no code change anywhere near it (S11 and S12 added; both 0, both
961/961 — §4c):

| run | report | line 229 | neighbours (214 / 228) |
|---|---|---|---|
| S3 (segment 1 final) | `/tmp/cli-final4-lcov.info` | **2** | 953 / 953 |
| S4 (segment 2 intermediate) | `/tmp/cli-r2s2-lcov.info` | **0** | 954 / 954 |
| S5 (segment 2 final) | `/tmp/cli-r2s2-final-lcov.info` | **0** | 1017 / 1017 |
| S6 (segment 3 intermediate) | `/tmp/cli-r2s3-lcov.info` | **0** | 956 / 956 |
| S7 (segment 3, pre-normalisation) | `/tmp/cli-r2s3-final-lcov.info` | **1** | 954 / 954 |
| S8 (same revision as S7, re-run) | `/tmp/cli-r2s3-final2-lcov.info` | **0** | 953 / 953 |
| **S9 (final revision)** | `/tmp/cli-r2s3-final3-lcov.info` | **0** | 953 / 953 |
| **S10 (same revision as S9, re-run)** | `/tmp/cli-r2s3-final4-lcov.info` | **1** | 994 / 994 |
| **S11 (segment 4, first run)** | `/tmp/cli-r2s4-lcov.info` | **0** | 961 / 961 |
| **S12 (segment 4 final, same claim)** | `/tmp/cli-r2s4-final-lcov.info` | **0** | 961 / 961 |

The two enclosing statements always agree with each other (they are the same
check either side of the lock), but their count is *also* not stable — 953, 954,
956, 994, 1017 across runs of one suite: another argument for reading `DA` as
0-vs-nonzero only, never as a comparable number.

```
$ python3 /tmp/covline.py /tmp/cli-r2s3-final3-lcov.info cdp_connection.rs 229
   cdp_connection.rs:229 hits=0
$ python3 /tmp/covline.py /tmp/cli-r2s3-final4-lcov.info cdp_connection.rs 229
   cdp_connection.rs:229 hits=1
$ diff <(covlines.py detail …/cli-r2s3-final3-lcov.info) \
       <(covlines.py detail …/cli-r2s3-final4-lcov.info)
1d0
< cli/src/browser/chromium/cdp_connection.rs: 229
```

Mechanism: the line is reachable only when another thread's `disconnect()` lands
between the unlocked `is_connected()` check (214) and the locked re-check (228) —
a window with **no await point** in it, so it opens only under thread
interleaving. Nothing a deterministic test can do; and the count it gets differs
run to run. This is a coverage-metric flake, not a test flake, and both measured
pairs show it end to end — **two identical runs whose whole residual differs by
this one line** (122 vs 123 at S7/S8, 119 vs 118 in scope at S9/S10). The
convention a reader should apply is §6.11.

### 6.8 Four rows of segment 1's ledger were classified wrongly

Each was re-derived from the code and is now backed by a test:

* `commands/doctor.rs:140` was OPEN, with "an `auth.json` whose `future` entry
  has `"key": ""`" as the fixture. That fixture **cannot** reach the line:
  `get_future_auth_entry` filters the empty key (`.filter(|key|
  !key.is_empty())`, `auth.rs:530-534`), so `entry.key` is never `Some("")` and
  the check takes the *missing-key* arm two lines above. Category:
  `unreachable-by-construction`. Evidence:
  `login_with_an_empty_key_reads_as_not_logged_in` asserts
  `FutureAuthEntry::key == None` for that file and then that the check reports
  "Not logged in".
* `commands/session_status.rs:84` (`duplicate option: --`) was OPEN. Unreachable:
  the `else if flag.starts_with("--")` arm above returns first for `--`, and
  every string equal to `--` starts with `--`. Evidence:
  `options_are_strict_and_require_a_session` asserts the unknown-option error for
  `["--", "--"]`.
* `commands/workspace.rs:215` was OPEN with "a missing `app.db` parent
  directory". That is the *body* (213-214, covered — `add` on a fresh HOME does
  exactly that); 215 is the `}`. Category: `attribution-artifact`.
* `commands/session_status.rs:281` was `attribution-artifact` "because 279 is
  OPEN". Both are covered now: 281 is the `if let Some(parentSessionId)` region
  for a payload without the member at all, which no fixture had.

Two more rows moved number, not category, because of the `rustfmt`
normalisation in §6.6: `cdp_transport.rs` 250→255,
`chromium_manager.rs` 234/238/241/242/245→244/248/251/252/255, and
`browser_tools.rs` 2566/2567/2827/2983/3026/3322/3340→2556/2557/2817/2973/3016/3310/3328.

### 6.8b `main.rs:31`: from `platform-unmeasured` to a test (REWORK-CLI)

`--linux-sandbox-helper`'s re-exec is reached only when the *unified* `future`
binary is started with that hidden flag. The only tests that passed the flag were
`agent/tests/linux_sandbox_smoke.rs` — which is `#![cfg(target_os = "linux")]`,
has every case marked `#[ignore = "requires a native Linux host with a working
system bwrap"]`, and spawns `env!("CARGO_BIN_EXE_future-agent")`, the standalone
agent binary — so no report measured `cli/src/main.rs:31`, and the category was
first moved to `unreachable-in-this-environment`.

`review-cli.md` §3.1.3 then **measured** the line `0 → 1` on this host with
`future --linux-sandbox-helper probe` (exit 1, "unavailable on this platform"),
which settles it: the reaching input is the unified binary plus three argv
entries, and it needs no Linux host at all (the *refusal* is the platform's
outcome here; Linux's own arm exits 125 on an undecodable request).
`cli/tests/bin.rs` now asserts both outcomes per platform
(`the_hidden_linux_sandbox_helper_entry_is_dispatched_by_the_binary`), so the row
is gone from §4c and no platform's CI run is the authority for it any more.

### 6.9 Product findings (reported, not changed)

Three behaviours were found while building fixtures. None is a coverage matter
and none was silently "fixed" — the second and third are now pinned by tests so
that a decision to change them is visible:

1. **`edit` accepts a session-retention combination its own `add` refuses.**
   `add --session existing --session-retention delete` is refused with
   "needs `--session new`" (`task.rs:700-702`), but `edit --session-retention
   delete` stores the same contradiction on a task whose policy is `existing`:
   the check exists only on the `add` path, so the validation is bypassable.
   Reproduction:
   `future task add --name t --prompt p --cwd /tmp; future task edit t
   --session-retention delete` → "Updated task". Not fixed here: it is a
   production-behaviour change, and this unit's contract is tests + record.
2. **A provider whose `api` the CLI does not recognise is offered verbatim and
   then refused by the CLI's own validator.** With `"api": "my-proto-v9"`, the
   protocol prompt reads `Select protocol [my-proto-v9]: ` (deliberately: it must
   not silently rewrite a value it has no opinion about), and pressing Enter then
   fails with `unsupported provider API type` (`agent/src/config/providers.rs:179`
   accepts three values). The failure is loud and writes nothing — the right
   outcome, but the message does not say what to do next, and the trap is now
   visible in `an_existing_protocol_is_offered_by_number_or_verbatim_and_kept`.
3. **Two diagnosis strings would be wrong if the code they guard were reached
   with different state** — not a bug today, recorded because the tests that
   cover them are the only place the wording is checked: `SessionFiling::Refused`
   with a reason containing "does not exist" advises "create the directory"
   (right only when the directory is what is missing), and
   `check_login` reports a *degraded* journal only when the reason is non-empty
   (an empty `eventJournalError` reads as healthy, asserted).

### 6.10 The crate's clippy gate was also red

`cargo clippy -p future-cli --all-targets -- -D warnings` (CI's flags) failed with
**4 errors**: three `clippy::await_holding_lock` in the peer-owned
`commands/mcp_client.rs` (559/586/865 — reported to that unit, not touched) and
one in this unit's segment-1 test code,
`commands/session_wait.rs:330-331` (`clippy::field_reassign_with_default`), which
is fixed here by building the `MockAgent` with `..Default::default()`. After the
fix the crate is clippy-clean with CI's flags (`rc=0`).

### 6.11 Convention: how to read a race-guard line in this metric

Settled here because the alternative — "get 229 to stop moving" — is not
available without inventing a production seam.

**Decision: the line stays waived as `unreachable-in-this-environment`** (§4c),
with the interleaving window named as the mechanism, and the reader-facing
exclusion rule is:

1. **Name the run.** Any 100 %-of-coverable-lines claim on `cli/src` must state
   the report it was read from. The current claim is **S12**
   (`/tmp/cli-r2s4-final-lcov.info`, revision `7197e39fe`): `uncovered = 52`, of
   which **48** in scope. S11 (`/tmp/cli-r2s4-lcov.info`, 56 / 52) is the same
   claim one test batch earlier; S9 (123) and S10 (122 — same revision, same
   suite, `119 / 118` in scope) are the segment-3 pair and are kept as history.
2. **Allow ±1 on this line and only this line.** §4c's "48 in scope" is the
   number to compare a re-run against. If a re-run differs on any other line, that
   is a real difference — investigate it; do not absorb it into "the metric
   wobbles".
3. **How a line qualifies for this class** (and not for `attribution-artifact`):
   locate the two statements that bracket the window. If there is **no await
   point and no lock release between them**, no deterministic test can open it;
   the only fixture that would is a second thread whose sleep is tuned to the
   interleaving, which measures the scheduler rather than the code — the
   definition of a flake, and forbidden here.
4. **Do not make the claim true on one run.** Reporting S9 and never repeating it
   is exactly the campaign failure mode this record exists to prevent; the S10
   re-run is the second measurement that keeps the ledger honest.

Why not a seam: opening the window needs an `await` (or a lock release) inserted
between `cdp_connection.rs:214` and `:228`, i.e. a production change whose only
purpose is the metric — the shape PLAN §4b forbids for genuine residue. Why not
an `#[ignore]`d probabilistic test: it would add a flaky test to buy a count that
the next run may not reproduce.

### 6.12 The §5.1 gates for this pass, with exit codes

| command | exit | result |
|---|---|---|
| `cargo test -p future-cli` | **0** | lib 1159 passed / 0 failed, main 2, `cli/tests/bin.rs` 16, `event_wire_contract.rs` 2, `version_discovery.rs` 1, doc-tests 0 — all green (this exact plain run was repeated on the final revision, S9's code: `exit=0`) |
| `cargo clippy -p future-cli --all-targets -- -D warnings` | **0** | clean (CI's flags), re-run after the final comment normalisation |
| `cargo fmt --check` (workspace, CI's command) | **1 at gate time, 0 on the final re-run** | **only** other units' files: `agent/src/rpc/session_prompt/tests.rs:3091` and `tui/src/terminal_posix.rs:460/502/526` were dirty earlier in the pass and clean by the last check, with `tui/tests/agent_sidecar_readiness.rs:86` appearing and being fixed in between. Never this crate's files |
| `cargo fmt -p future-cli -- --check` | **0** | the crate is clean, on every run |
| `rustfmt --edition 2021 --check` on the three files this pass edited | **0** | `commands/browser_tools.rs`, `browser/safari/safari_manager.rs`, `commands/auth.rs` |
| `cargo llvm-cov -p future-cli --no-fail-fast --lcov …` (S6–S10) | **0** | `LLVMCOV_EXIT=0` five times; each run's suite state green (above) |
| `cargo test -p future-cli` (segment 4, revision `7197e39fe`) | **0** | lib **1166** passed / 0 failed, main 2, `cli/tests/bin.rs` **24**, `event_wire_contract.rs` 2, `version_discovery.rs` 1, doc-tests 0 |
| `cargo clippy -p future-cli --all-targets -- -D warnings` (segment 4) | **0** | clean with CI's flags after the new tests (`Session 4`'s build is otherwise untouched) |
| `cargo fmt -p future-cli -- --check` + per-file `rustfmt --edition 2021 --check` on this segment's files | **0** | `version.rs`, `commands/doctor.rs`, `commands/browser_tools.rs`, `browser/safari/safari_manager.rs`, `tests/bin.rs` — the crate-wide form is safe here because this unit is the only writer of these five files |
| `cargo llvm-cov … --lcov --output-path /tmp/cli-r2s4-lcov.info` (S11) | **0** | suite green (1163 / 2 / 24 / 2 / 1) and the report was written by the run itself |
| `cargo llvm-cov … --lcov --output-path /tmp/cli-r2s4-final-lcov.info` (S12) | **143** → `cargo llvm-cov report --lcov …` **0** | the suite had finished green when a peer's signal killed the invocation during report generation; the report was rendered from that run's own profraw files (41), which is §1's documented fallback. The numbers are the same run's — no test was re-run |

The workspace `fmt` row was the one red gate while peers were mid-edit; it is
**green on the final re-run** (exit 0, no `Diff in` lines). It was never red for a
file of this unit's, and this unit must not reformat a peer's file to make it
green (§6.6: `cargo fmt` across a crate rewrote a peer's in-flight files once
already). The per-file and per-crate forms — the ones that match CI for `cli/` —
were clean on every run.

### 6.13 A mock connection count that stood in for a call which never happens

`create_session_skips_refined_save_for_non_chromium_saved_config` used to hand
`serve_json_version_connections(…, 1)` a count of one connection and never check
it. `create_session` makes **no** probe when the saved kind is already refined —
the `/json/version` refine runs only for `browserKind == "chromium"`
(`browser_tools.rs:616`) — so that mock's `accept` was never reached, and the
swallowed `let _ = timeout(2 s, server)` hid it for a whole segment. The rebuilt
mock records the requests it answered and the test now asserts the list is
**empty** ("an already-refined kind is not probed again"): the count became a
fact instead of a hope. It surfaced the moment the count was asserted rather than
swallowed — a count-based loop with an awaited handle hung for its timeout and
failed, which is how a stand-in for a call that never comes should fail.

## 7. Next useful check

1. **Re-verify §4c against the report it names.** S12
   (`/tmp/cli-r2s4-final-lcov.info`, revision `7197e39fe`) → in-scope **48**
   (`uncovered = 52` including the peer's four MCP lines), tool exits `0`. S11
   (`/tmp/cli-r2s4-lcov.info`) → 52 / 48 in scope one test batch earlier. The only
   line allowed to differ between a reviewer's run and these is
   `cdp_connection.rs:229` (§6.11); a difference anywhere else is a real finding.
2. **The two remaining measured-counterexample lines.** `socket_http.rs:233` was
   *measured* `0 → 1` for the identical shape in another module
   (`channels/transport/webhook.rs` 149-151, own process, `setrlimit`). Doing the
   same for the CLI needs a new integration-test file **and** a `setrlimit` call,
   i.e. a `libc`-style dependency: `cli/Cargo.toml` is outside this unit's
   declared write set (and `libc` is not a dev-dependency of `future-cli`), so the
   fixture needs the supervisor to widen the write set. Its row names exactly
   this. The same reasoning applies to `doctor.rs:325/357` only by analogy — those
   are invariants, not OS errors.
3. **Re-check the `platform-unmeasured` rows on the platform each names.**
   `doctor.rs:173,183,186-189` and the reclassified `doctor.rs:197-201` need the
   **Linux** and **Windows** CI runs respectively (`197-201`'s only `?` is the
   Windows probe, so a Linux run will keep showing 0 — that is expected, not a
   defect); `chromium_manager.rs:251,252,255` needs a host whose browser scan
   *misses* (`/Applications` on macOS, or the Linux candidate paths in a bare
   container). If Linux reports the sandbox available *with* a version, the
   `186-189` hint rows are wrong and get reclassified the way `main.rs:31` was
   (§6.8b).
4. **The `#[cfg(not(test))]` rule is now written down (§4c-i) and applies to zero
   lines.** A future segment that adds such a line must not re-derive a row per
   line: run the shipped binary with a fixture (a mock platform, the in-process
   `MockAgent`, `script(1)` for a console) or register
   `unreachable-in-this-environment` naming the missing fixture.
5. **`browser_tools.rs:366-367` now has a unix test that asserts the current
   under-reporting** (exit 0 + `status: "starting"` after the 10 s window, where
   Windows exits 1 with "Failed to launch browser"). That is §6.9's product
   question, and the test makes a decision to change it visible: if the unix
   detached-launch path is fixed to surface the spawn error, this test must be
   updated in the same commit as the product change.
6. CI checkpoints for the next segment: per-file `rustfmt --edition 2021` for
   `cli/` (§6.6 — never crate-wide over a peer's files) and
   `cargo clippy -p future-cli --all-targets -- -D warnings` must still exit 0;
   §6.12 has this segment's exit codes. The workspace-wide `cargo fmt --check` had
   been red earlier on other units' files and was green at the last check of
   segment 3; re-run it, but a red diff is a peer's unless it names a file in
   §3's list.
7. **Coordination note (not a coverage item).** This worktree is shared: two peer
   sessions' broad `git add` sweeps committed this unit's in-flight changes
   (`03e7df445`, `30db8cdc4`) before this unit committed the rest itself
   (`7197e39fe`), and one peer's signal terminated S12's report generation
   mid-flight. Anyone reading history here should treat `git blame` on
   `cli/tests/bin.rs` as unreliable for these three commits.
