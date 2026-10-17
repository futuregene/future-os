# Verification (adversarial) units — the pattern

The campaign's "100 %" claim rests on two kinds of evidence: tests that execute a
line, and **waivers** that assert a line cannot be reached. A waiver is a claim,
and the repo's own policy says so:

> The gate checks STRUCTURE only. Whether each reason is true is the reviewer's
> job, and rev-rust / rev-fe are tasked to falsify them by counterexample — a
> waiver that survives is evidence, a waiver that falls is rework.
> — `docs/testing/waiver-ledger.md`

So every unit that registers waivers gets a **separate** adversarial review unit,
owned by a different agent, whose deliverable is a verdict per waived line.

## Rules for a review unit

1. **Only frozen modules.** A waiver cites `file:line`, and any later edit shifts
   the numbering (a documented trap in this repo). Review a module only after its
   coverage unit has stopped writing, and re-read the line numbers from the
   module record at review time.
2. **The reviewer never edits what it reviews.** Its only write target is its own
   report (`docs/testing/review-frontend.md`, `docs/testing/review-rust.md`,
   …). Counterexample tests are built in a **scratch checkout** the reviewer
   creates itself, never inside the campaign worktree.
3. **Build the scratch checkout with `git archive`, not `git worktree add`:**

   ```bash
   mkdir -p /tmp/rev-x && git archive <base-sha> | tar -x -C /tmp/rev-x
   rsync -a --delete <module>/src/ /tmp/rev-x/<module>/src/     # current sources
   ln -sfn "$PWD/node_modules" /tmp/rev-x/node_modules          # as needed
   ```

   `git worktree add` mutates the shared `.git/worktrees/` and leaves an entry
   every peer has to reason about — the same shared-state hazard that cost this
   campaign one cleanup. `git archive` writes nothing into the repository. (This
   came from the REV-FE unit, which chose it over the instruction it was given;
   it is now the prescribed technique.)
4. **Verdicts are falsifiable, not opinions.** Per line, one of:
   * `SURVIVED` — state the specific input you constructed and how it failed to
     reach the line;
   * `FELL` — give the counterexample, the exact command, and the covered-line
     evidence (the tool output showing the line is no longer `DA:0`);
   * `UNVERIFIABLE` — state precisely what you would need (a host, a fixture, a
     refactor access to a private symbol).
   "It looks unreachable" is not a verdict.
5. **Publish the probe sources and their hashes**, plus the exact commands, so the
   supervisor can verify the review rather than trust it. Keep the scratch tree
   alive until the review is accepted (the supervisor will hash a probe or re-run
   one), then it may be deleted.

   5a. **Embed the source of every probe that carries a verdict directly in the
   record**, in a fenced block, in addition to the hash. Hash-only does not survive
   the tree: if the scratch dir is deleted (or clobbered by another writer) a hash
   cannot be checked and cannot be re-run, and the verdict becomes unfalsifiable.
   REV-DESK segment 3 did this for `D54` and it is now the standard.
   5b. **A hash must verify against the bytes the supervisor can see.** In this
   campaign one probe hash did not match the source embedded in the same document
   (the tree had been deleted), while the readout instrument's hash did match
   byte-for-byte — so the discipline works when the bytes are reachable. If a hash
   cannot be verified, say so in the record rather than leaving a bare hash that
   reads as evidence.
   5c. **Use a unique scratch directory per segment**, and never reuse
   `/tmp/rev-<module>` across segments or runs: that path has had more than one
   writer, and one segment lost six probes to a clobber while finding a probe it
   did not author in their place (the leftover was a dead run's file, not a peer's
   - but the segment could not tell the difference, which is the real hazard).
6. **A fallen waiver is not a failure of the review.** It is rework for the
   owning unit; the reviewer reports it and the supervisor creates the follow-up.
7. **Correcting a mechanism is a result too.** A waiver that survives with a
   *wrong stated reason* is worth fixing in the record: it tells the next author
   which refactor would revive the line. REV-FE produced four such corrections
   while returning 13/13 SURVIVED.
8. **Start where the author says to.** The coverage worker names its own weakest
   waivers in its handoff; attack those first, then sweep the rest.
