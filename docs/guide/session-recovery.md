# Releasing a session fenced by a restart

When the agent restarts (or is killed) while a tool call is in flight, that
call's result never reaches the journal. The agent does not know whether the
call ran: the file may already be written, the command may already have changed
something. Recovery therefore keeps the *whole session* fenced — every mutating
tool (`shell`, `write`, `edit`, …) is refused, only `read` gets through — and
every attempt returns the same message:

```
[RECOVERY_UNCERTAIN] The previous operation's effect could not be verified:
shell (call_00_…). This is a session-wide fence, not a transient error: every
tool except `read` is refused for the rest of this session, and retrying one of
these calls returns this same message. Do not retry them. …
```

Retrying does not help, and neither does asking the model to try a different
command: the fence is per session until a person releases it.

## Release it

```bash
future session unblock <session-id>                       # release
future session unblock <session-id> --why "checked it"    # with a recorded reason
future session unblock <session-id> --json                # machine-readable
```

The command is idempotent: running it again on a released (or never fenced)
session reports `not fenced: nothing to release` and appends nothing.

Only the fenced calls are released, and only the calls that were actually
fenced — the entry names exact `(run_id, call_id)` pairs, so an acknowledgement
cannot silently cover a call it was not about.

## What release does — and does not — do

- It **does** make the session usable again: from the next run, `shell`,
  `write` and `edit` work as usual.
- It **does** keep the interrupted operation questioned. Its next attempt
  returns `[RECOVERY_ABORTED_OPERATION]`, telling the model to verify the result
  read-only first and to re-run the operation only if that check shows it did
  not happen. That warning appears once per run, not forever — the same tier a
  user-aborted operation already has.
- It does **not** replay the interrupted call, and it does not execute anything.
- It does **not** mark the operation as verified. "Verified" is a fact about the
  world (`RecoverySafety` proves it for a complete `write` it can read back, and
  for a shell command that provably cannot change anything); a human decision
  is not that fact. Claiming otherwise is exactly the lie the fence exists to
  prevent.

## Why it is not automatic

The fence protects against one specific loss: an unknown side effect being
replayed or silently assumed. Nobody was watching that shell finish, so the
agent must not decide on its own that it is fine to continue — the call could
have deleted a directory, half-written a file, or pushed a commit. A model
asserting "it looks fine" is not evidence either. So the release is a person's
decision, recorded in the session's own history, and it is deliberately narrow:
one entry, the exact calls, who released them, when, and an optional reason.

If you release without checking anything, you accept that risk; the agent will
still warn the model to verify before re-running.

## What is recorded

One appended `custom` entry named `recovery_acknowledged`:

```json
{
  "type": "custom",
  "name": "recovery_acknowledged",
  "content": {
    "calls": [{"run_id": "run-…", "call_id": "call_00_…", "tool": "shell"}],
    "acknowledged_at": "2026-10-25T22:31:07+08:00",
    "by": "cli",
    "reason": "checked the build directory myself"
  }
}
```

Nothing else in the session history is touched: the interrupted call and its
run-terminal marker stay exactly as they were, so the question "did it happen?"
is still answerable after the release.

The entry is journal-only. Conversation views (`future session transcript`, the
history pages, the HTML export) and the model's context all skip entry types
they do not model, so an acknowledgement never leaks into what the model reads
and never shows up as a chat turn. To read it back, and to see the call ids it
released next to the `[RECOVERY_UNCERTAIN]` text the interrupted run recorded:

```bash
sqlite3 "file:$HOME/.future/agent/agent.db?mode=ro" \
  "SELECT timestamp_ms, json_extract(metadata_json,'$.name'), content_json
     FROM entries
    WHERE session_id = '<session-id>' AND entry_type = 'custom'
    ORDER BY position;"
```

`mode=ro` keeps this a read: the agent is the only writer of its store.

## Checking before you release

There is no way for the agent to prove what happened, but you can usually find
out cheaply:

- `future session transcript <id> --grep <text>` — see whether a later call
  already recorded a result for that work.
- Look at the target of the interrupted `write`/`edit`: does the file already
  contain the content?
- For a shell command, check its observable effect (`git status`, the build
  directory, the output file).

Then release and let the model proceed; it is told to verify read-only anyway.

## Other clients

The release is one RPC command, `acknowledge_recovery`
(`session.id` = the session, `message` = the optional reason, `created_by` =
who released it), so a client that does not shell out to `future` can call it
directly. Its response lists the released calls:

```json
{"released": [{"run_id": "run-…", "call_id": "call_00_…", "tool": "shell"}],
 "already_released": false, "replayed": false}
```
