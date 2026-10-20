# AGENTS.md

This file provides guidance to coding agents working with code in this repository.

FutureOS: one AI agent everywhere — terminal (TUI), desktop (GUI), mobile (Android/iOS), CLI, and IM bots. The core is Rust: a gRPC agent backend plus a channel bridge, goal control plane, CLI, and TUI that all connect to it. The desktop app is Tauri + React (TypeScript) and mobile is React Native (Expo). For architecture and module breakdown read `docs/README.md` (the docs index), `docs/guide/directory-layout.md` (what lives under `~/.future/`), and the code directly.

## Workspace layout

The Rust workspace (`Cargo.toml`) members and their slice of `~/.future/` (see `docs/guide/directory-layout.md`):

- `host/` — `future-host`, the GUI-free shared application backend under extraction. Owns shared Store/migrations, credentials/platform and OAuth/account services, identity/lock, files/review, Agent client/hosting, the application bridge, task execution/conversation lifetime, maintenance scheduling and request ownership. Remote host/Peer services and shared shutdown coordination/teardown now also live here. Entry points install runtimes and adapt GUI resources; CLI owns the unified `future headless` foreground entry and its runtimes; final acceptance remains in progress. See `docs/architecture/headless-package-development.zh-CN.md`.
- `agent/` — `future-agent`, the gRPC backend. Owns `~/.future/agent/` (settings, models, auth, JSONL sessions, skills).
- `channels/` — `future-channel`, the Feishu / DingTalk IM bridge. Owns `~/.future/channels/config.json`.
- `orchestration/goal/` — `future-goal`, the goal control plane (durable goals/todos/gates, deterministic should-run kernel, event-sourced state). Owns project-local `<cwd>/.future/goal/` by default (`FUTURE_GOAL_ROOT` overrides it); see `docs/architecture/goal-control-plane.md`.
- `tui/` — `future-tui`, the terminal UI (a gRPC client of the agent). Owns `~/.future/tui/`.
- `cli/` — `future-cli`, builds the unified `future` binary that embeds headless/agent/tui/channel/goal.
- `packages/rpc/` — `future-rpc`, the protobuf wire-contract crate (single source of truth; see proto notes below).

`desktop/src-tauri` (`futureos`) is a workspace member: every Rust entry point shares the root `Cargo.lock`, `target/`, and release profile (Thin LTO). `default-members` keeps GUI dependencies out of ordinary root builds; select `-p futureos` explicitly for Desktop, or `--workspace --exclude futureos` for all CLI/server crates. `packages/` also holds shared npm packages (`markdown`, `thread-projection`, `json-preview`) consumed by desktop/mobile; the repo-root `package.json` declares npm workspaces `["packages/*", "desktop", "mobile"]`, so a single `npm install` at the root hoists all deps.

## Project memory

`FUTURE.md` is a workspace memory index → `.future/memory/*.md` (institutional gotchas: testing on Linux CI vs macOS, Rust toolchain quirks, GitHub CI/PR flow, `future goal` CLI operation, llvm-cov coverage measurement). Read the relevant entries before working in those areas — they encode hard-won, non-obvious constraints not visible in the code.

## Development workflow

Development happens in an isolated git worktree at `.worktrees/<name>` on the per-developer integration branch `dev-<github username>`. `<name>` is the short slug of the worktree directory. Derive the username at runtime — `GH_USER=$(gh api user --jq .login)` — the value is never written into this file. Never work in the local main branch. The local main branch (`main`) is used by the user for local integration testing and may contain their own unrelated changes — do not treat it as a development branch.

