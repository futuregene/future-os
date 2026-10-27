# dsh-TUI 0.17 parity inventory and FutureOS gap analysis

What dsh-TUI 0.17 actually is, what makes it feel like a workbench, and where
`future-tui` now stands against it. This is the **evidence and gap** document.
The frozen implementation contract (type names, module layout, layout constants,
keymap) is [workbench-spec.md](workbench-spec.md); the ordered change list is
[workbench-redesign.md](workbench-redesign.md); the per-requirement evidence,
with the screenshot of each state, is [verification.md](verification.md).

- **Subject A**: `@deepseek-harness-tui/dsh-tui` **0.17.0**, read from the
  installed package and from the supervisor's live captures.
- **Subject B**: `future-tui` at `dev-huichen` **130c0277d** — the commit where
  `future tui` became the workbench and the inline UI was deleted.
- **Written**: 2026-10-25 (first pass at `9d5f02beb`, this revision after the
  workbench landed).
- **Rule for this document**: every claim carries an evidence tag. A claim with
  no tag is a design proposal and is marked as one.

## 0. Evidence tags

| Tag | Meaning | Where it comes from |
|---|---|---|
| `[S:<file>]` | a captured terminal frame | `docs/tui/assets/<file>` (ours) and `.future/workbench-lab/shots/dsh_*.png` + `raw_*.txt` (dsh) |
| `[R]` | the package README | `README.md` of the installed 0.17.0 package (≈72 KB) |
| `[C:<path>]` | compiled package source | `lib/types/**` of the installed package (931 `.js` modules) |
| `[F:<path>:<line>]` | FutureOS TUI source | `tui/src/**` at `130c0277d` (branch `docs/workbench-parity`) |
| `[P]` | the RPC wire contract | `packages/rpc/proto/future.proto` |
| `[V]` | measured live this round | a tmux session against an isolated agent; commands in `verification.md` §7 |
| `[D]` | a design proposal | not implemented, not measured |

## 1. Method, and what was actually inspected

The dsh-TUI 0.17.0 build is installed on this machine as a dsh profile
dependency, so the **real package** was read rather than a marketing page:

- `~/.dsh/profiles/dsh-tui/package.json` pins
  `@deepseek-harness-tui/dsh-tui: 0.17.0`; the package tree is at
  `~/.dsh/profiles/dsh-tui/node_modules/@deepseek-harness-tui/dsh-tui`.
- The dsh captures in `.future/workbench-lab/shots/` were taken from a live
  session at 16:18–16:23 on 2026-10-25 in the chat workspace
  `~/.future/workspaces/chat/thread-20261025-160805-bca419/`. Seven PNG frames
  plus seven ANSI raw frames of the same screens, plus one `future_base.png`
  of the **pre-workbench** FutureOS TUI for side-by-side comparison. Measured
  from the raw frames, the dsh session ran in a **120-column × 38-row**
  terminal — the same 120×38 that the contract picks as its primary size.

The FutureOS side was measured, not remembered. This round the workbench was
driven live under tmux at 120×38, 90×24 and 60×20 against an isolated agent:

- `FUTURE_HOME=/tmp/dvhome future agent --grpc-addr 127.0.0.1:5511` — a fresh
  data root holding a copy of `settings.json`, `models.json`, `skills/` and
  `tasks/tasks.db`, and **no credentials** (`auth.json` was deliberately not
  copied).
- One real session (`20261021-005514-…`, 2,360 entries, 2,799 message blocks,
  16 runs) was copied into that root with `ATTACH`-and-`INSERT` — sessions,
  entries, message blocks and runs only. That is why the frames show a real
  transcript, a real `ctx 128.9k/200.0k 64.4%` and a real `≈¥18.84`, while the
  launchpad frame (a second instance, no session) shows `‹no model›`: with no
  `auth.json` the agent has a catalogue but no key, which is exactly what a
  first-run launchpad looks like.
- Panels read local state, so the todo panel shows this repository's real goal
  (`✔/◉/○/✖`, priorities, truncated text) and the jobs/skills panels read
  `tasks.db` and the installed skill directories.

Every frame in `docs/tui/assets/` is the real compositor's output, captured with
`tmux capture-pane -p -e` and rendered to PNG; nothing is a mock. `[V]`

## 2. What dsh-TUI is, in one paragraph

dsh-TUI is a **React/Ink terminal application** (its own reconciler, Yoga
layout, `[R]`) that wraps an agent backend — DSH, or Claude/Codex through
adapter kernels — and presents one continuous surface: a landing page with a
real composer, a transcript with clickable tool cards, a persistent side column
of live panels, and a status bar carrying model, effort, cache-hit rate, session
cost and context occupancy `[S:dsh_chat]`. It is a *client with a design system*,
not a chat log with prompts: the pixel whale, the block wordmark, the panel
symbols and the nine settings groups are all part of one kit
`[C:lib/types/components/design-system]`, and it is bilingual (zh/en)
throughout `[S:dsh_setup1]` vs `[S:dsh_setup3]`. Its own README is explicit that
it changes nothing in the backend: "zero core changes: it consumes only DSH's
public exports" `[R]`.

