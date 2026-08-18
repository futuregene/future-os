# 文档核验结果：错误 / 过时 / 缺失清单

> **内部工作文档**（非用户文档）。产出自 todo_8bd559237c7d「对照源码核验文档事实清单」。
> 核验对象：README + `docs/` 下所有文档的关键事实声明，对照当前工作树源码
> （branch `claude/loop-hardening`，HEAD `3ed92ad7` 之后）。
> 判定标准：文档声明 vs 源码实际行为。行号以 2026-08-06 工作树为准。

---

## A. 错误（文档表述与源码不符，应修正）

### A1. CLI 二进制名：`future`，不是 `future-cli`

| 声明 | 位置 | 依据 |
|---|---|---|
| 「The release binary is named **`future-cli`**…user-facing wiki must always use `future-cli`, never `future`」 | docs/wiki-prompt-en.md L127, L170-171, L173-174, L219 | `cli/package.json` `bin: {"future": "dist/index.js"}`；Makefile `build-cli` → `bun build --compile dist/index.js --outfile dist/future`（Makefile L159）；Tauri sidecar 名 `future-<triple>`（desktop/src-tauri/tauri.conf.json externalBin）。**release 二进制就是 `future`**。zh 版 wiki-prompt.md L171/L219 的 `future` 禁令才是对的 |

→ ✅ **已修复（todo_285b37996a0d，commit 见下）**：wiki-prompt-en.md 全部 `future-cli` 改为 `future`（§6 表格/侧边栏、§7 CLI 定位/位置/运行/命令组/小贴士、§9 A.4 自检项），与 zh 版一致。

→ ✅ **wiki 页面也已修复（todo_bcf715c7cc0e，commit 见下）**：en/zh 的 CLI.md 重写（Windows 行改为「安装版与便携版都带」、删去「仅便携包」注释、agent 节改为「CLI 不能启停 agent」、命令组全量对齐 index.ts）、Feishu.md/DingTalk.md 的「启动 Bridge」节改为 `make build-channels` + `./target/release/future-channel` + 「无 future channel 命令」说明、排障里 `future channel status` 改为检查 `future-channel` 进程。

### A2. `future agent start/stop/restart` 与 `future channel …` 已从 CLI 移除（2026-07-16）

commit `eed93369`（2026-07-16）`refactor(cli): remove service management and launcher commands` 删除了 `future agent *`、`future channel *`、`future gui`、`future tui`。现状：`future agent` 只有 `status`；**没有 `channel` 命令组**。用户按 `make install` 后直接运行 `future-agent` / `future-channel` 二进制。

受影响位置（en/zh 均需改）：

| 声明 | 位置 |
|---|---|
| `future agent start`（agent 必须运行） | docs/wiki/{en,zh}/CLI.md L41, L59-61, L119；docs/wiki/{en,zh}/Feishu.md L22；docs/wiki/{en,zh}/DingTalk.md L22 |
| `future channel start/status/stop/restart`（服务管理） | docs/wiki/{en,zh}/Feishu.md L144-147(en)/L145-148(zh)；docs/wiki/{en,zh}/DingTalk.md L96-99；docs/wiki/{en,zh}/Feishu.md L185(en)/L186(zh)、DingTalk.md L151（排障「future channel status」） |
| wiki-prompt 命令组描述 | docs/wiki-prompt.md L177, L183, L185；docs/wiki-prompt-en.md L177, L183, L185 |

→ ✅ **wiki-prompt 部分已修复（todo_285b37996a0d）**：两个 prompt 的 CLI 节全面对齐 `cli/src/index.ts` 实际命令面——`agent` 组仅 `status`（无 start/stop，CLI 不能启停 agent，已注明）、删除不存在的 `channel` 组、skills 补 `install-builtin`/`update`、auth 补 `credential`、补齐 `init`/`account`/`models`/`session`/`doctor` 组、tools 补 `describe` 与 `--input` 等旗标、run 补 `--fork`/`--session`/`--permission`；同时修正「agent 必须在运行」与 FAQ 排障里的 `future agent start`（改为打开桌面应用或手动运行 `future-agent`）。

⚠️ **wiki 页面本身（docs/wiki/{en,zh}/CLI.md、Feishu.md、DingTalk.md）仍待后继 todo 修复**（下述 A3/A4/B1/B7 与 Feishu/DingTalk 页内 `future channel *`、`build-channels-release` 等）。另：本次发现 Windows 安装版也带 CLI——build.yml 把 `future.exe` 复制为 Tauri sidecar `binaries/future-<triple>.exe` 打进 NSIS 安装器，故「CLI 只在便携包」的旧表述（含 wiki CLI.md 表格下注释）应改为「安装版与便携版都带」。

改法建议：改为「启动组件：`future-agent`（agent）、`future-channels`（渠道桥）；服务管理不再由 CLI 提供，桌面应用会自动拉起 agent」等（对应移除 commit 的意图）。

→ ✅ **已修复（todo_bcf715c7cc0e）**：en/zh 的 CLI.md/Feishu.md/DingTalk.md 均已改为上述口径（CLI 不能启停 agent；渠道桥是独立服务 `future-channel`，无 `future channel` 命令）。

### A3. `make build-channels-release` 不存在

| 声明 | 位置 | 依据 |
|---|---|---|
| 启动渠道桥用 `make build-channels-release` | docs/wiki/{en,zh}/Feishu.md L137(en)/L138(zh)、DingTalk.md L89(en/zh) | Makefile 无此目标（grep 0 命中）。正确目标：**`make build-channels`**（Makefile L186） |

→ ✅ **已修复（todo_bcf715c7cc0e）**：en/zh 均改为 `make build-channels` + `./target/release/future-channel`（顺带修正了二进制名——channels/Cargo.toml 为 `future-channel`，非 `future-channels`）。

### A4. Feishu「每 6 分钟左右重连」无源码依据

