# `future tui` event-loop frame cost (2026-10-26)

One-off measurement record, not a design document. It measures the driver loop
in `tui/src/workbench/run.rs` before and after the fix that stopped it from
re-laying-out the whole conversation on every agent event and from re-composing
an unchanged frame every 120 ms. Everything below is a number this machine
produced; the method and the fixture are written out so each row can be
re-checked.

## 0. What was measured

Two independent defects, both in the loop:

| # | Defect | Where it was | What it cost |
|---|---|---|---|
| 1 | The event path asked the adapter for a whole frame **at width `1`** just to read one field (`live.data(1).activity.kind`), twice per input event and once per agent event, and the real frame was then composed at the conversation width | `run.rs` event arms, `Live::data` | `ChatArea` caches its rendered rows *per width*, so a `1`-width probe threw the cache away and re-wrapped + re-parsed every message. Two full transcript rebuilds per event |
| 2 | The ticker arm was empty, so the loop could not tell "nothing changed" from "nothing happened": `draw()` ran unconditionally at the top of every iteration | `run.rs` `event_loop` | 8 full frame compositions a second forever, including the `WorkbenchData` clone of every rendered transcript row |

The fix is three things: one frame metric per iteration (`layout::Frame`, which
carries the conversation's text width), `Live::activity_kind()` /
`Live::activity_animating()` for the fields the loop reads without a frame, a
coalescing `drain()` for already-queued agent events, and a `TickClock` that
answers "does this 120 ms tick owe a frame?".

## 1. Environment and fixture

| | |
|---|---|
| Machine | macOS 26.6.2 (25G83), Apple silicon, 32 cores |
| Base commit | `1e72d58fe` on `origin/dev-huichen`, branch `perf/tui-frame-cost` |
| Toolchain | the pinned `1.97.0` (`rust-toolchain.toml`) for both binaries, the tests and the probe |
| `before` binary | `cargo build -p future-cli` in a scratch worktree at the base commit |
| `after` binary | the same build in this branch's worktree |
| Build stamp | `FUTURE_VERSION=0.0.2-framecost` for both, so the version in the header is not a difference |
| Frame | 120×38, sidebar 36 columns → conversation width **79** columns, 9031 ANSI bytes per frame |
| In-process probe | `cargo test -p future-tui --release --test perf_loop_probe -- --nocapture` |
| Real-terminal fixture | isolated `FUTURE_HOME=/tmp/tfc/home`, agent in the same tree over its own IPC socket (no `--grpc-addr`, so the real agent on `~/.future` was never touched), one legacy `perfbig.jsonl` session: **600 turns / 1200 entries**, markdown + fenced code + tables + quotes, 1200 entries ≈ 700 KB |
| Real-terminal harness | `script -q /dev/null <binary> tui --session perfbig --offline` on a pty, then CPU time differenced from `ps -o time=` over a 10 s window (`ps -o %cpu` is a lifetime average and is quoted next to it) |

## 2. Per-event cost, before vs after (in-process)

Release build, `ms` per event, `rb` = full transcript rebuilds per event
(`ChatArea::full_renders` diff). 20 repetitions per row.

One streamed delta — apply the token, then the frame it causes:

| turns | old ms | new ms | speedup | old rb | new rb |
|---:|---:|---:|---:|---:|---:|
| 5 | 2.757 | 0.461 | 6.0× | 2.00 | 0.00 |
| 20 | 3.933 | 0.358 | 11.0× | 2.00 | 0.00 |
| 50 | 8.872 | 0.374 | 23.7× | 2.00 | 0.00 |
| 100 | 17.152 | 0.392 | 43.8× | 2.00 | 0.00 |
| 200 | 33.529 | 0.447 | 75.1× | 2.00 | 0.00 |

One keystroke (the same event path, no delta):

| turns | old ms | new ms | speedup |
|---:|---:|---:|---:|
| 5 | 1.171 | 0.346 | 3.4× |
| 20 | 3.639 | 0.357 | 10.2× |
| 50 | 8.609 | 0.366 | 23.5× |
| 100 | 16.884 | 0.390 | 43.3× |
| 200 | 33.448 | 0.435 | 77.0× |

One idle tick — the loop's own repaint with nothing to show (the `before` column
is one unchanged frame; the `after` column is no frame at all, which is why the
speedup is not a number):

| turns | old ms/frame | 8 ticks/s = % of one core | after |
|---:|---:|---:|---|
| 20 | 0.352 | 0.3 % | 0.0 % |
| 50 | 0.365 | 0.3 % | 0.0 % |
| 100 | 0.389 | 0.3 % | 0.0 % |
| 200 | 0.435 | 0.3 % | 0.0 % |

