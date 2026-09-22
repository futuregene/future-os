# Channel provider contract

This page is the contract for adding a channel. The bridge — duplicate and stale
filtering, access policy, conversation → session routing, per-conversation
queueing, streaming replies, approval routing, chunking, throttling and retry —
is already written and shared. **A provider holds platform knowledge and nothing
else.**

Read `channels/src/providers/cli.rs` first: it is a complete, working provider in
about 150 lines, and it shows the shape every other provider takes. The
user-facing side of channels is in [Channel providers](channels-providers.md).

## What a provider is

A provider is two halves plus metadata, declared in one file under
`channels/src/providers/`:

```rust
pub static DEFINITION: ChannelDefinition = ChannelDefinition { /* metadata */ };

pub fn provider() -> Box<dyn Provider> { Box::new(MyChannel) }
```

* [`ChannelDefinition`] — identity, capabilities, message limits, maturity, a
  minimal config example, and the external requirements. Both the starter and
  `future channel list` read it, so a channel is described in exactly one place.
* `Provider` — `definition()`, `sender(&ctx)`, `run(ctx)`, and optionally
  `probe(&ctx)`. `run` is the listener; `sender` builds the outbound half (used by
  `run`, by `future channel test` and by `future channel send`, so construction
  must not start long-running work).
* `ChannelSender` — `send_text()`, and optionally `edit_text()`, `typing()`,
  `react()`.

Register the channel with one entry in `channels/src/providers/registry.rs`
(declaration + factory). Nothing else in the framework needs to change.

## Inbound

Your listener parses platform events into `bridge::Inbound` and hands each one to
the bridge:

```rust
let outcome = ctx.handle(inbound, sender.clone()).await;
```

That call is the whole pipeline. Do **not** implement duplicate filtering,
policy checks, session creation, queueing or streaming yourself; if you find
yourself writing one of them, stop — either the interface is missing something
(report it) or the logic belongs in the bridge.

`Inbound` carries what the bridge needs:

| Field | Notes |
|---|---|
| `message_id` | Platform message id; the bridge deduplicates on it (namespaced per channel). |
| `sender.id` | The platform-stable user id that policy matches against. |
| `conversation` | Conversation id, optional `thread_id`, and `ChatKind`. A thread is its own conversation and therefore its own agent session. |
| `text` | Plain text, with platform markup already reduced. |
| `media` | Attachments. Put image bytes in `data` (or a `url` when you cannot download); the bridge saves bytes under the channel's data directory and turns images into model input. |
| `addressed_to_bot` | True when the bot was mentioned or it is a direct message. Group policy uses it. |
| `created_at_ms` | Platform timestamp in Unix **milliseconds**. Messages older than the freshness window are dropped as replays, so do not invent one. |
| `raw` | The untouched platform payload, for a platform-specific extra you will need later. |

## Outbound

`send_text` returns the platform message id (`Some(id)`) when the platform gives
one; return `None` otherwise and the bridge will not attempt progressive edits.

Implement `edit_text` only when the definition advertises `Capabilities::edit`.
The default implementation refuses, which is the honest answer for a channel that
cannot rewrite a message.

Never chunk, throttle, retry or escape markup inside a provider, with one
exception: the platform's own *dialect* (for example Telegram's MarkdownV2). The
bridge already splits against `DEFINITION.max_text_len` and `length_unit` on
character boundaries and without cutting a code fence in half.

`typing` and `react` are best-effort: a failure is logged, and must not abort a
turn.

## Lifecycle

* `run(ctx)` returns when the channel stops or fails. The supervisor then
  restarts it with exponential backoff, so return a *fatal* error and handle a
  recoverable one inside your own loop.
* Call `ctx.mark_running()` once the transport is up, so `future channel status`
  stops reporting `starting`.
* Call `ctx.mark_failed("reason")` for a recoverable failure worth publishing.
* Exit promptly on `ctx.shutdown().notified()`.
* Keep everything you download or remember under `ctx.data_dir()`; never write
  into the user's home or the repository.
* `probe(ctx)` backs `future channel test <id>`: make the smallest real request
  that proves the credentials work and return one line (`"connected as @bot"`).
  Never report success without having made a request.

## Configuration

The channel's block in `~/.future/channels/config.json`:

```jsonc
{
  "providers": {
    "my-channel": { "enabled": true /* ...your fields... */ }
  }
}
```

