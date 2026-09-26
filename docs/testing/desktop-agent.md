# Desktop TS — `src/features/agent` subtree (coverage group `d-agent`)

Handoff + waiver ledger for the **d-agent** slice of the desktop TypeScript
module. Scope: `desktop/src/features/agent/` (44 measured files). This file is
also the waiver document the `js-module group:d-agent` gate consults.

**Status: gate-green.** The gate command in §1, run with the concrete id
`d-agent`, exits 0: every file is at 100% of coverable lines or is waived in §5 with a
policy category, and no reachable line is left uncovered without a waiver.

## 1. Run identity and exact measurement

```bash
# from the goal cwd (D:/future-os/.worktrees/cov100)
cd desktop && npx vitest run --coverage \
    --coverage.reportOnFailure=true \
    --coverage.reportsDirectory=coverage/d-agent
python .future/cov100/verify.py js-module group:d-agent 99.99 \
    docs/testing/desktop-agent.md \
    desktop/coverage/d-agent/coverage-summary.json
```

* Vitest 4.1.11 / v8 provider; `desktop/vite.config.ts` `include` unchanged.
* `--coverage.reportOnFailure=true` is required because other workers' suites in
  the shared checkout are red while they work; without it Vitest writes **no**
  report at all on a failing run (this cost one wasted full measurement).
* The run covers the whole `src/` tree, so a file's number here includes tests
  written in *other* subtrees that happen to render it; that is intentional —
  the group filter selects measured files by path, not by test-owner.
* Report: `desktop/coverage/d-agent/coverage-summary.json` (this worker's own
  file; never `coverage/coverage-summary.json`, which the supervisor owns).
* **The 4th argument must be a literal path — never `coverage/<agent-id>/`.**
  `<` and `>` are cmd.exe redirection operators, so a command carrying the bracketed
  placeholder never reaches Python with the intended arguments. **Re-measured exactly,
  segment 28** (argv-dumping script, scratch dir inside `coverage/d-agent/`, probe removed
  afterwards):
  * **No file named `agent-id` in the CWD** → cmd fails before launching Python:
    `The system cannot find the file specified.`, exit 1, **empty stdout**. This is the
    failure mode the harness has been reporting.
  * **A file named `agent-id` present** → cmd *does* launch Python, and the token
    `desktop/coverage/<agent-id>/coverage-summary.json` is parsed as **three** pieces:
    the argument `desktop/coverage/`, stdin redirected **from** the file `agent-id`, and
    stdout redirected **to** `/coverage-summary.json` — which on Windows resolves to the
    **drive root** (`D:\coverage-summary.json`). The script therefore received
    `argc = 6` with `argv[5] = 'desktop/coverage/'` (a *directory*, so `read_text()`
    raises `IsADirectoryError`), and **its output vanished to the drive root instead of
    the console**.
  So neither spelling of the placeholder can work, and creating the missing file cannot
  rescue it — it only moves the failure from "cmd cannot find the file" to "Python is
  handed a directory", while silently discarding the gate's own stdout. `verify.py`'s
  `cmd_js_module` already defaults `summary` to
  `desktop/coverage/coverage-summary.json`, so the fix is to **pass `d-agent` (or
  `d-agent/coverage-summary.json`) concretely, or simply drop the 4th argument**, and not
  to invent a path the filesystem cannot represent. I removed both probes; the
  drive-root file this test created was my own argv dump and I deleted it, reporting it
  here rather than leaving it.

## 2. Before / after (real measurements)

| | lines | uncovered lines | files with uncovered lines | files at 100% |
|---|---|---|---|---|
| baseline (first run, 01:54) | 72.1054% (2161/2997) | 836 | 25 | 19 / 44 |
| after segment 1 (02:48) | 77.3106% (2317/2997) | 680 | 13 | 31 / 44 |
| after segment 2 (03:52) | 88.4218% (2650/2997) | 347 | 10 | 34 / 44 |
| after segment 3 (05:10) | 95.0951% (2850/2997) | 147 | 10 | 34 / 44 |
| after segment 4 (07:2x) | 99.3327% (2977/2997) | 20 | 10 | 34 / 44 |
| after segment 5 (branch work) | 99.3327% (2977/2997) | 20 | 10 | 34 / 44 |
| **after segment 31** | **99.5329% (2983/2997)** | **14** | **9** | **35 / 44** |
| **after segment 32 — after a self-inflicted test loss (see §8)** | 98.8989% worst, **99.4995% (2982/2997) now** | **15** now (33 at worst) | **10** now (16 at worst) | **34 / 44** |
|  | *(fully recovered: **14 lines / 9 files, arms 55, branch 97.83%** — the incident's damage is undone)* |  |  |  |

Segments 4 and 5 show the same line figures on purpose: segment 4 closed the
last four *lines*, and segment 5 moved only **branches** (93.36% → 94.62%),
which is why the line table does not move there. At that point the uncovered
count stood at 20 lines in 10 files, all waived in §5.

Segment 10 moved the line figures again — **three more lines covered and one file
removed from the ledger** — after an audit of the waiver ledger against the
provider's raw `coverage-final.json` found that the `useAgentThreadState.ts` row
was miscategorized (see §4). `useAgentThreadState.ts` no longer has any uncovered
statement, so those rows are gone rather than waived.

Branch coverage for the group rose from 68.33% (1728/2529) to **97.07%**
(2401 → 2455/2529) on the same run. Segment 5 was spent on exactly that gap: the
remaining uncovered branches are boundary cases and are accounted for in §3.
The 14 remaining uncovered lines are exactly the waivers in §5 — no reachable
line is left uncovered. Three later branches (2392 → 2398 → 2401) came from the
mutation campaign: the run-window tie fixture below, and the three
`useAgentThreadState` fixtures that closed the miscategorized row.

### Branch coverage closed this segment

The gate's informational list named seven files with 100% lines but branches
below 90%. Of those, **four are now at 100% of both** by asserting the second
side of each condition rather than only the side a happy-path test happens to
take:

| file | branches before | after | the two sides now asserted |
|---|---|---|---|
| `ThreadHeader.tsx` | 62.5% | **100%** | `thread?.title ?? fallback` with a null thread *and* a null `title`; the optional shell `action` rendered *and* absent |
| `attachments.ts` | 83.3% | **100%** | a path with no basename (`/`) *and* a normal one; a dotless and a leading-dot name *and* a real extension; `classifyAttachment` returning `image` *and* `file` |
| `clipboardAttachments.ts` | 85.7% | **100%** | an undecodable `file://%` skipped *and* a bare `file://` → `/` kept; a separators-only list |
| `composerDraft.ts` | 86.7% | **100%** | every shape of "empty draft" (`{}`, blank text, empty list) clearing the slot *and* a blank-text-with-attachment draft being kept |
| `MessageBlock.tsx` | 85.6% | **94.82%** | its settled-state dividers (`noFinalReply` *and* stopped *and* a plain termination notice), the retry *and* continue affordances (including each being withheld), the fork button painted *and* painted out, a USER message's mention *and* link segments *and* plain text, and the compaction label for all 9 status × trigger × token shapes |

**Three** files are left below 90% of branches, not two — the count here was wrong
until segment 11, when I stopped asserting and read `coverage-summary.json` per
file: `externalLinks.ts` (75.0%), `mentionMarkdown.ts` (77.8%) and
**`MentionEditor.tsx` (89.6%, 242/270)**. In the first two the uncovered arms are
**provably dead fallbacks**: each ends with `?? ""` on a regex capture group that
the pattern makes mandatory, and they are pinned by tests asserting no input can
reach them (the malformed `[](https://x.com)`, `[label]()`, `[a](./)`, `[a](<>)`
forms all stay literal text), so the tests fail if the pattern is ever loosened.
`MentionEditor.tsx` is a different case and is discussed under the branch audit
below — it has real uncovered arms, not dead ones.

(An earlier draft of this section also claimed `MessageBlock.tsx`'s uncovered arms
were "the not-hovered halves … which `MessageList` covers" — that was wrong, and
the arms it actually had are the ones now asserted above. Corrections recorded
rather than quietly dropped; the numbers in a handoff have to be ones a reviewer
can re-derive.)

### Branch audit: 56 uncovered arms, and what §5 previously implied

The line gate passes, and `verify.py` prints branch coverage as **informational**.
But this document made a completeness claim it had not earned: it listed four
"branch-only dead paths (no line impact, recorded so a reviewer need not
rediscover them)", which reads as an exhaustive list. It was not. Enumerating every
zero-hit arm in `coverage-final.json` gives:

| | arms |
|---|---|
| total uncovered branch arms in the subtree | **56** (branch coverage 97.79%) |
| explained by a waived guard (the arm *is* the guard) or one of the documented dead/redundant paths | 56 |
| **remaining, previously undocumented** | **0** |

**Every uncovered arm now carries an explanation.** The two counts converged this
segment: of the last 8, four were closed by new tests and four were proven dead. An
uncovered arm is therefore not automatically a gap — it is a claim, and each one has a
named mechanism, a measured hit count, or both. A reviewer re-deriving the census should
note that an arm's line often differs from the line the ledger names: a guard's arm sits
on the `if`, while the waived *statement* is its `return;` one line below (the
`Composer.tsx` four, `AgentThread.tsx`:354, `MentionEditor.tsx`:157), and two arms belong
to findings rather than to rows (`agentMessageFormatters.ts`:24 = F7;
`buildContinuePrompt.ts`:73 = the documented dead `?? []`).

By type the residue is predominantly `binary-expr` / `if` / `cond-expr` arms — and by
class they
split three ways, which matters because only one class is testable:

* **Dead fallbacks of exactly the documented kind** (the majority). Every
  `editorRef.current?.getContent() ?? ""` in `Composer.tsx` (lines 303, 365, 429,
  430, 438, 489), the `?? ""` runs in `MentionEditor.tsx` (804, 827, 831, 860, 864,
  880, 888, 892, 893), `item.children ?? []` (`AgentActivityList.tsx`:117),
  `source.attachments ?? []` (`AgentThread.tsx`:217), `result?.entries ?? []`
  (`useThreadMessages.ts`:250), `run.errorMessage ?? ""`
  (`threadRunProjection.ts`:799/800), and `run?.id ?? null` (`sendPipeline.ts`:138) —
  the last one dead by an earlier dereference rather than by type. Same category as the
  four that *were* listed; the left operand is not nullish on any reachable path.
* **Genuinely reachable but untested** — the class this audit was worth running
  for. `ApprovalPrompt.tsx`:132 and `:152` are the `String(reason)` arm of
  `reason instanceof Error ? reason.message : String(reason)`, i.e. a **non-Error
  rejection**, which this suite rejects nowhere. `Composer.tsx`:955 is the
  alternative arm of the `modelsEmptyReason` copy, so only one of the two reasons
  is exercised. `threadRunProjection.ts`:636 is the `run.startedAt` arm of
  `run.endedAt ?? run.updatedAt ?? run.startedAt`.
* **Platform-dependent** — `Composer.tsx`:911/916/917/918/919 select the sandbox
  tier's description by `isWindows` / `isLinux`, so on any one machine some arms
  are unreachable by construction; that is a real `platform-cfg` case, and §3's
  `platform-cfg` row now points here rather than claiming the dimension is purely
  N/A.

**Eight of the named arms were closed this segment**, leaving 101. Five came from one
new isolated suite, and three from two existing ones:

* `ApprovalPrompt.tsx`:132 and `:152` — the `String(reason)` arm in `confirmRule` and
  `confirmCapabilityRules`, both driven by a non-`Error` rejection *after* the rule
  write succeeds. The pre-existing "stringifies a non-Error rejection" test only
  reaches `decide`'s arm, so its name promised coverage it did not give. Two new
  tests in `ApprovalPrompt.actions.test.tsx`.
* `Composer.tsx`:955 — `modelsEmptyReason`, a prop **no test had ever passed**, so
  only its `undefined` arm had run. Two new tests in `Composer.panels.test.tsx`.
* `Composer.tsx`:910-920 — the whole tier-description chain (`sandboxChecking`, the
  Windows and Linux arms, and the fallback). On any one machine only one is
  reachable, so this needed `Composer.platform.test.tsx`, which pins
  `src/lib/platform.ts` (the module that reads `navigator.userAgent`) instead of the
  user agent itself, plus the `useSandboxAvailability` hook, and drives all three
  arms in one run.

**Forty-three arms have been closed and thirty-two more *attributed* — and the residue is now zero.**
The last segment took the final 8: four closed by tests (`useStickyAutoScroll.ts`:130, `useRunReattach.ts`:106,
`ApprovalPrompt.tsx`:177, `Composer.tsx`:701) and four proven dead
(`threadRunProjection.ts`:733, `threadSearchCache.ts`:17, `threadSearchRanges.ts`:35 and `:61`).
`ThreadSearch.tsx` and `useThreadMessages.ts` are completely clear, and every
remaining uncovered arm in the subtree carries a named mechanism. Earlier closures in the
campaign: `useThreadMessages.ts`:134, `AgentThread.tsx`'s two live arms,
`MentionEditor.tsx`'s `insertMention` cluster, and `useMessagePaging.ts`'s five reachable
arms — the optional
full-history loader, the empty-page callback, the missing-row anchor, an empty thread's
window anchor, and the non-cancelable wheel — three of which a mutation proves
discriminating and two of which are documented as non-discriminating (F11). **Six findings**
were **product bugs or latent redundancy** rather than test gaps (F7–F12, sixteen arms
between them), and F8/F9/F10/F11/F12 share a cause worth naming: **guards
doubled by their callees or by a sibling write**, which is why
several uncovered arms turned out to cost nothing to remove.

The closures and attributions, most recent first:

* **The post-unmount imperative handle** (`MentionEditor.tsx`:172, `:184`) — and this
  one also retired two *ledger rows*. A parent can hold the ref past the child's
  unmount (an async send settling after the view is gone) and call
  `clear`/`restore`/`getContent`; both `if (editorRef.current)` guards must no-op
  instead of touching a detached editor. Every existing test called the handle while
  mounted. Covering this took `:801` and `:876` with it — the same `!editor` family
  in `isEditorEmpty`/`serialize`, which I had waived as unreachable — because
  `getContent()` after unmount calls `serialize(null)`. **Those two waiver rows are
  deleted rather than re-argued**: the guards are genuinely reachable through the
  handle even though they are not reachable through the mounted component, so the
  earlier waiver was too strong.
* **A non-block wrapper element in the serializer** (`MentionEditor.tsx`:907) — the
  `element.tagName === "DIV" || element.tagName === "P"` newline rule. Both operands
  were uncovered because the tests only ever fed text nodes (which return early) and
  block wrappers; pasted rich text produces a `SPAN`, which must **not** gain a line
  break.
* **Enter in an empty editor** (`MentionEditor.tsx`:407) — `insertNewline` adds a
  leading space only when the editor already holds text, so a blank composer must not
  collect a stray space.
* **An input event with no selection at all** (`MentionEditor.tsx`:651, `:667`) — a
  webview can report an empty `Selection` (the range was dropped, e.g. because the
  node under the caret was replaced), so both `selection.rangeCount > 0` guards must
  tolerate it rather than throwing on `getRangeAt(0)`. The observed contract is that
  no trigger resolves without a caret, so the menu stays shut — I wrote the test
  asserting the opposite first and corrected it against the measured behaviour.
* **`threadRunProjection.ts`'s two `Number.isFinite` guards and its status fallback**
  (3 arms). `collectTurnTimestamps` skips a message whose `createdAt` cannot be parsed,
  and `applyRunToMessage` falls back to `"complete"` for a status-less row. Both
  guards' arms are now covered by two new tests in `threadRunProjection.test.ts`,
  which pin the observable contract: a malformed `createdAt` neither creates nor
  destroys trust evidence, so `recoverFailedRuns` never stamps a bubble on a guess
  (the direction matters - an assistant *inside* the run window makes a run
  non-orphan, while a malformed one leaves it orphaned). **The status fallback's test
  is proven discriminating** (`expected undefined to be 'complete'`), **the two
  finite-guard tests are not - by design and by measurement**: deleting both guards
  leaves every assertion passing, because `NaN` and absence are indistinguishable to
  both consumers. That is recorded as finding **F8** and the test comments say
  plainly what they do and do not prove, rather than implying the guards matter.
* **`ThreadSearch.tsx` closed completely — five arms, three live tests, one redundant
  guard** (this segment). This was the file I flagged as the most likely remaining real
  gap, and all of it is now covered.
  * The **four range-identity operands** (`:173-176` plus `:181`'s `preservedIndex`
    fallback). The existing tests took the all-equal arm or failed at the FIRST operand
    (a replaced text node). Two `near-miss` tests close the rest, each choosing a
    construct that keeps an earlier operand equal so a later one is actually
    evaluated: mutating `Text.data` keeps the **node identity** while moving the match
    offset (the `startOffset` operand), and **`splitText`** keeps the original node
    while moving the tail to a sibling, so a match at the same offset now ENDS in a
    different node (the `endContainer` operand). Both assert the panel falls back to a
    valid index and count rather than "0 / N" or "-1".
  * `:245`'s `move` guard — pressed Enter in the search field with **zero matches**,
    which a keyboard user can do even though the next/previous buttons are `disabled`.
    The arm is covered, but **the guard is redundant (finding F10)**: `showMatch`
    already returns early on an empty range list, so weakening the guard leaves the
    test passing - measured, not assumed.
* **Two `MentionEditor.tsx` guard arms — both live, both discriminating** (previous
  segment). These were the ones I flagged last segment as most likely to be real
  gaps, and they were.
  * `:250` — the debounced workspace search's `.catch` is guarded on `!cancelled`, so a
    rejection arriving *after* the effect was torn down must not clear a newer query's
    results. No existing test arranged that: the sibling *"closes the menu when the
    search fails"* rejects a promise the effect is still awaiting (the true arm). The
    new test leaves the first search pending, supersedes it with a second query, lets
    that one resolve, and only then rejects the stale one. **Proven discriminating:**
    removing the guard fails with `expected null not to be null` - the menu is cleared
    out from under the query on screen.
  * `:518` — `if (item)` in the `/`-menu's Enter handler. `slashItems` is derived from
    the `contextTools`/`skills` **props**, but the highlight index resets only when the
    `/` **query** changes, so a catalogue that shrinks while the menu is open (the
    real case: the skills list reloading) leaves `selectedSlashItem` past the end.
    This one needed more than a fixture: removing the guard makes `selectSlashItem(undefined)`
    throw `Cannot read properties of undefined`, but React raises it **asynchronously**,
    so it escaped a `try`/`catch` around the dispatch and failed only the RUN via an
    unhandled error - my assertions stayed green. I converted it into real evidence by
    capturing `window`'s `error` event and asserting the list is empty, after which the
    mutant fails with `expected [ …(1) ] to deeply equal []`. That distinction matters:
    an unhandled error kills the run without proving the guard is what prevents it, and
    a test that looks green in isolation is worse than no test.
