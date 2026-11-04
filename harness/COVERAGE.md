# Coverage matrix

What the library actually exercises today, and what it does not. This is the
honest answer to "is it comprehensive?": the list is representative, not
exhaustive, and every gap below is a candidate for the next iteration.

Legend: ✅ two or more cases · ◐ one smoke case · — not covered.

## Desktop

| Area | Coverage | Cases |
|---|---|---|
| Conversation list, recent ordering, active row | ✅ | `open-conversation-list`, `rapid-thread-switching` |
| Paged list (`再显示 N 个`) | ✅ | `paged-conversation-list` |
| Search, filtered results, open result | ✅ | `search-conversation`, `search-result-open` |
| Create chat, send, model selection | ✅ | `create-chat-and-delete`, `switch-model-before-send`, `thinking-level-high-persists` |
| Delete current / non-current / cancel | ✅ | `delete-open-conversation`, `create-chat-and-delete`, `cancel-delete-conversation` |
| Rename, pin/unpin, archive/restore | ✅ | `rename-conversation`, `pin-and-unpin-conversation`, `pin-then-archive`, `archive-and-review`, `archive-open-conversation` |
| Workspace create / chat inside / archive | ✅ | `create-workspace`, `workspace-chat`, `archive-workspace-with-open-chat` |
| Workspace rename / pin / delete / restore | — | next |
| Batch select, batch archive/delete | — | next |
| Attachments (file/image), paste, remove | — | next |
| Skill `/` and mention `@`/`#` menus | ✅ | `composer-slash-menu`, `composer-mention-menu` |
| Skill recommendation, guide banner | — | next |
| Context panel (files/run/review), narrow window | ✅ | `context-panel-tabs`, `narrow-context-panel` |
| Terminal panel | ✅ | `terminal-panel` |
| Fold/unfold tool calls, fork, copy, question nav | — | next |
| Approval card approve/reject | — | next |
| Settings pages (models/skills/tasks/remote) | ✅ smoke | `side-panels-smoke`, `settings-page`, `remote-page` |
| Settings tabs (general/models/skills/tasks/remote) | ✅ | `settings-tabs`, `settings-page`, `remote-page` |
| Language / theme persistence across reload | ✅ | `settings-language`, `settings-theme` |
| Approval tier persistence | — | next |
| Send queue while a reply runs | — | blocked: shot mock has no agent queue |
| Fusion discussion | — | next |
| Reload persistence | ✅ | `reload-persistence`, `thinking-level-high-persists`, `model-persists-after-reload` |
| Empty send, empty states | ✅ | `empty-send-noop` |

## Mobile

| Area | Coverage | Cases |
|---|---|---|
| Home recent / active / workspace / chat tabs | ✅ | `mobile-home-list`, `mobile-workspace-tab`, `mobile-chat-tab` |
| Open conversation, transcript, composer | ✅ | `mobile-open-conversation`, `mobile-compose-draft` |
| Search | ✅ | `mobile-search` |
| Manage menu (rename/delete/cancel) | ✅ | `mobile-manage-menu` |
| Settings, archived view | ✅ | `mobile-settings`, `mobile-archived-view` |
| Device picker / fleet, task reminders, share intent | — | next |
| Mobile send queue, attachment download | — | next |

## TUI

| Area | Coverage | Cases |
|---|---|---|
| Startup, chat, composer | ✅ | `tui-open-chat` |
| Help, palette | ✅ | `tui-help`, `tui-palette` |
| Submit echo, queue while running | ✅ | `tui-submit-echo`, `tui-queue-during-run` |
| Sidebar panels (sessions/usage/skills/todo), preferences | — | next |
| Theme/language, launchpad, workspace picker, stop, fold, search, git/files panels | — | next |

## CLI

| Area | Coverage | Cases |
|---|---|---|
| Version/help, unknown command | ✅ | `cli-version-and-help`, `cli-unknown-command` |
| Config get/set roundtrip | ✅ | `cli-config-get`, `cli-config-set-roundtrip` |
| Workspace list/add/archive/restore, duplicates, missing | ✅ | 4 cases |
| Skills list, task list, MCP add/list/remove, doctor | ✅ | `cli-skills-list`, `cli-task-list`, `cli-mcp-*`, `cli-doctor` |
| Session list/status/transcript (agent-backed) | — | needs a fixture agent |
| Models list/refresh, data backup/verify | — | needs agent / backup fixture |
| Goal commands | — | next |

## Remote

| Area | Coverage | Cases |
|---|---|---|
| Desktop + mobile render the same session | ◐ | `remote-parser-pair` |
| Live NATS relay, paired host, remote prompt/stream | — | next |

## Performance

| Metric | Case | Current baseline (this machine) |
|---|---|---|
| Desktop boot to interactive | `perf-desktop-boot` | ~2.1 s |
| Thread switch (click → 2×rAF) | `perf-thread-switch` | 290–380 ms |
| 10 switches: JS heap / RSS growth | `perf-memory-switching` | ~ -6 MB / +5 MB |
| 20 repeated switches: latency drift / slope / heap | `perf-switch-scaling` | drift ~1.0 / ~0 ms per step / +3–34 MB |
| IPC p95 / max | `perf-ipc` | 2–8 ms / 7–110 ms |
| CLI version / workspace list (warm) | `perf-cli-startup` | 6–9 ms / 21–24 ms |
| TUI startup / help panel | `perf-tui-startup` | 2.3 s / 366–410 ms |
| First-screen IPC requests / bytes | `perf-desktop-boot` | 120 / 114 KB |

Thresholds are **relative to a recorded baseline** (`harness/perf/baseline.json`),
not bare absolute ceilings:

```
limit = max(baseline × (1 + tolerance), baseline + floor, floor)
        then min(limit, max)          # `max` is the catastrophe guard only
```

Measured noise decides the tolerance: the metrics above vary by 1.00–1.1×
between runs, so a case may use 15–30% and still not flake; the ones that vary
more (IPC max, RSS growth) get an additive floor or stay a pure ceiling and are
marked as such in the report. The report prints actual / limit / baseline per
assertion, so a threshold that has drifted loose is visible instead of hidden.

`node --test harness/lib/perf.test.mjs` proves both directions: observed noise
passes, a 30% regression fails, and a missing baseline degrades to the ceiling
instead of turning a fresh clone red.

`perf-switch-scaling` is the complexity probe: it repeats the same interaction
20 times and asserts on **drift** (last third ÷ first third) and **slope**
(ms per repetition), because a leak or an accidental O(n) inside a repeated
path shows up as growth over repetitions, which no max/avg can see. The series
is also handed to the vision model with the numbers, so it can argue about the
trend in prose (and call out superlinear growth) rather than only fail a bound.

## Adding coverage

1. Put the case under `cases/<surface>/`, with a `surface` field for anything
   other than desktop.
2. Prefer a real database value or a visible DOM label over a hard-coded index;
   the real `app.db` changes between runs.
3. Add the row to this file — a case the matrix does not mention is a case that
   will be forgotten.
