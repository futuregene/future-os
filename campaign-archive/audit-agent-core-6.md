# Supervisor audit — `docs/testing/module-agent-core.md` §6 ledger vs its pinned report

Report checked: `/tmp/agb-final3.info` (the one §6 pins; residual 212 in 21 files).
Method: a checker that resolves every claim per **full path inside `agent/src`** and refuses to
guess when a basename is ambiguous. `reconcile.py` (after the ledger-row-shape fixes) agrees.

Result: **COVERED = 5** (the ledger says uncovered, the report shows a hit) and **ABSENT = 2**
(no `DA:` record at all). Everything else in the section matches.

| doc line | the row cites | what the report actually has |
|---|---|---|
| 380 | `llm/mod.rs`: 179, 201, 401, 526, **1355, 1516, 1607, 1949** (count 8) | 179/201/401/526 uncovered as cited. **1355 → count 1, 1516 → count 2, 1949 → count 1 (covered).** **1607 has no `DA:` record.** The file's real uncovered set is `32,36,179,201,268,401,526,1297,1458,1549,1891` — and you cite `1297/1458/1549` correctly at doc line 504 and `1891` at doc line 431, so the right numbers are already in the document. |
| 363 | `run_loop.rs`: 478, 703, **4957, 4990** (count 4) | 478 and 703 uncovered as cited. **4990 → count 6 (covered).** **4957 has no `DA:` record.** The real set contains `4954` and `4987`. |
| 364 | `run_loop.rs`: **2005** (count 1) | **2005 → count 6 (covered).** The real set contains `2002`. |

## Why the regeneration assertion did not catch this — the part worth fixing for good

The table was regenerated with an assertion of the form
`rows: 21 claimed: 212 report: 212`, i.e. it compared **counts**. Count equality is preserved by a
wrong **set**: replacing 2005→2002, 4990→4987, 1355→1297, 1516→1458, 1607→1549, 1949→1891 keeps
every total at exactly 212. This is the same failure mode the repo already records from an earlier
campaign (a table that wrote 7 where 8 was meant and 10 where 9 was meant, with the total still
reading 22, so nobody noticed).

**Fix:** assert on the sorted **set** of `(path, line)` pairs — the table's set must equal the
report's uncovered set — not on the size. Then a swapped number cannot pass, and a cited line that
is covered (or has no record) fails loudly at regeneration time.

## Disposition

* Correct the three rows to the report's numbers, **or** state (with evidence) that this table pins a
  different revision than `/tmp/agb-final3.info`. Either answer is fine; the table's numbers must
  match the revision the table names.
* The two ABSENT lines are not disagreements — they simply have no per-line record (a blank line, a
  comment, or a line belonging to another revision). They still need either the right number or a
  category that does not claim a per-line record.
* Re-run the set-based check and report its output, plus the PLAN §5.1 gate exit codes.

Supervisor note on why this is being raised as an audit item: I accepted this unit's delivery on the
strength of reproduced numbers + a tripwire that reported 0 MISSING. Both of those were true and
both were insufficient — MISSING only checks that every uncovered line is *mentioned somewhere*, and
a count-based assertion only checks the size. This finding is what a per-full-path set comparison
adds, and it is now part of the closeout checklist.
