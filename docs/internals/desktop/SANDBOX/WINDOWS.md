# Windows: Unelevated write protection

> ([中文](WINDOWS.zh-CN.md)) Updated: 2026-09-04. W1–W7 backend, approval,
> maintenance, and product integration are implemented; Windows 11 ordinary
> users have native, GUI core-write, installer lifecycle, and multi-host
> historical PASSes. **This is not a read/write sandbox equivalent to Seatbelt,
> nor Elevated separate-user isolation.** This page unifies implementation,
> trade-offs, real-machine operations, and evidence; shared rules/UI/references
> are in [COMMON.md](COMMON.md).

## 1. Scope and design choice

Current supported baseline: Windows 11, ordinary non-admin user, absolute local
NTFS paths, PowerShell 7 preferred / 5.1 fallback. No Git Bash/WSL assumption,
no local account creation, no UAC trigger, no firewall, no shell deny-read.

| Candidate | Trade-off |
|---|---|
| WRITE_RESTRICTED + capability SID + NTFS ACL | current approach, no admin needed, read-compatible, accepts write-protection limits |
| Elevated separate user | needs provisioning/UAC, cross-user runner/read ACLs; future candidate, not a defect this cycle |
| Full restricted token + many read ACEs | breaks APPDATA/Cargo/Rustup/npm/Git/Python compatibility, widely modifies real ACLs; not adopted |
| Low-IL | would change real workspace integrity labels and may widen the write surface for other Low-IL processes; not adopted |
| WSL / minifilter | the former is not the current native approach; the latter needs admin, driver signing, and kernel maintenance; not implemented |

The product tier is named "write protection". The entry appears only after the
full host probe passes; no separate W7 switch. Shared rules still apply to
native read/write/edit; **the shell by default can read data readable by the
current user and exfiltrate over the network** — the sensitive guard list must
not be treated as a Windows shell read-deny guarantee.

## 2. Design and execution principles

```text
RuleSet → WindowsSandboxPlan
  → validate declared additional paths + optional approval receipt
  → policy/request capability records + active lease
  → persist metadata → apply own ACEs → reclaim inactive old generations
  → RestrictedToken + private desktop
  → CREATE_SUSPENDED → join Job → ResumeThread → shell/descendants
  → Job end / exit reset / next-start GC / uninstall cleanup
```

### 2.1 Rule projection and token

`windows_plan.rs` is a platform-independent pure plan:

| Field | Meaning |
|---|---|
| `writable_roots` | workspace, actual `temp_roots()`, literal allow-write roots; deduplicated by containment |
| `write_carveouts` | literal ask/deny write protections, original decision kept |
| `unenforced_read_rules` | read ask/deny not enforced by the shell, diagnostics only |
| `unsupported_write_globs` | write globs NTFS cannot express; native tools still follow rules |

The same matcher suppresses lower-priority duplicates but keeps partially
overlapping parent-child rules. NTFS deny-wins cannot express every first-match
exception: an explicit ask/deny carveout may still be denied even with a
narrower allow SID added — no claim of complete rule equivalence.

`CreateRestrictedToken(DISABLE_MAX_PRIVILEGE | LUA_TOKEN | WRITE_RESTRICTED)`
keeps the current real user principal. Restricted write access must pass both
the ordinary token and the restricting-SID check; it cannot add permissions the
current user did not already have. `SeChangeNotifyPrivilege` is restored for
path traversal.

The production restricting set is **capability + logon SID + Everyone, without
the real User SID**. PowerShell/CLR startup needs to write session/Everyone-
accessible kernel objects — capability-only really does fail with HRESULT
80070005; the real User SID would broadly hit user-file ACLs and break the
external write boundary. The compatibility-wide SIDs bring the existing-ACL
limits in §4. The token default DACL and the private desktop grant only the
current user + capability; new objects are not additionally opened to
Everyone/logon.

The capability SID is generated from a deterministic name as an
account-domain-shaped SID, used only as a FutureOS ACL trustee/restricting SID
— **not an AppContainer process, and no `DeriveCapabilitySidsFromName`-generated
AppContainer SID**.

