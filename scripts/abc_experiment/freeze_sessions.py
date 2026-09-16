"""Freeze the three real sessions into the git-ignored research directory.

The report is being rewritten, and its numbers have to be reproducible. The real
sessions are read live from the Agent database, and one of them (`real-stream`) was
still being written to while earlier runs were in progress, so its record count grew
under the measurements. This snapshots all three to immutable JSON once, and every
later step reads the snapshot.

Columns kept: role, kind, text, tool call id, tool name, arguments (for the path),
and journal position. Nothing outside the git-ignored directory is written.
"""
import json, pathlib, sqlite3, sys

DB = pathlib.Path.home() / ".future" / "agent" / "agent.db"
ROOT = pathlib.Path("/Users/geilige/future-os/.future/research/abc-summary-a47313")
FREEZE = ROOT / "frozen-sessions"
CFG = json.loads((ROOT / "real-sessions.json").read_text())


def load(session):
    con = sqlite3.connect(f"file:{DB}?mode=ro", uri=True)
    con.row_factory = sqlite3.Row
    rows = con.execute("""
        SELECT e.position AS position, e.entry_type AS role, mb.kind, mb.text, mb.ordinal,
               mb.tool_call_id, mb.tool_name, mb.arguments_json
        FROM entries e JOIN message_blocks mb
          ON mb.session_id = e.session_id AND mb.entry_position = e.position
        WHERE e.session_id = ? AND mb.kind IN ('text','tool_call','tool_result')
        ORDER BY e.position, mb.ordinal""", (session,)).fetchall()
    out = []
    for r in rows:
        rec = {"position": r["position"], "role": r["role"], "kind": r["kind"],
               "text": r["text"] or ""}
        if r["kind"] == "tool_call":
            rec["call"] = r["tool_call_id"] or f"call-{r['position']}-{r['ordinal']}"
            rec["tool"] = r["tool_name"] or "tool"
            try:
                args = json.loads(r["arguments_json"] or "{}")
            except json.JSONDecodeError:
                args = {}
            rec["path"] = args.get("path") or args.get("file_path") or "" if isinstance(args, dict) else ""
        elif r["kind"] == "tool_result":
            rec["call"] = r["tool_call_id"] or ""
        out.append(rec)

    # Drop tool results with no call and calls with no result: keeping either half
    # alone makes the array invalid for a provider.
    calls = {r["call"] for r in out if r["kind"] == "tool_call" and r.get("call")}
    results = {r["call"] for r in out if r["kind"] == "tool_result" and r.get("call")}
    dangling = calls - results
    kept = [r for r in out
            if not (r["kind"] == "tool_result" and r.get("call") not in calls)
            and not (r["kind"] == "tool_call" and r.get("call") in dangling)]
    return kept, len(dangling), len(results - calls)


def main():
    FREEZE.mkdir(parents=True, exist_ok=True)
    manifest = {}
    for name, sid in CFG["chains"].items():
        records, dangling, orphans = load(sid)
        path = FREEZE / f"{name}.json"
        # `id` is what the compaction driver keys journal entries on; derive it from
        # the frozen position so it is stable across runs.
        for ordinal, r in enumerate(records):
            r["id"] = f"{sid}-{r['position']}-{ordinal}"
        path.write_text(json.dumps({"session": sid, "chain": name,
                                    "records": records}, ensure_ascii=False) + "\n")
        manifest[name] = {"session": sid, "records": len(records),
                          "dangling_tool_calls": dangling, "orphan_results": orphans,
                          "path": str(path)}
        print(f'  {name}: {len(records)} records frozen '
              f'(dropped {dangling} dangling calls, {orphans} orphan results)')
    (FREEZE / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")
    print(f"\nfrozen to {FREEZE}")


if __name__ == "__main__":
    sys.exit(main())
