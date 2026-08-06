# 文档事实清单（核验依据）

> **内部工作文档**（非用户文档）。本文件是文档核验任务的中间产物：逐条记录
> README + `docs/` 下所有文档中的**关键声明及其位置**（file:line / 章节），
> 作为后续「对照源码核验」的逐条依据。
> 本文件只记录「文档说了什么 + 在哪说的」，**不判定对错**；判定留给核验步骤。
> 行号以 2026-08-06 工作树为准。

- 生成时间：2026-08-06
- 范围：`README.md`、`README.zh-CN.md`、`docs/` 全部 **45 个文件**（39 个 .md + 6 个 .txt；含 docs 根 6 个、wiki en/zh 各 13 个、architecture-audit 5 个、dist 6 个）
- 不在范围：`CLAUDE.md`（agent 指令）、`gui/CLAUDE.md`、`mobile/README.md`、`skills/README*.md`（组件级 README，非 README+docs 范畴）

---

## 0. 文档清单总表

| # | 文件 | 行数 | 类型 | 语言 |
|---|---|---|---|---|
| 1 | README.md | 143 | 用户文档（根） | en |
| 2 | README.zh-CN.md | 138 | 用户文档（根） | zh |
| 3 | docs/build-and-install.md | 206 | 用户文档（构建/安装） | en |
| 4 | docs/build-and-install.zh-CN.md | 190 | 用户文档（构建/安装） | zh |
| 5 | docs/loop-control-plane.md | 179 | 用户文档（功能指南） | en |
| 6 | docs/loop-control-plane.zh-CN.md | 128 | 用户文档（功能指南） | zh |
| 7 | docs/wiki-prompt.md | 229 | **生成提示词**（给 AI，非用户文档） | zh |
| 8 | docs/wiki-prompt-en.md | 229 | **生成提示词**（给 AI，非用户文档） | en |
| 9-21 | docs/wiki/en/*.md（13 个） | 见 §7 | 用户文档（wiki） | en |
| 22-34 | docs/wiki/zh/*.md（13 个） | 见 §8 | 用户文档（wiki） | zh |
| 35 | docs/architecture-audit/README.md | 18 | 内部审计报告 | zh |
| 36 | docs/architecture-audit/01-agent-guirust-boundary.md | 187 | 内部审计报告 | zh |
| 37 | docs/architecture-audit/02-guirust-guireact-boundary.md | 154 | 内部审计报告 | zh |
| 38 | docs/architecture-audit/03-large-modules-split.md | 278 | 内部审计报告 | zh |
| 39 | docs/architecture-audit/04-react-rendering-performance.md | 149 | 内部审计报告 | zh |
| 40-45 | docs/dist/readme-{macos,windows,linux}[-en].txt（6 个） | 20-34 | 发布包内附说明 | zh/en |

**wiki 页面清单**（en/zh 文件名一一对应，13 对）：Home、Installation、Quick-Start、
Using-FutureOS、Settings、Skills、CLI、FAQ、Feishu、DingTalk、Models、_Sidebar、_Footer。
> 注意：wiki-prompt 第 6 节页面清单**没有** Feishu/DingTalk，但实际 wiki 存在这两个页面
> 且 _Sidebar 有「Integrations」分组 —— 属于 prompt 与实际页面的偏差（见 §12-X6）。

---

## 1. README.md（en）

| # | 声明 | 位置 |
|---|---|---|
| R1 | 定位：local-first AI agent workspace，TUI/GUI/CLI/Feishu/DingTalk 多端，macOS/Linux/Windows | L6-9（标题下引言） |
| R2 | **「1000+ built-in models across 100+ providers」** | L26（特性表 Model Flexibility） |
| R3 | 模型配置三途径：A) `future auth login` 设备码登录自动配好；B) `~/.future/agent/auth.json` 按 provider 名索引（`{"openai":{"type":"api_key","key":"sk-..."}}`），Azure 类带 `baseUrl` 字段；C) `~/.future/agent/models.json` 自定义 provider（`providers[].apiKey/baseUrl/models[].{id,name,contextWindow}`） | L46-79 |
| R4 | **agent 必须先运行，监听 `127.0.0.1:50051`**；`future-agent` 启动，`future-tui` 启动 TUI | L85-94 |
| R5 | 连接/gRPC 错误 ⇒ 几乎都是 agent 没启动 | L96 |
| R6 | TUI 斜杠命令 12 个：`/help` `/model` `/new` `/sessions` `/compact` `/scoped-models` `/clone` `/fork` `/tree` `/name [n]` `/status` `/stop` | L105-118（表头 L105） |
| R7 | TUI 快捷键：`ctrl+p` 循环模型、`ctrl+t` 循环思考级别、`ctrl+r` 浏览会话、`ctrl+c` 中断/退出、`tab` 补全、`enter` 提交、`escape` 关弹窗、`↑↓` 滚动 | L122-131（表头 L122） |
| R8 | 排障：连接错误 → `lsof -i :50051` 查端口占用；auth/"no model" → `future auth login` 或 models.json 加 provider | L133-140 |
| R9 | 特性表其余声明：流式+思考链（off ↔ xhigh）；工具 read/write/edit/shell + 审批 + **sandbox tiers (off / manual / macOS Seatbelt)**；JSONL 会话 + fork/clone/tree；YAML skills 多目录发现；自动压缩 + 指数退避重试；loop 控制面（loopx Rust 改写）；Rust 核心 | L22-33 |
| R10 | License: MIT | L143 |
| R11 | 外链：wiki（github.com/futuregene/future-os/wiki）、future-skills 仓库 | L3, L5 |

---

## 2. README.zh-CN.md（zh）

与 en 对应，关键差异：

| # | 声明 | 位置 |
|---|---|---|
| RZ1 | 「内置 1000+ 模型，覆盖 100+ Provider」 | L23 |
| RZ2 | 模型配置三途径（同 R3，链接指向 docs/wiki/zh/ 与 docs/loop-control-plane.zh-CN.md、docs/build-and-install.zh-CN.md） | L43-77 |
| RZ3 | agent 监听 `127.0.0.1:50051`；`future-agent`/`future-tui` | L80-89 |
| RZ4 | 斜杠命令表（同 R6，12 个） | L100-113（表头 L100） |
| RZ5 | 快捷键表（同 R7） | L117-126（表头 L117） |
| RZ6 | 排障表（同 R8） | L128-133 |
| RZ7 | 特性表（同 R9；沙箱「关闭 / 手动 / macOS Seatbelt」） | L19-30 |
| RZ8 | MIT | L138 |

---

## 3. docs/build-and-install.md（en）

| # | 声明 | 位置 |
|---|---|---|
| B1 | 前置要求：**Rust 1.97+**（`rust-toolchain.toml` 固定）、**Node.js 24+**（`.nvmrc`）、**Bun 必需**（TUI 构建与 CLI/GUI 打包用 `bun build`）、可选 Python 3（仅 `make generate-models`）、可选 protoc（仅 `make generate-proto`，生成代码已入库） | L12-18 |
| B2 | 克隆：`git clone https://github.com/futuregene/future-os.git` | L22-25 |
| B3 | macOS 依赖：`xcode-select --install`、rustup、`brew install node oven-sh/bun/bun`、可选 `brew install protobuf` | L30-37 |
| B4 | macOS 构建：`make install` → **/opt/homebrew/bin**；`make install-nogui`；`make package-gui` → **.app + .dmg**（gui/src-tauri/target/release/bundle/）；`scripts/build-macos-dmg.sh`（自动用唯一 Developer ID Application 签名 → `*-sign.dmg`，`--help` 看证书选择/输出目录/**Apple 公证**选项） | L40-51 |
| B5 | Linux 依赖：`build-essential mold libssl-dev libwebkit2gtk-4.1-dev libgtk-3-dev librsvg2-dev libayatana-appindicator3-dev patchelf` + rustup + bun + nvm(Node 24) + 可选 protobuf-compiler | L58-64 |
| B6 | **mold 在 x86_64 上必需**（`.cargo/config.toml` 传 `-fuse-ld=mold`）；ARM Linux 不需要 | L67 |
| B7 | Linux 构建：`make install` → /usr/local/bin (sudo)；`make package-gui` → **.deb** | L71-75 |
| B8 | Windows 工具链：VS Build Tools（Desktop C++ workload）、`winget install Rustlang.Rustup`（host triple x86_64-pc-windows-msvc）、Node 24+、Bun、WebView2（随 Win10/11 自带） | L79-86 |
| B9 | Windows 终端栈（等价 install-nogui）：cargo build agent/channels → `npm install; npm run gen-version; npm run build; bun build --compile ... --outfile dist/future-tui.exe`（tui）与 `...--outfile dist/future.exe --external chromium-bidi`（cli）→ 复制到 **%USERPROFILE%\.future\bin**（future-agent.exe、future-channel.exe、future-tui.exe、future.exe）→ `& "$bin\future.exe" skills install`（Windows 上不用符号链接） | L88-108 |
| B10 | Windows 桌面应用：sidecar 以 host triple 命名（future-agent-$triple.exe / future-$triple.exe）→ `npx tauri build --no-bundle` → 复制 futureos.exe → **future-gui.exe** | L110-122 |
| B11 | Windows 安装包：`node scripts\version.mjs --set-bundle` + `npm run tauri:build` → **NSIS .exe**（bundle\nsis\） | L124-128 |
| B12 | 脚本说明：`scripts/start-gui-test.bat` 开发模式；`build-macos-dmg.sh`/`build-windows-portable.ps1`/`build-windows-installer.ps1` 复刻 CI 流水线（DMG/便携 zip/NSIS），需 protoc；产物含 GUI+agent+CLI，**不含 TUI** | L130-133 |
| B13 | loop 控制面：`orchestration/loop`，`cargo build -p future-loop`（debug→target/debug，release→target/release）；`bash scripts/install-future-loop.sh` → **~/.local/bin/future-loop** + **~/.future/agent/skills/**；`future-loop status` 验证 | L135-155 |
| B14 | 技能安装：`make install-skills`（内置 skills/ 子模块符号链接）；`future skills install`（**约 13 个**）；`future init`（装技能 + macOS/Linux 链接本地命令）；符号链接进 `~/.future/agent/skills/`；`future skills list`/`future skills update` | L162-178（`future skills install` L171，`future skills update` L177） |
| B15 | 验证：`make test`（cargo test agent + loop）、`make lint`（agent+channels+TUI+CLI+GUI） | L168-171 |
| B16 | 开发：`make build/lint/fmt/test/clean` | L174-183 |
| B17 | Proto：规范 API 是 `proto/future.proto`；生成代码已入库；`make generate-proto`（agent + channels + TUI） | L186-206 |

---

## 4. docs/build-and-install.zh-CN.md（zh）

与 en 对应一致（B1-B17 镜像）。关键行号：前置要求 L10-16、macOS L28-47、Linux L54-70、
Windows L73-127、loop L129-146、技能 L148-166（`future skills update` 升级 → L163）、
proto L180-190。**无内容差异**（除链接指向 .zh-CN 版本）。

---

## 5. docs/loop-control-plane.md（en）

| # | 声明 | 位置 |
|---|---|---|
| L1 | 定位：本地控制面，位于 `orchestration/loop`，`future-loop` CLI + `/future-loop` agent 技能 | L5-7 |
| L2 | **`future-loop` 是 loopx（github.com/huangruiteng/loopx）的 Rust 改写版**，针对 FutureOS 适配扩展（项目本地状态、gRPC 执行桥、quota 内核、扩展与多 agent） | L8-11 |
| L3 | 目标：`goal init / cancel / delete`，状态在 `<cwd>/.future/loop/`，事件账本 + 重放 | L40-41 |
| L4 | Todos：`todo add / claim / complete / supersede / update / archive`；advancement/user-gate/monitor/blocker 类别；`--blocks` 依赖链；claim+租约；完成契约（每个完成 todo 声明后继或显式 no-follow-up） | L42-45 |
| L5 | 人工门禁：`gate resolve` | L47 |
| L6 | 监控：`--class monitor --cadence ...`，无变化退避 | L49 |
| L7 | 决策内核：`future-loop run`，纯函数可注入时钟，**九种 disposition**（terminal/monitor-wait/active work/consistency repair/human gate/quiet wait/…），fail-closed | L54-60 |
| L8 | 额度：run/agent/heartbeat 三来源 slot 记账、24h/7d 汇总、stall repair | L62-65 |
| L9 | 调度：节奏归一化（`once / hourly / daily / weekly` 或 `15m / 1h / 2d`）、原子持久化、host 失败跟踪 | L67-68 |
| L10 | 事件溯源：内容寻址事件 id、幂等追加、fail-closed 冲突检测；`QuotaSpent`/`EvidenceAttached` 事件；markdown 回填 | L72-76 |
| L11 | 迁移桥（verify / migrate / bridge）；隐私分级投影（public-safe/local-private/private-pointer）；run 生命周期（history/compaction/index/retention/stale 检测） | L77-80 |
| L12 | 独立验证：`todo add --verify "cargo test" --max-validation-attempts 5`，退出码 0 才完成，预算耗尽 replan | L84-86 |
| L13 | 扩展与多 agent：能力框架（declared→installed→enabled→ready）、能力门禁（run/ask-owner/repair-bridge/skip）、扩展 manifest + install/enable/disable/rollback + readiness doctor（v1 声明式不执行代码）、identity 范围多 agent、supervisor 提案/回执、任务租约、交接文档交付契约、todo 依赖图、attention queue/operator inbox | L88-108 |
| L14 | 诊断：benchmark（protocol/run/ledger）、replay（record/run、corpus）、canary（`core-control-plane`/`extension-runtime`/`release-gate`）；`version`/`doctor`/`history`/`turn`/`todo-event`/`evidence-log`、`backup`/恢复 | L110-113 |
| L15 | **CLI 一览**（命令面，逐条核验对象）：goal / todo / agent / capability / extension / ops / work-items / handoff / benchmark / replay / canary / cli（子命令详见原文 L115-127） | L115-127 |
| L16 | 技能模式快速开始：`/future-loop <目标>`；`future-loop run --max-turns 1` | L132-144 |
| L17 | 直连示例：`future-loop goal init --objective ... --cwd ...`；`todo add --goal <id> --text ... --priority P0 [--blocks ...] [--verify "test -f report.md"]`；`status --goal <id>`；`run --goal <id> --model future/deepseek-v4-flash --max-turns 1` | L148-155 |
| L18 | 状态布局：`registry.json`（真相源）、`goals/<id>/events.jsonl`、`goals/<id>/ACTIVE_GOAL_STATE.md`（参考兼容投影）、`runs/`；运行时状态不写出项目外；`.future/loop/` 加 .gitignore | L159-167 |
| L19 | 安装：`bash scripts/install-future-loop.sh` 或 `cargo build -p future-loop` | L171-173 |

## 6. docs/loop-control-plane.zh-CN.md（zh）

与 en 一致（L1-L19 镜像），行号：L7 定位、L11-14 loopx 改写、L40-49 目标/todos/门禁/监控、
L54-60 内核、L62-80 额度/调度/事件溯源、L84-86 独立验证、L88-108 扩展多 agent、
L110-113 诊断、L115-127 CLI 一览、L132-144 快速开始、L148-155 直连示例、
L159-167 状态布局、L171-173 安装。**无内容差异**。

---

## 7. docs/wiki-prompt.md（zh，生成提示词）

| # | 声明 | 位置 |
|---|---|---|
| W1 | 本文件是给 AI 的**生成提示词**，不是用户文档；可据此（重新）生成 docs/wiki/ 整套页面 | L1-6 |
| W2 | 读者是普通用户；**不暴露 gRPC/端口号/模块名**；反复强调「你始终掌控」 | L10-14 |
| W3 | 只写已实现功能；**当前不写：Research 入口、Data 入口、Remote/手机远程**（ActivityRail.tsx featureItems 空数组即隐藏） | L18-29 |
| W4 | 中英双语目录 en/ 与 zh/，文件名一一对应，**互不互链**、无语言切换链接 | L31-48 |
| W5 | **平台范围：只支持 macOS 和 Windows，不写 Linux** | L50 |
| W6 | 页面清单 10 页（Home/Installation/Quick-Start/Using-FutureOS/Settings/Skills/CLI/FAQ/_Sidebar/_Footer）——**无 Feishu/DingTalk** | L52-92 |
| W7 | 侧边栏结构（无 Integrations 分组） | L94-109 |
| W8 | Installation 参考内容：dmg / nsis / zip 产物；**命令行工具 `future` 随每个下载包附带**（装在应用旁边）；「正式发布的 macOS/Windows 安装包均已签名，macOS 同时完成 Apple 公证」；WebView2；.future 数据位置；Settings→检查更新 | L123-134 |
| W9 | Quick-Start 参考：FutureGene Connect 流程；New Chat / Workspace；**每轮最多 4 张图片（单张 25 MiB），非图片不限数量** | L135-145 |
| W10 | Settings 参考：General（Language / Approval mode 手动·沙盒[仅 macOS]·无限制 / Show thinking）；Providers（FutureGene Connect + 自定义 provider：id/名称/API 类型/Base URL/API key/模型列表，校验 id 唯一）；Models（按 provider 分组、可见性、可搜索） | L159-169 |
| W11 | Skills 参考：11 个内置技能表（Account/Web/Paper/Deep research/Document/Image/Browser/Hand-drawn posters/Hand-drawn slides/Subagent/Skill creator） | L150-158（技能表） |
| W12 | CLI 参考：**「命令名统一为 `future`……全文一律用 `future`，不要写成 `future-cli`」**（明确禁令）；macOS 位置 `/Applications/FutureOS.app/Contents/MacOS/future`；命令组 auth(login/status/logout)、agent(start/stop/restart/status)、run(--model 支持 model:thinking、--thinking、--continue/-c、--cwd、--mode json、--no-session、@<path>)、tools(list/call --args/--output/--stdin)、skills(list/install/uninstall；**没有 update**)、channel | L170-186 |
| W13 | FAQ 参考：macOS 打不开（右键打开 / `xattr -dr com.apple.quarantine`）；SmartScreen；WebView2；便携版同文件夹；未登录；切模型；批准机制（不超时）；.future 位置；更新；卸载；**平台 = macOS 和 Windows** | L188-201 |
| W14 | 自检：链接完整性、泄漏扫描（禁 Linux/.deb/.tar.gz/apt、TUI、gRPC/端口如 50051、Research/Data/Remote）、中英对齐、**CLI 名称用 `future`** | L209-219 |
| W15 | 禁止事项：不生成 TUI 页；不写隐藏功能；不写 Linux；不写发布/CI 维护流程；不暴露内部实现细节 | L221-229 |

## 7b. docs/wiki-prompt-en.md（en，生成提示词）

与 zh 大体对应，但**与 zh 存在方向性冲突**：

| # | 声明 | 位置 |
|---|---|---|
| WE1 | 页面清单中 CLI.md 标题为 **CLI (`future-cli`)**；侧边栏 `CLI (future-cli) → CLI` | L81, L104 |
| WE2 | 「The CLI tool **`future-cli`** ships with every download」 | L127 |
| WE3 | **「The release binary is named `future-cli`… dev-time `future` installed via npm link is only a development alias — user-facing wiki must always use `future-cli`, never `future`」**（与 zh W12 完全相反） | L170-171 |
| WE4 | macOS 位置 `.../MacOS/future-cli`；Windows `future-cli.exe` | L173-174 |
| WE5 | 自检第 4 条：「use `future-cli` throughout, never bare `future`」 | L219 |
| WE6 | 其余（页面清单、技能表、4 图/25MiB、命令组、FAQ、平台 macOS+Windows、无 Feishu/DingTalk 页）与 zh 一致 | L76-229 |

---

## 8. docs/wiki/en/*.md（13 个页面，关键声明）

### Home.md（40 行）
| # | 声明 | 位置 |
|---|---|---|
| EH1 | 定位：desktop AI agent workbench，可看可核对 agent 工作 | L1-4 |
| EH2 | 三步开始：Installation / Quick-Start / Using-FutureOS（wiki 内链 `[[...]]`） | L14-19 |
| EH3 | 你能做什么：流式思考/工具调用；Chat 或绑定文件夹 Workspace；风险操作前暂停批准；右侧 Files/Runs/Review（✅ 2026-08-06 由 Artifacts 改）；Skills 自动使用 | L23-29 |
| EH4 | 底部：**runs on macOS and Windows** | L40 |

### Installation.md（77 行）
| # | 声明 | 位置 |
|---|---|---|
| EI1 | 平台：macOS and Windows | L3 |
| EI2 | 下载：macOS `.dmg`；Windows 安装器 `.exe` 或便携 `.zip`；Releases 链接 | L10-17 |
| EI3 | **命令行工具 `future` 随每个下载包附带**（安装包与便携包都有，装在应用旁边） | L19-21 |
| EI4 | **「Formal macOS and Windows installers are signed, and the macOS build is also notarized by Apple」**（已签名+已公证） | L24 |
| EI5 | macOS 首启：拖入 Applications → 双击 | L27-30 |
| EI6 | Windows：安装版跑 .exe；**便携版 FutureOS.exe 与 future-agent.exe 同文件夹**；SmartScreen 提示核对发布者；**需要 WebView2 Runtime**（Win10 近期/Win11 一般内置，缺则装 Evergreen）；zip「来自 Internet」标记 → 右键属性解除锁定 / `Get-ChildItem -Recurse | Unblock-File` | L32-42 |
| EI7 | 数据位置：macOS `~/.future`，Windows `C:\Users\<you>\.future` | L49-54 |
| EI8 | 更新：Settings → Check for updates（签名验证）；便携版替换文件夹；.future 保留 | L56-59 |
| EI9 | 卸载：macOS 删 FutureOS.app；Windows 设置卸载或删便携文件夹 | L61-65 |

### Quick-Start.md（71 行）
| # | 声明 | 位置 |
|---|---|---|
| EQ1 | 登录流程：齿轮图标 → Providers → Built-in → FutureGene → **Sign in**（✅ 原「Connect」，按钮实际文案为 Sign in）→ 浏览器授权（不自动开则用验证码+可复制链接） | L9-19 |
| EQ2 | New Chat vs Workspace 对比表 | L24-35 |
| EQ3 | 发送：流式回复、工具活动展示、风险操作暂停批准；**每消息最多 4 张图片（25 MiB each），其他文件不限数量**；回形针/拖拽/粘贴 | L38-48 |
| EQ4 | 模型选择器在输入框内，旁边有 thinking level 控制；Settings → Models 管理 | L50-56 |
| EQ5 | 右侧面板：每个会话 Files + Runs；Workspace 另有 Review（✅ 2026-08-06 重写，Artifacts 已停用） | L58-66 |

### Using-FutureOS.md（91 行）
| # | 声明 | 位置 |
|---|---|---|
| EU1 | 三栏布局：左导航（New Chat、Models 快捷入口、Skills、Workspaces、Chats、Settings，可折叠）；中对话区（流式回复/计划/工具活动/命令预览/错误/批准卡片，输入框固定底部）；右上下文面板（Files/Runs/Review，✅ 原 Runs/Review/Artifacts，可折叠） | L6-18 |
| EU2 | Chat vs Workspace 表：Chat 右栏显示 Files+Runs（✅ 原 Runs+Artifacts）；Workspace 显示 Files+Runs+Review；每个会话独立 | L22-34 |
| EU3 | 重命名/置顶/删除会话（左栏菜单） | L36-37 |
| EU4 | 对话：Enter 发送、Shift+Enter 换行；逐会话切模型；附件限制同 EQ3；流式时发送按钮变停止 | L39-46 |
| EU5 | **批准机制**：读写文件/跑 shell/删除/写 workspace 之外 → 停止 + 批准卡片 + **不超时**；Allow once / Deny / Allow in this workspace or chat（可编辑路径规则）；**键盘 Cmd/Ctrl+Enter 批准、Esc 拒绝** | L48-64 |
| EU6 | **批准模式**：Manual（读写文件前询问，只读命令自动跑）/ **Sandboxed（macOS only）** / Unrestricted；可在 Settings → General 或输入框盾牌控件设置 | L66-72 |
| EU7 | Runs：卡片显示真实命令/状态/计数；Inspect/Terminate/Clear finished | L76-81 |
| EU8 | Review：文件列表、类型（added/modified/deleted/renamed）、逐文件 diff；版本控制下可切「Last run changes」 | L83-85 |
| EU9 | Files：每个会话展示工作区文件（Workspace=项目文件夹，Chat=临时会话文件夹）；预览/系统打开/从目录树附加到对话（✅ 2026-08-06 替换原 Artifacts 小节） | L87-89 |

### Settings.md（82 行）
| # | 声明 | 位置 |
|---|---|---|
| ES1 | 入口：左下齿轮；左栏 Models 快捷入口 | L3 |
| ES2 | 设置页：General / Providers / Models + **Check for updates / Reset** | L5 |
| ES3 | General：Language、Approval mode（Manual/Sandboxed[macOS only]/Unrestricted）、Show thinking process、**Auto-upgrade skills**（✅ 2026-08-06 补，应用打开时静默升级已装技能） | L9-18 |
| ES4 | Providers：FutureGene 内置 **Sign in**（✅ 原 Connect；验证码+链接；登录态只有 Sign out，无 Sign in again）；**其他内置 provider（DeepSeek/OpenAI/Anthropic/Google 等）点 Configure**（✅ 原 Set key/Update key，对话框标题 Set <provider> key）；More providers 展开完整列表 | L22-32 |
| ES5 | 自定义 provider 字段：Name（可选）、Provider ID（小写字母/数字/-/_）、**API type = OpenAI Completions / OpenAI Responses / Anthropic**、Base URL、API Key、Models（可带显示名）；校验 + id 唯一；Edit/Remove | L34-45 |
| ES6 | Models：按 provider 分组、搜索、可见性开关；输入框选择器同源并显示 provider | L48-54 |
| ES7 | Check for updates：检查并下载对应系统安装包 | L57-59 |
| ES8 | Reset：Clear local data 清本地数据并重启 | L61-64 |

### Skills.md（56 行）
| # | 声明 | 位置 |
|---|---|---|
| ESK1 | Skills = 能力包，安装且相关时自动使用；左栏 Skills 入口 | L3-6 |
| ESK2 | 两标签：Installed / All（All 需联网）；分类下拉 + 搜索；Install/Uninstall | L9-14 |
| ESK3 | **14 个内置技能表**（✅ 2026-08-06 修正，原 11 项含 3 个不存在的 Hand-drawn posters/Hand-drawn slides/Subagent）：Account、Browser、Database lookup、Deep research、Document、Experimental design、Image、Paper、Peer review、Scientific writing、Skill creator、Slides、Software install、Web | L18-30 |
| ESK4 | 使用方式：无需手动调用，描述任务即可 | L38-44 |

### CLI.md（127 行）
| # | 声明 | 位置 |
|---|---|---|
| EC1 | 工具名 **`future`**，随每个下载包附带；「你多半用不到」 | L1-5 |
| EC2 | 位置：macOS（.dmg）`/Applications/FutureOS.app/Contents/MacOS/future`；Windows（便携 .zip）`future.exe`；**Windows 命令行工具只在便携包，安装版没有 future.exe** | L9-18 |
| EC3 | 运行：`future --help`；可加 PATH / 别名（macOS alias 示例） | L21-27 |
| EC4 | **agent 必须在运行**：桌面应用开着则已运行，否则 `future agent start` | L29-35 |
| EC5 | 命令组：auth（login/status/logout）、agent（start/stop/restart/status）、run（--model 支持 `model:thinking` 如 `sonnet:high`、--thinking off/minimal/low/medium/high/xhigh、@<path>、--continue/-c、--cwd、--mode json、--no-session；示例含管道输入）、tools（list / call --args/--stdin/--output，文件路径参数自动转换）、skills（list/install/uninstall）、channel（start/stop/restart/status，进阶） | L37-93 |
| EC6 | 小贴士：macOS 首次被拦 → 右键打开应用一次；Connection refused → `future agent start` 或打开桌面应用 | L95-99 |

### FAQ.md（65 行）
| # | 声明 | 位置 |
|---|---|---|
| EF1 | **「The current build isn't notarized, so this is expected」**（当前版本未公证 —— 与 Installation L24「已公证」冲突） | L9 |
| EF2 | macOS 打不开：右键打开两次；已损坏 → `xattr -dr com.apple.quarantine /Applications/FutureOS.app` | L10-15 |
| EF3 | SmartScreen：More info → Run anyway | L18-19 |
| EF4 | Windows 没反应：装 WebView2 Evergreen；便携版同文件夹；zip 解除锁定 | L21-28 |
| EF5 | 用不了模型/未登录：Settings → Providers → FutureGene → **Sign in**（✅ 原 Connect） | L30-32 |
| EF6 | 切模型：输入框选择器 / Settings → Models | L34-35 |
| EF7 | agent 停下询问 = 批准机制（不超时）：Allow once/Deny/允许本项目 | L37-39 |
| EF8 | 数据位置：~/.future / C:\Users\<you>\.future | L41-45 |
| EF9 | 更新：下载覆盖安装 / 替换文件夹；.future 保留；Settings → Check for updates | L47-49 |
| EF10 | 卸载/清数据：删应用 + 删 .future；Settings → Reset 也可 | L51-53 |
| EF11 | 平台：**macOS and Windows** | L55-57 |

### Feishu.md（204 行）
| # | 声明 | 位置 |
|---|---|---|
| EFE1 | 架构图：Bridge (WebSocket) ↔ Agent (gRPC 127.0.0.1:50051) | L8-14 |
| EFE2 | 前提：飞书开发者账号（open.feishu.cn / open.larksuite.com）、机器人能力应用、**agent 已运行（`make run-agent` 或 `future agent start`）** | L17-22 |
| EFE3 | 创建应用步骤：企业自建应用 → 启用 Bot → 记 App ID/App Secret | L24-31 |
| EFE4 | 权限：im:message、im:message.p2p_msg:read、im:message.group_msg:read、im:message:send_as_bot、im:resource、contact:user.base:read | L33-42 |
| EFE5 | 事件订阅：im.message.receive_v1；Request URL 任意 HTTPS（WebSocket 不实际回调但必填） | L44-49 |
| EFE6 | config.json：agent{grpc_addr(默认 http://127.0.0.1:50051)、cwd、model(future/deepseek-v4-pro)、thinking_level(off..xhigh)、permission_level(all/workspace/none)}；feishu{enabled、app_id、app_secret、domain("feishu"/"lark")} | L51-87 |
| EFE7 | 策略：dm_policy(open/allowlist[默认]/disabled)、dm_allowlist、group_policy(disabled[默认])、group_allowlist、require_mention(默认 true) | L89-116 |
| EFE8 | 行为：streaming(true)、resolve_sender_names(true)、max_image_mb(10)、typing_indicator(false) | L118-126 |
| EFE9 | 启动：**`make build-channels-release`** + `./target/release/future-channels`；服务管理 `future channel start/status/stop/restart`（macOS launchctl / Linux systemd）；config.json 不存在则创建模板并退出 | L128-143 |
| EFE10 | 斜杠命令 9 个：/new /status /model /models /effort /stop /compact /cwd /help；本地处理不经过 agent；无法识别转发为普通消息 | L145-156 |
| EFE11 | 回复效果：流式（默认）CardKit 实时卡片 + 可折叠引用块；非流式单条 markdown | L158-163 |
| EFE12 | 排障：机器人不回复（bridge 状态/enabled/策略/日志）；**每 6 分钟重连（30s keepalive ping）**；图片（im:resource + max_image_mb） | L165-182 |
| EFE13 | 末尾 See also 链接文字为 **`[[CLI (future-cli)|CLI]]`**（与 CLI.md 页标题「future」不一致） | L204 |

### DingTalk.md（170 行）
| # | 声明 | 位置 |
|---|---|---|
| EDT1 | 架构图：Bridge (Stream Mode) ↔ Agent (gRPC 127.0.0.1:50051)；api.dingtalk.com | L8-14 |
| EDT2 | 前提：钉钉开发者账号（open.dingtalk.com）、Stream Mode 应用、agent 已运行（`make run-agent`/`future agent start`） | L17-22 |
| EDT3 | 创建应用：open-dev.dingtalk.com → 机器人 → 消息接收模式 = Stream Mode → Client ID(AppKey)/Client Secret(AppSecret) | L24-30 |
| EDT4 | 权限：im.message.receive、im.message.send、qyapi_robot_webhook_message_send | L32-39 |
| EDT5 | config.json：dingtalk{enabled、client_id、client_secret、domain(默认 api.dingtalk.com)}（agent 块同 EFE6） | L41-70 |
| EDT6 | 启动：`make build-channels-release` + `./target/release/future-channels`；`future channel start/status/stop/restart` | L72-89 |
| EDT7 | 斜杠命令 9 个（同 EFE10；**DingTalk 版声明「所有斜杠命令均由 Bridge 本地处理」**，与 Feishu 版措辞不同） | L91-103 |
| EDT8 | 回复效果：markdown 经 session webhook；**每条回复是新消息（webhook 不支持原地编辑）**；思考 `> 💭` 引用块 | L105-111 |
| EDT9 | 与飞书区别表：连接（pbbp2 protobuf vs Stream Mode JSON）、流式（CardKit vs 新消息）、思考链、Emoji 反馈（✅ vs ❌ API 未公开）、多模态（图片/文件 vs 仅文本 markdown） | L113-120 |
| EDT10 | 排障：机器人不回复；**频繁重连（keepalive 每 20 秒）**；markdown 双换行 | L122-135 |
| EDT11 | 末尾 See also 链接文字 `[[CLI (future-cli)|CLI]]`（同 EFE13 不一致） | L170 |

### _Sidebar.md（21 行）
| # | 声明 | 位置 |
|---|---|---|
| ESB1 | 分组：Getting started（Install/Quick Start）、Using the app（Using FutureOS/Settings/Skills）、Command line（**CLI (future)**）、**Integrations（Feishu/DingTalk）**、Help（FAQ） | L4-20 |
| ESB2 | 链接文字「CLI (future)」→ CLI（用 `future`，与 prompt en 的 future-cli 冲突） | L14 |

### _Footer.md（3 行）
| # | 声明 | 位置 |
|---|---|---|
| ESF1 | macOS and Windows；Download / Report an issue 外链 | L3 |

### Models.md（en/zh 各 4983 行）—— **生成文件**
| # | 声明 | 位置 |
|---|---|---|
| EM1 | 头部声明「**3826 models across 143 providers**」/「3826 个模型，覆盖 143 个 Provider」 | L3 |
| EM2 | 生成机制：`scripts/generate_models.py`（`make generate-models`），数据源 models.dev / openrouter / vercel；脚本内 `generate_wiki_docs()` 直接写 `docs/wiki/{en,zh}/Models.md` | scripts/generate_models.py L231-331 |
| EM3 | 结构：Provider Summary 表 + 每 provider 一节（Base URL + Model ID/Name/Context/Max Output/Image/Reasoning 表） | L5 起 |
| EM4 | **README 声称「1000+ 模型 / 100+ providers」与此处 3826/143 不一致**（README 数据疑似过时，需核验） | README L26 vs Models L3 |

---

## 9. docs/wiki/zh/*.md（zh，与 en 对应的关键行）

zh 页面与 en 内容一致，行号基本镜像（zh 多 1 行于 Feishu L205、DingTalk L170、_Sidebar 用「命令行工具(future)」）。
需要特别记录的两处：

| # | 声明 | 位置 |
|---|---|---|
| ZI1 | Installation.md：「正式发布的 macOS 与 Windows 安装包均经过签名，**macOS 版本同时经过 Apple 公证**」 | zh/Installation.md L24 |
| ZF1 | FAQ.md：「**当前版本未公证**,这属于正常现象。」（与 Installation L24 冲突） | zh/FAQ.md L9 |
| ZC1 | CLI.md：工具名 `future`；macOS `.../MacOS/future`；Windows 便携包 `future.exe` | zh/CLI.md L1, L15-16 |
| ZC2 | Feishu.md / DingTalk.md 末尾 See also 链接文字「命令行工具(**future-cli**)」 | zh/Feishu.md L205, zh/DingTalk.md L170 |
| ZQ1 | Quick-Start.md / Using-FutureOS.md：「每条消息最多附 **4 张图片**（每张最大 25 MiB），其他文件类型不限制数量」 | zh/Quick-Start.md L45, zh/Using-FutureOS.md L38 |
| ZM1 | Models.md「3826 个模型，覆盖 143 个 Provider」 | zh/Models.md L3 |

---

## 10. docs/architecture-audit/（4 份审计 + README，2026-08-05 生成）

> ⚠️ **时点快照，已标注历史（2026-08-06，todo_41f779819879）**：审计基准 dev @ 8aa82925（2026-08-05）；文档与首批修复同批入库（commit `306cf05f`）——报告 01 H2/H3/H4、报告 04 H1/H2/H4 已在该 commit 修复；仍成立高危项为报告 01 H1/H5、报告 04 H3。各报告相关条目已标 ✅；README 顶部时效性说明详述 file:line 漂移与修复清单。

| # | 声明 | 位置 |
|---|---|---|
| A0 | 审计基准：`dev @ 8aa82925`（2026-08-05）；调查在工作树 `8164b8e1` 进行，两树内容一致（diff 为空）；报告 file:line 在**审计时点**（2026-08-05）可用——⚠️ 2026-08-06 起已漂移（见本表顶部警示） | audit/README.md L8-9 |
| A0b | 四报告主题与一句话结论表 | audit/README.md L5-7 |
| A1 | 报告 01 结论：agent ↔ gui_rust 边界**泄漏且双向**：影子 JSON 契约 + 7 条文件系统旁路 + 编译期源码 include（`#[path]`）；H1-H5 / M1-M7 / L1-L5 详情 | audit/01 L1-7, L15-119 |
| A1b | 报告 01 最强证据：RpcResponse.data/StreamEvent.data 为 JSON 字符串（proto:220/392）；get_state 返回 ~35 键 ad-hoc JSON（agent/src/rpc/mod.rs:339-376）；GUI 写 auth.json/models.json（auth_store.rs:84-149、write.rs:92-258）；`#[path="../../../../agent/src/models/builtin/mod.rs"]`（catalog.rs:15-16）；cleanup.rs:173-241 探测 `{id}.jsonl` | audit/01 L32-54, L143-152 |
| A2 | 报告 02 结论：gui_rust ↔ gui_react 边界架构干净（103 个 #[tauri::command] 全注册于 lib.rs:600-704；8 个事件；invokeCommand 102 处 0 裸 invoke），但契约全手工同步（39+ 对类型，3 处已漂移）；S1-S8 | audit/02 L1-9, L15-52 |
| A3 | 报告 03 结论：18 个超大模块候选：3 个 Tier1（agent_bridge/mod.rs 1343 行、session/mod.rs 3624 行、Composer.tsx 704 行）、9 个 Tier2、6 个内聚不拆 | audit/03 L1-9, L14-33 |
| A4 | 报告 04 结论：React 流式热路径 4 个 HIGH（H1 handleFork 依赖 messages 击穿唯一 memo；H2 threadRunStatuses reducer 从不 bail-out → AppShell 25Hz 全树渲染；H3 流式尾部 markdown 全量重解析 O(n²)；H4 Composer+MentionEditor 每推送重渲染）；后端 40ms 推送合并（lib.rs:286-330）≈25 次/秒 | audit/04 L1-9, L18-61 |
| A5 | 审计均为只读，未修改文件；「可分别修复」 | audit/README.md L3 |

---

## 11. docs/dist/*.txt（发布包内附说明，6 个）

> ✅ **已核验（2026-08-06，todo_41f779819879）**：全部声明与当前源码一致，**无需修改**。这些文件是活文档——build.yml（macOS dmg / Windows portable / Linux portable）与 scripts/build-windows-portable.ps1、build-windows-signed.yml 打包时逐字复制为各包内 `Readme.txt`（`-en.txt` 为参考译文，打包只用 zh `.txt`）。二进制三件套（`futureos`/`FutureOS.exe` + `future-agent`/`future-agent.exe` + `future`/`future.exe`）与 build.yml 装配步骤逐条吻合（L239-301）；macOS「not Apple-notarised」与 FAQ/B8 口径一致；WebView2/WebKitGTK 运行时要求与 Tauri 默认一致；`~/.future` / `C:\Users\<用户名>\.future` 数据目录正确。

| # | 声明 | 位置 |
|---|---|---|
| D1 | macOS：**「This build is not Apple-notarised」**（未公证）；拖入 Applications；右键打开 / `xattr -dr com.apple.quarantine`；~/.future；future 位于 FutureOS.app/Contents/MacOS/future | readme-macos-en.txt L6-22；readme-macos.txt 对应 |
| D2 | Windows：便携 zip；**FutureOS.exe 与 future-agent.exe 同文件夹**；SmartScreen More info→Run anyway；zip 解除锁定 / `Get-ChildItem -Recurse | Unblock-File`；WebView2 Evergreen；`C:\Users\<username>\.future`；future.exe 同目录 | readme-windows-en.txt L4-31；readme-windows.txt 对应 |
| D3 | Linux：便携 tar.gz（`tar -xzf FutureOS-portable-linux.tar.gz` + `./futureos`）；**futureos、future-agent、future 同文件夹**；WebKitGTK（Debian/Ubuntu `libwebkit2gtk-4.1-0`，Fedora `webkit2gtk4.1`）；~/.future | readme-linux-en.txt L4-22；readme-linux.txt 对应 |
| D4 | 三平台说明均称命令行工具名为 `future`（Linux/macOS/Windows 一致） | 各文件 Notes |

---

## 12. 跨文档冲突 / 待核验观察（仅描述，不判定）

> 以下冲突已在文档间确认存在，具体哪边正确由「对照源码核验」步骤判定。

| # | 主题 | 冲突双方 | 位置 |
|---|---|---|---|
| X1 | **CLI 二进制名** | `future`：wiki-prompt.md W12（L171/L219）、README R3、wiki CLI.md EC1-EC2、_Sidebar ESB1、dist D4；`future-cli`：wiki-prompt-en.md WE3-WE5（L171/L219，且明确「never bare future」） | 见左 |
| X2 | **CLI 名称在 wiki 内部也不一致** | wiki CLI.md/侧边栏用 `future`，但 Feishu.md/DingTalk.md 末尾 See also 用 `future-cli`（en L204/L170，zh L205/L170） | §8/§9 |
| X3 | **macOS 公证状态** | 已公证：wiki Installation en L24 / zh L24、wiki-prompt W8；未公证：wiki FAQ en L9 / zh L9（「current build isn't notarized」）、dist readme-macos-en L8（「not Apple-notarised」） | §8/§9/§11 |
| X4 | **skills 是否有 `update` 子命令** | 有：build-and-install.md B14（L165 `future skills update`）、zh-CN L163；没有：wiki-prompt W12（L183「**没有 update**」）、WE（L183「no `update`」） | §3/§7 |\n| — | **X4 已裁决（2026-08-06，todo_cbbb063d2fd4）** | cli/src/commands/skills.ts L19/L48-49/L88/L287-328 已实现 `update`（updateSkills 真实执行升级）。**build-and-install 正确，wiki-prompt W12/WE 错误**——留待 wiki-prompt todo 修正 | |
| X5 | **模型数量** | README「1000+ models / 100+ providers」（R2 L26）；Models.md「3826 models / 143 providers」（EM1 L3） | §1/§8 |
| X6 | **wiki-prompt 页面清单 vs 实际 wiki** | prompt 清单 10 页无 Feishu/DingTalk、侧边栏无 Integrations（W6/W7）；实际 wiki 有 Feishu.md+DingTalk.md+Integrations 分组（ESB1） | §7/§8 |
| X7 | **make 目标** | wiki Feishu/DingTalk 用 `make build-channels-release`（EFE9/EDT6）；Makefile 中无该目标（grep 0 命中）——疑似应为 `make build-channels` 或 release 变体，需核验 | §8；Makefile |
| X8 | **沙箱术语** | README「sandbox tiers (off / manual / macOS Seatbelt)」（R9）；wiki「Manual / Sandboxed (macOS only) / Unrestricted」（EU6/ES3）；wiki-prompt W10「手动 / 沙盒[仅 macOS] / 无限制」 | §1/§8/§7 |
| X9 | **思考级别集合** | README「off ↔ xhigh」；wiki CLI.md `--thinking` 列 off/minimal/low/medium/high/xhigh（EC5）；README 未列全级别 | §1/§8 |
| X10 | **architecture-audit 时效** | 审计基准 dev @ 8aa82925（2026-08-05）与当前工作树关系需核验；若源码已变，file:line 可能失效 | §10-A0 |
| — | **X10 已裁决（2026-08-06，todo_41f779819879）** | 审计为**时点快照**：文档与首批修复同批入库（commit `306cf05f`）——报告 01 H2/H3/H4、报告 04 H1/H2/H4 已在该 commit 修复（代码注释引用审计编号）；仍成立高危项：报告 01 H1/H5、报告 04 H3。file:line 已漂移，README 顶部已加时效性说明并逐条标注 ✅（详见 errors-outdated-missing.md §E） | |
| X11 | **loop CLI 命令面** | loop-control-plane.md L15 CLI 一览 vs 实际 `future-loop --help`（本轮已见 goal/todo/ops/cli 等组在跑，完整命令面需核验） | §5 |
| — | **X11 已裁决（2026-08-06，todo_63c718c2a3d5）** | 与 `build_cli_registry()`（main.rs L176-471）逐一比对：goal 组缺 `models`/`diagnose`、extension 缺 `upgrade`、`cli registry` 缺 `--include-experimental`，已修正 en/zh 两份 CLI 一览（B9 详录）。其余全部一致 | |
| X12 | **`future init` 行为** | build-and-install B14：`future init` = 安装技能 + macOS/Linux 链接本地命令；需核验 cli 源码 | §3 |

---

## 13. 供核验的关键声明索引（按主题，对应核验 todo 的核验面）

| 核验面 | 涉及声明 | 文件:行 |
|---|---|---|
| TUI 斜杠命令（12 个） | R6 / RZ4 | README.md L100-114 |
| TUI 快捷键（8 个） | R7 / RZ5 | README.md L118-130 |
| agent 端口 127.0.0.1:50051 | R4 / RZ3 / EFE6 / EDT5 | README L89；wiki Feishu L70-87、DingTalk L56-73 |
| 配置路径 ~/.future/agent/{auth,models}.json、~/.future/channels/config.json、~/.future/loop/ | R3 / EFE6 / EDT5 / L18 | README L46-79；wiki Feishu L51-87 |
| 工具链版本：Rust 1.97+、Node 24+、Bun | B1 / B3-B8 | build-and-install L12-18, L30-86 |
| .cargo/config.toml mold 声明 | B6 | build-and-install L67；.cargo/config.toml |
| Makefile 目标面（install/install-nogui/package-gui/build-*/run-*/generate-*/install-skills/install-loop…） | B4-B17 / X7 | build-and-install 全文；Makefile |
| CLI 命令面（auth/agent/run/tools/skills/channel + 各选项） | EC5 / W12 / X1/X2/X4 | wiki CLI.md L37-93；wiki-prompt L175-185 |
| channels 配置与斜杠命令 | EFE6-EFE10 / EDT5-EDT7 | wiki Feishu/DingTalk |
| future-loop CLI 命令面 | L15 / X11 | loop-control-plane.md L115-127 |
| proto（future.proto、生成代码、generate-proto） | B17 | build-and-install L186-206 |
| GUI 功能声明（批准机制/设置页/技能清单/4图25MiB/Artifacts 等） | EU5-EU9 / ES3-ES8 / ESK3 / EQ3 | wiki Using-FutureOS/Settings/Skills/Quick-Start |
| 生成文件（Models.md） | EM1-EM3 | scripts/generate_models.py L231-331 |

