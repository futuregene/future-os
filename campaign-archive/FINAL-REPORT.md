# Coverage campaign — FINAL REPORT

**Merged:** PR #1167 (`fd2164fd6`), squash-merged into `main` 2026-10-17 12:51:41Z, all 15 CI
checks green. Branch, remote branch and PR worktree cleaned up.

**Revised 2026-10-17 after the merge:** `tauri-shell`'s figure was an interval (107–127) when this
report was first written. TAU-SHELL-LAND then measured it (**115**, of which 25 `host-state-blocked`)
and SHELL-TAIL closed the last three reachable rows (**109**, projection 87),
so §2 and §5 carry the measured number, not the interval. **Then TAU-D8 measured `tauri-headless` at
HEAD (300 -> 283) and corrected the `cli_tool` statement set**, so its row and §7 carry that too.

**This report states what each module and each test claim rests on.** It deliberately does **not**
state a repository-wide percentage, and §6 states the parts of the user's bar that are not met by
this report alone.

---

## 1. The rule the campaign enforced (口径 A)

A scope is done when its per-line `lcov DA:0` count is zero, **or** every remaining uncovered line is
registered in `docs/testing/waiver-ledger.md` with a category **and a mechanism verified in the
source**. For the JS modules the same rule applies to per-line Istanbul records.

Two metric guards, both of which caught real defects:

* **`summary.lines` is forbidden.** `module-agent-llm`'s headline figure was `summary.lines` = 396
  while its `DA:0` view was **115** — 252 of those lines exist in LLVM's line table but belong to no
  region, so no test can ever close them. Corrected (INCIDENTS F.87).
* **A ledger row's readability must not be bought with its parseability.** A trailing `(N lines)`
  inside the claim cell makes the row parse as **zero** claims, so its lines silently vanish from the
  check; an en-dash range does the same. `module-tauri-remote` was therefore PASS while certifying
  **331 of its 450** lines (INCIDENTS F.94).

## 2. Figures of record

Each row is that module's own self-report against its own pinned artifact. **These are not summable**
— see §3.

| module | uncovered | state |
|---|---|---|
| goal / tasks / packages | **77** | ledger PASS |
| agent-core (AG-B) | **213** | ledger PASS |
| agent-rpc (AG-A) | **202** | ledger PASS |
| agent-llm (slice of agent-core) | **115** | `DA:0`; record's 396 was `summary.lines` |
| channels | **8** | ledger PASS |
| cli | **52** | ledger PASS |
| cli-mcp | **4** | disjoint from `cli` by its own declaration |
| desktop frontend | **59** | 55 waived + 4 open (needs a product decision) |
| mobile | **13** | 13/13 survived adversarial review |
| tui | **13** | ledger PASS |
| review-rust | **24** | ledger PASS |
| tauri-appdata | **374 = 348 WAIVED + 24 OPEN + 2 exclusions** | REV-TAURI-APP falsified the old 368+10: 11 lines were measurably coverable (four probes, `DA` 0->1), 21 more carried categories their mechanisms do not support, and the last UNVERIFIABLE row closed via a test-only fault hook; seven tests now close those 12. Three of the 24 OPEN are `cost-deferred` with a measurement (`capture()` completes in 332.25 s over 8115 files — expensive, not impossible) |
| tauri-remote | **1205** = **353 product + 852 `#[cfg(test)]` harness** | the harness lines are `#[ignore]`d for want of a real broker; 450 product lines before REAL-NET added its own harness |
| tauri-headless | **283** | 300 -> 283 at HEAD `36c568972`: `agent_supervisor` 81 -> 67 (14 lines covered by the macOS-only mock-loop quit test) and `agent_host` 26 -> 23 (oscillating attach arms). Split 88 OPEN / 195 waived. `cli_tool.rs`'s 11 `DA:0` lines re-derived as 83, 92, 285-288, 293, 305, 356, 672, 759 — the statement set changed, not just moved |
| tauri-shell | **109 measured** | `SHELL-TAIL` closed the last reachable rows (`lib.rs` 212/215/244 via a mock window labelled `main`, plus the three `if let` braces) after TAU-SHELL-LAND measured 115. Independently re-checked: 109 uncovered / 109 mentioned / **MISSING 0**; `lib.rs` 69 (was 75). **22 `host-state-blocked`** remain (need `RunEvent::Ready`, absent on this host), so the projection for a Ready session is **87**. The "107 achievable" was my arithmetic and the review's 127 was its expectation — both refuted by the instrument. |

