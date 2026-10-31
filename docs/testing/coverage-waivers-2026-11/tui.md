# tui (rust, llvm-cov) — 2026-11 coverage waivers

**Metric A** — a line counts as uncovered iff its LCOV record is `DA:<line>,0`. The
llvm-cov *summary* percentages (Lines/Functions) are not the acceptance number.

**Lane**: `cargo llvm-cov -p future-tui` (crate `future-tui`), rust **1.99.1** pinned by
`rust-toolchain.toml`, with that toolchain's own `llvm-tools-preview`.

**This file's status: no waiver is claimed.** Both candidates that survived wave 5 are
reachable in principle and are waiting on a probe or another platform's run, not on an
exemption — the repo's rule is that a waiver which cannot name its own falsifier is not
evidence (`docs/testing/waiver-ledger.md`).

## How this lane is measured

```bash
cd coverage/2026-11/after/tui        # this directory
./run-cov-tui.sh                     # DA:0 without the meters
METERS=1 ./run-cov-tui.sh            # DA:0 with the `#[ignore = "meter, not a check"]` meters run once
python3 ../../../tools/lcov-missed.py tui-after.info --filter tui/src --json after-missed.json
python3 per_file.py after-missed.json          # per file
python3 missed_by_fn.py <source.rs> after-missed.json <path-suffix>   # per function
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

## Closed since wave 4 (they were candidates, not waivers)

| was | how it closed |
|---|---|
| `paste_clipboard`'s four answers (`live.rs:3974-4004`) | `ClipboardCapture::with_runner` is the paste module's own seam: a fake tool writes the 1x1 PNG a probe would write, and a runner that errors carries the reason. Probe: `the_clipboard_pastes_a_picture_or_says_why_it_cannot`. The `Unavailable("")` arm is reached through it too, so it is covered rather than waived. |
| `open_external`'s happy path and its "no opener" arm (`live.rs:4417-4445`) | a stub `open`/`xdg-open` script on `PATH` (under `test_env::lock` + `EnvVarGuard`), and an empty `PATH` for the failure. Probe: `opening_a_link_spawns_the_opener_and_reports_a_missing_one` (`#[cfg(unix)]`). |
| `poll_tree_read`'s two reachable arms, `toggle_folder` (`live.rs:705-725, 3830-3841`) | the fixture clears `local_reads` before `refresh_local()` (the scan only starts when the directory *changed*), so the tree lands rows. Probe: `the_file_panel_installs_a_scan_and_toggles_a_folder`. |

## The two rows that remain candidates

| # | file:line | category | the code that would have to be defeated | falsifier | status |
|---|---|---|---|---|---|
| 1 | `tui/src/workbench/live.rs:722-725` | **W2** | `poll_tree_read`'s `Err(std::sync::mpsc::TryRecvError::Disconnected)` arm. It runs only when the spawned reader thread ends **without** `tx.send(tree)`. Both producers — `refresh_tree` (`live.rs:4009`) and `toggle_folder` (`live.rs:3830`) — end with an unconditional `let _ = tx.send(tree);`, and their thread body is `tree.rescan()` / `tree.toggle(&path)`: pure in-memory operations that return `()`. | a panic inside `FileTree::rescan`/`toggle` (the thread dies before the send), or a seam that drops the sender | **not waived** — the positive control now exists (`the_file_panel_installs_a_scan_and_toggles_a_folder` pins the two reachable arms by name), so only the injection seam is missing |
| 2 | `tui/src/workbench/live.rs:4436-4441` (`open_external`, Windows half) | **platform-unmeasured** | the `cmd /C start` branch is compiled out here, and the Windows opener is a shell builtin, so a `PATH` stub cannot stand in for it. | a Windows CI run that measures this file (the branch is `cfg!(windows)`, so the line exists but only that runtime executes it) | **not waived** — it is a measurement gap: run the lane on Windows before calling it anything else |

Nothing in this file is W4 (race window), W5 (attribution artifact) or W6 (ghost line): the
`cfg(test)` twins that produced attribution artifacts in this crate are gone from the
report's `DA:0` list. `run.rs`'s `run()` (the process entry) is **not** waived either — it
needs a PTY child process, which `tui/tests/signal_shutdown.rs` already demonstrates.

## What a waiver row must carry when it is filed

Per the ledger: `file:line`, the category, **the mechanism quoted from the source** (not a
paraphrase), the positive control — the probe test that pins the surrounding reachable code —
and the falsifier, i.e. the input or host state that would make the line execute. A row
missing the probe is a claim, not a measurement.
