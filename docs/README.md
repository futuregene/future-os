# FutureOS Documentation

> Navigation index for the `docs/` directory. Links resolve within this repo;
> the user-facing wiki lives under [docs/wiki/](wiki/en/Home.md) (en) and
> [docs/wiki/zh/Home.md](wiki/zh/Home.md) (zh).
>
> Every document under `docs/` ships in two languages (`name.md` = en,
> `name.zh-CN.md` = zh); `scripts/docs/check-docs.py` enforces directory placement,
> bilingual pairing, links, script-path references and fences. Historical
> archives under `archives/` keep their original dates and commit boundaries.

## Guides (`guide/`)

| Doc | What it covers |
|---|---|
| [Build & Install](guide/build-and-install.md) ([中文](guide/build-and-install.zh-CN.md)) | Prerequisites, per-platform toolchains (macOS / Linux / Windows), `make` targets, GUI packaging, `future-loop` install, skills install |
| [TUI](guide/tui.md) ([中文](guide/tui.zh-CN.md)) | The terminal UI (`future-tui`): slash commands, keyboard shortcuts, settings |
| [Channels configuration](guide/channels-config.md) ([中文](guide/channels-config.zh-CN.md)) | Unified reference for `~/.future/channels/config.json` (agent / providers / Feishu / DingTalk blocks, defaults) |
| [Channel providers](guide/channels-providers.md) ([中文](guide/channels-providers.zh-CN.md)) | Shared-bridge channels: capability matrix, per-channel config, maturity, diagnostics, proactive send |
| [Channel provider contract](guide/channels-provider-contract.md) ([中文](guide/channels-provider-contract.zh-CN.md)) | Adding a channel: the provider traits, the inbound/outbound boundary, config, errors, tests and coverage |
| [Headless Desktop](guide/desktop-headless.md) ([中文](guide/desktop-headless.zh-CN.md)) | Foreground `futureos-headless` startup, terminal login/pairing QR codes and links, Ctrl+C, GUI-free server builds and troubleshooting |
| [Directory layout](guide/directory-layout.md) ([中文](guide/directory-layout.zh-CN.md)) | What lives where under `~/.future/` (agent, channels, TUI, GUI, loop) |
| [Session history recall](guide/session-history.md) ([中文](guide/session-history.zh-CN.md)) | Read-only history search/entry reads, byte paging, and post-compaction model guidance |
| [Mobile latency diagnosis](guide/mobile-latency-diagnosis.md) ([中文](guide/mobile-latency-diagnosis.zh-CN.md)) | Mobile end-to-end latency measurement and diagnosis |
| [Screenshot harness](guide/screenshots.md) ([中文](guide/screenshots.zh-CN.md)) | Rendering the real desktop/mobile UI against demo data to produce screenshots, feature diagrams and illustrated documents without a display |

The repo-root [README](../README.md) ([中文](../README.zh-CN.md)) is the
entry point; the [wiki](wiki/en/Home.md) is the user-facing app guide.

## Architecture (`architecture/`)

| Doc | What it covers |
|---|---|
| [Loop Control Plane](architecture/loop-control-plane.md) ([中文](architecture/loop-control-plane.zh-CN.md)) | `future-loop` — goals/todos/gates/monitors, should-run kernel, quota, event sourcing, delivery closure, multi-agent, supervisor/worker messaging, web dashboard |
| [Long-Run Evidence Ledger](architecture/long-run-evidence-ledger.md) ([中文](architecture/long-run-evidence-ledger.zh-CN.md)) | Accountability record for long-range loop goals — wall clock, spend, validation results, explicit boundaries per closed goal |
| [SQLite migration](architecture/sqlite-migration.md) ([中文](architecture/sqlite-migration.zh-CN.md)) | Agent/desktop SQLite storage layout and migration policy |
| [Response outcomes](architecture/response-outcomes.md) ([中文](architecture/response-outcomes.zh-CN.md)) | End-of-response outcome semantics (stop / refusal / filter / pause) |
| [loop/](architecture/loop/README.md) | The `future-loop` crate: [architecture](architecture/loop/ARCHITECTURE.md) (en/zh), [upstream attribution](../orchestration/loop/UPSTREAM.md), [decision-kernel snapshot](architecture/loop/snapshots.md) |
| [Shared packages](architecture/packages.md) ([中文](architecture/packages.zh-CN.md)) | `packages/` conventions: npm/workspace packages and crate boundaries |
| [Channel test coverage](architecture/channels-test-coverage.md) ([中文](architecture/channels-test-coverage.zh-CN.md)) | What the channel framework's tests promise, and the 22 lines that are deliberately not covered, with the reason for each |
| [RPC crate](architecture/rpc.md) ([中文](architecture/rpc.zh-CN.md)) | `packages/rpc` — the protobuf wire contract (single source of truth) |

