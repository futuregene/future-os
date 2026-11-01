# tui (rust, llvm-cov) — 2026-11 coverage waivers

**Metric A** — a line counts as uncovered iff its LCOV record is `DA:<line>,0`. The
llvm-cov *summary* percentages (Lines/Functions) are not the acceptance number.

**Lane**: `cargo llvm-cov -p future-tui` (crate `future-tui`), rust **1.99.1** pinned by
`rust-toolchain.toml`, with that toolchain's own `llvm-tools-preview`.

**This file's status: 0 waived.** Every row below is marked *not waived*, with the reason it
is not covered yet and the falsifier that would prove it reachable. A waiver that cannot name
its own falsifier is not evidence (`docs/testing/waiver-ledger.md`).

**This file is re-read against the lcov every wave, and entries a later test covered are
deleted** — a stale exemption is worse than a missing one: the next reader would start from a
premise that is no longer true. The wave-10 pass removed two (below) and corrected three line
numbers that had shifted.

## How this lane is measured

```bash
cd coverage/2026-11/after/tui        # this directory
./run-cov-tui.sh                     # DA:0 without the meters
METERS=1 ./run-cov-tui.sh            # DA:0 with the `#[ignore = "meter, not a check"]` meters run once
python3 ../../../tools/lcov-missed.py tui-after.info --filter tui/src --json after-missed.json
python3 per_file.py after-missed.json          # per file
python3 missed_by_fn.py <source.rs> after-missed.json <path-suffix>   # per function
python3 arms.py after-missed.json <source.rs> <lo>-<hi>               # which lines in a range
python3 check_claims.py after-missed.json <path-suffix> <lo>-<hi>     # is this row still true?
```

`METERS=1` adds a second instrumented pass with `--include-ignored` and merges its profile:
the crate's three cost meters (`session_rail_cost_meter`, `workspace_change_cost_meter`,
`a_switched_conversations_first_frame_cost_meter`) and `git_panel::dump_real_log` are
**instruments**, not checks — they assert nothing, so running them once at measurement time
is how their lines stop counting as missed. They are *not* waived here, and no new
`#[ignore]` was added anywhere.

Category names are the ones in `.future/memory/coverage-llvm-cov.md`: W1 cfg(windows),
**W2 defensive dead arm**, W3 OS failure injection, W4 race window, W5 attribution artifact,
W6 ghost line, W7 mock closure, W8 dead producer.

## Closed in an earlier wave (candidates, never waivers)

| was | how it closed |
|---|---|
| `paste_clipboard`'s four answers (`live.rs:3974-4004`) | `ClipboardCapture::with_runner` is the paste module's own seam: a fake tool writes the 1x1 PNG a probe would write, a runner that errors carries the reason, and the text tool's answer reaches the draft. Probe: `the_clipboard_pastes_a_picture_or_says_why_it_cannot`. |
| `open_external`'s accepted path (`live.rs:4417-4445`) | a stub `open` script **prepended** to `PATH` (under `test_env::lock` + `EnvVarGuard`) covers it. Prepending, not replacing: replacing `PATH` outright made every other test that spawns a tool fail with `NotFound` — see the row below and wave 13. Probe: `opening_a_link_spawns_the_opener_and_reports_a_missing_one` (`#[cfg(unix)]`). |
| `poll_tree_read`'s two reachable arms, `toggle_folder` (`live.rs:705-725, 3830-3841`) | the fixture clears `local_reads` before `refresh_local()` (the scan only starts when the directory *changed*), so the tree lands rows. Probe: `the_file_panel_installs_a_scan_and_toggles_a_folder`. |
| **`notify` (`run.rs:1870-1880`) — deleted in wave 10** | covered by the PTY harness: the reader's settings turn notifications on, the agent streams one approval, and the terminal receives the OSC 9 sequence. Probe: `an_approval_streamed_to_a_real_terminal_rings_the_desktop` (`tui/tests/driver_run.rs`). The row that stood here said it "needs a PTY child process"; that harness now exists, so the row was removed rather than reworded. |
| **`run()`'s process wiring (`run.rs:106-246`) — deleted as a blanket row in wave 10** | 82 of its 101 lines now execute in `the_real_entry_point_paints_a_frame_and_leaves_a_settings_file` (`tui/tests/driver_run.rs`): the terminal start, the reader thread, the first frame, the key loop and the exit-time persistence of `settings.json`. The row used to claim the whole function needed a PTY; the 19 lines that still do are listed below, by branch. |

## Reachable, but not yet taken — `run()`'s remaining branches (`run.rs`)

Measured 2026-11-01 (`python3 check_claims.py after-missed.json workbench/run.rs 106-246` →
19 lines). None of them needs a new harness: the PTY one already runs the process.

