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

"""Does the real C selector lose the values the approximation lost?

Compares, for every measured boundary and with no model call:

  approximation : the Python `render_c` the earlier numbers were produced with
                  (last 6 tool results, 380+100 char head/tail clip)
  real C        : the projection `abc_c_probe` obtained from `prepare_evidence`
                  in this checkout

and for each, whether the exam's values survive. The exam items are rebuilt with the
same seeds, so the two runs are asked about exactly the same values.
"""
import json, pathlib, random, sys

WT = WORKTREE
sys.path.insert(0, str(WT / "scripts"))
sys.path.insert(0, str(WT / "scripts/abc_experiment"))
import realistic_exam as exam
import abc_cache_shapes as shapes
from realistic_eval import load_real

ROOT = ROOT
SYNTH = (0, 3, 7)
REAL_FRACTIONS = (0.4, 0.7, 1.0)
PROTECTED_CHARS = 64_000 * 4


def clip(text, head, tail):
    if len(text) <= head + tail:
        return text
    return text[:head] + f"\n[... {len(text) - head - tail} characters omitted ...]\n" + text[-tail:]


def render_c(covered):
    """The approximation the earlier C numbers came from."""
    kept = [r for r in covered if r["kind"] == "text"]
    tools = [r for r in covered if r["kind"] == "tool_result"][-6:]
    tail = covered[-24:]
    blocks = "\n\n".join(clip(shapes._plain([r]), 380, 100) for r in tools)
    originals = shapes._plain(kept)
    if len(originals) > PROTECTED_CHARS:
        originals = clip(originals, PROTECTED_CHARS // 2, PROTECTED_CHARS // 2)
    return ("<archived-conversation>\n<protected-originals>\n" + originals
            + "\n</protected-originals>\n\n<deterministic-evidence>\n" + blocks
            + "\n</deterministic-evidence>\n\n<recent-history>\n"
            + clip(shapes._plain(tail), 8000, 2000) + "\n</recent-history>\n</archived-conversation>")


def boundaries():
    cfg = json.loads((ROOT / "real-sessions.json").read_text())
    out = []
    for task in ("export", "analysis", "pipeline"):
        for stage in SYNTH:
            data = json.loads((ROOT / "data" / f"{task}-{stage}.json").read_text())
            out.append((f"{task}__s{stage}", data["archive"] + data["tail"]))
    for name, sid in cfg["chains"].items():
        records = load_real(sid)
        for index, frac in enumerate(REAL_FRACTIONS):
            out.append((f"{name}__s{index}", records[:max(1, int(len(records) * frac))]))
    return out


print(f'{"boundary":18s} {"items":>6s} {"approx":>8s} {"real C":>8s} {"gain":>6s} '
      f'{"approx tok":>11s} {"real tok":>9s}')
rows = []
for identity, covered in boundaries():
    real_path = ROOT / "Creal" / "projections" / f"{identity}.json"
    if not real_path.exists():
        continue
    real = json.loads(real_path.read_text())
    approx_text = render_c(covered)
    real_text = real["text"]

    # same seeds as the experiments
    stage_index = int(identity.split("__s")[1])
    present, _ = exam.build_exam(covered, len(covered), random.Random(9000 + stage_index))
    approx_in = sum(1 for v in present if v in approx_text)
    real_in = sum(1 for v in present if v in real_text)
    rows.append({"identity": identity, "items": len(present),
                 "approx": approx_in, "real": real_in,
                 "approx_tokens": len(approx_text) // 4,
                 "real_tokens": len(real_text) // 4})
    print(f'{identity:18s} {len(present):>6d} {approx_in:>8d} {real_in:>8d} '
          f'{real_in-approx_in:>+6d} {len(approx_text)//4:>11d} {len(real_text)//4:>9d}')

total_items = sum(r["items"] for r in rows)
total_approx = sum(r["approx"] for r in rows)
total_real = sum(r["real"] for r in rows)
print(f'\n{"TOTAL":18s} {total_items:>6d} {total_approx:>8d} {total_real:>8d} '
      f'{total_real-total_approx:>+6d}')
print(f'\n  approximation misses: {total_items - total_approx}')
print(f'  real C misses:        {total_items - total_real}')
print(f'  the earlier 20% gap was an artefact of: {total_real-total_approx:+d} values')

out = ROOT / "c-real-vs-approx.json"
out.write_text(json.dumps(rows, indent=2) + "\n")
print(f"\nwrote {out}")
