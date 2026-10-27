# Conversation storage space baselines

Date: 2026-10-27. Chinese: [会话存储空间基线](stream-storage-space-baselines.zh-CN.md).
Active [development plan](stream-storage-development.md); complete previous [unpublished plan](stream-storage-development-unreleased-2026-10-26.md).

Preserve the original survey, previous implementation and prototype as different evidence. Add new records rather than replacing old values.
The [machine-readable ledger](../testing/stream-storage-space-baselines-2026-10-27.json) records provenance/scope/exact bytes/limits.
The [previous raw acceptance JSON](../testing/segmented-storage-acceptance-2026-10-26.json) remains unchanged, including concurrency/browser observations and both storage samples.
This page contains no private dialogue, conversation IDs, tool content or credentials.

## B0: original event-layout survey

Source: section 2 of the previous plan at `e339334e`, historical survey dated 2026-10-26. Only these rounded figures remain.

| Scope | Recorded value |
|---|---:|
| Agent database | Approximately 96.8 MiB |
| Raw events and their indices | Approximately 85.2 MiB |
| Reasoning fragment events | 181,480 |
| JSON of those reasoning events | Approximately 38.7 MiB |
| Corresponding reasoning text | Approximately 1.68 MiB |

This identifies event-envelope/index overhead. Original exact bytes, WAL state and source database snapshot were not archived here; do not invent them.
The 83% from an earlier historical-copy compaction experiment is not new-write acceptance or a promise of this plan.
This workload differs from the synthetic cases below; do not compute cross-workload saving percentages.

## B1a / B1b: previous unpublished segmented implementation

Source: raw acceptance `earlierStorageSample` and `storage`. The combined artifact names candidate `3f9e2208`,
macOS 27.0.1 arm64 and Rust 1.99.1. The earlier sample lacks its own execution commit; do not infer byte-identical code to the final candidate.
Workload: 55-byte reasoning fragments, batches of 128 events; actual body sizes 1,048,630 / 10,485,805 bytes.
Format 1 is the synthetic legacy control; format 2 is the previous segmented implementation.

| Sample | Approximate body | Format-1 DB+WAL bytes | Format-2 DB+WAL bytes | Format-1 terminal µs | Format-2 terminal µs |
|---|---|---:|---:|---:|---:|
| B1a earlier | 1 MiB | 19,382,456 | 8,972,824 | 39,555 | 1,661 |
| B1a earlier | 10 MiB | 200,795,664 | 21,868,544 | 60,283 | 1,633 |
| B1b final | 1 MiB | 19,566,776 | 6,087,936 | 33,564 | 1,464 |
| B1b final | 10 MiB | 200,381,968 | 21,993,656 | 54,323 | 1,448 |

The earlier sample overlapped regression tests; the final sample ran after other regressions exited. Retain both, rather than selecting the prettier result.
Each is a single synthetic sample. Batch-sampled sizes include WAL and depend on checkpoint timing/reuse; neither used historical compaction or VACUUM.
Direct writer/terminal timings are not UI frames, full-run latency or live persistence backlog.
The original JSON also retains rows, timing distributions, separate database/WAL/checkpoint observations and concurrent reader/writer results.

## B1c: actual previous-layout audit

A read-only aggregate observation on 2026-10-27 covers four turns, 54 transcript/run entries, 75 content objects,
66,086 body bytes, 1542 segments and 1730 changes. No inspected hash/range/cursor/tool-pair/terminal inconsistency was found;
this does not establish every failure-mode contract.

| Structure | Allocated bytes | Allocation |
|---|---:|---:|
| `stream_changes` | 675,840 | 660 KiB |
| `content_segments` | 106,496 | 104 KiB |
| `stream_changes_entry_tool` | 90,112 | 88 KiB |
| `stream_change_content` | 57,344 | 56 KiB |
| `stream_change_content_body` | 40,960 | 40 KiB |

Page size is 4096 bytes. Allocations cover all observed canonical data, not only the latest turn. Segment count is not page count.
Latest-turn aggregates separately: 26,925 body bytes, 550 segments, 595 changes spanning 6382 event indices and 209,787 change-JSON bytes.
This supports prioritizing metadata; it does not justify longer commit intervals or run-end segment organization.

