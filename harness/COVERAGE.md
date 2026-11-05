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
| Workspace rename / pin / delete / restore | ✅ | `workspace-rename`, `workspace-pin-unpin`, `workspace-delete` |
| Batch select, batch archive/delete | ✅ | `batch-select-delete`, `batch-select-all-cancel` |
| Attachments (file/image), paste, remove | ◐ | `attachment-add-remove` |
| Skill `/` and mention `@`/`#` menus | ✅ | `composer-slash-menu`, `composer-mention-menu` |
| Skill recommendation, guide banner | ✅ | `skill-recommend-card`, `skill-guide-banner` |
| Context panel (files/run/review), narrow window | ✅ | `context-panel-tabs`, `narrow-context-panel` |
| Context panel loading state converges | ✅ | `context-panel-finishes-loading` |
| Terminal panel | ✅ | `terminal-panel`, `terminal-panel-bottom` |
| 终端溢出到屏外：滚到底能回到最后一行 | ◐ | `terminal-panel-bottom` |
| Fold/unfold tool calls, fork, copy | ✅ | `tool-call-fold-expand`, `fork-conversation`, `copy-message-feedback` |
| Approval card approve/reject | ✅ | `approval-card-approve`, `approval-card-reject` |
| Settings pages (models/skills/tasks/remote) | ✅ smoke | `side-panels-smoke`, `settings-page`, `remote-page` |
| 定时任务：列表、下次运行、运行记录、失败原因 | ◐ | `tasks-panel-detail` |
| 定时任务：新建（含与已有任务同名） | ◐ | `tasks-create-same-name` |
| 定时任务：别的客户端改了任务，面板收到 `tasks-updated` 就重读 | ◐ | `tasks-panel-refresh` |
| 远程桌面：设备列表、控制/待命、解除一台 | ◐ | `remote-device-list` |
| 远程桌面：添加设备的邀请不占死名额 | ◐ | `remote-add-device-repeat` |
| Settings tabs (general/models/skills/tasks/remote) | ✅ | `settings-tabs`, `settings-page`, `remote-page` |
| Language / theme persistence across reload | ✅ | `settings-language`, `settings-theme` |
| Approval tier persistence | ✅ | `approval-tier-persists` |
| Send queue while a reply runs | ✅ | `send-queue-while-streaming` |
| Fusion discussion | — | blocked: the fixture's fusion run is keyed to `th_review`, but the runs panel reads runs by session id and renders none, so the entry point cannot be reached from the UI; needs a fixture fix first |
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
| Device picker / fleet | ✅ | `mobile-device-picker` |
| Standby: a machine another device holds | ✅ | `mobile-standby-takeover` |
| Pairing endings: revoked vs merely dropped | ✅ | `mobile-pairing-revoked`, `mobile-connection-dropped-escape` |
| Scheduled tasks: list, detail, `编辑` entry | ✅ | `mobile-task-list-and-edit` |
| Scheduled tasks: form, time picker, save write-through | ✅ | `mobile-task-time-picker` |
| Scheduled tasks: draft across a cold start, cancel clears it | ✅ | `mobile-task-draft-survives-restart` |
| Scheduled tasks: enable/disable reaches the machine | ◐ | `mobile-task-toggle-reaches-machine` |
| Chat panels (files, token usage) | ✅ | `mobile-chat-panels` |
| Share intent intake | ✅ | `mobile-share-intake` |
| Task reminders | — | next: the reminder is scheduled by the OS; the Expo build has no scheduler to observe |
| Task dependencies / run detail paging | — | next (the model keeps no deps; runs have one page) |
| Mobile send queue, attachment download | — | next |

## TUI

| Area | Coverage | Cases |
|---|---|---|
| Startup, chat, composer | ✅ | `tui-open-chat` |
| Help, palette | ✅ | `tui-help`, `tui-palette` |
| Submit echo, queue while running | ✅ | `tui-submit-echo`, `tui-queue-during-run` |
| Sidebar panels (sessions/workspaces), preferences, tool fold | ✅ | `tui-sidebar-panels`, `tui-preferences`, `tui-tool-fold` |
| Launchpad / workspace picker · stop · git & files panels · theme & language | ✅ | `tui-launchpad-workspace-picker`, `tui-stop-generation`, `tui-git-files-panels`, `tui-preferences` |

## CLI