### 2.2 SID generations, ACLs, and lifecycle

- The base identity is determined by "normalized write roots + effective policy
  fingerprint". The same generation can be reused; rule changes create a new
  generation and never modify deny ACEs still used by old Jobs.
- One approval creates a request-scoped ephemeral SID for the base write roots
  and each approved target; target identity binds request id/path with file or
  subtree scope — deduplication by parent root must not lose independent
  approved targets, and historical subtree ACEs must not be reused to widen a
  file approval.
- `~/.future/windows-capabilities.json` schema1 stores names, semantics, actual
  carveouts, and approved targets — no SID pointers; saved atomically in the
  same directory. Persist a cleanable record first, then add ACLs; failures
  leave metadata for recovery.
- `FrozenPath` accepts only absolute local NTFS, opens with
  `FILE_FLAG_OPEN_REPARSE_POINT`, and rejects reparse/device/UNC and other
  unsupported forms; the handle final path must match the frozen canonical
  path. Re-verify before applying ACLs — a path change invalidates the
  approval; one string canonicalization is not enough.
- `SetSecurityInfo` only idempotently adds/revokes FutureOS's own SID ACEs,
  preserves the rest of the DACL, never changes the owner and never overwrites
  the whole table. Existing literal protections get deny-write; NotFound
  targets are skipped; other errors return failure.
- Roots and descendants get write ACEs per scope; subtree descendants may have
  DELETE — **no FILE_DELETE_CHILD capability on parent directories**; a file
  gets content-write only, no DELETE. This does not remove the real user's
  existing delete permission on parent directories.
- Preparation is in-process serial; each child holds a lease, the first lease
  holds a byte-range file lock on the metadata directory. A process crash
  releases the lock via the OS; when this or another process has an active Job,
  GC/reset refuse revocation and never terminate commands. Holding a lock is
  not a cross-project permanent grant.
- Only the current generation/request SIDs go into new tokens; stale ACEs do
  not directly grant future tokens; GC reclaims them. Remove own SIDs with
  REVOKE_ACCESS; failures keep the metadata — the generation boundary must not
  depend on cleanup succeeding the instant a command ends.

### 2.3 Process, shell, and output

A UUID private desktop is created in `Winsta0` with ACL current-user +
capability; no custom window station, which ordinary users cannot reliably
create. `CreateProcessAsUserW(CREATE_SUSPENDED)` with a STARTUPINFOEXW handle
allowlist inherits only stdio; ResumeThread only after joining a no-breakaway,
KILL_ON_JOB_CLOSE Job. Initialization failure terminates the suspended process
first — the command must not run.

Normal shell exit also cleans up residual descendants; the unsandboxed detached
browser's BREAKAWAY_OK/disarm logic is not reused. cwd/env/Unicode command
lines, wait/timeout/cancel all go through the restricted driver.

Restricted PowerShell 5.1 enters Constrained Language Mode. UTF8Encoding
construction, Console.OutputEncoding setters, or .NET byte writes are not
reliable solutions; the wrapper's encoding setup is put in a try, and command
errors are judged by `$Error.Count` increments, so initialization encoding
exceptions do not pollute a successful exit.

Capture side `decode_restricted_shell_output`: 5.1 uses
`MultiByteToWideChar(CP_OEMCP)`, pwsh7 uses UTF-8. Without a console, 5.1 pipes
use the OEM fallback, not CP_ACP; Chinese is 936 in both, masking the problem —
Western European 1252 vs 437/850, Russian 1251 vs 866, Greek 1253 vs 737
differ. Characters beyond the OEM codepage already encoded as `?` cannot be
recovered at decode; no claim of lossless arbitrary Unicode on 5.1.

### 2.4 Probe, maintenance, and app ownership

