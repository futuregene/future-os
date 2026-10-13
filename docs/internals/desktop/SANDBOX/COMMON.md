# Sandbox: shared rules, approvals, and references

> ([中文](COMMON.zh-CN.md)) Updated: 2026-09-07 (defaults re-checked). The Linux
> implementation has merged in #496; platform documents keep their own
> candidate-version acceptance boundaries — "implemented" is not release
> certification. Historical tests only prove their own commits; they cannot
> automatically cover later changes.

## 1. Document boundaries and platform overview

This directory has four primary documents: this one maintains shared semantics,
protocol, decisions, and references; [MACOS.md](MACOS.md), [LINUX.md](LINUX.md),
[WINDOWS.md](WINDOWS.md) each maintain implementation, differences, acceptance,
historical evidence, and plans. The [product document](../PRODUCT.md#46-approval)
keeps user promises and the [data structures](../ER.md) keep persistence
semantics.

| Dimension | macOS | Linux | Windows |
|---|---|---|---|
| OS backend / name | Seatbelt / sandbox protection | system Bubblewrap / sandbox protection | Unelevated RestrictedToken + NTFS ACL / write protection |
| shell read protection | SBPL path rules | masking of targets existing at launch | **not provided** |
| shell write protection | SBPL dynamic path rules | read-only root + writable/protected mounts | capability SID write boundary, limited by existing ACLs and delete permissions |
| Missing protected target / new glob | matched by profile once the path appears | missing targets skipped; new targets only end-detection | no object to attach an ACE to; globs not enforced |
| Extra authorization | precise per-command read/write grant, still sandboxed | precise per-command read/write grant, still sandboxed | declares concrete write capability, still restricted after approval |
| Network | open | open, no seccomp | open |
| Availability check | `sandbox-exec` existence check | PATH / version & args / real namespace-mount probe | real token/ACL/private desktop/shell/cleanup probe |
| Progress | v2 implemented, historical smoke pass | L0–L4/L6 integration implemented; latest hardening and L5 release matrix pending re-verification | W1–W7 integrated, native and installer historical PASS; some product interactions need item-by-item re-verification |

Note: sharing rules across three platforms does not mean each OS can express
equivalent rules. Tool-layer approval is also not the same as every shell
subprogram receiving the same protection.

## 2. Enablement, tiers, and the execution chain

Product goal ordering: simple configuration, smooth development flow,
reasonable security; not an isolation scheme against hostile hosts or root
attackers.

| `tier` | native tools like read/write/edit | shell |
|---|---|---|
| `manual` (opt-in) | path rules, three states | read-only whitelist skips prompts, other commands ask first; no OS sandbox |
| `sandbox` | same path rules | current platform's OS backend; grant semantics in platform docs |
| `off` (desktop default) | no approval prompts | run directly without OS wrapping |

The GUI sends the policy via `set_sandbox_policy` when a session is created.
The `SandboxTier` enum's default/unknown value is manual, but the desktop
persisted setting's default/unknown is off, and new Agent sessions default to
allow-all permissions; these must not be conflated into one product default.
TUI/CLI/channels without a delivered policy do not auto-enable this system;
they keep their own `permission_level`/workspace boundaries. Callers without an
approval UI cannot be assumed able to complete GUI interaction. `off` is
released at the Agent layer, not by the frontend auto-clicking approval.

```text
GUI policy + workspace → ResolvedSandbox + RuleSet
  ├─ native tool：normalized concrete path → allow / ask up-front approval / deny
  └─ shell：manual command approval / off direct run / sandbox platform executor
approval events → RPC → Desktop storage & push → Desktop/phone cards → decision back to Agent
```

When the basic probe is clearly unavailable, sandbox resolves to manual with
tool rules still active; Desktop saves the explicit result as a fallback —
transient connection failures must not be treated as permanent lack of support.
**manual is not an OS sandbox**; whitelisted or approved commands run with the
current user's privileges. A real command initialization failure is not the
same as an unavailable basic probe; do not auto-switch to off, run bare, or
re-execute.

A single tool failure returns to the model as a tool result/error and the
conversation can continue. No new command-level auto-degradation, temporary
manual mode, circuit breaker, or recovery UI is added for now; the model can
explicitly request de-sandboxed approval separately but cannot approve itself,
and recovery from every failure is not guaranteed.

## 3. Rule model and persistence

```json
{
  "version": 1,
  "rules": [
    { "path": "dist", "access": "write", "action": "allow" },
    { "path": "~/notes", "access": "write", "action": "ask" },
    { "path": "private-data", "action": "deny" }
  ]
}
```

- Two files: `${WORKSPACE}/.future/approval_rule.json` and
  `~/.future/approval_rule.json`. The Agent reads them every turn; the GUI
  writes them through the trusted Tauri path; users may edit by hand and commit
  to git.
- `access` is read/write/both, defaulting to both. `action` is allow/ask/deny.
  Read and write are evaluated separately.
- Relative paths resolve against the workspace; user rules should use absolute
  paths or `~/`. A matcher without wildcards matches itself and its subtree;
  `*` within a segment, `**` across segments, `?` one character — no claim of
  supporting all shell glob syntax.
- `paths.rs` handles `~`, `..`, nearest existing ancestor, symlinks, path
  component boundaries, and macOS case matching. Links are judged by their
  final target; Linux/Windows execution must also re-check the object — one
  canonicalization is not a race-free proof.
- Priority: **overrides → guards → session → workspace file → user file →
  fallback**, in written order within a layer, first match returns. Fallback:
  read allow; write allow inside workspace and `temp_roots()`, ask outside.
- temp is the environment/system temp root the code actually resolves — not a
  blanket claim that every platform has an isolated "per-session temp". `.git`
  is not excluded from the writable workspace.
- A missing rule file is treated as empty; corrupted/unreadable layers record
  `resolution_errors`, and the Linux sandbox compilation refuses to execute;
  other paths still process the layers the loader has — no claim of uniform
  fail-closed across platforms. Individual entries missing path or with invalid
  action are skipped by the parser; **no unified GUI bad-rule warning is
  guaranteed**.

### 3.1 Built-in non-overridable layer and the credential exception

| Path | Rule |
|---|---|
| `.future/approval_rule.json` under workspace and HOME | write deny, read allowed |
| `~/.future/agent/models.json`, `~/.future/agent-app/models.json` | read + write deny |

This protects the scope of rule decisions and platform enforceability, not a
cross-platform "never writable" promise: creating a missing rule file on
Linux/Windows is an accepted gap; a user-approved whole command out of the
sandbox on macOS/Linux is not bound by these OS rules either. The ordinary
`.future` directory cannot be fully sealed because Chat workspaces live inside
it.

`auth.json` (agent/agent-app) was temporarily removed from the hard-deny list
for the official `future` CLI's tests; **this is not a new unconditional
allow-write** — it remains subject to other rules and write roots, but a default
read may expose credentials. The path sandbox cannot trust the official CLI
inside the same command while distrusting `cat`.

Follow-up candidates: short-lived, scope-limited credentials, or Agent RPC with
peer-credential verification; after completion restore the auth.json deny.
Acceptance must prove the official CLI authenticates normally, arbitrary shells
cannot directly read/write credentials, and tokens never enter logs/history/tool
output or uncontrolled subprocess environments. Not currently scheduled; the
temporary exception must not be advertised as a security channel.

### 3.2 Sensitive guard list (read + write ask)

Under HOME:

| Category | Path relative to HOME |
|---|---|
| SSH/GPG | `.ssh`, `.gnupg` |
| Package managers | `.npmrc`, `.pypirc`, `.cargo/credentials`, `.cargo/credentials.toml`, `.gem/credentials` |
| Plaintext credentials | `.netrc`, `.git-credentials`, `.env` |
| Cloud/orchestration | `.aws`, `.azure`, `.config/gcloud`, `.terraform.d`, `.kube/config` |
| Container/CLI/system | `.docker/config.json`, `.config/gh`, `Library/Keychains` |

Under workspace: `.env`, `.env.*`, `**/*.pem`, `**/*.key`, `**/*.p12`,
`**/id_rsa*`. A root-level `.env.*` is not the same as `.env.*` in every
directory recursively.

Guards outrank session/user/workspace allows; a broad directory allow does not
lift a guard. Native-tool sensitive access only offers "allow once", never a
persistent allow. Linux existence differences and Windows shells not denying
reads cannot be directly inferred into complete hard protection from this list.

### 3.3 Saving and same-turn effect

Desktop's `approval_rules.rs` reads, modifies, and writes the whole workspace
file, preserving unknown fields and existing rules with deduplication. After
saving, `inject_session_rule` / `add_session_rule` injects into the current
shared `SessionRules`; later calls in the same turn see it immediately; the
next turn reads from the file again. This is not upgrading every "allow once"
into a persistent directory rule. Sensitive guards still outrank session.

## 4. Approval protocol and UI

Desktop reuses `ApprovalPrompt`; phones use native cards but share the trusted
semantic projection. What persists are requests/decisions, not the old SQLite
rule source of truth. Push and polling restore pending state — popups in memory
alone are not enough.

- The trusted backend generates a "behavior + target" title; single targets do
  not duplicate fields, multiple targets are listed in full, Windows lists at
  most 8 targets at once — no "and N more" hiding scope.
- Command/file previews go in a collapsible detail; the model's
  reason/justification is auxiliary context, not an override of the real grant
  scope.
- Per scenario show: not allowed / allow once only / always allow in this
  project. Persistent allows only write explicit path rules; sensitive paths,
  manual shells, and macOS/Linux whole-command escalation do not offer a
  persistent-rule button.
- Ordinary cards do not display or edit raw globs, ACLs, SIDs, backends, hashes,
  or raw JSON. When the trusted payload cannot be parsed or has mismatched
  types, no approve button is shown.
- Phone v1 only has deny / allow-once; a local memory decision must not be
  passed off as a persisted project rule. Multi-target decisions apply to the
  whole group; changing scope requires a new request.

### 4.1 Single-command file permissions and explicit escalation (macOS/Linux)

The on-request flow returns sandbox failures directly to the main model with the actual exit, output and partial effects. It never requests approval or replays a command automatically. The model inspects completed effects and submits only remaining work with the required `additional_permissions`.

Before execution, the host validates and freezes the actual command, canonical paths, file/directory scope and rule snapshot. Following user or existing reviewer approval, that command runs once inside a sandbox with precise temporary grants. Cards show the purpose, command, complete paths and each access/scope; allow once or reject. Hashes, inode identities and policy digests remain internal. No persistent-rule button is offered.

Explicit `escalated: true` plus justification remains a compatibility route: approval covers the whole command outside the sandbox once and cannot be combined with precise permissions. Failure diagnostics grant nothing. Historical `sandbox_failure` cards remain replayable, but the executor no longer emits passive retry approvals. Approval never establishes that an earlier script can safely be replayed.

### 4.2 Protocol and code index

`SandboxPolicy` proto fields 1–6 are reserved and not reused, `string tier = 7`.
The old three-modes × three-policies, command-prefix rules, and SQLite
`approval_rules`/`sandbox_config`/`approval_policy_config` are removed; the
three tables and `approval_config.rs` were cleaned up on 2026-07-05.

| Code (repo-root relative) | Responsibility |
|---|---|
| `agent/src/sandbox/{mod,rules,paths,backend}.rs` | tiers, rules, paths, PreparedShell and receipt |
| `agent/src/tools/mod.rs` | native tools, shell execution, timeout/cancel, retry |
| `agent/src/rpc/{approval,session_prompt}.rs` | approval requests, path diagnostics, session injection |
| `agent/src/rpc/commands/settings.rs` | policy/probe RPC |
| `packages/rpc/proto/future.proto`, `packages/thread-projection/src/approval.ts` | wire and shared approval projection |
| `desktop/src-tauri/src/{approval_rules.rs,commands/approvals.rs,agent_bridge/}` | saving, decisions, connection and fallback |
| `desktop/src/features/agent/ApprovalPrompt.tsx`, `desktop/src/integrations/agent/useSandboxAvailability.ts` | card and availability |
| `mobile/src/components/TimelineCard.tsx`, `mobile/src/remote/types.ts` | phone native card/remote data |

## 5. Progress, decisions, and follow-up plans

v2 file rules, read up-front approval, Seatbelt compilation, sensitive guards,
GUI saving and same-turn injection are implemented. Historical per-round test
counts from the v1/v2 work are neither the current total nor a current
acceptance pass, so they are not reproduced here.

Retained decisions: V1–V9 (2026-07) established pure path rules, open network,
per-lane fallback, file source of truth, three tiers, and project allows; V10's
"macOS display only" was superseded by V11–V14 (2026-08 Windows unelevated,
concrete capabilities, shared trusted UI, accepted identity/delete limits) and
the Linux L-D1–L-D11. Conflicting old state sections are no longer kept.

Recent focus: native re-verification of current platform versions, security
review, keeping documentation promises consistent with executors; per-platform
acceptance checklists are in the platform documents. Explicitly not doing for
now: command-prefix persistent rules, network approval/domain filtering,
auto-review agent, a general MCP/new-tool sandbox spec, and a full user-rule
editor in settings. The manual read-only whitelist is a no-friction mechanism,
not static analysis of arbitrary shells; after approval, `git push --force`,
`npm publish`, etc. can still execute.

### 5.1 macOS/Linux single-command execution_grants

`additional_permissions` has read/write arrays of `{path, scope, reason}`, at most eight literal targets total. file means an existing regular file; subtree means an existing directory and descendants. Creating a file requires its existing parent directory; use approved write/edit when sensitive carveouts cannot be expressed. Filesystem/home roots and wildcards are rejected. Unsupported OS scopes fail before approval.

Receipts bind the tool-call ID, command hash, cwd, policy digest, canonical targets, access/scope and target inode. Preparation is repeated before execution, the receipt must match exactly and is consumed once. Session rules are not mutated. Only originally-ask targets receive grants; declaring an already-allowed parent cannot unguard its children. All explicit denies remain above execution_grants, which precede secret ask guards and session/workspace/user rules.

macOS uses literal/subpath SBPL; Linux reuses narrow mount/reopen compilation and private reports. Write grants retaining a read denial, and child grants inside opaque mode-000 directories, are rejected before approval; use approved native file tools for those files. Windows retains its write capability, without shell read grants or a promise to reopen ask carveouts under ACL deny-wins. Failures never replay automatically on any platform. No arbitrary script-line continuation or rollback is promised.

## 6. References and historical retrieval

References only explain the source of trade-offs; they do **not** mean
FutureOS has implemented all of the reference project's capabilities. The local
Codex research snapshot is `~/workspace/codex` @ `f20b63e85c` (2026-09-02/04);
later reviews should record the new commit.

| Codex repo path | Notes and FutureOS choices |
|---|---|
| `codex-rs/sandboxing/src/manager.rs` | PermissionProfile, platform selection, helper seams; FutureOS keeps its own RuleSet |
| `codex-rs/sandboxing/src/bwrap.rs` | system search, userns probe, WSL; FutureOS does not support / specifically detect WSL |
| `codex-rs/linux-sandbox/README.md`, `src/launcher.rs` | system/bundled, capability detection; FutureOS is system-only, no download or bundling |
| `codex-rs/linux-sandbox/src/bwrap.rs` | same-root glob grouping, narrow reopen, missing roots, writable symlinks; FutureOS has an internal no-follow walker, no wholesale adoption of external rg/files-only/globset |
| `codex-rs/linux-sandbox/src/linux_run_main.rs`, `landlock.rs` | outer/inner, PID 1, cap/no_new_privs, seccomp/network policy, synthetic target cleanup; FutureOS does not adopt the Landlock fallback or host placeholder + cleanup |
| `codex-rs/vendor/bubblewrap/{bubblewrap.c,utils.c}` | tmpfs setup's ensure_dir/mkdir: a mount namespace does not isolate writes to host directory contents |
| `codex-rs/core/src/tools/{orchestrator,sandboxing}.rs` | typed SandboxErr::Denied and retry after approval; not the same as auto-degradation on any initialization failure |
| `codex-rs/sandboxing/src/{violation,denial}.rs`, `codex-rs/cli/src/doctor/sandbox.rs` | diagnostics and rejection judgment |
| `codex-rs/windows-sandbox-rs/src/{token,desktop}.rs` | legacy unelevated SID compatibility, private desktop; an elevated User SID belongs to a separate sandbox account and cannot be added directly to FutureOS's restricting set |

Codex's old-package compatibility includes using the executable path when
`--argv0` is unsupported, and using `/proc/self/fd/N` with an inner re-check
when `--ro-bind-fd` is missing. FutureOS explicitly re-executes subcommands and
does not rely on argv0; the FD-backed mount re-check is a primary-path security
measure, not a removable compatibility burden.

Upstream references (kept at review-time versions, not claimed "latest"):

- [OpenAI Windows sandbox design](https://openai.com/zh-Hans-CN/index/building-codex-windows-sandbox/): identity differences between unelevated and separate-user/elevated.
- [Bubblewrap v0.9.0 argument parsing](https://github.com/containers/bubblewrap/blob/v0.9.0/bubblewrap.c#L1527), [v0.11.1](https://github.com/containers/bubblewrap/blob/v0.11.1/bubblewrap.c#L1637): `--args` only recursively parses OPTIONS, with a combined 9000-argument limit.
- [GHSA-pxhw-h44j-8pfx](https://github.com/containers/bubblewrap/security/advisories/GHSA-pxhw-h44j-8pfx): old documentation recorded that 0.12.0 fixed setup absolute symlink traversal; 0.9.0 is the compatibility floor, not a security-patch guarantee. FutureOS has removed the missing-target creation path, but cannot claim all upstream risks are gone from that.

Migration index: the old APPROVAL_PLAN and SANDBOX_PLAN shared content is
consolidated here, Seatbelt/Windows sections went to the platform documents;
the five LINUX_SANDBOX documents merged into LINUX.md;
WINDOWS_SANDBOX_REAL_MACHINE_VALIDATION merged into WINDOWS.md. Old per-round
diffs, superseded designs, and the original long acceptance tables can be found
in git history; the current four documents keep the effective contract, key
trade-offs, and evidence indexes, and no longer maintain old drafts in
parallel.

## Single-command shell results (2026-10-09)

The shell input retains `command`; there is no `steps` protocol or host command
splitting. The model chooses whether to use one script or multiple tool calls.
Execute dependent calls in order and inspect each result. Scripts depending on
shared `cd`, variables, functions or
control flow remain one command. Each call starts a fresh host shell in the
project workspace with the existing inherited environment and PATH/PWD overrides;
changes in a previous process do not carry into the next call. Shell semantics
are preserved: no global `set -e` or pipeline changes are injected.

A complex script exposes only its whole process exit status. An earlier failure
may be masked by a successful final command. A later nonzero query does not
prove a previous action failed. Exit 0 means normal process termination, not
completion of the user's task. Model guidance requires interpreting output and
checking partial effects before requesting a retry. The host does not claim to
observe operations inside Node/Python programs or to guarantee exactly-once
execution. Output keywords alone do not override a successful process exit or
trigger automatic escalation.

The host captures `ShellResult` before generating model text. It contains the
command, cwd, total duration, status, nullable exit code, existing normal-query
verdict, approval outcome, note and ordered execution attempts. Attempts retain
actual status, nullable exit code, duration, merged output, truncation and
escalation flags. New recovery calls do not replace original facts; historical two-attempt results remain readable. `exited`, `launch_failed`, `execution_failed`, `timed_out`,
`cancelled` and `not_started` distinguish process outcomes and approval stops.
No missing exit code is replaced with zero; stdout markers cannot replace
process status. Bare grep/test-style exit 1 keeps the existing normal-query
semantics, while its actual code remains visible.

Approval review and platform restrictions remain unchanged: Jev's three numeric
Choice questions, matrix, confidence thresholds, four sourced evidence classes,
investigation budgets, cancellation, context validity and Linux helper checks
still apply. New approval covers the actual operation prepared by the model. Approval permits execution; it does not establish safe replay. The model must inspect partial effects and submit only remaining work. Network behavior is unchanged.

Inputs are bounded to 65,536 command bytes and 131,072 serialized bytes. The execution budget is 1–600 seconds (default 120), starting at the shell handler; pre-execution approval uses its existing lifecycle. Each invocation has one process attempt with up to 250,000 retained output bytes. No passive approval or retry budget remains. Serialized facts are capped at 1 MiB, reducing retained output
when JSON escaping requires it. Model text reserves at most 60,000 bytes for
output and retains status outside that budget, below the transcript's existing
100K-byte cap. Timeout and cancellation preserve captured partial output.

`tool_end.shell_result` crosses typed RPC and is stored in the canonical SQLite
tool-message metadata. Tool inspection, history, run replay and remote lean
transport retain the same facts. Desktop and mobile thread views consume host
verdicts while retaining their existing tool rows, grouping and summaries. The
run inspector also retains its existing presentation; no new execution or
retry detail widgets are added. Structured process facts remain available to
the host and model and do not prove the outcome of the user's task.
Old records without facts keep their existing display; attempt history is never
invented. Legacy `command` calls remain compatible; unsupported `steps` input
is rejected before approval or execution.

The default prompt separates `Tool Execution`, `Approval Feedback` and `User
Communication` into Markdown templates under `agent/src/prompt/`. Per-tool
rules are appended only to execution; additional caller guidelines follow the
three behavior sections. Project context and append overrides keep their
later position, and a custom prompt still replaces the default identity and
behavior sections. Jev's review questions remain a separate prompt.

The default identity describes a task assistant in FutureOS rather than a
coding-only role. Communication leads with the result, adapts depth to the
request, uses plain language, and includes technical details only when useful
for the answer, evidence or a decision. Simple actions usually need a brief
confirmation and the relevant result or file link; longer work gets meaningful
progress updates and a self-contained final answer. Routine verification is
summarized, without a required code walkthrough, warning checklist or extra
approval question.

This borrows global communication and task-completion principles from the full
local Codex `gpt-6.1-sol` instruction template in
`codex-rs/models-manager/models.json` at `e974aad3b1` (2026-10-08). That template
scales explanation to the request; it does not specifically ban code examples
or require modification and verification to be separate commands. FutureOS
keeps its own tools, file-link format, memory limits and approval policy rather
than importing Codex's app channels or permission model. Prompt wording may
influence verbosity, but source comparison alone does not establish its cause.

Execution rules preserve earlier user authorization within the same task and
scope, respect later restrictions, and require concrete preparation before a
necessary user decision. Approval feedback cannot grant new user authorization.
After denial, changing tools, interpreters or wrappers to achieve the same
rejected effect is prohibited; a genuinely narrower permitted alternative must
still pass normal checks. Shell guidance is split into short rules under
`agent/src/tools/shell/guidance.rs`. It leaves execution-unit organization and
diagnostic status printing to the model, while requiring interpretation of
the actual process status, output and intended effects.
Known file contents prefer `read` when available, without banning shell reads.
Platform hints describe the shell wrapper's syntax and actual version limits;
they do not prohibit running programs or scripts with their own syntax.
The RPC prompt builder no longer adds a conflicting blanket prohibition on
shell file writes. Ordinary file changes prefer write/edit; an explicitly
requested script or command-line method uses shell under the existing checks.
Later user instructions can supply new authorization for fresh review, but do
not bypass policy or revive an old approval.

Execution metadata is internal diagnostic information. Routine approval and
retry recovery are handled through tools without narrating them. Successful
recovery is reported as the task result, without recounting the recovered
failure, sandbox restriction or approval process. An approved tool result does
not establish that the user personally confirmed it. User replies
report the task outcome and useful verification; exit codes, shell mechanics
and approval traces are explained only for requested debugging or an unresolved
blocker/decision. Failures, partial completion, uncertainty and meaningful side
effects must still be reported in plain language. Model text retains original/
retry evidence but does not repeat generic exit-zero and successful approval
caveats on every call; approval notes remain in structured audit records.
These are model guidance rules, not a deterministic filter on generated replies.

Regression tests cover independent action/verification calls, unchanged complex
script semantics, stdout status spoofing, nullable exit codes, timeout,
cancellation, bounded output, approval denial/context expiry, failure without replay, isolated single-use grants and typed/persisted/lean/live/replay results. Tests are compiled
locally, with execution and native cross-platform validation left to CI.
