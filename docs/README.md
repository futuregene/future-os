# FutureOS Documentation

> Navigation index for the `docs/` directory. Links resolve within this repo;
> the user-facing wiki lives under [docs/wiki/](wiki/en/Home.md) (en) and
> [docs/wiki/zh/Home.md](wiki/zh/Home.md) (zh).
>
> Every document under `docs/` ships in two languages (`name.md` = en,
> `name.zh-CN.md` = zh); `scripts/check-docs.py` enforces directory placement,
> bilingual pairing, links and fences. Historical archives under
> `archives/` keep their original dates and commit boundaries.

## Guides (`guide/`)

| Doc | What it covers |
|---|---|
<<<<<<< HEAD
| [Build & Install](build-and-install.md) ([中文](build-and-install.zh-CN.md)) | Prerequisites, per-platform toolchains (macOS / Linux / Windows), `make` targets, GUI packaging, `future-loop` install, skills install |
| [Loop Control Plane](loop-control-plane.md) ([中文](loop-control-plane.zh-CN.md)) | `future-loop` — goals/todos/gates/monitors, should-run kernel, quota, event sourcing, delivery closure, multi-agent, supervisor/worker messaging, web dashboard |
| [Long-Run Evidence Ledger](long-run-evidence-ledger.md) ([中文](long-run-evidence-ledger.zh-CN.md)) | Accountability record for long-range loop goals — wall clock, spend, validation results, explicit boundaries per closed goal |
| [Headless Desktop](desktop-headless.md) ([中文](desktop-headless.zh-CN.md)) | Foreground `--headless` startup, terminal login/pairing QR codes and links, Ctrl+C, GUI-free server builds and troubleshooting |
| [TUI](tui.md) ([中文](tui.zh-CN.md)) | The terminal UI (`future-tui`): slash commands, keyboard shortcuts, settings |
| [Directory layout](directory-layout.md) ([中文](directory-layout.zh-CN.md)) | What lives where under `~/.future/` (agent, channels, TUI, GUI, loop) |
| [Session history recall](session-history.md) ([中文](session-history.zh-CN.md)) | Read-only history search/entry reads, byte paging, and post-compaction model guidance |
| [C compaction](compaction.md) ([中文](compaction.zh-CN.md)) | 80%/256K trigger, S2 original protection, fixed-budget deterministic evidence, zero summary calls and schema-3 restore |
| [Channels configuration](channels-config.md) ([中文](channels-config.zh-CN.md)) | Unified reference for `~/.future/channels/config.json` (agent / Feishu / DingTalk blocks, defaults) |
=======
| [Build & Install](guide/build-and-install.md) ([中文](guide/build-and-install.zh-CN.md)) | Prerequisites, per-platform toolchains (macOS / Linux / Windows), `make` targets, GUI packaging, `future-loop` install, skills install |
| [TUI](guide/tui.md) ([中文](guide/tui.zh-CN.md)) | The terminal UI (`future-tui`): slash commands, keyboard shortcuts, settings |
| [Channels configuration](guide/channels-config.md) ([中文](guide/channels-config.zh-CN.md)) | Unified reference for `~/.future/channels/config.json` (agent / Feishu / DingTalk blocks, defaults) |
| [Headless Desktop](guide/desktop-headless.md) ([中文](guide/desktop-headless.zh-CN.md)) | Foreground `--headless` startup, terminal login/pairing QR codes and links, Ctrl+C, GUI-free server builds and troubleshooting |
| [Directory layout](guide/directory-layout.md) ([中文](guide/directory-layout.zh-CN.md)) | What lives where under `~/.future/` (agent, channels, TUI, GUI, loop) |
| [Mobile latency diagnosis](guide/mobile-latency-diagnosis.md) | Mobile end-to-end latency measurement and diagnosis |
>>>>>>> origin/main

The repo-root [README](../README.md) ([中文](../README.zh-CN.md)) is the
entry point; the [wiki](wiki/en/Home.md) is the user-facing app guide.

## Architecture (`architecture/`)

