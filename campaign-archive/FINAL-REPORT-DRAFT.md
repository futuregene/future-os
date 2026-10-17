# Coverage campaign — FINAL REPORT (DRAFT)

**Status: DRAFT, written 2026-10-17 ~12:35 while units are still running. It is NOT the frozen figure.**
The frozen pass (`measure-final.sh` after the tree is quiet) supersedes every number below; this draft exists so
the campaign's conclusions survive a context compaction and so the relay can review the shape before the freeze.

## 0. What is being claimed, and what is not

**Claimed:** every module listed below has either (a) `DA:0 = 0` under the campaign's metric, or (b) a
**per-line ledger** that accounts for every uncovered line with a category and a mechanism, and (c) an
**adversarial review verdict** for the waivers where one has been done. Numbers are each module's own
self-report against its own pinned artifact; **they are NOT summed** (see §4).

**Not claimed:** a single repository-wide percentage. The objective is per-scope, and three of the records are
**slices of a parent's scope**, so a total would double-count.

## 1. The metric and how it is read (口径 A)

Per-line lcov `DA:0` (or, for the JS modules, per-line Istanbul with the statement-all-zero rule) — **never
`summary.lines`**, never "statements". This mattered: one record's headline figure (396) turned out to be
`summary.lines`, and its own `DA:0` view was **115** (INCIDENTS F.87). Tools exit non-zero on a zeroed/partial
report so a stale artifact cannot be published as a number.

## 2. Figures of record (each self-reported; the gate's view is the pairing check)

| module | uncovered | split / class note | artifact | gate verdict |
|---|---|---|---|---|
| goal / tasks / packages | **77** | 11 platform-unmeasured + 9 unreach-env + 21 unreach-construct + 36 attribution | `/tmp/r2d-lcov.info` | PASS |
| agent-core (AG-B) | **213** | 2 categories under correction by REV-AGENT | `/tmp/agb-p6.info` | PASS |
| agent-rpc (AG-A) | **202** | `cli.rs:590` now COVERED (my ruling was overturned by REV-AGENT) | `/tmp/agent-svc-rework-norm.info` | PASS |
| agent-llm (slice of agent-core) | **115** | `DA:0`, **not** the record's 396 (`summary.lines`) | record's own table | n/a (slice) |
| channels | **8** | 16/24 waiver rows fell under review | `channels` artifact | PASS |
| cli | **52** | 4c/S12 ledger; 10 of 33 rows fell under review | `/tmp/cli-r2s4-final-lcov.info` | PASS |
| cli-mcp | **4** | disjoint from cli by its own declaration | `/tmp/cli-r2s4-final-lcov.info` | PASS |
| desktop frontend | **59** | 55 waived + 4 open (ImagePreview decision pending) | `desktop/coverage/coverage-final.json` | PASS |
| mobile | **13** | 13/13 survived review (the strongest ledger) | `/tmp/mobile-lcov.info` | PASS |
| tui | **13** | 11 of 24 fell under review | `/tmp/tui-r2-crate.info` | PASS |
| remote (tauri) | **450** | three trees: `remote` 331 + `remote_host` 106 + `remote_peer` 13; 450 claims / 450 uncovered / 0 STALE after the F.94 repair | `/tmp/tauri-tau-remote-lcov4.info` | PASS |
| tauri-appdata | **378** | 368 WAIVED + 10 OPEN; four blocked families registered under my ruling; two rows closed by count after the ruling was refuted (one of them refuted one of MY mechanisms, F.94-adjacent) | `/tmp/appdata-taub-s23-normalized-lcov.info` | PASS (378/378, MISSING 0) |
| **tauri-shell** | **107–127 pending** | was published as 150; REV-TAURI-SHELL falsified 90/217 rows, **43 of them uncovered in the healthy artifact too**. 75 lines remain `host-state-blocked` (environment-attributed, not waivers) | `/tmp/tauri-shell-lcov-c8union.info` (healthy) | **FAIL** (record defect ticketed) |
| tauri-headless | **300** | was 359; the segment-3 tests covered 59. All 27 formerly-unclassified lines crosswalked in §4.4. The record's §4 heading discusses 1205 for `desktop/src-tauri` as a whole, of which 852 are new `#[cfg(test)]` harness lines added by REAL-NET | `.future/cov100/tauri-tau3-union-lcov.info` | PASS (300/300, MISSING 0) |

## 3. Independent review — the campaign's most important result

Nine adversarial reviews attempted to falsify the waiver tags by constructing the inputs the tags said could not
exist. **Roughly half to two-thirds of author-assigned waivers fell**:

| review | rows attacked | fell |
|---|---|---|
| REV-FE (mobile) | 13 | **0** |
| REV-DESK (desktop) | 61 | 3 |
| REV-TUI | 24 | 11 |
| REV-CH (channels) | 24 | 16 |
| REV-CLI | 33 | **10 rows / 76 lines** |
| REV-S6 (goal fixtures) | the segment's own | **0** (fixtures a unit built and controlled survived) |
| REV-AGENT | 469 lines | 5 rows / 33 lines, **including one of MY rulings** |
| REV-GOAL | 82 | 6 rows / 8 lines |
| REV-GOAL-2 | 15-row table | 4 defects, 1 declared falsifiability gap |
| **REV-TAURI-SHELL** | 217 | **90** |

