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

`future channel list` covers both kinds and says which is which: the `BRIDGE`
column is `shared` for a framework channel and `own` for a self-bridged one.

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
| everything else | the channel | Platform credentials and options. See the per-channel blocks below. |

The defaults are deliberately closed: the DM policy is an allowlist and it starts
empty, so a channel that configures only its credentials answers **nobody** until
`dm_allowlist` (or `dm_policy: "open"`) says otherwise.

A channel that is enabled but not implemented in this build is reported as
`unsupported` and logged once at startup — it never silently does nothing.

`providers.<id>` also wins over the legacy top-level block for `feishu` and
`dingtalk`, so a channel can be migrated to the new shape without a flag day.

`future channel test <id>` and `future channel send --channel <id>` build one
channel straight from its block, so the block has to exist (`enabled` only decides
whether the bridge *starts* the channel); the bridge does not have to be running.

## Provider matrix

`future channel list` prints this matrix for the build you are running, including
each channel's maturity and its declared external requirements. The capability
columns are what the bridge can rely on:

- **Edit** lets the bridge stream a progressive answer by rewriting one message;
  a channel without it gets a single message at the end.
- **Threads** makes each thread its own agent session and its own conversation.
- **Typing** means the channel has a progress/typing signal. The bridge asks for
  it when a turn starts and while the model is thinking; a channel that answers
  `no` either has no such signal or treats the request as a no-op.
- **Reactions** means the channel can add an acknowledgement emoji. Slack and
  Mattermost put 👀 on a message they accepted; the others expose it but nothing
  calls it yet.
- **Media in** means inbound attachments are read (images become model input).
  No channel sends attachments yet, in either direction (`ChannelSender` has no
  outbound media method).
- **Mention gate** is what the provider can determine about whether a group
  message addressed the bot; the gate itself is the `require_mention` policy,
  applied by the shared policy engine. A direct-only channel — WhatsApp,
  iMessage, Email, Terminal — is `no`, because everything it receives is
  addressed to it by construction.
- **Max message** is the platform limit in the unit the platform counts:
  characters everywhere except Slack (UTF-16 code units, so an emoji costs two)
  and IRC/Email (bytes).

| Channel | Id | Maturity | Inbound | Edit | Threads | Typing | Reactions | Media in | Mention gate | Max message | Requires |
|---|---|---|---|---|---|---|---|---|---|---|---|
| Telegram | `telegram` | preview | long poll / webhook | yes | no | yes | yes | yes | yes | 4096 chars | a bot token from @BotFather |
| Slack | `slack` | preview | Socket Mode / Events | yes | yes | no | yes | yes | yes | 4000 UTF-16 units | a Slack app with Socket Mode enabled or an Events API endpoint |
| Discord | `discord` | preview | gateway websocket | yes | yes | yes | yes | yes | yes | 2000 chars | a Discord application bot token with the message content intent |
| Mattermost | `mattermost` | preview | websocket events + REST | yes | yes | no | yes | no | yes | 4000 chars | a Mattermost bot or personal access token |
| Signal | `signal` | preview | local daemon (long poll) | no | no | yes | yes | yes | yes | 4000 chars | a running `signal-cli` daemon with a registered number |
| WhatsApp | `whatsapp` | preview | Cloud API webhook | no | no | no | yes | yes | no | 4096 chars | a Meta WhatsApp Business app and a publicly reachable webhook URL |
| Linq | `linq` | preview | signed webhook | no | no | no | no | yes | yes | 4000 chars | a Linq account with a provisioned sender |
| IRC | `irc` | preview | TCP or TLS | no | no | no | no | no | yes | 400 bytes | — |
| QQ | `qq` | planned | gateway websocket | no | no | no | no | no | yes | 4000 chars | a QQ open-platform bot |
| iMessage | `imessage` | planned | macOS only | no | no | no | no | no | no | 20000 chars | macOS, and Full Disk Access for the terminal running the bridge |
| Email | `email` | planned | IMAP + SMTP | no | no | no | no | no | no | 100000 bytes | a mailbox that allows IMAP and SMTP with a password or app password |
| Terminal | `cli` | live | stdin/stdout | no | no | yes | no | no | no | 100000 chars | — |