The host probe actually starts a private-desktop restricted shell in a
temporary NTFS fixture, verifies allowed writes succeed and adjacent
normal-user-writable paths are denied, and cleans up ACEs. It is not just token
creation, nor proof that every host directory's ACL is safe. The probe waits at
most 10 seconds; a timeout terminates the whole no-breakaway Job and releases
the capability lease, avoiding background tasks holding locks after an RPC
timeout. Stable results include available or `backend_initialization_failed`,
`write_boundary_failed`, `restricted_shell_failed`, `probe_timeout`; internal
Win32 diagnostics stay in local logs, not the ordinary UI.

```powershell
future agent --probe-windows-sandbox
future agent --reset-windows-sandbox
```

The common `--probe-sandbox` also works. The probe prints JSON on successful
execution; available:false is a supported unavailable result. Reset returns
`removedCapabilities`; an active Job fails and keeps the metadata — no
privilege elevation, no killing tasks. Maintenance CLI/RPC has no session and
is callable before the singleton.

The long-lived Agent holds the user-level lock
`~/.future/agent/agent-instance.lock`; changing the gRPC port does not bypass
it. Normal desktop exit, task convergence after confirmed force-quit, clear
data/environment switch/update restart all reset before terminating while the
bundled Agent is alive; external Agents are not cleaned up or terminated by
Desktop. The Agent's own Ctrl+C also resets idempotently. Timeout, crash, or
transient failure best-effort keeps metadata; startup GC, settings reset, and
uninstall continue reclaiming.

NSIS installMode is currentUser; the uninstall pre-hook cleans up under the
same user profile; a failed reset should keep the retryable sidecar rather than
deleting the cleanup tool first. No manual `icacls /reset` overwriting
unrelated ACLs.

## 3. Path capability approval

Windows cannot know the denied target from `Access is denied`/error 5 alone, so
there is **no whole-command re-run outside write protection**. The shell call
declares up front:

```json
{
  "command": "Copy-Item C:\\build\\artifact.zip D:\\release\\artifact.zip",
  "additional_permissions": {
    "write": [{"path": "D:\\release", "scope": "subtree", "reason": "create release artifacts"}]
  }
}
```

1. Parse and freeze canonical absolute paths; file accepts only existing
   regular files, subtree only existing directories. Reject volume roots, the
   user HOME root, invalid/unsupported paths. Creation, replacement, and rename
   must **explicitly request an existing parent directory subtree** — the
   backend does not silently widen.
2. The same RuleSet evaluates by write: allow skips prompting, fallback ask
   merges into the up-front approval, deny rejects directly. Explicit ask
   carveouts cannot be reopened via narrow SIDs due to deny-wins; they are
   rejected before the prompt — no promise of an "approval still fails" flow
   shown to the user.
3. At most 8 targets, decided as a whole group; over-broad or too-many requires
   the caller to rewrite the request — no automatic expansion of multiple
   children into a parent directory.
4. The backend trusted semantics show "modify file" or "create, modify, rename,
   and delete files in directory"; the title target matches the real scope. The
   model's reason does not decide scope. The ordinary UI shows no
   SID/ACL/hash/glob; commands are collapsed; an unparseable payload shows no
   approval.
5. The receipt binds request id, command hash, normalized paths, scope; rules
   and handles are re-verified before and after approval. Any add/remove/change
   requires a new request.
6. One approval is an ephemeral capability for this command only; project
   allows are saved by the trusted GUI as an allow-write for the same target
   with session injection, generating a new policy generation. Sensitive,
   hard-deny, config/rule paths, or unsafe/over-broad targets have no
   persistent allow. Phone v1 is once/deny only.
7. After approval the RestrictedToken still runs; the user's own privileges are
   not raised. After an undeclared external write fails, the model can be
   hinted to declare the path and retry; stderr inference only helps diagnosis
   and generates no grant. Fully opening up requires the user to explicitly
   switch to off.

## 4. Differences from macOS/Linux and accepted gaps

