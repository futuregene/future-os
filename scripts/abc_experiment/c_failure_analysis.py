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

"""Characterise the values C's projection drops, and why.

Every missed exam item exists in the raw archive, so the loss is in selection, not
availability. This classifies the dropped values by shape and locates them in the
source records, to identify what C's evidence rule is failing to keep.
"""
import json, pathlib, random, re, sys, collections

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent.parent))
sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
import realistic_exam as exam
from realistic_eval import load_real
import abc_cache_shapes as shapes

ROOT = ROOT
PROTECTED_CHARS = 64_000 * 4
REAL_FRACTIONS = (0.4, 0.7, 1.0)


def clip(text, head, tail):
    if len(text) <= head + tail:
        return text
    return text[:head] + f"\n[... {len(text) - head - tail} characters omitted ...]\n" + text[-tail:]


def render_c(covered):
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


HEXISH = re.compile(r"^[0-9a-f]{6,40}$", re.I)
SIZEISH = re.compile(r"^\d+(?:\.\d+)?\s?(?:MiB|MB|GB|GiB|KB|B)$", re.I)
COUNTISH = re.compile(r"^[\d,]+\s?(?:项|个|条|套件|次|tests?)$", re.I)

cfg = json.loads(require(
        ROOT / "real-sessions.json",
        "the real-session id list",
        "See scripts/abc_experiment/README.md: create it with the session ids "
        "you want to measure.",
    ).read_text())
kinds = collections.Counter()
locations = collections.Counter()
detail = []
for name, sid in cfg["chains"].items():
    records = load_real(sid)
    for stage_index, frac in enumerate(REAL_FRACTIONS):
        covered = records[:max(1, int(len(records) * frac))]
        projection = render_c(covered)
        present, _ = exam.build_exam(covered, len(covered), random.Random(9000 + stage_index))
        dropped = [v for v in present if v not in projection]
        for value in dropped:
            if HEXISH.match(value):
                kind = "hex hash / id"
            elif SIZEISH.match(value):
                kind = "size"
            elif COUNTISH.match(value):
                kind = "count"
            elif value.isdigit():
                kind = "bare number"
            else:
                kind = "other"
            kinds[kind] += 1
            # where does it live in the source records?
            for i, r in enumerate(covered):
                if value in (r.get("text") or ""):
                    if r["kind"] == "tool_result":
                        age = len(covered) - i
                        bucket = ("among the last 6 tool results (kept, head/tail clipped)"
                                  if age <= 6 else
                                  "in an OLDER tool result (dropped entirely)")
                    elif r["kind"] == "text":
                        bucket = f"in a {r['role']} message"
                    else:
                        bucket = "elsewhere"
                    locations[bucket] += 1
                    detail.append((name, stage_index + 1, value, kind, bucket))
                    break

print("dropped values by kind:")
for kind, n in kinds.most_common():
    print(f'  {kind:16s} {n:>3d}')

print("\nwhere they live in the source:")
for loc, n in locations.most_common():
    print(f'  {n:>3d}  {loc}')

print("\nsample:")
for name, stage, value, kind, loc in detail[:12]:
    print(f'  {name} s{stage}: {value!r:24s} {kind:14s} {loc}')
