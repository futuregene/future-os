# A 30-second window on a 26-second prologue

(`cli/tests/remote_start_refusal.rs::a_broker_that_refuses_the_credential_is_reported`)

A one-off record: the failure as the runner logged it, why the wait it used measured
the host as much as the product, the change that removes that measurement, the two
experiments that prove the new wait still fails when it should, and the other
fixed-seconds waits that were looked at and left alone. Everything below was measured;
commands are written out so each row can be re-checked.

## 0. Summary

| | |
|---|---|
| Observed | Windows CI, run 40267289153 **attempt 1**, job 121392429655 (`Rust Windows test / tui-cli`, 2026-10-26): the child had not printed `Remote connection stopped` 30 s after it was spawned; attempt 2 of the same run on the same commit passed |
| Cause | the wait window contained the child's own startup: the embedded agent's bring-up took **26.2 s**, and no `Remote:` line had appeared at all when the window closed — the remote supervisor had not started |
| Fix | the wait ends on an **outcome** (the line, the child ending, or the cap) instead of on a duration; `TERMINAL_CAP` = 300 s is a backstop against a hang, not a budget |
| Assertions | unchanged: the same line, the same `ServiceAuthorization` class, the same non-zero exit, the same platform/broker/compensation-worker effects |
| Proven by | a child whose prologue is artificially 45 s: **red before the change (49.25 s), green after (49.84 s)**; a child that never reports: **fails at 300.27 s with "timed out with the child still running"** |
| Also changed | three more waits of the same shape (a line that arrives after the prologue, budgeted from `spawn`) in the same file — §5 |
| Left alone | 27 other waits in the same file: eight `Remote: Ready` (a readiness budget, and the prologue is what is being waited for), eighteen that already start after it, and one answer-file poll with a 17× margin — §5 |
| Still possible | a runner slower than ~11× the observed prologue reports a timeout rather than a false pass — §6 |

## 1. The failure, as the runner recorded it

`gh run view --job 121392429655 --log` — the whole transcript the test printed when it
panicked is six lines, and the timestamps in it are the evidence:

| time (UTC) | what the child printed |
|---|---|
| 04:15:19.704 | the test target started (`Running tests\remote_start_refusal.rs`) |
| 04:15:21.249458 | `FutureOS: host starting version=0.0.0-e4505c9+local pid=2296 mode=auto log=…\desktop.log` |
| 04:15:21.255 | `startup progress is logged to … and mirrored in this terminal.` / `FutureOS host - foreground mode.` |
| — | **26.165 s with nothing on stdout or stderr** |
| 04:15:47.414666 | `INFO Agent startup: loading the model catalogue and credentials` |
| 04:15:47.491584 | `future-agent: no model configured yet — starting the gRPC server anyway …` |
| 04:15:47.493241 | `INFO session storage: opening database=…\.future\agent\agent.db` |
| 04:15:51.25 | the test's own 30 s window (`spawn` + 30 s) closes here |
| 04:16:40.099 | `panicked at cli\tests\remote_start_refusal.rs:855:5: the refused start must be reported by name; transcript:` followed by exactly those lines |

Two things follow, and they are the whole diagnosis:

1. **The 30 s was spent before the remote supervisor existed.** The transcript contains
   no `Remote: …` line at all. Those lines come from the headless entry's status poll,
   which runs after the embedded agent is up; without one, the entry had not reached
   the code that fetches a credential and connects to the broker. The broker therefore
   never saw the refused connection — the event the test waits for was never attempted.
2. **The window closed 3.8 s after the last mirrored stage, inside the prologue.** The
   agent's startup is the stage the 26.2 s gap belongs to: it is the first thing after
   the host entry prints its banner, and it is what the following lines report about.
   The prologue's true end is not in the log (the window closed first), so 26.2 s is a
   lower bound on it.

What I cannot see: the runner's `desktop.log` (its `…\Temp\fos-refused-7568` directory
went away with the ephemeral runner) would say which step inside the prologue the time
went to. The mirrored lines are enough for the claim that matters — the window closed
inside the prologue, before the refused connection was attempted — but not for a
breakdown of the 26.2 s, and this record does not claim one.

