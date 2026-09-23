# 通道 provider 参考

本页说明运行在共享桥上的通道：如何配置、各自支持什么、以及通道不工作时的排查方式。
文件位置与 `agent` 配置块见[通道配置](channels-config.zh-CN.md)。

本仓库里被称为「通道」的东西有两类：

- **框架通道**：实现 `Provider` 契约（`channels/src/providers/`），从共享桥获得去重、访问策略、
  会话路由、每会话排队、流式回复与审批回信路由。除最后一节外，本页讲的都是这一类。
- **自带桥的通道**：飞书与钉钉。它们保留各自的桥，承载框架尚未建模的平台行为（交互卡片、
  流式卡片元素、审批按钮、斜杠命令）。它们使用既有的顶层配置块，见
  [通道配置](channels-config.zh-CN.md)。

`future channel list` 覆盖两类通道并标明归属：`BRIDGE` 列为 `shared` 的是框架通道，
`own` 的是自带桥的通道。

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
| 其余字段 | 该通道 | 平台凭据与选项，见下文各通道配置块。 |

默认值是刻意收紧的：私聊策略是白名单且初始为空，因此只填了凭据的通道**谁都不回复**，
直到 `dm_allowlist`（或 `dm_policy: "open"`）明确放开。

已启用但本构建未实现的通道会显示为 `unsupported` 并在启动日志中明确写出一次，
不会静默什么都不做。

对飞书与钉钉而言，`providers.<id>` 优先于既有顶层块，因此可以逐个迁移而不必一刀切。

`future channel test <id>` 与 `future channel send --channel <id>` 直接从配置块构建单个通道，
因此该块必须存在（`enabled` 只决定桥是否**启动**该通道）；桥本身不需要在运行。

## Provider 能力矩阵

`future channel list` 会打印你当前构建的这张矩阵，含每个通道的成熟度与声明式外部依赖。
能力列是桥可以依赖的行为：

- **编辑**：桥可以改写同一条消息来做渐进式流式回复；不具备该能力的通道在结束时一次性发送。
- **线程**：每个线程是独立的 agent 会话，也是独立的会话。
- **typing**：通道有进度/输入中信号。回合开始与模型思考时桥都会请求该信号；
  标注为「否」的通道要么没有这种信号，要么把请求当成空操作。
- **reactions**：通道能添加确认表情。Slack 与 Mattermost 会给已接收的消息加 👀；
  其余通道实现了该方法，但目前没有调用方。
- **收媒体**：入站附件会被读取（图片转成模型输入）。目前还没有任何通道发送附件，
  两个方向都没有（`ChannelSender` 没有出站媒体方法）。
- **提及门**：该 provider 能否判定群消息是否指向机器人；门本身由共享策略引擎的
  `require_mention` 决定。纯私聊通道——WhatsApp、iMessage、Email、Terminal——为「否」，
  因为它们收到的消息在语义上都已经指向机器人。
- **单条上限**：平台限制，单位就是平台计数的单位：除 Slack（UTF-16 单元，一个 emoji 算两个）
  与 IRC/Email（字节）外都是字符。

