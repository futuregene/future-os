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
| **工具执行** | read, write, edit, shell，带审批控制和沙箱保护（关闭 / 手动 / macOS Seatbelt）；上下文超 90% 自动压缩 |
| **会话持久化** | JSONL 格式存储，支持 fork、clone、树形导航和问答计数 |
| **自动压缩与重试** | 上下文自动压缩；上下文超长时指数退避自动重试 |
| **Channel Bridge** | 飞书和钉钉机器人——markdown 流式输出、斜杠命令、通过聊天管理会话 |
| **技能系统** | 可插拔的 YAML 定义 Skill 包，从多目录自动发现 |
| **跨平台** | macOS、Linux、Windows（GUI 基于 Tauri + WebView2） |

## 快速开始

### 环境要求

每个平台的完整构建（agent + TUI + CLI + GUI）都需要：

- **Rust** 1.97+（由 `rust-toolchain.toml` 固定）
- **Node.js** 24+（见 `.nvmrc`）
- **Bun** —— 必需项，非可选：TUI 构建和 CLI/GUI 打包均使用 `bun build`
- 可选：**Python 3** —— 仅 `make generate-models` 需要
- 可选：**protoc**（Protocol Buffers 编译器）—— 仅 `make generate-proto` 需要；生成的代码已提交，正常构建无需安装

### 各平台环境搭建与构建

```bash
git clone https://github.com/futuregene/future-os.git
cd future-os
```

#### macOS

安装依赖：

```bash
xcode-select --install                                            # 系统工具链（Tauri 依赖）
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh    # Rust
brew install node oven-sh/bun/bun                                 # Node.js 24+ / Bun（也可用 nvm，见 .nvmrc）
brew install protobuf                                             # 可选 —— 仅 make generate-proto 需要
```

构建：

```bash
make install        # 构建全部组件，安装到 /opt/homebrew/bin
make install-nogui  # 仅终端组件（跳过 Tauri GUI）
make package-gui    # 桌面安装包 → .app + .dmg，位于 gui/src-tauri/target/release/bundle/
scripts/build-macos-dmg.sh  # 本地 DMG；检测到 Developer ID 证书时自动签名
```

`scripts/build-macos-dmg.sh` 会将 agent、CLI sidecar 和 GUI 一起构建。它会
自动使用 macOS 钥匙串中唯一的 `Developer ID Application` 身份并输出
`*-sign.dmg`；如果无法确定唯一身份，则自动降级为普通 DMG。证书选择、输出
目录和 Apple 公证参数请运行脚本的 `--help` 查看。

#### Linux（Debian/Ubuntu）

安装依赖：

```bash
sudo apt update
sudo apt install -y build-essential mold libssl-dev \
  libwebkit2gtk-4.1-dev libgtk-3-dev librsvg2-dev libayatana-appindicator3-dev patchelf
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh    # Rust
curl -fsSL https://bun.sh/install | bash                          # Bun
# Node.js 24+ —— 用 nvm install 自动读取仓库的 .nvmrc
sudo apt install -y protobuf-compiler                             # 可选 —— 仅 make generate-proto 需要
```

> x86_64 上必须安装 `mold` —— `.cargo/config.toml` 会给链接器传 `-fuse-ld=mold`。ARM Linux 不需要。

构建：

```bash
make install        # 构建全部组件，安装到 /usr/local/bin（sudo）
make install-nogui  # 仅终端组件（跳过 Tauri GUI）
make package-gui    # 桌面安装包 → .deb，位于 gui/src-tauri/target/release/bundle/
```

#### Windows

安装工具链：

