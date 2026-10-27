# Released Agent database layouts

These synthetic fixtures freeze the table/index DDL from historical commits.
They contain no personal data and do not depend on Git history at test time.
Do not regenerate them from the current schema with a changed `user_version`:
that hides exactly the missing-table differences migration needs to support.

| Schema | Source commit | Change |
| --- | --- | --- |
| 2 | `d8ba9842` | First released SQLite session store |
| 3 | `9a6ec1af` | Skills install records |
| 4 | `9d4f8a5e` | Skill installations and operations |
| 5 | `273f09a5` | Completed-run snapshots, including `raw_pending` |
| 6 | `6502cded` | Bounded history reads and snapshot cleanup changes |
| 7 | `c43d9c71` | Database identity/generation, tombstones and queued run receipts |

Each fixture contains the released session schema batch, its skills table
batch where applicable, and the session-scoped event identity index. Runtime
views are installed by the current migration. Tests insert synthetic historical
rows before opening the database through the real Agent storage code, and check
both the preserved snapshot and the upgraded database.


The unpublished schema-8 segmented candidate is not another released fixture.
The [reviewed storage plan](../architecture/stream-storage-development.md) replaces
that draft directly and requires fresh released-schema migration results after
implementation. Existing green results do not establish the revised layout's
correctness. Keep schema 2–7 DDL frozen; test the real upgrade, readable source
snapshot, reopen/retry, old-session continuation and new compact conversations.

`unreleased-segmented-v8.sql` freezes only the previous unpublished content
layout at `537bcddf`, for negative admission tests. It is intentionally excluded
from `RELEASED_SCHEMAS`: opening it must fail before WAL/schema mutation and
preserve the original file, rather than migrate it into the compact layout.