| 通道 | id | 成熟度 | 接入方式 | 编辑 | 线程 | typing | reactions | 收媒体 | 提及门 | 单条上限 | 外部依赖 |
|---|---|---|---|---|---|---|---|---|---|---|---|
| Telegram | `telegram` | preview | 长轮询 / webhook | 是 | 否 | 是 | 是 | 是 | 是 | 4096 字符 | 来自 @BotFather 的 bot token |
| Slack | `slack` | preview | Socket Mode / Events | 是 | 是 | 否 | 是 | 是 | 是 | 4000 UTF-16 单元 | 启用 Socket Mode 或提供 Events API 端点的 Slack 应用 |
| Discord | `discord` | preview | 网关 WebSocket | 是 | 是 | 是 | 是 | 是 | 是 | 2000 字符 | 带消息内容 intent 的 Discord 应用 bot token |
| Mattermost | `mattermost` | preview | WebSocket 事件 + REST | 是 | 是 | 否 | 是 | 否 | 是 | 4000 字符 | Mattermost bot 或私人访问 token |
| Signal | `signal` | preview | 本地守护进程（长轮询） | 否 | 否 | 是 | 是 | 是 | 是 | 4000 字符 | 运行中且已注册号码的 `signal-cli` 守护进程 |
| WhatsApp | `whatsapp` | preview | Cloud API webhook | 否 | 否 | 否 | 是 | 是 | 否 | 4096 字符 | Meta WhatsApp Business 应用 + 公网可达的 webhook |
| Linq | `linq` | preview | 签名 webhook | 否 | 否 | 否 | 否 | 是 | 是 | 4000 字符 | 已分配发送线的 Linq 账号 |
| IRC | `irc` | preview | TCP 或 TLS | 否 | 否 | 否 | 否 | 否 | 是 | 400 字节 | — |
| QQ | `qq` | preview | 网关 WebSocket | 否 | 否 | 否 | 否 | 否 | 是 | 4000 字符 | QQ 开放平台机器人 |
| iMessage | `imessage` | preview | 仅 macOS | 否 | 否 | 否 | 否 | 否 | 否 | 20000 字符 | macOS，且运行桥的终端具备「完全磁盘访问权限」 |
| Email | `email` | preview | IMAP + SMTP | 否 | 是 | 否 | 否 | 否 | 否 | 100000 字节 | 支持密码或应用专用密码的 IMAP/SMTP 邮箱 |
| WeCom | `wecom` | preview | 加密回调 | 否 | 否 | 否 | 否 | 否 | 否 | 2048 字节 | 一个自建应用，其回调 URL 能到达本机 |
| Terminal | `cli` | live | 标准输入输出 | 否 | 否 | 是 | 否 | 否 | 否 | 100000 字符 | — |

飞书与钉钉不在本表中，因为它们不是框架通道；两者都是 `live`，见
[通道配置](channels-config.zh-CN.md)。

### 成熟度

每个通道声明三档之一，`future channel list` 会显示：

- **live**：已在真实平台上跑通，可以依赖。终端通道天然符合（它不依赖任何第三方）；
  飞书与钉钉在走各自桥的前提下同样是 live。
- **preview**：按平台公开 API 文档实现，但尚未在真实部署上验证。请自行验证安装，
  并反馈问题。
- **planned**：已声明但未实现，启用时启动日志报 `unsupported`，`future channel test <id>`
  也会如实说明。

preview 不等于半成品：它周围的链路（策略、去重、会话、流式、审批）是共享且有测试的；
未经验证的是平台方言——确切的 JSON 字段、签名头、限流算法。

## 各通道配置

下列每个块都是该 provider 真正读取的最小集合：它自己的 `config_example`（即
`future channel list` 打印的内容）加上它认可的其他键。所有键都有默认值，标注 *必填* 的凭据
除外。标注 *测试口* 的键用于把通道指向 mock 服务，生产环境留空。策略键（`dm_policy`、
`dm_allowlist`、`group_policy`、`group_allowlist`、`require_mention`）由桥为每个通道读取；
块里列出的是该通道示例中点到的键，其余键取上一节的默认值。

不存在 `streaming` 键：通道是否流式取决于它是否声明 `edit` 能力。该键仍出现在 Discord 与
Mattermost 的 `future channel list` 示例中，但不被读取。

### Telegram

```jsonc
{
  "enabled": true,
  "bot_token": "",                        // 必填：来自 @BotFather
  "mode": "long_poll",                    // long_poll（默认）| webhook
  "webhook": {
    "addr": "127.0.0.1:8787",             // webhook 模式的监听地址
    "path": "/telegram",                  // Telegram 投递更新的路径
    "secret_token": ""                    // 通过 setWebhook 注册，之后每次投递都会被校验
  },
  "dm_policy": "allowlist", "dm_allowlist": [],
  "group_policy": "disabled", "group_allowlist": [], "require_mention": true,
  "api_base": ""                          // 测试口：替换 https://api.telegram.org
}
```

