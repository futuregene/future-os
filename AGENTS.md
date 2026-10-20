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

Development happens in an isolated git worktree at `.worktrees/<name>` on its **own task branch** `<type>/<name>`, cut from the per-developer integration branch `dev-<github username>`. `<name>` is the short slug of the worktree directory; `<type>` is the Conventional Commit type the work will carry (`fix/`, `feat/`, `docs/`, `refactor/`, …). Derive the username at runtime — `GH_USER=$(gh api user --jq .login)` — the value is never written into this file. The integration branch `dev-<username>` is checked out in the **main worktree only**, where the user parks it (*The main worktree holds `dev-<username>`* below); a task worktree never checks it out. A task's branch is merged into `dev-<username>` as soon as the task is done (*Finishing a task* below) — finished work must never be left sitting on an unmerged task branch. Never work in the local main branch: it is the user's checkout for local integration testing and may contain their own unrelated changes.

- **One branch per developer, many tasks.** Each task works on its own `<type>/<name>` branch and merges it into `dev-<username>`; `dev-<username>` outlives every task and its worktree: never delete the remote branch after a merge, and never drop it as "cleanup".
- **`dev-<username>` carries its owner's work and nothing else.** Never merge another author's branch, PR or commits into it — not to "help it land", not because it touches the same files, not even when the merge is clean. Their work belongs on their own branch and lands through its own PR, so pulling it here takes their review, their PR timeline and their authorship onto someone else's branch. Measured 2026-10-20: two commits by another author rode into `dev-<username>` that way (five open branches folded in at once), and getting them out again meant rewriting the branch and restoring theirs. If a branch looks ready, say so and let its owner (or the sweep on their behalf) take it through `main`; do not fold it in.
- **The main worktree holds `dev-<username>` — leave it there.** The user keeps that checkout on their dev branch so they always see real work; `main` stays current in the background instead of being visited. So: never `git checkout`/`switch` that worktree, never `git branch -f` the branch it holds, and never update `main` by checking it out — use `git fetch origin main:main`, which writes the ref without touching the checkout (*After a PR merges* below has the rest). The one write allowed there is *Finishing a task*: fast-forward the local branch to `origin/dev-<username>`, merge a finished task branch, push. If that worktree is dirty (the user is editing), stop and say so instead of stashing, resetting or merging around their changes.
- **Who opens the PR? Not the session.** A **scheduled task named `dev 分支 PR 合并`** in this repo, running hourly on whichever machine runs it (`future task list --json` finds it locally; `future task show <id> --prompt` is its current instruction), owns the PR lifecycle for `dev-<username>` (*PRs and merges* below). A task session's last step is **push `dev-<username>` and stop**: never `gh pr create`, never `gh pr merge --auto`. Not finding that task on this machine changes nothing — another machine submits the PR, and a branch that waits for the next pass is fine.
- **The local `dev-<username>` should track `origin/dev-<username>`** (and `origin/main` right after a merge). Fetch and reconcile before cutting a task branch from it:
  ```bash
  GH_USER=$(gh api user --jq .login)
  git fetch origin --prune
  git rev-parse dev-$GH_USER origin/dev-$GH_USER   # if both exist and differ, see below
  ```
  Fast-forward it whenever it is behind and nothing is holding it: `git branch -f dev-$GH_USER origin/dev-$GH_USER` (or `origin/main` right after a merge). A *checked-out* branch cannot be updated that way — the main worktree normally holds it, so update it there with `git merge --ff-only origin/dev-$GH_USER`, and never yank it out from under a running session.
  If the local branch is **ahead** of the remote, those commits are unpushed work — usually a finished task merged but not pushed yet: push them from the main worktree, or report them. Never reset or force such a branch to "make it current", with one exception: **a branch whose content is already on `main`** (`git diff --quiet origin/main origin/dev-$GH_USER` succeeds) is a squash merge's leftover, not work — collapse it onto `origin/main` before cutting the task branch (*Collapse a stranded branch* below). Cutting from a stranded branch is what hands the next PR a commit list full of landed commits.
