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
| **Loop 控制面** | `future-loop`：持久化目标/todos/门禁/监控、quota should-run 内核、事件溯源状态、验证器、扩展与多 agent（[指南](docs/loop-control-plane.zh-CN.md)）——基于 [loopx](https://github.com/huangruiteng/loopx) 的 Rust 改写版，针对 FutureOS 做了定制 |
| **跨平台** | macOS、Linux、Windows（GUI 基于 Tauri + WebView2） |

## 快速开始

### 安装

从预编译安装包或安装脚本安装 FutureOS——无需下载源码。各平台（macOS / Linux /
Windows、桌面应用、`future-loop` 控制面）的详细安装步骤见
**[构建与安装](docs/build-and-install.zh-CN.md)** 文档。

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

### 启动 Agent

所有客户端——TUI、GUI、CLI、channels——都只是轻量 gRPC 客户端。**必须先启动 Agent**，监听 `127.0.0.1:50051`：

```bash
future-agent      # 在终端启动 agent（日志打到 stdout，Ctrl-C 停止）
```

然后启动任意客户端：

```bash
future-tui        # 终端界面
future-gui        # 桌面应用
future-channel    # 渠道桥接
```

> 客户端如果报连接 / gRPC 错误，几乎都是 Agent 还没启动——见 [故障排查](#故障排查)。

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

## 故障排查

| 现象 | 解决 |
|---|---|
| 客户端报连接 / gRPC 错误退出 | Agent 没启动。先启动它(`future-agent`)，并确认端口没被占用：`lsof -i :50051`。 |
| Agent 回复鉴权 / "no model" 错误 | 还没配置模型。运行 `future auth login`，或在 `models.json` 里加一个 provider——见 [配置模型](#配置模型)。 |
| 构建 / 安装问题 | 见 [构建与安装](docs/build-and-install.zh-CN.md)（平台工具链、链接器、GUI 打包）。 |

## License

MIT
