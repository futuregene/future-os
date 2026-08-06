# 文档事实清单（Document Fact Inventory）

> 目标 goal_fa74d6a794b5 的 P0 交付物：通读 README + docs 下所有文档，逐条提取**关键声明 + 位置**，
> 作为后续对照源码核验（todo_8bd559237c7d）、修正（todo_5d852f73fcb6 等）、补缺（todo_9bb2c6dd1c38）的依据。
>
> - 建立时间：2026-08-06（worktree 分支 `claude/loop-hardening`，HEAD `c36d71fa`）
- 覆盖范围：`README.md`、`README.zh-CN.md`、`docs/` 下全部 `.md` 与 `docs/dist/*.txt`
- 标注：`[事实 ID] 声明内容 — 位置（file:line）`；`⚠️` = 阅读时已发现的文档间冲突或疑似过时（待源码核验确认）
- 行号以当前工作树为准（各文件的行号已用 grep 复核）

---

## A. README.md / README.zh-CN.md（顶部入口文档）

### A1. README.md（143 行）

| ID | 事实（声明） | 位置 |
|---|---|---|
| A1-1 | 定位："local-first AI agent workspace — terminal, desktop, messaging platforms, all through one backend"；统一体验覆盖 TUI、GUI、CLI、Feishu、DingTalk，支持 macOS/Linux/Windows | README.md:14-18 |
| A1-2 | 特性表共 9 行：Multi-Interface / Model Flexibility / Streaming & Thinking / Tool Execution / Session Persistence / Skills System / Compaction & Retry / Loop Control Plane / Rust Core | README.md:22-33 |
| A1-3 | **"1000+ built-in models across 100+ providers"** — ⚠️ 与 Models.md 的 "3826 models across 143 providers"（docs/wiki/en/Models.md:2）不一致，疑似过时 | README.md:26 |
| A1-4 | 思考级别 "off ↔ xhigh" | README.md:27 |
| A1-5 | 会话 JSONL 存储，支持 fork/clone/tree 导航/问答计数 | README.md:29 |
| A1-6 | Loop Control Plane：`future-loop`，是 [loopx] 的 **Rust 重写版**（"a Rust rewrite of the loopx control plane"） | README.md:32 |
| A1-7 | 安装：预编译安装包/脚本，无需源码构建；指向 docs/build-and-install.md | README.md:37-41 |
| A1-8 | 配置模型三方式：A) `future auth login`（设备码）；B) `~/.future/agent/auth.json` 按 provider 名索引 `{"openai": {"type":"api_key","key":...}}`，Azure 可加 `baseUrl`；C) `~/.future/agent/models.json` 自定义 provider（`apiKey`/`baseUrl`/`models[{id,name,contextWindow}]`） | README.md:45-83 |
| A1-9 | **"The agent must be running first, listening on 127.0.0.1:50051"**；`future-agent` 前台启动（日志到 stdout，Ctrl-C 停）；`future-tui` 启动终端 UI；"terminal and CLI clients are thin gRPC clients" | README.md:86-98 |
| A1-10 | TUI 斜杠命令 13 个：`/help /model /new /sessions /compact /scoped-models /clone /fork /tree /name /status /stop`（+模型名参数） | README.md:104-118 |
| A1-11 | TUI 快捷键：`ctrl+p` cycle model、`ctrl+t` cycle thinking、`ctrl+r` browse sessions、`ctrl+c` interrupt/exit、`tab` autocomplete、`enter` submit、`escape` close popup、`↑↓` scroll | README.md:121-131 |
| A1-12 | 故障排查：连接/gRPC 错误 → agent 未启动，`lsof -i :50051` 查端口；鉴权/"no model"错误 → 未配置模型；构建问题 → build-and-install | README.md:134-141 |
| A1-13 | License: MIT | README.md:143 |
| A1-14 | 徽章链接：wiki（github.com/futuregene/future-os/wiki）、LICENSE、future-skills 仓库 | README.md:2-8 |

### A2. README.zh-CN.md（138 行）

| ID | 事实（声明） | 位置 |
|---|---|---|
| A2-1 | 与英文版结构一一对应（定位/特性表/快速开始/斜杠命令/快捷键/故障排查），仅语言不同 | README.zh-CN.md 全文 |
| A2-2 | "内置 1000+ 模型，覆盖 100+ Provider"（同 A1-3，⚠️ 疑似过时） | README.zh-CN.md:26 |
| A2-3 | `future-loop` 为 loopx 的 Rust 改写版，链接 docs/loop-control-plane.zh-CN.md | README.zh-CN.md:32 |
| A2-4 | gRPC 端口 127.0.0.1:50051；`future-agent`/`future-tui`；auth.json/models.json 示例 | README.zh-CN.md:86-98 |

---

## B. docs/build-and-install.md / build-and-install.zh-CN.md（构建安装）

### B1. build-and-install.md（206 行）

