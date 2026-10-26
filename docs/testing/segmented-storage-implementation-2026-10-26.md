# Segmented storage implementation record — 2026-10-26

Status: M1–M5 wired; M6 final local verification/repair in progress. Not yet a manual acceptance candidate.
Approved scope: [development plan](../architecture/stream-storage-development.zh-CN.md), M1–M6 in full.

The user explicitly requested no tests during implementation. After all components
are implemented, run the affected local test suites, then hand off a runnable
candidate and manual acceptance steps. Do not treat the foundations as completion.

## Current working state

- Task worktree: `.worktrees/segmented-storage`, branch `refactor/segmented-storage`.
- Task HEAD: `91410a8120ec142de610662306ef6bcae4c5ad09` (merge of current remote main).
- Implementation changes are uncommitted in this worktree. Main integration
  worktree remains unchanged. Do not remove the worktree or merge this candidate.
- Goal remains active; there is no external blocker.

## Implemented paths, not yet an acceptance candidate

- Additive Agent schema 8, shared RPC schema constant, frozen actual schema-7
  fixture. Existing authoritative identity/queue/tombstone checks apply before
  upgrading schema 7; no historical-body conversion.
- Canonical `content_objects`, bounded immutable UTF-8 segments, scoped ownership,
  sealed-body reuse, verified semantic JSON digest for argument whitespace, and
  transactional stream changes/watermarks.
- Entry body staging in bounded writer jobs and `resolved_message_blocks` adapter;
  pure tools/recall/transcript reads use separate query-only connections. Format-2
  selection APIs exist but are deliberately not enabled in production yet.
- Explicit-reference-removal collection queue: at most four segments per idle
  transaction, shared-body protection, no whole-database scan or run-end cleanup.
- Incremental live projection with a 16 KiB mutable tail and shared closed segments.
  Snapshot body cloning/materialization happens after the broadcaster lock is released.
- The model loop now assigns a stable assistant entry ID before its first delta,
  emits format-2-only message start/end boundaries, and carries exact ordered
  content keys through the save callback. Text/reasoning final entries prefer
  their scoped stream objects, rather than an unrelated equal digest. Tool result
  IDs originate at the broadcast sequencing point before result persistence.
- Canonical producer stamps content identity and UTF-8 offsets at the broadcast
  sequencer. A format-aware writer actor packs adjacent fragments before segment
  insertion, coalesces change ranges, stages large facts in bounded jobs, seals
  individual blocks and verifies final JSON argument identity. Message-end seals
  remaining open blocks in batches of at most 128 metadata operations. New data still is
  not enabled by default.
- Canonical replay materializes scoped ranges on query-only readers. Shared Rust
  and TypeScript overlap cursors, host collector/observer, remote coalescing,
  remote replay coverage and shared Desktop/mobile projectors have initial range
  support. Consumer and fault acceptance remain incomplete.
- Canonical admission is nonblocking under broadcaster locks and retains byte
  reservations through commit/retry: 32 MiB per session, 128 MiB globally, two
  seconds maximum admitted backlog age. Queue overflow interrupts explicitly;
  callbacks wait for semantic receipts outside locks. Canonical writer window is
  25 ms, legacy remains 100 ms. These are implementation starting values, unmeasured.
- Flush/recovery/shutdown waits now occur outside journal locks. Canonical ring
  uses a deque with a 4 MiB byte budget; live projection has a conservative 8 MiB
  admission budget and explicit paged-replay fallback. Transport ring and async
  runtime blocking still need audit (see next work).
- Canonical entry fingerprint checks do not hydrate old bodies on the writer.
  New display-index updates append only; bounded page reads join run annotations.
  Healthy display-page reads use query-only workers.
- Shell facts are canonical JSON; stream and entry body references can render the
  shared `future_rpc::shell_result::model_summary` instead of storing that output
  again. A pure SQLite function is registered on readers/writer for compatibility
  views. No rewrite of legacy shell history.
- Release approval persistence drops diagnostics only after full decision
  computation. Debug builds retain them. Missing diagnostic fields use the JSON
  wire carrier; Desktop diagnostic type fields are optional.
- Production compactor startup/scheduling is removed for both formats, historical
  snapshot readers retained. Legacy compaction builders are test-only fixtures.
  Canonical terminal commit failure no longer triggers full-history rewrite.
- Initial startup recovery walks only active format-2 run metadata in bounded
  semantic pages, reuses published ranges under the exact original message/result
  IDs, restores reasoning signatures and complete tool arguments, publishes partial
  history and marks the run interrupted. Unpublished staged bytes are excluded.
  Terminal entry and recovered agent_end boundary commit together. Unsigned partial
  reasoning is kept intact for display and lowered to marked text for future model
  requests; existing runtime tool-repair provides unknown-result placeholders.
  Recovery is not yet fault-verified and tool/result/identity cases need audit.
- Initial canonical fork path reads body-free header pages, pins/shares references
  in batches of 64 headers, and publishes revision 0 only after all writes. A
  request receipt with revision -1 identifies interrupted hidden staging for
  explicit retry cleanup. Parent revision is checked before/after reads and every
  write; same-database fork requests serialize across Manager handles. More fault,
  deletion and restart auditing is required before enabling.
- Authored, unexecuted specification tests cover UTF-8 ranges, retry conflicts,
  rollback, sharing/deletion, canonical transcript and shell round trips, verified
  JSON identity, semantic watermarks, reader/writer isolation and overlap cursors.

## Additional implementation in the current continuation

- Semantic JSON hashes now recursively sort object keys while streaming values
  into SHA-256; raw bytes remain immutable. Whitespace and key order can share
  a verified argument object without rewriting its original bytes.
- Canonical queued receipts stage user bodies once and store thin entry headers;
  the unused `payload_json` request copy is NULL for format 2. Accepted/coalesced
  execution retires unpublished old receipt owners and requests bounded GC.
  The legacy raw-event insertion API now explicitly rejects canonical sessions.
- Added `get_event_content_range`, proto `ContentRangeRequest`/RpcCommand field
  191, host bridge, shared command deadline/read-only registry and remote route.
  Authority includes session/run, database ID, restore generation, epoch, exact
  published change cursor and JSON field pointer. Every body page re-resolves
  these under the SAME read transaction as the body read; recycled IDs cannot
  authorize another body. ETags describe the immutable published range and do
  not change when a growing object is subsequently sealed.
- Canonical live facts are previewed before broadcaster locks. Live fragments
  split at UTF-8 boundaries into <=4 KiB pieces; fact frames stay <=64 KiB.
  Oversized fields carry explicit `content_refs`; tool status remains a small
  authoritative `is_error` boolean. Aggregate-budget omitted fields, even below
  the per-field threshold, are also staged by the encoder. Argument preview has
  a 2 KiB cumulative budget. No large JSON serialization in the sequencer.
- Canonical replay hydrates only bounded fragments/small facts. Large facts use
  the same scoped references. Source replay stops after 8 MiB plus ONE real
  lookahead event before constructing the full tail; unlimited oversized reads
  fail explicitly so RPC callers must page. Snapshot RPC maps that to its
  existing size fallback. Atomic attach/resume is connected in the continuation
  recorded below; its acceptance specifications remain unexecuted.
- Canonical writer consumes borrowed pending events instead of cloning its
  complete batch, and clones/parses each body only once in the encoder.
- Added shared Rust/TypeScript scoped field readers with UTF-8 byte progress,
  restore/ETag/encoding/length checks, 64 KiB pages, yielding, and an explicit
  16 MiB materialization budget. Detail page iterators support larger bodies.
  Required semantic fields are hydrated for user messages, approval requests,
  errors, terminal errors and compaction summary/error events. Tool facts stay
  lazy; their complete scoped detail APIs remain available.
- CLI/TUI, native host observer/collector/full replay, mobile serial lanes and
  Desktop remote live/replay paths now use required-field hydration. Observer
  wire mirroring keeps bounded original events. Desktop remote orders required
  reads before later live events with a bounded queue and durable recovery on
  failure/overflow. Consumer/epoch/capability acceptance remains incomplete.
- Added optional replay watermark to EventsSince (proto field 6), optional
  RpcCommand `replay_until_idx` field 192, source watermark capture, native page
  window pinning and remote FIRST-page source pagination. Modern remote reads
  no longer drain the full Agent tail just to determine their watermark.
- Authored additional unexecuted specs for replay watermark pinning/regression,
  large fact references, scoped reads,
  restore isolation, UTF-8 edges, prefix ETag stability, aggregate wire budgets,
  replay memory ceilings, semantic hydration and serial event ordering.

## Paged attachment and recovery continuation

- `RunAttachment` now optionally carries a captured canonical backfill boundary.
  Registration and capture occur under the stamping lock, with no initial full
  history materialization. The gRPC stream walks exact-epoch pages on bounded
  blocking jobs, sends normal event frames, then polls the already-registered
  live receiver. Native/remote consumers therefore keep their existing wire.
- A batch can coalesce across the produced attachment boundary. The backfill
  emits that complete range, advances its live cutoff to the stored range end,
  and filters the receiver's covered source fragments. No arbitrary byte split
  or silent cursor skip. Gaps/deletion/epoch mismatches fail explicitly.
