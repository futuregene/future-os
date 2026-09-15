#!/usr/bin/env python3
"""External compaction strategies, transcribed from upstream source.

These are *reimplementations of the selection rules*, not forks: each function
mirrors the upstream algorithm closely enough that the resulting projection can
be compared with our own arms. Provenance (repository, commit, git blob SHA of
every file read) is recorded in `abc_external_provenance.json` next to this file.

Codex (openai/codex) — local inline path, used for providers without remote
compaction (`codex-rs/core/src/tasks/compact.rs` dispatches to
`crate::compact::run_compact_task` when `RemoteCompactionSupport::Unsupported`
and the `TokenBudget` feature is off):

  * The summarization request is the whole current history plus
    `SUMMARIZATION_PROMPT` as a synthetic user turn.
  * The replacement history is `build_compacted_history`:
    `COMPACT_USER_MESSAGE_MAX_TOKENS = 20_000` tokens of *user messages only*,
    selected newest-first and truncating the oldest one that does not fit, then a
    single summary item. Assistant text and every tool result are dropped.
  * The summary is prefixed with `SUMMARY_PREFIX`.
  * Two variants were not modelled: remote v2 compaction (server-side, needs an
    OpenAI-hosted provider) and the `Feature::TokenBudget` path, which installs a
    fresh context window with no summary at all.

OpenCode (anomalyco/opencode):

  * `packages/core/src/session/compaction.ts` defines the prompt, the serializer
    and the recursive prior-summary update; `packages/opencode/src/session/compaction.ts`
    defines the retained tail (`select` + `tail_start_id`).
  * Tail budget = `min(MAX_PRESERVE_RECENT_TOKENS, max(MIN_PRESERVE_RECENT_TOKENS,
    floor(usable * 0.25)))`, i.e. 15_000 tokens on a large-context model.
  * The new context is `[compaction user message, summary, ...retained tail...]`
    (`packages/opencode/src/session/message-v2.ts`).
  * Serialized tool results are truncated to `TOOL_OUTPUT_MAX_CHARS = 2_000`.
"""

CODEX_SUMMARIZATION_PROMPT = """You are performing a CONTEXT CHECKPOINT COMPACTION. Create a handoff summary for another LLM that will resume the task.

Include:
- Current progress and key decisions made
- Important context, constraints, or user preferences
- What remains to be done (clear next steps)
- Any critical data, examples, or references needed to continue

Be concise, structured, and focused on helping the next LLM seamlessly continue the work."""

CODEX_SUMMARY_PREFIX = (
    "Another language model started to solve this problem and produced a summary of its thinking process. "
    "You also have access to the state of the tools that were used by that language model. Use this to build "
    "on the work that has already been done and avoid duplicating work. Here is the summary produced by the "
    "other language model, use the information in this summary to assist with your own analysis:"
)

CODEX_USER_MESSAGE_MAX_TOKENS = 20_000

OPENCODE_SUMMARY_TEMPLATE = """Output exactly the Markdown structure shown inside <template> and keep the section order unchanged. Do not include the <template> tags in your response.
<template>
## Objective
- [one or two brief sentences describing what the user is trying to accomplish]

## Important Details
- [constraints/preferences, decisions and why, important facts/assumptions, exact context needed to continue, or "(none)"]

## Work State
### Completed
- [finished work, verified facts, or changes made; otherwise "(none)"]

### Active
- [current work, partial changes, or investigation state; otherwise "(none)"]

### Blocked
- [blockers, failing commands, or unknowns; otherwise "(none)"]

## Next Move
1. [immediate concrete action, or "(none)"]
2. [next action if known, or "(none)"]

## Relevant Files
- [file or directory path: why it matters, or "(none)"]
</template>

Rules:
- Keep every section, even when empty.
- Use terse bullets, not prose paragraphs.
- Preserve exact file paths, symbols, commands, error strings, URLs, and identifiers when known.
- Do not mention the summary process or that context was compacted."""

OPENCODE_SUMMARY_UPDATE_INSTRUCTIONS = """The <prior-summary> summarizes everything that happened before the <conversation>. Construct a new summary that combines both. The <prior-summary> is discarded after this: anything you do not carry into the new summary is lost.

When combining:
- Carry forward objectives, constraints, user directives, decisions, and parallel workstreams from the <prior-summary> even when the <conversation> does not mention them. Drop only what is finished and no longer needed.
- The <conversation> is more recent than the <prior-summary>. Where they conflict, the conversation wins: state the corrected fact and drop the old claim.
- Add new progress, decisions, constraints, and context from the conversation.
- Move completed work from "Active" to "Completed".
- If a blocker has been resolved, update the summary to reflect that while keeping any details still needed to continue the work.
- Update "Objective" and "Next Move" to reflect the current work state."""

