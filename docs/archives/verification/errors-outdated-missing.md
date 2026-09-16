# Document verification results: error / outdated / missing list (historical record)

> This is a faithful paragraph-by-paragraph English translation of the historical snapshot [文档核验结果：错误 / 过时 / 缺失清单（历史记录）](./errors-outdated-missing.zh-CN.md)（2026-08-06，commit `71f22b0b`）. Conclusions, dates and commit boundaries are preserved verbatim; this translation is not a new verification result.

> **Historical snapshot — cannot be used to skip current verification.** Every "fixed / no change needed / PASS" below is valid only for its original date and commit;
> old conclusions such as no Linux sandbox, default TCP, per-tool approval no longer apply to current source. The current 2026-09-07
> documents already incorporate these changes; see the [document index](../../README.md), [approval and sandbox](../../wiki/en/Sandbox.md)
> and [phone remote](../../wiki/en/Remote.md). The original acceptance process, line numbers and results are preserved below; historical PASS is not rewritten.

> **Internal working document** (not user documentation). Produced by todo_8bd559237c7d "verify the document fact inventory against source".
> Verification targets: key factual claims in README + all documents under `docs/`, checked against the current worktree source
> (branch `claude/loop-hardening`, after HEAD `3ed92ad7`).
> Criterion: document claim vs actual source behavior. Line numbers are as of the 2026-08-06 worktree.

---

## A. Errors (document statements that contradict source, should be corrected)

### A1. CLI binary name: `future`, not `future-cli`

| Claim | Location | Basis |
|---|---|---|
| 「The release binary is named **`future-cli`**…user-facing wiki must always use `future-cli`, never `future`」 | docs/wiki-prompt-en.md L127, L170-171, L173-174, L219 | `cli/package.json` `bin: {"future": "dist/index.js"}`；Makefile `build-cli` → `bun build --compile dist/index.js --outfile dist/future`（Makefile L159）；Tauri sidecar name `future-<triple>`（desktop/src-tauri/tauri.conf.json externalBin）。**The release binary is `future`**. The zh wiki-prompt.md L171/L219 ban on `future` is the correct one |

→ ✅ **Fixed (todo_285b37996a0d, commit below)**: wiki-prompt-en.md changed all `future-cli` to `future`（§6 table/sidebar、§7 CLI positioning/location/running/command groups/tips、§9 A.4 self-check items）, consistent with the zh version.

→ ✅ **wiki pages also fixed (todo_bcf715c7cc0e, commit below)**: en/zh CLI.md rewritten (Windows line changed to "both installer and portable include it", "portable-only" note removed, agent section changed to "CLI cannot start/stop the agent", command groups fully aligned with index.ts), Feishu.md/DingTalk.md "Start the Bridge" sections changed to `make build-channels` + `./target/release/future-channel` + "no future channel command" note, troubleshooting `future channel status` changed to checking the `future-channel` process.

### A2. `future agent start/stop/restart` and `future channel …` were removed from the CLI (2026-07-16)

commit `eed93369`（2026-07-16）`refactor(cli): remove service management and launcher commands` deleted `future agent *`, `future channel *`, `future gui`, `future tui`. Current state: `future agent` only has `status`; **there is no `channel` command group**. After `make install`, users run the `future-agent` / `future-channel` binaries directly.

Affected locations (both en/zh need changes):

| Claim | Location |
|---|---|
| `future agent start`（agent must be running） | docs/wiki/{en,zh}/CLI.md L41, L59-61, L119；docs/wiki/{en,zh}/Feishu.md L22；docs/wiki/{en,zh}/DingTalk.md L22 |
| `future channel start/status/stop/restart`（service management） | docs/wiki/{en,zh}/Feishu.md L144-147(en)/L145-148(zh)；docs/wiki/{en,zh}/DingTalk.md L96-99；docs/wiki/{en,zh}/Feishu.md L185(en)/L186(zh)、DingTalk.md L151（troubleshooting「future channel status」） |
| wiki-prompt command-group descriptions | docs/wiki-prompt.md L177, L183, L185；docs/wiki-prompt-en.md L177, L183, L185 |

→ ✅ **wiki-prompt part fixed (todo_285b37996a0d)**: both prompts' CLI sections fully aligned with the actual command surface of `cli/src/index.ts` — `agent` group only has `status`（no start/stop；CLI cannot start/stop the agent, noted）、deleted the nonexistent `channel` group、skills adds `install-builtin`/`update`、auth adds `credential`、adds `init`/`account`/`models`/`session`/`doctor` groups、tools adds `describe` and flags such as `--input`、run adds `--fork`/`--session`/`--permission`；also corrected "agent must be running" and the `future agent start` in FAQ troubleshooting (changed to opening the desktop app or running `future-agent` manually).

⚠️ **The wiki pages themselves (docs/wiki/{en,zh}/CLI.md、Feishu.md、DingTalk.md) still await a follow-up todo**（A3/A4/B1/B7 below and the in-page `future channel *`, `build-channels-release` in Feishu/DingTalk）. Also: this pass found the Windows installer also ships the CLI — build.yml copies `future.exe` as the Tauri sidecar `binaries/future-<triple>.exe` into the NSIS installer, so the old statement "CLI is portable-only"（including the note under the wiki CLI.md table）should become "both installer and portable include it".

Suggested wording: "startup components: `future-agent`（agent）、`future-channels`（channel bridge）；service management is no longer provided by the CLI, the desktop app starts the agent automatically"（matching the intent of the removal commit）.

→ ✅ **Fixed (todo_bcf715c7cc0e)**: en/zh CLI.md/Feishu.md/DingTalk.md changed to the wording above（CLI cannot start/stop the agent；the channel bridge is a standalone service `future-channel`，no `future channel` command）.

### A3. `make build-channels-release` does not exist

| Claim | Location | Basis |
|---|---|---|
| Channel bridge started with `make build-channels-release` | docs/wiki/{en,zh}/Feishu.md L137(en)/L138(zh)、DingTalk.md L89(en/zh) | Makefile has no such target (grep 0 hits). Correct target: **`make build-channels`** (Makefile L186) |

→ ✅ **Fixed (todo_bcf715c7cc0e)**: en/zh changed to `make build-channels` + `./target/release/future-channel`（also corrected the binary name — channels/Cargo.toml is `future-channel`, not `future-channels`）.

### A4. Feishu "reconnects every ~6 minutes" has no source basis

