# Headless Package Development Plan

> [中文](headless-package-development.zh-CN.md)
>
> Design date: 2026-10-20. Source baseline: `904eab2b`, including Remote extraction merged through PR #1235.
> Status: in progress. Source/dependency baselines are captured and shared-host extraction is starting; phases 0–6 are incomplete.

Extract the application host without a GUI from the Desktop crate and start it in the foreground with `future headless`. Introduce `future-host`, a Rust library without GUI dependencies, for the application backend shared by Desktop GUI and headless. Desktop and CLI own their entry points and interaction. Preserve current functionality, data, pairing, approval, sandbox and shutdown behavior.

The existing contract is documented in the [Headless guide](../guide/desktop-headless.md), [Desktop Agent guide](../guide/desktop-agent.md) and [Remote extraction plan](remote-package-development.md). Update operational guides as implementation lands; commands described as targets here are not commands already provided by the current release.

## 1 Goals and scope

The final entry points are:

```bash
future headless
future headless --relay http://192.168.1.10:8099
future headless --relay https://relay.example.com --relay-ca relay-ca.pem
future headless --help

future remote devices
future remote devices add
future remote devices remove <id>
future remote devices use <id>
```

`future headless` remains in the foreground, guides login and pairing when needed, reuses valid credentials, serves mobile and existing remote requests, and exits on Ctrl+C. `future agent` continues to start the Agent service alone. `future headless` hosts the complete application backend and starts an embedded Agent or connects to a compatible external Agent under the existing ownership rules.

Final distributions provide headless through `future`, without requiring a separate `futureos-headless` executable. Desktop GUI remains `futureos`. Daemonization, system-service installation, background flags, new Remote protocols and new CLI/TUI controller product features are outside this migration.

This changes entry points and code ownership. Data paths, locks, communication contracts and business semantics do not change with package names. Concurrent GUI/headless ownership of the same data directory is outside scope.

## 2 Current coupling and target structure

The [current headless launcher](../../desktop/src-tauri/src/headless/mod.rs) belongs to `futureos`. Features already exclude GUI system libraries, but it still uses Desktop Store, Agent management, login, Remote host adapters, scheduling, observers and shutdown coordination. Adding a CLI dependency on Desktop would retain these build and test ownership problems.

Create `host/` with the crate name `future-host`, as an application host library in the root Cargo workspace, sharing the lockfile and build artifacts. It is an ordinary Rust library, with no separate executable or `staticlib`/`cdylib` targets. `host/` is an application runtime and may compose services such as `future-agent`; it differs from the product-independent protocol and role libraries under `packages/`.

```mermaid
flowchart TD
    CLI[future-cli headless entry] --> Host[future-host without GUI dependencies]
    GUI[futureos Tauri GUI entry] --> Host
    Host --> Agent[future-agent and Agent RPC]
    Host --> Remote[Remote Server and required role services]
    Host --> State[Application data tasks approvals files and synchronization]
    Remote --> Protocol[Remote Protocol and Crypto]
```

Dependency requirements:

- `future-cli` and `futureos` may depend on `future-host`. `future-host` must not depend on `future-cli`, `futureos`, Tauri, React, Expo or a WebView, including through an optional GUI feature.
- The host can use existing `future-app-settings`, `future-app-workspaces`, `future-tasks`, RPC and Remote libraries. Do not duplicate their models, state machines or persistence implementations.
- Remote libraries must not depend back on `future-host`. They continue to access business services through host interfaces; protocol and crypto tests do not start the complete application host.
- Cargo feature unification when building CLI and GUI together must not introduce GUI dependencies into the CLI. Desktop owns GUI adapter implementations.

### 2.1 Code ownership

| Capability | Target owner | Boundary |
|---|---|---|
| Headless arguments, help, terminal login/pairing display, signals and exit codes | CLI entry module | Calls shared services; terminal interaction stays outside protocol and Store |
| Future home, environment initialization, instance lock, logging and lifecycle | `future-host` | One initialization sequence and ownership model, without GUI startup |
| Agent startup, compatibility/readiness, embedded/external ownership and stop | `future-host`, reusing Agent APIs | Preserve user, singleton and endpoint rules; do not create another Agent implementation |
| Store, session/run metadata, workspaces, configuration, identity and pairing persistence | `future-host` and existing shared data libraries | Preserve authoritative storage, atomic writes, migrations and lock order |
| Business, event projection, recovery and observers in `agent_bridge` | `future-host` | Deliver events through interfaces without Tauri types |
| `remote_host` business/catalog/file/pairing adapters and local Remote control | `future-host` | Preserve real Agent, approval, sandbox and file behavior |
| Model refresh, skill bootstrap, task ticks, delete outbox and watchdog | Explicitly started/stopped shared host services | Start each once per host; all tasks support stop and awaited cleanup |
| Windows, Tauri commands, menus, tray, system plugins, clipboard and GUI events | Desktop | Thin adapters call business APIs; do not move the entire commands module into the library |

Classify code by actual call chains rather than copying whole directories. Split business logic from Tauri types before moving mixed modules. Expose capabilities the two entries need, without making all Store or supervisor internals public for migration convenience.

### 2.2 Lifecycle and maintenance dependencies

