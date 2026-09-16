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

"""Does the summary carry anything the rest of the projection lacks?

The ablation shows C-with-summary and C-without-summary score identically. This tests the
mechanism: for every exam value the deterministic projection does not contain, is it
present in the summary?

If the answer is "no", the summary is a lossy third copy of material that is already
retained verbatim, and can add nothing on a question set that asks about exact values.
"""
import json, pathlib, random, sys, collections

WT = WORKTREE
sys.path.insert(0, str(WT / "scripts"))
sys.path.insert(0, str(WT / "scripts/abc_experiment"))
import realistic_exam as exam
import abc_cache_shapes as shapes

ROOT = ROOT
FROZEN = ROOT / "frozen-sessions"


def covered_sets():
    out = {}
    for task in ("export", "analysis", "pipeline"):
        for stage in (0, 3, 7):
            p = ROOT / "data" / f"{task}-{stage}.json"
            if p.exists():
                d = json.loads(p.read_text())
                out[f"{task}__s{stage}"] = d["archive"] + d["tail"]
    manifest = json.loads(require(FROZEN / "manifest.json", "frozen real sessions",
                                "Run freeze_sessions.py against your own Agent database first.").read_text())
    for name, meta in manifest.items():
        records = json.loads((FROZEN / meta["path"]).read_text())["records"]
        for index, frac in enumerate((0.4, 0.7, 1.0)):
            out[f"{name}__s{index}"] = records[:max(1, int(len(records) * frac))]
    return out


def summary_of(identity):
    """The model-written half of the slot, taken from the committed checkpoint."""
    payload = json.loads((ROOT / "C3proj" / "projections" / f"{identity}.json").read_text())
    slot = (payload.get("checkpoint") or {}).get("summary") or []
    if not slot:
        return ""
    text = slot[0].get("text", "")
    mark = "Model handoff summary"
    return text[text.find(mark):] if mark in text else ""


rows = []
for identity, covered in covered_sets().items():
    stage_index = int(identity.rsplit("__s", 1)[1])
    present, _ = exam.build_exam(covered, len(covered), random.Random(9000 + stage_index))
    det = json.loads((ROOT / "Cdet" / "projections" / f"{identity}.json").read_text())["text"]
    summary = summary_of(identity)
    for value in present:
        rows.append({"identity": identity, "value": value,
                     "in_deterministic": value in det,
                     "in_summary": value in summary})

total = len(rows)
in_det = sum(1 for r in rows if r["in_deterministic"])
in_sum = sum(1 for r in rows if r["in_summary"])
print(f"exam values                                    {total}")
print(f"  present in the deterministic projection      {in_det}  ({100*in_det/total:.1f}%)")
print(f"  present in the summary                       {in_sum}  ({100*in_sum/total:.1f}%)")
print(f"  present in either                            "
      f"{sum(1 for r in rows if r['in_deterministic'] or r['in_summary'])}")

only_sum = [r for r in rows if r["in_summary"] and not r["in_deterministic"]]
print(f"\nvalues the summary adds that the rest lacks: {len(only_sum)}")
for r in only_sum[:12]:
    print(f'  {r["identity"]:18s} {r["value"][:44]!r}')

# Values the whole C3 projection lacks, split by whether the summary could have helped
c3_missing = [r for r in rows if not r["in_deterministic"] and not r["in_summary"]]
print(f'\nvalues neither part contains: {len(c3_missing)}')
by_chain = collections.Counter(r["identity"].rsplit("__s", 1)[0] for r in c3_missing)
for chain, n in by_chain.most_common():
    print(f'  {chain:14s} {n}')

out = ROOT / "summary-value-check.json"
out.write_text(json.dumps(rows, ensure_ascii=False, indent=2) + "\n")
print(f"\nwrote {out}")