- Canonical `start_run_with_sequence` reads at most the last eight changes,
  preserving the durable next cursor while marking a partial projection
  incomplete. It does not decode an old full run. Snapshot RPC returns its
  existing `run_snapshot_unavailable` fallback for incomplete prefixes. A run
  ID cannot silently change epoch; restarted execution needs a fresh identity.
- Added unexecuted broadcaster and gRPC specs for >8 MiB initial prefixes,
  short-tail restoration, subscription-before-read/live ordering, coalesced
  boundary extension and epoch identity refusal.
- Startup recovery now gives persisted `run_stop_requested` precedence over
  restart. The terminal entry and synthetic replay terminal both say cancelled
  with `user_stop`, instead of later disagreeing after marker reconciliation.
- Unfinished tool arguments recover as referenced text, never executable tool
  calls. Explicit provider snapshots replace the unfinished prefix in its
  original chronological slot. Completed tool arguments replace that same slot
  as JSON (fixed the old Text encoding bug); provider metadata and tool error
  flags survive, including empty error results. The first tool's omitted
  `tc_index` uses the released default of zero.
- Added unexecuted recovery specs for stop precedence/idempotency, incomplete
  parameters, block ordering/typed JSON/provider metadata/error results and
  replacement snapshots. Normal fault-terminal persistence already orders all
  save callbacks before terminal acknowledgement (`SessionPersistence::CommitRun`
  refuses after any prior append failure), so a failed save leaves the run active
  for recovery. Verify this with final fault tests before deciding a separate
  terminal-recovery intent table is necessary.

## Canonical reset/reference publication continuation

- Added durable `content_operations` receipts for explicitly staged resets and
  a separate `retired_streams` queue. These have active production paths; they
  are not new diagnostic history. Startup inspects only operation receipts,
  never scans historical bodies. A process-local active-operation guard keeps
  another Manager opening the same DB from reclaiming a live stage.
- Canonical reset captures revision, plans and writes the lossless backup off
  the writer, maps retained block ordinals, pins byte-identical source ranges,
  then prepares thin headers under checkpoint owners. Unsealed recovered prefixes
  are pinned directly; they are not copied, rehashed or sealed with staged tails.
- Publication checks revision again, atomically replaces entry owners/headers,
  and marks affected streams unreadable. Fixed the shared planner to include
  changed SURVIVING entries in affected_runs (previously only removed entries
  could retire their stale replay). Shifted text/argument ordinals cannot read
  old reasoning references after a reset.
- Staging-owner cleanup examines/deletes at most 64 references per writer job,
  leaves a receipt on failure, supports startup cancellation, and preserves
  published owners. Reader/replay/range lookups exclude retired streams
  immediately. Idle retirement drops at most 64 body links, 64 link-free
  changes and 64 stream-owner references per slice; segment GC remains bounded.
  Explicit prune now also retires canonical streams. No normal run-end pruning.
- Reset publication currently has one atomic metadata transaction proportional
  to retained header/reference count; it never reads/hashes/parses full bodies on
  the writer. Measure large-header fairness in the final perf pass rather than
  claiming this metadata transaction is constant-time.
- Reset backups now use owner-only temporary files, streamed JSON serialization,
  file sync, no-clobber publication and Unix directory sync before destructive
  publication. The original portable backup format/selectors remain supported.
- A separate history_reset_in_progress fence is set under the session write
  guard, excluding accepted queued work, new prompts and delete admission.
  Prior persistence/journal queues drain outside locks before planning. The fence
  is RAII-released on success/failure. A successful reset clears the broadcaster's
  old in-memory run/projection so reset content cannot survive in its replay cache.
- Queue receipt admission also checks the durable reset receipt transactionally.
  Existing legacy history keeps its original reset read/plan compatibility path.
- Authored (not executed) reset specs cover ordinal rebinding/body reuse, portable
  backups, changed revisions, crash-stage cleanup, another Manager's live stage,
  unsealed recovered prefixes, publication rollback, bounded retirement, immediate
  replay invalidation and prompt/delete/queued-work admission.
- Startup recovery also releases unpublished fact staging in 64-owner pages,
  scoped only to the interrupted run/epoch (including pre-first-change staging).
  Published changes/recovered entries retain their references. An unexecuted
  specification proves another run's staging is not swept.

## Async durability and control admission checkpoint

- Added `broadcaster/journal_control.rs`: a separately bounded 64-slot control
  lane carries each barrier's last successfully admitted body sequence. The
  dedicated actor selects controls without polling; it consumes through that
  captured sequence before flushing/acknowledging. Controls cannot overtake body,
  nor compete with the 4,096 body slots. Only one control is held outside its queue.
  Body and control senders use crossbeam-channel (already in the locked dependency
  graph; now an explicit Agent dependency). Shutdown closes admission first and
  drains through the same sequence fence.
- Canonical model events now use `broadcast_deferred`; live publication and
  bounded admission no longer synchronously wait for a durable receipt on the
  model's Tokio worker. `StreamContext.durability_barrier` awaits a Tokio oneshot
  at MessageEnd (before any assistant save), tool intent (before execution), after
  the approval gate (before effects), and tool completion (before result save).
  Interrupted intent fences recheck stop before starting a tool. Fences themselves
  retain no run/journal lock and cancellation does not discard admitted bodies.
- Async run finalization fences trailing source events before the terminal
  transcript commit. Terminal events are admitted without sync waits and the
  final drain awaits the independent control lane. Legacy source callbacks retain
  old admission behavior; do not claim legacy lock waits are eliminated.
- Canonical save callbacks refuse to publish whole in-memory bodies after stream
  health failure. Recovery remains authoritative for the committed prefix.
- The sync compatibility approval hook yields its Tokio worker with block_in_place
  on a multithread runtime, retaining tool task-locals. Manual approval no longer
  waits for an unsaved request or returns approved after a failed decision commit.
  This hook is still a synchronous compatibility interface, not a fully async
  approval subsystem. Shell escalation/sandbox notifier contexts remain to audit.
- Authored (NOT RUN): a current-thread async fence with a completely full body
  lane, cancelled fence retaining its body, failure of intent/approval durability
  preventing a filesystem effect, and stop during intent preventing tool start.
  Existing direct writer fixtures updated for the sequence/control contract.
- Static Agent/host/CLI/TUI library compilation passed after the implementation;
  formatting and diff whitespace passed after specification edits. No test body
  was compiled or executed in this checkpoint.

The settled-run crash gap has now been addressed by the atomic terminal path
below; its authored specifications remain unexecuted. Large terminal metadata
and fault publication still require audit before format 2 is enabled.

## Atomic terminal, capability and observability checkpoint

- Explicit `SessionPersistence::CommitRun` now routes through a dedicated Manager
  commit and `content/terminal.rs`. Format-2 terminal transcript entries, derived
  runs state and an `agent_end` stream change commit in ONE transaction. Repeated
  entry identity reuses the same boundary. Import/ordinary append/fork/reset do
  not invoke this synthesis. Active-run recovery still has its own atomic path.
- After commit, the async run task reads/publishes that exact persisted terminal;
  it never enqueues a second end or emits a new error event ahead of that cursor.
  `RunState::record_event` shares bounded ring/projection insertion. Canonical
  explicit settle also publishes its terminal in the same transaction. Legacy
  user-stop reconciliation excludes format 2, whose startup recovery already
  preserves user-stop precedence with replay in one transaction.
- `stream::commit_batch` refuses new changes or staged growth after a terminal,
  while allowing identical already-committed retries. Missing stream owners are
  now checked against terminal state before staging (see checkpoint below).
  Normal source finalization still fences all prior writer work.
- Authored NOT RUN: terminal stream trigger failure rolls back transcript/status;
  retry has one terminal; restart after commit but before live publication retains
  completed replay; post-terminal append refuses; live publication reads exactly
  the committed event without another actor commit.
- Remote `segmented_content_v1` is now declared by BOTH Rust peer and TS clients,
  advertised by the server, recorded per connection/pairing, and cleared on
  replacement transports. Scoped command context reaches the host adapter. An old
  peer gets metadata-only `get_session_storage_format` preflight for content reads,
  prompt/continue/fork/compaction: existing legacy remains available; canonical or
  prospective unsupported new format returns `client_upgrade_required`. Stop,
  approval decision and delete remain available. Invalid/unavailable preflight
  refuses explicitly instead of guessing. Internal test adapter calls without a
  remote scope retain their previous context.
- Every canonical live/replay event carries coordinator-owned `content_format:2`;
  RPC typed fallback preserves that marker, including small usage/user facts.
  Unsupported live peers receive one explicit error terminal per cached run and
  no referenced content. The per-connection refusal cache is capped at 64, resets
  on reconnect, and conveys no original body. Full publisher reconnect/role tests
  still need final verification. No declaration is inherited by another peer.
- New format creation is still OFF: `NEW_SESSION_FORMAT` is 1 and reserve_segmented
  remains unconnected. Change that value consistently with actual creation, only
  after every consumer and recovery/read audit is complete.
- Pricing now selects usage/model changes from canonical stream metadata using
  a partial pricing index. It runs on the read pool for both formats, without raw
  event duplication or body hydration. Authored NOT RUN canonical pricing spec.
- Added fixed-size in-memory batch timing buckets/counters, batch size maxima and
  pending age/bytes/global-budget snapshots to get_runtime_metrics. Duration is
  actor batch/staging/commit time, NOT an individual fsync/SQLite transaction time.
  Counts include attempted retries. It does not measure UI frames or WAL size.
  Added typed RPC field 12 for the metrics JSON and regenerated future.proto via
  `REGENERATE_PROTO=1 cargo check -p future-rpc --lib`; no tests were run.