## B2: in-memory compact-change layout model

The model projects B1c metadata into an in-memory database. It changes no personal database, copies no body and is not a production runtime test.
It retains original timestamps/unknown JSON and includes context/cursor/billing/reverse-body indices.

| Scope | Current layout | Candidate model |
|---|---:|---:|
| Compared metadata table/index allocation | 921,600 bytes | 385,024 bytes |
| Allocation in KiB | 900 | 376 |
| Changes | 1730 | 1730 |
| Reconstructed metadata | Original input | All 1730 semantically equal |

Local reduction is 58.2%. There are 1522 direct-range changes, 214 context descriptors and 70 remaining extra-field links.
Experimental codes/SQL are not frozen production contracts. The model does not prove RPC, recovery, authorization, collection or migration.
It excludes body storage, WAL/write volume, CPU/RSS, commit backlog, UI and disk-failure measurements.

## Rules for future B3 records

- Write a new result file with commit, target layout/conversation format, hardware/toolchain, workload, page size and build mode.
- Compare matching workloads on the same machine, retaining legacy/previous baselines without making new production code migrate the previous development database.
- Record WAL/FULL/checkpoint/sampling/interference/repetitions/percentiles/maxima; measure both allocation and write volume.
- Keep raw output, summaries, failures and fallbacks. Never overwrite B0/B1/B2 or extrapolate local ratios to all historical data.
- Label missing original evidence; additional measurements are new records, not corrections to old values.
- Revalidate released-fixture migrations after implementation and associate fresh results with B3. Previous migration passes remain historical evidence.

Preserved acceptance artifact SHA-256:
`564045f7ca4bc9389ac6532c63ce3f47a6e57524c132d3b8e99fed2cb318a6be`.
The ledger also records original-plan hashes and source commits for later preservation checks.

## B3.1: first candidate, withheld because of regression

[Raw record](../testing/compact-storage-candidate-1-2026-10-27.json) compares `ac9986d0` with previous production code `537bcddf` on the same machine and workload: 55-byte fragments, four fragments per flush, approximately 1 MiB body, long source identities. Values below cover allocated stream table/index pages, excluding bodies and WAL.

| Scenario | Previous metadata bytes | Candidate bytes | Change | Previous/new batch p99 µs |
|---|---:|---:|---:|---:|
| High reuse | 3,649,536 | 946,176 | −74.1% | 473 / 562 |
| Frequent new contexts | 7,639,040 | 12,259,328 | +60.5% | 794 / 1014 |

The latter also grows the WAL file. Delivery is withheld while stable identities are normalized further. Each scenario is a single sample, so timing differences cannot be attributed to one cause. Keep these failed boundaries too; future candidates append records instead of replacing them. Functional/migration/fault passes do not establish space/performance acceptance.

## B3.2: dictionary and UUID candidate, withheld for write amplification

[Candidate 2 raw record](../testing/compact-storage-candidate-2-2026-10-27.json) retains the same long-identity workload, previous `537bcddf` control, current source hashes, binary hashes and test inventory. Frozen binaries run sequentially, old then new, excluding compilation; each scenario is one sample. B0/B1/B2/B3.1 are not replaced.

| Scenario | Previous/new stream metadata bytes | Allocation change | Previous/new logical WAL page bytes | Write change | Previous/new batch p99 µs |
|---|---:|---:|---:|---:|---:|
| High reuse | 3,649,536 / 958,464 | −73.7% | 187,793,408 / 177,676,288 | −5.4% | 445 / 518 |
| Frequent new contexts | 7,639,040 / 5,738,496 | −24.9% | 415,535,104 / 614,146,048 | +47.8% | 693 / 939 |

Stream metadata still sums tables/indices whose names contain `stream_`, excluding body segments and `content_references`. A future ownership-table change must redefine matching comparison scopes explicitly. Logical page bytes are successful WAL-mode writer `SQLITE_DBSTATUS_CACHE_WRITE` deltas times 4096, excluding WAL frame/header bytes, checkpoint copies and device I/O. Raw records separately retain WAL file lengths and whole-process CPU/RSS; neither is cumulative writes or per-case resource consumption.