长轮询只需出网 HTTPS，并会把 offset 持久化（`<通道>/offset.json`），因此重启既不丢已排队的
更新，也不会重放已经回答过的消息。webhook 模式需要一个指向 `webhook.addr` 的公网地址
（请放在反代之后以启用 TLS）以及 `secret_token`，通道会校验 Telegram 回显在请求头里的值，
再解析请求体。

只有普通的 `message` 更新会变成提示：编辑消息、频道帖子与服务事件都被忽略。群消息在机器人的
用户名出现在 mention 实体中、或该消息回复了机器人自己的消息时，才算被指向。回复会被转义成
Telegram 的 MarkdownV2 方言，若 Telegram 以格式错误拒收则原样重试一次纯文本。429 属于可重试
（响应体里的 `retry_after` 会写进错误文本）；403 表示机器人被拉黑，属于永久错误。图片与文档
通过 `getFile` 解析（上限 20 MB）并下载到该通道的数据目录。

### Slack

```jsonc
{
  "enabled": true,
  "bot_token": "",                        // 必填：xoxb-…（Web API、文件下载、auth.test）
  "app_token": "",                        // xapp-…：Socket Mode（不需要公网地址）
  "signing_secret": "",                   // 仅 Events API 回退路径使用
  "webhook_port": 3100,                   // Events API 监听端口
  "webhook_path": "/webhooks/slack",      // 平台投递事件的路径
  "dm_policy": "allowlist", "dm_allowlist": [],
  "group_policy": "disabled", "group_allowlist": [], "require_mention": true,
  "api_base": ""                          // 测试口：替换 https://slack.com/api
}
```

必须提供 `bot_token`，并**二选一**：`app_token`（Socket Mode）或 `signing_secret`
（Events API）；两者都没有时通道会拒绝启动并说明原因。推荐用 Socket Mode 接入：
`apps.connections.open` 返回 WebSocket，每个事件信封都必须 ack，否则会被重投，
而且不需要公网端点。Events API 路径会回答 URL challenge 并校验签名与时间戳新鲜度，
过旧的投递会被拒绝而不是重放。

Slack 按 UTF-16 单元计算长度（上限 4000），因此 emoji 较多的回复会比其他平台更早分片——
桥按平台计数的单位而不是 `chars()` 计数，并且不会把围栏代码块切成两半。事件类型是
`app_mention`，或正文中提到机器人的 user id 时，才算被指向。`invalid_auth` 一类属于永久错误；
`ratelimited`/`timeout` 由共享 HTTP 帮助层重试。

### Discord

```jsonc
{
  "enabled": true,
  "bot_token": "",                        // 必填，且需开启消息内容 intent
  "guild_allowlist": [],                  // 已声明但本构建未生效
  "backfill_limit": 20,                   // 重新 IDENTIFY 后每个会话补拉的消息数（上限 100）
  "dm_policy": "allowlist", "dm_allowlist": [],
  "group_policy": "disabled", "group_allowlist": [], "require_mention": true,
  "api_base": "",                         // 测试口：替换 https://discord.com/api/v10
  "gateway_url": ""                       // 测试口：替换 wss://gateway.discord.gg
}
```

机器人必须开启**消息内容** intent，否则消息正文会是空的。心跳间隔取自网关的 `HELLO`，
而不是硬编码常量；`RESUME` 失败时会重新 IDENTIFY 并补齐缺口，而不是丢消息。机器人的 id
出现在 `mentions` 中时，服务器消息才算被指向；私聊始终算。`429` 响应带头 `Retry-After` 与
`X-RateLimit-*`（含 global 标志），`400` 属于永久拒绝。

`guild_allowlist` 会被反序列化但不会被使用：真正让繁忙服务器保持安静的闸门是提及门。
留空即可。

### Mattermost