| ID | 事实（声明） | 位置 |
|---|---|---|
| B1-1 | 覆盖 agent 后端、TUI/CLI 前端、桌面 GUI、channel bridge、loop 控制面（future-loop） | build-and-install.md:3-5 |
| B1-2 | 前置：**Rust 1.97+**（rust-toolchain.toml 固定）、**Node.js 24+**（.nvmrc）、**Bun 必需**（TUI 构建 + CLI/GUI 打包用 `bun build`）；可选 Python3（仅 `make generate-models`）、protoc（仅 `make generate-proto`，生成代码已入库） | build-and-install.md:13-18 |
| B1-3 | macOS：`xcode-select --install`、rustup、`brew install node oven-sh/bun/bun`、可选 protobuf | build-and-install.md:25-36 |
| B1-4 | macOS 构建目标：`make install`（→/opt/homebrew/bin）、`make install-nogui`、`make package-gui`（→ gui/src-tauri/target/release/bundle/ 的 .app+.dmg）、`scripts/build-macos-dmg.sh`（自动签名 Developer ID，生成 `*-sign.dmg`，--help 含公证选项） | build-and-install.md:38-49 |
| B1-5 | Linux 依赖：`build-essential mold libssl-dev libwebkit2gtk-4.1-dev libgtk-3-dev librsvg2-dev libayatana-appindicator3-dev patchelf`；**mold 仅 x86_64 必需**（.cargo/config.toml 传 `-fuse-ld=mold`），ARM Linux 不需要 | build-and-install.md:57-67 |
| B1-6 | Linux：`make install`（→/usr/local/bin，sudo）、`make install-nogui`、`make package-gui`（→.deb） | build-and-install.md:69-74 |
| B1-7 | Windows 工具链：VS Build Tools（C++ workload）、Rust MSVC、Node.js 24+、Bun、WebView2（Win10/11 自带） | build-and-install.md:76-88 |
| B1-8 | Windows 终端栈 PowerShell 等价命令；安装到 `%USERPROFILE%\.future\bin`；拷贝 `future-agent.exe`、**`future-channel.exe`**（单数）、`future-tui.exe`、`future.exe`；内置技能用 `& "$bin\future.exe" skills install`（Windows 上不用 symlink） | build-and-install.md:93-115 |
| B1-9 | Windows 桌面应用：sidecar 按 host triple 命名 `future-agent-$triple.exe` / `future-$triple.exe` 放入 gui/src-tauri/binaries；`npx tauri build --no-bundle`；安装为 `future-gui.exe` | build-and-install.md:118-126 |
| B1-10 | Windows 安装包：`node scripts\version.mjs --set-bundle` + `npm run tauri:build` → NSIS `.exe`（gui/src-tauri/target/release/bundle/nsis/） | build-and-install.md:129-131 |
| B1-11 | scripts/ 下打包脚本（build-macos-dmg.sh / build-windows-portable.ps1 / build-windows-installer.ps1）复刻 CI 流水线（DMG/便携 zip/NSIS），需 protoc；**产物含 GUI、agent、CLI，不含 TUI** | build-and-install.md:133-135 |
| B1-12 | future-loop 位于 `orchestration/loop`，`cargo build -p future-loop [--release]`；`bash scripts/install-future-loop.sh` → CLI 到 ~/.local/bin/future-loop、skill 到 ~/.future/agent/skills/ | build-and-install.md:139-153 |
| B1-13 | 技能安装：`make install-skills`（symlink 自 skills/ 子模块）；`future skills install`（**"install all future-* skills (~13)"** ⚠️ 数量待核）；`future init`（装技能 + macOS/Linux 链接本地命令）；`future skills list` / `future skills update` ⚠️（wiki-prompt 说 skills 无 update 子命令，此处说有——文档间冲突） | build-and-install.md:156-168 |
| B1-14 | 验证：`make test`（agent + loop）、`make lint`；开发：`make build/lint/fmt/test/clean` | build-and-install.md:170-190 |
| B1-15 | Proto：规范 API 为 `proto/future.proto`，生成代码入库；`make generate-proto`（agent + channels + TUI） | build-and-install.md:192-206 |

### B2. build-and-install.zh-CN.md（190 行）

| ID | 事实（声明） | 位置 |
|---|---|---|
| B2-1 | 与英文版内容一致（同 B1-1..B1-15 的全部声明），仅语言不同；`future-channel.exe`（单数）同样出现在 Windows 拷贝命令 | build-and-install.zh-CN.md 全文（如 :100） |

---

## C. docs/loop-control-plane.md / loop-control-plane.zh-CN.md（future-loop 指南）

### C1. loop-control-plane.md（179 行）