| line | branch | what the harness has to be given |
|---|---|---|
| 118-122 | the settings file could not be read → notice | a *corrupt* `settings.json` in the child's home (one byte of junk) |
| 137 | `--continue` → `attach_most_recent()` | `Options { continue_recent: true }` |
| 153 | a first message on the command line | `Options { initial_prompt: Some("…") }` |
| 162-166 | the startup update check | `Options { offline: false }` — this one reaches the **network**, so it needs an injectable manifest URL or a local stub before it is worth measuring |
| 191-192 | the reader thread's resize callback | a `TIOCSWINSZ` on the pty from the parent, or `SIGWINCH` |
| 200-201 | the reader thread's signal callback | a signal sent to the child (the `signal_shutdown` test already does this) |
| 226 | an explicit theme carried over to the next start | a theme change before quitting (e.g. the `/theme` picker's row) |
| 242-243 | `run()` returning `Err` | an agent that refuses the attach, or an unwritable settings path |

## Rows that remain candidates (still **not waived**)

| # | file:line | category | the code that would have to be defeated | falsifier | status |
|---|---|---|---|---|---|
| 1 | `tui/src/workbench/live.rs:724-725` | **W2** | `poll_tree_read`'s `Err(std::sync::mpsc::TryRecvError::Disconnected)` arm: it runs only when the spawned reader thread ends **without** `tx.send(tree)`. Both producers — `refresh_tree` (`live.rs:4009`) and `toggle_folder` (`live.rs:3830`) — end with an unconditional `let _ = tx.send(tree);`, and their thread body is `tree.rescan()` / `tree.toggle(&path)`: pure in-memory operations that return `()`. | a panic inside `FileTree::rescan`/`toggle` (the thread dies before the send), or a seam that drops the sender | **not waived** — positive control is `the_file_panel_installs_a_scan_and_toggles_a_folder`; only the injection seam is missing |
| 2 | `tui/src/workbench/live.rs:717` | **W3** | the same function's `self.notice = Some(error)` — a *directory read* failed (`FileTree.error`). | a directory under the workspace the process cannot read (mode 000, or a file where a directory is expected) | **not waived** — an ordinary `chmod` fixture on unix reaches it; Windows would need the equivalent ACL case |
| 3 | `tui/src/workbench/live.rs:3998-4004` | **W3** | `paste_clipboard`'s `spawn_blocking(..).await` **join error**: the closure is `capture.capture(os, pid, now)`, which returns a value and does not panic, so nothing produces the failure. | a panic inside `ClipboardCapture::capture` (the seam would have to be a runner that panics rather than answers) | **not waived** |
| 4 | `tui/src/workbench/live.rs:4426-4432` | **platform-unmeasured** | `open_external`'s two non-macOS branches: `Command::new("cmd").args(["/C","start","",target])` and `Command::new("xdg-open")`. On macOS `cfg!(windows)` is false at run time and `cfg!(target_os = "macos")` picks the first arm, so neither line executes here. | the same lane on Windows (the `cmd` branch) and on Linux (the `xdg-open` branch) | **not waived** — a measurement gap, not unreachability: run the lane on those platforms |
| 5 | `tui/src/workbench/live.rs:3818-3822` (with `open_external`'s `spawn()?` at `4434-4438`) | **needs its own process** | `fn open_url`: `if let Err(error) = open_external(url) { self.notice = Some(i18n::render(..)) }`. The `Err` exists only when `command.spawn()` fails, i.e. when the opener does not resolve at all: `let mut child = command.stdin(..).spawn()?;`. | a process whose `PATH` holds no `open`/`xdg-open` | **not waived** — see the two measurements below |

Row 5 was covered until wave 13, by emptying `PATH` for the length of one assertion. That is
a process-global sabotage while the rest of this suite runs in the same process, and it was
**the cause of the `run.rs` repository flake** that looked like a race: with `PATH` emptied,
any test spawning `git` fails with `Os { code: 2, kind: NotFound }` — measured 2 full parallel
runs in 3 failing (`run.rs:4397`, `run.rs:4431`), 1 in 3 with the then-new tests skipped
(which is what wrongly cleared them), and **4 of 4 passing** once `PATH` was prepended
instead. The arm is left open rather than paid for with a nondeterministic suite.

The second measurement rules out the cheap way back: shadowing the opener with a
non-executable file does **not** reach the arm — `execvp` skips a match it may not execute
and finds the real `/usr/bin/open`, so the notice stayed `None`. Reaching it needs a child
process with no opener on `PATH` (the PTY harness in `tui/tests/driver_run.rs` is the shape),
which is a measurement task for whoever owns that file.

Nothing in this file is W4 (race window), W5 (attribution artifact) or W6 (ghost line): the
`cfg(test)` twins that produced attribution artifacts are gone from `DA:0`, and a line that
reads 0 only because of a macro/brace attribution would be a bug in the report, not a waiver.

## What a waiver row must carry when it is filed

Per the ledger: `file:line`, the category, **the mechanism quoted from the source** (not a
paraphrase), the positive control — the probe test that pins the surrounding reachable code —
and the falsifier, i.e. the input or host state that would make the line execute. A row
missing the probe is a claim, not a measurement.
