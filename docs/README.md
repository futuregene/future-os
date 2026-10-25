# FutureOS Documentation

> Navigation index for the `docs/` directory. Links resolve within this repo;
> the user-facing wiki lives under [docs/wiki/](wiki/en/Home.md) (en) and
> [docs/wiki/zh/Home.md](wiki/zh/Home.md) (zh).
>
> Every user-facing document under `docs/` ships in two languages (`name.md` = en,
> `name.zh-CN.md` = zh); the agent task records under `testing/` are the
> deliberate English-only exception. `scripts/docs/check-docs.py` enforces
> directory placement, bilingual pairing, links, script-path references and
> fences. Historical archives under `archives/` keep their original dates and
> commit boundaries.

## Guides (`guide/`)

| Doc | What it covers |
|---|---|
| [Build & Install](guide/build-and-install.md) ([中文](guide/build-and-install.zh-CN.md)) | Prerequisites, per-platform toolchains (macOS / Linux / Windows), `make` targets, GUI packaging, `future-goal` install, skills install |
| [Continuous integration](guide/ci.md) ([中文](guide/ci.zh-CN.md)) | Affected-project selection, dependency graph, shared runners, merge gate and manual specialty suites |
| [TUI](guide/tui.md) ([中文](guide/tui.zh-CN.md)) | The terminal UI (`future tui`): the full-screen layout, keys and mouse, the commands the palette runs, and what is wired to the agent |
| [Channels configuration](guide/channels-config.md) ([中文](guide/channels-config.zh-CN.md)) | Unified reference for `~/.future/channels/config.json` (agent / providers / Feishu / DingTalk blocks, defaults) |
| [Channel providers](guide/channels-providers.md) ([中文](guide/channels-providers.zh-CN.md)) | Shared-bridge channels: capability matrix, per-channel config, maturity, diagnostics, proactive send |
| [Channel provider contract](guide/channels-provider-contract.md) ([中文](guide/channels-provider-contract.zh-CN.md)) | Adding a channel: the provider traits, the inbound/outbound boundary, config, errors, tests and coverage |
| [Desktop Agent](guide/desktop-agent.md) ([zh](guide/desktop-agent.zh-CN.md)) | Embedded/external hosting, exit scope, CLI clients, development launchers, logs and update recovery |
| [Host mode](guide/desktop-host.md) ([中文](guide/desktop-host.zh-CN.md)) | Foreground `future host` startup, platform login QR, pairing from `future remote devices add`, Ctrl+C, GUI-free server builds and troubleshooting |
| [Self-hosted relay](guide/self-hosted-relay.md) ([中文](guide/self-hosted-relay.zh-CN.md)) | `future relay`: an account-free NATS relay for remote control, its threat model, coexistence with the official relay, and operator notes |
| [Directory layout](guide/directory-layout.md) ([中文](guide/directory-layout.zh-CN.md)) | What lives where under `~/.future/` ((agent, channels, TUI, GUI, goal)) |
| [Data backup and recovery](guide/data-backup.md) ([中文](guide/data-backup.zh-CN.md)) | `future data`: offline backup, verify, restore to the original home, resume/rollback after an interrupted restore, and the startup fence |
| [Session history recall](guide/session-history.md) ([中文](guide/session-history.zh-CN.md)) | Read-only history search/entry reads, byte paging, and post-compaction model guidance |
| [Self-inspection](guide/self-inspection.md) ([中文](guide/self-inspection.zh-CN.md)) | Reading an agent's own state (settings, skills, tools, sessions) and the user–agent record, and changing global settings from the CLI |
| [Mobile latency diagnosis](guide/mobile-latency-diagnosis.md) ([中文](guide/mobile-latency-diagnosis.zh-CN.md)) | Mobile end-to-end latency measurement and diagnosis |
| [Screenshot harness](guide/screenshots.md) ([中文](guide/screenshots.zh-CN.md)) | Rendering the real desktop/mobile UI against demo data to produce screenshots, feature diagrams and illustrated documents without a display |

The repo-root [README](../README.md) ([中文](../README.zh-CN.md)) is the
entry point; the [wiki](wiki/en/Home.md) is the user-facing app guide.

## Architecture (`architecture/`)

