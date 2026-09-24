# 目录布局：`~/.future/` 下各目录的职责

FutureOS 的多数持久用户状态存放在 `~/.future/` 下（Windows 为
`%USERPROFILE%\.future\`）；loop 默认存放在项目内，Linux runtime IPC 可位于
`XDG_RUNTIME_DIR`，详见下文。本页给出各目录、所属组件与内容。
以下路径使用 macOS/Linux 写法；Windows 布局相同，根为 `%USERPROFILE%\.future\`。

```text
~/.future/
├── agent/                     # agent 后端（future-agent）
│   ├── settings.json          # agent 设置（模型默认值、沙箱等）
│   ├── models.json            # provider/模型目录：apiKey、baseUrl、models[]
│   ├── auth.json              # 凭据，按模型 id 或 provider 键控
│   ├── agent.db               # 会话、运行和事件的 SQLite 权威存储
│   ├── sessions/              # 保留的旧 JSONL 迁移来源
│   ├── run-events/            # 保留的旧事件迁移来源
│   ├── agent-instance.lock    # 每用户 Agent 单例锁
│   ├── agent-instance.json    # 安装器可读取的进程身份信息
│   ├── skills/                # 已安装的用户技能（APP_SKILLS_DIR）
│   ├── browser/               # CLI 浏览器工具状态（config.json、profile/、artifacts/）
│   ├── images/                # CLI 图片工具输出目录
│   └── logs/agent.log         # agent 日志（启用日志时）
├── agent-app/                 # 遗留凭据目录（auth.json），向后兼容读取
├── channels/
│   ├── config.json            # 所有通道的配置（见 channels-config.zh-CN.md）
│   ├── <channel>/             # 每通道数据：会话文件、下载的附件
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
├── approval_rule.json         # 用户级路径审批规则
├── windows-capabilities.json  # Windows 沙箱 ACL 清理元数据
├── run/agent.sock             # Unix IPC 回退路径（Windows 不使用）
└── bin/                       # CLI / agent 链接：`future`、`future-agent`（见下）
```

## `~/.future/agent/` — agent 后端

归 `future-agent`（默认每用户本地 IPC 的 gRPC 后端）所有。其配置
完全从本目录的文件读取——没有任何模型相关的 CLI 旗标或环境变量：

- `settings.json` — agent 设置。
- `models.json` — provider 目录，形如
  `{"providers": {"<provider>": {"apiKey": …, "baseUrl": …, "models": [{"id", "name", "contextWindow"}]}}}`。
  `future auth login` 会自动同步此文件；也可以手工编辑。
- `auth.json` — 凭据，先按模型 id、再按 provider、最后按默认条目键控：
  `{"<provider>": {"type": "api_key", "key": …, "baseUrl": …}}`。
- `agent.db` — 会话、消息、运行及回放事件的 SQLite 权威存储。
- `sessions/`、`run-events/<session_id>/` — 保留的旧 JSONL 迁移来源；自定义旧目录的事件来源为 `.run-events/`。迁移后不再更新这些文件。排队 prompt 仍只保存在内存中。
- `agent-instance.lock` — 当前 FutureOS home 的单例锁。测试应隔离 HOME（Windows 同时
  隔离 USERPROFILE），或用 `FUTURE_HOME` / `future agent --home` 把 Agent 指向另一个
  FutureOS home（见[多实例运行](#多实例运行future_home)）；仅更换 TCP 端口无法绕过单例锁。
- `agent-instance.json` — 记录 PID、可执行文件路径、FutureOS home，以及 Windows
  上的进程创建时间，供安装器核对进程身份。异常退出后文件可能残留；以操作系统锁为准。
- `skills/` — 两个技能发现目录之一（`APP_SKILLS_DIR`）；另一个是
  `~/.agents/skills/`（`AGENTS_SKILLS_DIR`）。技能是含 `SKILL.md` +
  YAML frontmatter 的普通目录。
- `browser/` — CLI 浏览器工具状态（`config.json`、Chromium 的 `profile/`、
  截图的 `artifacts/`）。遵循 `FUTURE_HOME`。
- `images/` — CLI 图片生成/编辑工具（`future tools call image …`）的输出目录。
- `logs/agent.log` — 启用日志时写入。

## IPC、审批规则与 Windows 清理

Unix socket 依次选择显式 `FUTURE_AGENT_SOCKET`；设置 `FUTURE_HOME` 时使用该 home 自己的
`<home>/run/agent.sock`（XDG 运行时目录是按用户而非按实例的，被重定向的实例不能绑定在
那里）；Linux 上已设置时使用 `$XDG_RUNTIME_DIR/future/agent.sock`；最后回退到
`~/.future/run/agent.sock`（也是 macOS 默认）。Windows 使用每用户命名管道而不是 socket
文件——home 被重定向时管道名会带上该 home 的标记。`future agent --grpc-addr <host:port>`
显式开启 TCP；客户端可用 `FUTURE_AGENT_GRPC_ADDR` 覆盖（渠道使用 `agent.grpc_addr`），
显式客户端 TCP 地址具有权威性——连接失败会直接报错，不会回退本地 IPC。

用户路径规则位于 `~/.future/approval_rule.json`，项目规则位于
`<workspace>/.future/approval_rule.json`，见[审批与沙箱](../wiki/zh/Sandbox.md)。
Windows 将 capability/ACL 清理元数据保存在 `~/.future/windows-capabilities.json`；
使用 `future agent --reset-windows-sandbox` 清理，不要在 ACL 尚存时手动删除元数据。
权限仍被活动沙箱使用时，reset 会拒绝清理。

## 多实例运行（`FUTURE_HOME`）

`FUTURE_HOME` 替换整个 FutureOS home（即 `~/.future` 根目录本身）。启动时的等价开关是
`future agent --home DIR`（该选项默认取 `$FUTURE_HOME`），这正是第二个完全隔离实例的基础：

```bash
# 实例 A：默认 home
future agent

