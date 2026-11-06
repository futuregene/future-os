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
| `remote-live` | a real self-hosted relay: `future relay init` + `nats-server` + `future relay serve` + `future host --relay`, paired by a real second home | real processes, each with its own `FUTURE_HOME` | `"surface": "remote-live"` |
| `remote-cn` | the deployed Future platform over the network (read-only contract check; full pairing on request) | the real platform at `future-os.cn` | `"surface": "remote-cn"` |
| `remote-mobile-live` | the **phone's own client code** (`mobile/src/remote/`) against a real host | real relay + real `future host`; only the expo platform layer is stubbed | `"surface": "remote-mobile-live"` |

Two boundaries stated up front: `mobile`/`remote` use the screenshot harness's
synthetic remote dataset (the pair check proves both clients render the same
session, not a live NATS link), and `tui` uses `demo_agent` rather than a real
model. `desktop` and `cli` run against real database snapshots. The relay itself
is covered — for real — by `remote-live`, which runs the actual binaries.

The three remote surfaces answer three different questions, and none of them
substitutes for the others:

| Surface | Question it answers | What is fake |
|---|---|---|
| `remote` | do the desktop and the phone render the same session the same way | both clients talk to shot mocks |
| `remote-live` | does a pairing actually work end to end (relay, broker, invitation, bridge, device book) | nothing; all four processes are real, on one machine |
| `remote-cn` | is the **official** relay's control plane still the contract we ship against | nothing; a real network call to `future-os.cn` |

The official path needs a signed-in account (`future auth login`), which an API
key is not. `remote-cn` says so and runs the read-only contract checks anyway,
which is what catches a moved route, a changed error shape or an outage. Set
`FUTURE_REMOTE_CN_FULL=1` on a signed-in machine to pair a real device through
the deployed platform; it removes the pairing again before returning. Cases
marked `needsNetwork` stay out of a default run and are selected by id or tag.

## Quick start

```bash
node harness/cli.mjs prepare              # snapshot ~/.future/app/app.db (once)
# CLI/TUI cases run the built binaries: build both with one FUTURE_VERSION, or the
# TUI refuses to attach and every TUI case fails on its own unrelated assertion.
FUTURE_VERSION=$(node scripts/version.mjs) \
  cargo build -p future-cli -p future-tui --example demo_agent
node harness/cli.mjs list                 # the case library, all surfaces
node harness/cli.mjs run --case create-chat-and-delete      # desktop
node harness/cli.mjs run --case cli-workspace-add-list-archive   # CLI
node harness/cli.mjs run --case tui-help                     # TUI
node harness/cli.mjs run --case mobile-home-list             # mobile
node harness/cli.mjs run --case remote-parser-pair           # desktop + mobile
node harness/cli.mjs run --tag perf                          # latency/CPU/memory/network
node harness/cli.mjs analyze --run harness/runs/<id> --mode full
node harness/cli.mjs report  --run harness/runs/<id>
```

`COVERAGE.md` is the honest matrix of what the library exercises and what it
does not; add a row there when adding a case.

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
    provenance.mjs        which values the harness wrote, and what that rules out
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

Three layers, cheapest first, each answering a question the one before it cannot.

1. **Deterministic** (`--mode heuristic`): failed checks, exceptions, console
   errors, network failures, IPC errors, heap/main-thread growth, slow steps.
   Answers "did the contract hold" — precise, but it can only see what someone
   wrote down in advance.
2. **Model acceptance** (`--mode full`, `judge` in a case): the ordered step
   screenshots, a 12-frame filmstrip cut from the recording, the final page text,
   console/exception/network logs, CLI runs **and the performance numbers with
   their per-repetition series** go to the model, against natural-language
   criteria ("a person clicking this should see …"). Answers "does this look like
   a working product, and are these numbers plausible" — including the dynamic
   and layout problems no assertion encoded.
3. **Synthesis** (`--mode full`): `future run --mode json` reads the timeline,
   deterministic signals and the per-case verdicts, and returns findings as
   strict JSON.

The judge is told **which deterministic checks already passed on the live DOM**
and not to re-litigate them. Without that instruction a filmstrip misread
produced a medium "the panel does not show the thread you clicked" finding on a
case whose assertion had just proved the opposite; with it, the same evidence
came back as "视觉证据不足" and severity low.

If the analysis model accepts image input (its `models.json` entry lists
`image`), screenshots are attached to it directly (`future run --image`);
otherwise the `read_image` tool is the fallback.

**Two passes, on purpose.** The first judges the case's expectations. The second
is given the same evidence and **no checklist** — "you are opening this product
for the first time; say what looks wrong" — and its findings are marked
`【发现】` and counted separately in the report. A single pass that is only asked
"did these hold" can only find what someone already wrote down; the second pass
is where timing, stale states, inconsistent copy and layout surprises come from.
It is skipped on surfaces without screenshots (relay, CLI): with nothing to
look at, an open-ended prompt fills the gap by inventing visual expectations the
product never promised.

Analysis is resumable and non-fatal: a missing CLI or a failed vision call is
recorded and the remaining evidence still becomes a report.

### What the evidence cannot prove

Two classes of false finding cost more than they look: they read exactly like a
defect, and a human has to read the code to refute each one. Measured on the
2026-11-05 run (`final-v12`, 19 cases, 33 issues filed): of the twelve issues a
reviewer later closed as "not a defect", five came from data the harness wrote
itself, and three came from reading one frame as proof of a state that frame
could not show — one of them taken before the action it was read as following.

