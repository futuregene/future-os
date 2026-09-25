# Linux: Bubblewrap sandbox

> ([中文](LINUX.zh-CN.md)) Implemented, merged into main in #496. The progress and
> test evidence below is recorded as of 2026-09-04; merging does not
> automatically upgrade it to a latest-candidate acceptance pass. L0–L4 and L6
> product integration are implemented; users have reported native Linux works,
> and the previous round has a 7/7 smoke record with bwrap 0.11.1. **The latest
> private-report/resource hardening has not been natively re-verified; the L5
> distro, architecture, installer, and independent security review are still
> incomplete.** This page unifies design, implementation, installation,
> anomalies, verification, and plans; shared rules and references are in
> [COMMON.md](COMMON.md).

## 1. Confirmed scope

| Decision | Current contract |
|---|---|
| L-D1 / D4 | The only Linux backend is system Bubblewrap; no bundling, no downloading, no automatic Landlock fallback |
| L-D2 / D11 | Network open; phase one has no seccomp filter, defense in depth comes separately later |
| L-D3 | The sandbox development branch integrates directly into the product with no hidden switch; availability and release criteria are separate |
| L-D5 | Existing glob matches at launch are hard-protected; new matches within a command are end-detection only |
| L-D6 | Phase one whole-command de-sandbox approval; phase two execution_grants together with macOS |
| L-D7 / D10 | Secure PATH, version/args, real-run three-layer probe; minimum bwrap **0.9.0** |
| L-D8 / D9 | Native Linux only; WSL not supported / not specifically detected; no scanning extra compatibility install dirs; no argv0 compatibility branch |
| 2026-09-04 addition | Missing Ask/Deny (including approval_rule), complex reopen, and missing writable allows are not must-fix items this cycle; see §4 |

0.9.0 is the product compatibility floor, not a promise of all upstream
security fixes. Only the bwrap file being root-owned is required; parent
directory permissions and mode bits are not recursively audited — a phased
trade-off the maintainers explicitly accepted. If an attacker can already
modify system programs as root, this sandbox is no longer their security
boundary. Hardening of root-owned path chains modifiable by ordinary users is
left for later and must not be described as verified.

## 2. Installation, availability, and troubleshooting

Install through the distro's trusted channel; installs below 0.9.0 must still
be upgraded:

```bash
# Ubuntu / Debian
sudo apt update && sudo apt install bubblewrap
# Fedora
sudo dnf install bubblewrap
future agent --probe-sandbox
future doctor
```

After installing or repairing, **fully exit and restart FutureOS**. The Linux
Settings/Composer keeps a stable option layout: disabled while detecting or
unavailable, selectable only after success — never deleting the option on
failure. The hint structure is "reason + remedy + code", e.g. "Bubblewrap is
not installed on this system; please install Bubblewrap. (`binary_missing`)";
install commands and restart hints are viewable on the settings page. Ordinary
cards do not stack extra network explanations.

The probe CLI does not start a resident Agent and is not blocked by the
singleton lock; the unified `future agent` and standalone `future-agent` share
one implementation, and doctor consumes the same result. The JSON contains
`available/backend/code`, and on success the system path/version/capabilities;
no full sensitive path sets are printed.

| code | Cause / handling |
|---|---|
| `available` | full probe passed |
| `binary_missing` | system bwrap not found; install and ensure a trusted absolute system directory is on PATH |
| `path_rejected` | relative/project-directory candidate or not root-owned; use the distro system package |
| `binary_invalid` | not an executable regular file or unsafe identity read; repair the system package |
| `version_unreadable` / `version_too_old` | version unparseable or below 0.9.0; upgrade |
| `required_feature_missing` | missing required args such as `--args`; upgrade the complete system package |
| `user_namespace_disabled` | host/container/security policy forbids userns; administrator evaluates support configuration |
| `proc_mount_restricted` | cannot establish a fresh `/proc`; check host policy |
| `probe_timeout` / `probe_failed` | probe timed out or failed; check doctor/local logs |
| `binary_identity_changed` | file replaced after probing; reject old credentials and re-probe |
| `probe_transport_error` | UI/Agent connection problem, not treated as permanently unsupported |