| ID | 事实（声明） | 位置 |
|---|---|---|
| C1-1 | future-loop 位于 `orchestration/loop`，是 `future-loop` CLI + `/future-loop` agent skill | loop-control-plane.md:7 |
| C1-2 | 定位：把对话变成持久、可复盘、可长期运行的目标；deterministic kernel 一次一个有界回合 | loop-control-plane.md:3-11 |
| C1-3 | **"Rust rewrite of the loopx control plane, adapted and extended"**（loopx 引用） | loop-control-plane.md:13-14 |
| C1-4 | Goals：`goal init/cancel/delete`；状态在 `<cwd>/.future/loop/`，事件账本 + 重放 | loop-control-plane.md:40-41 |
| C1-5 | Todos：`todo add/claim/complete/supersede/update/archive`；advancement/user-gate/monitor/blocker 类别；`--blocks` 依赖链；claim+lease；**完成契约（每个完成的 todo 声明后继或显式 no-follow-up）** | loop-control-plane.md:43-45 |
| C1-6 | Human gates：`gate resolve`；Monitors：`--class monitor --cadence`，no-change backoff | loop-control-plane.md:46-48 |
| C1-7 | should-run 内核：identity 范围边界、user-gate 优先级、repair 预算、outcome floors、replan 义务、接受度缺口；**调度仲裁层把决策归入 9 种 disposition**（terminal/monitor-wait/active work/consistency repair/human gate/quiet wait/…）；fail-closed | loop-control-plane.md:51-59 |
| C1-8 | Quota：跨 run/agent/heartbeat 三来源 slot 记账、24h/7d 汇总、stall repair | loop-control-plane.md:61-64 |
| C1-9 | Scheduler：节奏归一化（`once/hourly/daily/weekly` 或 `15m/1h/2d`）、原子持久化、host 失败跟踪；Monitor polls → `MonitorPolled` 事件 | loop-control-plane.md:65-67 |
| C1-10 | 事件溯源：内容寻址事件 id、幂等追加、fail-closed 冲突检测；`QuotaSpent`/`EvidenceAttached`；markdown backfill；schema 迁移桥（verify/migrate/bridge）；隐私分级投影；run 生命周期（history/compaction/index/retention/stale） | loop-control-plane.md:69-74 |
| C1-11 | 独立验证：`todo add --verify "cargo test" --max-validation-attempts 5` | loop-control-plane.md:76-78 |
| C1-12 | 扩展：能力框架 provider 生命周期（declared→installed→enabled→ready）、能力门禁（run/ask-owner/repair-bridge/skip）、per-capability 命令钩子；扩展 manifest + install/enable/disable/rollback（revision-retained）+ readiness doctor，**v1 声明式、绝不执行扩展代码** | loop-control-plane.md:80-84 |
| C1-13 | 多 agent：identity 范围边界、lane 推荐、supervisor 提案/回执事件、任务租约、handoff 文档 + 交付契约、todo 依赖图、attention 队列/operator inbox | loop-control-plane.md:85-87 |
| C1-14 | 评估诊断：benchmark（protocol/run/ledger）、decision replay + model-behavior corpus、canary（core-control-plane/extension-runtime/release-gate）；`version/doctor/history/turn/todo-event/evidence-log` + `backup`/restore | loop-control-plane.md:89-91 |
| C1-15 | CLI 命令组清单（goal/todo/agent/capability/extension/ops/work-items/handoff/benchmark/replay/canary/cli registry） | loop-control-plane.md:95-116 |
| C1-16 | 快速开始：`/future-loop` skill 用法 + 终端驱动示例（`goal init`/`todo add --blocks --verify`/`status`/`run --model future/deepseek-v4-flash --max-turns 1`） | loop-control-plane.md:124-155 |
| C1-17 | 状态布局：`<cwd>/.future/loop/registry.json`（registry 真相源）、`goals/<id>/events.jsonl`、`goals/<id>/ACTIVE_GOAL_STATE.md`、`runs/`；运行时状态不写项目之外；加 .gitignore | loop-control-plane.md:159-168 |
| C1-18 | 安装：`bash scripts/install-future-loop.sh` 或 `cargo build -p future-loop` | loop-control-plane.md:170-174 |

### C2. loop-control-plane.zh-CN.md（128 行）

| ID | 事实（声明） | 位置 |
|---|---|---|
| C2-1 | 与英文版一致（同 C1-1..C1-18），仅语言不同；CLI 一览为中文注释版 | loop-control-plane.zh-CN.md 全文 |

---

## D. docs/wiki-prompt.md / wiki-prompt-en.md（AI 生成提示词）

### D1. wiki-prompt.md（229 行，中文版）

| ID | 事实（声明） | 位置 |
|---|---|---|
| D1-1 | 定位：给 AI 的生成提示词，用于（重新）生成 docs/wiki/ 整套页面；只管内容不管发布 | wiki-prompt.md:3-7 |
| D1-2 | 读者是普通用户；讲怎么用不讲怎么实现；不暴露内部模块名、端口号（除非命令需要） | wiki-prompt.md:12-16 |
| D1-3 | 写作前先读代码；**只写已实现功能**；明确不写：Research 入口（导航隐藏）、Data 入口（隐藏）、Remote/手机远程（开发中）；判断依据 `gui/src/components/layout/ActivityRail.tsx` 的 featureItems | wiki-prompt.md:21-37 |
| D1-4 | 输出中英双语，en/ 与 zh/ 文件名一一对应；不互链、不放语言切换链接 | wiki-prompt.md:39-54 |
| D1-5 | **平台范围：只写 macOS 和 Windows，禁止 Linux 任何内容** | wiki-prompt.md:56-57 |
| D1-6 | 页面清单 10 页（Home/Installation/Quick-Start/Using-FutureOS/Settings/Skills/CLI/FAQ/_Sidebar/_Footer），**不生成 TUI 页面**；侧边栏结构（去掉 TUI） | wiki-prompt.md:59-88 |
| D1-7 | 每页内容要点 + 代码入口清单（如 SettingsDialog.tsx、FutureLoginDialog.tsx、Composer.tsx、ActivityRail.tsx 等） | wiki-prompt.md:90-160 |
| D1-8 | **CLI 名称裁决：命令名统一为 `future`**（发布产物二进制名与 npm link 一致），"不要写成 future-cli" | wiki-prompt.md:171, 219 |
| D1-9 | CLI 命令组：auth（login/status/logout）、agent（start/stop/restart/status）、run（--model 支持 model:thinking、--thinking、@<path>、--continue/-c、--cwd、--mode json、--no-session）、tools（list/call --args/--stdin/--output）、skills（list/install/uninstall，**没有 update**）、channel；**去掉 tui 组** | wiki-prompt.md:178-183 |
| D1-10 | 自检：链接完整性、泄漏扫描（Linux/.deb/.tar.gz/apt、TUI、gRPC/端口如 50051、Research/Data/Remote）、中英对齐、CLI 名称；偏差报告反哺提示词 | wiki-prompt.md:180-206 |
| D1-11 | 技能表（参考）：Account/Web/Paper/Deep research/Document/Image/Browser/Hand-drawn posters/Hand-drawn slides/Subagent/Skill creator | wiki-prompt.md:165 |
| D1-12 | 批准模式：手动 / 沙盒（仅 macOS）/ 无限制 | wiki-prompt.md:151-152 |
| D1-13 | FAQ 覆盖问题清单（macOS 打不开/Windows SmartScreen/WebView2/登录/切模型/批准/数据位置/更新/卸载/平台=macOS+Windows） | wiki-prompt.md:153-160 |

### D2. wiki-prompt-en.md（229 行，英文版）