* **`MentionEditor.tsx`'s nine-arm `?? ""` family and its `else if (segment.text)` arm —
  ten more attributed, the family by sentinel experiment** (this segment). The nine are
  `?? ""` on DOM properties the helpers have already narrowed to text/element nodes;
  substituting `"MUTANT"` into all nine at once leaves **1001 tests passing**, so no
  test executes any of them. **Upgraded this segment to a `throw` probe** (see §4): the
  same nine sites, each `?? ""` replaced by `?? (() => { throw new Error("PROBE-L<n>") })()`,
  leave **1023/1023 tests passing** — a reachable arm would now fail loudly rather than
  merely carrying a different string, which is the stronger experiment. `:189`'s false arm is dead by a different route: it would
  need a parsed segment with empty text, and `parseMentionMarkdown`'s regex
  (`\[([^\]]+)\]`) plus its non-empty-slice guards make that impossible. Both are
  recorded in §5 with their mechanisms; no test was written, because none could be
  honest.
* **`Composer.tsx`'s six `editorRef.current?.getContent() ?? ""` arms — a whole family
  attributed in one experiment** (previous segment). All six are `?.`-then-`?? ""` on a
  detached-editor defence, and none is reachable: `editorRef` is set in the commit
  that renders the editor and cleared only on unmount, while each runs from a mounted
  handler or effect (`:438` additionally pre-empted by `:436`'s guard). Rather than
  argue six times, I substituted a `"MUTANT"` sentinel into **all six sites at once**
  and ran the subtree: **1001 tests passed, zero failures** - no test ever executes
  one. **Upgraded this segment to a `throw` probe** alongside the nine `MentionEditor`
  sites: all fifteen `?? ""` fallbacks replaced by throwing IIFEs, and **1023/1023 tests
  still pass** (§4). Documented in §5 as a named family.
* **`Composer.tsx`:243's `|| skill.description` — covered but provably redundant
  (finding F9, this segment).** I wrote the no-Chinese-text fallback test, watched it
  pass, then mutated the arm away and watched it **still pass**: when the right
  operand runs, the left is null, so `null || skill.description` yields exactly what
  omitting the OR would. The test asserts a real user-visible boundary and is kept,
  but its comment and F9 state plainly that it does not discriminate the arm.
* **`MentionEditor.tsx`'s caret fallback — a third arm proved dead by trying to reach it**
  (previous segment). `insertMention`'s `selection && selection.rangeCount > 0 ? … : null`
  looks like a live "no caret" branch, and the two stale-row tests above it *look*
  like they cover it. They do not, and nor can any test: `insertMention` calls
  `editor.focus()` two lines earlier, and focusing a contenteditable always leaves a
  caret. I measured that three ways — a cleared-selection file-row click inserts
  **nothing** (that path bails in `insertFile` before reaching `insertMention`),
  and calling the exposed handle with the selection cleared puts the pill at the
  **front** of the text (`"[alpha.ts](./src/alpha.ts) see"`), which is the *true* arm
  running on the offset-0 caret `focus()` synthesised; `disabled: true` behaves the
  same. Documented in §5; the draft test was deleted rather than kept as a
  meaningless pass. **No metric moved this segment** — the deliverable is the
  classification plus the census correction (explained 26 → 27).
* **Three `MentionEditor.tsx` rendering arms — all live, all discriminating** (3 arms; previous segment).
  Classification-first paid off after a run of dead arms: these are *not* `?? ""`
  fallbacks on non-null DOM properties, they are genuine "optional extra absent"
  renders.
  * `:130` `buildSlashMenuGroups(..., skills ?? [])` — `skills` is an **optional prop**
    (`skills?: SkillMentionOption[]`), so a caller that has not loaded a catalogue
    renders without it; without the fallback the helper receives `undefined` and
    throws on `.filter`.
  * `:721` the file row's `dir ? … : null` — a file at the workspace **root** has
    `dir === ""`, so the label must be omitted rather than rendering an empty span.
  * `:789` the skill row's description ternary — a skill with no description shows
    only its name.

  All three were verified together by breaking all three arms at once (removing the
  fallback, rendering the label unconditionally, forcing the ternary true):
  **exactly the three new tests fail**, 66 others pass - attribution clean enough to
  say each test kills its own arm. I also hit a real trap doing it: my first mutation
  produced unbalanced JSX and a `PARSE_ERROR`, so I restored the file with
  `git checkout --` (it had no intended changes) and used a syntax-safe mutation that
  only flips the condition.
* **This segment's earlier work was attribution, not new tests — and that is the honest result.**
  I examined the next four arms with the question *"what would deleting this change?"*
  and **all four are dead**: two belonged to the `"empty"` page status that finding F5
  already established has no producer (`useThreadMessages.ts`:424, and the ternary's
  false arm at `:358`), and two are the compaction-divider guards
  (`threadRunProjection.ts`:465 is the branch arm of the already-waived `:466`, and
  `:467`'s `?? []` is unreachable because `isCompactionDivider` demands
  `segments?.length === 1`). None can be covered by a test that does not lie about a
  type, so I attributed each to its root cause and moved them from *undocumented
  residue* to *explained* rather than writing four tests that would pass for the wrong
  reason. That took the explained total from 24 to 26 and left 59. **No test was added
  this segment and no metric moved** — deliberately, because the alternative was
  filing coverage-shaped tests for code that cannot run.
* **`useAgentThreadState.ts`'s prompt-dedup arm — a second arm proved dead by trying to
  test it.** The catch that clears `consumedPromptRef` is guarded on the ref still
  holding *that* prompt, so a newer prompt's record is never clobbered. I could not
  reach the false arm: only L235 writes a non-null ref, and that write happens in
  another prompt's effect, whose own `handleSend` is refused outright while this send
  holds the send lock — so its `.catch` nulls the ref again immediately. The window
  the guard defends against would need the other effect's write to land after its own
  rejection was delivered but before its reaction microtask, which is the same
  checkpoint and so cannot open. The experiment agreed: my arrangement produced
  `hits=[1, 0]`, true arm only, so the draft was deleted and the arm documented (§5).
  The *line* stays covered by the existing retry test, so this costs no waiver.
* **`useAgentThreadState.ts`'s `activeRunStartedAt` fallback** (2 arms → 0, one of them
  dead). The chain `recentRun?.startedAt ?? recentRun?.createdAt ?? null` feeds the
  streaming bubble's `runStartedAt`, which drives the live elapsed timer. The
  `createdAt` arm was uncovered because every fixture built a run with an explicit
  `startedAt`; a legacy row (optional `startedAt?`, required `createdAt`) reaches it,
  and the test asserts the bubble carries exactly `createdAt` by driving a real
  projection through the reattach path (verified discriminating: dropping the arm
  fails with `expected undefined to be 1789171195000`). **Writing the test for the
  THIRD arm is what proved it dead** - `StoredRun.createdAt` is required, so `?? null`
  is unreachable by construction and `tsc` refused the cast a test would need. The
  draft was deleted and the arm recorded as a dead path in §5, which is the honest
  outcome rather than a forced fixture.
* **`useAgentThreadState.ts`'s `visibilityState` guard** — and the sibling test that
  *looked* like it covered this one did not. `visibilitychange` fires both when a tab
  returns and when it goes away, so `reconcileHungSend` is gated on
  `document.visibilityState === "visible"`. The false arm was uncovered, and the
  existing test is why: `useAgentThreadState.abort.test.tsx` sets `"visible"`, fires
  the event and asserts *no* reconcile - but that view has **no hung send**, so
  `reconcileHungSend` returns early whatever the guard says. It passes for either
  value. The new test in the lifecycle suite - where a send genuinely is stuck -
  asserts the hidden case does nothing and the foreground case reconciles, and it is
  proven discriminating: inverting the guard makes it fail with
  `expected 'completed' to be 'running'`, i.e. the app abandoning a send while the
  user cannot see it.
* **The wheel `deltaMode` scaling** (`useMessagePaging.ts`:416, `:418`) — a wheel event
  carries a `deltaMode`, and the handler converts the delta to pixels per engine:
  Chromium reports PIXEL (0), **Firefox reports LINE (1)** for a mouse wheel, and some
  report PAGE (2). Both non-pixel arms were uncovered because **no test ever set
  `deltaMode`** — every fixture used the constructor default — so a Firefox user's
  wheel was untested end to end. Two tests now dispatch mode 1 and mode 2 and assert
  the scaled position (for the line-mode test, `600 + 8 x 20px`), and both are
  proven discriminating: collapsing the scaling to a constant
  `1` fails them with `expected 608 to be 760` / `expected 608 to be 2200` — the raw
  pixel delta, i.e. exactly the bug. For the line-mode test I set `lineHeight`
  explicitly rather than relying on the environment's `normal`, so the assertion
  tests the **multiplication** instead of the code's `|| 16` fallback.
* **`ThreadSearch.tsx`'s three `!controller.signal.aborted` guards** (8 arms → 5). The
  prepare effect's cleanup **aborts** the previous controller when a new query lands,
  and these three guards are what stop the abandoned call from writing state. A branch
  audit found all three uncovered: every existing test passes a trivial
  `onPrepareSearch` (`async () => {}`) that resolves before a second query can
  supersede it. Two tests now use
  a controllable promise and supersede it mid-flight. **When I first mutated the
  guards, only one test failed** - the resolve variant passed anyway, because the
  stale write changes nothing *at that instant*; the harm is that every later rescan
  bails on `preparedQuery !== query` (ThreadSearch.tsx:157) and the panel freezes. So
  the test now changes the transcript after the stale write and asserts the total
  still tracks it: with the guard removed the panel reads `1 / 1` forever instead of
  `1 / 2`. **My own expectation was wrong first too** - I asserted `2 / 2`, but the
  correct render is `1 / 2` (the index stays on the match the reader was on, only the
  total grows), which the failure message showed before I adjusted it.