| Doc | What it covers |
|---|---|
| [Loop Control Plane](architecture/loop-control-plane.md) ([中文](architecture/loop-control-plane.zh-CN.md)) | `future-loop` — goals/todos/gates/monitors, should-run kernel, quota, event sourcing, delivery closure, multi-agent, supervisor/worker messaging, web dashboard |
| [Long-Run Evidence Ledger](architecture/long-run-evidence-ledger.md) ([中文](architecture/long-run-evidence-ledger.zh-CN.md)) | Accountability record for long-range loop goals — wall clock, spend, validation results, explicit boundaries per closed goal |
| [SQLite migration](architecture/sqlite-migration.zh-CN.md) | Agent/desktop SQLite storage layout and migration policy (zh only for now) |
| [Response outcomes](architecture/response-outcomes.zh-CN.md) | End-of-response outcome semantics (stop / refusal / filter / pause) (zh only for now) |
| [loop/](architecture/loop/README.md) | The `future-loop` crate: [architecture](architecture/loop/ARCHITECTURE.md) (en/zh), [upstream attribution](../orchestration/loop/UPSTREAM.md), [decision-kernel snapshot](architecture/loop/snapshots.md) |
| [Shared packages](architecture/packages.md) | `packages/` conventions: npm/workspace packages and crate boundaries |
| [RPC crate](architecture/rpc.md) | `packages/rpc` — the protobuf wire contract (single source of truth) |

## Internals (`internals/`)

Per-module working docs, previously scattered under `desktop/DEV_MD/`,
`mobile/docs/`, `tui/`, `packages/`, `orchestration/loop/` and `tests/`.

- [desktop/](internals/desktop/PRODUCT.md) — product semantics, data model (`ER.md`), colors, sandbox (macOS/Windows/Linux), connection & remote, embedded terminal, compaction (formerly `desktop/DEV_MD/`; see `desktop/CLAUDE.md` for the document map)
- [mobile/](internals/mobile/README.md) — mobile build/TestFlight, iOS platform parity, streaming-sync performance/audits, harmonyOS compatibility (formerly `mobile/README.md` + `mobile/docs/`)
- [tui/](internals/tui/tests.md) — TUI test harness conventions (formerly `tui/tests/README.md`)
- [Desktop NATS bridge](internals/desktop-nats.md) (formerly `desktop/nats/README.md`)
- [Windows installer/update](internals/desktop-windows.md) (formerly `desktop/src-tauri/windows/README.md`)
- [Provider protocol tests](internals/provider-protocol.md) (formerly `tests/provider-protocol/README.md`)

## Wiki (user-facing app guide)

- English: [Home](wiki/en/Home.md), [Installation](wiki/en/Installation.md),
  [Quick Start](wiki/en/Quick-Start.md), [Using FutureOS](wiki/en/Using-FutureOS.md),
  [Settings](wiki/en/Settings.md), [Sandbox](wiki/en/Sandbox.md), [Remote](wiki/en/Remote.md), [Skills](wiki/en/Skills.md),
  [CLI](wiki/en/CLI.md), [FAQ](wiki/en/FAQ.md),
  [Feishu](wiki/en/Feishu.md), [DingTalk](wiki/en/DingTalk.md),
  [Models](wiki/en/Models.md) *(auto-generated — do not edit by hand)*
- 中文: [首页](wiki/zh/Home.md), [安装](wiki/zh/Installation.md),
  [快速开始](wiki/zh/Quick-Start.md), [使用 FutureOS](wiki/zh/Using-FutureOS.md),
  [设置](wiki/zh/Settings.md), [审批与沙箱](wiki/zh/Sandbox.md), [手机远程](wiki/zh/Remote.md), [技能](wiki/zh/Skills.md),
  [命令行工具](wiki/zh/CLI.md), [FAQ](wiki/zh/FAQ.md),
  [飞书](wiki/zh/Feishu.md), [钉钉](wiki/zh/DingTalk.md),
  [模型目录](wiki/zh/Models.md) *(自动生成)*

## Packaging readmes (`dist/`)

These files are copied verbatim into the release packages as `Readme.txt`
(macOS / Windows / Linux portable). They are **live artifacts** — edit them
only together with the packaging pipelines, and note that the release workflows
reference this exact path, so the directory cannot be renamed without updating
`.github/workflows/build-{macos-signed,windows-signed,linux}.y*ml`.

