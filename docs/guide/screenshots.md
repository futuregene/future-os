# Screenshot harness: real UI, no display

[中文](screenshots.zh-CN.md)

Product screenshots used to need a person at a machine running `make run-desktop`:
the desktop app is a Tauri webview, the mobile app is React Native, and both draw
their content from a local backend that a browser cannot reach. This harness
renders the **real frontends** in a local Chrome instead, with fixed demo data
standing in for the backend, so screenshots can be captured on a headless box —
CI, a remote shell, or a session with no screen-recording permission.

Only the data source is faked; components, hooks, stores, styles and i18n are the
shipped code. Taps and typing are dispatched as real input events, so what you
capture is what the UI does.

## What it is for

Typical requests this covers — in each case, produce the images from the real UI,
then assemble them (see *Building a document*):

- "Release notes for the changes between version X and Y" — capture the affected
  screens, then a PDF whose items pair prose with one or two screenshots.
- "A test checklist for the changes between X and Y" — actually walk each change
  in the harness, then write test points that other people can replay (see
  *Recipes* 2).
- "A diagram of the desktop `<feature>`" — one scenario, one PNG.
- "A walkthrough of the mobile `<feature>` flow" — a scenario per step (list →
  dialog → result), assembled into a document.
- "Record a demo video" — `video-desktop` / `video-mobile` record scenarios to
  mp4, on both platforms.
- "Show the CLI output for `<command>`" — the `terminal` subcommand renders a
  command's real stdout in a terminal frame.

Scope follows what the app can actually show: if a feature has no screen (a wire
protocol change, a TUI-only notice), there is nothing to capture, and saying so
is better than illustrating it with a stand-in.

## Recipes

These seven cover most requests. In every case, start the relevant server first.

### 1. Two versions → release-notes PDF

```bash
git log --oneline vX..vY                       # what changed
git log --oneline vX..vY --format="%h|%s" | grep -E "feat|refactor"   # features only
```

For each commit, decide which screen it touches, find or add a scenario, and
capture it. Skip performance work and bug fixes. Then write a content file
following `scripts/screenshots/examples/release-notes-1.1.8.json` and run the
`pdf` subcommand. Captions should say what to look at, not restate the prose.

For a non-technical audience the *why* matters more than the *what*: say what
was painful before, what it is like now, and only then where to find it. Put
the screenshot beside the "what it is like now" part.

### 2. Two versions → test checklist

Same change list, but the output is a checklist rather than a document:

1. For each feature change, **actually walk it** in the harness — find the
   scenario (add one if it does not exist) and capture it.
2. Write each test point as precondition → action → expected result, with the
   expected result phrased as something visible in the UI.
3. Attach the screenshot for that step as the evidence for the expectation, and
   name its path under the test point.
4. Changes that cannot be verified through the UI (protocol, internal refactor)
   go in a separate "needs interface/log verification" section — do not invent UI
   steps for them.

This is where the harness earns its keep: the test points are derived from
clicking through the real UI rather than reading the diff, so a reviewer can
replay them from the screenshots.

### 3. One feature → one diagram (desktop or mobile)

Add a scenario to `scenarios.json` that does only what is needed to open the
feature, ending in one `shot`. Then
`capture-desktop <scenario>` or `capture-mobile <scenario>`. If the feature is
behind a menu, add a `tap`; for a control revealed on hover, `hover` its container
first.

### 4. A flow walkthrough → document (mostly mobile)

Split the flow into steps and make **one scenario per step** (not one long one),
so each step can be re-shot and cited on its own; then assemble with `pdf`. A
mobile flow might be: open the list → open a conversation → type `/` and pick a
skill → read the result. For video, use `video-mobile <scenario>` — one scenario
is one continuous piece of footage.


### 5. Pick one of several styles → variant sheet

Declare `variants` in a scenario, one inject script per variant; the harness renders
the same screen once per variant and composes a labelled comparison sheet:

```json
"running-icon-variants": {
  "variantsTitle": "运行指示图标 · 三个备选样式",
  "variants": [
    { "label": "A · 实心圆点", "inject": "injects/running-dot.js" },
    { "label": "B · 同心圆环", "inject": "injects/running-ring.js" }
  ],
  "steps": [ { "wait": 900 }, { "inject": "{variantInject}" }, { "shot": "d-icon.png" } ]
}
```

```bash
python3 scripts/screenshots/capture.py variants-desktop running-icon-variants
```

Worth knowing:

- **The `{variantInject}` placeholder** lets one scenario serve every variant: the
  injection happens *inside the steps*, not at boot, because React re-renders and
  would restore what an earlier mutation changed. If the change affects layout,
  inject after the last interaction.
- **This is a visual proposal.** The injection edits the DOM in the browser; the
  product is untouched. Whatever is chosen still has to be implemented.
- Inject scripts are plain JS under `scripts/screenshots/injects/`; the committed
  `running-*.js` files are working examples.

### 6. Compare two versions' styling → highlighted differences

Capture each version, then compare:

```bash
# in version Y's checkout
python3 scripts/screenshots/capture.py capture-desktop remote-conversation
cp .screenshots/d-remote-conversation.png /tmp/y.png
# switch to version X (or use another worktree) and capture the same scenario
python3 scripts/screenshots/capture.py capture-desktop remote-conversation
python3 scripts/screenshots/capture.py compare-desktop /tmp/y.png .screenshots/d-remote-conversation.png \
    --output /tmp/style-diff.png --label-left "1.1.8" --label-right "1.1.7"
```