Feishu and DingTalk are not in this table because they are not framework
channels; both are `live` and are described in
[Channels configuration](channels-config.md).

### Maturity

Each channel declares one of three levels, and `future channel list` shows it:

- **live** — exercised against the real platform; safe to rely on. Only the
  terminal channel qualifies today.
- **preview** — implemented against the platform's public API, but not yet
  exercised against a live deployment. Expect to verify your own setup; report
  what breaks.
- **planned** — declared, not implemented. Enabling it is reported as
  `unsupported` at startup, and `future channel test <id>` says so too.

A preview channel is not a half-finished one: the pipeline around it (policy,
deduplication, sessions, streaming, approvals) is shared and tested. What is
unverified is the platform dialect — the exact JSON, the signature header, the
rate-limit arithmetic.

## Per-channel configuration

Each block below is the minimal shape the provider reads: its own
`config_example` (what `future channel list` prints) plus every other key it
honours. Every key has a default and can be omitted except the credentials marked
*required*. Keys marked *test seam* point a channel at a mock server and are
empty in production. The policy keys (`dm_policy`, `dm_allowlist`,
`group_policy`, `group_allowlist`, `require_mention`) are read by the bridge for
every channel; the blocks show the ones that channel's own example names, and the
defaults from the table above apply to the rest.

There is no `streaming` key: a channel streams when it declares the `edit`
capability. The key still appears in `future channel list` examples for Discord
and Mattermost, where it is ignored.

### Telegram

```jsonc
{
  "enabled": true,
  "bot_token": "",                        // required; from @BotFather
  "mode": "long_poll",                    // long_poll (default) | webhook
  "webhook": {
    "addr": "127.0.0.1:8787",             // bind address for webhook mode
    "path": "/telegram",                  // path Telegram posts updates to
    "secret_token": ""                    // registered with setWebhook, then verified on every delivery
  },
  "dm_policy": "allowlist", "dm_allowlist": [],
  "group_policy": "disabled", "group_allowlist": [], "require_mention": true,
  "api_base": ""                          // test seam: replaces https://api.telegram.org
}
```

Long polling needs nothing but outbound HTTPS and remembers its offset across
restarts (`<channel>/offset.json`), so a restart does not lose queued updates and
does not replay what it already answered. Webhook mode needs a public URL that
forwards to `webhook.addr` (put it behind a reverse proxy for TLS) and a
`secret_token`, which Telegram echoes in a header that the channel verifies
before parsing the body.

