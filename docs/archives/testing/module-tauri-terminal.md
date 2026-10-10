# Archived testing records — `docs/testing/module-tauri-terminal.md`

Moved out of `docs/testing/module-tauri-terminal.md` on 2026-09-27 during the
historical-pollution cleanup (task T4a, goal `goal_39567c2a6b22`,
worktree `docs-testing`). The text below is verbatim, with its original
dates; the live record keeps a one-line pointer where each block was
removed.

---


**`gate-green` is STILL NOT established — the gate now fails on two files**, and
both are honest, precisely-scoped gaps rather than measurement problems:

```
desktop-tauri/tau-terminal: 96.1877% lines (9285/9653) across 30 files in
  3 dir(s), 368 uncovered line(s) in 21 file(s), target 99.99%
FAIL: 2 file(s) declared OPEN in desktop-tauri/tau-terminal
  - desktop/src-tauri/src/commands/agent.rs (21 uncovered)
  - desktop/src-tauri/src/terminal/server.rs (88 uncovered)
```

**`lines-100-or-waived`: not yet met.** Every other gate check is green: the
absent-file check passes, and the "not properly waived" list is empty — the
uncovered files *outside* those two carry a verified category. The **OPEN file
count fell 8 → 2** this attempt, and the two that remain are the ones with a
concrete blocker (below) rather than missing effort.

---


**Superseded by the 10th attempt below for `agent.rs` and `server.rs`; kept as
history.**

Each of these moved because every remaining uncovered line in it was traced to a
category that is *true*, with the test proving the neighbouring code ran named on
the row:

| file | was | why it is no longer OPEN |
|---|---|---|
| `commands/update.rs` | 59 | every line is one of the five groups already waived: the hard-coded CDN endpoint, the `platform_allows_updates()` guard, the signed-package extraction, `app.restart()`, wrapper attribution. No reachable-and-untested line was left. |
| `commands/threads.rs` | 20 | 12 of the 13 were `#[tauri::command]` attribute lines (attribution-artifact, with the IPC test named); the 13th — `418`, the no-session arm of `get_session_entries_page` — was a **real gap**, now covered by `get_session_entries_page_is_empty_for_a_thread_without_a_session`. |
| `commands/runs.rs` | 13 | 7 attribute lines; the rest was `218`, the pagination advance, now covered by `an_advancing_tool_page_is_followed_and_a_repeat_is_refused` (see below). |
| `commands/skills.rs` | 11 | 3 attribute lines plus `spawn_builtin_skills`, waived on their own rows. |
| `commands/debug.rs` | 4 | `18` (`app.restart()`) and `113` — a **blank line** that nevertheless carries a coverage region, i.e. pure span attribution. |
| `terminal/session.rs` | 23 | every line is on an existing row: the reader arms this ConPTY cannot reach, the `#518` unreachable `continue`, four failure-only assertion arguments, and the unix arm of a `cfg!(windows)`. |

New tests this attempt, all green, each with a real assertion:

* `server::end_to_end::a_session_can_be_read_and_partially_updated` — `GET` on a
  live session returns the same document `create` did, and a `PATCH` carrying
  **only one** size axis changes the title but applies *no* resize (the
  `(Some, Some) => … , _ => None` arm), with a complete pair afterwards proving
  the resize path itself works.
* `server::end_to_end::the_pump_answers_pings_carries_input_and_ends_with_the_shell`
  — the pump's client-facing half: a `Ping` is answered with a matching `Pong`;
  a `Text` frame reaches the shell's input (its echo is asserted); driving
  `Session::on_eof` while a viewer is attached sends the exit control frame and
  then a **normal** `Close`; and a viewer attaching *after* the exit gets the
  same replay, control frame and `Close` instead of a socket that never speaks.
* `commands::threads::get_session_entries_page_is_empty_for_a_thread_without_a_session`
  — the no-session arm answers an empty page **without** consulting the agent,
  and the assertions are chosen so a default mock answer could not satisfy them.
* `commands::runs::an_advancing_tool_page_is_followed_and_a_repeat_is_refused` —
  a page that advances the cursor is followed (which is what *executes* the
  `offset = next` line), the repeat is refused by name, and the same page marked
  final is returned with its row mapped.

