# Host internals map

> [中文](host.zh-CN.md)
>
> **This is a map, not a design document.** It says where things live in
> `host/` and what to read before changing them; the design argument, where one
> exists, is in the document each row links to.

`host/` builds `future-host`, the GUI-free application backend shared by two
entry points: the Desktop GUI (`futureos`) and the foreground `future host`
command (`cli/src/headless/`). It owns the Desktop store at
`~/.future/app/app.db`, the Agent hosting/bridge layer, remote, scheduled tasks
and shutdown. Its crate header states the boundary: entry points own runtime
creation and presentation, and the library does not depend on Desktop, Tauri or
the CLI (`host/src/lib.rs:1-4`).

`host/src/lib.rs` declares 41 modules — 35 `pub mod` and 6 private
(`error`, `git_diff_parse`, `proc`, `home`, `remote_host`, `remote_platform`);
`test_support` is gated on `test` / the `test-support` feature. The tables below
cover all of them.

Note the two file layouts. `store`, `shadow_review`, `shutdown`, `tasks`,
`agent_host` and `test_support` keep their module root in `host/src/`, e.g.
`host/src/store.rs` with submodules under `host/src/store/`. `agent_bridge`,
`agent_providers`, `application`, `remote`, `remote_host` and `remote_peer` keep
it in the directory, e.g. `host/src/agent_bridge/mod.rs`.

**Where the GUI ends and this crate begins.** `desktop/src-tauri` keeps thin
forwarding modules plus the GUI-only parts: `desktop/src-tauri/src/store.rs` is
`pub(crate) use future_host::store::*`, and
`desktop/src-tauri/src/agent_bridge/mod.rs` re-exports
`future_host::agent_bridge::*` and adds only presentation. A change to stored
data, Agent bridging or remote belongs here, not there.

## Talking to the Agent

| Module | Responsibility | Start reading |
|---|---|---|
| `agent_client.rs` | gRPC client construction, endpoint discovery, error mapping | `host/src/agent_client.rs` |
| `agent_commands.rs` | Builds `RpcCommand`s and turns `RpcResponse` into `Result` | `host/src/agent_commands.rs` |
| `agent_host.rs` (+ `agent_host/`) | Shared GUI/headless owner of the Agent service: start/stop phases, singleton lock held through teardown | `host/src/agent_host.rs` |
| `agent_bridge/` | The application-side bridge: consume a prompt's event stream, per-session observers, persistence projection, session/approval/run-control commands, headless prompt pipeline | `host/src/agent_bridge/mod.rs`, `host/src/agent_bridge/stream.rs`, `host/src/agent_bridge/observer.rs`, `host/src/agent_bridge/persist.rs`, `host/src/agent_bridge/headless.rs` |
| `agent_events.rs` | Host event outlet; observers know only this sink, not remote transport | `host/src/agent_events.rs` |
| `agent_providers/` | Reads/writes the Agent's model and provider configuration for the Providers settings page | `host/src/agent_providers/mod.rs` |
| `request_scope.rs` | Per-host command admission and exact-run ownership | `host/src/request_scope.rs` |

## Local data and Desktop operations

