# Desktop Embedded Agent Development Plan

> [中文](embedded-agent-development.zh-CN.md)
>
> Design date: 2026-10-13. Source baseline: `88fe90a8`.
> Status: phases one through four are implemented and pass local macOS acceptance and static checks.
> User-requested tests ran on 2026-10-14: section 10.4 records historical phase-one-through-three results; section 10.5 records phase four.
> Phases one through six are implemented. Phases five and six initially received static checks only; subsequent user-authorized macOS testing is recorded in section 10.7. Phases seven and eight, native Windows/Linux, real installation/upgrade and PR CI remain pending. Section 10.6 describes phase six.
> API sketches and acceptance matrices describe the complete target.

Desktop will run the complete Agent inside its Rust process when the default
local endpoint has no external Agent and it acquires the instance lock.
Compatible external services remain attached; incompatible or unreachable lock
owners produce explicit errors. External Agents and CLI, TUI, channels and loop
clients retain the shared RPC interface. The change reduces Windows startup and
upgrade failures caused by divergent Desktop and sidecar lifetimes while
preserving instance locking, tools, persistence and external client access.

Extract the service first, integrate hosts second, then switch development,
build and update paths. The first release keeps the storage format and business
RPC contract and continues to bundle the `future` CLI. CLI slimming, database
migration and tool permission changes are separate work.

## 1 Product behavior and scope

### 1.1 Service selection

| Situation | Desktop behavior | Data and ownership constraint |
|---|---|---|
| Compatible external Agent at the default local endpoint | Connect after a successful handshake | Do not take ownership or start a duplicate |
| Incompatible external Agent | Report a version or protocol conflict and stop Agent business calls | Do not start the embedded service or automatically terminate the external process |
| Unresponsive external Agent with an occupied instance lock | Wait within a deadline, then report busy or unavailable | Do not delete the lock file or open/migrate Agent storage |
| No service at the default endpoint and successful lock acquisition | Start the complete embedded Agent | Hold the lock through initialization and the last background operation |
| Concurrent startup attempts | Only the lock winner initializes | Other clients wait and repeat the handshake; never bypass the lock |
| Explicit TCP or other external endpoint | Connect only to that endpoint; report failure | Do not redirect to the default local instance or bind the configured address implicitly |
| Installer owns an upgrade lease | Report temporary unavailability and refuse startup | Repeat locking and identity checks after lease release |

An explicit endpoint means an external service. Embedded TCP listeners needed
for development or tests use controlled startup configuration; a failed
connection does not authorize binding an address.

### 1.2 Exit and clients

Exiting Desktop in embedded mode stops every task on that Agent, including
tasks initiated by CLI, TUI, mobile, channels and loop. The exit prompt uses an
Agent-wide activity snapshot covering running work, queues, approvals and
long-running session mutations. Choosing to keep running must not close
admission early.

After force-quit confirmation, close admission, cancel relevant work, wait for
persistence and cleanup, then exit. If cleanup exceeds its total deadline, the
host can force the process to exit. Abnormal exit cannot promise delivery of
cancellation events; normal cancellation must publish a terminal outcome and
an understandable shutdown reason.

In external mode, ordinary Desktop exit must not globally stop the external
Agent or cancel a CLI task merely because it was imported into the GUI. Exit
handling for Desktop-owned tasks must use task provenance rather than the local
mirror alone. Installation and uninstallation have separate authorization rules
in section 7.

Preserve current window-close and tray behavior. The new lifetime binding is to
host process exit; this work does not introduce permanent background residency.

### 1.3 Retained capabilities and exclusions

Retain models, tools, approvals, sandboxing, skills, sessions, compaction,
events, queues and persistence. CLI clients use RPC rather than directly
accessing the active Agent's memory or database.

Keep `future agent` and standalone `future-agent`. TUI may still launch its own
external sidecar when no service exists. GUI and headless share the service;
Agent core must not depend on Tauri, WebView, GTK or the Desktop application
crate.

The first release does not support multiple Agent data roots inside one
process, automatic takeover of an occupied lock, or restart before all old work
has stopped. Preserve the existing `shutdown` RPC semantics: reject new prompts
while accepted runs continue. Full host shutdown is a separate lifecycle API.

## 2 Existing structure and required changes

| Existing entry | Current behavior | Required change |
|---|---|---|
| [Agent CLI](../../agent/src/cli.rs) and [signal handler](../../agent/src/cli/shutdown.rs) | Own locking, environment, logging, runtime and forced process exit | Service code must not parse process arguments, initialize global logging, register exit signals independently or exit the process |
| [GUI supervisor](../../desktop/src-tauri/src/agent_supervisor.rs) | Probe and own a `future agent` child; reset Windows permissions and kill on exit | Manage an embedded handle or an external connection and use complete shutdown |
| [GUI startup](../../desktop/src-tauri/src/lib.rs) and [host lock](../../desktop/src-tauri/src/instance.rs) | Background work starts in setup; GUI/headless lock is acquired after Tauri build | Defer dependent work until host locking and compatibility validation |
| [Business connection](../../desktop/src-tauri/src/agent_bridge/client.rs) | Cache Channels and probe reachability; status checks compatibility separately | Gate all business calls and reconnection on compatibility and instance identity |
| [Headless Agent owner](../../desktop/src-tauri/src/headless/agent.rs) and [TUI supervisor](../../tui/src/agent_supervisor.rs) | Independently manage children or reuse a reachable Agent | Integrate headless with the common host; retain TUI ownership and share handshake/conflict diagnostics |
| [SQLite worker](../../agent/src/session/database/mod.rs), [Manager](../../agent/src/session/manager/mod.rs), [event writer](../../agent/src/rpc/broadcaster/journal_writer.rs) | Ordered writes, terminal transactions, maintenance and destructor joins | Explicitly drain, stop, join and report failures rather than relying on process death |
| [RPC shutdown](../../agent/src/rpc/commands/session_lifecycle.rs) and [scheduler](../../agent/src/rpc/session.rs) | Reject new prompts; completed runs may still trigger dequeue | Add host shutdown that closes admission and dequeue atomically and publishes queued cancellation |
| [RPC server](../../agent/src/grpc/mod.rs) and [global broadcaster](../../agent/src/rpc/mod.rs) | Production servers wait indefinitely; global events use process statics | Add explicit server shutdown and termination of global/session subscriptions |
| [Tasks loop](../../desktop/src-tauri/src/tasks.rs) | Continuously creates user work without an explicit host stop entry | Stop production before shutdown and reconcile `tasks.db` |
| [Updater](../../desktop/src-tauri/src/commands/update.rs) | download_and_install and Windows on_before_exit sidecar cleanup | Complete fallible shutdown before installation; keep the hook as an idempotent fallback |
| [Skills bootstrap](../../desktop/src-tauri/src/skills_bootstrap.rs) | Runs `future init` through the bundled CLI | Continue packaging and managing the CLI |
| [Linux runner](../../agent/src/sandbox/linux/runner.rs) | Re-executes current_exe as its sandbox helper | Explicitly select a helper supported by the new host |
| [CI](../../.github/workflows/ci.yml) | desktop_tauri paths omit Agent changes; Windows/macOS primarily compile | Expand dependency triggers and native lifecycle/package checks |

Agent owns `agent/agent.db`, Desktop owns `app/app.db`, and the task ledger is
`tasks.db`. Legacy JSONL imports run under the Agent lock. Preserve these
authority boundaries; see [SQLite operations](sqlite-migration.md).

## 3 Service layer and host API

### 3.1 Dependency direction

```text
GUI Desktop ──────┐
headless Desktop ─┼── future-agent service ── RPC, runtime, storage, tools, sandbox
future agent ─────┤
future-agent ─────┘

CLI / TUI / channels / loop / Desktop bridge ── shared RPC ── active Agent
```

