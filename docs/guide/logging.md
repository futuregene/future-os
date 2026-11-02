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