- **One branch per developer, many tasks.** Every task commits onto `dev-<username>`, which outlives the task: never delete the remote branch after a merge, and never drop it as "cleanup".
- **Who opens the PR?** Normally the **会话总管** (session supervisor), on a ~3-hour cadence; a task session then only pushes and never creates a PR or arms auto-merge itself (*PRs and merges* below). **If that supervisor is not available** — no supervisor deployed, it is paused or not running, no PR appeared on the branch well past its cadence, or the user says to skip it — **fall back to the old path**: open the PR from `dev-<username>` yourself and take it to the merge, exactly as *Addressing a failing or stale PR* below describes (self-test → push → `gh pr create` → `gh pr merge --squash --auto` → `gh pr checks --watch`). Leaving the work unmerged is worse than a task session opening its own PR.
- **The local `dev-<username>` must always be current** (`= origin/dev-<username>`, and after a merge `= origin/main`). Reconcile it first:
  ```bash
  GH_USER=$(gh api user --jq .login)
  git fetch origin --prune
  git rev-parse dev-$GH_USER origin/dev-$GH_USER   # if both exist and differ, see below
  ```
  Fast-forward it whenever it is behind and nothing is holding it: `git branch -f dev-$GH_USER origin/dev-$GH_USER` (or `origin/main` right after a merge). A *checked-out* branch cannot be updated that way — if `git worktree list` shows it live, update it inside that worktree with `git merge --ff-only origin/dev-$GH_USER`, and never yank it out from under a running session.
  If the local branch is **ahead** of the remote, those commits are unpushed work: push them (`git push origin HEAD:dev-$GH_USER`) or report them. Never reset or force such a branch to "make it current".
- Create the worktree from the branch tip, after the fetch above:
  - branch exists on the remote: `git worktree add --no-track .worktrees/<name> -b dev-$GH_USER origin/dev-$GH_USER`
  - branch exists only locally: `git worktree add --no-track .worktrees/<name> dev-$GH_USER`
  - first task ever (no branch yet): `git worktree add --no-track .worktrees/<name> -b dev-$GH_USER origin/main`
  `--no-track` matters: without it the branch tracks `origin/main`, and a bare `git push` would target `main`.
  A branch can be checked out in only one worktree at a time. If `git worktree list` shows `dev-<username>` already live elsewhere, that session is still working: wait for it to finish, or work on a scratch `<type>/<name>` branch cut from `origin/dev-<username>` and merge it into `dev-<username>` before pushing. Do not create a second worktree on the same branch.
- All code changes, including fmt / clippy / lint fixes, are made and committed in the worktree on `dev-<username>`, with Conventional Commit messages (a squash-merged PR title uses the same type).
- Push explicitly: `git push origin HEAD:dev-$GH_USER`. The branch has no upstream (`--no-track`), so a bare `git push` fails by design; never add one. If the push is rejected, `git fetch origin`, rebase onto `origin/dev-$GH_USER` (resolve conflicts in your worktree), re-run the affected checks, push again. **Never force-push** from a task session.
- **After a verified push the worktree can be deleted** — the commits are on `origin/dev-<username>`, and removing the worktree affects neither the branch nor an open PR:
  ```bash
  git -C .worktrees/<name> status --short     # must be clean
  git -C .worktrees/<name> rev-parse HEAD     # must equal origin/dev-$GH_USER
  git ls-remote origin refs/heads/dev-$GH_USER
  git worktree remove .worktrees/<name>
  ```
  Keep it if anything is unpushed. Keep the *branch* — it is the developer's continuous branch; only a deliberately abandoned branch gets `git branch -D`. Never remove another session's worktree. Removing the worktree frees the branch so the supervisor (or the next task) can fast-forward the local copy.
- **Never commit to `main`, and never merge a worktree branch into it.** PRs are squash-merged, so a branch commit fast-forwarded into local `main` gets a *different* SHA than the squashed commit that lands on `origin/main`. That one local commit then blocks `git merge --ff-only origin/main` permanently, and the only way out is a manual `git reset --hard origin/main` — so "let the user test it" quietly becomes a divergent local `main` that every later PR has to stop and clean up. (Not hypothetical: `git reflog main` showed 12 such fast-forwards against 8 manual resets.) To let the user test a change, point them at the worktree (`cd .worktrees/<name> && make run-mobile-android`, …); to get it onto `main`, let the PR squash-merge and fast-forward from `origin/main` (see *After a PR merges*). If the user explicitly asks for the change on `main` first, do it in a scratch worktree (`git worktree add /tmp/x origin/main`) rather than committing to `main`.
- Do not merge the local main branch (`main`) into the worktree; if `main` has user changes you need, ask the user rather than merging local main in.
- **Other sessions work in this repo at the same time**, each in its own worktree and branch (`git worktree list` shows the live ones; `main` may have moved since you last looked). Stay inside your own worktree and branch: never `git worktree remove` or prune someone else's, never commit, reset, rebase, or force on their behalf, and if you find a commit on `main` you did not make, report it instead of cleaning it up.

