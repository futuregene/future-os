# CLI authorization and Loop tool refactor

Date: 2026-09-21

Status: development design draft. Interfaces, delegated authorization, and security fixes described here are targets, not implemented or verified features.

[中文](CLI_REFACTOR.zh-CN.md) · [Remote execution design](REMOTE_EXECUTION_DESIGN.md)

## 1. Purpose and decisions

This project precedes remote execution. It first closes the local sandbox exception introduced to let skill CLI processes read account credentials, while establishing one CLI architecture for Local and Remote execution.

The two core goals are:

1. CLI processes request restricted, temporary operation authorization from the trusted Agent instead of reading account keys. Local connects directly; Remote reaches the same Agent authorization service through the runner and the existing SSH control connection.
2. Loop becomes a native Agent tool alongside read/write/edit/shell. A skill teaches the workflow; the model invokes the tool directly instead of orchestrating itself through `future loop` shell commands.

“Get a key from the Agent” means obtaining a **platform-recognized, short-lived credential restricted to an operation and its permissions**. There is no `get_key(provider)` API returning an account or model master key. Returning the same master key through a socket would move the exposure without fixing it.

Command parsing, request normalization, authorization, service calls, file handling, and output semantics are shared. The full local distribution and lightweight remote distribution need not expose identical administrative commands. Packaging the CLI with the Agent does not mean sharing process memory, credentials, or privileges with CLI subprocesses.

## 2. Current evidence

| Current behavior | Evidence | Required change |
| --- | --- | --- |
| Sandbox hard-deny temporarily excludes `agent/auth.json` and legacy `agent-app/auth.json` for skill CLI compatibility | [rules.rs](../../agent/src/sandbox/rules.rs), [sandbox contract](../internals/desktop/SANDBOX/COMMON.md) | Restore read/write denial after authorization works; executable names cannot establish path exemptions |
| Cloud tools read `FUTURE_API_KEY`, then auth.json, with a test-key fallback; account commands have a separate file reader | [tools.rs](../../cli/src/commands/tools.rs), [account.rs](../../cli/src/commands/account.rs) | Use one runtime authorization client and remove production secret fallbacks |
| `future auth credential` prints the account key | [auth.rs](../../cli/src/commands/auth.rs) | Restricted CLI must neither obtain nor export master keys |
| Unified CLI depends directly on Agent/TUI/channel/Loop; ordinary commands also use Agent path helpers | [Cargo.toml](../../cli/Cargo.toml), [main.rs](../../cli/src/main.rs) | Separate shared runtime from full distribution entry points |
| Builtin tools currently consist of read/write/edit/shell; definitions are collected at run start and enumerated in the prompt | [tools](../../agent/src/tools/mod.rs), [run loop](../../agent/src/agent/run_loop.rs), [prompt](../../agent/src/prompt/mod.rs) | Add Loop handler and skill-triggered schema activation |
| Loop is already a library, but orchestration remains concentrated in console and validators launch the host shell | [library](../../orchestration/loop/src/lib.rs), [console](../../orchestration/loop/src/console.rs), [validator](../../orchestration/loop/src/validator.rs) | Extract shared services; do not wrap console or validate remote projects on the controller |

These are static source findings. This design work did not extract credentials, execute an attack, or exercise a real platform authorization endpoint.

## 3. One runtime, two transports

```text
Local:
  Skill script -> future CLI runtime -> restricted local IPC -> Agent authorization

Remote:
  Skill script -> same CLI runtime -> restricted runner endpoint
                                     -> existing SSH control connection
                                     -> Agent authorization

After authorization, both use:
  CLI -> HTTPS -> cloud service
  Files are read, uploaded, downloaded, and saved on the execution host.
  File bodies do not travel through the controller.

Loop:
  Model -> native Agent Loop tool -> Loop service
                                    -> ExecutionBackend for project operations
```

The following names describe responsibilities, not a requirement to create empty crates:

