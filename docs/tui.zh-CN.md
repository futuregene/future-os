# FutureOS 终端界面（TUI）

TUI 是终端客户端：`future-tui`，默认通过**每用户本地 IPC**连接 Agent。
macOS/Linux 使用 Unix socket，Windows 使用仅当前用户可访问的命名管道。
Unix 优先使用 `FUTURE_AGENT_SOCKET`；Linux 否则使用 `$XDG_RUNTIME_DIR/future/agent.sock`，
未设置时回退到 `~/.future/run/agent.sock`（也是 macOS 默认路径）。
若无可连接 Agent，TUI 会启动 sidecar 并在退出时关闭自己启动的进程，无需手动预先启动。
也可以自行运行：

```bash
future agent      # 终端 1：agent
future tui        # 终端 2：终端界面
```

远程/开发环境可给 Agent 传入 `--grpc-addr <host:port>` 显式启用 TCP，客户端设置
`FUTURE_AGENT_GRPC_ADDR=<host:port>`（先尝试显式 TCP，再回退本地 IPC）。

`future tui <args>` 在进程内运行 TUI；独立二进制 `future-tui` 与之等价，但已
不再默认安装（需要时用 `cargo build -p future-tui` 构建）。`future tui --help`
可查看全部选项（print 模式、`--list-models`、`--session` 等）。

- 构建 / 安装：见 [构建与安装](build-and-install.zh-CN.md)。
- 会话持久化、模型配置、工具审批都由 agent 处理；TUI 只是前端。
- 新会话默认权限为 `all`，TUI 不主动开启桌面沙箱策略。CLI 的 `future run --permission none` 禁止所有工具
  调用，而非只禁止写入；区别见[沙箱指南](wiki/zh/Sandbox.md)。

## 斜杠命令

以下命令均由 TUI 本地处理（不会发给模型）。命令名不区分大小写；`arg` 为命令
名之后的内容。

| 命令 | 用途 |
|---|---|
| `/help` | 显示帮助浮层（快捷键 + 核心命令） |
| `/model [name]` | 直接设置模型；不带参数时打开模型选择器 |
| `/sessions` | 浏览并切换会话 |
| `/new` | 新建会话 |
| `/clone` | 克隆当前会话（在新分支继续） |
| `/fork` | 从选中的消息分叉 |
| `/tree` | 会话树（fork/clone 层级） |
| `/name <name>` | 设置会话名称 |
| `/scoped-models` | 配置模型启用/禁用列表 |
| `/compact` | 压缩对话上下文 |
| `/status` | 会话状态、模型、token 用量、成本 |
| `/stop` | 停止当前生成 |
| `/cwd <dir>` | 切换工作目录 |
| `/approve <request-id>` | 批准待执行工具 |
| `/reject <request-id>` | 拒绝待执行工具 |
| `/cancel <run-id>` | 取消排队中的运行 |
| `/reload` | 重载技能 + 上下文文件 |
| `/export` | *TUI 中不可用*（占位，回复提示） |
| `/import` | *TUI 中不可用*（占位，回复提示） |

> 应用内帮助浮层（`/help`）只列出其中一部分命令；以上完整分发集为准
> （tui/src/app.rs `handle_submit`）。

## 键盘快捷键

| 按键 | 动作 |
|---|---|
| `ctrl+p` | 循环切换模型 |
| `ctrl+t` | 循环切换思考级别 |
| `ctrl+o` | 展开 / 收起思考内容 |
| `ctrl+r` | 浏览会话 |
| `ctrl+c` | 中断 / 退出 |
| `tab` | 自动补全 |
| `enter` | 提交 / 接受 |
| `escape` | 关闭弹窗 |
| `↑↓` | 滚动 / 导航列表 |

## 设置与本地文件

TUI 把客户端侧设置持久化到 `~/.future/tui/settings.json`（如 `defaultModel`、
`defaultThinkingLevel`、`defaultPermissionLevel`、`enabledModelIds`）。可选
的用户键位覆盖可放在 `~/.future/tui/keybindings.json`。日志：`PI_DEBUG_REDRAW=1` 时把调试重绘日志写入
`~/.future/tui/debug.log`；`PI_TUI_WRITE_LOG=1` 时原始屏幕写入记录到
`~/.future/tui/write.log`。

## 排障

| 症状 | 解决办法 |
|---|---|
| 启动即连接 / gRPC 错误 | 未找到或未能启动 Agent。检查 sidecar 错误、用户与 IPC 环境，或手动运行 `future agent`；仅显式 TCP 模式检查配置端口是否占用。 |
| auth / 「no model」错误 | 未配置模型。运行 `future auth login`，或向 `~/.future/agent/models.json` 添加 provider——见仓库 README「配置模型」 |

参见：[目录布局](directory-layout.zh-CN.md)（`~/.future/` 下各目录职责）、
wiki [命令行工具](wiki/zh/CLI.md) / [设置](wiki/zh/Settings.md)（桌面应用版）。
