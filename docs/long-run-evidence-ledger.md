# Long-Run Evidence Ledger

> ([中文](long-run-evidence-ledger.zh-CN.md)) Accountability record for
> long-range goals — multi-day, multi-turn efforts driven through the
> [loop control plane](loop-control-plane.md). One entry per closed
> long-range goal.

A long-range goal spends real wall-clock time, tokens, and money across many
bounded turns. This ledger exists so that each such effort leaves behind a
factual, verifiable record: **how long it took, what it cost, what was
validated, and — just as important — what was explicitly NOT done** (accepted
residuals, waivers, scope exclusions). Entries are written at goal closure
from primary sources, not from memory.

## How to append an entry

Data sources (all local, all verifiable):

- **Turns / wall clock / spend** — the goal's run-history ledger
  `.future/loop/goals/<goal-id>/runs.jsonl` (per-run `terminal_state`,
  token deltas, `cost`, tool-call count; live per-run logs under
  `.future/loop/runs/<run-id>.live.jsonl`) and the compact projection from
  `future loop runs history --goal <id> --format json` /
  `future loop quota usage --goal <id>`.
- **Validation** — the goal's official measurement (name the tool, command,
  commit, and date), plus `future loop evidence-log --goal <id>`.
- **PRs** — `git log --grep` over the goal's working window.
- **Boundaries** — the goal's waiver / sign-off records (user gates,
  acceptance todos) and anything a reader could otherwise mistake for an
  omission.

Entry schema:

1. **Goal** — id, objective, closure date.
2. **Wall clock & turns** — first-run → last-run timestamps, span, run
   classifications (completed / error / incomplete).
3. **Spend** — tokens in/out and cost as recorded by the run-history ledger,
   with a per-turn breakdown.
4. **Validation results** — the acceptance metric, baseline → final, how it
   was verified.
5. **Explicit boundaries** — scope exclusions, accepted residuals with their
   sign-off, attribution limits of the data.
6. **Lessons** — only durable, reusable ones (details live in FUTURE.md).

---

## Entry 2026-08-12 — Workspace test-coverage goal

**Goal** `goal_4a742a954e3c` — drive the Rust workspace to ~100% line
coverage (official metric: `cargo llvm-cov` summary **Lines**, per crate +
TOTAL) via per-crate test pushes, then reconcile and accept the residual.
Closed 2026-08-12 with user sign-off on the waiver inventory.

### Wall clock & turns

- First run: 2026-08-09 23:05 (+08:00) — last run: 2026-08-12 10:21 (+08:00).
- **Span: ~59.3 hours (2 days 11 h 16 m).**
- **10 bounded runs**: 7 completed, 2 error, 1 incomplete. Every errored /
  incomplete push was retried to completion within the goal.
- 14 ledger todos (6 crate pushes, tooling + baseline, cli residual,
  acceptance, user gate, deliverables, onboarding; 2 superseded).

### Spend (as recorded by the run-history ledger)

Totals: **1,777,043,598 tokens in / 3,562,578 tokens out / ≈ $1,946.84**,
5,979 tool calls.

