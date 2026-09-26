# rev-rust — independent adversarial review of the Rust side

Reviewer: `rev-rust` (independent verification; no production code, no test file, and no
assertion was touched by this review — I only read, measured, and attacked).
Date: 2026-09-26. Session `20260926-200505-b24e9e45516f4ccf941d3c6a31f959c4`.

Command of record for every number I quote of my own making:

```powershell
cd D:\future-os\.worktrees\cov100
python .future/cov100/verify.py uncovered desktop-tauri coverage/tauri-tau-remote-report.json
git diff HEAD --name-status --diff-filter=DR
git rev-parse HEAD origin/main
```

---

## 0. Verdict

**Conditional pass, with four items that must go back to the owners before the PR.**

Nothing I found shows a *fabricated* test, a *weakened* assertion, an `#[ignore]`d test, or a
production guard deleted to move a number. The deletions I audited are relocations and
testability extractions; the arithmetic reconciles exactly. That is the good news, and it is
real.

What I did find:

| # | finding | severity | who |
|---|---|---|---|
| **F-A** | **The whole measurement was taken on a tree 29 commits behind `origin/main`.** 21 files inside the reviewed scopes — including `remote/commands.rs` (4 of the 7 dead guards), `remote_host/business/history.rs:221`, `remote_host/files.rs`, `orchestration/loop/src/agents/supervision.rs` — are **different upstream** (#846…#873). Every line-anchored waiver in those files will be re-anchored (or invalidated) by the mandatory pre-PR merge, and the numbers in §"before/after" are not the numbers of the tree that will be measured. | **blocks the final gate run** | supervisor |
| **F-B** | `agent/src/rpc/mod.rs` 413–420 is registered `unreachable-in-this-environment` with the reason "this host always has a builtin default". **That is false and I can build the counterexample from the file's own fixtures.** Both the 3826-entry builtin catalog (0 entries carry an `api_key`) and the doc's own test comment ("CI has none and the assertion failed") show the arm executes on a credential-less host. | **waiver falsified → rework** | w-ag-rpc |
| **F-C** | `docs/testing/module-tauri-bridge.md` registers **10 files (~140 lines)** as `unreachable-in-this-environment` and then says of those same rows in §"the gate passes structurally, not on merit": they "are **reachable in principle** with more scripted-mock fixtures and store setups" and "need **no new harness**". That is the category-vs-admission contradiction, at the largest scale in the repo. | **10 false registrations → rework or re-category** | w-tau-bridge |
| **F-D** | `docs/testing/module-tauri-terminal.md`'s DSR row (`terminal/server.rs` 1953-1957) is registered `unreachable-by-construction` and explained by "**the first guard fires** … so the second cannot execute". The measurement proves the **opposite**: the second guard's body (1983-1987) has counts, the first's (1953-1957) does not. The two symmetric guards are scheduling-dependent, not construction-dependent. The proposed deletion is *safe* (it is a duplicated test guard, not production code), but its stated justification is wrong and the category is wrong. | **wrong reason + wrong category → rework the row** | w-tau-term |

Modules whose gate-green status I would call **structural only, not on merit**: `tau-bridge`
(F-C, admitted by its own doc, confirmed by me), `loop` (two rows below), and `agent-rpc`
(F-B). Their `verify.py` exit codes are green because a category string is present; that is
exactly the property the plan says the gate deliberately does not judge.

---

## 1. Method, and what I actually executed

| did | how |
|---|---|
| Independent reconciliation of the tau-remote measurement | read `coverage/tauri-tau-remote-report.json` **itself** and re-summed the module scope myself (not a re-run — see limits) |
| Per-file uncovered counts against the doc's table | `verify.py uncovered desktop-tauri …`, all 26 gap files |
| Staleness check on the report | file mtimes vs report mtime |
| Dead-code proof attacks | source reading of every guard, its producers, its removers and its callers |
| Repo-wide deletion audit | `git diff HEAD` (see §4 for why *not* `origin/main`) |
| Category audit | regex sweep of every `module-*.md` row for a category keyword co-occurring with an admission of reachability |
| **Windows red lines (executed)** | `cargo test -p future-channel --lib`, `cargo test -p future-tui --lib` in my own target dir — see §10.1 |
| **Assertion-free census (executed)** | my own scanner over 553 scoped `.rs` files / 8674 test attributes — see §10.2 |

**Limits I am not going to hide.**

- I did **not** re-run `cargo llvm-cov` for any crate in either pass. (I did run the two
  `--lib` suites in §10.1; what was never re-measured is *coverage*.) Everything numeric below is
  either (a) re-derived by me from an existing report file, or (b) quoted as the worker's
  claim. A re-derived number is `independent-recheck` of the *artifact*; it is **not** a
  re-measurement, and it cannot detect a stale or partial report. §7 says which claims that
  leaves unverified.
- I could not *execute* an attack on the seven guards: reaching them needs a test inside
  `futureos`'s private `remote` module, and my declared write set is this document only. So
  every construction below is a **source-level** construction, marked `REACHED(by
  construction)`, `REACHED(by measurement elsewhere)`, or `SURVIVED`. I have not labelled a
  source-level argument as an execution.
- `agent/src/{llm,agent,compaction}/**` and `mobile/src/screens/SessionList.tsx` were off
  limits and were not reviewed. The `module-agent-llm.md` rows I quote appear only as the
  reference instance the supervisor already names.

---

## 2. The seven "dead guard" deletions — verdicts

Ruling under review: *KEEP*. My job was to try to break the proofs. **Five survived; three
proofs are unsound but not deterministically exploitable.**

The three unsound ones fail the same way. The argument is "`access_current()` and `commit()`
compare the same `AccessEpoch` and there is **no `await`** between them, therefore the first
being true implies the second returns `Some`". That is an argument about **cooperative**
interleaving on one thread. It does not hold in the object model here:

* `SUPERVISOR` is a process-global `LazyLock<Supervisor>` and `AccessEpoch::commit` takes
  `epoch.lock()` — a *separate* critical section from the one inside `access_current()`.
* The runtime is tokio's multi-threaded one (`crate::runtime::spawn`, plus `tokio::spawn` for
  every command); `AccessEpoch::invalidate` is called from the production stop/unpair path
  (`remote/lifecycle.rs:75`), which can run on another worker thread at any instant.
