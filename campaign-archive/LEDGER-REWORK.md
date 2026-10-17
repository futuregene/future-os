# Supervisor handoff: the ledgers `reconcile.py` cannot verify, and why (todo_10ad23544fa1, extended by todo_57a55c087b81)

> **This is now a registered decision, not a suggestion**: `todo_6a850a842666`
> (P1, `user_gate`, role `user`) asks the supervisor to choose between the two options
> in "The change that does fix it" below. It carries the per-doc numbers, so the gate
> can be answered without re-reading this file.

Produced 2026-10-16 ~20:35 by the reconcile-hygiene unit. Every number below is
reproducible with the commands at the end; nothing here is an estimate.

## The finding in one sentence

**Eight module records have no machine-readable per-line ledger at all** — their
tables are coverage arithmetic and prose, they contain between **0 and 7**
`path:LINE` citations in the whole document, and **0** in a claim-shaped first cell —
so `reconcile.py`'s STALE half has nothing to read, its MISSING half reports them as
mostly-missing, and four of them contain no `path:NNN` citation anywhere.

This is a PLAN §5.2 compliance gap, not a regex gap: a shape fix cannot repair it,
and trying is how you invent claims (measured, see below).

## Per-doc facts   (`python3 .future/cov100/blindrework.py`)

| doc | table rows | `path:LINE` citations anywhere | in a table | outside a table | claim-shaped rows | own report present? |
|---|---|---|---|---|---|---|
| `module-agent-sandbox.md` | 248 | **0** | 0 | 0 | 0 | no (`coverage/agent-ag-sandbox-report.json`) |
| `module-agent.md` | 328 | **0** | 0 | 0 | 0 | no (`coverage/agent-report.json`) |
| `module-cli-tui.md` | 353 | **0** | 0 | 0 | 0 | no (`coverage/tui-report.json`) |
| `module-agent-session.md` | 271 | 7 | 1 | 6 | 0 | no (`coverage/agent-ag-session-report.json`) |
| `module-tauri-bridge.md` | 215 | 7 | 3 | 4 | 0 | no (`coverage/llvm-cov-tauri.json`) |
| `module-tauri-core.md` | 212 | 3 | 1 | 2 | 0 | no (`coverage/tauri-tau-core-report.json`) |
| `module-tauri.md` | 250 | 3 | 0 | 3 | 0 | no (`coverage/llvm-cov-tauri.json`) |
| `module-tauri-terminal.md` | 610 | 1 | 0 | 1 | 0 | no (`coverage/tauri-tau-terminal-report.json`) |

`claim-shaped` = a row whose FIRST cell is `path:LINE` — the only shape
`claimed_uncovered()` accepts as a per-line claim. Zero, in all eight.

**Update (todo_5b84d616508d, the final pass): 4 of the 13 docs that had no pairing are
now gated, leaving 9.** `module-cli-mcp` (4 claims), `module-tauri-appdata` (610),
`module-tauri-remote` (122) and `module-tauri-headless` (261) parsed claims but were
never *checked*, because a doc can parse and still have no pairing — the gap between
"the tool can read it" and "the gate checks it", which `--stamp` now reports as
`docs_not_covered_by_any_pairing`. Adding them was validated the same way as every other
pairing (MISSING 0; see `pairings.json` for each one's provenance and its one
reconstructed case).

The remaining 9 are:

* the eight above, and
* **`module-agent-llm.md`** — a DIFFERENT case worth stating precisely: it does have a
  parseable ledger (28 rows / 106 claims) and it is not prose, but its report
  (`coverage/agent-*.json`) is **absent from this worktree**, so no pairing can be
  sound. It belongs to the re-measure branch of the decision below, not the
  prose-restatement branch.

The reports are the second half of the problem: `coverage/` is empty in this
worktree, so even a doc-side fix could not be re-verified without re-measuring those
units.

## What the tripwires do with them today (characterisation, not a gate)

These runs use a report another unit measured for an overlapping scope, because each
doc's own report is absent. They show the *shape* of the problem, not a verdict on
the unit:

| doc | report used | uncovered in scope | mentioned | MISSING |
|---|---|---|---|---|
| `module-agent-sandbox.md` (scope `agent/src/sandbox`) | `/tmp/agb-final3.info` | 43 | **0** | 43 |
| `module-agent-session.md` (scope `agent/src/session`) | `/tmp/agent-a-s6-norm.info` | 89 | 13 | 76 |
| `module-agent.md` (scope `agent/src`) | `/tmp/agb-final3.info` | 565 | 60 | 505 |
| `module-cli-tui.md` (scope `tui/src`) | `/tmp/tui-lcov-final6.info` | 24 | 6 | 18 |

