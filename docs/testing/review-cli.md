# REV-CLI — adversarial review of the `future-cli` waiver ledgers

Scope: `docs/testing/module-cli.md` §4c (119 in-scope residual lines at its own
measurement S9) and `docs/testing/module-cli-mcp.md` §6 (4 rows). Reviewer unit
`REV-CLI`, dispatched 2026-10-17 00:52 (`e8b37fe1`). **Write set: this file
only** — the module records and the sources were not touched.

Reviewed by: session `20261017-005103-6aaf62066274403981a93784624eaa0b`, model
`future/deepseek-flash`, host macOS aarch64 (the campaign's own host), pinned
toolchain `1.97.0-aarch64-apple-darwin`.

## 0. Verdict

**Of the 33 rows / 119 lines in §4c: 10 rows / 76 lines FELL, 23 rows / 43 lines
SURVIVED.** Of the 76 fallen lines, **14 were measured 0 → 1 by a probe in the
scratch tree** (5 rows); the other 62 fall on the ledger's own rule 5a — the
stated *mechanism* does not support the category it claims — with the reaching
input named per row (§3.2). No row of §4c was left unaccounted for:
`reconcile.py` over this document (S9 report, `--section "0. Verdict"`) prints
`mentioned: 123 / MISSING: 0` and no STALE (§9.4) — the verdict table's rows are
ledger rows, so that run exercises the STALE half as well as the MISSING one.

Of `module-cli-mcp.md`'s 4 rows: **3 SURVIVED** (each with its neighbour counts
re-checked) and **1 (`mcp_client.rs:385`) is stale-paired** — the construct the
row names no longer exists at `HEAD`, which rewrote it and moved the residual to
`mcp_client.rs:406` (§6.2).

| row (S9 numbering) | lines | registered as | verdict | basis |
|---|---|---|---|---|
| `browser/chromium/cdp_connection.rs:229` | 1 | `unreachable-in-this-environment` | **SURVIVED** (reading sharpened) | reached in 6 of the 18 cli reports on disk, 0 in 12 — the ±1 of §6.11; the category's own text admits "a race window with no injection seam" (§4.1) |
| `browser/chromium/cdp_transport.rs:255` | 1 | `attribution-artifact` | **SURVIVED** | `}` of the wait `if let`; neighbours 253/254 = 1/1 (§4.2) |
| `browser/chromium/chromium_manager.rs:244,248` | 2 | `attribution-artifact` | **SURVIVED** | block ends; 239–243 and 245–247 covered (§4.2) |
| `browser/chromium/chromium_manager.rs:251,252,255` | 3 | `platform-unmeasured` | **SURVIVED** (condition narrowed) | macOS discovery is a fixed list with no env seam and the deterministic miss is Windows-only; the row must name the *host state* (no browser found), not "the Linux CI run" (§4.3) |
| `browser/chromium/socket_http.rs:233` | 1 | `unreachable-by-construction` | **FELL** (category) | the failure left is `accept()`'s `Err` (EMFILE/ENFILE) — the campaign measured that exact shape 0 → 1 in `channels/transport/webhook.rs` (review-rust §3.5) (§3.2.5) |
| `browser/safari/safari_manager.rs:209` | 1 | `unreachable-in-this-environment` | **SURVIVED** | the outer 2 s timeout's `Err(_)` arm: 0 in **all 18** cli reports on disk, while the inner-timeout arm is covered (§7) |
| `browser/safari/safari_manager.rs:820` | 1 | `unreachable-by-construction` | **SURVIVED** | test-side `_ => None` for totality; `safari_start` constructs only `Webdriver` connections (§4.4) |
| `browser/discovery.rs:180` | 1 | `attribution-artifact` | **SURVIVED** | block end; 179 = 1, 181 = 1 (§4.2) |
| `commands/auth.rs:89` | 1 | `unreachable-by-construction` | **FELL (measured 0 → 1)** | `future auth login --url http://127.0.0.1:1` through the shipped binary (§3.1.1) |
| `commands/auth.rs:465-488` | 24 | `unreachable-by-construction` | **FELL** (category) | the row itself says "reachable only from the shipped binary with a live agent" — a *reaching input*, so not "no input reaches it"; 484–486 need no agent at all (§3.2.2) |
| `commands/browser_tools.rs:232` | 1 | `attribution-artifact` | **SURVIVED at S9; citation stale at HEAD** | S9-era text = the row's `}`; HEAD's line 232 is a match arm (§6.1) |
| `commands/browser_tools.rs:302,303,310,316,317,318,320` | 7 | `attribution-artifact` | **SURVIVED at S9; citation stale at HEAD** | S9-era text = brace/comment/attribute lines of the `#[cfg(not(windows))]` block, 304–309/311–315 covered (§6.1) |
| `commands/browser_tools.rs:319` | 1 | `unreachable-by-construction` | **FELL** (category) | `#[cfg(not(test))] drop(child)` is the non-test twin of a *covered* block; the reaching route is the shipped binary's `browser start` (§3.2.4) |
| `commands/browser_tools.rs:536` | 1 | `unreachable-by-construction` | **SURVIVED at S9; citation stale at HEAD** | second `parse`'s `_` arm; `EndpointTarget::parse` is a pure string function, so the guard's proof transfers (§4.5) |
| `commands/configure.rs:45,46,49,50,51,52,53` | 7 | `unreachable-in-this-environment` | **FELL (measured 0 → 16/16/1/1/1/1/1)** | an existing integration test already drives `future config`; piped stdin answers 45/46 and a pty answers 49–53 (§3.1.2) |
| `commands/configure.rs:636,765` | 2 | `attribution-artifact` | **SURVIVED** | the `}` of the two `if let Ok(number) = value.parse::<…>()`; 632–635/637 and 761–764/766 covered (§4.2) |
| `commands/configure.rs:820,822-833,835,837-846,848,850` | 26 | `unreachable-by-construction` | **FELL** (category) | `provider_rpc` is reached from the agent-up branch the same row calls "reachable … against a real console" — the mechanism contradicts the category; needs a live agent, not an impossibility (§3.2.1) |
| `commands/configure.rs:855-862` | 8 | `unreachable-by-construction` | **FELL (measured 0 → 1 for 855,856,862)**; 857-861 still 0 | the `#[cfg(not(test))]` block of `save_custom_provider` runs in a non-test build with no agent (§3.1.2) |
| `commands/configure.rs:1022-1026` | 5 | `attribution-artifact` | **SURVIVED** | message arguments of a passing `assert!`; 1018–1021 covered (§4.2) |
| `commands/doctor.rs:140` | 1 | `unreachable-by-construction` | **SURVIVED** | `FutureAuthEntry` is built in exactly one place and that place filters the empty key (`auth.rs:530-534`) (§4.4) |
| `commands/doctor.rs:173,183,186-189` | 6 | `platform-unmeasured` | **SURVIVED** (shape (b)) | macOS arm: `available = /usr/bin/sandbox-exec exists` = true, `version: None` → no fixture here; the Linux probe arm is what carries path/version/availability (§4.3) |
| `commands/doctor.rs:197,198,199,200,201` | 5 | `unreachable-in-this-environment` | **FELL** (category) | the macOS arm of `platform_sandbox_probe_product()` cannot fail at all; the only `?` is the Windows one — the ledger's own refined definition and its `settings.rs` precedent put this in `platform-unmeasured` (§3.2.3) |
| `commands/session.rs:1336` | 1 | `unreachable-by-construction` | **SURVIVED** | the 13 `applied.push((` keys enumerated against the 13 named arms (§4.4) |
| `commands/session_status.rs:84` | 1 | `unreachable-by-construction` | **SURVIVED** | `flag.starts_with("--")` returns first, and every string equal to `--` starts with it (§4.4) |
| `commands/session_transcript.rs:279` | 1 | `unreachable-by-construction` | **SURVIVED** | the `matches!` guard admits exactly the 8 flags the `match` enumerates (§4.5) |
| `commands/session_transcript.rs:337,560` | 2 | `attribution-artifact` | **SURVIVED** | 334–336 = 13/10/10, 557–559 = 9/9/9 (§4.2) |
| `commands/session_transcript.rs:853` | 1 | `unreachable-by-construction` | **SURVIVED** | `allowed` comes from the first `match`, whose `_` arm is `(false, None)` → `continue`; the second `match` enumerates the same four kinds (§4.5) |
| `commands/settings.rs:180` | 1 | `attribution-artifact` | **SURVIVED** | 178–179 = 6/6 (§4.2) |
| `commands/skills.rs:564` | 1 | `attribution-artifact` | **SURVIVED** | 563 = 1, 565 = 4 (§4.2) |
| `commands/tools.rs:891` | 1 | `attribution-artifact` | **SURVIVED** | 890 = 9, 892 = 8 (§4.2) |
| `commands/workspace.rs:215` | 1 | `attribution-artifact` | **SURVIVED** | 213–214 = 12/12; the row's §6.8 correction (the body *is* the missing-parent fixture) is right (§4.2) |
| `main.rs:31` | 1 | `unreachable-in-this-environment` | **FELL (measured 0 → 1)** | `future --linux-sandbox-helper probe` exits 1 with the agent's own refusal message (§3.1.3) |
| `version.rs:96,99` | 2 | `unreachable-in-this-environment` | **FELL (measured 0 → 1)** | a `.git`-less tree is a fixture this machine provides, and it is the fixture the row itself names (§3.1.4) |

