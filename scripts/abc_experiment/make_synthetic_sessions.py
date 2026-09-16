import os as _os
import pathlib as _pathlib
import subprocess as _subprocess


def _checkout():
    """The checkout this script lives in (…/<checkout>/scripts/abc_experiment/x.py)."""
    return _pathlib.Path(__file__).resolve().parents[2]


def _main_checkout():
    """The main checkout, which owns the shared .future directory.

    `--git-common-dir` resolves to <main>/.git even when running from a worktree, so the
    research directory is found without depending on any absolute path.
    """
    try:
        out = _subprocess.run(
            ["git", "rev-parse", "--path-format=absolute", "--git-common-dir"],
            cwd=_pathlib.Path(__file__).resolve().parent, capture_output=True, text=True,
            timeout=30)
        if out.returncode == 0 and out.stdout.strip():
            return _pathlib.Path(out.stdout.strip()).parent
    except Exception:
        pass
    return _checkout()


def _research():
    override = _os.environ.get("ABC_ROOT")
    if override:
        return _pathlib.Path(override)
    return _main_checkout() / ".future" / "research" / "abc-summary-a47313"


WORKTREE = _checkout()
REPO = _main_checkout()
ROOT = _research()

"""Make the synthetic fixtures readable through the archive CLI.

C's open-book arm uses `future session history`, which reads the Agent's session
database. The synthetic fixtures have session ids that do not exist there, so that arm
had **nothing to read** on half the exam (0 bytes returned) and its synthetic score was
a floor rather than a measurement.

This writes each fixture stage into an isolated copy of the database as a real session,
using the same tables the runtime uses, so the interface works exactly as it does for a
real session. The production database is never touched: the copy is made with SQLite's
backup API and only the copy is written.
"""
import json, pathlib, sqlite3, sys, time

REAL_DB = pathlib.Path.home() / ".future" / "agent" / "agent.db"
ROOT = ROOT
OUT = ROOT / "synthetic-session-db" / "agent.db"


def write_session(con, session_id, records, cwd):
    """One session per fixture boundary, mirroring the runtime's row shapes."""
    con.execute("DELETE FROM message_blocks WHERE session_id=?", (session_id,))
    con.execute("DELETE FROM entries WHERE session_id=?", (session_id,))
    con.execute("DELETE FROM sessions WHERE id=?", (session_id,))
    now = int(time.time() * 1000)
    # The session registry: `search_history` resolves the session through this table,
    # so rows in `entries` alone are invisible to the archive CLI.
    # `title`, `cwd`, `model`, `thinking_level` and `parent_session_id` are all generated
    # columns; the metadata JSON is the only place they can be supplied.
    con.execute(
        "INSERT INTO sessions(id,revision,created_at_ms,updated_at_ms,current_metadata_json)"
        " VALUES(?,?,?,?,?)",
        (session_id, 1, now, now,
         json.dumps({"cwd": cwd, "model": "future/deepseek-flash",
                     "thinkingLevel": "medium", "sessionName": session_id,
                     "tokensIn": 0, "tokensOut": 0, "totalCost": 0.0})))
    # a session_info entry first, as a real session has
    con.execute(
        "INSERT INTO entries(session_id,position,entry_id,entry_type,role,run_id,"
        "timestamp_ms,metadata_json,content_json) VALUES(?,?,?,?,?,?,?,?,?)",
        (session_id, 0, f"{session_id}-info", "session_info", "system",
         f"run-{session_id}", now, json.dumps({"meta": {}}),
         json.dumps({"session": {"cwd": cwd, "model": "future/deepseek-flash",
                                 "thinkingLevel": "medium", "sessionName": session_id}})))
    position = 1
    for record in records:
        kind = record["kind"]
        entry_type = "tool" if kind == "tool_result" else record.get("role", "user")
        role = entry_type
        content = []
        blocks = []
        if kind == "text":
            content = [{"type": "text", "text": record.get("text", "")}]
            blocks = [{"kind": "text", "text": record.get("text", "")}]
        elif kind == "tool_call":
            call = record.get("call", "")
            args = json.dumps({"path": record.get("path", "")})
            content = [{"type": "tool_call", "id": call, "name": record.get("tool", "read"),
                        "args": json.loads(args)}]
            blocks = [{"kind": "tool_call", "tool_call_id": call,
                       "tool_name": record.get("tool", "read"), "arguments_json": args}]
        elif kind == "tool_result":
            call = record.get("call", "")
            content = [{"type": "tool_result", "tool_call_id": call,
                        "content": record.get("text", ""),
                        "is_error": bool(record.get("error"))}]
            blocks = [{"kind": "tool_result", "tool_call_id": call,
                       "text": record.get("text", ""),
                       "is_error": 1 if record.get("error") else 0}]
        else:
            continue
        con.execute(
            "INSERT INTO entries(session_id,position,entry_id,entry_type,role,run_id,"
            "timestamp_ms,metadata_json,content_json) VALUES(?,?,?,?,?,?,?,?,?)",
            (session_id, position, record.get("id", f"{session_id}-{position}"),
             entry_type, role, f"run-{session_id}", now, json.dumps({"meta": {}}),
             json.dumps(content)))
        for ordinal, block in enumerate(blocks):
            con.execute(
                "INSERT INTO message_blocks(session_id,entry_position,ordinal,kind,text,"
                "tool_call_id,tool_name,arguments_json,is_error,metadata_json) "
                "VALUES(?,?,?,?,?,?,?,?,?,?)",
                (session_id, position, ordinal, block.get("kind"), block.get("text"),
                 block.get("tool_call_id"), block.get("tool_name"),
                 block.get("arguments_json"), block.get("is_error"),
                 json.dumps({})))
        position += 1


def main():
    OUT.parent.mkdir(parents=True, exist_ok=True)
    if OUT.exists():
        OUT.unlink()
    src = sqlite3.connect(f"file:{REAL_DB}?mode=ro", uri=True)
    dst = sqlite3.connect(OUT)
    with dst:
        src.backup(dst)
    src.close()
    print(f"copied the session database to {OUT} ({OUT.stat().st_size/1e6:.0f} MB)")

    written = {}
    for task in ("export", "analysis", "pipeline"):
        for stage in (0, 3, 7):
            p = ROOT / "data" / f"{task}-{stage}.json"
            if not p.exists():
                continue
            d = json.loads(p.read_text())
            session_id = d["source_session"]
            records = d["archive"] + d["tail"]
            # the driver's record ids are what the CLI returns as entryId, so keep them
            for ordinal, r in enumerate(records):
                r.setdefault("id", f"{session_id}-{ordinal}")
            write_session(dst, session_id, records, f"/tmp/{task}")
            written[f"{task}__s{stage}"] = {"session": session_id, "records": len(records)}
            print(f"  wrote {session_id}: {len(records)} records")
    with dst:
        dst.commit()
    (ROOT / "synthetic-session-db" / "manifest.json").write_text(
        json.dumps(written, indent=2) + "\n")
    dst.close()
    print(f"\n{len(written)} synthetic sessions written")


if __name__ == "__main__":
    sys.exit(main())
