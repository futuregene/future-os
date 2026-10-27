# Compact session storage and streaming development plan

Status: UUID and structured body ownership, bounded multi-fact commits, and scoped identity reuse are implemented, 2026-10-27. Realistic-workload comparisons and final full acceptance remain in progress; delivery is not yet approved.
Exact layout: [implementation contract](compact-stream-layout.md). Implementation status does not establish delivery acceptance.
[Candidate 1 raw results](../testing/compact-storage-candidate-1-2026-10-27.json) include a fresh previous-layout control.
[Candidate 2](../testing/compact-storage-candidate-2-2026-10-27.json) records binary/source validation, real UUID migration cases and page-write comparisons without replacing candidate 1.
Candidate 1 metadata improves 74.1% with high reuse but grows 60.5% under frequent new contexts; that result must not land. Per-stream short-key dictionaries and partial source indices are now implemented, with 16-byte business UUID integration continuing (section 14). Final space, write-volume and responsiveness comparisons must use the same completed version rather than reusing candidate 1's passing acceptance.
Chinese: [精简会话存储开发计划](stream-storage-development.zh-CN.md).

The complete previous plan remains in the [unpublished-plan archive](stream-storage-development-unreleased-2026-10-26.md).
Original, previous-implementation and prototype observations have separate [space baseline records](stream-storage-space-baselines.md).
The [previous raw acceptance JSON](../testing/segmented-storage-acceptance-2026-10-26.json) remains unchanged.

## 1. Non-negotiable goals

1. Generation, typing, scrolling, switching and cancellation remain responsive. No whole-run body conversion inside rendering, broadcaster locks or the SQLite writer.
2. Acknowledged user inputs, necessary execution/approval boundaries and acknowledged completed runs have durable commit evidence. Persist continuously to minimize the crash tail.
3. Support released historical layouts. Do not scan, compact, re-encode or rewrite stored historical bodies for space savings. New conversations start compact.
4. Finalization only seals current content blocks and commits terminal state: no whole-run snapshots, historical-segment merging or concentrated raw-event deletion.
5. History, tools, model context, reconnect and recovery share one durable body. SQLite remains the sole durable store, without another file journal/import pipeline.

Live display may precede commit; a sudden crash can lose the uncommitted tail. Severe storage lag backpressures or interrupts production while the UI and cancellation remain available. Do not claim saved data or buffer without bounds.
Model-context summaries, recall, attachments and fork/reset retain their existing product semantics.

## 2. Review against the previous plan

| Topic | Previous implementation / earlier proposal | Reviewed decision |
|---|---|---|
| One body | Immutable segments and shared references exist | Retain; no mutable tail pages or longer commit intervals to fill segments |
| Main overhead | Segment count initially suggested the priority | Use actual B-tree allocation: repeated change JSON, long-key indices and single-field links dominate |
| Change encoding | Thin batch JSON still repeats long identities | Store immutable context once; changes reference short keys and numeric columns; preserve unknown fields |
| Single-body deltas | A change row plus a separate range-link row | Inline the ordinary range in the compact change; use extra links for multi-field facts |
| Read path | Bounded readers/transfers/cooperative decoding exist | Retain; batch page context/link reads, avoiding N+1 queries or reparsing context per row |
| Version policy | Earlier proposal introduced per-run codecs for the old candidate | Remove that compatibility layer: the candidate was never released; replace its target layout directly, without draft migration, dual writing or dual decoding |
| Historical migration | Previous passing results exist | Rerun released Agent/Desktop/import/backup paths as blocking acceptance requirements |
| Space acceptance | Synthetic and runtime observations exist | Preserve records, add independent same-workload measurements, and report database/WAL/indices/write latency separately |

The old plan's implementation chronology and completed/default-off statements are archived history, not the current revision's runtime status.
Previous green tests do not establish correctness of new context ownership, collection or historical migrations.

## 3. Evidence and limits of the estimate