Output is a three-panel sheet: A (changed regions boxed in red), B (boxed in blue,
numbered), and a difference panel (unchanged content faded, changes red).

Worth knowing:

- **Both captures must be the same size** — same viewport (`--w/--h`) and same
  scenario does it; if they differ, the right one is resized to match.
- **`--threshold`** sets what counts as a change (default 24, so anti-aliasing
  noise stays out); **`--min-area`** drops regions too small to label.
- **Older versions have no harness**: `scripts/screenshots/`, `desktop/shot/` and
  `desktop/vite.shot.config.ts` only exist from #703. To capture an older
  checkout, copy those three into it first; the old frontend then renders against
  the current mock data, and logs `[mock] UNHANDLED COMMAND` for anything the mock
  does not answer — add those as they appear.
- Version diffs are noisy by nature: text rendering, timestamps and scroll
  position all register as changes. Line the two up first — set an absolute
  `scrollTop` with `eval` rather than a relative wheel scroll.

### 7. Pixel spacing / movement between elements → offset diagram

Add an `offsets` step and the harness measures each element's box (with name and
size) and draws the pixel distance between them:

```json
{ "offsets": {
    "axis": "y",
    "mode": "gap",
    "targets": [ { "at": "新对话", "name": "新对话" },
                 { "at": "模型", "name": "模型" } ] } }
```

- **`mode: "gap"`** (default) labels the distance between consecutive elements —
  "how far apart are these two rows". **`mode: "edge"`** labels each element's
  distance from the viewport edge — "are these aligned".
- **`axis`** is `"y"` (vertical) or `"x"` (horizontal).
- **The numbers are CSS pixels** at the emulated viewport, not device pixels, so
  they match what a design file would quote.
- `targets` matching is **ranked** (exact `aria-label` first, then an interactive
  element's own exact text, …) and the driver prints which element each target
  actually hit — a wrong match is visible instead of silent (`技能` used to hit the
  composer's `选择技能` button).
- To show **how far each element moved since a previous version**, record a
  baseline and compare against it:

```bash
# version X
python3 scripts/screenshots/capture.py measure-desktop rail-offsets
cp .screenshots/desktop-rail-offsets-measure.json /tmp/base.json
# after switching to version Y, capture with that baseline
python3 scripts/screenshots/capture.py capture-desktop rail-offsets --baseline /tmp/base.json
```

With a baseline each element gains a green label stating its movement and size
change (e.g. `位移 +0, +8px · 宽 +4px`).

### 8. The terminal UI → a real screenshot

The terminal UI (`future tui`) is a full-screen interface, so it is captured
like the desktop and mobile apps rather than rendered through `terminal.html`:

```bash
cargo build -p future-cli --bin future
cargo build -p future-tui --example demo_agent
python3 scripts/screenshots/capture.py tui tui-chat.png
```

The program in the picture is the shipped binary. `capture.py tui` starts
`demo_agent` — a gRPC stand-in that answers with one fixed session — then runs
`future tui --offline --grpc-addr …` in a pty of a fixed size and keeps the
bytes it wrote there. `tui_terminal.py` replays that stream through a terminal
cell model (absolute positioning, SGR colours, double-width CJK, the 256-colour
palette, and the caret the app left behind) and has headless Chrome rasterize
the glyphs.

What the flags are for:

- `--cols` / `--rows` size the pty (default 130×40). The frame is a function of
  them, so a layout change shows up as a different picture instead of a
  squeezed one.
- `--keep-stream <file>` writes the raw pty bytes — the first thing to look at
  when a capture comes out blank.
- `--scale` sets the device pixel ratio (default 2).
- `--session <id>` opens a specific session instead of leaving it to the agent.
- `--scenario <name>` drives the surface to a screen a reader would reach:
  `chat` (as it opens), `help` (`?`), `palette` (`/`), `palette-filter` (`/`
  plus a query, without Enter), the sidebar panels `todo` / `usage` / `skills` /
  `sessions` (selected by clicking their tab — the workbench switches panels by
  click and has no key for it), the two palette verbs that name a panel
  (`command-usage`, `command-skills`), `launchpad` (`/new`), `mention-sessions`
  (`#` in the draft: the session-mention menu), `full-panel`
  (`ctrl+b` on a narrow terminal), `scrolled` (a click, then three wheel-ups),
  `light` (`/theme`, one row down, Enter — it persists, so it goes last) and
  `drag` (grab the divider and pull it six columns right). Every coordinate is
  resolved from the frame on screen when the step runs — the tab click from the
  painted label, the drag from the painted seam — so they follow the layout
  instead of a hard-coded column. `git` (`ctrl+g`) is the one scenario that
  *reads the checkout* rather than the scripted payload, so it needs a real
  repository to look at: `FUTURE_DEMO_CWD=$PWD tui/scripts/shot.sh git`. Unset,
  the scripted session keeps its fictional `/Users/you/future-os` and the panel
  says the directory is not there.
- `--scenario streaming` is the one frame that is *in motion*: it types a prompt
  and submits it, and the demo agent answers with a scripted turn — one
  `text_chunk` every 120 ms, never an `agent_end` — so the picture is a reply
  still arriving, with the activity row live and the transcript stopping
  mid-sentence. Give it long enough to reach the deltas (`--seconds 6`). It is
  the frame to look at when a change touches streaming, the activity row or the
  transcript's incremental rendering; a finished reply shows none of that.
- `--addr <host:port>` attaches to an agent that is already running (a *real*
  session) instead of starting `demo_agent`, and leaves the caller's
  `FUTURE_HOME` alone.
- `--lang <system|zh|en>` pins the whole frame's language (default `zh`, the
  language every committed asset was shot in): the chrome's copy goes into the
  TUI's `settings.json` and the scripted data is handed to `demo_agent` as
  `FUTURE_DEMO_LANG`. Both halves have to agree, or the frame is an English
  shell around a Chinese session list. The English `streaming` scenario also
  submits its prompt in English — the question is the reader's words, not a
  keystroke. Both payloads are the same session — 58 conversations in the rail
  and a two-turn transcript that exercises a heading, bold and italic runs,
  inline code, a highlighted `rust` block, a bordered table and a quote — written
  in each language rather than translated sentence by sentence, because a panel
  with three rows in it is a picture of a mock. That also flushed out three
  labels the chrome had left in English: the settled activity row (`idle`), a
  session with no model chosen, and the reasoning effort beside the model
  (`medium`), plus the age of the newest conversation (`now`, two cells wider as
  刚刚 — the age column is measured over what is painted). It is also taller than a 54-row band by itself, so an
  English frame is usually shot at 190×66 rather than at the documented 120×38.
- `--sidebar-width <cols>` pins the rail's width (default: the surface's own,
  `SIDEBAR_DEFAULT` = 36). The rail reserves a third of itself for the workspace
  name, so at 36 columns an English session title gets about nineteen cells
  before the ellipsis while the same list in Chinese fits; pass this when a
  frame needs the titles to read through (the ceiling is 60).