Each probe subcommand currently times out after 1 second; a success receipt is
cached for 300 seconds with binary-identity verification; failures are not
permanently cached. Empty/relative PATH and workspace/cwd candidates are
ignored; after canonicalization the same absolute path is pinned for
version/help/runtime execution.

The runtime probe runs a read-only root, user/PID/IPC namespaces, cap-drop,
minimal dev/proc, and a mode-0700 `/tmp` tmpfs, verifying temporary writes and
the read-only root. **It is not a full rehearsal of the real
workspace/HOME production plan**; probe success does not guarantee every rule
combination, resource state, or cwd can launch.

When the basic probe is clearly unavailable, the Agent/resolution layer falls
back to manual, Desktop persists the fallback and shows the reason; transient
connection failures keep the setting. Runtime plan/helper errors return tool
failures — neither auto-degradation nor bare runs. The model can explicitly
request whole-command de-sandboxing; it runs only with user approval; the
conversation itself need not stop. Avoid blindly replaying commands that may
already have had side effects.

## 3. Implementation principles and code map

```text
RuleSet snapshot → LinuxSandboxPlan → PreparedShell + request FD
  → current future/future-agent re-execs the outer helper
  → pinned bwrap executable FD + --args OPTIONS FD + short COMMAND argv
  → inner helper: mount/capability re-check, no_new_privs, FD convergence
  → shell / descendants → status pipe → outer re-scan → private report → Agent tool output
```

The hidden helper dispatches before normal runtime/singleton and loads no
models and starts no RPC. Desktop still packages only the unified `future`
sidecar — no second helper artifact; the helper uses an explicit subcommand
rather than argv0 dispatch.

| Module (under `agent/src/sandbox/`) | Responsibility |
|---|---|
| `backend.rs`, `linux/runner.rs` | PreparedShell, re-exec arguments, request/report files and spawn |
| `linux/probe.rs` | system binary, version/args/runtime, receipt/cache |
| `linux/plan.rs` | rule snapshot, writable/read-only/unreadable/reopen, missing omission, digest |
| `linux/glob_scan.rs` | same-root merging, bounded no-follow scan, shared pre/post |
| `linux/request.rs` | versioned request, size/FD/path/phase validation |
| `linux/helper.rs` | mount FDs, bwrap, inner re-check, status, signals/descendants, re-scan |
| `linux/post_scan.rs` | item-by-item missing-target detection, failure/unchecked counts |
| `linux/report.rs`, `linux/violation.rs` | private report authentication, English detection descriptions, non-authoritative output cleanup, rejection heuristics |

### 3.1 Mount and process boundaries

Production uses `--new-session --die-with-parent --unshare-user --unshare-pid
--unshare-ipc --cap-drop ALL`; first `--ro-bind / /`, then open the
workspace/temp/allow-write roots, overlay protections and supported narrow
reopens. `--dev /dev`, fresh `/proc`; no `--unshare-net`.

Write protection is usually a read-only bind; read protection uses a mode-000
opaque source mask and must not be faked as "reading an empty file succeeded".
The same path with read+write protection keeps only the stronger opaque mask —
no duplicate mounts. Missing protection targets are fully omitted from mounts;
no host placeholder objects are created. Unsupported matchers like `[]`/`{}`
error explicitly; read-only reopens keep write denied, and a write allow that
would bypass a still-effective read deny is refused at compile time — no silent
weakening.

Mount sources are pinned as O_PATH FDs, and bwrap mounts via `/proc/self/fd/N`;
the inner side re-checks dev/inode with type/permissions. Directory size/mtime
changes with normal host writes and is not used as mount identity; the bwrap
executable still gets strict receipt verification and executes via a pinned FD.
For path checks, only a real NotFound counts as missing; permission errors,
ENOTDIR, and dangling symlinks are not ignored as empty paths. No unconditional
support promise for reachability/overlap combinations.