---


* **`commands/agent.rs` (21)** needs a prompt that **succeeds**, which needs a
  scripted `prompt` ack plus a stream script. The fixture that provides them is
  `agent_bridge::tests::pipeline_fixture`, whose `mod test_support` is
  `#[cfg(test)]` and **private to `agent_bridge`** (`src/agent_bridge/mod.rs:22`).
  Reaching it needs a one-word change — `pub(crate) mod test_support;` — in a file
  **outside this task's write set**. I did not make that change; the row below
  records it as the next step. (`5`/`27` are attribute lines, already waived.)
* **`terminal/server.rs` (88)** is the `pump` block plus, now, the defensive arms
  of the test added above. The pump arms that remain need a client that makes a
  *send* fail at a chosen moment (a replay send, the meta frame, a data frame,
  the exit frame, a `Pong`) — the ticket-capacity 429 at `358`/`360` needs a full
  10 000-entry ticket store. Both are reachable in principle and need a
  purpose-built harness; I ran out of budget before building it, and did not
  waive lines I had not reached.

---

Everything below this section is the 8th, 7th, 6th and 4th attempts' run status,
kept as history.

---


**Kept as history only; its conclusion was wrong and §1 (8th attempt) replaced
it.** The six tests it lists as unfixable now all pass by driving `on_eof`
directly, with no product change. The *measurement* half of this section is
still true and still worth knowing: this host's ConPTY never closes the master,
so the reader thread's EOF genuinely never arrives — which is why the reader
lines remain waived (`unreachable-in-this-environment`, see WAIVED).

What follows is the original 6th-attempt text.

Measured directly, not inferred. At the **PTY** level everything works: with the
child's cursor-position query answered, `cmd /C exit 3` is reaped with code 3
after ~100 ms (`terminal::pty::tests::spawns_a_real_pty_and_reports_the_exit_code`
passes). But the master's reader **never returns EOF**:

```
exited at 101.2959ms code=3        # PtySession::try_wait() -> Some(3)
no EOF after 15.0477458s           # a blocking read on the master never returns 0
```

`Session` learns about an exit *only* from the reader thread's `Ok(0) => break`
→ `on_eof()`. On this host that arm is never reached for a child that exits by
itself, so `Session::is_running()` stays `true` forever (confirmed: 30 s of
`wait_until(...)`, `try_wait()` returning `Some(3)` throughout while
`info().status` stayed `Running`). The six failing tests are exactly the ones
whose predicate is "the session noticed the child exit on its own":

| failure | root cause |
|---|---|
| `session::a_real_child_exit_code_is_recorded_and_delivered` | waits for `!is_running()` |
| `session::a_written_line_is_executed_by_the_child` | same |
| `session::write_to_an_exited_session_is_rejected` | same |
| `manager::exited_sessions_stay_listed_until_evicted` | same |
| `manager::retention_never_evicts_a_running_session` | same |
| `manager::the_exited_retention_limit_is_enforced` | same |

