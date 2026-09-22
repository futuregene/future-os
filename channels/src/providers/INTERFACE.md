# Channel provider interface (frozen)

This document is the contract for writing a channel provider. The bridge —
deduplication, access policy, session routing, per-conversation queueing,
streaming replies, approval routing, chunking, throttling and retries — is
already written and shared. **A provider file holds platform knowledge and
nothing else.**

Read `providers/cli.rs` first: it is a complete, working provider in about 150
lines, and it is the shape every other provider should take.

## 1. Files you may change

| Path | Who owns it |
| --- | --- |
| `channels/src/providers/<your-channel>.rs` | you |
| `channels/src/providers/<your-channel>_tests.rs` | you (optional; inline `#[cfg(test)] mod tests` is also fine) |
| everything else | the framework task — do not edit |

`providers/registry.rs` and `providers/mod.rs` already declare and register every
channel. Your module is already listed; you do not add anything to a shared file.
If the interface genuinely cannot express what your platform needs, say so in
your handoff instead of editing a shared file — a parallel worker is editing that
file at the same time.

## 2. What you implement

```rust
pub static DEFINITION: ChannelDefinition = ChannelDefinition { /* metadata */ };

pub fn provider() -> Box<dyn Provider> { Box::new(MyChannel) }
```

and two traits:

* `Provider` — `definition()`, `sender(&ctx)`, `run(ctx)`, optional `probe(&ctx)`.
* `ChannelSender` — `definition()`, `send_text()`, and optionally `edit_text()`,
  `typing()`, `react()`.

`DEFINITION` is already written for you with the platform's real limits and
capabilities. **Flip `maturity` from `Maturity::Planned` to `Maturity::Preview`**
(and to `Maturity::Live` only if you exercised it against the real platform) and
delete the `planned_provider!(DEFINITION);` line.

### Inbound

Your listener parses platform events into `bridge::Inbound` and calls:

```rust
let outcome = ctx.handle(inbound, sender.clone()).await;
```

That call is the whole pipeline. Do not implement dedup, policy checks, session
creation, queueing or streaming yourself; if you find yourself writing any of
them, stop and report why the interface is insufficient.

`Inbound` fields that matter:

* `message_id` — platform message id; dedup keys on it.
* `sender.id` — the platform-stable user id policy matches against.
* `conversation` — id, optional `thread_id`, and `ChatKind`. A thread is its own
  conversation (its own agent session).
* `text` — plain text, platform markup already reduced.
* `media` — attachments; put image bytes in `data`, or a `url` when you cannot
  download. The bridge saves bytes to `data_dir/inbox` and turns images into
  model input.
* `addressed_to_bot` — true when the bot was mentioned or it is a direct
  message. Group policy uses it.
* `created_at_ms` — platform timestamp in Unix **milliseconds** when available.
  Messages older than the freshness window are dropped as replays, so do not
  invent a timestamp.

Fill `raw` with the untouched payload if a platform-specific extra (an
interactive callback id) will be needed later.

### Outbound

`send_text` returns the platform message id (`Some(id)`) when the platform gives
one; return `None` if it does not, and the bridge will not attempt progressive
edits. Implement `edit_text` only when `Capabilities::edit` is set in the
definition — the default implementation refuses, which is the honest answer.

Never chunk, throttle, retry or escape markup inside a provider unless the
platform's *dialect* requires it (for example Telegram's MarkdownV2): the bridge
already splits against `DEFINITION.max_text_len`/`length_unit` on character
boundaries and without breaking code fences.

`typing` and `react` are best-effort: return `Err` and the bridge logs it, but do
not let a failure there abort a turn.

### Lifecycle

* `run(ctx)` returns when the channel stops or fails. The supervisor restarts it
  with exponential backoff, so a *fatal* error should be returned and a
  recoverable one handled inside your own loop.
* Call `ctx.mark_running()` once the transport is connected, so
  `future channel status` stops saying `starting`.
* Call `ctx.mark_failed("reason")` when a recoverable failure is worth
  publishing.
* Exit promptly on `ctx.shutdown().notified()`.
* Store anything you download or remember under `ctx.data_dir()`; never write
  into the user's home or the repo.
* `ctx.probe(&ctx)` backs `future channel test <id>`: do the smallest real
  request that proves the credentials work and return a one-line summary
  (`"connected as @bot"`). Never return success without having made a request.

## 3. Configuration

Your channel's block in `~/.future/channels/config.json`:

```jsonc
{
  "providers": {
    "your-channel": { "enabled": true /* ...your fields... */ }
  }
}
```

The framework reads `enabled` and the access-policy keys (`dm_policy`,
`dm_allowlist`, `group_policy`, `group_allowlist`, `require_mention`) from the
same block; declare your own struct with `#[serde(default)]` on every field and
**do not** add `#[serde(deny_unknown_fields)]` — the policy keys are not yours.
Read it with `ctx.config::<MyConfig>()?`, which produces an error naming the
channel when a field is malformed.

Update `DEFINITION.config_example` so it matches the fields you actually read.

## 4. Errors

Classify errors where the platform makes it unambiguous, and let
`ErrorClass`/`delivery::is_permanent_error` handle the rest:

* honour `Retry-After` (the HTTP helper does this already);
* treat a blocked bot, a deleted channel or bad credentials as permanent;
* never retry a send that could duplicate a user-visible message unless the
  platform provides an idempotency key.

## 5. Tests

Unit-test what is platform-specific and easy to get wrong:

* payload/event parsing including the shapes you intend to ignore;
* message splitting edge cases *specific to your dialect* (markup escaping,
  byte-capped protocols, UTF-16 counting) — the generic splitter is already
  covered by `transport::text`;
* the addressing rule (mention detection, reply-to-bot, direct message);
* signature verification for webhook providers (valid, invalid, missing);
* error classification;
* config defaults.

Use `crate::test_support` (`temp_dir`, `home_lock`, `spawn_mock_grpc`,
`spawn_http`, `spawn_ws`) rather than reaching for the network. Tests must never
contact a live platform, and must never reach an agent: an unreachable address
(`http://127.0.0.1:1`) is how the bridge tests stay deterministic.

Prefer a test that fails when the logic is wrong over a test that asserts the
implementation back to itself.

## 6. Working in a shared checkout

Several providers are written at the same time, in the *same* working tree. That
has two consequences worth knowing before you start:

* **`cargo` compiles the whole crate.** A peer's half-written file can fail your
  `cargo test -p future-channel providers::<yours>`. Filter to your own module
  and re-run once; a failure whose file is not in your write set is not your
  defect, but do say so in your handoff instead of silently retrying forever.
* **Never `git stash`, `git checkout --`, `git clean` or `git reset`.** Those
  operations act on the shared tree and can discard a peer's in-flight work.
  Read-only inspection (`git show`, `git diff -- <your file>`) is fine. If you
  need to isolate your own file, run the test filter, not a tree operation.

Keep the tree compiling as you go: write your module, then
`cargo check -p future-channel` before you start on tests. When you are done,
`cargo fmt -p future-channel` and `cargo clippy -p future-channel --all-targets`
should be clean for your file.

## 7. Ground rules

* **Write the implementation from the platform's public API documentation.** Do
  not copy code, comments, identifier names or constant tables from another
  chat-bridge project.
* **No third-party project may be named** in code, comments, tests, docs or
  commit messages. Describe platform behaviour, not who else implemented it.
* Cross-platform: no hard-coded `/`, `~`, `\` or shell syntax. Use
  `ctx.data_dir()`, `PathBuf`, and platform-neutral code.
* Comment the *why* (a platform quirk, an ordering constraint, a rate limit),
  never the *what*.