| Claim | Location | Basis |
|---|---|---|
| 「Bridge reconnects every ~6 minutes」/「每 6 分钟左右重连」 | docs/wiki/en/Feishu.md L190；zh L191 | channels/src/feishu/feishu_ws.rs：`DEFAULT_PING_INTERVAL=30`（keepalive ping 30s，this wiki statement is correct）、`HEARTBEAT_TIMEOUT=120`；mod.rs reconnect wait **5s**（`Duration::from_secs(5)`）。No 6-minute constant anywhere. → ✅ **Fixed (todo_bcf715c7cc0e)**: en/zh Feishu.md "Bridge reconnects every ~6 minutes" changed to "Bridge reconnects automatically" — keepalive ping 30s（feishu_ws.rs DEFAULT_PING_INTERVAL=30）、disconnect reconnect 5s（mod.rs L66）. |

### A5. `make test` does not run the loop control-plane tests —— ✅ fixed (todo_cbbb063d2fd4)

| Claim | Location | Basis |
|---|---|---|
| `make test # cargo test (agent + loop control plane)`（zh：「agent + loop 控制面」） | docs/build-and-install.md L182；zh L168 | Makefile `test:` = test-agent + test-channels + test-cli + test-tui + test-desktop + test-desktop-rust + test-mobile（Makefile L203-229）。**There is no test-loop target**，loop is not in make test |
| （same file, development section）`make test # cargo test (agent)` | docs/build-and-install.md L194；zh L180 | same as above，actually 7 suites |

→ Both changed to「all 7 suites: agent, channels, CLI, TUI, GUI, GUI Rust, mobile」（zh same）. `make clean`（removes build artifacts + installed binaries）、`future init`（installs skills + links local commands on macOS/Linux）、mold（x86_64-linux only）、loop is a workspace member（Cargo.toml L17）— these statements re-checked and correct.

---

## B. Outdated (features evolved / counts changed, should be updated)

### B1. wiki CLI.md command-group table outdated (missing new groups, has deleted groups)