The inner side confirms via `capget` that effective/permitted capabilities are
all zero, then sets `PR_SET_NO_NEW_PRIVS`, closes unneeded FDs, and starts the
shell. The Agent listener, logs, database, etc. must not be inherited by the
command. Inside the PID namespace the helper forwards signals and reaps
descendants; the status pipe carries the raw wait status. Normal completion,
signals, timeout/abort, and parent death must cooperate; no detached
descendants are left.

### 3.2 ARG_MAX, payload, and resource budgets

| Resource | Current limit / semantics |
|---|---|
| helper JSON request | v3, 8 MiB; production outer/inner both use anonymous file FDs, argv carries a short `fd:3` reference |
| mount / shell argv | 16,384 mounts; shell argv combined 96 KiB, still subject to system environment-size limits |
| bwrap OPTIONS file | NUL-separated, 16 MiB; together with real argv at most 9000 arguments |
| FD | reads `/proc/self/fd` and RLIMIT_NOFILE before opening mounts; 16 slots reserved internally |
| report | 64 KiB, version 1, at most 4 digest-matching detection-only events |

`--args FD` solves many mount arguments blowing execve ARG_MAX, but **COMMAND
must not go in the arguments file**: bwrap's recursive OPTIONS parsing stops at
`--` and will not hand the file-tail COMMAND back to the outer side. The real
argv must keep `-- current_exe helper-args`. User commands/environment, FDs,
temporary disk, and kernel mounts still have resource limits; pre-checks cannot
eliminate concurrent races — ENOSPC/EMFILE/spawn failures still return errors
without relaxing the sandbox.

Requests reject unknown versions, invalid/duplicate/phase-mismatched FDs,
non-absolute targets, NULs, and over-limit payloads; bounded base64 paths exist
only for direct invocation/negative tests, not as a production large-payload
channel. The old `MissingProtected` wire variant is kept but explicitly
rejected in both phases; tmpfs missing mounts are no longer emitted.

### 3.3 Large repository scanning

The same static root is scanned only once, with precompiled patterns and
pruning by prefix and maximum match depth: `.env.*` only looks at the root
level; a bounded `pkg-*/secrets/*.key` does not recurse the whole repo; `**` is
the only thing allowing the needed recursion. Hidden, gitignored,
`node_modules`, `target`, and `.git` are not skipped; matching directories are
kept too; directory symlinks are not descended, and matching links keep both
lexical and canonical targets.

The old 100,000-node hard limit is removed; node counts are statistics only.
Current shared budget: 30 seconds, 256 unique patterns, 2048 unique paths,
4 MiB of estimated result-associated bytes, 64 layers. Pre-launch and
post-command are timed separately; post's glob and missing-target checks share
the budget. Pre-launch scanning runs in `spawn_blocking` and responds to Abort;
after completion the main task confirms cancellation again — the scan worker
must not start commands on its own.

The 30 seconds is a cooperative budget; it cannot preempt a stuck filesystem
syscall. Errors carry phase/root/pattern plus visited/matches/elapsed/limit;
codes include `glob_scan_timeout`, `glob_scan_match_limit`,
`glob_scan_result_bytes_limit`, `glob_scan_pattern_limit`,
`glob_scan_depth_limit`, `glob_scan_io_error`, `glob_scan_cancelled`,
`glob_scan_pattern_invalid`. Pre-launch failures do not execute; post-completion
failures only report incompleteness without changing the completed state.

### 3.4 Detection reports and retry trustworthiness

`omitted_missing_protected_paths` means: **did not exist at launch, so no
protection mount was installed; after completion it only checks whether they
appeared — it does not prevent creation and does not undo modifications.** It
is not the same as having no rules or having received an allow.