That constraint is the same one FutureOS is under (`packages/rpc` is the only
wire contract, and this work must not touch it), which is why it is a useful
reference rather than a competitor.

## 3. Feature inventory (0.17)

### 3.1 Launch and first run

| Capability | Evidence |
|---|---|
| Every ordinary launch lands on a **launchpad**: two stacked 5-row block wordmarks (`DSH`, `TUI`), a large bordered composer with a real caret (`❯ 说点什么，或输入 / 看命令…`), one parameter line `deepseek-flash · Max · standard · 工作区读写`, then cwd + `dsh-tui v0.17.0`, then backend lines (`▸ DSH · dsh-core v0.2.1-rc.2`, `Claude · 未安装 · 按 Enter 安装`, `Codex · 未安装`) | `[S:dsh_launchpad]` |
| The launchpad composer is a **real input**, not a splash: typing, `/` completion, `Tab` fill, `Enter` runs/clears, `@` attachments and image paste all work there; `Shift+Enter` opens chat with the draft intact | `[R]` |
| Four-step first-run wizard: ① API key + connectivity ② language + theme ③ model + effort + workspace ④ shortcuts and commands; breadcrumb shows progress; `←/→` steps, `Enter` acts, `Esc` skips | `[S:dsh_setup1]`, `[S:dsh_setup3]` |
| The wizard re-runs on demand with `/setup`; language switches live between English and Chinese | `[R]`, `[S:dsh_setup1]` vs `[S:dsh_setup3]` |
| Launchpad is on by default and can be turned off (`/settings` → `launchpad` off boots straight into chat) | `[R]` |
| Three startup intros and a whale that wakes on click; the whale freezes after the first task | `[R]` |

### 3.2 Shell and header

| Capability | Evidence |
|---|---|
| Five-row header: pixel whale, three-row block wordmark, `deepseek-flash · Max effort`, the session cwd, then a rotating tip line | `[S:dsh_chat]` |
| The header is the first of three liveness layers (header, activity line, status bar) — see §4.1 | `[S:dsh_chat]`, `[R]` |
| Everything visible is inside one alt screen; there is also an inline mode, where the transcript stays in the terminal's own scrollback and mouse selection is the terminal's | `[R]` |

### 3.3 Transcript

| Capability | Evidence |
|---|---|
| Role glyphs and one visual rhythm: `❯` user, `⏺` assistant, `• Bash(uname -srm; …) · 0s` for a tool call with `⎿` for its output, `⚓ 思考 （ctrl+o 展开）` for a collapsed reasoning block | `[S:dsh_chat]` |
| Code blocks get a titled frame (`┌─ code ─…`) and their own background; CJK prose wraps at exactly two cells per glyph with no ragged columns | `[S:dsh_chat]` |
| Streaming Markdown, tool cards, `/` and `@` completion, `#L12-14` line ranges, history search | `[R]` |
| Diff cards: split two-column by default, **one-sided** files get the full body width instead of an empty pane; folding/streaming reveal is per file | `[R]` |
| A clickable **timeline rail**: every turn is a tick; timeline / scrollbar / hidden gutter modes | `[R]` |
| Mermaid fences drawn as Unicode diagrams; LaTeX rendered as Unicode or, on graphics terminals, as images | `[R]` |
| Images: Kitty and Sixel thumbnails, centred preview with zoom and pan, paste-time fitting, text fallback | `[R]` |
| Fullscreen drag selection copies per pane, joins soft wraps without inventing newlines, keeps code indentation; double/triple click select word/line | `[R]` |
| A completed text-only reply whose only line is a status marker stays visible and copyable; live status lines do not pollute the transcript | `[R]` |

### 3.4 Composer

| Capability | Evidence |
|---|---|
| Bordered box with `❯`, a session-manager affordance `⌸` on the left and a fullscreen-editor affordance `⛶` on the right | `[S:dsh_chat]` |
| Context occupancy sits immediately above the status bar, right-aligned (`6.9k/1.0M 0.7%`), so "how full am I" is co-located with "what will it cost" | `[S:dsh_chat]` |
| While the model works: `Enter` steers, `Tab` queues a follow-up, `Ctrl+Enter` interrupts and sends; text that names a command still runs the command rather than becoming a steer | `[R]` |
| `Alt+↑` recalls the last message; `Ctrl+U` deletes to line start preserving the right-hand text; `Alt/⌥+←/→` moves by word | `[R]` |
| Image chips behave like atoms (arrows step across them, Backspace at an edge removes the whole chip) | `[R]` |
| Draft, caret and staged images survive backend startup, session switches, `/btw`, and the fullscreen editor; nothing auto-sends on arrival | `[R]` |
| Fullscreen draft editor on `Ctrl+Shift+E`; external editor; vim mode | `[R]` |

### 3.5 Side panel

`Ctrl+B` splits the chat with a panel column once the terminal is wide enough;
the divider is draggable; narrow terminals and inline mode fall back to
full-screen panels `[R]`. Eight built-in panels, all enabled by default
`[R]`, `[C:lib/types/components/sidePanel/builtinPanels.js]`:

| id | icon | order | min columns | capabilities |
|---|---|---|---|---|
| `todo` | `≡` | 10 | — | goal/todo list; `Ctrl+Q` folds, header click hides while idle |
| `info` | `ⓘ` | 15 | 24 | scroll |
| `jobs` | `▸` | 20 | 28 | scroll, sendToChat, fullscreen |
| `btw` | `?` | 22 | 28 | scroll, sendToChat, fullscreen |
| `trajectory` | `∿` | 25 | 28 | scroll, fullscreen |
| `agents` | `◆` | 30 | 28 | scroll, fullscreen (subagent dashboard) |
| `workspace` | `⌗` | 35 | 28 | scroll, fullscreen |
| `companion` | `♥` | 40 | — | the pet |

The panel bar renders each *inactive* panel as its symbol, the active one as its
title, `◀`/`▶` at the ends and `⤢` for fullscreen — observed as
`◀  ⌗  ?  ♥  待办  ▸  ◆  ⓘ  ∿  ▶  ⤢` `[S:dsh_panel]`. Empty states are
explained rather than blank: the agents panel shows `○ 当前会话暂无子代理 / …`,
the jobs panel shows `0 运行中  0 已完成` `[S:dsh_panel]`, `[S:dsh_model]`. The
focused panel owns input, and `Esc` returns to chat `[S:dsh_panel]`.

### 3.6 Live state

| Readout | Evidence |
|---|---|
| Activity line: animated frame + verb + elapsed + tool count + TPS, pausing when idle or unfocused | `[R]`, `[C:lib/types/components/ActivityLine.js]` |
| Context bar whose fill follows backend occupancy and whose colours estimate content composition (compaction clears stale composition) | `[R]` |
| Status bar: `deepseek-flash · max · 缓存 96.6% · ≈¥0.01 谷 · dshlab` then `ctx 0.7% (6.9k/1.0m)`; `≈¥` includes subagent usage; an unlisted model shows tokens only and is marked unpriced | `[S:dsh_chat]`, `[R]` |
| Git branch and session metadata follow a session that moves directory (worktree, `working_directory` tool) | `[R]` |
| Activity ownership: the activity line says which subagent is working | `[R]`, `[C:lib/types/components/ActivityLine.js]` |

### 3.7 Command surface and pickers

- Two dozen built-in commands `[R]`: `/resume` `/home` `/agentview` `/bg`
  `/model` `/new` `/compact` `/export` `/btw` `/tree` `/fork` `/rewind`
  `/settings` `/setup` `/status` `/cost` `/jobs` `/skills` `/mcp` `/provider`
  `/auth` `/login` `/update` `/dsh` `/codex` `/claude` `/kernel` `/trace`
  `/agentview` `/rename` `/recap` `/clear`.
- `?` opens a **two-column overlay**: shortcuts on the left, commands with
  one-line descriptions on the right; the footer teaches the pager
  (`↑/↓ 滚动 · PgUp/PgDn 翻页 · Home/End 首尾 · Esc 关闭`) `[S:dsh_help]`.
- `/model` puts models **and** effort on one page: provider tabs with
  `最近使用` first, an effort row (`推理强度 ←/→ 调整`, `Off Low High Max`) and a
  per-tier description line; `Enter` applies, `Esc` cancels, the mouse can click
  tabs/rows/tiers `[S:dsh_model]`, `[R]`.
- Completion is shared between launchpad and chat: `Tab` fills, `Enter` runs a
  command and clears it `[R]`.
- Nine settings groups with bilingual labels and per-setting hints, including a
  `side-panel` page that lists every panel as a checkbox `[C:lib/settings.json]`, `[R]`.

### 3.8 Sessions and workspaces

One session manager behind `/resume`, `/home`, `/agentview`, `/bg` and `⌸`:
workspace rail, session rows with title + status/time, filter, `★` pins; it
focuses the most recently used session in the current workspace, preloads the
list in the background, and keeps manually selected rows stable while a read
completes `[R]`. Background sessions (`/bg`) keep running inside the process and
stop when the TUI exits `[R]`. `/fork` copies a session with its history;
`/rewind` restores an earlier point; `/tree` browses the fork tree; `/export`
writes the transcript `[R]`.

### 3.9 Mouse and selection (fullscreen only)

`[R]`: drag to select and copy, double/triple click word/line, click tool cards,
click timeline ticks, click `[Image #N]` previews, wheel scrolls, drag the panel
divider to resize, click panel bar items, click model picker rows; hover shows a
panel name and truncated goal text; overlays dim the backdrop and occlude text
underneath; automatic copies leave the selection in place until `Esc`.

Robustness around the protocol is treated as a feature: fragmented SGR mouse
reports are held and re-joined only while mouse reporting is active, a completed
report is consumed as an event, and the hold is bounded (1 s, 64 bytes) so
literal input is delayed but never dropped `[R]`.

### 3.10 Rendering extras

256-colour / truecolour themes with hot reload, `NO_COLOR`/`FORCE_COLOR`
honoured, one shared fallback for Apple Terminal and tmux `[R]`; Bidi text
support `[C:package.json]`; semantic copy (tool cards copy as their content, not
as decorated rows) `[R]`.