## 3. Three kinds of claim, kept separate

The user asked to be able to tell these apart at a glance, so every module row falls into one of:

**(a) Machine-checkable.** A per-line ledger plus a pairing that re-derives it:
`goal, agent-core, agent-rpc, channels, cli, cli-mcp, desktop, mobile, tui, review-rust,
tauri-appdata, tauri-remote, tauri-headless`.

Verification is one command per row:

```
python3 .future/cov100/reconcile.py --report <artifact> --scope <scope> --doc <record> --section <needle>
```

which prints `mentioned / MISSING / STALE`. The aggregate gate:

```
python3 .future/cov100/staletable.py --gate
-> authoritative pairings: 15 -> clean 14, justified covered rows 1, unverifiable 0, failing 0
```
(The two lines that still read FAIL are `role=wide`, whole-document pairings that over-count by
design; they are informational, not verdicts.)

**(b) Prose-only.** Eight round-1 records (`module-agent`, `module-agent-sandbox`,
`module-agent-session`, `module-tauri`, `module-tauri-core`, `module-tauri-bridge`,
`module-tauri-terminal`, `module-cli-tui`) carry no per-line claim rows. **`reconcile.py` cannot
check them** — it will report nothing for them, which is not the same as reporting them clean. They
are listed as pointers in the ledger.

**(c) Environment-attributed.** `module-tauri-shell`'s 75 `host-state-blocked` lines: verified
uncovered in the degraded artifact **and covered in the pinned healthy one**
(`/tmp/tauri-shell-lcov-c8union.info` = 150). The mechanism was measured, not assumed: this host's
agent chain cannot reach the current GUI session (`launchctl print gui/$(id -u)` -> 125,
`screencapture` fails), and restarting the agent does **not** help — an agent started 7h40m *after*
the session restart is equally stuck, which falsified my earlier "start time" explanation and is
recorded as INCIDENTS F.90.

## 4. Independent review — the most important result here

Nine adversarial reviews tried to falsify the waiver tags by constructing the inputs the tags said
could not exist:

| review | rows attacked | fell |
|---|---|---|
| REV-FE (mobile) | 13 | **0** |
| REV-DESK (desktop) | 61 | 3 |
| REV-TUI | 24 | 11 |
| REV-CH (channels) | 24 | 16 |
| REV-CLI | 33 | **10 rows / 76 lines** |
| REV-S6 | the segment's own fixtures | **0** |
| REV-AGENT | 469 lines | 5 rows / 33 lines, **including one of my rulings** |
| REV-GOAL | 82 | 6 rows / 8 lines |
| REV-GOAL-2 | 15-row table | 4 defects + 1 declared falsifiability gap |
| **REV-TAURI-SHELL** | 217 | **90 (41 %)** |

**Conclusion carried into the record:** a waiver that has not been attacked is a hypothesis.
**"The supervisor accepted it" is not evidence of correctness** — two reviews overturned my own
rulings, and the largest falsified 41 % of one module's residual. Fixtures a unit built and controlled
itself survived in every case.

## 5. Where the campaign was wrong, and how it was found

Recorded because the corrections are part of the deliverable, not noise:

1. **A published figure was wrong by 43 lines, and then my correction was wrong too.** `tauri-shell`
   was published as 150; REV-TAURI-SHELL showed 43 of those lines are uncovered in the *healthy*
   artifact too, i.e. they were never environmental. I then estimated **107** by arithmetic (150 − 43)
   and wrote that into the ledger as "achievable" — an estimate presented as a residual. The actual
   measurement was **115** and then **109** after the last reachable rows were landed (22 of the 109 are
   `host-state-blocked`, projection 87). So the review was right, my
   arithmetic was wrong, and the *review's own* expectation of 127 was wrong as well. Three different
   numbers, one artifact: this is the cleanest case in the campaign for **a measured number replacing an
   estimate**, and the reason the ledger now says "measured" rather than "achievable".