Read-only allocation for the new test conversation: `stream_changes` 660 KiB, segments 104 KiB, tool lookup index 88 KiB, and content links/their reverse index 56/40 KiB.
Its 1542 segments share SQLite pages; multiplying segment count by page size is invalid. Bodies total 66,086 bytes, with 1730 change rows.

The in-memory layout model retains all 1730 changes, original timestamp strings and unknown JSON, with semantic metadata equality after reconstruction.
Including candidate indices, compared metadata structures decrease from 921,600 to 385,024 bytes (58.2%); 1522 increments use direct ranges and 70 field links remain in the extra structure.
This model implements neither RPC/recovery/migration/collection nor WAL/CPU/UI measurements. It is not a whole-database saving or release result.
See the [baseline ledger](stream-storage-space-baselines.md). Final automation uses synthetic workloads, never private dialogue text or conversation IDs committed as fixtures.

## 4. Release and historical boundaries

| Source | Required treatment |
|---|---|
| Released Agent schemas 2–7 | Frozen original fixtures; supported upgrade into the final layout. Existing conversations remain legacy format 1; do not convert their bodies/events |
| Released JSONL/event sources and skills-only bootstrap | Preserve import/admission rules; imported history remains legacy, including skipped/retried/deleted identity behavior |
| Released Desktop layouts and contracts 1–3 | Preserve released SQL/checksums, identities and transaction boundaries; converge on the final target contract |
| Empty database / new conversation | Create the final layout directly; new conversations use logical format 2 |
| Previous unpublished schema-8/format-2 development database | No upgrade, compatibility decoder or conversion; reject a mismatching layout explicitly and preserve its files |

The current unpublished Agent schema-8, Desktop contract-4 and logical format-2 targets may be edited directly. Do not introduce schema 9 or per-run dual codecs solely to support the draft.
Confirm final numbering against the release ledger; never rewrite released versions, fixtures or migration checksums.

The same development `user_version=8` does not prove layout equality. The final database records a layout marker and validates required tables/columns/foreign keys/indices. Reject a nonempty mismatching development layout before initialization, WAL configuration or table creation.
Do not create empty tables/fake compatibility columns, delete the database, reset its home or report successful migration to hide incompatibility.
Development databases containing real history remain preserved and readable with the original candidate. Validate the new layout in an isolated root or from supported released-version backup copies; never automatically clear user data.

The prohibition on historical space rewriting does not remove necessary released JSONL import, lossless GUI archiving or backup operations.
Those existing contracts still run in order and preserve bytes; this optimization must not add historical compaction or startup body reorganization.

## 5. Target data model

Names below describe logical contracts. M1 freezes exact DDL, indices, codes and constraints; experimental SQL is not a production schema.

| Object | Content and constraints |
|---|---|
| Sessions/runs | Existing database identity, restore generation, conversation format, epoch/order/state and durable watermark |
| Immutable context | Stable message/tool/block/content identities and necessary type/field-presence information; short integer ID; reuse equality, append a version when it changes |
| Compact change | Stream/context identity, first/last cursor, explicit event code, timestamp, flags, required body range and residual fields |
| Multi-field links | Field/encoding/body range for low-frequency semantic facts; ordinary single-field increments avoid a duplicate link row |
| Content objects/segments | Existing lengths, sealed digests, immutable UTF-8 segments and shared references; no duplicate model summary or final streamed body |
| Display index | Necessary order/linkage/thin metadata only; no body copy or whole-history cache population on paged reads |

Context scope includes conversation, run, epoch and stable fields. Equal text keys cannot merge different messages/runs.
Event and known-field codes have frozen explicit values, independent of Rust enum ordering. Context/body IDs do not grant cross-session RPC access.
Unknown events/blocks/multi-field extensions need a generic representation, not truncation, silent dropping or guessed defaults.

Context identity checks its complete semantic key/descriptor; a digest can accelerate lookup but cannot replace collision verification.
Long external IDs live in context or authoritative message metadata; hot changes and indices use short keys.