Peer rows (`module-cli-mcp.md` §6, reviewed here because §4c counts them):

| row | lines | registered as | verdict | basis |
|---|---|---|---|---|
| `commands/mcp_servers.rs:85` | 1 | `attribution-artifact` | **SURVIVED** | 83/84 = 9/9, 86 = 8 (§5) |
| `commands/mcp_servers.rs:342` | 1 | `unreachable-by-construction` | **SURVIVED** | `targets` ⊆ `config.servers` keys, verified at 322–343 (§5) |
| `commands/mcp_servers.rs:412` | 1 | `attribution-artifact` | **SURVIVED** | 407–411 = 3/3/7/7/7 (§5) |
| `commands/mcp_client.rs:385` | 1 | `attribution-artifact` | **UNVERIFIABLE AS CITED** (stale pairing) | the record pins md5 `6f2a1ba6…`, where 385 *is* the `if let`'s `}`; `08108be6` rewrote that block to `.map()` — the residual is now `:406` (§6.2) |

## 1. The artefact, the pairing, and the 123/122 question

REVIEW.md rule 1 says to re-read the line numbers from the record at review time,
so the first thing established is *which* report the row set belongs to.

```
$ python3 .future/cov100/covlines.py total /tmp/cli-r2s3-final3-lcov.info \
      --under .worktrees/cov100-r2/cli/src
covered=39970 measured=40093 uncovered=123 lines%=99.6932          # exit 0
$ python3 .future/cov100/covlines.py total /tmp/cli-r2s3-final4-lcov.info \
      --under .worktrees/cov100-r2/cli/src
covered=39971 measured=40093 uncovered=122 lines%=99.6957          # exit 0
$ python3 .future/cov100/covlines.py detail /tmp/cli-r2s3-final3-lcov.info \
      --under .worktrees/cov100-r2/cli/src | wc -l                 # 21 files
```