`future-host` exposes explicit configuration, startup results, event output and observable stop/wait interfaces. Entries adapt terminal display, GUI events and required interaction responses. Display variants must not change credential commits, invitation lifetime or established-pairing detection.

The entry owns the runtime; `future-host` service APIs do not create runtimes or call `block_on`. CLI follows its [existing component dispatch](../../cli/src/main.rs), routing `headless` before the ordinary CLI runtime and creating the runtime in that entry to drive the host. GUI supplies its existing runtime capabilities. Retain current headless resource limits unless separately validated. Neither entry nests runtimes.

Current headless maintenance invokes a matching CLI for commands such as `future init`. Inject maintenance capability from the entry: CLI may invoke its current executable with the `init` subcommand; Desktop keeps using a CLI validated for version and capabilities. `future-host` must not import CLI command implementation or redispatch `headless` recursively. Preserve the current behavior where maintenance failures are reported without blocking remote service.

The in-process host may initially retain the current single-instance model; multiple hosts within one process are not a requirement. Give background tasks explicit lifecycle ownership rather than creating two independent copies of process-global state.

## 3 Functional and data compatibility

| Contract | Required behavior after migration |
|---|---|
| Data root | Preserve `FUTURE_HOME`, HOME/USERPROFILE resolution and default paths; no new headless-specific data tree |
| Application and Agent data | Keep `~/.future/app/app.db`, Agent configuration/sessions and workspace layout, including migrations, run/approval recovery and file references |
| Identity and credentials | Preserve device/Noise identities, pairing book, control pointer, pending revocations, account credentials and file formats; established devices do not re-pair |
| Same-directory exclusion | GUI and headless share the existing lock, including `app/desktop.lock`; renaming a command must not create two locks or delete active lock files |
| Remote entry | Preserve first login/pairing, stored pairing reuse, active/standby devices, refresh/recovery and revocation; retain wire fields and the `futureos://remote/pair` invitation scheme |
| Self-hosted relay | Preserve `--relay`, `--relay-ca`, saved settings and current invitation-origin/TLS policy; do not require platform login for the account-free relay path |
| Local device management | Existing `future remote devices` subcommands remain; `add` obtains a usable invitation from the active host through the existing `run/remote-control` file request/reply channel |
| Business and synchronization | Mobile sees the same sessions, history, models, workspaces, files, tasks and approvals; Agent and Store retain their authoritative roles |
| Models and skills | Keep initial/periodic model refresh, post-login skill bootstrap and failure handling without introducing parallel caches/configuration |
| Security boundaries | Preserve approvals, sandbox/path restrictions, file hash/cache rules, atomic credential writes and sanitized diagnostics; help/version do not start services or write data |
| Exit | Foreground operation; Ctrl+C and Unix SIGTERM/SIGHUP work during login, pairing and service; stop admission/recovery before bounded cleanup |
| Agent ownership | Stop owned Agent services according to current rules; preserve external Agents and cancel only this host's owned requests |

Preserve current runtime/next-start behavior of commands such as `remote devices use`. Entry migration does not change device-management semantics. Update obsolete program names in command hints, status text and diagnostics rather than preserving incorrect usage text.

Both upgraded GUI and headless must read the same old data. Use dedicated test-user data created by the old version, then verify upgrade and sequential restart of both entries. New-format fixtures cannot replace stored-pairing compatibility checks. Package renaming does not justify an irreversible schema change.

## 4 Implementation phases

| Phase | Deliverables | Exit condition |
|---|---|---|
| 0 Baseline and ownership | Fixed commit, startup/exit/data/release contracts, original-test destination inventory and build baseline | Every existing function/test has a retained owner; original GUI/CLI/headless dependency graphs are comparable |
| 1 Shared library | `host/` / `future-host`, ordinary library target, error/event/configuration boundaries and independent tests | CLI can depend on it without `futureos`; its graph contains no GUI framework |
| 2 Shared backend | Move Store/config, Agent hosting, business bridge, Remote host, scheduling and shutdown incrementally; reconnect GUI | GUI shares one implementation; existing protocol, data, approval and sandbox paths remain |
| 3 Unified command | Add `headless` to `future`; migrate terminal interaction, signals, maintenance and device-control wiring | `future` alone hosts the full backend; arguments, new/stored pairings and shutdown at every stage remain compatible |
| 4 Test migration | Move core tests to their libraries; retain CLI/GUI entry and real product integration; add versioned compatibility scenarios | Migration inventory closes; GUI-free tests are independent and shared core suites are not repeated per entry |
| 5 Build and release | Update CI, make, sidecar, install/uninstall, upgrade, static builds and distribution instructions | Supported release packages/source installs run `future headless` without hidden dependence on the old binary |
| 6 Acceptance and cleanup | Same-candidate CI, artifact smoke, performance records and manual confirmation; remove old entry/adapters | Required automation passes, manual results are confirmed, duplicate implementation is removed and docs match artifacts |

Commit traceable batches in an isolated branch and update the phase table. A temporary old launcher may forward to shared implementation to keep intermediate builds working. The final `future headless` must not launch an external `futureos-headless`, and no complete backend copy remains.

## 5 Test ownership and acceptance boundaries

### 5.1 Independent future-host tests