## 6. Lossless encoding and stable protocol

- Extract fixed fields only from an explicit whitelist. Preserve residual fields in bounded extension JSON/body references.
- Distinguish absent, explicit null, empty string/array and false with stable presence encoding.
- Initially retain original timestamp strings. A later integer representation must preserve precision and necessary representation information, not truncate to milliseconds.
- Preserve provider signatures, opaque content and original JSON strings at their required byte fidelity. Ordinary metadata semantic equality is a different contract.
- Keep required reasoning signatures, tool error/interruption flags, multimodal attachments, unknown extensions, approval decisions and billing facts even if UI does not show them.
- Production omits only diagnostic redundancy established by the retention matrix, such as distributions or duplicated audit payloads. Bounded debug settings do not change recovery authority.
- Preserve external events, keys, field paths, cursors and range semantics through the storage adapter. Use the generic extension path or an explicit error for unrepresentable data, never an empty-history fallback.

Existing `segmented_content_v1` may remain when its semantics are unchanged. Database admission, logical conversation format and wire capabilities are separate concerns.
Every local/remote consumer still needs verification; prototype JSON equality alone is not RPC compatibility evidence.

## 7. Writes, transactions and crash consistency

Retain the dedicated writer, bounded queues, independent live path and critical boundaries. Context lookup/encoding occurs in persistence work, outside long broadcaster/session critical sections.
Merge same-context contiguous increments within a batch. Do not merge/reorder across message, tool, epoch or semantic boundaries.

One publication transaction atomically includes new context, required segments/ranges, compact changes, extra links and durable watermark.
Large bodies may stage across bounded commits, but cannot advance the watermark before their published references are complete. Retries validate logical identity, offsets, body bytes and complete field sets.
Do not conceal differences with ignored uniqueness conflicts or newly allocated short IDs.

Context caches have count/byte limits; misses use indexed lookup. Only committed IDs may be reused by later batches.
Invalidate IDs allocated by rolled-back work. Restart resolves committed context identities; cache contents are not durable authority.

| Boundary | Preserved contract |
|---|---|
| User acceptance | Input and necessary run/request identity are committed; request IDs support idempotent retry |
| Tools/approval/external effects | Necessary intent/authorization is committed; a persistence failure stops subsequent effects for the whole run |
| Displayed, uncommitted content | A crash may lose the tail; restore the committed prefix and mark interruption |
| Commit with lost ACK | Retry does not duplicate bytes; conflicting retries fail explicitly |
| Terminal acknowledgement | Required body, transcript terminal, completion event and watermark have consistent commit evidence |
| Unknown external execution result | Do not automatically repeat side effects or promise general exactly-once execution |

Finalization still seals current blocks and commits terminal state, without rewriting context, building a whole-run snapshot or filling historical bodies.

## 8. Budgets, scheduling and reads

| Item | Retained baseline |
|---|---|
| New-format scheduling | Approximately 25 ms, 128 events or 64 KiB; deadline from oldest pending data |
| Segment/write/raw read page | 16 KiB / 64 KiB / 256 KiB |
| Pending capacity | 32 MiB per session, 128 MiB global, 2-second age threshold; independent controls |
| SQLite | WAL + FULL, dedicated writer, bounded query-only readers, independent PASSIVE checkpoint |
| Clients | Bounded native transfers, cooperative large JSON decode, generation cancellation and decoded-object propagation |

Scheduling parameters are neither crash-loss limits nor arbitrary-disk latency guarantees. Do not lower durability, extend commit intervals, queue without limits or block cancellation to improve space figures.

Read bounded compact rows, then fetch the page's distinct contexts and extra fields in batches. Query count scales with pages, not events; parse/assemble outside database workers.
Context and residual JSON count toward page/memory budgets; bounding change-row count alone is insufficient.
Capture identity/generation/epoch/watermark in the read snapshot; atomically attach or deduplicate overlap using content offsets. Do not combine epochs or history generations.