| Event | Meaning of `affectedCount` |
|---|---|
| `missing_protected_created` | targets that were missing and now exist; creator unknown |
| `missing_protected_scan_failed` | targets whose check failed or was not checked — **not a violation count** |
| `dynamic_glob_created` | new sensitive-rule match count; multiple rules may hit the same path |
| `dynamic_glob_scan_failed` | detection incomplete; 0 does not mean no violation |

`message` is generated by the backend per kind as an English explanation;
structured count fields stay unchanged. Creation-class events explicitly state
not prevented, not rolled back, creator unknown; check-failure class states the
result is unknown — neither command failure nor a passed check. The four
post-hoc checks are by default for the model's internal judgment only; briefly
state the actual impact and next steps only when they affect task conclusions,
require user action, or the user asks — without restating internal fields,
counts, and scan mechanics; access-restricted cases only guide requesting
approval when a necessary operation was blocked. Explanations do not change the
exit code or grant de-sandboxing. Received messages do not participate in trust
decisions; old records without message still parse. The information is still
appended to the shell tool output — the raw report is not hidden, and no new
popups/notifications are added; this is expression guidance for the model, not
a guarantee the model never restates.

Production creates an independent anonymous report file per spawn; the writer
is given only to the outer helper; the outer removes `report_fd` from the inner
request, sets CLOEXEC, and does not add it to the bwrap keep-list. The command
and its descendants cannot inherit the writer. Only after command completion,
descendant reaping, and re-scan does the outer write the complete report; the
Agent reader validates length/version/digest/event types and does not trust a
stdout marker.

A command printing `__FUTURE_SANDBOX_VIOLATION__:` itself is marked
`untrusted command text; not a sandbox report`; trusted events are appended
after command-output truncation. Direct helper debugging can still print the
marker with newlines, but that does not constitute origin authentication. When
detection-only events exist or the private report is missing/corrupt/over-limit/
mismatched, **passive** de-sandbox retry is forbidden — the original exit is
kept with a detection-unknown hint; the model is not prevented from separately
requesting approval explicitly.

After a valid empty report, ordinary failures still use the
`Permission denied`/`Operation not permitted`/`Read-only file system` text
heuristics, excluding exits 0, 2, 125, 126, 127. It is not errno
certification: a program can forge ordinary error text; final approval still
requires the user. The private channel does not defend against an attacker who
already controls the host's same-user Agent.

Deleting an existing protected file (like `.env`) may return
`Device or resource busy` (EBUSY): that object is a mount point inside the
sandbox, not a sign a program holds it. That text shares passive-approval
recognition with the permission errors above: a valid private completion report
with no detection events and a classifiable non-zero exit are required, but the
path no longer needs to exactly match the launch request, and the command no
longer needs to be a simple `rm`/`rmdir`.

Relative paths, absolute paths, variable expansion, cwd switching, scripts, and
other programs all go through the same error-output classification; path
extraction is only for approval display — failure does not block approval, and
the script's absolute target is not guessed from the initial cwd. No
synthesized "confirmed protected mount target" diagnostics. Ordinary device
busy or forged text can also trigger approval; swallowed, localized,
unrecognized-format, or excluded-exit-code output can miss cases — no promise
all programs' failures are passively recognized. The model can still request
actively. After approval it is one whole-command de-sandbox run; rejection does
not delete the file; incomplete detection/new-target detection keeps passive
retry forbidden.

## 4. Platform differences and accepted gaps

The following trade-offs are confirmed; the old draft conclusion "missing
targets must be hard-protected or this cycle is blocked" is no longer kept:

