# DingTalk Integration

Connect FutureOS to DingTalk so you can talk to the agent from any DingTalk chat. The **Channel Bridge** runs as a local service that bridges DingTalk messages to your FutureOS agent via gRPC.

---

## How it works

```
DingTalk user  ──→  DingTalk server  ──→  Channel Bridge (Stream Mode)  ──→  Agent (gRPC)
                    (api.dingtalk.com)    (your machine)                      (127.0.0.1:50051)
```

The bridge uses **DingTalk Stream Mode** — no public callback URL needed. It opens a WebSocket connection to DingTalk, receives messages in real time, forwards them to the agent, and replies via the `sessionWebhook` URL included in each event.

---

## Prerequisites

1. A **DingTalk developer account** at [open.dingtalk.com](https://open.dingtalk.com).
2. A **DingTalk app** with Stream Mode enabled.
3. The FutureOS agent already running (`make run-agent` or `future agent start`).

---

## Create a DingTalk App

1. Go to the [DingTalk Developer Console](https://open-dev.dingtalk.com).
2. Create a new application.
3. Under **Bot** → **Message Reception Mode**, select **Stream Mode**.
4. Note the **Client ID** (AppKey) and **Client Secret** (AppSecret) from the credentials page.

### Bot Permissions

The bot needs these permissions (granted when you enable the bot capability):

| Scope | Purpose |
|---|---|
| `im.message.receive` | Receive messages via Stream Mode |
| `im.message.send` | Send messages back to chats |
| `qyapi_robot_webhook_message_send` | Send messages via webhook |

### Add the Bot to Chats

After deployment, add the bot to DMs and group chats. Users can find the bot by name in DingTalk and start a conversation.

---

## Configuration

Edit `~/.future/channels/config.json`:

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
    "client_secret": "your-client-secret",
    "domain": "api.dingtalk.com"
  }
}
```

| Field | Description |
|---|---|
| `agent.grpc_addr` | Agent gRPC address (default: `http://127.0.0.1:50051`) |
| `agent.cwd` | Default working directory for new sessions |
| `agent.model` | Default model for channel sessions (e.g. `future/deepseek-v4-pro`) |
| `agent.thinking_level` | Default thinking level: `off`, `minimal`, `low`, `medium`, `high`, `xhigh` |
| `agent.permission_level` | Default permission level: `all`, `workspace`, `none` |
| `dingtalk.enabled` | Set to `true` to start the DingTalk bridge |
| `dingtalk.client_id` | Client ID (AppKey) from the DingTalk Developer Console |
| `dingtalk.client_secret` | Client Secret (AppSecret) from the DingTalk Developer Console |
| `dingtalk.domain` | API domain (default: `api.dingtalk.com`) |

---

## Start the Bridge

```bash
# Build and run the channel bridge
make build-channels-release
./target/release/future-channels
```

Or manage it as a service:

```bash
future channel start    # macOS launchctl / Linux systemd
future channel status   # check status
future channel stop     # stop
future channel restart  # restart
```

The bridge loads `~/.future/channels/config.json` on startup. If the file doesn't exist, a template is created and the bridge exits — edit the template and restart.

---

## Slash Commands

In any DingTalk chat with the bot, use these commands:

| Command | Description |
|---|---|
| `/new` | Start a new session |
| `/status` | Show current session info (model, tokens, cost) |
| `/stop` | Abort the current generation |
| `/model <provider/model>` | Switch model (e.g. `deepseek-v4-flash` or `openai/gpt-4o`) |
| `/models` | List available models |
| `/effort <level>` | Set thinking level: `off`, `minimal`, `low`, `medium`, `high`, `xhigh` |
| `/compact` | Compact conversation context |
| `/cwd <path>` | Set working directory |
| `/help` | Show available commands |

All slash commands are handled locally by the bridge without hitting the agent. Any unrecognized command is forwarded to the agent as a normal prompt.

---

## How Responses Appear

- Responses are sent as **markdown** via the session webhook.
- Each reply is a **new message** (DingTalk Stream Mode webhooks don't support in-place editing).
- Thinking content appears as a blockquote (`> 💭`) separated from the main content by a divider.
- Slash command responses are brief status messages.

---

## Differences from Feishu

| Feature | Feishu | DingTalk |
|---|---|---|
| Connection | WebSocket (pbbp2 protobuf frames) | WebSocket (Stream Mode JSON) |
| Streaming | CardKit real-time card updates | New markdown message per reply (no in-place streaming) |
| Thinking | Collapsible blockquote in CardKit | Blockquote with `> 💭` prefix |
| Emoji reactions | ✅ (ACK/DONE processing indicators) | ❌ (API not publicly available) |
| Multi-modal | Image and file support | Text and markdown only |

---

## Troubleshooting

### Bot doesn't respond

1. Check that the bridge is running: `future channel status`
2. Check that `dingtalk.enabled` is `true` in `config.json`
3. Verify the `client_id` and `client_secret` are correct.
4. Check the bridge logs for Stream Mode connection errors.

### Bridge reconnects frequently

DingTalk Stream Mode WebSocket has a keepalive ping every 20 seconds. Reconnection is automatic and transparent — sessions are preserved.

### Markdown formatting looks wrong in DingTalk

DingTalk markdown requires double line breaks (`\n\n`) for paragraph separation. Single `\n` doesn't create a visible break. The bridge handles this automatically, but if something looks off, it may be a DingTalk rendering limitation.

---

## See also

- [[Feishu Integration|Feishu]] — connect FutureOS to Feishu/Lark.
- [[Settings]] — configure FutureOS settings and providers.
- [[CLI (future-cli)|CLI]] — command-line tools for service management.