OPENCODE_COMPACTION_SYSTEM_PROMPT = """You are a context summarization agent. You are given a conversation between a user and an agent. Your goal is to produce a structured summary matching the format specified so another coding agent can continue the work.

Always follow the exact output structure requested by the user prompt. Keep every section, preserve exact file paths and identifiers when known, and prefer terse bullets over paragraphs.

Do not continue the conversation. Do not respond to any questions in the conversation. Only output the structured summary in the exact format requested by the user prompt. Respond in the same language as the conversation."""

OPENCODE_TOOL_OUTPUT_MAX_CHARS = 2_000
OPENCODE_MAX_PRESERVE_RECENT_TOKENS = 15_000
OPENCODE_MIN_PRESERVE_RECENT_TOKENS = 2_000


def estimate_tokens(text):
    from abc_compaction_experiment import tokens as harness_tokens

    return harness_tokens(text)


# ─── shared plain-text rendering (used for the summarization request input) ───


def plain(records):
    """Serialize records the way every arm's summarizer reads them."""
    lines = []
    for record in records:
        if record["kind"] == "tool_call":
            lines.append(f"[Assistant tool call {record['call']}]: read({_json_args(record)})")
        elif record["kind"] == "tool_result":
            label = "error" if record.get("error") else "result"
            lines.append(f"[Tool {label} {record['call']}]: {record['text']}")
        else:
            lines.append(f"[{record['role'].title()}]: {record['text']}")
    return "\n\n".join(lines)


def _json_args(record):
    import json

    return json.dumps({"path": record.get("path")})


# ─── Codex ───────────────────────────────────────────────────────────────────


def codex_summary_request(records):
    """Whole history plus the synthetic compaction prompt as the final user turn."""
    return f"{plain(records)}\n\n[User]: {CODEX_SUMMARIZATION_PROMPT}"


def codex_truncate(text, max_tokens):
    """`truncate_text(..., TruncationPolicy::Tokens(n))`: keep a prefix."""
    if estimate_tokens(text) <= max_tokens:
        return text
    low, high = 0, len(text)
    while low < high:
        middle = (low + high + 1) // 2
        if estimate_tokens(text[:middle]) <= max_tokens:
            low = middle
        else:
            high = middle - 1
    return text[:low]


def codex_build(records, summary):
    """`build_compacted_history` with the default 20_000-token user budget."""
    users = [r for r in records if r["kind"] == "text" and r["role"] == "user"]
    selected = []
    remaining = CODEX_USER_MESSAGE_MAX_TOKENS
    for record in reversed(users):
        if remaining == 0:
            break
        cost = estimate_tokens(record["text"])
        if cost <= remaining:
            selected.append(record["text"])
            remaining -= cost
        else:
            selected.append(codex_truncate(record["text"], remaining))
            break
    selected.reverse()
    body = [f"[User]: {text}" for text in selected]
    body.append(f"[Context compaction summary]: {CODEX_SUMMARY_PREFIX}\n{summary}")
    return "\n\n".join(body)


# ─── OpenCode ────────────────────────────────────────────────────────────────


def _opencode_truncate(value):
    if len(value) <= OPENCODE_TOOL_OUTPUT_MAX_CHARS:
        return value
    return f"{value[:OPENCODE_TOOL_OUTPUT_MAX_CHARS]}\n[truncated]"


def opencode_entries(records):
    """Group records into OpenCode messages.

    OpenCode stores a tool call and its result on one assistant message, so an
    assistant `tool_call` record and the following `tool_result` record become a
    single entry with a completed tool part.
    """
    entries, index = [], 0
    while index < len(records):
        record = records[index]
        if record["kind"] == "tool_call" and index + 1 < len(records) and records[index + 1]["kind"] == "tool_result":
            entries.append({"role": "assistant", "records": [record, records[index + 1]], "order": record["order"]})
            index += 2
            continue
        entries.append({"role": record["role"], "records": [record], "order": record["order"]})
        index += 1
    return entries