Indices cover stream cursors, context/message/tool lookup, billing and collection. M1 provides actual queries/EXPLAIN evidence and removes unused long-JSON-key indices from the new target layout.
Released legacy queries still require their indices. Prove lookup/deletion cannot fall back to scanning history before removing any index.

## 9. Ownership, collection and consumers

Contexts/direct ranges are new body-reference paths and must participate in ownership and collection.
Publish context, changes, content and ownership atomically. A foreign key alone is insufficient if GC ignores the new owner and deletes still-referenced bytes.
Only explicit reference removal schedules bounded idle collection. Freeze reverse indices/deletion triggers with the contract and isolate shared parent/child views.

Cover history, tool arguments/results, model/provider input, full-text/original-range recall, titles/sidebar, fork, reset, context checkpoints, export, attachments, tombstones/outbox, queued receipts and backup/restore.
Hide unpublished fork/operation state on failure. Startup handles necessary unfinished runs and explicit staging owners, without sweeping completed history.

Desktop, mobile, TUI, CLI, channels, host and remote share semantics; embedded and independent Agent need acceptance.
Ephemeral mode drains/detaches persistence before prompt admission, bounds memory replay and never presents volatile cursors as durable facts.
Existing explicit legacy-journal maintenance remains unscheduled at startup/completion and cannot process format-2 bodies.

## 10. Historical migration: mandatory rerun matrix

Run the whole matrix after implementation and record fresh source revisions, results and fault evidence.
This is the requested migration revalidation; previous green results cannot substitute for it.

| Source/fault | Required evidence |
|---|---|
| Released Agent schemas 2, 3, 4, 5, 6, 7, each frozen DDL | Real open/upgrade into the target; original messages/blocks/events/snapshots/pricing/skills/metadata and identity preserved. Continue old conversations as legacy; create new compact format-2 conversations |
| Empty Agent and supported skills-only bootstrap | Correct initialization; foreign applications, forged versions and unsupported development layouts rejected before schema writes |
| JSONL and separate event sources | Import, skipped reports, explicit retry, repeated startup, changing source, deletion tombstones and GUI binding; no overwrite of successful/deleted identities |
| Desktop pre-agent, v1.2.1, CLI settings/workspace bootstrap and contracts 1–3 | Released step order/checksums, lossless archive, FK rebuild, unique binding, source ownership and exact M0 deletion scope; preserve business data/attachment ownership |
| Nonempty source, WAL and pre-upgrade snapshots | Snapshot includes committed WAL, verifies and never overwrites; backup failure prevents upgrade; original snapshot remains readable |
| Migration commit boundaries/kill/full disk | Current step atomic; committed earlier steps retain their original contract. Restart retries safely without claiming whole-chain rollback |
| Reopen and damaged current structure | No repeated migration/cleanup/backup; missing authority/FKs/identity/critical indices produce explicit failure, not empty-table repair |
| Actual post-upgrade use | History, original recall, tool detail, model continuation, fork/reset, queued receipts, cancel/terminal, delete and reconnect all work |
| Coordinated backup/restore/interruption | Both databases/managed files agree; identity retained appropriately, restore generation rotates and old cursors fail. Verify/resume/rollback and each file-switch failure boundary pass |
| Unpublished old candidate | Require rejection and file preservation only; no upgrade support or addition to the released-schema list |

Anchors: `agent/src/session/database/historical_upgrade_tests.rs`, frozen `agent/tests/fixtures/database/v2.sql` through `v7.sql`, legacy_import, `host/src/store/migrations.rs`/fixtures and CLI data tests.
Never synthesize old fixtures by changing current DDL's version header. Extend existing fixtures to prove continuation/new-old coexistence/failure retry, not just version numbers or empty database creation.

## 11. Stages and exit criteria