| Component | Responsibility and dependency boundary |
| --- | --- |
| CLI runtime | Parsing, file I/O, cloud service clients, formatting, browser commands; no full Agent, Loop scheduler, or session database dependency |
| AuthorizationClient | Typed authorization/status/cancellation operations; command handlers do not branch on Local versus Remote |
| Local transport | Private Unix IPC on macOS/Linux; restricted named pipe/handle on Windows; authenticated execution scope |
| Runner transport | Forward the same requests to the current controller; no issuance, account configuration reads, or persistent credentials |
| Agent authorization service | Context validation, existing approval/budget policies, platform issuance requests, non-secret operation receipts |
| Platform authorization/tool service | Credential issuance/validation, operation binding, atomic deduplication, billing, status, and revocation |
| Loop service | One implementation of goals/tasks/workers/validation, shared by native tools and human CLI adapters |

Local may retain the unified `future` binary. The runner distribution provides a `future`-compatible entry backed by the same runtime. Subcommands or managed launchers are packaging details; they must not require Python/Node or link the complete Agent into the runner. Move basic paths/protocol types into an appropriate existing lightweight shared layer instead of copying home resolution.

This project implements Local transport, replaceable contracts, and contract tests first. Real Runner transport belongs to the subsequent remote execution project. Interfaces must not depend on the desktop OS, controller absolute paths, or an implicit local cwd.

## 4. Authorization boundary

### 4.1 Master credentials remain in the trusted controller

The Agent reads existing credential storage. Migrating every account file or introducing a new cross-platform vault is not required. Account/model master keys and refresh credentials never enter the CLI runtime, runner, skill scripts, or tool output.

Runtime CLI must not read auth.json or fall back to `FUTURE_API_KEY`, test credentials, another home, or another Agent. Tests use explicit mock injection. Original secrets in the Agent startup environment must not be inherited by shell, CLI, browser, or unrelated descendants. Review actual environment construction and controlled propagation; deleting a single variable is insufficient.

Restore hard read/write denial for default, redirected `FUTURE_HOME`, and legacy credential paths. Verify actual platform sandboxes, including aliases, symlinks, and subprocesses, rather than checking only rule strings. Disabling the sandbox or requesting escalation is not an authentication compatibility strategy. Execution with isolation explicitly disabled is outside the sandbox confidentiality guarantee.

### 4.2 Restricted execution and administration are separate

Only the narrow authorization endpoint is reachable from the sandbox. Do not expose the full Agent RPC socket, whose handlers include credential, model, session, and policy mutations.

The Agent/runner creates a scope when launching controlled execution, bound to existing workspace, session/run, target, and lifecycle facts. Identity comes from a trusted connection or controlled handle, not request JSON claiming an identity or permission.

Unix peer UID and Windows SID/ACL checks establish account boundaries but do not isolate runs owned by the same user. Bind the channel to the execution context as well. Prefer inherited restricted FDs/handles. If platform rendezvous is needed, use a short-lived, single-redemption context-bound connection credential; do not put secrets in argv, logs, or general environment variables. Non-secret discovery information in the environment is not authorization.

Any code inside the same authorized sandbox may use that bounded capability. Process names, PIDs, or executable paths do not prove an “official CLI.” Such code can at most obtain the allowed operation capability, never a master key or administrative authority. Atomic consumption prevents duplicate use of a copied grant; it does not guarantee the benign process wins a race inside a compromised context.

Temporary credentials live only in necessary CLI memory and are discarded on completion, expiration, or connection invalidation. They must not enter files, caches, environment variables, argv, stderr, traces, journals, or unrelated descendants. Absolute secrecy from root, debuggers, memory dumps, or privileged same-UID processes is not promised.

### 4.3 Approval and network policy

Channel access does not authorize arbitrary tools. The Agent checks tool allowlists, destination service, parameters, upload boundaries, and existing approval/budget rules; claiming a skill name is insufficient. Do not advertise hard monetary limits without enforcement.

