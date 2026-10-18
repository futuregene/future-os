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

## Managed directory cleanup

Restoring an older app.db never authorizes deleting newer files. Unowned images, chat workspaces and review repositories are recorded in `managed_directory_cleanup` and kept in place. Explicit conversation/workspace deletion records cleanup intent in the deletion transaction. Cleanup rechecks live shared owners under a write lock and retains intent on failure. Age alone never authorizes deletion of unknown data.

## Database admission and upgrade contract

Desktop now uses application_id `0x46554150` and user_version `3`. Known legacy
Desktop layouts and CLI settings/workspace bootstrap databases are adopted through
the existing release migrations. `app_schema_migrations` records the new contract
checksum; `app_database_identity` persists database and restore identities. The
historical migration identities remain unchanged. A current contract checks core
columns, foreign-key targets and the conversation binding index before serving.
It does not replay historical destructive cleanup. Row-level orphan runs remain
addressable for reconciliation and cancellation.

Before upgrading a populated unversioned Desktop database, a verified SQLite
snapshot is published under `app/backups/`. Before upgrading a recognized older
Agent schema, a snapshot is published under `agent/backups/`. Snapshots contain
committed WAL contents, use unique names, retain original tables and are never
overwritten on retry. Failure to create/verify the snapshot prevents migration.
These are independent database snapshots, not a coordinated two-store backup or
an attachment backup; the coordinated commands below provide that separately.

The Desktop adoption contract is committed only after structural validation.
Legacy migration steps retain their own transaction boundaries: a failed later
step leaves earlier committed steps resumable, rather than claiming the entire
upgrade rolled back. The original snapshot remains available. Corrupt current
history structures are reported, never masked by creating empty authoritative
tables. Shared CLI writers reject foreign/future Desktop database headers too.

## Offline coordinated backup and restore

`future data backup /absolute/new-backup-directory` reserves both instance locks
and both database write boundaries before snapshotting. The backup includes both
databases, managed images/review/chat roots, legacy session/event sources and Agent
settings/models/auth files. It must be outside the FutureOS home, is published
only after SQLite integrity checks and SHA-256 file verification, and never
replaces an existing backup. Unsupported symbolic links/file types fail the
operation rather than being followed or silently omitted. User-selected external
workspace directories are not copied. Backups contain credentials and must stay
private.

`future data verify DIRECTORY` validates an existing set without starting Agent.
After stopping all writers, `future data restore DIRECTORY --yes` restores to the
original data root so saved working-directory paths keep their meaning. It stages
and verifies replacements, rotates restore identities, then fences normal startup
with `.restore-incomplete.json` while switching managed roots. Original files,
including prior WAL/SHM, remain under `.restore-work-<id>/previous/`.

`future data resume` completes an interrupted switch; `future data rollback`
restores the previous files. Repeating either after a process interruption is
safe at every rename boundary. The fence is removed only after the operation has
finished; a successful forward restore also verifies the final file set. Recovery
work directories are retained, never automatically reclaimed by orphan cleanup.
The tool never stops an existing process or removes instance lock files.

## Fault termination

Abort and persistence retry share a serialized fault-outcome boundary. An immediate SQLite transaction preserves existing terminal results and settles only an unfinished exact run. A live task cannot receive a fabricated terminal or release its lease. Repeated cancellation of a settled run is idempotent while stale requests cannot affect newer work. `abort_retry` propagates recovery failures.


## Source ownership, queue receipts and disposable indexes

Agent storage schema 7 persists a database family ID and restore generation.
Desktop contract 2 binds each conversation to that source. A changed database
cannot silently claim an existing conversation. Restores rotate the generation,
which fences history cursors and replaces cached display windows instead of
mixing snapshots. Legacy GUI archives use bounded pages and remain read only;
the UI explains that the matching Agent source must be restored before sending.

`session_tombstones` records native and imported deletions independently of
session rows. Bounded `get_session_presence` requests distinguish present,
deleted, missing and failed import. Reconciliation removes a mirror only for
proven deletion from its recorded database; absence alone preserves it.