| Doc | What it covers |
|---|---|
| [Goal Control Plane](architecture/goal-control-plane.md) ([中文](architecture/goal-control-plane.zh-CN.md)) | `future-goal` — goals/todos/gates/monitors, should-run kernel, quota, event sourcing, delivery closure, multi-agent, supervisor/worker messaging, web dashboard |
| [Long-Run Evidence Ledger](architecture/long-run-evidence-ledger.md) ([中文](architecture/long-run-evidence-ledger.zh-CN.md)) | Accountability record for long-range goal-plane goals — wall clock, spend, validation results, explicit boundaries per closed goal |
| [Scheduled tasks](architecture/tasks.md) ([中文](architecture/tasks.zh-CN.md)) | `future-tasks` — reusable prompt + trigger (schedule or on demand), full-permission runs, deterministic `next_due` kernel, run ledger and prompt versions |
| [SQLite migration](architecture/sqlite-migration.md) ([中文](architecture/sqlite-migration.zh-CN.md)) | Agent/desktop SQLite storage layout and migration policy |
| [Desktop embedded Agent development plan](architecture/embedded-agent-development.md) ([中文](architecture/embedded-agent-development.zh-CN.md)) | Complete in-process Agent, service/host lifecycle, locks, shutdown, updates/installers, development tooling and phased acceptance |
| [Automatic update transactions](architecture/automatic-updates.md) ([中文](architecture/automatic-updates.zh-CN.md)) | Shared state machine, prepared platform executors, crash recovery, startup confirmation and CI acceptance |
| [Agent startup arbitration](architecture/agent-supervision.md) ([中文](architecture/agent-supervision.zh-CN.md)) | Who starts the Agent in one data root: instance-lock arbitration between `future agent` and `future host`, yielding instead of failing, bounded waits, a crash-loop breaker, and why the TUI only attaches (D11) |
| [Response outcomes](architecture/response-outcomes.md) ([中文](architecture/response-outcomes.zh-CN.md)) | End-of-response outcome semantics (stop / refusal / filter / pause) |
| [goal/](architecture/goal/README.md) | The `future-goal` crate: [architecture](architecture/goal/ARCHITECTURE.md) (en/zh), [upstream attribution](../orchestration/goal/UPSTREAM.md), [decision-kernel snapshot](architecture/goal/snapshots.md) |
| [Shared packages](architecture/packages.md) ([中文](architecture/packages.zh-CN.md)) | `packages/` conventions: npm/workspace packages and crate boundaries |
| [Remote package development plan](architecture/remote-package-development.md) ([中文](architecture/remote-package-development.zh-CN.md)) | Planned Rust/TypeScript protocol, client and server extraction; independent Crypto, host adapters, migration phases and CI acceptance |
| [Headless package development plan](architecture/headless-package-development.md) ([中文](architecture/headless-package-development.zh-CN.md)) | Implemented `future host` entry, GUI-free `future-host` library, shared backend, data compatibility, test ownership and build/release migration |
| [Headless release acceptance](testing/headless-release-acceptance.md) ([中文](testing/headless-release-acceptance.zh-CN.md)) | Final successful CI evidence and deferred packaging, signing, installation, performance and same-candidate acceptance |
| [Headless unsigned downloads/results](testing/headless-unsigned-results.md) ([中文](testing/headless-unsigned-results.zh-CN.md)) | Verified dev payloads, public URLs, SHA-256, CLI/sidecar and static/old-Linux checks |
| [Channel test coverage](architecture/channels-test-coverage.md) ([中文](architecture/channels-test-coverage.zh-CN.md)) | What the channel framework's tests promise, and the handful of lines that are deliberately not covered, with the reason for each |
| [RPC crate](architecture/rpc.md) ([中文](architecture/rpc.zh-CN.md)) | `packages/rpc` — the protobuf wire contract (single source of truth) |

## Internals (`internals/`)

Per-module working docs for the desktop app and the mobile client: product
semantics, data models, platform sandboxes, connection and remote behaviour,
context compaction, the mobile build and its platform parity notes, and the TUI
test harness conventions.

