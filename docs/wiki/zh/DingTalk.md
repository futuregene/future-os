# 钉钉集成

将 FutureOS 接入钉钉，即可在任何钉钉聊天中与 Agent 对话。**Channel Bridge** 以本地服务方式运行，通过 gRPC 将钉钉消息转发给你的 FutureOS Agent。

---

## 工作原理

```
钉钉用户  ──→  钉钉服务器  ──→  Channel Bridge (Stream Mode)  ──→  Agent (gRPC)
              (api.dingtalk.com)  (你的机器)                        (127.0.0.1:50051)
```

Bridge 使用**钉钉 Stream Mode**——无需公网回调 URL。它通过 WebSocket 连接钉钉，实时接收消息，转发给 Agent，再通过每条事件中附带 `sessionWebhook` 地址回复。

---

## 前提条件

1. **钉钉开发者账号**：[open.dingtalk.com](https://open.dingtalk.com)。
2. 已创建并启用 Stream Mode 的**钉钉应用**。
3. FutureOS Agent 已运行（`make run-agent` 或 `future agent start`）。

---

## 创建钉钉应用

1. 进入[钉钉开发者控制台](https://open-dev.dingtalk.com)。
2. 创建新应用。
3. 在**机器人** → **消息接收模式**中，选择 **Stream Mode**。
4. 在凭证页面记录 **Client ID**（AppKey）和 **Client Secret**（AppSecret）。

### 机器人权限

启用机器人能力后即自动获得以下权限：

| 权限 | 用途 |
|---|---|
| `im.message.receive` | 通过 Stream Mode 接收消息 |
| `im.message.send` | 向聊天发送回复 |
| `qyapi_robot_webhook_message_send` | 通过 Webhook 发送消息 |

### 将机器人添加到聊天

部署后即可在钉钉中搜索机器人名称，添加到私聊或群聊中开始对话。

---

## 配置文件

编辑 `~/.future/channels/config.json`：

```json
{
  "agent": {
    "grpc_addr": "http://127.0.0.1:50051",
    "cwd": "/home/yourname",
    "model": "future/deepseek-v4-pro",
    "thinking_level": "xhigh",
    "permission_level": "all"
  },
  "dingtalk": {
    "enabled": true,
    "client_id": "dingxxxxxxxxxxxx",
    "client_secret": "你的-client-secret",
    "domain": "api.dingtalk.com"
  }
}
```

| 字段 | 说明 |
|---|---|
| `agent.grpc_addr` | Agent gRPC 地址（默认 `http://127.0.0.1:50051`） |
| `agent.cwd` | 新会话的默认工作目录 |
| `agent.model` | Channel 会话默认模型（如 `future/deepseek-v4-pro`） |
| `agent.thinking_level` | 默认思考级别：`off`、`minimal`、`low`、`medium`、`high`、`xhigh` |
| `agent.permission_level` | 默认权限级别：`all`、`workspace`、`none` |
| `dingtalk.enabled` | 设为 `true` 启动钉钉 Bridge |
| `dingtalk.client_id` | 钉钉开发者控制台中的 Client ID（AppKey） |
| `dingtalk.client_secret` | 钉钉开发者控制台中的 Client Secret（AppSecret） |
| `dingtalk.domain` | API 域名（默认 `api.dingtalk.com`） |

---

## 启动 Bridge

```bash
# 构建并运行 Channel Bridge
make build-channels-release
./target/release/future-channels
```

或作为系统服务管理：

```bash
future channel start    # macOS launchctl / Linux systemd
future channel status   # 查看状态
future channel stop     # 停止
future channel restart  # 重启
```

Bridge 启动时加载 `~/.future/channels/config.json`。如果文件不存在，会自动创建模板并退出——编辑模板后重新启动即可。

---

## 斜杠命令

在钉钉聊天中向机器人发送以下命令：

| 命令 | 说明 |
|---|---|
| `/new` | 新建会话 |
| `/status` | 查看当前会话状态（模型、token、费用） |
| `/stop` | 中断当前生成 |
| `/model <provider/model>` | 切换模型（如 `deepseek-v4-flash` 或 `openai/gpt-4o`） |
| `/models` | 列出可用模型 |
| `/effort <level>` | 设置思考级别：`off`、`minimal`、`low`、`medium`、`high`、`xhigh` |
| `/compact` | 压缩对话上下文 |
| `/cwd <path>` | 设置工作目录 |
| `/help` | 显示可用命令 |

所有斜杠命令均由 Bridge 本地处理，不经过 Agent。无法识别的命令会作为普通消息转发给 Agent。

---

## 回复效果

- 回复通过 session webhook 以 **Markdown** 格式发送。
- 每条回复都是**一条新消息**（钉钉 Stream Mode webhook 不支持原地编辑）。
- 思考内容以引用块（`> 💭`）展示，与正文用分隔线隔开。
- 斜杠命令的回复为简短状态消息。

---

## 与飞书的区别

| 特性 | 飞书 | 钉钉 |
|---|---|---|
| 连接方式 | WebSocket（pbbp2 protobuf 帧） | WebSocket（Stream Mode JSON） |
| 流式输出 | CardKit 实时卡片更新 | 每条回复是新消息（不支持原地流式） |
| 思考链 | CardKit 可折叠引用块 | 带 `> 💭` 前缀的引用块 |
| Emoji 表情反馈 | ✅（ACK/DONE 处理状态指示） | ❌（API 未公开） |
| 多模态 | 支持图片和文件 | 仅文本和 Markdown |

---

## 常见问题

### 机器人不回复

1. 检查 Bridge 是否运行：`future channel status`
2. 检查 `config.json` 中 `dingtalk.enabled` 是否为 `true`
3. 确认 `client_id` 和 `client_secret` 正确无误。
4. 查看 Bridge 日志中的 Stream Mode 连接错误。

### Bridge 频繁重连

钉钉 Stream Mode WebSocket 每 20 秒发送 keepalive ping。重连自动且透明——会话不受影响。

### 钉钉中 Markdown 格式显示异常

钉钉 Markdown 需要双换行（`\n\n`）才能产生段落分隔，单个 `\n` 不显示可见换行。Bridge 已自动处理此问题，但如果仍有异常，可能是钉钉渲染限制所致。

---

## 参见

- [[飞书集成|Feishu]] —— 将 FutureOS 接入飞书/Lark。
- [[设置|Settings]] —— 配置 FutureOS 设置和 Provider。
- [[命令行工具(future-cli)|CLI]] —— 服务管理的命令行工具。