- Rust Agent/host/CLI/TUI/remote server/client library compilation and shared remote
  TS checking passed during this checkpoint. Formatting/spec edits followed; run
  the final static source check below. No unit/integration/perf/fault tests ran.

Terminal errors and large truncation metadata are now externalized/shared by the
large-extension checkpoint below; its authored specifications have not run.
Configuration metadata and bounded readers remain incomplete. Thin final
assistant callbacks are now connected below but still need final verification
before format 2 is enabled.

## Large extension/body checkpoint

- Added `content/extensions.rs`: large provider metadata, signatures, opaque JSON,
  image/data URL fields, unknown block fields and entry metadata are staged through
  the existing bounded immutable store. JSON pointer escaping preserves `/` and
  `~` in namespaced keys. String and JSON encodings retain their original types.
  Scalar/array unknown blocks can also be referenced as `/content/<ordinal>`.
  Known text/argument bodies still use their existing authoritative columns/ranges.
- Extension staging leaves typed empty JSON placeholders so provider_metadata stays
  a map and attachments stay arrays while body-free shape/index code inspects it.
  The compatibility views restore block extensions and entry extensions with a pure
  `future_restore_content_json` UDF, installed on every writer/read connection.
  Block text vs JSON-valued unknown text/content fields remain distinguished.
- Explicit terminal errors now stage a single text body (with a bounded 512-char
  preview for derived runs metadata). The run commit consumes prepared thin
  terminal headers and publishes `/data/error` by linking the same range as the
  transcript `/content/error` in the SAME transaction. Large truncation/cause
  extensions are shared too. Required-body readers already hydrate terminal refs.
- Thinking/tool-input recovery no longer reads/decodes a large provider metadata
  body and copies it into a header. It binds the exact committed JSON range into
  the recovered entry, preserving signatures/opaque state by scope. Tool slot
  replacement preserves the provider range after clearing unfinished argument links.
- Reset pinning now maps both nested block references and whole scalar blocks;
  non-array content fields keep their exact path. Prelinked JSON placeholders keep
  their source shape, avoiding redundant restaging/hash work for retained bodies.
- New stream fact staging checks terminal state before creating a missing owner;
  late facts cannot leave a new orphan owner before commit_batch refuses a closed
  run. Existing sealed-owner retries still validate identity without rewriting.
- Header views now reuse only relational assembly and DO NOT run the body-restore
  select. This preserves typed sentinels/previews for fork/index/control paths.
  Canonical index rebuild uses that body-free view. Canonical run admission/resume
  reads at most the latest start/stop/terminal headers, not all prior run markers.
  Explicit settle checks only its indexed target terminal; full error reads use
  the reader pool for one target terminal. Removed the unused whole-marker helper.
- Fixed the display-page run annotation join to read run_id from the entries table
  (the entry_records compatibility view does not expose that column).
- Authored NOT RUN: large signatures/opaque JSON/image URLs/unknown blocks and
  escaped keys round-trip with thin headers and bounded segments; a large terminal
  error has one shared body with paged required replay; large provider state
  recovery shares its original body instead of copying it into block metadata.
- Agent/host/CLI/TUI library compilation and formatting passed during this slice;
  no SQL fixtures, unit/integration, performance, fault or UI tests ran.

Remaining related work (do not enable format 2 yet):

1. `session_info` control metadata is deliberately excluded from the generic
   extension stager for now. Its current-metadata merge/update paths bypass body
   preparation; moving its fields now would lose refs on rename/settings writes.
   Audit/bound those paths or connect lossless reference-aware updates before
   treating arbitrary large configuration fields as supported. This is still
   incomplete, not a final storage-policy exemption.
2. Normal entry/header compatibility reads still use full `entry_records` SQL
   body aggregation on read workers. Finish paged canonical header/ref/body reads
   with reset/delete/rebind identity fencing and cooperative budgets. Display
   history currently materializes a full entry before projection/fragmentation;
   large invisible provider metadata should not be loaded for a display page.
3. Source-bound thin assistant callbacks are now connected (checkpoint below).
   Verify their identity/partial-argument/metadata contract in the final pass.
   Finish the remaining tool representation budget and model-block assembly audit;
   preserve model/review context and exact provider state without body guesses.
4. Finish fork's final full load, export/backup/context ownership and remaining
   client render/transport audits; then metrics/workloads, full default switch and
   the single final local test/perf/fault pass. No manual candidate yet.

## Source-bound assistant persistence checkpoint

- Added `content/stream_entry.rs`: canonical assistant callbacks build only a thin
  SessionEntry plus typed Rust-only selectors. The recipe holds ordered IDs,
  text byte counts and argument/provider shape flags; it never clones streamed
  text, arguments or provider payloads. It requires committed `message_end` in
  the exact session/run/epoch/entry, checks text ownership/sealed lengths, resolves
  semantic end/start records, links original ranges in <=64-reference jobs, then
  rechecks the source boundary and publishes the header. No final text hash or
  body serialization/staging is performed by this path.
- Added bounded expression indexes for source lookup: message/tool boundaries,
  reasoning-end keys, and tool input identities. They are PARTIAL indexes;
  ordinary text/reasoning delta rows are not included. Static matching predicates
  preserve use of those indexes without scanning token ranges. Query-plan and
  scaling specifications still need final execution; do not claim performance.
- The canonical save callback no longer clones complete messages twice into a
  second mid-run shared context. It enqueues the typed recipe for assistants;
  bounded tool/placeholders use a compact converter with one ordered content
  serialization and no repeated `thinking`, tool_call list or tool_args copies.
  Review evidence still observes the original full in-memory message.
- The run loop now wraps an inner borrowed-context executor. On success its owned
  Vec returns normally; on error an optional callback transfers that Vec to the
  session without body clones. Healthy canonical failures therefore retain the
  same generated model context even though mid-run persistence no longer copies
  messages into shared memory. Unhealthy journal state stays recovery-owned.
  Old context is dropped after releasing the shared messages lock.
- Canonical durability fences now asynchronously drain both the source journal
  and the FIFO transcript worker (blocking work is offloaded), so a failed source
  recipe/header prevents a following external tool effect. Normal terminal commit
  still refuses after a prior transcript failure; no canonical full-history heal.
- Tool strings have an explicit `tool_args_string_v1` ENTRY reference encoding.
  Raw committed input can be shared, while the compatibility reader derives the
  exact same repaired-string JSON value as the model. Moved the existing pure
  repair behavior to `future_rpc::tool_arguments::repair_string_v1`; source and
  reader share that version. It does not rewrite/copy a repaired body in SQLite.
  Empty raw ranges now read as empty strings instead of SQL NULL. Stream RPCs
  explicitly refuse this entry-only encoding rather than misinterpreting it.
- Canonical missing tool IDs are assigned at source before broadcasting and
  model accumulation; final argument-string normalization also precedes source
  publication. ToolInputStart now preserves provider metadata on canonical wire.
  Complete and unfinished tool fields bind by source identity; repeated IDs in
  different assistant entries are scoped separately. Ambiguous duplicate tool IDs
  within ONE assistant message fail closed before execution instead of guessing.
- Authored NOT RUN: large text/provider fields create no extra body when saving
  the assistant; wrong run/epoch/uncommitted end refuses; unfinished argument
  strings share the raw body and retain the model value; failed run context is
  transferred with the same underlying text allocation (no body clone).
- Agent/host/CLI/TUI library compilation passed during the integration. Specs were
  only formatted, not compiled/run. Existing repair specs still call small
  test-only wrappers of the shared behavior. No unit/integration/perf/fault or UI
  tests have run, and new-format creation remains OFF.

Next related audit: constructor/transport callbacks still assemble the in-memory
model blocks from accumulator strings, which is model-context work rather than a
persistence snapshot. Measure it in the final UI/CPU workload, and move ownership
where practical. Non-streamed tool results are capped/model-derived and still use
compact owned entries plus canonical reuse; finish their representation/memory
budget audit. Session_info reference-aware updates, page-based canonical body
readers, display-only field selection, fork final loading, export/backup/context
ownership, metrics/workloads, default switch and final acceptance remain required.

## Paged transcript and targeted detail readers (latest implementation)

- Added `content/reader.rs`: captured database/restore identity, explicit history
  rewrite generation and append-only end position. Header/block/reference identity
  pages hold at most 64 rows; raw column and content pages return at most 256 KiB.
  JSON decoding, extension restoration, argument-string repair and shell-summary
  derivation run in the requesting blocking task, outside both reader workers.
- Every body page checks the exact session/entry position/ID/field/range/encoding
  within the same read transaction as the byte read. Empty ranges check ownership
  too. Scope checks refuse tombstones, database restores and history rewrites.
  Appending new messages does not invalidate a captured immutable prefix. Explicit
  rewrite/reset advances the additive `content_history_versions` metadata fence;
  it does not scan or convert any historical body.
- Canonical `typed_entries` and complete terminal reads use this reader. A full
  history API still necessarily materializes its requested result; bounded DB jobs
  do NOT imply a bounded full-session return value. Fork's final load still needs
  return-contract audit even though it now uses paged IO.