```jsonc
{
  "enabled": true,
  "base_url": "https://mattermost.example.com",  // 服务根地址，结尾不带 /api/v4
  "token": "",                            // 必填：bot 或私人访问 token
  "channel_allowlist": [],                // 机器人作答的频道；留空表示它可见的每个频道
  "dm_policy": "allowlist", "dm_allowlist": [],
  "group_policy": "disabled", "require_mention": true,
  "api_base": "",                         // 测试口：替换 <base_url>/api/v4
  "ws_url": ""                            // 测试口：替换推导出的 WebSocket 地址
}
```

入站走 WebSocket 事件流：帖子本身以 **JSON 字符串**形式出现在 `data.post` 中（不是对象），
编辑、删除、typing 与机器人自己的帖子都会被丢弃。`data.mentions` 含机器人 user id 时频道
消息才算被指向；私聊始终算。启动时会用 `GET /api/v4/users/me` 解析机器人自身 id，失败是
启动错误而不是告警——否则机器人会回复自己的帖子。`401`/`403` 永久，`429`/`5xx` 可重试。

### Signal

```jsonc
{
  "enabled": true,
  "http_url": "http://127.0.0.1:8080",    // 守护进程基址，结尾不带斜杠
  "number": "+15550001234",               // 必填：守护进程发送所用的账号，E.164 格式
  "uuid": "",                             // 可选：本账号 uuid，用于识别无法解析的 mention
  "receive_timeout_s": 25,                // 守护进程保持 receive 请求打开的时间（上限 30）
  "poll_interval_ms": 1000,               // 空轮询后的间隔
  "dm_policy": "allowlist", "dm_allowlist": [],
  "group_policy": "disabled", "group_allowlist": [], "require_mention": true
}
```

Signal 没有机器人 API。这里的对接目标是由**你自己**运行的 `signal-cli` 守护进程，
通道是它的 HTTP 客户端，因此必须先完成注册并确保可达。只有带正文或附件的 `dataMessage`
信封会变成提示；群组是独立会话（`group:<id>`）。机器人出现在消息的 `mentions` 中、
或该消息引用了机器人自己的消息时，群消息才算被指向。图片附件会被下载并作为模型输入；
下载失败时降级为附件引用，而不是丢消息。守护进程还没起来属于可重试；账号未注册或凭据被拒
属于永久错误。

### WhatsApp

```jsonc
{
  "enabled": true,
  "phone_number_id": "",                  // 必填：发送所用的业务号码
  "access_token": "",                     // 必填：Graph API token
  "verify_token": "",                     // 接收必填：回应 Meta 的订阅校验
  "app_secret": "",                       // 接收必填：校验 X-Hub-Signature-256
  "webhook": {
    "addr": "127.0.0.1:8788",             // 监听地址
    "path": "/webhooks/whatsapp"
  },
  "dm_policy": "allowlist", "dm_allowlist": [],
  "api_base": "",                         // 测试口：替换 https://graph.facebook.com
  "api_version": ""                       // 测试口：替换 Graph API 版本段
}
```

入站需要公网可达的 HTTPS 端点；内建服务器只说明文 HTTP，请放在终止 TLS 的反代之后。
每次投递都用 app secret 对原始请求体签名，未签名或签名不符的请求体会在解析前被拒绝。
Cloud API 一次只和一个客户对话，所以每条消息都是私聊会话，由私聊策略把闸。
入站附件以媒体 id 形式到达，字节藏在两次带鉴权的调用之后，由 provider 在桥看到消息前解析。

出站只能在 24 小时客服窗口内发送（否则需模板），因此发送失败有可能是永久性的
（Meta 错误码 131047）。限流（130429 / 131048 / 80007）属于可重试。

### Linq

```jsonc
{
  "enabled": true,
  "api_key": "",                          // 必填：bearer 密钥
  "from": "",                             // 发送所用的线路，E.164；留空由平台选择
  "webhook": {
    "addr": "127.0.0.1:8789",             // 监听地址
    "path": "/webhooks/linq",
    "signing_secret": ""                  // 接收必填：订阅的 whsec_ 密钥
  },
  "dm_policy": "allowlist", "dm_allowlist": [],
  "group_policy": "disabled", "group_allowlist": [], "require_mention": true,
  "api_base": "",                         // 测试口：替换 API 源
  "api_version": ""                       // 测试口：替换 API 版本段
}
```