- Create the worktree on a **fresh task branch** cut from the integration branch tip, after the fetch above:
  ```bash
  git worktree add --no-track .worktrees/<name> -b <type>/<name> origin/dev-$GH_USER
  ```
  First task ever (no `dev-<username>` anywhere): cut the task branch from `origin/main` the same way and create the integration branch in the main worktree (`git branch dev-$GH_USER origin/main`).
  `--no-track` matters: without it the branch tracks `origin/main`, and a bare `git push` would target `main`.
  **Never check out `dev-<username>` in a task worktree.** A branch lives in one worktree at a time, so doing that locks the integration branch for as long as the worktree exists: the next task cannot create its worktree, the user's own `git checkout dev-<username>` in the main worktree fails, and the post-merge fast-forward has to wait. That is exactly why the task branch exists — cutting and merging one costs nothing. (Measured 2026-10-20: `.worktrees/standby-rows` had taken `dev-huichen`, and the user's `git checkout dev-huichen` in the main worktree died with "already used by worktree".) If `git worktree list` shows `dev-<username>` held by a worktree, that is a session running the old flow: leave it alone, create your own worktree on a task branch, and merge when theirs is finished.
- All code changes, including fmt / clippy / lint fixes, are made and committed in the worktree on its task branch `<type>/<name>`, with Conventional Commit messages (a squash-merged PR title uses the same type).
- **Task branches stay local**: do not push them and do not open a PR from one. The only branch this developer pushes is `dev-<username>`, once, at the end of the task (*Finishing a task* below); the PR on top of it belongs to the scheduled task, never to a task session. `--no-track` keeps a bare `git push` from ever targeting `main`; never add an upstream. **Never force-push** from a task session.
- **Remove the worktree only after its work is merged** (*Finishing a task* below) — the task branch lives nowhere else until then, so deleting the worktree earlier throws the work away:
  ```bash
  git -C .worktrees/<name> status --short                              # must be clean
  git -C .worktrees/<name> merge-base --is-ancestor HEAD dev-$GH_USER  # must succeed
  git worktree remove .worktrees/<name>
  git branch -d <type>/<name>
  ```
  Keep the *integration branch* `dev-<username>` — it is the developer's continuous branch; only a deliberately abandoned branch gets `git branch -D`. Never remove another session's worktree. Removing the worktree frees `dev-<username>`'s checkouts so the scheduled task (or the next task) can fast-forward the local copy.
- **Never commit to `main`, and never merge a worktree branch into it.** PRs are squash-merged, so a branch commit fast-forwarded into local `main` gets a *different* SHA than the squashed commit that lands on `origin/main`. That one local commit then blocks `git merge --ff-only origin/main` permanently, and the only way out is a manual `git reset --hard origin/main` — so "let the user test it" quietly becomes a divergent local `main` that every later PR has to stop and clean up. (Not hypothetical: `git reflog main` showed 12 such fast-forwards against 8 manual resets.) To let the user test a change, point them at the worktree (`cd .worktrees/<name> && make run-mobile-android`, …); to get it onto `main`, let the PR squash-merge and fast-forward from `origin/main` (see *After a PR merges*). If the user explicitly asks for the change on `main` first, do it in a scratch worktree (`git worktree add /tmp/x origin/main`) rather than committing to `main`.
- Do not merge the local main branch (`main`) into the worktree; if `main` has user changes you need, ask the user rather than merging local main in.
- **Other sessions work in this repo at the same time**, each in its own worktree and task branch (`git worktree list` shows the live ones; `main` and `origin/dev-<username>` may have moved since you last looked). Stay inside your own worktree and branch: never `git worktree remove` or prune someone else's, never commit, reset, rebase, or force on their behalf, and if you find a commit on `main` you did not make, report it instead of cleaning it up. The merge into `dev-<username>` is the one place tasks serialize: fetch it first, merge onto whatever it is now, and resolve conflicts in the main worktree without rewriting the commits that already landed.

### Before committing and pushing

Run the self-test pass in the worktree on the CI toolchain (the repo pins `rust-toolchain.toml`; `make lint-rust` uses the same clippy flags CI uses). It runs before the task branch is **merged into `dev-<username>` and pushed**: whoever opens the PR takes whatever is on `dev-<username>` without running tests for you.

