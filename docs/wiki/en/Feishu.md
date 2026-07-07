# Feishu Integration

Connect FutureOS to Feishu (Lark) so you can talk to the agent from any Feishu chat. The **Channel Bridge** runs as a local service that bridges Feishu messages to your FutureOS agent via gRPC.

---

## How it works

```
Feishu user  ──→  Feishu server  ──→  Channel Bridge (WebSocket)  ──→  Agent (gRPC)
                  (open.feishu.cn)     (your machine)                   (127.0.0.1:50051)
```

The bridge opens a long-lived WebSocket connection to Feishu. When someone sends a message to your bot, Feishu pushes it over the WebSocket, the bridge forwards it to the agent, and the response is streamed back to the chat via CardKit (real-time card updates with markdown rendering).

---

## Prerequisites

1. A **Feishu developer account** at [open.feishu.cn](https://open.feishu.cn) (or [open.larksuite.com](https://open.larksuite.com) for Lark).
2. A **Feishu app** with bot capability enabled.
3. The FutureOS agent already running (`make run-agent` or `future agent start`).

---

## Create a Feishu App

1. Go to the [Feishu Developer Console](https://open.feishu.cn/app).
2. Click **Create Custom App** (企业自建应用).
3. Give it a name (e.g. "FutureOS Bot") and a description.
4. Under **Features** → **Bot**, enable the bot capability.
5. Note the **App ID** and **App Secret** from the **Credentials** page.

### Bot Permissions

Under **Permissions**, add these scopes and apply for release:

| Scope | Purpose |
|---|---|
| `im:message` | Read and send messages |
| `im:message.p2p_msg:read` | Read DM messages |
| `im:message.group_msg:read` | Read group messages |
| `im:message:send_as_bot` | Send messages as the bot |
| `im:resource` | Download images and files |
| `contact:user.base:read` | Resolve user names |

### Event Subscription

Under **Event Subscription**, subscribe to these events:

| Event | Purpose |
|---|---|
| `im.message.receive_v1` | Receive new messages |

Set the **Request URL** to any valid HTTPS endpoint (the bridge uses WebSocket, so this URL is never actually called — but Feishu requires it to enable events).

### Add the Bot to Chats

Once published (even to just your org), you can add the bot to DMs and group chats. Search for your bot by name in Feishu and start a conversation.

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
  "feishu": {
    "enabled": true,
    "app_id": "cli_xxxxxxxxxxxx",
    "app_secret": "your-app-secret",
    "domain": "feishu"
  }
}
```

| Field | Description |
|---|---|
| `agent.grpc_addr` | Agent gRPC address (default: `http://127.0.0.1:50051`) |
| `agent.cwd` | Default working directory for new sessions |
| `agent.model` | Default model for channel sessions (e.g. `future/deepseek-v4-pro`) |
| `agent.thinking_level` | Default thinking level: `off`, `minimal`, `low`, `medium`, `high`, `xhigh` |
| `feishu.enabled` | Set to `true` to start the Feishu bridge |
| `feishu.app_id` | App ID from the Feishu Developer Console |
| `feishu.app_secret` | App Secret from the Feishu Developer Console |
| `feishu.domain` | `"feishu"` for Feishu, `"lark"` for Lark (Lark uses `open.larksuite.com`) |

### Policy Configuration

Control who can talk to your bot:

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

| Field | Values | Description |
|---|---|---|
| `dm_policy` | `"open"` / `"allowlist"` / `"disabled"` | Who can DM the bot. `"allowlist"` (default) only lets users in `dm_allowlist`; `"open"` lets anyone; `"disabled"` blocks all DMs |
| `dm_allowlist` | List of user open_ids, or `["*"]` for all | Users allowed to DM (when dm_policy is `"allowlist"`) |
| `group_policy` | `"open"` / `"allowlist"` / `"disabled"` | Which groups the bot responds in. `"disabled"` (default) blocks all groups unless explicitly enabled per-chat |
| `group_allowlist` | List of group chat_ids, or `["*"]` for all | Groups allowed (when group_policy is `"allowlist"`) |
| `require_mention` | `true` / `false` | When `true` (default), the bot only responds to messages that @mention it |

> **Finding open_ids and chat_ids:** The bot replies with the ID when it denies access — use that to populate your allowlists.

### Behavior Configuration

| Field | Default | Description |
|---|---|---|
| `streaming` | `true` | Stream responses via CardKit in real time |
| `resolve_sender_names` | `true` | Resolve open_ids to display names (slower but friendlier) |
| `max_image_mb` | `10` | Maximum image size in MB the bot will download |
| `typing_indicator` | `false` | Show typing indicator while processing |

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

In any Feishu chat with the bot, use these commands:

| Command | Description |
|---|---|
| `/new` | Start a new session |
| `/status` | Show current session info (model, tokens, cost) |
| `/model <provider/model>` | Switch model (e.g. `deepseek-v4-flash` or `openai/gpt-4o`) |
| `/models` | List available models |
| `/effort <level>` | Set thinking level: `off`, `minimal`, `low`, `medium`, `high`, `xhigh` |
| `/stop` | Abort the current generation |
| `/compact` | Compact conversation context |
| `/cwd <path>` | Set working directory |
| `/help` | Show available commands |

Commands like `/new`, `/status`, `/model`, `/models`, `/effort`, `/compact`, `/cwd`, and `/help` are handled locally by the bridge without hitting the agent. Any unrecognized command is forwarded to the agent as a normal prompt.

---

## How Responses Appear

- **Streaming mode** (default): The bridge creates a CardKit card and updates it in real time as the agent generates. Thinking content is shown in a collapsible blockquote.
- **Non-streaming mode**: The full response is sent as a single markdown message when complete.

---

## Troubleshooting

### Bot doesn't respond

1. Check that the bridge is running: `future channel status`
2. Check that `feishu.enabled` is `true` in `config.json`
3. Check the DM/group policy — the bot may be denying access. Look for the denial message with your open_id or chat_id.
4. Check the bridge logs for WebSocket connection errors.

### Bridge reconnects every ~6 minutes

This is expected Feishu WebSocket behavior. The bridge sends a keepalive ping every 30 seconds and reconnects automatically on timeout. Reconnection is transparent — sessions are preserved.

### Images aren't working

Ensure `im:resource` permission is granted. Check that images are under the `max_image_mb` limit (default 10 MB).

---

## See also

- [[DingTalk Integration|DingTalk]] — connect FutureOS to DingTalk.
- [[Settings]] — configure FutureOS settings and providers.
- [[CLI (future-cli)|CLI]] — command-line tools for service management.