**Which reading the verdicts are against.** Every row verdict in §0 is against
**S9** (`/tmp/cli-r2s3-final3-lcov.info`, `uncovered=123`, of which **119 in
scope** — the four peer-owned MCP lines are the fifth record's). That is the
report §4c names for its line numbers, and it is the one I verified row by row:
`reconcile.py --section "4c. Ledger after the mock-span pass"` against S9 prints
`files with uncovered lines=21 uncovered lines=123 / mentioned: 123 / MISSING: 0`
with no STALE block at all. **S10** (`…-final4-lcov.info`, `122` / `118`) is the
second measurement of the same suite, and it differs in exactly one line —
`cdp_connection.rs:229`, which S10 shows as *covered*. I re-read the ledger's own
description of the `browser_tools.rs` rows against the S9-era source it must have
been written from (`ce1fd8a0`, 4347 lines, whose last instrumented line is 4344 —
the report's own maximum for that file), and all of them match at S9's numbers;
at `HEAD` they do not (§6.1). So: S9 for the row inventory, S10 as the ±1, and
§4.1 for what the ±1 means.

**The peer record's pairing.** `module-cli-mcp.md` pins the md5 of both files:
`mcp_servers.rs` = `05c5e3ccb3d12f7caf555a0a24168218` — **still exact** at HEAD;
`mcp_client.rs` = `6f2a1ba6e217a2ab2038d6ea4f6c4cb6` = `fd018191` — **no longer
HEAD's bytes** (`005636fdb91e23d1d0e9292359e191ad`), because `08108be6` changed
23 lines of it (§6.2).

## 2. Method

Everything below was produced in a scratch tree under `/tmp/rev-cli-s1` (rule 5c:
a fresh directory per segment; `/tmp/rev-cli` was not reused). **The campaign
worktree was written to only for this report**: the scratch tree's `cli/src` is
byte-identical to the campaign tree's (`diff -r` over `cli/src` → no output), and
the only file that differs is the probe test added under `cli/tests/`.

```
mkdir -p /tmp/rev-cli-s1/tree
git -C .worktrees/cov100-r2 archive fc964fb6 | tar -x -C /tmp/rev-cli-s1/tree   # no .git: see 3.1.4
# probe added: /tmp/rev-cli-s1/tree/cli/tests/rev_probe_cli.rs  (embedded in §9.1)
# a 30-line DA reader: /tmp/rev-cli-s1/counts.py                (embedded in §9.2)

# Run A — the whole suite at the frozen revision, in the .git-less tree (the
# version.rs probe is the tree itself; everything else is the control)
CARGO_TARGET_DIR=/tmp/rev-cli-s1/target HOME=/tmp/rev-cli-s1/home \
  cargo llvm-cov -p future-cli --no-fail-fast --lcov \
  --output-path /tmp/rev-cli-s1/runA-lcov.info -j 4
# Run B — the probe target alone: the line counts for every measured fall
  … --lcov --output-path /tmp/rev-cli-s1/runB-lcov.info -j 4 \
      --test rev_probe_cli -- --nocapture
# Run C — one test alone, to separate a load flake from a real failure (§6.3)
  … --lcov --output-path /tmp/rev-cli-s1/runC-lcov.info -j 4 --lib -- \
      --exact browser::safari::safari_manager::tests::start_launch_success_against_fake_driver
```

Two things this buys that a single-run review does not: Run A is the **same
revision without the probe**, so every measured fall has a same-revision 0 to go
with the probe's 1; and Run B is *filtered to the probe target*, so the counts it
reports for the probe lines are the probe's own and nothing else's (the technique
`review-rust.md` §2 used with `-- rev_probe`).

## 3. Falls

### 3.1 Measured falls (5 rows, 14 lines)

The "before" column is Run A (full suite, same revision, probe file not built —
`grep -c rev_probe runA.log` → `0`). The "after" column is Run B. Every number is
a `DA:` count from the named report.

| line | before (Run A) | after (Run B) | what the probe did |
|---|---|---|---|
| `main.rs:31` | 0 | **1** | `future --linux-sandbox-helper probe` |
| `auth.rs:89` | 0 | **1** | `future auth login --url http://127.0.0.1:1` |
| `configure.rs:45` | 0 | **16** | `future config` under a pty, wizard answers piped |
| `configure.rs:46` | 0 | **16** | same |
| `configure.rs:49,50,51,52,53` | 0,0,0,0,0 | **1,1,1,1,1** | same (the API-key prompt) |
| `configure.rs:855` | 0 | **1** | same (`save_custom_provider`, non-test build, no agent) |
| `configure.rs:856` | 0 | **1** | same |
| `configure.rs:862` | 0 | **1** | same |
| `version.rs:96` | 0 (Run A… see 3.1.4) | **1** | the `.git`-less tree is the fixture |

#### 3.1.1 `commands/auth.rs:89` — FELL

The row: *"`#[cfg(not(test))] let auth_data = Value::Null;` — the test build takes
the other arm (`load_auth_file`), so this line is not in the test translation
unit"*, category `unreachable-by-construction`. The mechanism is a claim about
*unit* tests; the category claims no input reaches the line. Line 89 is the first
statement of `login()` in every non-test build, and the suite already contains the
invocation: `cli/src/lib.rs:653 auth_login_url_parsing_reaches_login` drives the
same argv in-process (where line 87 is compiled instead), and
`cli/tests/bin.rs` spawns the real binary. The probe drives it through the shipped
binary:

```
REV-PROBE login code=Some(1) stdout="" stderr="Network error: error sending request for url
  (http://127.0.0.1:1/client/v1/oauth/device/code)\n"
```

Reaching input: the shipped binary with `auth login --url <unreachable>`. The fix
is a test, not a waiver — it can live in `cli/tests/bin.rs` next to
`config_without_a_tty_reports_closed_input`.

#### 3.1.2 `commands/configure.rs:45,46,49,50,51,52,53` and `:855-862` — FELL

`45/46` are `StdioPrompter::read_line`'s closing brace and success arm; `49-53` is
`read_secret` (`rpassword`); `855-862` is `save_custom_provider`'s
`#[cfg(not(test))]` agent-RPC block. The row's mechanism — *"read_line's success
arm reads the process stdin and read_secret opens the console device; a test
process has neither"* — is contradicted by the record's **own** §4 row, which says
of line 46: *"answerable in principle by an integration test that pipes stdin at
the wizard (piped stdin answers `read_line`), which no test does yet"*. The suite
also already has the harness: `cli/tests/bin.rs:80
config_without_a_tty_reports_closed_input` spawns `future config` with
`Stdio::null()` and its comment names *this* arm ("the `StdioPrompter::read_line`
EOF arm") — it covers 42–44 (S9: `43 → 1`, `44 → 1`).

Probe: spawn `future config` from the test with the wizard's answers on stdin,
under `script -q /dev/null` so the child has a pty (its stdin is the pty slave and
`/dev/tty` resolves for `rpassword`). Full child output (Run B, `--nocapture`):

```
Select provider [1]: Provider ID [custom]: Provider display name [revprobe]: API protocol: …
Select protocol [1]: Base URL: API key (leave blank for a keyless local endpoint): 
Add a model? [Y/n]: y … Model ID (sent to the API): … Prices per 1M tokens in CNY …
Add another model? [y/N]: Configured custom provider 'revprobe' with 1 model(s): revprobe/rev-model.
Model configuration: /var/folders/…/rev-probe-cli-53682/.future/agent/models.json
```

Two probe-authoring notes worth keeping, because both cost an iteration: the
command is **`future config`**, not `configure` (`lib.rs:93`); and `script(1)`
forwards its own stdin EOF to the pty as an **EOT**, which the line discipline
turns into `read() == 0` — a wizard fed through a pty reports *"Input closed
before configuration completed."* at its first prompt unless the caller's pipe is
kept open until the child exits (§9.1 carries the flag that does it).

`857-861` (the agent-up branch, and therefore `provider_rpc` at 820ff) stayed 0 in
the probe: that half needs a live agent. See §3.2.1 — the point is the category,
not the count.

#### 3.1.3 `main.rs:31` — FELL

The row: the hidden `--linux-sandbox-helper` re-exec, category
`unreachable-in-this-environment`, and §6.8b's own words: *"Reaching it needs a
test that spawns the unified `future` binary with the hidden flag, which cannot be
made meaningful (or non-hanging) off a Linux+bwrap host."*

The agent crate refutes the "cannot be made meaningful" half by itself:
`agent/src/cli.rs:213-218` bails with *"Linux sandbox helper is unavailable on
this platform"* off Linux, and `agent/src/cli.rs:752
the_linux_sandbox_helper_flag_is_rejected_off_linux` already asserts exactly that.
So the dispatch line runs and the process exits in milliseconds:

```
REV-PROBE helper code=Some(1) stdout="" stderr="Error: Linux sandbox helper is unavailable on this platform\n"
```

Reaching input: `future --linux-sandbox-helper probe` — no Linux, no bwrap, no
hang. It is a one-line addition to `cli/tests/bin.rs`, where the
"dispatch before the CLI runtime" family already lives.

#### 3.1.4 `version.rs:96,99` — FELL

`GIT_COMMIT`/`GIT_COMMIT_SHORT` come from `env!()` values that `cli/build.rs`
injects from `git rev-parse` (`"unknown"` outside a checkout). The test
`the_commit_is_either_a_real_object_name_or_absent` selects its arm from that
value, and lines 96/99 are the `None =>` arm's two assertions. The row's category
is `unreachable-in-this-environment`, whose definition requires a fixture **this
machine cannot provide** — yet the row's own mechanism names the fixture ("needs
a source tree without `.git`"), and that tree is what `git archive | tar -x`
produces.

Run A *is* that tree; the two directions are both measured (all three columns from
`runA-lcov.info`, the control from S9's report of the campaign tree):

| line | meaning | `.git`-less tree (Run A) | checkout with `.git` (S9) |
|---|---|---|---|
| `version.rs:81,88,93` | the `Some(commit)` arm | 0, 0, 0 | 1, 1, 1 |
| `version.rs:96,99` | the `None =>` arm | **1, 1** | 0, 0 |

The arm is selected purely by the presence of a checkout, and the row's fixture
flips it. (The same table is why Run A's `cli/src` residual cannot be read as a
like-for-like replacement for S9's — §6.3.)

### 3.2 Category falls (mechanism weaker than the category — rule 5a)

These rows are genuinely 0 in every measurement I have; what falls is the
*category*. Each of them claims `unreachable-by-construction`, i.e. "**no input
reaches it** (needs the invariant)", and each states a mechanism that is about
something narrower: a translation unit, or a route the row itself concedes is
walkable.

#### 3.2.1 `commands/configure.rs:820,822-833,835,837-846,848,850` (26 lines) — FELL

The row: *"`#[cfg(not(test))] fn provider_rpc` — in no `cfg(test)` translation
unit; reachable only from the shipped binary."* Its §4 ancestor says more:
*"The `future` binary's copy is reachable only by an interactive `future configure`
against a real console."* A statement that names a route by which the line *does*
run is not "no input reaches it". The concrete input, on this host: the unified
binary, an isolated home, and a scripted wizard — plus a live agent, because
`provider_rpc` is called only from the `probe_agent().is_ok()` branch
(`configure.rs:857-861`, measured 0 here because the probe's home has no agent).
`future agent --home <dir>` is the documented way to give the CLI its own socket
under the same `FUTURE_HOME` (AGENTS.md), so the fixture exists in-repo. Honest
category: `unreachable-in-this-environment` (a live agent + a scripted wizard), or
an OPEN row with that fixture named — not `unreachable-by-construction`.

#### 3.2.2 `commands/auth.rs:465-488` (24 lines) — FELL

The row: *"`#[cfg(not(test))] async fn save_auth` … Absent from every test binary;
reachable only from the shipped binary with a live agent."* Again the mechanism
concedes a route. It is also too strong in one direction: `save_auth`'s body
covers **both** the agent-up path (471–483) and the "agent not running" fallback
(484–486, the write to `auth.json`), and the fallback needs no agent at all — only
a login that gets as far as a token. The call site is `auth.rs:153`, inside
`login()`'s success branch; the two requests in front of it are plain POSTs to
`{platform_url}/client/v1/oauth/device/code` and `…/device/token`, which a local
mock HTTP server answers (the campaign's own fixture practice; the target's
`--url` flag exists precisely to point the flow somewhere else). So 484–486 are
reachable with the shipped binary plus a mock platform, and 471–483 add a live
agent. Reclassify (or open with the fixture named).

#### 3.2.3 `commands/doctor.rs:197-201` (5 lines) — FELL

The row: `check_sandbox`'s `Err` arm, category `unreachable-in-this-environment`,
mechanism: *"needs `platform_sandbox_probe_product()` to fail. The previous
campaign tried to force it (`FUTURE_HOME` at a temp dir whose
`windows-capabilities.json` is `{not json`) and the probe still answered
`available=true`."*

Read the function (`agent/src/sandbox/mod.rs:1054`): the **macOS arm has no
failure path at all** — it is `Ok(SandboxProbeResult { available: path.exists(),
… })`; the only `?` in the whole function is
`probe_windows_sandbox_product()?` in the `#[cfg(target_os = "windows")]` arm. So
on this host the arm is not a host-state that is hard to produce; it is *another
platform's code path*. That is exactly the case `docs/testing/waiver-ledger.md`
records being reclassified before: *"`agent/src/rpc/commands/settings.rs`'s eight
rows (the only `Err` producer is the Windows branch)"* → `platform-unmeasured`.
Correct category: `platform-unmeasured` shape (b), platform = Windows, condition =
the Windows sandbox probe returning `Err`. This is the same mistake the ledger's
history paragraph exists to prevent, one file over.

#### 3.2.4 `commands/browser_tools.rs:319` — FELL

The row: *"`#[cfg(not(test))] drop(child)` — not in the test translation unit"*,
category `unreachable-by-construction`. It sits in the same `#[cfg(not(windows))]`
block as 304–309, which the S9 report shows at **4 hits each** — i.e. the launch
block runs in the test build, and only the `drop(child)` half is compiled away
there. The reaching input is the non-test build of that same path: the shipped
binary's browser `start` (the tool is registered by `commands/tools.rs:1560`
`is_browser_tool`, so `future tools` reaches it). One line, and it is the twin
problem rule 9 of REVIEW.md describes rather than an unreachability claim.

#### 3.2.5 `browser/chromium/socket_http.rs:233` — FELL

The row (reclassified in §3.11, so it is this unit's own decision):
*"the test-side `serve_and_close` accept guard. The listener is bound in that
function and moved into the spawned task, so **no input of the fixture can close
it**; the only failures left are OS-level resource errors on `accept`
(EMFILE/ENFILE)."* A sentence that names `EMFILE` and then calls the arm
"no input" is the mechanism/category mismatch rule 5a names — and this campaign
has already *measured* the counterexample in the identical shape: review-rust §0
rows 11–13 and §3.5, `channels/transport/webhook.rs` 149–151, **0 → 1** with
"`RLIMIT_NOFILE` = current usage, fd table filled, one connection queued, in a
dedicated test process". A test-side `accept()` is no less reachable by fd
exhaustion than a production one. I did not rebuild that fixture here (it needs a
dedicated fd-exhausting process); the counterexample is the campaign's own
measurement, and the honest label is `unreachable-in-this-environment` at best.

## 4. Survived, with the reason re-established

### 4.1 `cdp_connection.rs:229` — SURVIVED, and the ±1 naming

Mechanism as measured, over every cli report still on disk (13 of them; this is a
cross-run table rather than a single reading, which is what §6.11's convention
needs):

| line | `cli-lcov` | `cli-final2` | `cli-final4` | `clamcp-after3` | `r2s3-final` | `r2s3-final4` | the other 12 reports |
|---|---|---|---|---|---|---|---|
| `cdp_connection.rs:229` | 1 | 1 | 2 | 1 | 1 | 1 | 0 |

So the line **is** reached — non-deterministically, by another thread's
`disconnect()` landing between the unlocked check (214, 953 hits) and the locked
one (228, 953 hits). The category's own definition admits this shape ("a race
window with no injection seam"), §6.11 documents the ±1 convention, and S9/S10 are
the two identical runs whose residual differs by exactly this line. What I would
change is only the *reading*: this row is not an unreachability claim to be
compared against a count of 0; it is a metric flake whose honest label is
"non-deterministically reached". The row's citation is valid at HEAD
(`cdp_connection.rs` unchanged since 18:42).

### 4.2 The `attribution-artifact` rows — all five stand, checked against neighbours

Rule: an `attribution-artifact` row needs its cited line to be 0 *while its
neighbours are covered* — a brace or macro region with no counter of its own.
Every one of the 25 such lines in §4c was checked against the S9 report; the five
`}`/macro rows are quoted here with their neighbours, and each holds:

```
cdp_transport.rs:253,254,255  = 1, 1, 0       (the wait `if let` whose body always breaks)
chromium_manager.rs:239-247   = 1,1,1,·,·,1,1,1,1   (both `if let` bodies execute) ; 244,248 = 0
discovery.rs:179,180,181      = 1, 0, 1
configure.rs:632-637          = 20,20,19,1,0,1 ; :761-766 = 29,29,27,2,0,2
configure.rs:1018-1026        = 1,1,1,·,0,0,0,0,0
session_transcript.rs:334-338 = 13,13,10,10,0,3 ; :557-561 = 9,9,9,0,14
settings.rs:178-181           = 6,6,0,6 ; skills.rs:563-565 = 1,0,4
tools.rs:890-892              = 9,0,8 ; workspace.rs:213-216 = 12,12,0,12
browser_tools.rs:229-233      = 16,4,4,0,12   (S9 text: the `}` at 232)
browser_tools.rs:302-320      = 304-309 all 4, 311-315 all >0, the brace/comment/attr lines 0
```

One hygiene note, not a fall: `configure.rs:1022-1026` is the *message-argument*
shape ("a passing assertion cannot evaluate them"). It is a legitimate
`attribution-artifact`, but the sibling unit chose the other resolution for the
identical shape — `module-cli-mcp.md` §5 records replacing such an assertion with
a `bool` and formatting nothing. Either is in policy; the campaign should not
leave two answers on the record.

### 4.3 The `platform-unmeasured` rows — shape named, condition narrowed

* `doctor.rs:173,183,186-189` (6) — **shape (b)**: the line compiles here and only
  another platform's runtime can execute it. Verified from the source: the macOS
  arm of `platform_sandbox_probe_product()` reports `available = /usr/bin/sandbox-exec
  exists` (= true on this host) and `version: None`, so `173` (the `Warn` branch),
  `183` (a version line) and `186-189` (`cfg!(target_os = "linux") && !probe.available`)
  cannot be reached here by any fixture; the Linux arm (`probe_linux_sandbox_host()`)
  is what carries an availability answer, a path and a version. Note `186-189` is
  compiled here with a `cfg!` constant that is statically false — under the
  refined definition that is still shape (b) (it *has* a `DA` record), not the
  shape (a) "not compiled here".
* `chromium_manager.rs:251,252,255` (3) — **shape (b)**, but the condition is
  asserted rather than evidenced. The mechanism's first half is verified: the
  macOS discovery list is fixed (no env seam) and the deterministic miss lives in
  the Windows-only `launcher_lookup_misses_when_the_roots_are_unset`. The second
  half — *"a bare container, i.e. the Linux CI image"* — is an assumption about
  the runner: the workflow installs no browser (`ci.yml` "Install Linux deps
  (base)" = build-essential/binutils/mold/pkg-config), but whether the
  `ubuntu-latest` image already carries Chrome is exactly the thing the row needs
  as evidence. Recommendation: state the condition as **"a Linux host whose
  candidate scan finds no launcher"** (and cite the CI log's `find_browser`
  answer if the Linux leg is to be called the authority), because "the Linux CI
  run" is only an authority if that run's answer is the miss.

### 4.4 The invariant rows — three spot-checked by enumeration

* `session.rs:1336` — all 13 `applied.push((` sites (1136, 1152, 1166, 1174,
  1180, 1203, 1211, 1227, 1242, 1248, 1254, 1260, 1266) resolve to the 13 named
  arms; the three multi-line pushes carry `"parent"`, `"systemPrompt"`,
  `"appendSystemPrompt"` (checked at 1136/1211/1227). Invalidating token: a 14th
  key pushed without a match arm.
* `doctor.rs:140` — `FutureAuthEntry` is constructed **once** (`auth.rs:528`) and
  that literal passes through `.filter(|key| !key.is_empty())` (`auth.rs:530-534`),
  so `entry.key` is never `Some("")`. Invalidating token: a second constructor, or
  deleting the filter.
* `safari_manager.rs:820` — inside `safari_start` the only
  `BrowserConnectionConfig::` constructions are `Webdriver` (158, 204); the `Cdp`
  one at 387 belongs to another test. The arm below the match requires the
  `Webdriver` variant. Invalidating token: any non-`Webdriver` return path from
  `safari_start`.

### 4.5 The dead-arm rows — the guard is the invariant

* `browser_tools.rs:536` (at S9) — the second `EndpointTarget::parse(&endpoint)`
  falls to `_` only if the endpoint is not a socket, "which the guard immediately
  above has already proved". `EndpointTarget::parse` (`browser/target.rs:47-74`) is
  a pure string function — trim, two prefixes, one scheme test — with no I/O and
  no env read, so the two calls on the same string cannot disagree. Invalidating
  token: making `parse` impure.
* `session_transcript.rs:279` — the `matches!` guard (183-197) admits exactly
  `--session/--select/--tool/--grep/--cursor/--limit/--max-bytes/--truncate`, and
  the `match` below enumerates exactly those eight. Invalidating token: adding a
  name to one list and not the other (the two lists are coupled only by
  convention).
* `session_transcript.rs:853` — `allowed` is produced by the first `match`
  (`740-788`, whose `_` arm is `(false, None)`), so it is true only for
  `text`/`reasoning`/`tool_call`/`tool_result`, which are precisely the second
  match's arms (`797-852`). Same coupling caveat.
* `session_status.rs:84` — `flag.starts_with("--")` returns at 74-78 before the
  duplicate check, and every string equal to `--` starts with `--`.

## 5. The peer record's 4 rows (`module-cli-mcp.md`)

```
$ python3 .future/cov100/covlines.py total /tmp/clamcp-final-lcov.info \
      --under .worktrees/cov100-r2/cli/src/commands/mcp_servers.rs \
      --under .worktrees/cov100-r2/cli/src/commands/mcp_client.rs
covered=2703 measured=2707 uncovered=4 lines%=99.8522            # exit 0
$ python3 .future/cov100/reconcile.py --report /tmp/clamcp-final-lcov.info \
      --scope …/mcp_servers.rs --scope …/mcp_client.rs \
      --doc docs/testing/module-cli-mcp.md --section "Waiver ledger — the 4 residual lines"
[cli-mcp] files with uncovered lines=2 uncovered lines=4
  mentioned in module-cli-mcp.md: 4
  MISSING (uncovered, not mentioned): 0            # no STALE
```

Neighbour counts (from `/tmp/clamcp-final-lcov.info`):

* `mcp_servers.rs:85` (`}` of `save_config`'s `if let Some(parent)`) — 83/84 = 9/9,
  86 = 8. **SURVIVED.**
* `mcp_servers.rs:342` (`continue;` in the `let … else`) — 341 = 9, 344 = 9, 342 = 0.
  The invariant is verifiable at 322-343: `targets` is either a single name
  `resolve(&config, name)?` has proved present, or `config.servers.keys()`; the
  same `config` binding is used in the loop. **SURVIVED.**
* `mcp_servers.rs:412` (`}` of `if let Some(error) = report["error"].as_str()`) —
  407-411 = 3/3/7/7/7. **SURVIVED.**
* `mcp_client.rs:385` — see §6.2.

The record's own §6.1 (four lines that used to be waived and are now executed) is
consistent with the report: those four numbers are gone from the residual, and the
poisoned-mutex arm really is executed (S9 agrees: the residual in that file is
only 385, and 382-384 are at 41).

## 6. Pairing defects and drift (rework for the owning unit, not tag falls)

### 6.1 §4c's citations for four files no longer resolve at HEAD

`module-cli.md` was last written at 20:56 on 2026-10-16; the CLI's other worker
(CLI-B) kept editing `cli/src` until 00:45 and a verification pass touched it
again, so the record's line numbers describe a revision the tree has left:

| file (S9 revision) | first insertion after S9 | effect on §4c's rows |
|---|---|---|
| `browser_tools.rs` (`ce1fd8a0`, 4347 lines) | `@@ -138,0 +139,48 @@` then `@@ -322,2 +370,16 @@` | 232 → 280, 302-320 → 350-368, 536 → 599. At HEAD line 232 is `"snapshot" => with_session(…)`; the `#[cfg(not(windows))]` block the row describes sits at 349-367 |
| `safari_manager.rs` (`ce1fd8a0`) | `@@ -42,0 +43,43 @@` | 209 → 252, 820 → 862 |
| `mcp_client.rs` (`fd018191`) | `@@ -62,0 +63,6 @@` and `@@ -382 +388 @@` | 385 → 391, and the construct was rewritten (§6.2) |
| `doctor.rs` (`1ac12a94`) | `@@ -206,7 +206,20 @@` (all drift is *after* 206) | **none** — 140, 173, 183, 186-189, 197-201 still name their constructs; I checked the S9-era text and HEAD's byte for byte |

The rows themselves are not wrong — at S9 they match the S9-era source exactly
(e.g. `ce1fd8a0:232` is the `}` closing `if let Ok(EndpointTarget::Socket(spec))`,
`ce1fd8a0:302-320` is the `#[cfg(not(windows))]` block, `ce1fd8a0:536` is the
second parse's `_` arm). What is owed is a re-measure of the four drifted files,
not a re-wording: at HEAD the construct at each cited number is something else
(tabulated above), and for two of them (`232`, `536`) the *new* content at that
number is code that any run covers — a reviewer comparing the two without the
revision would read them as stale rows.

### 6.2 `mcp_client.rs:385` — the row's construct was rewritten away

`module-cli-mcp.md` pins the md5 of the revision it measured; the file changed
after it (`08108be6`, 23 lines, and this block is exactly what it changed):

```diff
-        if let Some(stderr) = child.stderr.take() {
+        let stderr_drain = child.stderr.take().map(|stderr| {
             let tail = Arc::clone(&stderr_tail);
-            tokio::spawn(async move { drain_stderr(stderr, tail).await });
-        }
+            tokio::spawn(async move { drain_stderr(stderr, tail).await })
+        });
```

So the `if let` (and its implicit else) no longer exists; at HEAD lines 388-391
are covered (**41 hits** in Run A) and the file's single residual is now
`mcp_client.rs:406` — the `}` of the *new* `if let Some(drain) = stderr_drain`,
the same attribution-artifact shape one construct later. The old row is
unverifiable as cited and must be re-derived (it is very likely already resolved
by the rewrite, but the *new* line needs its own row — re-measure, do not
re-point). This is the same drift class as §6.1 and the reason REVIEW.md rule 1
exists.

### 6.3 What §4c's headline describes now

Measured for the record (Run A, HEAD = `fc964fb6`, scratch tree, probe file not
built):

```
$ python3 .future/cov100/covlines.py total /tmp/rev-cli-s1/runA-lcov.info \
      --under /private/tmp/rev-cli-s1/tree/cli/src
covered=40162 measured=40329 uncovered=167 lines%=99.5859        # exit 0
$ python3 .future/cov100/reconcile.py --report /tmp/rev-cli-s1/runA-lcov.info \
      --scope /private/tmp/rev-cli-s1/tree/cli/src \
      --doc docs/testing/module-cli.md --section "4c. Ledger after the mock-span pass"
[cli-head] files with uncovered lines=21 uncovered lines=167
  mentioned in module-cli.md: 113
  MISSING (uncovered, not mentioned): 54
     browser_tools.rs:280 350 351 358 364-368 382 · doctor.rs:325 357 1103 ·
     files.rs:161 · mcp_client.rs:406 · safari_manager.rs:196 267 361-367 · …
```

Two caveats, both of which make 167 an **upper bound** rather than a replacement
for 119, and neither of which explains the 54 MISSING lines away:

1. The scratch tree has no `.git`, so `version.rs`'s arm is flipped: `96/99`
   (2 lines) leave the residual and `82-85, 88-90, 93` (8 lines) enter it (§3.1.4).
2. Run A had **one test failure** — `browser::safari::safari_manager::tests::start_launch_success_against_fake_driver`
   panicked at `safari_manager.rs:929` ("safaridriver did not respond … within
   10s"), and its tail (933-945, 947, 949, 955-959, 963-966, 969) therefore never
   ran; that accounts for most of the `safari_manager.rs` additions. Run C is the
   separation: the **same test, alone, passes in 8.09 s** (`1 passed; 0 failed;
   1161 filtered out`) and in Run C's own report `safari_manager.rs:361-367`,
   `933`, `940`, `969` are all **1** — so those lines are the failing run's
   artifact, not the revision's residual (only `915/916`, braces inside the poll
   loop, stay 0). Two observations — 1 failure in 1 full-suite run under `-j 4`,
   0 in 1 solo run — are a *flake signal*, not a rate: `.future/cov100/flake-rate.sh`
   is the instrument, and the record's own §6.3/§7.2 already list this family as
   load-flaky. The honest reading is "a wall-clock assertion on a loaded host".

What neither caveat explains: **`cli/src/utils/files.rs:161` is a file §4c never
mentions**, and `doctor.rs` gained residuals at 253/325/357/1103 from CLI-B's own
new code. So the ledger as published is both **stale** (four files' citations) and
**incomplete** (≥54 uncovered lines unmentioned at HEAD): the 119/118 headline
describes S9/S10, and the record should say so explicitly in its header — it says
it in §6.11's "name the run" rule, but not next to the number.

### 6.4 The record's statement about its own mechanical self-check is now false

§4c ends: *"`MISSING = 0` and `STALE = 0` on both — every residual line is
accounted for … the STALE half is scoped to files that still have an uncovered
line (under S10, `cdp_connection.rs` has no uncovered line at all, so its row is
not compared there)."* That describes the pre-fix `reconcile.py`. After the
supervisor's fix (`5da793b6`, "the STALE half skipped fully-covered files"), the
same invocation reports the row:

```
$ python3 .future/cov100/reconcile.py --report /tmp/cli-r2s3-final4-lcov.info \
      --scope .worktrees/cov100-r2/cli/src --doc docs/testing/module-cli.md \
      --section "4c. Ledger after the mock-span pass" --label cli-s10
[cli-s10] files with uncovered lines=20 uncovered lines=122
  mentioned in module-cli.md: 122
  MISSING (uncovered, not mentioned): 0
  STALE (…): 1
    COVERED (DA>0 - a real disagreement, the actionable half): 1
       cdp_connection.rs:229
```

The row is **not** a defect — S10 measured 229 covered, which is the ±1 §6.11
describes, and the row is correct against S9 (against which I reviewed it, §4.1).
But the *sentence about the tool* must be updated: it promises a blindness the
tool no longer has, and a future reviewer running the documented command against
S10 will meet a STALE block the record says cannot appear. (The S9 run of the same
command prints no STALE block, so the record's S9 half stands.)

## 7. Cross-run tables (the stability evidence)

`cdp_connection.rs:229` and `safari_manager.rs:209`, over every cli report on
disk (`python3 /tmp/rev-cli-s1/xruns.py …`, §9.2 — **18** cli reports, S3 → S10 plus
the peer unit's runs):

| report | `cdp_connection.rs:229` | `safari_manager.rs:209` |
|---|---|---|
| `cli-lcov`, `cli-final2`, `clamcp-after3`, `cli-final4` | 1, 1, 1, 2 | 0, 0, 0, 0 |
| `cli-final`, `cli-r2s2`, `cli-r2s2-final`, `cli-r2s3` | 0, 0, 0, 0 | 0, 0, 0, 0 |
| `cli-r2s3-final`, `-final2`, `-final3` (S9), `-final4` (S10) | 1, 0, 0, 1 | 0, 0, 0, 0 |
| `clamcp-before`, `-after1`, `-after2`, `-after4`, `-after5`, `clamcp-final` | 0, 0, 0, 0, 0, 0 | 0 × 6 |

Reading: `229` is a flake (6 of 18 nonzero, opposite neighbour-counts whichever
way it lands → 0-vs-nonzero only, as §6.11 says); `209` is stable at 0 in 18 of
18, which is what upgrades its mechanism from an argument to evidence.

## 8. What I could not check

1. **A clean HEAD number.** Run A's `167` carries the two caveats of §6.3 — the
   `.git`-less arm flip and one failing test's tail. A fresh Linux/CI-equivalent
   measure, or one re-run with `.git` present and that test green, is the number
   the record should carry next.
2. **The Linux-side conclusions.** Every `platform-unmeasured` row's *condition*
   (a probe answering `available=false`, a probe reporting a version, a host whose
   browser scan misses) is only observable on another host. I verified the
   *mechanisms* from the source and named the shape; the conditions need the
   Linux leg's own output, per §7.3 of the record. In particular the
   `chromium_manager.rs` rows' "bare container" claim needs the runner image's
   browser inventory as evidence (§4.3).
3. **`socket_http.rs:233`'s 0 → 1.** I name the counterexample (fd exhaustion) and
   the campaign's measurement of the identical shape; I did not build the
   dedicated process. A reviewer who wants the number can reuse review-rust's
   `rev_probe_emfile` harness.
4. **`auth.rs:471-483` / `configure.rs:820ff` with a live agent.** Both need an
   agent on the isolated home's socket; my pty probe deliberately ran without one
   (which is why 855/856/862 flipped and 857-861 did not). The fixture is
   `future agent --home <dir>`, but wiring it into the probe is a second harness
   and I stopped at the category finding.
5. **The `#[cfg(not(test))]` family as a class — 60 of §4c's 68 `unreachable-by-construction`
   lines** (`provider_rpc` 26, `save_custom_provider`'s agent branch 8, `save_auth` 24,
   `auth.rs:89`, `browser_tools.rs:319`). I checked the three representative shapes (a
   `let` binding, a function, a `drop`) and each fell on the same reasoning; I did
   not enumerate every remaining line individually, and the record should decide
   the class-level rule once (the ledger's `unreachable-in-this-environment` / OPEN
   categories are the only honest ones for "compiled out of the test binary").

## 9. Probe inventory (sources, hashes, commands)

Scratch tree `/tmp/rev-cli-s1` (kept alive for the supervisor to hash or re-run).
Nothing here is in the repository.

| artefact | sha256 |
|---|---|
| `tree/cli/tests/rev_probe_cli.rs` (the probe; embedded in §9.1) | `b5c63cd4926f88621f550b4c7806e8b1d1719c96a043f6a0faa7575e7ab5e18e` |
| `runA-lcov.info` (full suite, `.git`-less tree, `fc964fb6`) | `0b7f5ffbd3ef575804bc50146917eb3d6d1ca3ca62a2820af78753415727ccbb` |
| `runB-lcov.info` (probe target only) | `d40b269c2c47df89abe309ce5a01ef86d6a0f31ed1dfbead0c3cc91d1e35ab2a` |
| `runC-lcov.info` (one test alone, flake separation) | `a12b7a428134f2c0d637f049ceb3b13ad3939ffb44a3d4650b9126ea55138e55` |
| `runA.sh`, `runB.sh`, `runC.sh` (the run scripts, embedded in §9.3) | `30fefb3edc8845215414a6485a3b4283861d632a4951f2247f59829647f41812`, `1975ce9fafcb9d4852822dc20c4b08af1bfab123595f33bd94a25830115458d1`, `b99742e1c01b9aab4d737df681c065b1284df15bd37ed39dd15a77c6ba56ecab` |

The scratch tree's `cli/src` and campaign tree's `cli/src` are byte-identical
(`diff -r` → no output), so every DA number above is a number about the same
source bytes the campaign worktree holds at `fc964fb6`.

### 9.1 The probe, in full

```rust
//! REV-CLI probe — scratch tree only (never in the campaign repository).
//!
//! Question: are the `#[cfg(not(test))]` lines that `docs/testing/module-cli.md`
//! §4c registers as `unreachable-by-construction` reachable from the *shipped*
//! binary on this host?
//!
//! Every probe spawns the instrumented `future` binary (`CARGO_BIN_EXE_future`),
//! so the child's profile merges into this test's — the technique
//! `cli/tests/bin.rs` already uses ("also lets cargo-llvm-cov capture main()'s
//! coverage through the inherited LLVM_PROFILE_FILE").

use std::io::Write;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_future")
}

/// An isolated FutureOS home per probe, so no probe can touch the real one.
fn home() -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("rev-probe-cli-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    dir
}

/// Spawn `program args...`, feed `stdin`, and return
/// `(exit code, stdout, stderr)` — bounding the wait so a hang is a failure
/// with a message rather than a stuck suite.
fn run_bounded(program: &str, args: &[&str], stdin: &str, secs: u64) -> (Option<i32>, String, String) {
    run_bounded_inner(program, args, stdin, secs, true)
}

/// `close_stdin = false` keeps the caller's pipe open until the child has exited:
/// `script(1)` forwards its own EOF to the pty as an EOT (`^D`), which the pty's
/// line discipline turns into `read() == 0` — so a wizard fed through a pty sees
/// "input closed" at its *first* prompt unless the pipe stays open.
fn run_bounded_inner(
    program: &str,
    args: &[&str],
    stdin: &str,
    secs: u64,
    close_stdin: bool,
) -> (Option<i32>, String, String) {
    let mut child: Child = Command::new(program)
        .args(args)
        .env("HOME", home())
        .env("USERPROFILE", home())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap_or_else(|e| panic!("spawn {program}: {e}"));
    child
        .stdin
        .as_mut()
        .expect("stdin piped")
        .write_all(stdin.as_bytes())
        .expect("write stdin");
    child.stdin.as_mut().expect("stdin piped").flush().expect("flush");
    if close_stdin {
        drop(child.stdin.take());
    }
    let deadline = Instant::now() + Duration::from_secs(secs);
    loop {
        if child.try_wait().expect("try_wait").is_some() {
            break;
        }
        if Instant::now() > deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("{program} {args:?} did not exit within {secs}s");
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    drop(child.stdin.take());
    let out = child.wait_with_output().expect("wait_with_output");
    (
        out.status.code(),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

/// `cli/src/main.rs:31` — `return run_agent(args)` for the hidden
/// `--linux-sandbox-helper` entry. `future_agent::cli::run_from_args` refuses the
/// helper off Linux ("unavailable on this platform"), which is exactly the
/// observable this probe asserts: the dispatch ran, and it neither hung nor
/// needed a Linux+bwrap host.
#[test]
fn rev_probe_hidden_helper_dispatch_reaches_main() {
    let (code, stdout, stderr) = run_bounded(bin(), &["--linux-sandbox-helper", "probe"], "", 30);
    eprintln!("REV-PROBE helper code={code:?} stdout={stdout:?} stderr={stderr:?}");
    assert_eq!(code, Some(1), "the helper entry exits 1 off Linux");
    assert!(
        stderr.contains("Linux sandbox helper is unavailable on this platform"),
        "the agent's refusal reached stderr: {stderr:?}"
    );
}

/// `commands/auth.rs:89` — `#[cfg(not(test))] let auth_data = Value::Null;` is the
/// first statement of `login()`. The in-process unit test
/// (`lib.rs::auth_login_url_parsing_reaches_login`) drives the same argv; this
/// probe drives it through the *shipped* binary, where the line is compiled.
#[test]
fn rev_probe_login_entry_reaches_the_non_test_binding() {
    let (code, stdout, stderr) = run_bounded(
        bin(),
        &["auth", "login", "--url", "http://127.0.0.1:1"],
        "",
        30,
    );
    eprintln!("REV-PROBE login code={code:?} stdout={stdout:?} stderr={stderr:?}");
    assert_eq!(code, Some(1), "an unreachable --url fails");
    assert!(
        stderr.contains("Network error"),
        "the device-code POST failure reached stderr: {stderr:?}"
    );
}

/// `commands/configure.rs:46` (`StdioPrompter::read_line`'s success arm),
/// `:50-52` (`read_secret` via `rpassword`) and `:855-862`
/// (`save_custom_provider`'s `#[cfg(not(test))]` agent-RPC block).
///
/// A test process has no console device, so the wizard's secret prompt cannot be
/// answered in-process. `script(1)` gives the child a *real* pty — its stdin is
/// the pty slave and `/dev/tty` resolves — and the answers below drive the whole
/// custom-provider flow. With no agent listening under the isolated home the
/// probe falls through to the offline write, which is the branch that makes the
/// success line observable.
#[test]
fn rev_probe_configure_wizard_under_a_pty() {
    let answers = concat!(
        "2\n",                      // provider choice: Custom
        "revprobe\n",               // provider id
        "\n",                       // display name (default)
        "\n",                       // protocol (default 1)
        "http://127.0.0.1:1/\n",    // base URL
        "\n",                       // API key (secret prompt, blank)
        "y\n",                      // Add a model?
        "rev-model\n",              // model id
        "\n",                       // model display name (default)
        "\n",                       // context window (default)
        "\n",                       // max output tokens (default)
        "\n",                       // image input (default n)
        "\n",                       // input price (default)
        "\n",                       // output price
        "\n",                       // cache read price
        "\n",                       // cache write price
        "n\n",                      // Add another model?
    );
    let (code, stdout, stderr) = run_bounded_inner(
        "/usr/bin/script",
        &["-q", "/dev/null", bin(), "config"],
        answers,
        60,
        false,
    );
    eprintln!("REV-PROBE configure code={code:?}\n--- stdout ---\n{stdout}\n--- stderr ---\n{stderr}");
    let seen = format!("{stdout}{stderr}");
    assert!(
        seen.contains("Configured custom provider 'revprobe'"),
        "the wizard completed through the pty: {seen:?}"
    );
}
```

### 9.2 The two readers

`counts.py` — the per-line `DA` reader used for every neighbourhood table
(`usage: counts.py <lcov> <path-substring>:<line[,line…]>`):

```python
#!/usr/bin/env python3
import sys

def load(path):
    out, cur = {}, None
    for raw in open(path, encoding="utf-8", errors="replace"):
        line = raw.rstrip("\n")
        if line.startswith("SF:"):
            cur = line[3:]; out.setdefault(cur, {})
        elif line.startswith("DA:") and cur is not None:
            n, _, c = line[3:].partition(",")
            try: out[cur][int(n)] = int(c.split(",")[0])
            except ValueError: pass
    return out

lcov = load(sys.argv[1])
for spec in sys.argv[2:]:
    body, _, span = spec.partition("~")
    path, _, lines = body.rpartition(":")
    span = int(span) if span else 0
    for k in [k for k in lcov if k.endswith(path) or path in k]:
        da = lcov[k]
        nums = [int(lines)] if span else [
            int(p) for part in lines.split(",") for p in
            (list(range(int(part.split("-")[0]), int(part.split("-")[1]) + 1))
             if "-" in part else [int(part)])]
        for ln in nums:
            print(f"{k.rsplit('/',1)[1]}:{ln}\t{da.get(ln,'ABSENT')}")
```

`xruns.py` — the cross-run scanner behind §7 (same loader, one row per report):

```python
#!/usr/bin/env python3
import glob, sys
specs = sys.argv[1:]
files = sorted(glob.glob('/tmp/*lcov.info'))
print("report".ljust(34) + "".join(s.rjust(26) for s in specs))
for r in files:
    cur = None; da = {}
    for raw in open(r, encoding='utf-8', errors='replace'):
        l = raw.rstrip('\n')
        if l.startswith('SF:'): cur = l[3:]; da.setdefault(cur, {})
        elif l.startswith('DA:') and cur:
            n, _, c = l[3:].partition(',')
            try: da[cur][int(n)] = int(c.split(',')[0])
            except ValueError: pass
    row = r.split('/')[-1].ljust(34)
    for s in specs:
        path, _, ln = s.rpartition(':'); ln = int(ln)
        cands = [k for k in da if k.endswith(path)]
        vals = [v for v in (da[k].get(ln, None) for k in cands) if v is not None]
        row += (("absent" if not vals else ",".join(str(v) for v in vals))).rjust(26)
    print(row)
```

### 9.3 The run scripts

```sh
#!/bin/sh                      # runA.sh — full suite, .git-less tree (the control)
cd /tmp/rev-cli-s1/tree
export PATH=/Users/geilige/.rustup/toolchains/1.97.0-aarch64-apple-darwin/bin:$PATH
export CARGO_HOME=/Users/geilige/.cargo RUSTUP_HOME=/Users/geilige/.rustup
export HOME=/tmp/rev-cli-s1/home CARGO_TARGET_DIR=/tmp/rev-cli-s1/target
ulimit -n 10240
mkdir -p "$HOME"
cargo llvm-cov -p future-cli --no-fail-fast --lcov \
      --output-path /tmp/rev-cli-s1/runA-lcov.info -j 4
# note: the run's own `--lcov` report is written only when the test run exits 0;
# this one exited 101 (one failing test), so the report came from the profraw:
#   cargo llvm-cov report --lcov --output-path /tmp/rev-cli-s1/runA-lcov.info
```

```sh
#!/bin/sh                      # runB.sh — the probe target alone (the falls)
# … same exports …
cargo llvm-cov -p future-cli --no-fail-fast --lcov \
      --output-path /tmp/rev-cli-s1/runB-lcov.info -j 4 \
      --test rev_probe_cli -- --nocapture
```

```sh
#!/bin/sh                      # runC.sh — one test alone (flake separation)
# … same exports …
cargo llvm-cov -p future-cli --no-fail-fast --lcov \
      --output-path /tmp/rev-cli-s1/runC-lcov.info -j 4 --lib -- \
      --exact browser::safari::safari_manager::tests::start_launch_success_against_fake_driver \
      --nocapture
```

### 9.4 This document's own tripwire

```
$ python3 .future/cov100/reconcile.py --report /tmp/cli-r2s3-final3-lcov.info \
      --scope .worktrees/cov100-r2/cli/src --doc docs/testing/review-cli.md \
      --section "0. Verdict" --label rev-cli
[rev-cli] files with uncovered lines=21 uncovered lines=123
  mentioned in review-cli.md: 123
  MISSING (uncovered, not mentioned): 0            # and no STALE block
```

## 10. Segment handoff

* **Artifacts**: this file; scratch tree `/tmp/rev-cli-s1` (probe, run scripts,
  `runA/B/C-lcov.info`, the two readers). Nothing in the campaign worktree was
  written except this report.
* **New evidence vs the author's own record**: 14 lines measured 0 → 1 with a
  same-revision control (`main.rs:31`, `auth.rs:89`, the seven `configure.rs`
  prompt lines, `configure.rs:855/856/862`, `version.rs:96/99`); five category
  falls with the reaching input named; the cross-run table that turns
  `safari_manager.rs:209`'s mechanism into evidence and re-labels
  `cdp_connection.rs:229` as a flake; the drift map (§6.1) and the peer row that
  was rewritten away (§6.2); and the correction that §4c's claim about its own
  self-check no longer describes the tool (§6.4).
* **Rejected approaches**: (a) a pty probe that closes stdin — `script(1)`
  forwards EOF as an EOT and the wizard dies at prompt one; (b) `future configure`
  as the command name — it is `future config`; (c) probing `browser_tools.rs:319`
  by launching a browser — the line is reachable in principle and I did not want a
  probe spawning Chrome on a shared host; the category verdict does not need it;
  (d) re-pointing the S10 report to make `cdp_connection.rs:229` disappear — the
  supervisor's message forbids it and §4.1 explains why the row is right.
* **Uncertainties**: the HEAD residual number (two caveats, §6.3); whether the
  `ubuntu-latest` image has a browser (the chromium_manager rows' condition);
  the exact Linux CI sandbox answer behind the doctor platform rows.
* **Next useful check** (successor work, in this order):
  1. The owning unit re-measures the four drifted files and re-derives §4c's
     numbers at the revision it intends to claim (plus a row for
     `utils/files.rs:161`, which §4c never mentions).
  2. The 5 measured-fall fixtures become tests: `bin.rs` gains `auth login`
     (unreachable URL), the `--linux-sandbox-helper` dispatch, and a piped-stdin
     `future config`; the pty answer is only needed for the *secret* prompt.
  3. A decision on the `#[cfg(not(test))]` class rule: 60 of the 68
     `unreachable-by-construction` lines rest on "in no `cfg(test)` translation
     unit", which is a fact about the test build and not an unreachability claim. The honest choices are OPEN-with-fixture or
     `unreachable-in-this-environment` with the *real* missing thing named (a live
     agent, a console, the shipped binary's own run).
  4. Re-point `module-cli-mcp.md`'s `mcp_client.rs` row at a fresh measure and add
     the new line `:406`.