- Canonical display queries select only thin index annotations and entry locators,
  with 64-row/256-KiB read-job limits. They no longer access `entry_records` or
  resolved body views. Body assembly is lazy per selected row; page and index
  identity are captured in one transaction to prevent combining pre-reset
  ordinals with post-reset bodies. Canonical projection borrows ordered blocks
  and avoids the old complete entry serialize/parse round-trip and legacy mirror
  hydration. Full display still preserves all existing fields; invisible-field
  selection and first-load CPU/memory budgets remain unfinished.
- Oversized display JSON spills once to a buffered anonymous temporary file,
  hashing the same stream. A canonical derived fragment cache reuses it on
  consecutive requests, reading at most 1 MiB per fragment. The cache is limited
  to 16 files/128 MiB, with lazy 120-second idle expiry and LRU eviction. These are
  starting budgets, NOT measured acceptance numbers. It is disposable derived
  output, has no fsync or durable log/ingest role, and losing it loses no session
  data. Entries exceeding cache admission still work but may be recomputed;
  concurrent/evicted requests and total materialization memory still need audit.
- Render cache keys include database/restore/history generation, session and entry
  identities plus captured run/index annotations. Normal scope validation still
  runs before returning bytes. Changes during a fragmented exchange cause the
  existing checksum/identity contract to reject mixed versions rather than accept
  a partially mixed message. No cache token grants raw body access.
- Added targeted canonical tool field reads and small tool identity queries.
  Tool listing, one call's arguments and one result/facts no longer aggregate
  resolved compatibility bodies in SQL or decode full JSON in a reader worker.
  Each body field still has exact entry/field authorization. Tool listing caps a
  metadata job at 64 rows; response-wide transport/memory budgets and original
  text search remain separate unfinished work.
- Canonical original-text `read_history_entry` now pages block/ref identities and
  slices ordinary text and raw JSON argument ranges before transfer. It retains
  concatenated UTF-8 offsets, exact block identities, omitted-kind reporting and
  scoped ownership without loading unrelated signatures. Inline values remain
  thin; shell summaries and repaired argument-string representations derive
  outside SQLite workers. Those derived representations can still materialize
  and repeat work across requests; their cache/budget audit remains required.
- Corrected staged reference encoding: JSON arguments/extensions/facts are bound
  as JSON, raw strings as text, and semantic facts use their actual wire type.
  Reusing a field with a conflicting encoding fails before mutation. A generic
  retry cannot extend an already-published unsealed recovered prefix. Otherwise
  a failed retry could mutate a committed field before its eventual header
  conflict, violating the reader/cache immutability assumptions. Per-piece
  transactions repeat the publication check and advance reference lengths with
  max(existing, next): an identical concurrent retry with an older staging offset
  cannot temporarily shrink a reference that another producer already published.
  Missing/deleted/rebound owners fail before appending another segment.
- Header-only block assembly preserves JSON-valued text placeholders rather than
  manufacturing string fields over them. Removed the unreferenced
  header_content_references view creation; only its retirement DROP remains.
- Authored NOT RUN: lossless canonical/compatibility equivalence (including
  signatures, images, scalar blocks and escaped field names); reading after the
  compatibility views are removed; append-prefix vs reset/restore/delete fences;
  precise reference rebinding refusal; reads while an unrelated writer is held;
  UTF-8 fragment serialization/hash/LRU/expiry; fragment cache loss and rebuilding;
  canonical tool isolation/large facts and original-text range crossing without
  loading a damaged unrelated signature; encoding conflict and published
  recovered-prefix protection; late identical pieces cannot shrink published
  ranges, and publication between staging pieces prevents extension atomically.
- Latest Agent/host/CLI/TUI library compilation passed. No test bodies have been
  compiled or executed in this phase. Default format-2 creation remains OFF.

Next concrete scope: canonical original-text search, derived recall caching and
summaries; compaction receipt/header guards;
remaining ownership, client/render and global-budget audits; workload/fault
instrumentation; only then enable new sessions and begin the final acceptance
phase. No external blocker or manual candidate exists yet.

## Reference-aware metadata and small run boundaries (latest implementation)

- Removed the temporary session_info exclusion from extension staging. Large
  system prompts, source metadata and unknown config values now use scoped
  immutable references; required control strings stay inline. Metadata header
  admission starts at 128 top-level fields, 64 KiB of thin JSON and 4 KiB per
  inline control string. These are initial implementation budgets, not measured
  acceptance results. Rejection reports an error rather than truncating data.
- Added content/metadata.rs patch publication. It stages only supplied values
  under a durable checkpoint-owner receipt, then merges the CURRENT thin metadata
  inside the publication transaction. Unchanged large fields transfer ownership
  without reading, serializing or hashing them. A large -> small/null replacement
  releases the old owner; generic session_info replacement also retires its prior
  owner's references atomically instead of leaking one owner per update.
- content_operations now supports independent concurrent metadata receipts as
  well as exclusive reset receipts. Its session index serves exclusion/deletion;
  reset checks all in-flight receipts and metadata checks active reset. Cleanup
  validates kind/UUID/exact prefix, removes at most 64 staged references per job,
  and is recoverable after crash. In-process active guards protect a live stage
  from another Manager's recovery. It never scans historical bodies.
- Added metadata_generation to the additive per-session rewrite-fence row.
  Session-info/model changes advance it in their transaction; ordinary message
  appends do not. Full configuration reads check it during and after assembly,
  preserving config consistency without invalidating reads for unrelated appends.
- Canonical runtime hydration now pages thin message headers and restores only
  the actual configuration bodies needed by the model/runtime. Canonical fork
  header loading uses the same bounded reader. Complete display session_info
  hydration also restores its referenced fields so the existing additive public
  metadata contract never silently returns a placeholder as the real value.
  Display-only config selection remains a separate contract/performance task.
- Frequent get_state polling no longer uses load_metadata for canonical sessions:
  counts/lineage come from small indexed queries, interrupted-state checks use
  latest run markers and an explicitly requested terminal loads only that entry.
  Parent lineage itself reads its inline scalar on a query-only worker. Historical
  message bodies and large config blobs are not loaded by normal state polling.
- Canonical run completion no longer calls Manager.load to preserve lineage or
  latest_session_info_content to merge frozen config. The source callback omits
  frozen system prompts/provenance from its terminal snapshot, and both Manager
  and Store select only run-owned counters/automatic title before serializing.
  The metadata/terminal receipt prepares bounded fields outside the writer, then
  publishes new metadata ownership, terminal header, runs state and agent_end in
  ONE transaction. Existing explicit title/settings win over the frozen run;
  a frozen explicit title is not resubmitted as an automatic title after a clear.
- A closed-run retry compares staged/published range tuples plus the existing
  immutable header and terminal identity. It returns without changing counters
  or transcript revision, so retrying an older run cannot overwrite newer run
  usage. A conflicting terminal rolls back metadata and terminal together.
- Added duplicate-field intent checks before generic existing-entry staging.
  Removed referenced fields, empty replacements of nonempty referenced text and
  small JSON values replacing an externalized JSON placeholder must validate
  actual intent, not only thin-header equality. JSON semantic digests avoid full
  body reads when available; rare non-whole/unsigned cases use scoped paged reads.
  Source-bound streamed assistant publication keeps its dedicated source contract.
- Authored NOT RUN: full config round-trip and body sharing after a rename;
  released owners and null replacement; concurrent independent large patches;
  abandoned/live receipt recovery; metadata/terminal rollback; old terminal retry
  after a newer run; runtime metadata and finalization with an unreadable unrelated
  historical body; clear-vs-placeholder conflicts; independent metadata generation.
- Agent/host/CLI/TUI library compilation passed during integration. Tests remain
  uncompiled/unexecuted as requested; no perf/fault/manual acceptance has run.

Known required follow-ups, BEFORE enabling format 2:

The former summary/search, checkpoint and fork gaps below were superseded by
sections 20–22 of the architecture plan and the latest implementation record.
They are connected in source, with authored specifications only. Derived recall
representations, enormous-first-message title processing, display field selection,
global concurrency/budgets and end-to-end consumer/fault/performance acceptance
remain unfinished. No tests have run.

## Verification so far

Only formatting, source inspection, `git diff --check` and library compilation:

```sh
rustup run 1.99.1 cargo fmt -p future-agent -p future-rpc -p future-host -p future-remote-server
RUSTC_WRAPPER= CARGO_TARGET_DIR=/Volumes/ZHITAI/Workspace/future-os/target rustup run 1.99.1 cargo check -p future-agent -p future-host --lib
npm run typecheck --workspace desktop
```

Latest Agent/host/CLI/TUI library compilation and Desktop, mobile and shared
remote-client TypeScript checking passed. These checks compile source only; no
test bodies were executed. The unused
`reserve_segmented` API remains intentionally unconnected until consumer/recovery
work is complete; do not suppress its warning. Some subsequent test/doc edits have
only been formatted. No unit, integration, performance or fault tests have run.

Worktree-local ignored `node_modules` links reuse root installed dependencies but
point `@future-os/*` at THIS worktree. Without them TypeScript resolves packages in
the main checkout and checks the wrong source.

## Required next work

1. Verify the now-connected native/remote capability and metrics contracts, then
   finish deterministic workloads. Scoped range/window contracts and field
   retention are documented but still require acceptance verification. Do not
   claim current numeric budgets as measured.
