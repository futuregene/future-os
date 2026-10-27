# Run-journal cost and reclamation

[中文](run-journal.zh-CN.md)

Every Agent run appends its raw event stream to `run_events` in
`<home>/agent/agent.db`: one row per streamed delta, tool-output slice and
token-usage report. `future session journal` reports what that table costs and
`--reclaim` folds settled runs into `run_snapshots`. This page records what it
actually cost on one real store, why the reads around it stayed fast, and the
procedure with its measured timings.

Nothing here is automatic on a timer, and no command on this page deletes
sessions, entries or message blocks.

## The short version

- On a **4.43 GiB** store, `run_events` and its uniqueness index held **3.56 GiB
  (76 %)** while the conversation itself (`entries` + `message_blocks`) held
  **378 MB**.
- **The size did not slow the reads down.** `session list`, `session status`
  (`get_state`) and `session transcript` measured the same at 4.43 GiB and at
  1.30 GiB; they are index-driven reads of one session, so they never touch the
  journal pages that grew.
- The journal is still unbounded by construction: only a run-completion
  notification folds it, so a store that missed those notifications keeps every
  row. On this store **135 of 1,166 settled runs** still held their raw journal.
- `future session journal --reclaim --yes` folded **6,958,103 events of 85 runs**
  in about **70 s**; a single `VACUUM` then took **5.4 s** and took the file from
  **4.43 GiB to 1.30 GiB**. All **337** session transcripts were **byte-identical**
  before and after.

## What the file was made of

Measured with `dbstat` on a copy taken 2026-10-27 11:18 from a live store
(macOS 26.6.2, Apple M3 Ultra, APFS, `future` v0.0.2):

| Object | Bytes | Share |
| --- | --- | --- |
| `run_events` (table) | 2,828 MB | 59 % |
| `sqlite_autoindex_run_events_1` | 802 MB | 17 % |
| `run_snapshots` | 322 MB | 7 % |
| `message_blocks` | 224 MB | 5 % |
| `entries` | 155 MB | 3 % |
| `history_display` | 50 MB | 1 % |
| `history_shapes` | 44 MB | 1 % |
| everything else (indexes, `sessions`, `runs`) | ≈ 60 MB | 1 % |

The file was **4,753,215,488 B** with `page_size` 4096, 1,160,453 pages and
**0 free pages** — every page SQLite had ever freed was already in use again, so
this was not fragmentation left behind by an earlier cleanup. Row counts:
`run_events` 7,745,220 · `entries` 116,281 · `message_blocks` 154,295 ·
`sessions` 337 rows (300 listable, 37 tombstoned) · `run_snapshots` 1,020.

## Who writes it

Grouped by `json_extract(payload,'$.event_type')`:

| Event kind | Rows | Payload | Per row |
| --- | --- | --- | --- |
| `thinking_delta` | 4,075,791 | 847 MB | 217 B |
| `tool_delta` | 3,485,150 | 840 MB | 252 B |
| `tool_end` | 16,265 | 63 MB | 4,103 B |
| `tool_start` | 32,608 | 20 MB | 663 B |
| `text_chunk` | 99,845 | 15 MB | 159 B |
| `usage` | 15,327 | 5 MB | 351 B |
| every other kind | ≈ 20,000 | < 2 MB | — |

Two streaming deltas are **97.5 % of the rows** and **94 % of the payload**.
It is not a few oversized payloads: 91 % of the rows are 128–255 bytes, and the
whole store holds only 454 rows above 16 KiB (largest 404,616 B). The cost is
therefore *row count*, not row size — and each row costs more than its payload,
because the uniqueness index (`session_id`, `run_id`, `idx`, `epoch`) is paid on
top: 3,630 MB of table plus index for 7,745,220 rows is **≈ 491 B per 243-byte
event**.

The same holds per session: the busiest conversation carried 327,479 events
(77 MB), and 135 settled runs together held 7.72 M events. On that day 5 runs
were `running`, 947 `completed`, 149 `cancelled`, 50 `interrupted`, 20 `failed`.

## Does the size cost anything?

Wall clock of the CLI (process start included, median of 3–5 runs, warm page
cache), against the same content before and after the fold:

| Command | 4.43 GiB store | 1.30 GiB store |
| --- | --- | --- |
| `future session list` | 0.11 s | 0.11 s |
| `future session info <id>` | 0.09 s | 0.08 s |
| `future session status <id>` (`get_state`) | 0.01 s | 0.01 s |
| `future session transcript --session <id>` | 0.02 s | 0.02 s |
| `future session transcript --session <id> --all` | 0.21 s | 0.20 s |
| `future session journal` (the report) | 23.8 s cold / 11.0 s warm | 0.56 s |

So the honest answer for a store of this size is: **the conversation-facing
commands do not need the journal shrunk**. They read `entries`,
`message_blocks` and a handful of indexes for one session at a time, so their
cost follows the conversation, not the journal. The only command that tracked
the file size was the journal report itself, because it scans the table it
describes — that is why `get_journal_stats` now runs under the maintenance
deadline instead of the 30 s storage deadline: a cold report on this store was
measured at 23.8 s, and an earlier cold run hit the 30 s wall and printed
`Timeout expired`, that is, the diagnostic failing on the database it exists to
diagnose.

## What reclamation removes

`future session journal --reclaim --yes` runs the same per-run fold the Agent
does on completion, but explicitly and in bounded slices (20 s of work per RPC,
`hasMore` until the sweep ends):

- **Folded, not truncated.** For a settled run whose raw journal ends with
  `agent_end`, the delta stream is folded into one `run_snapshots` row (the
  `DeltaFolder` keeps the visible text stream and the pricing events), and only
  then are the raw rows deleted — publication and the first delete slice share
  one transaction, so a crash cannot leave a reader without either.