| ID | 事实（声明） | 位置 |
|---|---|---|
| D2-1 | 与中文版结构一致（角色/读者/代码优先/双语/平台/页面清单/自检/禁止项） | wiki-prompt-en.md 全文 |
| D2-2 | **CLI 名称裁决与 D1-8 相反：⚠️ 声明发布二进制名为 `future-cli`**，"The release binary is named **`future-cli`** … user-facing wiki must always use `future-cli`, never `future`"；dev 期 `future` 只是 npm link 别名；路径 `/Applications/FutureOS.app/Contents/MacOS/future-cli`、`future-cli.exe` | wiki-prompt-en.md:171-174, 177, 185, 219 |
| D2-3 | 页面清单表格中 CLI 页标题写 "CLI (`future-cli`)"；侧边栏写 "CLI (future-cli)" — ⚠️ 与生成出的 wiki 页面（en/CLI.md 用 `future`）不一致 | wiki-prompt-en.md:81, 104 |
| D2-4 | 技能表、批准模式、FAQ 覆盖点与 D1 对应 | wiki-prompt-en.md:140-160 |

---

## E. docs/wiki/en 与 zh（用户 wiki 页面）

> en/ 与 zh/ 全部 12 页已通读；两语言版本标题结构逐一 diff 一致（仅语言差异，无内容漂移）。

### E1. Home.md（en 40 行 / zh 40 行）

| ID | 事实（声明） | 位置 |
|---|---|---|
| E1-1 | 定位：**"desktop AI agent workbench"**；能看到并核对 agent 工作（读了什么/跑了什么命令/改了哪些文件/等什么批准） | en/Home.md:1-3 |
| E1-2 | 核心卖点："You stay in control"，风险操作前停下征求批准；所有工作可见可复核 | en/Home.md:9-10 |
| E1-3 | 开始使用三步：Installation → Quick-Start → Using-FutureOS | en/Home.md:14-17 |
| E1-4 | 能做：agent 对话（流式思考/工具调用）、Chat 或绑定文件夹的 Workspace、批准机制、右侧面板（Runs/Review/Artifacts）、Skills | en/Home.md:20-26 |
| E1-5 | **"FutureOS runs on macOS and Windows."**（刻意不含 Linux，与 wiki-prompt §5 一致） | en/Home.md:40 |

### E2. Quick-Start.md（en 71 行 / zh 71 行）

| ID | 事实（声明） | 位置 |
|---|---|---|
| E2-1 | 登录流程：左下齿轮 → Settings → Providers → 内置 FutureGene → Connect → 浏览器授权（不自动打开时给验证码 + 可复制链接） | en/Quick-Start.md:8-14 |
| E2-2 | 两种开始方式：New Chat（提问/一次性任务）vs Workspace（绑定文件夹） | en/Quick-Start.md:22-31 |
| E2-3 | 发消息：流式回复、工具活动展示、风险操作暂停等批准；**每消息最多 4 张图片（每张 25 MiB），其他文件类型不限数量** | en/Quick-Start.md:37-45 |
| E2-4 | 模型选择器在输入框内；thinking level 控件在旁边；Settings → Models 管理 | en/Quick-Start.md:48-52 |
| E2-5 | 右侧面板三视图：Runs / Review（Workspace 文件改动）/ Artifacts（Chat 产出） | en/Quick-Start.md:54-62 |

### E3. Installation.md（en 77 行 / zh 77 行）

| ID | 事实（声明） | 位置 |
|---|---|---|
| E3-1 | 平台：**macOS 和 Windows** | en/Installation.md:3 |
| E3-2 | 下载：Releases 页；macOS `.dmg`；Windows 安装器 `.exe` 或便携 `.zip` | en/Installation.md:10-16 |
| E3-3 | CLI `future` 随每个下载附带（安装在应用旁边，无需单独安装） | en/Installation.md:18-19 |
| E3-4 | **"Formal macOS and Windows installers are signed, and the macOS build is also notarized by Apple."** ⚠️ 与 en/FAQ.md:9（"isn't notarized"）及 docs/dist/readme-macos.txt（"未做 Apple 公证"）矛盾 | en/Installation.md:24 |
| E3-5 | macOS 安装：拖入 Applications；Windows：安装版跑 .exe、便携版解压整个文件夹后双击 FutureOS.exe（**FutureOS.exe 与 future-agent.exe 须同文件夹**） | en/Installation.md:27-33 |
| E3-6 | SmartScreen 提示处理；**需要 Microsoft Edge WebView2 Runtime**（Win10 近期版/Win11 一般自带，缺失装 Evergreen） | en/Installation.md:35-36 |
| E3-7 | 便携 zip "来自 Internet" 解锁技巧（属性→Unblock；或 PowerShell `Get-ChildItem -Recurse | Unblock-File`） | en/Installation.md:38-40 |
| E3-8 | 数据位置：macOS `~/.future`；Windows `C:\Users\<you>\.future` | en/Installation.md:50-55 |
| E3-9 | 更新：Settings → Check for updates（签名验证下载安装 + 重启）；可手动覆盖；便携版替换文件夹；`.future` 数据保留 | en/Installation.md:57-59 |
| E3-10 | 卸载：macOS 删 FutureOS.app；Windows 设置卸载或删便携文件夹；清数据再删 `.future` | en/Installation.md:61-63 |

### E4. Using-FutureOS.md（en 91 行 / zh 91 行）