端点是公开的，因此无法用 `webhook.signing_secret` 验证的投递会被直接拒绝，而不会进入解析。
只有 `message.received` 事件会变成提示。1:1 会话始终算被指向；群聊中只有某个文本部分报告
提及了本线路时才算。2025-01-01 版负载只报告「存在提及」而不说明是谁被提及，因此仍使用该
版本的订阅需要 `require_mention: false`，群聊才会被作答。图片部分会从签名 CDN 地址拉取；
其他类型只保留引用。

### IRC

```jsonc
{
  "enabled": true,
  "server": "irc.libera.chat",            // 必填
  "port": 6697,
  "tls": true,                            // 明文端口填 false
  "nick": "",                             // 必填
  "password": "",                         // 可选：服务器密码（PASS），与 SASL 不同
  "sasl": { "account": "", "password": "" },  // 要么都填，要么都不填；用 SASL PLAIN 认证
  "channels": ["#future"],                // 本桥作答的唯一频道集合
  "idle_ping_seconds": 120,               // 客户端 PING 间隔；0 表示关闭
  "require_mention": true
}
```

被邀请到别处并不会把那个频道的闲聊送进 agent：只有配置里的 `channels` 才是会话。
频道消息以机器人昵称开头（`nick:`、`nick,` 或 `@nick`）时才算被指向。每条出站行都会切分，
保证**整行**（含命令与 CRLF）落在 IRC 的 512 字节协议上限内——这就是为什么声明的上限是
正文可用的 400 字节而不是 512；控制字符会被替换，避免消息注入命令。昵称冲突会改名处理，
表示「该目标永远不会成功」的数值回复会被记住，投递队列因此不再反复重试。

### QQ

```jsonc
{
  "enabled": true,
  "app_id": "",
  "app_secret": "",
  "sandbox": false,
  "group_allowlist": [],
  "require_mention": true
}
```

使用开放平台 v2 机器人：先取 app access token 并在过期前留出余量刷新；接入 WebSocket 网关
（`op 10` hello、`op 2` identify、`op 1` 心跳、`op 11` ack、`op 0` dispatch）；回复带上原始
`msg_id` 与逐条 `msg_seq`，因此被分片的一条回答仍属于同一条回复。网关下发 `op 7`/`op 9` 时
是重新 identify 而不是退避重试。仅支持文本；群里 `require_mention` 生效，因为群事件只有被
@ 时才会下发。`sandbox` 用于切换平台的沙箱网关。

### iMessage（仅 macOS）

```jsonc
{
  "enabled": true,
  "recipients": [],
  "sender_allowlist": [],
  "poll_seconds": 5,
  "db_path": ""
}
```

发送通过 `osascript` 驱动 Messages.app；接收只读读取本地 Messages 数据库，并把 Apple 纪元
（2001-01-01，纳秒）换算成 Unix 毫秒。`db_path` 默认指向标准位置，存在的意义是让测试指向
夹具。运行桥的进程需要「完全磁盘访问权限」，且该通道只在 macOS 存在——其他平台上每个入口都
报 `unsupported`，不会假装可用。访问规则是 `sender_allowlist`；这里的 iMessage 会话都是私聊，
因此没有提及门。

### Email

```jsonc
{
  "enabled": true,
  "imap": { "host": "imap.example.com", "port": 993, "username": "", "password": "", "mailbox": "INBOX", "security": "implicit", "timeout_seconds": 60 },
  "smtp": { "host": "smtp.example.com", "port": 587, "username": "", "password": "", "from": "", "security": "starttls", "timeout_seconds": 60 },
  "poll_seconds": 30,
  "sender_allowlist": [],
  "subject_prefix": ""
}
```

