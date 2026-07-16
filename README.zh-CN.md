<p align="center">
  <a href="https://github.com/futuregene/future-os/wiki"><img src="https://img.shields.io/badge/Docs-Wiki-FFD700?style=for-the-badge" alt="Documentation"></a>
  <a href="https://github.com/futuregene/future-os/blob/main/LICENSE"><img src="https://img.shields.io/badge/License-MIT-green?style=for-the-badge" alt="License: MIT"></a>
  <a href="https://github.com/futuregene/future-skills"><img src="https://img.shields.io/badge/Skills-future--skills-blue?style=for-the-badge" alt="Skills"></a>
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
| **模型灵活** | 内置 1000+ 模型，覆盖 100+ Provider（[完整目录](docs/wiki/zh/Models.md)）；通过 `models.json` 自定义 Provider；支持模型范围限定 |
| **流式输出与思考链** | 实时 token 流式传输，可折叠的思考链展示；可配置思考深度（off ↔ xhigh） |
| **工具执行** | 读写、编辑、bash，带审批控制和沙箱保护（关闭 / 手动 / macOS Seatbelt）；上下文超 90% 自动压缩 |
| **会话持久化** | JSONL 格式存储，支持 fork、clone、树形导航和问答计数 |
| **自动压缩与重试** | 上下文自动压缩；上下文超长时指数退避自动重试 |
| **Channel Bridge** | 飞书和钉钉机器人——markdown 流式输出、斜杠命令、通过聊天管理会话 |
| **技能系统** | 可插拔的 YAML 定义 Skill 包，从多目录自动发现 |
| **跨平台** | macOS、Linux、Windows（GUI 基于 Tauri + WebView2） |

## 快速开始

### 环境要求

完整 `make build`（agent + TUI + CLI + GUI）所需：

- **Rust** 1.96+（由 `rust-toolchain.toml` 固定）
- **Node.js** 24+（见 `.nvmrc`）
- **Bun** —— 必需项，非可选：TUI 构建和 CLI/GUI 打包均使用 `bun build`
- **Linux 必需**（所有构建都需要）：
  - `sudo apt install build-essential mold`
- **Tauri 系统依赖**（构建 GUI 需要）：
  - macOS：`xcode-select --install`
  - Linux（Debian/Ubuntu）：`sudo apt install libwebkit2gtk-4.1-dev libgtk-3-dev librsvg2-dev libayatana-appindicator3-dev patchelf`
  - Windows：WebView2 Runtime（Win 10/11 自带）+ MSVC 构建工具
- 可选：**Python 3** —— 仅 `make generate-models` 需要
- 可选：**protoc**（Protocol Buffers 编译器）—— 仅 `make generate-proto` 需要；生成的代码已提交，正常构建无需安装
- 平台：macOS / Linux / Windows

### 构建与安装

```bash
git clone https://github.com/futuregene/future-os.git
cd future-os
make install   # 构建全部组件并安装到系统路径
```

二进制安装路径：macOS `/opt/homebrew/bin`、Linux `/usr/local/bin`、Windows `%USERPROFILE%\.future\bin`。

> **只构建终端版？** 跳过 GUI 工具链：`make install-nogui`

### 配置模型

Agent 至少需要一个带 API key 的模型才能回复。四种方式:

**A —— FutureOS 托管模型。** 设备码登录会自动配好 key 和模型列表:

```bash
future auth login
```

**B —— 使用已知 Provider。** 将 API Key 放入 `~/.future/agent/auth.json`，按 Provider 名索引。[目录](docs/wiki/zh/Models.md)中的 Provider 自动使用内置 Base URL 和模型列表：

```json
{
  "openai": { "type": "api_key", "key": "sk-..." }
}
```

**C —— 已知 Provider，自定义 Base URL。** 部分 Provider 的 Base URL 含占位符（如 Azure 的 `YOUR_RESOURCE`、Google Vertex 的 `PROJECT_ID`），需在 `~/.future/agent/models.json` 中指定 `apiKey` + `baseUrl`，模型仍自动发现：

```json
{
  "providers": {
    "azure": {
      "apiKey": "sk-...",
      "baseUrl": "https://my-resource.openai.azure.com/openai/v1"
    }
  }
}
```

**D —— 自定义 Provider。** 不在内置目录中的 Provider，需在 `models.json` 中指定完整模型列表：

```json
{
  "providers": {
    "my-provider": {
      "apiKey": "sk-...",
      "baseUrl": "https://my-api.example.com/v1",
      "models": [
        { "id": "my-model", "name": "My Model", "contextWindow": 128000 }
      ]
    }
  }
}
```

随时用 TUI 里的 `/model <id>` 切换当前模型,或 `ctrl+p` 循环切换。

### 启动 Agent

