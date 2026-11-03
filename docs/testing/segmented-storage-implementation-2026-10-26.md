# Segmented storage implementation and local acceptance — 2026-10-26/27

Status: implementation, native/frontend builds and local checks complete; ready for manual acceptance.
Approved scope: [M1–M6 development plan](../architecture/stream-storage-development.zh-CN.md).
Manual checklist: [人工验收单](segmented-storage-manual-acceptance-2026-10-27.md).
Measurements: [raw synthetic results](segmented-storage-acceptance-2026-10-26.json).

The user requested all implementation first, then local tests, then a candidate for
manual testing. The implementation phase ended before the first test pass. Failures
were repaired during final verification. This record replaces the chronological
working notes: their old process handles and default-off states are obsolete.

## Candidate and scope

Implementation commit `afdfba7e`; latest-main integration `3f9e2208` (main base
`3022f942`). Final follow-up commits update delivery evidence only. The main working
checkout is the user's `dev-tallcode`; source work was confined to
`.worktrees/segmented-storage` on `refactor/segmented-storage`.

| Plan requirement | Implemented path / evidence |
|---|---|
| Additive compatibility | Agent schema 8; actual frozen schema-7 fixture; per-session format identity; shared RPC storage-schema negotiation. Fresh identities select format 2; imported/deleted/old identities cannot be silently converted. |
| One durable body | `agent/src/session/content/`: immutable UTF-8 segments, typed references, semantic JSON identity, idempotent offsets and atomic semantic boundaries. New-format reasoning/text/tool bodies do not also populate raw token events or completion snapshots. |
| Responsive write path | `rpc/broadcaster/{admission,content_sequence,journal_writer,journal_control}.rs`: stable identity before deltas; ownership transfer to a dedicated writer; bounded body/control queues; async fences; independently scheduled live delivery. |
| Durability | WAL+FULL; produced vs committed cursors; user/intent/approval/terminal boundaries wait for the necessary commit. Whole-run fence-failure latch prevents external effects after failure. Terminal transcript and terminal event commit together. |
| Bounded storage work | 16 KiB immutable segments, 64 KiB body-write jobs, 256 KiB raw reads; 25 ms / 128 events / 64 KiB actor batching. Per-session/global pending-byte budgets, age limit, reserved controls and explicit backpressure. These are bounds/scheduling parameters, not hard crash-loss timing guarantees. |
| Readers and WAL | Dedicated bounded query-only readers; body/JSON conversion outside SQLite workers; separate PASSIVE checkpoint thread. No foreground TRUNCATE/checkpoint or run-end whole-history rewrite. Queue/job/COMMIT/checkpoint/WAL observations are bounded metrics. |
| All consumers | Agent RPC; shared Rust/TS range and overlap cursors; host observer/collector; remote feature `segmented_content_v1`; Desktop, mobile, CLI, TUI and channels. Unsupported old peers receive an explicit capability error for new formats. |
| Frontend transfer/decoding | Native immutable read transfers use 64 KiB chunks, bounded concurrency/queue and window ownership; cancellation/generation fences. Cooperative large JSON composition preserves syntax, Unicode, duplicates/prototype semantics; private Symbol carries decoded graphs without repeated full stringify/parse. |
| History and model context | Bounded history/replay, tools, recall/title/sidebar, metadata/config, fork/reset/checkpoints and export/backup/delete paths use the appropriate format reader. Provider-required opaque/signature fields and unknown extensions are retained. |
| Fork/reset/delete | Hidden staged operations, immutable-body sharing, exact owner/generation checks, retirement and bounded idle collection. Deleting a view cannot destroy another view's shared body. |
| Runtime modes | Independent/embedded Agent and ephemeral mode; ephemeral selection drains/detaches persistence before admission and cannot reuse volatile cursors as durable ones. |
| Release storage policy | Production diagnostic selection omits redundant debug payloads. Normal startup/completion starts no compactor for either format. Old histories are not scanned for conversion/compression. |

M0 remains part of the candidate: Desktop contract 4 retires `workspace_files`,
`reference_targets`, `object_references` and the duplicate review-run index, with
versioned migration/snapshot/rollback contracts. Live history/business tables remain.

The latest main added `future session journal` as an explicit legacy maintenance
command. It remains available, but is never scheduled automatically and explicitly
skips format 2. Regression `explicit_legacy_reclaim_never_rewrites_canonical_conversations`
verifies its format guard and preserves the body. No historical sweep was executed.

## Final verification evidence

Rust uses pinned toolchain 1.99.1, `RUSTC_WRAPPER=` and the shared build target.
Tests requiring synthetic listeners used the approved local-listener override.
Controlled test invocations clear inherited `NO_COLOR` and set `RUST_LOG=info` so
ANSI/log assertions actually observe the intended test environment.

