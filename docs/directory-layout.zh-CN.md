# 目录布局：`~/.future/` 下各目录的职责

FutureOS 的所有用户状态都存放在 `~/.future/` 下（Windows 为
`%USERPROFILE%\.future\`）。本页给出各子目录与所属组件及其内容的对应关系。
以下路径使用 macOS/Linux 写法；Windows 布局相同，根为 `%USERPROFILE%\.future\`。

```text
~/.future/
├── agent/                     # agent 后端（future-agent）
│   ├── settings.json          # agent 设置（模型默认值、沙箱等）
│   ├── models.json            # provider/模型目录：apiKey、baseUrl、models[]
│   ├── auth.json              # 凭据，按模型 id 或 provider 键控
│   ├── sessions/              # 扁平 JSONL 会话存储（每个会话一个文件）
│   ├── skills/                # 已安装的用户技能（APP_SKILLS_DIR）
│   ├── browser/               # CLI 浏览器工具状态（config.json、profile/、artifacts/）
│   ├── images/                # CLI 图片工具输出目录
│   └── logs/agent.log         # agent 日志（启用日志时）
├── agent-app/                 # 遗留凭据目录（auth.json），向后兼容读取
├── channels/
│   ├── config.json            # 飞书 / 钉钉桥配置（见 channels-config.zh-CN.md）
│   └── feishu/                # 飞书桥数据（会话文件、接收的文件）
├── tui/                       # 终端界面（future-tui）
│   ├── settings.json          # defaultModel、defaultThinkingLevel 等（见 tui.zh-CN.md）
│   ├── keybindings.json       # 可选按键绑定覆盖
│   ├── debug.log              # 调试重绘日志（仅 PI_DEBUG_REDRAW=1）
│   ├── write.log              # 原始屏幕写入日志（仅 PI_TUI_WRITE_LOG=1）
│   └── crash.log              # 崩溃时的 panic 回溯
├── app/                       # 桌面 GUI（FutureOS App）
│   ├── app.db                 # SQLite 数据库（会话线程、run、审批等）
│   ├── images/                # 每线程图片树（thumb/ + origin/）
│   ├── review/                # 每个 workspace 的影子 git 评审仓库
│   └── run_events/            # 每个 run 的事件日志（JSONL）
├── workspaces/
│   └── chat/                  # 每线程聊天工作区（agent 会话 / 线程 id）
├── remote_pairing.json        # 桌面远程桥身份（nkey_seed + user_jwt）
└── bin/                       # CLI / agent 链接：`future`、`future-agent`（见下）
```

## `~/.future/agent/` — agent 后端

归 `future-agent`（运行在 `127.0.0.1:50051` 的 gRPC 后端）所有。其配置
完全从本目录的文件读取——没有任何模型相关的 CLI 旗标或环境变量：

- `settings.json` — agent 设置。
- `models.json` — provider 目录，形如
  `{"providers": {"<provider>": {"apiKey": …, "baseUrl": …, "models": [{"id", "name", "contextWindow"}]}}}`。
  `future auth login` 会自动同步此文件；也可以手工编辑。
- `auth.json` — 凭据，先按模型 id、再按 provider、最后按默认条目键控：
  `{"<provider>": {"type": "api_key", "key": …, "baseUrl": …}}`。
- `sessions/` — 扁平的 JSONL 会话文件目录（agent 的默认会话目录）。
- `skills/` — 两个技能发现目录之一（`APP_SKILLS_DIR`）；另一个是
  `~/.agents/skills/`（`AGENTS_SKILLS_DIR`）。技能是含 `SKILL.md` +
  YAML frontmatter 的普通目录。
- `browser/` — CLI 浏览器工具状态（`config.json`、Chromium 的 `profile/`、
  截图的 `artifacts/`）。遵循 `FUTURE_HOME`。
- `images/` — CLI 图片生成/编辑工具（`future tools call image …`）的输出目录。
- `logs/agent.log` — 启用日志时写入。

## `~/.future/agent-app/` — 遗留凭据目录

agent 解析 `auth.json` 时会先读 `~/.future/agent-app/auth.json`，再读
`~/.future/agent/auth.json`（向后兼容旧版 GUI 写入的凭据）；GUI 的文件访问
守卫把 `agent/` 与 `agent-app/` 都视为凭据位置。新的写入都落到 `~/.future/agent/`。

## `~/.future/channels/` — 渠道桥

归 `future-channel`（飞书 / 钉钉桥）所有。`config.json` 包含 `agent`、
`feishu`、`dingtalk` 三个块——完整 schema 与默认值见
[channels-config.zh-CN.md](channels-config.zh-CN.md)。若文件不存在，桥会
写入默认模板并退出，提示编辑后重启。`feishu/` 是飞书桥的数据目录（会话文件
与接收的文件/图片）。

## `~/.future/tui/` — 终端界面

归 `future-tui` 所有。`settings.json` 持久化客户端侧设置（`defaultModel`、
`defaultThinkingLevel`、`defaultPermissionLevel`、`enabledModelIds`）；
可选的按键绑定覆盖放在 `keybindings.json`；`debug.log` 在设置 `PI_DEBUG_REDRAW=1` 时写入调试重绘日志，设置 `PI_TUI_WRITE_LOG=1` 时 `write.log` 记录原始屏幕写入；`crash.log` 在 TUI 崩溃时接收 panic 回溯。
见 [tui.zh-CN.md](tui.zh-CN.md)。

## `~/.future/app/` — 桌面 GUI

归 Tauri 桌面应用所有（见 `desktop/`）：

- `app.db` — SQLite 数据库（线程、run、审批请求等）。
- `images/` — 持久化的每线程图片树（`<thread_id>/thumb/`，工作区对话另有
  `<thread_id>/origin/`）。放在 `~/.future` 而非系统缓存目录，是因为 macOS
  可能清理缓存目录。
- `review/` — 评审功能使用的影子 git 仓库，每个 workspace 的 run 共享一个
  `<workspace_id>` 子目录。
- `run_events/` — 每个 run 的事件日志（JSONL），派生自 agent 的 JSONL 会话。

桌面远程桥把配对身份（`nkey_seed` + `user_jwt` + NATS 地址）保存在
`~/.future/remote_pairing.json`（位于 `~/.future` 根，而非 `app/` 下）。

## `~/.future/workspaces/chat/` — 聊天工作区

GUI 的每线程聊天工作区，每个子目录以 agent 会话 id（已知时，例如从导入
获得）或 GUI 线程 id 命名。用户自选的工作区位于别处，清理本目录时绝不会
触碰它们。

## loop 控制面 — 项目本地，不在 `~/.future/` 下

`future-loop` 的状态是**项目本地**的：在项目目录运行它，全部状态存放在
`<cwd>/.future/loop/` 下（`FUTURE_LOOP_ROOT` 可为特殊场景覆盖状态根；
`~/.future/loop/` 不会被使用）。见
[loop-control-plane.zh-CN.md](loop-control-plane.zh-CN.md)。

```text
<cwd>/.future/loop/
├── registry.json                  # 目标注册表（每个目标一条）
├── goals/<goal_id>/
│   ├── events.jsonl               # 事件源账本（权威状态）
│   ├── runs.jsonl                 # 权威花费/运行账本
│   ├── next_action.txt            # 内核 should-run 决策快照
│   ├── schema.json                # 事件 store schema 版本戳
│   ├── ACTIVE_GOAL_STATE.md       # 人可读的活跃状态投影
│   ├── status-cache.json          # status 投影缓存
│   ├── read_diagnostics.json      # 账本读取诊断（未知事件类型）
│   ├── scheduler-state/           # 调度器状态（随目标一起备份）
│   └── runs/                      # run-history（compaction/retention，LoopX 风格）
│       └── index.jsonl            # 追加式 run 索引
├── runs/
│   └── <run_id>.live.jsonl        # live 进行中 worker run 日志
├── inbox/
│   └── *.json                     # operator inbox（活性告警等）
└── backups/
    └── <ts>-<goal_id>/            # 每目标备份（账本 + scheduler-state + 注册表项）
```

## `~/.future/bin/` — CLI 链接

`future init` 会安装内置技能，并在 macOS/Linux 上把 `future`（以及位于
`future` 可执行文件旁时的 `future-agent`）符号链接到 `~/.future/bin/`，
同时打印 PATH 配置提示。默认安装只链接 `future`（独立二进制已不再默认安装）。
Windows 安装版同样安装到 `%USERPROFILE%\.future\bin`。

## 相关

- `~/.agents/skills/` — 第二个技能发现目录（`AGENTS_SKILLS_DIR`），
  例如存放机器级技能。
- 项目本地 `.future/` — GUI 聊天工作区与 loop 控制面也会在项目内使用
  `.future/` 目录（例如 `.future/loop/`、`.future/approval_rule.json`）。
  该目录应加入 `.gitignore`。