# 实例 B：自己的锁、数据库、会话、日志、技能与 IPC 端点
future agent --home /tmp/futureos-b

# 客户端设置同一个 home，即接入实例 B
FUTURE_HOME=/tmp/futureos-b future tui
```

FutureOS 自己拥有的一切都会随之移动：`<home>/agent`（settings、models、`auth.json`、
`agent.db`、sessions、`agent-instance.lock`、skills、images、logs）、
`<home>/run/agent.sock`、`<home>/approval_rule.json` 与
`<home>/windows-capabilities.json`。因此两个实例不会共享锁、数据库或端点，相互也无需停掉。

**不**随之移动的是操作系统的 home：针对 `~/.ssh` 的沙箱守卫和共享的 `~/.agents/skills`
目录仍指向真实用户 home。覆盖值必须是绝对路径——相对或空的 `FUTURE_HOME` 会被忽略，
`future agent --home` 会直接报错——目录不存在时会自动创建。客户端通过设置同一个
`FUTURE_HOME` 选择实例；客户端自身的 UI 状态（`~/.future/tui/`、桌面应用的
`~/.future/app/`）仍留在真实 home 中。

## `~/.future/agent-app/` — 遗留凭据目录

agent 解析 `auth.json` 时会先读 `~/.future/agent/auth.json`，只有该文件无法加载时
才回退到 `~/.future/agent-app/auth.json`（向后兼容旧版 GUI 写入的凭据）。
规范位置的空对象仍然是权威配置，不会重新启用旧密钥；GUI 的文件访问
守卫把 `agent/` 与 `agent-app/` 都视为凭据位置。新的写入都落到 `~/.future/agent/`。
这不表示沙箱隔离了凭据：`auth.json` 当前为 CLI 技能保留了硬拒绝例外，见
[SECURITY](../../SECURITY.md)。

## `~/.future/channels/` — 渠道桥

归 `future-channel` 所有。所有通道——[通道 provider](channels-providers.zh-CN.md)
里列出的框架通道，以及自建桥的两个——都配在同一份 `config.json` 里：一个 `agent`
块、每个通道一个 `providers.<id>` 块，另有遗留的顶层 `feishu` / `dingtalk` 块。
完整 schema 与默认值见 [channels-config.zh-CN.md](channels-config.zh-CN.md)。
若文件不存在，桥会写入默认模板并退出，提示编辑后重启。

`<channel>/` 保存该通道自己持久化的东西：会话映射文件，以及它下载过的附件。
`feishu/` 是飞书桥的目录，其中还保留了从平台收到的文件。

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
`~/.future/loop/` 不是默认位置）。见
[loop-control-plane.zh-CN.md](../architecture/loop-control-plane.zh-CN.md)。

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