2. **A published product defect did not exist.** `real-network.md` recorded "journal coalescing grows
   the lane at the production 100 ms window" and I carried it into the PR body and this report's
   predecessor. Measured properly the lane is **byte-identical**: `Coalescer::flush` returns a lone
   fragment unchanged (`merged == 0`), and that journal's chunks are 100.9–110 ms apart against a
   100 ms window, so nothing *may* merge. **The defect was the measurement's strict `coalesced <
   today` assertion.** The first reading was a guess made necessary by the driver printing its
   numbers *after* its assertions; a burst-paced journal now demonstrates 2.30× merging when
   fragments really arrive inside the window. Corrected in `real-network.md` §3.3a,
   `module-tauri-remote.md` §9 and the PR body.
3. **A waiver mechanism that was simply false.** 42 `tauri-remote` rows were waived as "only the
   `cfg(test)` copy of the lib runs under the harness". Integration tests spawn the instrumented
   `futureos`/`futureos-headless` children, which link the lib **without** `cfg(test)` and do call
   `remote::start()`. Those rows execute; corrected in place, and 5 genuinely reachable lines were
   moved to an OPEN table rather than waived.
4. **Three of my own instruments were wrong in the same direction** (false DRIFT / false "all
   registered"): a tool that appended `/` to file scopes, one that read a file count as a line count,
   one whose reader used substring matching on event JSON. Each was caught by hand-checking one row
   against the authority, not by the tool's own output (F.79, F.82, F.86).
5. **One `#[test]` I "added" never ran** — it landed inside the body of another test, which Rust
   accepts as a local item. Caught by asking cargo how many tests the name matched: **0**.

## 6. What the user's two completion conditions do and do not have

**(i) "Test all functionality thoroughly in a real network environment" — delivered as evidence, with
its boundary stated.** `docs/testing/real-network.md` (404 lines) + `real_network.rs` (two-process
harness). Real `nats-server`, real Noise v2 handshake and per-record AEAD, real NKey signing, two OS
processes, a real `future-agent` behind the host:

* 6 of the lane's `#[ignore]`d drivers executed **without removing a single `#[ignore]`**;
* pairing, a 1.2 MB / 3-chunk upload, a 25 B attached upload, a 960 KB / 2-chunk download **byte-identical**,
  and a prompt through the real Agent (26 text chunks, answer echoed);
* failure paths: socket close then command OK; broker killed mid-transfer (12 s outage) -> request
  timed out, **160 MB retry exact** after the host returned; peer restarted with the same identity ->
  Noise_IK resumed.

**Explicitly NOT exercised** (real-network.md §7, quoted rather than summarised): TLS and `wss://`
transport (both sides call `require_tls(true)` only under `cfg(not(test))`); FutureOS platform
pair-code/claim/refresh/revoke; the `futureos-headless` and GUI desktop processes; a real LLM vendor;
the real mobile app; `download_prepare`'s policy layer; packet loss/latency/NAT rebinding; a NATS
cluster/JetStream/auth callout; revocation while connected; four `remote_host` measurement drivers
and other crates' ignored tests.

**So: the boundary is "the remote lane, plaintext, loopback, one broker, two processes".** Whether
that is "充分" is the user's call, and this report does not decide it.

**(ii) "Coverage to 100 %" — met under 口径 A, not as a bare number.** 14 of 15 authoritative
pairings are clean with 0 failing; every remaining line is either in a per-line ledger with a
category and a source-verified mechanism, or is explicitly listed as prose-only (§3b) or
environment-attributed (§3c). Two scopes are **not** at 0 and are not claimed to be:
`tauri-shell` is a measured **109** (87 of them registered with mechanisms; 22 `host-state-blocked`
pending a `Ready` host), and `agent-core`/`agent-rpc`/`desktop`/`tauri-*` carry open rows registered
with mechanisms. No module figure in this report is an interval.

