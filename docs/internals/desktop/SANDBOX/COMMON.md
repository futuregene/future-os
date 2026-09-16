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
| Extra authorization | whole command out of sandbox once | whole command out of sandbox once | declares concrete write capability, still restricted after approval |
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

### 4.1 Active and passive de-sandboxing (macOS/Linux)

| Trigger | payload trigger | i18n key | English title |
|---|---|---|---|
| model explicit `escalated: true` + justification | `model_request` | `approval.escalationRequestTitle` | Model requests running this command outside the sandbox |
| sandboxed command failed and matched the rejection classification | `sandbox_failure` | `approval.escalationRetryTitle` | Running this command outside the sandbox is needed |
| historical request with missing/unknown trigger | fallback | `approval.escalationTitle` | Run this command outside the sandbox |

What is approved is the **whole command executing outside the OS sandbox
once** — not only access to the card's path; the global tier is unchanged.
Rejection means that de-sandboxed execution does not run. A first failure may
already have had side effects; re-check results before re-running.

The model prompt defaults to trying inside the sandbox first, not requesting on
guesses; approval is requested only when execution results show a necessary
operation was blocked by the sandbox, retrying only the blocked operation where
possible. Necessary operations must keep the real failure status and error
output — never hide failures with `|| true`, forced success, or swallowed
output; clearly ignorable failures may be tolerated but are not treated as
success. This is a prompt constraint, not a new execution gate: an overall exit
code of 0 still does not trigger passive approval, and no new permission-error
hint after successful exits was added.

The diagnostic path is display inference only: extract absolute paths from
`Operation not permitted`, Linux `Permission denied`/`Read-only file system`/
`Device or resource busy`, supporting quotes, spaces, and
`program: line N: /path: error`; dedupe preserving order, at most 5 items. No
guessing relative cwd, no treating URL/helper initialization errors as targets,
no guarantee all languages/programs' errors parse. An active request without
failure output may have no path. Display parsing does not change the escalation
decision or grant scope, nor expand the sources of persistent-rule suggestions.

Windows does not use this whole-command approval; see
[path capabilities](WINDOWS.md#3-路径-capability-审批).

Linux treats `Device or resource busy` (EBUSY) like other permission errors as a
text heuristic for passive approval, without requiring a command form or a
successfully parsed path; the private completion report must still allow
retries. This is not kernel-rejection certification; ordinary device busy can
also falsely trigger approval, and swallowing error output can miss cases. See
[Linux detection and retry](LINUX.md#34-检测报告与重试可信度).

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

2026-07-04 v2 R1/R2/R3 complete: file rules, read up-front approval, Seatbelt
compilation, sensitive guards, GUI saving and same-turn injection. Historical
results: R1 Agent 55 lib + 10 rules + 9 smoke; R2 GUI/frontend 39; R3 Agent 58
lib + 9 smoke, GUI 72, frontend 39, lint/check-desktop pass. The early v1's
Agent 67, GUI 69, frontend 39, smoke 9 are only the old architecture baseline,
not the current total test count.

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

### 5.1 macOS/Linux phase-two execution_grants (not implemented)

Bind concrete read/write paths, access, scope, command hash, and request id to
a single approval, and recompile a plan that is **still inside the OS sandbox**.
Cannot simply reuse session allows: they are lower than guards. Candidate
priority: hard deny > approved execution_grants > secret ask guards >
session/workspace/user; only accept grants for originally-ask decisions; any
deny and layer 0 are not overridable.

Seatbelt can add temporary literal/subpath allows; Linux needs to prove
mount/reopen can express the same scope. Windows's request capability provides
protocol experience but cannot be copied wholesale with its non-read-deny /
ACL-deny-wins limits. Freeze the cross-platform model before changing the UI;
if whole-command de-sandboxing is kept, it should be an explicit advanced
option. This proceeds together with macOS and is not mixed into Linux phase
one.

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