2. Verify the new pre-stream message/ordered block/result identity contract across
   provider retries, omitted ends, repeated provider IDs and parallel tools. Author
   exact partial/repeated-text specifications. Ensure verified JSON semantic digests
   canonicalize object key ordering consistently. Keys now sort in the hash;
   the authored specification has not been executed. No body ownership guesses from text/timestamps.
3. Audit/finish partial recovery: user-stop precedence, complete/incomplete argument
   blocks, opaque extensions and signatures, tool errors/empty results, crashed
   staging retries, active-run epoch/watermark scope and fault-terminal paths. Current
   startup selection is status=running. Audit the callback/terminal FIFO guarantee
   above with fault tests; if a terminal can bypass unpublished callbacks, add a
   scoped recovery intent rather than scanning old failed/interrupted history.
   Avoid automatic reruns of uncertain external side effects.
4. Complete queued-receipt/coalescing/deletion specifications and audit every
   remaining canonical write bypass. Thin canonical receipts and mixed raw-journal
   refusal are implemented, but recovery and ownership have not been verified.
5. Finish all consumers (native CLI/TUI/channels, remote wire trimming/capping,
   bootstrap, overlap/cursor/epoch/restore-generation transitions). The shared
   projectors cover several clients but do not prove end-to-end completeness.
6. Bound transport/render work and total live memory, including tokio broadcast
   rings with oversized tools. Canonical journal jobs and replay body reads are
   bounded. Paged atomic attach and short-tail start_run are implemented, but
   their authored specs are unexecuted. Full native replay aggregation, shell-summary derivation and
   detail/history/export materialization still need memory/CPU audit. End snapshot
   and strip-repeated-args paths need audit.
7. Audit the newly connected async durability/control contract above, especially
   shell escalation/sandbox notifier contexts and terminal transcript/replay
   atomicity. Legacy append still has original locked wait semantics. Cancellation,
   control fairness and storage health/backlog transitions need fault coverage and
   metrics; the new authored specs have not run.
8. Complete fork/reset/delete/export/backup/context ownership. Audit hidden fork
   publication/retry and failed parent revisions, mixed legacy/canonical forks,
   content reference reclamation, tombstones and cancellation. Canonical fork
   implementation is not yet verified.
9. Fix repeated diagnostic documentation and author the remaining specification,
   perf/fault harnesses and operations/manual steps. Enable new-session reservation
   only after ALL consumers and recovery are connected. Production remains legacy.
10. When every component is implemented, run the final affected local suites and
    required static gates, repair failures, produce the runnable manual candidate,
    then finish-lock integration/push. Do not stop at the current foundations.

The full approved objective is not achieved, there is no external blocker, and
this branch must remain unmerged while the above implementation work continues.

## Next concrete integration boundary

Finish asynchronous durability/control admission and capability negotiation,
then the remaining content/read/ownership audit before enabling new sessions.
Canonical model producers now defer receipts and await semantic fences; the
independent bounded control lane carries a body sequence target. Verify this
contract, including cancellation, actor failure, controls under full body pressure
and tool intent/approval durability. Save callbacks now refuse a failed canonical
stream. Preserve durable/produced watermark honesty and audit terminal stream
visibility after its transcript commit.

Canonical reset/reference publication and paged full-transcript readers are now
implemented with authored, unexecuted specs. Canonical display and tool details
also use thin locators plus paged body fields. Canonical recall/search, summaries, configuration patches and checkpoint guards
now have dedicated reader/staging paths. Derived recall, huge-title processing,
display-only field selection and total concurrency/budgets still need work.
Do not silently drop provider/attachment data.

Canonical fork RPC now returns identity without a final full-body load; the
compatibility create_fork API explicitly hydrates for callers requesting a Session.
Selected checkpoint identity fields are remapped before sharing other fields. Remote capability negotiation and old-client refusal are now connected but
unverified; do not enable new-session reservation before the remaining audits.
There is no external blocker, and no tests have run.

Latest static checkpoint: `cargo check -p future-agent -p future-host -p future-cli
-p future-tui --lib` passed (only intentionally unused `reserve_segmented`);
Desktop/mobile/shared remote-client TS checking passed. `git diff --check` passed.
No tests, benchmarks, fault injections, manual acceptance, commit or integration
merge has run for M1–M6. Work remains uncommitted on the task branch at the HEAD
listed above; the main integration checkout was verified clean.


## Latest fork, checkpoint, entry-stage and search integration

- Canonical fork captures immutable prefix identity; only selected current
  checkpoint identity fields are hydrated/remapped. Unchanged body references
  share storage, changed checkpoint fields get new bodies. RPC uses identity-only
  completion. Indexed hidden-fork recovery uses bounded ownership/header slices
  and shared process-active guards. Fork specs were authored, not run.
- Context-checkpoint admission uses four indexed dialogue-kind tails plus entry
  identity and rewrite generation; no historical body hash on the writer.
  Successful receipts reference/digest-check the checkpoint rather than duplicate
  its summary. Failed errors use scoped body references. Checkpoint/header/body
  ownership and successful receipt publish atomically. Existing failure/started
  semantics do not silently repeat model work. Checkpoint specs were authored,
  not run, including injected publication failure and immutable-ID conflicts.
- Generic entry preparation now returns a lease-bearing PreparedEntries value.
  Callers take headers while retaining leases OUTSIDE the database worker until
  commit returns. Durable content_operations(kind=entry) and content_entry_stages
  identify exact entry/run ownership. Failure/crash cleanup checks indexed entry
  identity, competing leases and the exact queued receipt before deleting at most
  64 unowned references per transaction. Reset excludes all these stages. Queued
  acceptance checks only reset, so it does not refuse its own preparation lease.
  Tests cover failed publication, abandoned/live/concurrent preparation, queued
  ownership and restart, authored but not run.
- Requested-run collision checks use entries_run in canonical sessions instead
  of Manager.load. New-format meta rejects non-object/non-null envelopes before
  run ownership could overwrite opaque data; legacy behavior is unchanged.
- Original-text search uses 64-entry locator pages and 256 KiB body reads, with
  query overlap PLUS 120-byte snippet lookbehind. Matching/snippet work is outside
  DB workers. Exact tool IDs, ASCII-insensitive literal matches and original byte
  offsets remain. Cross-session search handles mixed formats in independently
  scheduled reads and caps candidates between sessions. Oracle/parity/edge specs
  are authored, not run.
- Sidebar summaries now query thin metadata, timestamps/counts and the selected
  first-user text. They skip large configuration, image/protocol fields and other
  dialogue. Existing title derivation still materializes that selected text; this
  is an OPEN memory/CPU issue for enormous first messages, not a bounded-title
  claim. Metadata generation fences the whole summary. Irrelevant-body-damage
  and full-load-equivalence specs are authored, not run.

Latest completed source checks: Agent/host/CLI/TUI library compilation passed
including the final search-lookbehind/meta-guard edits; formatting and
`git diff --check` passed. `make check-docs` checked all 349 documents, bilingual
pairs, links and provider matrix successfully. Only the intentionally unused
reserve_segmented warning remains. No tests/spec bodies were compiled/executed,
no performance or fault workloads ran, no default switch, commit, integration
merge or push. Main integration checkout remains clean at e4d2a9a4.

Next concrete work: derived-recall representations and display-only selection;
streamed title preview/global admission; then remaining consumers/ownership and
acceptance harnesses. Finish all implementation before the single final local
test phase. Full M1–M6 goal remains active, production still creates legacy data.


## Latest paged-title, derived-cache, admission and CLI integration

- Agent title generation uses a paged 4 KiB extractor for the shared markdown
  grammar and 40 visible columns, with a 1 KiB zero-width output cap. Canonical
  summaries page their selected text reference directly. Full-message title
  expansion is removed. Long valid/invalid link lookahead can still scan source;
  equivalence and Unicode-page specs are authored, not run.
- Canonical recall/search caches shell-summary/repaired-argument representations
  as anonymous files. Identity/range/encoding/version keys and exact reference
  authorization precede cache service and every fragment. SQL owns original
  bytes; losing cache files cannot lose data. Repaired JSON validation now uses
  a non-retaining serde visitor (including finite numbers / recursion limits).
- Derivation work is process-bounded to two jobs / 256 MiB charged at source*16
  plus 64 KiB, at most 32 blocking waiters for five seconds. Builds happen outside
  DB workers. Budget excess is explicit, not truncation. Disposable files have
  an additional process-wide actual-write reservation: 64 files / 512 MiB,
  including Arcs retained after cache eviction. Cache budgets remain 16 items /
  128 MiB and lazy 120-second expiry. Initial estimates await final profiling.
- gRPC read admission happens asynchronously before blocking-pool dispatch:
  heavy commands=2, pages=4, stream backfill=2; each has 32 waiting slots.
  Cancellation cannot release a still-running job's permit. A slow subscription
  retains its backfill-page permit until page drain or drop. Controls/live event
  delivery bypass those lanes. Cancellation/ownership specs are authored, not run.
- Canonical display reads skip only fields the existing public projection
  discards: unknown root envelope fields and extra fields on known blocks.
  Unknown blocks, public provider metadata, attachments, shell facts and public
  metadata/checkpoints remain complete. Full/context/export reads retain ALL
  source fields. Projection-equivalence with damaged irrelevant-body specs exists.
