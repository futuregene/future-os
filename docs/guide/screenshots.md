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
  and `react-dom`; `make screenshots-mobile` installs them into the mobile
  workspace on demand and leaves the manifests alone (see *Nothing is committed*).

## Desktop

Two terminals, because the dev server stays in the foreground:

```bash
make screenshots-desktop        # terminal 1: serves http://localhost:5299/shot/
make screenshots-capture-desktop   # terminal 2: runs every scenario
make screenshots-capture-desktop -- d-rail-tree d-steps-folded   # or pick some
```

The captured PNGs land in `.screenshots/` (gitignored). Open
<http://localhost:5299/shot/> yourself to explore by hand; the harness also starts
a stand-in PTY server on port 7391 so the embedded terminal panel has something
to show.

## Mobile

```bash
make screenshots-mobile         # terminal 1: Expo web on http://localhost:8099/
make screenshots-capture-mobile    # terminal 2
```

The mobile harness renders at a phone viewport (390×844 at 2× by default) and
serves the demo figures over HTTP, so inline images and the zoomable preview
resolve. `make screenshots-mobile` exports `SHOT_WEB=1`; that flag is the only
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
visible text), `hover`, `type`, `key`, `scroll`. Three platform-level knobs are
worth knowing:

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

## Nothing is committed

Two deliberate rules, so this stays a tool rather than a pile of binaries:

- **Captured screenshots are never checked in.** They land in `.screenshots/`,
  which is gitignored; so is `desktop/shot/assets/`, the demo figures
  `gen-demo-assets.py` generates for the conversations. `capture.py` regenerates
  those figures automatically when they are missing (it warns, and captures
  anyway, if matplotlib is absent).
- **The harness-only web dependencies are never saved to a manifest.**
  `make screenshots-mobile` installs `react-native-web`, `@expo/metro-runtime` and
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
  for any command it does not answer. Both `make screenshots-*` and the browser's
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
| `scripts/screenshots/cdp.mjs` | Chrome DevTools Protocol driver (viewport, input, screenshots) |
| `scripts/screenshots/scenarios.json` | The scenario table for both platforms |
| `scripts/screenshots/document.css` | Print stylesheet for generated documents |
| `scripts/screenshots/terminal.html` | Terminal-styled frame for CLI output |
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
| Empty or partial mobile screen | Expo web is still bundling on the first request; wait for the first capture, then re-run. If the page is blank and the console reports "Incompatible React versions", the mobile workspace's react/react-dom pair drifted — `make screenshots-mobile` checks this and prints the fix. |
| `error: nothing is listening on port …` | The dev server is not running: start `make screenshots-desktop|mobile` first. |