This is low-level staging stress: high-churn blocks in one message exceed the final entry's 1024-field limit, and that oversized assistant header is not published. It is not a normal UI conversation or a production UUID-shape workload: source identifiers are long opaque strings. Readers request 128 events but return different byte-budgeted work; `readerWork` is retained and reader timing alone does not establish equal-work latency.
One regression run that reused the old binary in a shared Cargo target was explicitly invalidated. Valid results verify source records, distinct binary hashes and the new 2695-test inventory.

The dictionary removes candidate 1's allocation regression but still increases cumulative writes. Normalize body ownership and remeasure; allocation savings cannot hide write amplification.

## B3.3: structured body ownership, intermediate candidate

[Candidate 3 raw evidence](../testing/compact-storage-candidate-3-2026-10-27.json) retains one measurement of integer stream/symbol owners, binary and production-source hashes, and process resources.
For the same high-churn stress, the complete database is 7,933,952 bytes, versus 11,964,416 for the old implementation and 10,407,936 for candidate 2.
Logical WAL page bytes are 576,319,488, down from candidate 2's 614,146,048 but still about 38.7% above the old 415,535,104; batch p99 is 1049 microseconds.
High-reuse logical WAL page bytes remain 177,676,288. Delivery remains withheld pending further write-amplification work.

Ownership moved from `content_references` to `stream_body_owners`; a `stream_` name subtotal is no longer comparable with earlier candidates.
These figures use complete database files and the same page-write counter scope. Raw evidence retains all structure allocations; final acceptance still needs matched ownership totals, repeated samples and equal-work reads.

## B3.4: bounded batches and scoped identities, before first-read optimization

[Candidate 4 raw evidence](../testing/compact-storage-candidate-4-2026-10-27.json) retains staged diagnostics, three paired valid UUID transcript repetitions, per-process resources, binary/source hashes and the source patch.
The workloads publish 20/150 assistant messages and matching tool results, with thinking and a tool call per assistant, eight original fragments per admitted batch and synthetic lengths shaped by historical aggregates. No private bodies are included.
Each pair has identical SHA-256 over message IDs, types, roles and complete content; thinking, arguments and tool outputs also match explicit expected values.

| Scenario | Old/new complete allocation bytes | Old/new logical WAL page bytes | Allocation / write change |
|---|---:|---:|---:|
| 20 tool calls | 1,712,128 / 1,032,192 | 104,177,664 / 95,350,784 | −39.7% / −8.5% |
| 150 tool calls | 10,522,624 / 4,972,544 | 795,967,488 / 728,698,880 | −52.7% / −8.5% |

Allocation includes all dbstat structures, avoiding ownership-table renaming bias. Allocation and logical writes match across all three repetitions. Matching debug toolchains run sequentially without concurrent compilation or heavy tests.
Unthrottled write-time medians remain approximately 27%/31% above the old implementation. Producer cadence, queue responsiveness and UI need separate measurement; submillisecond commits do not erase CPU cost.
Old/new first-read samples are `[2377,2180,2216]` / `[5861,3010,2754]` microseconds for 20 tools and `[3104,3187,3634]` / `[3548,4536,3667]` for 150 tools, still a regression.
This is not final acceptance. Later reader schema-readiness initialization is outside this frozen candidate and requires separate evidence rather than overwriting these results.

## B3.5: reader schema readiness, final full acceptance still in progress

[Candidate 5 raw evidence](../testing/compact-storage-candidate-5-2026-10-27.json) references implementation checkpoint `cc0ae84b`; every frozen-program source hash was verified against that checkpoint.
The same valid transcripts and logical-content digest checks run for three new sequential old/new repetitions. Complete allocation and logical WAL values remain identical to B3.4.

| Scenario | Old first page/body microseconds, three runs | New first page/body microseconds, three runs | Maximum per-run complete-read p95, old/new microseconds |
|---|---|---|---:|
| 20 tools | 3491 / 2214 / 2441 | 1206 / 1054 / 1094 | 25,756 / 25,526 |
| 150 tools | 3063 / 3256 / 3120 | 1406 / 1566 / 1802 | 192,631 / 195,868 |