Extract `agent/src/service/`, with suggested modules for configuration,
instance ownership, startup, activity, shutdown and background work tracking.
GUI/headless share host management that does not depend on Tauri; GUI adapts
windows, prompts and events.

The service owns Agent locking, storage initialization, serving, activity,
cancellation, durable completion and resource release. Hosts own arguments,
startup environment, logging, TLS provider, signals, confirmation, updates and
final process exit.

### 3.2 Suggested API

The following describes responsibilities, not existing types or compilable
signatures:

```rust
AgentService::start(config) -> Result<AgentHandle, StartError>
AgentHandle::wait_ready(deadline) -> Result<AgentIdentity, StartError>
AgentHandle::activity_snapshot() -> Result<AgentActivity, ActivityError>
AgentHandle::shutdown(request) -> Result<ShutdownReport, ShutdownError>
```

`AgentConfig` fixes the data root, listener, tool environment and resource
configuration. Resolve the production root once at startup and keep it
immutable; incrementally replace runtime global environment reads with explicit
paths. Tests inject isolated paths and endpoints.

`AgentIdentity` includes an instance ID, version and protocol compatibility.
Use data-root identity to validate local scope without exposing credentials or
unrelated machine paths through remote APIs. Generate a new instance ID for
every service start; Desktop PID is not an instance ID.

`ShutdownRequest` carries exit/restart/update/host-failure reason, total deadline
and a confirmed force policy. `ShutdownReport` distinguishes complete stop,
cancellation/persistence failure, permission cleanup failure, timeout and
remaining work. Only proven completion allows lock release and in-process
restart.

Use a dedicated owner thread with an explicitly owned Tokio runtime to avoid
blocking the UI. Measure worker and blocking-pool budgets against total Desktop
resources. Validate handle Send requirements, lock lifetimes and destructor
ordering during implementation; remove lock allocations whose leaked lifetime
assumes a single startup per process.

### 3.3 State machine

```text
Select → Validate external → Attached external
       → Wait for lock → Initialize → Ready
       → Incompatible or occupied → Explicit error

Ready → Quiescing → Cancelling → Draining → Cleaning → Stopped
                 cleanup deadline exceeded → FailedStopping
```

`Stopped` proves that RPC, async/blocking work, writers, compaction workers and
managed subprocesses have completed. Only then may the lock be released.
`FailedStopping` forbids another embedded instance. The host may retain a
failure UI or exit the process after confirmed interruption.

Startup errors unwind acquired resources. Concurrent host exit cancels startup
and waits for cleanup; if completion cannot be proven, retain the lock until
process exit. Startup must not publish a late Ready after host shutdown.

Separate a slow-start notice from the actual initialization deadline. GUI
currently has a 15-second startup grace while headless can wait 180 seconds.
Unify status semantics without spawning replacements for slow storage, imports
or model initialization. Ready requires business readiness; login must pass
this gate before receiving one-time authorization credentials.

## 4 Connection and locking rules

### 4.1 Locks and data initialization

Reuse `agent-instance.lock`, its Windows byte-range protocol and the installer
lease. Preserve identification fields in `agent-instance.json`. Metadata helps
diagnostics; the OS lock decides whether startup is allowed. Never truncate an
occupied lock or delete/recreate it to bypass its owner.

On the service startup path, do not open/migrate Agent storage, import legacy
data, reset shared sandbox capabilities or overwrite metadata before locking.
Existing sessionless maintenance commands retain their separate paths and
dedicated capability or other locks; they need not take the long-lived service
lock and cannot become a way to start a second service. An existing but
lockable file is not a live service; stale metadata does not authorize takeover
of an occupied lock.

Resolve `FUTURE_HOME`, default home, IPC names, metadata and installer
environment consistently. Cover case, symlinks/junctions, UNC, empty and
relative paths. Locking must protect the same physical data directory even
when endpoint spellings differ.

Keep Desktop and Agent locks separate. Align the host lock with the actual
Desktop data root: current `InstanceGuard` uses HOME `.future/app`, which must
be reconciled with the `FUTURE_HOME` override. Single-instance window activation
must respect the same instance scope.

### 4.2 Compatibility and reconnection

Read readiness before business calls. The minimum solution preserves the
repository's coordinated upgrade contract and prevents false compatibility
between different development builds sharing a numeric version. If additional
fields are required, define explicit protocol version and capabilities in the
shared RPC crate.

Unknown readiness, missing required fields or unsupported protocol is an
explicit incompatibility. Additional identity fields do not imply mixed-version
business protocol support. Apply the gate to tasks, config, import, deletion
and observers, not just the frontend send button.

Bind cached connections to a validated instance. On disconnect or instance ID
change, repeat the handshake, stop old observers and discard stale stream
cursors and process-local queue assumptions. Transport reconnection must not
carry old authorization into a replacement instance. If necessary, carry an
expected instance ID on mutating requests and verify it server-side to close
the handshake/write race.

Preserve current-user named-pipe DACLs, Unix socket protection and explicit TCP
semantics. Embedded mode does not introduce a publicly exposed listener.

## 5 Complete shutdown protocol

### 5.1 Ordered cleanup

1. Stop host tasks ticks, automatic submission, remote sending and loops that
   create new business calls. Observers may remain briefly to receive outcomes.
2. Atomically enter Quiescing and close mutation admission and scheduler
   dequeue. Synchronize with acceptance already in progress; an atomic flag
   checked before a later lock is insufficient.
3. Track and finish accepted mutations. Retain only status, cancellation,
   required persistence and controlled cleanup. Compaction, config, deletion
   and skill mutations obey the same stopping boundary.
4. Cancel queues with observable outcomes, pending approvals, active runs and
   session mutations. Initiating client does not change embedded shutdown scope.
5. Wait for managed model requests, tools and children. Reuse shell cancellation
   and Windows Job Objects; do not kill arbitrary user process trees. Processes
   deliberately detached after successful commands follow the existing contract.
6. Wait for durable run/session/event completion. Preserve terminal transaction
   semantics and do not report successful cancellation after persistence failure.
7. Publish the shutdown reason and available terminal outcomes, close global
   and session subscriptions and stop listeners. Bound long-stream shutdown.
8. Stop manual/journal compaction, background refresh and SQLite workers; wait
   for service work and close storage connections.
9. Clean Windows capabilities after managed sandbox work ends. Retain repair
   metadata after failure for startup, reset or uninstall retries.
10. Remove this instance's metadata and release the Agent lock. Complete host
    terminal, remote and ledger cleanup before exit or installer handoff.

Actual broadcaster, worker, storage and lock disposal follows resource
dependencies; no writer may survive lock release. Agent, app and task databases
are reconciled by their respective owners, without fabricated cross-database
terminal states.

### 5.2 Concurrency and total deadline

Window/menu exit, update, restart and repeated clicks share one single-flight
coordinator and result. Never duplicate cancellation, joins or capability
cleanup. Confirmation is asynchronous; cleanup must not block the UI thread
waiting on work that needs UI/runtime locks.

Prefer complete normal cleanup. A confirmed force quit permits final process
exit after the deadline; an update requires explicit or equivalent existing
authorization to interrupt tasks. An initial proposed total budget is 15
seconds, to be adjusted by native acceptance. It is a design budget, not a
current behavior or performance guarantee.

The final watchdog must not depend on Agent runtime, logging locks or database
locks. `shutdown_timeout` does not prove termination of blocking threads; never
release the lock and start a replacement in the same process after timeout.

SQLite WAL protects committed transactions; an undurable tail may be lost.
Startup reconciles incomplete runs as interrupted. Do not delete WAL files,
fake persistence success or recreate damaged databases to make exit succeed.