| Area | Coverage | Cases |
|---|---|---|
| Version/help, unknown command | ✅ | `cli-version-and-help`, `cli-unknown-command` |
| Config get/set roundtrip | ✅ | `cli-config-get`, `cli-config-set-roundtrip` |
| Workspace list/add/archive/restore, duplicates, missing | ✅ | 4 cases |
| Skills list, task list, MCP add/list/remove, doctor | ✅ | `cli-skills-list`, `cli-task-list`, `cli-mcp-*`, `cli-doctor` |
| 定时任务存储：间隔任务的下次槽位、排队运行、同名拒绝 | ✅ | `cli-task-interval-next-slot`, `cli-task-run-twice`, `cli-task-add-same-name` |
| Session list/status/transcript (agent-backed) | ✅ | `cli-session-list-status-transcript` (scripted `demo_agent` on its own port) |
| Models list, data backup/verify | ✅ | `cli-models-and-data-backup` |
| Goal commands | ✅ | `cli-goal-commands` |

## Remote

Four surfaces: the two UIs side by side (`remote`), the relay itself with real
Rust processes (`remote-live`), the **phone's own client** against a real desktop
(`remote-mobile-live`), and the deployed platform (`remote-cn`).

`remote-mobile-live` is the one that removes the last mock: it bundles
`mobile/src/remote/pairing.ts` + `client.ts` and runs them in Node with only
`expo-device` / `expo-secure-store` / `react-native` stubbed, so the invitation is
really claimed, the WebSocket really connects, and a write from the phone really
lands in the desktop's `app.db`.

| Area | Coverage | Cases |
|---|---|---|
| Desktop + mobile render the same session | ✅ | `remote-parser-pair`, `remote-page` |
| Desktop UI: both directions of the remote page | ✅ | `remote-page-peers-held`, `remote-page` |
| Self-hosted relay: invitation → pairing → serving | ✅ | `remote-live-pair-and-connect` |
| Relay + broker restart recovery | ◐ | `remote-live-relay-restart` |
| Unpair, then pair again with a fresh invitation | ◐ | `remote-live-repair-after-unpair` |
| Host-side revocation: the machine stops serving the removed device | ◐ | `remote-live-host-unpair` |
| Phone (real client code) pairs and reads the desktop catalogue | ✅ | `mobile-live-pair-and-list` |
| Phone writes to the desktop (pin) and the DB confirms it | ✅ | `mobile-live-pin-roundtrip` |
| Two clients: standby with a holder, takeover moves control | ◐ | `mobile-live-two-clients-control` |
| Official platform: reachability, route and error-shape contract | ✅ | `remote-cn-platform-contract` |
| Official platform: full pairing through the deployed relay | ◐ | `remote-cn-platform-contract` with `FUTURE_REMOTE_CN_FULL=1` (needs `future auth login`) |
| Phone/desktop driving a prompt and watching the stream | — | next: needs a model behind the host (costs credits), and a client runtime the CLI cannot hold |
| File transfer, image latency, weak network | — | unit-tested in `packages/remote-*`; not in this library |
| Phone UI states while standby / revoked / reconnecting | ✅ | `mobile-standby-takeover`, `mobile-pairing-revoked`, `mobile-connection-dropped-escape` (the screens, on mocks; the transport itself is `mobile-live-*`) |

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

## Known defect records (red on purpose)

`cli-task-add-same-name` is red against today's build: naming a second task the
same as an existing one is refused, and the refusal reaches the user as
`UNIQUE constraint failed: tasks.name` — the store's own words, in both the CLI
and the desktop editor. Users asked for repeated names to be *allowed*; what the
case encodes is the weaker promise that whichever policy wins, the sentence the
user sees is written for them. Fix it and the case turns green; do not delete it
to make a run look clean.

## Not covered, and why

- **The scheduler tick itself** (`host::tasks::tick` → claim → run). A real tick
  needs an agent that answers the prompt, so the harness stops at the store: the
  CLI cases prove a task has a real next slot and a queued request, and the
  desktop case proves the panel reads what the tick wrote. Neither proves the
  tick picks it up.
- **One wedged task freezing the whole schedule.** That is the failure mode
  behind the reported "tasks stop firing for twelve days", and it lives in the
  tick, not in any surface the harness owns.
- **Agent-side queue acks** (`recentTerminalAcks`, e.g. a finished turn's
  `settled` row that must not surface as "a waiting message left the queue").
  The host filters those before any client sees them
  (`host/src/agent_bridge/queue.rs`), so it is covered there by unit tests; a
  browser case would have to fake the filtering it is meant to check.
