# Preview performance: retained chat and bounded reads

Follow-up to PR #664. This round changes Mobile only; it does not change the remote protocol or cancel model generation.

## Changes

- While the file panel, preview modal or download modal covers chat, retain the existing transcript/composer subtree. Ignore new parent render snapshots until uncovered, then install the latest children/callbacks. Preserve native list identity, expanded rows and draft state rather than unmounting them.
- Propagate presentation visibility through that memo boundary so elapsed-time and stop-diagnostic timers pause while covered. Remote state continues synchronizing. Accessibility hiding stays outside the frozen subtree and updates immediately.
- Read text previews through a read-only file handle, in at most 64 KiB chunks, yielding every 256 KiB. Read no more than the existing 2 MiB preview limit, even for a 10 MiB file. Close handles on success, cancellation and failure.
- Do not use Expo `File.slice()` for bounded reads: the installed implementation calls `bytesSync()` first and would still load the whole file.
- Validate UTF-8 without `TextDecoder` fatal/stream options, which the mobile `fast-text-encoding` fallback rejects. At a truncation boundary, omit an incomplete valid trailing code point rather than inserting a replacement character; reject invalid interior UTF-8 and binary control bytes.
- Render Markdown file previews using a bounded FlatList of blocks rather than mounting all blocks inside a ScrollView. Large previews (over 128 Ki UTF-16 code units) bypass the shared message parse cache, so their AST can be collected on close.

## Validation

Mobile typecheck, ESLint and all **99 suites / 1,399 tests** passed locally.

New deterministic regression workloads:

| Workload | Verified result |
|---|---|
| 500 covered parent updates | No extra content renders; no unmount; local expanded state retained; latest snapshot installed on uncover |
| File panel / preview / download overlay while new chat items arrive | Same FlatList instance retained; new data visible after uncover; file-panel accessibility hiding still applied |
| 2,000-paragraph Markdown file | All blocks remain in data, but RN test renderer initially mounts only 8 text blocks; large-file parsing is not cached globally |
| 10 MiB reported file size | Only 2 MiB requested/read, requests at most 64 KiB, another queued task runs before completion, whole-file API never called |
| Cancel after a read yield | No further reads, read-only handle closed |
| UTF-8 / EOF / decoder compatibility | CJK/emoji boundary handling, invalid/overlong/surrogate sequences, unexpected EOF, and a fallback decoder without fatal/stream support covered |

The file-I/O tests use instrumented native-handle mocks. The rendering tests use React's test renderer, not a phone or RN-Web. These prove bounded work, state preservation and correctness; they do **not** establish disk throughput, native frame rate, or battery savings. The prior round's real historical browser measurements are not reused as measurements of this round.

## Remaining limits

The full bounded Markdown prefix still needs parsing. A single giant table/code block is one list item and is not virtualized internally. Full download and integrity verification are unchanged; this only avoids the subsequent whole-file preview read. Timeline projection/catalog synchronization continue while an in-app overlay is open. Overall timeline-cache accounting and radio/transport energy remain separate work.

Release-device verification is still needed for native scroll retention, modal transitions, keyboard behavior, memory peaks and power use.