| ID | 事实（声明） | 位置 |
|---|---|---|
| E4-1 | 三栏布局：左=导航（New Chat、Models 快捷入口、Skills、Workspaces、Chats、Settings，可折叠）；中=对话（输入框固定底部）；右=上下文面板（Runs/Review/Artifacts，可折叠） | en/Using-FutureOS.md:6-10 |
| E4-2 | 每个会话是独立 agent session，互不干扰 | en/Using-FutureOS.md:13 |
| E4-3 | Chat vs Workspace 对比表（建立方式/适用场景/右侧面板显示 Runs+Artifacts vs Runs+Review）；可重命名/置顶/删除会话 | en/Using-FutureOS.md:15-27 |
| E4-4 | Shift+Enter 换行；流式中发送按钮变 stop 按钮 | en/Using-FutureOS.md:31-39 |
| E4-5 | 批准机制：风险操作（**读写文件、跑 shell 命令、删文件、写到 workspace 外**）停止并弹批准卡，**无超时**；Allow once / Deny / 按 workspace/chat 记住规则（可编辑路径模式）；键盘 Cmd/Ctrl+Enter 批准、Esc 拒绝 | en/Using-FutureOS.md:42-53 |
| E4-6 | 批准模式（Settings → General 或输入框盾牌控件）：**Manual**（文件读写在批准前；只读命令自动跑）/ **Sandboxed（仅 macOS）**（命令跑在 macOS sandbox 内；文件操作仍提示）/ **Unrestricted**（无提示无沙盒） | en/Using-FutureOS.md:55-59 |
| E4-7 | Runs 视图：每张卡显示真实命令、状态、运行/完成计数；可 Inspect/Terminate/Clear finished | en/Using-FutureOS.md:63-69 |
| E4-8 | Review 视图（仅 Workspace）：文件列表、变更类型（added/modified/deleted/renamed）、per-file diff；版本控制下可切 **"Last run changes"** 视图 | en/Using-FutureOS.md:71-73 |
| E4-9 | Artifacts 视图（仅 Chat）：产出预览/复制/导出/打开原件；也可自己上传文件 | en/Using-FutureOS.md:75-77 |

### E5. Settings.md（en 82 行 / zh 82 行）

| ID | 事实（声明） | 位置 |
|---|---|---|
| E5-1 | 设置页构成：**General / Providers / Models** + Check for updates + Reset | en/Settings.md:3-5 |
| E5-2 | General：Language、Approval mode（Manual/Sandboxed[macOS only]/Unrestricted）、Show thinking process | en/Settings.md:8-17 |
| E5-3 | Providers：FutureGene（内置）Connect 登录，可 Sign in again / Sign out；其他内置 provider（DeepSeek、OpenAI、Anthropic、Google 等）可 Set key/Update key；"More providers" 展开全列表 | en/Settings.md:21-29 |
| E5-4 | 自定义 provider 字段：Name（可选）、Provider ID（小写字母/数字/-/_）、API type（**OpenAI Completions / OpenAI Responses / Anthropic**）、Base URL、API Key、Models；校验 id 唯一；可 Edit/Remove；**API key 与其他凭据分开存储** | en/Settings.md:31-39 |
| E5-5 | Models：按 provider 分组、可搜索、可切换每个模型可见性（隐藏后从选择器移除）；输入框选择器同源并显示 provider | en/Settings.md:43-47 |
| E5-6 | Check for updates：检查新版本并下载安装器 | en/Settings.md:50-51 |
| E5-7 | Reset：**Clear local data 清空本地数据并重启应用**（会话与本地设置被移除） | en/Settings.md:54-55 |

### E6. Skills.md（en 56 行 / zh 56 行）

| ID | 事实（声明） | 位置 |
|---|---|---|
| E6-1 | Skills 是能力包；安装后 agent 在相关时**自动使用**，无需手动开启 | en/Skills.md:3-4 |
| E6-2 | Skills 页两个标签：Installed / All（All 需联网）；分类下拉 + 搜索；每个技能显示名称/描述/版本/分类；Install/Uninstall | en/Skills.md:8-14 |
| E6-3 | 内置技能表（11 个）：Account、Web、Paper、Deep research、Document、Image、Browser、Hand-drawn posters、Hand-drawn slides、Subagent、Skill creator（注明目录随在线目录变化） | en/Skills.md:18-38 |

### E7. CLI.md（en 127 行 / zh 127 行）

| ID | 事实（声明） | 位置 |
|---|---|---|
| E7-1 | 工具名 **`future`**（⚠️ 与 wiki-prompt-en 的 `future-cli` 裁决矛盾）；可选、随下载附带 | en/CLI.md:3-5 |
| E7-2 | 位置：macOS（.dmg）`/Applications/FutureOS.app/Contents/MacOS/future`；Windows（便携 .zip）`future.exe`；**Windows 便携包才有 CLI，安装版没有单独 future.exe** | en/CLI.md:11-18 |
| E7-3 | 运行：`future --help`；PATH/别名（macOS 别名示例） | en/CLI.md:23-33 |
| E7-4 | **agent 必须运行**：桌面应用开着则 agent 已在跑，否则 `future agent start` | en/CLI.md:36-41 |
| E7-5 | 命令组：auth（login/status/logout）、agent（start/stop/restart/status）、run、tools（list / call --args/--stdin/--output）、skills（list/install/uninstall）、channel（start/stop/restart/status） | en/CLI.md:43-111 |
| E7-6 | run 选项：`--model`（支持 `model:thinking`，如 `sonnet:high`）、`--thinking`（off/minimal/low/medium/high/xhigh）、`@<path>` 包含文件、`--continue`/`-c`、`--cwd`、`--mode json`、`--no-session`；管道输入示例 | en/CLI.md:63-81 |
| E7-7 | 小贴士：macOS 首次被拦→先右键打开应用；"Connection refused"→agent 未运行 | en/CLI.md:113-119 |

### E8. FAQ.md（en 65 行 / zh 65 行）