- Canonical unpaged get_session_entries returns history_paging_required.
  CLI full-history now follows forward cursors; forward/tail calls verify and
  assemble fragments with future_rpc::history::read_page. Existing host/TUI
  callers already use that contract. CLI mixed-fragment/cursor and server refusal
  specs are authored, not run. Full result assembly is explicitly client-owned.

Remaining implementation: finish consumer/ownership audits and acceptance/fault
harnesses, including request/render allocations beyond these read-job/cache gates.
Do not treat the new gates as proof of total model/client memory or UI latency.
No tests have run; new-format creation remains disabled, work is uncommitted.


Latest static result after this integration: Agent/host/CLI/TUI library checks
passed on toolchain 1.99.1, with only intentionally unused reserve_segmented.
Formatting, diff whitespace checks and all 349 docs/provider-matrix checks passed.
No tests/spec bodies were compiled or executed. Main remains clean at e4d2a9a4;
task HEAD remains 91410a8120ec142de610662306ef6bcae4c5ad09 with uncommitted work.

Next concrete audit items: retire obsolete failed-checkpoint error owners/receipts
after explicit canonical rewrites without scanning bodies; remaining full model
context/initial_messages clones; client fragment/render allocations and slow
transport ownership; WAL/fsync observability and deterministic acceptance/fault
workloads. The paged title/derived recall/display selection/native CLI gaps above
are now connected in source, and should not be reimplemented on continuation.
Finish all implementation, then run the final local suites and acceptance gates,
repair, integrate/push dev by the finish-lock flow and hand off manual testing.


## Latest receipt-retirement, model-input and WAL integration

- Canonical checkpoint_generations and retired_checkpoint_generations track
  explicit rewrite ownership. Claim records generation atomically; fail checks
  it before staging and again before publication. Rewrite advances a watermark,
  not a history-body sweep. Idle slices remove one indexed old receipt / at most
  64 exact error-owner fields; shared/current/legacy data remain protected.
  Recovery/rollback/bounded-cleanup specs are authored, not run.
- Each LLM request now owns its projected work_messages rather than cloning
  them. ConvertToLLM for logging runs only when verbose is enabled. First-run
  context copying is still a full snapshot, now off the ServerSession command
  lock and Tokio workers. Admission captures count/tail identity; a child-work-
  owned blocking job checks that prefix and cancellation, clones it, then seeds
  the run's reviewer before any model/tool callback executes. Reviewer source,
  expected policy generation and cancellation remain frozen at admission.
- Writer WAL + FULL remains, wal_autocheckpoint=0. A separately owned connection/
  thread runs PASSIVE checkpoints every 250 ms. Idle reclamation no longer runs
  TRUNCATE on the ordered writer. Background failure does not lose durable WAL;
  long readers can still prevent checkpoint progress/grow the file. The 8 MiB
  journal_size_limit is not a hard WAL bound. Shutdown joins the worker.
- Fixed memory database metrics cover queue/job timings, helper COMMIT elapsed
  (including FULL sync, not pure xSync), checkpoint/frame progress, and sampled
  file sizes/peak. No SQL/path/content/per-token persistence. Storage-health
  counts now read on independent readers and append IO observations separately.
- DurableTransaction delegates transaction access but measures its consumed
  commit. No durability settings were weakened. Metrics are scoped to this
  Database worker; legacy direct transactions are included in job duration but
  not necessarily the helper COMMIT histogram.
- Authored (unexecuted) specs cover stale checkpoint failures, generations and
  shared bytes, 64-field cleanup, deferred input identity/cancellation, reviewer
  source preservation, a long-lived reader while new FULL commits continue, and
  deterministic checkpoint observations without test sleep polling.

Latest source-only library checks passed for Agent/host/CLI/TUI (reserve_segmented
is intentionally still unused); no test bodies compiled/executed. Main remains
clean. Formatting/diff checks passed before the final docs/metric-label edit;
refresh those source checks before ending this checkpoint.

Next concrete work: production new-session reservation wiring and final opt-in
switch (only after remaining source paths are connected); client fragment and
large JSON decode/render scheduling, using existing SyncEngine frame budgets
rather than adding another per-token queue; acceptance/fault/performance harness.
Existing cooperativeJson.ts explicitly yields only between native UTF-8 decode
and JSON.parse; the large native parse is NOT preemptible. Its prior custom-
parser wall-time result does not establish UI latency, so measure the right
criterion in final acceptance. Do not reimplement already-connected receipt,
context-preparation or WAL work, and do not run tests mid-implementation.


Verification refreshed after all source/metric-label edits above: Agent/host/CLI/
TUI --lib checks passed on toolchain 1.99.1 (only the intentionally unused new-
session reservation API warning). Formatting/diff checks and 349-document bilingual/
link/provider-matrix checks passed. Tests, benchmark/fault runs and manual checks
have not run. NEW_SESSION_FORMAT remains 1. Task HEAD is still 91410a8120ec142de610662306ef6bcae4c5ad09;
all implementation is uncommitted on refactor/segmented-storage. Main integration
checkout remains clean at e4d2a9a4. The complete M1–M6 goal remains active.

## Latest client decode, native history transfer and creation wiring

This checkpoint supersedes the previous next-work note: the native-only
cooperativeJson implementation has been replaced, production reservation wiring
is connected, and local Desktop history no longer returns the assembled large
JSON object in a single IPC response. NEW_SESSION_FORMAT is still 1.

- Shared thread-projection cooperativeJson composes large arrays/objects in
  yielding slices, decodes UTF-8 in 64 KiB blocks, and validates approximately
  8 KiB string fragments natively. Small JSON still uses JSON.parse. Native own
  property/duplicate-key/Unicode semantics are retained. Admission is two active
  decodes, estimated 256 MiB charges and 32 waiters/5 seconds. Depth 128 and
  oversized primitive literal refusals are explicit. Final joins and allocation
  remain intrinsic synchronous work; final UI latency must be measured. Remote
  byte reads and JSON content fields use the shared decoder.
- Desktop native history now uses commands/history_transfer.rs and frontend
  integrations/storage/historyTransfer.ts. The backend acquires two-active/
  32-waiter admission BEFORE reading, then serializes on blocking workers with a
  hard 16 MiB output writer. A canceled worker result drops its lease, not an
  unreachable cache entry. Window-scoped immutable copies send only 64 KiB raw
  IPC chunks (also bounded in WebView byte-array fallback); final read removes
  the copy. Failure releases it, expired abandoned copies prune on access after
  120 seconds. Frontend admission covers read through decoding, two operations
  at a time. The hook passes source/request/older-history generation checks into
  the transport, stopping obsolete reads between chunks and decode slices.
  This is a disposable read cache, never a source of durable conversation data.
- bind_events now calls one transactional format-policy reservation before
  broadcaster construction. Existing placeholders and imported identities stay
  format 1; existing format 2 remains 2 regardless of the new default. Deleted
  identities/import receipts and skipped imports reject rebinding. Failure to
  enable format 2 rolls back the new placeholder. reserve_segmented is now a
  cfg(test) explicit-format fixture, no longer an unused production API.
- Authored, UNEXECUTED specs: forced cooperative/native parity, malformed JSON,
  yields/cancellation, UTF-8/BOM boundaries, depth/literal limits; new/old/imported/
  deleted identity decisions and transactional rollback; native IPC binary and
  fallback assembly, cancel/failure release, invalid sizes/bytes, window/range
  ownership, expiry permits, bounded JSON encoding. Existing Desktop paging
  specs retain cursor/source/error assertions through the new adapter.

Static verification after these edits: Agent/host/CLI/TUI --lib compilation;
Desktop GUI --lib compilation on toolchain 1.99.1; Desktop, mobile and shared
remote-client TypeScript checks; targeted Desktop/shared-package ESLint; Rust
formatting and diff checks; 349-document bilingual/link/provider-matrix checks.
No Rust test bodies compiled/executed, no TS tests executed, no performance/fault
workloads run. The initially missing Desktop parking_lot/uuid imports were fixed
using existing std mutex/nkeys dependencies; no dependency was added.

Next concrete work: required-event hydration still stringifies/reparses complete
materialized values (remote replay, shared content lane and native peer projection),
so pass parsed values through consumer projections before claiming UI completion.
Inspect ephemeral session mode: constructor binds the journal before the later
set_ephemeral command, while transcript/terminal commits skip ephemeral writes;
ensure live memory mode cannot leave canonical stream facts without transcript
publication before enabling format 2 by default. Desktop remote_peer_request still
reassembles a possibly large object in Rust before native IPC; extend the bounded
transport adapter or preserve fragments there as appropriate. Other explicit
detail/export readers must retain their bounds and ownership. Finish final
deterministic performance/fault harnesses, enable only new sessions, then run the
one final local verification phase and integrate/push dev for manual handoff.

Task HEAD remains 91410a8120ec142de610662306ef6bcae4c5ad09, all implementation
uncommitted. Main remains clean at e4d2a9a4d87f59af184e4fa592a581e0a73a4985.
No PR opened. No blocker; the complete approved M1–M6 goal remains active.

## Final local verification phase started (supersedes the no-tests checkpoints)

The implementation phase ended and the user-authorized final verification/repair
phase has begun. NEW_SESSION_FORMAT now equals FORMAT_VERSION (2) IN THE TASK
WORKTREE ONLY. It is not merged/installed. Do not revert the production default
or globally pin tests to format 1 merely to regain green results. Legacy raw-
journal/compaction specifications need explicit legacy fixtures; canonical and
production-default specifications must exercise format 2.

