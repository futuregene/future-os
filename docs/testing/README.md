# Testing task records

These pages are the task records of the per-module coverage campaign: how one
module or app subtree was measured, which lines remained uncovered, which
waivers were registered and why, and which acceptance gate was run. They are
agent work records, not user-facing product documentation — and they are the
only `docs/` subtree deliberately exempted from the bilingual pairing rule
(`PAIR_EXEMPT` in `scripts/docs/check-docs.py`): a Chinese pair is optional
here, and most records have none, though this index and the release-acceptance
records do.

## What is here

| record | what it holds |
| --- | --- |
| `module-*.md` | one record per Rust crate or app subtree: run identity, measurement command, uncovered lines, per-dimension evidence, waiver ledger, open items |
| `desktop-*.md` | the same for the desktop subtrees (agent, shell, panels, settings, packages) |
| [review-rust.md](review-rust.md), [review-frontend.md](review-frontend.md) | adversarial reviews that tried to falsify the waivers |
| [weak-test-audit.md](weak-test-audit.md), [disabled-test-audit.md](disabled-test-audit.md), [weak-test-fixes.md](weak-test-fixes.md) | audits of weak and disabled tests, and the fixes that followed |
| [mutation-report.md](mutation-report.md) | the mutation-testing campaign: scope, per-mutant verdicts, survivor adjudication (runner and evidence: `scripts/measure/mutation/`) |
| [platform-coverage.md](platform-coverage.md) | the authoritative platform for each unmeasured line |
| [dimension-matrix.md](dimension-matrix.md) | index of the boundary / error-path / concurrency / property / platform-cfg / serialization evidence quoted from the module records |
| [waiver-ledger.md](waiver-ledger.md) | the waiver registry and the category policy |
| `flaky-*.md` | one record per suite: the investigations that traced an intermittent failure to its cause, the mechanism, and what closed it |
| `headless-*.md`, `headless-*.json` | headless release acceptance: the manual acceptance scripts, the unsigned-download results, and the raw CI run payloads (`headless-ci-*.json`, `headless-baseline-*.json`) those two pages cite |
| [real-network.md](real-network.md) | the suite that exercises a real NATS server and real network paths, and what it needs to run |
| [remote-client-fixtures.md](remote-client-fixtures.md), [remote-interop.md](remote-interop.md) | the remote-stack fixtures and the cross-implementation interoperability run |
| [remote-package-manual-acceptance.md](remote-package-manual-acceptance.md) | the manual acceptance record for the extracted remote packages (has a Chinese pair) |
| [agent-database-fixtures.md](agent-database-fixtures.md) | the released-schema SQL fixtures and the upgrade tests that replay them |

Section §1 (`## 1.`) of each record states the run identity (machine, revision,
suite state) and the exact measurement command. To re-run a gate, copy that
command from the module's §1 — do not reconstruct it from memory: the flags are
load-bearing (for example `--no-fail-fast` and the private `CARGO_TARGET_DIR`).

## How these records are used

- Source comments cite specific sections, so a section number is a reference
  target — keep it stable or keep its meaning. Examples:
  `orchestration/goal/src/webui/server.rs` and `orchestration/goal/tests/*.rs`
  cite `module-goal.md`; `tui/src/app.rs` cites `module-tui.md`; desktop tests
  cite `desktop-*.md`.
- The campaign's gate checked structure only: every file with uncovered lines
  had to name a waiver category on the same line as its path. Whether a reason
  was true was the reviewers' job — an unfalsified waiver counted as evidence,
  a fallen one was rework.
- The waiver categories and the policy around them
  (`unreachable-by-construction`, `unreachable-in-this-environment`,
  `attribution-artifact`, `platform-unmeasured`; `OPEN` / `not waived` always
  counts as not waived) are defined in [waiver-ledger.md](waiver-ledger.md);
  [platform-coverage.md](platform-coverage.md) is authoritative for
  platform-specific lines.

Each record's body keeps only what is still current — live waivers, open items,
gate commands, operating notes. Superseded snapshots move to
`docs/archives/testing/` and leave a one-line pointer in place.

## Related

- User-facing documentation index: [docs/README.md](../README.md)
- Coverage and profiling tooling: `scripts/measure/`; the durable ratchet gate
  is `scripts/measure/coverage.sh --check` with
  `scripts/measure/coverage_ratchet.py`
- The documentation gate and its rules: `scripts/docs/check-docs.py`