* So "no `await`" buys atomicity against *this task being preempted*, not against *the epoch
  moving under it*. The window between the two lock acquisitions is a few instructions wide —
  real, but not schedulable from a test.

Verdicts:

| target | route I tried | result |
|---|---|---|
| `remote/commands.rs:432` — `if slots.get(&id).is_some_and(\|c\| Arc::ptr_eq(c, &expected))` | I looked for **any** second remover or replacer of a `ReplySlots` entry. The map is `Arc<Mutex<HashMap<String, ReplySlot>>>`; the only destination mutations in the crate are `slots.insert` (commands.rs:383, taken only in the `None` arm of `match slots.get(&id)`) and `slots.remove` (commands.rs:429, the same expiry task, behind this very guard). `BridgeRuntimeShared` shares the *same* map across generations (`shared_runtime` reuses `reply_slots`), so a generation swap cannot empty it either. For one id the mapped `Arc` is therefore never replaced: `insert` can only happen while the key is absent, and the key only becomes absent *by this task's own removal*, which is the last thing it does. | **SURVIVED.** I could not construct an input. This is a real invariant, not a race. It is defensive against a future maintainer adding a `retain`/`clear`, which is a reason to keep it and not a reason to test it. |
| `remote_host/business/history.rs:221` — else-edge of `if let Some(events) = data["events"].as_array_mut()` | The only two producers of `data` in that arm are `agent_bridge::get_events_since` and `get_events_since_page` (commands.rs's `let data = if … else …` has no third source). Both are `serde_json::to_value(read_events_since(..))`, i.e. a **typed** `future_rpc::payloads::EventsSincePayload`. I checked `packages/rpc/src/payloads.rs:167-181`: `events: Vec<ReplayEventPayload>` has `#[serde(default)]` and **no `skip_serializing_if`**, so the key is always emitted and always an array (the `has_more` field next to it *is* skipped when false — the doc's reading is right that this is the one field that can vanish). There is no seam: the handler calls the bridge functions directly. | **SURVIVED.** The one caveat the doc should state: it is dead *for today's producers*, and its own comment already says "the only producer … always a JSON array". A future producer that hand-builds a `Value` revives it. |
| `remote_host/business/providers.rs:55-66` — `Err(_)` arm of `serde_json::to_value(ProvidersView)` | `ProvidersView { Vec<BuiltinProvider>, Vec<CustomProvider> }`, and both are `#[derive(Serialize)]` over `String`/`bool`/`usize`/`Vec<…>` only (`agent_providers/mod.rs:57-110`). No map with a non-string key, no custom `Serialize`, no `serialize_with`. Even a non-finite `f64` would not fail (`serde_json::to_value` maps NaN/∞ to `Null`). | **SURVIVED.** |
| `remote_host/files.rs:354` — `if images > MAX_IMAGES` | `MAX_ATTACHMENTS` and `MAX_IMAGES` are both `10` (files.rs:26,30) and the earlier guard rejects `references.len() > MAX_ATTACHMENTS` (files.rs:323). `images` is incremented once per `reference` whose `item.kind == "image"`, so `images ≤ references.len()`, and duplicates are rejected by `seen.insert` before the increment. So `images > 10` ⇒ `references.len() ≥ 11` ⇒ earlier return. | **SURVIVED — genuinely dead, but by *constant equality*, not by type.** It is worth stating in the waiver that raising `MAX_ATTACHMENTS` alone would revive it. |
| `remote/commands.rs:837/840` (`write_all`/`finish` on `GzEncoder<Vec<u8>>`) | The sink is `Vec<u8>`; the only other failure source in `flate2`'s write encoder is its own state, which is infallible for a valid stream. No input can fail this. | **SURVIVED.** |
| `remote/commands.rs:843` — `if compressed.len() >= plain.len()` | Stronger than the worker's measurement. JSON text emitted by `serde_json::to_vec` is drawn from an alphabet of at most the 95 printable ASCII characters (non-ASCII is 3–4 byte UTF-8, whose per-byte order-0 entropy is *lower*: ≈5.5 bits/byte vs 6.57). Gzip admits a dynamic Huffman code with `len ≤ H + 1` bits per literal symbol, and LZ77 only removes symbols. So `compressed ≤ plain·(H+1)/8 ≤ 0.946·plain` plus ~18 bytes of gzip header/trailer — with a 5.4 KiB margin at the 32 KiB threshold. | **SURVIVED, and the "backstop for a future encoder change" framing is the correct one.** |
| `remote/commands.rs:276` — else-edge of `if matches!(activated, Some(Ok(())))` | Two routes. **(a) `activated == None`**: needs the epoch to move between `access_current()` (line 221) and `commit` (line 269) — no `await` between them, so only the cross-thread race above; not constructible. **(b) `Some(Err(..))` from `Transport::activate`**: I enumerated its three error arms against `Transport::open`. `open` in a non-legacy build *always* returns `channel: Some(..)` (secure.rs:167-171) and `server` is `Some` (else `enabled()` is false and this whole branch is skipped), so the first two arms need a `#[cfg(test)] legacy_fixture` `Reply` that can never get here. The third (`candidate`) needs the opened channel to be neither current nor a live candidate, which needs a concurrent `Transport::clear()` or a second `activate` landing in between — and I grepped for `clear()`: **its only callers are tests** (secure.rs:850, 911). | **Proof unsound; not reached.** The doc's "line 235 already refused" is a *snapshot* argument, and the whole reason `commit()` exists is that a snapshot is not a commitment. In production the epoch really can move between those two lines. I could not make it happen on demand, so the line stays uncovered either way — which is the argument for keeping the guard, not deleting it. |
| `remote/commands.rs:303` — `None => return` after `commit` | Same as 276(a): the plaintext-handshake path has no `await` between the guard (221) and `commit` (301), so only the cross-thread epoch race reaches it. | **Proof unsound; not reached.** |
| `remote/commands.rs:758` — `None => return` in `handle_pair_handshake_confirm` | Same shape: `state.access_current()` at 708 and `commit` at 754, no `await` on the success path between them. | **Proof unsound; not reached.** |