- [desktop/](internals/desktop/PRODUCT.md) — product semantics, data model (`ER.md`), colors, sandbox (macOS/Windows/Linux), connection & remote, embedded terminal, compaction (see `desktop/CLAUDE.md` for the document map)
- [compaction/](internals/compaction/compaction.md) — runtime context compaction: the two strategies and their parameters, the production request shape, the developer map (`compaction-development.md`), retrieval interfaces, and the two experiments — closed book (`compaction-closed-book-experiment.md`) and open book (`compaction-open-book-experiment.md`)
- [mobile/](internals/mobile/README.md) — mobile build/TestFlight, iOS platform parity, streaming-sync design and measurements, HarmonyOS compatibility (the dated audits and measurement reports are in `archives/verification/`)
- [skill_reco/](internals/skill_reco/harness.md) — skill recommendation: the Jev recommender's [evaluation](internals/skill_reco/evaluation.md), its [harness](internals/skill_reco/harness.md) and the [integration plan](internals/skill_reco/integration-plan.md) (a historical design record)
- [agent/](internals/agent/README.md) — module map of the gRPC backend (`agent/`): the turn loop, tools, LLM layer, sessions, sandbox, and what to read before changing each one
- [host](internals/host.md) — module map of `future-host`, the GUI-free shared backend (store, agent bridge, tasks, remote, identity)
- [Desktop NATS bridge](internals/desktop-nats.md)
- [Windows installer/update](internals/desktop-windows.md)
- [Provider protocol tests](internals/provider-protocol.md)

## TUI design (`tui/`)

| Doc | What it covers |
|---|---|
| [Workbench TUI spec](tui/workbench-spec.md) ([中文](tui/workbench-spec.zh-CN.md)) | The frozen contract behind the [terminal UI](guide/tui.md): layout constants, mouse and keyboard contract, type names and acceptance commands. The surface it describes is built; the agent adapter and P1 items are not |
| [Interface text that never reached the table](tui/i18n-homeless-strings.md) ([中文](tui/i18n-homeless-strings.zh-CN.md)) | The guard that fails a build when the workbench draws chrome it did not take from `workbench/i18n.rs`: what it reads, what it cannot see, the English-only text it froze on the way in, and the port list for the branches that left before the table |

## Wiki (user-facing app guide)

- English: [Home](wiki/en/Home.md), [Installation](wiki/en/Installation.md),
  [Quick Start](wiki/en/Quick-Start.md), [Using FutureOS](wiki/en/Using-FutureOS.md),
  [Settings](wiki/en/Settings.md), [Sandbox](wiki/en/Sandbox.md), [Remote](wiki/en/Remote.md), [Skills](wiki/en/Skills.md),
  [Scheduled tasks](wiki/en/Scheduled-Tasks.md),
  [CLI](wiki/en/CLI.md), [FAQ](wiki/en/FAQ.md),
  [Feishu](wiki/en/Feishu.md), [DingTalk](wiki/en/DingTalk.md),
  [Models](wiki/en/Models.md) *(auto-generated — do not edit by hand)*
- 中文: [首页](wiki/zh/Home.md), [安装](wiki/zh/Installation.md),
  [快速开始](wiki/zh/Quick-Start.md), [使用 FutureOS](wiki/zh/Using-FutureOS.md),
  [设置](wiki/zh/Settings.md), [审批与沙箱](wiki/zh/Sandbox.md), [手机远程](wiki/zh/Remote.md), [技能](wiki/zh/Skills.md),
  [命令行工具](wiki/zh/CLI.md), [FAQ](wiki/zh/FAQ.md),
  [飞书](wiki/zh/Feishu.md), [钉钉](wiki/zh/DingTalk.md),
  [模型目录](wiki/zh/Models.md) *(自动生成)*

Both language trees also carry the `_Sidebar.md` / `_Footer.md` navigation pages.

## Packaging readmes (`dist/`)

These files are copied verbatim into the release packages as `Readme.txt`: the
unsigned macOS test DMG, the Windows portable zip and the Linux portable
tarball. They are **live artifacts** — edit them only together with the
packaging pipelines, and note that the release workflows and the `scripts/build/`
scripts reference this exact path, so the directory cannot be renamed without
updating `.github/workflows/build-{macos-signed,windows-signed,linux}.y*ml`.

- [readme-macos.txt](dist/readme-macos.txt) / [en](dist/readme-macos-en.txt)
- [readme-windows.txt](dist/readme-windows.txt) / [en](dist/readme-windows-en.txt)
- [readme-linux.txt](dist/readme-linux.txt) / [en](dist/readme-linux-en.txt)

## Archives (`archives/`)

