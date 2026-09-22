# 通道 provider 参考

本页说明运行在共享桥上的通道：如何配置、各自支持什么、以及通道不工作时的排查方式。
文件位置与 `agent` 配置块见[通道配置](channels-config.zh-CN.md)。

本仓库里被称为「通道」的东西有两类：

- **框架通道**：实现 `Provider` 契约（`channels/src/providers/`），从共享桥获得去重、访问策略、
  会话路由、每会话排队、流式回复与审批回信路由。除最后一节外，本页讲的都是这一类。
- **自带桥的通道**：飞书与钉钉。它们保留各自的桥，承载框架尚未建模的平台行为（交互卡片、
  流式卡片元素、审批按钮、斜杠命令）。它们使用既有的顶层配置块，见
  [通道配置](channels-config.zh-CN.md)。

## 配置

框架通道写在 `providers` 下，每个通道一个块：

```jsonc
{
  "agent": { /* 所有通道共用，见 channels-config.zh-CN.md */ },
  "providers": {
    "telegram": {
      "enabled": true,
      "bot_token": "123456:ABC...",
      "dm_policy": "allowlist",
      "dm_allowlist": ["123456789"],
      "group_policy": "disabled",
      "require_mention": true
    },
    "slack": { "enabled": false }
  }
}
```

每个块只由一个通道读取；桥需要的部分也来自同一个块：

| 键 | 读取方 | 含义 |
|---|---|---|
| `enabled` | 桥 | 是否启动该通道。缺省或 `false` 表示永不启动，状态显示为 `disabled`。 |
| `dm_policy` | 桥 | `open`、`disabled` 或 `allowlist`（默认 `allowlist`）。 |
| `dm_allowlist` | 桥 | 允许私聊机器人的发送者 id；`["*"]` 表示不限制。 |
| `group_policy` | 桥 | 群/频道策略：`open`、`disabled`（默认）或 `allowlist`。 |
| `group_allowlist` | 桥 | 允许触达机器人的会话 id；`["*"]` 表示全部允许。 |
| `require_mention` | 桥 | 在允许的群里是否必须被明确 @ 才回复（默认 `true`）。 |
| 其余字段 | 该通道 | 平台凭据与选项，见下文各通道表格。 |

已启用但本构建未实现的通道会显示为 `unsupported` 并在启动日志中明确写出一次，
不会静默什么都不做。

对飞书与钉钉而言，`providers.<id>` 优先于既有顶层块，因此可以逐个迁移而不必一刀切。

## Provider 能力矩阵

`future channel list` 会打印你当前构建的这张矩阵，含每个通道的成熟度。能力列是桥可以依赖的
行为：`edit` 决定可否渐进式流式回复，`threads` 决定每个线程是否独立会话，`typing`/`reactions`
是确认信号，`media` 表示能接收附件（图片会转成模型输入）。

| 通道 | id | 接入方式 | 编辑 | 线程 | typing | reactions | 收媒体 | 单条上限 | 外部依赖 |
|---|---|---|---|---|---|---|---|---|---|
| Telegram | `telegram` | 长轮询 / webhook | 是 | 否 | 是 | 是 | 是 | 4096 字符 | bot token |
| Slack | `slack` | Socket Mode / Events | 是 | 是 | 否 | 是 | 是 | 4000 UTF-16 单元 | Slack 应用 |
| Discord | `discord` | 网关 WebSocket | 是 | 是 | 是 | 是 | 是 | 2000 字符 | 带消息内容 intent 的 bot token |
| Mattermost | `mattermost` | REST + WebSocket | 是 | 是 | 否 | 是 | 否 | 4000 字符 | 服务端 + token |
| Signal | `signal` | 本地守护进程 | 否 | 否 | 是 | 是 | 是 | 4000 字符 | `signal-cli` |
| WhatsApp | `whatsapp` | Cloud API webhook | 否 | 否 | 否 | 是 | 是 | 4096 字符 | Meta 应用 + 公网 webhook |
| QQ | `qq` | 网关 WebSocket | 否 | 否 | 否 | 否 | 否 | 4000 字符 | 开放平台机器人 |
| Linq | `linq` | 签名 webhook | 否 | 否 | 否 | 否 | 否 | 4000 字符 | Linq 账号 |
| iMessage | `imessage` | 仅 macOS | 否 | 否 | 否 | 否 | 否 | 20000 字符 | macOS + 完全磁盘访问权限 |
| IRC | `irc` | TCP 或 TLS | 否 | 否 | 否 | 否 | 否 | 400 字节 | — |
| Email | `email` | IMAP + SMTP | 否 | 否 | 否 | 否 | 否 | 100000 字节 | 支持应用密码的邮箱 |
| Terminal | `cli` | 标准输入输出 | 否 | 否 | 是 | 否 | 否 | 100000 字符 | — |