## 7. Still open (nothing here is hidden by the merge)

* **`tauri-shell`: SETTLED to a measured 109** (`todo_f65783eb7365` and its successor `todo_5bedaa237a61`,
  both delivered). 88 of the review's 90
  fallen rows are now covered by landed fixtures — `macos_power` 10/10 by posting the NSWorkspace
  notification names to the observers' own centre (no seam), `menu.rs` by the `harness = false`
  main-thread target, `launch.rs`/`main.rs` by the pty launcher plus `CARGO_BIN_EXE_futureos`. The two
  that resisted are named (`main.rs:51` a closing brace; `lib.rs:1040` `APP_HANDLE.set`, the setup hook).
  Remaining work is a declared successor (`todo_5bedaa237a61`: `lib.rs` 212/215/244 with a mock window,
  projected 115 -> 112, and it measured **109** because the three `if let` braces fell with the condition
  lines) plus the 22 remaining `host-state-blocked` rows, which need a host that delivers `Ready`.
* **`cli_tool.rs`: SETTLED** (`todo_2090adf0ae54`, delivered). The file is 767 lines (not 758) and its
  11 `DA:0` lines are **83, 92, 285-288, 293, 305, 356, 672, 759** — re-derived, not carried over. Two of
  the numbers the record had were simply wrong (357 and 666 were 356 and 672) and one statement is NEW
  (759, from the NEW-B seam) while another is now covered (338, EnvGuard's restore arm). So the module's
  §2.3 note "the statement set is unchanged" was false: the set changed, which is why the re-measure
  mattered.
* **Flake work** (F-TAURI-APP family): a new instance was recorded during PR #1167's CI —
  `shadow_review::maintenance::tests::recover_one_finished_reuses_after_snapshot` failed once in the
  full Linux suite (`files_changed` 0 vs 1) and passes 5/5 in isolation here. Mechanism
  undiagnosed; recorded on the ticket with two candidate mechanisms and a suggested first check.
* **Five test cases and one harness target are macOS-only**, each with its measured reason in the
  source and in the PR body's table. That is a real reduction in what Linux and Windows verify.
* **Tool views differ on one pairing, and the difference is the slice boundary.**
  `staletable.py --gate` reads `tauri-remote` PASS with MISSING 0 over 1484 claims (289 absent rows
  registered); `reconcile.py --section "4. Uncovered-line ledger"` reads MISSING **2**
  (`verify_e2e.rs:1923`, `:2121`, both test-side lines written after §4's crosswalk, and both
  mentioned in §8). Same record, narrower slice. This is the heading-level trap recorded in PLAN §5.2:
  the gate's needle spans the sections the record calls its ledger, a narrower needle stops sooner.
  Neither tool is wrong; **quote which one you ran**, which is why §8 names both.
* **The campaign worktree (`.worktrees/cov100-r2`) is deliberately kept.** It holds the goal control
  plane's ledger (`.future/goal/`, still open tickets) and these campaign documents, none of which were
  in the PR payload because `.future/` is gitignored upstream. Removing it would delete the only copy of
  the goal state. `main`'s working tree is clean apart from that ignored directory.
* **Two product decisions for the user:** the `ImagePreview` space-drag gesture (implement or delete
  the dead state), and whether to keep the 4096-byte rule-pattern bound as-is.

## 8. How to re-derive any number here

Every figure names an artifact. `reconcile.py` (per row), `staletable.py --gate` (all rows),
`claim-cell-sweep.py` (rows that look like claims but parse as none), `ledger-census.py` (modules that
are complete but unregistered) and `leg-flavour-check.py` (a gui/headless leg of the wrong flavour)
each have a self-test, and each was written **because** it caught a defect the manual pass missed.
If this document and the tools disagree, **the tools are right** — this is a summary, and the campaign
has caught four summary defects of mine along the way.
