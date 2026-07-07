<p align="center">
  <a href="https://future-os.cn/">FutureOS</a>
</p>
<p align="center">
  <a href="https://github.com/futuregene/future-os/tree/main/docs/wiki/zh"><img src="https://img.shields.io/badge/Docs-Wiki-FFD700?style=for-the-badge" alt="Documentation"></a>
  <a href="https://github.com/futuregene/future-os/blob/main/LICENSE"><img src="https://img.shields.io/badge/License-MIT-green?style=for-the-badge" alt="License: MIT"></a>
  <a href="README.md"><img src="https://img.shields.io/badge/Lang-English-blue?style=for-the-badge" alt="English"></a>
</p>

<p align="center">
  <img src="docs/banner.png" alt="FutureOS" width="600">
</p>

# FutureOS

> 本地优先的 AI Agent 工作台——终端、桌面、消息平台，一个后端全搞定。

FutureOS 提供统一的 AI Agent 体验，覆盖 TUI、GUI、CLI、飞书和钉钉。Rust 后端负责 LLM 编排、工具执行和会话持久化。TypeScript 前端和 Tauri/React 桌面应用通过 gRPC 连接。写代码、做调研、管理文件——从终端、聊天软件或原生桌面窗口，无缝切换。

## 特性

| 类别 | 说明 |
|---|---|
| **多端统一** | 终端界面 (TUI)、桌面应用 (GUI)、命令行 (CLI)、飞书机器人、钉钉机器人——一个 Agent，无处不在 |
| **模型灵活** | 内置 906+ 模型（OpenAI、Anthropic、DeepSeek、Qwen 等）；通过 `models.json` 自定义 Provider；支持模型范围限定 |
| **流式输出与思考链** | 实时 token 流式传输，可折叠的思考链展示；可配置思考深度（off ↔ xhigh） |
| **工具执行** | 读写、编辑、bash，带审批控制和沙箱保护（关闭 / 手动 / macOS Seatbelt）；上下文超 90% 自动压缩 |
| **会话持久化** | JSONL 格式存储，支持 fork、clone、树形导航和问答计数 |
| **自动压缩与重试** | 上下文自动压缩；上下文超长时指数退避自动重试 |
| **Channel Bridge** | 飞书和钉钉机器人——markdown 流式输出、斜杠命令、通过聊天管理会话 |
| **技能系统** | 可插拔的 YAML 定义 Skill 包，从多目录自动发现 |
| **跨平台** | macOS、Linux、Windows（GUI 基于 Tauri + WebView2） |

## 快速开始

### 环境要求

- **Rust** 1.80+
- **Node.js** 22+ / **Bun**（TUI 运行时）
- macOS / Linux / Windows

### 构建与运行（60 秒）

```bash
# 克隆并构建
git clone https://github.com/futuregene/future-os.git
cd future-os
make install   # 安装所有依赖
make build     # 构建 agent + TUI + CLI + GUI

# 启动 Agent（gRPC 服务，监听 127.0.0.1:50051）
make run-agent &

# 启动客户端
make run-tui    # 终端界面
make run-gui    # 桌面应用
```

### CLI 快速上手

```bash
future auth login                          # 登录
future run "用 Python 写个排序函数"         # 单次对话
future tui                                 # 打开 TUI
future agent start                         # 将 Agent 安装为系统服务（macOS launchctl / Linux systemd）
```

### 常用斜杠命令（TUI）

| 命令 | 说明 |
|---|---|
| `/help` | 显示所有命令和快捷键 |
| `/model <id>` | 切换模型（如 `deepseek-v4-pro`） |
| `/status` | 会话状态、token 用量、费用 |
| `/sessions` | 浏览和切换会话 |
| `/new` | 新建会话 |
| `/stop` | 中断当前生成 |
| `/compact` | 压缩对话上下文 |
| `/scoped-models` | 配置模型启用/禁用列表 |
| `/tree` | 会话树（含 fork/clone 层级） |

### 键盘快捷键（TUI）

| 按键 | 功能 |
|---|---|
| `ctrl+p` | 循环切换模型 |
| `ctrl+t` | 循环切换思考级别 |
| `ctrl+r` | 浏览会话列表 |
| `ctrl+c` | 中断 / 退出 |
| `↑↓` | 滚动聊天 / 列表导航 |
| `Tab` | 自动补全 |

## 架构

```
                         ┌──────────────────────────┐
                         │   Rust Agent (gRPC)      │
                         │   LLM · 工具 · 会话       │
                         │   127.0.0.1:50051        │
                         └──────────┬───────────────┘
                                    │
          ┌─────────────────────────┼─────────────────────────┐
          │                         │                         │
   TypeScript TUI           Channel Bridge             TypeScript CLI
   (终端, bun)              飞书 · 钉钉               认证 · MCP · 技能 · run
                             
   Tauri / React GUI
   (桌面, WebView)
```

所有客户端独立通过 gRPC 连接 Agent，互不依赖。

- **Agent** (`agent/`) — Rust，tokio，tonic。LLM 客户端（OpenAI 兼容 HTTP+SSE），工具执行，JSONL 会话持久化，gRPC 服务。
- **TUI** (`tui/`) — TypeScript，bun。差分渲染，Markdown（marked），Kitty 图片协议，14 个 UI 组件。
- **GUI** (`gui/`) — Tauri 2 + React + TypeScript。独立的 gRPC 客户端。三栏布局（导航 / 对话 / 上下文），审批提示，技能浏览，设置。
- **CLI** (`cli/`) — TypeScript。设备码 OAuth 登录，服务管理，MCP 工具调用，TUI 启动器。
- **Channel Bridge** (`channels/`) — Rust。飞书（pbbp2 WebSocket + CardKit 流式）和钉钉（Stream Mode）。

## 配置

所有配置位于 `~/.future/` 目录：

| 路径 | 说明 |
|---|---|
| `agent/settings.json` | 队列模式、压缩、重试、权限级别 |
| `agent/auth.json` | API Key（FutureGene + 自定义 Provider） |
| `agent/models.json` | 自定义模型配置（Base URL、API Key、兼容参数） |
| `agent/sessions/` | JSONL 会话文件（每个会话一个文件） |
| `tui/settings.json` | 默认模型、思考级别、模型范围列表 |
| `channels/config.json` | 飞书/钉钉凭据、Agent 地址、Channel 默认参数 |

## 开发

```bash
make lint     # 全量检查（agent clippy + channels clippy + TUI tsc + CLI tsc + GUI eslint）
make fmt      # cargo fmt（agent + channels）
make test     # cargo test（agent）
make clean    # 清理所有构建产物
```

### Proto

权威 API 定义在 `proto/future.proto`。生成代码在构建时通过 `build.rs`（tonic-build）自动更新。手动生成：

```bash
make generate-proto          # agent + channels
cd tui && npm run generate-proto  # TUI 内嵌 proto
```

## License

MIT
