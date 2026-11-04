# UI harness — end-to-end tests that watch the product like a person

A multi-surface harness for FutureOS. It drives the **real desktop UI**, the
**real mobile UI (Expo web)**, the **real TUI (in a pty)** and the **real CLI**,
renders the desktop against a **real database snapshot**, records what happened
on every channel available (screenshots, video, console, network, IPC, database,
CPU/memory, stdout/stderr), then has a vision model and a text model look for
defects. Findings become a report; high-severity ones become issue drafts that a
human approves before anything is filed.

The goal is not "a test suite that passes". It is a case library that keeps
finding real problems as the product changes — every case is readable enough to
replay by hand, and every run leaves evidence a person can check.

## Surfaces

| Surface | What it drives | Data | Case field |
|---|---|---|---|
| `desktop` | real desktop frontend in headless Chrome (1280×860) | copy of the real `app.db`, via the fixture server | default |
| `mobile` | real mobile app via Expo web (`SHOT_WEB=1`, 390×844, touch) | the mobile shot mocks (synthetic fleet/parser data) | `"surface": "mobile"` |
| `tui` | real `future tui` in a pty, rendered through the terminal cell model | `demo_agent` (scripted gRPC) | `"surface": "tui"` |
| `cli` | real `future` binary in an isolated `FUTURE_HOME` | copy of the real `app.db` + settings/tasks snapshot | `"surface": "cli"` |
| `remote` | desktop and mobile clients at the same time | shot mocks on both sides | `"surface": "remote"` |

Two boundaries stated up front: `mobile`/`remote` use the screenshot harness's
synthetic remote dataset (the pair check proves both clients render the same
session, not a live NATS link), and `tui` uses `demo_agent` rather than a real
model. `desktop` and `cli` run against real database snapshots.

## Quick start

```bash
node harness/cli.mjs prepare              # snapshot ~/.future/app/app.db (once)
node harness/cli.mjs list                 # the case library, all surfaces
node harness/cli.mjs run --case create-chat-and-delete      # desktop
node harness/cli.mjs run --case cli-workspace-add-list-archive   # CLI
node harness/cli.mjs run --case tui-help                     # TUI
node harness/cli.mjs run --case mobile-home-list             # mobile
node harness/cli.mjs run --case remote-parser-pair           # desktop + mobile
node harness/cli.mjs analyze --run harness/runs/<id> --mode full
node harness/cli.mjs report  --run harness/runs/<id>
```

`run` starts only the servers the selected cases need: Vite for desktop, Expo
for mobile, `demo_agent` for TUI, nothing for CLI. Artifacts go to
`harness/runs/<run-id>/<case-id>/`.

Each run gets its **own free port** for Vite/Expo. A fixed port is how a run
ends up testing another worktree's code: `:5299` may already be a dev server
from a different branch. Pass `--reuse-server` when you deliberately want to
attach to the conventional ports (5299 / 8099).

## What is real, what is substituted

| Real | Substituted |
|---|---|
| Every component, hook, store, style and i18n string under `desktop/src/` | Tauri `invoke` — answered by `harness/lib/fixture-server.mjs` |
| The database schema and 700+ real conversations / workspaces | The store queries, reimplemented in JS over a **copy** of `app.db` |
| Clicks, typing, scrolling, hover (CDP input events) | `agent_prompt` streaming — still the screenshot mock, no model is called |
| Chrome's rendering, CSS, layout, React lifecycle | Conversation transcripts — synthesised from the thread title |
| Console / network / DOM / performance / process RSS | Paired devices — none in the fixture (single-machine runs) |

**What this does not prove:** backend Rust/SQL logic, real model answers, real
network conditions, or mobile. Those need a different harness; this one is
deliberately the fastest way to exercise the desktop UI against realistic data.

## Architecture