def opencode_serialize(entry):
    if entry["role"] == "user":
        text = "\n".join(r["text"] for r in entry["records"] if r["kind"] == "text")
        return f"[User]: {text}" if text else ""
    parts = []
    for record in entry["records"]:
        if record["kind"] == "text":
            parts.append(f"[Assistant]: {record['text']}")
        elif record["kind"] == "tool_call":
            parts.append(f"[Assistant tool call]: read({_json_args(record)})")
        elif record["kind"] == "tool_result":
            if record.get("error"):
                parts.append(f"[Tool error]: {record['text']}")
            else:
                parts.append(f"[Tool result]: {_opencode_truncate(record['text'])}")
    return "\n".join(parts)


def opencode_turns(entries):
    """User-message-delimited turns; `end` is the next user message."""
    starts = [i for i, e in enumerate(entries) if e["role"] == "user"]
    turns = [{"start": s, "end": len(entries)} for s in starts]
    for i in range(len(turns) - 1):
        turns[i]["end"] = turns[i + 1]["start"]
    return turns


def opencode_preserve_budget(context_window, output_tokens=8_192, buffer=20_000):
    """`preserveRecentBudget` = min(15000, max(2000, floor(usable * 0.25)))."""
    reserved = min(buffer, output_tokens)
    usable = max(0, context_window - reserved)
    return min(OPENCODE_MAX_PRESERVE_RECENT_TOKENS, max(OPENCODE_MIN_PRESERVE_RECENT_TOKENS, usable // 4))


def estimate_full(records):
    """Real (untruncated) model-message size.

    OpenCode budgets the retained tail against the *model messages* it would
    actually send, not against the truncated serialization used for the summary
    prompt: `select` calls `MessageV2.toModelMessagesEffect(...)` before
    comparing with the budget.
    """
    total = 0
    for record in records:
        if record["kind"] == "tool_result":
            total += estimate_tokens(record["text"])
        elif record["kind"] == "tool_call":
            total += estimate_tokens(_json_args(record)) + 8
        else:
            total += estimate_tokens(record["text"]) + 8
    return total


def opencode_select(entries, budget):
    """`select`: walk turns newest-first, keeping the ones that fit the budget."""
    turns = opencode_turns(entries)
    if not turns:
        return 0

    def size(items):
        return sum(estimate_full(e["records"]) for e in items)

    total, keep = 0, None
    for turn in reversed(turns):
        turn_size = size(entries[turn["start"]:turn["end"]])
        if total + turn_size <= budget:
            total += turn_size
            keep = turn["start"]
            continue
        # splitTurn: the first sub-slice of this turn that fits the remainder
        remaining = budget - total
        for start in range(turn["start"] + 1, turn["end"]):
            if size(entries[start:turn["end"]]) > remaining:
                continue
            return start
        break
    if keep is None or keep == 0:
        return 0
    return keep


def opencode_summary_request(head_text, previous_summary):
    conversation = f"Here is the conversation so far:\n\n<conversation>\n{head_text}\n</conversation>"
    if not previous_summary:
        return "\n\n".join(
            [
                conversation,
                "Create a new anchored summary from the conversation history in the <conversation> tags above so another coding agent can continue the work.",
                OPENCODE_SUMMARY_TEMPLATE,
            ]
        )
    return "\n\n".join(
        [
            conversation,
            f"Here is the summary of the conversation before the <conversation> above:\n\n<prior-summary>\n{previous_summary}\n</prior-summary>",
            OPENCODE_SUMMARY_UPDATE_INSTRUCTIONS,
            OPENCODE_SUMMARY_TEMPLATE,
        ]
    )


def opencode_build(entries, tail_start, summary):
    """New context = [summary, ...retained tail...].

    The tail is the *retained messages* (their real content), while the summary
    saw the truncated serialization of the head.
    """
    tail_parts = []
    for entry in entries[tail_start:]:
        for record in entry["records"]:
            if record["kind"] == "tool_call":
                tail_parts.append(f"[Assistant tool call]: read({_json_args(record)})")
            elif record["kind"] == "tool_result":
                label = "Tool error" if record.get("error") else "Tool result"
                tail_parts.append(f"[{label}]: {record['text']}")
            else:
                tail_parts.append(f"[{record['role'].title()}]: {record['text']}")
    tail = "\n\n".join(tail_parts)
    return f"[Context compaction summary]: {summary}\n\n<recent-history>\n{tail}\n</recent-history>"