| Situation | Current behavior / risk |
|---|---|
| Existing rule files, models, HOME/workspace secrets | protected mounts installed per read/write rules; a missing policy is not expanded into an allow for existing files |
| Missing `approval_rule.json`, custom Deny, `.ssh`/`.env`/models, etc. | not mounted, end-check only; if the parent directory is writable they may be created and read/written. **Linux does not promise creation interception for missing Denys**; rule-file creation may also affect the next turn's rules — accepting the risk is not the same as no security risk |
| Missing target in a read-only domain | the command usually cannot create it, but after a concurrent host process creates it, dynamic read-denial is not guaranteed |
| New glob in a writable domain | created/read/used within the command is not dynamically intercepted; the next command re-scans existing objects |
| Wide deny/read-only + narrow allow, overlapping masks | may fail due to a mode-000 ancestor being untraversable or final mount view/identity conflicts; no guarantee of complete first-match equivalence |
| Missing allow-write/reopen root, invalid cwd | may fail to open the mount source/chdir and refuse that launch; no automatic source creation or widened parent authorization |

For example, a high-priority allow `private/output` plus a wider deny `private`
cannot be declared reopenable just by sorting by mount depth; and an external
allow root that does not exist yet cannot guarantee `pwd` launches. These two
classes are unchanged for now; users can adjust rules or explicitly request
whole-command approval.

Scanning is a snapshot, not an audit: "create → use → delete" is missed; other
host processes' creations may be detected as appearing but cannot be
attributed. A single metadata failure continues checking later targets and
reports incompleteness separately; cancellation/budget record unchecked counts.
The outer continuously records TERM/INT/HUP/QUIT, but SIGKILL, crashes, missing
status, or report-write failures can still leave no complete result — **no
report is not no violation**.

Field clarification: the model first creates a missing file via one shell call
`printf ... > .env`, then a second shell call `cat .env`; the first creation
produces a `missing_protected_created` detection, the second launch re-scans,
the file now exists and is masked, and the read failure triggers approval.
Approval was not triggered by interception of the creation, nor by the
detection marker. If `printf` and `cat` were in the same shell call, no mount
view would be rebuilt between them, and the second call's protection conclusion
cannot be applied.

No host synthetic placeholder + cleanup. bwrap's tmpfs setup first does
ensure_dir/mkdir: a read-only parent gives EROFS, a writable parent really
creates host objects — the mount namespace itself does not isolate such writes.
If hard protection of future names is later required, a separate design is
needed (parent-directory isolation view/broker/FUSE, etc.) while preserving
write-back semantics; simply tmpfs-masking the parent directory would lose
output. Codex comparison in COMMON §6.

## 5. Field failures and fix history

| Field / review ID | Root cause and current fix |
|---|---|
| `.env.*: sandbox glob scan limit exceeded` (`d469719b`) | old per-rule whole-repo scans falsely hit the 100,000-node limit; same-root grouping, pruning, shared budget — §3.3 |
| `pwd; whoami` exit 125, `.aws mount source is unavailable` (`d51963c9`) | missing guards went into both ordinary binds and the missing list; classification dedup, strict NotFound, ordinary mounts don't compare directory mtime/size |
| `Can't mkdir ... .aws: Read-only file system` | missing tmpfs targets did ensure_dir; changed to omission + detection, no host targets created |
| bwrap usage / command status not reported (`01b7e413`) | OPTIONS file swallowed COMMAND; COMMAND back to real argv; added real default-rules production full-chain smoke |
| SR-01 / SR-02 | root-owner verification and trade-off comments; cross-layer/same-layer compiled by first-match to avoid later rules taking effect repeatedly |
| SR-03 | old "namespace-only creation" assumption withdrawn; current omission, legacy wire mount rejected |
| SR-04 / SR-05 | original stdout-marker trust replaced by the private report; re-scan failure detection-only with exit kept, avoiding a second execution after side effects |
| SR-06 / SR-07 | dual FD transport for helper JSON and bwrap OPTIONS; later added the 9000-argument and RLIMIT budgets |
| SR-08 / SR-09 / SR-10 | 0.9.0, capget/no_new_privs, no seccomp; UI stable layout/disabled items/reason-remedy-code/restart |

