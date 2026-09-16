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
    """The experiment root: fixtures, frozen sessions, ledgers and results.

    Deliberately outside any repository -- it holds real session data and large ledgers
    that must never be committed. `ABC_ROOT` overrides the default.
    """
    override = _os.environ.get("ABC_ROOT")
    if override:
        return _pathlib.Path(override)
    return _pathlib.Path.home() / "compact-exp"


def require(path, what, how=""):
    """Return `path` or stop immediately with an explanation.

    Inputs used to be skipped when absent, so a run without them produced a partial result
    that looked complete. Failing here is the difference between "the numbers are wrong"
    and "the numbers are missing".
    """
    path = _pathlib.Path(path)
    if path.exists():
        return path
    raise SystemExit(
        f"missing {what}:\n  {path}\n"
        + (f"  {how}\n" if how else "")
        + "  Set ABC_ROOT to the experiment root, or see "
          "scripts/abc_experiment/README.md."
    )


WORKTREE = _checkout()
REPO = _main_checkout()
ROOT = _research()

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
ROOT = ROOT
FREEZE = ROOT / "frozen-sessions"
CFG = json.loads(require(
        ROOT / "real-sessions.json",
        "the real-session id list",
        "See scripts/abc_experiment/README.md: create it with the session ids "
        "you want to measure.",
    ).read_text())


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
        # Relative to the frozen-sessions directory, so the experiment root can be
        # moved without invalidating the manifest.
        manifest[name] = {"session": sid, "records": len(records),
                          "dangling_tool_calls": dangling, "orphan_results": orphans,
                          "path": path.name}
        print(f'  {name}: {len(records)} records frozen '
              f'(dropped {dangling} dangling calls, {orphans} orphan results)')
    (FREEZE / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")
    print(f"\nfrozen to {FREEZE}")


if __name__ == "__main__":
    sys.exit(main())
