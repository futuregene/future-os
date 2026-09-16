# Documentation ↔ Code Mismatch Audit

[中文版](doc-code-mismatches.zh-CN.md)

Audit of the migrated documentation against **current code**, produced as part
of the docs-reorg effort (todo `todo_9b5f2e830753`, session
`20260916-113427-73c5a9`). This file is a findings list only — it does not
modify the audited documents.

- **Baseline:** worktree `/Users/geilige/future-os/.worktrees/docs-reorg`,
  branch `claude/docs-reorg`, HEAD `7c2611b5` (docs: bilingualize
  internals/architecture part 1). A parallel session is landing further
  bilingualization commits while this audit runs; several
  `docs/internals/desktop/` files were renamed mid-audit
  (`CONNECTION.md` → `CONNECTION.zh-CN.md` etc.). Re-resolve `file:line`
  references at fix time.
- **Method:** every entry below was checked against source, with the document
  location (`file:line`) and code evidence (`file:line`) cited. Nothing here
  is from memory or inference. Claims that could not be confirmed are marked
  **【待核实】** with the missing evidence named.
- **Scope:** commands and flags; file/directory paths; environment variables;
  config keys and defaults; behavior/state-machine descriptions; removed or
  renamed features; architecture/data-model assertions; cross-document
  contradictions. Historical records (`docs/archives/`,
  `long-run-evidence-ledger.md`, dated verification notes) keep their
  timestamps and commit boundaries per the reorg rules and were not re-audited
  as "current truth".
- **Categories:** 【错误】 doc contradicts code · 【过时】 doc describes a
  past state · 【缺失】 doc omits something user- or operator-visible.

## Verified clean (high-risk areas, no findings)

These were checked line-by-line against code and **matched**:

- `docs/guide/tui.md` — all 19 slash commands match
  `tui/src/app.rs` `handle_submit` (model/sessions/help/reload/compact/
  export/import/clone/fork/tree/new/name/scoped-models/cwd/approve/reject/
  stop/cancel/status); help-screen subset matches `tui/src/help_screen.rs`;
  `future tui` flags match `tui/src/index.rs`; settings keys
  (`defaultModel`, `defaultThinkingLevel`, `defaultPermissionLevel`,
  `enabledModelIds`) match `tui/src/app.rs:294-332`; `PI_DEBUG_REDRAW=1` /
  `PI_TUI_WRITE_LOG=1` paths match `app.rs:4048-4056` and
  `terminal.rs:460-464`.
- `docs/guide/channels-config.md` — all 15+ config keys and their defaults
  match `channels/src/config.rs` (agent block, feishu block, dingtalk block);
  9 slash commands match both bridges; 30s/20s keepalive pings match
  `feishu_ws.rs:78` / `dingtalk_ws.rs:32`; 128-event per-conversation buffer
  matches `feishu/bridge.rs:134` / `dingtalk/bridge.rs:64`; `max_image_mb`
  enforced without Content-Length matches `feishu_rest.rs:769`; first-run
  template-and-exit matches `config.rs` `load()`.
- `docs/architecture/loop-control-plane.md` — 7 groups / 41 commands and
  every group count match `orchestration/loop/src/console.rs`
  `build_cli_registry` (5/6/6/18/3/2/1); todo classes
  (advancement/monitor/blocker/coordination/user_gate/user_action) match
  `state.rs:54-79`; `--parent` 3-level limit, `--gate-question`, `--role`,
  `--max-validation-attempts`, `--parent-session` match `console.rs:705-727,
  1364`; validator 120s default + `FUTURE_LOOP_VALIDATOR_TIMEOUT_SECS` match
  `validator.rs:7` / `executor.rs:209`; dashboard port 7717/127.0.0.1/GET-only
  match `console.rs:3175-3177` / `webui/server.rs:54`; 32-note batches match
  `agents/supervision.rs:13`; 5-minute verification-pending flag matches
  `agents/supervision.rs:214`; 3-turn delivery follow-up matches
  `work_items/delivery_outcome.rs:215-225`; semantic-history N=50 matches
  `decision/goal_frontier/semantic_history.rs:21`.
