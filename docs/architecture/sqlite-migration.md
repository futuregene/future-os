# Agent SQLite migration and operations

> ([中文](sqlite-migration.zh-CN.md)) The Agent's only runtime store is
> `~/.future/agent/agent.db`; Desktop keeps using its independent
> `~/.future/app/app.db`. The database model is in
> [ER](../internals/desktop/ER.md#7-agent-sqlite-storage), the user experience in
> [PRODUCT](../internals/desktop/PRODUCT.md). Agent and all clients must upgrade
> in sync; no old/new RPC mixing is supported.

## Automatic import

The new Agent, under the instance lock and before serving externally, imports
old `sessions/*.jsonl` and their sibling `run-events/` files. Startup scans again
for files restored later; per-session records prevent duplicate imports,
automatic retries of skipped sessions, and resurrection after deletion. Each session is
an independent transaction: successful data commits together with the import
record; material corruption skips the whole session while other sessions
continue. Only a confirmable truncated trailing line is allowed, keeping the
complete prefix and recording a warning; disk, SQLite, and other global
failures stop startup.

Source files are neither deleted nor updated. After import only SQLite is read
and written — no JSONL dual-write, offline fallback, or downgrade. Migration
records are kept independently of sessions: successful items are not
re-imported, skipped items are not auto-retried, and deleted items are not
resurrected. A skipped session may still keep its Desktop entry, which fails
explicitly when opened — never faking an empty history.

## Older Desktop databases

The `v1.2.4-history-preservation` migration runs transactionally before legacy
GUI tables are removed. It copies every column of `messages`, `run_events`,
`tool_calls`, and `tool_outputs` to read-only `legacy_*` archives. Tables with
foreign keys to the retired tables are also archived and rebuilt from the
current schema, preserving shared columns and reinstalling indexes. Migration
failure keeps the original tables and leaves the marker unapplied for retry.
Fresh databases record the same migration without creating empty archives.

`legacy_thread_bindings(thread_id, session_id)` preserves read bindings before
the released duplicate-binding migration detaches older projections. Agent
history remains authoritative. Missing or empty sources can display preserved
GUI messages (including mixed-message attachment metadata); this recovery read
returns the complete archive without mixing Agent pagination cursors. Archives
are not LLM context: an unbound recovered thread cannot silently create an empty
Agent session. Restore its original Agent data to continue it faithfully.

Missing source data without an archive is an explicit history error, not an
empty conversation. Startup catalogue reconciliation preserves missing
conversations and assets: absence from a different or partially restored Agent
database is not proof of deletion. Explicit conversation deletion clears its
archives too. Already deleted history needs an earlier backup; this migration
cannot recreate bytes removed by an older build.

## Inspection and explicit retry

First stop that user's Agent and any clients that might start it; always obey
the one-Agent-per-OS-user rule and never start a second instance by changing
the socket/port.

```sh
future-agent --migrate-sessions
future-agent --retry-session-import SESSION_ID
```

Reports contain only the session identifier, status, error source/line
number/controlled category, and warning count — no conversation bodies. Only
skipped items can be retried; after fixing the source, retry explicitly —
never overwriting successful or deleted items.

For isolated verification, copy `sessions/` and its sibling `run-events/` into
a separate directory and use:

```sh
future-agent --migrate-sessions --migration-source /absolute/path/isolated-copy/sessions
future-agent --retry-session-import SESSION_ID --migration-source /absolute/path/isolated-copy/sessions
```

The database is created in that `sessions/`'s parent directory. An isolated
path does not exempt the Agent instance lock.

## Backup and recovery boundaries

- Stop the Agent, then back up the complete database and the still-present
  `-wal` / `-shm`; never copy only `agent.db` while it is running.
- JSONL is only a pre-import snapshot and does not contain later SQLite new
  conversations. Rebuilding the database cannot recover those additions and
  should not be a routine troubleshooting measure.
- Unreleased development layouts keep no upgrade chain; unrecognized layouts
  are explicitly refused to open, never auto-deleted or overwritten. Schema
  changes after official release must provide migrations.
- High-frequency deltas commit in 100 ms / 128 entries / 64 KiB micro-batches;
  semantic events, reads, and closes form flush boundaries. An abnormal exit
  may lose uncommitted deltas; 100 ms is a scheduling target, not a hard cap on
  the loss window.
- `WAL`, `synchronous=FULL`, and transactions protect committed data; the WAL
  retention target is not a hard disk-occupancy cap, and `VACUUM` is not
  needed at every startup.

## Regression requirements

Automated fixtures are fully constructed, use temporary directories, and never
read personal conversations. Keep tests for transaction rollback,
uniqueness/conflicts, import idempotency/skip, deletion tombstones, fork, run
recovery, tool isolation, paging ranges, event cursors, and queue failures.
Real-machine verification covers cold open, first reply, scroll-up, tool
details, compaction, anomalies, restart, and reconnect; unit tests do not
substitute for cross-platform and WebView acceptance.

## Batched session deletion

`delete_sessions` accepts up to 32 distinct session IDs; `delete_session` remains
compatible and uses the same close-before-delete barrier. Ready sessions and
import tombstones commit together, while busy sessions return retryable outcomes.
Reclamation runs once per batch (up to 4096 pages), rather than once per session.
This is a page budget, not a latency guarantee; large freelists can remain for
later deletes or startup compaction. Desktop uses bounded local transactions and
one Agent client, with durable per-session acknowledgement/retry handling.