| Check | Result / local log |
|---|---|
| Agent complete crate tests after latest-main merge | PASS: lib 2661 passed / 8 explicitly ignored; all default integration/doc targets pass, including cli_smoke 25 and service_startup 15. `/tmp/future-segmented-agent-after-main-final.log` |
| Explicit final storage/fault/concurrency workloads | PASS 7/7, after other regressions exited; `/tmp/future-segmented-acceptance-quiet-final.log`. Helper/default ignored status is deliberate; the acceptance command uses `--ignored`. |
| Desktop Rust lib after latest-main merge | PASS 369/369; `/tmp/future-segmented-gui-after-main-final.log`. |
| Host complete repair pass + remaining-case repair | Full pass 1807 passed / 1 failed / 11 ignored; remaining bounded-replay fixture corrected and exact test PASSED. `/tmp/future-segmented-host-repair.log`, `/tmp/future-segmented-host-replay-final.log`. No unresolved Host failure. |
| CLI / TUI / channel consumers | Lib tests PASS 1400 / 1229 / 1501, plus integration targets. Initial TUI i18n guard failure was fixed by upstream; final guard PASS 8/8 without changing the new budget. CLI upstream journal module PASS 7/7 after merge. Logs `/tmp/future-segmented-consumers-final.log`, `/tmp/future-segmented-tui-guard-final.log`, `/tmp/future-segmented-cli-journal-after-main.log`. |
| Entire Rust workspace Clippy, all targets | PASS `cargo clippy --workspace --all-targets --offline -- -D warnings`, after latest-main merge; `/tmp/future-segmented-clippy-after-main-final.log`. No lint suppression added for the repairs. |
| Rust fmt / diff / docs | PASS whole-workspace fmt, `git diff --check`, `make check-docs` (including strict bilingual and channel matrices). |
| Desktop Node suite | PASS 383 files / 5304 tests; `/tmp/future-segmented-desktop-tests.log`. Merge-affected remote/native-history/message modules additionally PASS 109/109 after main; `/tmp/future-segmented-desktop-after-main-tests.log`. |
| Mobile / remote client / shared projection suites | PASS 150 files / 2888 tests; 31 suites / 676 tests; 26 files / 652 tests. Logs `/tmp/future-segmented-mobile-tests.log`, `/tmp/future-segmented-remote-client-tests.log`, `/tmp/future-segmented-shared-tests.log`. |
| Types and ESLint | PASS Desktop, mobile, remote client; shared package standalone typecheck PASS. Desktop repeated after main. |
| Native executables | PASS `cargo build -p futureos -p future-cli --offline`; `/tmp/future-segmented-native-build-final.log`. |
| Production frontend | PASS `npm run build`, including the production worker artifact check; `/tmp/future-segmented-desktop-build-final.log`. |
| Isolated real browser workloads | PASS 8 JSON cases (five rounds each) and 4 Markdown cases; no page exceptions. `/tmp/future-segmented-browser-report.json` and committed raw record. |

The original broad workspace pass had failures. They are not erased or described
as a green full run. Relevant complete/scoped reruns above close them: backup schema
ceiling, empty column/snippet handling, actual terminal overlays, skipped-import
history guard, empty reservation vs hidden-fork inspection, broken-worker exit
receipts, explicit legacy fixtures, new lifecycle events, parsed JSON carrier
comparison, canonical content ranges and honest persistence-degraded outcomes.

## Measured outcome and limits

Final synthetic reasoning workload uses 55-byte fragments in 128-event batches:

| Body | Format | Database + WAL sampled bytes | Terminal commit, microseconds |
|---|---:|---:|---:|
| 1,048,630 bytes | 1 | 19,566,776 | 33,564 |
| 1,048,630 bytes | 2 | 6,087,936 | 1,464 |
| 10,485,805 bytes | 1 | 200,381,968 | 54,323 |
| 10,485,805 bytes | 2 | 21,993,656 | 1,448 |

Format 2 wrote zero raw `run_events` and zero `run_snapshots`. No VACUUM or
historical compaction contributed to these results. Full bodies were checked.
The size/termination samples are evidence for the architectural improvement, not
promises about every user's database: WAL reuse/checkpoint timing and workload
shape affect disk samples. Direct writer batch timing is not live queue backlog.

Fault cases kill actual child processes around commit boundaries, inject SQLITE_FULL,
apply an external write lock and unwind a real writer transaction. They prove the
committed-prefix/acknowledgement contracts under those failures, not physical power
loss on arbitrary hardware or exactly-once external side effects.

In isolated headless desktop Chrome, cooperative decoding of the approximately
18 MiB object graph reduced median timer delay from about 28 ms to about 3 ms,
while total decode time increased. No decode long task above 50 ms was observed.
These are not Tauri IPC or mobile-device timings. The 3000-row table worker path
still takes approximately 31 ms median push, above one 16.7 ms frame; do not claim
all Markdown rendering meets the frame target. Normal first-visible latency and
physical-device interaction remain part of manual acceptance.

## Handoff and release boundary

Use the manual checklist for new/old conversations, long streaming, cancellation,
large tool output, crash/restart, remote reconnect, provider continuation, fork/reset
and backup. Manual testing and Linux/Windows/device CI are release acceptance;
local macOS checks do not substitute for them. Old binaries that do not support
schema 8 must not open a database after the new schema is written.

The session finishes by syncing latest main, merging under the finish lock into
clean `dev-tallcode` and pushing that integration branch. The scheduled PR task
owns PR creation/auto-merge. No task branch is pushed, and no personal database
was converted or compressed by this session.