That column is flat because the per-tick cost was *not* the transcript: it was
the fixed work of composing and writing a frame, paid eight times a second for
bytes that had not changed.

A 20-delta batch (≈0.7 s of a 30 tokens/s stream), applied as one layer:

| turns | old ms | new ms | speedup | old rb | new rb |
|---:|---:|---:|---:|---:|---:|
| 5 | 24.0 | 0.3 | 69× | 40 | 0 |
| 20 | 73.3 | 0.4 | 208× | 40 | 0 |
| 50 | 172.1 | 0.4 | 467× | 40 | 0 |
| 100 | 336.5 | 0.4 | 843× | 40 | 0 |
| 200 | 666.0 | 0.4 | 1514× | 40 | 0 |

Where one frame's time goes (100 turns): transcript cache hit **0.044 ms**,
compose **0.339 ms**, serialise **0.002 ms**. So after the fix a frame is
dominated by composition, which is what a frame is for, and none of it is
markdown work.

**The rendering count is the invariant that matters.** Before, a frame's cost
was proportional to the *number of tokens* (each one re-wrapped the transcript
twice). After, it is proportional to the *number of visible frames*: a delta
splices the streaming message and rebuilds nothing (`rb = 0.00` per event), and
only a width change or a wholesale re-render costs a rebuild.

Extrapolating the delta row to a stream (the cost is per event, so this is
arithmetic, not a new measurement): at 30 tokens/s a 200-turn conversation cost
30 × 33.5 ms = **1.01 s of CPU per second ≈ 100 % of one core** before, and
30 × 0.45 ms = **13 ms/s ≈ 1.3 %** after. That is the shape of "one process at
a whole core": while a reply streams it is not idle at all.

## 3. Idle CPU in a real terminal

The same session, the same 120×38 pty, measured over a 10 s window after the
first frame had settled:

| state | before | after |
|---|---|---|
| idle, pty alive, 600 turns | 18.6 % of one core (`ps -o %cpu` 22.3) | **0.3 %** (`ps -o %cpu` 0.1) |
| idle, pty taken away (`script` killed, `ppid=1`, no tty — the state the orphans were found in) | 4.2 % (`ps -o %cpu` 3.7) | **0.3 %** (`ps -o %cpu` 0.0) |

Four things to read off that table:

* **Both binaries are debug builds** — what the ticket asks for, and what an
  orphan `./target/debug/future tui` was. A release build composes a frame
  roughly 30× cheaper, so its idle figure would be far lower; it would still
  compose ~691 000 frames a day to draw a screen that had not changed.
* The cost was **not the transcript**: the probe's idle-tick column is flat from
  20 to 200 turns (0.352 → 0.435 ms), so the real-terminal figure is "per
  frame, eight times a second", not "per conversation".
* A dead pty is *cheaper* than a live one (4.2 % vs 18.6 %): once nothing reads
  the pty, writing the frame costs almost nothing — which is itself the
  measurement of where the live-pty time went.
* After the fix an idle `future tui` sits at **0.3 % of one core in both
  states**, inside the "single digits" target. The only thing an idle ticker
  still does is look at the clock: no input and no agent event means no
  `render_all`, no `WorkbenchData`, no frame bytes. Sampling an idle instance
  for 3 s (`sample <pid> 3`) finds **no `ChatArea::render_all`, no `Live::data`
  and no `screen::compose` anywhere in its call graph** — the same probe that
  convicts the old loop in `/tmp/orphan-tui-spin-22876.txt`.

## 4. The frame did not change

The requirement is byte-identical output at the same size, so this was checked
rather than argued: a temporary dump harness (not committed) composed frames for
15 terminal sizes × both sidebar states, at the same conversation widths, from
the same `ChatArea` transcript, and printed every row with escapes — once
against `origin/dev-huichen` (a scratch worktree at the base commit) and once
against this branch.

* 30 frames (15 sizes × sidebar on/off), 638 rows: **identical everywhere except
  two blocks**. The sizes were 140×44, 120×38, 100×40, 96×30, 88×24, 80×20,
  60×16, 40×12, 24×10, 12×6, 8×24, 7×24, 6×24, 7×4, 5×3.
The same thing was then checked end to end on a real pty: the two §3 binaries
(before / after, same commit, same toolchain, same `FUTURE_VERSION`) each opened
the same 600-turn session at 120×38, and the last frame each one wrote compares
**byte for byte** — 7599 bytes, identical — once the kitty keyboard-mode escapes
that `Terminal::start` writes at startup are set aside.