- `docs/internals/desktop/` sandbox & connection — `SANDBOX/MACOS.*` claims
  (`deny default`, `(allow file-read*)`, `(allow network*) (allow
  system-socket)`, `/usr/bin/sandbox-exec` existence check,
  "Operation not permitted"/"sandbox-exec" heuristics, auth.json temporary
  override) match `agent/src/sandbox/seatbelt.rs:89-102` and
  `sandbox/mod.rs:924, 1220`; `CONNECTION.*` module references
  (`remote/services.rs`, `remote_host/`, `agent_events.rs`, `headless/`,
  `--no-qr`/`--re-pair`, `remote_pending_revokes.json`, support codes
  PA001/LC003) match the desktop sources; `REMOTE_E2EE.md` Noise patterns
  match `packages/remote-crypto/src/lib.rs:34-35`; `embedded-terminal.md`
  routes/TTL match `desktop/src-tauri/src/terminal/server.rs:188` and
  `ticket.rs:20`; `desktop-windows.md` matches
  `windows/installer-hooks.nsh` (exit 32, `/UPDATE`, legacy exe names).
- `docs/wiki/**` — CLI/Settings/Installation/Quick-Start/Sandbox/Skills/
  Feishu/Home/FAQ/Remote pages: commands, defaults (approval mode `off` =
  `desktop/src/integrations/storage/appSettings.ts:44`), 4-image/25-MiB
  limits (`attachments.ts:11,19`), Bubblewrap ≥ 0.9.0 (`linux/probe.rs:351`),
  diagnostic codes (`sandbox/mod.rs:979`, `linuxSandboxStatus.ts`), skills
  table (15 builtins), `future init` behavior (`commands/init.rs`), install
  scripts (`scripts/install.sh`, `install.ps1`).
- `docs/architecture/rpc.md`, `docs/internals/provider-protocol.md`,
  `docs/architecture/sqlite-migration.*` (flags, 100ms/128/64KiB micro-batch
  at `agent/src/rpc/protocol.rs:227-229`, WAL/FULL at
  `session/database.rs:122-123`), `docs/guide/build-and-install.md` (make
  targets, Rust 1.97.0, Node 24, mold), `docs/guide/directory-layout.md`
  (every listed path exists; loop layout files all present in
  `orchestration/loop/src/`), `docs/guide/desktop-headless.md`,
  `docs/internals/desktop/ER.*` (table inventory matches
  `desktop/src-tauri/src/store/schema.rs`).

---

## 【错误】 Errors — the doc contradicts the code

### E1. "Explicit TCP tries TCP first, then local IPC" — no such fallback exists

- **Doc:** `docs/guide/channels-config.md:62` —
  "An explicit `http://host:port` tries TCP first, then local IPC";
  `docs/guide/channels-config.zh-CN.md:61` ("先尝试 TCP 再回退 IPC");
  `docs/guide/directory-layout.md:83` — "An explicit client TCP address is
  tried before local IPC"; `docs/guide/directory-layout.zh-CN.md:75`.
- **Code:** `packages/rpc/src/transport.rs:59-61` — "An explicit TCP endpoint
  is authoritative: a failed remote/development target must not silently
  redirect commands to an unrelated local Agent.";
  `packages/rpc/src/transport.rs:62-75` — `connection_plan()` maps any
  non-`auto` value to `vec![AgentEndpoint::Tcp(..)]` only; there is no
  `Local` entry in the plan, so no IPC fallback can occur.
- **Cross-doc contradiction:** `docs/guide/tui.md:18` states the correct
  behavior: "an explicit TCP target is authoritative and never falls back to
  local IPC."
- **Suggested fix:** reword the two guide files to "an explicit
  `http://host:port` is authoritative — connection failures are reported, not
  redirected to local IPC". Also drop the "tries TCP first" phrasing from
  both languages.

### E2. `~/.future/tui/keybindings.json` is documented but never read

- **Doc:** `docs/guide/tui.md:81-82` — "Optional user keybinding overrides
  can be placed at `~/.future/tui/keybindings.json`";
  `docs/guide/tui.zh-CN.md:75`; `docs/guide/directory-layout.md:29` and
  `:115`; `docs/guide/directory-layout.zh-CN.md:28,104`.
- **Code:** the mechanism exists but is dead: `tui/src/keybindings.rs:102-103`
  documents `apply_overrides` "e.g. from ~/.future/tui/keybindings.json", yet
  **no non-test caller exists** — `KeybindingManager::new()` starts with an
  empty override map (`keybindings.rs:44-49`), the app registers bindings at
  `tui/src/app.rs:855-960` without loading any file, and the only file the
  TUI ever reads is `~/.future/tui/settings.json` (`tui/src/index.rs:756-760`,
  `tui/src/app.rs:3360`). A repo-wide grep finds `keybindings.json` nowhere
  outside `keybindings.rs` itself.