9. **Before accepting an `unreachable-*` tag, check whether a *named test* already
   claims that arm.** The signature is: a test whose name matches the arm exists and
   passes, the line reads `DA:0`, and the author recorded "unreachable". That is the
   trap TAU-B found (GOTCHAS.md, "一个'名字对、断言也过、但什么都没覆盖'的测试"): the
   fixture was *weaker than the app's own contract* (a minimal `list_sessions` row
   cannot decode into `SessionSummaryPayload`, so the list came back empty and the
   function returned at its empty-list guard before the loop), so two tests named for
   the import panic/error arms passed while covering nothing. Detection is three
   steps and cheap: (a) suspect the TEST first, not "unreachable"; (b) assert
   *directly* on the reader the test depends on (`list_agent_sessions()` is non-empty
   for this fixture) - the failing probe is the proof; (c) fix the fixture to the
   shape the app actually requires and watch the lines go 0 -> >0 (measured:
   `import.rs` 448/727/730/731/732 went 0/0/0/0/0 to 2/4/2/2/2). In the other
   direction, a fixture *more permissive* than the app is the false-green class this
   campaign already recorded.


## Instances

**Outcomes as of 2026-10-17.** The `fell` counts are the reviews' own tallies; where a number decides
what the final report may claim, I re-derived it (each one is noted in the verdict note recorded through
`future goal delivery record`, and the report file carries the per-row verdict table).

| review unit | module | waivers reviewed | report | outcome |
|---|---|---|---|---|
| REV-FE | mobile | 13 | `docs/testing/review-frontend.md` | **0 fell** — 13/13 survived, 3 as type/visibility boundaries. The strongest ledger in the campaign |
| REV-DESK | desktop frontend | 61 | `docs/testing/review-desktop.md` | **3 fell**; a fourth (D41) disputed and reworked by its owner |
| REV-TUI | tui | 24 | `docs/testing/review-tui.md` | **11 fell** |
| REV-CH | channels | 24 | `docs/testing/review-rust.md` | **16 fell** — the first review, and the one that set the calibration |
| REV-CLI | cli | 33 rows / 119 lines | `docs/testing/review-cli.md` | **10 rows / 76 lines fell** — I verified its strongest fall myself (`main.rs:31` is reachable on macOS; the ledger's "impossible" was false) |
| REV-S6 | goal S6 fixtures | the segment's own fixtures | `docs/testing/review-goal-s6.md` | **0 fell** — author-built fixtures survived; **4 record corrections** |
| REV-AGENT | agent (AG-A/AG-B) | 469 residual lines across the two ledgers | `docs/testing/review-agent.md` | **5 rows / 33 lines fell**, including one that **overturned a ruling of mine** (`cli.rs:590` declared "cost-prohibitive", then covered in ~5 s) |
| REV-GOAL | orchestration/tasks/goal | 82 | `docs/testing/review-goal.md` | **6 rows / 8 lines fell**; its `compat.rs:270` finding (a dangling symlink as a lock) is the cleanest mechanism of the campaign, and I re-derived the `MAX_LOCK_ATTEMPTS = 512` correspondence myself |
| REV-GOAL-2 | goal rework (second pass) | 15-row verdict table | `docs/testing/review-goal-2.md` | **1 declared falsifiability gap** (315-316 covered but not discriminated), 4 defects, 1 stated mechanism **rejected** by its own measurement |

**Still open** (all doc-only writes, gated on the reviewed module being frozen): REV-TAURI-SHELL,
REV-TAURI-HL, REV-TAURI-APP (one unit owns all three report files), REV-DESK segment 4, VERIFY-CLI-R2.

**What these nine establish, and the number the final report should carry:** of the residual lines
**authored as waivers**, roughly **half to two-thirds fall** when someone tries to reach them with the
inputs the waiver said could not exist (REV-CH 16/24, REV-CLI 10/33 rows, REV-TUI 11/24, REV-AGENT
5 rows/33 lines, REV-GOAL 6/82, REV-DESK 3/61), while **fixtures a unit built and controlled itself
survive** (REV-S6 0). **And "the supervisor accepted it" is not evidence of correctness** — REV-AGENT
overturned one of my rulings and REV-GOAL-2 rejected a mechanism I had not questioned. A waiver that has
not been attacked is a hypothesis, not a waiver; that is why the category table in the final report must
say which rows were attacked and by whom.

## Dispatch check (learned the hard way)

A flake unit's or a review unit's `--blocks` gate names *specific todos*, and units
here deliver in **segments**: when a coverage unit finishes a segment it creates a
successor and keeps writing the same files, so the gate opens while its owner is
still writing. Before dispatching, confirm on the live board that every todo whose
`required_write_scope` overlaps the unit's scope is **done** — not merely that the
named blocker is. Two real cases from this campaign: `flake-mobile` was clear once
mobile finished (no successor), while `flake-agent` and `flake-cli` had to be
re-pointed at their units' live successor todos.