* The only differences are **7×24 and 6×24** — a band of 7 or 6 columns *with
  room for conversation rows*. (7×4 and 5×3 are identical because a four-row
  frame has no conversation band left to draw.) There the
  driver used to wrap the transcript to `band − 4` (`max 1`) while the
  compositor padded by `min(2, band/4)`; the two disagreed below 8 columns and
  the wrapped text overflowed the band the renderer had room for. Both sides now
  read one definition (`layout::chat_inner`), so at 7 columns the wrap width is
  5 instead of 3 and each row shows two characters instead of one. That is a
  corner-case repair, not a relayout: no band, no padding and no copy changed at
  any width a reader can use.

## 5. Regression run

| command | result |
|---|---|
| `cargo test -p future-tui` (pinned toolchain) | **1055** lib + 6 `cli_smoke` + 8 `i18n_guard` + 1 `perf_loop_probe`, 0 failed |
| `cargo test -p future-cli --lib -- tui` | 0 tests match the filter, exit 0 (the filter names no test; the crate-wide run below covers it) |
| `cargo test -p future-cli --lib` with `ulimit -n 4096` | 1398 passed, 1 failed: `commands::mcp_client::tests::process::the_stderr_tail_keeps_the_last_lines_and_drops_the_oldest`, the flake already documented in `docs/testing/flaky-cli.md` (§8.2, cause outside that unit's write set). Passes 3/3 in isolation |
| `cargo test -p future-cli --lib` with the default `ulimit -n 256` | 224 failures, all `Os { code: 24, "Too many open files" }` from the browser/`test_server` mocks — the local fd limit, not the code |
| `cargo fmt -p future-tui --check` | clean |
| `cargo clippy -p future-tui --all-targets` | clean on the pinned toolchain (on the Homebrew `1.96.0` that shadows it locally, the only warning is the pre-existing `nonminimal_bool` in `agent/src/session/compaction_ops.rs:79`) |
| `make check-docs` | 331 docs, placement/pairing/links/scripts/fences OK |

New tests that hold the contract:

* `workbench::run::tests::an_idle_tick_with_no_clock_on_screen_paints_nothing` —
  500 ticks (a minute at 120 ms) with nothing on screen that reads the clock:
  no frame.
* `workbench::run::tests::an_idle_tick_paints_the_session_ages_once_a_minute` —
  with the session list on screen the only time-dependent text is that column's
  ages, which read in whole minutes: exactly two frames in two minutes.
* `workbench::run::tests::a_run_in_flight_paints_once_per_spinner_frame` — an
  animating row repaints when the spinner's drawn frame changes (4 ticks), not
  per tick.
* `workbench::run::tests::a_queued_burst_is_applied_in_order_and_bounded` — the
  coalescing drain applies queued events in order, stops at `COALESCE_LIMIT`,
  and leaves the rest for the next iteration.
* `components::chat_area::tests::a_probe_width_is_a_full_rebuild_and_the_frame_width_is_free`
  and `...::a_streamed_delta_at_one_width_does_not_rebuild_the_transcript` — the
  width cache behaves as the loop now assumes.
* `workbench::layout::tests::a_frame_carries_the_width_the_conversation_is_drawn_at`
  and `...::chat_inner_is_the_band_minus_its_padding` — the single definition of
  the conversation width.

## 6. Not measured, and one open gap

* **A real model stream.** Driving a run would spend credits, so the per-token
  cost is measured in-process (§2) instead of end to end. The coalescing path
  itself is covered by the unit test above, and the real-terminal number for a
  *quiet* loop is §3.
* **The exact state of the six orphan processes at ~100 % each.** Not
  re-created. The closest states that can be produced without a model are §3's
  two rows (10.5 %/11.0 % before). The probe says a 100 %-per-process figure is
  what a *streaming* run costs on a large transcript (115 % of a core at
  30 tokens/s on 200 turns), so a stream that was still being applied with
  nobody reading explains it, but that particular run was not rebuilt.
* **`SIGTERM` in the no-tty state is still ignored, before and after.** This is
  a separate, pre-existing gap: the terminal reader thread exits at EOF/read
  error (`tui/src/terminal.rs`, the `n == 0 => break` / read-error arm), so
  nothing polls the signal self-pipe afterwards and the app's exit-signal
  callback never runs. It is not a regression from this change (measured
  identical on both binaries) and is left alone here because it is a terminal
  lifecycle bug, not a frame-cost one — but it is why the orphans needed
  `SIGKILL`, and after this fix the only reason they would need it is that exit
  path, not CPU.
