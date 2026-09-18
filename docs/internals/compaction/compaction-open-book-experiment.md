# Open book: does retrieval help the summarised strategy?

One arm, `summarized`, against the closed-book chain set. The question: if the model can
search the original history after compaction, does it recover what the projection dropped?

**Answer: not on its own.** Retrieval was never attempted in any of the three prompt variants
that left it optional, and the only configuration that produced a gain was one in which the
exam required verification — which is not a production behaviour. The recall guidance was
removed from the runtime as a result; the retrieval CLI is retained.

This page records the rounds, including the ones that failed.

## The instrument

Production's request shape, taken from the Rust code rather than restated
(`production_shape.RequestShape` → `compaction_probe --print-request-shape`):

| | value |
|---|---|
| system prompt | the session's own, byte for byte |
| tools | `coding_tools()` — `read`, `write`, `edit`, `shell` |
| retrieval affordance | `future session history search` / `get`, behind the ordinary shell tool |
| archive under test | the same records the closed arm was scored on, in an isolated Agent database |
| tool execution | production's handlers, through `production_tool_executor` |
| model | `future/deepseek-flash`, matched generation settings |
| scoring | `realistic_exam.score`, identical to the closed run's |

Every tool call runs through production code. The harness adds one thing: an `allowed_roots`
boundary, because production's file tools resolve an absolute path as-is and would otherwise
reach outside the case (see [Isolation](#isolation)).

## Rounds

All runs are 18 cases. "Tool calls" counts executions that reached a handler.

| # | What changed | Design | Recall | Tool calls |
|---|---|---|---:|---:|
| — | *(closed baseline, frozen)* | no tools | 147/178 (82.6%) | 0 |
| 1 | production prompt + tools, nothing else | autonomous | 148/178 (83.1%) | **0** |
| 2 | + stronger production guidance | autonomous | 149/178 (83.7%) | **0** |
| 3 | + exam wording that stops calling the projection "the record" | autonomous | 145/178 (81.5%) | **0** |
| 4 | closed re-scored under round 3's wording | no tools | 147/178 (82.6%) | 0 |
| 5 | + an explicit verification instruction, first call forced | required | **166/178 (93.3%)** | **100** |

Readings:

* **Rounds 1–3 are the same result three times.** Tool calls are zero, and the spread in
  recall (145–149) is one-flip noise on 178 values: those runs differ from the closed baseline
  only in which in-projection values the model happened to report. Nothing was retrieved in any
  of them, so nothing was added.
* **Round 4 is the control for round 3.** If a wording change moved the score, the closed arm
  would move with it; it did not (147 both ways), so round 3's −2 is noise rather than the
  model getting worse.
* **Round 5 is the upper bound, not a result.** It is what the same model does when told to
  verify and made to call once. It is not reachable in production, where no such instruction
  exists.

### Why the misses happened

Every one of the 178 examined values, classified by whether it was in the projection, whether
the model reported it, and whether a search query ever contained it:

| | closed | rounds 1–3 | round 5 |
|---|---:|---:|---:|
| reported, and present in the projection | 147 | 148–149 | 146 |
| reported, absent from the projection (**retrieved**) | 0 | 0 | **20** |
| present in the projection, not reported | 4 | 2–5 | 5 |
| absent, searched for, still missed | 0 | 0 | 3 |
| absent, never searched for | **27** | **27** | 4 |

The 27 is constant. Those values are in the archive and the model never looked: the required
round recovers 20 of the 28 absent ones by searching. So the gap is **behavioural, not
capability** — nothing was lost that could not be found, only never sought.

That is also why prompt-side changes could not have worked. Rounds 1–3 each tried a different
way of saying "you may look"; the model answered from the projection every time, at the cost of
omitting values it could have confirmed. Round 5 differs not in how retrieval is described but
in whether it is required.

### What was tried, precisely

Round 2 rewrote production's own guidance (`history_recall.rs`) to lead with
"absence from the visible context is not evidence that something never happened" and to make
a targeted search the expected default, keeping every scope and safety rule. Round 3 left the
prompt alone and changed the exam question, which had been telling the model
"the conversation above **is the record**" while the guidance said "partial projection, not the
complete record", and had been asking only about values that "appeared **in that
conversation**". Both were reverted; see [Outcome](#outcome).

## Cost

As recorded by the harness, in CNY:

| run | calls | charged |
|---|---:|---:|
| round 1 | 18 | 0.3265 |
| round 2 | 18 | 0.3278 |
| round 5 | 98 | 0.1780 |
| round 4 (closed re-probe) | 18 | 0.0177 |

**These do not reconcile and should not be used quantitatively.** Round 5 issued 5× the calls
of round 1 for a little over half the charge, implying a per-token rate roughly 11× lower.
The provider reports a per-call credit cost and the runs' own cache counters read zero
throughout, so the difference is not explained by anything this harness measured — the most
likely cause is provider-side caching of the shared prefix across the runs rather than any
property of the arms. Treat the figures as "under a tenth of a yuan per 18-case arm" and no
more.

## Isolation

Production's shell tool is bounded by the OS sandbox, but it is not bounded to the case: the
model ran `ls`/`grep` against paths outside its workspace.

| observation | count |
|---|---:|
| shell calls naming a path outside the case | 18 |
| …of those, returning non-trivial output from the operator's filesystem | 15 |
| calls refused by the harness `allowed_roots` boundary (a `read`) | 1 |
| calls that hit the 120 s shell timeout | 1 |

None of it reached an answer — every recovered value was attributed to a `future session
history` query, not to a host file — but two things follow. A replay that can read the host can
be contaminated by it, and a study that expects real session data to stay private cannot rely
on the tool layer alone. The protocol's existing caveat that this is "not a global
read-isolation guarantee" is a measured fact.

## Limits

* **One arm.** Only `summarized` was run. The other strategies are not covered.
* **One draw per case.** 18 cases, 178 values; a few points is one flip and nothing here is
  resolvable below that.
* **The exam is recognition.** It asks for exact values, which is what verbatim retention is
  best at. A task-shaped exam could plausibly behave differently.
* **The required round is an upper bound.** It measures what is retrievable, not what a
  session does.
* **Retrieval was proven, not optimised.** Round 5 issued 100 calls and hit the output budget;
  no attempt was made to find a better query strategy.

## Outcome

The recall guidance was removed from the runtime (`agent/src/agent/history_recall.rs` deleted,
call sites simplified). The reasoning is a property of the feature rather than of this
experiment: the guidance described a capability the model already had, and describing a
capability more firmly is not what makes a model use it. Three variants, 54 cases, zero
changed behaviour.

The retrieval CLI is retained — `future session history search` / `get`, the two RPC commands,
and the journal's entry references. What is gone is the runtime advertising it.

One consequence worth noting, and an improvement: a session's system prompt no longer changes
when a checkpoint is committed. It used to grow the guidance, which meant the request budget
had to reserve the growth in advance and the summary request had to reproduce it exactly or
lose the provider's prefix. Both problems are gone, and a test asserts the prompt is constant
across a checkpoint rather than the two call sites being kept in step by hand.

What the numbers say to try next is anything that acts rather than describes: a deterministic
pre-retrieval pass over the newest user turn, a forced first call after a checkpoint, or
widening what the projection itself retains. Two rounds of prompt rewriting are enough evidence
that the wording is not the lever.

## Reproducing

Everything is in [scripts/compaction_experiment/](../../../scripts/compaction_experiment/);
[README.md](../../../scripts/compaction_experiment/README.md) has the commands and
[OPEN_BOOK_PROTOCOL.md](../../../scripts/compaction_experiment/OPEN_BOOK_PROTOCOL.md) the design,
including which flags change the question.

```sh
cargo build -p future-agent --example compaction_probe --example production_tool_executor \
                          --example model_bridge
cargo build -p future-cli --bin future

# validate the whole path with no model call, then run for real
python3 scripts/compaction_experiment/run_open_book.py \
    --closed ~/compact-exp/v4-forced --arm summarized --output ~/compact-exp/open-summarized \
    --future target/debug/future \
    --executor   target/debug/examples/production_tool_executor \
    --driver     target/debug/examples/compaction_probe \
    --shape-probe target/debug/examples/compaction_probe \
    --base-prompt ~/compact-exp/shape/system-prompt.txt \
    --bridge target/debug/examples/model_bridge \
    --smoke --only export
```

Drop `--smoke` for the real run. The rounds differ only by flags, so each one is the same
command:

| Round | Flags |
|---|---|
| 1 | *(none)* |
| 2 | *(none)* — with the guidance strengthened in `agent/src/agent/history_recall.rs`, since reverted |
| 3 | `--exam-wording v2` |
| 4 | `--exam-wording v2 --closed-reprobe` |
| 5 | `--require-retrieval` |

Interrupting is safe: finished cases are kept and skipped on restart. `--retry-unsettled`
is the explicit escape hatch if a call was in flight when the run was aborted.

Before quoting any number: run `verify_request_shape.py`, which needs no model and checks that
the prompt is the session's own, unmodified, and that the tool definitions and the retrieval
CLI are intact. `--smoke` then exercises the archive end to end for one chain.

The frozen inputs and the per-case results stay in `~/compact-exp`, outside any repository,
because the real-session chains are private. Third-party reproduction of that half means
substituting your own sessions.