### 成熟度

每个通道声明三档之一，`future channel list` 会显示：

- **live**：已在真实平台上跑通，可以依赖。
- **preview**：按平台公开 API 文档实现，但尚未在真实部署上验证。请自行验证安装，
  并反馈问题。
- **planned**：已声明但未实现，启用时启动日志报 `unsupported`。

preview 不等于半成品：它周围的链路（策略、去重、会话、流式、审批）是共享且有测试的；
未经验证的是平台方言——确切的 JSON 字段、签名头、限流算法。

## 各通道配置

下列示例都是能启动该通道的最小块；所有键都有默认值，可以省略。

### Telegram

```jsonc
{
  "enabled": true,
  "bot_token": "",
  "mode": "long_poll",                 // long_poll | webhook
  "webhook": { "addr": "127.0.0.1:8787", "path": "/telegram", "secret_token": "" },
  "dm_policy": "allowlist", "dm_allowlist": [],
  "group_policy": "disabled", "group_allowlist": [], "require_mention": true,
  "streaming": true
}
```

长轮询只需出网 HTTPS，并会持久化 offset，重启不会丢失已排队的更新。webhook 模式需要一个
指向 `webhook.addr` 的公网地址（请放在反代之后以启用 TLS）以及 `secret_token`，
通道会校验 Telegram 回显在请求头里的值。

### Slack

```jsonc
{
  "enabled": true,
  "bot_token": "", "app_token": "", "signing_secret": "",
  "dm_policy": "allowlist", "dm_allowlist": [],
  "group_policy": "disabled", "group_allowlist": [], "require_mention": true,
  "streaming": true
}
```

推荐用 `app_token`（Socket Mode）接入，因为它不需要公网地址；只有用 HTTP 投递 Events 时才
需要 `signing_secret`。Slack 按 UTF-16 单元计算长度，因此 emoji 较多的长回复会比其他平台更早
分片——桥使用平台计数的单位而不是 `chars()`，以保证切分合法。

### Discord

```jsonc
{
  "enabled": true,
  "bot_token": "",
  "guild_allowlist": [],
  "dm_policy": "allowlist", "dm_allowlist": [],
  "group_policy": "disabled", "group_allowlist": [], "require_mention": true,
  "streaming": true
}
```

机器人必须开启 **message content** intent，否则消息正文会是空的。心跳间隔取自网关的
`HELLO`；`RESUME` 失败时会重新 IDENTIFY 并补齐缺口，而不是丢消息。

### Mattermost

```jsonc
{
  "enabled": true,
  "base_url": "https://mattermost.example.com",
  "token": "",
  "channel_allowlist": [],
  "require_mention": true,
  "streaming": true
}
```

### Signal

```jsonc
{
  "enabled": true,
  "http_url": "http://127.0.0.1:8080",
  "number": "+15550001234",
  "dm_policy": "allowlist", "dm_allowlist": [],
  "group_policy": "disabled", "group_allowlist": [], "require_mention": true
}
```

Signal 没有机器人 API。这里的对接目标是由**你自己**运行的 `signal-cli` 守护进程，
通道是它的 HTTP 客户端，因此必须先完成注册并确保可达。

### WhatsApp

```jsonc
{
  "enabled": true,
  "phone_number_id": "", "access_token": "",
  "verify_token": "", "app_secret": "",
  "webhook": { "addr": "127.0.0.1:8788", "path": "/webhooks/whatsapp" },
  "sender_allowlist": []
}
```

入站需要公网可达的 HTTPS 端点。`verify_token` 用于回应 Meta 的订阅校验，
`app_secret` 用于校验每次投递的 `X-Hub-Signature-256` 头；未签名的请求体会被拒绝。
出站只能在 24 小时客服窗口内发送（否则需模板），因此发送失败有可能是永久性的。

### QQ

```jsonc
{
  "enabled": true,
  "app_id": "", "app_secret": "",
  "sandbox": false,
  "group_allowlist": [],
  "require_mention": true
}
```

### Linq

```jsonc
{
  "enabled": true,
  "api_key": "", "from": "",
  "webhook": { "addr": "127.0.0.1:8789", "path": "/webhooks/linq" },
  "sender_allowlist": []
}
```

### iMessage（macOS）