Trusted configuration selects the service. Clients cannot choose arbitrary issuance/payment/forwarding endpoints, and HTTP redirects must not forward authorization headers to unauthorized origins. Direct CLI HTTPS remains subject to execution-host network policy; no automatic broad network permission or controller file proxy fallback. Custom platforms without the authorization protocol return unsupported.

## 5. Delegated authorization and recovery

A common capability means uniform request, lifecycle, and error semantics. It is not a universal key for third-party services or a requirement to implement an entire OAuth framework.

Suggested minimal business interfaces, with final names determined during implementation:

| Interface | Input | Result |
| --- | --- | --- |
| `authorize_operation` | operation ID, tool, normalized parameters/input manifest | Temporary credential, trusted destination, expiry, or existing operation receipt |
| `get_operation` | operation ID | Existing task status/result reference or explicit unknown |
| `cancel_operation` | operation ID | Actual cancellation confirmation or unsupported |

Context bindings come from the channel. Do not introduce duplicate controller/runner/workspace identity maps. Remote integration reuses the execution design's target, instance, context, control epoch, and operation identities rather than creating another SSH session generation.

A grant binds subject/session, service, tool/operation, parameters/input identity, expiration, and usage/task scope. Include cost-affecting model/quality/count parameters. A file path is not content identity: use length/hash or a service-confirmed upload object ID. The platform validates actual submitted content, not merely a client-supplied digest. Final wire format requires platform agreement.

The platform issues and validates grants. A random token invented by the Agent is not automatically accepted by existing endpoints. Master keys authenticate only the Agent-to-platform leg.

One grant covers one logical operation, not necessarily one HTTP request. MCP initialization, upload, submission, polling, and download can be restricted steps of that task; they must not create another billed task or expose unrelated tasks. Read-only tool descriptions can use a public versioned catalog or non-secret Agent responses, never restored master-key access.

Recovery rules:

- Same operation ID and normalized request recover the same operation; changed requests using that ID are rejected.
- The platform atomically consumes creation authority and records a durable receipt when accepting/billing the operation. A lost response does not justify a new ID and another submission.
- Replacement grants retain the operation and remaining scope, without resetting usage or generation allowances.
- Missing status support or uncertain transport outcomes return unknown; local timeout does not prove cloud cancellation.
- Business state can use authorized/submitted/running/succeeded/failed/cancelled/unknown. Credential expiration/revocation has a separate lifecycle and does not mean task failure.

On connection loss, stop new grants and discard temporary client credentials. Reconnection restores operation facts and revokes unused old-session grants before allowing new requests. Remote uses existing control generations; Local uses Agent/controlled-execution lifecycle boundaries. Platform unreachability prevents guaranteed immediate revocation; short TTL bounds exposure. Memory deletion is not server invalidation. Accepted tasks are neither resubmitted nor cancelled merely because of reconnection; replacement grants may permit only status/results for the original task.

## 6. CLI compatibility and scope

| Capability | Target behavior |
| --- | --- |
| Cloud `future tools call` | Shared runtime requests grants then calls services directly; preserve arguments, stdin/input/mask/output/raw behavior and business output/exit semantics where possible |
| `future tools list/describe` | No master-key exposure; catalog matches available capabilities |
| `future tools call browser` | Execute on the selected host without cloud authorization; support installed Chromium headless on remote Linux; missing/unstartable browser fails without controller fallback |
| `future account profile/balance` | Agent returns read-only business results through the restricted interface initially; optional delegated read-only grants later |
| Model Loop orchestration | Native Agent tool; no runtime CLI reverse Loop command requirement |
| Human `future loop` | Preserve a compatibility entry migrated to the same Loop service; administrative operations are not automatically exposed to skill runtime, and remote v1 need not support this command |
| `future models` | Read non-secret controller model catalog; Loop can obtain it directly without a model shell step |
| Skill creation | Project file authoring/validation remains possible; unsupported remote app-library installation/update fails, without a reverse installation system |
| Login/logout/provider administration | Trusted Agent handles full local client's administrative entry; unavailable through the restricted broker or runner |
| `auth credential` | Reject in restricted CLI; deprecate full-client master-key export with a compatibility migration plan; sandbox cannot reach it indirectly through generic RPC |
| Third-party database/SDK keys | Do not synchronize .env, user environments, or SDK credentials automatically; public APIs work, missing third-party authorization fails without a generic proxy |