**Fixture provenance.** The UI is real; some of the values it renders are not.
`lib/provenance.mjs` builds a block for the per-case judge and the open-ended
pass naming, for *this run*, which channels are fixture data — the version and
build number, the account balance, the assistant's reply text, the simulated
send queue, the demo approval card, the device fleet, the replayed terminal —
plus the IPC commands that fell through to the in-page demo mock
(`server.fallbacks`, read from the result rather than guessed). The rule it
states is narrow on purpose: a *value* the fixture wrote is not a defect, while
*how the product draws a value* — truncation, overlap, a bare empty state, a
button that does nothing, a count that disagrees with its own rows — still is.
To keep that from being advice a model may skip, `markFixtureFindings()` demotes
findings that quote a value no real build can emit (`0.0.0-harness`), so the
issue flow cannot file them; the report still shows them as observations.
Which value belongs on that list is the whole design: a corpus check over the
mock sources was tried and removed, because it demoted a real defect whose
evidence quoted the draft the case itself typed.

**Frame provenance.** Every screenshot is labelled with the step that produced
it and, since the screencast records its own clock, with its position in the
recording (`第 12 步 · click · 录像 18.4s`). Those labels are now part of the
prompt — before, the model received the images in order with nothing saying which
step each one was, which is how "the fork still shows the original reply" was
reported about a frame taken *before* the fork. Both passes are told that a
claim of the form "it stayed / it did not change / the count is wrong" must be
visible in the frame marked 「最终状态」, and that anything seen only in a middle
frame is `info` at most.

Run the guard's tests with `node --test harness/lib/provenance.test.mjs`.

### Discovery, two levels

The first level judges one case and asks a second time with no checklist: look at
this evidence as a stranger and say what is wrong. That pass is where timing,
stale states, inconsistent copy and layout surprises come from, and it is marked
`【发现】` in the report and `discovered` in the analysis so a reader can tell
judgement from a checklist.

The second level reads only text — every case's title, expectations, verdict,
summary and findings — and looks for what a person notices after a morning of
clicking: one concept with three names, a count that disagrees between two
screens, an empty state that contradicts the rest of the product, a state one
screen explains and another does not. It must attribute every finding to the
cases it came from, and it may not infer a screen the evidence never reached.

```bash
node harness/cli.mjs analyze --run harness/runs/<id> --mode full   # both levels
```

A cross-case pass needs per-case findings to compare, so it runs after them and
only in `--mode full`. Its findings carry `source: "cross-case"` and appear in
the report under 模型验收.

### Filing issues

`issues-en` writes one English issue per problem, grouped across cases, with the
case ids, the case's own expectations, the exact reproduction command, and how it
was found (failed assertion / model verdict / model discovery) so a reader can
calibrate. The model writes the prose; the structural parts are appended from the
run, and a draft whose cases no longer exist in the library is dropped instead of
being filed as unverifiable. Findings that quote a value the fixture wrote are
filtered before drafting and the count is reported, so a run cannot file its own
test data as a product defect.

```bash
node harness/cli.mjs issues-en --run harness/runs/<id>                    # drafts only
node harness/cli.mjs issues-en --run harness/runs/<id> --create --yes --repo owner/name
```

### Performance judgement

Perf cases carry numbers, not just pass/fail. `perfDigest` hands the model the
summary, each measured series (values, mean, drift, slope) and how close every
assertion came to its limit, with instructions to reason about **trend and
magnitude** — a repeated interaction whose cost climbs per repetition means
something is retained, a monotonic heap means a leak, an operation in the
seconds range is out of class for local work. The model is also told that noise
in small values is not a defect, so it argues about the shape rather than
reacting to a single spike.

Demonstrated: from the numbers alone it flagged the boot skeleton as long, and
correctly read a 20-repeat switch series (drift 0.97, slope 0.02 ms/step) as
"no accumulation" — the leak class it was asked to look for was absent, and it
said so.

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
  A case that sends and then reads the conversation back can set
  `"query": {"transcript": "page"}` to read the page's own transcript instead,
  which is the only place the message it just wrote exists.
- **Store queries are reimplemented.** A bug in `host/src/store/*.rs` cannot be
  found here; the harness proves UI behaviour against real-shaped data.
- **`agent_prompt` is still mocked.** Sending does not call a model, so reply
  content is fixed demo prose; what it does model is the *lifecycle* — a reply
  that streams, a stop that ends it, and (with `?slowStream=`) one long enough to
  write a second message during, which the fixture then holds in the
  conversation's own queue (`session_queue`) and runs at the turn boundary.
- **Mobile and remote use the shot mocks.** They exercise the real mobile UI and
  both layouts, but not a live NATS link; a two-host relay harness is the next
  step.
- **TUI uses `demo_agent`.** It verifies the terminal UI's behaviour, not a real
  model answer; pointing it at a real agent is a flag away once an agent home
  fixture exists.
- **CLI cases are offline.** They cover commands that read `app.db`/settings/
  tasks without an agent; agent-backed commands need a fixture agent.
- **Video is analyzed as a filmstrip, not as motion.** A 12-frame strip catches
  obvious state changes; it is not a temporal model.

- **The desktop send queue is simulated by the page, not by an agent.**
  `session_queue` answers from the shot mock's own per-conversation queue, and a
  prompt that arrives mid-run is held and answered with the agent's `run_queued`
  ack. That is enough to observe the queue row and its drain
  (`send-queue-while-streaming`), but it is the mock's reading of the protocol,
  not a real session's queue: coalescing several held messages into one turn, a
  parked row and "start it now" are still unexercised here.

- **The embedded terminal is a stand-in, not a shell.** `desktop/shot/terminal-server.mjs`
  replays one transcript and answers a few fixed commands (`seq 1 N` prints N
  lines, which is how a case overflows the panel); typed input is otherwise
  echoed. It is enough to prove scrolling and layout, and says nothing about the
  real PTY on the Rust side.