| Situation | Current Windows guarantee / limit |
|---|---|
| shell reading `.ssh`, models, `.env` | **not read-denied**, even where native tools would deny/ask; read + network exfiltration is an explicit non-goal |
| workspace/temp/literal allow writes | opened by capability ACEs, base roots from actual temp_roots; no independent per-session private temp promised |
| external content writes | denied by default on targets where the shape is supported and ACLs satisfy the boundary; a concrete capability can be approved, no privilege raise |
| existing literal ask/deny | deny-write ACE extra hardening, not the same as intercepting every delete/rename |
| missing `.env`/approval_rule or future names | no object to attach an ACE to; when the parent directory allows creation, only a future name cannot be blocked; **no Linux-style post-scan report** |
| globs | not enforced for shell read or write; fields are diagnostics only — diagnostics must not be read as protection |
| wide deny + narrow allow | deny-wins, stricter; priority exceptions cannot be fully expressed; not solvable by one more whole-command approval prompt |
| file scope and deletion | not granting DELETE is not forbidding deletion; the ordinary user's FILE_DELETE_CHILD on the parent directory may still allow deleting the target or a sibling |
| existing wide ACLs | write rights hitting the restricting SID via Everyone/logon etc. can weaken the external-content-write boundary; delete rights of ordinary principals like Users/Authenticated Users on parent directories must also be reviewed — no claim every external object is decided by capability alone |
| state and host modification | modifies real NTFS ACLs, needs metadata/lease/GC/reset/uninstall; macOS profiles and Linux mounts have no such persistent ACL lifecycle |

A real-machine AccessCheck once showed an external directory with
FILE_DELETE_CHILD=true but DELETE/FILE_WRITE_DATA=false, and Remove-Item then
succeeded. This is a known limit of the WRITE_RESTRICTED model — tests must not
be changed to advertise "file approval absolutely forbids deletion"; nor does
this demand adding Elevated this cycle.

Network/read and other mode differences are explained in the product docs,
first-time-enable/settings, and developer materials; concrete-path approval
highlights only current behavior + target. The existing-wide-ACL trade-off must
be noted in test records; one temporary fixture's external-write-deny PASS must
not be extrapolated into a host-wide no-bypass claim.

## 5. Progress, code map, and historical pitfalls

W0 contract freeze, W1 pure plan, W2 token/ACL/capability, W3 restricted
driver, W4 bound approval, W5 shared UI/phone, W6 lifecycle/diagnostics, W7
dynamic entry are all landed; the full chain ships together — no entry without
enforcement, no ACEs without reclaim.

| Code (repo-root relative) | Responsibility |
|---|---|
| `agent/src/sandbox/windows_plan.rs` | platform-independent projection and difference fields |
| `agent/src/sandbox/windows/{capability,token,acl,audit}.rs` | generation/request identity, token/SID, own ACEs, handle verification |
| `agent/src/sandbox/windows/{process,runner}.rs` | private desktop, Job, shell, lease, persistence/GC/probe/reset |
| `agent/src/tools/mod.rs`, `agent/src/rpc/approval.rs` | additional_permissions pre-check, receipt and execution |
| Desktop supervisor/shutdown, NSIS hooks | bundled ownership, reset-before-terminate, currentUser uninstall |
| `scripts/test-windows-sandbox*.ps1` | native and installer lifecycle acceptance, not in CI |

Key conclusions from 2026-08-21~24 native troubleshooting:

| Symptom | Root cause / existing fix |
|---|---|
| CreateWindowStationW error 5 | ordinary users cannot use a custom station; switched to a UUID private desktop in Winsta0 |
| CLR HRESULT 80070005 | capability-only insufficient; added logon/Everyone for compatibility, no real User SID |
| successful command exit 1, CLIXML encoding error | CLM forbids constructing/setting encodings, polluting $Error; try + error increments, capture-side OEM decode |
| Remove-Item still succeeds after file approval | FILE_DELETE_CHILD limit; keep the known boundary, no fabricated delete-proof conclusion |
| Chinese fine but Western scripts garbled | CP_ACP misuse; 5.1 changed to CP_OEMCP, 7 uses UTF8; already-lost characters unrecoverable |
| probe production lib E0433 | tempfile was a dev-only dependency; added cfg(windows) production dependency |
| singleton test unlocked, wrote to real user dirs | dirs ignored test HOME; unified absolute non-empty HOME→USERPROFILE→system profile, consistent lock/credential/rule/workspace/capability |
| Desktop lib-test crash 0xc0000139 | TaskDialogIndirect needs comctl32v6; Tauri only gave the bin a manifest; build.rs embeds v6 for all targets, verified with mt.exe |
| false failures in Windows lint/log tests | Rust 1.97 lint fixes, cfg on Unix-only imports, log smoke pins RUST_LOG=info |
| PS5.1 lifecycle Snapshot empty-array null | `if` pipeline flattened arrays; `41d458b3` explicitly initializes arrays |