Read workers only parse schema before advertising readiness; they do not read historical bodies. Reopen times are retained too, so shifted initialization cost is not hidden.
There are only three first-read samples/scenario, not a robust population p95. New direct-batch p99 is below 1 ms in every run, but unthrottled write-time medians remain approximately 29%/32% higher. Interpret CPU/RSS, queue responsiveness and UI together.
This artifact preserves its contemporaneous acceptance status; running full regressions and pending fault/concurrency/UI checks are not recorded as passing.

## B3.6: final automated acceptance and native manual handoff

[C6 raw final evidence](../testing/compact-storage-acceptance-2026-10-27.json) records fresh 1/10 MiB, small/large tools, native UUID 1/4/16 concurrency, actual admission, browser and migration/fault results, plus exact synthetic baseline overlays. C5's three paired realistic runs remain unchanged: allocations fall approximately 39.7%/52.7% for 20/150 tools and logical WAL approximately 8.5%; unthrottled total write/publication median wall time rises approximately 29%/32%.

Observed admission-to-durable p99 stays within 39.5/36.5/44.2 ms for 1/4/16 sessions. Terminal work for 1/10 MiB takes 1.65/1.78 ms. Large tool publication appends no duplicate bodies but full hash verification still takes approximately 26.5/268.8 ms; single-tool WAL rises about 3.4%/0.4%. File length, logical page writes and UI blocking are distinct metrics; checkpoint differences are not savings. See C6 and plan section15 for raw values, resource use, failures and limitations.

## B5: native ID recheck and three-generation storage comparison

Read-only review after testing application `80bea897`, 2026-10-27. The [machine-readable review](../testing/storage-id-and-savings-review-2026-10-27.json) retains exact bytes, formulas and evidence hashes. B0–B4 remain unchanged.

The latest five-prompt conversation has one session key,42 message keys,41 message/run references,five run keys and49 body-owner keys, all BLOB16 UUIDv4. All runs completed; ownership, foreign-key and integrity checks pass. The five Host run UUIDs exactly match the Agent, but Host run keys remain TEXT36 and Host thread IDs remain timestamp-prefixed TEXT29. Agent business identities and Host default run generation are covered; not every self-generated identity in every database is unified. Historical TEXT, provider opaque IDs, integer technical keys and auxiliary empty run keys remain unchanged.

Allocation sums every recorded dbstat structure, including indices, schema and fixed empty pages. Reduction is `(1 - current_bytes / baseline_bytes) * 100%`; negative means growth. Long-body and tool workloads differ, so reductions cannot be added or multiplied. WAL file length is excluded.

| Identical continuous body | Original event layout MiB | Previous segmented MiB | Current MiB | Reduction vs original | Change vs previous |
|---|---:|---:|---:|---:|---:|
| About1 MiB | 7.8984 | 1.6563 | 1.6758 | 78.78% | 1.18% growth (20 KiB) |
| About10 MiB | 76.0625 | 13.2539 | 13.0625 | 82.83% | 1.44% reduction (196 KiB) |

Raw inputs are C6 final-space-old formats1/2 and final-space-new format2. Fixed new-layout overhead slightly increases the small1 MiB case. Multi-tool savings must not be generalized to continuous large bodies.

| Identical tools, three paired runs | Previous KiB | Current KiB | Allocation reduction | Logical WAL page-write reduction |
|---|---:|---:|---:|---:|
| 20 calls | 1672 | 1008 | 39.71% | 8.47% |
| 150 calls | 10,276 | 4856 | 52.74% | 8.45% |

These use C5 three-run medians with matching logical-content digests. Logical WAL bytes are not physical device I/O. The observed whole personal Agent database is102,359,040 bytes (about97.62 MiB), including unchanged history; whole-file differences do not measure new-conversation savings. The actual new conversation has37,431 body bytes,54 objects and751 segments. It is an observation, not a saving comparison against an older conversation with different content.
