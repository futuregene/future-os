# Continuous integration

[中文](ci.zh-CN.md)

## Regular checks

[`ci.yml`](../../.github/workflows/ci.yml) runs on pull requests, merge groups,
and pushes to main. A manual dispatch runs every regular project. Main pushes
select only their affected projects, including deletions and rename sources.

| Job | Runner | Work |
|---|---|---|
| Rust Linux / Windows / macOS test | Corresponding native OS | Build and tests for selected Rust crates |
| Rust check | Linux | fmt and all-target Clippy, warnings denied |
| TypeScript test | Linux | Typecheck, build and tests for selected npm projects |
| TypeScript check | Linux | ESLint and applicable Stylelint |
| Android test | Linux | Debug app build, JVM module tests and staging IO tests |
| iOS test | macOS | Unsigned Simulator app build and Foundation ShareInbox tests |
| CI result | Linux | Require success for every job selected by the plan |

Build and tests share a runner and Cargo target directory. Source-only TS
libraries verify JS/declaration emission into a temporary directory. Mobile JS
builds both Android and iOS bundles on Linux; Jest runs once. Native jobs test
existing JVM/Swift code without signing or publishing. The iOS Foundation tests
do not claim Simulator UI/E2E coverage.

Rust test invocations use `--no-fail-fast`: a failed target still fails its job,
but Cargo continues through the other selected test targets so one run exposes
all their failures instead of revealing them one target at a time.

## Selection and dependency graph

[`affected.mjs`](../../scripts/ci/affected.mjs) reads Cargo workspace metadata
(including path dependencies used by build/dev/target configurations) and npm
workspace manifests. Source changes select the owner and transitive consumers.
Test-only inputs select the owner. A mixed Rust/TS directory is selected by input
language; shared schema/fixture inputs can select both languages. Interoperability
projects are explicitly excluded from regular execution.

In the diagrams, arrows point from a **dependency to its consumer**: changes
propagate along the arrows. Repeated edges that can be inferred through an
intermediate project are omitted for readability. The tables list every direct
workspace dependency, including those omitted edges. Rust names below omit
`future-`; `futureos` is the Desktop Rust crate. The manifests remain authoritative;
CI prints the current execution plan and selection reasons in its summary.

```mermaid
flowchart TD
    rpc["rpc"] --> agent["agent"]
    rpc --> tasks["tasks"]
    rpc --> channel["channel"]
    rpc --> goal["goal"]
    rpc --> tui["tui"]

    settings["app-settings"] --> workspaces["app-workspaces"]
    settings --> goal

    crypto["remote-crypto"] --> protocol["remote-protocol"]
    protocol --> client["remote-client"]
    protocol --> server["remote-server"]

    agent --> host["host"]
    tasks --> host
    workspaces --> host
    client --> host
    server --> host

    host --> cli["cli"]
    host --> desktop["futureos"]
    channel --> cli
    goal --> cli
    tui --> cli
```

| Rust crate | Direct workspace dependencies |
|---|---|
| host | agent, app-settings, app-workspaces, remote-client, remote-crypto, remote-protocol, remote-server, rpc, tasks |
| agent | rpc |
| rpc | — |
| app-settings | — |
| app-workspaces | app-settings |
| remote-client | remote-crypto, remote-protocol |
| remote-crypto | — |
| remote-protocol | remote-crypto |
| remote-server | remote-crypto, remote-protocol |
| tasks | rpc |
| channel | rpc |
| goal | app-settings, rpc |
| tui | rpc |
| cli | agent, app-settings, app-workspaces, channel, goal, host, rpc, tasks, tui, remote-crypto (dev) |
| futureos | agent, app-settings, app-workspaces, host, remote-client, remote-crypto, remote-protocol, remote-server, rpc, tasks |

TS names omit their npm scopes (`@future-os/`, Desktop uses `@futureos/`).

```mermaid
flowchart TD
    crypto["remote-crypto"] --> protocol["remote-protocol"]
    projection["thread-projection"] --> protocol
    protocol --> client["remote-client"]
    client --> mobile["mobile"]

    projection --> desktop["desktop"]
    markdown["markdown"] --> desktop
    markdown --> mobile
    json["json-preview"] --> desktop
    json --> mobile
```