```jsonc
{
  "enabled": true,
  "recipients": [],
  "sender_allowlist": [],
  "poll_seconds": 5,
  "db_path": ""
}
```

发送通过 `osascript` 驱动 Messages.app；接收只读读取本地 Messages 数据库。
运行桥的进程需要「完全磁盘访问权限」，且该通道只在 macOS 存在——其他平台会报
`unsupported`。

### IRC

```jsonc
{
  "enabled": true,
  "server": "irc.libera.chat", "port": 6697, "tls": true,
  "nick": "",
  "sasl": { "account": "", "password": "" },
  "channels": ["#future"],
  "require_mention": true
}
```

长度上限以**包含协议开销的字节数**计，所以这里声明的上限是 400 而不是 512。

### Email

```jsonc
{
  "enabled": true,
  "imap": { "host": "", "port": 993, "username": "", "password": "", "mailbox": "INBOX" },
  "smtp": { "host": "", "port": 587, "username": "", "password": "", "from": "" },
  "poll_seconds": 30,
  "sender_allowlist": [],
  "subject_prefix": ""
}
```

使用密码或应用专用密码，不支持 OAuth。附件会被识别但不会下载；桥也绝不回复邮箱自身的地址。

### Terminal

```jsonc
{ "enabled": true }
```

从标准输入读取提示、把回答写到标准输出。适合在不注册平台账号的情况下跑通整条链路，
也是新增通道时的参考实现。

## 诊断

```bash
future channel list              # 全部通道、成熟度、能力，以及是否已配置
future channel status            # 逐通道状态、运行时长、计数器与最近错误
future channel test <id>         # 凭据与连通性自检（会发出一次真实请求）
future channel send --channel <id> --to <会话> --text "..." [--durable]
```

`status` 读取的是桥在工作中不断重写的快照，因此在桥**没有**运行时它同样能回答：
缺少快照或 `updated_unix` 过旧，说明进程不在，而不是「什么都没发生」。逐通道计数器包含
接收、发送、去重抑制、因背压丢弃，以及被新消息抢占的回合数。

`send` 是不属于任何会话的出站路径：定时任务、loop 门控通知，或 agent 自己的「完成后通知我」。
加 `--durable` 会先写入投递队列，因此限流或重启都不会丢；不加则立即上报失败。

### 运行时写入的文件

| 路径 | 内容 |
|---|---|
| `~/.future/channels/config.json` | 配置（见上）。 |
| `~/.future/channels/status.json` | 最新状态快照，原子重写。 |
| `~/.future/channels/deliveries.json` | 持久出站队列（待发 / 已发 / 失败）。 |
| `~/.future/channels/<通道>/sessions.json` | 会话 → agent session 映射。 |
| `~/.future/channels/<通道>/inbox/` | 下载的入站附件。 |

## 所有通道共享的行为

- **新鲜度与去重。** 同一条消息只回一次。平台时间戳超过新鲜度窗口的消息按「长时间断线后的
  回放」处理并丢弃，因此重连不会倾泻一堆过期回答；不提供时间戳的平台不做这种过滤。
- **一个会话一条回合。** 每个会话（线程算独立会话）有定长邮箱并顺序执行回合，不同会话之间
  并发。洪泛时会拒绝超出部分并记录告警，而不是无上限增长内存。
- **新消息优先。** 在 agent 仍在作答时发纠正，会让旧回合在下一个事件处停止并回答最新消息，
  而不是对同一个问题贴出两个答案。
- **先过访问策略。** DM/群策略与提及门通过之前，任何消息都不会到达 agent。被拒的私聊会收到
  载明发送者 id 的说明，便于管理员加入白名单；群里未被 @ 的拒答则保持安静。
- **审批在聊天里完成。** agent 挂起受门控的动作时，通道会说明要做什么以及如何答复。单独的
  `yes`/`no`（也含 确认/拒绝）会作为决定送达，且不会被当成新提示；更长的文本按普通消息处理，
  因此真正的提问不会被吞掉。
- **流式适配平台。** 能编辑消息的通道得到渐进式回答，其余在结束时一次性发送。无论如何，
  失败的回合都会说明原因，被取消的回合保持安静。
- **工具活动可见。** 只跑了工具的回合也会给出简短的说明，因此「安静」绝不等于「什么都没发生」。

## 参见

- [通道配置](channels-config.zh-CN.md) — 文件位置、`agent` 块、飞书与钉钉。
- [目录布局](directory-layout.zh-CN.md) — 通道文件的位置。
- `channels/src/providers/INTERFACE.md` — 新增通道的契约。
- `channels/src/bridge/` — 共享链路。