| ID | 事实（声明） | 位置 |
|---|---|---|
| E8-1 | macOS 打不开：**"The current build isn't notarized, so this is expected"**；右键→Open；"已损坏"用 `xattr -dr com.apple.quarantine /Applications/FutureOS.app` ⚠️ 与 Installation.md:24 的 notarized 矛盾 | en/FAQ.md:9-15 |
| E8-2 | Windows SmartScreen："More info → Run anyway" | en/FAQ.md:17-18 |
| E8-3 | Windows 启动没反应：装 WebView2 Evergreen；便携版 FutureOS.exe 与 future-agent.exe 同文件夹；"来自 Internet" 解锁 | en/FAQ.md:20-25 |
| E8-4 | 用不了模型/未登录：Settings → Providers → FutureGene → Connect 或加自己的 provider | en/FAQ.md:27-28 |
| E8-5 | 切模型：输入框选择器或 Settings → Models | en/FAQ.md:30-31 |
| E8-6 | agent 停下询问 = 批准机制，无超时 | en/FAQ.md:33-35 |
| E8-7 | 数据位置：`~/.future`（macOS）/ `C:\Users\<you>\.future`（Windows） | en/FAQ.md:37-42 |
| E8-8 | 平台：**macOS 和 Windows** | en/FAQ.md:59-60 |

### E9. Feishu.md（en 205 行 / zh 205 行）

| ID | 事实（声明） | 位置 |
|---|---|---|
| E9-1 | 架构图：Feishu user → Feishu server (open.feishu.cn) → Channel Bridge（WebSocket）→ Agent (gRPC 127.0.0.1:50051)；回复经 CardKit 流式卡片 | en/Feishu.md:8-13 |
| E9-2 | 前提：飞书开发者账号、机器人能力 app、agent 已运行（`make run-agent` 或 `future agent start`） | en/Feishu.md:15-19 |
| E9-3 | 建 app：open.feishu.cn/app → 企业自建应用 → 启用 Bot；凭据 App ID/App Secret | en/Feishu.md:22-28 |
| E9-4 | 权限 scope 5 个：`im:message`、`im:message.p2p_msg:read`、`im:message.group_msg:read`、`im:message:send_as_bot`、`im:resource`、`contact:user.base:read`（表列 6 行） | en/Feishu.md:31-38 |
| E9-5 | 事件订阅：`im.message.receive_v1`；Request URL 任意 HTTPS（桥用 WebSocket 不会真调） | en/Feishu.md:40-43 |
| E9-6 | 配置 `~/.future/channels/config.json`：`agent{grpc_addr,cwd,model,thinking_level,permission_level}` + `feishu{enabled,app_id,app_secret,domain}`；默认模型示例 `future/deepseek-v4-pro` | en/Feishu.md:65-93 |
| E9-7 | 策略：dm_policy（open/allowlist/disabled，默认 allowlist）、dm_allowlist、group_policy（默认 disabled）、group_allowlist、require_mention（默认 true） | en/Feishu.md:96-112 |
| E9-8 | 行为配置：streaming（默认 true）、resolve_sender_names（默认 true）、max_image_mb（默认 10）、typing_indicator（默认 false） | en/Feishu.md:116-122 |
| E9-9 | 启动：`make build-channels-release` + `./target/release/future-channels`（⚠️ channels crate 名为 `future-channel`，见 channels/Cargo.toml:2 — 二进制名待核）；或 `future channel start/status/stop/restart`（macOS launchctl / Linux systemd）；配置不存在时生成模板并退出 | en/Feishu.md:136-150 |
| E9-10 | 斜杠命令 9 个：/new /status /model /models /effort /stop /compact /cwd /help；本地处理不经过 agent | en/Feishu.md:155-170 |
| E9-11 | 流式：CardKit 卡片实时更新，思考内容折叠 blockquote；非流式单条 markdown | en/Feishu.md:172-174 |
| E9-12 | 排错：机器人不回复 4 步；**重连约每 6 分钟（keepalive 30s）**；图片需 im:resource 且 < max_image_mb | en/Feishu.md:176-196 |

### E10. DingTalk.md（en 170 行 / zh 170 行）

| ID | 事实（声明） | 位置 |
|---|---|---|
| E10-1 | 架构：DingTalk user → api.dingtalk.com → Bridge（Stream Mode）→ Agent (gRPC)；无公共回调 URL；经 sessionWebhook 回复 | en/DingTalk.md:8-13 |
| E10-2 | 前提：钉钉开发者账号、Stream Mode app、agent 已运行 | en/DingTalk.md:15-19 |
| E10-3 | 建 app：open-dev.dingtalk.com → 机器人 → 消息接收模式选 **Stream Mode**；凭据 Client ID(AppKey)/Client Secret | en/DingTalk.md:22-27 |
| E10-4 | 权限：`im.message.receive`、`im.message.send`、`qyapi_robot_webhook_message_send` | en/DingTalk.md:29-33 |
| E10-5 | 配置：`dingtalk{enabled,client_id,client_secret,domain=api.dingtalk.com}` + agent 段（同 Feishu，含 `permission_level`：all/workspace/none） | en/DingTalk.md:51-82 |
| E10-6 | 启动：`make build-channels-release` + `./target/release/future-channels`（⚠️ 同 E9-9 的二进制名问题） | en/DingTalk.md:89-90 |
| E10-7 | 斜杠命令 9 个（同 Feishu 列表）；**"All slash commands are handled locally"**（与 Feishu 的"部分本地"措辞不同） | en/DingTalk.md:110-120 |
| E10-8 | 回复：markdown 经 sessionWebhook；**每条回复是新消息（webhook 不支持就地编辑）**；思考内容 blockquote `> 💭` | en/DingTalk.md:122-129 |
| E10-9 | 与 Feishu 差异表：连接（pbbp2 protobuf vs Stream Mode JSON）、流式（CardKit vs 新消息）、思考展示、emoji 反应（✅ vs ❌ API 不可用）、多模态（图文 vs 纯文本） | en/DingTalk.md:131-143 |
| E10-10 | 排错：机器人不回复 4 步；**keepalive 20s**；钉钉 markdown 需双换行 `\n\n` | en/DingTalk.md:145-166 |