When bwrap/inner has no completion and was not signal-terminated, the helper
returns infrastructure 125 with stderr merged and captured. **A user command
can also exit 125 by itself; missing completion does not prove nothing
executed**; never auto-replay on the exit code alone.

## 6. Historical evidence and development progress

- Initial branch baseline `fd3e1771`; the Linux implementation comes from
  `claude/linux-bwrap-sandbox`; on 2026-09-03 `origin/main@15d7df79`
  (`0867b0fd`) was merged. These are historical anchors, not current-latest
  main claims.
- 2026-09-03 Ubuntu 26.04/Linux 7.0/x86_64, `/usr/bin/bwrap` 0.11.1, initial
  5/5 ignored smoke, basic probe PASS; not full target-distro certification.
- Same-day historical cross-platform record: sandbox 115 tests, Rust workspace
  tests pass; Tauri 1095 items with one first-run remote runtime timing
  failure, targeted re-run passed. Old Desktop 687/Mobile 551, availability 12
  PASSes kept as historical; some later hosts lacked Node and were not re-run —
  they cannot be combined into one all-green candidate.
- 2026-09-04 large-repo macOS fixture: Linux modules 40 PASS/1 ignored;
  explicit large fixture 1 PASS, 100,013 items first/repeat ≈777/783ms, a
  single `.env.*` access 3 items. OS cache uncontrolled — not Linux
  cold-cache/bwrap performance.
- First-round `.aws` fix on macOS: 45 PASS/1 ignored; Linux-only/helper not run.
- `01b7e413` second-round committer record: Linux 53 PASS/1 ignored, 7/7 bwrap
  smoke, Linux Clippy/fmt pass. Full Agent 1651 PASS/2 FAIL
  (`models::future::cache_save_and_concurrent_load_never_torn`,
  `models::tests::registry_injects_future_models_from_disk_cache`);
  independent re-runs passed — recorded as suspected shared-cache parallel
  flakiness, **not a first-run all-green**.
- Latest explicit items/private-report hardening: macOS fmt, Agent all-targets
  Clippy, diff check pass; new tests not executed. The Linux cross-check was
  blocked in the ring build by missing `x86_64-linux-gnu-gcc`; no claim that
  the Linux helper compiles. User feedback that real machines work does not
  cover all the latest anomaly branches.

| Phase | Current status |
|---|---|
| L0 probe / L1 plan and seams / L2 helper | implemented; real production plans and boundaries still need target-host re-verification |
| L3 rules, scanning, escalation | implemented as a bounded version, not complete dynamic-path equivalence |
| L4 packaging diagnostics / L6 product entry | local integration complete; system-only, unified probe/doctor, Desktop English/Chinese |
| L5 release verification | native distro/arch, artifacts, and independent security review pending; not exempted because the dev-branch entry is optional |

## 7. Real-machine acceptance operations and matrix

Run at the candidate repo root in a native Linux ordinary environment (VM ok;
container/WSL is not a substitute):

```bash
./scripts/tests/test-linux-sandbox-real-machine.sh
# optional full Rust workspace tests
./scripts/tests/test-linux-sandbox-real-machine.sh --full
# GUI development launch
./scripts/dev/start-desktop-linux.sh
```

The script collects the environment, builds the probe, Linux unit tests, the
100,000+-item fixture, stderr capture regression, all ignored smokes,
fmt/clippy, and outputs `linux-sandbox-evidence-*.tar.gz`. It installs no
software and changes no system policy; a `skipping Linux sandbox smoke` must
fail. **Do not hard-code "7 tests"** — use the actual suite in the candidate
sources.