### 3.11 Notifications and terminal title

Task completion notifies only after successful live work settles with no queued
follow-up; title `dsh-TUI · Task complete`; body is the latest settled reply
flattened to one line (≤200 chars); OSC 777/9/99 → Windows toast → BEL ladder,
`unfocused`/`always`/`off`; replay/cancellation/failures never notify `[R]`. The
terminal title carries a streaming marker `[R]`.

### 3.12 Extensibility and agent integrations

Cordis plugins contribute panels, commands, scenes and feeds `[R]`; agent
presets (including a "minimal" one-tool preset), skills, MCP servers, goals and
todos, subagent teams, questionnaires and approvals, browser/computer-use
extensions, Claude/Codex kernels, OAuth accounts `[R]`.

### 3.13 Engineering for long sessions

The session log is the source of truth; the UI is an event-driven projection
rendered in O(visible window), with virtualization and bounded caches `[R]`.
This is the part that decides whether a workbench survives a 5-hour session, and
it is a *rendering architecture* claim, not a feature.

## 4. The eight experience capabilities

Feature lists do not transfer; these do. Each is the thing to preserve when the
feature itself is implemented differently.

1. **Three layers of liveness, stable at rest.** Header (who/what/where),
   activity line (what is happening now, with elapsed and rate), status bar
   (cost of what happened). Idle means *still*: animations pause when idle or
   unfocused `[R]`, `[S:dsh_chat]`.
2. **A mouse that can do the common 80%.** Scroll the chat, switch a panel,
   click a tool card open, click a timeline turn, focus the composer, drag the
   divider, click a model row. Every one of those has a hit region and every
   hit region has a visible affordance `[R]`, `[S:dsh_panel]`, `[S:dsh_model]`.
3. **One surface, no mode maze.** Launchpad → chat is the same composer with
   the same completion; overlays stack and `Esc` closes the innermost layer
   *before* interrupting a running turn `[R]`.
4. **Progressive disclosure that never mutates data.** `ctrl+o` expands one
   thing, `ctrl+d` folds runs of calls, `ctrl+q` folds todos, the goal footer
   hides while idle and returns on the next snapshot; hiding is display-only
   `[R]`.
5. **Truthful metrics.** Occupancy comes from the backend, cost includes
   subagents, an unpriced model says so instead of showing ¥0, cache-hit rate
   and TPS are measured rather than implied `[R]`, `[S:dsh_chat]`.
6. **A degradation ladder, not a breakpoint.** Wide: chat + side column with
   symbol bar. Narrow: panels become full-screen. Short: model descriptions go
   first, then neighbouring rows, then the effort row. CJK never shifts a
   column `[R]`, `[S:dsh_chat]`.
7. **Composer continuity.** Draft, caret and attachments survive startup,
   switches, overlays and the fullscreen editor; nothing sends itself `[R]`.
8. **Discoverability without documentation.** `?` is two columns of answers,
   every command has a description wherever it appears, completion carries
   hints, settings are grouped and searchable, and the UI is bilingual `[R]`,
   `[S:dsh_help]`, `[S:dsh_setup1]`/`[S:dsh_setup3]`.

## 5. FutureOS `future-tui` today (evidence)

`future tui` **is the workbench**: `index::run` hands interactive use to
`workbench::run::run` `[F:tui/src/index.rs:951]`, the surface owns raw mode, the
alternate screen, the mouse mode and the frame clock `[F:tui/src/workbench/run.rs:92]`,
and the previous inline UI — its page and its `--legacy`/`--classic` flags — is
gone, pinned by `help_describes_this_build`
`[F:tui/src/help.rs:96]`. The workbench tree is 15 modules, 9,076 lines, with 119
`#[test]`s `[V]`.

