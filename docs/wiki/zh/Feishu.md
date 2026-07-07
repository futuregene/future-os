# 飞书集成

将 FutureOS 接入飞书（或 Lark），即可在任何飞书聊天中与 Agent 对话。**Channel Bridge** 以本地服务方式运行，通过 gRPC 将飞书消息转发给你的 FutureOS Agent。

---

## 工作原理

```
飞书用户  ──→  飞书服务器  ──→  Channel Bridge (WebSocket)  ──→  Agent (gRPC)
              (open.feishu.cn)    (你的机器)                     (127.0.0.1:50051)
```

Bridge 与飞书维持一条长连接 WebSocket。当有人给你的机器人发消息，飞书通过 WebSocket 推送消息，Bridge 转发给 Agent，回复通过 CardKit 实时流式更新卡片（支持 Markdown 渲染）返回聊天。

---

## 前提条件

1. **飞书开发者账号**：[open.feishu.cn](https://open.feishu.cn)（Lark 用户使用 [open.larksuite.com](https://open.larksuite.com)）。
2. 已创建并启用机器人能力的**飞书应用**。
3. FutureOS Agent 已运行（`make run-agent` 或 `future agent start`）。

---

## 创建飞书应用

1. 进入[飞书开发者控制台](https://open.feishu.cn/app)。
2. 点击**创建企业自建应用**。
3. 填写名称（如"FutureOS Bot"）和描述。
4. 在**功能** → **机器人**中，启用机器人能力。
5. 在**凭证**页面记录 **App ID** 和 **App Secret**。

### 机器人权限

在**权限管理**中添加以下权限并申请发布：

| 权限 | 用途 |
|---|---|
| `im:message` | 读取和发送消息 |
| `im:message.p2p_msg:read` | 读取私聊消息 |
| `im:message.group_msg:read` | 读取群聊消息 |
| `im:message:send_as_bot` | 以机器人身份发送消息 |
| `im:resource` | 下载图片和文件 |
| `contact:user.base:read` | 解析用户名 |

### 事件订阅

在**事件订阅**中订阅以下事件：

| 事件 | 用途 |
|---|---|
| `im.message.receive_v1` | 接收新消息 |

**请求网址**填写任意 HTTPS 地址即可（Bridge 使用 WebSocket，不会实际回调此 URL，但飞书要求必填才能启用事件）。

### 将机器人添加到聊天

发布后（哪怕是仅组织内发布），即可在飞书中搜索机器人名称，添加到私聊或群聊中开始对话。

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
  "feishu": {
    "enabled": true,
    "app_id": "cli_xxxxxxxxxxxx",
    "app_secret": "你的-app-secret",
    "domain": "feishu"
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
| `feishu.enabled` | 设为 `true` 启动飞书 Bridge |
| `feishu.app_id` | 飞书开发者控制台中的 App ID |
| `feishu.app_secret` | 飞书开发者控制台中的 App Secret |
| `feishu.domain` | `"feishu"`（飞书）或 `"lark"`（Lark，使用 `open.larksuite.com`） |

### 权限策略配置

控制谁可以与机器人对话：

```json
{
  "feishu": {
    "dm_policy": "allowlist",
    "dm_allowlist": ["ou_xxxxxxxxxxxx", "*"],
    "group_policy": "disabled",
    "group_allowlist": ["oc_xxxxxxxxxxxx"],
    "require_mention": true
  }
}
```

| 字段 | 取值 | 说明 |
|---|---|---|
| `dm_policy` | `"open"` / `"allowlist"` / `"disabled"` | 私聊策略。`"allowlist"`（默认）仅允许名单内用户；`"open"` 允许所有人；`"disabled"` 禁止所有私聊 |
| `dm_allowlist` | 用户 open_id 列表，或 `["*"]` 表示全部 | 允许私聊的用户（当 dm_policy 为 `"allowlist"`） |
| `group_policy` | `"open"` / `"allowlist"` / `"disabled"` | 群聊策略。`"disabled"`（默认）禁止所有群聊，除非单独开启 |
| `group_allowlist` | 群 chat_id 列表，或 `["*"]` 表示全部 | 允许的群聊（当 group_policy 为 `"allowlist"`） |
| `require_mention` | `true` / `false` | 为 `true`（默认）时，仅响应 @机器人的消息 |

> **获取 open_id / chat_id：** 被拒绝时机器人会回复你的 ID，用这些 ID 来填充白名单。

### 行为配置

| 字段 | 默认值 | 说明 |
|---|---|---|
| `streaming` | `true` | 通过 CardKit 实时流式更新卡片 |
| `resolve_sender_names` | `true` | 将 open_id 解析为显示名称（更友好，但稍慢） |
| `max_image_mb` | `10` | 机器人下载图片的最大 MB 数 |
| `typing_indicator` | `false` | 处理中显示输入状态 |

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

在飞书聊天中向机器人发送以下命令：

| 命令 | 说明 |
|---|---|
| `/new` | 新建会话 |
| `/status` | 查看当前会话状态（模型、token、费用） |
| `/model <provider/model>` | 切换模型（如 `deepseek-v4-flash` 或 `openai/gpt-4o`） |
| `/models` | 列出可用模型 |
| `/effort <level>` | 设置思考级别：`off`、`minimal`、`low`、`medium`、`high`、`xhigh` |
| `/stop` | 中断当前生成 |
| `/compact` | 压缩对话上下文 |
| `/cwd <path>` | 设置工作目录 |
| `/help` | 显示可用命令 |

`/new`、`/status`、`/model`、`/models`、`/effort`、`/compact`、`/cwd`、`/help` 等命令由 Bridge 本地处理，不经过 Agent。无法识别的命令会作为普通消息转发给 Agent。

---

## 回复效果

- **流式模式**（默认）：Bridge 创建 CardKit 卡片并实时更新，思考内容以可折叠引用块展示。
- **非流式模式**：生成完成后发送单条完整 Markdown 消息。

---

## 常见问题

### 机器人不回复

1. 检查 Bridge 是否运行：`future channel status`
2. 检查 `config.json` 中 `feishu.enabled` 是否为 `true`
3. 检查 DM/群聊策略——机器人可能在拒绝访问，拒绝消息中会包含你的 open_id 或 chat_id。
4. 查看 Bridge 日志中的 WebSocket 连接错误。

### Bridge 每 6 分钟左右重连

这是飞书 WebSocket 的正常行为。Bridge 每 30 秒发送 keepalive ping，超时后自动重连。重连对用户透明——会话不受影响。

### 图片无法处理

确认已授权 `im:resource` 权限，并检查图片大小是否超过 `max_image_mb` 限制（默认 10 MB）。

---

## 参见

- [[钉钉集成|DingTalk]] —— 将 FutureOS 接入钉钉。
- [[设置|Settings]] —— 配置 FutureOS 设置和 Provider。
- [[命令行工具(future-cli)|CLI]] —— 服务管理的命令行工具。
