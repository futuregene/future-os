# Format-2 compact stream storage contract

Date: 2026-10-27. Chinese: [紧凑变更契约](compact-stream-layout.zh-CN.md).
Related: [development plan](stream-storage-development.md), [space baselines](stream-storage-space-baselines.md).
This is the implementation contract; verification results are recorded separately.

## Layout and release boundary

Agent schema 8 directly replaces the unpublished target; logical conversation format remains 2.
The `stream_layout` singleton is `compact-owned-stream-v2`. There is no draft conversion, dual codec,
dual writing or historical body compression. Released schema 2–7 and released Desktop SQL/checksums
remain unchanged, and existing conversations continue using format 1.
Admission checks the marker, authoritative columns, index definitions, foreign keys and collection
triggers before configuring WAL or initializing an already-current database. Incompatible draft files
are refused and preserved. Use an isolated data root rather than clearing personal history.

Exact DDL: `agent/src/session/content/schema.sql`; codec: `compact.rs` in the same directory.

## Business identities and historical compatibility

New-format generated session, message and run UUIDs retain their complete 16-byte values in storage.
APIs and read-only inspection views render canonical lowercase 36-character UUIDs. Released historical
TEXT remains exact, including UUID-looking strings; opaque provider IDs retain their spelling.
Historical SQL declarations may still say TEXT: the actual storage class (`typeof`) is authoritative,
not the declaration or string length. `session/identity.rs` owns typed primary/foreign-key bindings;
internal stream, context and symbol references continue using integers.

Session lookup includes sessions, deletion tombstones and import registrations. Ambiguous TEXT/BLOB
representations of one public identity fail explicitly. Creation checks identity reservations in the
same transaction before selecting native UUID storage; existing conversations keep their format.
Restored JSONL sources cannot compete with an existing new-format conversation: preserve the file
and skip import, rechecking session/tombstone ownership under the commit lock. Never create another
representation, overwrite committed history or resurrect a deleted conversation.
`sessions_inspect`, `entries_inspect`, `runs_inspect` and `stream_symbols_inspect` compute readable
projections without persisting another UUID string copy.

## Immutable contexts

`stream_contexts` assigns a short integer ID to a descriptor scoped to one stream/run/epoch:
`[stable metadata object, direct field or null, direct encoding or null, symbol-reference object, kind code]`.
The extraction whitelist is envelope `run_sequence` and data `type`, `entry_id`, `tool_id`,
`tool_name`, `tool_type`, `tc_index`, `content_key`, `block_id`. Missing and explicit null remain distinct.
Changed fields create another context; a database trigger rejects UPDATE. Descriptors are capped at 16 KiB.

Lookup uses `(stream_id, SHA-256)` and compares the complete descriptor before reuse. Hash equality
alone is insufficient; collision candidates are bounded and overflow fails explicitly. A single-column foreign key and primary-key lookup triggers require the change and context to share a stream and event kind. No short-ID cache crosses transaction
boundaries, so aborted allocations cannot leak into later publications; SQLite's statement cache
contains statements only.

`stream_symbols` interns entry_id, tool_id, content_key, block_id, tool_name and tool_type within each stream.
Canonical UUIDs use lossless 16-byte storage; other strings retain their exact TEXT. Symbol UPDATE is refused.
Contexts contain integer symbol references; foreign keys check existence and primary-key lookup triggers enforce the owning stream.
The descriptor and expanded symbols each have a 16 KiB limit. Long identifiers no longer repeat in every
context or source index. `stream_contexts_entry/tool/key` find source identities and
`stream_changes_context` finds their event types and cursors. Partial source indices cover only the
corresponding event kinds rather than every delta. Source lookups filter contexts before changes.

## Changes and ranges

`stream_changes` stores stream/context IDs, first/last cursor, explicit kind code, remainder JSON,
range and presence flags. `metadata_json` is a remainder, not a protocol envelope; consumers must decode it.
Original timestamp strings retain their precision. Unknown kind code 0 keeps its spelling in `unknown_kind`.
`event_type` is a virtual generated column, avoiding another stored type string.

| Code | Event | Code | Event |
|---:|---|---:|---|
| 1 | agent_start | 10 | thinking_end |
| 2 | agent_end | 11 | tool_start |
| 3 | message_start | 12 | tool_delta |
| 4 | message_end | 13 | tool_input_end |
| 5 | text_start | 14 | tool_end |
| 6 | text_chunk | 15 | tool_result |
| 7 | text_end | 16 | usage |
| 8 | thinking_start | 17 | model_changed |
| 9 | thinking_delta | 18 | tool_sandboxed |

A single-field body range lives directly in the change; its field/encoding live in the context.
Bits 1/2/4 restore `content_offset` / `content_bytes` / `content_first_idx` from range/cursor only
when the original value matches exactly. Other values stay in JSON. Unknown fields, false, null,
empty collections and opaque provider strings remain intact. Multi-field facts use `stream_change_content`.
`stream_content_ranges` is a UNION ALL view over direct and extra ranges, with no duplicated storage;
low-frequency terminal extensions also read through this authority.

## Publication and lifecycle

Body staging, watermarks and confirmation boundaries retain the segmented contract. Staged bodies
use `stream_body_owners`, keyed by integer `(stream_id,key_id)` and sharing the stream symbol dictionary.
The encoded `[run,epoch,key]` adapter is transient and is never persisted in ownership rows or indices.
Staging may reserve a stream, symbol and body before publication; it does not advance the durable cursor.
The transaction publishing a change atomically publishes its contexts, ranges and watermark over durable bytes.
Unpublished owners protect staged bodies from collection; deletion queues collection and retired streams
release owners in bounded slices. Retries compare first cursor,
kind, complete decoded metadata and all body ranges/encodings; differing data conflicts explicitly.
Delta coalescing requires all metadata except derived range fields and the last-fragment envelope
clock to match, preserving changing extensions instead of overwriting them.

Range authorization still requires session/run/epoch/cursor/field and database identity; integers
are not access credentials. Metadata pages capture identity and watermark in the same snapshot,
rejecting restore-generation changes rather than mixing old metadata and new bodies.
Recovery and collection use the unified range authority; direct removal has its own reverse index
and collection trigger. Reset/prune retire changes, contexts, symbols and owners in bounded slices without
new historical scans or completion-time rewriting.

## Bounded pages

Cursor selection estimates remainder, descriptor and expanded symbol bytes using the 16 KiB page budget, existing
single-row exception and a 256-row limit. Changes, distinct contexts, extra fields and distinct symbols are then fetched
in batches; query count does not grow linearly with event count. Database workers return strings and
ranges; replay/recovery callers parse each context once and assemble events off the database worker.
Raw selections have a separate 256 KiB hard memory cap including extra fields and expanded symbols. Transactional terminal
and source boundaries may decode one bounded fact, without reading bodies or complete runs.

The 16 KiB segment / 64 KiB write / 256 KiB range read budgets, 25 ms/128 event/64 KiB scheduling,
WAL+FULL, independent checkpoints and backpressure remain unchanged. Savings require joint migration,
fault, WAL, read and responsiveness evidence.

Scoped references use single-column foreign keys for existence and primary-key lookup triggers for stream/kind ownership; immutable symbols/contexts prevent later ownership changes. Redundant composite unique indices over globally unique integer primary keys are removed. Admission verifies exact trigger definitions.
Bounded batches retain separate semantic facts while sharing a SQLite transaction. Encoding explicitly flushes prepared facts before body reads or independent staging; agent_end flushes immediately, and persist returns with no uncommitted pending batch. Byte, event and metadata bounds remain unchanged, without extending producer acknowledgement windows.
