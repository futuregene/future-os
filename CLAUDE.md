# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

FutureOS: one AI agent everywhere — terminal (TUI), desktop (GUI), mobile (Android/iOS), CLI, and IM bots. The core is Rust: a gRPC agent backend plus a channel bridge, loop control plane, CLI, and TUI that all connect to it. The desktop app is Tauri + React (TypeScript) and mobile is React Native (Expo). For architecture and module breakdown read `docs/README.md` (the docs index), `docs/directory-layout.md` (what lives under `~/.future/`), and the code directly.

## Workspace layout

The Rust workspace (`Cargo.toml`) members and their slice of `~/.future/` (see `docs/directory-layout.md`):

- `agent/` — `future-agent`, the gRPC backend. Owns `~/.future/agent/` (settings, models, auth, JSONL sessions, skills).
- `channels/` — `future-channel`, the Feishu / DingTalk IM bridge. Owns `~/.future/channels/config.json`.
- `orchestration/loop/` — `future-loop`, the loop control plane (durable goals/todos/gates, deterministic should-run kernel, event-sourced state). Owns `~/.future/loop/`; see `docs/loop-control-plane.md`.
- `tui/` — `future-tui`, the terminal UI (a gRPC client of the agent). Owns `~/.future/tui/`.
- `cli/` — `future-cli`, builds the unified `future` binary that embeds agent/tui/channel/loop.
- `packages/rpc/` — `future-rpc`, the protobuf wire-contract crate (single source of truth; see proto notes below).

`desktop/src-tauri` is deliberately **not** a workspace member (excluded): it builds on its own schedule via npm/tauri, and membership would pull it into every root cargo invocation. `packages/` also holds shared npm packages (`markdown`, `thread-projection`, `json-preview`) consumed by desktop/mobile; the repo-root `package.json` declares npm workspaces `["packages/*", "desktop", "mobile"]`, so a single `npm install` at the root hoists all deps.

## Project memory

`FUTURE.md` is a workspace memory index → `.future/memory/*.md` (institutional gotchas: testing on Linux CI vs macOS, Rust toolchain quirks, GitHub CI/PR flow, `future loop` CLI operation, llvm-cov coverage measurement). Read the relevant entries before working in those areas — they encode hard-won, non-obvious constraints not visible in the code.

## Development workflow

Development happens in an isolated git worktree (this repo uses `.claude/worktrees/<name>` on a `claude/*` branch), never in the local main branch. The local main branch (`main`) is used by the user for local integration testing and may contain their own unrelated changes — do not treat it as a development branch.

- All code changes, including fmt / clippy / lint fixes, are made and committed in the worktree branch.
- To let the user test locally, merge the worktree branch into the local main branch (`main`) — never edit code directly on `main` and then merge it back into the worktree.
- Do not merge the local main branch (`main`) into the worktree; if `main` has user changes you need, ask the user rather than merging local main in.

### Before opening a PR

Run the pre-PR pass in the worktree on the CI toolchain (the repo pins `rust-toolchain.toml`; `make lint-rust` uses the same clippy flags CI uses):