### 5.3 Exit guard and recovery

Activity includes running, queued, approval and session-mutation work. Snapshot
failure is Unknown rather than zero; let the user explicitly choose to continue
or force quit with an explanation that task status cannot be verified.

After confirmation, obtain current activity and use service-wide shutdown to
cover new tasks accepted while the prompt was open. Before confirmation, keep
admission unchanged. Canceling the prompt preserves normal operation; after
shutdown starts, do not simply flip back to Ready.

The first release restarts only after complete Stopped. Incomplete cleanup
retains the lock and requests a host restart. Classify process statics such as
global events, log initialization, model caches and revision counters by
lifetime. Move restart-sensitive state into service instances or reset it
explicitly; old subscriptions and work cannot leak into new instances.

## 6 Hosts and platform integration

### 6.1 GUI and headless

GUI chooses and initializes services asynchronously after host locking. Gate
tasks, login and dependent synchronization on ready; present actionable storage
errors. Missing initial model configuration remains a supported startup.

Headless uses the same service and selection policy, with host-owned Ctrl+C,
console-close and Unix termination signals. Preserve first-login terminal
requirements and the GUI-free dependency graph. `--help` and first
non-interactive refusal must not initialize Agent storage or listeners early.

TUI and standalone CLI reuse initialization, locks and shutdown while retaining
their arguments and signal ownership. A TUI-owned Agent is external to Desktop;
after it exits, Desktop must validate lock availability and identity before
takeover.

### 6.2 Logging and environment

Initialize tracing and the TLS provider once per host. Keep Agent file logs and
CLI console behavior, recording host mode, instance ID, lifecycle phase and
cleanup outcome without credentials, complete prompts or sensitive paths.

Windows console detachment must not remove startup diagnostics or Agent logs.
Do not initialize blocking reqwest clients on async/UI threads. Agent reqwest
0.12 currently selects ring; Desktop reqwest 0.13 uses the host's ring provider.
Verify the merged feature graph rather than assuming two reqwest versions
necessarily conflict.

Hydrate macOS/Linux login-shell environment before multithreading or pass an
explicit tool environment. Do not mutate global `FUTURE_HOME`, PATH or shell
settings after startup. Working directories remain session-scoped; no process
chdir.

Apply build-channel platform policy before Agent first reads auth/config.
Preserve production locking for releases and the development test default;
startup reordering must not send Desktop and Agent to different platforms.

Profiling, the dhat global allocator, Ctrl+C forced exit and maintenance
arguments belong to CLI/hosts. Normal Desktop builds do not enable profiling.
The dependency direction must keep GUI libraries out of standalone Agent and
headless.

### 6.3 Sandbox helper and tool resolution

Prefer an injected Linux helper path pointing to the matching bundled
`future agent --linux-sandbox-helper`, preserving helper protocol and security
boundaries. If a hidden host helper entry is chosen instead, dispatch before
GUI, host/Agent locks, storage and network initialization. Implement one
explicit strategy; executable basename guessing is insufficient.

Verify real Linux isolation, request versioning and cancellation. The helper is
a tool subprocess, not a persistent Agent sidecar.

Tool PATH explicitly includes the matching CLI directory. Root
`target/debug/future` and Desktop dev outputs are not adjacent. Verify portable,
macOS app, Linux deb and standalone headless layouts separately. Missing CLI
does not prevent embedded core startup, but CLI-dependent skills/helper calls
must fail clearly and preserve sandboxing rather than run unconfined.

## 7 Updates and installation

### 7.1 In-app update

```text
Check → Download and verify → Confirm interruption → Complete shutdown → Install
                             download/verification failure: keep serving
                             shutdown failure: do not install implicitly
```

Split `download_and_install`; perform fallible async shutdown before
installation. The current updater's `on_before_exit` cannot return a Result.
Keep it as an idempotent final fallback, not a place to signal recoverable
cleanup failure through panic or process exit.

Successful download/signature verification does not guarantee extraction or
installer launch. Define recovery both before and during the installation call:
complete Stopped with no installer takeover may restart with a new instance ID;
incomplete shutdown cannot resume a partially stopped instance. Preserve
Windows updater process exit without relying on `RunEvent::Exit`.

Verify launch-error observability, including ShellExecute handling: the current
dependency may not report all launch failures. Prefer a host adapter or a
validated dependency correction that exposes failure. Neither hook completion
nor disappearance of the app proves successful installation. After installer
handoff, verify logs, rollback and next-start diagnostics.

Coordinate remote entry, terminals, tasks and app writers. Windows portable
packages remain outside NSIS in-place update; macOS/Linux installation and
restart also use complete shutdown.

### 7.2 Windows install and uninstall

Update [preflight](../../desktop/src-tauri/windows/installer-preflight.ps1),
[NSIS hooks](../../desktop/src-tauri/windows/installer-hooks.nsh) and
[operations](../internals/desktop-windows.md). Recognize `futureos` as an Agent
lock owner, and headless if it enters the same distribution path.

Verify executable path, process creation time, data root and role. Metadata is
supporting identification. Preserve retired `future-agent`/`future-desktop`
handling, writable-file probes, installer lease, loader checks and executable
rollback.

Ordinary exit leaves external services intact. Where installers must stop a
same-data-root external service, retain authorized install/uninstall behavior
and show affected tasks; in-app updates explain this before installation.
Cross-installation termination requires verification and explicit scope
authorization. Adding a `futureos` name must not silently broaden termination.
Generic silent/passive installs still do not close tasks and retain busy 32,
access failure 5 and other existing outcomes.

An old uninstaller runs its own original hooks; new hooks cannot repair every
old uninstall-first path. Preserve recovery guidance for old VC++ runtime
requirements. The installation lease excludes new Agents until completion and
is released after rollback or abnormal installer exit.

Executable rollback does not roll back user databases. Keep database schema
unchanged for this work; later schema changes require a separate compatibility
and backup review. Restoring old binaries must not imply they can read a newer
schema.

### 7.3 Restart and reset

Integrate [clear_app_data and environment switching](../../desktop/src-tauri/src/commands/debug.rs)
and [post-update restart](../../desktop/src-tauri/src/commands/update.rs).
Pause producers, stop applicable Agent/writer work, then delete Desktop data or
change shared environment configuration and restart.

External mode preserves ownership boundaries. Writing shared auth config and
expecting an external Agent to restart is not a valid reconciliation strategy.

## 8 Build and development tooling

| Area | Work |
|---|---|
| Root/Desktop Cargo | Add a one-way Desktop path dependency on Agent; update both lockfiles; inspect workspace features, Rustls, SQLite, pprof and release settings |
| Tauri config/build | Retain `future` externalBin and valid CLI placeholders; remove the assumption that placeholders require manual Agent startup |
| Makefile | Update run-desktop, build-desktop, headless and check targets/comments; retain CLI staging |
| `scripts/dev/start-desktop-macos.sh` | Default to embedded; remove default external PID/start/monitor flow; keep explicit external debugging |
| `scripts/dev/start-desktop-linux.sh` | Same, plus real helper path and tool PATH verification |
| `scripts/dev/start-desktop-windows.bat` | Same; migrate BUILD_AGENT/REUSE_AGENT semantics and retain encoding/log diagnostics |
| `scripts/build/build-desktop-macos.sh`, `scripts/build/build-desktop-linux.sh` | Keep CLI staging and verify embedded GUI/headless artifacts |
| `scripts/build/build-desktop-windows-installer.ps1`, `scripts/build/build-desktop-windows-portable.ps1` | Retain both executable signatures, versions and CRT checks; verify default startup without a persistent Agent child |
| Release workflows | Coordinate version injection, signing, packaging, embedded smoke and artifact checks; retain GUI-free musl headless checks |
| Doctor/status UI | Report embedded/external, actual instance/version, lock conflicts, stopping and cleanup failure; missing standalone future-agent is not an embedded failure |
| Documentation | Update build/install, headless, layout, Windows, CLI and sandbox documentation in both languages |