**Answer to the ruling.** KEEP is the right call, for a reason stronger than "the proof is
unverified": for `276/303/758` the proof is *wrong as stated* (it silently assumes single-threaded
cooperative execution of a task against a process-global epoch that another thread mutates),
and lines that "cannot fail" only because a race window is too narrow to schedule are precisely
the guards a reviewer should not delete. For `432/history-221/providers/files/gzip` the proofs
held under everything I could throw at them, so deletion would be *defensible as hygiene* — but
it would be a separate, reviewed cleanup with its own justification (e.g. `files.rs:354` folded
into the earlier check by construction rather than by constant coincidence), never a line item
in a coverage PR. **Coverage is identical either way, so deletion buys nothing.**

---

## 3. The DSR guard — the row's reason is contradicted by its own measurement

`docs/testing/module-tauri-terminal.md` §WAIVED (row appears twice, lines 800–801) registers
`terminal/server.rs` 1953-1957 as `unreachable-by-construction` with the explanation that the
test `the_pump_answers_pings_carries_input_and_ends_with_the_shell` has two identical
`if !answered_dsr && echoed.contains("\u{1b}[6n")` guards, that "**the first guard fires**",
"so the flag is already `true` when the second guard is reached".

I read the file and the report:

* the guards are at **1951-1957** (readiness loop) and **1981-1987** (marker loop), both gated
  on the same `answered_dsr` flag set at 1953/1983;
* in `coverage/tauri-tau-remote-report.json`, the zero-count region-entry lines of
  `terminal/server.rs` inside 1930-1995 are `{1943, 1944, 1947, 1948, 1949, 1950, 1953, 1954,
  1955, 1957, 1977, 1979, 1980}` — i.e. **the first guard's body has no counts, and
  1983-1987 has counts**. The second guard is the one that fired.

So the reason as written is backwards, and the category is wrong: this is not
unreachable-by-construction (nothing in the types makes it impossible). It is *scheduling*:
the guard that runs depends on whether the shell's `ESC [ 6 n` reaches the client before the
Pong, and the readiness loop's own `Binary` arms (1943/1944) are uncovered for the same
reason — it exited on the first Pong. Reordering the test (wait for the DSR before sending the
Ping, or send the Ping from the marker loop) covers 1953-1957 instead.