| Module | Responsibility | Start reading |
|---|---|---|
| `store.rs` (+ `store/`) | `~/.future/app/app.db`: schema, migrations, threads, runs, approvals, artifacts, review snapshots, workspace files | `host/src/store.rs`, `host/src/store/schema.rs`, `host/src/store/migrations.rs` |
| `conversations.rs` | One meaning for "delete a conversation", wherever it is asked for | `host/src/conversations.rs` |
| `workspaces.rs` | Workspace application services | `host/src/workspaces.rs` |
| `files.rs` | File/attachment business and path-access boundaries | `host/src/files.rs` |
| `git_review.rs` | Working-tree diff review | `host/src/git_review.rs` |
| `shadow_review.rs` (+ `shadow_review/`) | Last-run delta, snapshotted into an isolated bare git repository | `host/src/shadow_review.rs`, `host/src/shadow_review/policy.rs`, `host/src/shadow_review/snapshot.rs` |
| `git_diff_parse.rs` | Splitting a unified patch per file and parsing `--numstat`, shared by both review paths | `host/src/git_diff_parse.rs` |
| `auth_store.rs` | Strict atomic `0600` read of `~/.future/agent/auth.json`; local fallback writer only | `host/src/auth_store.rs` |
| `config_io.rs` | Strict read + atomic write for `models.json`, `auth.json`, `approval_rule.json` | `host/src/config_io.rs` |
| `approval_rules.rs` | Appends workspace `allow` rules to `${WORKSPACE}/.future/approval_rule.json` | `host/src/approval_rules.rs` |
| `application/` | Shared application operations behind entry adapters (approvals, review, runs, skills, threads) | `host/src/application/mod.rs`, `host/src/application/runs.rs` |

## Remote

| Module | Responsibility | Start reading |
|---|---|---|
| `remote/` | Host role: devices pair *into* this machine. Pairing, operator commands, relay, supervisor, diagnostics | `host/src/remote/mod.rs`, `host/src/remote/pairing.rs`, `host/src/remote/control.rs`, `host/src/remote/relay.rs`, `host/src/remote/supervisor/mod.rs` |
| `remote_host/` | The business side of a paired device's command: catalog, sessions, files, lean pages | `host/src/remote_host/mod.rs`, `host/src/remote_host/business/mod.rs`, `host/src/remote_host/catalog.rs` |
| `remote_peer/` | Client role: this desktop pairing *out* to another desktop | `host/src/remote_peer/mod.rs` |
| `remote_platform.rs` | The shared HTTP client for Remote roles and relay control | `host/src/remote_platform.rs` |

## Process lifecycle and shared services

| Module | Responsibility | Start reading |
|---|---|---|
| `instance.rs` | One backend per data directory; the file and its OS lock stay open for the process lifetime | `host/src/instance.rs` |
| `startup.rs` | Startup steps plus a watchdog that keeps a stalled launch diagnosable | `host/src/startup.rs` |
| `shutdown.rs` (+ `shutdown/`) | One process-wide stop operation shared by every exit/reset/update caller | `host/src/shutdown.rs`, `host/src/shutdown/services.rs` |
| `runtime.rs` | Shared task ownership, bounded cleanup; entry points install the runtime | `host/src/runtime.rs` |
| `scheduler.rs` | Process-lifetime scheduler primitives and the fixed-interval jobs (app update, Future balance, Future models) | `host/src/scheduler.rs` |
| `events.rs` | Presentation outlet for committed backend changes | `host/src/events.rs` |
| `environment.rs` | Shared process preparation and diagnostics | `host/src/environment.rs` |
| `home.rs` | Home directory and `future_home_root()` (`FUTURE_HOME`-aware) | `host/src/home.rs` |
| `device_identity.rs` | Stable installation `device_id` | `host/src/device_identity.rs` |
| `build_info.rs` | Version and distribution channel injected at build time | `host/src/build_info.rs` |
| `cli_tool.rs` | Discovery of the CLI binary; never executes an empty placeholder | `host/src/cli_tool.rs` |
| `proc.rs` | Child-process helpers (no console window on Windows) | `host/src/proc.rs` |
| `error.rs` | The unified application `Error` | `host/src/error.rs` |
| `run_error.rs` | Classifies raw agent error strings into UI categories | `host/src/run_error.rs` |

## Accounts, tasks, onboarding

| Module | Responsibility | Start reading |
|---|---|---|
| `future_login.rs` | FutureGene device-code OAuth; stateless, the caller polls | `host/src/future_login.rs` |
| `future_platform.rs` | Resolves the platform/model-API base URL from `auth.json` | `host/src/future_platform.rs` |
| `tasks.rs` (+ `tasks/`) | The Desktop's tick loop and `Executor` wiring for `future-tasks` | `host/src/tasks.rs` |
| `skills.rs` | The unauthenticated platform skills guide used by Skills onboarding | `host/src/skills.rs` |
| `test_support.rs` (+ `test_support/`) | Test fixtures shared by library and entry tests | `host/src/test_support.rs` |