- **Impact:** users who create the file per the docs see no effect.
- **Suggested fix:** either wire the file load (read + `apply_overrides` at
  startup) or mark the file as reserved/not-yet-supported in all four doc
  locations. Do not silently keep documenting a non-functional feature.

### E3. Feishu "per-group overrides at runtime" — API exists but nothing calls it

- **Doc:** `docs/guide/channels-config.md:85-86` — "Per-group overrides are
  possible at runtime (e.g. disable a specific chat); the config file above
  only sets the defaults";
  `docs/guide/channels-config.zh-CN.md:84`.
- **Code:** `PolicyEngine::set_override` exists
  (`channels/src/feishu/policy.rs:105-108`) and `check_group` consults the
  override map (`policy.rs:54-60, 66-95`), but the bridge never calls it:
  `channels/src/feishu/bridge.rs:84` constructs
  `PolicyEngine::new(feishu_cfg.policy.clone())` and a repo-wide grep finds
  `set_override` only inside `policy.rs` (its own unit tests). There is no
  runtime path (CLI, hot-reload, API) for an operator to disable a specific
  chat.
- **Suggested fix:** remove the claim, or state explicitly that per-chat
  overrides exist only as an internal API with no operator-facing entry
  point (and file the gap if the feature is intended).

---

## 【缺失】 Missing from the docs

### M1. DingTalk wiki omits `sender_allowlist` entirely (user-visible, high cost)

- **Doc:** `docs/wiki/en/DingTalk.md:53-72` (config JSON example) and
  `:75-82` (field table) — no `sender_allowlist`;
  `docs/wiki/zh/DingTalk.md:53-72, 75-82` — same omission.
- **Code:** `channels/src/config.rs:56-60` — `DingtalkChannelConfig.
  sender_allowlist` with the comment "Empty denies all"; enforced at
  `channels/src/dingtalk/bridge.rs:99-101` (authorization check, which also
  gates slash commands — `dingtalk/bridge.rs:195` dispatches only after the
  allowlist check).