### Before committing and pushing

Run the self-test pass in the worktree on the CI toolchain (the repo pins `rust-toolchain.toml`; `make lint-rust` uses the same clippy flags CI uses). It runs before **every** push: whoever opens the PR takes whatever is on `dev-<username>` without running tests for you.

1. `git fetch origin main` then merge `origin/main` into `dev-<username>` (resolve conflicts here). Branch protection requires the PR head to be up to date with `main`, so keep the branch current as you go.
2. **Scope checks to the modules the PR touches** - targeted beats exhaustive:
   - Rust: `cargo fmt -p <crate> --check` + `cargo clippy -p <crate> --all-targets -- -D warnings` + `cargo test -p <crate>` for each crate with code changes (run the crate's own test targets; integration tests under `tests/` are included by `cargo test -p <crate>`). Cross-crate public-API changes: also test the direct consumers of the changed API.
   - Desktop TS: `tsc --noEmit`, `eslint`, `vitest run` only when `desktop/` files changed; plus `cargo fmt --check` / `cargo clippy` under `desktop/src-tauri` only when it changed.
   - Docs / `AGENTS.md` only: `make check-docs` (if it already fails on `origin/main` for an unrelated file, say so instead of fixing it here).
   - Do NOT run workspace-wide `make test` / `make lint-rust` for a module-scoped PR - CI runs the full matrix and is the backstop. Escalate to a full local pass only when a change cuts across many crates (e.g. `packages/rpc` wire-contract changes) or after a CI failure local repro is needed.
3. Commit fmt/clippy fixes together with the work and push (`git push origin HEAD:dev-$GH_USER`). Do **not** open a PR or arm auto-merge here — that belongs to the supervisor (below), or to you only in the fallback case.

**Always sync with `origin/main` right before pushing** — not just at the start of the pass. Re-run `git fetch origin main`; if main moved while you were running checks, merge it again and re-run the checks it affects. Branch protection requires the PR head to be up to date with main, and on a fast-moving main a stale branch bounces between BEHIND and re-queued CI.

Do not skip steps or use narrower flags than CI — a green local check on a smaller scope does not guarantee CI passes (e.g. clippy without `--all-targets` misses test code). `make help` lists every target.

During normal development you don't need to run this full suite every time — iterate on targeted checks (`cargo check`, a single test, `tsc`) to save time. The full pass is mandatory before every push; without it the PR will fail.

### PRs and merges

**Default: the 会话总管 (session supervisor) owns the PR.** It checks `origin/dev-<username>` roughly every 3 hours:

- No open PR and `origin/dev-<username>` has commits `origin/main` lacks → it opens one against `main` and arms squash auto-merge (`gh pr create --base main --head dev-$GH_USER --fill`, then `gh pr merge <n> --squash --auto`). A PR never stays open without auto-merge armed.
- PR is BEHIND → it merges `origin/main` into `dev-<username>`, re-runs the checks the merge touches, and pushes, instead of letting the PR bounce between BEHIND and re-queued CI.
- Required checks fail → it opens a **new session** in this repo with the failing job/run evidence, rather than messaging the session that wrote the code. That session fixes the failure on `dev-<username>` by the flow above and pushes; the next pass re-checks. Do not merge a red PR, re-run checks indefinitely, or weaken assertions to get it green.
- Checks pass → the squash merge lands by itself; the supervisor then runs *After a PR merges*.

**Fallback: the task session does the old path.** When no supervisor is going to do it — none deployed, it is paused/stopped, no PR appeared well past its cadence, or the user says so — the session that owns the work opens the PR and takes it to the merge itself:

```bash
gh pr create --base main --head dev-$GH_USER --fill
gh pr merge <n> --squash --auto          # always arm it; never leave a bare PR
gh pr checks <n> --watch --required --fail-fast --interval 5
```

Then *Addressing a failing or stale PR* below still applies, and so does *After a PR merges*. Record in the PR description (or a comment) that the fallback path was used, so nobody later wonders why a task session opened it.

### Addressing a failing or stale PR

**If a PR opens BEHIND / "update branch" is requested, update it immediately — before anything else.** The moment `gh pr view` (or the GitHub banner) shows the branch is not up to date with main: `git fetch origin main && git merge origin/main` into `dev-<username>` (resolve conflicts here), re-run the scoped checks the merge touches, push, and let `--auto` re-fire. Do not start new work while your PR sits BEHIND — on a fast-moving main it bounces between BEHIND and re-queued CI, and every queued PR behind it waits too.

**Check PR status synchronously — never `sleep`-poll.** Wait with `gh pr checks <n> --watch --required --fail-fast` (add `--interval 5`); `--watch` blocks until the required checks finish and `--fail-fast` returns on the first failure. Do not loop `sleep N; gh pr checks`. On a red required check, read the failing job's log (`gh run view <run-id> --log-failed`) and fix the cause; opening a separate repair session is the supervisor's move (above), not a substitute for reading the log.

### After a PR merges

The supervisor does this right after the squash merge; in the fallback path the task session does it. A session that merely notices the merge only needs to bring its own local copies up to date — never re-merge or re-target anything.

1. **Update the local main branch**: `git fetch origin main`, then fast-forward it (`git merge --ff-only origin/main` from the main worktree). If the fast-forward is refused, identify what local `main` is carrying before concluding anything — do not assume it is the user's work:
   - `git log --oneline origin/main..main` lists the commits upstream does not have.
   - A **stale duplicate** is a branch commit that was later squash-merged: its tree matches the squashed commit, and that squashed commit is already an ancestor of `origin/main`. Confirm both (`git diff --stat <local-sha> <squashed-sha>` prints nothing; `git merge-base --is-ancestor <squashed-sha> origin/main` succeeds), then ask the user before dropping it with `git reset --hard origin/main`.
   - **Anything else is live work** — the user's, or another session's (`git worktree list`, and `git branch --contains <sha>` says whose). Never rebase, reset, or force it: stop and tell the user the commit, its branch, and that it has not been pushed.
2. **Re-point `dev-<username>` at the new `main`** so the next task starts from what actually landed — the branch's own commits are upstream already, under different squash SHAs. With no worktree holding it: `git branch -f dev-$GH_USER origin/main`, then `git push --force-with-lease origin dev-$GH_USER` if the remote branch still needs the reset; with a worktree holding it, `git merge --ff-only origin/main` inside that worktree instead. **Keep commits made after the PR head**: rebase them onto the new `origin/main` and push them — never drop them. This post-merge reset is the **only** sanctioned force-push in this repo; task sessions never force-push otherwise. **Do not delete the remote branch** — it is the continuous per-developer branch the next task continues on, and deleting it discards the PR review timeline.
3. **Clean up temporary files**: scratch scripts, logs, captured CI output, temp HOME dirs, and any other debris created while working. `git status --short` in every remaining worktree must show no untracked scratch files.

### Desktop builds and CLI sidecars

Use `make build-desktop` / `make package-desktop` or `npm --prefix desktop run tauri:build`. `scripts/build/build-desktop.mjs` asks Tauri to build `future-cli` and `futureos` in one Cargo invocation so shared dependencies use the same resolved features. It stages the CLI only after compilation, then invokes `tauri bundle` unless `--no-bundle` was requested. Windows signs the staged sidecar between these phases. All Rust and bundle outputs live in root `target/`.

Desktop's build script disables tauri-build's sidecar copy locally: copying into the shared target directory could overwrite Cargo's `future` executable with a stale sidecar. Plain Cargo checks do not require placeholder sidecars. Linux release builds compile the musl CLI including headless, then set `FUTURE_DESKTOP_CLI` to that artifact for the separate GNU GUI build; machine code cannot be shared across those target triples.

## Design principles

- **Don't add features, refactors, or abstractions beyond what the task requires.** A bug fix doesn't need surrounding cleanup; a one-shot operation doesn't need a helper. Don't design for hypothetical future requirements. Three similar lines beat a premature abstraction.
- **Cross-platform from the start.** Code must work on Windows, macOS, and Linux, on both x86-64 and arm64. Don't assume POSIX: paths can use `\` separators and `.exe` suffixes, filesystems are case-insensitive on some platforms, and shell runs differ (PowerShell on Windows). Never hard-code `/`, `~`, or shell-specific syntax when a platform-neutral form exists.

## Conventions and gotchas

### Build / run

- Prefer `make` targets from repo root (`make build`, `make test`, `make lint`, ...). `make help` lists them all. For more control, use cargo/npm directly. See `README.md` Quick Start for the common flows.
- The Rust binary `future-agent` is the gRPC backend, defaulting to per-user local IPC. Unix uses `FUTURE_AGENT_SOCKET` when set; otherwise the endpoint always lives under the instance's FutureOS home (`FUTURE_HOME`, i.e. `future agent --home DIR`, moves the whole `~/.future` root: lock, database, sessions, logs, and `<home>/run/agent.sock`), defaulting to `~/.future/run/agent.sock` — shared runtime directories such as `$XDG_RUNTIME_DIR` are deliberately not consulted. Windows uses a current-user-only named pipe. TCP requires explicit `--grpc-addr`; clients use `FUTURE_AGENT_GRPC_ADDR` (channels: `agent.grpc_addr`, default `auto`). TUI/Desktop can start a sidecar when no agent is reachable. `future <cmd>` is the unified entry point for every Rust component (`future agent|tui|channel|goal <args>` — each runs the same code as the standalone `future-agent` / `future-tui` / `future-channel` / `future-goal` binaries, which remain buildable (`cargo build -p <crate>`) but are no longer installed by default; `make run-*` targets also still work).
- **Testing the agent: always use a fresh port, never disturb the running agent.** When starting an agent for tests or manual gRPC debugging, use an isolated HOME/USERPROFILE and bind `--grpc-addr 127.0.0.1:<new-port>` (never an existing service's port, including the conventional `50051`) and never `kill` an existing agent. A fresh port alone does not bypass the per-user agent singleton lock. Replay sessions against an isolated FutureOS home (e.g. `future agent --home /tmp/x`, or `HOME=/tmp/x future agent`) with `--grpc-addr 127.0.0.1:PORT`; the real `:50051` agent is unaffected, and `--home` also gives the test instance its own IPC endpoint. See `.future/memory/agent-e2e-grpcurl.md`.
- Proto codegen is opt-in (`REGENERATE_PROTO=1`), checked into git, and CI fails if it goes stale. `make generate-proto` regenerates both generated files: `packages/rpc/src/generated/proto.rs` (from `future.proto` — the single source of truth for the RPC wire contract; the old per-crate copies in `agent/` and `desktop/src-tauri/` are gone, every Rust consumer of the RPC contract depends on `future-rpc`) and `channels/src/generated/feishu_ws.rs` (from `channels/proto/feishu_ws.proto` — the separate Feishu WebSocket pbbp2 frame schema, kept in `channels` only).
- Typed-RPC wire contract: `RpcResponse.payload` / `StreamEvent.payload` (field 20) carry typed `oneof` payloads for Tier-1 commands/events. **Command-response dual-write is retired**: typed commands carry the typed `payload` only (empty `data`); untyped commands keep the JSON `data` string. **Event streams still dual-write** `data` + `payload` (the `data` string is byte-stable for journal/NATS consumers and `event_data` stays data-first). The legacy casing alias machinery (`inject/strip_legacy_aliases` + `*_ALIASES` constants) is removed — canonical camelCase only. Decoding: Rust clients (`future_rpc::decode::response_data`) are typed-first with a JSON `data` fallback. The former TypeScript clients (`@future-os/rpc` in `future-rpc/ts`) were removed when the TUI/CLI were ported to Rust. Field numbers are stable / never reused; `optional` marks fields whose JSON distinguishes null/absent.

### Screenshots, videos and illustrated documents

Screenshots, feature diagrams, demo videos, variant sheets for picking a style, version/style comparison sheets, pixel-offset measurement diagrams and illustrated documents (release notes, feature walkthroughs, test point checklists) are produced from the **real** desktop/mobile UI, rendered in a local headless Chrome against demo data — no display, no running app needed. Read `docs/guide/screenshots.zh-CN.md` (English: `docs/guide/screenshots.md`) before doing this work: its "常见请求怎么做 / Recipes" section has a worked recipe for each case, plus the commands (`serve-*`, `capture-*`, `video-*`, `variants-*`, `measure-*`, `compare-*`, `terminal`, `pdf`), the scenario table, the document-assembly schema, and how to extend the mocks when a screen changes. Captures and generated figures are gitignored — never commit them.

### Config
Agent config lives under `~/.future/agent/` (`settings.json`, `models.json`, `auth.json`, `sessions/`). Model config reads purely from these files — no model-related CLI flags or env vars. Channel config is under `~/.future/channels/config.json`, auto-created with defaults on first run: framework channels use a `providers.<id>` block (each channel reads its own; the access-policy keys `dm_policy`/`group_policy`/`require_mention` are read by the bridge), while Feishu and DingTalk also accept their legacy top-level block — `providers.<id>` wins when both exist. The TUI persists client-side settings to `~/.future/tui/settings.json`.

API key resolution order: `auth.json` (by model ID) → `auth.json` (by provider) → model built-in key → `auth.json` default key.

### Desktop (`desktop/`)

See `desktop/CLAUDE.md` for the desktop development guide. The desktop app owns `~/.future/app/` (SQLite `app.db`, images, review repos) and per-thread chat workspaces under `~/.future/workspaces/chat/`.

### Channels (`channels/`)

Two kinds of channel live here, and the distinction decides where a change
belongs:

- **Framework channels** (`channels/src/providers/<id>.rs`) implement
  `providers::traits::Provider` + `ChannelSender` and get duplicate filtering,
  access policy, session mapping, per-conversation queueing, streaming replies
  and chat-based approvals from `channels/src/bridge/`. A provider holds
  platform knowledge only: parse events into `bridge::Inbound`, hand them to
  `ProviderCtx::handle`, send/optionally edit text. Chunking, throttling and
  retries are the bridge's, not the provider's. The contract is
  `docs/guide/channels-provider-contract.md`; `providers/cli.rs` is the reference
  implementation. New channels are registered in `providers/registry.rs`; a
  `Maturity::Planned` channel refuses to start and reports `unsupported`.
- **Self-bridged channels** (`channels/src/feishu/`, `channels/src/dingtalk/`)
  keep their own bridges, which is where platform behaviour the framework does
  not model yet lives (interactive cards, streaming card elements, approval
  buttons, slash commands). They are declared in `providers/native.rs` so the
  CLI and docs describe them too.

Shared pieces worth knowing before adding code: `channels/src/policy.rs` (the
dm/group access policy used by every channel), `channels/src/session_store.rs`
(conversation → agent session), `channels/src/delivery.rs` + `outbox.rs` (the
durable outbound queue and its drainer), `channels/src/transport/` (retrying
HTTP, reconnecting websocket, webhook signatures, an inbound webhook server,
text splitting), `channels/src/status.rs` (the snapshot `future channel status`
reads). `future channel list|status|test|send` are handled by
`channels/src/cli_cmd.rs` **before** the bridge starts, so diagnostics work with
no bridge running.

- **Feishu API base URLs:** `api_base()` = `https://open.feishu.cn/open-apis` (REST), `api_domain()` = `https://open.feishu.cn` (WS bootstrap). Do NOT append `/open-apis` again.
- **CardKit streaming lifecycle:** Create card → stream element updates at 250ms throttle → finalize: FIRST `set_card_streaming_mode(false)`, THEN `update_cardkit_card` with complete card. Order matters (settings first clears the "[生成中...]" status).
- **CardKit gotchas:** `update_multi` must stay `true` (cannot change to `false`, returns 300302). Settings API returns empty body on success (use HTTP status, not `.json()`).
- **WebSocket:** pbbp2 protobuf binary frames. Events filtered by `create_time` — messages older than 60s are skipped (stale reconnect replays). Dedup via in-memory `HashSet` of processed message IDs.
- **DingTalk Stream Mode:** Subscribe to `{"type": "CALLBACK", "topic": "/v1.0/im/bot/messages/get"}` — NOT `{"type": "EVENT", "topic": "*"}` (prevents CALLBACK delivery). ACK format `{"code":200, "headers":{"messageId":"...","contentType":"application/json"}, "message":"", "data":"..."}`. The `data` field in CALLBACK frames is a JSON string (parse it first). Reply by POSTing markdown to the `sessionWebhook` URL from the event. Webhook replies create NEW messages each time — no in-place editing.