- **Never touched:** `entries`, `message_blocks`, `history_*`, `sessions`,
  `runs`, and the journal of any run that is still `running` or has not settled.
- **Refused instead of guessed:** a journal whose last event is not `agent_end`,
  or that contains provider deltas that cannot be proven duplicates, is kept and
  counted (`Kept journals` in the report).

Why the fold is smaller: on this store `run_e4a62a1c…`'s 45.8 MiB of `thinking_delta`
and `tool_delta` became a **952 KiB** snapshot, and 6.96 M raw events
(≈ 1.7 GiB of payload) became 85 snapshots (≈ 114 MiB).

## Procedure

Measured on the 4.43 GiB store above; the whole sequence took under two minutes
of work plus the backup.

1. **Stop what owns the home.** Quit the Desktop app and stop the Agent
   (`future agent`, or the host that started it). `VACUUM` needs an exclusive
   transaction and the Agent's `busy_timeout` is 5 s: measured on this store, a
   writer that arrived during a 24 s `VACUUM` failed with `database is locked`
   after 5.01 s. `future data backup` also refuses to run while the home is
   owned, which is the point.
2. **Back up.** `future data backup <new-directory>` writes a verified copy
   (SHA-256 per file) of `agent.db` and the rest of the home. It is the only
   supported backup path — see [Data backup and recovery](data-backup.md).
3. **Report.** `future session journal` — cost, largest run, what is
   reclaimable, what is kept and why.
4. **Fold.** `future session journal --reclaim` to see the plan, then
   `--reclaim --yes` to apply it. Interrupting is safe and resumable; re-running
   continues. On this store: 4 slices, **85 runs / 6,958,103 events in ≈ 70 s**,
   ending with `quick_check: ok`.
5. **Shrink the file.** The sweep frees pages but does not return them to the
   filesystem: after step 4 the file still measured 4.22 GiB with 752,688 free
   pages (3.08 GiB). Those pages are reused by new events, so the store stops
   growing; a one-off `VACUUM` is what makes the file smaller. With the Agent
   still stopped:
   ```bash
   sqlite3 ~/.future/agent/agent.db "VACUUM;"
   ```
   Measured **5.4 s** (4.22 GiB → 1.30 GiB file), and **24.0 s** on the
   unreclaimed 4.43 GiB file (which only drops to 4.18 GiB, because it has no
   free pages — there is nothing to give back). Needs free disk space for the
   final file (SQLite builds it beside the original) and leaves
   `PRAGMA quick_check` = `ok`.
6. **Verify.** `future session journal` again (rows and size), then read a
   conversation: `future session transcript --session <id> --all`. The
   transcripts, not the byte counts, are the acceptance test — see below.

Windows: the same steps with `%USERPROFILE%\.future\agent\agent.db`, and the
sqlite3 invocation from a shell that has `sqlite3.exe` on `PATH`.

## What the verification looked like

The property that matters is "removing events must not change what a reader
sees", so the check is a diff, not a promise:

- `future session transcript --session <id> --select all --all --max-bytes 0
  --json` for **all 337** session ids (235 MB of output) — identical before the
  fold, after the fold, and after `VACUUM`. The diff is falsifiable: appending a
  single byte to one dump makes it report that file.
- `entries` (116,281 rows) and `message_blocks` (154,295 rows) dumped in primary
  key order — identical MD5 before and after (`1be02f36…`, `0e0dcae1…`).
- `PRAGMA quick_check` = `ok` after the sweep and after `VACUUM`.
- `cargo test -p future-agent journal_reclaim` covers the same invariants on a
  synthetic store, including that a sweep leaves `entries` and `message_blocks`
  byte-identical.

## What is left behind

Measured on the store above after the fold — these are known limits, not bugs
being worked around here:

| Leftover | Size | Why |
| --- | --- | --- |
| 50 settled runs whose journal does not end with `agent_end` (49 `interrupted`, 1 `cancelled`) | 761,984 events · 179 MiB | The fold refuses a journal a producer may still append to. These runs were cut off mid-stream (the last event on the largest one is a `tool_start`), so they stay until a future decision lets a settled run's stream be folded regardless of its tail. |
| 5 `running` runs | — | Live runs are never candidates, by design. |
| Free pages inside the file | 3.08 GiB before `VACUUM` | The sweep hands pages to SQLite's freelist; idle incremental vacuum returns them in 10 ms slices, and only `VACUUM` rewrites the file. |

Also worth knowing when reading the numbers: the automatic fold is triggered by
a completion notification and has no startup sweep and no retry, so a store that
was closed mid-run, upgraded, or simply busy keeps its raw journals. That is how
this one reached 4.43 GiB with `journal.reclaimed_runs` = 62 before the explicit
sweep took it to 147.

## Reproducing the measurement

```bash
# a copy, never the live store
export FUTURE_HOME=/tmp/j            # its own agent/, so its own lock and socket
mkdir -p "$FUTURE_HOME/agent"
cp ~/.future/agent/agent.db* "$FUTURE_HOME/agent/"   # db, -wal and -shm together
future agent --grpc-addr 127.0.0.1:5099 &
FUTURE_AGENT_GRPC_ADDR=127.0.0.1:5099 future session journal
FUTURE_AGENT_GRPC_ADDR=127.0.0.1:5099 future session journal --reclaim --yes
sqlite3 "$FUTURE_HOME/agent/agent.db" "PRAGMA quick_check; PRAGMA freelist_count;"
```

Running an Agent against a copy needs its own `FUTURE_HOME` and a free port; it
never shares the endpoint or the lock of the Agent you are actually using.