Do not require merging Cargo workspaces. Desktop may retain its independent
workspace; verify dependency builds and both lockfiles. Keep
`tauri-plugin-shell` while skill bootstrap uses it.

## 9 Development phases and deliverables

Execute phases in order. Establish each interface and targeted CI coverage
before changing downstream default behavior. These are implementation tasks,
not completed changes.

### Phase one Extract the service

- [x] Extract config, lock/metadata, storage/engine/AppState initialization,
      listener and ready notification.
- [x] Reuse the service from both CLI entries, retaining CLI arguments,
      logging, signals, maintenance and profiling.
- [x] Add identity, startup cancellation, resource ownership and structured
      failure; initialization failure leaves no live work.
- [x] Classify service versus process state; preserve RPC, storage and tools.

Acceptance: CLI entries remain equivalent; lock conflicts do not initialize
storage; unconfigured startup reaches ready; corruption fails safely; successful
stop releases resources and failure retains the lock. Agent builds without GUI.

Phase one implementation: branch `codex/agent-service-layer`, implementation baseline `73ed4183`. The actual API uses `AgentService::with_instance`, `prepare`, `PreparedAgent::initialize` and `RunningAgent::serve`. Borrowed lock ownership requires host runtime teardown before the scope returns; each scope permits only one initialization so one lock cannot protect two service instances. The service does not initialize global logging/signals, mutate environment, create the host runtime or exit the process.

[Service](../../agent/src/service/mod.rs) owns the extraction while [CLI](../../agent/src/cli.rs) retains argument, maintenance, logging, signal and profiling policy. [Startup](../../agent/src/service/startup.rs) reuses engine, SQLite import and AppState initialization. [Instance](../../agent/src/service/instance.rs) preserves the lock path, Windows byte protocol and metadata fields without leaking the lock allocation. Ready follows initialization, binding and the server's first successful poll. Shared Unix transport now cleans an unpolled bound socket on cancellation.

[Isolated regression fixtures](../../agent/tests/service_startup.rs) cover real TCP/local IPC, contention, duplicate initialization in one scope, cancellation, corrupt storage, occupied endpoints and original-format import. Existing lifecycle tests moved with their implementation; service-wide shutdown tests now cover approval interruption. Local Agent/RPC and CLI/TUI/channels/loop all-target Clippy passed with warnings denied, along with the `dhat-heap` feature check, formatting and diff checks. Local unit/integration execution on 2026-10-14 is recorded in section 10.4; native Windows/Linux and CI outcomes remain pending.

Phase one preserved storage implementation/schema, protobuf, binary entry points and CLI flag declarations. Phase two adds explicit writer closure and work registration while preserving schema, storage formats and business RPC. Global broadcaster subscriptions terminate through a service shutdown signal. Dropping serving produces a failed stop rather than evidence of complete cleanup. Phase four integrates Desktop.

### Phase two Complete shutdown and activity

- [x] Add global activity and synchronized admission/dequeue gates.
- [x] Cancel queues, approvals, runs and compaction with durable outcomes.
- [x] Add explicit listener/stream shutdown and track native/blocking work.
- [x] Add deadlines, idempotent reports and host force-exit policy; retain
      Windows startup repair.

Acceptance: CLI-only tasks are included; queues stop; terminal data reloads;
subscribers do not block exit; deliberately stuck work produces FailedStopping
rather than unsafe lock release.

Phase two implementation adds [activity](../../agent/src/service/activity.rs), [shutdown](../../agent/src/service/shutdown.rs) and [work](../../agent/src/service/work.rs). `RunningAgent::handle()` returns a cloneable `AgentHandle`, exposing `activity_snapshot()`, `request_shutdown(Duration)` and asynchronous `shutdown(Duration)`. The host must continue polling `RunningAgent::serve` while awaiting shutdown and server completion concurrently. Awaiting shutdown before starting to poll serving is not a valid host lifecycle.

Shutdown synchronously closes command/dequeue admission, drains already admitted commands and cancels every session's queue, run, shell and approvals. Persistent queued requests use the existing `run_terminal` format with `cancelled` and their original `run_sequence`; ephemeral sessions gain no persistence from this path. Queue terminal writes follow active-run finalization so a healing rewrite cannot overwrite cancellation outcomes. Tracking covers runs, manual compaction, catalog refresh, LLM stream pumps, blocking finalizers, image reads, Linux sandbox preparation and Windows process waits. Idle schedulers and cancellation monitors are tracked separately from user activity.

The supervisor then waits for scheduler/cancellation monitors, closes and joins session writers, flushes and joins event writers, terminates session/global subscriptions, waits for listener completion, stops journal compaction, closes and joins SQLite, and finally removes Windows sandbox permissions. Activity counts include running/queued work, approvals, manual compaction, commands, background work and pending journal compactions. An unavailable short lock, concurrent work change or `FailedStopping` returns Unknown rather than idle.

A separate native deadline watcher depends on neither the Agent runtime, logging nor database acknowledgements. The first request fixes the deadline and subsequent callers receive the same terminal report. Timeout, worker panic, writer/permission failure or dropping serving produces `FailedStopping`; the scoped host retains the lock guard, descriptor and metadata until process exit, and another service cannot start in that process. A successful stop permits a new scope for the same root. Existing process caches/config helpers require frozen `FUTURE_HOME`; switching data roots within one process is rejected.

CLI now uses full shutdown while retaining the independent second-Ctrl-C/5-second force-exit protection. The service never exits the process. The existing business `shutdown` RPC still rejects only new prompts and permits accepted runs to finish. Windows startup repair runs at most once per instance scope. Desktop exit confirmation/force-exit integration, updater and installer cleanup remain later phases.

[Service regressions](../../agent/src/service/tests.rs), [real approval regression](../../agent/src/service/approval_tests.rs) and [isolated startup fixtures](../../agent/tests/service_startup.rs) cover real prompt finalization with queued work never starting, cancellation during Starting, retained lock/metadata, both subscription kinds, failure and idempotent reporting. Static checks compile every test target. Local execution on 2026-10-14 is recorded in section 10.4; native Windows/Linux execution and PR CI acceptance remain pending.

### Phase three Connection gates and clients

- [x] Define readiness compatibility in shared types, preserving coordinated
      business protocol upgrades.
- [x] Gate business connections/observers on version, protocol and data instance.
- [x] Handle Channel caches, instance changes, reconnects and expected-instance
      validation on requests.
- [x] Align TUI/headless external endpoint and conflict rules.

Acceptance: incompatible Agents receive no Desktop mutations or automatic
sync; replacement triggers a handshake; explicit TCP failure never redirects;
concurrent startup has one storage owner.

Phase three adds the [shared connection gate](../../packages/rpc/src/connection.rs). Read-only `get_agent_readiness` now advertises protocol, data-root identity and storage schema. Validation requires equal three-part numeric release versions (ignoring prerelease/build suffixes), protocol 1, schema 6 and a valid instance identity. Missing fields in legacy services, handshake errors and timeouts reject business calls, without a `get_agent_info` fallback or permission to start. Additive protobuf fields do not change database or business-command storage formats.

The data-root identifier hashes the canonical directory path with SHA-256; it is not a database GUID and creates no storage file. Default local connections validate the current `FUTURE_HOME`. Explicit TCP connections honor their configured target and bind the advertised root and instance UUID after compatibility validation. HTTPS endpoints fail explicitly without a configured TLS connector, rather than sending plaintext.