---

## 14. 通读阶段遗留观察（来自被取代的 docs-verification/DOC-FACTS.md，commit 3ed92ad7）

> 早期通读草案（346 行）后被本文件（400 行，§0-13 重构版）取代。以下条目在该草案中
> 存在、且未被 §A-E 完全吸收，仍为开放项，供后继 todo（尤其 todo_9bb2c6dd1c38 补缺）参考。

### 14a. 未完全吸收的待核验观察

| # | 观察 | 状态 |
|---|---|---|
| H11 | Feishu 权限 scope 表：wiki 列 6 行（含 contact:user.base:read），旧草案疑「5 个」——以实际 wiki 页为准（wiki 页核验属 todo_bcf715c7cc0e） | 开放 |
| H14 | 读文件是否要求批准：Using-FutureOS 批准机制写「read or write a file」需批准，而 Manual 模式又写「read-only commands run automatically」——内部一致性属 GUI 核验面（见 errors-outdated-missing.md §E） | ✅ 已解决（todo_cab9a84ced24）：**一致，无矛盾**——file read 工具访问触发 file_read 审批（approval.rs），而 Manual 模式下「read-only commands」指**只读 shell 命令**自动放行（shell_auto_allow 分类），两者对象不同 |
| H15 | wiki-prompt §7 引用的 GUI 源文件（ActivityRail.tsx featureItems、SettingsDialog.tsx、Composer.tsx）与审计报告 02/03 行号需交叉确认存在 | ✅ 已解决（todo_cab9a84ced24）：三文件均存在——ActivityRail.tsx featureItems 为空数组（Research 已移除，PRODUCT.md §4.9）、SettingsDialog.tsx devOnly 机制（Remote/Environment 仅 dev）、Composer.tsx 盾牌/模型/思考级别/停止钮 |