The new library must compile, lint and test independently without Desktop, Tauri mocks, WebView or npm. Commands are `cargo fmt -p future-host --check`, `cargo clippy -p future-host --all-targets -- -D warnings` and `cargo test -p future-host`.

Library tests use explicit event receivers, controlled time, fake host capabilities, temporary roots and isolated services. Cover lifecycle, task ownership, startup-failure cleanup, stop/wait, Store/config, synchronization, recovery and scheduling. Shared service tests run in their owning library, without repeating the complete suite in CLI, GUI and headless configurations.

Independent testing includes units and necessary library integration. GUI-free does not mean replacing real business behavior with mocks: migrations, atomic writes, permission/approval boundaries, real Agent RPC and embedded/external ownership still require real integration tests, without a graphical app.

### 5.2 Original test destinations

| Current tests or behavior | Target owner | Retained validation |
|---|---|---|
| Arguments and terminal cases in `headless/mod.rs` | CLI headless module/process tests | Rejected flags, side-effect-free help, TTY, login/pairing display and exit codes |
| `headless/agent.rs`, `agent_host`, shutdown coordinator and shared scheduling | `future-host` units/integration | Agent ownership, startup/readiness failure, stop order, total deadline and cleanup |
| Store, business bridge, config/identity and Remote business host | `future-host` or existing shared packages | Original database/file/approval assertions, beyond verifying interface calls |
| [headless_cli.rs](../../desktop/src-tauri/tests/headless_cli.rs) | Real CLI `future headless` process tests; retain GUI-specific portions in Desktop | Built `future`, first-run non-TTY refusal, existing config, signals, exclusion and external-Agent survival |
| [remote_start_refusal.rs](../../desktop/src-tauri/tests/remote_start_refusal.rs) | CLI headless product process tests; shared fault policy in host/Remote libraries | Real active host, platform refusal, single-device revocation, recovery budgets and persisted results |
| Remote packages and [standalone interoperability](../../tests/remote-interop/Cargo.toml) | Retain their existing owners | Protocol, crypto, role state machines and real endpoint interoperability remain outside the host |
| Tauri commands, windows/menus, IPC and GUI lifecycle | Desktop | Real shared-backend wiring, window exit and GUI plugin behavior |

Map each original test name and assertion responsibility. Establish replacement coverage before removing originals. Test shared business once and each entry's wiring separately, while retaining process, platform and real side-effect coverage.

### 5.3 CLI and application integration

CLI tests execute Cargo's actual `future` artifact with the `headless` subcommand, rather than just an internal startup function or a fake script. GUI tests retain business/IPC results through the same host. Mobile keeps using the wire contract; this Rust migration does not require rewriting its client.

Key scenarios include old-data upgrade, new/stored pairings, self-hosted relay, device addition, workspace/session streaming, approval refusal/allowance, sandbox refusal, file transfer, tasks, background recovery, stop during transfer and restart. Cover both owned embedded Agent startup and attachment to an existing external Agent.

Never connect to or stop the user's normal Agent. Use isolated homes/user directories and unoccupied endpoints. Same-directory exclusion uses two real processes. Isolate environment/global-state cases in processes instead of introducing cross-test races.

### 5.4 Automated and manual acceptance

During development, run necessary compilation and static checks. Scope pre-submission fmt and Clippy `--all-targets` to changed modules, adding type-check/ESLint only for TS adapter changes. Follow the PR workflow: no local unit execution, with automated acceptance in CI, unless the user explicitly changes that arrangement.

CI reports shared library, CLI entry, Desktop GUI and artifact checks separately. Shared fixtures/schema, Cargo manifests/lock, features and build/install scripts participate in affected selection. Host changes select both Rust consumer entries; Rust-only headless changes do not unconditionally rerun all Mobile/frontend unit suites. Retain necessary end-to-end compatibility validation.

After automation passes, freeze a full candidate commit and use matching GUI, CLI and Mobile for manual testing. Record which Windows/macOS/Linux and Android/iOS combinations ran. Missing environments remain unexecuted rather than borrowing another platform's success.

| Manual scenario | Expected behavior |
|---|---|
| Fresh `future headless`, login and pair as needed | Complete terminal flow and working mobile; restart reuses valid credentials |
| Upgrade old pairing/data, start GUI/headless sequentially | Existing sessions, workspaces, models, tasks, approvals and files remain without re-pairing |
| Self-hosted relay and `future remote devices add` from another terminal | Existing auth/trust rules, usable new invitation and intact established devices |
| Remote prompts, streaming tools, approval/sandbox, files and tasks | Same results as the old host, without device/session mixing |
| Ctrl+C at startup stages or during requests/transfers, then restart | Bounded exit, no late writes/resurrected work and retained pairings |
| Attach to external Agent, exit, and try a second same-directory host | External Agent survives; second host is refused without state damage |
| CLI-only server and official package upgrade | Complete headless without Desktop/GUI dependencies; correct version, help, paths and uninstall |

## 6 Build and release adjustments

### 6.1 Cargo and make

`future-host` is an ordinary library without GUI features. `future-cli` includes headless by default; CLI-only distributions need no separate backend download. Desktop uses the same library for GUI. Once migration completes, remove Desktop's `headless` feature, standalone headless bin and compile branches used solely for the old configuration.