The GUI dev-launch script is not a read-only probe: it builds/starts an
independent Agent and runs Tauri, cleans stale runs/approvals by default, and
reclaims the Agent it started on exit. Set `CLEAN_STALE_APP_TASKS=0` to keep
those records; `DRY_RUN=1` for diagnostics only. Also settable: `REUSE_AGENT`,
`BUILD_AGENT`, `BUILD_CLI`, `RUN_CHECKS`, `FUTURE_AGENT_GRPC_ADDR`,
`DESKTOP_DEV_PORT`. An unavailable bwrap does not block Desktop startup,
allowing the unavailable-hint to be verified; the authoritative security check
remains with the Agent.

Run independently:

```bash
cargo test -p future-agent --test linux_sandbox_smoke -- --ignored --test-threads=1 --nocapture
cargo test -p future-agent --lib sandbox::linux::glob_scan::tests::large_workspace_exceeds_old_node_limit_without_failing -- --ignored --test-threads=1 --nocapture
```

Important smokes: real default-HOME-rules production chain
`production_plan_with_real_default_rules_starts_a_shell`; no_new_privs/exit,
unreadable/FD, raw signals, parent death, production request FD, missing
creation detection, glob re-scan failure; the latest
`missing_scan_reports_partial_failure_after_unterminated_command_output` and
`command_cannot_write_or_forge_private_helper_report` must be executed.
Unterminated output must not swallow diagnostics, forged stdout must not
pollute the private report, other targets must still be found after a partial
scan failure, exit 23 unchanged.

New `removing_existing_env_reports_a_busy_protection_mount`: real default
plan/helper/bwrap deletes an existing `.env`, returns EBUSY with an empty
private report, hits the text classification, host file unchanged. Unit tests
cover path-format-independent EBUSY classification, kept markers, exit-code
exclusion, and approval refusal / report gating across command forms; Unix
cases cover deletion after approval. New/adjusted cases still await CI/native
Linux execution; they do not inherit the old smoke's PASS.

Evidence template: Tester, UTC date, Host ID, native/VM, distro/kernel/arch/
glibc, desktop session, candidate commit+dirty diff, app version, artifact
SHA256, bwrap path/version/package version, log directory, reviewer. Collect
`git rev-parse HEAD`, `git status --short`, `cat /etc/os-release`, `uname -a`,
`bwrap --version`, probe, doctor; do not upload credentials/full rules/
sensitive path lists.

Status uses only PASS/FAIL/NOT RUN/ENVIRONMENT LIMIT; PASS requires the
matching machine/commit log. The following target matrix is still to be
executed row by row; a correct rejection of a too-old version is not recorded
as a normal-availability PASS:

| ID | Native host | Arch / expectation |
|---|---|---|
| RH-01 | Ubuntu22.04 | x86_64; if official package <0.9.0 verify version_too_old, normal operation needs a trusted upgrade |
| RH-02 / RH-03 / RH-04 | Ubuntu24.04 / Debian stable / supported Fedora | x86_64, record exact versions |
| RH-05 | Ubuntu24.04 | aarch64 |
| RH-06 | Debian stable or Fedora | aarch64, covering one more distro |

Installer PKG-01~04: `.deb` fresh install, in-place upgrade, uninstall for both
architectures, plus the portable tarball. Record actual artifacts; GUI/sidecar
re-exec works, session configuration preserved, no bwrap bundled, uninstall
does not remove system bwrap. Install per the artifact README; only for the
selected test package run `sudo apt install ./FutureOS_<version>_<arch>.deb`
and later `sudo apt remove futureos`. No AppImage/rpm this cycle.

Each normal host SM-01~06: all smokes with no skips, probe/doctor consistent,
workspace writes succeed/external writes denied, infra and detection events not
passively retried, tester's local HTTP service reachable, Settings/Composer and
manual fallback correct. Also check active/passive titles, concrete paths
diagnostic-only, rejection not executed, one-time approval not changing the
global setting; repeated `pwd` in a large repo on Desktop recording
first/repeat timings (uncontrolled cache — not called cold/warm), hidden/
ignored secrets still protected.