* **`sendPipeline.ts`'s entire stale-send guard family** (8 arms, now 1 — and that last one is dead). Every
  `if (isCurrentSend())` in that module is the guard that stops a **superseded send**
  from writing into a view that now belongs to another conversation — and a branch
  audit found the false side of **all seven** uncovered. The cause is precise: the
  test file's `makeDeps` hardcodes `isCurrentSend: () => true`, and across 499 lines
  the only occurrence of the identifier is that hardcode. The *hook* test that looks
  like it covers this (`useSendMessage.test.tsx`, *"drops a run-accept that arrives
  after the send was superseded"*) **mocks the pipeline**, so it asserts the hook's
  gating and never executes the module's own guards. Three tests now flip the flag
  mid-flight and assert the contract the module documents but never proved: **durable
  work still happens, the superseded view is never touched** — on the interrupted
  path, the cancelled path, and the \"attach failed but the run durably completed\"
  path. Each asserts on user-visible state (the content patched into the view) rather
  than only on call counts, and I verified the kill by deleting one guard: the test
  fails with `expected [ 'partial answer' ] to not include 'partial answer'` — i.e.
  one conversation's partial reply painted into another. **The last two arms closed this
  segment**, both on the same "superseded mid-flight" harness (`getRun` flips the flag
  while `isCurrentSend` reads it): the cancelled-with-no-text finalization (`:225`) and
  the post-failure sidebar refresh (`:359`). Both were killed by deleting the guard, and
  the two failures land on **different assertions**, which is what makes the attribution
  clean — `expected true to be false` (a stopped bubble painted into the new view) for
  the first, `expected "vi.fn()" to not be called at all` (`refreshRecentRun`) for the
  second. The first test also pins that the *durable* half still happens: `createRun`'s
  own refresh fires exactly once, outside the guard.
* **`sendPipeline.ts`:138's `run?.id ?? null` is dead — by an earlier dereference, not by
  a type.** The line looks like a defensive nullish fallback, and the declared type is
  the widened `let run: StoredRun | null = null`, which reads like a real possibility.
  But the **immediately preceding line** in the same `if` block is
  `patchMessage(setMessages, optimisticUserId, { runId: run.id })` — an **unguarded**
  `run.id`. If `run` were nullish that line throws first, so the `?.`/`?? null` pair on
  the next line can never be reached. `createRun`'s signature
  (`invokeCommand<StoredRun>`, and `invokeCommand` only ever returns or rejects) is
  corroborating, but the sibling dereference is the proof: **the arm is protected by a
  line that would already have thrown.** `unreachable-by-construction`.
* **`Composer.tsx`'s `lastTextRef` fallback** (`:277`) — the one arm I could not explain
  last segment, now closed. **My stated hypothesis was wrong in its premise**: I had
  guessed Composer had its own composition `requestAnimationFrame` handler. It has
  none — it delegates composition entirely to `MentionEditor`. The *mechanism* I was
  reaching for was right, though: that editor schedules its post-composition work
  (trigger rescan, empty-state sync, `onChange`) from a `requestAnimationFrame`, and a
  frame still pending when the view unmounts is **not cancelled** — so `onChange`,
  which is this component's `saveDraft`, runs with the editor already detached. That
  is the only route by which `editorRef.current` is null inside `saveDraft`, and the
  fallback exists so the save reads the last known text instead of `""` from a
  detached node — without it, a late composition frame would **erase the user's
  half-written draft from storage**. The test composes, commits, unmounts, *then*
  flushes the captured frame, and asserts the drafted text was persisted.
  I verified the arm rather than trusting the green: the report shows
  `branch 14 at L277 hits=[9, 1]` (the fallback taken exactly once), and removing the
  fallback makes the test fail with `""` reaching `saveComposerDraft` — so the
  assertion discriminates rather than passing because the attached editor would have
  read the same text.
* **`Composer.tsx`'s drag fallback** (`:696`, `dragStateRef.current ?? "accept"`) — the webview can deliver `over` **before any `enter`** (the pointer entered the window already inside the drop zone), so no verdict is recorded and the fallback decides. Every existing drag test sends `enter` first, so the arm was never taken.
* **`Composer.tsx`'s dotless attachment chip** (`:811`, `{ext ? … : null}`) — a file named without a dot has `ext: ""`, so no extension span renders. Asserted on the **DOM** rather than the text, and that distinction mattered: an empty `<span>` contributes no text, so a text-only assertion could not tell the two branches apart. The control (a `.ts` chip) is asserted in the same test.
* **The install separator, both operands** (`Composer.tsx`:489, `:490`) — after a successful install the composer appends `/name` to the draft, adding a separating space *only* when text is already present, so the empty and non-empty cases need opposite fixtures. Two things surfaced while writing it: the assertion has to be on **what gets sent**, not the editor, because a successful install immediately submits and clears the composer (an editor assertion would pass on `""` for the wrong reason); and `submitValue` **trims**, so the trailing space the restore adds is gone by then — what the empty-draft arm actually governs is the *absence of a leading space* (`"/future-web"`, not `" /future-web"`).
* **The entire Chinese skill-description path** (`Composer.tsx`:243 — all three
  arms of `useZh ? descriptionZh || skill.description : skill.description`). No
  Composer test had ever rendered in Chinese, including the one *named* "uses the
  platform catalogue's Chinese text for a skill description" — which asserts the
  English `Paper` and so documents the opposite of its own name. Two new tests in
  `Composer.panels.test.tsx` switch the locale with `i18n.changeLanguage("zh")` and
  pin **both** operands of the fallback separately: one where the *installed* skill
  carries `descriptionZh` (`/future-paper论文`) and one where it does not and the
  *platform catalogue* supplies it (`/future-web网页`). The `afterEach` restores
  `en`, because the i18n instance is shared across files.
* **`threadRunProjection.ts`:619, `:636`, `:799`, `:800`, `:804` — five fallbacks on
  one fixture.** A legacy run row carrying `startedAt` but *neither* `endedAt` nor
  `updatedAt`, and no `errorMessage`, reaches the third arm of
  `run.endedAt ?? run.updatedAt ?? run.startedAt`, the `null` arm of
  `typeof ms === "number" ? … : null`, both `run.errorMessage ?? ""` sites, and the
  `?? new Date().toISOString()` bubble timestamp. Every other fixture supplies a
  complete run, so all five were at zero hits. The test asserts the bubble still
  gets a **finite** `createdAt` and never renders the literal `"undefined"`.
* `ApprovalPrompt.tsx`:65 — the escalation title chosen by **trigger**. The
  neighbouring test sets `category: "sandbox_escalation"` in the payload while the
  title branch keys off `approval.kind`, so it never entered the arm. Both trigger
  variants (`model_request`, `sandbox_failure`) are asserted.
* `ApprovalPrompt.tsx`:488 — `isEditableTarget`'s `tagName === "select"` arm: a
  keyboard event dispatched from a `<select>` must reach the select, not be
  swallowed by the prompt's shortcut handler.
* `ReplySteps.tsx`:29 and `:51` — the two `tools ? … : null` arms, i.e. a steps
  group with reasoning and **no** tool calls. Reaching it needed a subtlety:
  `buildReplyBlocks` folds a group only when it has **at least two** consecutive
  settled steps (`run.length > 1`), so a lone thought renders standalone and never
  forms a group; two thinking segments do it.
* `NewConversation.tsx`:200 and `:255` — the `modelId || defaultAgentModelId`
  fallback on the **chat** send path and the **skill-guide** path. The existing
  fallback test renders with the default mode, so it only ever reached the
  workspace branch; all three call sites repeat the expression.

**`agentMessageFormatters.ts`:24 is not a coverage gap — it is bug F7.**
`buildAgentFailureTitle` ends `return title === titleKey ? i18n.t("agent:failure.runTitle") : title;`,
expecting `i18n.t` to hand back the key unchanged on a miss. This i18n instance
returns the key **without its `agent:` namespace**, so `title === titleKey` can
never be true: the neutral-title fallback is **dead code**, and the user is shown
the raw mangled key as the heading. `insufficientCredit`, `rateLimited`,
`serverError`, `contextLimit`, `network` and `userStopped` are all classified but
have no `…Title` translation, so all six hit it — measured, an HTTP 402
insufficient-credit failure renders `failure.insufficientCreditTitle`. The test
pins the observed string on both sides so a fix only has to flip one expectation;
the one-line fix (compare the namespace-stripped form, or use `i18n.exists`) is in
product code outside this task's write set. This is also why the arm cannot be
"covered" honestly: it is unreachable, but for a reason worth reporting rather
than filing as a dead fallback.

Three harness details fell out of these tests failing first: the empty-model copy
lives inside the picker popup, so it needs `trigger("Model")` before the assertion;
two `render()` calls in one `it` share the root, so the popup stays open and the
second trigger click *closes* it (hence separate tests); and a `vi.mock` factory
**snapshots** a plain property at import time, so a mocked platform flag must be
exposed through a **getter** or the Windows arm can never be reached - which is
exactly how the first version of that test failed.

**What this does and does not mean.** It does not affect the acceptance contract,
which is on lines, and no unreachable arm costs a line. It does mean the phrase
"recorded so a reviewer need not rediscover them" was over-claiming, and that
branch coverage in this subtree is *not* a finished story: 97.07% of arms, with the
residue dominated by dead fallbacks but containing at least the handful of real
gaps named above. The full 31-arm list with source text is reproducible in a few
lines of Python against `coverage-final.json`.

`npx vitest run` over the whole desktop project → **243 test files, all passing**
at the time of writing, and `npx vitest run --coverage src/features/agent` →
**68 files / 955 tests, all passing** for this subtree.
Two operational notes, both learned the hard way: the *whole-project* coverage
run exceeds a 40-minute wall clock under parallel-worker contention (243 files,
several workers' suites red in their own subtrees), and when it is killed the
report directory has already been cleaned, so the gate sees **no report at all**
and reports a missing file rather than a coverage failure. Running the scoped
form — `npx vitest run --coverage --maxWorkers=4 --coverage.reportsDirectory=coverage/d-agent src/features/agent`
— produces a report the gate accepts unchanged (*"report covers all 44 expected
source file(s)"*) in a fraction of the time, and its numbers are identical or
better (line totals are the same 2977/2997).
**A third note, added segment 27, because it cost me a confusing minute:** vitest
**wipes `reportsDirectory` at the start of every coverage run**. Any scratch helper
written there (a dump script, a mutation driver, a census script) is deleted by the
next coverage run — which is why the writers in this campaign re-create their helper
immediately before using it, and why a "file not found" for a script that ran fine a
moment ago is expected behaviour rather than a lost file. The up side is that the
directory self-cleans, so scratch helpers left there are not debris a reviewer has to
notice; the down side is that they must never be the only copy of anything.
`npx tsc --noEmit` reports no error under `src/features/agent/`, and
`npx eslint src/features/agent` is clean.

### Mutation validation — three survivors, two distinct failure modes

The objective asks for test *effectiveness*, not just coverage, and this is
instrumented for Rust only in this repo (`verify.py mutation` reads a
cargo-mutants `summary.json`; there is no TS mutation runner — no stryker/mutant
in `package.json`, no `mutation/` directory). So I ran a mutation campaign by
hand against five product files **inside this write set**, one mutation at a
time, reverting each and re-verifying by SHA-256 (all five mutated files were
restored byte-identical; `git status` shows no product file modified).

Each mutant was applied in place, the targeted spec run, and the mutant counted
**killed only if its spec failed**:

| # | file | mutation | dimension | result |
|---|---|---|---|---|
| 1 | `buildContinuePrompt.ts` | `tools.slice(0, 8)` → `slice(0, 9)` | boundary | **killed** |
| 2 | `mentionMarkdown.ts` | `if (match.index > last)` → `>=` | serialization | **killed** (2 tests) |
| 3 | `attachments.ts` | `size > READ_SOURCE_MAX_BYTES` → `>=` | boundary | **survived → fixed → re-killed** |
| 4 | `composerDraft.ts` | `version !== DRAFT_VERSION` → `>` | error-path | **survived → fixed → re-killed** |
| 5 | `attachments.ts` | `splitFileName`'s `dot <= 0` → `dot < 0` | boundary | **killed** |
| 6 | `mentionMarkdown.ts` / `externalLinks.ts` | `?? ""` → `?? "MUTANT"` (control) | — | **survived, by design** |

**The two survivors are the finding, and they are uncomfortable ones**: both files
were already at **100% of lines *and* 100% of branches** (`attachments.ts` 18/18,
`composerDraft.ts` 15/15) at the moment their mutants survived. Coverage said
"done"; the mutants said the boundary was unpinned:

* **#3** — the size tests used `READ_SOURCE_MAX_BYTES + 1` and `1024`, so `>` and
  `>=` were indistinguishable: **the exact cap was never exercised**. Fixed with
  *"accepts an image at exactly the byte limit, so the cap is exclusive"*.
* **#4** — the only version-mismatch case was `version: 999` (a *newer* schema),
  which `>` also rejects, so `!==` and `>` were indistinguishable: **no stale-schema
  case existed**. `2026-09-26` a `version: 0` and a missing-`version` assertion were
  added; under `>` a pre-bump draft would have been *resurrected* instead of
  ignored, which is a real (if minor) product bug the coverage number hid.

Both were re-mutated after strengthening and both were then killed, so the loop
closed honestly. **Control #6** is the other half of the evidence: injecting a
visible `"MUTANT"` sentinel into the `?? ""` fallbacks that §2 claims are dead
changed no test outcome — the arm is never reached, which is a stronger and more
direct justification for those two files' branch percentages than the regex
argument alone.

The generalisable lesson, recorded because it cost real time and is not
derivable from the repo: **a green branch percentage is not a pinned boundary.**
Both sides of a comparison can be exercised (hence 100%) while the equality case
is never tested, and only a mutant exposes that.

#### Second batch — one survivor, and a second, different failure mode

A follow-up batch put five more mutants in five files (one per file, for clean
attribution), each reverted and SHA-256-verified after:

| # | file | mutation | dimension | result |
|---|---|---|---|---|
| 7 | `buildContinuePrompt.ts` | `previousUserForRun`'s `runMessageIndex >= 0` → `> 0` | boundary | **survived → fixed → re-killed** |
| 8 | `attachments.ts` | `extOf`'s `dot > 0` → `dot >= 0` | boundary | **killed** (a `.bashrc` became an extension) |
| 9 | `composerDraft.ts` | `(attachments?.length ?? 0) > 0` → `>= 0` | error-path | **killed** (2 tests: an emptied draft was saved) |
| 10 | `mentionMarkdown.ts` | `MENTION_LINK.lastIndex = 0` → `1` | serialization | **killed** (2 tests) |
| 11 | `externalLinks.ts` | `EXTERNAL_LINK.lastIndex = 0` → `1` | serialization | **killed** |

**#7 is the interesting one, because its cause was different from #3/#4.** The spec
already *had* a case named for this boundary — *"returns null when there is no user
message to recover"*, commented *"matched assistant at index 0 leaves nowhere to
walk back to"* — yet the mutant passed all 14 tests. The fixture was a **singleton**:
`[assistant("a1", "run-1")]`. In a one-element history the correct branch
(`startIndex = -1`, scan finds nothing) and the mutant's fallback
(`startIndex = length - 1 = 0`, `messages[0]` is the assistant, not a user) **both
return null**, so the assertion could not discriminate. Adding a later user message
makes them diverge — and the mutant then fails with exactly the wrong value:

```
AssertionError: expected { id: 'u2', role: 'user', …(2) } to be null
```

which is the real bug the mutant stands in for: a thread whose first message is the
run's assistant reply, *and* which contains later user messages, would have the
"continue" flow quote **the last, unrelated user message** as the thing to continue
from. The test now asserts both the two-element and three-element shapes.

So the second lesson is distinct from the first: **a boundary test is only as
discriminating as its fixture.** If the fixture is degenerate (singleton, empty,
single-branch), the correct code and the fallback often agree, and the assertion
passes for both. #3/#4 were "the boundary value was never used"; #7 was "the
boundary *was* used, but the surrounding data made both paths equivalent".

#### Third batch — four more files' logic, four kills

Two files not mutated before, to see whether the pattern held elsewhere
(`threadSearchCache.ts`, the search prefilter, and `useWorkspaceForm.ts`, the
create/open-workspace state machine):

| # | file | mutation | dimension | result |
|---|---|---|---|---|
| 12 | `threadSearchCache.ts` | empty-query guard `return false` → `return true` | boundary | **killed** |
| 13 | `useWorkspaceForm.ts` | `cancel`'s `workspaces[0]` → `workspaces[length - 1]` | boundary | **killed** |
| 14 | `threadSearchCache.ts` | `message.role === "user"` → `!==` | error-path | **killed** (4 tests) |
| 15 | `useWorkspaceForm.ts` | `samePath`'s trailing-separator strip removed | serialization | **killed** |

All four died, which is the outcome a reviewer should want: it says these two
modules' boundaries *were* pinned, by contrast with #3/#4/#7. #13 is worth naming
because the failure it stands in for is user-visible — cancelling the dialog would
land on the **least**-recently-used workspace instead of the most recent — and #15
is the *only* part of the `samePath` rule that is pinned, which is exactly why F1
(a missing `/`-vs-`\` unification) could be found by reading rather than by
mutation: removing the separator strip breaks a test, but *adding* the missing
unification would not, because no test covers cross-separator comparison.

#### Fourth batch — component-level mutants

The first three batches mutated *pure modules and hooks* only, so they said nothing about
the jsdom **behaviour** tests that were the bulk of this task's work. This batch
mutated the React components themselves:

| # | file | mutation | dimension | result |
|---|---|---|---|---|
| 16 | `MessageBlock.tsx` | status-divider ternary: swapped `noFinalReply` ↔ `stopped` | boundary | **killed** (2 tests) |
| 17 | `Composer.tsx` | image cap `imageCount >= MAX_IMAGES_PER_TURN` → `>` | boundary | **killed** |
| 18 | `MentionEditor.tsx` | `isEditorEmpty`'s `trim().length === 0` → `!== 0` | boundary | **killed** (3 tests) |
| 19 | `AgentThread.tsx` | `agentState?.isCompacting ?? false` → `?? true` | concurrency | **survived → fixed → re-killed** |
| 20 | `useThreadMessages.ts` | run-window filter `(run.endedAt ?? run.updatedAt) >= firstTime` → `>` | boundary | **survived → fixed → re-killed** |

Three of the four component mutations died immediately, which is the assurance
this batch was for: the interaction tests assert observable behaviour rather than
merely touching lines. **#19 is the fourth weak spot the campaign has found, and it
is a wiring gap, not a logic gap.** `AgentThread` passes
`compactionInProgress={agentState?.isCompacting ?? false}` into the composer, and
the test's `Composer` mock simply never exposed that prop — so *no* test could see
it, and `?? true` left all 951 subtree tests green. The user-visible failure would
be a thread whose agent state has not loaded yet showing the composer as
mid-compaction, withdrawing the compact affordance. Fixed by exposing
`data-compacting` on the mock and asserting all three shapes (`isCompacting` true,
false, and `agentState = null`); the mutant now fails on exactly the `null` case
(`expected 'true' to be 'false'`) and the clean code passes 52 tests.

**#20 was closed rather than left open.** It is the case that needed a fixture of a
different *shape*, and building that fixture was the whole fix. The mechanism, derived
then measured: the clause is observable only when `hasMore === true` (otherwise
`!result.hasMore` short-circuits the `||`), a settled run's `endedAt`/`updatedAt`
exactly **ties** the page's first message timestamp, and the page forms an **exchange**
so `applyRunMetadata` has an assistant row to stamp. The spec already asserted
`listRuns` was *called*, but every page fixture here comes from `history()`, which
emits `role: "user"` entries only — with no assistant row there is nothing to stamp,
so the correct code and the mutant projected identically (measured: both gave
`status=complete` with no run fields). Three tests were added, each with its own mount,
because `mount()` reuses the same root and state leaks between calls inside one `it`:

1. the tie with `hasMore: true` — the reply **is** stamped with the run's failure
   bubble (`terminationNotice`/`terminationTitle` set);
2. a control run that settled one millisecond earlier — **not** stamped, so a mutant
   that dropped the window entirely cannot pass either;
3. the tie with `hasMore: false` — stamped through the short-circuit, guarding the
   direction of the `||` as well as the comparison.

Re-mutating `>=` to `>` now fails **exactly one** test (the tie, `expected undefined to be truthy`) with the other 49 in the file passing — attribution precise enough to say
the kill is this mutant and nothing else. The lesson is the sharpened form of #7's: **a
mutant surviving the whole suite can mean the suite has no fixture of the right shape,
not that the assertion is absent** — and it is worth adding the fixture rather than
waiving the line, because the fixture is where the missing coverage actually lives.

#### Fifth batch: concurrency, and the two claimed-dead branches verified from raw v8 data

The fourth batch's survivors were all boundary or wiring cases, so this batch went
after the **concurrency** dimension, the one the campaign had touched least:

| # | file | mutation | dimension | result |
|---|---|---|---|---|
| 21 | `useMessagePaging.ts` | `finishCooldown`'s `dataPendingRef \|\| renderPendingRef` -> `&&` | concurrency | **killed** |
| 22 | `threadMessageCache.ts` | eviction `snapshots.size > MAX_CACHED_THREADS` -> `>=` | boundary | **killed** |

Both died, and both stand in for real regressions. #21 would release the wheel
protection and finish the cooldown while a data read *or* a render was still
outstanding (the guard must stop when **either** is pending, so `&&` weakens it to
"only if both"), leaving a half-corrected viewport. #22 would hold **11** cached
conversations instead of 12, because with `>=` the loop evicts as soon as the size
*reaches* the cap instead of exceeding it.

**The §2 claim that `externalLinks.ts` and `mentionMarkdown.ts` have only dead
fallback arms is now verified from the raw data, not inferred.** The gate reports
those two files at 75.0% and 77.8% branches, and the write-up asserted the
uncovered arms are the `?? ""` fallbacks on mandatory regex groups. I read the
v8 provider's own `coverage-final.json` (in the report directory) and listed every
arm with a zero hit count:

```
externalLinks.ts      arm 1[1] binary-expr line 29
                      arm 2[1] binary-expr line 29
mentionMarkdown.ts    arm 1[1] binary-expr line 29
                      arm 2[2] binary-expr line 29
buildContinuePrompt.ts arm 7[1] binary-expr line 73
```

and confirmed the source at each line is exactly the claimed construct:
`match[1] ?? ""` / `href: match[2] ?? ""` (externalLinks.ts:29),
`match[2] ?? match[3] ?? ""` (mentionMarkdown.ts:29), and
`outputsByTool[tool.id] ?? []` (buildContinuePrompt.ts:73). **No uncovered arm sits
anywhere else in those files**, which is what makes "provably dead fallback" a
defensible claim rather than an assumption - and it is the check a reviewer should
repeat with `coverage-final.json` rather than trusting the percentage.

### Files taken to 100% of lines

| file | uncovered before | evidence |
|---|---|---|
| `AgentThread.tsx` | 143 | `AgentThread.test.tsx` (51 tests) |
| `NewConversation.tsx` | 71 | `NewConversation.test.tsx` (31 tests) |
| `ApprovalPrompt.tsx` | 48 | `ApprovalPrompt.actions.test.tsx` (32 tests) — 2 unreachable lines remain, §5 |
| `ThreadSearch.tsx` | 26 | `ThreadSearch.navigation.test.tsx` (22 tests) — **fully clear of uncovered arms** |
| `MessageBlock.tsx` | 25 | `MessageBlock.behaviour.test.tsx` (15 tests) + `MessageBlock.states.test.tsx` (13 tests, new: the second side of every settled-state branch) |
| `AgentActivityList.tsx` | 14 | `AgentActivityList.test.tsx` (16 tests, new) |
| `useAgentThreadState.ts` | 13 | `useAgentThreadState.abort.test.tsx` (9 tests) + `.lifecycle.test.tsx` (12 tests) — **now fully covered of lines; the former waiver row is deleted (see §4)** |
| `buildContinuePrompt.ts` | 37 | `buildContinuePrompt.test.ts` (+2 tests) |
| `useWorkspaceForm.ts` | 53 | `useWorkspaceForm.test.tsx` (13 tests, new) |
| `useMessagePaging.ts` | 9 | `useMessagePagingHook.test.ts` (42 tests) |
| `useRunReattach.ts` | 13 | `useRunReattach.events.test.tsx` (13 tests, new) |
| `MessageList.tsx` | 26 | `MessageList.test.tsx` (12 tests, new) |
| `NewConversationWorkspaceForm.tsx` | 4 | `NewConversationWorkspaceForm.test.tsx` (5 tests, new) |
| `useSendMessage.ts` | 5 | `useSendMessage.test.tsx` (8 tests, new) |
| `useSkillRecommendation.ts` | 3 | `useSkillRecommendation.test.ts` (+8 tests) |
| `threadSearchCache.ts` | 3 | `threadSearchCache.test.ts` (+4 tests) |
| `useComposerInset.ts`, `useStickyAutoScroll.ts`, `ThreadHeader.tsx` | 1 each | `useComposerInset.test.tsx`, `ThreadHeader.test.tsx` |
| `MentionEditor.tsx` | 153 → 2 | `MentionEditor.menus.test.tsx` (71 tests, new) |
| `Composer.tsx` | 118 → 4 | `Composer.tools.test.tsx` (21), `Composer.panels.test.tsx` (27), `Composer.clipboard.test.tsx` (15), `Composer.platform.test.tsx` (3, new — the platform-conditional tier copy) |
| `useThreadMessages.ts` | 66 → 2 | `useThreadMessages.events.test.tsx` (47 tests, new) |

## 3. `dimensions` — where the evidence is

| dimension | evidence (specific cases) |
|---|---|
| `boundary` | `AgentThread.test.tsx`: a streaming bubble *before* the last user message is an older turn, not the current run; no thread / store still loading; a retry whose trigger id is unknown must not resend a different message; 9 tool calls vs the 8-row cap. `MentionEditor.menus.test.tsx`: a bare `@` is a valid empty query and picking still replaces just the `@`; a pill bounds the next query (a second `@` after it is fresh); `foo@bar`, `./@x`, `a/@b` never open the menu; a path query (`a/b`) never opens the `/` menu; a 5000-character query is still just a query; `count: 1`-style pluralization is pinned where it applies; a very long CJK draft submits unchanged. `Composer.tools.test.tsx`: the 5th image is refused against `MAX_IMAGES_PER_TURN` (4); a duplicate drop does not create a second chip; an `over` with no paths keeps the verdict decided on `enter`; an image on a text-only model is flagged, and the same image on a vision model is not. `NewConversation.test.tsx`: no workspaces → chat mode; the requested workspace appears only on a later render. `AgentActivityList.test.tsx`: `count: 1` never pluralises; a child with no target keeps its empty row. `ApprovalPrompt.actions.test.tsx`: an empty glob is refused; `\\?\C:\…` and `\\?\UNC\…` render as ordinary paths. `ThreadSearch.navigation.test.tsx`: a one-character query stays inert; stepping past the last match wraps to the first, and before the first wraps to the last.  `buildContinuePrompt.test.ts`: 1300 CJK chars and 1250 emoji truncated by code point (asserts no `U+FFFD`); exactly-1200 vs 1201. **Segment 5 added the *second side* of each condition** for files whose lines were already 100% — `ThreadHeader.test.tsx` (a null thread *and* a null `title` both fall back to "FutureOS", in the header and in the usage dialog; the optional shell `action` present *and* absent), `attachments.test.ts` (a separator-only path has no basename so the path is its own name; a dotless name *and* a leading-dot dotfile have no extension while `.TS`→`ts` and `.tar.gz`→`gz` do; `classifyAttachment` reaches `image` *and* `file`), `clipboardAttachments.test.ts` (an undecodable `file://%` is skipped while a bare `file://` is a real root path; a separators-and-comments-only list yields nothing), `composerDraft.test.ts` (each of `{}`, blank text and an empty attachment list clears the slot, while a blank-text draft *with* an attachment is kept), `externalLinks.test.ts`/`mentionMarkdown.test.ts` (the empty-label, empty-href and empty-path link forms all stay literal text, which is why their `?? ""` fallbacks have no input), and `liveStreamTick.test.ts` (deterministic `isActive`/`stopped` combinations plus the coalescing boundary). **Segment 10:** `useAgentThreadState.abort.test.tsx` — aborting with **no conversation open** (null thread) is a no-op at the storage layer; `ReplySteps.test.tsx` — a steps group holding **two thinking segments and no tool calls** (note `buildReplyBlocks` folds only when `run.length > 1`, so a lone thought renders standalone and never forms a group); `NewConversation.test.tsx` — the default-model fallback on **all three** call sites (workspace, chat, skill guide) when no model is chosen yet. **Segment 13:** `MentionEditor.menus.test.tsx` — **Enter in an empty editor** inserts no leading space, and a **non-block wrapper** (`<span>`) serializes with no invented newline. **Segment 14:** `Composer.tools.test.tsx` — an `over` arriving **before any `enter`** still accepts the drop; a **dotless** attachment renders no extension span (asserted on the DOM, since an empty span contributes no text). **Segment 17:** `useMessagePagingHook.test.ts` — a wheel in **line mode** scales by the line height and a **page-mode** wheel by the viewport height, against a pixel-mode baseline of the same `deltaY` (608) that proves the scaling. **Segment 21:** `MentionEditor.menus.test.tsx` — the composer opened with **no `skills` prop at all** (it is optional), a file at the workspace **root** (empty directory label omitted), and a skill carrying **no description**. **Segment 25:** `useMessagePagingHook.test.ts` — an **empty conversation** renders with no window anchor (`visibleMessages[0]` and `messages[start]` are both undefined) and scrolling stays inert; a **declared-but-unloaded** history (`hasOlderHistory: true` with zero local messages and **no loader** — the two props are independent) takes the non-loading branch with `messages[start]` undefined and must not dereference a missing row; an **empty page callback** from the loader leaves the window exactly where it was; and the **optional full-history loader** is what lets a search reach a match the local window never loaded (the pinned window start is only observable once the caller merges the fetched transcript, which is how `useThreadMessages` drives it). **Segment 26:** `MentionEditor.menus.test.tsx` — `insertMention` from the file tree with an **empty** editor and the caret outside it must not prefix a separator space (and a fix here was needed: the test that *named* this case never moved the caret out of the editor, so it passed through the caret-inside path — the setup now focuses the editor first, which is what makes a foreign selection survive jsdom's `focus()`). **Segment 27:** `AgentThread.test.tsx` — a retry of a message that carries **no attachments** (`attachments` is optional on `AgentMessage`, so the ordinary retry has `undefined` here; every previous test supplied them), and a fork of the **newest reply when a retry has left two assistant bubbles in the thread** — the fork point is found by walking back to the owning user message, so the walk must step over the older reply rather than take its immediate predecessor. **Segment 30:** `ApprovalPrompt.actions.test.tsx` - an **unrelated key** (typing, Tab, unmodified Enter) must be inert while the prompt is up and must leave it usable; only Escape and Enter had ever been dispatched, so the last guard's false arm never ran. `Composer.tools.test.tsx` - a **drag event of an unrecognised type** must not be treated as a drop or a leave: nothing attaches and the verdict on screen is unchanged. |
| `error-path` | `MentionEditor.menus.test.tsx`: a failed workspace search closes the menu instead of leaving stale results; a superseded slow answer is dropped; a stale row click with no caret inserts nothing and throws nothing; a newline and a paste with no selection are no-ops. `Composer.tools.test.tsx`: a rejected compaction request re-offers the tool (nothing stays pending); a rejected attachment is reported with its reason while the good path in the same batch still attaches; an unreadable skill catalogue still leaves the `/` menu working from the installed list alone; a refused send keeps the draft. **Segment 14:** `Composer.panels.test.tsx` — a successful install appends the skill to an **empty** draft with no leading space, and to a non-empty one with exactly one separator (asserted on the sent content). `AgentThread.test.tsx`: `forkThread` rejection → toast with the backend's reason; a failed compaction and an unnamed compaction failure (never prints `undefined`); a 30-minute compaction timeout (fake timers); a failed history load with a working retry. `ApprovalPrompt.actions.test.tsx`: a rejected decision shows the reason and re-enables the controls; a failed rule save keeps the request pending; an unparsable capability payload disables both allow buttons while Deny stays live. `useAgentThreadState.abort.test.tsx`: an abort the backend refuses still reconciles. `MessageBlock.behaviour.test.tsx`: a thumbnail that fails to load falls back to the named pill; `openPath` rejecting toasts the missing file. **Segment 10:** `useAgentThreadState.lifecycle.test.tsx` — an **unreadable run row** during a hung-send reconcile leaves the send in place rather than abandoning it on a guess (exercises the `.catch(() => null)` arm); `useAgentThreadState.abort.test.tsx` — a failed history read **surfaces** to the user and `retryHistory` re-reads and clears it; `agentMessageFormatters.test.ts` — an HTTP 402 insufficient-credit failure: the observed heading is the raw `failure.insufficientCreditTitle` because the neutral-title fallback is dead (**bug F7**, pinned on both sides so a fix flips one line); `ApprovalPrompt.actions.test.tsx` — a keyboard event from a `<select>` reaches the select instead of being swallowed. **Segment 13:** `MentionEditor.menus.test.tsx` — an input event with **no selection at all** (a dropped range must not throw on `getRangeAt(0)`; no trigger resolves without a caret, so the menu stays shut). |
| `concurrency` | `MentionEditor.menus.test.tsx`: a superseded debounced search cannot replace a newer result; a stale row click after the caret is gone does nothing. `Composer.tools.test.tsx`: the `/compact` tool is withdrawn while the compaction is pending and while a run streams, so no second request can queue; the drag listener is registered asynchronously and removed on unmount. `AgentThread.test.tsx`: superseded-send late writes; the settle-reload fires once and is skipped while a local send owns the view; a push arriving before `listen` resolves is still seen; listeners, live tick and interval all detach on unmount; a compaction cancelled by unmounting is not reported as a failure. `ApprovalPrompt.actions.test.tsx`: a second Enter while the rule is saving is dropped; the keyboard is inert during an in-flight decision. `ThreadSearch.navigation.test.tsx`: a superseded frame callback does not repaint; a rescan re-anchors to a surviving match instead of leaving the cursor at −1; IME composition suspends the search. `NewConversation.test.tsx`: the workspace-adoption effect runs exactly once so a poll-tick re-render cannot undo the user's choice; the guide cannot start two threads. `useSendMessage.test.tsx`: a superseded send's late `setRecentRun` is dropped; an unmounted send does not release the next conversation's lock. `useAgentThreadState.abort.test.tsx`: a thread handover abandons the outgoing send; focus/visibility listeners are removed on unmount. `useMessagePagingHook.test.ts`: re-entrant `loadOlder`; a page landing after unmount; abort mid-walk. **Segment 10:** `useAgentThreadState.lifecycle.test.tsx` — a prompt composed for **another** conversation is not delivered (the thread switch races the async message load). **Segment 13:** `MentionEditor.menus.test.tsx` — the imperative handle driven **after unmount** (`clear`/`restore`/`getContent` must no-op rather than touch a detached editor). **Segment 14:** `Composer.panels.test.tsx` — a composition `requestAnimationFrame` still pending at unmount is not cancelled, so `onChange` runs against a detached editor; the draft must be persisted from the last known text (removing the fallback makes the test fail, so it discriminates). **Segment 15:** `sendPipeline.test.ts` — a send superseded mid-stream must **persist the durable work but repaint nothing**, asserted on all three outcome paths (interrupted stream, user-cancelled, and attach-failed-but-durably-completed); deleting one guard makes the test fail with `expected [ 'partial answer' ] to not include 'partial answer'`, i.e. one conversation's reply painted into another. **Segment 16:** `ThreadSearch.navigation.test.tsx` — a prepare superseded mid-flight must not write state: the stale resolve is discarded (with the guard removed the panel freezes at `1 / 1` instead of `1 / 2`), and the stale *rejection* is treated as a supersede rather than a search failure. **Segment 18:** `useAgentThreadState.lifecycle.test.tsx` — a `visibilitychange` while the tab is **hidden** must not reconcile (and must not abandon a send the user cannot see); the sibling abort-suite test asserts the same "no reconcile" on a view with no hung send, which passes either way. **Segment 22:** `MentionEditor.menus.test.tsx` — a **stale search rejection** must not clear a newer query's results (remove the guard and the menu is cleared out from under the on-screen query); pressing Enter when the **highlighted slash row no longer exists** (the `skills` prop shrank under an open menu) must be inert rather than crash - asserted by capturing `window`'s `error` event, because React raises the failure asynchronously. **Segment 23:** `ThreadSearch.navigation.test.tsx` — a **near-miss re-anchor**: mutating `Text.data` keeps the node identity while the match offset moves (the `startOffset` operand), and `splitText` keeps the original node while moving the tail to a sibling so a match at the same offset ends in a different node (the `endContainer` operand); plus Enter pressed in the search field with **zero matches**, which a keyboard user reaches even though the buttons are `disabled`. **Segment 24:** `sendPipeline.test.ts` — the **same superseded-send harness on the two paths the earlier three tests did not reach**: a run cancelled with **no text landed** must not finalize its stopped bubble into the conversation now on screen (and must not fire the finalization refresh, while `createRun`'s own refresh still fires exactly once outside the guard), and a **failed** send that was superseded must not refresh the sidebar while still writing the durable failed row. Deleting either guard fails a *different* assertion - `expected true to be false` for the first, `expected "vi.fn()" to not be called at all` for the second - which is what makes the attribution clean. **Segment 26:** `MentionEditor.menus.test.tsx` — the two composition-frame races. A `compositionend` schedules its re-check one frame later, and **a new composition can begin inside that frame**: the re-check must be skipped (the frame re-reads the ref rather than a value captured at schedule time), asserted as "no second change report" - removing the re-read makes it `expected "vi.fn()" to be called 1 times, but got 2 times`. And when the composed text lands with the **caret already gone** (ranges cleared), the frame must skip the caret reveal rather than call `getRangeAt(0)` on an empty selection - a throw raised inside a rAF callback escapes any `try`/`catch`, so it is captured with a `window` error listener and the assertion is that the list is empty. **Segment 28:** `useThreadMessages.events.test.tsx` - a **failed page read superseded by a session replacement** must not surface its error on the conversation that replaced it (the rejection is driven from a deferred page read taken *while* the replacement happens, then asserted with `historyError` still null and the replacing thread's own rows present). Recorded with a corrected mechanism: the swallow comes from the post-await epoch check, not from the catch's own re-check, which is dead (see §5). **Segment 29:** `useThreadMessages.events.test.tsx` - a **run row cached by a replaced conversation** must not become the replacing one's `recentRun`. `setRecentRun` is exposed to the parent, so a parent can invoke the outgoing conversation's captured setter after a session switch; the test asserts both halves - the replacing conversation's own setter *does* write, and the stale one is refused - and replacing the guard with `if (true)` fails it (the dead conversation's run row wins). `hits` `[68, 0]` → `[68, 1]`. **Segment 31:** `useStickyAutoScrollHook.test.ts` - the **echo of the hook's own anchor correction** must not re-derive stickiness: the correction lands inside the follow threshold, and mistaking its echo for a user scroll flips auto-follow back on and drags a mid-history reader to the bottom on the next layout change (mutant: guard forced true → `expected 1000 to be 800`). `useRunReattach.events.test.tsx` - the **listener registration resolving after unmount** must not re-arm a stopped tick (the arm is closed, but measured the guard is redundant - F12). |
| `property` | `AgentThread.test.tsx`: `it.each` over `completed`/`failed`/`cancelled` — one contract for the whole terminal set; the four `decide`-related invariants (locked while deciding, unlocked after). `AgentActivityList.test.tsx`: a 4×2 table of kind × status labels, plus a table of burst labels; the "running ⇒ `animate-pulse`, settled ⇒ not" invariant. `ApprovalPrompt.actions.test.tsx`: a table over the four mapped kinds plus one unmapped fallback; `already_compacted` true/false → two distinct messages; three path spellings for the extended-prefix rule. `useRunReattach.events.test.tsx`: the terminal-status table plus "every push for another thread/run is inert". `useWorkspaceForm.test.tsx`: path equality is invariant under case **and** a trailing separator. `ThreadSearch.navigation.test.tsx`: the wrap-around invariant in both directions. Pre-existing invariants kept: `messageHash` stability/distinctness, `computePageStart`'s user-boundary table, `dailyRecommendationLimit`'s release/test switch. |
| `serialization` | `MentionEditor.menus.test.tsx`: `getContent()`/`restore()` round-trip for text, file pills and skill pills; brackets in a label neutralized to parens; a whitespace/paren path angle-wrapped; `<div>`/`<p>`/`<br>` wrappers serialized as newlines; a comment node contributes nothing; `restore()` does not fire `onChange`; a `/token` becomes a pill only once the skill is installed. `buildContinuePrompt.test.ts`: tool `input` decoding (JSON `{command}`, non-JSON text, `null` → the tool name); a terminal event with `payload` absent *and* `null`. `AgentThread.test.tsx`: the compaction event payload round-trip (`operation_id` string vs missing, `error` string vs missing, `already_compacted` boolean) including an event buffered before the operation id is known. `ApprovalPrompt.actions.test.tsx`: `actionPayload`/`saveSuggestion` as JSON strings, as parsed objects, and as garbage; the capability `targets[].scope` round-trip. `useSkillRecommendation.test.ts`: the day-state payload across three backend vintages (`null`, a rejected read, a half-migrated row). `threadSearchCache.test.ts`: `segments` joined and markdown nodes flattened identically to `MarkdownContent`. `threadMessageCache.test.ts`: a snapshot is keyed by `(threadId, agentSessionId)`. **Segment 12:** `Composer.panels.test.tsx` — the skill-description localization chain resolved from **both** sources, the installed skill's own `descriptionZh` *and* the platform catalogue's when the installed entry lacks one (the two operands of the same `||`); `threadRunProjection.test.ts` — a **legacy run row** with `startedAt` but no `endedAt`/`updatedAt` and no `errorMessage` still yields a bubble with a finite timestamp (the three-deep `??` chain). **Segment 19:** `useAgentThreadState.lifecycle.test.tsx` — a run row with `createdAt` but no `startedAt` anchors the streaming bubble's `runStartedAt` on `createdAt` (the third arm of that chain is dead by construction, §5). **Segment 20:** `threadRunProjection.test.ts` — a malformed `createdAt` (user *and* assistant) leaves the orphan detection's trust evidence unchanged, and a status-less message keeps the run's own status. |
| `platform-cfg` | **Not N/A — corrected this segment.** The earlier claim here was "N/A by construction: no `platform` fork anywhere in the subtree, jsdom exercises the DOM path only". The first half is literally true (there is no `process.platform` / `#[cfg]`-style fork) but the conclusion was wrong: `Composer.tsx`:910-920 chooses the sandbox tier's description by `isWindows` / `isLinux`, which `src/lib/platform.ts` derives from **`navigator.userAgent`** — a runtime, *testable* fork, and a branch audit found all of its arms uncovered because any single machine reaches only one. `Composer.platform.test.tsx` now pins that module (through **getters** in the `vi.mock` factory, since a plain vendor export is snapshotted at import) plus the `useSandboxAvailability` hook, and exercises the checking, Windows, Linux and fallback arms in one run. Together with the other platform-sensitive rules already covered — Windows extended-length path prefixes in `ApprovalPrompt.actions.test.tsx`; `/`-and-`\` separators plus case- and trailing-separator-insensitive path equality in `useWorkspaceForm.test.tsx`; macOS `Cmd+Enter` **and** Windows `Ctrl+Enter` plus the Chinese-IME composition path (`isComposing`, legacy `keyCode` 229) in `ApprovalPrompt.actions.test.tsx`, `ThreadSearch.navigation.test.tsx` and `MentionEditor.menus.test.tsx`; a per-model vision flag in `Composer.tools.test.tsx` — the dimension has real evidence rather than an N/A. **Segment 17 adds a genuinely cross-engine case:** the wheel `deltaMode` scaling, where Chromium reports PIXEL (0) but **Firefox reports LINE (1)** and some engines PAGE (2) — `useMessagePagingHook.test.ts` now dispatches all three and asserts the resulting scroll position, so the engine-dependent rule is pinned rather than assumed. Known gap: `samePath` does not unify separator *direction* (F1). **Segment 25 adds the momentum-tail counterpart:** WebKit reports the uncancelable tail of a gesture as a wheel with `cancelable === false`, and `useMessagePagingHook.test.ts` now drives that case past the top to assert the load still happens while `preventDefault` is unavailable — the spec-defined no-op that makes the guard at `:436` non-discriminating (F11). |

**A cross-document contract these rows must respect (found and fixed this segment).**
`docs/testing/dimension-matrix.md` is *derived* from module docs by
`.future/cov100/gen-final-docs.py`. Its `cell_for(module, dim)` walks every table row in
the module's docs, keeps those where **any cell contains the dimension word**
(word-boundary match), and then picks the **longest** non-dimension cell across all of
them. Two consequences follow, and the first one bit this doc:

* **A long row that merely *mentions* another dimension's word can hijack that
dimension's cell.** The `platform-cfg` row below used to contain the phrase *"since a
plain **property** is snapshotted at import"* (describing why the platform mock needs
getters). That made the row a candidate for the `property` query, and at 2186 characters
its cell beat this doc's real `property` cell (1002) — so the matrix's desktop/`property`
cell quoted the **platform-cfg** evidence. Rephrasing it to "a plain vendor export is
snapshotted at import" preserves the point exactly and removes the false candidate;
after that change the `property` cell resolves to genuine property evidence (from
`desktop-packages.md`'s round-trip table, which the matrix documents as its aggregation
rule for the module). **Verified by replaying the generator's own selection logic over all
10 desktop docs: before the fix `desktop/property` was the only wrong-dimension cell
across all 8 modules; after it there are zero.** The rule for future edits here: when a
row grows long, do not let it contain another dimension's bare word.
* **The longest-cell rule can still quote the wrong *row* of the right dimension.** It
does so today in `module-loop.md` (which also serves `future-loop` and `future-rpc`):
`boundary` resolves to a `store_api_drive` test-table row (503 chars) rather than the
doc's own `boundary` row (276), `error-path` to a row labelled `dimensions` (327 vs 241),
and `property` to a `steer_poll_arms` row (559 vs 214) — six matrix cells that name a
plausible-but-different row. Those docs are outside this task's write set, so the finding
is reported rather than fixed. `module-mobile.md` resolves all six dimensions to rows
labelled `mobile` (a different table layout), which the label check cannot judge and a
reviewer should confirm by reading.

## 4. `weak-tests-fixed`

* `useMessagePagingHook.test.ts` → *"clears pending cooldown and render work on
  unmount"* had **no assertion at all** (it only commented "no post-unmount
  setState warning/crash"). It now asserts the cooldown was armed
  (`coolingDown === true`, container `overflow-y: hidden`) and that the cleanup
  released the native scroll lock (`overflow-y` restored, priority cleared). A
  removed `releaseNativeScrollLock()` in that cleanup now fails the test.
* `useSendMessage.test.tsx` no longer calls `vi.restoreAllMocks()`: in Vitest 4
  that restores *module* mocks to the real implementation, which silently turned
  later tests in the file into real-pipeline runs (a weak test that passes for
  the wrong reason). It was replaced with per-test mock re-arming in
  `beforeEach`.
* Four tests were corrected because they asserted a plausible-but-wrong
  contract — the same class of weakness as no assertion at all, because they pass
  or fail for the wrong reason: `AgentThread`'s "an explicit initial mode wins"
  (an explicitly requested workspace outranks the mode); `useMessagePagingHook`'s
  "adopts the first message of a loaded page" (the window is pinned only after
  the caller merges the page); `ApprovalPrompt`'s copy/deny labels; and
  `AgentActivityList`'s "`count: 1` pluralises". Each was fixed against the
  observed behaviour, not by weakening the assertion.
* **A waiver row was miscategorized, found by auditing the ledger against the raw
  report, and the honest fix was to cover the lines rather than re-label them.**
  `useAgentThreadState.ts`:201/232/265 was filed as `attribution-artifact` —
  "spans the provider attributes without a count inside statements every test
  reads". That explanation was wrong. Listing the provider's own zero-hit
  statements from `coverage-final.json` showed **real uncovered statements**:
  two guard `return;`s (the `!threadId` head of `handleAbort`, and the
  `pendingPrompt.targetThreadId !== thread.id` guard on the fast-thread-switch
  path) that never fired because every fixture satisfied them, and
  `retryHistory`'s arrow body, which **nothing called at all** — its only consumer
  is `AgentThread.tsx`:503's retry button, and `AgentThread.test.tsx` mocks the
  whole hook, so the real implementation had no test. The same audit turned up a
  fourth: line 151's `.catch(() => null)` on the hung-send reconciler, invisible in
  the *line* count because another statement on that line ran. Four behavioural
  tests now cover all of it — a failing history read surfacing and the retry
  clearing it, abort with no conversation open, a prompt composed for another
  conversation not being delivered, and an unreadable run row leaving a hung send
  in place — and the file has **no uncovered statement left**, so the row is
  deleted rather than re-labelled (§5). Three of those four are new *dimension*
  evidence too: error-path (unreadable run row, failed history read), boundary
  (no thread open), concurrency (a prompt racing a thread switch). The lesson:
  **an `attribution-artifact` claim must be evidenced the same way as an
  unreachable one** — if the raw statement map shows a zero count on a statement
  that *can* execute, the category is "untested", not "artifact".
* **A test name is not evidence — two tests promised coverage they did not give.**
  `Composer.panels.test.tsx`'s *"uses the platform catalogue's Chinese text for a
  skill description"* asserts the **English** `Paper`, and no Composer test ever
  called `i18n.changeLanguage`, so the entire `useZh` true-arm was uncovered while
  the test's name said otherwise. Similarly `ApprovalPrompt.actions.test.tsx`'s
  escalation test sets `category: "sandbox_escalation"` in the payload while the
  title branch keys off `approval.kind`, so it never entered the arm it looked
  like it covered. Neither was a *weak assertion* — the assertions were real and
  passed for the right reason — they were **wrong fixtures**, which is harder to
  see. Both now have fixtures that reach the intended arm, and the misleading name
  is kept with a corrected comment rather than silently renamed, so the next
  reader can see what happened.
* **I audited my own new tests for vacuous assertions, and tightened six.** The
  arm campaign kept showing that an assertion can pass for the wrong reason, so I
  swept every `toBeTruthy()` / `toBeDefined()` / `not.toThrow()` in this subtree's
  specs (19 sites) and classified each. **`toBeTruthy()` went from 7 calls to 2**,
  and both survivors are legitimate **precondition guards** inside helpers —
  `ApprovalPrompt.test.tsx`'s `button()` asserting it found the control before
  clicking it, and `Composer.reasoning.test.tsx` checking a menu item exists before
  a click whose effect is then asserted. Three of the removed ones genuinely
  discriminate (the value is `undefined` when the behaviour breaks) but are pinned
  to exact copy now anyway:
  `threadRunProjection.test.ts` — the recovered bubble's `terminationTitle` is
  `"Model service error"` and its `terminationNotice` is
  `"The run failed. Please try again later."` (the latter replacing a
  `not.toContain("undefined")` containment check, which a notice reading
  `"undefined happened"` would have passed);
  `useAgentThreadState.abort.test.tsx` — `historyError` is the backend's own
  `"db is locked"`, not a generic string; `useThreadMessages.events.test.tsx` — the
  tie fixture's notice/title pinned to `"Please try again later."` /
  `"Model service error"`, in both the tie test and its short-circuit sibling.
  **The tightened assertion was then re-mutated** (`>=` → `>`) and still fails
  exactly one test, now with a precise message
  (`expected undefined to be 'Please try again later.'`) rather than a vague
  truthiness failure — so tightening added discriminating power instead of hiding
  it. Coverage numbers did not move, which is itself the evidence that these were
  quality fixes rather than coverage fixes.
* **Two tests whose name or comment promised an arm their body never reached** (segment 26) — the same "wrong fixture" class, found by *reading the arm's hit count while trying to cover it*:
  * `MentionEditor.menus.test.tsx` — *"avoids a doubled space when the editor is empty and the caret is outside"*. The body called `insertMention` with no selection setup at all, so `insertMention`'s own `editor.focus()` parked a caret **inside** the editor and the test passed down the caret-inside path; the `if (!isEditorEmpty(editor))` arm it named stayed at zero. The fix is a real setup (focus first, then park a **detached** selection), which is what makes jsdom leave a foreign selection alone — see the harness note. The assertion was already exact and did not need relaxing.
  * `MentionEditor.menus.test.tsx` — *"tolerates the imperative handle being used after unmount"*, whose comment read *"Each entry point must no-op"* while the body drove only `clear`/`restore`/`getContent`. The handle's fourth member, `insertMention` — the one the **file tree** calls, per its own doc comment — was never driven, so its `!editor` guard stayed uncovered *and* the ledger had waived that guard as dead. Both were wrong: driving `insertMention` after unmount now covers the arm (and retires the waiver). A comment claiming "each" or "all" is a coverage claim and has to be checked like one.
* **Known and deliberately not fixed: five pre-existing `act()` warnings in `MentionEditor.menus.test.tsx`.** React reports *"An update to MentionEditor inside a test was not wrapped in act(...)"* against *"does not crash a newline with no caret in the document"*, *"…a paste with no caret…"*, *"hands every keystroke to the IME while composing"*, *"ignores keys that are not ours"* and *"serializes a non-block wrapper element…"*. I checked whether this segment introduced them: each of the four tests this segment touched, run in isolation, emits **none**, and the pre-existing IME composition test emits none either — so the warning is a leftover state update (most likely a real `requestAnimationFrame` from a composition trigger firing after its test finished) surfacing in whichever test is running when it lands. It is a warning, not a failure, and the suite is green with it; fixing it means adding a frame flush to `afterEach` across a heavily instrumented file, which is churn I did not want to spend a reviewer's diff on. Recorded here so it is a known, located item rather than an unexplained stderr line.
* **The throwing-sentinel probe: turning a waiver's argument into a measurement** (segment 32). Every `unreachable-by-construction` row in §5 rests on an argument about callers or types — and the thinnest rows rest on arguments *plus* a small function hit count, which is the weakest support in this document, because "the guard never fired" over three runs is one refactor away from being false. The fix is a four-step probe that needs no test at all: (1) replace the guarded arm's body (its `return;`) with `throw new Error("PROBE-<line>")`; (2) run the **whole subtree**, not just the file's own tests, since a component test can reach a hook's arm; (3) if every test passes, no reachable state produces that arm — empirical unreachability over the tested state space; (4) revert and confirm the file compares clean against HEAD. Applied this segment to the five thinnest rows (`useThreadMessages.ts`:504, whose enclosing function runs twice, and `Composer.tsx`:209/476/505/636, whose run 4/2/3/3 times) — **all five sentinels survived all 1023 subtree tests**, so the entire "thin run count" caveat is now closed by experiment rather than by reassurance. Caveats stated honestly: this bounds reachability *within the tested state space*, so it is evidence for an `unreachable-by-construction` claim rather than a proof of one (a state no test reaches could still exist); and it cannot distinguish "unreachable" from "redundant" — that needs the arm *reached* first, which is why F12 was found by a different route.
* **A wrong unreachability argument, caught by writing the one obvious test** (segment 33). `threadSearchCache.ts`:40/`:41` (`if (typeof node.code === "string")` / `return node.code`) had been filed in §8 as *probably* dead, with the argument that a code node's `code` is a substring of the raw `text`, so the raw-text check at `:20` would have matched first. **The argument was wrong, and the counter-example is one line of test**: `mayContainThreadSearch(message("```ts\nconst answer = 42;\n```"), "zzz-absent")` returns `false` *by running both lines*. The flaw is a conflation — "the code text appears in `text`" is not "the needle matches `text`": `textLeaves` is reached **precisely when the needle is absent**, and at that moment every code node is visited regardless of whether its text is in the raw input. Both lines are now covered and the file has zero uncovered statements. **Rule:** an `unreachable-by-construction` argument that reasons about a value's presence *in the input* must not be confused with the *condition that gets you to the line*; and the cheapest check for either is to write the obvious test and watch whether the line moves — which is exactly how the probe technique above is meant to be used. Had I trusted the argument, this row would have shipped as a waiver over live code.
* **The same probe, applied to the `?? ""` family in its strongest form** (segment 32). Fifteen fallback arms — nine in `MentionEditor.tsx` (`:804`, `:827`, `:831`, `:860`, `:864`, `:880`, `:888`, `:892`, `:893`) and six in `Composer.tsx` (`:303`, `:365`, `:429`, `:430`, `:438`, `:489`) — had been proven unreachable only by substituting a **`"MUTANT"` string**, which is weaker: that experiment shows no test *observes* the value, but a test could execute the arm and never assert on it. Each `?? ""` was therefore replaced by `?? (() => { throw new Error("PROBE-L<n>") })()`, so any execution crashes. **Result: all 69 files and all 1023 tests pass**, which is a strictly stronger statement than the MUTANT run — it rules out execution, not merely observation. Both files compare clean against HEAD afterwards. Worth noting the probe survived compilation untouched, so the IIFE form is safe inside these expressions (including the template-literal site at `:888`) — a syntax error would have failed all 69 files at import.
* **I wrote a test whose comment claimed the wrong mechanism, caught it by measuring, and corrected the comment rather than the claim** (segment 28). The stale-page-read test asserted the right behaviour and passed, but the arm it was written for (`useThreadMessages.ts`:445) stayed at `hits=[3, 0]` before **and** after - which is what sent me looking. `loadFromAgent` never throws (it catches everything and returns `{ status: "failed" }`), so the rejection is absorbed by the post-await epoch check at `:420`, and the catch's own re-check is dead code. **The test was kept** - it pins user-visible behaviour and is discriminating for `:420` - but its comment now names the real guard and states that the dead arm is not credited to it. The generalisable rule, alongside the ones this section already records for test names and "each/all" comments: **a test's stated mechanism is a claim, and the hit count is what checks it.** A passing test with an unmoved arm means the comment is wrong or the arm is dead; it does not mean the test is weak.
* **A substring class check is not a behaviour assertion.** Two new tests in
  `MentionEditor.menus.test.tsx` first asserted
  `row.className).not.toContain("bg-surface-subtle")` for the un-highlighted row;
  that passes only by accident, because the *unhighlighted* rows carry the
  Tailwind variant `hover:bg-surface-subtle`, which contains the substring by
  construction — so the assertion could never fail. Replaced with
  `classList.contains("bg-surface-subtle")` behind a `highlightedIndex()` helper
  that asserts the index, not the class text.
* Two new tests asserted a *different* bug: `Contains("No files")` against the
  real copy `"No matching files."` would have passed while the panel showed
  nothing at all if the label changed; the exact-equality form is kept.
* **The substring-in-a-class-name trap recurred in segment 5, in a test I wrote
  myself.** Asserting `!className.includes("opacity-100")` for the un-hovered fork
  button could never pass: the button's *base* class list contains
  `focus-visible:opacity-100`, so the substring is present on every row whatever
  the hover state. Caught by the test failing, fixed by asserting class *tokens*
  (`classList.contains(...)`). Both sides — painted and painted out — are now
  asserted in `MessageBlock.states.test.tsx`. Recorded twice on purpose: it is a
  mistake that is easy to make in a second file.
* A test I wrote and then **deleted rather than weakened**: "does not send
  without a model to send to". `Composer.submitValue` deliberately does not gate
  on the catalogue being empty — that gate is the parent's `disabled` prop
  (asserted in `AgentThread.test.tsx` and `NewConversation.test.tsx`). Asserting
  it here would have pinned a contract this component does not own.
* **Mutation-style sensitivity probe (this is the goal's `以变异测试客观验证测试有效性`
  axis, done without touching product code).** I wrote a throwaway spec asserting
  the **inverse** of six behaviours this subtree pins, one from each dimension, and
  confirmed every inverse **fails** — an inverse that passed would prove the
  matching real assertion could never fail:
  * `truncate`'s "a 1200-char body is not truncated" (boundary) — failed as expected
  * Batch 2 (3 more inversions, child-mocked `MessageList`): the tail assistant's
    recovery source is the *preceding* user message and not the earliest one; an
    assistant-only tail has no recovery source; only the last row carries `isLast`
    — all three inversions failed as expected.
  * Batch 3 (8 more inversions, pure functions): a bare prompt carries no summary
    section; a failed run-summary read yields an explanatory line and not empty
    text; a failed tool-output read does not lose its neighbour's row; a
    comment-only URI list yields nothing; plain text parses as no mention; a
    dotless name and a dotfile both expose no extension; a `.ts` path is not an
    image — all eight inversions failed as expected.
  * `localPathsFromUriList("file://")` → `["/"]` (platform-cfg) — failed as expected
  * `parseMentionSegments` matching the `<./a b/...>` form (serialization) — failed as expected
  * "an emptied draft is still persisted" (error-path/serialization) — failed as expected
  * a 6-push burst yielding 7 projections instead of 1 trailing run (concurrency) — failed as expected
  * **one inverse PASSED, and that was a real finding about my own spec**: asserting
    the `"x" * 1199 + "..."` suffix is true even for correct output, because a suffix
    of repeated identical characters is satisfied by a longer run too. The original
    assertion therefore caught a `cap=1199` mutation but **not** a `cap=1201` one.
    Fixed by putting a marker character at the cap (`x*1199 + "Z" + x*200`) so a
    single line catches both directions — verified by modelling `truncate` at
    caps 1199/1200/1201 (assertion holds only at 1200). The lesson is in the spec as
    a comment: a boundary assertion on repeated characters pins only one bound.
  * **Scope limit of this evidence, stated plainly:** 17 inverted assertions across
    the six dimensions is *not* a mutation score. There is **no TypeScript mutation
    harness in this repo** — `verify.py mutation` reads `mutation/summary.json` from a
    **cargo-mutants** job only, there is no stryker/mutant tooling in
    `package.json`, and no `mutation/` directory exists. So the objective's
    mutation-testing axis is instrumented for Rust only, and an objective per-module
    mutation score for `desktop/` would need a TS mutation runner that does not exist
    yet and is outside this task's write set.
  * **What I *could* do — and did — is run real mutants by hand.** Five batches,
    **22 mutants across 14 product files** inside this write set — pure modules and
    hooks first, then the React components so the jsdom behaviour tests are exercised
    too, then the concurrency dimension. Each was applied one at a time with the file
    verified restored after (every mutated file matches HEAD — `git status` shows no
    product file modified, a stronger check than my own recorded hashes):
    **16 killed outright and 5 that survived, were strengthened, and were re-mutated
    to confirm the kill — plus 1 control that was expected to survive and did. None
    is left open.** The survivors were the value of the exercise — they exposed a
    missed exact-cap boundary (`attachments.ts`), a missing stale-schema case
    (`composerDraft.ts`), a **singleton fixture that made an existing boundary
    assertion unable to discriminate** (`buildContinuePrompt.ts`, which would have
    let the "continue" flow quote an unrelated later user message), a **prop no test
    could even observe** (`AgentThread.tsx`'s `compactionInProgress`, invisible
    because the test's `Composer` mock never exposed it), and a **run-window equality
    whose fixture had the wrong shape** (`useThreadMessages.ts`, where every page
    fixture emitted user entries only so the reply had no row for a run outcome to
    attach to). Full tables and all three lessons in §2's *Mutation validation*.
    This is stronger evidence than the inversion probe — it measures the real code,
    not a re-statement of the contract — and it is what turned the five weak spots
    up. The last batch's two mutants (paging's cooldown guard, the snapshot-cache
    eviction bound) both died, and that negative result is the point: the concurrency
    and resource-boundary code was already genuinely pinned.
* No snapshot-only tests and no `it.skip`/`xit` exist in this subtree
  (`verify.py skipped` finds none here), so nothing had to be deleted for that
  reason.
* Two more gotchas this segment, both of which cost real time:
  **`vi.clearAllMocks()` does not clear a `mockImplementation`** (only the call
  history), so an implementation installed by one test leaks into every later test
  in the file — three tests passed alone and failed in the suite until the hook
  switched to `vi.resetAllMocks()` plus per-test defaults; and **awaiting, inside
  one `act`, a promise whose completion depends on that act's flush deadlocks** —
  React defers the state flush until the callback resolves while the awaited
  promise is waiting for the flush. The search tests start the promise, let `act`
  flush, and only then await it.
* Reusable testing gotcha: **a rejecting `act(async …)` scope corrupts React's
  act queue** for the rest of the file — later renders silently no-op and tests
  fail with a confusing `Cannot read properties of undefined`. Keep
  `await expect(promise).rejects…` outside an act scope, and await any promise
  created inside one before starting the next ("overlapping act() calls").

## 5. Waiver ledger — `lines-100-or-waived`

Line numbers are from the final measurement. There are **13 rows across 9 files**
(14 lines), and each carries its policy category on the same line as the path:

* **12 `unreachable-by-construction`** — each a defensive duplicate of a check its
  caller has already made, so the state the guard rejects cannot be constructed
  through any entry point. Four of them are one-line guards inside `Composer.tsx`
  and `useThreadMessages.ts` that section 7's steering asked to be *tested out*
  rather than waived: I tried, found each to be pre-empted by its own caller's
  gating, and recorded the caller that does it in the row.
* **1 `unreachable-in-this-environment`** — `liveStreamTick.ts`:92. This row was
  `unreachable-by-construction` and **wrong** until segment 5: the line *is*
  constructible (a throwing `project`/`afterProject` with a push already queued),
  but reaching it emits an unhandled rejection, which vitest fails a file for, and
  the config that would tolerate it is outside this task's write range. I built the
  driver, watched the line execute under an `unhandledRejection` listener, and
  watched the file fail anyway — the measurement, not an argument, is what the row
  now rests on.
* **0 `attribution-artifact`** — this row existed until segment 10 and was **wrong**.  It covered `useAgentThreadState.ts`:201/232/265, described as spans the report
  attributed without counting. Reading the provider's own `coverage-final.json`
  showed them to be real zero-hit *statements*, not attribution noise: two guard
  `return;`s that never fired because every fixture satisfied the guard, and
  `retryHistory`'s arrow body, which nothing called at all (the only consumer is
  `AgentThread.tsx`:503's retry button, and `AgentThread.test.tsx` mocks the whole
  hook). Three behavioural tests now cover those paths plus a fourth statement the
  same audit turned up on line 151, and the file has no uncovered statements left —
  see the correction note in §4. All four statements were reachable, so
  `attribution-artifact` was the wrong category and no replacement row is needed.

**Completeness check, verifiable from the report.** For a line-oriented ledger like
this one, a gap can hide where a *line* is covered but a statement on it never ran —
that is exactly how `useAgentThreadState.ts`:151 escaped until segment 10. So the
audit was repeated as a statement-level sweep: for all 44 files, list every
zero-hit statement in `coverage-final.json` whose line also has at least one
*executed* statement. **The result is now zero files** — every zero-hit statement
in the subtree lies on one of the 17 waived lines above, so the ledger is complete
at statement granularity, not merely at line granularity. A reviewer can re-run
that check in a few lines of Python against the report directory.

**Entry check — the evidence that separates "unreachable" from "untested".** The
`unreachable-by-construction` category is only meaningful if the guard's *function*
actually runs; if the whole function were never entered, the row would be
mis-labelled and the honest category would be "untested" (which is how
`useAgentThreadState.ts` turned out). `coverage-final.json` carries per-function
counts in `fnMap`/`f`, so this is checkable — and **all 17 waived lines sit inside
functions with a non-zero hit count**:

| waived line | enclosing function | function hits |
|---|---|---|
| `liveStreamTick.ts`:92 | `drain` | 64 |
| `threadRunProjection.ts`:466 | `isSupersededCompactionDivider` | 5 |
| `threadMessageCache.ts`:48 | `setThreadMessageSnapshot` | 150 |
| `ApprovalPrompt.tsx`:95 / `:141` | `decide` / `confirmCapabilityRules` | 10 / **3** |
| `MentionEditor.tsx`:158 | MutationObserver effect | 165 |
| `MentionEditor.tsx`:390 | `insertMention` | 9 |
| `MentionEditor.tsx`:801 / `:876` | `isEditorEmpty` / `serialize` | 547 / 125 |
| `AgentThread.tsx`:355 | `handleCompactContext` | 11 |
| `NewConversation.tsx`:141 | workspace-adoption effect | 38 |
| `useThreadMessages.ts`:425 / `:504` | `loadFromAgent` / indicator timer | 21 / **2** |
| `Composer.tsx`:209 / `:476` / `:505` / `:636` | see §4 | 4 / **2** / **3** / **3** |

The bolded counts are the honest weak spots and are stated rather than glossed:
`useThreadMessages.ts`:504's function runs **twice** and `Composer.tsx`:476's twice,
so "the guard never fired" rests on a thinner base of runs than a 547-count row
does. **The `:504` row has since been re-probed and is no longer the weakest:** a throwing
sentinel in its arm survived the whole 1023-test subtree, so the thin run count is no
longer the only support it has. **The four `Composer.tsx` guards were probed the same way in
the same pass, and all four are also unreachable:** a `throw` installed at each `return;`
(`:209`, `:476`, `:505`, `:636`) was reached by **none** of the 1023 subtree tests. So the
whole "thin run count" caveat is closed — every row it named is now backed by an
experiment that would have failed loudly had the arm been reachable, and all probes were
reverted (both files compare clean against HEAD). That does not change the category — the condition is pre-empted by a caller
in each case, and §4 names the caller — but a reviewer probing this ledger should
start with the low-count rows, because a guard whose function runs twice is one
refactor away from being exercised in a state nobody anticipated.

No row claims a waiver for a line I did not try to cover. A reviewer with a
counterexample should send the row back rather than accept it: the two weak
candidates are named above (`liveStreamTick.ts`:92 and the `?? ""` fallbacks that
are documented as dead in §2 but which the gate does not require as rows, because
they cost no *lines*).

* `desktop/src/features/agent/liveStreamTick.ts`:92 — `unreachable-in-this-environment`, **re-decided**: the earlier `unreachable-by-construction` claim in this row was **wrong** and is retracted. `void drain()` in the `finally` re-arm is reachable, and the one path is an exception: `project` is an awaited callback, so holding a projection in flight, letting a `request()` land (which sets `queued = true` and returns early because `running` is still true), and then having `project` reject — or `afterProject` throw — carries the exception into the `finally`, where `queued && !stopped && isActive()` is all true and the re-arm fires. What makes it un-coverable is the *environment*, not the construction: `request()` and the re-arm both call `drain()` with a bare `void`, so that rejection is unobserved by anything in the module, and Vitest fails the whole test file on an unhandled rejection. I built the driver and ran it: my own `process.on("unhandledRejection")` listener did observe the rejection (proving the line executes), and the file was still marked failed and exited 1; `desktop/vite.config.ts` is outside this task's write range, so the flag that would tolerate it (`dangerouslyIgnoreUnhandledErrors`) cannot be set. See finding **F6** — the same bare `void` is why the guard's stated purpose ("a request … would otherwise be stranded") is not actually achieved on its only path. Even with no test, the neighbouring code is proven to run by *"stops permanently and drops a queued request"*, *"stops driving when the owner goes inactive"*, *"coalesces pushes that arrive while a projection is in flight"* and *"runs a leading projection immediately and coalesces a burst into one trailing run"* (7 tests in `liveStreamTick.test.ts`).
* `desktop/src/features/agent/threadRunProjection.ts`:466 — `unreachable-by-construction`. `return false;` in `isSupersededCompactionDivider` for a message that is not a compaction divider. The only call site is `!isCompactionDivider(lastAssistant) || isSupersededCompactionDivider(lastAssistant, …)`, so the predicate is already true by the time the body runs; the type system cannot express "the caller only calls me when this holds". Whole-file projections in `threadRunProjection.test.ts` / `.async.test.ts` prove the surrounding code ran. **Its *branch* arm at `:465` is the same dead path** (the `if`'s then-branch is this `return`), and the ledger's census counts the arm as explained for that reason. **A second, distinct dead arm in the same function is `:467`** — `return (message.segments ?? []).some(...)`: the `?? []` can never be taken, because the L465 guard requires `isCompactionDivider(message)`, and that predicate itself demands `message.segments?.length === 1`. So `segments` is provably defined at every point this line runs.
* `desktop/src/features/agent/threadMessageCache.ts`:48 — `unreachable-by-construction`. `break;` for `oldest === undefined` inside `while (snapshots.size > MAX_CACHED_THREADS)`: a `Map` with more than 12 entries cannot yield `undefined` from `keys().next().value`; the check exists only because the DOM lib types that value as `string | undefined`. Eviction itself is covered by *"bounds snapshots and refreshes recency on reads"*.
* `desktop/src/features/agent/ApprovalPrompt.tsx`:95 and `:141` — `unreachable-by-construction`. `if (deciding) return;` in `decide` and `if (deciding || !saveSuggestion?.rules) return;` in `confirmCapabilityRules`. Every caller is a control rendered with `disabled={deciding !== null}`, or the global key handler whose own first line is the same guard; and the capability function's only entry point is the "allow in this workspace" button, which routes to it only when `saveSuggestion.rules` is truthy. `ApprovalPrompt.actions.test.tsx` covers both functions themselves (a successful save, a failed save, a refused decision, an in-flight decision).
* `desktop/src/features/agent/MentionEditor.tsx`:158 — `unreachable-by-construction`. `if (!editor) return;` in the MutationObserver effect. The effect runs after the commit that attaches `editorRef`, and the editable div (`ref={editorRef}`) is **rendered unconditionally** — every branch of the JSX reaches it, so the ref is non-null for every run of this effect. Measured in a scoped run: **`hits=[0, 76]`** — 76 effect passes, zero with a null editor. **`:390`, the sibling `!editor` guard in `insertMention`, was waived on the same reasoning and is now RETIRED — it was reachable, and I covered it.** A parent can hold the handle past the child's unmount (an async send settling after the view is gone) and call `insertMention`, which is the fourth member of the documented handle and the one the file tree calls; the post-unmount handle test now drives it. This is the *second* time this file's ledger note has had to retire a `!editor` waiver — `:801`/`:876` were retired in segment 13 for the same reason — and the lesson recorded there applies verbatim: **a guard behind the imperative handle is reachable after unmount even though it is not reachable through the mounted component.** 89 tests across `MentionEditor.menus.test.tsx`, `.test.ts`, `.placeholder.test.tsx` and `.scroll.test.tsx` prove the surrounding code ran, including both `getContent()` and the empty-state reconciliation.
* `desktop/src/features/agent/MentionEditor.tsx`:189 — `unreachable-by-construction`. `else if (segment.text)` in `restore`'s loop. The false arm would need a parsed segment with EMPTY text, and `parseMentionSegments` cannot produce one: a non-mention segment is pushed only when its slice is non-empty (`match.index > last`, and `last < content.length` for the tail), and a mention segment's text is `match[1]` from `/\[([^\]]+)\]/`, whose `+` requires at least one character. So `segment.text` is truthy for every segment the parser emits, and this arm is dead for every caller - `restore` is the only consumer and it always feeds the parser's output.
* `desktop/src/features/agent/MentionEditor.tsx`:394 — `unreachable-by-construction`. `const caret = selection && selection.rangeCount > 0 ? selection.getRangeAt(0) : null;` in `insertMention` — the `: null` fallback (and the `else` branch it feeds, which appends at the end with a leading space) were uncovered, and the reason is structural: `insertMention` calls **`editor.focus()` two lines earlier** (L391), and focusing a contenteditable always leaves a caret, so `selection.rangeCount > 0` is true by the time the line runs. I tried to reach it three ways and measured each: a **file-row click** with the selection cleared inserts *nothing* (the row path goes through `insertFile`, which bails at `if (!editor || !context)` because `mentionContext` reads the caret to find the `@` trigger - so it never reaches `insertMention`); calling the exposed handle with the selection cleared puts the pill at the **front** of the existing text (`"[alpha.ts](./src/alpha.ts) see"`), which is the *true* arm running on the caret at offset 0 that `focus()` synthesised; and the same holds with `disabled: true`, so jsdom's focus does not skip the range. The `else` branch is therefore reachable only through the other falsy path - a caret **outside** the editor. **Both of that branch's arms are now covered**: the `if (!isEditorEmpty(editor))` at `:407` by two tests - a non-empty draft (leading space inserted) and an **empty** editor (no leading space), the latter needing `editor.focus()` *before* the foreign selection is parked, because jsdom's `focus()` on a not-yet-active contenteditable replaces the selection while on an already-active one it leaves it alone. Worth noting the measured detail: had the `: null` fallback run as its comment intends, the pill would have been appended after `"see"`, not prepended. The `: null` fallback itself remains dead: `insertMention` calls `editor.focus()` two lines earlier (L391), so `selection.rangeCount > 0` holds by the time the line runs.
* `desktop/src/features/agent/AgentThread.tsx`:355 — `unreachable-by-construction`. `if (!thread) return;` at the head of `handleCompactContext`. That callback is handed to the composer only when `thread?.agentSessionId` is truthy (`onCompactContext={thread?.agentSessionId ? handleCompactContext : undefined}`), so the closure that reaches the body always closed over a non-null thread.
* `desktop/src/features/agent/MessageBlock.tsx`:107, `:252`, `:363` — `unreachable-by-construction`, three independent proofs found in one pass over this file's residual arms:
  * **`:107`** — `message.content ?? ""` on the copy-button text. `AgentMessage.content` is declared **`content: string`** (required) in `packages/thread-projection/src/model.ts`, so the `?? ""` arm needs a message without content, which the type forbids.
  * **`:252`** — `<p className={cn(…, message.stopped ? "text-ink-muted" : "text-ink-soft")}>`. The element is the consequent of the ternary at `:250`, whose condition is `isLast === true && !message.stopped`. The body therefore only runs when `message.stopped` is **false**, so the *true* arm is unreachable — the same "guard doubled by its enclosing condition" class as F8–F11, this time doubled by a ternary rather than a callee or a sibling write.
  * **`:363`** — `<SafeLink href={linkSegment.href ?? ""}>`. `ExternalLinkSegment.href` is optional, but the only producer of a `link: true` segment is `splitExternalLinkSegments`, which sets `href: match[2] ?? ""` from `EXTERNAL_LINK = /\[([^\]]+)\]\((https?:\/\/[^)\s]+)\)/g` — the capture group's `+` requires at least one character past the scheme, so `href` is a non-empty string on every segment this branch can see.
* `desktop/src/features/agent/AgentThread.tsx`:389 — `unreachable-by-construction`. `].includes(detail.eventType ?? "")` in the compaction-event listener. `detail` is an untyped cast of a `CustomEvent`, so the optional `eventType?: string` reads as if a producer could omit it — but the only dispatcher of `future:agent-event` is `integrations/agent/agentStateCache.ts`, which begins `const eventType = typeof p._eventType === "string" ? p._eventType : null; if (!sessionId || !eventType) return;` and passes that **already-narrowed** value at both of its dispatch sites (`detail: { threadId, sessionId, eventType, payload: p }`). A missing `eventType` is filtered out before the event exists, so the `?? ""` arm cannot be reached. Verified by reading both sites, not inferred from the cast.
* `desktop/src/features/agent/AgentThread.tsx`:217, `:293` — **both were live, and both are now covered** (they were the file's remaining unexplained arms): the `source.attachments ?? []` fallback on the **UI retry** path, and the backward walk in `handleFork`. Recorded in §3's `boundary` row with the fixtures that reach them.
* `desktop/src/features/agent/useThreadMessages.ts`:250 and `:314` — `unreachable-by-construction`, both the same "declared wider than the producers" shape, established by reading the *producer* rather than the declaration:
  * **`:250`** — `const entries = result?.entries ?? []`. `getSessionEntriesPage` is `async function …: Promise<SessionHistoryPage>` (`integrations/storage/threads.ts`:191) and `SessionHistoryPage` declares `entries`/`hasMore`/`nextOffset` as **required**, so both the `?.` and the `?? []` need a nullish result the contract does not admit. Contrast `sendPipeline.ts`:138, where the same shape had a stronger proof available (an unguarded sibling dereference) — worth noting that the cheap version of the argument is the producer's return type, and the expensive version is a sibling line that would already have thrown.
  * **`:314`** — `buildStreamingPreview(activeRunId, activeRunStartedAt ?? null)`. `activeRunStartedAt?: number | null` is *declared* optional, but its only producer is `activeRunRef.current.startedAt`, and `activeRunRef` is typed `useRef<{ runId: string | null; startedAt: number | null }>` — **never `undefined`**; the other call site passes the literal `null`. So the `?? null` arm has no input. It is *also* redundant, since `buildStreamingPreview`'s own parameter defaults to `null` (F9's shape), so the arm is doubly safe and singly untestable.
* `desktop/src/features/agent/useThreadMessages.ts`:134 — **was live, now covered.** `if (aliveRef.current && sourceRef.current.owner === source.owner)` in `setRecentRun`. `setRecentRun` is exposed to the parent, so a parent holding the outgoing conversation's callback can invoke it after a session replacement; the closure captures its own `source`, which is what the guard compares against `sourceRef.current`. Covered by *"ignores a run cached by a conversation that was replaced"*, which asserts both halves (the replacing conversation's own setter does write, the captured stale one does not) and whose mutant — the guard replaced by `if (true)` — fails the test with the two run rows' `createdAt` values. `hits` went `[68, 0]` → `[68, 1]`.
* `desktop/src/features/agent/useThreadMessages.ts`:445 — `unreachable-by-construction`. `if (epoch === historyEpochRef.current) setHistoryError(...)` in `loadOlderHistory`'s catch. **Found by writing the test and watching the hit count not move** (`hits=[3, 0]` before and after a purpose-built stale-rejection driver): the two-part proof is that (1) between the post-await re-check `if (epoch !== historyEpochRef.current) return;` at `:420` and the catch at `:444` there is **no `await`**, so the epoch cannot change over that stretch — every throw that reaches the catch (`status === "failed"`, the `"empty"` guard, the cursor-did-not-advance guard) is thrown *after* that check passed, so the condition is true whenever the catch runs; and (2) the only way to enter the catch without passing `:420` is for `loadFromAgent` to throw, which it structurally cannot — it wraps its whole body in `try/catch` and returns `{ status: "failed", error }` at `:333-335` instead of rethrowing. A test that rejects the page read therefore surfaces as a failed *result* and is absorbed by `:420`'s early return. The test I built for it is **kept**, with its mechanism corrected in the comment: it asserts a real, user-visible behaviour (a failed page read superseded by a session replacement is not reported on the conversation that replaced it), it is discriminating for the `:420` guard, and the arm above is recorded as dead rather than credited to it.
* `desktop/src/features/agent/NewConversation.tsx`:141 — `unreachable-by-construction`. `setSelectedWorkspace(first.id)` in the fallback effect. Its guard is `if (activeWorkspace || !first) return;`, but `activeWorkspace` is `options.find(id === selected) ?? options[0]`, so whenever `options[0]` exists `activeWorkspace` is truthy and the effect returns early — the two conditions cannot both hold. `NewConversation.test.tsx` covers both halves (a list that shrinks, and an empty list).
* `desktop/src/features/agent/threadRunProjection.ts`:733 — `unreachable-by-construction`. `segments: projection.segments.length > 0 ? projection.segments : message.segments` in `applyRecoveredEvents`. The `: message.segments` arm needs `projection.segments.length === 0` *at this line*, which forces the early return at `:728` to have been passed by its **other** disjunct — `projection.content.trim()` non-empty. But `content` is built the same way the segments are: `liveApply.ts`:234 does `content += text` and `:240-242` appends that same `text` to a text **slot**, and `buildSegments` (`:623-624`) emits a text segment whenever a slot's `text.trim()` is non-empty. So non-empty content implies a non-empty segment list, and the arm would require both at once. (The projection builder lives in `packages/thread-projection`, outside this write set — read, not modified.)
* `desktop/src/features/agent/threadSearchCache.ts`:17 — `unreachable-by-construction`. The `""` arm of `segment.kind === "text" ? segment.text : ""` inside the `segments.map(...)`. The map only runs when `message.segments?.length` is truthy, and **six lines earlier** the same collection is already filtered by `message.segments?.some(segment => segment.kind !== "text")`, which returns `true` (and leaves the function) for any collection holding a non-text segment. So by the time the map runs every segment is a text segment and the `""` arm is dead — the F8–F12 class again, a guard doubled by an earlier guard on the same operand. **Measured, and promoted from argument to measurement in segment 33:** a throwing sentinel installed in that arm (`? segment.text : (() => { throw new Error("PROBE-17"); })()`) survived **all 1015 tests in the subtree**, so no reachable state executes it. The file was copied aside before mutating and restored from that copy afterwards (verified byte-identical to HEAD) — the same discipline the `git checkout` incident taught, applied on purpose this time. Note the contrast with `:40`/`:41` in the same file: *there* the argument was wrong because the earlier check compared the **needle** to the text, a different thing from visiting nodes; *here* the earlier guard **exits the function** on the very collection the map walks, which is why this row holds.
* `desktop/src/features/agent/threadSearchRanges.ts`:35 — `unreachable-by-construction`. `const value = node.textContent ?? ""`. The `TreeWalker`'s `acceptNode` (`:27-28`) rejects any node without a parent **or without truthy `textContent`** (`if (!parent || !node.textContent || parent.closest(...)) return FILTER_REJECT`), so every node the loop sees has non-null text. Same shape as the `externalLinks.ts`/`mentionMarkdown.ts` `?? ""` fallbacks already recorded.
* `desktop/src/features/agent/threadSearchRanges.ts`:61 — `unreachable-by-construction`. `if (startRun && endRun)`. Both operands are evaluated (`hits=[71, 71]`) and both are always defined, because the runs **tile** the searched string: each successive run starts exactly where the previous one ended (`start = text.length` before the append, `end` after it), so `runs` covers `[0, text.length)` with no gaps. Every match index lies inside `text`, so the two index walkers (`:54-58`) always stop on a run that covers the match start and end respectively. The guard is a defensive tail for an invariant the walker already guarantees.
* `desktop/src/features/agent/useStickyAutoScroll.ts`:130 — **was live, now covered.** The own-echo guard (see F12's neighbour below and §3). Its else arm (skip the re-derive) was uncovered because no test produced a programmatic correction *and then delivered its echo*. Covered by *"ignores the echo of its own anchor correction instead of resuming auto-follow"*, in `useStickyAutoScrollHook.test.ts`, whose mutant (guard forced true) fails with `expected 1000 to be 800` — the app resuming auto-follow and dragging a mid-history reader to the bottom. `hits` `[33, 0]` → `[0, 1]`.
* `desktop/src/features/agent/AgentThread.tsx`:354 (`if (!thread)`) is the arm of the guard whose `return;` at `:355` is waived above; `Composer.tsx`:208/475/504/635 are the arms of the four guards whose `return;`s are waived above; `ApprovalPrompt.tsx`:177's arm is now covered (see §3). Listed here so the census arithmetic closes against the rows above.

Branch-only dead paths — **representative, not exhaustive**; §2's branch audit has
the full measured picture (74 uncovered arms, 31 of them previously
undocumented). **One family is worth naming explicitly because it accounts for nine
of them: every `?? ""` fallback on a DOM property inside `MentionEditor.tsx`'s
`serialize` / `isEditorEmpty` / trigger-scan helpers** (`:804` `editor.textContent`,
`:827`/`:860` `node.textContent`, `:831`/`:864` `match[2]`, `:880` `node.textContent`,
`:888` `getAttribute("data-skill")`, `:892` `element.textContent`, `:893`
`getAttribute("data-path")`). Each reads a property the DOM guarantees to be a
string on the node types these helpers have already narrowed to.
I verified the whole family with **one** experiment rather than nine arguments:
substituting a `"MUTANT"` sentinel into all nine sites at once leaves **1001 tests
passing, zero failures** - no test in the suite ever executes one. **That experiment was
then re-run in its strongest form** (segment 32): each `?? ""` replaced by a **throwing
IIFE**, so a reachable arm fails loudly rather than carrying a different string, across
this family *and* the `Composer.tsx` one below - **all fifteen sites, 1023/1023 tests
passing**. See §4 for the probe method and its limits. **A second family
accounts for six more: every `editorRef.current?.getContent() ?? ""` in
`Composer.tsx`** (`:303` in the skills effect, `:365` in `submitValue`, `:429`/`:430` in `sendNow`,
`:489` in `installRecommendedSkill`, and `:438` in `clearComposer`). All six are
`?.`-then-`?? ""` on a **detached-editor** defence, and none is reachable:
`editorRef` is set in the same commit that renders the editor and cleared only on
unmount, while each of these runs from a mounted event handler or effect (and
`:438` is additionally pre-empted by `:436`'s `if (!editorRef.current || …) return;`).
I verified the whole family with one experiment rather than six arguments:
substituting a `"MUTANT"` sentinel into **all six** sites leaves **1001 tests
passing, zero failures** — no test in the suite ever executes one. The ones checked
arm-by-arm here are:
`buildContinuePrompt.ts`:73 (`outputsByTool[tool.id] ?? []` — the key set is exactly
the rendered set), `threadSearchCache.ts`:17 (the `: ""` arm of a non-text segment,
already returned for at line 12), `MessageBlock.tsx`:252
(`message.stopped ? "text-ink-muted" : "text-ink-soft"` inside a `<p>` that only
renders when `isLast === true && !message.stopped`, so the consequent cannot be
reached), `agentMessageFormatters.ts`:24 (**dead, but a bug rather than a benign
fallback — see F7**), and `useAgentThreadState.ts`:100's third arm —
`recentRun?.startedAt ?? recentRun?.createdAt ?? null`, where the **`?? null` is
unreachable by construction**: `StoredRun.startedAt` is optional but
`createdAt: number` is **required**, so the chain can never fall through to null. I
found this by trying to write the test for it and hitting the type system — which is
the honest outcome, and better than forcing it with a cast (the deleted draft used
`as StoredRun` on a row with `createdAt: undefined`, and `tsc` correctly refused).
The other two arms of that same chain *are* covered, including the `createdAt`
fallback, which reaches the streaming bubble as `runStartedAt`.

A second dead arm found the same way, this segment: `useAgentThreadState.ts`:243's
false arm (`consumedPromptRef.current === promptId` in the prompt-send catch). The
guard exists to avoid clobbering a *newer* prompt's record, but that state cannot
arise: only L235 writes a non-null ref, and it runs inside another prompt's effect,
whose own `handleSend` is **refused up front** while this send holds `sendingRef`
(`useSendMessage.ts`:61) - so that prompt's `.catch` immediately nulls the ref again.
For the failing send's catch to observe a foreign id, the other effect's L235 would
have to run after its own rejection was delivered but before its reaction microtask -
the same checkpoint, so the window cannot open. I tried to construct it and the
experiment agreed: the arrangement that got past the guard produced `hits=[1, 0]`,
the true arm only. The test draft was deleted; the *line* remains covered by
*"keeps a rejected first prompt available for retry on a later visit"*.

* `desktop/src/features/agent/useThreadMessages.ts`:425-427 — `unreachable-by-construction`. `if (result.status === "empty") throw new Error("History page returned no entries before its cursor.")`. The `AgentLoadResult` union declares an `empty` member, but `loadFromAgent` **has no producer for it** (`grep -c 'status: "empty"'` on the file returns 1 — the type declaration only): the two paths that find nothing return `{ status: "loaded", messages: [] }` instead. So the guard cannot fire, and the union member plus this guard are dead code — deleting both is the clean fix (a reviewer's call, since it also changes a type other code reads). Note the *reachable* half of the same behaviour **is** covered: a page that projects to no exchange adds no rows and reports no error (*"adds no rows for an older page that holds no exchange"*). Recorded as finding F5. **The same dead condition accounts for two more uncovered *arms* that the line ledger does not see**, and I attributed them this segment rather than leaving them as unexplained residue: the guard's own `if` branch at `:424`, and the false arm of `:358` (`setHistoryError(result.status === "failed" ? result.error : i18n.t("agent:thread.messagesLoadFailed"))`). Both require `status === "empty"`, which `loadFromAgent` never returns - `grep` over the whole file finds exactly one `status: "empty"` (the type declaration at :57) and three `"loaded"` producers plus one `"failed"` - so the ternary's else-branch is unreachable for the same reason the throw is.
* `desktop/src/features/agent/useThreadMessages.ts`:504 — `unreachable-by-construction`. `if (hasWarmSnapshotRef.current) return;` inside the indicator's show timer, defending against "the snapshot warmed up while the timer was pending". The only assignment that makes the snapshot warm is `hasWarmSnapshotRef.current = true` (line 375), which sits in the same commit as `setBlockingLoad(false)` (line 379) — so the render that warms the snapshot also clears `loadingThread`, and the indicator effect's cleanup then clears this timer before it can fire. The other disjunct of `loadingThread` (`ownerChanged = source.owner !== sourceRef.current.owner`) is a single-commit flag that the source-sync effect has already reconciled before any async read can commit. I could not construct a deterministic driver for it; if a reviewer finds one, the missing test — not the guard — is what to add. Named tests prove the surrounding code ran: *"shows the indicator after the delay and holds it for its minimum"*, *"holds a shown indicator for the rest of its minimum, then hides it"*, *"never shows the indicator over a warm snapshot"*. **Re-probed this segment because this was the thinnest row in the ledger** (its enclosing function runs only twice, so "the guard never fired" rested on a small number of runs). Two measurements upgraded it from an argument to a measurement: (1) the two-flag premise holds exactly as claimed — the only writer of the warm flag is `hasWarmSnapshotRef.current = true` at `:375`, sitting in the same synchronous block as `setBlockingLoad(false)` at `:379`, and `loadingThread = blockingLoad || ownerChanged` (`:123`); (2) a **throwing sentinel** installed in the guard's arm (`if (hasWarmSnapshotRef.current) throw new Error("PROBE-REACHED-503")`) was **not reached by any of the 1023 tests in the subtree** — not merely by that hook's own 67. The probe was reverted (the file compares clean against HEAD). This is empirical unreachability within the whole tested state space, which is the strongest evidence available short of a proof; the structural argument remains the reason it *cannot* be reached, and the sentinel is the reason to believe the argument.
* `desktop/src/features/agent/Composer.tsx`:209 — `unreachable-by-construction`. The flagged statement is the guard's `return;` at the head of `handleContextToolSelect` (whose `if (toolId !== "compact" || !onCompactContext || compactionPending)` sits on the line above, 208). The provider's own counts settle it: **`return;` hits=0 while the statement immediately after it (line 210) hits=4** — the callback *is* entered and always passes the guard, which is what "cannot fire" means. The only caller is the `/`-menu's context-tool row, and the tool list is built as `[]` unless `onCompactContext` is set and neither `sending` nor `compactionPending` is true (line 198), while the menu only ever contains the one `id: "compact"` entry. Every disjunct is therefore pre-empted by the caller's own gating.
* `desktop/src/features/agent/Composer.tsx`:476 — `unreachable-by-construction`. The guard's `return;` at the head of `installRecommendedSkill` (`if (!reco?.card || installingSkill)` on line 475). Counts: **`return;` hits=0, the next statement (line 477) hits=2** — entered, never rejected. The button that calls it is rendered only inside the recommendation card (so `card` is non-null) and is `disabled={installingSkill}`, so a click cannot arrive while either disjunct holds. The *reachable* half is covered: *"keeps the recommendation card up when the install fails"* drives the function through `onInstall` resolving `false`.
* `desktop/src/features/agent/Composer.tsx`:505 — `unreachable-by-construction`. The guard's `return;` in `dismissRecommendedSkill` (`if (!skillRecommendation?.card)` on line 504), whose only caller is the same card's "send without it" button. Counts: **`return;` hits=0, the next statement (line 506) hits=3** — entered three times, never rejected. Covered by *"sends the draft unchanged when the card is dismissed"* for the reachable path.
* `desktop/src/features/agent/Composer.tsx`:636 — `unreachable-by-construction`. The guard's `return;` at the head of `handleAttachFiles` (`if (disabled)` on line 635), whose trigger button carries `disabled={disabled}` (line 863), so the handler cannot be entered in the state the guard rejects. Counts: **`return;` hits=0, the next statement (line 641) hits=3** — entered, never rejected. Covered by *"disables the attach and send controls while the composer is locked"* for the observable half.
* `desktop/src/features/agent/useMessagePaging.ts`:148 — `unreachable-by-construction`. The `if (cooldownTimerRef.current !== null)` inside `finishCooldown`: its else needs `wheelBlockedUntilRef.current > performance.now()` **and** a null timer, but the only writer of `wheelBlockedUntilRef` (`:204`, inside `protectViewport`) sets `cooldownTimerRef.current` at `:210` in the same synchronous block, and nothing ever assigns the timer ref back to `null` (the other sites only `clearTimeout` it). Measured in a scoped run: **`hits=[10, 0]`** — ten entries into the guard, zero else. Recorded as part of finding F11. Every other arm of the file is now covered: `:261` and `:436` are executed but non-discriminating (F11), and `:267`/`:323`/`:339` are covered by tests whose mutants they kill.

## 6. Findings (reported, not fixed)

* **F1 — `samePath` does not normalise separator direction.** `useWorkspaceForm.ts`'s `samePath` strips a trailing separator and lowercases, but does not unify `/` with `\`. A workspace stored as `C:/work/app` and a folder picked as `C:\Work\App` are therefore treated as different directories, so the *"… already exists"* notice is missed — even though `@tauri-apps/plugin-dialog` returns the OS-native form. The shipped test pins only the case/trailing-separator rule, so fixing this will not have to delete an assertion that enshrines the gap. Low severity (both paths normally come from the same picker).
* **F2 — `.txt` files have no in-app preview.** `previewKindForPath` (in `src/features/filepreview/`, another worker's subtree) classifies by extension and its text list omits `txt`, so a plain `.txt` attachment is handed to the OS handler while `.ts`/`.md`/`.json` open in the overlay. `MessageBlock.behaviour.test.tsx` pins the current behaviour on both sides, so a fix only has to flip the `.txt` expectation. Not fixed here: it is outside this subtree's write range.
* **F3 — `useWorkspaceForm` exposes no `setPath`.** The returned object offers `setDisplayName` only, so a caller cannot prefill or correct a path; the native picker is the sole writer. Not a bug by itself, but it blocks a "recent directory" affordance.
* **F4 — the compaction wait has no caller-visible cancellation.** `AgentThread`'s report says the prompt is unlocked after the 30-minute timeout, but `handleCompactContext` swallows the `AbortError` and the compose button's `compactionInProgress` flag (from `agentState.isCompacting`) is the only signal. A user watching a stuck compaction sees a spinner, not an explanation.
* **F5 — `useThreadMessages`'s `"empty"` page status has no producer, so its guard is unreachable.** `loadFromAgent` returns `AgentLoadResult`, whose union declares an `empty` member, but **no path returns it** (`grep -c 'status: "empty"'` on the file returns 1 — the type declaration only): both "found nothing" paths return `{ status: "loaded", messages: [] }` instead. The guard `if (result.status === "empty") throw new Error("History page returned no entries before its cursor.")` (lines 425-427) can therefore never run, and the union member plus that guard are dead code. Deleting both is the clean fix — a reviewer's call, because the type is read by other code. The *reachable* half of the same behaviour is covered (*"adds no rows for an older page that holds no exchange"*). The waiver row is in §5.
* **F6 — `liveStreamTick`'s re-arm leaves an unhandled rejection, so its own guard does not achieve its stated purpose.** Both call sites of the private `drain()` use a bare `void` (`request()` and the `finally` re-arm at :92), and `drain()` does not catch a throwing `project`/`afterProject`. The result is two-fold: (a) when a projection throws while a push is queued, the very path the guard was written for ("a request landing after the loop's final check … would otherwise be stranded") emits an unobserved rejection instead of a clean recovery; (b) it also makes that line un-coverable here, because Vitest fails a test file on an unhandled rejection and the config that would tolerate it is outside this task's write range. A one-line fix — `void drain().catch(() => {})`, or wrapping the `finally` body in a `try` — would both keep the coalescing contract and make the branch testable. Not fixed here: `liveStreamTick.ts` is product code. My earlier waiver row for this line called it "unreachable-by-construction"; that was wrong, and the corrected row is in §5.
* **F7 — `buildAgentFailureTitle`'s neutral-title fallback is dead, so a missing `...Title` key is shown to the user verbatim.** The function reads `const title = i18n.t(titleKey, params); return title === titleKey ? i18n.t("agent:failure.runTitle") : title;`, assuming a miss returns the key unchanged. This i18n instance returns the key **without the `agent:` namespace**, so `title === titleKey` is never true and the guard cannot fire. `insufficientCredit`, `rateLimited`, `serverError`, `contextLimit`, `network` and `userStopped` are all classified by `classifyAgentError` but have no `...Title` translation in `agent.json`, so all six render the raw string: an HTTP 402 failure shows `failure.insufficientCreditTitle` as the error heading (measured, not inferred). One-line fix - compare against the namespace-stripped form, or test `i18n.exists(titleKey)` first. Not fixed here: `agentMessageFormatters.ts` is product code outside this task's write set. `agentMessageFormatters.test.ts` now pins the observed string on both sides, with a comment naming this finding, so the fix only has to flip one expectation.
* **F8 — `collectTurnTimestamps`' two `Number.isFinite` guards are behaviourally redundant.** The function skips an unparseable `createdAt` instead of pushing `NaN`, which reads like essential protection - and its comment implies as much. It is not: **both** consumers are `timestamps.<role>.some(at => at >= window.start && at <= window.end)`, and a `NaN` element makes that predicate `false` exactly as an *absent* element does (verified: `[NaN].some(...)` and `[].some(...)` are both `false`). Deleting both guards leaves every assertion in the two new tests passing - I measured that rather than assuming it, which is also why those tests are documented as pinning the *contract* and not the guard. Worth knowing rather than fixing: the guards are `latent` defensive code whose value depends on the consumers staying comparison-only. If one ever changes to e.g. `Math.min(...timestamps.assistant)` or a sort, a `NaN` would propagate through it and the guards would suddenly matter. Not fixed here: it is product code outside this task's write set, and removing them would be a behaviour-neutral refactor rather than a correction. The function reads `const title = i18n.t(titleKey, params); return title === titleKey ? i18n.t("agent:failure.runTitle") : title;`, assuming a miss returns the key unchanged. This i18n instance returns the key **without the `agent:` namespace**, so `title === titleKey` is never true and the guard cannot fire. `insufficientCredit`, `rateLimited`, `serverError`, `contextLimit`, `network` and `userStopped` are all classified by `classifyAgentError` but have no `...Title` translation in `agent.json`, so all six render the raw string: an HTTP 402 failure shows `failure.insufficientCreditTitle` as the error heading (measured, not inferred). One-line fix — compare against the namespace-stripped form, or test `i18n.exists(titleKey)` first. Not fixed here: `agentMessageFormatters.ts` is product code outside this task's write set. `agentMessageFormatters.test.ts` now pins the observed string on both sides, with a comment naming this finding, so the fix only has to flip one expectation. Both call sites of the private `drain()` use a bare `void` (`request()` and the `finally` re-arm at :92), and `drain()` does not catch a throwing `project`/`afterProject`. The result is two-fold: (a) when a projection throws while a push is queued, the very path the guard was written for ("a request landing after the loop's final check … would otherwise be stranded") emits an unobserved rejection instead of a clean recovery; (b) it also makes that line un-coverable here, because Vitest fails a test file on an unhandled rejection and the config that would tolerate it is outside this task's write range. A one-line fix — `void drain().catch(() => {})`, or wrapping the `finally` body in a `try` — would both keep the coalescing contract and make the branch testable. Not fixed here: `liveStreamTick.ts` is product code and the task's write set is tests + this document. My earlier waiver row for this line called it "unreachable-by-construction"; that was wrong, and the corrected row is in §5.
* **F9 — `Composer.tsx`:243's `|| skill.description` arm is redundant, like F8.** The
  expression is `useZh ? descriptionZh || skill.description : skill.description`, and
  `descriptionZh` is itself `skill.descriptionZh || catalogue.descriptionZh || null`.
  So whenever the RIGHT operand runs, the left is null, and
  `null || skill.description` yields precisely what omitting the OR would yield -
  replacing the whole expression with `skill.description` leaves the new test passing
  (measured). The arm cannot change the result; it is defensive code whose value would
  appear only if the left operand could be falsy-but-meaningful. Not fixed: product
  code outside this task's write set, and removing it would be a behaviour-neutral
  refactor.
* **F10 — `ThreadSearch.tsx`'s `move` guard is redundant with its callee.** `move` is
  `if (eligible && !searching && !searchFailed && rangesRef.current.length > 0) showMatch(...)`,
  and `showMatch` begins `if (ranges.length === 0) { currentIndexRef.current = -1; setCurrentIndex(-1); return; }`.
  So the `length > 0` operand cannot change the outcome: weakening it leaves the new
  zero-match Enter test passing (measured). The operand is defensive only, and the
  third instance of this pattern after F8 and F9 - worth noting as a *class*: this
  codebase's guards are frequently doubled by their callees, which is why several
  uncovered arms turn out to cost nothing to remove. Not changed: product code outside
  this task's write set.
* **F11 — `useMessagePaging.ts`:261 and `:436` are reached but non-discriminating; `:148` is dead.** All three came out of one classification pass over this file's six arms, and each needed a measurement rather than an argument:
  * **`:261`** `setWindowStartId(visibleMessages[0]?.id ?? null)` — the `?? null` arm is now **executed** (an empty thread reaches it, 3 hits) but substituting `"MUTANT"` for `null` changes nothing. The reason is structural: any value the fallback yields is *absent from `messages`*, so `messages.findIndex(id === …)` returns `-1` either way and `effectivePageStart` stays the default page. The null is a **type-level requirement** (`useState<string | null>`), not a behaviour switch.
  * **`:436`** `if (event.cancelable) event.preventDefault()` — executed (1 hit) and non-discriminating: the DOM spec defines `preventDefault()` on a non-cancelable event as a silently ignored no-op, so removing the guard leaves the new non-cancelable-wheel test passing (measured). Kept as the idiomatic guard.
  * **`:148`** `if (cooldownTimerRef.current !== null)` inside `finishCooldown` — **dead**, `unreachable-by-construction`. The guard's else needs `wheelBlockedUntilRef.current > now` **and** a null timer, but the only writer of `wheelBlockedUntilRef` (`:204`, inside `protectViewport`) sets `cooldownTimerRef.current` at `:210` with no `await` between them; the ref is never reset to null (only `clearTimeout`ped). Measured: `hits=[10, 0]` with the guard's consequent at 10 - i.e. ten entry attempts, zero else. This is the F8/F9/F10 class again but reached through the *write* side: two refs written in the same synchronous block cannot disagree.
* **F12 — `useRunReattach.ts`:106's `if (!cancelled)` is *triply* redundant, and I found it by mutating a test I had just written.** The arm is reached (a view can unmount while `listen` is still registering, which is the concurrency state the test drives) but **removing the guard changes nothing**: `liveStreamTick`'s `request()` returns early once `stop()` has run (`liveStreamTick.ts`:97-98), and the cleanup that sets `cancelled` also calls `liveTick.stop()` — while the drain re-checks the very same flag through `options.isActive()` (`:72`, `:91`), since `isActive` *is* `() => !cancelled`. Measured: with the guard forced to `true`, all 14 tests in the file still pass. The test was kept (it is the only driver of the unmounted-during-registration state, and it closes the arm) and its comment now states that it does **not** discriminate the guard. This is the sixth arm of the F8–F12 class — a guard doubled by its callee — and the first where the doubling comes from **three** independent mechanisms at once.

## 7. Open items

**None — and both prior caveats are now closed.** The incident recorded in §8 is fully
recovered: the module is back to **99.5329% lines (2983/2997), 14 uncovered in 9 files** —
*exactly* the pre-loss line state — with **97.83% branches (55 arms)**, one arm better than
before the loss, and **0 undocumented arms**.

The two open questions this section previously carried are both resolved by measurement:

1. **`threadSearchCache.ts`:40/`:41` is NOT dead — my unreachability argument was wrong, and
the obvious test proved it.** Both lines are covered now; §8 keeps the corrected reasoning
and the generalisable lesson, because the mistake (confusing a value's presence in the input
with the condition that reaches the line) is the kind that produces a plausible-looking
`unreachable-by-construction` row for live code.
2. **The `liveStreamTick.ts`:92 row** was re-decided in segment 5 and stands as
`unreachable-in-this-environment`, resting on a measurement (the guard's only route emits an
unobserved rejection, which fails the file) rather than on an argument.

None. The two files that segment 3 left as `OPEN (reachable, not covered, deliberately not waived)` — `Composer.tsx` and `useThreadMessages.ts` — are no
longer open: segment 4 tested them from 67 and 66 uncovered lines down to 4 and 2,
and those last six lines are now waived in §5 with a per-line reason (each is a
guard whose caller has already excluded the state). Segment 5 additionally
re-decided the `liveStreamTick.ts`:92 row rather than leaving my weakest waiver
standing.

Every file in the subtree is therefore either at 100% of coverable lines or
waived above with a category, and no reachable line is left uncovered without a
waiver.

**No open test gap remains.** The one item left reported-unfixed in the previous
segment — the surviving mutant #20 on `useThreadMessages.ts`'s run-window filter —
is now closed: a page fixture holding a real exchange was added, the mutant fails
exactly one test under re-mutation, and clean code passes. See §2's *Mutation
validation*, batch four, for the three tests and the fixture-shape lesson.

## 8. Incident: a `git checkout` on a directory reverted my own test files (segment 32)

**What I did.** While probing a batch of guard arms with a throwing sentinel, I reverted the
probes with `git checkout -- src/features/agent` — **the whole directory** instead of the
specific product files. `git checkout` reverts *tracked* files, so it also reverted every
**tracked test file** this module's work had extended. My 20 *new* spec files were untracked
and survived; the additions I had made to tracked specs did not.

**Why it was unrecoverable.** Nothing had been committed (the task forbids `git add`/`commit`
and the supervisor commits only at checkpoints, of which there were none for this branch):
`git log --all -S "<a distinctive test name>"` finds no commit, `git stash list` is empty, and
there is no second worktree holding the work. The test bodies existed only in the working tree.

**Measured damage and recovery state.** Loss peaked at **98.8989% lines (33 uncovered / 16
files), 95.45% branches (115 uncovered arms), 1023 → 965 tests**. Recovery restored
`useMessagePagingHook.test.ts` (+16 tests), `threadSearchCache.test.ts` (+2),
`useStickyAutoScrollHook.test.ts` (+2), and then — written from the module source, since the
lost bodies were gone — four more specs: `useSkillRecommendation.test.ts` (+4),
`useAgentThreadState.lifecycle.test.tsx` (+2), `ThreadHeader.test.tsx` (+1),
`useComposerInset.test.tsx` (+1). A second recovery batch then rebuilt the **arm** losses from
the module sources: `sendPipeline.test.ts` (+5 tests — its whole stale-send guard family),
`attachments.test.ts` (+3 tests, 3 arms), `clipboardAttachments.test.ts` (+1, 1 arm),
`composerDraft.test.ts` (+1, 2 arms), `ReplySteps.test.tsx` (+1, 2 arms) and
`threadRunProjection.test.ts` (+4 tests — the legacy-run-row cluster: `:610`, `:619`, `:636`,
`:655`, `:661`, `:799`, `:800`, `:804`). Final state: **99.4995% lines (2982/2997), 15 uncovered in
10 files, 96.56% branches (87 arms), 992 tests**, and the line gate passes. After the second
batch: **97.39% branches (66 arms)** and 92 tests added net.

**Nothing is still missing.** Every line and every arm the incident destroyed has been rebuilt,
and the module is back to — and slightly past — its pre-loss figures:

| | worst point | **now** | pre-loss |
|---|---|---|---|
| lines | 98.8989% (33 uncovered / 16 files) | **99.5329% (2983/2997), 14 / 9** | 99.5329%, 14 / 9 — *exactly equal* |
| branches | 95.45% (115 arms) | **97.83% (55 arms)** | 97.79% (56 arms) — *one arm better* |
| tests | 965 | **1017** | 1023 |

**Restored by new tests** (all written from the module sources, since the lost bodies were gone):
`useSkillRecommendation.test.ts` (+4), `useAgentThreadState.lifecycle.test.tsx` (+6),
`ThreadHeader.test.tsx` (+1), `useComposerInset.test.tsx` (+1), `useMessagePagingHook.test.ts`
(+19), `threadSearchCache.test.ts` (+4), `useStickyAutoScrollHook.test.ts` (+2),
`sendPipeline.test.ts` (+5), `attachments.test.ts` (+3), `clipboardAttachments.test.ts` (+1),
`composerDraft.test.ts` (+1), `ReplySteps.test.tsx` (+1) and `threadRunProjection.test.ts` (+6).

**The `threadSearchCache.ts`:40/`:41` row: my unreachability argument was WRONG, and the test
proves it.** I had argued that the pair (`if (typeof node.code === "string")` / `return
node.code`) could not be reached because a code node's `code` is a substring of `text`, so the
raw-text check at `:20` would have matched first. **That conflates two different things:**
`textLeaves` is reached *precisely when the needle is absent from the raw text*, and at that
moment **every** code node is still visited — whether or not its text is in `text`. So the
branch runs for any assistant reply containing a fenced block together with a needle that
misses the raw markdown, which is an ordinary search miss:

```ts
mayContainThreadSearch(message("```ts\nconst answer = 42;\n```"), "zzz-absent")  // → false, having run :40/:41
```

Both lines are now **covered** by *"flattens a code block's code when the needle misses the
raw text"*, and the zero-uncovered state of that file is measured, not argued. The lesson is
worth keeping next to the probe technique in §4: **an unreachability argument that reasons
about a value's *presence in the input* must not be confused with the *condition that gets you
to the line*.** The cheap check that would have caught it is the one that did: write the
obvious test and see whether the line moves.

**Arm-level recovery: 55 arms — one *better* than the pre-loss 56, and branch coverage 97.83%
against the pre-loss 97.79%.** Three recovery batches followed the four statement-level
rebuilds: `sendPipeline.test.ts` (+5 tests — its whole stale-send guard family),
`attachments.test.ts` (+3, 3 arms), `clipboardAttachments.test.ts` (+1, 1 arm),
`composerDraft.test.ts` (+1, 2 arms), `ReplySteps.test.tsx` (+1, 2 arms),
`threadRunProjection.test.ts` (+6 — the legacy-run-row cluster and the two
status/`Number.isFinite` arms), `useAgentThreadState.lifecycle.test.tsx` (+3 — the anchor
chain's two fallbacks and the hidden-tab guard), `ThreadHeader.test.tsx` (+1, 3 arms),
`useMessagePagingHook.test.ts` (+3 — the search loop's two short-circuit operands and the
fall-through past its batch wait) and `threadSearchCache.test.ts` (+4 — including the pair I
had wrongly called dead).

**Every one of the 55 uncovered arms is documented** — verified by intersecting the arm
census against the ledger programmatically rather than by eye: **0 undocumented**. The
residue is `MentionEditor.tsx` 13 (the nine `?? ""` arms proven unreachable by throwing
sentinel plus four waived guards), `Composer.tsx` 10 (six sentinel-proven `?? ""` plus the
four documented guard `return;`s), `useThreadMessages.ts` 6, `MessageBlock.tsx` 3,
`threadRunProjection.ts` 3, and the rest across `AgentThread.tsx` (2), `ApprovalPrompt.tsx` (2),
`externalLinks.ts` (2), `mentionMarkdown.ts` (2), `threadSearchRanges.ts` (2),
`threadSearchCache.ts` (1 — the `""` arm of the text-segment map, the F8-class guard doubled by
its own earlier filter), `useMessagePaging.ts` (1 — the documented dead timer guard),
`AgentActivityList.tsx` (1), `NewConversation.tsx` (1), `agentMessageFormatters.ts` (1),
`buildContinuePrompt.ts` (1), `liveStreamTick.ts` (1), `sendPipeline.ts` (1 — the documented
dead `?? null`), `threadMessageCache.ts` (1) and `useAgentThreadState.ts` (1 — the documented
dead `consumedPromptRef` re-check).

**The lesson, stated so it transfers.** A *targeted* `git checkout -- <file> <file>` is safe;
`git checkout -- <directory>` is a **mass revert of every uncommitted change in that tree**,
including work unrelated to the probes. During a long uncommitted session that is data loss
with no undo. Revert the files you touched, by name, or `cp` them aside first. I had already
used the targeted form correctly earlier in the same session and then became careless — the
mistake was one command of convenience in an otherwise disciplined workflow.