## Change recipes

| I want to change… | Read first |
|---|---|
| The Desktop data model or a stored row | `host/src/store/schema.rs`, `host/src/store/migrations.rs`, then [ER](desktop/ER.md) and [SQLite migration](../architecture/sqlite-migration.md) |
| What a Desktop run persists from the Agent stream | `host/src/agent_bridge/stream.rs`, `host/src/agent_bridge/persist.rs` |
| The prompt send path (GUI, phone, task) | `host/src/agent_bridge/prompt.rs`, `host/src/agent_bridge/headless.rs`, `host/src/application/runs.rs` |
| Approval decisions or the approval UI data | `host/src/agent_bridge/approval.rs`, `host/src/application/approvals.rs`, `host/src/approval_rules.rs`, then [auto approval](desktop/AUTO_APPROVAL.md) and [SANDBOX](desktop/SANDBOX/COMMON.md) |
| What a phone may ask this machine for | `host/src/remote_host/business/mod.rs`, `packages/remote-server`, then [CONNECTION](desktop/CONNECTION.md) and [REMOTE_E2EE](desktop/REMOTE_E2EE.md) |
| Pairing or the self-hosted relay | `host/src/remote/pairing.rs`, `host/src/remote/relay.rs`, then the [relay guide](../guide/self-hosted-relay.md) |
| Startup, lock or shutdown behaviour | `host/src/startup.rs`, `host/src/instance.rs`, `host/src/shutdown.rs`, then [agent supervision](../architecture/agent-supervision.md) and the [host guide](../guide/desktop-host.md) |
| A periodic maintenance job | `host/src/scheduler.rs` plus the entry that ticks it (`desktop/src-tauri/src/scheduler/mod.rs`) |
| An app settings key or default | `packages/app-settings/src/lib.rs` (single source of truth) and `host/src/store/app_settings.rs` |
| Scheduled-task behaviour | `host/src/tasks.rs`, then [scheduled tasks](../architecture/tasks.md) |
| The "last-run changes" diff | `host/src/shadow_review.rs`, `host/src/git_diff_parse.rs` |
| Skills onboarding copy | `host/src/skills.rs`; actual skill installation lives in the Agent (`agent/src/skills/mod.rs`) |

## Existing documents that already cover part of this crate

- [Headless package development](../architecture/headless-package-development.md) — how `future-host` was extracted and what acceptance it reached
- [Host mode guide](../guide/desktop-host.md) — operating `future host`
- [Desktop Agent guide](../guide/desktop-agent.md) — embedded/external hosting, logs, update recovery
- [ER](desktop/ER.md) — Desktop product data model and Agent storage
- [CONNECTION](desktop/CONNECTION.md), [REMOTE_E2EE](desktop/REMOTE_E2EE.md) — remote transport and end-to-end encryption
- [Remote package development](../architecture/remote-package-development.md) — the remote package boundary (`packages/remote-server`, `packages/remote-client`, `packages/remote-protocol`, `packages/remote-crypto`)
- [Scheduled tasks](../architecture/tasks.md) — the `future-tasks` kernel this crate drives
- [SQLite migration](../architecture/sqlite-migration.md) — `app.db` and Agent schema policy
- [Agent startup arbitration](../architecture/agent-supervision.md) — instance-lock ownership across entries
- [Directory layout](../guide/directory-layout.md) — every `~/.future/` path this crate writes

## Out of scope for this map

The Agent's own modules are mapped in [agent/README.md](agent/README.md). The
GUI's remaining modules (`desktop/src-tauri/src/terminal/`, windows and menus,
platform power helpers, update transactions) still live in `desktop/src-tauri` and are described by the
[desktop documents](desktop/PRODUCT.md) rather than here.