| Negative ID | Dedicated VM/fixture scenario | Assertion |
|---|---|---|
| NEG-01 /02 | missing bwrap, relative/project fake binary, non-root owner | fake binary not executed; trusted system candidates may still be searched, otherwise stable failure |
| NEG-03 /04 | userns disabled, fresh proc restricted | stable code, unavailable, no weaker-backend substitution |
| NEG-05 /06 | old version/bad output/missing args, timeout | version/feature/timeout codes; re-probe after repair |
| NEG-07 | binary inode replaced after probe | old receipt not executed |
| NEG-08 | WSL | record unsupported scope only; do not demand a nonexistent WSL detection code; not counted toward native certification |

Do not bypass organizational security policy on daily-use hosts; when a
scenario cannot be safely constructed, record ENVIRONMENT LIMIT and switch to a
suitable VM — never delete the failed row.

### 7.1 Independent security review (SEC-01~15)

Review the actual candidate diff; give each item PASS/FAIL with source/log
evidence; running tests alone is not a substitute:

1. Mount lexical/canonical/symlink/FD identity and replacement races.
2. request/mount/status/report FD phase isolation; user commands do not inherit
   Agent resources.
3. root-owned/setuid bwrap, capget zero, no_new_privs.
4. user/PID/IPC, dev/proc, open-network contract.
5. signal/abort/timeout/parent death for outer/bwrap/inner/descendants across
   wait, execution, and re-scan phases.
6. missing targets **not mounted, no host placeholder created**; appearance
   detection is not a creation-interception guarantee.
7. whole-file bad rules, unsupported matchers, scan budget/I/O/cancel fail
   before launch; keep per-entry parse differences for individual invalid
   entries.
8. deny/reopen final reachability; do not present accepted complex-combination
   failures as complete equivalence.
9. probe receipt consistent with execution path/version/identity/expiry.
10. private report authentication, stdout untrusted, detection/invalid reports
    not passively retried; negative corpus for ordinary-text false positives.
11. UI/log/RPC desensitization; necessary diagnostics can locate without
    leaking secrets.
12. helper version/size/count/FD/NUL/path validation and pre-singleton entry.
13. explicitly no seccomp now; no syscall or network filtering claims.
14. `--args` OPTIONS / real COMMAND separation, 9000/16MiB/FD budgets and
    ENOSPC/EMFILE.
15. GHSA-pxhw-h44j-8pfx related setup path re-check; deleting MissingProtected
    does not mean all symlink risks are immune.

Sign-off records Reviewer/UTC/Candidate/Decision/Blocking issues/Follow-ups/
Evidence. Release-readiness is declared only when L5-01 target hosts, L5-02
actual packages, and L5-03 review are complete with no blocking items;
accepted scope gaps are recorded per §4 — no fabricated "intercepted" PASS.

## 8. Follow-up priorities

| Priority | Work |
|---|---|
| P0 release evidence | latest Linux-only code compile/full smoke, large-repo default rules, RH/PKG/SEC matrix; old OR-01/OR-06 |
| P1 | ordinary stderr heuristic false positives and replay side effects (OR-03); phase-two execution_grants; old-bwrap-version security re-check (OR-08) |
| P2 | seccomp defense in depth: freeze the syscall policy, ptrace/process_vm/io_uring compatibility and failure semantics first, then implement; not enabled in phase one |
| P2 | diagnostic package, scan budget and FD performance calibration, helper/cwd reachability hints; credential channel in COMMON |
| P3 / accepted | path-chain hardening beyond root-owner (OR-07); missing Deny, complex reopen, missing allow stay as-is — no unauthorized scope expansion this cycle |

OR-02 (mount argv's ARG_MAX) is closed by the dual-FD transport with resource
limits kept; OR-05 minimum version is closed; OR-04 seccomp moved to follow-up.
No "starts in every environment" promise holds: when the command cannot be
safely prepared, fail explicitly, continue the conversation / request approval
explicitly — never silently reduce protection.