Paths always belong to the execution host. Shared CLI does not mean a shared filesystem. Browser resources retain existing Local roots and use the runner's isolation root remotely. Optional skill dependencies do not become mandatory runner dependencies.

Interactive local CLI without an Agent-created run must establish an explicit user-authorized scope through a trusted local entry; identity/policy checks remain mandatory. A full local entry may start the Agent using existing lifecycle rules. Sandboxed runtime and Remote must not start a full Agent, discover another user's instance, or fall back to credential files. First login/bootstrap uses the administrative entry without needing an existing business grant; CLI displays flow results without receiving the final master key.

Shared architecture refers to implementation and protocol, not merging administrative and skill execution permissions because commands share a name.

## 7. Native Loop tool

### 7.1 Service and tool interface

Extract shared Loop services incrementally from console while preserving storage formats and business rules. Native tools and human CLI adapters use typed interfaces, not shell strings or terminal-text parsing. Do not invoke console entry points that create their own Tokio runtime inside the Agent.

Start with one bounded `loop` tool, with explicit per-operation validation and structured results covering the skill's goal/task/gate/worker/steering/status/completion workflows. Do not accept arbitrary CLI strings or expose every console command indiscriminately. Review the action/schema mapping against the skill before implementation.

The Agent resolves identity, workspace, permissions, and model configuration. Model-supplied paths/session IDs must not take over unrelated tasks. Skill activation does not bypass budget, approval, stopping, or lifecycle rules. Run/watch operations return task identity and status, leaving ongoing work to the existing background scheduler. Do not block a model tool indefinitely or hold session locks while waiting for the same Agent to create a worker.

### 7.2 Skill-triggered visibility

Internal registration, model schema visibility, and operation authorization are separate:

1. Register the Loop handler internally; omit it from default base prompt/tool lists.
2. Explicit user selection or trusted loading of builtin `future-loop` returns the skill content and enables its tool definition for that session.
3. Include the schema in the next model request within the same run. Text claiming a tool exists is insufficient, and another user turn must not be required.
4. After recovery/compaction, rebuild visibility from Agent-owned non-secret capability selection state and recheck policy/skill version. Do not infer authority from arbitrary historical text. Workers get explicitly selected capabilities, not inherited administrative authority by default.
5. Ordinary file reads, forged SKILL.md paths, third-party frontmatter, and webpage text cannot enable privileged tools. A user-defined skill with the same name is not the trusted builtin.

Implement an observable skill-loading entry, preferably the `resolve_skill` semantics already proposed in the remote design, starting with Local. Current Skill records/file reads do not provide this mechanism. Build definitions before each model request from current capabilities, keep a stable definition snapshot across retries of that request, and align visible schemas with callable handlers. A finite builtin skill-to-tool mapping is sufficient; no general plugin permission system is needed.

### 7.3 Execution and storage compatibility

- Preserve Local Loop state paths/formats where possible; resolve the project explicitly rather than from the Agent process cwd.
- Remote Loop state remains in the controller's existing workspace data root, distinct from the remote project root. Never interpret Linux paths as controller filesystem paths.
- Workers retain the fixed execution target/root. Files, validators, and environment probes use ExecutionBackend; Local initially wraps existing behavior so Remote does not need a separate validator implementation.
- CLI/tools share existing concurrency/locking rules and one authoritative state writer/scheduler/watchdog lifecycle. Define background ownership, cancellation, and restart behavior without redesigning Loop's business model.