直接实现两个协议而不引入邮件库：聊天桥需要的子集很小且稳定。发送走 SMTP 提交（`EHLO`、
`STARTTLS` 或隐式 TLS、`AUTH PLAIN`/`AUTH LOGIN`、点转义）；接收走 IMAP 轮询
（`UID SEARCH UNSEEN` 后 `UID FETCH BODY.PEEK[]`，并标记为已读）。正文解析
`multipart/alternative` 与 `mixed`，支持 quoted-printable、base64 与 RFC 2047 头，优先
`text/plain`，回退到去标签的 `text/html`。回复通过 `In-Reply-To`/`References` 串线程，
因此能力里 `threads` 为是。附件只被**列出**而不下载，桥也绝不回复邮箱自身的地址。
`security` 取 `implicit`（连上即 TLS，993/465 端口）或 `starttls`（587/25 端口）；
`timeout_seconds` 限制每一次协议读取，因此「接受连接后不再应答」的服务器会被判为可重试错误，
而不是把轮询循环挂死。不支持 OAuth——请用密码或应用专用密码。

### WeCom

平台自己的名字是“企业微信”；本页其他小节标题也用英文，而且锚点必须保持 `#wecom`——
标题里带上中文名会 slug 成 `wecom-企业微信`，因为这些字符属于 alphanumeric。

```jsonc
{
  "enabled": true,
  "corp_id": "",                         // 企业 ID，ww…；回调的接收方 ID 要与它一致
  "agent_id": 0,                         // 自建应用 AgentId；每次发送都要带上
  "secret": "",                          // 应用 Secret，用于换取 access_token
  "token": "",                           // 回调 Token
  "encoding_aes_key": "",                 // 回调 EncodingAESKey，43 个字符
  "webhook": { "addr": "127.0.0.1:8790", "path": "/webhooks/wecom" },
  "dm_policy": "allowlist", "dm_allowlist": [],
  "api_base": ""                         // 测试接缝：替换应用 API 的源
}
```

企业微信是**回调**模式，而且请求体是**加密**的，两侧都要校验。回调的 `msg_signature` 是
对 Token、时间戳、随机数和密文排序拼接后取 SHA-1；它**先于解密**校验，因为来自网络的密文
是攻击者可选定的输入。随后用 `encoding_aes_key` 做 AES-256-CBC 解密，得到的信封（16 字节
随机、长度、消息本体、以及它本来要发给谁）**必须以本企业 ID 结尾**，否则从别的企业截获的
回调不能在这里重放。

回调的应答是空响应体：这是企业微信规定的“不做被动回复”，也是让平台停止重试的原因。真正的
回复通过应用 API（`message/send`）作为新消息发出，access_token 会缓存到临近过期前再刷新。

调试“发了但没反应”之前，有两件事值得先知道：

* **HTTP 200 不等于成功。** 被拒绝的发送是 `200`，错误在响应体的 `errcode` 里。每次调用都
  检查它，错误信息里也会带上 `errcode` 及其分类。
* **自建应用只收到成员发给它的单聊消息。** 因此每个会话都是单聊，由 `dm_policy` 把关，
  没有“是否提及”可判。群聊是另一种集成（群机器人 Webhook，只能发不能收），不在此范围内。

`max_text_len` 是 2048 **字节**而非字符：一个中文字算三个字节，所以通用分片器会被告知计数
单位，而不是自行假设。

### Terminal

```jsonc
{ "enabled": true }
```

从标准输入读取提示（`/quit` 或 `/exit` 结束），把回答写到标准输出，并带有 `⏳ working…`
进度信号。适合在不注册平台账号的情况下跑通整条链路——策略、会话、排队、流式、审批路由——
也是新增通道时的参考实现。注意它的默认私聊策略是空白名单，因此只有
`{"enabled": true}` 时谁都不接受；请加上 `"dm_policy": "open"`（或把发送者 id `cli`
写进白名单）。

## 诊断

```bash
future channel list                     # 全部通道：成熟度、配置状态、桥类型、外部依赖、能力
future channel status                   # 逐通道状态、计数器与最近错误，外加出站队列
future channel test <id>                # 凭据与连通性自检（会发出一次真实请求）
future channel send --channel <id> --to <会话> --text "..." [--thread <id>] [--durable]
```

