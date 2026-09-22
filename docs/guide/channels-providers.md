# Channel providers

This page describes the channels that run on the shared bridge: how to configure
them, what each one supports, and how to diagnose a channel that is not working.
For the file layout and the `agent` block, see
[Channels configuration](channels-config.md).

Two things are called "a channel" in this repository:

- **Framework channels** implement the `Provider` contract
  (`channels/src/providers/`) and get duplicate filtering, access policy, session
  routing, per-conversation queueing, streamed replies and approval routing from
  the shared bridge. Everything on this page except the last section is one of
  these.
- **Self-bridged channels** — Feishu and DingTalk — keep their own bridge, which
  carries platform behaviour the framework does not model yet (interactive
  cards, streaming card elements, approval buttons, slash commands). They use the
  legacy top-level config blocks; see
  [Channels configuration](channels-config.md).

## Configuration

Framework channels are configured under a `providers` key, one block per channel:

```jsonc
{
  "agent": { /* shared by every channel; see channels-config.md */ },
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

Every block is read by exactly one channel, and the parts the bridge needs are
read from the same block:

| Key | Read by | Meaning |
|---|---|---|
| `enabled` | bridge | Start this channel. Absent or `false` means it is never started and is reported as `disabled`. |
| `dm_policy` | bridge | `open`, `disabled`, or `allowlist` (default `allowlist`). |
| `dm_allowlist` | bridge | Sender ids allowed to DM the bot. `["*"]` allows anyone. |
| `group_policy` | bridge | `open`, `disabled` (default), or `allowlist` for groups and channels. |
| `group_allowlist` | bridge | Conversation ids allowed to reach the bot. `["*"]` allows all. |
| `require_mention` | bridge | Inside an allowed group, reply only when the bot was addressed (default `true`). |
| everything else | the channel | Platform credentials and options. See the per-channel tables below. |

A channel that is enabled but not implemented in this build is reported as
`unsupported` and logged once at startup — it never silently does nothing.

`providers.<id>` also wins over the legacy top-level block for `feishu` and
`dingtalk`, so a channel can be migrated to the new shape without a flag day.

## Provider matrix

`future channel list` prints this matrix for the build you are running,
including each channel's maturity. Capabilities are what the bridge can rely on:
`edit` enables progressive streamed replies, `threads` makes each thread its own
agent session, `typing`/`reactions` are acknowledgement signals, and `media`
means inbound attachments (images become model input).

| Channel | Id | Access | Edit | Threads | Typing | Reactions | Media in | Max message | External requirement |
|---|---|---|---|---|---|---|---|---|---|
| Telegram | `telegram` | long poll / webhook | yes | no | yes | yes | yes | 4096 chars | bot token |
| Slack | `slack` | Socket Mode / Events | yes | yes | no | yes | yes | 4000 UTF-16 units | Slack app |
| Discord | `discord` | gateway websocket | yes | yes | yes | yes | yes | 2000 chars | bot token with message content intent |
| Mattermost | `mattermost` | REST + websocket | yes | yes | no | yes | no | 4000 chars | server + token |
| Signal | `signal` | local daemon | no | no | yes | yes | yes | 4000 chars | `signal-cli` daemon |
| WhatsApp | `whatsapp` | Cloud API webhook | no | no | no | yes | yes | 4096 chars | Meta app + public webhook URL |
| QQ | `qq` | gateway websocket | no | no | no | no | no | 4000 chars | open-platform bot |
| Linq | `linq` | signed webhook | no | no | no | no | no | 4000 chars | Linq account |
| iMessage | `imessage` | macOS only | no | no | no | no | no | 20000 chars | macOS + Full Disk Access |
| IRC | `irc` | TCP or TLS | no | no | no | no | no | 400 bytes | — |
| Email | `email` | IMAP + SMTP | no | no | no | no | no | 100000 bytes | mailbox with app password |
| Terminal | `cli` | stdin/stdout | no | no | yes | no | no | 100000 chars | — |

### Maturity

Each channel declares one of three levels, and `future channel list` shows it:

- **live** — exercised against the real platform; safe to rely on.
- **preview** — implemented against the platform's public API, but not yet
  exercised against a live deployment. Expect to verify your own setup; report
  what breaks.
- **planned** — declared, not implemented. Enabling it is reported as
  `unsupported` at startup.

A preview channel is not a half-finished one: the pipeline around it (policy,
deduplication, sessions, streaming, approvals) is shared and tested. What is
unverified is the platform dialect — the exact JSON, the signature header, the
rate-limit arithmetic.

## Per-channel configuration

Each example is the minimal block that starts the channel; every key shown has a
default and can be omitted.

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

Long polling needs nothing but outbound HTTPS and remembers its offset across
restarts, so a restart does not lose queued updates. Webhook mode requires a
public URL that forwards to `webhook.addr` (put it behind a reverse proxy for
TLS) and a `secret_token`, which Telegram echoes in a header that the channel
verifies.

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

`app_token` (Socket Mode) is the recommended inbound path because it needs no
public URL; `signing_secret` is only used when Events are delivered over HTTP.
Slack counts message length in UTF-16 units, so a long emoji-heavy answer is
split earlier than on other platforms — the bridge uses the counted unit, not
`chars()`, to keep the split valid.

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

The bot needs the **message content** intent, otherwise message text arrives
empty. The heartbeat interval is taken from the gateway's `HELLO`, and a failed
`RESUME` re-identifies and backfills instead of losing messages.

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

Signal has no bot API. Endpoint here is a `signal-cli` daemon **you** run; the
channel is an HTTP client of it, so the daemon must be registered and reachable
before the channel starts.

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

Inbound requires a publicly reachable HTTPS endpoint. `verify_token` answers
Meta's subscription challenge and `app_secret` verifies the
`X-Hub-Signature-256` header of every delivery; the channel refuses unsigned
bodies. Outbound is only possible inside the 24-hour customer service window
unless a template is used, so a send may legitimately fail permanently.

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

### iMessage (macOS)

```jsonc
{
  "enabled": true,
  "recipients": [],
  "sender_allowlist": [],
  "poll_seconds": 5,
  "db_path": ""
}
```

Sending drives Messages.app through `osascript`; receiving reads the local
Messages database read-only. The process running the bridge needs Full Disk
Access, and the channel only exists on macOS — on other platforms it reports
`unsupported`.

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

The message limit is in **bytes including the protocol overhead**, which is why
the declared maximum is 400 rather than 512.

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

Uses a password or app password; OAuth is not supported. Attachments are
recognized but not downloaded, and the bridge never answers the mailbox's own
address.

### Terminal

```jsonc
{ "enabled": true }
```

Reads prompts from stdin and writes answers to stdout. Useful for exercising the
whole pipeline without a platform account, and as the reference implementation
for a new channel.

## Diagnostics

```bash
future channel list              # every channel, its maturity, capabilities and whether it is configured
future channel status            # per-channel state, uptime, counters and the last error
future channel test <id>         # credentials + connectivity self-check (makes a real request)
future channel send --channel <id> --to <conversation> --text "..." [--durable]
```

`status` reads a snapshot the bridge rewrites as it works, so it also answers
when the bridge is **not** running: a missing or stale `updated_unix` means the
process is down rather than that nothing happened. Per-channel counters include
received, sent, suppressed duplicates, messages dropped by backpressure, and
turns superseded by a newer message.

`send` is the outbound path that does not belong to a conversation: cron jobs,
loop gate notifications, or an agent's own "notify me when done". With
`--durable` it is written to the delivery queue first, so a rate limit or a
restart cannot lose it; without it, a failure is reported immediately.

### Files written at runtime

| Path | Contents |
|---|---|
| `~/.future/channels/config.json` | Configuration (see above). |
| `~/.future/channels/status.json` | Latest status snapshot; rewritten atomically. |
| `~/.future/channels/deliveries.json` | Durable outbound queue (pending / sent / failed). |
| `~/.future/channels/<channel>/sessions.json` | Conversation → agent session mapping. |
| `~/.future/channels/<channel>/inbox/` | Downloaded inbound attachments. |

## Behaviour shared by every channel

- **Freshness and duplicates.** A message is answered once. A platform timestamp
  older than the freshness window is treated as a replay after an outage and
  dropped, so a reconnect does not dump a backlog of stale answers. Platforms
  that omit a timestamp are never filtered this way.
- **One conversation, one turn.** Each conversation (a thread counts as its own)
  gets a bounded mailbox and runs turns in order, while different conversations
  run concurrently. When a conversation floods, the excess is rejected with a
  logged warning rather than growing memory without limit.
- **A newer message wins.** Sending a correction while the agent is still
  answering stops the stale turn at its next event and answers the newest
  message, instead of posting two answers to one question.
- **Access policy first.** Nothing reaches the agent until the DM/group policy
  and the mention gate allow it. A denied direct message gets an explanation
  naming the sender id so an administrator can allowlist it; a denied group
  message that did not address the bot stays silent.
- **Approvals happen in chat.** When the agent parks a gated action, the channel
  posts what it wants to do and how to answer. A bare `yes`/`no` (also 确认/拒绝)
  is delivered as the decision and is not treated as a new prompt; anything
  longer is an ordinary message, so a real question is never swallowed.
- **Streaming adapts to the platform.** Channels that can edit a message get a
  progressive answer; the rest get one message at the end. Either way a failed
  turn says why, and a cancelled turn stays quiet.
- **Tool activity is visible.** A turn that only ran tools still produces a short
  note naming them, so silence never implies "nothing happened".

## See also

- [Channels configuration](channels-config.md) — file location, `agent` block,
  Feishu and DingTalk.
- [Directory layout](directory-layout.md) — where channel files live.
- `channels/src/providers/INTERFACE.md` — the contract for adding a channel.
- `channels/src/bridge/` — the shared pipeline.
