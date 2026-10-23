# Agent internals map

> [中文](README.zh-CN.md)
>
> **This is a map, not a design document.** It says where things live in
> `agent/` and what to read before changing them. It deliberately does not
> argue for the design: each row points at the document that does, or at the
> module's own `//!` header.

`agent/` builds `future-agent`, the gRPC backend that owns `~/.future/agent/`.
The crate is also a library: `agent/src/main.rs` is an 11-line wrapper around
`future_agent::cli::run_from_args`, and `future agent` runs that same entry, so
the standalone binary and the embedded subcommand cannot diverge
(`agent/src/cli.rs:1-6`).

`agent/src/lib.rs` declares 22 public modules plus the private `system_one`.
The tables below cover all of them.

## Where a request goes

1. A client calls the `FutureAgent` gRPC service (`grpc/`), which dispatches
   every command through `handle_command_internal` (`agent/src/rpc/commands/mod.rs`).
2. A run-control command hands the run to `runtime::SessionRuntime`, which owns
   cancellation and the single background task slot for that session.
3. The turn itself is `agent/src/agent/run_loop.rs`: model stream → tool calls → repeat,
   emitting `RunEvent`s (`agent/src/agent/events.rs`).
4. Model access is `llm/` (HTTP + SSE, one adapter per wire protocol); built-in
   tools are `tools/`; tool processes and file access are gated by `sandbox/`.
5. History is written by `session/` to `~/.future/agent/agent.db`; what the
   model sees in place of the full history is `compaction/`.

## Top-level modules

| Module | Responsibility | Start reading |
|---|---|---|
| `agent/` | The turn loop: stream the model, execute tool calls, repeat; publishes `RunEvent`s | `agent/src/agent/run_loop.rs` (`run_streaming_with_messages`), `agent/src/agent/events.rs` |
| `types/` | Core protocol types (`AgentMessage`, `ContentBlock`, `LLMProvider`, `ToolDef`) | `agent/src/types/mod.rs` |
| `engine/` | Binds provider + model + tools + session + loop for one run | `agent/src/engine/mod.rs`; constructed in `agent/src/service/startup.rs` |
| `service/` | Host-independent startup, serving and shutdown with a scoped instance lock | `agent/src/service/mod.rs` (`AgentService::prepare`, `AgentHandle`) |
| `grpc/` | tonic `FutureAgent` service over local IPC (default) or TCP | `agent/src/grpc/mod.rs` (`AgentListener`) |
| `rpc/` | Command dispatch (`agent/src/rpc/commands/`, one file per domain), event delivery (`agent/src/rpc/broadcaster/`), control-plane revision, interrupted-run recovery (`agent/src/rpc/resume.rs`) | `agent/src/rpc/commands/mod.rs`, `agent/src/rpc/broadcaster/mod.rs`, `agent/src/rpc/control_plane.rs`, `agent/src/rpc/resume.rs` |
| `runtime/` | Per-session execution ownership: lifecycle, cancellation acknowledgement, queue budgets | `agent/src/runtime/session_runtime.rs`, `agent/src/runtime/scheduler_queue.rs` |
| `session/` | Durable transcript: entries, ordered SQLite worker, projections, fork, history paging, repair | `agent/src/session/mod.rs` (submodule list), `agent/src/session/persistence.rs` |
| `compaction/` | The bounded projection sent in place of the full history | `agent/src/compaction/mod.rs`, `agent/src/compaction/budget.rs`, `agent/src/compaction/durable.rs` |
| `llm/` | HTTP + SSE model client and per-protocol adapters | `agent/src/llm/mod.rs`, `agent/src/llm/adapters/`, `agent/src/llm/schema.rs` |
| `prompt/` | System-prompt assembly and cwd project instructions | `agent/src/prompt/mod.rs` (`build_prompt`), `agent/src/prompt/project_context.rs` |
| `models/` | Model catalog and resolution: built-ins, Future platform cache, `models.json` | `agent/src/models/mod.rs`, `agent/src/models/future.rs` |
| `config/` | `settings.json` read/write (`Settings`) and provider settings | `agent/src/config/mod.rs`, `agent/src/config/providers.rs` |
| `auth/` | API credentials from `~/.future/agent/auth.json` (fallback `~/.future/agent-app/auth.json`) | `agent/src/auth/mod.rs` |
| `tools/` | Built-in read / write / edit / shell tools | `agent/src/tools/mod.rs` (`all_tools`), `agent/src/tools/shell/` |
| `sandbox/` | OS sandbox backends and path-based approval rules | `agent/src/sandbox/mod.rs`, `agent/src/sandbox/backend.rs`, `agent/src/sandbox/rules.rs` |
| `approval_review/` | Model review that classifies Ask requests; deterministic policy owns execution | `agent/src/approval_review/mod.rs`, `agent/src/approval_review/policy.rs` |
| `skills/` | Skill discovery from `~/.future/agent/skills/` and `~/.agents/skills/` | `agent/src/skills/mod.rs` |
| `skill_reco/` | The Jev-backed `suggest_skill` recommender | `agent/src/skill_reco/mod.rs` |
| `cli/` | clap entry shared by `future-agent` and `future agent` | `agent/src/cli.rs` |
| `logfile.rs` | Log-file mirroring; keeps terminal and file output identical | `agent/src/logfile.rs` |
| `utils/` | Home/session directories, ID generation, path helpers | `agent/src/utils/mod.rs` |
| `system_one.rs` (private) | Shared Future System One transport; callers own the classification policy | `agent/src/system_one.rs` |

