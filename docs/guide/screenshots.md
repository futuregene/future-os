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
python3 scripts/screenshots/capture.py capture-desktop chat
cp .screenshots/d-chat.png /tmp/y.png
# switch to version X (or use another worktree) and capture the same scenario
python3 scripts/screenshots/capture.py capture-desktop chat
python3 scripts/screenshots/capture.py compare-desktop /tmp/y.png .screenshots/d-chat.png \
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

## Recording video

```bash
# terminals 1 and 2: start the servers (as above)
python3 scripts/screenshots/capture.py video-desktop              # every scenario
python3 scripts/screenshots/capture.py video-desktop chat rename  # or just some
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
- **Two things are stylised, not captured**: command-line output is rendered
  through `terminal.html` from a command's real stdout, and anything that lives
  only in the TUI (`future-tui`) is out of scope.

## Requirements

- Node.js (the driver uses `ws`, already a repo dependency) and Python 3.
- Chrome, Edge or Chromium. Safari is not supported (the driver needs CDP).
- `pip install pillow` is optional: it only shrinks images in the generated PDF.
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
python3 scripts/screenshots/capture.py capture-desktop d-rail-tree d-steps-folded              # or just some
```

The captured PNGs land in `.screenshots/` (gitignored). Open
<http://localhost:5299/shot/> yourself to explore by hand; the harness also starts
a stand-in PTY server on port 7391 so the embedded terminal panel has something
to show.

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

`desktop/shot/main.tsx` accepts `?lang=en` and `?settings=key:value,...` to pin
the UI language and seed app settings before boot; `mobile/shot/mock/shareIntent.ts`
reads `?share=1` so the share-intake sheet appears only in the share scenarios.

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
  desktop/src-tauri/target/debug/futureos --headless
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
| `scripts/screenshots/injects/` | Variant-styling JS (see recipe 5) |
| `scripts/screenshots/examples/` | Worked example: the 1.1.8 release notes content |
| `scripts/screenshots/gen-demo-assets.py` | Regenerates the demo figures (needs matplotlib) |
| `desktop/shot/` | Desktop mocks, demo data, harness entry, stand-in PTY server |
| `mobile/shot/` | Mobile mocks and demo data |

## Troubleshooting

| Symptom | Cause |
|---|---|
| `cdp: no page target on port …` | A stale browser from an earlier run holds the CDP port; close it or pass `--cdp-port`. |
| `target not found: …` | The `aria-label` changed, or the step ran before the screen settled — raise the preceding `wait`. |
| A tap does nothing | Inside a scrollable list, react-native-web prefers the scroll responder; the driver already sends touch sequences, so check `--touch` was not disabled. |
| Empty or partial mobile screen | Expo web is still bundling on the first request; wait for the first capture, then re-run. If the page is blank and the console reports "Incompatible React versions", the mobile workspace's react/react-dom pair drifted — `serve-mobile` checks this and prints the fix. |
| `error: nothing is listening on port …` | The dev server is not running: start `capture.py serve-desktop|serve-mobile` first. |