Only plain `message` updates become prompts: edited messages, channel posts and
service events are ignored. A group message counts as addressed when the bot's
username appears in a mention entity or when the message replies to one of the
bot's own. Replies are escaped into Telegram's MarkdownV2 dialect, and a message
Telegram refuses as malformed is retried once as plain text. A 429 is transient
(the body's `retry_after` is surfaced); a 403 means the bot was blocked and is
permanent. Photos and documents are resolved through `getFile` (20 MB cap) and
downloaded into the channel's data directory.

### Slack

```jsonc
{
  "enabled": true,
  "bot_token": "",                        // required: xoxb-… (Web API, file downloads, auth.test)
  "app_token": "",                        // xapp-…: Socket Mode (needs no public URL)
  "signing_secret": "",                   // Events API fallback only
  "webhook_port": 3100,                   // Events API bind port
  "webhook_path": "/webhooks/slack",      // path the platform posts events to
  "dm_policy": "allowlist", "dm_allowlist": [],
  "group_policy": "disabled", "group_allowlist": [], "require_mention": true,
  "api_base": ""                          // test seam: replaces https://slack.com/api
}
```

A `bot_token` plus **either** `app_token` (Socket Mode) **or** `signing_secret`
(Events API) is required; with neither, the channel refuses to start and says so.
Socket Mode is the recommended inbound path: `apps.connections.open` returns a
websocket, every event envelope must be acked or it is redelivered, and no public
endpoint is needed. The Events API path answers the URL challenge and checks
signature and timestamp freshness; an old delivery is refused rather than
replayed.

Slack counts message length in UTF-16 code units (4000 max), so an emoji-heavy
answer is split earlier than on other platforms — the bridge counts in the unit
the platform counts, not `chars()`, and does not cut a fenced code block in half.
A message is addressed to the bot when the event is an `app_mention` or the text
mentions the bot's user id. `invalid_auth` and friends are permanent;
`ratelimited`/`timeout` are retried by the shared HTTP helper.

### Discord

```jsonc
{
  "enabled": true,
  "bot_token": "",                        // required, with the message content intent
  "guild_allowlist": [],                  // declared but not enforced in this build
  "backfill_limit": 20,                   // messages replayed per conversation after a re-identify (capped at 100)
  "dm_policy": "allowlist", "dm_allowlist": [],
  "group_policy": "disabled", "group_allowlist": [], "require_mention": true,
  "api_base": "",                         // test seam: replaces https://discord.com/api/v10
  "gateway_url": ""                       // test seam: replaces wss://gateway.discord.gg
}
```

The bot needs the **message content** intent, otherwise message text arrives
empty. The heartbeat interval is taken from the gateway's `HELLO`, never from a
constant, and a failed `RESUME` re-identifies and backfills instead of losing
messages. A guild message counts as addressed when the bot appears in `mentions`;
direct messages always count. `429` responses carry `Retry-After` and
`X-RateLimit-*` bucket headers (including a global flag), and `400` is a
permanent rejection.

`guild_allowlist` is deserialized but not consulted: the gate that actually keeps
a busy guild quiet is the mention gate. Leave it empty.

### Mattermost

```jsonc
{
  "enabled": true,
  "base_url": "https://mattermost.example.com",  // server root, no trailing /api/v4
  "token": "",                            // required: bot or personal access token
  "channel_allowlist": [],                // channels the bot answers in; empty = every channel it can see
  "dm_policy": "allowlist", "dm_allowlist": [],
  "group_policy": "disabled", "require_mention": true,
  "api_base": "",                         // test seam: replaces <base_url>/api/v4
  "ws_url": ""                            // test seam: replaces the derived websocket URL
}
```

Inbound is the websocket event stream: the post itself arrives in `data.post` as
**JSON-encoded text**, not an object, and edits, deletes, typing and the bot's own
posts are dropped. A channel message counts as addressed when `data.mentions`
contains the bot's user id; direct messages always count. Startup resolves the
bot's own id through `GET /api/v4/users/me`, and a failure there is a startup
error rather than a warning — without it the bot would answer its own posts.
`401`/`403` are permanent, `429`/`5xx` are retryable.

### Signal

```jsonc
{
  "enabled": true,
  "http_url": "http://127.0.0.1:8080",    // the daemon's base URL, no trailing slash
  "number": "+15550001234",               // required: the account the daemon sends as, in E.164
  "uuid": "",                             // optional: this account's uuid, to recognise an unresolved mention
  "receive_timeout_s": 25,                // how long the daemon holds a receive call open (capped at 30)
  "poll_interval_ms": 1000,               // pause after an empty poll
  "dm_policy": "allowlist", "dm_allowlist": [],
  "group_policy": "disabled", "group_allowlist": [], "require_mention": true
}
```

Signal has no bot API. Endpoint here is a `signal-cli` daemon **you** run; the
channel is an HTTP client of it, so the daemon must be registered and reachable
before the channel starts. Only `dataMessage` envelopes with text or attachments
become prompts; a group is its own conversation (`group:<id>`). A group message
counts as addressed when the bot is in the message's `mentions` or the message
quotes one of the bot's own. Image attachments are downloaded and become model
input; if that download fails the message degrades to an attachment reference
rather than being dropped. A daemon that is not up yet is transient; an
unregistered account or rejected credentials is permanent.

### WhatsApp

```jsonc
{
  "enabled": true,
  "phone_number_id": "",                  // required: the business number messages are sent from
  "access_token": "",                     // required: Graph API token
  "verify_token": "",                     // required to receive: answers Meta's subscription challenge
  "app_secret": "",                       // required to receive: verifies X-Hub-Signature-256
  "webhook": {
    "addr": "127.0.0.1:8788",             // bind address
    "path": "/webhooks/whatsapp"
  },
  "dm_policy": "allowlist", "dm_allowlist": [],
  "api_base": "",                         // test seam: replaces https://graph.facebook.com
  "api_version": ""                       // test seam: replaces the Graph API version segment
}
```

Inbound requires a publicly reachable HTTPS endpoint; put the built-in server
(plain HTTP) behind a TLS reverse proxy. Every delivery is signed with the app
secret over the raw body, and an unsigned or mis-signed body is rejected before
it is parsed. The Cloud API talks to one customer at a time, so every message is
a direct conversation and the DM policy is the gate. Inbound attachments arrive
as a media id behind two authenticated calls, which the provider resolves before
the bridge sees the message.

Outbound is only possible inside the 24-hour customer service window unless a
template is used, so a send may legitimately fail permanently (Meta code 131047).
A rate limit (130429 / 131048 / 80007) is transient.

### Linq

```jsonc
{
  "enabled": true,
  "api_key": "",                          // required: bearer key
  "from": "",                             // the line to send from, E.164; empty lets the platform choose
  "webhook": {
    "addr": "127.0.0.1:8789",             // bind address
    "path": "/webhooks/linq",
    "signing_secret": ""                  // required to receive: the subscription's whsec_ secret
  },
  "dm_policy": "allowlist", "dm_allowlist": [],
  "group_policy": "disabled", "group_allowlist": [], "require_mention": true,
  "api_base": "",                         // test seam: replaces the API origin
  "api_version": ""                       // test seam: replaces the API version segment
}
```

The endpoint is public, so a delivery that does not verify against
`webhook.signing_secret` is refused rather than parsed. Only `message.received`
events become prompts. A 1:1 chat is always addressed; a group chat counts as
addressed only when a text part reports a mention of this line. On the
2025-01-01 payload version a part reports only that *some* mention exists without
saying whose, so a subscription still on that version needs
`require_mention: false` for group chats to be answered at all. Image parts are
fetched from a signed CDN URL; other kinds are left as references.

### IRC

```jsonc
{
  "enabled": true,
  "server": "irc.libera.chat",            // required
  "port": 6697,
  "tls": true,                            // false for a plaintext port
  "nick": "",                             // required
  "password": "",                         // optional server password (PASS), distinct from SASL
  "sasl": { "account": "", "password": "" },  // both or neither; authenticates with SASL PLAIN
  "channels": ["#future"],                // the only channels this bridge answers in
  "idle_ping_seconds": 120,               // client PING interval; 0 disables it
  "require_mention": true
}
```

Being invited elsewhere does not put that channel's chatter in front of the
agent: only the configured `channels` are conversations at all. A channel message
counts as addressed when it starts with the bot's nickname (`nick:`, `nick,` or
`@nick`). Every outbound line is split so the *whole* line — command and CRLF
included — stays inside IRC's 512-byte protocol limit, which is why the declared
maximum is the 400 bytes the payload may use rather than 512; control characters
are neutralised so a message cannot inject a command. A nickname collision is
handled by renaming, and the numeric replies that mean "this target will never
work" are remembered so the delivery queue stops retrying it.