| Stage | Work | Exit criterion |
|---|---|---|
| M0: retained cleanup | Retire three unused caches, duplicate review index and dead code | Fresh omission; historical migration re-proves rollback/retry and retained useful data |
| M1: freeze contract | This review, retention matrix, exact DDL/codes/uniqueness/ownership/query plans/budgets/workloads | Unpublished replacement boundary explicit; every consumer/failure boundary has acceptance coverage |
| M2: replace draft layout | Contexts, compact changes, direct ranges/extra links; admission marker and legacy dispatch | No previous-draft codec; reject old development shapes; released migrations reach the final target |
| M3: persistence | Bounded cache, idempotency, transactions, watermarks, backpressure, controls/checkpoint | IDs/references do not publish ahead of content; failure does not alter acknowledgement promises |
| M4: readers/lifecycle | Batched pages, protocol reconstruction, all clients/context, shared ownership, collection/backup | No N+1/body copies; lossless fields; fork/reset/delete/recovery cover the new reference paths |
| M5: migration/validation preparation | Frozen-DDL matrix, fault workloads/benchmarks, released-chain audit | Every supported source/failure boundary has explicit tests; previous results are not new acceptance |
| M6: unified acceptance/handoff | After implementation, local static/unit/integration/migration/fault/performance checks, CI and manual/device checks | Data-boundary defects block handoff; report all baselines/results/limits before manual testing |

Wire after M1 freezes. Do not enable a release candidate before M4/M5 finish. No tests during implementation; static compilation is allowed. Run tests together afterward and repair/retest failures in final verification.
This document task runs `make check-docs`, which is not runtime/migration verification.

## 12. Acceptance and durable space records