Historical audit snapshots; their conclusions apply only to the recorded
dates/commits, not current source. Kept for provenance — do not rewrite them
to match today's code.

- [bughunt/](archives/bughunt/README.md) — bug-hunt evidence and fix records
  (agent / apps / cli / goal-tui), incl. model-output provenance JSONs
- [verification/](archives/verification/errors-outdated-missing.md) — dated
  doc↔source verification snapshots: fact inventory, error/outdated/missing
  lists, [doc↔code mismatch audit](archives/verification/doc-code-mismatches.md),
  sandbox/E2EE/latency audits, and mobile snapshots: the
  [Markdown rendering audit](archives/verification/markdown-rendering-audit.md),
  [browser sync measurement](archives/verification/streaming-sync-browser-measurement.md),
  [cached-reopen measurement](archives/verification/streaming-sync-warm-measurement.md),
  plus performance/issue reports and the de-identified measurement JSONs
- [testing/](archives/testing/) — superseded coverage-campaign task-record
  snapshots, moved out of `testing/` whole and English-only under the same
  non-user-document exception

## Maintainers (`maintainers/`)

- [wiki-prompt.md](maintainers/wiki-prompt.md) ([中文](maintainers/wiki-prompt.zh-CN.md)) —
  generation prompt for (re)creating the wiki pages; defines scope, style and
  page inventory.

## Audits (`audits/`)

Reserved for in-progress doc↔code audit reports; completed dated reports are
archived under `archives/verification/` (e.g. the 2026-09-16
[doc↔code mismatch audit](archives/verification/doc-code-mismatches.md)).

## Testing records (`testing/`)

- [testing/](testing/README.md) ([中文](testing/README.zh-CN.md)) — the
  per-module coverage campaign's task records: run identity, measurement
  command, uncovered lines, dimension evidence, waivers and open items.
  Agent-facing working notes, not user documentation, and English-only under
  the deliberate `PAIR_EXEMPT` exception in `scripts/docs/check-docs.py`;
  superseded snapshots are kept in `archives/testing/`.

## How the docs stay correct

- `docs/wiki/{en,zh}/Models.md` are generated by
  `make generate-models` (scripts/docs/generate_models.py) — never hand-edit.
- The wiki pages are authored to the scope in
  [wiki-prompt.md](maintainers/wiki-prompt.md): macOS/Windows/Linux desktop, Android/iOS
  Remote, platform-specific sandboxing, no separate TUI page, CLI named `future`.
  Transport details belong in troubleshooting/CLI and the repository guides.
- Verify changed claims against current source and update both languages.
  Historical verification notes retain original evidence, not a permanent PASS.
- [Documentation check](../scripts/docs/check-docs.py): `make check-docs` (or
  `python3 scripts/docs/check-docs.py`) enforces placement, bilingual pairing, local
  links, wiki targets, fences and that every repo-internal `scripts/` path named
  in a current doc exists (Windows separators are normalised; a documented glob
  must match at least one file; `docs/archives/**` is exempt as frozen history).
  Two modes:
  - default — findings are errors; entries in `BILINGUAL_PENDING` are allowed
    and reported as a count, so a mid-migration tree can still be checked.
  - `--strict-pending` — final-acceptance mode: the debt list must be empty, so
    every document really has both languages.
  - `--scope docs/guide,docs/architecture` — report only findings about those
    paths, for working on one slice at a time. Findings are filtered by the
    file they are about, not by their wording.
- Every `.md` under `docs/` needs both languages (any new `docs/` subdirectory
  inherits this; `docs/wiki/` pairs by `en/`+`zh/` and `docs/dist/` by the `-en`
  suffix). `docs/testing/` and `docs/archives/testing/` are exempt as
  non-user documents (the coverage campaign's agent task records), and the
  `BILINGUAL_PENDING` debt list must be empty. Markdown outside `docs/` is
  allowed only for the paths in `WHITELIST`, and `EXTRA_PAIR_SCOPED` requires
  a pair for a few of them
  (`SECURITY*`, `THIRD_PARTY_NOTICES*`, `orchestration/goal/UPSTREAM*`).
- `make test-docs-check` runs the gate's own regression tests, including
  negative controls (they fail if the checker stops detecting violations).
- Nothing runs the checker automatically: it is not wired into CI or
  `make lint`. Add it to a pipeline only as a deliberate decision.