- **Impact:** a user following the wiki exactly gets a bridge that **denies
  every sender, including `/help`**, with no documented remedy. The guide
  page documents this correctly and even has the upgrade note
  ("previously configured DingTalk bridges must populate `sender_allowlist`
  before receiving prompts") — the wiki, which is the page users actually
  see, has neither.
- **Suggested fix:** add `"sender_allowlist": ["..."]` to the JSON example
  and a field-table row ("empty denies all senders; `["*"]` trusts all") in
  both languages, plus a one-line upgrade note.

### M2. TUI shortcuts table is missing registered bindings

- **Doc:** `docs/guide/tui.md:63-76` — 9 rows (ctrl+p/t/o/r/c, tab, enter,
  escape, arrows).
- **Code:** `tui/src/app.rs:855-960` additionally registers `ctrl+l` ("Clear
  screen / redraw"), `shift+tab` ("Cycle thinking"), `page up` / `page down`
  ("Scroll chat up/down"), `ctrl+↑` / `ctrl+↓` ("Scroll chat up/down (line)").
  The help overlay (`help_screen.rs:16-37`) also omits these, which is
  consistent, but the doc table presents itself as the shortcut list without
  a "subset" caveat.
- **Suggested fix:** add the missing rows (or add an explicit "the in-app
  help shows a subset" note for shortcuts as the doc already does for slash
  commands).

### M3. `packages.md` omits the `remote-crypto` package

- **Doc:** `docs/architecture/packages.md:7-10` — lists only `rpc`,
  `markdown`, `thread-projection`, `json-preview`.
- **Code:** `packages/` contains five directories; `packages/remote-crypto`
  is a Rust crate consumed by the desktop backend
  (`desktop/src-tauri/Cargo.toml:47` — `future-remote-crypto = { path =
  "../../packages/remote-crypto" }`), implementing the Noise E2EE patterns
  documented in `docs/internals/desktop/REMOTE_E2EE.md`
  (`packages/remote-crypto/src/lib.rs:34-35`).
- **Suggested fix:** add a bullet: "`remote-crypto`: Rust Noise-protocol
  end-to-end encryption shared by the desktop/mobile remote channel."

### M4. `loop-control-plane.md` omits `lease expire` and two scheduler verbs

- **Doc:** `docs/architecture/loop-control-plane.md:54` — "Lease |
  `lease claim/renew/release/status`"; `:60` — "Scheduler |
  `scheduler tick/show/liveness`" (also `loop-control-plane.zh-CN.md:39,44`).
- **Code:** registry usage strings include `expire` and the extra scheduler
  verbs: `orchestration/loop/src/console.rs:348` — "lease
  claim|renew|release|**expire**|status" (handler at `:5931`);
  `console.rs:521` — "scheduler tick|show|**record-host-failure**|**ack**|
  liveness" (handlers at `:3535-3536`).
- **Suggested fix:** add `expire` to the lease row and
  `record-host-failure`/`ack` to the scheduler row in both languages.

### M5. `channels-config.md` doesn't document the Feishu `domain` value forms

- **Doc:** `docs/guide/channels-config.md:74` — "`domain` | `feishu` | API
  domain." (zh: `channels-config.zh-CN.md:73`).
- **Code:** `channels/src/feishu/config.rs:56-84` — three accepted forms:
  `"feishu"` → `open.feishu.cn`, `"lark"` → `open.larksuite.com`
  (`api_base`/`api_domain`/`ws_base`), and a full `http(s)://` URL used
  verbatim (self-hosted gateways / test mocks).
  The wiki already documents `"lark"` (`docs/wiki/en/Feishu.md:92`).
- **Suggested fix:** extend the field reference to name the three forms so
  the guide is not less complete than the wiki.

---

## 【过时】 Outdated

### O1. `CONTEXT_COMPACTION.zh-CN.md` status line predates the semantic implementation

- **Doc:** `docs/internals/desktop/CONTEXT_COMPACTION.zh-CN.md:3` —
  "状态：**v2 数据底座已落地；语义压缩阶段设计已确认，待开发**
  （2026-08-24）"; the S1–S4 plan describes the semantic pipeline as future
  work.
- **Code:** the semantic machinery now exists in the agent:
  `agent/src/compaction/semantic.rs` (2325 lines) and the public entry points
  `prepare_semantic` / `prepare_semantic_with_phase` /
  `prepare_semantic_with_phase_and_fallback` /
  `prepare_semantic_with_lifecycle` (`agent/src/compaction/mod.rs:183-239`).
- **【待核实】:** whether the full S1–S4 scope is wired into the runtime
  (e.g. which callers invoke `prepare_semantic*`, and the provider-chain
  status). Missing evidence: a call-graph walk of `compaction/mod.rs` usage.
- **Suggested fix:** after that walk, refresh the status line to say which
  phases have landed and which remain, rather than the blanket "待开发".

---

## 【待核实】 Unconfirmed — evidence missing

### U1. "glibc ≥ 2.39" floor for published Linux GUI

- **Doc:** `docs/wiki/en/Installation.md` (Linux section), `docs/wiki/en/FAQ.md`
  ("Linux GUI won't start on an older server"), and
  `docs/guide/build-and-install.md`.
- **Evidence:** consistent with `.github/workflows/build-linux.yaml:40-48`
  (aarch64 built on `ubuntu-24.04-arm`; x86_64 on `ubuntu-latest`, which is
  currently Ubuntu 24.04 → glibc 2.39), but the floor itself is not pinned
  anywhere in the repo.
- **Needed:** release build logs or `ldd`/`objdump` inspection of a published
  binary to confirm the claim as stated.

### U2. Cross-repo / historical baselines

- `docs/internals/desktop/CONNECTION.*` cites FutureOS baseline `2774e4a9`
  and future-server `bbd6c23`; `docs/architecture/loop/UPSTREAM.md` cites the
  LoopX upstream base. Neither repository state is verifiable from this
  checkout. These docs self-mark 现状/目标/待验证 where appropriate, so no
  finding is recorded; re-verify the baselines when the referenced repos are
  available.

---

## Notes for the fix PRs

1. **E1/E2 affect multiple files per language** — fix both `.md` and
   `.zh-CN.md` of `channels-config`, `directory-layout`, and `tui` in the
   same PR so the checker's pairing rule stays satisfied.
2. **M1 is the highest user-visible cost** — the DingTalk wiki currently
   leads to a fully-denied bot; consider landing it ahead of cosmetic fixes.
3. **Path instability:** the concurrent bilingualization session renames
   `docs/internals/desktop/*` files (`CONNECTION.md` → `CONNECTION.zh-CN.md`
   etc.). Re-resolve every `file:line` above at fix time.
4. This audit intentionally did **not** modify any audited document; fixes
   belong to the follow-up fact-fix PR (task ③ in the goal objective).
