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

Development happens in an isolated git worktree (this repo uses `.claude/worktrees/<name>` on a `claude/*` branch), never in the local main branch. The local main branch (e.g. `dev`) is used by the user for local integration testing and may contain their own unrelated changes — do not treat it as a development branch.

- All code changes, including fmt / clippy / lint fixes, are made and committed in the worktree branch.
- To let the user test locally, merge the worktree branch into the local main branch (`dev`) — never edit code directly on `dev` and then merge it back into the worktree.
- Do not merge the local main branch (`dev`) into the worktree; if `dev` has user changes you need, ask the user rather than merging local main in.

### Before opening a PR

Run the full pre-PR pass in the worktree on the CI toolchain (the repo pins `rust-toolchain.toml`; `make lint-rust` uses the same clippy flags CI uses):

1. `git fetch origin main` then merge `origin/main` into the worktree branch (resolve conflicts here).
2. `make lint-rust` — CI's Rust fmt + clippy (workspace + `desktop/src-tauri`, `--all-targets` included).
3. Desktop: `tsc --noEmit`, `eslint "src/**/*.{ts,tsx}"`, `vitest run`; plus `cargo fmt --check` / `cargo clippy` under `desktop/src-tauri`.
4. `make test` — all unit suites (Rust crates + desktop + mobile).
5. Commit any fmt/clippy fixes, push, then create the PR.

Do not skip steps or use narrower flags than CI — a green local check on a smaller scope does not guarantee CI passes (e.g. clippy without `--all-targets` misses test code). `make help` lists every target.

During normal development you don't need to run this full suite every time — iterate on targeted checks (`cargo check`, a single test, `tsc`) to save time. The full pass is only mandatory right before a PR; without it the PR cannot merge.

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
- Proto codegen is owned by the `future-rpc` crate (single source of truth): `make generate-proto` regenerates `packages/rpc/src/generated/proto.rs` (server + all clients). It is opt-in (`REGENERATE_PROTO=1`), checked into git, and CI fails if it goes stale. The old per-crate generated copies (`agent/`, `channels/`, `desktop/src-tauri/`) are gone — every Rust consumer depends on `future-rpc`.
- Typed-RPC wire contract: `RpcResponse.payload` / `StreamEvent.payload` (field 20) carry typed `oneof` payloads for Tier-1 commands/events. The agent **dual-writes** the typed `payload` and the legacy JSON `data` string during the migration window. Decoding: Rust clients (`future_rpc::decode`) are typed-first with a JSON `data` fallback. The former TypeScript clients (`@future-os/rpc` in `future-rpc/ts`) were removed when the TUI/CLI were ported to Rust. Do not retire the `data` dual-write until every released client reads the typed payload. Field numbers are stable / never reused; `optional` marks fields whose JSON distinguishes null/absent.

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