| 声明 | 位置 | 依据 |
|---|---|---|
| 「Bridge reconnects every ~6 minutes」/「每 6 分钟左右重连」 | docs/wiki/en/Feishu.md L190；zh L191 | channels/src/feishu/feishu_ws.rs：`DEFAULT_PING_INTERVAL=30`（keepalive ping 30s，wiki 这句对）、`HEARTBEAT_TIMEOUT=120`；mod.rs 重连等待 **5s**（`Duration::from_secs(5)`）。全文无 6 分钟常量。→ ✅ **已修复（todo_bcf715c7cc0e）**：en/zh 的 Feishu.md「Bridge 每 6 分钟左右重连」改为「Bridge 自动重连」——keepalive ping 30s（feishu_ws.rs DEFAULT_PING_INTERVAL=30）、断线 5s 重连（mod.rs L66）。 |

### A5. `make test` 并不跑 loop 控制面测试 —— ✅ 已修复（todo_cbbb063d2fd4）

| 声明 | 位置 | 依据 |
|---|---|---|
| `make test # cargo test (agent + loop control plane)`（zh：「agent + loop 控制面」） | docs/build-and-install.md L182；zh L168 | Makefile `test:` = test-agent + test-channels + test-cli + test-tui + test-desktop + test-desktop-rust + test-mobile（Makefile L203-229）。**没有 test-loop 目标**，loop 不在 make test 内 |
| （同文件开发节）`make test # cargo test (agent)` | docs/build-and-install.md L194；zh L180 | 同上，实际 7 个套件 |

→ 两处均改为「all 7 suites: agent, channels, CLI, TUI, GUI, GUI Rust, mobile」（zh 同）。`make clean`（删构建产物+已安装二进制）、`future init`（装技能+macOS/Linux 链接本地命令）、mold（仅 x86_64-linux）、loop 为 workspace 成员（Cargo.toml L17）等声明复核无误。

---

## B. 过时（功能已演进 / 数量已变化，应更新）

### B1. wiki CLI.md 命令组表过时（缺新组、多已删组）