### QQ (planned)

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

Not implemented in this build: enabling it is reported as `unsupported`, and
`future channel test qq` says the same. The block is accepted so configuration can
be written ahead of the implementation.

### iMessage (planned, macOS)

```jsonc
{
  "enabled": true,
  "recipients": [],
  "sender_allowlist": [],
  "poll_seconds": 5,
  "db_path": ""
}
```

Not implemented in this build. The intended shape is `osascript` driving
Messages.app for sending and a read-only poll of the local Messages database for
receiving, which is why it is macOS-only and why the process running the bridge
needs Full Disk Access. On other platforms it reports `unsupported`.

### Email (planned)

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

Not implemented in this build. The intended shape uses a password or app
password (no OAuth), recognizes attachments without downloading them, and never
answers the mailbox's own address.

### Terminal

```jsonc
{ "enabled": true }
```

Reads prompts from stdin (`/quit` or `/exit` ends it) and writes answers to
stdout, with a `⏳ working…` progress signal. Useful for exercising the whole
pipeline — policy, sessions, queueing, streaming, approval routing — without a
platform account, and as the reference implementation for a new channel. Note
that its default DM policy is an empty allowlist, so a bare
`{"enabled": true}` accepts nothing; add `"dm_policy": "open"` (or list the
sender id `cli`).