The masking event: **attempt 2 of the same run, same commit, same test binary, passed**
(job 121397859138 reran the leg in 8m56s against attempt 1's 10m36s). That is a re-run of
a load condition, not evidence about the product, which is why the rerun did not count as
a fix.

## 2. Why the window was measuring the runner

The test's intent is "a broker that refuses the credential is **reported**". Its wait was
`wait_for("Remote connection stopped", Duration::from_secs(30))` measured from `spawn`,
so the window covered three things that are not the claim:

- the embedded agent's startup (26.2 s above, on this runner),
- the first credential fetch from the platform,
- the broker's refusal and the entry's report.

Only the last is the product behaviour under test. A window that also contains an
unbounded prologue fails when the *host* is slow, which is exactly what happened: the
same code passed on the same commit minutes before and minutes after.

## 3. The change

`ChildRun::await_line(needle, cap) -> Verdict` replaces the fixed window for waits whose
claim is *that* a line appears:

| terminal condition | verdict |
|---|---|
| the line is in the transcript | `Reported` |
| the child ended (and its settled transcript has no such line) | `EndedWithoutReport` |
| `cap` expired with the child alive | `TimedOut` |

`TERMINAL_CAP` (300 s) is the backstop only, and the doc comment on it says so, with the
measurement this record is built on. `Verdict::describe()` puts the outcome in the panic
message, because "still starting at 300 s" and "ended without saying it" are different
findings that a bare `false` used to erase.

In the refusing-broker test the sequence is now:

```rust
let verdict = child.await_line("Remote connection stopped", TERMINAL_CAP);
let exit = match verdict {
    Verdict::Reported => child.wait_for_exit(TERMINAL_CAP),
    Verdict::EndedWithoutReport | Verdict::TimedOut => {
        let _ = child.child.kill();
        child.child.wait().ok()
    }
};
```

The `kill` branch is what makes a hung child fail *as a timeout* instead of leaving the
test blocked in `wait()` forever — the old code's `child.child.wait()` had no bound at
all. Every assertion below it is unchanged, and one got strictly stronger: the run must
now end within a bound instead of eventually.

## 4. The two falsification experiments

Both use a temporary patch (not committed, not in the diff) that makes the spawned child
sleep before it is `exec`'d, so the prologue is as slow as a loaded runner's while the
pid, pipes and signal target stay the child's:

```rust
let mut command = if let Ok(delay) = std::env::var("FUTURE_FIXTURE_PROLOGUE_DELAY") {
    let mut command = Command::new("sh");
    command.arg("-c").arg(format!(
        "sleep {delay}; exec \"{}\" host",
        env!("CARGO_BIN_EXE_future")
    ));
    command
} else { … };
```

### 4.1 A prologue far beyond the old 30 s window

Same slow child (45 s), the pre-fix file, then the post-fix file:

```bash
FUTURE_FIXTURE_PROLOGUE_DELAY=45 cargo test -p future-cli --test remote_start_refusal \
    a_broker_that_refuses_the_credential_is_reported -- --exact
```

| version of the test | result | wall clock |
|---|---|---|
| pre-fix (`git show HEAD:…`) | **FAILED** — `the refused start must be reported by name; transcript:` | 49.25 s |
| post-fix (this change) | **ok** | 49.84 s |

The same child, the same product, only the wait differs: the old window ran out at 30 s
while the child was still in its prologue; the new wait sees the report at ~47 s and
passes.

The same experiment on the second changed site, which had the same shape at 30 s:

```bash
FUTURE_FIXTURE_PROLOGUE_DELAY=45 cargo test -p future-cli --test remote_start_refusal \
    a_retry_that_cannot_be_classified_ends_the_worker_and_rearms_it -- --exact
```

| version | result | wall clock |
|---|---|---|
| pre-fix | **FAILED** — `the first start must report the retryable failure; transcript:` | 30.01 s |
| post-fix | **ok** | 53.82 s |

### 4.2 A child that never reports at all

```bash
FUTURE_FIXTURE_PROLOGUE_DELAY=400 cargo test -p future-cli --test remote_start_refusal \
    a_broker_that_refuses_the_credential_is_reported -- --exact
```

```
test a_broker_that_refuses_the_credential_is_reported has been running for over 60 seconds
thread 'a_broker_that_refuses_the_credential_is_reported' (256611630) panicked at cli/tests/remote_start_refusal.rs:954:5:
assertion `left == right` failed: the refused start must be reported by name (timed out with the child still running); transcript:
test result: FAILED. 0 passed; 1 failed; 0 measured; 13 filtered out; finished in 300.27s
```

So the backstop is real: 300.27 s, exit code 101, the reason named — not a hang, and not
a pass. (The child is killed by the timeout branch, so the fixture's `Drop` has nothing
left to reap and the test binary exits normally.)

## 5. The same-class survey, and why only four sites changed

"Same class" here means one specific mistake: **a window that starts at `spawn` and is
sized for the event, so it also contains the child's prologue.** The prologue is unbounded
host work; the event after it is bounded product work.

### 5.1 Changed (4)

| test | line waited for | before | after |
|---|---|---|---|
| `a_broker_that_refuses_the_credential_is_reported` | `Remote connection stopped` | 30 s | outcome + 300 s cap |
| `a_retry_that_cannot_be_classified_ends_the_worker_and_rearms_it` | `Remote: Reconnecting` | 30 s | outcome + 300 s cap |
| `a_reconnect_refreshes_the_credential_and_rebuilds_the_slot` | `rebuilding generation (1/3)` | 40 s | outcome + 300 s cap |
| `a_reconnect_whose_rebuild_cannot_connect_is_retryable` | `rebuilding generation (1/3)` | 40 s | outcome + 300 s cap |

The first is the observed failure. The other three are the same sentence in the same
file — a line that cannot exist before the child's first start attempt, budgeted from
`spawn` at 30–40 s, i.e. 4–14 s of margin over the 26.2 s prologue measured above — and
two of them are demonstrated red-then-green with the same experiment (§4.1). Changing one
of the two identical `rebuilding generation (1/3)` waits and not the other would have been
arbitrary.

### 5.2 Left alone, with the reason (27 waits in the same file)

| group | waits | why untouched |
|---|---|---|
| `Remote: Ready`, `READY_BUDGET` = 90 s | 8 | the prologue is *what is being waited for*, and 90 s is a declared readiness budget with its own recorded rationale (`INCIDENTS F.105`). At 3.4× the observed prologue it has margin the flake's 30 s never had; and a failure here means "the child never came up", which is a different claim from the one this record fixes |
| downstream of `Remote: Ready` or of an explicit bridge/book poll | 18 | the prologue is already excluded from these windows, so their durations price only product latency (`critical task stopped` 60/90 s, `automatic bridge reconnect failed` 90 s, `[AU001]` 20 s, `invalid_remote_credential` 40 s, `server failure episode started` 10/25 s, `wait_for_exit` 30 s, the answer/book file polls at 20/30 s, `rebuilding generation (3/3)` 40 s) |
| the `no answer within 60s` poll in `an_unreadable_revoke_queue_is_reported_and_the_refusal_still_stands` | 1 | same shape (from `spawn`), but 60 s against a 26.2 s prologue leaves ~34 s for ~2 s of work — a 17× margin, not a 1.2× one. Widening it to the 300 s backstop would turn a genuine "the request was never answered" failure into a five-minute wait for no evidence it is needed |

### 5.3 Other files in `cli/tests/` and `tests/`

Surveyed for the same "spawn a child, then bound the event" shape; none is the same
source, so none was touched:

| file | fixed wait | why it is a different class |
|---|---|---|
| `cli/tests/headless_cli.rs` | `wait_for(…, 300 s)` for `Not signed in to FutureOS`, `Remote: Stopped`, `Remote: Reconnecting`, `Platform account signed in` | already the shape this record lands on — a wide backstop for a line that must appear. `wait_for_exit`/`wait_for_signal_exit` bound a child that is already up |
| `cli/tests/headless_cli.rs` | 15 s to the agent's ready marker, 8 s to an exit | in-process supervision of an already-started child; not a prologue-and-event window |
| `cli/tests/bin.rs` | 30 s for the config wizard to exit, 10 s misc | no embedded agent and no broker in that child; the 30 s is two orders of magnitude over its work |
| `cli/tests/relay_nats_auth.rs` | 15 s for `nats-server` to listen | an external binary binding a port, not our entry point's prologue |
| `cli/tests/remote_unpair_prompt.rs` | `BUDGET` = 60 s for `[y/N]` | a CLI prompt under a pty, no agent start; 60 s for a prompt read |
| `tests/remote-interop/tests/*.rs` | `tokio::time::timeout(…, 5 s)` | in-process async calls over a scripted socket — no spawned child, no prologue |

## 6. What can still misreport, and what this does not cover

- **A runner slower than ~11× the observed prologue** turns the new wait into a reported
  timeout at 300 s. That is a *failure*, not a false pass, and it prints where the child
  had got to — but it is still a red CI leg for a slow host. The alternative (a large
  fixed number) has the same property with less information, so the cap stays a backstop
  and not a budget: if this fires, the number to revisit is `TERMINAL_CAP`, next to the
  measurement in its doc comment.
- **Only this binary's waits were re-measured.** The 26.2 s prologue is from one Windows
  run on 2026-10-26; nothing here measures the distribution of prologue times, and a
  lane whose prologue is much slower than `READY_BUDGET` (90 s) would still fail the eight
  `Remote: Ready` waits — by design, since that is "the child never came up".
- **The Windows runner is not reproduced locally.** The experiments substitute a sleep
  for a slow prologue; they prove the *wait* is indifferent to how long the prologue
  takes, not that this particular Windows host is fast enough for the rest of the file.
- **Two waits still contain the prologue as an implicit part of their claim** — the eight
  `Remote: Ready` waits — and are deliberately left as readiness budgets (§5.2).
- **Not verified in this change:** whether the embedded agent's prologue can be shortened
  on Windows (the 26.2 s itself). The report is about not measuring it in a product
  assertion, not about making it faster.
