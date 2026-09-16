"""Definitive comparison: every C variant vs Codex, same interface, uncapped."""
import collections, json, pathlib

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

root = ROOT
FIELDS = ["project", "first_limit", "latest_limit", "format", "first_code", "old_version",
          "latest_version", "buried", "validation", "blocker", "deployment", "device"]
SUFFIXABLE = ("A", "B", "C", "codex", "opencode", "main", "Cplus", "Clive")


def rows_for(arm):
    out = []
    for p in sorted((root / arm / "results").glob("*retrievalcodex__uncapped*.json")):
        d = json.loads(p.read_text())
        marks = (d.get("grade") or {}).get("marks") or {}
        out.append({"correct": d["grade"]["correct"] if marks else 0, "marks": marks,
                    "status": d["status"], "calls": len(d["call_ids"]), "id": d["id"]})
    return out


print(f'{"arm":28s} {"score":>11s} {"no ans":>7s} {"req/probe":>10s}   summary input')
LABEL = {
    "C": ("C: evidence, no summary", "(none)"),
    "Cplus": ("C + summary (clipped input)", "clipped / protected material"),
    "Clive": ("C + summary (LIVE input)", "prior projection + new records"),
    "codex": ("Codex: user msgs + summary", "whole live history"),
    "A": ("A: originals + summary", "clipped material"),
    "B": ("B: no summary", "(none)"),
    "main": ("M (origin/main)", "clipped material"),
}
for arm in ("codex", "C", "Cplus", "Clive", "A", "B", "main"):
    items = rows_for(arm)
    if not items:
        continue
    total = sum(i["correct"] for i in items)
    blank = sum(1 for i in items if not i["marks"])
    calls = sum(i["calls"] for i in items) / len(items)
    name, source = LABEL.get(arm, (arm, "?"))
    print(f'{name:28s} {total:>4d}/{12*len(items):<6d} {blank:>7d} {calls:>10.1f}   {source}')

print("\nper-field, C variants vs Codex:")
print(f'{"field":18s} {"codex":>7s} {"C":>7s} {"C+clip":>7s} {"C+live":>7s}')
for f in FIELDS:
    cells = []
    for arm in ("codex", "C", "Cplus", "Clive"):
        items = rows_for(arm)
        n = sum(1 for i in items if i["marks"].get(f))
        cells.append(f'{n}/{len(items)}')
    print(f'{f:18s} ' + " ".join(f'{c:>7s}' for c in cells))

print("\nC+live: requests per probe (the summaries cost rounds)")
for item in sorted(rows_for("Clive"), key=lambda r: r["id"]):
    print(f'  {item["id"][:50]:50s} {item["status"]:12s} {item["correct"]:2d}/12 calls={item["calls"]}')

total = 0.0
for p in sorted(root.glob("*calls*.json")):
    try:
        rows = json.loads(p.read_text())
    except Exception:
        continue
    if isinstance(rows, list):
        total += sum(r.get("charged", r["reserved"]) for r in rows if isinstance(r, dict))
print(f'\nrecorded spend CNY {total:.4f}')