## Change recipes

| I want to change… | Read first |
|---|---|
| A built-in tool's schema or behaviour | `agent/src/tools/mod.rs` (`all_tools`), `agent/src/tools/shell/` for process spawning, `agent/src/sandbox/` for what it may touch |
| What the model sees as the system prompt | `agent/src/prompt/mod.rs` (`build_prompt` section order), `agent/src/prompt/project_context.rs` |
| How much history is compacted, or when | `agent/src/compaction/budget.rs` (`trigger_tokens`, `TARGET_HISTORY`), then [compaction](../compaction/compaction.md) and its [developer map](../compaction/compaction-development.md) |
| An RPC command or event shape | `packages/rpc/proto/future.proto` (then `make generate-proto`), `agent/src/rpc/commands/`, consumer side `host/src/agent_commands.rs`; see [RPC](../../architecture/rpc.md) |
| Run lifecycle, cancel or queueing | `agent/src/runtime/`, `agent/src/rpc/commands/run_control.rs` |
| How a reply may end (stop / refusal / filter / pause) | `agent/src/llm/schema.rs` (`FinishReason`), `agent/src/llm/adapters/`, then [response outcomes](../../architecture/response-outcomes.md) |
| Session storage or schema | `agent/src/session/database/`, `agent/src/session/sqlite_store/`, `agent/src/session/persistence.rs`; see [SQLite migration](../../architecture/sqlite-migration.md) and [ER](../desktop/ER.md) |
| A provider's wire protocol | `agent/src/llm/adapters/`, `agent/src/llm/schema.rs` |
| The model catalog or a provider entry | `agent/src/models/`, `agent/src/config/providers.rs` |
| Sandbox behaviour or a new rule | `agent/src/sandbox/` (backends: `agent/src/sandbox/seatbelt.rs`, `agent/src/sandbox/linux/`, `agent/src/sandbox/windows.rs`), then [SANDBOX](../desktop/SANDBOX/COMMON.md) |
| Approval review policy or its prompt | `agent/src/approval_review/`, then [auto approval](../desktop/AUTO_APPROVAL.md) |
| Agent startup, lock or exit | `agent/src/service/`, then [agent startup arbitration](../../architecture/agent-supervision.md) and the [Desktop Agent guide](../../guide/desktop-agent.md) |
| Skill discovery paths | `agent/src/skills/mod.rs`, then the [directory layout](../../guide/directory-layout.md) |
| Settings keys and defaults | `agent/src/config/mod.rs`, then [self-inspection](../../guide/self-inspection.md) for the read-back path |

## Existing documents that already cover part of this crate

- [compaction/](../compaction/compaction.md) — strategies, parameters, request shape, experiments
- [desktop/ER.md](../desktop/ER.md) — Agent SQLite storage, run and entry model
- [SANDBOX/](../desktop/SANDBOX/COMMON.md) — approval semantics and the three platform backends
- [architecture/rpc.md](../../architecture/rpc.md) — the wire contract this crate serves
- [architecture/response-outcomes.md](../../architecture/response-outcomes.md) — end-of-reply semantics
- [architecture/sqlite-migration.md](../../architecture/sqlite-migration.md) — schema upgrade and JSONL import
- [architecture/agent-supervision.md](../../architecture/agent-supervision.md) — who starts the Agent
- [skill_reco/evaluation.md](../skill_reco/evaluation.md) and [integration-plan.md](../skill_reco/integration-plan.md)
- [guide/session-history.md](../../guide/session-history.md) — reading history back after compaction

## Out of scope for this map

The Desktop application layer that talks to this crate is mapped separately in
[host](../host.md). The channel bridge and the TUI are their own crates
(`channels/`, `tui/`) and are not described here.