## Diagnostics

```bash
future channel list                     # every channel: maturity, configured state, bridge kind, requirements, capabilities
future channel status                   # per-channel state, counters and last error, plus the outbound queue
future channel test <id>                # credentials + connectivity self-check (makes a real request)
future channel send --channel <id> --to <conversation> --text "..." [--thread <id>] [--durable]
```

`list`, `status` and `test` accept `--format json` (or `--json`); the JSON carries
the same information as the tables, including the fields the text view leaves out.

`status` reads a snapshot the bridge rewrites as it works, so it also answers when
the bridge is **not** running: a missing snapshot, or an `updated_unix` older than
90 seconds, means the process is down rather than that nothing happened — and
`status` says which. Per-channel counters are received, sent, suppressed
duplicates, messages dropped by backpressure, and turns superseded by a newer
message; the JSON view reports all five, the text view the first four.

`test` builds the channel from its configuration (so the block must exist) and
runs the provider's probe: Telegram `getMe`, Slack `auth.test`, Discord
`users/@me`, Mattermost `users/me`, WhatsApp a read of the phone number, Linq the
account's lines, IRC a real connect and registration, Signal an account listing
that deliberately does **not** drain the daemon's message queue, and the terminal
channel nothing at all (it is always available). A channel whose provider has no
probe reports that instead of claiming success.

`send` is the outbound path that does not belong to a conversation: a cron job, a
gate notification, or an agent's own "notify me when done". Whatever runs the
notification calls this command. `--to` is the platform's conversation id and
`--thread` puts the message in a thread where the channel supports one. `--text -`
reads the body from stdin. With `--durable` the message is written to the
delivery queue first, so a rate limit or a restart cannot lose it, and the
command reports `queued`, `sent` or `failed`; without it the send is attempted
immediately and a failure is reported as `failed`, with a non-zero exit status.

### Files written at runtime

| Path | Contents |
|---|---|
| `~/.future/channels/config.json` | Configuration (see above). |
| `~/.future/channels/status.json` | Latest status snapshot, written by write-then-rename so a reader never sees half a file. |
| `~/.future/channels/deliveries.json` | Durable outbound queue (pending / sent / failed), capped at 2000 entries. |
| `~/.future/channels/<channel>/sessions.json` | Conversation → agent session mapping. |
| `~/.future/channels/<channel>/inbox/` | Downloaded inbound images, named from the sender's filename. |
| `~/.future/channels/<channel>/offset.json` | Telegram only: the long-poll update offset. |

## Behaviour shared by every channel

- **Freshness and duplicates.** A message is answered once: message ids are
  remembered in a bounded set (1024 per channel) and a repeat is suppressed. A
  platform timestamp older than the freshness window (60 seconds) is treated as a
  replay after an outage and dropped, so a reconnect does not dump a backlog of
  stale answers. Platforms that omit a timestamp are never filtered this way, and
  a message with no id is answered rather than swallowed
  (`channels/src/bridge/dedup.rs`).
- **One conversation, one turn.** Each conversation (a thread counts as its own)
  gets a bounded mailbox (8 messages) and runs turns in order, while different
  conversations run concurrently. When a conversation floods, the excess is
  rejected with a logged warning and counted, rather than growing memory without
  limit (`channels/src/bridge/queue.rs`).
