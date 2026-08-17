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

> 同一个 AI Agent，处处随你——终端、桌面、手机、飞书与钉钉。

FutureOS 提供统一的 AI Agent 体验，覆盖终端界面 (TUI)、桌面应用 (GUI)、移动端 App（Android · iOS）、命令行 (CLI) 和 IM 机器人——支持 macOS、Linux、Windows 与你的手机。写代码、做调研、管理文件——从终端、聊天软件、原生桌面窗口或口袋里的手机，无缝切换。

## 特性

| 类别 | 说明 |
|---|---|
| **多端统一** | 终端界面 (TUI)、桌面应用 (GUI)、移动端 App（Android · iOS）、命令行 (CLI)、IM 机器人——一个 Agent，无处不在 |
| **移动端 App（Android · iOS）** | 真正手机原生的 Agent 体验——多数 Agent 运行时只有桌面端；FutureOS 通过 CI 交付 Android（APK）与 iOS（TestFlight）构建，由同一个 gRPC Agent 服务驱动 |
| **模型灵活** | 内置 3800+ 模型，覆盖 140+ Provider（[目录](docs/wiki/zh/Models.md)）；通过 `models.json` 自定义 Provider；支持模型范围限定 |
| **Agent 服务** | Agent 以独立 gRPC 服务运行——运行时与 TUI、桌面端、移动端、IM 渠道桥、loop 控制面解耦，为新的客户端与扩展留足空间 |
| **极简工具执行** | read, write, edit, shell，带审批控制和沙箱保护（关闭 / 手动 / macOS Seatbelt）——Pi 式极简主义：工具集精简，杜绝 prompt 膨胀 |
| **可分支会话** | 像仓库一样为对话开分支——fork、clone、树形导航，JSONL 存储 |
| **强大的预设技能** | 内置 14+ 技能开箱即用，覆盖日常 Agent 场景——图片读取与生成、PDF/Word 解析、网页搜索、浏览器控制、幻灯片与软件安装（[builtin](https://github.com/futuregene/future-skills/tree/main/builtin)） |
| **Loop 工程** | 持久化目标/todos/门禁/监控，支撑 24+ 小时长程任务连续执行——确定性 should-run 内核、事件溯源状态、硬校验（证据下限/验收契约/verify 闸门）、租约活性自愈、多 agent（[指南](docs/loop-control-plane.zh-CN.md)）——基于 [loopx](https://github.com/huangruiteng/loopx) 的 Rust 改写版，针对 FutureOS 做了定制 |
| **Rust 核心** | Agent、IM 渠道桥、loop 控制面、CLI 与 TUI 均用 Rust 编写——高性能、内存安全 |

## 快速开始

### 安装

一行命令，无需源码构建：

**macOS / Linux** — 同一脚本自动识别平台：macOS 安装官方签名应用（自动识别 arm64 / Intel）；Debian/Ubuntu 安装 `.deb`（桌面应用 + 统一 `future` CLI），其他 Linux 安装便携版压缩包：

```bash
curl -fsSL https://dl.future-os.cn/install.sh | bash
```

**Windows**（PowerShell）— 静默运行签名安装程序：

```powershell
iex (irm https://dl.future-os.cn/install.ps1)
```

各平台（桌面应用、工具链、GUI 打包）的分步安装步骤见
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

终端与 CLI 客户端都是轻量 gRPC 客户端。**必须先启动 Agent**，监听 `127.0.0.1:50051`：

```bash
future agent      # 在终端启动 agent（日志打到 stdout，Ctrl-C 停止）
```

然后用同一个 `future` 命令启动终端界面：

```bash
future tui        # 终端界面
```

> `future <cmd>` 是所有 Rust 组件的统一入口：`future agent`、`future tui`、
> `future channel`、`future loop`。每个都运行与同名独立二进制完全相同的代码
> （`future-*` 二进制仍是构建目标，可用 `cargo build -p future-tui` 等构建，
> 但已不再默认安装）。
>
> 客户端如果报连接 / gRPC 错误，几乎都是 Agent 还没启动——见 [故障排查](#故障排查)。

### 常用斜杠命令（TUI）

| 命令 | 说明 |
|---|---|
| `/help` | 显示所有命令和快捷键 |
| `/model [name]` | 选择 / 切换模型 |
| `/new` | 新建会话 |
| `/sessions` | 浏览和切换会话 |
| `/compact` | 压缩对话上下文 |
| `/scoped-models` | 配置模型启用/禁用列表 |
| `/clone` | 克隆当前会话 |
| `/fork` | 分叉当前会话 |
| `/tree` | 会话树（含 fork/clone 层级） |
| `/name [n]` | 设置会话名称 |
| `/status` | 会话状态、token 用量、费用 |
| `/stop` | 中断当前生成 |
| `/cwd` | 切换工作目录 |
| `/approve` | 批准待执行的工具调用 |
| `/reject` | 拒绝待执行的工具调用 |
| `/cancel <run-id>` | 取消排队中的运行 |
| `/reload` | 重新加载技能与上下文 |

### 键盘快捷键（TUI）

| 按键 | 功能 |
|---|---|
| `ctrl+p` | 循环切换模型 |
| `ctrl+t` | 循环切换思考级别 |
| `ctrl+o` | 展开 / 收起思考内容 |
| `ctrl+r` | 浏览会话列表 |
| `ctrl+c` | 中断 / 退出 |
| `tab` | 自动补全 |
| `enter` | 提交 / 确认 |
| `escape` | 关闭弹窗 |
| `↑↓` | 滚动 / 导航列表 |

## 故障排查

| 现象 | 解决 |
|---|---|
| 客户端报连接 / gRPC 错误退出 | Agent 没启动。先启动它(`future agent`)，并确认端口没被占用：`lsof -i :50051`。 |
| Agent 回复鉴权 / "no model" 错误 | 还没配置模型。运行 `future auth login`，或在 `models.json` 里加一个 provider——见 [配置模型](#配置模型)。 |
| 构建 / 安装问题 | 见 [构建与安装](docs/build-and-install.zh-CN.md)（平台工具链、链接器、GUI 打包）。 |

## License

MIT