`make build-cli` / `make install-cli` produce the capable `future`. Add a clear `make run-headless` entry. During migration, existing `build-desktop-headless` / `install-desktop-headless` targets may remain forwarding aliases for CLI build/install, with help naming the new command and no separate headless output. Avoid building/installing the same CLI twice in `make install`.

Preserve combined CLI/GUI Cargo builds and shared feature resolution/artifacts. Adjust [build-desktop.mjs](../../scripts/build/build-desktop.mjs) and platform wrappers. Verify dependencies with `cargo tree` and actual artifacts, rather than treating `default-features = false` as proof.

### 6.2 Distributions and installation

| Distribution path | Adjustment |
|---|---|
| CLI-only platform archives | One `future` provides full headless; update help, capability discovery, version and smoke tests |
| Linux musl/server | Build static `future` and execute `future headless`; retain architectures, no dynamic GUI dependency and old-Linux baseline |
| Linux GUI deb/portable | Keep GUI plus matching CLI; remove checks/copies/manifests requiring `futureos-headless` |
| macOS app and signed packages | Preserve matching CLI sidecar version, signing/notarization, architecture and discovery; standalone CLI runs headless |
| Windows installer/portable | Support `future.exe headless`; retain CLI signing, CRT/sandbox helper and upgrade paths; verify console/signals separately |
| Installation, update and uninstall | Apply existing ownership rules to obsolete installed artifacts; do not delete user-created same-name files or move data; update service/scripts and obsolete hints |

Inspect [CI](../../.github/workflows/ci.yml), [generic builds](../../.github/workflows/build.yml), [Linux builds](../../.github/workflows/build-linux.yaml), [macOS signing](../../.github/workflows/build-macos-signed.yml), [Windows signing](../../.github/workflows/build-windows-signed.yml), release/installer workflows, [Makefile](../../Makefile) and packaging scripts. Do not assume only Linux needs changes.

Adapt [check-headless-linux.sh](../../scripts/ci/check-headless-linux.sh) to the actual released `future`: verify static ELF linkage, then execute `future headless --help` and first-run non-interactive refusal in an offline container with old Linux and no GUI stack. Retain both linkage inspection and runtime smoke. When GNU GUI builds consume a prebuilt musl CLI, do not rebuild an unused CLI or mix machine code across targets.

Adding the host must not make `future version --json`, help or early Linux sandbox-helper dispatch initialize databases, logs, Agent or locks. Matching sidecar capability/version checks and atomic replacement rules remain.

### 6.3 Performance records

Record build, link, test execution, binary size and startup costs under equivalent runner, toolchain, target, features, commands and cache conditions. Report the cost of including the complete host in CLI, GUI sharing benefits and core-test deduplication separately. Fewer executables or moved code alone do not establish a compilation speedup.

## 7 Documentation and completion checklist

Implementation updates CLI help; Headless, Agent, build/install and directory guides; device-management hints; self-hosted-relay instructions; distribution Readmes; wiki and AGENTS module ownership. Inventory old `futureos-headless` references in external deployment examples, service scripts and uninstall logic.

- [ ] `future-host` is a new GUI-free ordinary Rust library with independent compile, lint and tests.
- [ ] Desktop and CLI share the business backend; CLI/host depend on neither `futureos` nor GUI frameworks; Remote libraries remain independent.
- [ ] `future headless` alone provides current functionality, help/version have no side effects and runtimes are not nested.
- [ ] Old data, pairing, identity, locks and local control remain compatible; same-directory exclusion is retained.
- [ ] Login, relay, tasks/models/skills, real approval/sandbox/files, recovery and shutdown ownership are preserved.
- [ ] Original tests have traceable destinations; independent host tests and CLI/GUI entry/product integration boundaries are implemented.
- [ ] Affected CI, Linux static artifacts and platform install/update/signing/uninstall validation are complete.
- [ ] Final implementation/distribution has no separate `futureos-headless` dependency; transitional copies/forwarders are removed.
- [ ] Matched performance records, same-candidate automation and user-confirmed manual results are available.
- [ ] Bilingual documentation, command help and distribution instructions match implementation.

Creating a package, compiling an entry or merging a PR does not complete these requirements. Record actual phase changes, commands, candidate and remaining limitations; close the migration after manual confirmation.

## 8 Implementation record

### 2026-10-20 Baseline capture

Implementation uses `refactor/headless-host` in `.worktrees/headless-host`. Plan commit `e2251323` was integrated first; product source baseline is `904eab2b`. No PR is created and no local unit or NATS tests run.

The [test migration inventory](headless-test-migration.json) captures digests for 204 source files and 2143 test declarations identified by original path, line and function name. Declarations are not execution counts; final CI must reconcile conditional compilation, macros and runtime discovery. Initial ownership assigns 1535 to the shared host, 35 to CLI and 160 to Desktop, while 413 mixed-module cases need individual classification before originals may be removed. Planned destinations do not establish implementation or execution.

The [build/dependency baseline](../testing/headless-baseline-2026-10-20.json) records actual Cargo feature graphs for CLI, GUI and headless, Cargo/lock/build/release source digests, toolchain and compatible paths. Only metadata/dependency queries ran, without build timings or tests. Final performance still needs matched measurements.