1. **Visual Studio Build Tools**，勾选「使用 C++ 的桌面开发」工作负载（MSVC + Windows SDK）—— Rust MSVC 工具链和 Tauri 都需要。执行 `winget install Microsoft.VisualStudio.2022.BuildTools` 后在安装器中勾选该工作负载，或从 [visualstudio.com](https://visualstudio.microsoft.com/downloads/) 安装。
2. **Rust**：`winget install Rustlang.Rustup`（host triple 为 `x86_64-pc-windows-msvc`）
3. **Node.js 24+**：`winget install OpenJS.NodeJS`，或从 [nodejs.org](https://nodejs.org) 下载
4. **Bun**：`winget install Oven-sh.Bun`（或 `powershell -c "irm bun.sh/install.ps1 | iex"`）
5. **WebView2 Runtime**：Windows 10/11 自带 —— 是 GUI 的*运行时*依赖，新系统无需安装

Windows 上无需 `make` —— 以下 PowerShell 命令与各平台的 make 目标逐步对应，在仓库根目录执行。

**终端组件** —— 对应 `make install-nogui`：

```powershell
# Rust 组件：agent + channel bridge               （对应 make build-agent / build-channels）
cargo build --release --manifest-path agent/Cargo.toml
cargo build --release --manifest-path channels/Cargo.toml

# TypeScript 组件：TUI + CLI                      （对应 make build-tui / build-cli）
Push-Location tui; npm install; npm run gen-version; npm run build; bun build --compile dist/index.js --outfile dist/future-tui.exe; Pop-Location
Push-Location cli; npm install; npm run gen-version; npm run build; bun build --compile dist/index.js --outfile dist/future.exe --external chromium-bidi; Pop-Location

# 安装到 %USERPROFILE%\.future\bin                （对应 install-* 中的复制步骤）
$bin = "$env:USERPROFILE\.future\bin"
New-Item -ItemType Directory -Force -Path $bin | Out-Null
Copy-Item target\release\future-agent.exe, target\release\future-channel.exe, tui\dist\future-tui.exe, cli\dist\future.exe $bin

# 内置技能 —— make install-skills 使用符号链接；Windows 上改用 CLI 安装
& "$bin\future.exe" skills install
```

**桌面应用** —— `make install` 中的 GUI 部分（需先执行上面的终端组件步骤，sidecar 来自其产物）：

```powershell
# 将 agent + CLI 以 host triple 命名暂存为 Tauri sidecar
$triple = (rustc -Vv | Select-String '^host:').Line.Split(' ')[1]
New-Item -ItemType Directory -Force -Path gui\src-tauri\binaries | Out-Null
Copy-Item target\release\future-agent.exe "gui\src-tauri\binaries\future-agent-$triple.exe"
Copy-Item cli\dist\future.exe "gui\src-tauri\binaries\future-$triple.exe"

# 构建应用并安装为 future-gui.exe                 （对应 make install-gui）
Push-Location gui; npm install; npx tauri build --no-bundle; Pop-Location
Copy-Item gui\src-tauri\target\release\futureos.exe "$env:USERPROFILE\.future\bin\future-gui.exe"
```

**安装包** —— 对应 `make package-gui`（sidecar 暂存后执行）：

```powershell
node scripts\version.mjs --set-bundle
Push-Location gui; npm run tauri:build; Pop-Location   # → NSIS 安装包 .exe，位于 gui\src-tauri\target\release\bundle\nsis\
```

补充说明：

- `scripts\start-gui-test.bat` 可用本地构建的 agent 以开发模式启动 GUI。
- `scripts/` 下的脚本（`build-macos-dmg.sh`、`build-windows-portable.ps1`、`build-windows-installer.ps1`）把上述步骤封装为单条命令，复刻 CI 打包流水线（DMG / 免安装 zip / NSIS 安装包），适用于需要与 CI 完全一致产物的特殊场景。脚本会前置检查工具链，且要求 `protoc`（`brew install protobuf` / `choco install protoc`）。其产物包含 GUI、agent 和 CLI，不含 TUI。

### 配置模型

Agent 至少需要一个带 API key 的模型才能回复。三种方式:

**A —— FutureOS 托管模型。** 设备码登录会自动配好 key 和模型列表:

```bash
future auth login
```

**B —— 使用已知 Provider。** 将 API Key 放入 `~/.future/agent/auth.json`，按 Provider 名索引。查看[内置模型目录](docs/wiki/zh/Models.md)了解所有支持的 Provider——多数自带 Base URL，模型自动发现：

```json
{
  "openai": { "type": "api_key", "key": "sk-..." }
}
```

对于 Base URL 含用户特定值的 Provider（如 Azure 的 `YOUR_RESOURCE`），在 `auth.json` 中添加 `baseUrl` 字段：

```json
{
  "azure": { "type": "api_key", "key": "sk-...", "baseUrl": "https://my-resource.openai.azure.com/openai/v1" }
}
```

**C —— 自定义 Provider。** 不在内置目录中的 Provider，在 `~/.future/agent/models.json` 中指定完整信息：

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

### 安装技能（可选）

FutureOS 内置一套精选技能——针对深度研究、浏览器自动化、文档处理等常见任务的专业指令集。技能维护在 [future-skills](https://github.com/futuregene/future-skills) 仓库，是我们的推荐默认配置。

```bash
make install-skills                          # 从内置 skills/ 子模块创建符号链接
# 或从平台目录安装：
future skills install                        # 安装所有 future-* 技能（约 13 个）
future init                                  # 安装技能；macOS/Linux 上同时链接本地命令
```

> 技能以符号链接方式装入 `~/.future/agent/skills/`，Agent 会自动发现。使用 `future skills list` 查看可用技能，`future skills update` 升级。
> 在 macOS 和 Linux 上，`future init` 还会将 `future` 及同目录中存在的 `future-agent` 软链到 `~/.future/bin/`；可按需将该目录加入 `PATH`。

### 启动 Agent

所有客户端——TUI、GUI、CLI、channels——都只是轻量 gRPC 客户端。**必须先启动 Agent**,监听 `127.0.0.1:50051`：

| 模式 | 命令 | 适用场景 |
|---|---|---|
| **前台** | `make run-agent` | 从源码构建并运行，日志打到 stdout，Ctrl-C 停止 |
| **前台** | `future-agent` | 直接运行已构建好的 Agent，日志打到 stdout，Ctrl-C 停止 |

然后启动任意客户端：

```bash
future-tui           # 终端界面（需先 make install）
future-gui           # 桌面应用（需先 make install）
# 开发模式下直接运行（会自动构建）：
make run-tui         # 终端界面
make run-gui         # 桌面应用
```

> 客户端如果报连接 / gRPC 错误,几乎都是 Agent 还没启动——见 [故障排查](#故障排查)。

### CLI 快速上手

```bash
future run "用 Python 写个排序函数"         # 单次对话
future-tui                                 # 打开 TUI
future-gui                                 # 启动桌面应用
future-channel                             # 启动 Channel Bridge
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
- **CLI** (`cli/`) — TypeScript。设备码 OAuth 登录，单次对话（`run`），MCP 工具调用，技能管理，环境诊断（`doctor`）。
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
| 客户端报连接 / gRPC 错误退出 | Agent 没启动。先启动它(`future-agent` 或 `make run-agent`),并确认端口没被占用:`lsof -i :50051`。 |
| Agent 回复鉴权 / "no model" 错误 | 还没配置模型。运行 `future auth login`,或在 `models.json` 里加一个 provider——见 [配置模型](#配置模型)。 |
| GUI 找不到 Agent 二进制 | `make install-gui` 用你的宿主 target triple 复制 sidecar。如果 triple 与自动检测的不一致，手动复制：`cp target/debug/future-agent gui/src-tauri/binaries/future-agent-$(rustc -vV | sed -n 's/^host: //p')`。 |
| 构建时报 "unable to find linker 'mold'" | 安装 mold：`sudo apt install mold`（仅限 Linux x86_64，ARM Linux 不需要）。 |
| Linux 上 GUI 构建失败(webkit / gtk 报错) | 安装 Tauri 系统依赖——见 [Linux 环境搭建](#linux-debianubuntu)。 |

## License

MIT