| Subsystem | What exists now | Evidence |
|---|---|---|
| Frame | one full-screen frame of exactly `h` rows: header (+hairline), body, input band, activity, status, hint; overlays sheet over it | `[F:tui/src/workbench/screen.rs:29]`, `[S:workbench-chat.png]` |
| Terminal session | alt screen, bracketed paste, Kitty keyboard, mouse mode written with the alt screen and written back on stop/crash | `[F:tui/src/terminal.rs:233]`, `[F:tui/src/workbench/run.rs:92]`, `[V]` (`mouse_any_flag=1` while running, `0` after) |
| Frame bytes | every row is addressed absolutely (`CSI row;1H`) and the frame never advances the cursor with a line feed — a feed on the bottom row scrolls the alt screen and shifts every click by a row | `[F:tui/src/workbench/run.rs:368]` |
| Mouse | SGR 1006 parser (`Press/Release/Drag/Move/WheelUp/WheelDown`), hit map shared with the renderer, click on input/chat/divider/tabs/rows/picker rows, wheel on chat and the focused panel, divider drag (the edge follows the pointer; `30..48`, clamped), picker rows apply on click, `?1000h?1002h?1006h` only (no hover tracking) | `[F:tui/src/workbench/mouse.rs:56]`, `[F:tui/src/workbench/mod.rs:534]`, `[V]` (clicking `用量` switched the panel), `[S:workbench-panel-usage.png]`, `[S:workbench-drag-fixed.png]` |
| Layout | `NARROW_WIDTH = 88`, `SMALL_HEIGHT = 24`, `SIDEBAR_DEFAULT/MIN/MAX = 36/30/48`, 2 cells of chat padding; one `Metrics`/hit-region source for drawing and hit-testing | `[F:tui/src/workbench/layout.rs:65]` |
| Sidebar | five tabs `待办 任务 会话 用量 技能`, persistent at ≥88 columns, full-screen panel under `ctrl+b` below that, divider drag 30..48 persisted on release | `[F:tui/src/workbench/panels.rs:18]`, `[S:workbench-panel-full-60x20.png]` |
| Panel data | goal todos from `events.jsonl` (✔/◉/○/✖ + priority), skills from `get_state` + both skill dirs, jobs from a read-only `tasks.db`, sessions from `list_sessions`, usage from `get_state` | `[F:tui/src/workbench/local.rs:92]`, `[F:tui/src/workbench/live.rs:201]`, `[S:workbench-panel-todo.png]`, `[S:workbench-panel-skills.png]` |
| Launchpad | wordmark + tagline, framed composer, centred block, parameter chips (workspace/model/effort/approval); the block carries no key hints of its own — the footer's hint row has them, and `/` `@` `#` are in the placeholder | `[F:tui/src/workbench/launchpad.rs]`, `[S:workbench-launchpad.png]` |
| Status / activity | `model · effort · permission | gauge + ctx used/total + % · ≈¥`; activity `spinner + verb + elapsed + tools + tok/s`, `缓存 N%` | `[F:tui/src/workbench/screen.rs:468]`, `[F:tui/src/workbench/screen.rs:430]`, `[S:workbench-chat.png]` |
| Themes | six 256-colour palettes (`dark`, `light`, `one-dark`, `one-light`, `high-contrast`, `dracula`), the ids of the inline catalogue because `themeId` is one key for both surfaces; `/theme` opens a picker that previews as the highlight moves, `esc` restores, only `enter` persists | `[F:tui/src/workbench/theme.rs:54]`, `[F:tui/src/workbench/picker.rs:312]`, `[S:workbench-picker-theme.png]`, `[S:workbench-light-120x38.png]` |
| Pickers | one overlay for `/model`, `/theme`, `/sessions`: search line, group headings, `✓` on the value in force, the model picker's reasoning slider, a click moves the highlight **and** applies | `[F:tui/src/workbench/picker.rs]`, `[F:tui/src/workbench/screen.rs:721]`, `[S:workbench-picker-model.png]`, `[S:workbench-picker-sessions.png]` |
| Palette / help | `/` filter over 12 commands with descriptions; `/model`, `/theme`, `/sessions` open their picker, five entries still have no handler; `?` key table; both overlay the frame | `[F:tui/src/workbench/mod.rs:904]`, `[F:tui/src/workbench/run.rs:249]`, `[S:workbench-palette.png]`, `[S:workbench-help.png]` |
| Approvals | a parked approval owns `y`/`n`; the activity row names the tool and its summary | `[F:tui/src/workbench/run.rs:189]`, `[F:tui/src/workbench/live.rs:586]` |
| Transcript | reuses the existing markdown/tool-card/diff components' rendered rows; tail paging on `LoadOlder` | `[F:tui/src/workbench/live.rs:266]`, `[S:workbench-chat.png]` |
| Print mode | `future tui -p` unchanged (non-interactive) | `[F:tui/src/index.rs:617]` |

**What the workbench did *not* carry over.** Deleting the inline UI removed
screens whose replacement is not a panel: `/sandbox`, `/worktree`, `/keymap`,
`/copy` and the external editor are not reachable from the workbench at all
`[F:tui/src/workbench/live.rs:694]` (`command()` knows `quit/new/sessions/model/
thinking/reply-detail/refresh/help/export` plus the launchpad chips; the three
list verbs are intercepted earlier, in `run.rs:249`, and become pickers, and the
verbs that name something on screen are answered by the surface itself).
Five palette entries used to be advertised and then answer `Unknown command` —
`/workspace`, `/usage`, `/skills`, `/export`, `/transcript`; all five resolve
now `[V]` (`[S:workbench-usage-panel.png]`, `[S:workbench-skills-panel.png]`),
and `every_palette_entry_has_a_handler` keeps the next entry from rotting the
same way. That gap was a regression against what the inline UI could do, and it
was item P0-3 in the change list.

## 6. Gap matrix

Verdicts: **missing** (no equivalent), **partial** (a surface exists but not the
capability), **equivalent**, **ahead** (FutureOS does more), **n/a** (no data
source exists in this product, so parity is not the right goal). The `now`
column is what changed when the workbench landed; `was` is the pre-workbench
verdict from the first pass at `9d5f02beb`.