### E11. Models.md（en 4983 行 / zh 4983 行）

| ID | 事实（声明） | 位置 |
|---|---|---|
| E11-1 | **"3826 models across 143 providers."**（页首声明；README 说 1000+/100+，⚠️ 不一致） | en/Models.md:2 |
| E11-2 | Provider Summary 表（143 行）：302ai 92、abacus 92、alibaba 46、kilo 260、llmgateway 151、nano-gpt 193、OpenRouter 267、poe 134、Vercel AI Gateway 302、zenmux 120 等 | en/Models.md:5-151 |
| E11-3 | Per-Provider Details：每个 provider 一节，含 **Base URL** + 模型表（Model ID/Name/Context/Max Output/Image/Reasoning） | en/Models.md:155+ |
| E11-4 | 例：DeepSeek Base URL `https://api.deepseek.com`，4 模型（deepseek-chat/reasoner/v4-flash/v4-pro，均 1M 上下文） | en/Models.md:1001-1008 |
| E11-5 | 例：OpenRouter Base URL `https://openrouter.ai/api/v1`，267 模型 | en/Models.md:3072-3074 |
| E11-6 | 例：Vercel AI Gateway 302 模型 | en/Models.md:4322 |
| E11-7 | 无生成说明/更新时间戳/生成脚本引用（⚠️ 疑似生成文件但未注明生成方式；README/build-and-install 提到 `make generate-models` 需要 Python） | en/Models.md 全文 |

### E12. _Sidebar.md / _Footer.md（en/zh 各 21 行 / 3 行）

| ID | 事实（声明） | 位置 |
|---|---|---|
| E12-1 | 侧边栏分组：FutureOS/Home、Getting started（Install/Quick Start）、Using the app（Using FutureOS/Settings/Skills）、Command line（CLI）、Integrations（Feishu/DingTalk）、Help（FAQ） | en/_Sidebar.md:1-21 |
| E12-2 | 页脚："runs on macOS and Windows · Download(Releases) · Report an issue(issues)" | en/_Footer.md:3 |

---

## F. docs/architecture-audit/（4 份审计报告 + README）

| ID | 事实（声明） | 位置 |
|---|---|---|
| F-1 | 审计日期 **2026-08-05**；基准 `dev @ 8aa82925`（调查在工作树 8164b8e1 进行，树内容 diff 为空）；方法：4 并行调查，结论带 file:line 证据 | architecture-audit/README.md:1-9 |
| F-2 | 报告 01 结论：**agent ↔ gui_rust 边界泄漏且双向**——gRPC 是信封，里面是 stringly JSON（`RpcResponse.data`/`StreamEvent.data` 均为 string，proto 的 typed `SessionState` 无人用）；至少 7 条文件系统旁路（auth.json/models.json/skills/会话文件/approval_rule.json/源码 include/进程生命周期）；最强证据：`#[path]` 编译期 include agent 源码（catalog.rs:15-16）、GUI 是 auth.json 唯一写入者、cleanup.rs 依赖 `{id}.jsonl` 命名探测 | 01-agent-guirust-boundary.md:1-6, 40-53, 215-244 |
| F-3 | 报告 02 结论：gui_rust ↔ gui_react **架构干净、契约全手工且已漂移**——103 个 `#[tauri::command]`（lib.rs:600-704 注册）、8 个事件名、invokeCommand 102 处 0 裸 invoke；39+ 对类型手工同步已漂移 3 对（ThreadRecord↔StoredThread 少 archivedAt/deletedAt、AgentModelOption optionality、AgentPromptResponse.session_id 必填vs可选）；`thread-runtime-updated` 形状声明 4 次已漂移 | 02-guirust-guireact-boundary.md:1-5, 18-80, 230-260 |
| F-4 | 报告 03 结论：18 个超大模块候选——3 个 Tier1（agent_bridge/mod.rs 1343 行、session/mod.rs 3624 行、Composer.tsx 704 行）、9 个 Tier2、6 个内聚不拆；含逐文件行数/职责/拆分方案 | 03-large-modules-split.md:1-12 |
| F-5 | 报告 04 结论：React 流式热路径 4 个 HIGH（H1 handleFork 依赖 messages 击穿唯一 memo；H2 threadRunStatuses 全树 25Hz 重渲染；H3 流式 markdown 全量重解析 O(n²)；H4 Composer 每推送重渲染）+ 7 个 MED/LOW；后端 40ms 推送合并（lib.rs:286-330）≈25 次/秒 | 04-react-rendering-performance.md:1-7, 26-60 |
| F-6 | 报告间交叉引用：01 影子 JSON 与 02 agent 透传域同根因；03 拆分文件与 04 性能修复同批文件建议协调 | architecture-audit/README.md:10-13 |

---

## G. docs/dist/readme-*.txt（随安装包分发的用户说明）

| ID | 事实（声明） | 位置 |
|---|---|---|
| G-1 | macOS 版：未公证提示（"本版本未做 Apple 公证"⚠️ 与 Installation.md:24 矛盾）、右键打开、`xattr -dr com.apple.quarantine`、数据在 ~/.future、CLI 在 FutureOS.app/Contents/MacOS/future | readme-macos.txt:1-23 |
| G-2 | Windows 便携版：解压整个文件夹、FutureOS.exe 与 future-agent.exe 同文件夹、SmartScreen、WebView2 Evergreen、Unblock 技巧、数据 C:\Users\<用户名>\.future、CLI future.exe 同目录 | readme-windows.txt:1-26 |
| G-3 | **Linux 版存在**（readme-linux.txt）：tar.gz 解压、futureos/future-agent/future 三文件同文件夹、WebKitGTK（libwebkit2gtk-4.1-0 / webkit2gtk4.1）⚠️ 与 wiki 的"仅 macOS+Windows"口径不同（产品实际支持 Linux） | readme-linux.txt:1-21 |
| G-4 | 英文版 readme-macos-en.txt / readme-windows-en.txt / readme-linux-en.txt 与中文版内容一致 | 各 -en.txt |