| TypeScript project | Direct workspace dependencies |
|---|---|
| json-preview | — |
| markdown | — |
| thread-projection | — |
| remote-crypto | — |
| remote-protocol | remote-crypto, thread-projection |
| remote-client | remote-crypto, remote-protocol, thread-projection |
| desktop | json-preview, markdown, thread-projection |
| mobile | json-preview, markdown, thread-projection, remote-crypto, remote-protocol, remote-client |

Mobile also consumes the local `future-share-intent` and `future-file-handler`
modules. Their Android and iOS/extension/test trees select only the corresponding
native job. Shared module interfaces, app configuration and Mobile source inputs
conservatively select both native builds. Mobile test-only edits do not rebuild
native apps. Root npm manifest/lock changes select all TS/native projects; root
Cargo manifest/lock/toolchain/config changes select all regular Rust crates.
CI planner/runner edits select every regular project. Invalid planning fails the
gate, rather than silently skipping checks.

Examples: `json-preview/src/index.ts` selects json-preview, Desktop and Mobile
TS checks plus both native apps. A Rust remote-client source change selects
remote-client, Host, CLI and Desktop Rust. A schema input in remote-protocol
selects the affected projects of both languages, while schema freshness and
interop remain manual Misc suites.

## Shared Rust runner groups

[`rust-ci.yml`](../../.github/workflows/rust-ci.yml) expands these groups on each
native OS for build/test and on Linux for fmt/Clippy. A group contains only its
selected crates; absent groups allocate no runner. TS projects each get a Linux
build/test runner and a separate Linux lint runner.

[`typescript-ci.yml`](../../.github/workflows/typescript-ci.yml) owns the TS package
matrix and execution steps. The main workflow calls it through `TypeScript test`
and `TypeScript check`, matching Rust's reusable-workflow structure. Both languages
therefore appear as collapsible parent groups in the Actions sidebar, with their
crate groups or packages nested below them.

| Group | Possible members |
|---|---|
| libraries | rpc, app-settings, app-workspaces, channel, goal, tasks |
| remote | remote-crypto, remote-protocol, remote-client, remote-server |
| agent | agent |
| host | host |
| tui-cli | tui, cli |
| desktop | futureos |

The Desktop group also verifies GUI-disabled and combined CLI/GUI builds.
Remote client/server Clippy covers `test-support`. Linux Host tests use an
isolated broker. Other native platforms run the Host crate's own suite.
Core groups share one cache per OS; the main CLI test job is its sole writer.
Desktop has a separate GUI cache written by main Desktop test jobs. PR/check/Misc
jobs do not write large private Rust cache copies.

## Specialty validation

[`misc.yml`](../../.github/workflows/misc.yml) has **only** a manual trigger.
It is outside PR/main gating. It replaces the old `ci.yml` specialty scopes and
the standalone flaky-stress workflow. Regular native checks absorb the former
mobile-native-io workflow.

```bash
gh workflow run ci.yml --ref <branch>
gh workflow run misc.yml --ref <branch> -f suite=generated
gh workflow run misc.yml --ref <branch> -f suite=remote-interop
gh workflow run misc.yml --ref <branch> -f suite=windows-update
```

| `suite` | Validation |
|---|---|
| generated | Proto regeneration and Remote schema/codegen freshness |
| remote-interop | Rust/TS interoperability and Remote boundaries |
| windows-update | Offline installer/update fixtures |
| macos-notarization | Recovery/publication script regressions on Linux |
| flaky-stress | Repeated Agent, TUI and CLI tests |
| agent-lifecycle | Native three-OS lifecycle, Host/CLI boundaries and Host docs |
| remote-compile-measurement | Matched cold Windows compilation |
| headless-measurement | Matched Headless build/startup observation; optional `measurement_candidate` SHA |
| full-regression | All regular CI projects, without specialty suites |

Release signing, notarization, packaging and uploads remain in release workflows.

## Merge gate migration

Use the stable `CI result` check as the required status. It runs with `always()`
and rejects failed planning, failures, cancellations, missing results and
unexpected skips. Unaffected suites are allowed to skip. Dynamic matrix names
are diagnostic, not required-check names.

When landing this refactor, migrate the Protect Main ruleset's old individual
required contexts to `CI result` (GitHub Actions app id 15368), preserving strict
up-to-date enforcement and all other rules. Until that administrative migration,
the old contexts will remain pending even when the new checks pass. No ruleset
is changed by editing these workflow files.

Local validation uses syntax/static checks. Unit tests, including planner/gate
regressions in `scripts/tests/ci-plan.test.mjs`, run in CI.
