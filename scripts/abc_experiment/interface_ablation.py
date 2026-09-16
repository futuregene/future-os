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

"""Compare the three retrieval interfaces, and check the call-budget question.

Our interface: entryId + byte offsets, case-insensitive substring, 8 KB paging.
Codex's:       windows + item IDs, character offsets, case-sensitive literal,
               caller-chosen max_chars_per_item / limit_chars.
OpenCode's:    glob/grep/read over the working tree — regex, path:line, line
               ranges, only the newest version of each file.

All modes share the same 5-model-request and 32 KB returned-content budget.
"""
import collections, json, pathlib

root = ROOT
MODES = [("ours", ""), ("codex", "retrievalcodex"), ("opencode", "retrievalopencode")]


def load(mode_suffix):
    rows = collections.defaultdict(list)
    if not mode_suffix:
        # The original driver only ever ran the `ours` interface.
        for item in json.loads((root / "SCORED.json").read_text()):
            if item["retrieval"]:
                rows[item["arm"]].append(item)
    for arm in ("A", "B", "C", "codex", "opencode", "main"):
        for path in sorted((root / arm / "results").glob("*.json")):
            data = json.loads(path.read_text())
            if not data.get("retrieval"):
                continue
            if arm == "opencode" and "v2" not in data["id"]:
                continue
            if mode_suffix:
                if mode_suffix not in data["id"]:
                    continue
            elif "retrievalcodex" in data["id"] or "retrievalopencode" in data["id"]:
                continue
            rows[arm].append(data)
    return rows


def _correct(item):
    return item["fact_correct"] if "fact_correct" in item else item["grade"]["correct"]


def summarize(items):
    delivered = [i for i in items if i.get("status") == "completed"]
    calls = [len(i["call_ids"]) for i in items]
    return {
        "n": len(items),
        "delivered": len(delivered),
        "correct": sum(_correct(i) for i in items),
        "correct_delivered": sum(_correct(i) for i in delivered),
        "calls_mean": round(sum(calls) / len(calls), 2) if calls else 0,
        "exhausted": sum(1 for i in items if i.get("status") == "request_limit" or i.get("status") == "incomplete"),
        "bytes": sum(i.get("retrieved_bytes") or 0 for i in items),
    }


data = {name: load(suffix) for name, suffix in MODES}
print("Three retrieval interfaces, same projections and budgets\n")
print(f'{"arm":9s} {"ours":>28s} {"codex interface":>28s} {"opencode interface":>28s}')
for arm in ("A", "B", "C", "codex", "opencode", "main"):
    cells = []
    for name, _ in MODES:
        s = summarize(data[name][arm])
        cells.append(f'{s["correct"]}/{12 * s["n"]} · {s["delivered"]}/{s["n"]} · {s["exhausted"]}x')
    print(f'{arm:9s} {cells[0]:>28s} {cells[1]:>28s} {cells[2]:>28s}')
print('\ncells = correct/possible · delivered/exhausted probes ("Nx" = probes that ran out of model requests)')

print("\nmean model requests per probe:")
print(f'{"arm":9s} {"ours":>7s} {"codex":>7s} {"opencode":>7s}')
for arm in ("A", "B", "C", "codex", "opencode", "main"):
    print(f'{arm:9s} ' + " ".join(f'{summarize(data[name][arm])["calls_mean"]:>7.2f}' for name, _ in MODES))

print("\nbyte budget consumed (of 12 x 32768 available):")
for arm in ("C", "codex"):
    for name, _ in MODES:
        s = summarize(data[name][arm])
        print(f'  {arm:8s} {name:9s} {s["bytes"]:>7d} bytes')

print("\nwhich tools each mode's model actually called:")
for name, _ in MODES:
    used = collections.Counter()
    for arm in ("A", "B", "C", "codex", "opencode", "main"):
        for item in data[name][arm]:
            for call in item.get("tool_calls") or []:
                tool = call.get("tool") or (json.loads(call["call"]["function"]["arguments"])["command"].split()[3]
                                            if "call" in call else "?")
                used[tool] += 1
    print(f'  {name:9s} {dict(used.most_common(6))}')
