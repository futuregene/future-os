# tui (rust, llvm-cov) — 2026-11 coverage waivers

**Metric A** — a line counts as uncovered iff its LCOV record is `DA:<line>,0`. The
llvm-cov *summary* percentages (Lines/Functions) are not the acceptance number.

**Lane**: `cargo llvm-cov -p future-tui` (crate `future-tui`), rust **1.99.1** pinned by
`rust-toolchain.toml`, with that toolchain's own `llvm-tools-preview`.

**This file's status: no waiver is claimed yet.** Every candidate below is reachable in
principle and is waiting for a fixture, not for an exemption — the repo's rule is that a
waiver which cannot name its own falsifier is not evidence (`docs/testing/waiver-ledger.md`).
The table exists so the next wave does not have to re-derive each one, and so a reader can
see exactly which arms are *not* being excused.

## How this lane is measured

```bash
cd coverage/2026-11/after/tui        # this directory
./run-cov-tui.sh                     # DA:0 without the meters
METERS=1 ./run-cov-tui.sh            # DA:0 with the `#[ignore = "meter, not a check"]` meters run once
python3 ../../../tools/lcov-missed.py tui-after.info --filter tui/src --json after-missed.json
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

## Candidates, and why none is waived yet

| # | file:line | candidate category | the code that would have to be defeated | falsifier (what would prove it reachable) | status |
|---|---|---|---|---|---|
| 1 | `tui/src/workbench/live.rs:722-725` | **W2** | `poll_tree_read`'s `Err(std::sync::mpsc::TryRecvError::Disconnected)` arm: it runs only when the spawned reader thread ends **without** `tx.send(tree)`. Both producers (`refresh_tree`, `toggle_folder`) end with an unconditional `let _ = tx.send(tree);`, and their body is `tree.rescan()` / `tree.toggle(&path)` — pure in-memory operations that return `()`. | a panic inside `FileTree::rescan`/`toggle` (thread dies before the send), or a seam that drops the sender | **not waived** — needs a probe that pins the two reachable arms first (see #2) |
| 2 | `tui/src/workbench/live.rs` `poll_tree_read` (reachable arms) | — | the `Ok(tree)` arm and the `Empty` arm | a driver test that roots the file tree (`refresh_local()` + `poll_local_reads()`) and then `refresh_tree()` / `toggle_folder()` | **not waived** — the fixture does not land rows yet (measured 2026-11-01: `refresh_local` + `poll_local_reads` installs the files list but `tree.rows()` stays empty), so no positive control exists |
| 3 | `tui/src/workbench/live.rs:3974-4004` (`paste_clipboard`) | **W3** | the clipboard reader's `Err` arms: they need a host with no clipboard, a denied pasteboard, or an unsupported image format | a test that forces the reader to fail (the reader is `crate::clipboard::read_image`, which shells out to the platform tool) | **not waived** — the *happy* path is coverable and is the next thing to write; only its failure arms are W3 |
| 4 | `tui/src/workbench/live.rs:4417-4445` (`open_external`) | **W3** | `open`/`xdg-open`/`start` missing or failing: the `Err` arm needs a host where the opener refuses | a `PATH` without the opener, injected through `test_env::absolute_tool`'s own seam | **not waived** — the spawn path is coverable with a stub opener on `PATH` |
| 5 | `tui/src/workbench/run.rs:106-246` (`run()`) | — | not an arm: the process entry (`Terminal::start`, exit-signal callbacks, the startup update check, the persist-on-exit) | a PTY child process, which `tui/tests/signal_shutdown.rs` already demonstrates in this crate | **not waived** — coverable, just not yet |

Nothing in this file is W4 (race window), W5 (attribution artifact) or W6 (ghost line): the
attribution artifacts this crate did produce were the `cfg(test)` twins, and those are gone
from the report's `DA:0` list.

## What a waiver row must carry when it is filed

Per the ledger: `file:line`, the category, **the mechanism quoted from the source** (not a
paraphrase), the positive control — the probe test that pins the surrounding reachable code —
and the falsifier, i.e. the input or host state that would make the line execute. A row
missing the probe is a claim, not a measurement.