This is a **product finding, not a test artefact**: on Windows a shell that exits
is never reported as exited, so the manager's retention/eviction bookkeeping and
the "an exited shell stays addressable" contract (`embedded-terminal.md`) are
inert here. The repo documents that state itself — `embedded-terminal.md` line 3
says the feature is *"implemented for Linux … macOS/Windows not run"*, and its
table records Windows teardown as "not run". The fix belongs to the terminal
feature, not to this coverage task: either a per-session watcher that polls
`PtySession::try_wait()` (the normal Windows way to detect a child's exit) or a
reader that polls instead of blocking. Deliberately **not** done here — it is a
behaviour change outside "add tests and waivers", and it is reported instead (see
the handoff). The three manager tests cannot be restructured around it: `Manager`
has no "close this session but keep the entry" method, so the only producer of an
`Exited` registry entry is that same `on_eof`.

### 4. Five test bugs found and fixed, one test added — all verified passing

These were *wrong tests*, not wrong code; each is fixed in place with the claim
it can actually support. Verified by running them (individually and together) in
the copy: `7 passed; 0 failed` for the terminal batch, and `1 passed` each for
the two command tests.

| test | was | is |
|---|---|---|
| `session::live_output_reaches_an_activated_viewer` | built on `settled()`, an **already-exited** session, where `attach()` rightfully removes the subscriber after sending its end event — so `on_data` could never deliver anything, and the test asserted `""` against `"first"` | builds on `spawn_idle()`, a live session, which is what the name claims |
| `cwd::a_thread_resolves_to_its_own_workspace_and_is_refused_once_deleted` | expected `THREAD_READONLY` after `store::delete_thread`, but that function **hard-deletes** the row (`DELETE FROM threads`, `threads.rs`), so the refusal is `THREAD_NOT_FOUND` | asserts the real refusal, with the `readonly`/`deleted_at` guard documented as unreachable (see WAIVED) |
| `server::the_request_parser_is_bounded_and_reassembles_split_bodies` | sent its split-body and truncated-body requests **without** the bearer token, so every case came back `401 Unauthorized` and none of the parser's body paths ran (the first assertion failed on `response: HTTP/1.1 401`) | sends `authorization: Bearer {token}` in both heads, so the body loop, the reassembly and the truncation rejection are actually reached |
| `runs::tool_outputs_are_mapped_including_the_error_shape` | scripted the mock's **`list_tool_calls`** slot, but a single tool call is read through **`get_tool_output`** (`agent_bridge::queries::query_tools` picks the command by whether a `tool_call_id` is present), so the mock answered with an empty payload and `assert_eq!(outputs.len(), 1)` failed with `left: 0, right: 1` — the mapping under test never ran | scripts `get_tool_output`; all three payloads (ok / error / absent) now drive the real mapping |
| `remote::remote_start_propagates_a_local_failure` | **a weak test that passed for the wrong reason.** It asserted only `assert!(result.is_err())` on a fresh HOME, which is satisfied by the unrelated *"Not signed in to FutureOS."* error — and its `#[cfg(windows)]` arm then panicked on its own cleanup (`metadata()` on the credential file that `remote_start` had just deleted), because a read-only *file* is deletable on Windows (Rust clears the read-only attribute first) | keeps the legacy pre-v2 credential so `remote/supervisor/start.rs`'s PA003 branch runs, and asserts the pair of facts only that branch produces: the fault is the **local** one (`Not signed in`), and the unusable credential is **gone** afterwards. The `cfg` split is gone; one platform-neutral body now runs everywhere |

Added: `server::end_to_end::the_connect_route_authenticates_before_it_upgrades`
— the WebSocket route's own authentication boundary, which is where a browser
cannot send a header: `POST /terminal/<id>/connect` → 405 (method checked
explicitly), `GET` without an upgrade → 400, a real handshake with no ticket →
403, with a never-issued ticket → 403, for an unknown session → 404, and with a
freshly issued ticket → 101. `refused_handshake_status` is the helper.

### 5. The report is regenerated — the OPEN rows are now real, not stale

The 6th attempt's reading of `coverage/llvm-cov-tauri.json` (04:39:48) was that
its uncovered lines landed in test bodies of tests that exist now, so the OPEN
list was mostly measurement rot. That was right, and it is now moot: the report
has been regenerated from this subtree's own runs, and the two failing tests it
was complaining about (`commands/runs.rs`'s mapper, `commands/agent.rs`'s
wrapper) are fixed and covered. The counts in the OPEN section are measured
against the new file, so **do not** treat them as stale, and do not go looking
for the old "810 uncovered".

What did survive from that analysis is the reason the numbers moved so much:
`commands/runs.rs` went 727/797 → **784/797** and `commands/agent.rs`
178/268 → **247/268** purely by *running* the tests the stale file had never
attributed, which is the same failure mode §1/§2 describe (a failed or
unattributed run reports code as unexecuted when it executed).

### 6. What is still genuinely open, and what §3 costs

The six `terminal::*` failures (§3) are the only failures in this subtree that
this run did **not** close, and they are a **product** decision, not a test one:

* Fixing it means detecting a child's exit on Windows by something other than
  PTY EOF — `PtySession::try_wait()` is polled successfully by
  `pty::tests::spawns_a_real_pty_and_reports_the_exit_code`, so a per-session
  watcher is the obvious shape. That makes the six tests pass unchanged and makes
  the manager's retention/eviction bookkeeping measurable on Windows.
* It is **not** done here on purpose. `Session`'s documented invariant is that
  `Exited` is delivered *after* the last output byte (the reader thread is the
  only writer of output), and a watcher that polls `try_wait()` can observe the
  exit while the reader is still draining the child's final bytes — a possible
  ordering regression on every platform, introduced blind, in production code,
  with no way to measure it here. That is the wrong trade for a coverage task.
* If the answer is "leave the exit signal alone", those six tests cannot be made
  green on this host and the reader-driven `on_eof` transition stays an
  `unreachable-in-this-environment` waiver (see WAIVED).

### 7. Handoff order for the next run

The gate now fails on **one** thing only: the 10 OPEN files (351 lines of
reachable code with no test yet). Everything else — the measurement, the
absent-file check, the waiver coverage of the other 11 uncovered files — is
green. Cheapest first:

1. `commands/files.rs` 40, `commands/threads.rs` 31, `commands/agent.rs` 21,
   `terminal/test_support.rs` 4, `commands/workspaces.rs` 2,
   `commands/terminal.rs` 2, `commands/login.rs` 4 — ordinary command-layer
   tests; the `agent.rs` one is the `ipc_harness` follow-up the old doc already
   described, and `terminal.rs`/`login.rs` need one `LLVM_PROFILE_FILE` per
   process when re-measuring (§ the recipe: their children inherit the parent's
   path, which is why they read as uncovered here).
2. `terminal/server.rs` 76 — the `pump` timing arms; deserves its own harness.
3. `terminal/manager.rs` 93 and `terminal/session.rs` 67 — **decide §3 first**,
   because most of both files is the retention/exit machinery that only a
   detected child exit can reach. Fixing the exit signal turns that machinery
   measurable; leaving it alone means those lines stay OPEN, not waived.
4. Re-measure with the recipe above (skipping the six §3 tests), then re-run the
   gate. `coverage/tauri-tau-terminal-report.json` is the file the gate prefers
   for this group, so keep writing there.
5. **Unblock the in-tree build** when convenient — one line in
   `src/scheduler/mod.rs` (not this task's file): make `start` generic
   (`,<R: tauri::Runtime>(app: tauri::AppHandle<R>)`). Only then can
   `cargo llvm-cov` be used instead of the manual recipe, and only then can the
   other three `desktop/src-tauri` groups be measured at all.

Run harness (outside the repo, so not a deliverable): `%TEMP%\cov100-tt\` holds
the crate copy and the scripts — `measure-final.ps1` / `remeasure.ps1` (the
working recipe), `lcov-to-report.py` (LCOV → the gate's JSON shape) and
`unc.py` / `unc-lines.py` (per-file uncovered counts and line ranges, from the
report and from the LCOV respectively).

> ### Superseded run status (4th attempt). `gate-green` is NOT established.
>
> Current state of `python .future/cov100/verify.py rust-module tau-terminal 99.99 \
> docs/testing/module-tauri-terminal.md coverage/llvm-cov-tauri.json`:
>
> ```
> desktop-tauri/tau-terminal: 90.8402% lines (8033/8843) across 30 files in 3 dir(s),
>   810 uncovered line(s) in 21 file(s), target 99.99%
> FAIL: 6 undocumented uncovered file(s) in desktop-tauri/tau-terminal
> ```
>
> The six files still without a waiver are `desktop/src-tauri/src/commands/login.rs`,
> `desktop/src-tauri/src/commands/terminal.rs`,
> `desktop/src-tauri/src/terminal/cwd.rs`,
> `desktop/src-tauri/src/terminal/manager.rs`,
> `desktop/src-tauri/src/terminal/session.rs` and
> `desktop/src-tauri/src/terminal/test_support.rs` — see **OPEN** below. Every
> other file in the subtree is either covered or waived above.
>
> **(A) The real tree still does not compile for tests.** Another worker's
> *in-flight, uncommitted* change to `desktop/src-tauri/src/scheduler/mod.rs`
> (+252 lines, outside this task's write set) does not compile:
>
> ```
> error[E0308]: mismatched types
>    --> src\scheduler\mod.rs:410:15
>     |
> 410 |         start(handle);
>     |         ----- ^^^^^^ expected `AppHandle`, found `AppHandle<MockRuntime>`
>     |
>     = note: expected struct `AppHandle<tauri_runtime_wry::Wry<EventLoopMessage>>`
>                found struct `AppHandle<MockRuntime>`
> error: could not compile `futureos` (lib test) due to 1 previous error
> ```
>
> `scheduler/mod.rs`'s `mod tests` is gated on `#[cfg(test)]` only, and the crate
> has exactly one lib test target, so `cargo llvm-cov` from the tree fails before a
> single test runs. Fix needed (one line, in a file this task may not write): make
> `start` generic — `pub fn start<R: tauri::Runtime>(app: tauri::AppHandle<R>)` —
> or drop that test. The 04:39:48 report therefore did **not** come from a plain
> run in this tree; ask the supervisor for its provenance before treating its
> number as authoritative.
>
> **(B) A faithful out-of-tree run of the suite was made and its report was
> unusable, so it was deleted rather than published.** A copy of the crate outside
> the repo (crate sources + `packages/rpc` + `packages/remote-crypto` + the root
> `Cargo.toml` with only those two members + `rust-toolchain.toml` +
> `desktop/web/index.html`, the last two required by `build.rs` and
> `diagnostics.rs`), with the one-line `scheduler` genericity fix applied **to the
> copy only** and the real tree's warm `target/cov-tau-terminal` reused, builds and
> runs the suite for real:
>
> ```
> test result: FAILED. 1393 passed; 23 failed; 3 ignored; 0 measured; finished in 1299.17s
> ```
>
> Its report was broken — **143 file entries, only 3 with any coverage** (18 of 9176
> lines: `src/main.rs`, `commands/terminal.rs`, `terminal/server.rs`, i.e. processes
> other than the main lib-test process). The main lib-test process's profiles (six
> `*.profraw` of 514 920 bytes plus one of 1 152 632 bytes, all present under
> `llvm-cov-target/`) were not attributed, and re-running only the merge + report
> step reproduced 18/9176 exactly. Because the gate prefers this group's own report
> when it exists, leaving that file in place would have made the gate read
> **0.1962%** instead of the real number, so
> `coverage/tauri-tau-terminal-report.json` was **deleted**. If the 3-of-143 shape
> recurs, the profraw-to-object mapping for the `futureos_lib` test binary is what
> to investigate (`--no-clean` does not change it).
>
> **(C) The environment finding behind the 23 failures.** On this host a PTY child
> emits `ESC [ 6 n` (a cursor-position query) as it starts and then **blocks until
> a client answers it**: measured directly,
> `pty::tests::output_is_readable_through_the_master` observed exactly
> `"\u{1b}[6n"` for its full 20 s deadline and nothing else, and `cmd /C exit 7`
> stayed `STILL_ACTIVE` for 20 s. `terminal/server.rs`'s pre-existing end-to-end
> test already works around this by answering the query; the fixtures now do the
> same (`terminal::test_support::DSR_QUERY` / `DSR_REPLY` /
> `asks_for_the_cursor`). That turned the first run's 14 failures into a smaller,
> different set; **23 failures remain** and closing them is the first job of the
> next run.
>
> No regression is hidden in those 23: before this change every
> `pty.rs`/`session.rs`/`manager.rs` test here was `#[cfg(unix)]`, so on Windows
> the number of tests exercising that code was **zero**. They are new tests failing
> on first contact with a real ConPTY, not previously-green tests going red.

Rows are split into **WAIVED** (category + reason, established from source and
independent of a re-run), **NEW TESTS** (expected covered — must be confirmed by
the re-run) and **OPEN** (reachable and not yet covered, deliberately *not*
waived).
> *in-flight, uncommitted* change to `desktop/src-tauri/src/scheduler/mod.rs`
> (diff `+252` lines, outside this task's write set) does not compile:
>
> ```
> error[E0308]: mismatched types
>    --> src\scheduler\mod.rs:410:15
>     |
> 410 |         start(handle);
>     |         ----- ^^^^^^ expected `AppHandle`, found `AppHandle<MockRuntime>`
>     |
>     = note: expected struct `AppHandle<tauri_runtime_wry::Wry<EventLoopMessage>>`
>                found struct `AppHandle<MockRuntime>`
> error: could not compile `futureos` (lib test) due to 1 previous error
> ```
>
> `scheduler/mod.rs`'s `mod tests` is gated on `#[cfg(test)]` only, and the
> crate has exactly one lib test target (`futureos_lib`), so **every** tauri
> worker's `cargo llvm-cov --no-report` fails at compile time before a single
> test runs. `coverage/llvm-cov-tauri.json` therefore still holds the 02:09
> pre-change run, and every "expected covered" row below is **unverified**.
>
> Fix needed (one line, in a file this task may not write):
> `scheduler/mod.rs` must either make `start` generic
> (`pub fn start<R: tauri::Runtime>(app: tauri::AppHandle<R>)`) or drop that
> test. Reproduce with `cd desktop/src-tauri && cargo check --tests`.
>
> What *is* verified about the real tree: `cargo check --tests` reports **this
> task's files as clean** — the only error it emits is the `scheduler/mod.rs` one
> above.
>
> **(B) An out-of-tree measurement runs the suite but produces a report whose
> attribution is broken, so there is still no trustworthy number.**
>
> A faithful copy of the crate was built outside the repo (crate sources +
> `packages/rpc` + `packages/remote-crypto` + the root `Cargo.toml` with only
> those two members + `rust-toolchain.toml` + `desktop/web/index.html`, the last
> two required by `build.rs` and `diagnostics.rs`), with the one-line
> `scheduler` genericity fix applied **to the copy only** and the real tree's warm
> `target/cov-tau-terminal` reused. That copy builds, and the suite really runs:
>
> ```
> test result: FAILED. 1393 passed; 23 failed; 3 ignored; 0 measured; finished in 1299.17s
> Finished report saved to coverage/tauri-tau-terminal-report.json
> ```
>
> The report is unusable as a coverage measure: **143 file entries, of which only
> 3 carry any coverage** (18 of 9176 lines). The three are `src/main.rs` (the
> *bin* test binary's own tests), `commands/terminal.rs` and
> `terminal/server.rs` — i.e. processes other than the main lib-test process. The
> main lib-test process's own profiles (six `*.profraw` of 514 920 bytes plus one
> of 1 152 632 bytes, all present under `llvm-cov-target/`) are not attributed.
> Re-running only the merge + report step (`cargo llvm-cov report --json`) against
> the same target dir reproduces 18/9176 exactly.
>
> This is the "all-zero / broken measurement" condition, **not** a coverage
> result: it must not be read as "tau-terminal is at 0.2%". The 02:09 baseline
> (90.0845%) stays the only meaningful number for this subtree, and it predates
> every test in this change.
>
> Consequence for the next worker: measure in the **real** tree once (A) is
> fixed. If the same 3-of-143 shape appears there, the profraw-to-object mapping
> for the `futureos_lib` test binary is what to investigate (candidates: profiles
> flushed as the merge starts, or a build-id/signature mismatch after relink).
> `--no-clean` does not change it.
>
> **(C) The environment finding behind the 23 failures.** On this host a PTY child
> emits `ESC [ 6 n` (a cursor-position query) as it starts and then **blocks until
> a client answers it**: measured directly,
> `pty::tests::output_is_readable_through_the_master` observed exactly
> `"\u{1b}[6n"` for its full 20 s deadline and nothing else, and `cmd /C exit 7`
> stayed `STILL_ACTIVE` for 20 s. `terminal/server.rs`'s pre-existing end-to-end
> test already works around this by answering the query; the fixtures now do the
> same (`terminal::test_support::DSR_QUERY` / `DSR_REPLY` /
> `asks_for_the_cursor`). That is what turned the first run's 14 failures into a
> smaller, different set — **23 failures remain** and closing them is the first
> job of the next run.
>
> Note on regressions: none of the 23 is a regression. Before this change every
> `pty.rs`/`session.rs`/`manager.rs` test in this subtree was `#[cfg(unix)]`, so
> on Windows the count of tests exercising that code was **zero**. These are new
> tests failing on first contact with a real ConPTY, not previously-green tests
> going red.