**The conclusion the report must carry:** a waiver that has not been attacked is a hypothesis. **"The supervisor
accepted it" is not evidence of correctness** — two reviews overturned my own rulings, and the largest review
falsified 41% of a module's residual. Fixtures a unit built and verified itself survived in every case.

## 4. Why the figures are not summable

`module-tauri-headless` (359) and `module-agent-llm` (115) are **slices** of `module-tauri` and `module-agent-core`
respectively; adding them would double-count ≥474 lines. `module-cli-mcp` (4) is **disjoint** from `module-cli`
because that record explicitly excludes the two MCP files, so it may be added. The ledger carries a **DO NOT SUM**
note.

## 5. Run identity

- Agent build: restarted **10:51:39** (binary installed 10:44). **No published artifact postdates it** — the newest
  is 10:15 — so no number spans two orchestrator builds; the frozen pass re-measures everything anyway.
- Coverage numbers are properties of the **source tree**; the revision each artifact describes is named with it.
- Environment: macOS aarch64, `cargo-llvm-cov 0.8.7` (round-1 records said 0.9.1 — the wrapper differs between
  rounds).

## 6. What is still open (this is the list the freeze must clear)

1. **The frozen pass has not been run.** `endgame.sh --check` reports BUSY (pairings stale, stamp stale). Run it
   once the writers stop; it re-measures src-tauri and mobile after merging `origin/main`.
2. **Two pairings FAIL** and both are record defects with tickets: `tauri-appdata` (live writer, fix at freeze) and
   `tauri-headless` (27 unclassified lines, `todo_c150712617cf`).
3. **tauri-shell's figure is a range** (107–127) until `todo_f65783eb7365` lands the review's fixtures and measures.
4. **REAL-NET is not done** (`todo_f5453bf671b4`): the user requires all functionality exercised in a **real
   network** environment. Coverage was measured against an in-process fake broker; `nats-server` is installed and
   40 `#[ignore]`d tests name a real broker as their blocker. **The campaign cannot be called complete with this
   open.**

   *Status: DONE 2026-10-17, with one correction worth carrying.* `docs/testing/real-network.md` (404 lines) and
   `desktop/src-tauri/src/remote/real_network.rs` (two-process harness) record it: real broker, real Noise/NKey,
   1.2 MB upload and 960 KB download byte-identical, a prompt through a real Agent, and the failure paths.
   Its report first claimed a **product defect** (coalescing grows the lane at the production 100 ms window);
   a follow-up measured that claim and **refuted it** — the lane was byte-identical, the journal's chunks were
   100.9-110 ms apart against a 100 ms window, and `Coalescer::flush` returns a lone fragment unchanged. The
   defect was the measurement's strict `coalesced < today` assertion. Both documents now say so, and a
   burst-paced journal demonstrates the window merging 2.30x when fragments really arrive inside it. **This is
   the campaign's clearest example of a published conclusion being wrong and being corrected in place** — the
   first reading was a guess made necessary by a marker printed after the assertions.
5. Open flake/review work: REV-TAURI-HL, REV-TAURI-APP, VERIFY-CLI-R2, F-CLI-6, F-TUI-10, TAU-D4, CLI-ENVGUARD,
   plus F-SWEEP (the comprehensive flake verdict, owned by the supervisor).
6. **Two user decisions still standing:** the `ImagePreview` space-drag gesture (implement or delete the dead
   state), and whether to accept the 4096-byte rule-pattern bound as-is.

## 7. PR plan — SUPERSEDED: one PR, opened and merged by the supervisor

The user has since said directly: open the PR(s) and merge them when the work is done. So this
becomes **one PR** for the campaign rather than the per-module split below (the split would
require rewriting 440 commits into separate branches, and every module's evidence lives in the
same `docs/testing/` tree). The branch is built from `origin/main` with `.future/**` excluded:
that directory is gitignored and untracked upstream, and 11.1 M of this branch's 12.0 M added
lines are the agent's own run journals, which must not enter a PR.

*Historical per-module plan, kept for the record:*


One PR per module group, each with the full pre-PR pass in its worktree, squash-merged on the user's word:

1. `test(coverage): desktop frontend (vitest)` — desktop/src + docs
2. `test(coverage): mobile (jest)`
3. `test(coverage): channels` — includes the iMessage date-bomb fixture fix
4. `test(coverage): cli` — includes the EnvGuard conversion and the browser wall-clock sweep
5. `test(coverage): tui`
6. `test(coverage): agent` — AG-A + AG-B + the llm metric correction
7. `test(coverage): orchestration + packages` (goal, tasks, rpc)
8. `test(src-tauri): app-data / shell / headless / remote` — likely split four ways by module record

## 8. How to verify any number here

Every row names an artifact. `python3 .future/cov100/reconcile.py --report <artifact> --scope <scope> --doc
<record> --section <needle>` prints `mentioned / MISSING / STALE` for it; `staletable.py --gate --strict` prints
the per-pairing verdict table this draft's §2 is built from. If this draft and the tools disagree, **the tools
are right** — this document is a summary written under time pressure, and the campaign has caught three summary
defects of mine already (F.87's metric, F.89's stale briefs, F.93's survivor).