| # | Capability | dsh-TUI 0.17 | `future-tui` now | now | was | Level |
|---|---|---|---|---|---|---|
| 1 | Layered full-screen frame | yes `[S:dsh_chat]` | header/body/sidebar/input/activity/status/hint `[S:workbench-chat.png]` | equivalent | missing | — |
| 2 | Mouse click / wheel / drag-resize | SGR 1006, hit regions everywhere `[R]` | click+wheel+drag wired; **drag moves the rail the wrong way**, no hover `[V]` | partial | missing | P0 |
| 3 | Hover affordance and tooltips | hover names a panel, shows goal text `[R]` | `Move` is parsed and dropped; `?1003h` is never written | missing | missing | P1 |
| 4 | Launchpad with a real composer | yes `[S:dsh_launchpad]` | `[S:workbench-launchpad.png]` | equivalent | missing | — |
| 5 | First-run wizard, re-runnable | 4 steps, `/setup` | none | missing | missing | P1 |
| 6 | Persistent side column, draggable split | 8 panels `[R]` | 5 panels, drag works but inverted | partial | missing | P0 |
| 7 | Live goal/todo panel | `todo` panel + folding | real goal todos with state glyphs and priority `[S:workbench-panel-todo.png]` | equivalent | missing | — |
| 8 | Status bar: branch, cache rate, TPS, title | yes `[S:dsh_chat]` | model·effort·permission, gauge+ctx, cost, `缓存 N%`; **branch is never read** (`branch: String::new()`), TPS only while busy | partial | partial | P0 |
| 9 | Activity line (verb + elapsed + tools + rate) | ActivityLine `[R]` | `spinner + verb + elapsed + tools + tok/s`, still when idle | equivalent | partial | — |
| 10 | Filterable command palette over all commands | `/` overlay + `?` two columns | `/` filters 12 entries with descriptions; `/model`, `/theme`, `/sessions` open a picker; 5 entries still answer `Unknown command` `[V]` | partial | partial | P0 |
| 11 | 256-colour theme kit | themes + hot swap `[R]` | six palettes, a `/theme` picker that previews live, `esc` restores, `enter` persists `[S:workbench-picker-theme.png]`, `[S:workbench-light-120x38.png]` | equivalent | equivalent | — |
| 12 | Context bar with composition colours | yes `[R]` | accent-filled gauge, one colour | partial | partial | P1 |
| 13 | Clickable timeline rail | yes `[R]` | none | missing | missing | P1 |
| 14 | Split diff cards, one-sided full width | yes `[R]` | unified single-column diff | partial | partial | P1 |
| 15 | Session manager (rail, pins, filter, preload) | yes `[R]` | sessions panel + `ctrl+k` refresh; `/sessions` picker with search and a click-to-switch row; no pins, no workspace rail | partial | partial | P1 |
| 16 | Settings screen, grouped and searchable | 9 groups | no settings page; the pickers cover model, thinking and theme, the rest is per-command | partial | partial | P1 |
| 17 | Fullscreen draft editor | `Ctrl+Shift+E`, `⛶` | no affordance, no editor; external `$EDITOR` no longer reachable | missing | partial | P1 |
| 18 | History search scoped to the project | `Ctrl+R` `[R]` | prompt recall only | partial | partial | P1 |
| 19 | Steering vs queue vs interrupt | `Enter`/`Tab`/`Ctrl+Enter` | submit queues behind the run; `ctrl+x` aborts | partial | partial | P1 |
| 20 | Rewind to an earlier turn | double `Esc` + `/rewind` | `/fork` is not in the palette either | missing | missing | P2 |
| 21 | Mermaid fences, LaTeX | Unicode + images | none | missing | missing | P2 |
| 22 | Image preview zoom/pan, Sixel | yes | Kitty/iTerm2 passthrough only | partial | partial | P2 |
| 23 | Bilingual UI (zh/en) | yes `[S:dsh_setup3]` | Chinese chrome only | partial | missing | P2 |
| 24 | Drag-select copy, double/triple click | yes | none; the workbench holds the mouse, so terminal selection needs a modifier | missing | partial | P2 |
| 25 | Background sessions (`/bg`) | yes | one session at a time; queued runs exist instead | n/a | n/a | 不做 |
| 26 | Jobs panel for background shell jobs | yes | no shell-job concept on the wire; the 任务 panel shows **scheduled tasks** (`tasks.db`) instead | n/a | n/a | 不做 |
| 27 | Subagent / agent-team panels | `agents` panel | the agent has no subagents and the proto has no subagent messages `[P]` | n/a | n/a | 不做 |
| 28 | Plugin-contributed panels | Cordis registries | skills are the extension unit; no panel API | n/a | n/a | 不做 |
| 29 | Notifications, terminal title | yes `[R]` | `notifications.rs` exists but the workbench does not call it | partial | equivalent | P1 |
| 30 | Streaming Markdown, tool cards, thinking collapse | yes | yes, reused | equivalent | equivalent | — |
| 31 | `@` file completion, image paste | yes | yes, reused | equivalent | equivalent | — |
| 32 | CJK-safe width arithmetic | yes | `display_width` / `truncate_to_width`, used by every new width computation | equivalent | equivalent | — |
| 33 | Goal control plane | goal tool only | `future-goal` crate + `future goal` CLI, surfaced in the 待办 panel | ahead | ahead | — |
| 34 | Worktree integration | none | `/worktree` exists in the crate but is **unreachable from the workbench** | partial | ahead | P1 |
| 35 | Skills surface | `/skills` | panel with descriptions from two roots | ahead | ahead | — |
| 36 | Sandbox / approval policy surface | permission modes + cards | tiers are readable/cyclable via chips, approvals answerable, no `/sandbox` screen | partial | equivalent | P1 |
| 37 | Headless / print mode | none documented | `future tui -p` | ahead | ahead | — |
| 38 | Isolated instance (own home, own socket) | per-backend storage dirs | `FUTURE_HOME` isolation incl. IPC endpoint (used to take every capture in this document) | ahead | ahead | — |