```
harness/
  cli.mjs                 prepare | list | serve | run | analyze | report | issues
  cases/{core,mobile,tui,cli,remote}/  the case library (data, not code)
  lib/
    data.mjs              SQLite backup + CLI home snapshot
    fixture-server.mjs    Tauri command surface over a per-case DB copy
    cdp.mjs               Chrome DevTools driver + observations (desktop/mobile)
    run.mjs               desktop/mobile case execution and evidence bundle
    cli-driver.mjs        real `future` in an isolated FUTURE_HOME
    tui-driver.mjs        TUI case marshalling + result folding
    remote-driver.mjs     desktop + mobile clients in one case
    video.mjs             screencast frames → mp4 (real timing)
    analyze.mjs           deterministic signals → vision → synthesis
    report.mjs            report.md
    issues.mjs            GitHub issue drafts (human approval by default)
  tui/driver.py           pty driver, terminal cell model, PNG frames
  .data/                  base.db + models.json + cli snapshot (gitignored)
  runs/                   per-run evidence (gitignored)
```

The browser hook is `desktop/shot/mock/core.ts`: with `?fixture=http://…` it
forwards every `invoke` to the fixture server, and **falls back to the in-page
demo mock** for commands the server does not implement. That is why settings
panels and remote screens still work while the data surface is real.

Each desktop case gets its own database file (`harness/runs/<id>/<case>/app.db`),
so a case that deletes a conversation cannot affect the next one. The base
snapshot is made with SQLite's backup API, so the WAL is folded in. A CLI case
gets a whole isolated `FUTURE_HOME` (`<case>/home/`) instead: a copy of the same
`app.db` plus snapshotted settings, skills catalogue and tasks database, with
`FUTURE_AGENT_GRPC_ADDR` pointed at a closed port so an offline command can never
reach the user's running agent.

## A case is data

```json
{
  "id": "create-chat-and-delete",
  "title": "新建会话 → 发送 → 删除会话",
  "area": "conversation",
  "priority": "P0",
  "description": "为什么做这件事、用户会怎么操作",
  "preconditions": "应用已启动；数据库是真实副本",
  "expectations": ["发送后会话出现在列表顶部", "删除后列表与数据库都消失"],
  "steps": [
    { "click": { "aria": "新建对话" } },
    { "click": { "selector": "[role=textbox]" } },
    { "type": "测试会话" },
    { "click": { "aria": "发送" } },
    { "db": { "sql": "select count(*) n from threads where deleted_at is null", "delta": 1 } },
    { "shot": "created" }
  ]
}
```

Steps: `click` / `clickText` / `hover` / `type` / `press` / `scroll` / `wait` /
`waitFor` / `eval` / `shot` / `assert` / `expectText` / `expectAbsent` / `reload`
/ `db` / `server`.

- Targets accept `{aria}`, `{text}`, `{selector}` (and `{selector, text}` for
  menu items). Matching prefers an exact accessible name, then an exact visible
  text, then a prefix — the same ranking the screenshot harness uses.
- `db` runs a **read-only SELECT** against the case's database and can assert a
  value or a delta; `as` stores the value for `{{name}}` interpolation in later
  steps.
- `server` asserts how many times the UI invoked a given IPC command (and
  optionally its arguments) — the "did it call the right thing?" channel.
- Every non-wait step takes a screenshot automatically; explicit `shot` steps
  are the ones the vision model reads.

Other surfaces use the same case file shape with their own verbs:

- `surface: "tui"`: `wait`, `press`, `type`, `assertText` / `expectAbsent`,
  `shot`. `press` accepts named keys (`enter`, `esc`, `tab`, `up`, `ctrl+d`,
  `shift+right`, …) or a single character.
- `surface: "cli"`: `run` (argument array), `expectExit`, `expectStdout`,
  `expectStderr`, `expectJson` (`path`/`equals`/`lengthMin`/`contains`/`exists`),
  `db`, `mkdir`. One case gets one isolated `FUTURE_HOME`.
- `surface: "remote"`: `pages` maps each client to a query string, and every
  step names `surface: "desktop" | "mobile"`.
- `surface: "mobile"`: the desktop verbs, dispatched as touch input at a
  390×844 viewport.