---

## H. 通读阶段发现的文档间冲突 / 疑似过时（待源码核验确认）

| # | 冲突/疑点 | 涉及位置 |
|---|---|---|
| H1 | CLI 二进制名：`future` vs `future-cli`。wiki-prompt-en 强制 `future-cli`；wiki-prompt(zh) 强制 `future`；实际生成的 wiki（en+zh CLI.md）、dist readme、build-and-install 均用 `future`。需以发布配置（tauri.conf.json sidecar、CLI 打包、npm link）为准 | D1-8 vs D2-2 vs E7-1 vs G-1/2 vs B1-8 |
| H2 | macOS 公证：Installation.md 说"notarized by Apple"，FAQ.md 说"isn't notarized"，dist readme-macos.txt 说"未做 Apple 公证"，wiki-prompt 说"完成 Apple 公证"。需核 tauri.conf.json / CI notarize 步骤 | E3-4 vs E8-1 vs G-1 vs wiki-prompt.md:128 |
| H3 | 模型数量："1000+ models / 100+ providers"（README） vs "3826 models / 143 providers"（Models.md）。README 疑似过时 | A1-3 vs E11-1 |
| H4 | channel 二进制名：`future-channels`（Feishu/DingTalk 文档） vs `future-channel`（channels/Cargo.toml crate 名 + build-and-install Windows 拷贝名）。需核 `make build-channels-release` 产物名 | E9-9 vs E10-6 vs B1-8 |
| H5 | skills 命令面：build-and-install 说 `future skills update` 存在；wiki-prompt(zh) 明确"没有 update"。需核 cli/src/commands/skills.ts | B1-13 vs D1-9 |
| H6 | `future init` 命令：build-and-install 提到；CLI.md wiki 页未列。需核 cli/src/index.ts | B1-13 vs E7-5 |
| H7 | "~13 个 future-* skills"（build-and-install）需核 skills/ 子模块实际数量 | B1-13 |
| H8 | 平台口径：wiki 与 wiki-prompt 只写 macOS+Windows；README / build-and-install / dist readme 覆盖 Linux。属有意的分层（wiki 面向 GUI 用户），但 README 的 GUI 提及需保持一致 | E1-5 vs B1-5 vs G-3 |
| H9 | Models.md 无生成说明/时间戳；README 特性表与 build-and-install 提到 `make generate-models`（Python）——需核 scripts 下生成器，确认 Models.md 是否由它产出、如何再生成 | E11-7 |
| H10 | 审计报告为 2026-08-05 时点快照（dev @ 8aa82925），含大量 file:line；仓库持续变动，行号已可能漂移——需判定标注"历史快照"或更新 | F-1 |
| H11 | Feishu 权限 scope 表格标题写"add these scopes"，正文列了 6 行（含 contact:user.base:read）但文档在 E9-4 之前说 5 个——以实际为准核对 | E9-4 |
| H12 | DingTalk 说 "All slash commands are handled locally"；Feishu 说部分命令本地处理、未识别命令转发 agent——两处措辞行为需与 channels 源码核对 | E9-10 vs E10-7 |
| H13 | Settings API type 枚举：en/Settings.md 写 "OpenAI Completions, OpenAI Responses, or Anthropic"——需核 CustomProviderDialog 实际选项 | E5-4 |
| H14 | 读文件操作是否要求批准：README 特性表（Tool Execution）未细说；Using-FutureOS 批准机制写"**read or write a file**"需批准，而批准模式 Manual 又写"prompts before file reads and writes; read-only commands run automatically"——内部一致性待核 | E4-5 vs E4-6 |
| H15 | wiki-prompt §7 代码入口引用的文件（ActivityRail.tsx featureItems、SettingsDialog.tsx、Composer.tsx 等）与审计报告 02/03 的行号引用需在核验时交叉确认是否存在 | D1-7 vs F-3/F-4 |

---

## I. 通读阶段观察到的"缺失文档/章节"候选（供 todo_9bb2c6dd1c38 参考）

| # | 候选缺口 | 依据 |
|---|---|---|
| I-1 | Models.md 缺"如何再生成"说明（生成器脚本、命令、是否自动同步） | E11-7, H9 |
| I-2 | dist readme 有 Linux 版，但 wiki 无 Linux 页面——若产品支持 Linux 桌面版，wiki 平台口径需决策（加 Linux 页 vs 维持现状） | G-3 |
| I-3 | 无渠道（channels）配置的完整参考文档（Feishu/DingTalk 各自成篇但无统一 config.json schema 参考页） | E9-6, E10-5 |
| I-4 | loop-control-plane 指南未覆盖 `agent` 命令组详细用法（onboard/scope/lane/supervisor）与 work-items/handoff 用法示例 | C1-15 |
| I-5 | docs 顶层无索引 README（除 architecture-audit 有自身 README）——docs/ 目录本身缺导航 | 目录结构 |
| I-6 | 无 TUI 使用文档（wiki 刻意不含 TUI；但 README 有 TUI 斜杠命令/快捷键——TUI 详细指南缺失） | A1-10/11, D1-6 |
| I-7 | 无文档说明 `.future/` 目录完整布局（agent/channels/tui/app/workspaces 各子目录职责） | E3-8, CLAUDE.md 提及 |