## Internals (`internals/`)

Per-module working docs, previously scattered under `desktop/DEV_MD/`,
`desktop/nats/`, `desktop/src-tauri/windows/`, `mobile/`, `tui/tests/` and
`tests/`.

- [desktop/](internals/desktop/PRODUCT.md) — product semantics, data model (`ER.md`), colors, sandbox (macOS/Windows/Linux), connection & remote, embedded terminal, compaction (formerly `desktop/DEV_MD/`; see `desktop/CLAUDE.md` for the document map)
- [compaction/](internals/compaction/compaction.md) — runtime context compaction: the two strategies and their parameters, the production request shape, the developer map (`compaction-development.md`), retrieval interfaces, and the two experiments — closed book (`compaction-closed-book-experiment.md`) and open book (`compaction-open-book-experiment.md`)
- [mobile/](internals/mobile/README.md) — mobile build/TestFlight, iOS platform parity, streaming-sync design and measurements, HarmonyOS compatibility (formerly `mobile/README.md` + `mobile/docs/`; the dated audits and measurement reports are in `archives/verification/`)
- [tui/](internals/tui/tests.md) — TUI test harness conventions (formerly `tui/tests/README.md`)
- [skill_reco/](internals/skill_reco/harness.md) — skill recommendation: the Jev recommender's [evaluation](internals/skill_reco/evaluation.md), its [harness](internals/skill_reco/harness.md) and the [integration plan](internals/skill_reco/integration-plan.md) (a historical design record)
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
  (agent / apps / cli / loop-tui), incl. model-output provenance JSONs
- [verification/](archives/verification/errors-outdated-missing.md) — dated
  doc↔source verification snapshots: fact inventory, error/outdated/missing
  lists, [doc↔code mismatch audit](archives/verification/doc-code-mismatches.md),
  sandbox/E2EE/latency audits, and mobile snapshots: the
  [Markdown rendering audit](archives/verification/markdown-rendering-audit.md),
  [browser sync measurement](archives/verification/streaming-sync-browser-measurement.md),
  [cached-reopen measurement](archives/verification/streaming-sync-warm-measurement.md),
  plus performance/issue reports and the de-identified measurement JSONs

## Maintainers (`maintainers/`)

- [wiki-prompt.md](maintainers/wiki-prompt.md) ([中文](maintainers/wiki-prompt.zh-CN.md)) —
  generation prompt for (re)creating the wiki pages; defines scope, style and
  page inventory.

## Audits (`audits/`)

Reserved for in-progress doc↔code audit reports; completed dated reports are
archived under `archives/verification/` (e.g. the 2026-09-16
[doc↔code mismatch audit](archives/verification/doc-code-mismatches.md)).

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
  suffix). The only exception is the `BILINGUAL_PENDING` debt list, which must be
  empty. Markdown outside `docs/` is allowed only for the paths in `WHITELIST`,
  and `EXTRA_PAIR_SCOPED` requires a pair for a few of them
  (`SECURITY*`, `THIRD_PARTY_NOTICES*`, `orchestration/loop/UPSTREAM*`).
- `make test-docs-check` runs the gate's own regression tests, including
  negative controls (they fail if the checker stops detecting violations).
- Nothing runs the checker automatically: it is not wired into CI or
  `make lint`. Add it to a pipeline only as a deliberate decision.