| # | Started (+08:00) | State | Focus | Tokens in | Tokens out | Cost (USD) | Tools |
|---|---|---|---|---|---|---|---|
| 1 | 08-09 23:05 | completed | `scripts/coverage.sh` tooling + workspace baseline (PR #138) | 2,076,302 | 31,272 | 4.39 | 94 |
| 2 | 08-10 00:06 | completed | future-rpc → 100% (PR #139) | 20,874,338 | 192,220 | 32.56 | 214 |
| 3 | 08-10 07:21 | error | future-tui push, attempt (redone in turn 4) | 837,524,085 | 1,121,310 | 880.04 | 1,824 |
| 4 | 08-10 13:03 | completed | future-tui → 100% (PRs #140, #141) | 33,663,407 | 152,044 | 44.66 | 270 |
| 5 | 08-10 14:30 | error | future-cli push (dual-executor collision; landed via the concurrent session, PRs #146, #147) | 71,805,042 | 322,458 | 89.42 | 493 |
| 6 | 08-11 03:12 | completed | future-agent → 96.75% (PR #149) | 547,793,935 | 769,548 | 577.46 | 1,450 |
| 7 | 08-11 06:09 | incomplete | future-channel push (continued in turn 8) | 167,851,903 | 549,848 | 193.99 | 842 |
| 8 | 08-11 07:34 | completed | future-channel → 100% lcov DA (PR #150) | 81,609,442 | 310,002 | 101.68 | 546 |
| 9 | 08-11 08:27 | completed | final acceptance measurement + waiver reconciliation | 13,305,340 | 95,796 | 20.62 | 220 |
| 10 | 08-12 10:21 | completed | deliverables (`coverage/`) + gate method (FUTURE.md) | 539,804 | 18,080 | 2.03 | 26 |

### Validation results

Official measurement: single workspace run of `scripts/coverage.sh` on
`main@b24d5501` (2026-08-12) — **regions 98.26% / functions 98.03% /
lines 98.80%**, **3,864 tests, 0 failures**.

| Crate | Baseline @4d3dd2fc (lines) | Final (summary Lines; missed) | PRs |
|---|---|---|---|
| future-rpc | 91.77% | 99.79% (7 summary / 0 per-line) | #139 |
| future-tui | 67.80% | 100.00% (0) | #140, #141 |
| future-cli | 42.46% | 99.83% (40 summary / 15 per-line) | #146, #147 |
| future-loop | 77.72% | 98.43% (279 summary / 191 per-line) | #148 |
| future-agent | 84.44% | 96.75% (987 summary / 700 per-line) | #149 |
| future-channel | 31.50% | 99.84% (17 summary / 0 per-line; lcov DA 100%) | #150 |
| **TOTAL** | **70.15%** (54,559/77,775) | **98.80%** | — |

Tests: 2,173 → 3,864 (+1,691). Merged PRs: 10 — #138 (tooling), six crate
pushes (#139; #140+#141; #146+#147; #148; #149; #150), #151 (flake
hardening).

Residual reconciliation: summary-missed 1,330 lines = **906 real + 424
phantom** (the summary counts per function record; per-line tools max-merge
across generic instantiations). The 906 real missed lines = 446
non-executable attribution artifacts (365 closing braces, 74 line-1 anchors,
7 comments) + ~460 defensive / dead-arm lines, cross-validated as lcov DA
zero-hit = HTML uncovered-line.

Defects found by the push: **6 real bugs** — 4 in future-loop (#148:
`doctor --agent-addr` and `benchmark run --agent-addr` nested-`block_on`
panics, monitor no-change poll appending spurious `TodoCompleted`,
`try_claim_todo` reconstruction missing the expiry arm), 1 in
future-channel (#150: gRPC client `entry_id` shadowing), 1 in future-cli
(CDP dispatch-loop subscribe race) — plus **2 flake root causes** fixed in
#151 (spawn_mock port-steal race, `resolve_future_base_url` env leak).

### Explicit boundaries

- **Scope was the Rust workspace only.** `desktop/src-tauri` (~9,753
  baseline uncovered lines), the desktop frontend, and mobile were never in
  scope for this goal.
- **The gated metric is summary Lines**; regions (98.26%) and functions
  (98.03%) are reported but not gated at 100%.
- **The 906 real residual lines are accepted, not forgotten**: waiver
  categories W1–W8 (platform `cfg(windows)` / defensive-dead arms /
  OS-failure injection / race windows / attribution artifacts / summary
  phantoms / test-mock closures / dead producers) signed off by the user via
  gate todo_01368ac862e2 on 2026-08-11; full inventory in
  `coverage/acceptance-waivers.md`. Forcing them covered would mean deleting
  defensive code, adding prod `cfg(test)` hooks, or nightly-only
  `#[no_coverage]` — explicitly rejected.
- **424 phantom summary lines cannot be printed by any per-line tool**
  (verified immovable by tests); per-line truth = lcov DA zero-hit / HTML
  uncovered-line.
- **Deliverables are local-only**: `coverage/` (lcov.info, html/,
  summary.txt, missed-lines.txt, acceptance-waivers.md) is gitignored by
  design per `scripts/coverage.sh`.
- **Attribution limits**: run-history spend covers only the 10 loop runs.
  The future-loop push (PR #148) and the future-cli residual (PRs #146/#147)
  were executed partly by a concurrent interactive session sharing the
  worktrees (dual-executor collision), so their spend is not metered above.
  Token counts are cumulative-context tokens as recorded per run, not unique
  tokens; errored/incomplete runs' partial spend is included in totals.

### Lessons

- Sanitize the environment before any official measurement (unset `CARGO*`,
  rustup toolchain bin first in PATH, isolated `HOME`) — unit-hash desync
  and auth-file leaks otherwise corrupt counts.
- A summary "missed line" is real iff ALL regions on it are zero; always
  cross-check with per-line tools before chasing lines.
- Two executors on one goal need disjoint file sets + early/often commits;
  an untracked `COORDINATION-NOTE.md` in the shared worktree worked.
- Full-suite-only flakes: re-run the failed test standalone under the same
  sanitized env — both occurrences here were real defects, not noise.

---

*Next entry: append below this line when the next long-range goal closes.*