- **A newer message wins.** Every message bumps the conversation's generation, so
  sending a correction while the agent is still answering stops the stale turn at
  its next event and answers the newest message, instead of posting two answers to
  one question. A message the queue could not accept never supersedes a running
  turn.
- **Access policy first.** Nothing reaches the agent until the DM/group policy and
  the mention gate allow it. A denied direct message gets an explanation naming
  the sender id so an administrator can allowlist it; a denied group message that
  did not address the bot stays silent. A conversation can also be switched on or
  off at runtime, which is what a per-chat enable/disable reply rides on
  (`channels/src/policy.rs`).
- **Approvals happen in chat.** When the agent parks a gated action, the channel
  posts what it wants to do and how to answer. A bare `yes`/`no` (also 确认/拒绝)
  of at most twelve characters is delivered as the decision and is not treated as
  a new prompt; anything longer is an ordinary message, so a real question is
  never swallowed. A route exists only while a request is outstanding (15 minutes)
  (`channels/src/bridge/approval.rs`).
- **Streaming adapts to the platform.** A channel that declares `edit` gets a
  progressive answer, rewritten in place as the model writes and throttled so it
  does not trip a rate limit; the rest get one message at the end, split on the
  platform's own unit and boundaries. Tool activity becomes a compact note
  (`🔧 name`, `✅ name`) as it happens. Either way a failed turn says why, an edit
  that fails falls back to the text already sent, and a cancelled turn stays quiet
  (`channels/src/bridge/sink.rs`).
- **Media becomes model input.** A downloaded image is saved under the channel's
  `inbox/` directory with a sanitised name and passed to the model as base64 with
  its path; an oversized image is skipped with a warning rather than failing the
  turn. Non-image attachments are referenced, not fetched
  (`channels/src/bridge/mod.rs`).
- **Proactive sends are durable.** A message sent outside a turn goes through a
  persisted queue: it is retried with backoff (5 s, 25 s, 2 min, 10 min, up to
  five attempts), a permanent error stops the retries instead of burning them, and
  the entry stays visible as `failed` for inspection (`channels/src/delivery.rs`).

## Known limitations

These are the things this page would otherwise imply work better than they do:

- **Only the terminal channel is live-verified.** Every other framework channel
  is `preview`: written against the platform's public API, with unit tests for
  parsing, splitting, policy and error classification, but not yet run against a
  real deployment. Treat the first run as a verification exercise.
- **QQ, iMessage and Email are not implemented.** They are declared so the CLI,
  the configuration file and this page can describe them, and enabling one is
  reported as `unsupported`.
- **Nothing sends attachments.** Inbound images are model input; there is no
  outbound media path in the provider contract at all.
- **Signal can lose a message.** The `signal-cli` daemon removes a message from
  its queue as it hands it over, so a bridge that stops mid-poll does not get that
  message again. Its typing and reaction calls are best-effort against the
  daemon's response shapes rather than verified.
- **WhatsApp outbound is windowed.** Outside the 24-hour customer service window a
  send fails permanently unless a template is used.
- **Linq group replies depend on the payload version**, as described above; on the
  older version `require_mention: false` is the only way to be answered.
- **The built-in webhook server is plain HTTP.** Telegram, WhatsApp and Linq
  webhook modes expect a public URL; terminate TLS in a reverse proxy in front of
  `webhook.addr`.
- **`guild_allowlist` (Discord) is not enforced** — see the Discord block above.
  The mention gate is what keeps a busy guild quiet.
- **Feishu and DingTalk are out of scope here.** They are `live`, they run their
  own bridge, and their config blocks are documented in
  [Channels configuration](channels-config.md).

## See also

- [Channels configuration](channels-config.md) — file location, `agent` block,
  Feishu and DingTalk.
- [Directory layout](directory-layout.md) — where channel files live.
- `channels/src/providers/INTERFACE.md` — the contract for adding a channel.
- `channels/src/bridge/` — the shared pipeline.