`queued_run_receipts` commits the request identity and original input before a
queued acknowledgement. The canonical run start consumes receipts in the same
transaction as the user/start entries. Restart materializes unstarted accepted
input with an interrupted/cancelled terminal once; it never resumes model calls
or tools. Request retries retain the original run identity. Unrecoverable receipt
bytes remain stored and are reported, rather than blocking unrelated sessions.

`history_index_state` versions disposable projections. Transactional invalidation
triggers expose partial index changes; missing tables invalidate all index stamps
at open. Opening one affected conversation rebuilds shapes and display rows from
canonical records in batches, without rewriting history or replaying work. Normal
pages check one indexed stamp and load only selected bodies. Listings fetch the
first user, current session metadata and last model change, and count user rows
without hydrating every user message.

`future doctor` asks the running storage owner for schema/identity and catalogue
counts, including failed imports, unrecoverable queued inputs and pending index
rebuilds. Agent failures are reported as unverified health, not an empty healthy
catalogue. This is catalogue diagnosis, not a full SQLite integrity scan; offline
backup verification performs SQLite integrity and file-hash checks.

## Manual acceptance for this change

Local unit tests are intentionally not executed. Added fault cases compile with
static checks; CI can execute them once a PR is authorized. Validate a consistent
copy under an isolated `FUTURE_HOME`: historical database upgrades and repeated
startup; archive-only read-only paging; foreign database refusal; orphan-run
termination and repeated abort; accepted queued inputs after restart; Force quit
with a stalled cleanup (independent 25-second watchdog); restore interruption and
resume/rollback; and reconstruction of missing disposable history indexes with
canonical records unchanged. Previously erased bytes require an earlier backup
or original import source. Full backup currently rejects symlinks/special files,
requires stopped writers and restores only to its original home.


## Additional failure boundaries

Desktop contract 3 adds `agent_delete_sources`. Thread/workspace deletion and
explicit reset preserve proven database ownership with the pending intent, even
after thread rows disappear. Delivery checks that ownership through the same
instance-fenced client; foreign or unqualified old intent remains queued with an
error, never silently assigned to the current database. Migration from contract 2
qualifies only ownership still provable from existing source bindings.

History, fragment reads, prompt admission and observer replay use their verified
client throughout the operation. Cursor keys include the Agent instance; a new
instance or restore generation clears observer cursors and transient run bindings,
so lower restored event indexes can be read. State/title reads verify ownership;
stopping a known foreign source settles the local run without aborting foreign
work. Healthy history and runtime metadata use read transactions, reserving a
writer only for index reconstruction. Failed activation of coalesced queued input
materializes each unstarted input without waiting for restart or executing work.

Restore journal version 2 records deterministic preimage digests, checks retained
roots before moving more files and verifies originals before clearing a rollback
fence. Version 1 pending journals remain supported. Backup verification rejects
unmanifested SQLite sidecars; snapshot publication and complete tree staging sync
directory entries as well as file bodies. Added cases cover these boundaries but
are only compiled locally, as local unit-test execution is intentionally disabled.

### Preflight verification on repeated recovery

Before another forward rename, resume verifies every replacement root across its
installed and staged locations: hashes, SQLite integrity, and the complete managed
file/directory inventory must match the restore manifest. Omitted records, duplicate
directories or damaged replacements keep startup fenced before more originals move.
Rollback verifies original preimages independently, so a damaged replacement does
not prevent rollback of intact originals. An interrupted rollback retains its
direction even when the next command is `resume`.

Nonzero Desktop headers must agree with the entire migration ledger (count, range,
and each checksum). A regressed header or extra/future ledger row cannot replay
old steps. Background startup/run reconciliation and stream attachment verify known
source ownership on the same Agent connection before querying state, repairing local
rows or requesting fault cancellation. Compaction, title generation and remote
session settings obey the same source boundary. The five-second idle-confirmation
budget includes connecting, source verification and all RPCs; timeout is never
proof of a stopped session. The independent 25-second explicit Force quit watchdog
remains in place.

Regression cases cover omitted attachments, damaged staged replacements, repeated
rollback interruption, contradictory migration ledgers, foreign-database background
repair and mutation. Tests are compiled by static checks but are not run locally.