1. `git fetch origin --prune`, then look at `origin/dev-<username>` before touching anything. `git log --oneline HEAD..origin/dev-$GH_USER` lists what you are missing — if it prints anything, `git rebase origin/dev-$GH_USER` first (resolve conflicts here, re-run the checks those commits touch, since they may have changed the same files), and only then `git merge origin/main` (resolve conflicts here). Branch protection requires the PR head to be up to date with `main`, so keep the branch current as you go.
2. **Scope checks to the modules the PR touches** - targeted beats exhaustive:
   - Rust: `cargo fmt -p <crate> --check` + `cargo clippy -p <crate> --all-targets -- -D warnings` + `cargo test -p <crate>` for each crate with code changes (run the crate's own test targets; integration tests under `tests/` are included by `cargo test -p <crate>`). Cross-crate public-API changes: also test the direct consumers of the changed API.
   - Desktop TS: `tsc --noEmit`, `eslint`, `vitest run` only when `desktop/` files changed; plus `cargo fmt --check` / `cargo clippy` under `desktop/src-tauri` only when it changed.
   - Docs / `AGENTS.md` only: `make check-docs` (if it already fails on `origin/main` for an unrelated file, say so instead of fixing it here).
   - Do NOT run workspace-wide `make test` / `make lint-rust` for a module-scoped PR - CI runs the full matrix and is the backstop. Escalate to a full local pass only when a change cuts across many crates (e.g. `packages/rpc` wire-contract changes) or after a CI failure local repro is needed.
3. Commit fmt/clippy fixes together with the work on the task branch. Do **not** push, open a PR or arm auto-merge here — the push happens in *Finishing a task*, and the PR belongs to the scheduled task (below).

**Always sync with `origin/main` right before merging into `dev-<username>`** — not just at the start of the pass. Re-run `git fetch origin main`; if main moved while you were running checks, merge it into the task branch again and re-run the checks it affects. Branch protection requires the PR head to be up to date with main, and on a fast-moving main a stale branch bounces between BEHIND and re-queued CI.

Do not skip steps or use narrower flags than CI — a green local check on a smaller scope does not guarantee CI passes (e.g. clippy without `--all-targets` misses test code). `make help` lists every target.

During normal development you don't need to run this full suite every time — iterate on targeted checks (`cargo check`, a single test, `tsc`) to save time. The full pass is mandatory before the final merge into `dev-<username>`; without it the PR will fail.

### Finishing a task

A task is **not** finished while its work exists only on the task branch. Finish it by merging into the integration branch — always, even when the user did not say so, and before the session ends or its worktree is removed:

1. **In the task worktree**: `git fetch origin --prune`, then `git rebase origin/dev-$GH_USER` (and `git merge origin/main` when main moved); resolve conflicts here and re-run the scoped checks they touch. Leave the worktree clean (`git status --short` empty) and make sure the checks from *Before committing and pushing* pass.
2. **In the main worktree**, which holds `dev-<username>`: `git merge --ff-only origin/dev-$GH_USER` to bring the local branch current, then **collapse it onto `origin/main` if its content is already there** (*Collapse a stranded branch* below) — otherwise the merge below drags a previous PR's landed commits into this PR — and only then merge the task branch: `git merge --no-ff <type>/<name>`, so the merge also brings main's latest in with it and the task reads as one unit. Resolve conflicts in the main worktree and re-run the checks the resolution touches. If the main worktree has uncommitted changes (the user is working in it), do not stash or reset them: stop and report that the merge is waiting.
3. **Push from the main worktree**: `git push origin dev-$GH_USER`. If the push is rejected, `git fetch origin`, rebase onto `origin/dev-$GH_USER`, re-run the affected checks, push again. Never force-push.
4. **Then remove the worktree** with the guards in *Development workflow* above (`git worktree remove .worktrees/<name>`, `git branch -d <type>/<name>`).

If a task has to stop before it is merged — the user said to pause, or it is blocked — keep the worktree and say plainly that the branch `<type>/<name>` is unmerged, with its SHA and where it lives, so the next session (or the user) can finish it.

### Collapse a stranded branch

A squash merge puts the branch's *content* on `main` and leaves the branch's own commits where they were, so right after a merge `dev-<username>` reads "N commits ahead" while `git diff --stat origin/main...dev-<username>` prints nothing. **A PR must carry the difference from `main`, not the history that produced it**: cut a task branch from the stranded branch, or merge a task into it before it is collapsed, and the PR page lists commits that are already on `main` — the review claims work the PR does not contain, and every further task compounds it (2026-10-20: PR #1268 showed 16 commits for a two-file diff, seven of them in `main` since #1267; the session that landed them started two minutes after the merge, while the branch still held the old commits).

The drift is only collapsible while the branch's content still equals `main` — once a task lands on it the diff is real. So collapse at the first moment you are in the main worktree after a merge, and again before merging a finished task branch if it is still stranded. Use the guarded reset in *After a PR merges* step 2 (both worktree-held and branch-not-held forms are there): `git -C <main worktree> status --short` must be empty, `git -C <main worktree> diff --quiet origin/main` must succeed, then reset and force the branch — a bare `--force` on a stale view of the remote deletes whatever another machine pushed after your fetch, which is the one loss this repo cannot recover by re-running.

If either guard fails, leave the branch alone: a dirty worktree is someone's work in progress, and a non-empty diff is work that has not landed — that content belongs in the PR, so say so in the summary instead of dropping it. Never `reset --hard` a worktree whose `status --short` is not empty.

### PRs and merges

**A scheduled task owns the PR.** `dev 分支 PR 合并` runs in this repo hourly, on whichever machine has it, and does the whole lifecycle **for the branch of the account it runs as** (`dev-<that account's login>`) — never for a different author's branch, and never by folding other branches into it:

- No open PR and `origin/dev-<username>` has commits `origin/main` lacks → it opens one against `main` and arms squash auto-merge (`gh pr create --base main --head dev-$GH_USER --fill`, then `gh pr merge <n> --squash --auto`). A PR never stays open without auto-merge armed.
- PR is BEHIND → it merges `origin/main` into `dev-<username>` in the main worktree (the one that holds the branch) and pushes, instead of letting the PR bounce between BEHIND and re-queued CI. If that worktree is dirty, leave the branch alone for this pass and report it.
- Required checks fail → it opens a **new session** in this repo with the failing job/run evidence, rather than messaging the session that wrote the code. That session fixes the failure on a task branch and merges it into `dev-<username>` by the flow above, then pushes; the next pass re-checks. Do not merge a red PR, re-run checks indefinitely, or weaken assertions to get it green.
- Checks pass → the squash merge lands by itself; the task then runs *After a PR merges* and prints a short report (branch, ahead count, PR number, check result, fix session id).

**There is no session-side fallback.** If this machine has no `dev 分支 PR 合并` task — or its runs look broken — a task session still ends with `git push origin dev-$GH_USER` and nothing else: no `gh pr create`, no `gh pr merge --auto`. Another machine runs that task and opens the PR, so a branch that waits a few hours for it is fine; say in your summary that the PR has not been opened yet instead of opening it.

### When a PR is failing or stale

The scheduled task owns this as well: on BEHIND it merges `origin/main` into `dev-<username>` on the machine that holds the branch, and on a red required check it opens a session carrying the failing evidence. A session acts here only when it is asked to — as the session that wrote the code, or by the user.

- **Red required check**: read the failing job's log (`gh run view <run-id> --log-failed`), fix the cause on a task branch, merge it into `dev-<username>` by *Finishing a task*, and push. Never weaken an assertion or re-run checks until they go green.
- **BEHIND / "update branch" requested**: `git fetch origin main && git merge origin/main` into `dev-<username>` in the main worktree, resolve conflicts there, re-run the scoped checks the merge touches, and push; `--auto` re-fires by itself.
- **Check PR status synchronously — never `sleep`-poll.** Wait with `gh pr checks <n> --watch --required --fail-fast` (add `--interval 5`); `--watch` blocks until the required checks finish and `--fail-fast` returns on the first failure. Do not loop `sleep N; gh pr checks`.

### After a PR merges

The scheduled task does this right after the squash merge. A session that merely notices the merge only needs to bring its own local copies up to date — never re-merge or re-target anything.

1. **Update the local main branch**: `git fetch origin main:main` — that writes the fetched `main` straight into the local ref without checking it out, so it works while a worktree is parked on `dev-<username>` or on anything else. (Only fall back to `git merge --ff-only origin/main` when `main` itself is the branch checked out in the main worktree.) If the update is refused, identify what local `main` is carrying before concluding anything — do not assume it is the user's work:
   - `git log --oneline origin/main..main` lists the commits upstream does not have.
   - A **stale duplicate** is a branch commit that was later squash-merged: its tree matches the squashed commit, and that squashed commit is already an ancestor of `origin/main`. Confirm both (`git diff --stat <local-sha> <squashed-sha>` prints nothing; `git merge-base --is-ancestor <squashed-sha> origin/main` succeeds), then ask the user before dropping it with `git reset --hard origin/main`.
   - **Anything else is live work** — the user's, or another session's (`git worktree list`, and `git branch --contains <sha>` says whose). Never rebase, reset, or force it: stop and tell the user the commit, its branch, and that it has not been pushed.
2. **Re-point `dev-<username>` at the new `main`** so the next task starts from what actually landed — the branch's own commits are upstream already, under different squash SHAs. If nothing holds it (the main worktree is parked elsewhere): `git branch -f dev-$GH_USER origin/main`, then `git push --force-with-lease origin dev-$GH_USER` if the remote branch still needs the reset — the lease, not a bare `--force`, so a push that lands on a stale fetch cannot delete a commit another machine pushed meanwhile. This is *Collapse a stranded branch* seen from the merge's side: do it as soon as the merge lands, because the next task to start is what makes the drift permanent. **Keep commits made after the PR head**: rebase them onto the new `origin/main` and push them — never drop them. This post-merge reset is the **only** sanctioned force-push in this repo; task sessions never force-push otherwise. **Do not delete the remote branch** — it is the continuous per-developer branch the next task continues on, and deleting it discards the PR review timeline.

   **When the main worktree holds `dev-<username>`** — the normal case, since that is where the user parks it — `git branch -f` is refused, and `git merge --ff-only origin/main` inside it **cannot work**: after a squash merge the branch has its own commits, so the two are diverged and `--ff-only` is *not possible to fast-forward*. Nothing is lost by that refusal, but the branch then reads "N commits ahead of main" forever while `git diff --stat origin/main...dev-<username>` is empty. Do this instead, in the worktree that holds it, and only when **both** guards pass:

   ```bash
   git -C <that worktree> status --short     # must be empty: no uncommitted work to discard
   git diff --quiet origin/main              # must succeed: the content is already in main
   git -C <that worktree> reset --hard origin/main
   git push --force-with-lease origin dev-$GH_USER
   ```

   If either guard fails, leave the branch alone and say so: a dirty worktree is someone's work in progress, and a non-empty diff means commits that have not landed yet.

   **An empty diff is a reset, never a PR.** After a squash merge the branch still shows commits "ahead" (`origin/main..dev-<username>` non-zero) while `git diff --stat origin/main...dev-<username>` is empty — the content is already in `main`. Run the reset above; an empty-diff PR merges nothing and only resets the branch the long way round.
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
- The Rust binary `future-agent` is the gRPC backend, defaulting to per-user local IPC. Unix uses `FUTURE_AGENT_SOCKET` when set; otherwise the endpoint always lives under the instance's FutureOS home (`FUTURE_HOME` moves the whole `~/.future` root: lock, database, sessions, logs, and `<home>/run/agent.sock`), defaulting to `~/.future/run/agent.sock` — shared runtime directories such as `$XDG_RUNTIME_DIR` are deliberately not consulted. Windows uses a current-user-only named pipe. TCP requires explicit `--grpc-addr`; clients use `FUTURE_AGENT_GRPC_ADDR` (channels: `agent.grpc_addr`, default `auto`). TUI/Desktop can start a sidecar when no agent is reachable. `future <cmd>` is the unified entry point for every Rust component (`future agent|tui|channel|goal <args>` — each runs the same code as the standalone `future-agent` / `future-tui` / `future-channel` / `future-goal` binaries, which remain buildable (`cargo build -p <crate>`) but are no longer installed by default; `make run-*` targets also still work).
- **Testing the agent: always use a fresh port, never disturb the running agent.** When starting an agent for tests or manual gRPC debugging, use an isolated HOME/USERPROFILE and bind `--grpc-addr 127.0.0.1:<new-port>` (never an existing service's port, including the conventional `50051`) and never `kill` an existing agent. A fresh port alone does not bypass the per-user agent singleton lock. Replay sessions against an isolated FutureOS home (e.g. `FUTURE_HOME=/tmp/x future agent`) with `--grpc-addr 127.0.0.1:PORT`; the real `:50051` agent is unaffected, and `FUTURE_HOME` also gives the test instance its own IPC endpoint. See `.future/memory/agent-e2e-grpcurl.md`.
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