1. `git fetch origin main` then merge `origin/main` into the worktree branch (resolve conflicts here).
2. **Scope checks to the modules the PR touches** - targeted beats exhaustive:
   - Rust: `cargo fmt -p <crate> --check` + `cargo clippy -p <crate> --all-targets -- -D warnings` + `cargo test -p <crate>` for each crate with code changes (run the crate's own test targets; integration tests under `tests/` are included by `cargo test -p <crate>`). Cross-crate public-API changes: also test the direct consumers of the changed API.
   - Desktop TS: `tsc --noEmit`, `eslint`, `vitest run` only when `desktop/` files changed; plus `cargo fmt --check` / `cargo clippy` under `desktop/src-tauri` only when it changed.
   - Do NOT run workspace-wide `make test` / `make lint-rust` for a module-scoped PR - CI runs the full matrix and is the backstop. Escalate to a full local pass only when a change cuts across many crates (e.g. `packages/rpc` wire-contract changes) or after a CI failure local repro is needed.
3. Commit any fmt/clippy fixes, push, then create the PR.

**Always sync with `origin/main` right before pushing** — not just at the start of the pass. Re-run `git fetch origin main`; if main moved while you were running checks, merge it again and re-run the checks it affects. Branch protection requires the head branch to be up to date with main, and on a fast-moving main a stale branch bounces between BEHIND and re-queued CI (use `gh pr merge --squash --auto` so the merge fires as soon as checks go green).

Do not skip steps or use narrower flags than CI — a green local check on a smaller scope does not guarantee CI passes (e.g. clippy without `--all-targets` misses test code). `make help` lists every target.

During normal development you don't need to run this full suite every time — iterate on targeted checks (`cargo check`, a single test, `tsc`) to save time. The full pass is only mandatory right before a PR; without it the PR cannot merge.

### After a PR merges

Leave no leftovers — the next session must not inherit a stale worktree, branch, or scratch file:

1. **Update the local main branch**: `git fetch origin main`, then fast-forward it (`git merge --ff-only origin/main` from the main worktree). If the fast-forward is refused, the user has local commits there — stop and tell them; never rebase, reset, or force their branch.
2. **Delete the merged branch everywhere**: remove its worktree (`git worktree remove .claude/worktrees/<name>` — confirm `git -C <path> status --short` is clean first; investigate before reaching for `--force`), then `git branch -d claude/<name>` and drop the remote branch (`gh pr merge --delete-branch` already does this; otherwise `git push origin --delete claude/<name>`). Finish with `git worktree prune` and `git fetch --prune`.
3. **Clean up temporary files**: scratch scripts, logs, captured CI output, temp HOME dirs, and any other debris created while working. `git status --short` in every remaining worktree must show no untracked scratch files.

### GUI Tauri sidecar binaries in a worktree

`desktop/src-tauri/tauri.conf.json` declares `externalBin: ["binaries/future"]`, and `tauri-build`'s build script **aborts** with `resource path ... doesn't exist` if those files are missing. They are build artifacts — present in the main worktree but absent from a fresh worktree, so `cargo check`/`clippy`/`test` under `desktop/src-tauri` fails for environmental reasons, not your code.

CI works around this with **empty placeholder sidecars** (`.github/workflows/ci.yml`). Do the same before running GUI Rust checks in a worktree: `make desktop-sidecar-placeholder` creates the empty `future-$triple` file (gitignored); `make setup` bootstraps a fresh clone entirely (JS deps + skills submodule + placeholder).

## Design principles

- **Don't add features, refactors, or abstractions beyond what the task requires.** A bug fix doesn't need surrounding cleanup; a one-shot operation doesn't need a helper. Don't design for hypothetical future requirements. Three similar lines beat a premature abstraction.
- **Cross-platform from the start.** Code must work on Windows, macOS, and Linux, on both x86-64 and arm64. Don't assume POSIX: paths can use `\` separators and `.exe` suffixes, filesystems are case-insensitive on some platforms, and shell runs differ (PowerShell on Windows). Never hard-code `/`, `~`, or shell-specific syntax when a platform-neutral form exists.

## Conventions and gotchas

### Build / run

- Prefer `make` targets from repo root (`make build`, `make test`, `make lint`, ...). `make help` lists them all. For more control, use cargo/npm directly. See `README.md` Quick Start for the common flows.
- The Rust binary `future-agent` is the backend, always a gRPC server at `127.0.0.1:50051`. The TUI, GUI Tauri backend, and channel bridge all connect to it via gRPC. `future <cmd>` is the unified entry point for every Rust component (`future agent|tui|channel|loop <args>` — each runs the same code as the standalone `future-agent` / `future-tui` / `future-channel` / `future-loop` binaries, which remain buildable (`cargo build -p <crate>`) but are no longer installed by default; `make run-*` targets also still work).
- **Testing the agent: always use a fresh port, never disturb the running agent.** When starting an agent for tests or manual gRPC debugging, bind `--grpc-addr 127.0.0.1:<new-port>` (never the default `50051`) and never `kill` an existing agent. Replay sessions against an isolated `HOME` (e.g. `HOME=/tmp/x future agent --grpc-addr 127.0.0.1:PORT`); the real `:50051` agent is unaffected. See `.future/memory/agent-e2e-grpcurl.md`.
- Proto codegen is opt-in (`REGENERATE_PROTO=1`), checked into git, and CI fails if it goes stale. `make generate-proto` regenerates both generated files: `packages/rpc/src/generated/proto.rs` (from `future.proto` — the single source of truth for the RPC wire contract; the old per-crate copies in `agent/` and `desktop/src-tauri/` are gone, every Rust consumer of the RPC contract depends on `future-rpc`) and `channels/src/generated/feishu_ws.rs` (from `channels/proto/feishu_ws.proto` — the separate Feishu WebSocket pbbp2 frame schema, kept in `channels` only).
- Typed-RPC wire contract: `RpcResponse.payload` / `StreamEvent.payload` (field 20) carry typed `oneof` payloads for Tier-1 commands/events. **Command-response dual-write is retired**: typed commands carry the typed `payload` only (empty `data`); untyped commands keep the JSON `data` string. **Event streams still dual-write** `data` + `payload` (the `data` string is byte-stable for journal/NATS consumers and `event_data` stays data-first). The legacy casing alias machinery (`inject/strip_legacy_aliases` + `*_ALIASES` constants) is removed — canonical camelCase only. Decoding: Rust clients (`future_rpc::decode::response_data`) are typed-first with a JSON `data` fallback. The former TypeScript clients (`@future-os/rpc` in `future-rpc/ts`) were removed when the TUI/CLI were ported to Rust. Field numbers are stable / never reused; `optional` marks fields whose JSON distinguishes null/absent.

### Config

Agent config lives under `~/.future/agent/` (`settings.json`, `models.json`, `auth.json`, `sessions/`). Model config reads purely from these files — no model-related CLI flags or env vars. Channel config is under `~/.future/channels/config.json`, auto-created with defaults on first run. The TUI persists client-side settings to `~/.future/tui/settings.json`.

API key resolution order: `auth.json` (by model ID) → `auth.json` (by provider) → model built-in key → `auth.json` default key.

### Desktop (`desktop/`)

See `desktop/CLAUDE.md` for the desktop development guide. The desktop app owns `~/.future/app/` (SQLite `app.db`, images, review repos) and per-thread chat workspaces under `~/.future/workspaces/chat/`.

### Channels (`channels/`)

- **Feishu API base URLs:** `api_base()` = `https://open.feishu.cn/open-apis` (REST), `api_domain()` = `https://open.feishu.cn` (WS bootstrap). Do NOT append `/open-apis` again.
- **CardKit streaming lifecycle:** Create card → stream element updates at 250ms throttle → finalize: FIRST `set_card_streaming_mode(false)`, THEN `update_cardkit_card` with complete card. Order matters (settings first clears the "[生成中...]" status).
- **CardKit gotchas:** `update_multi` must stay `true` (cannot change to `false`, returns 300302). Settings API returns empty body on success (use HTTP status, not `.json()`).
- **WebSocket:** pbbp2 protobuf binary frames. Events filtered by `create_time` — messages older than 60s are skipped (stale reconnect replays). Dedup via in-memory `HashSet` of processed message IDs.
- **DingTalk Stream Mode:** Subscribe to `{"type": "CALLBACK", "topic": "/v1.0/im/bot/messages/get"}` — NOT `{"type": "EVENT", "topic": "*"}` (prevents CALLBACK delivery). ACK format `{"code":200, "headers":{"messageId":"...","contentType":"application/json"}, "message":"", "data":"..."}`. The `data` field in CALLBACK frames is a JSON string (parse it first). Reply by POSTing markdown to the `sessionWebhook` URL from the event. Webhook replies create NEW messages each time — no in-place editing.
