"""Cache-friendly summary requests: keep the conversation as the prefix.

The measurement in `cache_test.py` / `cache_probe.py` showed the difference:

    conversation as a message array + instruction at the tail -> 99.9% cache hit
    the same history flattened into one user message           -> 0%

Real Codex appends its summarization prompt to the live message history, so it
gets the discount. Our A/B/C driver flattened the material into a single user
message, which changes every byte and misses. This module builds both shapes so
the arms can be compared fairly, and records which one was used.
"""


def to_messages(records):
    """The conversation as a real chat message array (tool calls kept paired)."""
    import json
    messages = []
    for record in records:
        if record["kind"] == "tool_call":
            messages.append({"role": "assistant", "content": None,
                             "tool_calls": [{"id": record["call"], "type": "function",
                                             "function": {"name": "read",
                                                          "arguments": json.dumps({"path": record.get("path")})}}]})
        elif record["kind"] == "tool_result":
            messages.append({"role": "tool", "tool_call_id": record["call"], "content": record["text"]})
        else:
            messages.append({"role": record["role"], "content": record["text"]})
    return messages


def codex_summary_messages(records, instruction, system=None):
    """Codex's shape: live history, then the summarization prompt as the last turn.

    `run_inline_auto_compact_task` records the prompt into the existing history and
    sends the whole thing, so the request shares the conversation's prefix.
    """
    messages = to_messages(records)
    messages.append({"role": "user", "content": instruction})
    if system:
        messages.insert(0, {"role": "system", "content": system})
    return messages


def a_style_messages(records, instruction, protected=None, previous_summary=None,
                     template=None, system=None):
    """A's shape, but prefix-preserving.

    The protected originals and prior summary lead, the newly covered records
    follow as real messages, and the instruction closes. Because the leading
    blocks are identical between consecutive stages, the prefix stays stable.
    """
    messages = []
    if system:
        messages.append({"role": "system", "content": system})
    if previous_summary:
        messages.append({"role": "user", "content":
                         "The <prior-summary> covers everything before this conversation and will be "
                         "discarded after this update. Carry forward its still-relevant objectives, "
                         "constraints, user directives, decisions, and workstreams. The newer "
                         "conversation wins conflicts.\n\n<prior-summary>\n"
                         + previous_summary + "\n</prior-summary>"})
    if protected:
        messages.append({"role": "user", "content":
                         "<protected-originals>\n" + _plain(protected) + "\n</protected-originals>"})
    messages.extend(to_messages(records))
    messages.append({"role": "user", "content": instruction + ("\n\n" + template if template else "")})
    return messages


def _plain(records):
    import json
    lines = []
    for record in records:
        if record["kind"] == "tool_call":
            lines.append(f"[Assistant tool call {record['call']}]: read({json.dumps({'path': record.get('path')})})")
        elif record["kind"] == "tool_result":
            label = "error" if record.get("error") else "result"
            lines.append(f"[Tool {label} {record['call']}]: {record['text']}")
        else:
            lines.append(f"[{record['role'].title()}]: {record['text']}")
    return "\n\n".join(lines)
