<p align="center">
  <img src="docs/banner.png" alt="FutureOS" width="600">
</p>

<h3 align="center">同一个 AI Agent，处处随你。</h3>
<p align="center">
  终端、桌面、手机、飞书与钉钉——一个 Rust 核心，一个 Agent，3800+ 模型。<br>
  每一次工具调用都由你审批。本地优先。开源。
</p>

<p align="center">
  <img src="docs/desktop-screenshot.png" alt="FutureOS 桌面应用——新对话、模型、技能与工作区，一窗集成" width="800">
</p>

<!-- TODO(demo): 录制 60–90 秒演示 GIF（TUI 审批门控 → 桌面 GUI → IM 机器人进度 →
     会话 fork 树 → /future-loop 看板），保存为 docs/demo.gif 后替换上面的静态截图 -->

<p align="center">
  <a href="#快速开始">快速开始</a> •
  <a href="#特性">特性</a> •
  <a href="#配置模型">3800+ 模型</a> •
  <a href="#常用斜杠命令tui">命令</a> •
  <a href="#故障排查">故障排查</a>
</p>

<p align="center">⭐ 如果 FutureOS 对你有用，点个 Star 帮助更多人发现它。</p>

<p align="center">
  <img src="https://img.shields.io/badge/Core-Rust-orange?style=for-the-badge&logo=rust" alt="Rust 核心">
  <a href="https://github.com/futuregene/future-os/blob/main/THIRD_PARTY_NOTICES.md"><img src="https://img.shields.io/badge/License-MIT_%2B_Apache--2.0-green?style=for-the-badge" alt="License: MIT + Apache-2.0"></a>
  <a href="README.md"><img src="https://img.shields.io/badge/Lang-English-blue?style=for-the-badge" alt="English"></a>
</p>

---

## 为什么是 FutureOS

- **信任先于能力。** 每一次工具调用——读、写、编辑、shell——默认都需要你批准，没有任何文件写入和命令执行是静默发生的。当 Agent 手握你的凭证，信任不能只是一个配置项。
- **一个后端，所有界面。** 同一个 gRPC Agent 驱动终端界面、桌面应用、移动端、CLI 和 IM 机器人——同一份会话、同一份记忆、同一套技能，无论你身在何处。
- **长任务靠工程，不靠提示词。** 内置 loop 控制面为 24 小时以上的任务提供持久化目标、事件溯源状态与验证门控——晚上布置一个调研任务，早上在手机上验收结果。

## 特性

| 类别 | 说明 |
|---|---|
| **多端统一** | 终端界面 (TUI)、桌面应用 (GUI)、移动端 App（Android · iOS）、命令行 (CLI)、IM 机器人——一个 Agent，无处不在 |
| **信任优先的工具执行** | read, write, edit, shell——每次调用都需你审批；沙箱分级（`off` / `manual` / `sandbox`），macOS（Seatbelt）与 Windows（受限令牌）提供 OS 级沙箱；工具集精简，杜绝 prompt 膨胀 |
| **模型灵活** | 内置 3800+ 模型，覆盖 140+ Provider（[目录](docs/wiki/zh/Models.md)）；通过 `models.json` 自定义 Provider；支持模型范围限定 |
| **Loop 工程** | 持久化目标/todos/门禁/监控，支撑 24+ 小时长程任务连续执行——确定性 should-run 内核、事件溯源状态、硬校验（证据下限/验收契约/verify 闸门）、租约活性自愈、多 agent（[指南](docs/loop-control-plane.zh-CN.md)） |
| **强大的预设技能** | 内置 15+ 技能开箱即用，覆盖日常 Agent 场景——图片读取与生成、PDF/Word 解析、网页搜索、浏览器控制、幻灯片与软件安装，以及 `/future-loop` 长程目标编排器（[builtin](https://github.com/futuregene/future-skills/tree/main/builtin)） |
| **可分支会话** | 像仓库一样为对话开分支——fork、clone、树形导航，JSONL 存储 |
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

安装脚本最后会自动执行 `future init`，然后进入交互式 `future config` 模型提供商配置。

### 配置模型

Agent 至少需要配置一个模型提供商才能回复。交互式配置同时支持 FutureOS 和自定义 Provider：

```bash
future config
```

**FutureOS 托管模型** —— 设备码登录会自动配好 key 和模型列表：

```bash
future auth login
```

无论 agent 是否在运行都可以执行：agent 在线时 key 立即生效；未运行时 key 会写入 `~/.future/agent/auth.json`，agent 启动时自动读取。

<details>
<summary><strong>想用自己的 API key？（BYOK 与自定义 Provider）</strong></summary>

**使用已知 Provider。** 将 API Key 放入 `~/.future/agent/auth.json`，按 Provider 名索引。查看[内置模型目录](docs/wiki/zh/Models.md)——多数 Provider 自带 Base URL，模型自动发现：

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

**自定义 Provider。** 不在内置目录中的 Provider，在 `~/.future/agent/models.json` 中指定完整信息：

```json
{
  "providers": {
    "my-provider": {
      "apiKey": "sk-...",
      "baseUrl": "https://my-api.example.com/v1",
      "models": [
        { "id": "my-model", "name": "My Model", "contextWindow": 128000, "maxTokens": 16384 }
      ]
    }
  }
}
```

</details>

### 安装技能（可选）

技能是按需安装的能力包，agent 会从 `~/.future/agent/skills/` 加载。浏览技能目录并安装需要的技能——只需要网络连接，不要求 agent 在运行：

```bash
future skills list             # 浏览技能目录
future skills install <name>   # 安装指定技能
future skills install          # 不带名字：安装全部内置技能
```

跳过这一步 agent 也能正常回答——技能只是提供现成的工作流。卸载、升级等用法见 [CLI 参考](docs/wiki/zh/CLI.md)。

### 启动 Agent

终端与 CLI 客户端都是轻量 gRPC 客户端。**必须先启动 Agent**，监听 `127.0.0.1:50051`：

```bash
future agent      # 在终端启动 agent（日志打到 stdout，Ctrl-C 停止）
```

然后用同一个 `future` 命令启动终端界面：

```bash
future tui        # 终端界面
```

<p align="center">
  <img src="docs/tui-screenshot.png" alt="FutureOS 终端界面——内置技能加载，/help 命令面板" width="720">
</p>

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
| Agent 回复鉴权 / "no model" 错误 | 还没配置模型。运行 `future config`——见 [配置模型](#配置模型)。 |
| 构建 / 安装问题 | 见 [构建与安装](docs/build-and-install.zh-CN.md)（平台工具链、链接器、GUI 打包）。 |

## 社区

[💬 Discussions](https://github.com/futuregene/future-os/discussions) • [🐛 Issues](https://github.com/futuregene/future-os/issues) • [🔒 安全](SECURITY.md) • [📖 Wiki](https://github.com/futuregene/future-os/wiki) • [第三方声明](THIRD_PARTY_NOTICES.md)
