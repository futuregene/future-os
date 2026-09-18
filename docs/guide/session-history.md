# Session history recall

`future session history` reads original, persisted conversation records through the
Agent. It does not invoke a model, execute the recorded tool, modify a session, or
load the entire conversation into a model runtime. A matching Agent and CLI build
is required. No new model tools are introduced: the agent can use its existing
`shell` tool to run the CLI.

## Search, then read

```sh
future session history search --session SESSION_ID --query "ExpoSharing" --limit 5 --json
future session history get --session SESSION_ID --entry ENTRY_ID --json
```

Search is literal substring matching of user/assistant text, tool arguments and
tool results (ASCII case insensitive), or an exact `tool_call_id` match. It scans
only the selected session, newest entries first. It is not semantic search or an
FTS index; multiple words are one literal substring. Refine the query on no match.
Queries must be nonempty, at most 200 characters and contain no NUL. `--limit`
controls the match count, default 5, maximum 20. A match returns `entryId`,
`blockIndex`, role, timestamp, run/tool identifiers, a bounded snippet, and
`byteOffset`. A snippet can contain replacement characters at its byte-bounded
edges; use `get` to obtain exact text. `hasMore` means additional matches exist.

`get` uses the `(session_id, entry_id)` index. It returns the entry's text/argument
fields as ordered `chunks` with original `blockIndex`, `toolCallId`, `toolName`,
`field`, `blockByteOffset`, and `blockTotalBytes` metadata. It excludes reasoning,
media bodies and provider metadata; `omittedKinds` describes non-recall blocks.
Non-conversation entries such as session settings and checkpoints are not exposed.

### Bounded reads

```sh
future session history get --session SESSION_ID --entry ENTRY_ID --offset 8192 --limit 8192 --json
```

**Offset and limit are UTF-8 bytes, not line numbers or tokens.** The offset
addresses readable block contents concatenated in block order, without adding
separators. Default limit is 8192 bytes; allowed limits are 4..32768. The server
returns bounded BLOB substrings rather than copying a complete large result into
Rust/RPC output buffers; SQLite's internal field-read cost can still depend on
field size. It does not split a UTF-8 character. Use the returned `nextOffset`, or a search
match's `byteOffset`, rather than guessing character boundaries. Invalid offsets
fail explicitly. `hasMore=false` and `nextOffset=null` mark the end. JSON output
preserves exact text, including embedded newlines and NUL; tool arguments are
serialized JSON **text fragments**, not necessarily a complete JSON object on
every page. Returned block byte offsets let clients reconstruct each field.

## Entry IDs versus tool-call IDs

An `entry_id` is an Agent-generated journal identity, normally a timestamp plus a
random suffix. It is unique within its session. The in-memory
`_future_journal_entry_id` metadata binds a message to this record; ordinary model
adapters do not automatically add that ID to the model-facing text.

A model/provider's `tool_call_id` links a tool call with its result. It need not be
globally unique, and repeated IDs can have multiple candidate records. Search can
locate those candidates; use the returned `entryId` for the precise read. One
entry can have several blocks. Search/read results make these references visible
to the model without adding IDs to every historical message.

## How the model learns to use it

The runtime does **not** tell the model that this exists. A session's system prompt is its
own, unchanged: the earlier post-checkpoint `Archived conversation recall` section was
removed because it did not change behaviour (see
[compaction-open-book-experiment.md](../internals/compaction/compaction-open-book-experiment.md) for
the measurement), so what remains is the CLI plus whatever the session's own prompt says.

The reasoning behind the removal is worth keeping, because it is a property of this feature
rather than of that experiment: the guidance only ever described a capability the model
already had, and describing a capability more firmly is not what makes a model use it. The
retained affordance is `future session history search` / `get` behind the ordinary shell
tool, plus the entry ids the journal already puts in search and read results.

Summarization requests are tool-free and were never able to use it.

One consequence to know about: because the prompt no longer grows at a checkpoint, a
session's system prompt is now constant for its whole life. That is strictly better for
provider prefix caching, and it means the summary request and the turn it belongs to are
byte-identical without any code keeping them in step.

## Scope and limitations

Every request requires an explicit session ID; unknown IDs never fall back to a
default session. Entry reads require the entry to belong to that exact session.
The service uses the existing per-user RPC access model, **not a new per-session
ACL**. A trusted CLI user can explicitly browse another session they can access;
the model's instruction to stay in its current session is not a sandbox or an
authorization mechanism. No arbitrary SQL endpoint or database path is exposed.

The [S2 compaction policy](../internals/compaction/compaction.md) defines trigger and retention budgets;
the authoritative database history remains intact. Retrieval and conditional
usage guidance do not guarantee that every model will choose to retrieve
rather than guess. Search excludes hidden reasoning and is deliberately bounded.
