# Self-test fixture for `reconcile.py`'s STALE split (do not treat as a report)

Every line in the campaign's real pairings comes out `COVERED 0` / `ABSENT n`, so
the *COVERED* branch — the only actionable half — would otherwise never be executed.
This doc plus `fixture-lcov.info` drive all three states deliberately:

| claim | record in the fixture | expected |
|---|---|---|
| `demo.rs:10` | `DA:10,0` | not STALE (ledger agrees with the report) |
| `demo.rs:11` | `DA:11,7` | **COVERED** (a real disagreement) |
| `demo.rs:12` | `DA:12,3` | **COVERED** |
| `demo.rs:13` | no record | ABSENT (not evidence) |
| `gone.rs:5` | `DA:5,4` — and `gone.rs` has NO zero-count line | **COVERED** — the F.35 blind spot: a file whose uncovered set is EMPTY must still be checked |

Expected output: `uncovered lines=1`, `mentioned in fixture-doc.md: 1`,
`MISSING 0`, `STALE 4`, `COVERED 3`, `ABSENT 1`, **exit 0** — a real disagreement is
a finding for a human, not a gate failure, and `selftest.py` asserts that too.

The `gone.rs:5` row is the regression control for a defect found by the AG-A unit at
segment 16: the STALE half resolved claims against the report's *uncovered* map, so a
file whose uncovered set had become EMPTY was treated as "cannot tell" and SKIPPED —
exactly the state a coverage campaign produces, and a false clean in the half of the
check that exists to catch a ledger overstating its residual.

| `demo.rs:10` | self-test | a zero-count line the ledger agrees with |
| `demo.rs:11` | self-test | a covered line the ledger still claims |
| `demo.rs:12` | self-test | a covered line the ledger still claims |
| `demo.rs:13` | self-test | a line the report has no record for |
| `gone.rs:5` | self-test | a covered line in a file with NO uncovered lines (F.35 blind-spot control) |

> The rows below are kept after the fact on purpose: two of them describe lines the
> fixture report shows as covered. This sentence is what a `covered_justification`
> quote is matched against in `selftest/pairings-positive.json`.