Source completed in this turn:

- eventPayload.ts uses a private process-local Symbol for decoded graphs, carried
  through normalization, spread and explicit envelope/projector adapters. Required
  body hydration no longer stringify/reparses complete graphs. Shared projectors,
  remote client and desktop/mobile consumers use the same parsed value. Legacy
  large inline events decode cooperatively. Cursor parsing rejects oversized
  canonical authority (>64 KiB) and avoids parsing arbitrary large legacy bodies.
  Required hydration now has a cumulative 16 MiB / 128-field bound.
- Ephemeral selection drains/joins the journal actor, then detaches store/writer
  while retaining bounded volatile delivery/projection. It is rejected after
  prompt admission/run start. Rebinding after volatile events is rejected to
  prevent reuse of uncommitted cursor numbers. A pre-event selection can be
  undone. set_ephemeral now returns Result; RPC reports failures and all current
  callers/specs handle the result. The service fixture no longer bypasses it by
  assigning the public field directly.
- Native remote_peer_request read operations share the native history transfer
  adapter. Backend/JS admit before read; <=32 KiB replies remain unchanged, larger
  ones use window-scoped immutable 64 KiB chunks. Controls bypass this read lane.
  Frontend source checks propagate into replay, history and required-body reads.
  Window command adapters are generic over Runtime for real IPC test coverage.
- Added explicit ignored content/acceptance_tests.rs workloads: 1/10 MiB legacy/
  canonical comparison, 1/4/16 shared-writer sessions with bounded read overlap,
  real child kill after confirmation/in an open transaction, SQLite max-page FULL,
  external write lock and actual writer-thread panic. All data are synthetic and
  temporary; reports distinguish batch times, sampled WAL/file peaks and actual
  process kill from live backlog/UI/RSS/power loss. Must explicitly run ignored
  workloads in the final phase; they have NOT executed yet.
- Added desktop/scripts/storage-read-benchmark.{html,ts} for real browser 1/10 MiB
  string/object decode, frame/timer/long-task observations; existing streaming-
  markdown-benchmark separately covers rendering. New benchmark passed TS source
  compilation, but neither browser workload has run yet. Playwright/puppeteer
  are not installed in this task node_modules; use available browser tooling or
  bundled dependencies when executing them after heavy suites finish.

Verified final-phase results so far:

- Shared packages Vitest: 26 files / 652 tests PASS, log
  /tmp/future-segmented-shared-tests.log. Includes new cooperative parser, decoded
  event carry/projection, cursor limits, cumulative hydration and lane tests.
- Shared remote-client Jest: 31 suites / 676 tests PASS, log
  /tmp/future-segmented-remote-client-tests.log. Fixed an actual runId regression:
  hydration now uses a separate fallback scope, preserving an empty wire runId.
  Updated the exact capability declaration expectation for segmented_content_v1.
  These include the preexisting fake-network streaming performance specification;
  they are not browser/mobile-device performance measurements.
- Desktop Vitest: 383 files / 5304 tests PASS after repair, log
  /tmp/future-segmented-desktop-tests.log. Initial failures were paging assertions
  needing the new cancellation callback and a new test's concise beforeEach
  returning invokeMock (Vitest treated that function as a cleanup and called it
  without args). Block-bodied beforeEach fixes the fixture; production transfer
  was not changed to accommodate the test. Focused repair also 28/28 PASS.
- Mobile Jest: 150 suites / 2888 tests PASS, log
  /tmp/future-segmented-mobile-tests.log. Linked ignored task mobile/node_modules
  to existing /Volumes/ZHITAI/Workspace/future-os/mobile/node_modules for Expo's
  localized preset/React. Initial missing preset and one SIGSEGV process occurred;
  listTests and isolated settings/controller tests passed, then the complete
  normal run passed. Do not leave this marked as an unresolved blocker.
- Agent/host/CLI/TUI/Desktop GUI library compilation, TS static checks, targeted
  ESLint, rustfmt/diff checks and 349-document gates passed before final-phase
  fixture fixes. Refresh affected static gates at final integration.

CURRENT LIVE PROCESS (must resume, not blindly restart):

- exec session 44667: `RUSTC_WRAPPER= CARGO_BUILD_JOBS=4 CARGO_TARGET_DIR=/Volumes/ZHITAI/Workspace/future-os/target rustup run 1.99.1 cargo test --workspace --exclude futureos --offline --no-fail-fast`, output redirected to
  /tmp/future-segmented-rust-tests.log. Last poll confirmed LIVE, progressing
  through future-agent's 2646 tests. Roughly 130 FAILED lines already exist;
  many are old raw-journal/compaction fixtures using bind_events and thus now
  selecting the new default. A child failure explicitly says "canonical
  conversation requires the semantic stream writer" at journal_compaction_tests.rs
  fixture append_events. This requires explicit legacy fixture selection, not
  weakening canonical refusal or the default.
- The test rpc::broadcaster::tests::writer::event_batch_worker_exits_when_its_last_sender_disappears
  has emitted an over-60-seconds warning. Its fixtures bind_events on fresh IDs
  and feed an otherwise raw/empty-run event into the newly canonical actor,
  then synchronize on a Hold barrier. Inspect authoritative handle/log/process
  state before deciding it is a stuck fixture. It is not evidence to restart
  solely on a polling timeout. disconnect_worker_for_test currently drops only
  the body sender; review actor/control lifetime if an actual defect remains.
- Earlier full Rust compile attempts ended terminal (24568/3835). Fixed test
  errors: canonical_store clones the &SqliteStore; lifecycle tests use make_cmd
  instead of nonexistent RpcCommand::default; duplicate-BEGIN test matches Err
  without requiring Debug on the transaction; two proto EventsSince initializers
  now supply watermark (and assert it). Removed unused old test-only compactor
  shutdown/pending_count methods and an unused wildcard import. Current run
  compiled beyond these errors into actual tests. Do not redo those repairs.
- All Node test sessions are terminal and passed on the latest runs (59717,
  25275,19972; shared 23310). No GUI Rust tests, ignored acceptance workloads,
  real browser benchmarks, final Clippy/static matrix or integration push have
  been performed yet. No active Vite server was started.

Next concrete steps: finish/diagnose the live Rust run and repair failing cases;
pin ONLY genuinely legacy-specific journal/compaction fixtures explicitly to
format 1. Preserve canonical/default-creation coverage. Obtain detailed remaining
failures, run GUI Rust and meaningful affected final checks, execute all explicit
acceptance/fault workloads and browser render/decode benchmarks, measure/report
space and latency with honest limits, then finish-lock integrate/push dev and hand
off manual testing. Do not declare success on the green Node suites alone.

Task HEAD remains 91410a8120ec142de610662306ef6bcae4c5ad09, all implementation
uncommitted on refactor/segmented-storage. Last main inspection was clean at
e4d2a9a4d87f59af184e4fa592a581e0a73a4985. Goal remains active; no blocker.

## Latest Rust final-phase repair checkpoint (current state)

Supersedes the process notes immediately above. The original sandboxed full run
(44667) is TERMINAL, deliberately stopped after diagnosis. Only its exact cargo
PID 70073 and test PID 71626 were stopped after verifying command identity and
that they held this task's log. Its output is archived at
/tmp/future-segmented-rust-first-pass.log. A sandbox denied ps; the narrowly scoped
identity check/stop and mock-port test were approved outside the sandbox. No
automatic approval review rejection occurred, and no user app/service was stopped.

- A provider mock test concretely failed binding its local listener with
  Operation not permitted, then the same exact test passed outside the sandbox.
  Therefore the next full Rust matrix runs with the approved sandbox override;
  do not misclassify these port failures as implementation bugs.
- Added cfg(test) bind_legacy_events/configure_legacy_journal helpers and pinned
  ONLY historical raw-journal, old compaction/replay/reclamation and legacy
  broadcaster specifications to their intended layout. Production reservation,
  new/default and canonical fixtures remain format 2. NEW_SESSION_FORMAT is still
  FORMAT_VERSION. No global cfg(test) default override was introduced.
- Found and fixed an actual effect-boundary issue: a one-shot failed approval
  durability callback could be followed by a successful result fence. StreamContext
  now latches durability_failed for the entire run; a later callback cannot turn
  an uncertain durable outcome into a normal result. The original strict
  failed_intent_or_approval_fence_prevents_external_effects test now PASSES without
  weakening its no-effect/no-result assertions.
- Independently reproduced the worker-exit hang even after selecting legacy.
  Captured /tmp/future-segmented-worker-hang.sample for exact test PID 72693: only
  main/test threads remained; EventBatchWriter::drop waited on a sync receipt
  after the actual actor exited. Crossbeam retained queued reply senders while
  the caller still owned the command sender. This was a real lifetime race,
  not just slow storage or a polling timeout. Stopped only that verified old
  reproduction (9197 terminal, exit 137) after sampling and writing the fix.
- JournalControl now owns a worker-exit observer; the thread owns its exit guard.
  Sync control AND body receipts select reply vs actual exit. Async controls
  select oneshot vs a watch notification. Reply wins a simultaneous exit, so an
  acknowledged commit remains acknowledged. Actor exit/panic cannot leave an
  orphaned queued sender holding a waiter indefinitely. The Hold test seam now
  disconnects both body and control senders, as its existing contract promises.