现状（cli/src/index.ts + commands/*.ts）：`init`、`auth`（login/status/**credential**/logout）、`account`（profile/balance）、`run`、`skills`（list/install/install-builtin/uninstall/**update**）、`tools`（list/describe/call）、`models`、`agent`（**仅 status**）、`session`（list/info/rename/delete）、`doctor`。

| 问题 | 位置 |
|---|---|
| agent 组写了 start/stop/restart（已删） | docs/wiki/{en,zh}/CLI.md L59-61 |
| channel 组整组不存在 | docs/wiki/{en,zh}/CLI.md L84-93（en）/对应 zh 段 |
| skills 缺 `update`、`install-builtin`（`update` 已实现于 cli/src/commands/skills.ts L48） | docs/wiki/{en,zh}/CLI.md 技能段（en L106 附近） |
| auth 缺 `credential` | docs/wiki/{en,zh}/CLI.md auth 段 |
| 缺失整组：`session`、`models`、`account`、`init`、`doctor` | docs/wiki/{en,zh}/CLI.md |

→ ✅ **已修复（todo_bcf715c7cc0e）**：en/zh CLI.md 已重写，命令组 = init / auth（含 credential）/ account / run（补 --fork/--session/--permission）/ skills（补 install-builtin/update）/ tools（补 describe）/ models / agent（仅 status）/ session / doctor；`channel` 组删除。

### B2. 「future skills install 约 13 个」→ 实际 14 个 —— ✅ 已修复（todo_cbbb063d2fd4）

| 声明 | 位置 | 依据 |
|---|---|---|
| `future skills install # install all future-* skills (~13)`（zh 同） | docs/build-and-install.md L171；zh L159 | skills/builtin/ 现含 **14** 个 future-* 技能（future-account/browser/database-lookup/deep-research/document/experimental-design/image/paper/peer-review/scientific-writing/skill-creator/slides/software-install/web） |

→ 已改为「(14)」/「（14 个）」。另注：**`future skills update` 确实存在**（cli/src/commands/skills.ts L19/L48-49/L287-328 已实现 updateSkills）——build-and-install L165/zh L163 此声明正确，错误在 wiki-prompt W12/WE（说「没有 update」），留待 wiki-prompt todo 修正。→ ✅ **已修正（todo_285b37996a0d）**：两个 wiki-prompt 的 skills 子命令改为 `list` / `install [<name>]` / `install-builtin` / `uninstall <name>` / `update`。

### B3. README 模型数量表述过时（低估）—— ✅ 已修复（commit b3b2e114，todo_5d852f73fcb6）

| 声明 | 位置 | 依据 |
|---|---|---|
| 「1000+ built-in models across 100+ providers」（zh 同） | README.md L26；README.zh-CN.md L23 | 生成目录 docs/wiki/{en,zh}/Models.md L3 现为「3826 models across 143 providers」（`make generate-models` 由 scripts/generate_models.py 生成，README 手工数字已落后）。建议改为引用生成目录或写明「3800+ / 140+」 |

→ 已改为「3800+ models across 140+ providers」（en）/「内置 3800+ 模型，覆盖 140+ Provider」（zh）。

> **Models.md 生成说明补充（todo_bcf715c7cc0e）**：Models.md 确认为生成文件（`make generate-models` → scripts/generate_models.py 写入 docs/wiki/{en,zh}/Models.md）。已在该脚本的 en/zh 头部模板加入「Auto-generated by `make generate-models` (scripts/generate_models.py). Do not edit by hand.」注释行，并把同一注释手工补到当前已入库的两个 Models.md 文件头（下次重新生成时由脚本保留）。

> **README 全量复核（2026-08-06，todo_5d852f73fcb6 收尾）**：除 B3/C1 外，README 其余声明逐条对照源码全部无误——沙箱三档（off/manual/sandbox「macOS Seatbelt, macOS only」与 proto 注释完全一致）、JSONL 会话 + fork/clone/tree + query-count（agent/src/session/mod.rs）、YAML frontmatter 技能多目录发现（agent/src/skills/mod.rs APP_SKILLS_DIR + AGENTS_SKILLS_DIR）、自动压缩 + 上下文超长指数退避重试（agent/src/agent/run_loop.rs `is_retryable_size_error` → 压缩后重试，`delay_ms = 2000 * (1 << (retry_attempt-1))`；llm/mod.rs L299 `context_length_exceeded`）、`future auth login` 设备码 + 自动同步模型列表（cli/src/commands/auth.ts saveAuth + agent sync_future_models RPC）、future-agent:50051 / future-tui 二进制名、8 个快捷键、全部内部链接与 banner 均存在。**README en/zh 无需再改，后继 todo 可跳过。**

### B4. wiki-prompt 页面清单无 Feishu/DingTalk/Integrations

| 声明 | 位置 | 依据 |
|---|---|---|
| 页面清单 10 页、侧边栏无 Integrations 分组 | docs/wiki-prompt.md W6-W7（L52-109）；en 对应 | 实际 wiki 有 Feishu.md、DingTalk.md，_Sidebar 含 Integrations 分组（docs/wiki/en/_Sidebar.md L4-20） |

→ ✅ **已修复（todo_285b37996a0d）**：两个 prompt 的页面清单补入 `Feishu.md`（飞书集成）、`DingTalk.md`（钉钉集成），§6 侧边栏加「集成 / Integrations」分组，§7 新增 Feishu/DingTalk 内容要点（代码入口指向 `channels/src/`；注明渠道桥为独立服务 `future-channel`、无 `future channel` 命令、斜杠命令 9 个本地处理、未知斜杠转发给 agent、配置 `~/.future/channels/config.json`、CardKit 卡片流式回复）；另加注 **`Models.md` 由 `make generate-models` 自动生成（scripts/generate_models.py），不手写、不进侧边栏**。

同轮其它修正（均以源码核实）：Installation 节「已签名+Apple 公证」→「当前发布包未公证/未签名（以 `docs/dist/readme-*.txt` 为准；仓库另有签名/公证发布流水线）」，与 FAQ 口径一致；Settings 节页码清单改为实测值（`SettingsDialog.tsx`：用户可见页 General/Account/Update/About/Providers/Models/Reset；Remote/Environment 为 devOnly 不写）。

### B5. `make generate-proto` 覆盖范围少写一端 —— ✅ 已修复（todo_cbbb063d2fd4）

| 声明 | 位置 | 依据 |
|---|---|---|
| 「make generate-proto（agent + channels + TUI）」 | docs/build-and-install.md L186-206（B17）；zh 对应 | 实际还包含 **desktop/src-tauri**（Makefile L404-410：agent → channels → desktop/src-tauri → tui） |

→ 已改为「agent + channels + GUI (src-tauri) + TUI」（zh 同）。

### B6. `make lint` 范围少写两端 —— ✅ 已修复（todo_cbbb063d2fd4）

| 声明 | 位置 | 依据 |
|---|---|---|
| 「lint all (agent + channels + TUI + CLI + GUI)」（zh 同） | docs/build-and-install.md L183, L192；zh L169, L178 | 实际 = lint-agent + lint-channels + lint-tui + lint-cli + lint-desktop + **stylelint-desktop** + **lint-mobile**（Makefile L232-253） |

→ 已改为「agent, channels, TUI, CLI, GUI (+stylelint), mobile」（zh 同）。另补：`make fmt` 实际 = cargo fmt（agent+channels）+ fmt-mobile（Makefile L262-269），文档原「cargo fmt (agent + channels)」也已一并更新。

### B7. DingTalk 斜杠命令措辞不精确

| 声明 | 位置 | 依据 |
|---|---|---|
| 「所有斜杠命令均由 Bridge 本地处理」（隐含与 Feishu 不同） | docs/wiki/en/DingTalk.md L91-103；zh 对应 | 两桥一致：9 个命令本地处理（/new /status /stop /model /models /compact /effort /cwd /help），**未知斜杠命令都转发给 agent 当普通消息**（feishu/bridge.rs L714；dingtalk/bridge.rs L262-265）。建议统一措辞 |

→ ✅ **已修复（todo_bcf715c7cc0e）**：en/zh DingTalk.md 改为与 Feishu 页一致：「9 个命令由 Bridge 本地处理，无法识别的命令作为普通消息转发给 Agent」。

### B8. macOS 公证状态：文档间冲突为「语境不同」，建议澄清措辞

| 冲突 | 位置 | 依据 |
|---|---|---|
| 「macOS build is also notarized by Apple」 | wiki Installation.md L24（en/zh） | 官方签名流水线 `.github/workflows/build-macos-signed.yml` 确实签名+公证（notarytool/staple） |
| 「The current build isn't notarized」/「当前版本未公证」 | wiki FAQ.md L9（en/zh）；docs/dist/readme-macos*.txt | 常规 CI（build.yml）与 dist 发布包未签名未公证 |

两说都各自成立（不同产物）。建议 Installation.md 措辞改为「官方签名版本经 Apple 公证」并指向 FAQ 的「当前下载包未公证」说明。

→ ✅ **已修复（todo_bcf715c7cc0e）**：en/zh Installation.md 改为「官方签名发布流水线签名 macOS/Windows 安装包并对 macOS 做 Apple 公证；当前下载版本未签名、未公证，首次启动告警见 FAQ」。

### B9. future-loop CLI 一览的呈现方式过时（轻微）—— ✅ 已修复（todo_63c718c2a3d5）

| 声明 | 位置 | 依据 |
|---|---|---|
| CLI 一览以 `ops <cmd>`、`work-items <cmd>`、`cli registry` 分组呈现 | docs/loop-control-plane.md L115-127 | 实际 `future-loop` 是**扁平顶层命令**分发（orchestration/loop/src/main.rs L93-137，共 42 个顶层命令）；`ops`/`work-items`/`cli` 只是注册表里的帮助分组名（cli/registry.rs），**不是可运行命令**。所列底层命令（goal/todo/gate/capability/extension/handoff/benchmark protocol|run|ledger/replay record|run|corpus/canary smoke/version/doctor/history/turn/todo-event/evidence-log/backup/…）全部存在 ✓，仅呈现层级需校正（文档 L127 已有「无参运行看全量帮助」提示，属低危） |

→ 本轮（todo_63c718c2a3d5）与 `build_cli_registry()`（main.rs L176-471）逐一比对后修正 CLI 一览（en/zh 同步）：

- **goal 组漏 2 命令**：`models`（`models [--format json]`，列出 agent 可用模型，main.rs L1634）与 `diagnose`（`diagnose --goal G [--format json]`，per-goal 诊断面，L3694）→ 已补入 goal 行
- **extension 漏 `upgrade`**：实际为 `install|upgrade|enable|disable|rollback|status|capabilities`（L2597 `"install" | "upgrade"` 同分支）→ 已补入 extension 行
- **`cli registry` 少 `--include-experimental`**：实际 `registry [--json] [--include-experimental]`（main.rs L466）→ 已补

其余核对通过：`todo add|claim|complete|supersede|update|archive`（L613-618）、`gate resolve`、`replan ack`、`lease`、`task-graph`、`agent onboard/scope/lane/supervisor`、`capability list|propose|commands` + `catalog`、`handoff [--write]`、`benchmark protocol|run|ledger`、`replay record|run|corpus build|run`、`canary smoke [--profile core-control-plane|extension-runtime|release-gate]`、ops 组全 19 命令、quota 三来源 run/agent/heartbeat（quota/slot_accounting.rs L42-44）、九种 disposition（decision/）、`--class monitor --cadence`（L655-660）、`--verify/--max-validation-attempts`（L656-657）、backup `--restore`（L991-1002）、状态布局 registry.json + goals/<id>/events.jsonl + ACTIVE_GOAL_STATE.md + runs/、`cargo build -p future-loop`、`scripts/install-future-loop.sh` 均 ✓

---

## C. 缺失（已实现但文档未覆盖，建议充实）

### C1. README TUI 斜杠命令表不全（12/19）—— ✅ 已修复（commit b3b2e114，todo_5d852f73fcb6）

README.md L105-118（zh L100-113）列出 12 个命令，源码（tui/src/app.ts `handleSubmit`）实际处理 **19 个**（17 个可用 + 2 个 stub）：

- **已实现未写入 README**：`/cwd`、`/approve`、`/reject`、`/cancel <run-id>`、`/reload`（tui/src/app.ts L71-86 自动补全清单同样收录这 16 个；dispatch 另含 /compact）
- stub（回答「not available」）：`/export`、`/import`

→ 已补 5 个实现命令入 README（en/zh），现列 17/19；`/export`/`/import` 为 stub 未列入。

> 顺带（代码侧观察，非文档）：TUI help-screen（help-screen.ts）只列 10 个命令、漏 /status /stop /cwd /approve /reject /cancel /reload /compact（其中 /compact 在 help 有、自动补全清单反而没有）——✅ **已修复**（commit 042a7d07，todo_b98a9381ad9e）：help-screen 现列全部 17 个实现命令，自动补全清单补上 /compact，两处与 dispatch/README 一致。

### C2. wiki CLI.md 缺命令组

见 B1：`session`、`models`、`account`、`init`、`doctor` 五个组 + `auth credential` + `skills update/install-builtin` 均未写入 wiki CLI.md。

### C3. 缺失文档已按核验清单补齐（todo_9bb2c6dd1c38，2026-08-06）

按 fact-inventory §14b 的 I-1..I-7 全部处理完毕：

- **I-1** Models.md 再生成说明 → 已解决（todo_bcf715c7cc0e：`make generate-models` 注释写入脚本模板与文件头）。
- **I-2** Linux 页面决策 → **维持 macOS+Windows**：release.yml 只发布 macOS（arm64+x64 dmg/updater）+ Windows（x64 setup）；Linux 便携包为 tester-only，wiki 不加 Linux 页。
- **I-3** 新增 **docs/channels-config.md / zh**：`~/.future/channels/config.json` 统一参考——`agent`/`feishu`/`dingtalk` 三块 schema、全部默认值与合法取值（逐字段核对 channels/src/config.rs + feishu/policy.rs；dm_policy=open|disabled|allowlist、group_policy=open|disabled|allowlist、首启写模板并退出、9 个本地斜杠命令、未知斜杠转发、Feishu keepalive 30s / DingTalk 20s、DingTalk webhook 新消息回复）。
- **I-4** loop-control-plane.md / zh 新增「Multi-agent workflow / 多 agent 工作流」章节：`agent`（注册 + `onboard --capability`）、`scope`（identity 边界 frontier 输出）、`lane`、`supervisor propose|receipt|events`、`handoff [--write]`（交付契约 + HANDOFF.md）、`task-graph`、`attention [--goal|--all]`、`inbox --project` 的完整示例；并注明这些是**扁平顶层命令**（main.rs L94/L119-127 顶层分发；帮助输出里的 agent/todo/work-items 分组仅为展示）。
- **I-5** 新增 **docs/README.md / zh**：docs 目录导航索引（顶层指南表、wiki 页面清单、dist 活文档、内部工作文档、文档保持正确的方法）。
- **I-6** 新增 **docs/tui.md / zh**：TUI 使用文档——17 个可用斜杠命令（/export /import 为 stub）、8 个快捷键、settings.json/keybindings.json/debug.log/write.log 路径（`write.log` 仅在 `PI_TUI_WRITE_LOG=1` 时写入，见 tui/src/tui.ts L282-287；无 crash.log）、常见排障（依据 tui/src/app.ts 等）。
- **I-7** 新增 **docs/directory-layout.md / zh**：`~/.future/` 完整布局——agent（settings/models/auth/sessions/skills/logs）、channels、tui、app（app.db/images/review）、workspaces/chat、loop（FUTURE_LOOP_ROOT 可覆盖）、bin；另附 `~/.agents/skills/` 与项目本地 `.future/`（loop、approval_rule.json）说明。

> 交叉链接闭环：docs/README.md（索引）⇄ tui.md ⇄ directory-layout.md ⇄ channels-config.md ⇄ loop-control-plane.md ⇄ wiki 各页，无悬空链接（`grep -rn '](…' docs/*.md` 抽查通过）。

---

## D. 核验无误（供后继跳过，不再重查）

| 核验面 | 结论 |
|---|---|
| TUI 快捷键 8 个 | ✓ README L122-131 与源码完全一致：ctrl+c 中断/退出、ctrl+p 循环模型、ctrl+r 浏览会话、ctrl+t 循环思考、tab 补全、enter 提交/接受、escape 关弹窗、↑↓ 滚动（tui/src/app.ts L226-232；help-screen.ts 同） |
| agent 端口 | ✓ 默认 `127.0.0.1:50051`（agent/src/main.rs L14）；CLI/TUI/channels 默认连接一致 |
| 配置路径 | ✓ `~/.future/agent/auth.json` 格式 `{"provider":{"type":"api_key","key":…}}` + 可选 `baseUrl`（agent/src/auth/mod.rs serde）；`~/.future/agent/models.json` `providers{apiKey,baseUrl,models[{id,name,contextWindow}]}`（agent/src/models/mod.rs L402-432）；`~/.future/channels/config.json`（channels/src/config.rs default_path）；TUI 本地 `~/.future/tui/settings.json`；loop 状态根 `~/.future/loop/`（FUTURE_LOOP_ROOT 可覆盖） |
| channels 配置默认值 | ✓ grpc_addr=http://127.0.0.1:50051、model=future/deepseek-v4-pro、thinking_level=xhigh、permission_level=all；feishu dm_policy=allowlist、group_policy=disabled、require_mention/streaming/resolve_sender_names=true、max_image_mb=10、typing_indicator=false；dingtalk domain=api.dingtalk.com；config.json 不存在→写模板并退出（config.rs 全默认值 + main.rs 行为） |
| channels 斜杠命令 | ✓ 两桥各 9 个：/new /status /stop /model /models /compact /effort /cwd /help（feishu/bridge.rs L424-714；dingtalk/bridge.rs L141-262） |
| DingTalk keepalive 20s | ✓ PING_INTERVAL_SECS=20（dingtalk/dingtalk_ws.rs L32） |
| 工具链版本 | ✓ rust-toolchain.toml `1.97.0`；.nvmrc `24`；.cargo/config.toml：mold 仅 x86_64-unknown-linux-gnu、windows-msvc /DEBUG:NONE（「mold 在 x86_64 必需、ARM Linux 不需要」表述正确） |
| Makefile 其余目标 | ✓ install/install-desktop/install-agent|tui|cli|desktop|channels|skills|loop、uninstall、build*（agent/tui/cli/desktop/desktop-dist/channels/mobile）、package-desktop、run-agent|tui|cli|desktop|channels、generate-models、generate-proto、fmt、clean 全存在；install 前缀 per-OS 正确（macOS /opt/homebrew/bin、Linux /usr/local/bin sudo、Windows %USERPROFILE%\.future\bin）；install-skills 符号链接/Windows 拷贝；install-loop → scripts/install-future-loop.sh；scripts/（build-desktop-macos.sh、build-desktop-windows-portable.ps1、build-desktop-windows-installer.ps1、start-desktop-windows.bat）全存在 |
| build-desktop-macos.sh | ✓ 唯一 Developer ID 自动签名、`--identity`/`--out-dir`/`--notary-profile` 等选项与 B4 描述一致 |
| CLI run 选项 | ✓ --model 支持 `model:thinking`、--thinking off/minimal/low/medium/high/xhigh、@<path> 文件包含、--continue/-c、--cwd、--mode text|json、--no-session（cli/src/commands/run.ts 帮助文本） |
| CLI tools | ✓ list / describe / call（--key value、--args '<json>'、--stdin、--output、--timeout 等） |
| `future init` | ✓ = 装内置技能 + macOS/Linux 链接 future/future-agent 到 ~/.future/bin（cli/src/index.ts L41-46 帮助文本）——build-and-install B14 描述正确 |
| future-loop 命令面 | ✓ 顶层 42 命令（main.rs L93-137）；goal init/cancel/delete（L494-586）、todo add/claim/complete/supersede/update/archive（L613-618）、gate、capability/catalog、extension install|upgrade|enable|disable|rollback|status|capabilities（L2593-2670）、handoff（L2958）、benchmark protocol|run|ledger（L3393-3395）、replay record|run|corpus build|run（L3610-3614）、canary smoke --profile（L3631-）、run --goal/--model/--thinking-level/--max-turns（L375）、todo add --verify/--max-validation-attempts（L656-657）全部存在 |
| install-future-loop.sh | ✓ CLI → ~/.local/bin/future-loop、skill → ~/.future/agent/skills/future-loop/SKILL.md、`future-loop status` 验证 |
| proto | ✓ proto/future.proto 存在；生成代码入库（agent/src/grpc/generated/proto.rs、channels/src/generated/proto.rs）；make generate-proto 再生 agent+channels+desktop/src-tauri+tui |

---

## E. 遗留 / 超出本 todo 范围（建议后继核验）

- **GUI 功能声明**（wiki Using-FutureOS/Settings/Skills/Quick-Start + Home 的 Artifacts 提及）—— ✅ 已核验（todo_cab9a84ced24，2026-08-06），见 §G。发现并修正 6 项：Artifacts 面板已停用（→Files 视图）、右栏视图表、内置技能表（3 个不存在技能 → 实际 14 个）、Settings General 缺 Auto-upgrade skills、FutureGene「Connect」→「Sign in」、内置 provider「Set key/Update key」→「Configure」。
- **X10 架构审计时效 —— ✅ 已裁决（todo_41f779819879）**：architecture-audit 判定为**时点快照，标注历史，不逐行重核**。审计文档与首批修复同批入库（commit `306cf05f`，2026-08-06）：报告 01 H2/H3/H4、报告 04 H1/H2/H4 已在同一 commit 修复（代码注释直接引用审计编号，如 auth_store.rs「audit item 2」、useThreadStore.ts「(H2)」）；仍成立的高危项为报告 01 H1/H5、报告 04 H3。file:line 已漂移（报告 02 引用文件多处移动、报告 03 行数增长、报告 01 proto 行号失效），README.md 顶部已加时效性说明并逐条标注 ✅。后续如需更新审计，应基于当前工作树重跑调查而非修订旧行号。
- **docs/dist/*.txt —— ✅ 已核验（todo_41f779819879）**：6 个发布包内附说明为**活文档**（build.yml / build-desktop-windows-portable.ps1 / build-windows-signed.yml 打包时逐字复制进 dmg/zip/tar.gz），全部声明与当前源码一致（二进制名 `futureos`/`future-agent`/`future` 三件套与 build.yml 装配步骤完全吻合；macOS「未公证」与 FAQ/B8 口径一致；WebView2/WebKitGTK 运行时要求正确）。**无需修改**；`-en.txt` 为参考译文（打包只用 zh `.txt`）。后继 todo 可跳过。
- **X8 沙箱术语**：README「off / manual / macOS Seatbelt」vs wiki「Manual / Sandboxed (macOS only) / Unrestricted」——属 GUI 核验面。
- TUI help-screen / 自动补全清单与实现命令集不一致（代码侧小问题，见 C1 备注）—— ✅ 已修复（commit 042a7d07，todo_b98a9381ad9e）。

---

## F. 最终复核结果（todo_567768e616f3，2026-08-06）

> 在全部修复提交之后对 docs/ 全量做收尾复核，三项检查全部通过：

### F1. 中英一致性

- 成对文档标题结构逐一比对（排除代码围栏内的 shell 注释行）：README（10/10）、docs/README（6/6）、tui（5/5）、directory-layout（9/9）、channels-config（8/8）、loop-control-plane（**19/19**，h1/h2/h3 全等；初查 15 vs 14 的差异来自代码块内 `#` 注释行，非标题）、build-and-install（19/19）、wiki-prompt（23/23）。全部一致。
- 关键事实抽查一致：TUI 斜杠命令表 en/zh 均为 17 可用 + 2 stub（/export /import）；快捷键 4 项（ctrl+p/t/r/c）逐字一致；channels-config 默认值表 en/zh 数字一致（50051、deepseek-v4-pro、xhigh、10MiB、30s/20s）。

### F2. 链接有效性

- 全量链接检查（16 个 md：docs/*.md + README*），0 断链。检查器先剥离行内代码与代码围栏（wiki-prompt 里 `[x](Quick-Start)` 这类**语法示例**在反引号内，非真实链接；Quick-Start.md 实际存在于 en/zh 两目录）。
- wiki 交叉链接 + 锚点检查（en/zh 各 13 页，含 `#锚点` 解析为 GitHub slug），0 错误。
- 新增文档交叉引用闭环（docs/README ⇄ tui ⇄ directory-layout ⇄ channels-config ⇄ loop-control-plane ⇄ wiki）全部可达。

### F3. 变更清单（7383eca1..HEAD，15 commits，全部已推送分支）

| commit | 内容 |
|---|---|
| 05a9a575 | README en/zh 全量复核完成标记（B3 收尾） |
| dd14b60a | loop skill：长回合超过默认 shell 超时——阻塞运行 + 显式超时 |
| 9cac0b48 | build-and-install en/zh：make test/lint/fmt/generate-proto 范围 + 技能数 14（A5/B2/B5/B6） |
| 3a587ad4 | loop-control-plane en/zh：CLI 一览对齐实际命令面（B9：goal 补 models/diagnose、extension 补 upgrade、cli registry 补 --include-experimental） |
| 9512b972 | wiki-prompt en/zh：CLI 命令面/二进制名/打包/清单全面对齐（A1/A2/B4 + Models.md 生成说明） |
| c7c95d03 | wiki 全部页面 en/zh：CLI.md 重写、Feishu/DingTalk（A3/A4/B7）、Installation（B8）、Settings 页码、Models.md 生成头注 |
| fff6c18b | architecture-audit 标注时点快照 + dist readmes 核验（X10） |
| 8fe97b57 | loop 代码修复：流式文本 UTF-8 边界截断（panic 修复） |
| b19bf933 | loop skill：每回合后显式「反思与改进」步骤（超智能体主动再规划） |
| 950039f1 | **缺失文档补齐（C3/I-1..I-7）**：docs/README（索引）、tui、directory-layout、channels-config、loop-control-plane 多 agent 章节 |
| a2b7105c | tui/directory-layout en/zh：crash.log 事实修正 → write.log（PI_TUI_WRITE_LOG=1 才写），4 个文件 |

遗留（见 §E）：GUI 功能声明核验（Using-FutureOS/Settings/Skills/Quick-Start 页面内容，✅ 已核验 todo_cab9a84ced24）、TUI help-screen 与补全清单不一致（✅ 已修复 commit 042a7d07，todo_b98a9381ad9e）。

## 附：核验方法

- 声明→源码逐条比对；源码依据均含 file:line（见各条目「依据」列）。
- CLI 移除历史经 `git show eed93369` 确认（2026-07-16，删除 agent.ts/channel.ts/gui.ts/tui.ts 共 665 行）。
- 生成文件 Models.md 数字以文件头为准（3826/143），未重跑 `make generate-models`（需网络）。

---

## G. GUI 核验结果（todo_cab9a84ced24，2026-08-06）

> 对照 desktop/src-tauri（Rust commands）+ desktop/src（React）核验 wiki 四个 GUI 页面（Using-FutureOS / Settings / Skills / Quick-Start）+ Home 页的 Artifacts 提及。en/zh 成对修改，标题结构保持对齐。

### G1. 已修正（6 项）

| 条目 | 修正内容 | 源码依据 |
|---|---|---|
| Artifacts 面板已停用 | Using-FutureOS / Quick-Start / Home en+zh：删除「Chat 右栏 = Runs + Artifacts」「Artifacts 收集产出（预览/复制/导出/上传）」等描述，改为 **Files** 视图（每个会话都有 Files 标签：Chat=临时会话文件夹，Workspace=项目文件夹；可预览/系统打开/从目录树附加到对话） | ContextPanel.tsx L34-57：Artifacts 标签从 fileTabs/gitTabs 注释掉（commit 9756a7b2，2026-07-14「hide the Artifacts tab pending a decision on its purpose」）；chat=files+runs，workspace=files+runs+review；`isFutureReferenceType()` 恒 false → futureos:// 应用对象引用全部失效（parseFutureMarkdown.ts L427-438） |
| 右栏视图表/三栏描述 | 「Runs / Review / Artifacts」→「Files / Runs / Review」；Chat vs Workspace 表右栏改为 Files+Runs / Files+Runs+Review | 同上 |
| Skills 内置技能表 | 删除 3 个不存在的技能（Hand-drawn posters、Hand-drawn slides、Subagent），改为实际 **14 个**内置技能：Account/Browser/Database lookup/Deep research/Document/Experimental design/Image/Paper/Peer review/Scientific writing/Skill creator/Slides/Software install/Web | 在线目录 `/client/v1/skills`（2026-08-06 实拉 test.future-os.cn，139 技能中 14 个 `future-*`）；repo `skills/builtin/` 同名 14 目录；`future init`/install-builtin 按 `future-` 前缀过滤（cli/src/commands/skills.ts L246） |
| Settings General 缺「Auto-upgrade skills」 | 补上：应用每次打开时静默升级已装技能到最新版（默认开） | GeneralPage.tsx autoUpgradeSkills Switch；app_settings.rs `unwrap_or(true)`（结构体注释原写「Off by default」与实现不符，**已修正为 On by default**，commit 042a7d07） |
| FutureGene 按钮名 | 「Click **Connect**」→「Click **Sign in**」；删掉不存在的「Sign in again」（登录态只有 Sign out） | ProvidersPage.tsx：connect 按钮 = t("providers.connect") = "Sign in"（settings.json）；触发 show-onboarding（设备码 OAuth） |
| 内置 provider 按钮名 | 「Set key / Update key」→「**Configure**」（点开对话框标题为 Set <provider> key） | ProvidersPage.tsx t("providers.set") = "Configure"；keyDialogTitle = "Set {{provider}} key" |

### G2. 核验无误（供后继跳过，不再重查）

| 核验面 | 结论 |
|---|---|
| 批准机制 | ✓ 触发面：文件读写 / shell 命令 / workspace 外写入（agent/src/rpc/approval.rs approval_shape：file_read/file_write/outside_workspace_write/shell_command/sandbox_escalation；删除文件经 shell `rm` 审批，GUI ActionDetails 的 deletes 分支为防御性渲染）；卡片无超时（GUI 无 timeout，agent 侧 pending 审批跨重启保留）；Allow once / Deny / Allow in this workspace-chat（规则路径可编辑；secret 文件不出规则按钮 save_suggestion=None）；Cmd/Ctrl+Enter 批准、Esc 拒绝（规则编辑器开着则先关）（ApprovalPrompt.tsx） |
| 批准模式三态 | ✓ manual/sandbox/off 与 i18n 文案逐字一致：Manual=读写前询问、只读命令自动跑；Sandboxed=仅 macOS（GeneralPage `isMacOS` 才渲染选项）、命令入 macOS 沙箱、文件操作仍询问；Unrestricted=不询问不沙箱一切照跑；默认 off（app_settings.rs normalize_tier + read 默认） |
| 输入框 | ✓ 模型选择器 + 思考级别 + 盾牌（批准模式，ShieldCheck/Off/Question 三态图标）+ 回形针/粘贴/拖拽附件 + 流式时发送键变停止键（Composer.tsx） |
| 4 图 / 25MiB | ✓ MAX_IMAGES_PER_TURN=4、READ_SOURCE_MAX_BYTES=25*1024*1024、非图片文件不限额（attachments.ts L11-19）；后端 MAX_ATTACHMENT_IMAGE_BYTES=25*1024*1024（commands/files.rs L114） |
| 左栏结构 | ✓ New Chat / Models(设置快捷入口) / Skills / Workspaces(每项可展开会话、可折叠、+ 新建) / Chats / Settings 在底部；左栏可折叠（ActivityRail.tsx；layout.json） |
| Runs 面板 | ✓ 卡片显示真实命令（shell 行逐字）、工具状态、running/finished 计数；Inspect / Terminate / Clear finished（RunsPanel.tsx + runs.json） |
| Review 面板 | ✓ 文件列表 + 改动类型(added/modified/deleted/renamed) + 逐文件 diff；git workspace 才有「Last run changes」视图切换（非 git 只显示 last-run 单视图）；默认视图由后端 capabilities.defaultView 定（ReviewPanel.tsx + review.json） |
| 会话菜单 | ✓ 重命名 / 置顶(取消) / 删除（ThreadListItem.tsx + layout.json rename/pin/delete） |
| Settings 页签 | ✓ 发布构建仅 General/Providers/Models/Account/Check for updates/About/Reset；Remote、Environment 为 devOnly（SettingsDialog.tsx NAV_GROUPS + `devOnly`）——文档不提及二者正确 |
| FutureGene 授权流 | ✓ 设备码 OAuth：浏览器授权，未自动打开时显示验证码(user_code) + 可复制链接(verification_uri_complete)（future_login.rs） |
| 自定义 provider | ✓ Name(可选)/Provider ID(小写字母数字`-`_，长度+模式校验)/API type(OpenAI Completions|Responses|Anthropic)/Base URL(http/https 校验)/API Key/Models；ID 唯一性检查（CustomProviderDialog.tsx + settings.json 校验文案 idPattern/idLength/baseUrlInvalid） |
| Models 页 | ✓ 按 provider 分组、搜索（模型名或 provider 名）、可见性开关→隐藏模型从选择器移除（ModelsPage.tsx hiddenModels + providerNames） |
| 更新 / 账户 / 重置 | ✓ 检查更新→下载→安装→重启（UpdatePage + install_app_update/restart_after_app_update）；账户页资料+余额+充值（AccountPage + useFutureAccount）；重置=清除本地数据并重启（ResetPage clear_app_data） |
| Skills 页 | ✓ Installed/All 两标签、分类下拉+搜索+结果计数+清除过滤、Install/Uninstall(二次确认)/Upgrade/Upgrade all；首次无已装技能自动切 All；「All 需要联网」正确（SkillsView.tsx + skillsClient） |

### G3. 变更清单（本 todo，一次提交 `9f68ed0e`，14 个文件）

| 文件（en+zh 成对，7 对） | 内容 |
|---|---|
| Using-FutureOS | Artifacts→Files（三栏描述、Chat vs Workspace 表、Artifacts 小节整体替换） |
| Quick-Start | 第 5 步「最多三种视图」重写为 Files/Runs(+Review)；FutureGene「Connect」→「Sign in」 |
| Skills | 内置技能表 11 项 → 实际 14 项（删 3 个不存在的，补 6 个漏掉的，Slides 名称订正） |
| Settings | General 补 Auto-upgrade skills；FutureGene「Connect」→「Sign in」（删 Sign in again）；内置 provider「Set key/Update key」→「Configure」 |
| Home | 「generated outputs (Artifacts)」→ Files/Runs/Review 表述 |
| FAQ | 「FutureGene → Connect」→「FutureGene → Sign in」（EF5） |

核验过程另见 fact-inventory.md：EH3/EQ1/EQ5/EU1/EU2/EU9/ES3/ES4/ESK3/EF5 已更新；§14a H14/H15 关闭。

---

## H. 2026-08-18 复核：loop 文档、技能数、目录布局（branch `docs/claude-md-improvements`）

> 对照当前工作树源码（loop 控制面已删 capability/extension 生态、默认状态根改
> 为项目本地 `<cwd>/.future/loop/`；skills/builtin 含 15 个 future-* 技能）逐条核验。

### H1. 已修正（本轮）

| 条目 | 修正内容 | 源码依据 |
|---|---|---|
| loop 文档 todo 类别「三类」过时 | 五类：advancement / user-gate / user-action / monitor / blocker（en/zh 核心概念表） | state.rs `TaskClass` 枚举 5 变体；console.rs L824 `--role/--class` 校验文案 |
| loop 文档「PLAN_REVIEW checkpoints are agent-resolved」无源码依据 | 删除；改为 user-action 语义（`user_action` 展示给用户但**不冻结** agent，decision/mod.rs L111-114 注释 + UserChannel 分支） | 全库 grep `PLAN_REVIEW` 0 命中 |
| loop 文档「三端体验」（TUI/GUI/移动端可见 loop 状态）与代码不符 | 改为「loop 状态以 CLI 为准」：状态项目本地、TUI/GUI/移动端/IM 无内置 loop 视图，经 `/future-loop` 技能编排 `future loop` 命令驱动 | desktop/src、mobile/src、tui/src 均无 future_loop 引用（grep 0 命中）；loop 状态仅 CLI 可读 |
| loop 文档「渠道桥消息可触发 loop 操作、门禁回聊天」无集成代码 | 改为「任何客户端可经技能驱动；桥与 loop 无原生集成」 | channels/src 无 loop 集成（grep 0 命中） |
| loop 文档 CLI 分组清单过时 | 按 `future loop registry` 实测输出修正：agent 组补 `list`、ops 组补 `serve-status`、删除「质量组」改为 handoff / cli / benchmark / replay / canary 五组（10 组 43 命令数不变） | `./target/debug/future loop registry` 实测；cli/registry.rs |
| loop 文档 gate 示例缺必填 `--text` | 补 `--text`（`--gate-question` 默认为 `--text`） | console.rs `--text required` |
| directory-layout 声称 loop 状态根默认 `~/.future/loop/` | 实际默认 `<cwd>/.future/loop/`（项目本地），`FUTURE_LOOP_ROOT` 覆盖；`~/.future/loop/` 不再使用。已改章节标题与正文、从 `~/.future/` 目录树删除 `loop/` 行 | console.rs `root_dir()` L56-66（无 home 回退） |
| console.rs 模块注释「State lives under `--root` (default `~/.future/loop/`)」 | 改为项目本地 `<cwd>/.future/loop/`（`--root` 旗标亦不存在） | console.rs `root_dir()` |
| build-and-install「install all future-* skills (14)」 | → 15（skills/builtin 现含 future-loop） | `ls skills/builtin/` 15 目录 |
| build-and-install「开发（源码方式）」节重复两份（第二份为过时版：make test=cargo test (agent)、lint 少 GUI/mobile） | 删除过时副本；`make fmt` 注释改为 `cargo fmt --all (workspace) + desktop/src-tauri + mobile`（en/zh） | Makefile `fmt:` = cargo fmt --all + src-tauri + fmt-mobile |
| wiki Skills.md 内置技能表缺 Loop（14 项） | 补 **Loop** 行（en/zh），技能总数 15 | skills/builtin/future-loop（v3.0.2） |
| wiki-prompt 内置技能示例清单含 3 个不存在技能（Hand-drawn posters/slides、Subagent） | 替换为实际 15 个（含 Loop） | skills/builtin/ 目录实测 |
| README「14+ skills」、docs/README loop 行「extensions」 | README → 15+ 并补 `/future-loop` 例子；docs/README loop 行「扩展与多 agent」→「交付闭环、多 agent」（extension/capability 命令已删，`future loop extension status` 报 unknown command） | 实测 CLI 0 命中 extension/capability |

### H2. 核验无误（供后继跳过）

- loop 注册表「10 组 43 命令」、`quota should-run/usage/spend/decisions`、`scheduler tick|show|record-host-failure|ack|liveness`、`delivery status|record|followthrough`、`agent onboard|list|contract|recipe|succession|collective`、`lease claim|renew|release|expire|status`、`frontier show`、`run --goal/--agent-id/--model/--thinking-level/--max-turns` 与实测 registry 输出一致。
- 语义历史 N=50（goal_frontier/semantic_history.rs `SEMANTIC_HISTORY_CAP`）；交付 3 回合未验证派生跟进（work_items/delivery_outcome.rs）；空转 15 分钟记账（state.rs `TURN_NO_PROGRESS_IDLE_SECS_DEFAULT = 15 * 60`）；租约 pid 活性回收（state.rs claim → compat::pid_alive）。
- channels-config.md 全部字段与默认值 ↔ channels/src/config.rs 逐字段一致。
- TUI 斜杠命令 19 个（17 可用 + /export /import stub）与 tui.md/README 一致；9 个快捷键与 help_screen.rs 一致；README 沙箱三档与 future.proto L251 注释一致；Models.md 3826/143 与 README「3800+/140+」一致；`future init` 链接 future + future-agent 到 ~/.future/bin 与 init.rs 一致。
- wiki CLI.md 命令组与 `future --help` 实测一致（init/auth/account/run/skills/tools/models/session/doctor + agent/tui/channel/loop）。

### H3. 技能仓库遗留 → 已解决（future-skills#14，v3.0.3）

- skills 子模块 future-loop/SKILL.md 的「PLAN_REVIEW checkpoints are agent-resolved」
  已在上游修正（future-skills#14，squash 合入，b045032），同批还修了
  `agent register`（实际为 `onboard|list|contract|recipe|succession|collective`）
  与 `canary [--premerge]`（实际为 `canary smoke [--profile …] | canary premerge`）；
  版本 3.0.2 → 3.0.3。主仓库指针 bump 随本 round 一起提交。
