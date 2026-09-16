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

"""Why is C only slightly ahead, if it keeps far more assistant text?

Decomposes the exam by the source of each value (assistant prose / user turn / tool
result) and checks containment in each strategy's projection. No model calls: the
question "could this strategy possibly have answered from what it kept" is measurable
without asking a model.

If the exam contains few assistant-sourced items, the comparison cannot detect C's
advantage no matter how much assistant text it retains.
"""
import collections, json, pathlib, random, sys

WT = WORKTREE
sys.path.insert(0, str(WT / "scripts"))
sys.path.insert(0, str(WT / "scripts/abc_experiment"))
import realistic_exam as exam

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
    manifest = json.loads((FROZEN / "manifest.json").read_text())
    for name, meta in manifest.items():
        records = json.loads(pathlib.Path(meta["path"]).read_text())["records"]
        for index, frac in enumerate((0.4, 0.7, 1.0)):
            out[f"{name}__s{index}"] = records[:max(1, int(len(records) * frac))]
    return out


def source_of(value, covered):
    """Which record kind holds this value, preferring the earliest occurrence."""
    for r in covered:
        if value in (r.get("text") or ""):
            if r["kind"] == "text":
                return f"{r['role']} text"
            return "tool result"
    return "?"


sets = covered_sets()
by_source = collections.Counter()
contained = collections.defaultdict(collections.Counter)
detail = []

for identity, covered in sets.items():
    stage_index = int(identity.rsplit("__s", 1)[1])
    present, _ = exam.build_exam(covered, len(covered), random.Random(9000 + stage_index))
    proj = {}
    for arm, path in (
        ("C", ROOT / "C3proj" / "projections" / f"{identity}.json"),
        ("Codex", ROOT / "ExternalProj" / "projections" / f"codex__{identity}.json"),
        ("OpenCode", ROOT / "ExternalProj" / "projections" / f"opencode__{identity}.json"),
    ):
        proj[arm] = json.loads(path.read_text())["text"] if path.exists() else ""
    for value in present:
        src = source_of(value, covered)
        by_source[src] += 1
        for arm, text in proj.items():
            contained[(arm, src)][value in text and 1 or 0] += 1
        detail.append({"identity": identity, "value": value, "source": src,
                       "in_C": value in proj["C"],
                       "in_codex": value in proj["Codex"],
                       "in_opencode": value in proj["OpenCode"]})

print("Where the exam's values come from, and whether each projection contains them\n")
print(f'{"source":16s} {"items":>6s} {"in C":>8s} {"in Codex":>9s} {"in OpenCode":>12s}')
for src in ("assistant text", "user text", "tool result"):
    n = by_source[src]
    if not n:
        continue
    c_c = contained[("C", src)][1]
    c_x = contained[("Codex", src)][1]
    c_o = contained[("OpenCode", src)][1]
    print(f'{src:16s} {n:>6d} {c_c:>4d}/{n:<3d} {c_x:>5d}/{n:<3d} {c_o:>8d}/{n:<3d}')

total = sum(by_source.values())
print(f'\n{"TOTAL":16s} {total:>6d} '
      f'{sum(contained[("C", s)][1] for s in by_source):>4d}    '
      f'{sum(contained[("Codex", s)][1] for s in by_source):>5d}    '
      f'{sum(contained[("OpenCode", s)][1] for s in by_source):>8d}')

print("\nshare of the exam by source")
for src, n in by_source.most_common():
    print(f'  {src:16s} {n:>4d}  {100*n/total:5.1f}%')

# The decisive number: how many items separate C from Codex at all?
only_c = sum(1 for d in detail if d["in_C"] and not d["in_codex"])
only_codex = sum(1 for d in detail if d["in_codex"] and not d["in_C"])
print(f'\nvalues only C holds: {only_c}')
print(f'values only Codex holds: {only_codex}')
print(f'values neither holds: {sum(1 for d in detail if not d["in_C"] and not d["in_codex"])}')

out = ROOT / "exam-by-source.json"
out.write_text(json.dumps(detail, ensure_ascii=False, indent=2) + "\n")
print(f"\nwrote {out}")