`module-agent-sandbox.md` is the clearest: **mentioned 0 of 43** — the document cites
no per-line claim in any form, so a reviewer cannot check its waivers line by line,
and the STALE half cannot either. Reproduce any row with, e.g.:

```bash
python3 .future/cov100/reconcile.py --label sandbox-char --report /tmp/agb-final3.info \
    --scope "$PWD/agent/src/sandbox" --doc docs/testing/module-agent-sandbox.md
```

## Why "just add another pattern" is the wrong fix (measured)

The rows below are real, taken from these eight docs. A looser pattern would read
each as a claim and invent a finding; `patternprobe.py` asserts they stay rejected:

```
| `llm/schema.rs` 2, `llm/adapters/anthropic.rs` 1 | 3 | `expect_*_rejects_other_protocols` … |
      module-agent.md: a COUNT per file — a looser rule reads "line 2" and "line 1"
| `agent/src/logfile.rs` | 227/229, 2 | 227/229 | 2 | 99.13 |
      module-agent-session.md: coverage arithmetic, slashed
| `desktop/src-tauri/src/runtime.rs` | unreachable-by-construction | Lines 44-49 are … |
      module-tauri-core.md: the range is prose inside a LATER cell
| `desktop/src-tauri/src/windows_power.rs` (waived in §5.1 — listed for size only) | 42 | … |
      module-tauri.md: cell 1 is path + prose, the count is cell 2
| `future-cli` main / `bin.rs` / `event_wire_contract.rs` | 2 / 11 / 2 | ok |
      module-cli-tui.md: several files and counts in one cell, slashed
```

## The change that does fix it (doc-side, per doc)

This is the decision the supervisor holds (`todo_6a850a842666`): **either** those units
re-measure and give each residual line its own machine-readable row, **or** their
records are restated as prose-only with an explicit note that `reconcile.py` cannot
check them — so a reviewer knows which records carry a machine check and which do not.
The first option's row shapes, as `module-goal.md` §11.5 and `module-tauri-shell.md` §4
already write them:

```
| `path/to/file.rs:123` | <category> | <mechanism in prose> |
| `path/to/file.rs` 456, 789 | <category> | <mechanism> |     <- the spaced shape, also accepted |
| D12 | `desktop/src/…/File.tsx:267` | <category> | <mechanism> | <- tag-first, also accepted |
```

Then the unit can run the PLAN §5.2 invocation and get a real MISSING/STALE pair, and
the pairing joins `pairings.json` where `staletable.py --gate` will check it. This is
doc work in someone's round-2 section, i.e. a supervisor scheduling decision, not
something the tooling unit should do unilaterally (it is outside this unit's write set,
and it changes what those records claim).

## Also to schedule

* **Superseded ledgers kept in the same file** — the STALE half now reports disagreements
  in 5 of 13 pairings, and two of them are this: `module-cli.md` keeps §4 (measurement
  S3; its own note says *"the authoritative final ledger is §4b"*) beside §4b, and
  `module-agent-rpc.md` keeps the round-1 §6 beside the round-2 §0.5. Neither is a
  defect in the rows; the invocation is too wide. Options: require the round-2
  invocation to name its authoritative `--section` (the tool prints which section the
  covered rows came from), or delete/annotate the superseded table. Detail and the
  exact commands: `STALE-CHECK.md`.
* **`module-agent-rpc.md`** (revision 20:32:07, while its unit was editing it) — its
  documented invocation slices `unit \`AG-A\`` (§0) and now reads **347/347 mentioned,
  0 MISSING, 0 STALE** over 280 claims, so this one is closed. §6 is the superseded
  round-1 ledger and reads 99 covered / 144 stale against the same report: expected,
  not a finding.
* **`module-channels.md`**: 29 of its 49 §Round-2 rows carry trailing prose after the
  line list (`` | `providers/imessage.rs` 464-498 (33 lines) | … | ``) and are rejected
  by design; 20 rows / 15 claims parse. Its 3–5 covered rows sit in §10.2 **"What is
  covered now, and by which test"**, i.e. rows that say the line is covered — the same
  shape as the channels policy page's "covering test of every fallen row" table
  (6 rows). A doc-side decision: drop those rows from the ledger, or mark them.
* **`module-tauri-headless.md:512`** records "359 mentioned, 0 MISSING, 0 stale". That
  0 was vacuous (0 rows parsed before this week's fix). It now parses 42 rows / 261
  claims, but names neither report nor scope, so re-verifying it needs a re-measure.

## Reproduce

```bash
python3 .future/cov100/blindrework.py                 # the per-doc table above
python3 .future/cov100/patternprobe.py                # 24 accept/reject cases, exit 0
python3 .future/cov100/rowscan.py --samples           # same, as whole-doc counts
python3 .future/cov100/staletable.py                  # the 13 pairings that DO verify
```