CLI/TUI retain their existing display versions but use shared `EXPECTED_AGENT_VERSION` for gates. Coordinated builds use injected `FUTURE_VERSION`; bare Cargo development builds use the RPC/Agent `0.0.0` release baseline, avoiding false rejection due to CLI/TUI's `0.0.2` display scheme. Isolated real-service fixtures verify this shared expected version. Future changes to the Cargo release baseline must keep Agent/RPC aligned.

GUI, headless, CLI and TUI business commands and subscriptions use `VerifiedAgentClient`, rechecking readiness before dispatch. Verified Channels cannot transparently reconnect to replacement servers; a new physical connection requires a fresh handshake. Requests carry expected-instance and expected-root headers, validated by the Agent before command conversion, admission or subscription initialization. Legacy direct RPC callers may omit headers; gated clients always send both. Desktop caches distinguish endpoint/root configuration, discard failed verification and display specific blocking reasons. channels/loop retain their existing client interfaces and have not switched wrappers in this phase.

Only an absent/refused default local endpoint with an available instance lock yields an opaque `LocalStartPermit`. Failed handshakes, version/protocol/schema/root mismatches, held/unavailable locks, permission or unknown connection errors and explicit endpoint failures return errors. Preflight does not create/rewrite lock files or initialize databases. `AgentService::with_local_start` consumes the permit and reacquires the authoritative lock before host initialization, closing startup races. Existing sidecar launches also recheck permission and explicitly pass `--grpc-addr auto`. This historical phase-three record predates phase-four Desktop hosting and did not change the default hosting mode.

[Shared gate regressions](../../packages/rpc/src/connection/tests.rs), [isolated startup fixtures](../../agent/tests/service_startup.rs), server header checks and Desktop/TUI regressions cover compatible external connections, legacy contracts, instance/root changes, physical connection replacement by a legacy server, held locks, races after preflight, explicit TCP failures and UI diagnostics. Failure cases assert no business dispatch/subscriptions or storage initialization and preserved lock metadata. Static validation includes Rust Clippy on all test targets, GUI/headless builds, Desktop type-check/ESLint, fmt and documentation checks. macOS behavioral acceptance on 2026-10-14 is recorded in section 10.4; native Windows/Linux execution and PR CI remain pending.

### Phase four Embed in GUI and headless

- [x] Add the dependency/common host and reorder locking, startup, ready and
      dependent background work.
- [x] Replace GUI child supervision with embedded handles and external state.
- [x] Integrate headless while preserving GUI-free, terminal and signal behavior.
- [x] Use global activity for exit; coordinate tasks, remote, terminals and UI.
- [x] Integrate reset/restart/environment changes; restart only after full stop.

Acceptance: default GUI/headless have no persistent Agent child; CLI/TUI use
full tools; WebView reload preserves Agent; host exit cancels CLI conversations;
ordinary external-mode exit preserves the external process.

Implementation: GUI/headless share a native-thread `agent_host`. Desktop acquires its instance lock before initializing the app store; dependent schedulers and observers start only after connection gating, authoritative Agent locking and readiness. The GUI default no longer launches a persistent Agent child. Headless preserves early non-interactive refusal, terminal authorization and Ctrl+C ownership. Short CLI skill-bootstrap and maintenance commands remain packaged.

Concurrent starts share one startup result; exit during startup cancels initialization. Embedded exit consults global Agent activity, including CLI/TUI runs, queues, approvals, compaction and blocking commands. Unknown activity requires confirmation. External mode records only Desktop's exact submitted run IDs, including requested IDs when acknowledgements are lost; it preserves the external owner and other clients' runs.

Confirmed exit freezes Desktop RPC, task and terminal creation, notifies remote clients and stops reconnecting. Embedded mode drains the entire Agent; external mode cancels queued and active work only by recorded run ID. Background futures, native maintenance jobs, nested task collectors, terminal listeners and connections have explicit completion ownership. Interrupted task records retain their existing format; terminals verify OS child reaping. Cleanup failures remain sticky across repeated calls. Data reset, environment writes and relaunch occur only after successful cleanup. Forced quit records failure and exits, allowing the process to release retained Agent ownership.

The worker polls RPC serving and shutdown reports together so a failed connection drain cannot hang it indefinitely. Agent drain uses five seconds and host waiting ten; Desktop calls, exact external cancellation and background completion have separate bounds. Complete stop permits same-root restart with a new instance ID; failed stop prohibits in-process restart. Updater plumbing separates download/signature verification, fallible cleanup and install instead of relying on an exit hook that cannot veto installation. Installer identity/lease handling, install-failure recovery and real Windows upgrade acceptance remain phase six work.

### Phase five Platform tools and development startup

- [x] Resolve/version-check Linux helpers while retaining sandbox execution.
- [x] Fix host ownership of logging, TLS, environment, CLI paths and resources.
- [x] Default all three dev launchers to embedded, retaining external debugging.
- [x] Update Makefile, doctor, status, local builds and placeholder comments.

Acceptance: icon and script launches find tools; missing CLI has local
diagnostics; helpers do not start GUI, compete for Agent locks or bypass the
sandbox; launcher cleanup leaves no self-created Agent.

Implementation (2026-10-14): all three development entry points and `make run-desktop` launch Desktop directly. They no longer build/start a standalone Agent, use an open TCP port as readiness, reclaim stale PIDs or edit SQLite rows. `FUTURE_DESKTOP_AGENT_MODE=auto` uses the shared gate; `external` is strictly attach-only even when no endpoint exists, so start that Agent separately. Explicit endpoint, FUTURE_HOME and socket overrides pass through unchanged. BUILD_CLI still defaults to building/staging the actual CLI with CARGO_TARGET_DIR support. Empty externalBin files are compile-only and rejected at runtime. BUILD_AGENT/REUSE_AGENT/CLEAN_STALE_APP_TASKS no longer control launch. RUN_CHECKS performs static checks only; DRY_RUN does no builds, filesystem writes or process launches.

Shared `cli_tool` discovery prefers packaged CLI, development artifacts and absolute PATH entries, rejecting directories, empty placeholders and non-executable Unix files without searching cwd. GUI/headless skill bootstrap share discovery; missing CLI diagnoses dependent features without preventing embedded startup. Hosts hydrate the existing login-shell environment before Tauri/Tokio threads, then prepend the CLI directory and initialize ring TLS/tracing once. After the Desktop lock is acquired, `app/logs/desktop.log` records embedded Agent output and host mode/version/instance/lifecycle. Standalone `agent/logs/agent.log` is unchanged and external Agent logs are never opened. Embedded Agent/headless Tokio runtimes each use two workers, 2 MiB stacks and a 32 blocking-thread limit, not a claim about total process threads. CLI runtime/arguments remain unchanged; performance/package measurements remain phase seven work.

Linux helpers still enter the current host executable, retaining the unified CLI's `agent` prefix. Outer/inner helpers carry the expected Agent build version, verify it after parsing and enter the existing anonymous-request-FD path before HOME creation, GUI, service locking, storage or runtime initialization. System Bubblewrap path/identity/version probing, request protocol, mounts, approvals/grants, private reporting and fail-closed rules remain unchanged. Build mismatch does not execute commands or select an unconfined fallback.

The user requested no tests in this phase. Validation is limited to fmt, Clippy, ESLint/type-check, Bash syntax and bilingual documentation checks; no units, script dry runs, application launches or sandbox smokes were executed. The acceptance matrix above remains unexecuted. Native Windows PowerShell/batch and Linux builds/sandbox, tool conversations, development process cleanup and resource behavior require later verification. Sections 10.4/10.5 record earlier worktrees and cannot validate this phase.

### Phase six Updater and Windows installer

- [x] Split download/verification/confirmation/shutdown/install; add failure
      recovery and installer launch diagnostics.