Phase 0 source capture is recorded; test ownership remains open. Phases 1–6 are incomplete and the original entry remains current.

### 2026-10-20 Initial shared-library extraction

Created ordinary `future-host` in `host/`, without bin/FFI targets or GUI features. Existing shared error categories, boundary conversions and string serialization now live in `host/src/error.rs`; strict configuration reads, per-path locks, atomic writes and permissions live in `host/src/config_io.rs`. Original Desktop modules temporarily forward to the library. Thirteen original test declarations moved with implementation instead of being copied into Desktop. Fixture-only lenient/snapshot/restore helpers are available through `test-support` to existing application integration; their unit tests belong to the new library.

CLI and Desktop depend on the library, but `future headless` is not wired yet. `scripts/ci/host-boundaries.py` checks actual combined Cargo feature resolution with Desktop GUI enabled, rejecting Desktop/GUI dependencies in host/CLI, a host dependency back on CLI and host product-binary/FFI artifacts. Native macOS validation passes; CI must validate other targets separately.

The [static check record](../testing/headless-host-phase1-2026-10-20.json) covers default/test-support Host, CLI+GUI and original headless Clippy `--all-targets`, fmt and dependency boundaries. Test targets were compiled, with zero executions. Phase 1 lifecycle/event interfaces, most phase 2 backend work, phase 3 command wiring and subsequent test/release acceptance remain incomplete. Transitional facades will be removed at migration cleanup.

### 2026-10-20 Implementation continuation and backend leaves

A new worktree `.worktrees/headless-unified` on `refactor/headless-unified` inherits the committed baseline and initial extraction without modifying the earlier worktree. The original plan branch is explicitly merged (including `e2251323` ancestry); current `origin/main` is merged independently of local `main`.

Approval-rule atomic persistence, git diff parsing, run-error classification and platform child-process flags now live in `future-host`, with Desktop import facades. Original tests moved with their implementation; the inventory records their new ownership. Combined host/CLI/GUI Clippy `--all-targets`, formatting and dependency boundaries pass on native macOS. See [the check record](../testing/headless-host-leaves-2026-10-20.json). The documentation gate reports a pre-existing unwhitelisted Agent database fixture README. No unit/NATS tests run locally. All phases remain subject to the final CI, artifact and manual gates.

### 2026-10-20 Shared task registry and presentation outlet

The process task registry, future-drop acknowledgement, native-job tracking, admission freeze and bounded cleanup now live once in `host/src/runtime.rs`. Production code requires an entry-owned Tokio handle and does not construct a fallback runtime. Desktop supplies its existing Tauri runtime's Tokio handle; the transitional headless entry installs its own existing runtime. Seven original registry tests move to the host and use a registry lock/reset fixture instead of an unrelated HOME/Store fixture. Tests retain timeout, panic, unpolled-drop and task-lifetime assertions. Test-only runtimes remain isolated fixture infrastructure.

`host/src/events.rs` defines the presentation outlet without GUI types. Existing Desktop event serialization/coalescing stays in Desktop; setup attaches the adapter before producers start. Store/bridge notifications continue through the common outlet. Missing presentation is a no-op and does not disable Remote or persistence. The old `no_gui.rs` duplicate notification implementation is removed. Agent settings invalidation and provider configuration events use the same outlet and preserve event names/payloads.

These are incremental phase 1/2 changes. Agent owner, Store, Remote, scheduler and full shutdown coordination still remain in Desktop; this is not candidate acceptance. Original headless and GUI test targets compile via Clippy, with zero local test executions.

### 2026-10-20 Credentials, platform policy and skill guide

Credentials retain the strict atomic file reader/writer in `host/src/auth_store.rs`; platform URL and release-channel policy, build-channel classification, HOME resolution and skill-guide requests move to the host. Shared host build identity uses the embedded Agent's existing build version, keeping RPC and release policy aligned without a second host version generator. TLS provider initialization also lives once in the host.

The HOME fixture shares its serialization lock between Desktop and host. An entry-supplied fixture cleanup callback drains remaining Desktop resources and closes pooled connections before changing HOME. Core credential/platform/guide tests move with their implementation. The real pooled-database file-release regression remains in Desktop until the Store moves. No business assertion is replaced by a callback assertion. Static evidence is [recorded here](../testing/headless-host-config-2026-10-20.json); runtime compatibility and cross-platform acceptance remain pending.

### 2026-10-20 Shared Store and historical database fixtures

The application Store now lives in `host/src/store.rs` and `host/src/store/`: connection pool, schemas and versioned migrations, threads/runs, CAS status/projection writes, approvals and assessments, artifacts and files, workspaces, deletion outbox, legacy history, reviews and skill-recommendation records. All eight SQL files, including the old `pre-agent` and `v1.2.1` fixtures, are byte-identical to parent `40f993d4`. Paths, schemas and file formats are unchanged. CLI's existing data-upgrade fixture points at the same moved SQL file.

277 original test declarations (including the real pooled-connection cleanup regression) move to the host. Nine existing Agent/Store coordination tests remain in `desktop/src-tauri/src/store/reconcile.rs` with their original RPC and database assertions; inventory marks this intermediate location until the business bridge moves. The remaining adapter queries narrowly scoped candidate APIs; it cannot open Store internals directly. Database seed fixtures have one shared implementation. The legacy event writer/buffer is available only to test/test-support builds, preserving the existing on-disk reader assertions for entry integration.

