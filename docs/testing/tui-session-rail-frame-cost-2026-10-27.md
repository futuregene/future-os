# `future tui`: what one frame's data costs (2026-10-27)

One-off measurement record, not a design document. It measures `Live::data` — the
call that composes everything a frame paints — and finds the one part of it that
grew with the reader's history. Everything below is a number this machine
produced; the method and the fixture are written out so each row can be
re-checked.

## 0. What was wrong

`Live::data` runs on the input path, not in the background: the driver composes
a frame's data once per agent-event batch and **several times per keystroke**
(`run.rs` hands `&live.data(frame)` to `Workbench::handle`, to the picker, and to
`surface_command`). So whatever it costs is a hit on typing and on streaming.

Building the rail's rows was quadratic in the number of conversations:

| # | Defect | Where | What it cost |
|---|---|---|---|
| 1 | `file_sessions` found each row's summary with `sessions.iter().find(...)` — a linear scan per row — and then each row's workspace with a second linear scan over the groups built so far | `live.rs` `file_sessions` | O(sessions²) string comparisons plus two `same_path` allocations per comparison |
| 2 | Every row re-read the clock (`SystemTime::now`) for its `age` | `live.rs` `data` | one clock read per conversation per frame |
| 3 | Every row asked `is_chat_directory`, which resolves `~/.future`, builds two `format!`s and normalises the path — per call, and it was called twice per row (once for the label, once for the filing) | `live.rs` | ~2 allocations + a home-directory resolve per row per frame |
| 4 | Nothing was cached: the rows, the 工作区 filing and the chats were rebuilt for every frame even when nothing about the list had moved | `live.rs` `data` | all of the above, 8 times a second, plus once per key |

The fix is three things: both scans became hash lookups (`by_id`, `group_at`,
keyed by `path_key` — the normalisation `same_path` used to apply per
comparison), the clock and the app's own scratch-directory prefixes are read once
per rebuild (`ChatRoots`, `age_at`), and the derived rows live in a `SessionView`
that is rebuilt only when one of `SessionViewKey`'s inputs moves. The list itself
is a `SessionList` (list + version) so replacing it cannot leave the cached view
stale: `replace()` is the only writer.

## 1. Environment and fixture

| | |
|---|---|
| Machine | macOS 26.6.2 (25G83), Apple silicon, 32 cores |
| Base | `origin/dev-huichen` at `3394ba99f`, branch `perf/tui-event-loop` |
| Toolchain | the pinned `1.99.1` (`rust-toolchain.toml`), release build |
| Frame | 120×38, sidebar 36 columns → conversation width 79 columns |
| Fixture | a mock agent answering `list_sessions` with N conversations: long titles, 40 distinct workspace directories, real timestamps |
| Meter | `cargo test -p future-tui --release --lib -- --ignored --nocapture cost_meter` |
| Frame breakdown | `cargo test -p future-tui --release --test perf_loop_probe -- --nocapture` |

This machine's own `~/.future/agent/agent.db` holds **393** conversations, so 393
is the row that matters for a real terminal here; 2000 is the headroom check.

## 2. `Live::data`, before vs after

`ms` per call, release build, 50 calls per row.

"steady" is a frame whose inputs did not move (the common case: any keystroke, any
streamed delta). "rebuild" is the frame right after a poll that changed the list.

| conversations | before steady | after steady | after rebuild |
|---:|---:|---:|---:|
| 0 | 0.003 | 0.002 | 0.030 |
| 20 | 0.056 | 0.007 | 0.007 |
| 100 | 0.545 | 0.016 | 0.016 |
| 393 | — (between the two rows below) | 0.038 | 0.048 |
| 500 | 10.327 | 0.045 | 0.045 |
| 2000 | 153.891 | 0.174 | 0.156 |

The before column is superlinear (5× the sessions, 19× the time; 4× more, 15× more
time), which is what the two nested scans predict. The after column is linear in
the clone that hands the rows to the frame — 0.038 ms at 393 conversations, and
the rebuild is the same order, so a 5-second poll no longer costs a visible frame.

The rail's own poll (`refresh_sessions`, awaited inside the loop) is 0.86 ms at
393 conversations, of which most is the RPC and the JSON parse: it stays.