Counted now: **P0 = 3** (rows 2, 6, 8 — row 10 is P0's fix list too, but the
palette's remaining work is the five dead entries), **P1 = 12** (3, 5, 12–19,
29, 34, 36), **P2 = 5** (20–24), **不做 = 4** (25–28), **equivalent = 8**
(1, 4, 7, 9, 11, 30, 31, 32), **ahead = 5** (33, 35, 37, 38, and 34 before the
collapse).

The picker overlay (`tui/src/workbench/picker.rs`, 843 lines) landed after the
first pass and moves rows 10, 11, 15 and 16 forward without closing any of them:
the command surface now has three real menus (model with its reasoning slider,
theme with a live preview, sessions with search and click-to-switch), and six
palettes replace two. The five dead entries, session pins, a workspace rail and a
settings page are still open.

The lesson of the `was → now` columns: the workbench closed the *frame* gap
(rows 1, 4, 7, 9) in one pass, and what is left is almost entirely **behaviour
inside the frame** — a drag direction, an unread branch, five commands that
answer "Unknown command", hover, a wizard, a settings page. That is a cheaper
kind of work than the first pass, and it is what the change list orders.

## 7. gRPC impact

The constraint is absolute and held: **no change to
`packages/rpc/proto/future.proto`, no new agent method, no new event.** The
workbench reads what the inline UI already read.

### 7.1 Already served by `get_state` (`SessionState`) `[P]`

| Workbench need | Field |
|---|---|
| model, effort | `model` (1), `thinking_level` (2) |
| busy / compacting (activity line, spinner) | `is_streaming` (3), `is_compacting` (4) |
| session id, title, explicit vs auto | `session_id` (8), `session_name` (9), `explicit_session` (10) |
| auto-compaction badge | `auto_compaction_enabled` (11) |
| query count, queued prompts | `query_count` (12), `pending_message_count` (13), `queued_runs` (34), `queued_count` (35) |
| cwd (header, status bar) | `cwd` (15) |
| skills list (sidebar) | `skills` (16) |
| context bar: used / window / percent | `context_tokens` (19), `context_window` (20), `context_percent` (21) |
| permission level, sandbox tier (chips, status bar) | `permission_level` (28), `sandbox_tier` (41) |
| tokens in/out and cache read/write (cache rate) | `usage.input_tokens`, `usage.output_tokens`, `usage.cache_read_tokens`, `usage.cache_write_tokens` (40 → 1–4) |
| cost, cost split, "unpriced" detection | `usage.cost_cny`, `usage.cost_input_cny` … `cost_cache_write_cny` (40 → 5–9) |
| active run, interrupted run | `active_run` (33), `interrupted_run` (36) |
| approval cards (y/n) | `pending_approvals` (39) |
| journal health (status bar warning) | `event_journal_error` (45) |
| agent version / instance (header dot) | `version` (14), `agent_instance_id` (29) |

### 7.2 Served by other existing commands `[P]`

| Need | Command |
|---|---|
| session rows (title, model, cwd, updated, first message, query count, streaming) | `list_sessions` → `SessionSummary` `[F:tui/src/workbench/live.rs:220]` |
| transcript rehydration / older history on scroll-up | `get_session_entries` (tail page, then `LoadOlder`) `[F:tui/src/workbench/live.rs:266]` |
| model catalog for `/model` cycling | `list_models` |
| approval decisions | `approval_decision` `[F:tui/src/workbench/live.rs:843]` |

### 7.3 Not on the wire — and the local fallback chosen

| Need | Status | Fallback (no proto change) |
|---|---|---|
| git branch for the header/status bar | absent from the RPC | `GitCli::branch()` (`rev-parse --abbrev-ref HEAD`, timeout, read-only) exists in the crate; the workbench leaves `SessionMeta::branch` empty `[F:tui/src/workbench/live.rs:553]` — **P0-4** below |
| goal objective + todos | absent (a local control plane) | `events.jsonl` under `FUTURE_GOAL_ROOT`, else the nearest ancestor `.future/goal`, else the git toplevel's — so a `.worktrees/<name>` session reads the checkout it was cut from `[F:tui/src/workbench/local.rs:42]` |
| scheduled-task rows | absent | `tasks.db` opened `SQLITE_OPEN_READ_ONLY`; a missing database is "no scheduled jobs", never a newly created store `[F:tui/src/workbench/local.rs:403]` |
| skill descriptions | partial (`get_state` names skills, not their descriptions) | the two skill directories' `description:` line `[F:tui/src/workbench/local.rs:306]` |
| TPS (tokens/second) | absent | derived client-side from usage deltas over a short window, and only while a run is live `[F:tui/src/workbench/live.rs:457]` |
| session cost per *row* in the session list | absent (`SessionSummary` has no usage) | rows show model + age + query count; cost only for the active session |
| subagent list | absent, and the agent has no subagents | not built (§8) |
| MCP servers | absent (the `future mcp` CLI owns `~/.future/mcp.json`) | not built |

