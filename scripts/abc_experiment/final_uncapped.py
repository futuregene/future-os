"""Final table: capped vs uncapped, three interfaces, corrected Codex contract."""
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
ARMS = ("A", "B", "C", "codex", "opencode", "main")
INTERFACES = ("ours", "codex", "opencode")


def load():
    capped = collections.defaultdict(list)
    uncapped = collections.defaultdict(list)
    # original driver results belong to `ours` and were capped at 5
    for item in json.loads((root / "SCORED.json").read_text()):
        if item["retrieval"]:
            capped[(item["arm"], "ours")].append(
                {"correct": item["fact_correct"], "delivered": item["delivered"],
                 "calls": len(item["call_ids"])})
    for arm in ARMS:
        for path in sorted((root / arm / "results").glob("*.json")):
            d = json.loads(path.read_text())
            if not d.get("retrieval"):
                continue
            if arm == "opencode" and "v2" not in d["id"]:
                continue
            suffix = ("codex" if "retrievalcodex" in d["id"] else
                      "opencode" if "retrievalopencode" in d["id"] else "ours")
            row = {"correct": d["grade"]["correct"], "delivered": d["status"] == "completed",
                   "calls": len(d["call_ids"])}
            (uncapped if "__uncapped" in d["id"] else capped)[(arm, suffix)].append(row)
    return capped, uncapped


def line(rows):
    if not rows:
        return "—"
    correct = sum(r["correct"] for r in rows)
    lost = sum(1 for r in rows if not r["delivered"])
    calls = sum(r["calls"] for r in rows) / len(rows)
    return f'{correct}/{12*len(rows)} · {lost} lost · {calls:.1f} req'


capped, uncapped = load()
print("CAPPED AT 5 MODEL REQUESTS (the earlier run)")
print(f'{"arm":9s} ' + " ".join(f'{n:>26s}' for n in INTERFACES))
for arm in ARMS:
    print(f'{arm:9s} ' + " ".join(f'{line(capped[(arm, s)]):>26s}' for s in INTERFACES))

print("\nNO CAP, corrected Codex contract (cursor + selectable snippet placement)")
print(f'{"arm":9s} ' + " ".join(f'{n:>26s}' for n in INTERFACES))
for arm in ARMS:
    print(f'{arm:9s} ' + " ".join(f'{line(uncapped[(arm, s)]):>26s}' for s in INTERFACES))

print("\nmean model requests per probe, uncapped")
print(f'{"arm":9s} ' + " ".join(f'{n:>12s}' for n in INTERFACES))
for arm in ARMS:
    cells = []
    for s in INTERFACES:
        rows = uncapped[(arm, s)]
        cells.append(f'{sum(r["calls"] for r in rows)/len(rows):.1f}' if rows else "—")
    print(f'{arm:9s} ' + " ".join(f'{c:>12s}' for c in cells))

total = 0.0
for name in ("calls.json", "cache-test-calls.json", "cachebig-calls.json", "shape-test-calls.json",
             "shape-prime-calls.json", "uncapped-test-calls.json"):
    p = root / name
    if p.exists():
        total += sum(r.get("charged", r["reserved"]) for r in json.loads(p.read_text()))
print(f'\nrecorded spend CNY {total:.4f}')