- Field round-trip: codes/presence/null/empty/Unicode, provider raw strings, unknown extensions and multi-field facts; bounded pages/reconnect preserve overlap without missing/duplicate content.
- Crash matrix: kill around context/body/direct-range/terminal/watermark commits, lock/full disk/writer panic/lost ACK/rolled-back cached IDs; retain committed-prefix and side-effect promises.
- Migration matrix: rerun every row in section 10 and retain results. Validate both old and new conversations after upgrade, not only fresh-database tests.
- Sharing/collection: context-only/direct-range references, deleting a parent while a child remains, reset retirement and staged recovery cannot collect live bytes or leak another session's content.
- Performance: same machine/build/workload; 1/10 MiB bodies, small/large tools, 1/4/16 conversations plus history reads. Include high reuse and rapidly changing contexts/unknown-field-heavy worst cases.
- Real-workload evidence: the [historical context audit](../testing/stream-context-history-audit-2026-10-27.json) covers 27 conversations, 59 prompts and 439 assistant messages. Blocks/message have median 2, p95 3 and maximum 6. Tool calls/turn have median 2 and maximum 150; three turns have at least 100. In the latest four turns, represented fragments per thinking/tool-input/text body have medians 91/110/597. Normal acceptance must cover repeated fragments, a few blocks/message and long tool chains spanning messages. Four fragments/key and thousands of keys in one unpublished message remain boundary stress, not a typical-workload proxy or justification for unnecessary architecture complexity. The 52 aggregated snapshot runs lack fragment counts; do not mix them with the two complete raw runs. Legacy block IDs are reused, so count thinking_start lifecycles.
- Separately measure B-tree pages/payload for bodies/changes/links/contexts/each index; DB/WAL peaks, write volume, CPU/RSS, queue/transaction/COMMIT p95/p99/max, reads/first-visible latency, frames/input/cancel and finalization. In-memory page counts are not WAL write volume.
- For successful performance workloads, sample `SQLITE_DBSTATUS_CACHE_WRITE` before/after on the same writer connection. Report page deltas and logical page bytes (delta times page_size) separately from WAL file length; these are not physical device I/O, fsync counts or checkpoint writes. Do not use the undefined post-error counter for fault workloads. See [SQLite status-counter definitions](https://sqlite.org/c3ref/c_dbstatus_options.html).
- Do not identify a new/old worktree build solely from Cargo's up-to-date result in a shared target directory. Use isolated targets or freeze and verify binary hashes, test inventories and source records. One run that reused the old binary was identified and explicitly invalidated; it is excluded from new-version acceptance.
- Retain previous targets: normal-disk backlog p99 100 ms; Desktop incremental update p95 16.7 ms/mobile 33 ms; first-visible p95 regression no greater than 10%; approximately constant finalization work at fixed tail load for 1/10 MiB. Confirm on frozen devices/workloads, not from batch period.
- No space gains from longer commit intervals, weaker FULL, lost fields/recovery, final compaction or VACUUM. Prove latency/WAL do not materially regress alongside space savings. Retain the previous approximately 31 ms 3000-row-table rendering limitation rather than claiming every render meets one frame.

Preserve B0 original survey, B1a/B1b previous synthetic samples, B1c actual-layout audit and B2 in-memory model separately.
New implementation adds B3 and a new raw file with commit/schema/layout/toolchain/workload/page size/WAL/FULL/checkpoint/sampling/interference/repetitions/statistics.
Never overwrite or selectively replace older figures. Explain differing workloads, database-plus-WAL and table allocations separately.
Where the original personal survey retained rounded figures only, label missing raw byte counts rather than inventing precision.

M0 still removes only `workspace_files`, `reference_targets`, `object_references`, their indices and duplicate `idx_review_snapshots_run`.
Other apparently redundant history structures need read/write and migration evidence; an unpublished draft does not authorize deleting released historical sources.

## 13. Final verification entry points

Existing migration/storage entry points below run only after the new implementation is complete. They do not replace the full affected-crate, frontend or CI matrix.
Update names here if M1 adds/renames modules. Use the pinned toolchain and temporary synthetic databases.

```sh
cargo test -p future-agent --lib historical_upgrade_tests
cargo test -p future-host --lib store::migrations::tests
cargo test -p future-cli commands::data::tests
cargo test -p future-agent --lib acceptance_tests -- --ignored --nocapture --test-threads=1
```

Migration evidence identifies each source version, failure point, snapshot validation, reopen and post-upgrade use separately.
A total test count or correct target version does not replace that evidence.

## 14. Business identities: 16-byte storage, standard UUID output

User decision: new generated identities omit time and use the complete UUID v4. Store 16-byte values and expose canonical lowercase 36-character UUIDs; no Base64URL or packed timestamps. SQLite uses constrained BLOBs; time stays in record fields. A future PostgreSQL adapter can use native uuid; multiple backends are outside this implementation.

- Model generated session/message/run identities separately from opaque provider IDs and local integer keys. Preserve provider values. Generated approval/operation identities also omit classification prefixes; their owning module controls storage adaptation.
- Preserve and return released historical identities exactly, including time-prefixed IDs and UUID-looking TEXT. Never truncate suffixes, infer type from string length, batch-convert on startup or rewrite historical foreign keys.
- The typed storage boundary handles UUID BLOBs versus historical TEXT. Decode actual storage classes and preserve legacy writer semantics.
- Integrate primary/foreign-key bindings, indexed lookups, JSON projections, fork/reset, deletion, queued receipts and reopen together. A single converted column or symbol-table BLOB test is not completion of business identity storage.
- Store business keys as 16-byte UUIDs, intern stable per-stream values and retain integer references for high-frequency changes. Audit parent duplication in content keys and body owners structurally; removing prefixes cannot break client deduplication or recovery.
- Supply read-only inspection views using built-in SQLite SQL, rendering the same UUID as the API without another persisted text copy.

Compatibility boundary: the 36-character output guarantee applies to newly generated UUIDs. Historical API IDs and opaque provider IDs keep their original values. A historical non-UUID cannot become a UUID while retaining its original value; this implementation does not add a historical alias catalog. Historical conversations retain their existing write contract; new conversations use the new format from their first write.

In SQLite, equally spelled TEXT and UUID BLOB values are different index keys. Resolve the session's actual storage representation and bind consistent types for child primary/foreign keys. Parsing an API string alone cannot determine the historical key type; casting whole columns must not hide mismatches. If historical UUID-looking TEXT and its corresponding UUID BLOB create an ambiguous public identity, reject that ambiguity explicitly and preserve the database for recovery.

For a future backend change, the new UUID branch can map to PostgreSQL's native uuid type. Historical non-UUID strings still require a separate compatibility field or mapping; the mixed historical column cannot simply be altered to uuid. This implementation does not rewrite history for a future backend.

Capacity estimates concern the identity value alone: changing a UTF-8 36-character UUID to 16 bytes saves 20 bytes per occurrence (about 55.6%); measure total tables, indexes and WAL separately. Encoding is a fixed-size boundary conversion and adds no historical scan, compression job or volatile uncommitted identity mapping. Durable acknowledgment and crash-recovery boundaries remain as specified above.

Progress: session/identity.rs connects session/message/run keys, primary/foreign-key bindings and the main read/write lifecycle. Real UUID tests inspect storage classes/lengths and foreign-key integrity for sessions, entries, runs, stream_runs, body ownership, fingerprints and history projections. Coverage includes replay, range authorization, terminal retries, tools, search, sidebar/pages, fork sharing, reset, coalesced-input cleanup, checkpoint receipts, deletion and reopen. Unscoped new-format entries allocate UUID runs and reuse committed ownership on retry instead of composing history:session:message; historical rules remain.

The added Agent 2–7 matrix retains UUID-looking historical TEXT, raw JSON and pre-upgrade snapshots while new BLOB sessions coexist. Session resolution includes tombstones/import registrations; old JSONL sources cannot overwrite canonical owners or resurrect deleted identities. Recovery of 35 binary hidden-fork identities crosses a page boundary and preserves shared parent bodies.

After forced rebuilding and test-inventory validation, the complete Agent pass has 2686 passing tests, no failures and nine explicit acceptances not run. Fmt, all-target Clippy and documentation checks pass. Candidate 2 reduces high-churn stream metadata 24.9% but increases logical WAL page writes 47.8%, so delivery remains withheld. Normalize body owners, then rerun final migration, fault and performance acceptance against the same completed source.
Acceptance must create real UUID sessions and inspect actual storage class/length and references before exercising the complete lifecycle. Green fixtures using only s/m do not establish native UUID integration.

Follow-up progress: staged body ownership now uses integer `(stream_id,key_id)` and shares the semantic symbol dictionary; encoded `[run,epoch,key]` owners are not persisted. Retirement and unpublished cleanup use bounded integer queries, and collection protects both staging owners and published ranges. The content module has 96 passing tests; 13 native-identity-related tests pass, including the released historical migration matrix, and all-target Clippy passes. These targeted checks do not replace final acceptance.
The single intermediate [B3.3 raw measurement](../testing/compact-storage-candidate-3-2026-10-27.json) reduces the high-churn database file to 7,933,952 bytes but logical WAL page writes remain about 38.7% above the old implementation. Continue investigating dictionary/context and index write amplification without widening confirmation windows, weakening durability or adding completion-time compression. Final migration, fault, repeated performance and UI acceptance must precede main-workspace integration.

Current implementation follow-up: redundant composite identity indices are replaced by single-column foreign keys and primary-key ownership/kind guards, with exact trigger admission checks. Semantic facts remain distinct inside bounded atomic batches; encoder body dependencies flush explicitly, and no pending batch survives acknowledgement. Transaction-borrowed owner handles and captured physical snapshot keys avoid repeated UUID resolution without caching aborted allocations. Metadata budget counting avoids temporary JSON strings, while publication still checks actual bytes. Historical-parent forks now stage a native-UUID format-2 child under parent revision/database fences; historical parents are read, never rewritten. Targeted content checks pass 101 tests and the fork checks pass 49; final full-suite/migration/fault/performance validation must still be completed. One earlier parallel full-suite run had an elapsed-time failure in healing_rewrite_preserves_run_markers; its isolated rerun passed, so retain that failed run and rerun the final suite with bounded concurrency.
