# Channels configuration (`~/.future/channels/config.json`)

The channel bridge (`future-channel`, see `channels/`) reads a single config
file at `~/.future/channels/config.json`. It connects to the agent over gRPC
and exposes it through Feishu and DingTalk bots.

**First run:** if the file doesn't exist, the bridge writes a default template
and **exits** — edit the file and start it again.

The bridge only starts channels that are `enabled: true`. All fields are
optional; every field has a default, so `{}` is a valid config (agent block
with defaults, both channels disabled).

## Schema

```jsonc
{
  // Agent block — the model/session defaults for channel conversations.
  "agent": {
    "grpc_addr": "http://127.0.0.1:50051", // agent gRPC endpoint
    "cwd": "/home/you",                     // working dir for the agent
    "model": "future/deepseek-v4-pro",      // default model for channel sessions
    "thinking_level": "xhigh",              // off | minimal | low | medium | high | xhigh
    "permission_level": "all"               // all | workspace | none
  },

  // Feishu block — only enabled when "enabled": true.
  "feishu": {
    "enabled": false,
    "app_id": "",
    "app_secret": "",
    "domain": "feishu",                     // API domain; default "feishu"
    "dm_policy": "allowlist",               // open | disabled | allowlist
    "dm_allowlist": [],                     // open_ids, or ["*"] for everyone
    "group_policy": "disabled",             // open | disabled | allowlist
    "group_allowlist": [],                  // chat_ids, or ["*"] for all groups
    "require_mention": true,                // group replies need @bot
    "streaming": true,                      // stream replies (CardKit cards)
    "resolve_sender_names": true,
    "max_image_mb": 10,
    "typing_indicator": false
  },

  // DingTalk block — only enabled when "enabled": true.
  "dingtalk": {
    "enabled": false,
    "client_id": "",
    "client_secret": "",
    "domain": "api.dingtalk.com"
  }
}
```

## Field reference

### `agent`

| Field | Default | Meaning |
|---|---|---|
| `grpc_addr` | `http://127.0.0.1:50051` | gRPC endpoint of the agent. |
| `cwd` | `$HOME` | Working directory for agent runs in channel sessions. |
| `model` | `future/deepseek-v4-pro` | Default model for channel sessions. Empty means "use the agent's boot-time default". |
| `thinking_level` | `xhigh` | Default thinking level: `off` / `minimal` / `low` / `medium` / `high` / `xhigh`. |
| `permission_level` | `all` | Default tool permission level: `all` / `workspace` / `none`. |

### `feishu`

| Field | Default | Meaning |
|---|---|---|
| `enabled` | `false` | Start the Feishu bridge. |
| `app_id` / `app_secret` | empty | Feishu app credentials. |
| `domain` | `feishu` | API domain. |
| `dm_policy` | `allowlist` | Direct-message access: `open` (everyone), `disabled` (nobody), `allowlist` (only `dm_allowlist`). |
| `dm_allowlist` | `[]` | Allowed open_ids; `["*"]` allows everyone. |
| `group_policy` | `disabled` | Group-chat access: `open` / `disabled` / `allowlist`. |
| `group_allowlist` | `[]` | Allowed chat_ids; `["*"]` allows all groups. |
| `require_mention` | `true` | In groups, only reply when the bot is mentioned. |
| `streaming` | `true` | Stream replies (CardKit card streaming). |
| `resolve_sender_names` | `true` | Resolve sender display names. |
| `max_image_mb` | `10` | Max inbound image size in MiB. |
| `typing_indicator` | `false` | Show a typing indicator. |

> Per-group overrides are possible at runtime (e.g. disable a specific chat);
> the config file above only sets the defaults.

### `dingtalk`

| Field | Default | Meaning |
|---|---|---|
| `enabled` | `false` | Start the DingTalk bridge. |
| `client_id` / `client_secret` | empty | DingTalk app credentials. |
| `domain` | `api.dingtalk.com` | API domain. |

## Runtime behavior

- **Slash commands:** both bridges handle 9 commands locally
  (`/new /status /stop /model /models /compact /effort /cwd /help`); unknown
  slash commands are forwarded to the agent as ordinary messages.
- **DingTalk replies** are posted to the `sessionWebhook` from each event —
  every reply is a **new** message (no in-place editing).
- Both bridges reconnect automatically; Feishu keeps a 30s keepalive ping and
  DingTalk a 20s ping.

## See also

- [Directory layout](directory-layout.md) — where this file lives.
- Wiki [Feishu](wiki/en/Feishu.md) / [DingTalk](wiki/en/DingTalk.md) pages — setup
  and usage guides per platform.
- Source: `channels/src/config.rs` (schema + defaults), `channels/src/feishu/policy.rs`
  (dm/group access policies).