Current state（cli/src/index.ts + commands/*.ts）：`init`、`auth`（login/status/**credential**/logout）、`account`（profile/balance）、`run`、`skills`（list/install/install-builtin/uninstall/**update**）、`tools`（list/describe/call）、`models`、`agent`（**status only**）、`session`（list/info/rename/delete）、`doctor`。

| Problem | Location |
|---|---|
| agent group lists start/stop/restart（deleted） | docs/wiki/{en,zh}/CLI.md L59-61 |
| channel group does not exist at all | docs/wiki/{en,zh}/CLI.md L84-93（en）/corresponding zh section |
| skills missing `update`、`install-builtin`（`update` implemented at cli/src/commands/skills.ts L48） | docs/wiki/{en,zh}/CLI.md skills section（near en L106） |
| auth missing `credential` | docs/wiki/{en,zh}/CLI.md auth section |
| missing whole groups: `session`、`models`、`account`、`init`、`doctor` | docs/wiki/{en,zh}/CLI.md |

→ ✅ **Fixed (todo_bcf715c7cc0e)**: en/zh CLI.md rewritten, command groups = init / auth（with credential）/ account / run（adds --fork/--session/--permission）/ skills（adds install-builtin/update）/ tools（adds describe）/ models / agent（status only）/ session / doctor；`channel` group deleted.

### B2. 「future skills install about 13」→ actually 14 —— ✅ fixed (todo_cbbb063d2fd4)

| Claim | Location | Basis |
|---|---|---|
| `future skills install # install all future-* skills (~13)`（zh same） | docs/build-and-install.md L171；zh L159 | skills/builtin/ now contains **14** future-* skills（future-account/browser/database-lookup/deep-research/document/experimental-design/image/paper/peer-review/scientific-writing/skill-creator/slides/software-install/web） |

→ Changed to「(14)」/「（14 个）」. Also noted: **`future skills update` does exist**（cli/src/commands/skills.ts L19/L48-49/L287-328 implements updateSkills）— the build-and-install L165/zh L163 statement is correct；the error is wiki-prompt W12/WE（says "no update"），left for the wiki-prompt todo. → ✅ **Fixed (todo_285b37996a0d)**: both wiki-prompts' skills subcommands changed to `list` / `install [<name>]` / `install-builtin` / `uninstall <name>` / `update`.

### B3. README model-count statement outdated (undercounts) —— ✅ fixed (commit b3b2e114, todo_5d852f73fcb6)

| Claim | Location | Basis |
|---|---|---|
| 「1000+ built-in models across 100+ providers」（zh same） | README.md L26；README.zh-CN.md L23 | Generated catalog docs/wiki/{en,zh}/Models.md L3 now says「3826 models across 143 providers」（`make generate-models` runs scripts/generate_models.py；the manual README number is stale）。Suggest referencing the generated catalog or writing「3800+ / 140+」 |

→ Changed to「3800+ models across 140+ providers」（en）/「内置 3800+ 模型，覆盖 140+ Provider」（zh）.

> **Models.md generation note supplement（todo_bcf715c7cc0e）**: Models.md confirmed as a generated file（`make generate-models` → scripts/generate_models.py writes docs/wiki/{en,zh}/Models.md）. Added the「Auto-generated by `make generate-models` (scripts/generate_models.py). Do not edit by hand.」comment line to the script's en/zh header templates, and hand-added the same comment to the two currently committed Models.md headers（preserved by the script on the next regeneration）.

> **README full re-check（2026-08-06, todo_5d852f73fcb6 wrap-up）**: apart from B3/C1, every remaining README claim was checked against source one by one and is correct — sandbox three tiers（off/manual/sandbox「macOS Seatbelt, macOS only」matches the proto comment exactly）、JSONL sessions + fork/clone/tree + query-count（agent/src/session/mod.rs）、YAML frontmatter skills multi-directory discovery（agent/src/skills/mod.rs APP_SKILLS_DIR + AGENTS_SKILLS_DIR）、auto compaction + context-overflow exponential-backoff retry（agent/src/agent/run_loop.rs `is_retryable_size_error` → retry after compaction，`delay_ms = 2000 * (1 << (retry_attempt-1))`；llm/mod.rs L299 `context_length_exceeded`）、`future auth login` device code + auto model-list sync（cli/src/commands/auth.ts saveAuth + agent sync_future_models RPC）、future-agent:50051 / future-tui binary names、8 keyboard shortcuts、all internal links and banner exist. **README en/zh needs no further changes, follow-up todos can skip it.**

### B4. wiki-prompt page inventory has no Feishu/DingTalk/Integrations

| Claim | Location | Basis |
|---|---|---|
| Page inventory 10 pages, sidebar has no Integrations group | docs/wiki-prompt.md W6-W7（L52-109）；en equivalent | The actual wiki has Feishu.md、DingTalk.md，and _Sidebar contains an Integrations group（docs/wiki/en/_Sidebar.md L4-20） |

→ ✅ **Fixed (todo_285b37996a0d)**: both prompts' page inventory adds `Feishu.md`（飞书集成）、`DingTalk.md`（钉钉集成），§6 sidebar adds the「集成 / Integrations」group，§7 adds Feishu/DingTalk content points（code entry `channels/src/`；notes the channel bridge is a standalone service `future-channel`、no `future channel` command、9 slash commands handled locally、unknown slashes forwarded to the agent、config `~/.future/channels/config.json`、CardKit card streaming replies）；also notes **`Models.md` is auto-generated by `make generate-models`（scripts/generate_models.py），never hand-written and not in the sidebar**.

Other fixes in the same round (all verified against source): Installation section「signed + Apple notarized」→「current release packages are neither notarized nor signed（see `docs/dist/readme-*.txt`；the repo also has a signing/notarization release pipeline）」，consistent with the FAQ wording；Settings section page inventory changed to measured values（`SettingsDialog.tsx`：user-visible pages General/Account/Update/About/Providers/Models/Reset；Remote/Environment are devOnly and not documented）.

### B5. `make generate-proto` coverage missing one end —— ✅ fixed (todo_cbbb063d2fd4)

| Claim | Location | Basis |
|---|---|---|
| 「make generate-proto（agent + channels + TUI）」 | docs/build-and-install.md L186-206（B17）；zh equivalent | Actually also includes **desktop/src-tauri**（Makefile L404-410：agent → channels → desktop/src-tauri → tui） |

→ Changed to「agent + channels + GUI (src-tauri) + TUI」（zh same）.

### B6. `make lint` scope missing two ends —— ✅ fixed (todo_cbbb063d2fd4)

| Claim | Location | Basis |
|---|---|---|
| 「lint all (agent + channels + TUI + CLI + GUI)」（zh same） | docs/build-and-install.md L183, L192；zh L169, L178 | Actual = lint-agent + lint-channels + lint-tui + lint-cli + lint-desktop + **stylelint-desktop** + **lint-mobile**（Makefile L232-253） |

→ Changed to「agent, channels, TUI, CLI, GUI (+stylelint), mobile」（zh same）. Also added: `make fmt` actually = cargo fmt (agent+channels) + fmt-mobile（Makefile L262-269）；the document's original「cargo fmt (agent + channels)」was updated too.

### B7. DingTalk slash-command wording imprecise

| Claim | Location | Basis |
|---|---|---|
| 「All slash commands are handled locally by the Bridge」（implying a difference from Feishu） | docs/wiki/en/DingTalk.md L91-103；zh equivalent | Both bridges are the same: 9 commands handled locally（/new /status /stop /model /models /compact /effort /cwd /help），**unknown slash commands are forwarded to the agent as ordinary messages**（feishu/bridge.rs L714；dingtalk/bridge.rs L262-265）。Suggest unified wording |

→ ✅ **Fixed (todo_bcf715c7cc0e)**: en/zh DingTalk.md changed to match the Feishu page:「9 commands are handled locally by the Bridge；unrecognized commands are forwarded to the Agent as ordinary messages」.

### B8. macOS notarization status: the inter-document conflict is "different contexts", suggest clarifying wording

| Conflict | Location | Basis |
|---|---|---|
| 「macOS build is also notarized by Apple」 | wiki Installation.md L24（en/zh） | The official signing pipeline `.github/workflows/build-macos-signed.yml` does sign + notarize（notarytool/staple） |
| 「The current build isn't notarized」/「当前版本未公证」 | wiki FAQ.md L9（en/zh）；docs/dist/readme-macos*.txt | Regular CI（build.yml）and dist release packages are neither signed nor notarized |

Both statements are individually true（different artifacts）. Suggest Installation.md wording「official signed builds are notarized by Apple」and point to the FAQ's「current download is not notarized」note.

→ ✅ **Fixed (todo_bcf715c7cc0e)**: en/zh Installation.md changed to「the official signed release pipeline signs the macOS/Windows installers and Apple-notarizes macOS；the current download builds are neither signed nor notarized — first-launch warnings are in the FAQ」.

### B9. future-loop CLI overview presentation outdated (minor) —— ✅ fixed (todo_63c718c2a3d5)

| Claim | Location | Basis |
|---|---|---|
| CLI overview presented as `ops <cmd>`、`work-items <cmd>`、`cli registry` groups | docs/loop-control-plane.md L115-127 | Actual `future-loop` dispatches **flat top-level commands**（orchestration/loop/src/main.rs L93-137，42 top-level commands total）；`ops`/`work-items`/`cli` are only help group names in the registry（cli/registry.rs），**not runnable commands**. The listed lower-level commands（goal/todo/gate/capability/extension/handoff/benchmark protocol|run|ledger/replay record|run|corpus/canary smoke/version/doctor/history/turn/todo-event/evidence-log/backup/…）all exist ✓，only the presentation hierarchy needs correction（document L127 already has the「run with no args for full help」hint — low severity） |

→ This round（todo_63c718c2a3d5）compared against `build_cli_registry()`（main.rs L176-471）one by one and corrected the CLI overview（en/zh in lockstep）:

- **goal group missing 2 commands**: `models`（`models [--format json]`，lists models available to the agent，main.rs L1634）and `diagnose`（`diagnose --goal G [--format json]`，per-goal diagnostics surface，L3694）→ added to the goal row
- **extension missing `upgrade`**: actual surface is `install|upgrade|enable|disable|rollback|status|capabilities`（L2597 `"install" | "upgrade"` same branch）→ added to the extension row
- **`cli registry` missing `--include-experimental`**: actual `registry [--json] [--include-experimental]`（main.rs L466）→ added

All other checks passed: `todo add|claim|complete|supersede|update|archive`（L613-618）、`gate resolve`、`replan ack`、`lease`、`task-graph`、`agent onboard/scope/lane/supervisor`、`capability list|propose|commands` + `catalog`、`handoff [--write]`、`benchmark protocol|run|ledger`、`replay record|run|corpus build|run`、`canary smoke [--profile core-control-plane|extension-runtime|release-gate]`、all 19 ops-group commands、quota three sources run/agent/heartbeat（quota/slot_accounting.rs L42-44）、nine dispositions（decision/）、`--class monitor --cadence`（L655-660）、`--verify/--max-validation-attempts`（L656-657）、backup `--restore`（L991-1002）、state layout registry.json + goals/<id>/events.jsonl + ACTIVE_GOAL_STATE.md + runs/、`cargo build -p future-loop`、`scripts/install-future-loop.sh` all ✓

---

## C. Missing (implemented but undocumented, suggest filling in)

### C1. README TUI slash-command table incomplete (12/19) —— ✅ fixed (commit b3b2e114, todo_5d852f73fcb6)

README.md L105-118（zh L100-113）lists 12 commands；source（tui/src/app.ts `handleSubmit`）actually handles **19**（17 usable + 2 stubs）:

- **Implemented but not in README**: `/cwd`、`/approve`、`/reject`、`/cancel <run-id>`、`/reload`（the tui/src/app.ts L71-86 autocomplete list also includes these 16；dispatch additionally has /compact）
- stubs（answer「not available」）: `/export`、`/import`

→ Added the 5 implemented commands to README（en/zh），now listing 17/19；`/export`/`/import` are stubs and not listed.

> Also（code-side observation, not documentation）: TUI help-screen（help-screen.ts）lists only 10 commands, missing /status /stop /cwd /approve /reject /cancel /reload /compact（/compact is in help but NOT in the autocomplete list）——✅ **Fixed**（commit 042a7d07，todo_b98a9381ad9e）：help-screen now lists all 17 implemented commands，the autocomplete list adds /compact，both consistent with dispatch/README.

### C2. wiki CLI.md missing command groups

See B1: the five groups `session`、`models`、`account`、`init`、`doctor` + `auth credential` + `skills update/install-builtin` are not in the wiki CLI.md.

### C3. Missing documents filled in per the verification inventory（todo_9bb2c6dd1c38，2026-08-06）

All of fact-inventory §14b's I-1..I-7 handled:

- **I-1** Models.md regeneration note → resolved（todo_bcf715c7cc0e：`make generate-models` comment written into the script template and file headers）.
- **I-2** Linux page decision → **stays macOS+Windows**: release.yml publishes only macOS（arm64+x64 dmg/updater）+ Windows（x64 setup）；the Linux portable package is tester-only — no Linux wiki page.
- **I-3** added **docs/channels-config.md / zh**: unified reference for `~/.future/channels/config.json` — schema of the three blocks `agent`/`feishu`/`dingtalk`, all defaults and legal values（field-by-field checked against channels/src/config.rs + feishu/policy.rs；dm_policy=open|disabled|allowlist、group_policy=open|disabled|allowlist、first-run writes a template and exits、9 local slash commands、unknown slashes forwarded、Feishu keepalive 30s / DingTalk 20s、DingTalk webhook new-message replies）.
- **I-4** loop-control-plane.md / zh adds「Multi-agent workflow / 多 agent 工作流」section：complete examples for `agent`（registration + `onboard --capability`）、`scope`（identity boundary frontier output）、`lane`、`supervisor propose|receipt|events`、`handoff [--write]`（delivery contract + HANDOFF.md）、`task-graph`、`attention [--goal|--all]`、`inbox --project`；and notes these are **flat top-level commands**（main.rs L94/L119-127 top-level dispatch；the agent/todo/work-items groups in help output are display only）.
- **I-5** added **docs/README.md / zh**: docs directory navigation index（top-level guide table, wiki page inventory, dist live documents, internal working documents, how to keep documentation correct）.
- **I-6** added **docs/tui.md / zh**: TUI usage documentation — 17 usable slash commands（/export /import are stubs）、8 keyboard shortcuts、settings.json/keybindings.json/debug.log/write.log paths（`write.log` written only when `PI_TUI_WRITE_LOG=1`，see tui/src/tui.ts L282-287；no crash.log）、common troubleshooting（per tui/src/app.ts etc）.
- **I-7** added **docs/directory-layout.md / zh**: complete `~/.future/` layout — agent（settings/models/auth/sessions/skills/logs）、channels、tui、app（app.db/images/review）、workspaces/chat、loop（FUTURE_LOOP_ROOT overridable）、bin；plus notes on `~/.agents/skills/` and project-local `.future/`（loop、approval_rule.json）.

> Cross-link closure: docs/README.md（index）⇄ tui.md ⇄ directory-layout.md ⇄ channels-config.md ⇄ loop-control-plane.md ⇄ wiki pages，no dangling links（`grep -rn '](…' docs/*.md` spot-check passed）.

---

## D. Verified correct (for later todos to skip, not re-checked)

| Check area | Conclusion |
|---|---|
| TUI 8 keyboard shortcuts | ✓ README L122-131 exactly matches source: ctrl+c interrupt/quit、ctrl+p cycle models、ctrl+r browse sessions、ctrl+t cycle thinking、tab complete、enter submit/accept、escape close overlay、↑↓ scroll（tui/src/app.ts L226-232；help-screen.ts same） |
| agent port | ✓ default `127.0.0.1:50051`（agent/src/main.rs L14）；CLI/TUI/channels default connections match |
| config paths | ✓ `~/.future/agent/auth.json` format `{"provider":{"type":"api_key","key":…}}` + optional `baseUrl`（agent/src/auth/mod.rs serde）；`~/.future/agent/models.json` `providers{apiKey,baseUrl,models[{id,name,contextWindow}]}`（agent/src/models/mod.rs L402-432）；`~/.future/channels/config.json`（channels/src/config.rs default_path）；TUI local `~/.future/tui/settings.json`；loop state root `~/.future/loop/`（FUTURE_LOOP_ROOT overridable） |
| channels config defaults | ✓ grpc_addr=http://127.0.0.1:50051、model=future/deepseek-v4-pro、thinking_level=xhigh、permission_level=all；feishu dm_policy=allowlist、group_policy=disabled、require_mention/streaming/resolve_sender_names=true、max_image_mb=10、typing_indicator=false；dingtalk domain=api.dingtalk.com；no config.json → writes template and exits（config.rs all defaults + main.rs behavior） |
| channels slash commands | ✓ 9 per bridge: /new /status /stop /model /models /compact /effort /cwd /help（feishu/bridge.rs L424-714；dingtalk/bridge.rs L141-262） |
| DingTalk keepalive 20s | ✓ PING_INTERVAL_SECS=20（dingtalk/dingtalk_ws.rs L32） |
| toolchain versions | ✓ rust-toolchain.toml `1.97.0`；.nvmrc `24`；.cargo/config.toml：mold only on x86_64-unknown-linux-gnu、windows-msvc /DEBUG:NONE（「mold required on x86_64、not needed on ARM Linux」correct） |
| remaining Makefile targets | ✓ install/install-desktop/install-agent|tui|cli|desktop|channels|skills|loop、uninstall、build*（agent/tui/cli/desktop/desktop-dist/channels/mobile）、package-desktop、run-agent|tui|cli|desktop|channels、generate-models、generate-proto、fmt、clean all exist；install prefix per-OS correct（macOS /opt/homebrew/bin、Linux /usr/local/bin sudo、Windows %USERPROFILE%\.future\bin）；install-skills symlinks/Windows copies；install-loop → scripts/install-future-loop.sh；scripts/（build-desktop-macos.sh、build-desktop-windows-portable.ps1、build-desktop-windows-installer.ps1、start-desktop-windows.bat）all exist |
| build-desktop-macos.sh | ✓ auto-signs with the unique Developer ID、`--identity`/`--out-dir`/`--notary-profile` options match B4's description |
| CLI run options | ✓ --model supports `model:thinking`、--thinking off/minimal/low/medium/high/xhigh、@<path> file inclusion、--continue/-c、--cwd、--mode text|json、--no-session（cli/src/commands/run.ts help text） |
| CLI tools | ✓ list / describe / call（--key value、--args '<json>'、--stdin、--output、--timeout etc） |
| `future init` | ✓ = installs builtin skills + links future/future-agent into ~/.future/bin on macOS/Linux（cli/src/index.ts L41-46 help text）— build-and-install B14 description correct |
| future-loop command surface | ✓ 42 top-level commands（main.rs L93-137）；goal init/cancel/delete（L494-586）、todo add/claim/complete/supersede/update/archive（L613-618）、gate、capability/catalog、extension install|upgrade|enable|disable|rollback|status|capabilities（L2593-2670）、handoff（L2958）、benchmark protocol|run|ledger（L3393-3395）、replay record|run|corpus build|run（L3610-3614）、canary smoke --profile（L3631-）、run --goal/--model/--thinking-level/--max-turns（L375）、todo add --verify/--max-validation-attempts（L656-657）all exist |
| install-future-loop.sh | ✓ CLI → ~/.local/bin/future-loop、skill → ~/.future/agent/skills/future-loop/SKILL.md、`future-loop status` verification |
| proto | ✓ proto/future.proto exists；generated code committed（agent/src/grpc/generated/proto.rs、channels/src/generated/proto.rs）；make generate-proto regenerates agent+channels+desktop/src-tauri+tui |

---

## E. Leftovers / out of this todo's scope (suggested for follow-up verification)

- **GUI feature claims**（wiki Using-FutureOS/Settings/Skills/Quick-Start + Home's Artifacts mention）—— ✅ verified（todo_cab9a84ced24，2026-08-06），see §G. Found and fixed 6 items: Artifacts panel disabled（→Files view）、right-column view table、builtin skills table（3 nonexistent skills → actual 14）、Settings General missing Auto-upgrade skills、FutureGene「Connect」→「Sign in」、builtin provider「Set key/Update key」→「Configure」.
- **X10 architecture-audit timeliness —— ✅ adjudicated（todo_41f779819879）**: architecture-audit ruled a **point-in-time snapshot, marked historical, not re-verified line by line**. The audit documents entered the repo in the same batch as the first fixes（commit `306cf05f`，2026-08-06）：report 01 H2/H3/H4、report 04 H1/H2/H4 were fixed in that same commit（code comments cite audit numbers directly，e.g. auth_store.rs「audit item 2」、useThreadStore.ts「(H2)」）；still-standing high-risk items are report 01 H1/H5、report 04 H3. file:line has drifted（report 02's referenced files moved in several places、report 03 line counts grew、report 01's proto line numbers are invalid）；README.md top now has a timeliness note with per-item ✅ marks. Any future audit update should re-run the investigation against the current worktree rather than revising old line numbers.
- **docs/dist/*.txt —— ✅ verified（todo_41f779819879）**: the 6 release-package readmes are **live documents**（build.yml / build-desktop-windows-portable.ps1 / build-windows-signed.yml copy them verbatim into dmg/zip/tar.gz at packaging time），all statements match current source（binary trio `futureos`/`future-agent`/`future` matches build.yml assembly steps exactly；macOS「not notarized」consistent with FAQ/B8；WebView2/WebKitGTK runtime requirements correct）。**No changes needed**；`-en.txt` are reference translations（packaging uses only the zh `.txt`）. Follow-up todos can skip.
- **X8 sandbox terminology**: README「off / manual / macOS Seatbelt」vs wiki「Manual / Sandboxed (macOS only) / Unrestricted」— belongs to the GUI verification area.
- TUI help-screen / autocomplete list inconsistency with the implemented command set（small code-side issue, see C1 note）—— ✅ fixed（commit 042a7d07，todo_b98a9381ad9e）.

---

## F. Final re-verification results（todo_567768e616f3，2026-08-06）

> A closing full re-check of docs/ after all fix commits; all three checks passed:

### F1. zh/en consistency

- Paired documents' heading structures compared one by one（excluding shell comment lines inside code fences）: README（10/10）、docs/README（6/6）、tui（5/5）、directory-layout（9/9）、channels-config（8/8）、loop-control-plane（**19/19**，h1/h2/h3 all equal；the initial 15 vs 14 difference came from `#` comment lines inside code blocks, not headings）、build-and-install（19/19）、wiki-prompt（23/23）. All consistent.
- Key-fact spot checks consistent: TUI slash-command tables en/zh both 17 usable + 2 stubs（/export /import）；shortcuts 4 items（ctrl+p/t/r/c）verbatim equal；channels-config default-value tables en/zh numbers equal（50051、deepseek-v4-pro、xhigh、10MiB、30s/20s）.

### F2. Link validity

- Full link check（16 md files: docs/*.md + README*），0 broken links. The checker strips inline code and code fences first（wiki-prompt's `[x](Quick-Start)` **syntax examples** are inside backticks, not real links；Quick-Start.md actually exists in both en/zh directories）.
- wiki cross-links + anchor checks（en/zh 13 pages each，including `#anchor` resolution to GitHub slugs），0 errors.
- New-document cross-reference closure（docs/README ⇄ tui ⇄ directory-layout ⇄ channels-config ⇄ loop-control-plane ⇄ wiki）all reachable.

### F3. Change list（7383eca1..HEAD，15 commits，all pushed to the branch）

| commit | Content |
|---|---|
| 05a9a575 | README en/zh full re-check completion marker（B3 wrap-up） |
| dd14b60a | loop skill：long turns exceed the default shell timeout — blocking run + explicit timeout |
| 9cac0b48 | build-and-install en/zh：make test/lint/fmt/generate-proto scope + skill count 14（A5/B2/B5/B6） |
| 3a587ad4 | loop-control-plane en/zh：CLI overview aligned with the actual command surface（B9：goal adds models/diagnose、extension adds upgrade、cli registry adds --include-experimental） |
| 9512b972 | wiki-prompt en/zh：CLI command surface/binary name/packaging/inventory fully aligned（A1/A2/B4 + Models.md generation note） |
| c7c95d03 | all wiki pages en/zh：CLI.md rewritten、Feishu/DingTalk（A3/A4/B7）、Installation（B8）、Settings page counts、Models.md generated-header note |
| fff6c18b | architecture-audit marked as point-in-time snapshot + dist readmes verified（X10） |
| 8fe97b57 | loop code fix：streaming text UTF-8 boundary truncation（panic fix） |
| b19bf933 | loop skill：explicit「reflect and improve」step after every turn（superagent proactive replanning） |
| 950039f1 | **missing documents filled in（C3/I-1..I-7）**: docs/README（index）、tui、directory-layout、channels-config、loop-control-plane multi-agent section |
| a2b7105c | tui/directory-layout en/zh：crash.log fact correction → write.log（written only when PI_TUI_WRITE_LOG=1），4 files |

Leftovers（see §E）: GUI feature-claim verification（Using-FutureOS/Settings/Skills/Quick-Start page content，✅ verified todo_cab9a84ced24）、TUI help-screen vs completion-list inconsistency（✅ fixed commit 042a7d07，todo_b98a9381ad9e）.

## Appendix: verification method

- Claim→source one-by-one comparison；source evidence includes file:line（see each item's「Basis」column）.
- CLI removal history confirmed via `git show eed93369`（2026-07-16，deleted agent.ts/channel.ts/gui.ts/tui.ts，665 lines total）.
- Generated file Models.md numbers taken from the file header（3826/143），without re-running `make generate-models`（needs network）.

---

## G. GUI verification results（todo_cab9a84ced24，2026-08-06）

> Verified the four GUI wiki pages（Using-FutureOS / Settings / Skills / Quick-Start）+ Home's Artifacts mention against desktop/src-tauri（Rust commands）+ desktop/src（React）. en/zh changed in pairs, heading structure kept aligned.

### G1. Fixed (6 items)

| Item | Correction | Source basis |
|---|---|---|
| Artifacts panel disabled | Using-FutureOS / Quick-Start / Home en+zh：removed「Chat right column = Runs + Artifacts」「Artifacts collect outputs (preview/copy/export/upload)」etc，replaced with the **Files** view（every session has a Files tab：Chat=temp session folder，Workspace=project folder；preview/system-open/attach-from-directory-tree to a conversation） | ContextPanel.tsx L34-57：Artifacts tab commented out of fileTabs/gitTabs（commit 9756a7b2，2026-07-14「hide the Artifacts tab pending a decision on its purpose」）；chat=files+runs，workspace=files+runs+review；`isFutureReferenceType()` always false → futureos:// app object references are all dead（parseFutureMarkdown.ts L427-438） |
| right-column view table / three-column description | 「Runs / Review / Artifacts」→「Files / Runs / Review」；the Chat vs Workspace table right column changed to Files+Runs / Files+Runs+Review | same as above |
| Skills builtin skills table | removed 3 nonexistent skills（Hand-drawn posters、Hand-drawn slides、Subagent），changed to the actual **14** builtin skills: Account/Browser/Database lookup/Deep research/Document/Experimental design/Image/Paper/Peer review/Scientific writing/Skill creator/Slides/Software install/Web | online catalog `/client/v1/skills`（2026-08-06 real pull from test.future-os.cn，14 of 139 skills are `future-*`）；repo `skills/builtin/` same 14 directories；`future init`/install-builtin filter by the `future-` prefix（cli/src/commands/skills.ts L246） |
| Settings General missing「Auto-upgrade skills」 | added：the app silently upgrades installed skills to the latest on every open（default on） | GeneralPage.tsx autoUpgradeSkills Switch；app_settings.rs `unwrap_or(true)`（the struct comment said「Off by default」，contradicting the implementation — **corrected to On by default**，commit 042a7d07） |
| FutureGene button name | 「Click **Connect**」→「Click **Sign in**」；removed the nonexistent「Sign in again」（logged-in state only has Sign out） | ProvidersPage.tsx：connect button = t("providers.connect") = "Sign in"（settings.json）；triggers show-onboarding（device-code OAuth） |
| builtin provider button name | 「Set key / Update key」→「**Configure**」（the dialog opened is titled Set <provider> key） | ProvidersPage.tsx t("providers.set") = "Configure"；keyDialogTitle = "Set {{provider}} key" |

### G2. Verified correct (for later todos to skip, not re-checked)

| Check area | Conclusion |
|---|---|
| approval mechanism | ✓ trigger surface: file read/write / shell commands / writes outside the workspace（agent/src/rpc/approval.rs approval_shape：file_read/file_write/outside_workspace_write/shell_command/sandbox_escalation；file deletion goes through shell `rm` approval，the GUI ActionDetails deletes branch is defensive rendering）；cards have no timeout（GUI has no timeout，agent-side pending approvals survive restarts）；Allow once / Deny / Allow in this workspace-chat（rule paths editable；secret files get no rule button save_suggestion=None）；Cmd/Ctrl+Enter approves、Esc rejects（closes the rule editor first if open）（ApprovalPrompt.tsx） |
| approval three modes | ✓ manual/sandbox/off verbatim equal to the i18n copy：Manual=asks before reads/writes、read-only commands run automatically；Sandboxed=macOS only（GeneralPage renders the option only when `isMacOS`）、commands enter the macOS sandbox、file operations still ask；Unrestricted=no asking no sandbox everything runs；default off（app_settings.rs normalize_tier + read default） |
| composer | ✓ model selector + thinking level + shield（approval mode，ShieldCheck/Off/Question three-state icon）+ paperclip/paste/drag attachments + send key becomes stop while streaming（Composer.tsx） |
| 4 images / 25MiB | ✓ MAX_IMAGES_PER_TURN=4、READ_SOURCE_MAX_BYTES=25*1024*1024、non-image files unlimited（attachments.ts L11-19）；backend MAX_ATTACHMENT_IMAGE_BYTES=25*1024*1024（commands/files.rs L114） |
| left column structure | ✓ New Chat / Models(settings shortcut) / Skills / Workspaces(each expandable into sessions、collapsible、+ new) / Chats / Settings at bottom；left column collapsible（ActivityRail.tsx；layout.json） |
| Runs panel | ✓ cards show real commands（shell lines verbatim）、tool status、running/finished counts；Inspect / Terminate / Clear finished（RunsPanel.tsx + runs.json） |
| Review panel | ✓ file list + change type (added/modified/deleted/renamed) + per-file diff；git workspaces have the「Last run changes」view toggle（non-git shows only the last-run single view）；default view from backend capabilities.defaultView（ReviewPanel.tsx + review.json） |
| session menu | ✓ rename / pin(unpin) / delete（ThreadListItem.tsx + layout.json rename/pin/delete） |
| Settings tabs | ✓ release builds have only General/Providers/Models/Account/Check for updates/About/Reset；Remote、Environment are devOnly（SettingsDialog.tsx NAV_GROUPS + `devOnly`）— not mentioning them in docs is correct |
| FutureGene auth flow | ✓ device-code OAuth：browser authorization，when not auto-opened shows the verification code (user_code) + copyable link (verification_uri_complete)（future_login.rs） |
| custom provider | ✓ Name(optional)/Provider ID (lowercase alphanumeric `-`_，length+pattern validation)/API type (OpenAI Completions|Responses|Anthropic)/Base URL (http/https validation)/API Key/Models；ID uniqueness check（CustomProviderDialog.tsx + settings.json validation copy idPattern/idLength/baseUrlInvalid） |
| Models page | ✓ grouped by provider、search (model or provider name)、visibility switch → hidden models removed from the selector（ModelsPage.tsx hiddenModels + providerNames） |
| update / account / reset | ✓ check updates→download→install→restart（UpdatePage + install_app_update/restart_after_app_update）；account page profile+balance+top-up（AccountPage + useFutureAccount）；reset=clear local data and restart（ResetPage clear_app_data） |
| Skills page | ✓ Installed/All two tabs、category dropdown+search+result count+clear filter、Install/Uninstall(confirm)/Upgrade/Upgrade all；first time with no installed skills auto-switches to All；「All needs network」correct（SkillsView.tsx + skillsClient） |

### G3. Change list（this todo，one commit `9f68ed0e`，14 files）

| Files (en+zh pairs，7 pairs) | Content |
|---|---|
| Using-FutureOS | Artifacts→Files（three-column description、Chat vs Workspace table、whole Artifacts subsection replaced） |
| Quick-Start | step 5「up to three views」rewritten to Files/Runs(+Review)；FutureGene「Connect」→「Sign in」 |
| Skills | builtin skills table 11 → actual 14（removed 3 nonexistent、added 6 missing、Slides name corrected） |
| Settings | General adds Auto-upgrade skills；FutureGene「Connect」→「Sign in」（removed Sign in again）；builtin provider「Set key/Update key」→「Configure」 |
| Home | 「generated outputs (Artifacts)」→ Files/Runs/Review wording |
| FAQ | 「FutureGene → Connect」→「FutureGene → Sign in」（EF5） |

The verification process is also recorded in fact-inventory.md：EH3/EQ1/EQ5/EU1/EU2/EU9/ES3/ES4/ESK3/EF5 updated；§14a H14/H15 closed.

---

## H. 2026-08-18 re-check: loop docs, skill count, directory layout（branch `docs/claude-md-improvements`）

> Checked item by item against the current worktree source（the loop control plane has deleted the capability/extension ecosystem and the default state root is now the project-local `<cwd>/.future/loop/`；skills/builtin has 15 future-* skills）.

### H1. Fixed (this round)

| Item | Correction | Source basis |
|---|---|---|
| loop doc「three todo categories」outdated | five categories: advancement / user-gate / user-action / monitor / blocker（en/zh core-concepts table） | state.rs `TaskClass` enum 5 variants；console.rs L824 `--role/--class` validation copy |
| loop doc「PLAN_REVIEW checkpoints are agent-resolved」has no source basis | deleted；changed to user-action semantics（`user_action` is shown to the user but does **not** freeze the agent，decision/mod.rs L111-114 comment + UserChannel branch） | whole-repo grep `PLAN_REVIEW` 0 hits |
| loop doc「three-end experience」（TUI/GUI/mobile show loop state）contradicts code | changed to「loop state is CLI-first」：state is project-local、TUI/GUI/mobile/IM have no built-in loop view、driven via the `/future-loop` skill orchestrating the `future loop` command | desktop/src、mobile/src、tui/src have no future_loop references（grep 0 hits）；loop state readable only via CLI |
| loop doc「channel-bridge messages can trigger loop operations、gates reply into chat」has no integration code | changed to「any client can drive it via the skill；bridge and loop have no native integration」 | channels/src has no loop integration（grep 0 hits） |
| loop doc CLI group inventory outdated | corrected against actual `future loop registry` output：agent group adds `list`、ops group adds `serve-status`、removed「quality group」replaced by handoff / cli / benchmark / replay / canary five groups（10 groups 43 commands count unchanged） | `./target/debug/future loop registry` actual output；cli/registry.rs |
| loop doc gate example missing required `--text` | added `--text`（`--gate-question` defaults to `--text`） | console.rs `--text required` |
| directory-layout claims loop state root defaults to `~/.future/loop/` | actual default is `<cwd>/.future/loop/`（project-local），`FUTURE_LOOP_ROOT` overrides；`~/.future/loop/` no longer used. Section title and body changed、`loop/` row removed from the `~/.future/` tree | console.rs `root_dir()` L56-66（no home fallback） |
| console.rs module comment「State lives under `--root` (default `~/.future/loop/`)」 | changed to project-local `<cwd>/.future/loop/`（the `--root` flag also does not exist） | console.rs `root_dir()` |
| build-and-install「install all future-* skills (14)」 | → 15（skills/builtin now includes future-loop） | `ls skills/builtin/` 15 directories |
| build-and-install「development (from source)」section duplicated twice（the second is the outdated version: make test=cargo test (agent)、lint missing GUI/mobile） | deleted the outdated copy；`make fmt` comment changed to `cargo fmt --all (workspace) + desktop/src-tauri + mobile`（en/zh） | Makefile `fmt:` = cargo fmt --all + src-tauri + fmt-mobile |
| wiki Skills.md builtin table missing Loop（14 items） | added the **Loop** row（en/zh），skill total 15 | skills/builtin/future-loop（v3.0.2） |
| wiki-prompt builtin-skill example list contains 3 nonexistent skills（Hand-drawn posters/slides、Subagent） | replaced with the actual 15（including Loop） | skills/builtin/ directory actual |
| README「14+ skills」、docs/README loop row「extensions」 | README → 15+ and adds the `/future-loop` example；docs/README loop row「扩展与多 agent」→「交付闭环、多 agent」（extension/capability commands deleted，`future loop extension status` reports unknown command） | actual CLI 0 hits for extension/capability |

### H2. Verified correct (for later todos to skip)

- loop registry「10 groups 43 commands」、`quota should-run/usage/spend/decisions`、`scheduler tick|show|record-host-failure|ack|liveness`、`delivery status|record|followthrough`、`agent onboard|list|contract|recipe|succession|collective`、`lease claim|renew|release|expire|status`、`frontier show`、`run --goal/--agent-id/--model/--thinking-level/--max-turns` all match the actual registry output.
- semantic history N=50（goal_frontier/semantic_history.rs `SEMANTIC_HISTORY_CAP`）；delivery 3-turn unvalidated derived follow-through（work_items/delivery_outcome.rs）；1-hour idle accounting（state.rs `TURN_NO_PROGRESS_IDLE_SECS_DEFAULT = 60 * 60`）；lease pid liveness reclamation（state.rs claim → compat::pid_alive）.
- channels-config.md all fields and defaults ↔ channels/src/config.rs field-by-field consistent.
- TUI 19 slash commands（17 usable + /export /import stubs）consistent with tui.md/README；9 keyboard shortcuts consistent with help_screen.rs；README sandbox three tiers consistent with the future.proto L251 comment；Models.md 3826/143 consistent with README「3800+/140+」；`future init` linking future + future-agent into ~/.future/bin consistent with init.rs.
- wiki CLI.md command groups consistent with actual `future --help`（init/auth/account/run/skills/tools/models/session/doctor + agent/tui/channel/loop）.

### H3. Skills-repo leftover → resolved（future-skills#14，v3.0.3）

- the skills submodule's future-loop/SKILL.md「PLAN_REVIEW checkpoints are agent-resolved」
  was fixed upstream（future-skills#14，squash-merged，b045032），the same batch also fixed
  `agent register`（actually `onboard|list|contract|recipe|succession|collective`）
  and `canary [--premerge]`（actually `canary smoke [--profile …] | canary premerge`）；
  version 3.0.2 → 3.0.3. The main-repo pointer bump is committed with this round.

---

## I. 2026-08-26 re-check: all documentation（worktree `claude/docs-optimization`）

> Verified item by item against the current worktree（HEAD `41a02ac4`）: root README/SECURITY/CLAUDE、
> docs/ guides and wiki、docs/dist、module READMEs、desktop/CLAUDE and DEV_MD.
> Note: **section H (2026-08-18)「10 groups 43 commands」is outdated** — the loop control plane
> further slimmed its CLI surface after the capability/extension ecosystem deletion.

### I1. Fixed (this round)

| Item | Correction | Source basis |
|---|---|---|
| README/SECURITY sandbox three-tier names | 「off / manual / macOS Seatbelt」→「`off` / `manual` / `sandbox`」；SECURITY known-gaps updated（Windows now has a restricted-token sandbox、Linux has no sandbox） | agent/src/sandbox/mod.rs `SandboxTier{Off,Manual,Sandbox}`；`platform_sandbox_availability()` includes a windows probe；agent/src/sandbox/windows/ directory exists（#342） |
| CLAUDE.md local main branch name | `dev` → `main`（`git branch` actual output only has `main`） | `git branch` output |
| loop-control-plane CLI panorama | 「10 groups 43 commands」→「7 groups 40 commands」；removed handoff/benchmark/replay groups and serve-status；agent group removed top-level `list`（actually an `agent list` subcommand）、adds `worker`；goal group adds `ui` | `future loop registry` actual：goal5/todo6/agent5/ops18/work-items3/cli2/canary1 |
| tui/directory-layout debug.log | 「always written」→ written only when `PI_DEBUG_REDRAW=1` | tui/src/app.rs `log_redraw`（returns when `PI_DEBUG_REDRAW` != "1"） |
| long-run-evidence-ledger crate name | `future-channels` → `future-channel`（4 places en + 4 places zh） | channels/Cargo.toml `name = "future-channel"`（this name since PR #150） |
| orchestration/loop/UPSTREAM.md | removed the deleted「explore graph, pr_review_queue」 | loop/src whole-repo grep 0 hits（capability ecosystem deleted） |
| packages/rpc/README proto path | `../proto/future.proto` → `proto/future.proto` | packages/rpc/build.rs `rerun-if-changed=proto/future.proto` |
| desktop/CLAUDE.md | ①sandbox「(macOS only)」removed（Windows supported now）②dangling `DEV_MD/PLAN.md` → `agent_providers/validate.rs` ③`agent_providers.rs` → `agent_providers/` directory ④doc-map sizes + plan docs completed | GeneralPage.tsx `useSandboxAvailability`+`isWindows`；agent_providers/ directory；DEV_MD has no PLAN.md |

### I2. Verified correct (for later todos to skip)

- README：3826 models / 143 providers（agent/src/models/builtin/models.json）；15 builtin skills（skills/builtin/）；toolset = read/write/edit/shell（agent/src/tools/mod.rs `all_tools==coding_tools`）；17 slash commands + 9 shortcuts（tui/src/help_screen.rs）；`future skills list/install/install-builtin/uninstall/update`；`future auth login`.
- SECURITY four-tool toolset、trust model、reporting process consistent with source.
- CLAUDE.md：workspace members、packages npm、package.json workspaces、proto generated-file paths、API key resolution order（agent/src/rpc/session.rs `resolve_api_key` model→provider→model_key→default_key）、Feishu URL/60s stale filtering、`make lint-rust` == CI clippy flags — all correct.
- channels-config.md all fields/defaults consistent with channels/src/config.rs；9 slash commands per bridge；Feishu 30s / DingTalk 20s keepalive（feishu_ws.rs `DEFAULT_PING_INTERVAL=30`、dingtalk_ws.rs `PING_INTERVAL_SECS=20`）.
- build-and-install：rust-toolchain 1.97.0、.nvmrc 24、make targets/scripts exist、7 test suites.
- wiki 13 pages（en+zh）command groups/skill counts/model counts/Feishu-DingTalk wording consistent with `future --help`、group help and source；no stale command references.
- dist 6 readmes、wiki-prompt（15 skills、no Hand-drawn/Subagent、`future` not `future-cli`）accurate.
- module READMEs：mobile（bundleIdentifier `cn.futureos.mobile`、deploymentTarget 16.4）、packages（4 packages）、loop README/NOTICE、tui tests（golden-diff + tmux-diff harnesses）、tui/RESEARCH（P0/P1 decision log）accurate.
- DEV_MD：CONNECTION.md support codes consistent with remote/mod.rs；COLOR.md tokens consistent with tailwind.config.js；plan docs（SANDBOX/{COMMON,MACOS,LINUX,WINDOWS}/CONTEXT_COMPACTION/PRODUCT/ER）marked plan-vs-current per convention and not rewritten.

### I3. Scope notes (this round)

- **Excluded skills/ submodule**（git submodule，the future-skills repo，2000+ files）.
- **Excluded generated/state files**: Models.md（`make generate-models`）、THIRD_PARTY_NOTICES、
  `.future/memory/`、coverage/.
- **desktop/DEV_MD** got a「lightweight pass」per the user's decision — only fixed the CLAUDE.md doc map and dangling references、
  marked plan-vs-current，no line-by-line rewriting of planning documents.
- bilingual lockstep: all paired documents（README、docs/ guides、wiki、ledger）changed en/zh together.
