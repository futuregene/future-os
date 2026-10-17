# Campaign archive (backup, not a source of truth)

> ## RELOAD MARKER — read this before quoting any number below
>
> **Snapshot taken from:** branch `test/cov100-r2`, commit `692c6e2bd` (2026-10-18).
> **Refreshed on:** every push to this branch; the commit date above is the snapshot's date.
>
> **Open work will change these numbers.** `TAU-D9` (re-pin `module-tauri-remote` at HEAD and
> re-register 146 lines) and `TAU-D10` (`module-tauri-headless`: land 3 measured lines, fix 11
> registrations) are still in flight. When they land, `1195` and `283` **will move**.
>
> **Until this archive is refreshed against a source commit that contains those tickets' results,
> every figure here is provisional and the working branch `test/cov100-r2` is authoritative.**
> The refresh obligation is the supervisor's; if this marker's commit is older than the newest
> `docs/testing/` commit in the working branch, the archive is stale by definition.
>
> This warning exists because the campaign's own rule says a stale capture is worse than an absent
> one: this directory has the look of a complete, current record, and it will not always be one.

---

The live copies of these files are under `.future/cov100/` in the campaign worktree, which
`.gitignore` excludes and no remote branch tracks. This directory is a **second copy** so a lost
machine does not lose the campaign's reasoning: the verdict files, the incident log, the tools, the
closeout, and the final report.

**The source of truth for every coverage number is `docs/testing/`** (module records, the waiver
ledger, the nine review documents). That lives on this branch too, under its normal path.

Excluded from this backup on purpose:

* `*.info` (lcov captures, ~28 MB) — they are measurements of a specific revision; the records name
  which artifact each figure came from, and a stale capture is worse than an absent one.
* `baseline/` (23 MB) — the pre-campaign baseline, same reasoning.
* `__pycache__/`.

If you need to re-derive a figure: read the module record's "artifact" cell, re-run the measurement
with the recipe in that record, and compare with `reconcile.py`. The tools are in this directory.