2026-09-14 path audit (code-review conclusions, no new real-machine
reproduction):

| Symptom | Root cause / existing fix |
|---|---|
| `\\?\C:\...` in approval cards/persisted rules | `windows_request` used `Path::canonicalize` directly; changed to `paths::canonicalize_existing` keeping ordinary spelling, comparable with rule-layer paths |
| case-variant bypass of literal rules (e.g. self-created `.future\APPROVAL_RULE.JSON`) | `path_within` compared bytes on Windows while globs already ignore case; changed Windows to ignore ASCII case like macOS |
| ask carveout targets not rejected at pre-check | the two items above made `reject_explicit_ask_carveouts` comparisons fail (fail-closed but would prompt approval then fail); restored after ordinary spelling |

## 6. Native acceptance operations

Must run in PowerShell as a Windows 11 **non-administrator**; workspace and
TEMP on local NTFS; Rust per repo toolchain, MSVC C++ Build Tools/SDK
installed. Close FutureOS and any self-managed Agent; keep the user's dirty
files — no git clean/reset. Record candidate commit + git status; **do not
switch back to a merged historical branch per old documents**.

```powershell
git rev-parse HEAD
git status --short
$identity = [Security.Principal.WindowsIdentity]::GetCurrent()
$principal = New-Object Security.Principal.WindowsPrincipal($identity)
$principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
rustc -Vv
cargo -V
powershell -ExecutionPolicy Bypass -File .\scripts\test-windows-sandbox.ps1 -IncludeClippy
```

IsInRole must be False; the script rejects admins/non-NTFS TEMP;
`-AllowElevated` is diagnostics only, not product acceptance. Native tests
force single-threading, contain no UI automation, and are not in CI; save the
full log to `target\windows-sandbox-results\windows-sandbox-<time>.log`.

Coverage: token/SID/ACL/reparse/UNC/Job/PowerShell/Unicode/large output,
approval request/hash/path/scope, user-level singleton/force-kill lock
recovery, Desktop exit ordering, release unified CLI probe, Agent/Desktop
Clippy, capability zeroing. Tauri tests only create test placeholders when the
sidecar is missing, and at the end delete only files the script itself created
— never overwrite an existing real sidecar.

PASS requires every command exit 0, ending with both
`Remaining persisted Windows capability records: 0` and `RESULT: PASS`.
`RESULT: UNSUPPORTED` is a supported fail-closed unavailability, not a PASS;
other test/cleanup/probe failures record FAIL and keep the scene — no manual
ACL edits first. Do not read only the last line.

### 6.1 Installer lifecycle (RM-01~07)

Using the same candidate portable and NSIS:

```powershell
powershell -ExecutionPolicy Bypass -File .\scripts\build-desktop-windows-portable.ps1 -SkipDeps
powershell -ExecutionPolicy Bypass -File .\scripts\build-desktop-windows-installer.ps1 -SkipDeps
powershell -ExecutionPolicy Bypass -File .\scripts\test-windows-sandbox-lifecycle.ps1 -Action Snapshot
```

