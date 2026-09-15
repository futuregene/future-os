"""Three retrieval interfaces over the same archive.

Each mode reproduces the *published* interface of one agent so the experiment can
separate "how good is the projection" from "how good is the lookup tool".

`ours`      — `future session history search/get`: literal case-insensitive
              substring, `entryId` + byte offsets, fixed-size chunk paging.
`codex`     — `history.list_windows / list_items / read_item / search_contents`
              (codex-rs/ext/history-notes): items grouped into compaction windows,
              short opaque item IDs, **character** offsets, case-sensitive literal
              search, `max_chars_per_item` / `limit_chars` chosen by the caller.
`opencode`  — `glob / grep / read` (packages/opencode/src/tool): ripgrep **regex**
              over the working tree, `path:line` hits capped at 100, and line-range
              reads. OpenCode has no conversation-history tool at all, so this is
              its actual recovery path: re-read the project, not the transcript.

Every mode is given the same overall byte budget for retrieved content.
"""

import json, os, re, shlex, subprocess
from pathlib import Path

# ─── shared budgets ──────────────────────────────────────────────────────────

RETRIEVAL_BYTES = 32_768      # total content returned to the model per probe
NO_CALL_LIMIT = True          # probes run until the model stops on its own
RUNAWAY_GUARD = 60            # safety bound only; tripping it is reported, not treated as normal
GREP_MATCH_LIMIT = 100        # OpenCode's ripgrep limit
OPENCODE_READ_BYTES = 51_200  # OpenCode caps a single read at ~50 KB
CODEX_ITEM_CHARS = 4_000      # default truncated_content when the caller omits it
CODEX_SNIPPET_MODE = "head"   # "head" | "centered" — see module docstring


# ─── windows (Codex's model of history) ──────────────────────────────────────


def build_windows(root):
    """One window per stage: what Codex calls a context-window epoch.

    Reconstructed from the frozen fixtures, so it does not depend on any
    particular arm's projection.
    """
    windows, seen = [], set()
    for stage in range(8):
        for task in ("export", "analysis"):
            data = json.loads((root / "data" / f"{task}-{stage}.json").read_text())
            for record in data["newly_covered"]:
                if record["id"] in seen:
                    continue
                seen.add(record["id"])
                key = (task, stage)
                while len(windows) <= stage:
                    windows.append({})
                windows[stage].setdefault(task, []).append(record)
    return windows


def codex_windows(root, task, stage):
    """Windows visible to a probe at `stage`: one per stage up to and including it."""
    out = []
    for index in range(stage + 1):
        data = json.loads((root / "data" / f"{task}-{index}.json").read_text())
        out.append({"window_id": f"{task}-w{index}", "items": data["newly_covered"]})
    return out


# ─── Codex interface ─────────────────────────────────────────────────────────


def codex_short_id(record):
    """Short suffix, as Codex shows in a trailing `[id: ...]` marker."""
    return record["id"]


def codex_item(record, max_chars, mode=None, query=None):
    body = _item_body(record)
    mode = mode or CODEX_SNIPPET_MODE
    match_at = body.find(query) if (query and mode == "centered") else None
    truncated, _ = _snippet(body, max_chars, mode, match_at)
    return {"item_id": codex_short_id(record), "role": record["role"],
            "truncated_content": truncated, "content_chars": len(body)}