- [x] Add host identity fixtures while retaining legacy, lease, rollback,
      silent and uninstall behavior.
- [x] Unify exit, upgrade, post-update and debug restart cleanup.
- [x] Explain/authorize installer impact on external services.

Acceptance: download/signature failure leaves tasks running; cleanup failure
does not install implicitly; old sidecar releases upgrade; installation failure
restores executables and provides diagnostics; CLI locks/external owners remain
enforced.

Implementation and static checks only: behavioral acceptance, native
PowerShell/NSIS compilation and fault injection have not run.

### Phase seven CI and release acceptance

- [ ] Trigger GUI/headless/service integration checks on Agent and root
      manifest/lock changes; trigger CLI/TUI checks for their Agent dependency.
- [ ] Replace Windows child-count assertions and add real IPC/lifecycle fixtures.
- [ ] Build/sign NSIS/portable, macOS app/DMG, Linux deb and static headless.
- [ ] Smoke-test previous release, first embedded release, same-version repair
      and both upgrade paths with complete installers.
- [ ] Measure cold startup, threads, memory, package size and shutdown against
      the sidecar baseline.

Acceptance: required CI, artifact verification and native installer smoke pass.
Unexecuted platforms are not labeled passed; mock coverage is not lifecycle
acceptance.

### Phase eight Switch defaults and documentation

- [ ] Enable embedded GUI/headless defaults after acceptance and remove obsolete
      default child-supervisor branches.
- [ ] Retain standalone Agent, TUI ownership, CLI bundle and legacy installation
      compatibility.
- [ ] Update bilingual product/developer/troubleshooting docs and behavior tests.
- [ ] Explain all-client interruption, external conflicts and recovery in release
      notes.

Acceptance: normal users take the embedded path; development and artifacts
agree; no implicit external prelaunch is required for successful verification.

## 10 Test matrix and execution policy

### 10.1 Required behaviors

| ID | Scenario | Assertion |
|---|---|---|
| L1 | Concurrent hosts or Desktop/future agent startup | One owner; loser performs no migration/reset/metadata overwrite |
| L2 | Incompatible occupied service, unresponsive lock owner, install lease | Explicit error/bounded waiting, no second Agent or business writes |
| L3 | Stale file/metadata, reused PID, path aliases, different FUTURE_HOME | OS locking and full identity prevail; no unrelated termination |
| S1 | No model/login, corruption, failed legacy import, bind failure | Empty config is ready; real errors reclaim resources and stay diagnosable |
| S2 | Exit during startup, second GUI, GUI/headless conflict, slow storage | No late ready or partial writers; meaningful startup status |
| Q1 | GUI/CLI/TUI work, queues/approvals, requests while prompt is open | No early gate; confirmation stops all work and dequeue |
| Q2 | Acceptance/compaction races and concurrent exit entries | One coordinator; accepted work tracked; durable outcomes |
| Q3 | Idle chats with compaction/writer/stream work | Background work drained; subscriptions end; normal stop completes |
| Q4 | Full queue, unwritable disk, stuck worker, capability failure | No false success/early unlock; total deadline and repair metadata |
| D1 | Reopen after cancellation, forced exit or shutdown crash | Consistent transactions, interrupted-run recovery, retained history |
| C1 | CLI embedded connection, Desktop exit, reconnect/replacement | Clear cancellation/disconnect, fresh handshake, stale cursor/queue reset |
| C2 | External exit/restart, explicit unreachable endpoint, incompatible reconnect | Ownership respected, no endpoint takeover, effective gates |
| C3 | Mobile/channels/loop streams across shutdown and restart | Disconnect/cancellation is not success; approvals/queues converge; new instance is validated |
| U1 | Download, signature, extraction or launch failure | Early failure keeps tasks; post-stop failure has recovery |
| U2 | CLI/skill process, old orphan sidecar, other-install owner | File/scope/lease/outcome checks prevent mixed versions |
| U3 | Copy/loader failure, reinstall, uninstall-first upgrade, cleanup timeout | Executable rollback, retained data, truthful install/uninstall results |
| P1 | Native Windows pipe, restricted tools, cancellation and Job Objects | IPC permissions, tool execution, child cleanup and capability repair |
| P2 | Linux helper/socket and macOS icon/environment startup | Real isolation, correct paths, no helper GUI/environment races |
| B1 | GUI/headless debug/release, musl and Windows without VC runtime | Dependency boundaries, loader/static linkage and version injection |
| B2 | Missing/stale CLI, signatures, portable policy and dev scripts | Clear local failure, no sandbox bypass, matching programs, no portable NSIS |

### 10.2 Test layers

Unit tests cover admission, states, idempotence, queue cancellation and reports.
Integration tests drive real services/RPC to prove locking, cancellation,
durable reload, identity replacement and stream closure. Verifying that a
cleanup callback was invoked is insufficient.

Use separate fixture processes for locks, environment, forced exit and
watchdogs. Explicitly isolate `FUTURE_HOME`, HOME/USERPROFILE and endpoints.
Sandbox-sensitive homes belong under project target directories so temp-directory
allowance does not produce false passes.

Mock Tauri apps must not auto-start a real Agent. Inject factories/selectors for
UI tests; real embedded tests explicitly start isolated services. A dependency
is not built with `cfg(test)` merely because its parent runs tests.

Run real Windows pipes, Jobs, locks and NSIS fixtures natively; smoke full
installers in clean VMs. Verify macOS bundle paths/UI affinity/logging and Linux
bubblewrap/Unix IPC/GUI-free musl artifacts.

Update existing checks: [Windows sandbox](../../scripts/tests/test-windows-sandbox.ps1),
[Windows lifecycle](../../scripts/tests/test-windows-sandbox-lifecycle.ps1),
[installer preflight](../../scripts/tests/test-windows-installer-preflight.ps1),
[Windows install script](../../scripts/tests/test-install.ps1),
[Linux native sandbox](../../scripts/tests/test-linux-sandbox-real-machine.sh) and
[headless Linux artifact](../../scripts/ci/check-headless-linux.sh). Lifecycle
assertions become correct host ownership, locking and complete service behavior
rather than one child Agent process.

### 10.3 Local and PR checks

By default, follow repository PR preferences: no local unit tests; unit and automated
integration tests run in PR CI. On 2026-10-14 the user explicitly requested local unit tests
and acceptance of the preceding phases; this run follows that request (section 10.4). Native full-install smoke is release acceptance
in isolated VMs, never upgrades/uninstalls or kills the developer's real setup.

Use current local static commands and add new service/headless feature
combinations to CI:

```sh
cargo fmt --check --all
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --check --manifest-path desktop/src-tauri/Cargo.toml
cargo clippy --all-targets --manifest-path desktop/src-tauri/Cargo.toml -- -D warnings
cargo check --manifest-path desktop/src-tauri/Cargo.toml
cargo check --no-default-features --features headless --bin futureos-headless --manifest-path desktop/src-tauri/Cargo.toml
npm run typecheck --workspace @futureos/desktop
npm run lint --workspace @futureos/desktop
npm run stylelint --workspace @futureos/desktop
npm run build --workspace @futureos/desktop
python3 scripts/docs/check-docs.py --strict-pending
git diff --check
```

Documentation-only changes need documentation/diff checks. Implementation runs
the corresponding static checks by phase; do not substitute `make test` or an
aggregate target that executes local unit tests.

### 10.4 Local acceptance of phases one through three (2026-10-14)

Environment: macOS arm64, Rust 1.97.0, branch `codex/agent-service-layer`. Tests use isolated HOME/USERPROFILE, temporary databases and random loopback TCP/IPC endpoints. Approval-sensitive homes are under ignored worktree target directories so system-temp allowance does not change test semantics. Initial sandbox-denied bind results are excluded from code-failure conclusions and rerun with loopback binding enabled. Final GUI/TUI full reruns use four test threads; Agent and initial CLI/RPC full runs use one.