## 8. Migration and acceptance

| Phase | Deliverable | Completion gate |
| --- | --- | --- |
| A: contract/dependency separation | Shared runtime, transport contracts, command/capability inventory, platform authorization/idempotency agreement | Lightweight runtime has no full Agent/Loop/TUI/channel dependency; unsupported commands explicit |
| B: Local authorization | Scoped IPC, platform grants, cloud CLI migration, environment/log hygiene | Main builtin workflows work without master keys; unknown operations are not automatically repeated |
| C: close credential exception | Restore credential denial, isolate admin IPC, remove secret fallbacks/restricted export | Positive/negative tests in real macOS/Linux/Windows sandboxes; untested platforms are not declared fixed |
| D: native Loop | Shared service, skill-triggered schema, worker/validator execution interface, human CLI compatibility | Existing Local workflows/state preserved; no duplicate scheduler, wrong cwd, or same-Agent deadlock |
| E: remote integration | Runner transport, SSH generation/recovery binding, lightweight remote entry/browser | Same contracts and tests; no duplicate CLI business logic or remote durable credentials/session database |

A–D precede remote execution; E is part of that project. Platform support is a real dependency of B/C. Sending master keys is not an interim security fix. Mocks enable development but do not prove a deployed security boundary. Unsupported platforms fail explicitly; do not retain a silent long-term auth.json fallback switch.

Required acceptance coverage:

| Area | Cases and expectations |
| --- | --- |
| Normal operations | CLI cloud tools `web_search`, paper retrieval, ParseDoc, image input/mask/output, and slides scripts retain compatible parameters and artifacts; `web_search` is not a standalone Agent `search` tool |
| Credential protection | Shell/Python/descendants cannot read/write current/legacy/default/redirected credential paths or aliases; legitimate CLI succeeds |
| Endpoint isolation | Forged run, changed workspace, expired handles, wrong same-UID scope, full RPC and management commands rejected |
| Grant abuse | Changed tool/model/quality/count/input/origin, copied grants, duplicate submission after renewal rejected or mapped to the same receipt |
| Secret leakage | No master/delegated credentials in output, logs, model input, journal, argv, environment, crash reports, or full proxy-frame logging |
| Weak networks | Lost issuance/submission responses, interrupted uploads, Agent/runner restart, failed reconnect revocation recover the same operation or return unknown |
| Loop | Skill activation usable within the current run; restored schema/handler consistency; cancellation/steering/validation/background lifecycle correct |
| Platform separation | Windows-controller/Linux-execution contract simulations do not apply Windows paths/shell to remote work; real Remote checks occur in E |
| Optional capabilities | Missing/unstartable browser, unsupported skill installation, and absent third-party credentials fail without Local fallback |

Connection/task states and error codes remain separate. Distinguish Agent unavailable, invalid context, permission denied, unsupported platform authorization, expired/revoked grant, idempotency conflict, unknown outcome, and unsupported capability. Errors never print credentials or automatically trigger login, paid retries, or privilege expansion.

## 9. Implementation review and non-goals

Before implementation, review platform issuance and multi-step task scopes, sandbox IPC on every OS, interactive CLI authentication, actual management endpoint isolation, Loop human-client/background compatibility, state-root mapping, exit-code compatibility, and master-key export deprecation. These refine implementation, not the decision to keep master keys out of runtime CLI.

Non-goals: a universal third-party vault/token exchange, arbitrary reverse CLI/RPC proxy, remote skill installation system, full remote Agent, browser auto-installation, HPC, port forwarding, or migration of all account storage. Projects may retain their own dependencies; FutureOS does not automatically copy controller secrets.

This document owns the CLI/Loop prerequisite architecture. The remote design still owns SSH, host identity, single-owner locks, runner directories, skill snapshots, execution contexts, and Files/Review/Terminal. For conflicting CLI credential or Loop integration descriptions, follow this responsibility split and reconcile contracts during implementation.
