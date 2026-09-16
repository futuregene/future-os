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

"""Token composition of a C3 projection, split into its four parts.

The flattened projection is `protected originals + [compaction slot] + retained tail`,
and the slot text is stored verbatim in the checkpoint, so the split is exact and no
model call is needed. The slot itself is then split into the deterministic evidence
index and the model-written summary.

The wrapper is `[Context compaction: <slot>]`, so a few tokens belong to the wrapper
rather than to either half; they are reported as `slot overhead`.
"""
import collections, json, pathlib

ROOT = ROOT
PROJ = ROOT / "C3proj" / "projections"

EVIDENCE_MARK = "Deterministic C evidence index"
SUMMARY_MARK = "Model handoff summary"


def tok(text):
    return len(text) // 4


rows = []
for path in sorted(PROJ.glob("*.json")):
    d = json.loads(path.read_text())
    text = d["text"]
    checkpoint = d.get("checkpoint") or {}
    slot_blocks = checkpoint.get("summary") or []
    if not slot_blocks:
        continue
    slot = slot_blocks[0].get("text", "")
    at = text.find(slot)
    if at < 0:
        continue
    originals = text[:at]
    tail = text[at + len(slot):]

    if SUMMARY_MARK in slot:
        cut = slot.find(SUMMARY_MARK)
        evidence, summary = slot[:cut], slot[cut:]
    elif EVIDENCE_MARK in slot:
        evidence, summary = slot, ""
    else:
        evidence, summary = "", slot

    rows.append({
        "id": path.stem,
        "chain": d["chain"],
        "total": d["context_tokens"],
        "originals": tok(originals),
        "evidence": tok(evidence),
        "summary": tok(summary),
        "tail": tok(tail),
        "slot_overhead": max(0, tok(slot) - tok(evidence) - tok(summary)),
    })

print(f'{"projection":18s} {"total":>6s} {"orig":>6s} {"evid":>6s} {"summ":>6s} {"tail":>6s}')
for r in rows:
    print(f'{r["id"]:18s} {r["total"]:>6d} {r["originals"]:>6d} {r["evidence"]:>6d} '
          f'{r["summary"]:>6d} {r["tail"]:>6d}')

keys = ("total", "originals", "evidence", "summary", "tail")
agg = {k: sum(r[k] for r in rows) for k in keys}
n = len(rows)
print(f'\n{"mean":18s} ' + " ".join(f'{agg[k]//n:>6d}' for k in keys))
print(f'{"share of total":18s} ' +
      " ".join(f'{100*agg[k]/agg["total"]:>5.1f}%' for k in keys))

# same split, separately for synthetic and real chains
for label, pred in (("synthetic", lambda r: not r["chain"].startswith("real-")),
                    ("real", lambda r: r["chain"].startswith("real-"))):
    sel = [r for r in rows if pred(r)]
    if not sel:
        continue
    a = {k: sum(r[k] for r in sel) // len(sel) for k in keys}
    print(f'\n{label} (n={len(sel)}): total {a["total"]}, originals {a["originals"]}, '
          f'evidence {a["evidence"]}, summary {a["summary"]}, tail {a["tail"]}')
    print(f'  shares: originals {100*a["originals"]/a["total"]:.1f}%, '
          f'evidence {100*a["evidence"]/a["total"]:.1f}%, '
          f'summary {100*a["summary"]/a["total"]:.1f}%, '
          f'tail {100*a["tail"]/a["total"]:.1f}%')

out = ROOT / "c3-composition.json"
out.write_text(json.dumps(rows, indent=2) + "\n")
print(f"\nwrote {out}")