`list`、`status`、`test` 都接受 `--format json`（或 `--json`）；JSON 与表格信息一致，
并包含文本视图省略的字段。

`status` 读取的是桥在工作中不断重写的快照，因此在桥**没有**运行时它同样能回答：
快照缺失，或 `updated_unix` 超过 90 秒，说明进程不在，而不是「什么都没发生」——
`status` 会明说是哪一种。逐通道计数器包含接收、发送、去重抑制、因背压丢弃，以及被新消息
抢占的回合数；JSON 视图五项都有，文本视图给出前四项。

`test` 从配置构建该通道（因此配置块必须存在）并运行 provider 的自检：Telegram 用 `getMe`、
Slack 用 `auth.test`、Discord 用 `users/@me`、Mattermost 用 `users/me`、企业微信用换取
access_token 加读取本应用自身信息、WhatsApp 读回号码、Linq 列出账号线路、IRC 真正建立连接并
完成注册、Signal 用账号列表且**不会**抽干守护进程的消息队列、终端通道什么都不做（它始终可用）。
没有自检实现的通道会如实报告，而不是谎报成功。

`send` 是不属于任何会话的出站路径：定时任务、门控通知，或 agent 自己的「完成后通知我」。
谁负责发通知，就由谁调用这条命令。`--to` 是平台的会话 id，`--thread` 在通道支持时把消息
放进线程。`--text -` 从标准输入读取正文。加 `--durable` 会先写入投递队列，因此限流或重启都
不会丢，命令报告 `queued`、`sent` 或 `failed`；不加则立即尝试发送，失败会报告为 `failed`
并以非零状态退出。

### 运行时写入的文件

| 路径 | 内容 |
|---|---|
| `~/.future/channels/config.json` | 配置（见上）。 |
| `~/.future/channels/status.json` | 最新状态快照，先写临时文件再改名，读者不会看到半截文件。 |
| `~/.future/channels/deliveries.json` | 持久出站队列（待发 / 已发 / 失败），上限 2000 条。 |
| `~/.future/channels/<通道>/sessions.json` | 会话 → agent session 映射。 |
| `~/.future/channels/<通道>/inbox/` | 下载的入站图片，文件名取自发送方并做过清洗。 |
| `~/.future/channels/<通道>/offset.json` | 仅 Telegram：长轮询的更新 offset。 |

## 所有通道共享的行为

- **新鲜度与去重。** 同一条消息只回一次：消息 id 记在定长集合里（每通道 1024 条），
  重复投递会被抑制。平台时间戳超过新鲜度窗口（60 秒）的消息按「长时间断线后的回放」处理并
  丢弃，因此重连不会倾泻一堆过期回答；不提供时间戳的平台不做这种过滤，没有 id 的消息会被
  作答而不是吞掉（`channels/src/bridge/dedup.rs`）。
- **一个会话一条回合。** 每个会话（线程算独立会话）有定长邮箱（8 条）并顺序执行回合，
  不同会话之间并发。会话洪泛时，超出部分会被拒绝、记录告警并计数，而不是无上限增长内存
  （`channels/src/bridge/queue.rs`）。
- **新消息优先。** 每条消息都会推进会话的世代，因此在 agent 仍在作答时发纠正，会让旧回合在
  下一个事件处停止并回答最新消息，而不是对同一个问题贴出两个答案。被队列拒收的消息绝不会
  抢占正在运行的回合。
- **先过访问策略。** DM/群策略与提及门通过之前，任何消息都不会到达 agent。被拒的私聊会收到
  载明发送者 id 的说明，便于管理员加入白名单；群里未被 @ 的拒答则保持安静。会话还可以在
  运行时被开关，这正是「本群停用/启用」这类回复所依赖的机制（`channels/src/policy.rs`）。
- **审批在聊天里完成。** agent 挂起受门控的动作时，通道会说明要做什么以及如何答复。
  不超过十二个字符的单独 `yes`/`no`（也含 确认/拒绝）会作为决定送达，且不会被当成新提示；
  更长的文本按普通消息处理，因此真正的提问不会被吞掉。路由只在请求挂起期间存在（15 分钟）
  （`channels/src/bridge/approval.rs`）。