| Scope | Passed | Ignored | Execution |
|---|---:|---:|---|
| Agent library unit tests | 2,202 | 1 | `cargo test -p future-agent --lib` |
| RPC library units, including connection gates and wire parity | 146 | 0 | `cargo test -p future-rpc --lib` |
| CLI library units | 963 | 0 | `cargo test -p future-cli --lib` |
| TUI library units | 2,170 | 0 | `cargo test -p future-tui --lib` |
| Desktop GUI backend library units | 1,812 | 11 | `cargo test --manifest-path desktop/src-tauri/Cargo.toml --lib` |
| Desktop frontend, 303 test files | 4,029 | 0 | `npm run test` in desktop |
| GUI-free/headless targeted units | 17 | 0 | `cargo test --no-default-features --features headless --lib headless::` in desktop/src-tauri |
| Isolated service startup integration regressions | 10 | 0 | `cargo test -p future-agent --test service_startup` |
| Real Agent CLI integration regressions | 25 | 0 | `cargo test -p future-agent --test cli_smoke` |
| SQLite startup integration regressions | 3 | 0 | `cargo test -p future-agent --test sqlite_startup` |
| Unified CLI binary integration regressions | 16 | 0 | `cargo test -p future-cli --test bin` |

Final result: 11,339 passing unit-test executions and 54 passing integration regressions, zero failures. Twelve pre-existing ignored tests are not counted as passes. Headless targeted tests overlap with names in the GUI variant, so the total counts executions rather than entirely distinct tests.

Passing logs are retained locally under `agent/target/phase123-acceptance/results/` as ignored build artifacts.

These commands use `--offline --locked`; test concurrency is set through `-- --test-threads=N`. The initial four-library run used one combined Cargo command. Agent/TUI libraries with failures were rerun in full separately after fixes.

Acceptance fixed fixtures and waiting conditions: synthetic runs/queues now seed real durable sessions and metadata; independent Desktop mock fixtures start fresh verification and expose business counts separately from readiness diagnostics, retaining and asserting raw handshake records; macOS settings assertions respect built-in Seatbelt versus Windows/Linux probing; TUI history pagination waits for completion; Unix argv stubs use symlinks so copied macOS system binaries are not killed during execution. Storage schema, CLI arguments and production shutdown/connection gate policies were not changed to make tests pass.

Service/gate coverage includes no database initialization before held/raced locks, successful stop and retained ownership on failure, approval/queue cancellation, durable terminal outcomes, global/session stream closure, idempotent shutdown, rejected/timed-out readiness, version/protocol/schema/root/instance changes, legacy server replacement and failed explicit endpoints. Real CLI regressions include clean SIGINT shutdown. fmt, Clippy and diff checks were rerun after fixes.

This is the historical macOS acceptance record for phases one through three, before phase-four changes. Native Windows pipes/locks/Jobs/permission cleanup, Linux sandbox/helpers, embedded Desktop hosting, updater/installer changes, packaged artifacts and installation/upgrade smoke were outside that run. See section 10.5 for phase-four evidence.

### 10.5 Phase-four local acceptance (2026-10-14)

Branch `codex/agent-service-layer`, base `73ed4183`, macOS arm64, Rust 1.97.0 and the uncommitted phase-four worktree. Agent schema, storage formats and CLI arguments are preserved. Fixtures use temporary HOME/FUTURE_HOME, independent Agent sockets and mock loopback ports; no existing developer Agent was stopped. GUI/headless full suites ran sequentially because existing remote fixtures share port 8022.

| Scope | Passed | Ignored | Result log (directory below) |
|---|---:|---:|---|
| Full Agent library units | 2,202 | 1 | `agent-tests.log` |
| Full RPC compatibility/connection gate units | 146 | 0 | `rpc-tests.log` |
| Full GUI backend units | 1,800 | 11 | `gui-tests.log` |
| Full GUI-free/headless backend units | 1,461 | 11 | `headless-tests.log` |
| Isolated embedded-host regressions | 9 | 0 | `host-tests.log` |
| Final shutdown/background/reset/updater reruns | 36 | 0 | `shutdown-tests.log`, `runtime-tests.log`, `debug-tests.log`, `update-tests.log` |
| Final headless shutdown/entry/signal reruns | 19 | 0 | `headless-shutdown-tests.log`, `headless-entry-tests.log` |
| Agent startup, real CLI and SQLite integration | 38 | 0 | `agent-integration.log` |
| Frontend AgentStatusGate | 6 | 0 | `status-gate-tests.log` |

Targeted and full suites overlap; these numbers are executions rather than entirely distinct cases. After full-suite acceptance, cleanup-runtime construction failure was added as a fault injection and shutdown/reset/environment/updater/entry paths were rerun with GUI/headless features. Background-registry completion reclamation was separately rerun. Logs retain final passing results; ignored cases are not counted as passes.

Real host fixtures verify metadata PID equals the host PID, concurrent startup shares one service, complete restart changes instance ID, cancelled startup never becomes ready, held-lock refusal creates no database/auth, attached-host exit preserves the external owner, CLI streaming prompts and queued follow-ups persist cancellation, long CLI shell commands stop, and stalled client drains fail within the deadline while retaining metadata/lock and prohibiting restart. Additional regressions cover global/unknown activity decisions, late-work admission, native completion, interrupted task persistence, terminal reaping/creation freeze, and cleanup failure blocking reset/environment writes/relaunch. Signature failure asserts unchanged Agent state and Desktop admission.

GUI/headless and Agent/RPC/CLI/TUI all-targets Clippy (`-D warnings`), both manifest fmt checks, Desktop ESLint/type-check and bilingual documentation checks pass. The headless binary builds with no Tauri/Wry/GTK/WebKit in its dependency tree. Fresh-process smoke verifies help creates no FutureOS root and first non-interactive refusal initializes no Agent database, instance lock or listener.

Logs are preserved under ignored `agent/target/phase4-acceptance/results/`; rerun the table's scope with its corresponding features. Unit terminal transport fixtures own their listener across HOME guards, whereas production listeners/connections remain registered for Desktop cleanup. Standalone updater fixtures explicitly initialize the host TLS provider rather than depending on another test to do so first.

This accepts macOS service/backend paths. Real GUI dialog interaction, packaged CLI/TUI full-tool smoke, native Windows pipes/locks/Jobs/ACL, Linux helpers/sandbox, installer identity/lease handling, install-failure recovery, signed package size and real Windows installation/upgrades remain phases five through eight work. It does not establish resolution of all Windows upgrade issues. GUI/headless now default to embedded hosting on this development branch; release default rollout still requires the later build/script/installer acceptance.

### 10.6 Phase-six implementation and static checks (2026-10-14)

Installation confirmation follows download/signature verification and explains
the scope of cancellation, including CLI work on embedded hosts. A single
installation guard survives cancellation of the invoke while the native
cleanup/install worker runs. Exit, debug restart, upgrade and post-update restart
share fallible complete cleanup. Cleanup failure never calls installation or
reopens frozen producers.

Windows accepts the current signed raw NSIS EXE format. Format, staging, restart
arguments and writable diagnostics are prepared before shutdown; ZIP/MSI fail
before stopping work. A PowerShell observer acknowledges readiness within ten
seconds, holds the original Desktop process handle and waits for its exit
before launching installation. After sixty seconds without exit it aborts.
Installation itself is not killed on timeout, which could interrupt rollback.
The new `/P /R /UPDATE /FUTUREOS_CLEAN` path only checks occupancy: any Agent,
CLI or unknown file locker blocks replacement. Arguments/environment are
preserved and NSIS launches the new app on success. Failure records the exit
code and compares every current/legacy executable with its original SHA-256,
also checking the rollback-failure marker. Only complete restoration permits
launching the old Desktop. Diagnostics live at
`FUTURE_HOME/app/logs/update-recovery.log`; unverifiable or incomplete recovery
requires repairing the installation.

