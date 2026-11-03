# Rust diagnostics

[中文](logging.zh-CN.md)

Rust entry points share `future-logging`, using `tracing`, `tracing-subscriber`
and `tracing-appender`. File records are JSON Lines; terminal output remains
readable text. Frontend console capture and runtime diagnostic switching are
outside this implementation.

## Locate files

Paths are relative to the effective FutureOS data root (`~/.future`, overridden
by an absolute `FUTURE_HOME`):

| Component | Path |
|---|---|
| Desktop/Tauri/application Host | `app/logs/desktop.log` |
| Agent, embedded or standalone | `agent/logs/agent.log` |
| Foreground `future host` | `app/logs/host.log` |
| Channel bridge | `channels/logs/channel.log` |
| Terminal UI | `tui/logs/tui.log` |
| Ordinary CLI diagnostics | `cli/logs/cli.log` |

`future logs status [--json]` lists the canonical files and this CLI build's
default profile. It does not inspect another running process's effective filter.
`future logs path --component agent` prints one canonical path; other components
are `desktop`, `host`, `channel`, `tui`, and `cli`.

An embedded owner registers a separate Agent writer only after taking the Agent
instance lock. Attaching Desktop never opens an external Agent's file. The
writer guard outlives service cleanup and flushes on ordinary shutdown. Each
file has an OS ownership lock; overlapping TUI/CLI/Channel processes use
`<component>.<pid>.log` beside the canonical file instead of rotating its owner's
active file. Lock files are not evidence of a running owner: the OS lock is.

## Profiles and output

Distribution versions beginning with `0` select Detailed (business DEBUG);
formal releases select Normal (business INFO). Both keep unrelated library
chatter at WARN. Distribution identity is independent of Cargo's release/debug
optimisation profile. Routine LLM request and per-turn usage detail is DEBUG;
run results, warnings and errors remain available in Normal.

`RUST_LOG` overrides the filter at process startup using standard EnvFilter
syntax, for example `RUST_LOG=warn,futureos_lib::commands::history_transfer=trace`.
No CLI command changes a running process's filter. Standalone Agent's existing
`--verbose` requests Detailed; `--log-file [PATH]` / `FUTURE_AGENT_LOG_FILE` can
override its default file. File diagnostics are enabled by default.

Files include timestamp, level, target, component, PID, process instance,
version, structured event fields and span context. Machine-readable CLI output
is separate. TUI logging is file-only so it cannot overwrite the terminal UI.
Streamed thinking/reply output remains terminal/session-journal content, and is
not copied into the diagnostic files.

Each file rotates at 8 MiB. Detailed keeps up to 11 backups (approximately
96 MiB including the active file); Normal keeps two (approximately 24 MiB).
Old backups are pruned on open, after seven days for Detailed and three for
Normal. A new launch rotates its previous file, preserving pre-upgrade text
logs without mixing them with new JSONL. The Host also preserves its previous
1,000-line tail in `*.prev.log`. Agent's `--log-max-lines N` adds a line rotation
threshold; zero, the default, uses byte retention alone. Each writer has a
4,096-entry bounded background queue. Queue loss is reported to stderr on
retirement; file failure does not prevent application startup. Abrupt process
termination may lose queued records. Panic evidence retains its separate
best-effort `app/logs/crash.log` path.

## Storage and history investigation

The schema/content architecture introduced in PR #1393 is instrumented at
operation boundaries, without additional history scans or body serialisation:

- Agent/App database admission, source/target schema versions, verified upgrade
  snapshot creation, migration starts/commits/failures, and installation timing.
- Storage startup phases, interrupted-run recovery, projection invalidation and
  rebuilding, and activity-stamp repair.
- History page cursors, cache hit, projection rebuild, entry count, elapsed time,
  failures, and reads taking at least one second.
- Native history transfer creation/preparation, queue admission, size/chunk
  count, expiry, completion, explicit release and failed read context. Per-chunk
  offsets/age are TRACE; successful lifecycle detail is DEBUG. Existing returned
  error strings and transfer lifetime are unchanged.

Use `event`, session/run ID and `transfer_id` to correlate the Agent and Host
files. In particular, `history_transfer_retired` records `completed`, `expired`
or `released` when the snapshot disappears; a later missing read does not need
to guess why it disappeared. These new events record metadata, not prompt,
thinking, tool argument/result bodies or credentials.

## Diagnostic migration and output boundaries

Agent, Host and Desktop Rust diagnostics use `tracing` with severity chosen by impact:

| Level | Use |
|---|---|
| ERROR | Startup/service failure, run-state or approval persistence failure, failed config rollback, caught panic |
| WARN | Recoverable failure, degradation, failed maintenance, slow commands, first Agent event-stream interruption |
| INFO | Startup/exit, import/cleanup/repair results, remote service lifecycle |
| DEBUG | Reconnect attempts, cache eviction, routine refresh, expected deferral, shutdown detail |

Events retain their readable messages and attach structured event names, errors,
session/run IDs and counts where applicable. Windowed processes use the same
writers. Compatibility string callbacks still forward to tracing; new call sites
should select their severity directly.

Separate output boundaries remain intact: CLI JSON/help/argument errors, updater
and sandbox helper parent/child protocols, skill child output forwarding, and
model thinking/reply/newline streams. These outputs remain outside diagnostics.
Frontend code is outside this migration.

Run `cargo run -p future-logging --example audit_diagnostics` for a static source
audit. It follows the production module graph for Agent, Host and Desktop,
including platform branches and calls nested in DSL macros such as
`tokio::select!`, while excluding test code. The explicit allowlist in
`packages/logging/tests/support/output_boundaries.json` pins each preserved
output's source location, tokens and count. New raw diagnostics or changes to
preserved output fail the check. CI integration tests reuse the audit and check
its ability to find nested calls. This command only parses source: it starts no
application and runs no unit tests.


Storage worker diagnostics separate queue time from execution time. A
`storage_job_slow` row is emitted when either exceeds one second and includes
`job_id`, `lane` (`writer`, `reader`, `fence`, `snapshot`, or `checkpoint`), `job`, the Rust source
`operation` location, `queued_ms`, `elapsed_ms`, `total_ms`, `success`, and the
originating RPC `command` / `session_id` when available. SQL and conversation
bodies are not logged. `storage_job_waiting` reports work still pending after
one second (then every thirty seconds), with `phase=queued` or `running`.
Upgrade backups additionally emit `database_backup_step_started` /
`database_backup_step_completed` for source validation, copying, backup
validation and durable publication.

Read-only RPC cancellation and deadlines follow queued reads into SQLite's
progress handler. An expired queued read is skipped; an executing query is
interrupted cooperatively. Writes still complete their admitted durable work.
OS-level blocking I/O cannot be interrupted by the SQLite progress callback.


First-prompt startup is reported separately: `new_session_stage` splits storage
inspection, journal binding, configuration and publication; `prompt_start_stage`
splits Host session setup and the workspace review snapshot. `agent_command_slow`
records the command and session when one Host-to-Agent RPC takes at least a
second. A new session receives model/thinking selection in its creation request;
its first prompt does not repeat those configuration mutations.