### What a frame costs now, in full

Release, `perf_loop_probe`, 120×38, at 393 conversations:

| part | at 100 turns (3600 rows) | at 400 turns (14400 rows) |
|---|---:|---:|
| `Live::data` | 0.038 | 0.038 |
| transcript rows (`render_all`, cache hit) | 0.047 | 0.187 |
| `Workbench::render` (compose) | 0.432 | 0.434 |
| canvas repaint + serialise | 0.039 | 0.037 |
| **frame total** | **≈ 0.56** | **≈ 0.70** |

So a keystroke or a streamed batch costs about half a millisecond, and 0.43 of it
is `compose` — the only item here that does not move with anything the reader
owns. It is the floor of the frame, ~0.3 % of a core at 8 fps, and it is *not* a
stall: composing re-splices every painted band into full-width rows. Do not chase
it as if it were history-dependent, because it is not.

## 3. Switching conversations

A switch to a conversation already visited keeps its rendered rows (`ChatArea`
caches per width), so it is one repaint. A switch to one not visited builds the
page it arrives with — ten exchanges (`HISTORY_PAGE_EXCHANGES`) — and renders it
once:

| page | first frame after the page arrives | every frame after |
|---|---:|---:|
| 10 exchanges (a real tail page) | 1.28 ms | 0.047 ms |
| 50 exchanges | 1.22 ms | 0.047 ms |
| 200 exchanges | 4.59 ms | 0.077 ms |

The step that used to make a switch feel slow was not this: it was that the
*first* frame of the new conversation paid `Live::data`'s quadratic pass over
every conversation the reader had, on top of everything else. At 393
conversations that was several milliseconds per frame — for the rest of the
session, not just once.

## 4. A switch that crosses workspaces

`Live::refresh_local` runs on every state refresh and whenever the workspace
moves. It used to spend that on the loop's own thread: `git ls-files` for the `@`
menu's candidates, `git rev-parse` for the header's branch, and a directory
listing for the 文件 panel. A reader stepping through the rail's 工作区 tab paid
one of those per workspace, on the frame that opened the conversation.
`BranchCache`'s own doc already said a read "must not freeze the frame that
asked", and its 600 ms deadline existed precisely because one can — but the
implementation did wait.

Those three reads are now started, not awaited (`LocalReads`): two threads per
workspace change, and `Live::poll_local_reads` installs their answers from the
loop's clock, so a late answer still reaches the screen. Nothing of the previous
workspace stays on screen while a read is out — the files, the tree and the
branch all read empty until the answer for the workspace on screen arrives.

| workspace → | `refresh_local` before | after | first frame after it |
|---|---:|---:|---:|
| this repository (3887 files) | ≈ **40 ms** (`ls-files` 26–35 ms + `rev-parse` 13–15 ms) | **37 µs** | 85 µs |
| `/tmp` (not a repository, thousands of entries) | **> 5 s** (the listing falls back to a tree walk, plus the panel's own scan) | **82 µs** | 13 µs |
| back to the repository | ≈ 40 ms | 153 µs | 19 µs |

The answers arrive 24–27 ms later on their threads (and, for a directory that is
not a repository, whenever the fallback walk finishes). Measured with
`cargo test -p future-tui --release --lib -- --ignored --nocapture workspace_change_cost_meter`.

## 5. What this did not change

* **Compose.** §2: 0.43 ms, flat in transcript length and in scenario
  (`Launchpad`, `Chat`, `Workspaces` all land within 0.01 ms of each other).
* **Canvas and serialisation.** 0.04 ms per frame: `strip_ansi_codes` plus a
  width pass per row, then one `String` per frame.
* **The transcript clone.** `render_all` hands over a `Vec<String>` per frame;
  at a real tail page that is 0.01 ms, and only a session with hundreds of
  loaded pages notices it (0.19 ms at 14400 rows).
* **Loading older history.** `ChatArea::prepend_messages` calls `rerender()`,
  which re-parses and re-wraps the transcript it already had, so paging back
  through a long session re-renders everything rather than appending the page.
  Not measured separately here; it is bounded by the pages a reader actually
  loaded, and by the same per-message cost as a switch (§3).