## Observation channels

| Channel | Where | What it catches |
|---|---|---|
| Screenshots | `shots/` | visual state after every action, plus the failure frame |
| Video | `video.mp4` | timing, animation, flicker, "it flashed and disappeared" |
| Console / exceptions | `result.json` | React crashes, unhandled rejections |
| Network | `result.json` | failed requests, 4xx/5xx, missing assets |
| IPC log | `server.jsonl` | wrong command, wrong args, silent no-op |
| Database | case `app.db` | writes that did not happen, soft-delete semantics |
| Performance | `metrics[]` | JS heap, DOM nodes, main-thread task time, Chrome RSS |
| Server fallbacks | `server.fallbacks` | surfaces still answered by the demo mock (known boundary) |
| TUI screen text | `result.json` / timeline | clipped rows, wrong panel, missing shortcut |
| CLI stdout/stderr/exit | `run-*.stdout.log`, `result.json` | wrong JSON shape, misleading success, missing guidance |

The player's own controls (pause / skip back / skip forward) are not part of the
file: the recording is Chrome's screencast of the page. The analysis never
screenshots a player — it reads the per-step PNGs and a filmstrip that `ffmpeg`
extracts from the mp4 — so those controls cannot leak into a finding.

## Analysis

1. **Deterministic** (`--mode heuristic`): failed checks, exceptions, console
   errors, network failures, IPC errors, heap/main-thread growth, slow steps.
2. **Vision** (`--mode full`): up to three key screenshots per case are read with
   the case's own expectations as the question. If the analysis model accepts
   image input (its `models.json` entry lists `image`), the screenshots are
   attached to that model directly (`future run --image`); otherwise the
   `read_image` tool is the fallback. This is where "the page is blank", "the
   text is clipped" or "the dialog is empty" get found without an assertion
   having encoded them.
3. **Synthesis** (`--mode full`): `future run --mode json` reads the timeline,
   deterministic signals and vision notes, and returns findings as strict JSON.

Analysis is resumable and non-fatal: a missing CLI or a failed vision call is
recorded and the remaining evidence still becomes a report.

## Report and issues

`report.md` has a pass/fail table, a findings table, per-finding detail, links
to each case's evidence (screenshots/video), reproduction commands, and an
explicit boundary section. `issues` writes one markdown draft per high/medium
finding under `runs/<id>/issues/`; `--create --repo owner/name --yes` files them
with `gh`. Nothing is filed without that explicit flag.

## Case library rules

- One case, one user-visible goal. A long workflow that cannot be replayed from
  its description is two cases.
- An expectation is something a person can see or a SQL query can prove. "It
  works" is not an expectation.
- Prefer real data: take titles/ids from the DB with `db … as`, never hard-code
  a row that could change tomorrow.
- A case that fails because the product is wrong stays red. It is a defect
  record, not a broken test; do not weaken the assertion to make the run green.
- Add a case when a defect is found: the smallest sequence that reproduces it.

## Known gaps (next iterations)

- **Transcripts are synthetic on desktop too.** Real history lives in the 1 GB
  `agent.db` projection layer; only thread metadata is wired to real data today.
- **Store queries are reimplemented.** A bug in `host/src/store/*.rs` cannot be
  found here; the harness proves UI behaviour against real-shaped data.
- **`agent_prompt` is still mocked.** Sending does not call a model, so reply
  content and streaming states are out of scope for now.
- **Mobile and remote use the shot mocks.** They exercise the real mobile UI and
  both layouts, but not a live NATS link; a two-host relay harness is the next
  step.
- **TUI uses `demo_agent`.** It verifies the terminal UI's behaviour, not a real
  model answer; pointing it at a real agent is a flag away once an agent home
  fixture exists.
- **CLI cases are offline.** They cover commands that read `app.db`/settings/
  tasks without an agent; agent-backed commands need a fixture agent.
- **Video is analyzed as a filmstrip, not as motion.** Six sampled frames catch
  obvious state changes; it is not a temporal model.
