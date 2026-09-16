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

"""Does the real C gap depend on the context window?

The projection at window=32000 keeps only 132 of 1107 records, dropping middle
assistant messages entirely -- which is why the earlier Python approximation (which
allowed 256K characters of originals) measured a much smaller gap. Production uses the
model's real window, so this sweeps several windows and measures how many exam values
survive. No model calls.
"""
import json, pathlib, random, subprocess, sys, collections

WT = WORKTREE
sys.path.insert(0, str(WT / "scripts"))
sys.path.insert(0, str(WT / "scripts/abc_experiment"))
import realistic_exam as exam
from realistic_eval import load_real

ROOT = ROOT
DRIVER = REPO / "target" / "debug" / "examples" / "abc_c_probe"
WINDOWS = [32_000, 128_000, 256_000, 1_000_000]
cfg = json.loads((ROOT / "real-sessions.json").read_text())


def run(identity, records_file, window):
    out = subprocess.run([str(DRIVER), "--records", str(records_file),
                          "--model", "future/deepseek-flash", "--window", str(window)],
                         capture_output=True, text=True, timeout=900)
    if out.returncode != 0:
        return None
    return json.loads(out.stdout.strip().splitlines()[-1])


boundaries = []
for name, sid in cfg["chains"].items():
    records = load_real(sid)
    for index, frac in enumerate((0.4, 0.7, 1.0)):
        covered = records[:max(1, int(len(records) * frac))]
        exported = []
        for ordinal, r in enumerate(covered):
            item = dict(r)
            item["id"] = f"{sid}-{r.get('position', ordinal)}-{ordinal}"
            exported.append(item)
        rf = ROOT / "Creal" / "input" / f"{name}__s{index}.json"
        rf.parent.mkdir(parents=True, exist_ok=True)
        rf.write_text(json.dumps(exported, ensure_ascii=False) + "\n")
        boundaries.append((f"{name}__s{index}", covered, rf))

print(f'{"window":>9s} ' + " ".join(f'{b[0]:>16s}' for b in boundaries) + f'{"TOTAL":>10s}')
totals = {}
for window in WINDOWS:
    cells, got, want = [], 0, 0
    for identity, covered, rf in boundaries:
        payload = run(identity, rf, window)
        if payload is None:
            cells.append("driver error")
            continue
        text = "\n\n".join(m["text"] for m in payload["projection"])
        stage_index = int(identity.split("__s")[1])
        present, _ = exam.build_exam(covered, len(covered), random.Random(9000 + stage_index))
        hits = sum(1 for v in present if v in text)
        got += hits
        want += len(present)
        cells.append(f'{hits}/{len(present)} ({len(payload["projection"])}m)')
    totals[window] = (got, want)
    print(f'{window:>9d} ' + " ".join(f'{c:>16s}' for c in cells) + f'{got:>4d}/{want:<5d}')

print("\nsummary")
for window, (got, want) in totals.items():
    print(f'  window {window:>9,d}: {got}/{want} = {100*got/max(want,1):.0f}% of exam values present')