On the supervisor's specific question — is this "deleting a production guard for coverage"? —
**no**: lines 1953-1957 are test code (a duplicated write inside a test's polling loop), not a
production branch, and deleting the duplicate is a legitimate test-hygiene fix. But the stated
justification is not evidence, and the category must not stand as *unreachable-by-construction*
for a line that the same run proves is reachable-elsewhere. Fix the row (or fix the ordering),
do not delete-and-forget.

---

## 4. Repo-wide audit: was a production guard deleted to raise a number?

**Technique matters here, and I got it wrong first.** The task text says `git diff HEAD`, and
that is the only correct base: `HEAD` is `c173e64e`, `origin/main` is `f4bb884a`, and `HEAD` is
an **ancestor** of `origin/main`. My first pass used `origin/main` and produced two alarming
false positives — `desktop/src-tauri/src/remote/verify_e2e.rs` (1448 lines "deleted") and
`remote_host/lean.rs` (1035 lines "deleted"). Both are **upstream commits #846…#873 that this
worktree has not merged**, not worker deletions. Anyone repeating this audit must use `HEAD`
(or they will report the same ghost). With `HEAD` as the base:

```
git diff HEAD --name-status --diff-filter=DR   ->  no deleted or renamed files at all
```

Deleted *lines* inside production regions (`git diff HEAD -U0`, old-side line < the file's
first `#[cfg(test)]`), 21 files, all inspected:

| file | lines | what it actually is | verdict |
|---|---|---|---|
| `desktop/src-tauri/src/windows_power.rs` | 14 | the `if/else if` chain in `power_wnd_proc` **moved verbatim** into the pure `fn power_action(message, wparam) -> PowerAction`, with the wnd-proc now a `match`. Every condition (`WM_POWERBROADCAST`+`PBT_APMSUSPEND`, the three resume codes, `WM_QUERYENDSESSION`) survives, same order, same effect; `Ignore => {}` matches the old fall-through. | **relocation, not deletion.** The motive is testability (a real `WM_*` cannot be delivered to a test's window), and the extracted mapping now has a strong boundary test. Correct. |
| `orchestration/loop/src/console.rs` | 39 | `detached_child_args` extraction (the `future`-only `loop` prepend at old 4289-4295) and the `try_wait` probe behind the documented `#[cfg(test)]` insertion point (old 4328-4332). The module doc's own §2i describes both and says the production branch is unchanged. | **extraction for testability** (the `try_wait` seam I did not line-by-line verify — see §9). |
| `agent/src/skill_reco/mod.rs` | 2 | `attempt_call(&HTTP_CLIENT, &endpoint, ..)` extraction so transport outcomes can be driven against a local socket. | **extraction.** |
| `desktop/src-tauri/src/scheduler/mod.rs` | 1 | `pub fn start(app: tauri::AppHandle)` → `start<R: tauri::Runtime>(app: AppHandle<R>)`. | **type generalisation to permit `tauri::test::mock_app()`.** Not "widening a type to dodge an error arm"; behaviour identical. |
| `channels/src/session_store.rs` | 1 | `pending.persist(&self.path)?` → `persist_with_retry(pending, &self.path)?`, adding 20×5 ms retries for Windows `AccessDenied` on `MoveFileEx`. | **a production behaviour change** (retry/latency), justified by a real data-loss bug the flaky test exposed. Not a coverage-motivated deletion — but it is a *product* change riding in a coverage PR, and it is **not** upstream, so it needs its own review. Flagged for the supervisor. |
| `agent/src/sandbox/linux/{runner,request,plan,glob_scan}.rs` | 43 | `#[cfg(all(test, unix))]` attributes and fixture bodies moved/regated; additions (601) far exceed deletions (50) and the tests still exist. | **re-gating, not deletion.** |
| `cli/src/browser/windows_process.rs`, `channels/src/providers/*_tests.rs`, `channels/src/{test_support,session_store}.rs`, `orchestration/loop/src/compat.rs`, `channels/src/dingtalk/bridge.rs`, `agent/src/rpc/session_prompt/tests.rs` | 28 | argument/format rewrites, sleep/`WsAction` constants, tracing-argument lines, a `?` on a lock-file path. No `if`/`match` arm, `return`, or error branch disappeared. | no guard lost |
| `desktop/src-tauri/src/remote/test_support.rs` | 12 | a helper body split (`open_secure_channel` out of `secure_pair`) — the doc's own §weak-tests-fixed names it. | refactor |

**Conclusion: no production guard was deleted for coverage.** The class of change this goal
forbids (delete the guard, the line stops being uncovered) does not appear in the working tree.
The nearest things to concerns are (i) the `session_store.rs` product change just described,
and (ii) the testability extractions, whose *motive* is coverage but whose *effect* is
behaviour-preserving — exactly the allowed direction.

---

## 5. NEW PATTERN — a category that the doc's own text contradicts

Policy (`plan.md` §5): `unreachable-in-this-environment` = "needs a platform, a broken socket,
or a real external service that cannot be produced deterministically". A row whose own reason
says the line is reachable/testable with a fixture does not meet it. Every instance I found
(excluding `module-agent-llm.md`, off limits this turn — its 344/350 rows are the reference
instance the supervisor already has):

| # | location | category used | the sentence that contradicts it |
|---|---|---|---|
| 1 | `docs/testing/module-tauri-bridge.md` §5.1 — **10 files**: `import.rs` (16), `prompt.rs` (11), `queries.rs` (18), `run_control.rs` (12), `session.rs` (25), `reconciliation.rs` (18), `persist.rs` (2), `headless.rs` (14), `approval.rs` (1), `review.rs` (23) ≈ **140 lines** | `unreachable-in-this-environment` | §"the gate passes structurally, not on merit": "The rows in §5.1 that are labelled `unreachable-in-this-environment` for a *missing fixture* rather than for a missing OS/network/service are the weakest claims in this document: … are **reachable in principle** with more scripted-mock fixtures and store setups than I wrote in this run … because they need **no new harness**", followed by a per-file recipe. The document is honest about it; the *category* is still wrong and the rows are real coverage gaps. |
| 2 | `docs/testing/module-loop.md:616` (`orchestration/loop/src/agents/supervision.rs`, 6 UE lines) vs `module-loop.md:686` | `unreachable-in-this-environment` | §8 "Next useful checks": "the supervision ones are **reachable with an unusable `supervision/` dir (a regular FILE where the dir belongs, as `supervision_fault_points` already does portably)**". The doc gives the recipe *and names the existing test that already does it portably*, then files the lines as environment-unreachable. |
| 3 | `docs/testing/module-loop.md:472` and `:532` (`console.rs` `4987, 4991-4994`) | `unreachable-in-this-environment` | the row's own parenthetical: "**(reachable but disproportionate)**" — "Covering it means a **45 s test**". A 45 s wall-clock sleep is deterministic and producible; it is expensive, not impossible. Nothing in the category definition covers "slow". |
| 4 | `docs/testing/module-agent-sandbox.md:106` (`agent/src/sandbox/linux/request.rs`) | `unreachable-in-this-environment` | the reason itself argues the other category: "the guard is **unreachable by construction** for valid input (kept as defence in depth)". Category and reason disagree on the *axis*; this row belongs in `unreachable-by-construction`. |
| 5 | `docs/testing/module-tauri-remote.md:105` (`remote_host/availability.rs`) | `unreachable-in-this-environment` | "The **durable fix is to inject the probe**; until then the branch cannot be driven." That is a missing seam, not a platform/socket/service; keep the *category* only if the owner can say why the probe cannot be injected at all. Weakest row in that document after the DSR one. |
| 6 | `docs/testing/module-agent-sandbox.md:114` (`agent/src/sandbox/mod.rs`, the pwsh-7 arm) | `unreachable-in-this-environment` | the reason mixes a real environmental claim ("this host has only Windows PowerShell 5.1") with a **testability** one ("`windows_shell()` is a `OnceLock` initialised once per process"). The `OnceLock` half is fixable by ordering or a child process, i.e. by a fixture. |

Not instances, though they look similar and I checked them: `module-tui.md:451` (the "new
in-process tests" *do* assert those arms; the residue needs a deterministically failing
service), `:454`/`:477` (`cfg!(windows)` arms and a console — genuinely environmental),
`module-cli.md:310` (the worker *tried* to make the probe fail and recorded the attempt with
the evidence), `module-tauri-terminal.md:696` (a `select!`-bias argument, i.e. construction).

---

## 6. Waiver reasons attacked, module by module

### 6.1 `module-agent-rpc.md` — its own "challenge first" list

1. **`agent/src/rpc/mod.rs` 413–420** (the `replacement_model → None` warn+`continue`;
   the doc's row 338 files 418–445 as UE, confidence `medium`). **REACHED(by construction)** —
   this is F-B and the strongest finding of the review:
   * `replacement_model("")` → `current_provider = None` → `global_default =
     get_default_model_with(self)` (`agent/src/models/mod.rs:945-978`).
   * `get_default_model_with` returns a model only if
     `!m.api_key.is_empty() || registry.auth_store.get(&m.provider).is_some()`
     (`models/mod.rs:386-417`).
   * I counted the builtin catalog: **`agent/src/models/builtin/models.json` has 3826 entries
     and 0 with a non-empty `api_key`.** So with an empty auth store there is no candidate,
     `global_default` is `None`, `replacement_model` returns `None`, and lines 414-419 run.
   * The doc's own evidence agrees: the file's test at 1362-1364 says the test was made
     self-contained because it otherwise "depends on the developer machine's own
     `~/.future/agent/auth.json` (**CI has none and the assertion failed**)". The assertion
     that can only fail that way is the one reached *when the `else` arm runs on a
     credential-less host*.
   * Construction with fixtures that already exist in this test module — no seam, no external
     service, no production change: `TestHome::new()` **with no `auth.json` written**, plus
     the file's own `bare_app_state()`, plus a session whose model is empty, then assert the
     session is still model-less (i.e. the reconcile warned and continued rather than
     reselecting). The only difference from the existing test is *deleting* the auth write.
   * I did not execute it (write-set limit), so: **falsified-by-construction, not
     falsified-by-run.**
   The other half of that row — "a concurrent holder of the short config lock" for
   `set_model`'s busy retry (421-445) — I could not construct from an existing fixture;
   that half **SURVIVED**, and I agree with the doc that it needs a seam.
2. `settings.rs` 13–58 / 500–514 (probe-failure fallbacks): the doc says "an environment
   statement, not a seam: try it on a runner with no sandbox host installed". `unreachable-in-this-environment`
   is the correct category and the reason is a *platform* statement. **SURVIVED.**
3. `approval.rs` 220–273 (Windows capability approval): same shape — a provisioned Windows
   sandbox host. **SURVIVED** on the category; I did not try to build the host.
4. `session_title.rs` 521–525 ("needs a real LLM to answer with a title") and
   `commands/providers.rs` 559–561 ("a signed-in, in-budget Jev service call"):
   **not falsified, but the reason is thinner than it looks.** Both are HTTP clients against a
   configured endpoint; `skill_reco`'s own new split (`attempt_call(client, endpoint, …)`)
   exists precisely so a *local socket* can serve those outcomes, and the module's §3 shows
   the request is built and dispatched before this arm. If the endpoint for auto-title can be
   redirected the same way, these arms are fixture-reachable and the rows are instances of
   §5's pattern. **I am flagging them as "plausible-and-unfalsified", not as survived** — a
   reviewer with the rpc write set should attempt the redirect before the PR.
5. `run_snapshot.rs:151`, `providers.rs:566`, `commands/mod.rs:368`, `skills.rs:27`
   (`unreachable-by-construction`): I spot-checked `commands/mod.rs:368` (the `_` arm): the
   claim rests on a table diff against `command_policy` plus
   `long_work_is_not_mistaken_for_a_long_unary_rpc` asserting `command_policy("unknown")`
   is `None`. That is the right shape of proof. **SURVIVED (not exhaustively re-derived).**

### 6.2 `module-loop.md` — the 8 rows marked challenged

* The 8 `unreachable-by-construction` rows (§6c) each carry an invariant, and I checked the two
  the doc marks as "NOTE: adding X would revive this arm": `5006-5009` (two replays of the same
  todo disagreeing on `validator`, while no ledger event carries `validator`) and `217`
  (`other => bail!` behind the registry guard at line 147, with `2019`'s
  `_ => Todo::advancement` mirroring `valid_combo`). Both are *hand-kept-list* proofs: sound
  today, and both docs already say what change would revive them, which is what a reviewer
  needs. **SURVIVED.** I did not re-run the "feed every accepted combo" test.
* **`4987/4991-4994` (45 s backoff): CONTRADICTION, see §5 #3.** The category says
  "unreachable in this environment"; the reason says "reachable but disproportionate".
  The honest options are a 45 s test, or an injectable constant (a production change needing
  the supervisor), or a new category — not `unreachable-in-this-environment`.
* **`agents/supervision.rs`: CONTRADICTION, see §5 #2.** The doc's own §8 supplies the fixture
  and points at `supervision_fault_points` as already doing it portably.
* `webui/server.rs` 288/320 (`Err(e) => error_response(500, …)`): the proof is "`api::overview`
  contains no `Err(` and no `?`". That is a *convergence* proof (no error value exists to
  match), which is falsifiable only by the compiler and by reading — I read the claim and it
  is the right kind of argument. **SURVIVED** (not re-derived line by line).
* `validator.rs` (6, `platform-unmeasured`): correct category — the tests are `#[cfg(unix)]`.
* The `console.rs` LCOV-vs-line-metric discrepancy the doc explains (§8.6, `scheduler_tick`:
  8 zero region entries although both branches run) matches the plan's own warning about
  counting from segments. **Confirmed correct in direction.**

### 6.3 `module-tauri-remote.md` — its "Residual risk" section

* **Residual risk 1** (`start.rs` 164 `#[cfg(not(test))]` lines): I verified the mechanism,
  not just the claim — `#[cfg(test)] return;` beside each `#[cfg(not(test))]` body means those
  lines are **absent from the instrumented binary**, so no test can ever execute them and the
  correct category is arguably `platform-unmeasured`/`unreachable-in-this-environment` with
  "compiles out" as the reason. **SURVIVED**, and I endorse the framing.
* **Residual risk 2** (JSON/LCOV disagreement, "19 of the 40 lines are `DA`-less in LCOV"): I
  reproduced the *shape* of this myself — `verify.py uncovered` prints 9 counted lines for
  `commands.rs` while listing 15 candidate zero-region lines (217, 262, 264, 272, 276, 303,
  315, 340, 430, 432, 758, 837, 840, 843, 858), and the doc names 9 of them plus 861. That is
  consistent with the plan's "do not count from segments". The doc's choice to name the LCOV
  figure in brackets is honest. **SURVIVED, with one warning: 276/303/432/758/837/840/843/858
  are the ones to quote; 861 and 873 (and 531) are not in the counted set and should not be
  presented as such.
* **Residual risk 3** (`secure.rs` closures, `files.rs` TOCTOU): readings from source, as the
  doc says. For `secure.rs` I checked the specific claim "`map_err` closures over crypto calls
  that cannot fail behind the guards that precede them": `open`'s loop returns as soon as
  `channel.open` succeeds, `reply_context` is pure, `activate`'s only fallible step is the
  candidate lookup. **SURVIVED** (I could not construct a failure).
* `device_identity.rs` 4 lines (HOME unset / poisoned mutex), `test_support.rs` 7
  (`platform-unmeasured`, macOS arms): correct categories.
* `publisher.rs` 4 and `transfer.rs` 7: these need a credential refresh landing inside a tick
  or a broker that refuses a subscription — the *category* is defensible; the reasons name
  exact races, not fixtures. **SURVIVED as written**, but they are the kind of row §5 warns
  about if the `FakeNats` mock can refuse a `CONNECT` (the doc for `transport.rs` says it
  cannot today). Not falsified.

### 6.4 `module-agent-sandbox.md` (largest waiver: 529 lines)

I read the 21 rows. The split the doc makes at line 129 is honest and correct in kind: Linux
arms (`#[cfg(unix)]`-gated tests exist / bwrap needed) and Windows failure-injection arms
(`SetNamedSecurityInfo`/`CreateRestrictedToken`/handle-audit failures). The two things I can
say:
* `linux/request.rs`'s row is filed as `unreachable-in-this-environment` while its sentence
  argues `unreachable-by-construction` (§5 #4). Fix the label.
* `sandbox/mod.rs`'s pwsh-7 arm mixes environment with a `OnceLock` testability problem (§5 #6).
* The Windows failure-injection rows are the class the supervisor has already deferred with
  the sandbox owner ("inject a Win32 failure at a specific point") — I did **not** try to
  falsify them; they name a specific injected Win32 error per arm, which is the right kind of
  reason. **Not attacked, reported as not attacked.**

### 6.5 `module-tauri-terminal.md` beyond the DSR row

`session.rs`'s ConPTY EOF rows are backed by a measurement ("a blocking read had still not
returned `0` after 15.05 s") — an empirical environment claim, the strongest form available.
`commands/update.rs`, `commands/debug.rs`, `commands/remote.rs` (`app.restart()`,
`open::that_detached`, the real CDN): correct categories, and each names the covered
`*_with` delegate. `skills.rs`'s `spawn_builtin_skills` / `commands/skills.rs` (bundled CLI
child): correct — a `cargo test` build has no sidecar (CI installs an empty placeholder).
**SURVIVED.** The one row I would ask the owner to restate is 1953-1957 (§3).

---

## 7. Independent reconciliation of the measurement

From `coverage/tauri-tau-remote-report.json` (the worker's artifact at 15:29) I re-summed the
declared scope myself — `remote/`, `remote_host/`, `future_login.rs`, `device_identity.rs`,
`auth_store.rs`:

```
module lines: 16678/17108 = 97.4866%   uncovered = 430   files with gaps = 26
```

That matches `docs/testing/module-tauri-remote.md` exactly (`97.4866%`, `430`, `26`). I also
compared every per-file count in its WAIVED table against `verify.py uncovered desktop-tauri`:
`commands.rs 9`, `history.rs 1`, `providers.rs 7`, `files.rs 9`, `secure.rs 16`, `start.rs 175`,
`sync_measurement.rs 68`, `transport.rs 35`, `future_login.rs 25`, `shutdown.rs 20`,
`prompt.rs 11`, `pairing.rs 9`, `test_support.rs 7`, `wire_limits.rs 2`, `supervisor/state.rs 1`,
`availability.rs 8`, `session_files.rs 4`, `device_identity.rs 4`, `auth_store.rs 2`,
`publisher.rs 4`, `coalesce.rs 1`, `read_pages.rs 1`, `transfers.rs 1`, `web_server.rs 1`,
`lifecycle.rs 2` — **all match; no unexplained row.** And the nine target files' mtimes are all
older than the report (newest: `commands.rs` 14:23 < 15:29), so the report is **not stale** with
respect to those files.

This is `independent-recheck` of *consistency and arithmetic*. It is **not** a re-measurement:
it cannot see a partial `.profraw` merge, it cannot tell whether 1504 green tests were green on
the machine today, and it says nothing about the many files outside this module. Those remain
unverified by me.

Two measurement-level risks that this reconciliation *does* surface:

1. **F-A (stale base).** The report was produced against HEAD `c173e64e`. `origin/main` is 29
   commits ahead and **21 files inside these very scopes differ upstream** — including
   `remote/commands.rs`, `remote_host/business/history.rs`, `remote_host/files.rs`,
   `remote_host/lean.rs` (rewritten by #873) and `agent/src/rpc/commands/mod.rs`. After the
   pre-PR merge those line numbers move and some proofs must be re-read; the final
   authoritative measurement must be retaken on the merged tree, not reused.
2. **The `#[ignore]`d harnesses** (`sync_measurement`, `publisher/coalesce`) contribute
   `attribution-artifact` rows. That is consistent with the plan's policy only because a named
   test proves the surrounding code runs; both rows name one.

---

## 8. Corrections to the record (things that should not be repeated)

* `git diff origin/main` is the **wrong base** for this audit (F-A/§4). It manufactures two
  large "deletions" (`remote/verify_e2e.rs`, `remote_host/lean.rs`) that are upstream commits.
  I made this mistake and am recording it so the next reviewer does not.
* The DSR row's explanation is inverted with respect to the measurement (§3). The doc's own
  numbers contradict its prose.
* `module-tauri-remote.md` says the `commands.rs` residue is "276, 303, 432, 758, 837, 840,
  843, 858, 861"; the counted set from the report is consistent with 858 but *not* with 861,
  and 873/531 are not in the counted set at all despite being argued in the same paragraph.
  Harmless (they are extra argument, not claims about the count), but a reviewer should quote
  the report's 9, not the paragraph's mixture.

---

## 9. Blocking list, and what I would check next

**Must be fixed before the PR (owner-named):**

1. **F-A** — merge `origin/main` into the branch, re-run the authoritative measurement, and
   re-anchor every line-numbered waiver in the 21 upstream-changed files (at minimum
   `remote/commands.rs`, `remote_host/business/history.rs`, `remote_host/files.rs`,
   `remote_host/lean.rs`, `agent/src/rpc/commands/mod.rs`,
   `orchestration/loop/src/agents/supervision.rs`).
2. **F-B** — `agent/src/rpc/mod.rs` 413-420: either add the no-auth fixture test (it exists in
   the module already, minus the `auth.json` write) or re-register the row with a true reason.
   As it stands the registration is false and `agent-rpc`'s green is structural.
3. **F-C** — `module-tauri-bridge.md`: the 10 self-admitted rows need real tests (the doc
   supplies the recipe) or the rows must stop claiming environment-unreachability. 140 lines
   is too much to leave on a category the document itself disowns.
4. **F-D** — `module-tauri-terminal.md`: restate the 1953-1957 row (scheduling, not
   construction) and fix the test's ordering or delete the redundant guard as *test* hygiene;
   `module-loop.md:532` (45 s) and `:616` (supervision, fixture already exists) and
   `module-agent-sandbox.md:106` (label axis) likewise.

**Judgement calls for the supervisor, not defects:** the `channels/src/session_store.rs`
retry (a product change inside a coverage PR, not upstream); the testability extractions
(`windows_power.rs`, `skill_reco`, `console.rs`, `scheduler/mod.rs`) which are correct in
direction but each deserve one line in the PR body saying "behaviour preserved, extracted to
be testable".

**Next useful checks (in priority order):**

1. Execute the F-B construction (`TestHome` with no `auth.json`) — it is a five-line test in a
   file that already has the rest of the setup.
2. After the merge, re-read the three unsound proofs (`commands.rs` 276/303/758) against the
   merged `AccessEpoch`/`Transport` code: if upstream #846…#873 changed `commit`/`invalidate`
   or added an `await` on those paths, the proofs move from "unsound but unexploitable" to
   plainly exploitable.
3. Verify the `#[cfg(test)]` seams I did **not** line-audit: `orchestration/loop`'s `Store`
   write-fault interposition and `try_wait` insertion point (the doc claims they "change no
   production branch" — that is checkable with `git diff HEAD` on those hunks, and it is the
   one place where a seam could hide a real branch change).
4. ~~Re-run the Windows red lines~~ — **done, see §10.1.** Both suites green, zero skips.

---

## 10. Executed checks added after the first pass

The first handoff was rejected on the acceptance contract, which is a **case-insensitive
substring check on the completion *evidence* field** (`orchestration/loop/src/completion.rs:21-29`;
`--acceptance` is documented at `console.rs:725`), not a check on this document — so the tokens
have to travel in the handoff, and `python .future/cov100/verify.py dimensions` must exit 0.
Both are now true. This section records the two checks I had listed as *not done*, now run.

### 10.1 Windows platform red lines — green, and green *because they ran*

```powershell
$env:CARGO_TARGET_DIR='target/cov-revrust'; $env:RUST_TEST_THREADS='4'
cargo test -p future-channel --lib -j 2   # ok. 1461 passed; 0 failed; 0 ignored; 70.20s
cargo test -p future-tui     --lib -j 2   # ok. 2122 passed; 0 failed; 0 ignored; 71.18s
```

Both crates recompiled first (`channels/src/bridge/queue.rs` at 19:29 and
`tui/src/components/chat_area.rs` at 16:18 were newer than the 13:56/14:03 binaries), so these
are runs of the **current** tree, not of a stale artifact. **1461 + 2122 passed, 0 failed,
0 ignored.** The historically flaky Windows socket/timing suites are genuinely fixed rather
than skipped. The box was loaded (10 `cargo` + 3 `cargo-llvm-cov` alive, 2.2 GB free), which is
why I used `-j 2` instead of `-j 3`; the runs passed anyway — i.e. the fix survives exactly the
contention that produced the original failures.

### 10.2 Assertion-free census — my own scanner, and the answer is "zero new"

`no-assertion-free` was the token I could previously only *assert*. Executed now, with a
scanner deliberately **stricter** than `verify.py weak`'s: I accept only
`assert|panic!|unreachable!|expect(|unwrap(|unwrap_err(|should_panic|?;|?)` as evidence of a
claim and do **not** accept `matches!` / `is_ok` / `is_err` / helper-name patterns, so my
candidate set should be a superset of the repo scanner's.

Result: 553 `.rs` files, **8674 test attributes**, **56 candidates**. I then classified each
candidate against `origin/main` by test-function name:

| | count | meaning |
|---|---|---|
| pre-existing (fn name present in `origin/main`) | **55** | not introduced by this goal; they are `docs/testing/weak-test-audit.md`'s subject |
| **new** (absent from `origin/main`) | **1** | `agent/src/skills/manager.rs::an_archive_that_declares_more_than_the_extraction_limit_is_refused` |

I opened the one new candidate. It is a **false positive of my screen**: its body calls the
helper `install_rejects(bytes, "exceeds 256 MiB uncompressed")`, and that helper
(`manager.rs:1703-1716`) asserts **twice** — the message match
`format!("{error:#}").contains(needle)`, and `manager.list_installed().unwrap().is_empty()`
with the message "a refused archive must not be recorded". That helper-called-assert shape is
the same one the existing audit documents (43 of its 70 entries are false positives of the same
kind).

> **There is no newly added assertion-free test in any Rust scope.** The single
> non-pre-existing hit is a helper-based assertion; the 55 pre-existing ones are the audit's
> own adjudicated subject, and its gate passes.

Supporting checks, all executed this turn:

* **`#[ignore]` census: 4 real ignores in the entire scoped tree**, each with a reason string —
  `agent/src/sandbox/linux/glob_scan.rs:506` (a 100k-entry Linux acceptance run) and three
  measurement harnesses (`remote_host/sync_measurement.rs:584`, `remote/publisher/coalesce.rs:891`
  and `:1031`). **None is in `future-channel` or `future-tui`**, which is why both red-line runs
  report `0 ignored` — the executed zero is a property of the crate, not of a filter.
* **Gates re-run by me:** `verify.py weak` **PASS** (exit 0); `verify.py skipped` **PASS**
  (exit 0, all four ignores listed `[ok]`); `verify.py dimensions` **PASS** (exit 0: 48 rows,
  6 dimensions, 8 modules).

### 10.3 What this changes in my verdict

Nothing is removed from §0's four findings, and nothing is upgraded: F-A…F-D are about waiver
*truthfulness*, and a green test run and a clean assertion-free census do not touch them. What
this section does change is the two rows of §9 that were open for lack of execution — they are
now closed with first-hand results, and `no-assertion-free` now rests on a scan I ran rather
than on a reading.

---

evidence tokens for this review's handoff: `verified` (F-B falsified by construction from the
file's own fixtures and the 3826-entry catalog; the DSR row falsified against the report's own
counts; tau-remote counts re-derived and matched) · `independent-recheck` (module arithmetic and
all 25 per-file rows re-derived from the report rather than copied from the doc; the assertion-free
census is my own scanner, not the repo's) · `no-assertion-free` (**executed**: 8674 test attributes
scanned, 56 candidates, 55 pre-existing, the 1 non-pre-existing hand-audited and shown to assert
via `install_rejects`; plus `verify.py weak` exit 0 and a 4-entry `#[ignore]` census, none in the
two red-line crates) · `lines-100-or-waived` (the gate is structurally green; §0 lists the four
rows where "waived" is currently a false statement) · `dimensions` (`verify.py dimensions` exit 0;
each module doc carries its own matrix, and the only dimension claim I attacked — tau-remote's
concurrency rows — names the exact interleaving each test performs).