The framework reads `enabled` and the access-policy keys (`dm_policy`,
`dm_allowlist`, `group_policy`, `group_allowlist`, `require_mention`) from the
same block. Declare your own struct with `#[serde(default)]` on every field and
do **not** add `#[serde(deny_unknown_fields)]` — the policy keys are not yours.
Read it with `ctx.config::<MyConfig>()?`, which names the channel when a field is
malformed. Keep `DEFINITION.config_example` in step with what you read, and
document the block in [Channel providers](channels-providers.md): add a section
for the channel there and point `DEFINITION.docs` at it, including the anchor
(`docs/guide/channels-providers.md#my-channel`). `docs` is what
`future channel list --json` reports, so a path that resolves nowhere reads as
documentation without being any — `every_channel_documents_itself_somewhere_that_exists`
fails the build unless every channel's target, anchor included, exists.

## Errors

Classify an error where the platform makes it unambiguous; let the shared
helpers handle the rest (`transport::http::ErrorClass`,
`delivery::is_permanent_error`).

* Honour `Retry-After` — the HTTP helper already does.
* A blocked bot, a deleted channel or bad credentials is permanent.
* Never retry a send that could duplicate a user-visible message unless the
  platform provides an idempotency key.
* **Put the class in the message text** with `ErrorClass::label`. The durable
  queue stores the text and nothing else, so a classification that never
  reaches the text is lost — it falls back to matching the platform's own
  words, which does not know the codes you just classified. A permanent
  `channel_not_found` shares no words with the queue's "channel not found", so
  it was retried until the attempt cap. Assert it, as
  `a_classified_failure_is_readable_by_the_delivery_queue` does for Signal.

## Tests

Test what is platform-specific and easy to get wrong:

* payload and event parsing, including the shapes you deliberately ignore;
* splitting edge cases specific to your dialect (markup escaping, byte-capped
  protocols, UTF-16 counting) — the generic splitter is covered by
  `transport::text`;
* the addressing rule (mention detection, reply-to-bot, direct message);
* signature verification for webhook providers (valid, invalid, missing);
* error classification, including that the class reaches the durable queue
  (`delivery::is_permanent_error` on the real message text);
* config defaults, and that a malformed block fails with a readable message.

Use `crate::test_support` (`temp_dir`, `home_lock`, `spawn_mock_grpc`,
`spawn_http`, `spawn_ws`) instead of reaching for the network. Tests must never
contact a live platform, and must never reach an agent: an unreachable address
(`http://127.0.0.1:1`) is how the bridge's own tests stay deterministic. Bind
`127.0.0.1:0` rather than a fixed port, so concurrent test processes cannot
collide.

Prefer a test that fails when the logic is wrong over one that asserts the
implementation back to itself.

## Ground rules

* **Write the implementation from the platform's public API documentation.** Do
  not copy code, comments, identifier names or constant tables from another
  chat-bridge project.
* **Do not name another project** in code, comments, tests or docs. Describe
  platform behaviour, not who else implemented it.
* Cross-platform: no hard-coded `/`, `~`, `\` or shell syntax. Use
  `ctx.data_dir()`, `PathBuf` and platform-neutral code.
* Comment the *why* — a platform quirk, an ordering constraint, a rate limit —
  never the *what*.

## Working in a shared checkout

Channel providers may be written in parallel in the same checkout. Two
consequences:

* `cargo` compiles the whole crate, so a half-written peer file can fail your
  `cargo test -p future-channel <filter>`. Re-run once the peer compiles; a
  failure in a file you do not own is not your defect, but do say so rather than
  retrying forever.
* Never `git stash`, `git checkout --`, `git clean` or `git reset`: those act on
  the shared tree and can discard someone else's work. Read-only inspection
  (`git show`, `git diff -- <your file>`) is fine.

## Measuring coverage

New provider code is expected to be covered per line, not merely by a summary
percentage:

```bash
bash scripts/chan-cov.sh                     # whole crate: report + uncovered lines
bash scripts/chan-cov.sh --check channels/src/providers/my-channel.rs
python3 scripts/chan-missed.py providers/my-channel   # line numbers for one file
```

`--check` judges only the files you list, so a peer's in-flight file cannot fail
your gate. Aim for zero uncovered lines; where a line genuinely cannot be reached
in a unit test (an OS failure path, a platform API that cannot be called, a
tracing argument that is only evaluated with a subscriber installed), say so and
explain why it is not injectable rather than deleting the code or weakening an
assertion.