### 14b. 「缺失文档/章节」候选（供 todo_9bb2c6dd1c38 参考）

| # | 候选缺口 | 依据 | 状态（todo_9bb2c6dd1c38） |
|---|---|---|---|
| I-1 | Models.md 缺「如何再生成」说明（生成器脚本、命令、是否自动同步） | scripts/generate_models.py；EM1-EM3 | ✅ 已解决（todo_bcf715c7cc0e：脚本+文件头注释） |
| I-2 | dist readme 有 Linux 版，但 wiki 无 Linux 页面——产品平台口径需决策（加 Linux 页 vs 维持 macOS+Windows） | docs/dist/readme-linux*.txt | ✅ 已裁决：**维持 macOS+Windows**——release.yml 只发布 macOS（arm64+x64 dmg/updater）+ Windows（x64 setup），Linux 便携包为 tester-only，无需 wiki 页 |
| I-3 | 无 channels 配置的完整参考文档（Feishu/DingTalk 各自成篇，无统一 config.json schema 参考页） | EFE6/EDT5 | ✅ 已解决：新建 docs/channels-config.md + zh（schema 全字段/默认值，依据 channels/src/config.rs 逐字段核实） |
| I-4 | loop-control-plane 指南未覆盖 `agent` 命令组（onboard/scope/lane/supervisor）与 handoff 用法示例 | loop-control-plane.md L115-127 | ✅ 已解决：en/zh 新增「Multi-agent workflow / 多 agent 工作流」章节（agent onboard/注册、scope、lane、supervisor propose|receipt|events、handoff [--write]、task-graph、attention/inbox 示例）；并注明这些为扁平顶层命令（帮助里的 agent/todo/work-items 分组仅为展示） |
| I-5 | docs/ 顶层无索引 README（除 architecture-audit 外）——docs 目录缺导航 | 目录结构 | ✅ 已解决：新建 docs/README.md + zh（顶层指南表、wiki 清单、dist 说明、内部工作文档） |
| I-6 | 无 TUI 使用文档（wiki 刻意不含 TUI，但 README 有 TUI 斜杠命令/快捷键） | README R6/R7 | ✅ 已解决：新建 docs/tui.md + zh（17 个斜杠命令、8 快捷键、settings/keybindings/log 路径、排障） |
| I-7 | 无 `.future/` 目录完整布局文档（agent/channels/tui/app/workspaces 各子目录职责） | CLAUDE.md；配置路径核验面 | ✅ 已解决：新建 docs/directory-layout.md + zh（agent/models/auth/sessions/skills/logs、channels、tui、app(db/images/review)、workspaces/chat、loop、bin；另注 ~/.agents/skills 与项目本地 .future/） |