Preflight recognizes GUI/headless embedded hosts and legacy Desktop/Agent
names, retaining PID, executable path, data-root and process-start checks.
Legacy `/UPDATE` may close verified processes in this installation only;
another installation's lock owner blocks it. Interactive Yes explicitly
authorizes stopping verified external hosts using the same data root.
Ordinary silent/passive installation never implicitly stops tasks. The lease
covers permission cleanup, backup, replacement, verification and rollback.
The installer carries new maintenance CLI code so legacy installed versions
need not support the reset command. The local Windows installer builder and
release workflow copy the CLI after signing into
`binaries/installer-maintenance.exe`; direct NSIS builds must stage it too.
Cleanup failure/timeout releases the lease and aborts before replacing files.
Uninstall retains its existing best-effort cleanup policy.

Non-Windows failures keep the current process frozen. Restart recovery is
offered only when original program/CLI contents and permissions still match;
macOS checks the whole app bundle. Changed or missing files require package
repair rather than relaunching a mixed installation.

Added Rust test definitions cover cleanup blocking installation, installation
failure recovery, Windows argument escaping and changed program files blocking
restart. Installer fixtures cover embedded host identity, external-owner
gating and cleanup failure preserving executables.
`scripts/tests/test-windows-update-launcher.ps1` provides offline success,
launch failure, installer failure and incomplete rollback observer fixtures.
These tests and real installations were not run, as requested. macOS
GUI/headless Clippy, fmt, ESLint/typecheck and documentation checks are static
checks only. PowerShell, NSIS and a Windows Rust target are unavailable on this
host; native compilation and lifecycle acceptance remain phase-seven work.

### 10.7 macOS acceptance and Windows handoff (2026-10-14)

User-authorized macOS testing uses isolated homes/data and never connects to
or stops the user's Agent. New branch: `codex/embedded-agent-macos-validation`.
Initial restricted-environment local-network failures are not acceptance
results. Native reruns passed:

| Scope | Passed | Ignored |
| --- | ---: | ---: |
| Agent library | 2206 | 1 |
| RPC library | 148 | 0 |
| CLI library | 963 | 0 |
| TUI library | 2170 | 0 |
| Desktop GUI library | 1809 | 12 |
| Desktop headless library | 1466 | 12 |
| Agent CLI/startup/SQLite integration | 53 | 0 |
| CLI binary entry and event contract | 20 | 0 |
| AgentStatusGate / UpdatePage / remote replay | 56 | 0 |

Merged `origin/main` (`4bd5eab3`) after committing, then reran the affected
Agent, GUI and headless suites; their table entries are post-merge results.
Root/GUI/headless Clippy, fmt, ESLint/typecheck and all 260 documentation checks
passed. Updated the doctor diagnostic golden fragment too; the complete CLI
golden corpus was not run in this pass.

Fixed stale helper-argument, doctor-diagnostic and connection-gate assertions.
RPC roots preserve Agent's `HOME -> USERPROFILE -> platform profile` order,
preventing custom Windows HOME from splitting host/client state. Installer
checks reject drive-relative and root-only overrides. RPC's 148 cases and
nine real-host cases passed again after the root fix. GUI/headless suites run
sequentially to avoid shared fixture ports. Logs are in ignored directory
`agent/target/phase6-macos-tests/results/`.

Windows requires the existing Rust/MSVC, protoc and Node prerequisites:

```powershell
git fetch origin
git switch codex/embedded-agent-macos-validation
cargo test --locked -p future-agent -p future-rpc -p future-cli -p future-tui --lib
$triple = (rustc -Vv | Select-String '^host:').Line.Split(' ')[1]
$cli = "desktop/src-tauri/binaries/future-$triple.exe"
if (-not (Test-Path $cli)) {
    New-Item -ItemType Directory -Force desktop/src-tauri/binaries | Out-Null
    New-Item -ItemType File $cli | Out-Null # Compile only; preserve real CLI
}
cargo test --locked --manifest-path desktop/src-tauri/Cargo.toml --lib
cargo test --locked --manifest-path desktop/src-tauri/Cargo.toml --no-default-features --features headless --lib
```

Run Windows GUI/headless sequentially too. Empty CLI files are compile-only;
NSIS packaging requires real staged maintenance code. macOS results do not
validate Windows named pipes, Job/ACL behavior, PowerShell, NSIS compilation,
rollback or real upgrades.

## 11 Release conditions and rollback

Require evidence for real service/persistence behavior, native platform
lifecycle, complete installer/update/rollback, and aligned development/release
entry points. Record version, platform, package, mode, scenario, outcome and log
location rather than just a green CI status.

Measure first/repeat readiness, idle/concurrent threads and memory, signed
package size and normal/forced cleanup times. Establish a sidecar baseline on
the same machine/config/workload. Phase seven defines acceptable regression
thresholds; do not assume memory or startup improves.

A controlled sidecar host option may remain during migration for comparison
and rollback, but CI and smoke must exercise the default embedded path. It
cannot bypass locking, incompatibility or complete shutdown. Decide its final
retention during release acceptance.

Code rollback stays within the same storage contract. Change hosts/restart only
after complete stop. Preserve user data/WAL on installer rollback and do not
downgrade schema. Users upgrade or stop incompatible external services; recovery
does not delete data or kill unidentified processes.

Use a work branch for PRs. Fetch and merge current remote main before
submission, resolve conflicts, pass type-check, ESLint, fmt and Clippy, and do
not run local unit tests. Create a ready-for-review PR, enable auto-merge and
wait for CI/repository conditions. Split PRs where phase interfaces can be
validated independently; do not mark unfinished phases delivered.

## 12 Definition of done

- [ ] Complete embedded Agent is the GUI/headless default; CLI/TUI can execute
      real tool tasks through it.
- [ ] Incompatible/occupied external services cannot be bypassed; background
      business requests are gated too.
- [ ] Global prompts/cancellation include other clients; lock release follows
      complete cleanup.
- [ ] Update, exit, restart, reset and headless signals share cleanup semantics.
- [ ] Stuck cleanup never creates two in-process Agents; abnormal reopen
      recovers transactions and run state.
- [ ] Linux helpers, Windows sandbox, tool paths, file logs and GUI-free
      distributions remain correct.
- [ ] CLI packaging/signing, legacy upgrades, preflight, leases and rollback
      remain effective.
- [ ] Launchers, Makefile, CI filters, native checks and complete package smoke
      cover the new default.
- [ ] Performance/size have baselines and acceptance records; bilingual docs
      and product behavior agree.

## 13 References

- [RPC architecture](rpc.md), [shared packages](packages.md),
  [SQLite operations](sqlite-migration.md).
- [Build and install](../guide/build-and-install.md),
  [headless Desktop](../guide/desktop-headless.md),
  [directory layout](../guide/directory-layout.md).
- [Windows updates](../internals/desktop-windows.md),
  [Linux sandbox](../internals/desktop/SANDBOX/LINUX.md),
  [Windows sandbox](../internals/desktop/SANDBOX/WINDOWS.md).
- [Tauri updater API](https://v2.tauri.app/reference/javascript/updater/)
  describes Windows application exit after installer launch.
- [Tokio runtime](https://docs.rs/tokio/latest/tokio/runtime/struct.Runtime.html)
  describes runtime shutdown and blocking-work limitations.
- [Cargo features](https://doc.rust-lang.org/cargo/reference/features.html)
  describes dependency feature unification; actual acceptance uses both
  lockfiles and release builds.