### 7.4 What *would* need a proto change (recorded, not requested)

Only two workbench wants are genuinely blocked: **per-session usage in
`list_sessions`** (would let the session list sort by cost) and **any
subagent/job surface** (needs agent-side concepts first). Both are P2-or-never
and neither is allowed to grow into a wire change under this goal.

## 8. Where FutureOS should not copy dsh

1. **No subagent or job panels.** There is no subagent concept in the agent and
   nothing on the wire to show. A panel that renders an always-empty list is a
   lie about capability.
2. **No plugin/panel API.** Cordis registries are a whole extension model;
   skills are FutureOS's extension unit and they already have a panel target.
3. **No kernel switching (`/dsh` `/claude` `/codex`).** The backend is this
   repository's agent. What maps usefully is `/providers` + `/models`.
4. **No second session model.** `/bg` background sessions would duplicate the
   agent's own queue and session lifecycle; the workbench *surfaces*
   `queued_runs` instead of inventing a parallel notion of "background".
5. **Do not trade away what is already better**: `/worktree`, `/sandbox` tiers,
   skills install, the goal control plane, print mode, `FUTURE_HOME` isolation.
   The workbench is a shell around them; it must not lose them by default — and
   right now `/worktree` and `/sandbox` *are* lost (§5, P0-3/P1).

## 9. Verification and remaining unknowns

How to re-derive this document:

1. Read the pinned package: `~/.dsh/profiles/dsh-tui/package.json` (version) and
   `…/node_modules/@deepseek-harness-tui/dsh-tui/README.md` (capabilities).
2. Re-capture a dsh screen: run `dsh-tui` under tmux, drive it with keystrokes,
   then `tmux capture-pane -e -p` into `shots/raw_<name>.txt` and render the same
   buffer to `shots/dsh_<name>.png`.
3. Re-capture a FutureOS screen: `tui/scripts/shot.sh` (recipe in
   `verification.md` §7) — isolated `FUTURE_HOME`, isolated agent port, the size
   as its second argument (`120x38`). It runs the workbench in a pty of that
   size and renders what it wrote; no tmux, no `ansi2png.py`.
4. Check a FutureOS claim with its `[F:…]` tag; the greps that matter are
   `?100` (mouse), `sidebar`, `goal_todos`, `branch`.

Residual unknowns (deliberately not guessed):

- **Not measured**: mouse latency/feel, hover timing, notification delivery on
  this terminal, and how dsh's activity line behaves while a *nested* tool runs.
- **Not measured**: how dsh degrades at 90×24 — every dsh capture is 120×38
  cells. Its degradation ladder (§4.6) is `[R]`-level evidence; the FutureOS
  ladder is measured at 120×38 / 90×24 / 60×20 `[S:workbench-90x24.png]`,
  `[S:workbench-60x20.png]`.
- **Not verified**: whether every dsh panel survives a 28-column split without
  clipping; the `minColumns` values in §3.5 are structural facts from
  `[C:…/builtinPanels.js]`, not a rendering test.
- **New**: at 60 columns the status bar's left label is clipped with a trailing
  `…` (`future/deepse…`) while every number keeps its width `[V]`. That is
  deliberate (the gauge is laid out first) but it does read as a different
  model name at a glance; worth revisiting when the model label gets a short
  alias.

## 10. Open questions — and how the first pass's three were answered

1. ~~Default-on, or behind `--layout workbench`?~~ **Answered by shipping**:
   `future tui` is the workbench and there is no switch. `--layout`,
   `--legacy` and `--classic` do not exist; `help_describes_this_build` fails if
   any returns `[F:tui/src/help.rs:96]`. What the first pass called "an inline
   fallback" was deleted rather than kept: two renderers sharing one output
   stream was the thing that was rejected.
2. ~~May the todo panel read `<cwd>/.future/goal` directly?~~ **Answered by
   using it**: direct reads, no process spawn, best-effort, and the root is
   resolved per *session* cwd so a worktree session reads its own checkout
   `[F:tui/src/workbench/local.rs:42]`.
3. ~~Is an English-only launchpad the wrong first impression?~~ Still open, and
   now sharper: the whole workbench chrome is Chinese-only `[S:workbench-launchpad.png]`,
   so row 23 is a product decision, not a translation chore.

New questions for the supervisor:

4. Should the five dead palette entries be wired (`/usage` → focus the 用量
   panel, `/skills` → 技能, `/workspace` → cwd) or removed until they work? A
   command that answers "Unknown command" costs more trust than an absent one.
5. Should the divider drag be fixed to follow the pointer (one-line sign flip,
   P0-1), and is the current inverted behaviour ever intentional anywhere else?
