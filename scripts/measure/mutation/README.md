# Mutation sampling — runners, evidence and roll-up

The `cargo-mutants` samples taken for the coverage campaign live here, together
with the scripts that produced them and the evidence that is kept. The findings
themselves are written up in
[`docs/testing/mutation-report.md`](../../../docs/testing/mutation-report.md).

Committed here (the per-run `out-*/` trees are written by the runners and are
**not** committed — see `.gitignore` next to this file):

| path | what it is |
|---|---|
| `run-*.cmd` | the six Windows drivers that launched the detached `cargo-mutants` runs (default copy mode, `--gitignore true`, `--timeout 300`, never `--in-place`, no `CARGO_TARGET_DIR`) |
| `analyze-queue.py` | per-mutant attribution table: which tests failed in each mutant's *own* log |
| `verify-attribution.py` | verdict hygiene: catches whose only failing test is on the pre-fix flake list |
| `dump-fails.py` | failing-test list for selected mutants |
| `compare-runs.py` | primary run vs survivor re-check comparison |
| `summary.json` | the machine-readable roll-up (schema `future-cov100-mutation-summary-v1`): scope, counts, per-survivor adjudication, revision attestation |
| `out-*-attribution.txt`, `out-*-hygiene.txt` | the attribution/hygiene dumps the report quotes |
| `out-*-baseline.log` | the three *failed* baseline attempts (arg plumbing, disk-full) kept as evidence for the report's lessons |
| `revision-hashes-prerun.txt` | SHA-256 attestation of the three mutated files, recorded before the runs |
| `policy-defaults-finding.md` | the extracted raw evidence behind the `channels/src/policy.rs` section — the only findings note here; everything else is tooling or data |

## Re-run

From the repository root, on Windows:

```
scripts\measure\mutation\run-queue.cmd
```

The drivers `cd` to the repository root themselves and write `out-<run>/` next to
this file. `TEMP`/`TMP` must be on a volume with room — `cargo-mutants` copies
`target/debug` into its scratch tree (~1 GB per job) and the first attempt died
disk-full with `os error 112`. Then attribute the verdicts:

```
cd scripts/measure/mutation && python analyze-queue.py out-queue/mutants.out
```

## The big logs

`out-*-baseline.log` (~400 KB each) are the failed baselines of the arg-plumbing
attempts (`--skip=` is rejected before cargo's `--`; two test-name positionals are
rejected). They are kept because the report's "arg plumbing" section is derived
from them; they are not part of any score.

## Conventions

* Every verdict must be derived from the mutant's own log via its run's
  `outcomes.json` — the `_NNN` suffix on a log file name is a run-order dedup
  counter, not the `--list` order.
* A catch counts only if the failing assertion can observe the mutated line;
  `verify-attribution.py` reports the ones that rest on the pre-fix flake list.
* Never set `CARGO_TARGET_DIR` and never use `--in-place`.
