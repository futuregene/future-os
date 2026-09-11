# 渠道配置（`~/.future/channels/config.json`）

渠道桥（`future-channel`，见 `channels/`）只读取一个配置文件：
`~/.future/channels/config.json`。它通过 gRPC 连接 agent，并把 agent 暴露
为飞书与钉钉机器人。用 `future channel` 启动（统一 CLI 入口，与独立二进制
`future-channel` 完全一致）。

**首次运行：** 若文件不存在，桥会写入默认模板并**退出**——编辑该文件后
重新启动。

桥只启动 `enabled: true` 的渠道。所有字段都是可选的；每个字段都有默认值，
因此 `{}` 也是合法配置（agent 块用默认值，两个渠道均禁用）。

## Schema

```jsonc
{
  // agent 块——渠道会话的模型/会话默认值。
  "agent": {
    "grpc_addr": "auto",                   // 每用户本地 IPC
    "cwd": "/home/you",                     // agent 的工作目录
    "model": "future/deepseek-v4-pro",      // 渠道会话的默认模型
    "thinking_level": "xhigh",              // off | minimal | low | medium | high | xhigh
    "permission_level": "all"               // all | workspace | none
  },

  // 飞书块——仅在 "enabled": true 时启用。
  "feishu": {
    "enabled": false,
    "app_id": "",
    "app_secret": "",
    "domain": "feishu",                     // API 域名；默认 "feishu"
    "dm_policy": "allowlist",               // open | disabled | allowlist
    "dm_allowlist": [],                     // open_id 列表，或 ["*"] 表示所有人
    "group_policy": "disabled",             // open | disabled | allowlist
    "group_allowlist": [],                  // chat_id 列表，或 ["*"] 表示所有群
    "require_mention": true,                // 群里需要 @机器人 才回复
    "streaming": true,                      // 流式回复（CardKit 卡片）
    "resolve_sender_names": true,
    "max_image_mb": 10,
    "typing_indicator": false
  },

  // 钉钉块——仅在 "enabled": true 时启用。
  "dingtalk": {
    "enabled": false,
    "client_id": "",
    "client_secret": "",
    "domain": "api.dingtalk.com",
    "sender_allowlist": []                // 发送者 ID；空列表拒绝所有人，["*"] 显式信任所有人
  }
}
```

## 字段参考

### `agent`

| 字段 | 默认值 | 含义 |
|---|---|---|
| `grpc_addr` | `auto` | 每用户本地 IPC。显式 `http://host:port` 先尝试 TCP 再回退 IPC；Agent 需通过 `--grpc-addr` 开启 TCP。 |
| `cwd` | `$HOME` | 渠道会话中 agent 运行的工作目录。 |
| `model` | `future/deepseek-v4-pro` | 渠道会话的默认模型。为空表示「使用 agent 启动时的默认值」。 |
| `thinking_level` | `xhigh` | 默认思考级别：`off` / `minimal` / `low` / `medium` / `high` / `xhigh`。 |
| `permission_level` | `all` | `all` 不受限；`workspace` 启用审批门控；`none` 禁止所有工具。它不是 OS 沙箱选择器。 |

### `feishu`

| 字段 | 默认值 | 含义 |
|---|---|---|
| `enabled` | `false` | 启动飞书桥。 |
| `app_id` / `app_secret` | 空 | 飞书应用凭据。 |
| `domain` | `feishu` | API 域名。 |
| `dm_policy` | `allowlist` | 私聊访问：`open`（所有人）、`disabled`（禁止）、`allowlist`（仅 `dm_allowlist`）。 |
| `dm_allowlist` | `[]` | 允许的 open_id；`["*"]` 表示所有人。 |
| `group_policy` | `disabled` | 群聊访问：`open` / `disabled` / `allowlist`。 |
| `group_allowlist` | `[]` | 允许的 chat_id；`["*"]` 表示所有群。 |
| `require_mention` | `true` | 群里仅在 @机器人 时回复。 |
| `streaming` | `true` | 流式回复（CardKit 卡片流式）。 |
| `resolve_sender_names` | `true` | 为诊断日志解析已获授权发送者的显示名。 |
| `max_image_mb` | `10` | 下载附件（图片和文件）的大小上限（MiB）；读取响应时逐块检查，无 Content-Length 时同样有效。 |
| `typing_indicator` | `false` | 显示输入中指示。 |

> 运行时可以对单个群做覆盖（例如禁用某个特定群）；上面的配置文件只设置
> 默认值。

### `dingtalk`

| 字段 | 默认值 | 含义 |
|---|---|---|
| `enabled` | `false` | 启动钉钉桥。 |
| `client_id` / `client_secret` | 空 | 钉钉应用凭据。 |
| `domain` | `api.dingtalk.com` | API 域名。 |
| `sender_allowlist` | `[]` | 私聊和群聊中获准操作 agent 的发送者 ID。空列表拒绝所有消息及斜杠命令；`["*"]` 显式信任所有能联系机器人的人。 |

**升级提示：** 已配置的钉钉桥需要填写 `sender_allowlist` 才会接收 prompt。仅加入群聊并不获得操作 agent 的授权。

## 运行时行为

- 飞书模型、思考级别和权限默认值只初始化新会话；用户通过 `/model` 或 `/effort` 设置的选项会保留到后续消息。
- 审批按钮绑定原始聊天与会话（含线程）。渠道桥重启后，无法恢复绑定的旧卡片会明确报告未送达，而不是向当前选中的其他会话发送审批。
- 每个会话的准备阶段按消息到达顺序执行；延迟执行的旧任务不会覆盖新消息。流式回复期间不占用入站队列。每个会话最多缓存 128 个入站事件；超限会拒绝新事件并记录渠道警告，而不是无限增加任务与内存。传输 ACK 不代表过载事件已被执行，请在积压消退后重试。

- 新渠道会话不会主动开启桌面沙箱策略。应限制可操作机器人的用户；默认 `all` 不是默认审批。
- **斜杠命令：** 两个桥在桥接代码中分发 9 个命令（部分调用 Agent RPC，并非模型 prompt）
  （`/new /status /stop /model /models /compact /effort /cwd /help`）；
  无法识别的斜杠命令作为普通消息转发给 agent。
- **钉钉回复** 通过事件里的 `sessionWebhook` 发送——每次回复都是**新**消息
  （不支持就地编辑）。
- 两个桥都会自动重连；飞书 keepalive ping 30s，钉钉 20s。

## 参见

- [目录布局](directory-layout.zh-CN.md) —— 本文件所在位置。
- wiki [飞书](wiki/zh/Feishu.md) / [钉钉](wiki/zh/DingTalk.md) 页面——
  各平台的分步配置与使用指南。
- 源码：`channels/src/config.rs`（schema 与默认值）、
  `channels/src/feishu/policy.rs`（私聊/群聊访问策略）。
