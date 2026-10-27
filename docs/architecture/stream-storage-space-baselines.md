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