所有客户端——TUI、GUI、CLI、channels——都只是轻量 gRPC 客户端。**必须先启动 Agent**,监听 `127.0.0.1:50051`。启动方式有两种,对应两种场景:

| 模式 | 命令 | 适用场景 |
|---|---|---|
| **开发 / 前台** | `make run-agent` | 开发调试 Agent。从源码重新构建,跑在当前终端,日志打到 stdout,Ctrl-C 停止。 |
| **后台服务** | `future agent start` | 日常使用。安装为托管服务(macOS launchctl / Linux systemd / Windows sc),开机自启,启动一次即可。用 `future agent stop \| restart \| status` 管理。 |

然后启动任意客户端：

```bash
future tui           # 终端界面（需先 make install）
future gui           # 桌面应用（需先 make install）
# 开发模式下直接运行（会自动构建）：
make run-tui         # 终端界面
make run-gui         # 桌面应用
```

> 客户端如果报连接 / gRPC 错误,几乎都是 Agent 还没启动——见 [故障排查](#故障排查)。

### CLI 快速上手

```bash
future run "用 Python 写个排序函数"         # 单次对话
future tui                                 # 打开 TUI
future gui                                 # 启动桌面应用
future channel start                       # 启动 Channel Bridge
future --help                              # 查看全部命令
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
        ┌───────────────┬───────────┴───────────┬───────────────┐
        │               │                       │               │
 TypeScript TUI   Tauri/React GUI       TypeScript CLI   Channel Bridge
 (终端, bun)     (桌面, WebView)         认证 · MCP       飞书 · 钉钉
```

所有客户端独立通过 gRPC 连接 Agent，互不依赖。

- **Agent** (`agent/`) — Rust，tokio，tonic。LLM 客户端（OpenAI 兼容 HTTP+SSE），工具执行，JSONL 会话持久化，gRPC 服务。
- **TUI** (`tui/`) — TypeScript，bun。差分渲染，Markdown，Kitty 图片协议，14 个 UI 组件。
- **GUI** (`gui/`) — Tauri 2 + React + TypeScript。三栏布局（导航 / 对话 / 上下文），审批提示，技能浏览，设置。
- **CLI** (`cli/`) — TypeScript。设备码 OAuth 登录，服务管理，MCP 工具调用，TUI/GUI 启动器。
- **Channel Bridge** (`channels/`) — Rust。飞书（pbbp2 WebSocket + CardKit 流式）和钉钉（Stream Mode）。

## 配置

所有配置位于 `~/.future/` 目录：

| 路径 | 组件 | 说明 |
|---|---|---|
| `agent/settings.json` | Agent | 队列模式、压缩、重试、最大轮次 |
| `agent/auth.json` | Agent | API Key（按 Provider 索引） |
| `agent/models.json` | Agent | 自定义模型配置（Base URL、兼容参数） |
| `agent/sessions/` | Agent | JSONL 会话文件 |
| `tui/settings.json` | TUI | 默认模型、思考级别、启用的模型列表 |
| `app/app.db` | GUI | SQLite — 会话、运行、产出、审批、设置 |
| `channels/config.json` | Channels | Agent gRPC 地址、飞书/钉钉凭据 |

## 开发

```bash
make build    # 构建所有组件（不安装到系统）
make lint     # 全量检查（agent + channels + TUI + CLI + GUI）
make fmt      # cargo fmt（agent + channels）
make test     # cargo test（agent）
make clean    # 清理构建产物 + 已安装的二进制
```

### Proto

权威 API 定义在 `proto/future.proto`。生成的 Rust/TS 代码已提交到仓库——正常构建不会触碰。修改 `.proto` 文件后，运行：

```bash
make generate-proto          # agent + channels + TUI
```

## 故障排查

| 现象 | 解决 |
|---|---|
| 客户端报连接 / gRPC 错误退出 | Agent 没启动。先启动它(`make run-agent` 或 `future agent start`),并确认端口没被占用:`lsof -i :50051`。 |
| Agent 回复鉴权 / "no model" 错误 | 还没配置模型。运行 `future auth login`,或在 `models.json` 里加一个 provider——见 [配置模型](#配置模型)。 |
| GUI 找不到 Agent 二进制 | `make install-gui` 用你的宿主 target triple 复制 sidecar。如果 triple 与自动检测的不一致，手动复制：`cp agent/target/debug/future-agent gui/src-tauri/binaries/future-agent-$(rustc -vV | sed -n 's/^host: //p')`。 |
| 构建时报 "unable to find linker 'mold'" | 安装 mold：`sudo apt install mold`（仅限 Linux x86_64，ARM Linux 不需要）。 |
| Linux 上 GUI 构建失败(webkit / gtk 报错) | 安装 Tauri 系统依赖——见 [环境要求](#环境要求)。 |

## License

MIT