def codex_tools():
    """Tool schemas mirroring codex-rs/ext/history-notes/src/tools.rs.

    Names are `history_*` rather than the Responses-API `history.*`: Chat
    Completions rejects a dot in a function name. Optional arguments are simply
    omitted instead of spelled `["string","null"]`.
    """
    agent = {"type": "string", "description": "Agent whose history to inspect; omit for the current agent."}
    role = {"type": "string", "enum": ["user", "assistant", "tool", "system"],
            "description": "Message role to include; omit for all roles."}
    return [
        {"type": "function", "function": {"name": "history_list_windows",
            "description": "List context windows as window ID and item-count pairs.",
            "parameters": {"type": "object", "properties": {
                "limit": {"type": "integer", "minimum": 1},
                "recent_first": {"type": "boolean"},
                "agent_name": agent}, "required": []}}},
        {"type": "function", "function": {"name": "history_list_items",
            "description": "List history items with optional window and role filters.",
            "parameters": {"type": "object", "properties": {
                "limit": {"type": "integer", "minimum": 1},
                "recent_first": {"type": "boolean"},
                "role": role,
                "window_id": {"type": "string"},
                "max_chars_per_item": {"type": "integer", "minimum": 1},
                "agent_name": agent}, "required": []}}},
        {"type": "function", "function": {"name": "history_read_item",
            "description": ("Read a bounded range from private history. The short item ID is the suffix "
                            "shown in the target item's trailing `[id: ...]` marker."),
            "parameters": {"type": "object", "properties": {
                "item_id": {"type": "string"},
                "window_id": {"type": "string"},
                "offset_chars": {"type": "integer", "minimum": 0},
                "limit_chars": {"type": "integer", "minimum": 1},
                "agent_name": agent}, "required": ["item_id", "window_id"]}}},
        {"type": "function", "function": {"name": "history_search_contents",
            "description": "Search private history by literal substring (case-sensitive).",
            "parameters": {"type": "object", "properties": {
                "query": {"type": "string"},
                "limit": {"type": "integer", "minimum": 1},
                "recent_first": {"type": "boolean"},
                "role": role,
                "window_id": {"type": "string"},
                "max_chars_per_item": {"type": "integer", "minimum": 1},
                "agent_name": agent}, "required": ["query"]}}},
    ]


def codex_dispatch(name, arguments, windows, budget):
    """Execute one Codex-style call. Returns (text, bytes_returned)."""
    # Accept the Responses-API spelling (`history.list_items`) and the chat-completions
    # spelling (`history_list_items`); both must resolve to the same action.
    action = name.split(".", 1)[-1]
    if action.startswith("history_"):
        action = action[len("history_"):]
    flat = [(window["window_id"], item) for window in windows for item in window["items"]]
    if action == "list_windows":
        listed = windows[::-1] if arguments.get("recent_first") else windows
        limit = arguments.get("limit") or len(listed)
        rows = [{"window_id": w["window_id"], "item_count": len(w["items"])} for w in listed[:limit]]
        return json.dumps({"windows": rows}), 0
    if action in ("list_items", "search_contents"):
        pool = flat
        if arguments.get("window_id"):
            pool = [entry for entry in pool if entry[0] == arguments["window_id"]]
        if arguments.get("role"):
            pool = [entry for entry in pool if entry[1]["role"] == arguments["role"]]
        if action == "search_contents":
            query = arguments.get("query") or ""
            # Codex documents a case-sensitive literal substring match.
            pool = [entry for entry in pool if query and query in _item_body(entry[1])]
        else:
            pool = [entry for entry in pool if True]
        pool = pool[::-1] if arguments.get("recent_first") else pool
        limit = arguments.get("limit") or 20
        width = arguments.get("max_chars_per_item") or CODEX_ITEM_CHARS
        items, used = [], 0
        for window_id, record in pool[:limit]:
            item = codex_item(record, width, query=(arguments.get("query") or None))
            items.append({"window_id": window_id, **item})
            used += len(item["truncated_content"])
            if used > budget:
                break
        return json.dumps({"items": items, "matched": len(pool)}), used
    if action == "read_item":
        target = next((record for _, record in flat if codex_short_id(record) == arguments.get("item_id")), None)
        if target is None:
            return json.dumps({"error": "unknown item_id"}), 0
        body = _item_body(target)
        offset = max(0, int(arguments.get("offset_chars") or 0))
        limit = max(1, int(arguments.get("limit_chars") or 4_000))
        limit = min(limit, max(1, budget))
        chunk = body[offset:offset + limit]
        # Documented contract: {content, n_chars, next_offset_chars}. Without the
        # cursor the caller must guess where to resume, which is what the earlier
        # implementation forced the model to do.
        next_offset = offset + len(chunk)
        return json.dumps({"content": chunk, "n_chars": len(chunk),
                           "next_offset_chars": next_offset,
                           "total_chars": len(body)}), len(chunk)
    return json.dumps({"error": f"unknown action {action}"}), 0