Agent event projection and invalidation also move to the host with their existing Remote sink, independent of the GUI presentation sink. Native host-only, combined GUI/CLI and original headless Clippy `--all-targets`, formatting and dependency boundaries pass. [Evidence](../testing/headless-host-store-2026-10-20.json) records the SQL digests, interim tests and limits. No tests were executed locally. This is phase 2 progress; Agent ownership, business bridge, Remote, CLI command, CI and release acceptance remain open.

### 2026-10-20 Host CI ownership

CI now reports `future-host` tests as a separate Rust matrix group. Host source and shared build/schema/Remote inputs select this owner; host changes also select CLI, GUI and native lifecycle consumers without selecting Mobile or frontend unit suites merely because Rust host code moved. Combined GUI feature dependency validation runs with the existing Desktop quality pass. Full workflow dispatch on this implementation branch is the automation path; no PR is created and no local tests run. CI results remain pending until a concrete run completes.

### 2026-10-20 Identity, instance lock and review backend

Stable installation identity, Future home resolution and the existing `app/desktop.lock` implementation move to the shared host, including default-root compatibility and GUI single-instance namespace derivation. Git/shadow review and its real filesystem/Git tests also move, keeping the user's repository read-only and retaining the existing private review directory and attribution/retention policy. Desktop keeps import adapters; now-unused parser/process facades are removed. The reusable test environment guard moves with its users; CLI discovery remains in Desktop as an entry capability.