Portable is the root-level `FutureOS-portable-windows.zip`, keeping
FutureOS.exe/future.exe in the same directory; NSIS is under
`desktop\src-tauri\target\release\bundle\nsis\`. Each lifecycle call keeps an
independently timestamped log; full command lines are not recorded.

`-Action SeedCleanupFixture` only writes legal test metadata pointing at
`%TEMP%\FutureOS-Sandbox-Lifecycle-Fixture` when record is 0 — no ACEs added,
no real records overwritten. It proves the app/installer reaches reset, not by
itself that real ACEs were revoked (the native tests cover that). Each reset
scenario seeds first, expecting 1→0; after interruption with no active tasks,
the CLI reset can recover.

| ID | Operation and lifecycle Action | Must observe |
|---|---|---|
| RM-01 | ExpectClean→start Desktop→start again→ExpectBundled | one Desktop/one Agent per user, Agent parented to Desktop |
| RM-02 | ExpectBundled→SeedCleanupFixture→normal exit, wait up to 20 s→ExpectStopped | both processes exit, record 1→0 |
| RM-03 | seed separately then clear-data restart, dev-environment-switch restart, post-update restart→ExpectRecovered | old Agent exits, new Agent unique and correctly configured, port released, record 0; NOT RUN when untriggerable |
| RM-04 | manual `future.exe agent --grpc-addr 127.0.0.1:50051`, Desktop connects→ExpectExternalAttached→seed→exit Desktop→ExpectExternalSurvives | address matches FUTURE_AGENT_GRPC_ADDR; external Agent and record 1 kept; after its Ctrl+C ExpectClean back to 0 |
| RM-05 | bundled+seed→Task Manager force kill→Snapshot→restart→ExpectRecovered | force kill promises no synchronous cleanup; singleton recoverable, startup GC back to 0; active leases not wrongly revoked |
| RM-06 | ExpectClean→seed→real release CLI `agent --reset-windows-sandbox`→ExpectClean; settings reset | same primitive, outputs removedCapabilities; active Jobs refused and not killed |
| RM-07 | ordinary-user NSIS, exit/ExpectClean→seed→uninstall→ExpectClean | metadata cleaned, install dir removed, fixture/user data and unrelated DACLs kept; a failed reset does not delete the sidecar first |

### 6.2 Product and multi-host matrix

Each host records edition/build/arch/ordinary-user/NTFS/TEMP/shell/artifact/
Defender state. Minimum coverage: Windows 11 Home + Pro, PowerShell 5.1 + 7,
ASCII and Chinese usernames/workspaces, source debug + portable + NSIS,
Defender default-on; admin is diagnostics only.

| ID | Product check |
|---|---|
| W7-01 | probe available, Desktop option visible, only one bundled Agent |
| W7-02 | Desktop/paired phone consume the same Agent capability, semantics consistent |
| W7-03 | workspace writes succeed, unapproved pre-existing external target content write fails |
| W7-04 | card behavior + complete targets, rejected target unchanged first, then one approval only the needed content write succeeds |
| W7-05 | a file approval does not widen parent-directory/sibling content writes; another file needs new approval/denial |
| W7-06 | the external Agent's probe is authoritative; Desktop does not start/take over another |
| W7-07 | restart keeps sandbox after probe success; clearly unavailable falls back to manual/hides the Windows entry; transient connection faults follow common rules |
| W7-08 | normal exit ExpectStopped, record 0, user files/unrelated ACLs preserved |

W7-03~05 check content writes, not an absolute anti-delete guarantee; wide ACLs
and FILE_DELETE_CHILD are recorded per §4. Prepare controlled targets before
enabling:

```powershell
$outside = Join-Path $env:USERPROFILE "Desktop\FutureOS-W7-Outside"
New-Item -ItemType Directory -Force -Path $outside | Out-Null
Set-Content -LiteralPath (Join-Path $outside "approved.txt") -Value "before"
Set-Content -LiteralPath (Join-Path $outside "sibling.txt") -Value "before"
```

After rejection still "before", then approve only approved.txt; reset/uninstall
must not delete these two user test files.

Report template: commit+dirty state, Windows edition/build/arch, User
elevated=False, workspace/TEMP NTFS, shell versions, package hash, Batch
PASS/UNSUPPORTED/FAIL, per-item RM/W7 PASS/FAIL/NOT RUN, remaining records,
logs/screenshots, accepted limits. Status files may contain paths — do not
paste full JSON publicly.

## 7. Historical verification evidence (not an automatic PASS for the current candidate)

| Date / commit | Result and evidence |
|---|---|
| 2026-08-24 `a55c558200a80d2c2008c6ee2ef0c0c0ce86aa8e` | NT10.0.26200 AMD64, ordinary user, NTFS, PS5.1, Rust 1.97 MSVC; native 50, capability 11, singleton 1, Desktop shutdown 2, Agent/Desktop Clippy, release probe pass, record 0. Log `target/windows-sandbox-results/windows-sandbox-20260824-102215.log` |
| 2026-08-24 11:21 `471a8cd79da99c3186a88448a0b685c0130cb2e4` | clean workspace, Agent home C:\Users\FgClaw01; the behavior matrix above re-verified PASS, record 0. This report had no IncludeClippy; the same commit's Agent dev-side Clippy passed separately |
| same-host SID experiment | capability+Everyone starts, capability+logon does not; evidence for this host only — do not delete the production compatibility SIDs based on it without the matrix |
| later packaged RM-01~07 | original draft records all PASS: singleton/exit, three restarts, external ownership, crash startup recovery, CLI reset, real NSIS uninstall; RM-05 force-kill was already 0 at the time; after stopping, manual seed then restart was 1/1/0 |
| P2 multi-host | original draft records Pro/PS7/Chinese username & path/portable matrix complete; per-item commits and logs not attached — this pass fabricates no evidence; release re-verification should backfill |
| 2026-08-24 GUI Home | probe/entry, workspace content write, external content write denial, normal-exit record 0 pass; corresponds to the core of W7-01/03/08, not covering all scope buttons, phones, restart fallback, or siblings |

The old text's opening "all complete" and later "some product items pending
verification" must not be conflated: the exact low-level logs and the original
draft's later PASS records are kept, but full product evidence for
W7-02/04/05/06/07 still needs item-by-item verification. This documentation
pass ran no tests on Windows.

## 8. Follow-up plan and release requirements

Release still requires: candidate Home/Pro batch PASS, all applicable RM pass,
W7 scope/phone/fallback supplementary evidence, no high-priority security
issues, unsupported paths fail closed stably, ACL upgrade/exit/crash/reset/
uninstall recoverable. An open entry does not substitute for a security review.

| Priority | Work |
|---|---|
| P1 | complete native and product-interaction re-verification of the current candidate, keeping per-host/commit evidence; review wide ACLs and the minimal compatibility SID set |
| P1 | continuous audit of reparse/handle changes, receipt tampering, active lease/cross-process GC races, guaranteeing failure does not Resume / not run bare |
| P2 | independent design and compatibility evaluation of a single-dedicated-user candidate; not a commitment this cycle |
| Accepted | shell read/network open, globs and future filenames not enforced, deny-wins, real-user delete rights, existing wide ACLs, PS5.1 codepage loss |

### 8.1 Single dedicated user candidate (not implemented)

If product goals upgrade to an independent security principal, first evaluate a
Codex Elevated slim variant: a one-time elevated setup creates an ordinary local
user (working name FutureSandbox) and a dedicated group; random credentials
saved with DPAPI and unreadable by the sandboxed user; the real-user Agent
starts a cross-user runner via CreateProcessWithLogonW, which then creates a
WRITE_RESTRICTED token and a full Job under that user.

Workspace/temp need both ordinary access for the dedicated group and the
capability second ACE, continuing not to give the parent directory capability
FILE_DELETE_CHILD; the toolchain needs the necessary read/execute with
sensitive directories excluded. The real user's owner/parent-directory delete
rights no longer automatically match, but wide ACLs like
Everyone/Users/Authenticated Users are still not absolutely safe.

With an open network the candidate needs only one user — no wholesale adoption
of Codex Online/Offline dual users and firewalls; WFP is evaluated only for
future network isolation. It must separately cover UAC, account hiding,
credential secrecy, cross-user IPC, ACL refresh, upgrade/reset/uninstall
recovery, and must not be mixed into Unelevated maintenance.

PowerShell text parsing (cannot cover arbitrary subprograms), Low-IL (changes
real workspace trust labels), and minifilters (driver install/signing/
maintenance cost) are not preferred substitutes for this identity boundary.