- `--sidebar-position <left|right>` pins which side the rail sits on (the
  surface's own default is right).
- `--theme <id>` pins the palette (`dark`, `light`, `dracula`, `tokyo-night`,
  `gruvbox`, `rose-pine`, …; see `theme.rs`). The committed assets are all on the
  shipped default, so this is for a frame that wants to show the palette picker's
  effect rather than the product's own look.
- `--scenario composing` is the frame of a reader *in the middle of writing*
  after a reply has finished: four lines of draft in the composer (typed with
  `ctrl+j`, never sent), the **queue band** above it listing three prompts that
  are waiting to go out (`FUTURE_DEMO_QUEUE=idle` — the reply is done, the queued
  prompts have not been sent yet), and the last turn's work **opened as a tree**:
  `▾ 3 tool calls · 2 thoughts`, then `│ ▸ thinking` with the reasoning under it,
  `│ edit src/args.rs`, `│ $ cargo test …`. Opening it is a keyboard walk —
  `tab` to the fold, `enter`, `tab` into its first thought, `enter` again —
  because a click on a transcript row does not arrive through the pty (measured:
  the same SGR bytes open a rail tab and grab the divider, and leave a fold
  shut); `shift+right` (`PanelNext`) then switches the rail from its flat *Recent*
  list to the workspace view, so the conversations are filed under the heading
  and count of the checkout they belong to. Both shapes only exist when the
  session is *settled*: a run in flight renders every row flat, which is why the
  queue band is captured in the idle half of that state. At 190×72 the whole
  thing fits — 58 conversations in three workspace groups, the transcript's
  table/highlighted code/quote, the opened tree, the band and the draft.

The repository's own entry point for these frames is
`tui/scripts/shot.sh <scenario> [WxH] [out.png]`, a thin wrapper that supplies
`--scenario` and the size and names the file after the asset it belongs to:

```bash
tui/scripts/shot.sh chat 120x38 $PWD/docs/tui/assets/workbench-chat.png
tui/scripts/shot.sh usage 120x38 $PWD/docs/tui/assets/workbench-panel-usage.png
tui/scripts/shot.sh drag 120x38 $PWD/docs/tui/assets/workbench-drag-fixed.png
# An English frame, big, rail on the left and wide enough for the titles:
SHOT_LANG=en SHOT_SIDEBAR_WIDTH=56 SHOT_SIDEBAR_POSITION=left \
  tui/scripts/shot.sh composing 190x72
```

The third argument is the output path; without it the wrapper names the file
after the scenario and size (`workbench-chat-120x38.png`, `workbench-usage-120x38.png`),
which is not always the name an existing asset was committed under — pass the
path when re-shooting one of those.

Two runs produce byte-identical PNGs, which is what makes this usable as
review evidence: a difference means the UI changed, not that a model answered
differently. Two limits are worth stating: the glyphs are drawn by Chrome from
the captured cells rather than by the terminal that produced them, and the data
is scripted like the other mocks — a screenshot is never evidence about a real
user's session.

Glyphs are placed at their cell coordinates rather than flowed as text. That is
not a detail: in this font stack a hanzi advances 14px where two columns need
16.9px, and `。` advances 7.2px, so laying a row out as text squeezed every
Chinese line and pulled the rest of the row left — while the one-cell border
strokes stayed put, so the frame looked aligned at its edges and was wrong
inside.

Each capture re-reads its own PNG and checks it against the grid in **both**
directions: every cell the grid fills must carry ink, and no cell the grid leaves
blank may carry a glyph. One direction alone is not a check — a row pushed
sideways by a whole cell still leaves each character's box partly inked, which is
how a misaligned frame passed the first version of this gate. A wide cell is
measured over the two columns it owns: measuring one column reports "no ink" for
`（`, whose stroke sits on the right of its em, and that false positive blocked
every live frame containing a Chinese parenthesis.

A renderer fix does not fix the frames already on disk. `docs/tui/assets/` must be
re-shot after any change here — the 19 frames committed before the absolute
placement landed had the divider on the right column in only half their body
rows.

```bash
python3 scripts/screenshots/tui_terminal.py --self-test   # the cell model, no browser
python3 scripts/screenshots/capture.py --self-test        # the scenario steps, no browser
```

## Recording video

```bash
# terminals 1 and 2: start the servers (as above)
python3 scripts/screenshots/capture.py video-desktop              # every scenario
python3 scripts/screenshots/capture.py video-desktop steps-expanded rename  # or just some
python3 scripts/screenshots/capture.py video-mobile chat
```

Output is `<platform>-<scenario>.mp4` in `--out` (default `.screenshots/`), on
both platforms. Worth knowing:

- **ffmpeg is required** (`brew install ffmpeg`).
- **Pacing is real.** Chrome only emits a frame when the page changes, so the
  video is encoded with each frame's actual delay: pauses stay pauses instead of
  being flattened to a fixed frame rate.
- **What you record is the scenario's steps**, so make the `wait` values long
  enough to read (e.g. pause 1.5s after opening a menu) when the footage matters.
- **The pointer indicator is drawn in**, which is what makes mobile footage
  followable; see *Pointer indicator and callouts*.
- **Resolution**: stills come out at 2x (a 1280x860 viewport yields a 2560x1720
  PNG). Chrome emits screencast frames at the **CSS viewport size** — it ignores
  the emulated device scale factor — so the encode upscales them with Lanczos to
  the stills' pixel dimensions. That is interpolation, not extra optical detail.
- Video and screenshots share one scenario table: nothing to maintain twice.

## What it is, and what it is not

| Real | Replaced |
|---|---|
| `desktop/src/**` — every component, hook, store, worker, style | `@tauri-apps/*` IPC, answered in-page (`desktop/shot/mock/`) |
| `mobile/src/**`, `mobile/App.tsx` | `mobile/src/remote/RemoteContext` (the NATS client) and two native modules (`mobile/shot/mock/`) |
| Real Chrome rendering the real layout | Real data, real network, real agent |

Consequences worth knowing before trusting a screenshot:

- **Content is fabricated.** Demo conversations, file names and projects come from
  `desktop/shot/mock/data.ts` / `mobile/shot/mock/data.ts`. Never present a
  harness screenshot as evidence about real user data.
- **No backend behaviour is exercised.** Nothing here proves an RPC works, a run
  streams, or a sandbox boundary holds. Those need the real app.
- **Command-line output is stylised, not captured**: `terminal.html` renders a
  command's real stdout inside a terminal frame. The terminal UI is *not* in
  that category — `capture.py tui` (recipe 8) runs the real binary in a real pty
  and rasterizes the bytes it wrote, so the frame in the picture is the app's
  own. Its *data* is still scripted, exactly like the desktop and mobile mocks.
- **One web-only rendering deviation is corrected rather than shown.** On the web
  react-native-web gives an inverted list's refresh-control wrapper the list's own
  flip, so the transcript renders upside down — which native does not do. A
  mirrored phone screenshot would be worse than useless, so `cdp.mjs` neutralises
  that extra flip for mobile captures (the DOM shows the correction: the wrapper
  has no `transform`) and `check-upright-mobile` measures the result.

## Requirements

- Node.js (the driver uses `ws`, already a repo dependency) and Python 3.
- Chrome, Edge or Chromium. Safari is not supported (the driver needs CDP).
- The capture only drives a **headless** browser, and only one it started itself
  (or a headless one already listening on `--cdp-port`). A *visible* browser on
  that port is reported and skipped, never driven — driving it would navigate and
  screenshot a window you are looking at — and the run starts its own on the next
  free port instead. Interrupted runs (a turn timeout, an agent restart) leave a
  browser and a `future-shot-` profile behind; every command sweeps those at
  startup, and a SIGTERM handler takes the browser down when a run is killed.
  That handler is POSIX-only: Windows cannot deliver SIGTERM to the process, so
  there a killed run's browser is cleared by the next command's startup sweep —
  which reads the process list through `Get-CimInstance Win32_Process`, since
  Windows has no `ps`.
- `pip install pillow` is optional: it only shrinks images in the generated PDF.
  `compare-*` and `check-upright-mobile` do need it, and say so if it is missing.
- The mobile harness additionally needs `react-native-web`, `@expo/metro-runtime`
  and `react-dom`; `capture.py serve-mobile` installs them into the mobile
  workspace on demand and leaves the manifests alone (see *Nothing is committed*).
- **Video** additionally needs ffmpeg (`brew install ffmpeg` on macOS, the distro
  package elsewhere). It is checked before recording starts, so a missing ffmpeg
  fails immediately rather than after a long capture.

## Desktop

Two terminals, because the dev server stays in the foreground:

```bash
python3 scripts/screenshots/capture.py serve-desktop     # terminal 1: serves http://localhost:5299/shot/
python3 scripts/screenshots/capture.py capture-desktop       # terminal 2: runs every scenario
python3 scripts/screenshots/capture.py capture-desktop rail-tree steps-folded                # or just some
```

The scenario names are the keys in `scripts/screenshots/scenarios.json` (the `d-` prefix
belongs to the output PNG, not the scenario); `capture-desktop` reports an unknown name with
the ones it does have.

The captured PNGs land in `.screenshots/` (gitignored). Open
<http://localhost:5299/shot/> yourself to explore by hand; the harness also starts
a stand-in PTY server on port 7391 so the embedded terminal panel has something
to show.

### One session, two surfaces: the `parser-*` scenarios

`.screenshots/tui-zh/` and `.screenshots/tui-en/` are the TUI's pictures of one
scripted session — an off-by-one in an expression parser
(`tui/examples/demo_agent.rs`, `journal_zh` / `journal_en`). `parser-zh` and
`parser-en` show that same session in the app:

```bash
python3 scripts/screenshots/capture.py serve-desktop               # terminal 1
python3 scripts/screenshots/capture.py capture-desktop parser-zh   # → .screenshots/desktop-zh.png
python3 scripts/screenshots/capture.py capture-desktop parser-en   # → .screenshots/desktop-en.png
```

Both live in `scripts/screenshots/scenarios.d/parser-session.json` — a fragment,
so the shared table is not edited by a session that only adds a scenario.

`?thread=th_parser` opens that conversation and brings its workspace row with it;
the default fixture, which every other scenario starts from, is left exactly as
it was. `?lang=zh|en` pins **both halves** of the frame: the interface (read by
`shot/prefs.ts` before the app boots) and the fixture's own session list and
transcript (`desktop/shot/mock/data.ts` reads the same parameter). A rail whose
heading said `Sessions` over rows that said `整理两篇文献的结论对比表` is neither
language, so each scenario fails if a word of the other one reaches the frame.

The window is a 16-inch laptop's default 1728×1117 at 2×, and the frame reads as
a normal desktop app: the session rail, the transcript, and — opened from the
header's own panel button (`展开上下文面板` / `Expand context panel`) — the
workspace's files on the right. The panel seeds its **Files** tab when it opens
from that button, so the scenario taps it once and then the `src` row; what it
lists is the checkout the conversation is about — `parser.rs` and `args.rs` beside
`lexer.rs`, `main.rs`, `Cargo.toml` and `AGENTS.md`, from `workspaceFiles` keyed by
the workspace path. Opening it takes 384 px off the conversation column (1440 →
1056 at this width), but the reading column is capped, so the text re-centres
rather than re-wrapping; in a window too narrow for a third column the panel opens
as an overlay over the conversation instead (`context-panel-narrow` captures that).

A settled reply folds onto its answer, so the frame stands at the first turn and
keeps its work folded — `查看过程 · 工具调用 2 次 · 思考 1 次`, which is the TUI's own
`▸` line. `上一个问题` (`Previous question`, the transcript's own ↑ control) lands
the reader's previous question just under the header, which is what puts a whole
turn on one screen: the question, the folded process, and the diagnosis with its
highlighted `rust` block, three-row table and closing line, the text word for word
the TUI's. The second turn starts at the fold — its question is on screen, its
answer is not — because a laptop window cannot hold both turns, and a
taller-than-a-screen "window" was the alternative. The `eval` holds the frame to
that: every block of the first turn above the composer, the second question still
visible at the fold, and no word of the other language anywhere, the files panel
included.

Two limits worth stating. Desktop captures are **not** byte-reproducible the way
the TUI's are: they differ run to run in exactly two places — the relative age
labels beside each message (`刚刚` / `1分钟前`, read from the wall clock) and the
rotating indicator on a session the fixture reports as running. The content is
stable; those pixels are not. (`capture-desktop steps-folded` moves the same two,
so it is the surface rather than these scenarios.) And the fixture carries the
session list and transcript in both languages; the settings, tasks and skills
fixtures are still Chinese, which no English capture opens today.

The transcript itself needed two handlers the mock did not have. A history read
goes through the bounded transfer lane (`begin_session_history_transfer` and its
chunked reads), and with only the inline command answered every conversation came
out as `invalid_history_transfer` with a retry link — the transcript a screenshot
exists to show never rendered; opening a step row also reads
`list_approval_assessments`, which answered `null` and unmounted the app. Both are
answered in `desktop/shot/mock/backend.ts` now.

## Mobile

```bash
SHOT_WEB=1 python3 scripts/screenshots/capture.py serve-mobile     # terminal 1: Expo web on http://localhost:8099/
python3 scripts/screenshots/capture.py capture-mobile       # terminal 2
```

The mobile harness renders at a phone viewport (390×844 at 2× by default) and
serves the demo figures over HTTP, so inline images and the zoomable preview
resolve. `serve-mobile` sets `SHOT_WEB=1`; that flag is the only
switch — `mobile/metro.config.js` redirects the remote context and the two native
modules to `mobile/shot/mock/` **only** on the web platform and only when it is
set, so native builds (`make run-mobile-android`) are unaffected.

### One session, two surfaces: the `parser-*` scenarios

`.screenshots/tui-zh/` and `.screenshots/tui-en/` are the TUI's pictures of one
scripted session — an off-by-one in an expression parser (`tui/examples/demo_agent.rs`,
`journal_zh` / `journal_en`). `parser-zh` and `parser-en` show that same session on
the phone:

```bash
SHOT_WEB=1 python3 scripts/screenshots/capture.py serve-mobile    # terminal 1
python3 scripts/screenshots/capture.py capture-mobile parser-zh   # → .screenshots/mobile-zh.png
python3 scripts/screenshots/capture.py capture-mobile parser-en   # → .screenshots/mobile-en.png
```

`?parser=1` swaps the whole demo catalogue for that session — the machine's name,
the three workspace groups, the conversation (`mobile/shot/mock/parserSession.ts`)
— and `lang` pins the browser locale (`zh-CN` / `en-US`), so the chrome and the
transcript are in one language: the mock takes its words from `navigator.language`,
which is the same input the app takes its language from, and each scenario fails
if a word of the other language reaches the frame.

The phone cannot show both turns at once — the conversation is 1125 css px of
transcript and the band between the bar and the composer is 672 — so the frame
opens at the question and holds the folded process, the diagnosis, the fixed line
of code and the table; the second turn (the two commits) sits one scroll below.
The TUI's tree of tool rows has no phone equivalent: it is the fold row, whose
spoken label (`Show process · Tool calls 2× · Thought 1×`) is where the counts
live, and the scenario asserts those counts. The scenario is deterministic: two
runs produce byte-identical PNGs, because nothing in the frame reads the clock.

### Five desktops on one home screen: the `fleet-*` scenarios

The home screen opens on the recency page — every conversation, newest reply
first — and with more than one paired desktop the list merges them
(`mergeSessions`, the picker's *all desktops*). `?fleet=1` swaps in five machines
with fourteen conversations between them (`mobile/shot/mock/fleet.ts`):

```bash
python3 scripts/screenshots/capture.py capture-mobile fleet-zh          # → .screenshots/home-devices-zh.png
python3 scripts/screenshots/capture.py capture-mobile fleet-en          # → .screenshots/home-devices-en.png
python3 scripts/screenshots/capture.py capture-mobile fleet-picker-zh   # → .screenshots/home-devices-picker-zh.png
python3 scripts/screenshots/capture.py capture-mobile fleet-picker-en   # → .screenshots/home-devices-picker-en.png
```

The two list scenarios assert the merge rather than the picture: the row order
is the reply time each machine reported (not the machine), all five machines
contribute rows, and the two captions count 2 and 12. The `-picker-` pair opens
the sheet above the list, which is where the five machines are *named* — a row
names its machine by glyph alone, because a name there would eat the title's
width.

Nothing in either capture is still running: a `running` or `queued` row draws an
`ActivityIndicator`, whose frame differs every capture, so the fixture has both
unsettled conversations blocked on the reader's approval instead. Two runs
produce byte-identical PNGs.

### The transcript is upright (the inverted list's extra flip)

`mobile/src/features/chat/components/ChatTimeline.tsx` renders the transcript as
an `inverted` FlatList. react-native-web implements that with two flips — the
scroll view, and every cell back — which cancel out and put the newest row at the
bottom. A list that also has a `refreshControl` gets a *third* one:
`ScrollView.render` clones the refresh control with the scroll view's own style
(`React.cloneElement(refreshControl, { style: props.style })` in
`dist/exports/ScrollView/index.js`), which carries the inversion. The transcript
then renders upside down — glyphs mirrored, rows reversed — which a phone does
not, so such a capture disagrees with the device. (Reproduced with
react-native-web 0.21.4, the version `serve-mobile` installs today.)

`cdp.mjs` neutralises that one flip in the page for mobile captures
(`UNFLIP_INVERTED_LIST`): it finds the wrapper whose only child is the mirrored
scroller, takes its `transform` away and re-asserts it as React re-renders. No
product code and no dependency version changes, and the correction is visible in
the DOM. Every run reports what it did:

```
cdp: neutralized 1 inverted-list wrapper(s) (react-native-web gives a refresh control the scroll view's inversion)
```

The `chat` scenario asserts the outcome (`the transcript is not mirrored`), so a
mirrored transcript fails the capture instead of being published, and the
workaround cannot rot unnoticed. To judge a *capture* rather than a run:

```bash
python3 scripts/screenshots/capture.py check-upright-mobile chat
```

That captures the scenario with `upright-probe.js`, which puts the transcript's
own text beside an unflipped clone of itself in a single screenshot, and
correlates the two tiles straight and vertically mirrored. The clone hangs in
`document.body`, so it is upright by construction — no device, no second browser,
no OCR and no human reading it:

```
   in-place tile: 348x64 css px at 21,389
   control tile: 348x64 css px at 7,773
   upright  correlation 1.000   (the transcript against an unflipped copy of itself)
   mirrored correlation 0.205   (the same comparison, control flipped top-to-bottom)
   verdict: upright — the capture reads the right way up
```

Mirrored, the two numbers swap and the command exits non-zero; scores within 0.05
of each other are reported as inconclusive rather than as a pass. The judge and
that direction are pinned offline by
`scripts/tests/test-screenshot-upright-check.py` (`make test-screenshot-harness`).

The probe judges the scroller that is *visible*, not the tallest one: the phone
keeps the session list mounted underneath an open conversation, so a rail long
enough to scroll is a taller scroller of its own. Before that filter it outlined a
row of the list behind the chat, and the check reported "no in-place tile" for a
transcript that was perfectly upright (`parser-zh` / `parser-en`, whose rail is
long). It also takes a *paragraph* — a text block whose children are all inline
runs — rather than only a childless element, because a frame whose largest plain
leaf is one 13 px code line says nothing either way (0.045 upright against 0.053
mirrored: inconclusive).

## Scenarios

Scenarios are data, not code: `scripts/screenshots/scenarios.json`. Each one is a
short list of steps that a person could replay by hand.

```json
"rail-pin-menu": {
  "steps": [
    { "wait": 900 },
    { "tap": "单细胞转录组 的操作" },
    { "wait": 1500 },
    { "shot": "d-rail-pin-menu.png" }
  ]
}
```

Step kinds: `eval`, `wait`, `shot`, `tap` (by accessible name), `tapText` (by
visible text), `hover`, `type`, `key`, `scroll`, `inject`, plus the annotation,
offset and measurement steps below.
Three platform-level knobs are worth knowing:

- **`ready`** is a JavaScript expression the driver polls until it is true, and it
  is what keeps captures fast: a warm bundle is ready in about a second, a cold
  one can take 20. `settle` is only a small grace period after that. If a capture
  starts before the app paints, raise `readyTimeout` rather than `settle`.
- **`lang` pins the browser locale, not a harness-only switch.** A scenario that
  has to be in a language says `"lang": "en-US"`, and `cdp.mjs` makes the page's
  system locale that value before the app's first script runs: `Emulation`
  `setLocaleOverride` moves `Intl`, and the user agent override's `acceptLanguage`
  moves `navigator.language` (measured on Chrome 156 — the locale override alone
  leaves `navigator.language` at the host's `zh-CN`). The app reads the system
  locale exactly as it does on a phone; a stored in-app preference still wins.
- **`query` appends to the platform's URL.** `"query": "?parser=1"` keeps a
  scenario on whatever port `serve-mobile` is serving, instead of pinning
  `http://localhost:8099/` in the scenario the way a full `url` does.
- **`tap` needs an accessible name.** It matches `aria-label` first, then the
  element's own text for interactive roles. Where neither exists, use `tapText`;
  if you find yourself writing coordinate taps, that is a hint the control is
  missing a label.
- **Prefer `eval` for state a click cannot reach.** A control that only appears on
  hover needs a `hover` step on its container first (the driver reports a
  zero-sized target instead of clicking the page corner). For example the composer
  search opens on ⌘F, which the driver cannot send as a keystroke, so the scenario
  navigates to `?press=meta%2Bf` and `desktop/shot/main.tsx` dispatches the
  shortcut at the real window listener.
- **An assertion inside an `eval` step is a test.** A step eval that throws
  fails the capture run (it counts toward `step(s) could not reach their
  target`, same as an unreachable tap), which is what makes "the screen must be
  in this state" worth writing: `if (document.body.innerText.includes('正在生成')) throw new Error('…');`.
  The driver's own probes (`ready`, optional element reads) stay non-fatal.

**Fragments in `scenarios.d/`:** when two sessions add scenarios for different
platforms at the same time, neither edits `scenarios.json` (the later write would
lose the other's work). Each writes a fragment under
`scripts/screenshots/scenarios.d/` instead, and `capture.py` merges those on top
of the shared table at startup, in filename order:

```json
{ "mobile": { "audit-settings-bottom": { "steps": [ { "wait": 900 }, { "shot": "m-audit-settings-bottom.png" } ] } } }
```

A fragment hangs its entries under the platform it names and wins over a
same-named entry (useful for a temporary override). A wrong platform name or a
malformed fragment is an error rather than a silently empty capture.
`make test-screenshot-harness` pins those rules.

**A per-scenario `ready` is how readiness stops being a race.** `capture.py`
passes `spec.get("ready", config["ready"])`, so a scenario that needs more than
"the app painted something" can say so: the dark-window theme picker asserts
`document.documentElement` carries `dark`, so its readiness includes that class
instead of relying on a `wait` long enough to cover a cold bundle. Put the
condition in `ready` and keep the assertion as the assertion.

**What a capture cannot judge: `hitSlop`.** `react-native-web` does not implement
it — an `elementFromPoint` probe returns the drawn box — so a mobile fix that
only enlarges the touch area is invisible to both screenshots and the `TOUCH`
probe. Those are held by unit tests instead (`mobile/src/features/chat/messages/
touchTargets.ts`, asserted in `foldedRowTouchTargets.test.ts`), and the real
effect is observable on a device only. Do not "fix" such a scene by widening the
drawn box: that changes the design, not the target.

`desktop/shot/prefs.ts` (loaded before `main.tsx`) pins the preferences that are
read at import time: `?lang=en`, `?theme=dark|light|system` and
`?palette=<palette id>` (see `lib/theme.ts` `THEME_PALETTES`). It also *defaults*
them (zh / light / FutureOS), so a scenario that switches the color theme cannot
leak it into the next capture in the same run, and a dark-mode machine still
captures the screen most users see. `?settings=key:value,...` seeds app settings
before boot; `?remoteSlowRead=1` holds a host's answers back, which is the only
way a capture can show the state a reader is in *while* one is in flight;
`mobile/shot/mock/shareIntent.ts` reads `?share=1` so the share-intake sheet
appears only in the share scenarios.

### Pointer indicator and callouts

The driver draws these inside the page, so they appear in both stills and video.

- **The pointer indicator** follows every `tap`/`hover`: a blue dot, plus an
  expanding ring at the moment of a tap. It is **on by default while recording**
  (mobile footage is unfollowable without it) and **off by default for stills**, so
  a product shot does not gain a stray dot just because the scenario tapped
  something. Ask for it in a still with `--pointer true`, or place it deliberately
  with a `{"pointer": [x, y]}` step.
- **Callouts** come from a `marks` step: a numbered badge on the point plus a
  label beside it. They are what turn a screenshot into an explanation.
- **`--annotate false`** removes all of the above for a clean capture.

```json
{ "marks": [{ "at": "工作区", "label": "① 工作区列表", "dy": -34 },
            { "at": "对话",   "label": "② 对话列表，可左右滑动切换", "dy": -34 }] }
```

Each mark takes either `at` (an accessible name, same matching as `tap`) or raw
`x`/`y`. `dx`/`dy` shift the badge and `side` (`"left"`/`"right"`) puts the label
on the other side; adjacent targets usually need one of these, otherwise two
labels land on top of each other. Numbers count up across the scenario; use
`{"marksClear": true}` to start a fresh set, and `{"pointerHide": true}` to get
the dot out of a clean product shot.

## Nothing is committed

Two deliberate rules, so this stays a tool rather than a pile of binaries:

- **Captured screenshots are never checked in.** They land in `.screenshots/`,
  which is gitignored; so is `desktop/shot/assets/`, the demo figures
  `gen-demo-assets.py` generates for the conversations. `capture.py` regenerates
  those figures automatically when they are missing (it warns, and captures
  anyway, if matplotlib is absent).
- **The harness-only web dependencies are never saved to a manifest.**
  `serve-mobile` installs `react-native-web`, `@expo/metro-runtime` and
  `react-dom` into the mobile workspace with `--no-save`, so `mobile/package.json`
  and `package-lock.json` are untouched. Run `npm install` afterwards and the tree
  goes back to the app's own dependencies.

`mobile/shot/assets/` is intentionally absent: the mobile harness reads the
figures over HTTP instead (`assetsPort` in `scenarios.json`), so both platforms
share one copy.

## Building a document

`capture.py pdf` turns captured PNGs plus a content file into a PDF (this is how
`FutureOS 1.1.8 功能更新说明.pdf` was produced):

```bash
python3 scripts/screenshots/capture.py \
  --out .screenshots \
  pdf scripts/screenshots/examples/release-notes-1.1.8.json "$HOME/Documents/notes.pdf"
```

The content file's schema is visible in that example: a cover, an intro, a table
of contents, and sections whose items each carry prose plus one figure or a pair
of figures. Captions should say what to look at, not restate the prose.

For command-line output, capture the real stdout and render it in a terminal
frame:

```bash
python3 scripts/screenshots/capture.py terminal t-headless.png -- \
  target/debug/futureos --headless
```

## When the app changes

The mocks are the only part that tracks the product, and misses are loud rather
than silent:

- The desktop mock logs `[mock] UNHANDLED COMMAND <name>` in the browser console
  for any command it does not answer. Both the capture output and the browser's
  console are the fastest way to see what a new screen needs.
- The mobile mock returns a no-op for an unknown context member and warns
  `[shot] mock RemoteContext has no "<name>"`. Add a real value so the screen
  renders meaningfully.
- If a screen needs new demo content, extend the data modules rather than adding
  special cases to the mocks.

## Layout

| Path | Role |
|---|---|
| `scripts/screenshots/capture.py` | Entry point: serve, capture, terminal frames, PDF assembly |
| `scripts/screenshots/cdp.mjs` | Chrome DevTools Protocol driver (viewport, input, screenshots, screencast frames) |
| `scripts/screenshots/scenarios.json` | The scenario table for both platforms |
| `scripts/screenshots/document.css` | Print stylesheet for generated documents |
| `scripts/screenshots/terminal.html` | Terminal-styled frame for CLI output |
| `scripts/screenshots/tui_capture.py` | Runs the TUI in a pty of a fixed size and keeps its bytes (recipe 8) |
| `scripts/screenshots/tui_terminal.py` | Terminal cell model that replays those bytes into a PNG |
| `scripts/screenshots/injects/` | Variant-styling JS (see recipe 5) |
| `scripts/screenshots/upright-probe.js` | Puts a capture's own text beside an unflipped copy of it (see the mobile upright check) |
| `scripts/screenshots/examples/` | Worked example: the 1.1.8 release notes content |
| `scripts/screenshots/gen-demo-assets.py` | Regenerates the demo figures (needs matplotlib) |
| `desktop/shot/` | Desktop mocks, demo data, harness entry, stand-in PTY server |
| `mobile/shot/` | Mobile mocks and demo data |

## Troubleshooting

| Symptom | Cause |
|---|---|
| `cdp: no page target on port …` | An earlier run's **headless** browser still holds the CDP port with no page target; close it or pass `--cdp-port`. A *visible* browser there is reported and skipped rather than driven. |
| `note: CDP port … is held by …` / `… is a visible browser …` | Expected, not an error: that browser is not one this run may drive, so the capture started its own on the next free port. |
| `target not found: …` | The `aria-label` changed, or the step ran before the screen settled — raise the preceding `wait`. |
| A tap does nothing | Inside a scrollable list, react-native-web prefers the scroll responder; the driver already sends touch sequences, so check `--touch` was not disabled. |
| Empty or partial mobile screen | Expo web is still bundling on the first request; wait for the first capture, then re-run. If the page is blank and the console reports "Incompatible React versions", the mobile workspace's react/react-dom pair drifted — `serve-mobile` checks this and prints the fix. |
| `cdp: app not ready after … ms` with the UI stuck on its startup state | The new screen calls a command the mock does not implement. Unknown commands return null, so the frontend throws while rendering and the whole UI stops (on desktop it stays on the splash). Search the console for `[mock] UNHANDLED COMMAND` — that names the missing handler. |
| A pinch capture comes out blank (or the picture drifts away from the fingers) | The zoom is anchored on a window origin that was measured once per layout: a sheet that slides in lays out while it is still off-screen, so the stored origin stays most of a screen away and every pinch then translates the picture by `offset × (scale − 1)`, straight out of view. Anchor on the touches' coordinates *relative to the surface* instead, and assert in the scenario that a centred pinch leaves the translate near zero — that step fails loudly on the old behaviour. |
| `error: nothing is listening on port …` | The dev server is not running: start `capture.py serve-desktop|serve-mobile` first. |