75 original test declarations move in this batch. All original test names remain traceable; the inventory currently records 427 moved declarations in total, 9 intermediate coordination cases and 412 mixed-module cases still requiring review. Native host-only, combined GUI/CLI and original headless Clippy `--all-targets`, fmt and dependency checks pass. [Evidence](../testing/headless-host-identity-review-2026-10-20.json) also records interim CI run [39356975686](https://github.com/futuregene/future-os/actions/runs/39356975686) on `b5bd2f53`: its snapshot is pending, its host job is running, and it does not include this batch. No passing CI, runtime or final-candidate claim is made.

Current phase status:

| Phase | Actual status |
|---|---|
| 0 | Source/dependency baseline and named inventory captured; mixed-module ownership reviewed per test; matched build/runtime measurements open |
| 1 | Ordinary GUI-free crate, task registry, presentation outlet and consumer dependencies established; complete lifecycle/config boundary still open |
| 2 | Store, credentials/platform, identity/lock, review, files, Agent client/owner and request ownership moved; complete bridge, Remote, maintenance and stop coordination remain |
| 3 | Not implemented; the product entry is still `futureos-headless` |
| 4 | 566 original test declarations moved and pass locally on macOS; this candidate's cross-platform CI, real CLI process migration and remaining core coverage open |
| 5 | Host CI group and affected propagation added; builds, release artifacts, signing/install/update/uninstall still open |
| 6 | Not started; no frozen final candidate, matched performance, artifact acceptance or confirmed manual results |

### 2026-10-20 Command admission and exact-run ownership

`host/src/request_scope.rs` now owns the single command-admission counter and submitted-run ledger, including bounded caller drain, exact-run cancellation/reconciliation, positive terminal evidence and the existing admission RAII fixture. Desktop stop coordination calls these operations without access to internal mutex/state fields; reset still preserves active guards. No external Agent or unrelated run is added to cancellation.

Original RPC constructors and response classification move once to `host/src/agent_commands.rs`, still using `future-rpc` wire types and preserving the `desktop_` request ID prefix and all payload fields. Ten original pure tests move with this code. Existing actual RPC, acknowledgment/observer races and full-stop tests remain intact in Desktop pending bridge migration. [Static evidence](../testing/headless-host-request-scope-2026-10-20.json) records passing native host, combined consumers, legacy headless and boundary checks; no local tests run. Intermediate CI `39356975686` has a successful host group and a failed Agent group; failure diagnosis is open and the run is not accepted.

### 2026-10-20 Intermediate CI failure and journal repair

Interim run `39356975686` on `b5bd2f53` executed 354 host tests successfully (0 failed/ignored); one existing doc-test remained ignored. Agent executed 2437 passing tests, one failure and one ignored test. The run is not accepted. [The evidence and repair record](../testing/headless-host-ci-repair-2026-10-20.json) identifies the exact job, assertion and log digests.

The failed mixed-batch journal test exposed an acknowledgement/health race: the refusal fallback replied to durable storage failures before publishing unhealthy state, and could report a successful flush when no unacknowledged retry remained. Health now precedes the reply; storage failure determines the flush result independently of queue length. The original failed test and all assertions remain. An additional real-SQLite test parks the real writer, queues the mixed batch deterministically and verifies the empty-retry case remains failed even if the shared health report is cleared. Agent fmt and Clippy `--all-targets` pass; no local tests run and CI revalidation is required before accepting the repair.

### 2026-10-20 Mixed-module test ownership audit

All 412 remaining `review-required` declarations now have a per-test ownership decision in the inventory: 206 shared host business/fixture tests, 181 GUI/IPC/installer or named real GUI command integration tests, and 25 CLI headless entry/terminal tests. `ownershipRules` records the responsibility for each decision. Planned destinations are explicit files; their `planned` status does not claim an implementation or CI execution. Shared file/approval/run/conversation/task/review business is assigned to host services, while clipboard, asset-scope registration, OS opening, IPC validation, updater/install/relaunch and GUI event/entry integration remain in Desktop. This audit closes the unassigned-owner list, not phases 0–6.

The full documentation gate previously rejected the historical Agent database-fixture provenance README outside `docs/`. Its unchanged contents move to `docs/testing/agent-database-fixtures.md`; fixture SQL stays in place, and no gate exemption is added. Full documentation validation now runs on the normal rules.

### 2026-10-20 Shared Agent client with entry-owned channel runtime

Verified connection selection/cache, readiness, instance fencing, per-command deadlines and prompt ownership now live in `host/src/agent_client.rs`. The host receives a channel runtime handle; it creates no runtime and does not call `block_on`. The transitional Desktop/headless entry adapter retains the existing dedicated one-worker driver runtime and explicitly installs it before shared calls, preserving resource limits and the connection surviving transient caller runtimes. No second connection cache or ownership ledger is created.

One pure RPC-error mapping test moves. Existing actual-loopback, caller-runtime-drop, readiness and changed-instance tests remain at the entry until the canonical RPC fixture moves; assertions are unchanged. [Static evidence](../testing/headless-host-agent-client-2026-10-20.json) covers native host, GUI/CLI combined and legacy headless Clippy. Full documentation checks pass after the provenance relocation. New full CI [39358154196](https://github.com/futuregene/future-os/actions/runs/39358154196) was dispatched on repair commit `df56f5f2`; it predates this client batch and is not final acceptance.

### 2026-10-20 Windows adapter compilation repair

CI `39358154196` passed the Agent and host test groups, validating the journal repair on that candidate. Windows Desktop compilation failed because removal of the old `proc` facade missed the Windows-only terminal tree-kill import. The missing `NoWindow` trait caused E0432/E0599; native evidence upload then found no files because compilation had stopped. The existing trait is now exported narrowly from the host root and imported directly by the Windows terminal adapter. Process flags and termination scope are unchanged. [Evidence](../testing/headless-host-windows-repair-2026-10-20.json) records the exact failing job. Windows acceptance remains pending its next CI execution; no failed step is skipped.

### 2026-10-20 Shared files and attachment services

`host/src/files.rs` now owns canonicalization and credential access checks, attachment/image validation, thumbnail generation, ephemeral import/deletion, directory listing, relative preview resolution, bounded text reads/export and pasted image/file persistence. Original business bodies, limits, private serialized fields and `futureos-attachments` path are retained. Desktop keeps thin IPC wrappers, native clipboard, Tauri asset authorization and OS open/fallback handling. Image preview registration uses the shared validator and original stamp source/format without moving asset scope into the host.

27 original real-filesystem/image/persistence tests move to the host; nine GUI/clipboard/OS cases remain at the adapter. The real asset-scope case retains every assertion and uses an owned temporary directory for its fixture. [Static evidence](../testing/headless-host-files-2026-10-20.json) covers host-only, combined GUI/CLI, original headless, formatting and dependency boundaries. No local tests run; this batch still needs CI execution and later product acceptance.

### 2026-10-20 Native shared-host CI coverage

The host already has its own Linux test group. CI additionally runs host fmt, Clippy `--all-targets` and `cargo test -p future-host` on native macOS and Windows, with no npm installation or Desktop package build. The stable Rust-test gate includes both native results. Combined GUI/CLI dependency-closure verification also runs after Desktop compilation on every native build target, after an explicit dependency fetch. Native commands use separate steps so a later successful PowerShell command cannot hide an earlier failure. YAML syntax validation passes; new jobs and the Windows import repair await actual CI execution.

### 2026-10-20 Remote startup race repair and authorized local rerun

The headless process group in CI `39358154196` failed `a_reconnect_with_a_pre_v2_slot_forgets_it`. A due credential worker could run before the initial generation was published, see no active state and exit; supervision then reported a dead critical task before the intended broker refusal. Remote Server now gates that worker with a one-shot installation notification. Publication starts renewal; cancelled installation drops the notification alongside candidate cleanup. Original test assertions, deadlines, platform conditions and wire behavior are unchanged.

The user explicitly requested local verification after repair and before the next CI run. The original failing case passes (one test, 7.62 seconds); the complete real-process integration file passes (13 tests, zero failed or ignored, 189.97 seconds), recorded in the [repair evidence](../testing/headless-remote-start-repair-2026-10-20.json). The first sandbox attempt failed to bind loopback with EPERM; the authorized unsandboxed rerun uses isolated Future homes and temporary ports. Remote, legacy headless and combined GUI/CLI Clippy `--all-targets`, formatting, dependency boundaries and documentation checks pass. Native macOS results do not substitute for Linux/Windows CI, final CLI test migration, releases or manual acceptance. These local timings are provenance, not a matched performance comparison.

The Agent's original failed test and new real-SQLite regression were subsequently run locally: each passes one test with zero failures. Before pushing, `origin/main` at `2e282c65` (invitation retries) was merged, and the complete headless process file rerun on merge `1d32b73a`: 13 passed, zero failed or ignored, 189.16 seconds. Remote/GUI/CLI Clippy, legacy headless Clippy, formatting, Desktop type-check/ESLint and dependency boundaries pass again. Existing npm dependencies with the identical lockfile were temporarily linked for checks; links were removed afterward. The user explicitly chose CI for native Windows verification; only the macOS Rust target is installed locally, so Windows is not claimed as passed. Results and log digests are in the same repair evidence.

### 2026-10-20 Shared RPC/database fixture and connection integration tests

The original scripted FutureAgent real-loopback gRPC server and temporary SQLite fixture move once to `host/src/test_support/agent.rs`, without copying protocol, scripts or schema. Desktop injects only its existing observer/task cleanup; cleanup-before-HOME publication, database retries and close-before-remove ordering remain. The fixture driver compiles only for test/test-support. Production clients still require entry-provided runtimes, with no runtime fallback added.

Seven cached-channel/instance-fence/readiness/caller-runtime-drop tests and four fixture/real-database tests move to the host, preserving names, assertions and behavior. Host-only, combined GUI/CLI and original headless Clippy `--all-targets` plus formatting pass; see [migration evidence](../testing/headless-host-rpc-fixture-2026-10-20.json). No tests execute locally for this batch. It postdates CI candidate `5da65cce` (run `39361847974`), whose results cannot validate this batch. Agent ownership and full stop coordination still need migration.

### 2026-10-20 Agent ownership and entry-owned runtimes

Agent state, discovery/compatibility policy, scoped service lease, preparation, readiness, cancellation, bounded stop and cleanup move to `host/src/agent_host.rs`. Desktop retains only its native execution driver: construct the probe runtime, drive shared selection, then drop the probe. After the host obtains the existing Agent service permission and prepares it, the entry constructs and drives the Agent runtime. Runtime teardown retains the original one-second bound and precedes leaving the borrowed service scope, where existing Agent cleanup decides whether to release or retain its singleton lease. Two workers, 32 blocking threads, a 2 MiB stack and original start/stop deadlines remain. The production host service constructs no runtime and calls no `block_on`; internal state locks and Agent storage remain private.

Twenty-two original declarations move with real RPC, database import, FIFO blocking, process/shell exit and embedded/external Agent ownership assertions. The isolated refusal-before-start case closes shared admission directly; Desktop full-coordinator tests retain entry wiring coverage. Shared logging retains `app/logs/desktop.log` and the same mirror; CLI discovery reporting remains in the entry. Host-only, combined GUI/CLI and original headless Clippy `--all-targets` plus formatting pass; see [verification](../testing/headless-host-agent-owner-2026-10-20.json). This batch has not executed tests and postdates the current CI candidate. Compilation and cross-platform host results for the previous candidate cannot substitute for lifecycle acceptance. Complete business bridge, Remote hosting, scheduling and coordinated exit remain open.

### 2026-10-20 Windows dependency encoding repair and independent local host execution

Windows Desktop compilation passed in CI `39361847974`, resolving the missing trait. The new dependency step decoded Cargo's UTF-8 JSON with the default Windows codec and raised `UnicodeDecodeError`; native lifecycle and artifact steps were consequently not run. The script now explicitly decodes rustc/Cargo output as UTF-8, retaining every GUI dependency assertion. Real local metadata reproduces the cp1252 error; the fixed script passes actual dependency traversal and assertions with the default encoding forced to cp1252. See [encoding evidence](../testing/headless-host-boundary-encoding-2026-10-20.json). Native Windows verification remains in CI as directed by the user.

Following the requested local-before-CI validation, standalone `cargo test -p future-host` on host source `caa99a78` passes 500 tests, zero failed or ignored, in 10.31 seconds. One pre-existing ignored `record_macro` documentation example remains unchanged. No Desktop or npm environment is required. The 500 tests include 498 original migrated declarations and two new event-outlet cases, mapped individually from actual output; `migrated-local-pass` means native local macOS execution only. Thirteen real legacy entry tests also pass (7.51 seconds); Thirteen Remote process tests also pass (184.80 seconds), for 26 passed, zero failed or ignored across the two real entry files. See [local execution](../testing/headless-host-local-2026-10-20.json). This does not accept the final `future headless` CLI entry, other platforms or performance.

### 2026-10-20 Model, skills and provider services

Model catalog, skills RPC, provider/auth RPC and provider validation/view services move into the host; Desktop forwards to the same implementation. Production writes remain Agent-only. Local transaction helpers remain solely for existing real-file transaction, atomic-write and rollback tests. RPC types, serialized fields and configuration paths remain unchanged.

Sixty-eight original declarations move with these services: 13 model/skills/config RPC cases and 55 provider cases pass targeted local execution, preserving original assertions. Provider RPC cases reuse the canonical gRPC fixture. The original Remote mock catalog/snapshot responses are extracted once as shared test data and reused by both fixtures. Provider cases adopt canonical HOME-before-mock lock ordering, without a second protocol or backend. Host-only, combined GUI/CLI and original headless Clippy `--all-targets` plus formatting pass; see [evidence](../testing/headless-host-providers-2026-10-20.json). This batch postdates CI candidate `6e0ec7e7`; cross-platform execution still needs a subsequent run.