def _snippet(body, width, mode, match_at=None):
    """Build `truncated_content` for one item.

    `head` starts at character 0 (my original, unverified assumption).
    `centered` windows around the first match, which is what a search service
    would do to make the hit visible; the true backend behaviour is unobservable
    because the server truncates before encryption.
    """
    if len(body) <= width:
        return body, False
    if mode == "centered" and match_at is not None and match_at >= 0:
        half = max(0, (width - 64) // 2)
        start = max(0, min(match_at - half, len(body) - width))
        return body[start:start + width] + "\n[truncated]", True
    return body[:width] + "\n[truncated]", True


def _item_body(record):
    if record["kind"] == "tool_call":
        return f"[Assistant tool call {record['call']}]: read({json.dumps({'path': record.get('path')})})"
    if record["kind"] == "tool_result":
        label = "error" if record.get("error") else "result"
        return f"[Tool {label} {record['call']}]: {record['text']}"
    return f"[{record['role'].title()}]: {record['text']}"


# ─── OpenCode interface ──────────────────────────────────────────────────────


def opencode_tools():
    return [
        {"type": "function", "function": {"name": "glob",
            "description": "Find files by glob pattern.",
            "parameters": {"type": "object", "properties": {
                "pattern": {"type": "string"}, "path": {"type": "string"}}, "required": ["pattern"]}}},
        {"type": "function", "function": {"name": "grep",
            "description": "Search file contents with a regular expression.",
            "parameters": {"type": "object", "properties": {
                "pattern": {"type": "string"},
                "path": {"type": "string", "description": "The directory to search in."},
                "include": {"type": "string", "description": 'File pattern to include, e.g. "*.log"'}},
                "required": ["pattern"]}}},
        {"type": "function", "function": {"name": "read",
            "description": "Read a file, optionally a line range.",
            "parameters": {"type": "object", "properties": {
                "filePath": {"type": "string"},
                "offset": {"type": "integer", "description": "Line number to start reading from (1-indexed)"},
                "limit": {"type": "integer", "description": "Maximum number of lines to read"}},
                "required": ["filePath"]}}},
    ]


def materialize_workspace(root, task, stage, directory):
    """OpenCode's recovery path needs a working tree, so write the current state.

    Only the newest version of each path is present: that is what a coding agent
    actually has on disk. Values that an earlier version overwrote are therefore
    not reachable this way, which is a property of the interface, not a bug here.
    """
    data = json.loads((root / "data" / f"{task}-{stage}.json").read_text())
    latest = {}
    for record in data["archive"] + data["tail"]:
        if record["kind"] == "tool_result" and record.get("path"):
            latest[record["path"]] = record["text"]
    directory.mkdir(parents=True, exist_ok=True)
    for name, text in latest.items():
        (directory / name).write_text(text, encoding="utf-8")
    return sorted(latest)


def opencode_dispatch(name, arguments, workspace, budget):
    if name == "glob":
        pattern = arguments.get("pattern") or "*"
        import fnmatch
        hits = sorted(p.name for p in workspace.glob("*") if fnmatch.fnmatch(p.name, pattern))
        text = "\n".join(str(workspace / h) for h in hits) or "No files found"
        return text, len(text)
    if name == "grep":
        pattern = arguments.get("pattern") or ""
        include = arguments.get("include")
        try:
            regex = re.compile(pattern)
        except re.error as error:
            return f"Invalid regular expression: {error}", 0
        rows = []
        for path in sorted(workspace.glob("*")):
            if include:
                import fnmatch
                if not fnmatch.fnmatch(path.name, include):
                    continue
            for number, line in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
                if regex.search(line):
                    rows.append((str(path), number, line.strip()))
                    if len(rows) >= GREP_MATCH_LIMIT:
                        break
            if len(rows) >= GREP_MATCH_LIMIT:
                break
        if not rows:
            return "No files found", 0
        out = [f"Found {len(rows)} matches" + (" (more matches available)" if len(rows) >= GREP_MATCH_LIMIT else "")]
        current = ""
        for path, number, line in rows:
            if current != path:
                if current:
                    out.append("")
                current = path
                out.append(f"{path}:")
            out.append(f"  Line {number}: {line}")
        if len(rows) >= GREP_MATCH_LIMIT:
            out.extend(["", "(Results truncated. Consider using a more specific path or pattern.)"])
        text = "\n".join(out)
        return text[:budget], min(len(text), budget)
    if name == "read":
        target = workspace / Path(arguments.get("filePath") or "").name
        if not target.exists():
            return f"File not found: {target}", 0
        lines = target.read_text(encoding="utf-8").splitlines()
        offset = max(1, int(arguments.get("offset") or 1))
        limit = int(arguments.get("limit") or 2_000)
        window = lines[offset - 1:offset - 1 + limit]
        body = "\n".join(f"{offset + index}: {line}" for index, line in enumerate(window))
        tail = (f"\n(Showing lines {offset}-{offset + len(window) - 1} of {len(lines)}. Use offset="
                f"{offset + len(window)} to continue.)") if offset - 1 + limit < len(lines) else \
               f"\n(End of file - total {len(lines)} lines)"
        text = f"<path>{target}</path>\n<type>file</type>\n<content>\n{body}{tail}\n</content>"
        if len(text.encode()) > OPENCODE_READ_BYTES:
            text = text.encode()[:OPENCODE_READ_BYTES].decode("utf-8", errors="ignore")
        return text, len(text.encode())
    return f"Unknown tool {name}", 0


# ─── ours ────────────────────────────────────────────────────────────────────


def our_tools(session_id):
    return [{"type": "function", "function": {
        "name": "shell",
        "description": "Run one read-only history CLI command for this archive session.",
        "parameters": {"type": "object", "properties": {"command": {"type": "string"}},
                       "required": ["command"]}}}]


def our_dispatch(command, session_id, binary, env, cwd):
    """`future session history search|get` only; argv, never a shell."""
    argv = shlex.split(command)
    if argv[:3] != ["future", "session", "history"] or len(argv) < 4 or argv[3] not in ("search", "get"):
        return "TEST_POLICY: only history search/get allowed", 0
    if argv[4:] not in (["--help"], ["-h"]):
        options, index = {}, 4
        while index < len(argv):
            flag = argv[index]
            if flag == "--json":
                index += 1
                continue
            if flag not in ("--session", "--query", "--entry", "--offset", "--limit") or flag in options or index + 1 >= len(argv):
                return "TEST_POLICY: invalid option", 0
            options[flag] = argv[index + 1]
            index += 2
        if options.get("--session") != session_id:
            return "TEST_POLICY: wrong session scope", 0
    result = subprocess.run([str(binary), *argv[1:]], env=env, cwd=cwd, capture_output=True, text=True, timeout=60)
    output = result.stdout if result.returncode == 0 else result.stderr
    return output + f"\n[exit: {result.returncode}]", len(output.encode())


def guide(mode, session_id=None, workspace=None):
    """The system-prompt paragraph describing how to look things up."""
    if mode == "ours":
        return (f" Archive session ID: {session_id}. Use the existing shell tool ONLY for "
                f'`future session history search --session {session_id} --query "literal keyword" --limit 5 --json` or '
                f'`future session history get --session {session_id} --entry ENTRY_ID --offset BYTE_OFFSET --limit 8192 --json`. '
                "Search is literal. Use returned entryId/byteOffset/nextOffset; do not invent IDs, scan the filesystem, or run "
                "other commands. Multiple separate calls are allowed. Stop when the evidence is sufficient.")
    if mode == "codex":
        return (" Private history tools are available: `history.list_windows`, `history.list_items`, "
                "`history.read_item` and `history.search_contents`. Search is a case-sensitive literal substring. "
                "Items live in context windows; `read_item` needs the window ID shown alongside each item and accepts "
                "character offsets. Prefer a small number of wide reads over many narrow ones.")
    if mode == "opencode":
        return (f" The project working tree is at {workspace}. Use `glob`, `grep` and `read` to inspect the current files; "
                "`grep` takes a regular expression and `read` takes an optional line range. Only the newest version of each "
                "file is on disk.")
    raise SystemExit(f"unknown retrieval mode {mode}")
