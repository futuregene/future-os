# Campaign archive (backup, not a source of truth)

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