- New 5/5 deterministic journal_control tests PASS (orphaned sync/body/async and
  committed-ACK preservation), log /tmp/future-segmented-control-repair.log.
  Entire writer module 10/10 PASS, including the previously stuck test, log
  /tmp/future-segmented-writer-repair.log. Entire legacy compaction module 22/22
  PASS, log /tmp/future-segmented-legacy-repair.log. Its two old checkpoint tests
  now explicitly observe the separate PASSIVE worker while the reader is pinned;
  reclamation's return value no longer falsely represents checkpoint completion.
- Current Agent lib-test build has 2651 tests including the 5 new exit tests;
  binary from targeted build: target/debug/deps/future_agent-63d1c81744284228.
  New full workspace feature union may use another hash. Do not assume a binary
  from the earlier failed run is current after recompilation.

CURRENT LIVE HANDLE: exec session 46587, approved outside sandbox, running
`RUSTC_WRAPPER= CARGO_BUILD_JOBS=4 CARGO_TARGET_DIR=/Volumes/ZHITAI/Workspace/future-os/target rustup run 1.99.1 cargo test --workspace --exclude futureos --offline --no-fail-fast`
with output at /tmp/future-segmented-rust-tests.log. Poll that exact handle/log;
do not restart on an observation timeout. All earlier test/build handles are
terminal. No Vite server or browser benchmark is running. No GUI Rust tests,
ignored acceptance workloads or final Clippy/integration pass has executed.

All four latest Node suites remain green (shared 652, client 676, Desktop 5304,
mobile 2888). Recent repairs affect Rust only. Docs/diff gates passed after the
architecture phase update; refresh after this receipt. Main was verified clean
at e4d2a9a4, task HEAD remains 91410a8120ec142de610662306ef6bcae4c5ad09,
all source changes remain uncommitted. Complete M1–M6 goal remains active.

Latest handle update: GUI Rust --lib test compilation (47452) PASSED; GUI Rust
execution is now LIVE as exec session 68998, same approved override, command
`cargo test -p futureos --lib --offline`, log /tmp/future-segmented-gui-tests.log.
Full non-GUI Rust workspace execution remains LIVE as 46587; latest output has
39 failed lines and is still running future-agent. Detailed final failure report
is not yet available. Do not call these failures all fixtures without inspecting
them. No GUI tests/performance/ignored-fault workload has been claimed passing.
The browser benchmark and explicit ignored workloads are still pending. All
short repair/build handles (including 65864,24127,50563,20047,12118,68402) are
terminal; 9197 was explicitly stopped after its independent stack sample.

Newer results: GUI execution 68998 is now TERMINAL: 368 passed / 1 failed.
Failure is gui::startup_paths_tests::an_auto_connect_that_connects_is_reported_as_connected
at desktop/src-tauri/src/lib.rs:2256, mock remote status stays Reconnecting/
RemoteServer with a stopped device. Inspect/reproduce; no claim yet about its
cause. GUI test compilation was green. Full workspace 46587 remains LIVE and
has moved beyond Agent into other crates/CLI tests. Agent lib summary is 2604
passed / 39 failed / 8 ignored. There are also an Agent CLI smoke profile-timer
startup failure, service_startup logging failure and channel idle-worker eviction
failure (channel lib:1500 passed/1 failed). CLI backup/restore tests are now showing
failures too; await their final report and diagnose rather than assuming fixtures.

Bounded Agent diagnostics are saved at
/tmp/future-segmented-agent-failures-compact.log (39 case names, locations, errors,
each line truncated to 360 chars). USE THIS instead of dumping raw giant JSON
assertions from the full log. Actual canonical content journal/JSON identity,
display selection, recall and tool-detail cases are among these failures and need
careful source fixes. Other cases use unpaged get_session_entries (new format
intentionally requires paging), expect unpublished new-session tool inspection to
be empty, or target legacy raw events/healing/pricing. Keep production-default
coverage; only pin genuinely historical fixtures to legacy. No Rust pass is being
claimed. Main remains untouched and clean; no commit, push, PR or manual handoff.


## Final validation repair checkpoint (2026-10-26, after the initial matrix)

The original workspace run is terminal (exit 101), not waiting. Its seven failed
targets were Agent lib/cli_smoke/service_startup, channel lib, CLI lib, Host lib,
and TUI lib. Preserve the complete and bounded logs in `/tmp/future-segmented-*`.
Do not rerun passing suites merely to rediscover this result.

Production repairs in this checkpoint:

- Backup validation accepts supported Agent schema 8 through the shared schema
  ceiling; still rejects unsupported schema 1/future versions. Backup suite 12/12 passed.
- Zero-length SQLite substrings are valid empty body columns; NULL for such a
  substring no longer masquerades as a missing metadata page. Empty legacy recall
  snippets likewise remain empty values.
- Canonical display overlays require an actual terminal transcript marker before
  annotating an entry with run outcome/usage. Imported lineage-only run metadata
  must match the existing full projection.
- Paged history checks skipped legacy-import status before BOTH ready-index and
  fallback reads; an existing index cannot bypass unreadable-history protection.
- Empty canonical reservations can return empty tool inspection. Hidden fork or
  content operations, tombstones and imports retain the published-reader guard.
  New regression verifies the fork receipt cannot expose an unpublished child.
- Canonical missing fork sources return the existing actionable missing-on-disk
  error without disguising corrupt/published-reader failures.
- Read-admission error values are boxed internally and unboxed at gRPC boundaries;
  content source selectors/range tuples remove oversized signatures. Other Clippy
  repairs remove redundant reborrows/clones, preserving semantics.

Fixtures select legacy BEFORE reserving the session identity when testing actual
released raw-journal/pricing/corrupt-metadata/append-healing behavior. Default
creation remains canonical. New canonical snapshot equivalence also verifies
coalesced tool content identity/byte counts. JSON carrier strings compare parsed
payload fields, not incidental object-key serialization order. Non-advancing replay
pages are explicit errors. Modern bounded remote replay fixtures declare
`chunkedRead`, and mock Agents implement the storage-format capability preflight.
Failed canonical fences/terminal workers are asserted persistence_degraded with
no false completion acknowledgement; no test requests automatic history compaction.

Current verified results:

- Agent full second repair run: 2642 pass / 2 fail / 8 ignored. Both remaining
  cases subsequently fixed; affected complete command/prompt/tool scopes 188/188
  pass, separate canonical snapshot filter 7/7 pass. Additional Clippy-only source
  changes still need the appropriate final verification.
- Host full repair: 1807 pass / 1 fail / 11 ignored; remaining bounded replay
  fixture fixed and its exact production-path test passed.
- Explicit storage/fault/concurrency acceptance: 7/7 passed. Raw measurements are
  in `segmented-storage-acceptance-2026-10-26.json`; read their limits before citing.
- Desktop Rust auto-connect exact regression now passes (1/1); the whole GUI
  suite and integration targets still require a current final result.
- Desktop lint+typecheck, mobile lint+typecheck, shared projection typecheck,
  remote-client lint+typecheck, whole-workspace fmt and diff checks passed.
- Browser benchmark completed all 8 JSON and 4 Markdown cases with zero page
  exceptions. Large JSON yielded event-loop time. The 3000-row Markdown worker
  update still takes ~31 ms median push; do not claim every render meets 16.7 ms.
- Earlier Node suites remain green: shared 652, remote client 676, Desktop 5304,
  mobile 2888. Subsequent JS changes were a test-only formatting fix and adding
  standard Web API types to the shared package tsconfig.

Controlled final test environment clears inherited NO_COLOR and sets RUST_LOG=info.
The prior inherited NO_COLOR=1/RUST_LOG=warn suppressed assertions about ANSI and
startup/debug log output; do not change production behavior to fit those settings.
Test listeners run with the approved local-port sandbox override.

Active handles at this checkpoint (poll exact handles; do not restart on timeout):

- 97508: CLI/TUI/channel full consumers run, `/tmp/future-segmented-consumers-final.log`.
  Channel lib is green 1501/1501; CLI currently running.
- 99676: whole-workspace `cargo clippy --all-targets -- -D warnings`,
  `/tmp/future-segmented-clippy-final-clean.log`. First two passes found ordinary
  new-code lint issues, repaired; final result pending.
- 30303: local Vite server, port 14973, `/tmp/future-segmented-vite.log`; browser
  benchmark 16157 is terminal success. Stop only this task's server when done.

All earlier full Agent/Host, focused repairs, GUI exact test and acceptance handles
are terminal. Main remains clean on dev-tallcode; current task changes uncommitted.
A fresh fetch found origin/main moved to 3022f942 (11 commits after the task base),
including provider reasoning fixes and explicit journal-reclamation CLI support.
Origin/dev-tallcode still e4d2a9a4. After preserving the candidate in a task commit,
merge origin/main into THIS worktree, resolve source/contract conflicts, and rerun
checks affected by that merge before finishing into dev. Do not merge another
author's unlanded branch. No PR is to be opened by this session.

Remaining: current clean Clippy/consumer results; merge latest main and affected
rechecks; Agent startup/cli_smoke controlled-env regressions and full GUI; review
performance gates/limits; manual acceptance instructions; finish-lock guarded merge
and push dev-tallcode. Do not mark the full M1–M6 goal complete at this checkpoint.