- [readme-macos.txt](dist/readme-macos.txt) / [en](dist/readme-macos-en.txt)
- [readme-windows.txt](dist/readme-windows.txt) / [en](dist/readme-windows-en.txt)
- [readme-linux.txt](dist/readme-linux.txt) / [en](dist/readme-linux-en.txt)

## Archives (`archives/`)

<<<<<<< HEAD
- [Compaction developer guide](compaction-development.md) ([中文](compaction-development.zh-CN.md)) — implementation map, durable state machine, user CLI and proposed safe model-requested entry point.
- [Compaction strategy comparison](compaction-abc-experiment.md) ([中文](compaction-abc-experiment.zh-CN.md)) — six strategies on identical fixtures (ours A/B/C, `origin/main`, Codex, OpenCode): accuracy, cost and limits.
- [Legacy semantic prompts](compaction-prompts.md) ([中文](compaction-prompts.zh-CN.md)) — retained explicit A APIs; default C makes no summary-model calls.
- [wiki-prompt.md](wiki-prompt.md) ([en](wiki-prompt-en.md)) — generation prompt
  for (re)creating the wiki pages; defines scope, style and page inventory.
- [verification/](verification/errors-outdated-missing.md) — doc↔source
  historical verification snapshots (fact inventory, error/outdated/missing list).
  Their conclusions apply only to the recorded dates/commits, not current source.
=======
Historical audit snapshots; their conclusions apply only to the recorded
dates/commits, not current source. Kept for provenance — do not rewrite them
to match today's code.

- [bughunt/](archives/bughunt/README.md) — bug-hunt evidence and fix records
  (agent / apps / cli / loop-tui), incl. model-output provenance JSONs
- [verification/](archives/verification/errors-outdated-missing.md) —
  doc↔source verification snapshots (fact inventory, error/outdated/missing
  lists, sandbox/e2ee/latency audits)

## Maintainers (`maintainers/`)

- [wiki-prompt.md](maintainers/wiki-prompt.md) ([中文](maintainers/wiki-prompt.zh-CN.md)) —
  generation prompt for (re)creating the wiki pages; defines scope, style and
  page inventory.

## Audits (`audits/`)

Reserved for future doc↔code audit reports.
>>>>>>> origin/main

## How the docs stay correct

- `docs/wiki/{en,zh}/Models.md` are generated by
  `make generate-models` (scripts/generate_models.py) — never hand-edit.
- The wiki pages are authored to the scope in
  [wiki-prompt.md](maintainers/wiki-prompt.md): macOS/Windows/Linux desktop, Android/iOS
  Remote, platform-specific sandboxing, no separate TUI page, CLI named `future`.
  Transport details belong in troubleshooting/CLI and the repository guides.
- Verify changed claims against current source and update both languages.
  Historical verification notes retain original evidence, not a permanent PASS.
- [Documentation check](../scripts/check-docs.py): `make check-docs` (or
  `python3 scripts/check-docs.py`) enforces placement, bilingual pairing, local
  links, wiki targets and fences. Two modes:
  - default — findings are errors; entries in `BILINGUAL_PENDING` are allowed
    and reported as a count, so a mid-migration tree can still be checked.
  - `--strict-pending` — final-acceptance mode: the debt list must be empty, so
    every document really has both languages.
  - `--scope docs/guide,docs/architecture` — report only findings about those
    paths, for working on one slice at a time. Findings are filtered by the
    file they are about, not by their wording.
- Every `.md` under `docs/` needs both languages (any new `docs/` subdirectory
  inherits this; `docs/wiki/` pairs by `en/`+`zh/` and `docs/dist/` by the `-en`
  suffix). Exceptions are the files listed in `WHITELIST`/`EXTRA_PAIR_SCOPED`.
- `make test-docs-check` runs the gate's own regression tests, including
  negative controls (they fail if the checker stops detecting violations).
- Nothing runs the checker automatically: it is not wired into CI or
  `make lint`. Add it to a pipeline only as a deliberate decision.