- **流式适配平台。** 声明了 `edit` 的通道得到渐进式回答：随模型输出就地改写并做节流，
  避免触发限流；其余通道在结束时一次性发送，按平台自己的计数单位与边界切分。工具活动会即时
  变成简短说明（`🔧 名称`、`✅ 名称`）。无论如何，失败的回合都会说明原因，编辑失败会退回
  已经发出的文本，被取消的回合保持安静（`channels/src/bridge/sink.rs`）。
- **媒体成为模型输入。** 下载的图片会以清洗后的文件名存到该通道的 `inbox/` 目录，并以
  base64 携带路径交给模型；过大的图片会被跳过并记录告警，而不是让回合失败。非图片附件只保留
  引用，不下载（`channels/src/bridge/mod.rs`）。
- **主动外发是持久的。** 不属于回合的消息走持久队列：按退避重试（5 秒、25 秒、2 分钟、
  10 分钟，最多五次），遇到永久错误立即停止重试而不是耗尽次数，条目会保留为 `failed`
  以便排查。「永久」是拿平台错误文本去匹配一份刻意保守的短语表得出的，因此措辞落在表外的
  永久失败会被重试到次数用尽，再停留为 `failed`（`channels/src/delivery.rs`）。

## 已知限制

下面这些是本页若不写明就会显得比实际更好的地方：

- **只有终端通道经过了真实验证。** 其他框架通道都是 `preview`：按平台公开 API 编写，
  对解析、分片、策略与错误分类有单测，但尚未在真实部署上跑过。请把第一次运行当成一次验证。
- **QQ、iMessage、Email 是 preview，且各有平台形状的限制。** QQ 仅支持文本，其 `sandbox`
  网关与生产网关的差异由平台文档界定而非代码；iMessage 需要 macOS 与「完全磁盘访问权限」，
  且 `osascript` 发送路径在 Messages.app 不存在时会明确失败，因此无图形的服务器无法发送；
  Email 是轮询而非 IDLE，因此一条消息最多晚 `poll_seconds` 才被发现，并且只列出附件而不下载。
- **没有任何通道发送附件。** 入站图片是模型输入；provider 契约里根本没有出站媒体路径。
- **Signal 可能丢消息。** `signal-cli` 守护进程在把消息交给客户端时就将其从队列移除，
  因此轮询中途停下的桥不会再拿到那条消息。它的 typing 与 reaction 调用是针对守护进程响应
  形状的尽力而为，并未验证。
- **WhatsApp 出站受窗口限制。** 超出 24 小时客服窗口时，除非使用模板，发送会永久失败。
- **Linq 的群回复依赖负载版本**，见上文；在旧版本上只能靠 `require_mention: false` 被作答。
- **内建 webhook 服务器是明文 HTTP。** Telegram、WhatsApp、Linq、企业微信的 webhook/回调
  模式需要公网地址；请在 `webhook.addr` 前面用反代终止 TLS。
- **`guild_allowlist`（Discord）不生效**，见上面的 Discord 配置块。真正让繁忙服务器保持安静
  的是提及门。
- **飞书与钉钉不在本页范围内。** 它们都是 `live`，各自运行自己的桥，配置块见
  [通道配置](channels-config.zh-CN.md)。
- **企业微信只承载文本。** 图片、语音、视频回调会被丢弃，而不是变成一条空提示——本 provider
  不解析任何附件；但每个回调的签名与信封校验仍然照常执行。
- **企业微信群聊不可达。** 自建应用收的是单聊消息；群会话需要另一套群机器人集成。

## 参见

- [通道配置](channels-config.zh-CN.md) — 文件位置、`agent` 块、飞书与钉钉。
- [目录布局](directory-layout.zh-CN.md) — 通道文件的位置。
- [通道 provider 契约](channels-provider-contract.zh-CN.md) — 新增通道的契约。
- `channels/src/bridge/` — 共享链路。
