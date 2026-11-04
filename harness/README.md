# UI harness — end-to-end tests that watch the product like a person

A browser-driven harness for the FutureOS desktop UI. It opens the **real
frontend**, renders the **real database snapshot**, performs the same clicks a
tester would, records what happened on every channel available (screenshots,
video, console, network, IPC, database, CPU/memory), then has a vision model and
a text model look for defects. Findings become a report; high-severity ones
become issue drafts that a human approves before anything is filed.

The goal is not "a test suite that passes". It is a case library that keeps
finding real problems as the product changes — every case is readable enough to
replay by hand, and every run leaves evidence a person can check.

## Quick start

```bash
node harness/cli.mjs prepare              # snapshot ~/.future/app/app.db (once)
node harness/cli.mjs list                 # the case library
node harness/cli.mjs run --case create-chat-and-delete
node harness/cli.mjs analyze --run harness/runs/<id> --mode full
node harness/cli.mjs report  --run harness/runs/<id>
node harness/cli.mjs issues  --run harness/runs/<id>        # drafts only
```

`run` starts its own Vite dev server (or reuses one on `:5299`), one headless
Chrome, one database copy per case, and one fixture server per case. Artifacts go
to `harness/runs/<run-id>/<case-id>/`.

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
  cases/**/*.json         the case library (data, not code)
  lib/
    data.mjs              SQLite backup of the real app.db → .data/base.db
    fixture-server.mjs    Tauri command surface over a per-case DB copy
    cdp.mjs               Chrome DevTools driver + observations
    run.mjs               case execution and evidence bundle
    video.mjs             screencast frames → mp4 (real timing)
    analyze.mjs           deterministic signals → vision → synthesis
    report.mjs            report.md
    issues.mjs            GitHub issue drafts (human approval by default)
  .data/                  base.db + models.json (gitignored)
  runs/                   per-run evidence (gitignored)
```

The browser hook is `desktop/shot/mock/core.ts`: with `?fixture=http://…` it
forwards every `invoke` to the fixture server, and **falls back to the in-page
demo mock** for commands the server does not implement. That is why settings
panels and remote screens still work while the data surface is real.

Each case gets its own database file (`harness/runs/<id>/<case>/app.db`), so a
case that deletes a conversation cannot affect the next one. The base snapshot
is made with SQLite's backup API, so the WAL is folded in.

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

## Analysis

1. **Deterministic** (`--mode heuristic`): failed checks, exceptions, console
   errors, network failures, IPC errors, heap/main-thread growth, slow steps.
2. **Vision** (`--mode full`): up to three key screenshots per case are read by
   `future tools call read_image`, with the case's own expectations as the
   question. This is where "the page is blank", "the text is clipped" or "the
   dialog is empty" get found without an assertion having encoded them.
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

- **Transcripts are synthetic.** Real history lives in the 1 GB `agent.db`
  projection layer; only thread metadata is wired to real data today.
- **Store queries are reimplemented.** A bug in `host/src/store/*.rs` cannot be
  found here; the harness proves UI behaviour against real-shaped data.
- **`agent_prompt` is still mocked.** Sending does not call a model, so reply
  content and streaming states are out of scope for now.
- **Mobile** is not driven yet; the same CDP driver can attach to
  `serve-mobile` when a mobile case library is worth the runtime.
- **Video is not yet fed to the model** — only stills are; keyframe sampling is
  the next cheap step.
