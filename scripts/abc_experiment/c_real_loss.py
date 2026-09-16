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

"""Classify what real C drops, at a realistic context window.

The window sweep shows containment saturates at 128K (102/129) and that 32K is far
worse (84/129), so this classifies the losses at the realistic setting. For each exam
value the real C projection lacks: its shape, the kind of record that holds it, and
whether that record was kept, excerpted, or dropped outright.
"""
import collections, json, pathlib, random, re, subprocess, sys

WT = WORKTREE
sys.path.insert(0, str(WT / "scripts"))
sys.path.insert(0, str(WT / "scripts/abc_experiment"))
import realistic_exam as exam
from realistic_eval import load_real

ROOT = ROOT
DRIVER = REPO / "target" / "debug" / "examples" / "abc_c_probe"
WINDOW = 128_000
cfg = json.loads(require(
        ROOT / "real-sessions.json",
        "the real-session id list",
        "See scripts/abc_experiment/README.md: create it with the session ids "
        "you want to measure.",
    ).read_text())

HEXISH = re.compile(r"^[0-9a-f]{6,40}$", re.I)
SIZEISH = re.compile(r"^\d+(?:\.\d+)?\s?(?:MiB|MB|GB|GiB|KB|B)$", re.I)
COUNTISH = re.compile(r"^[\d,]+\s?(?:项|个|条|套件|次|tests?)$", re.I)


def kind_of(value):
    if HEXISH.match(value):
        return "hex hash / id"
    if SIZEISH.match(value):
        return "size"
    if COUNTISH.match(value):
        return "count with unit"
    if value.isdigit():
        return "bare number"
    return "other"


def run(records_file):
    out = subprocess.run([str(DRIVER), "--records", str(records_file),
                          "--model", "future/deepseek-flash", "--window", str(WINDOW)],
                         capture_output=True, text=True, timeout=900)
    return json.loads(out.stdout.strip().splitlines()[-1]) if out.returncode == 0 else None


shapes = collections.Counter()
origins = collections.Counter()
detail = []
for name, sid in cfg["chains"].items():
    records = load_real(sid)
    for index, frac in enumerate((0.4, 0.7, 1.0)):
        covered = records[:max(1, int(len(records) * frac))]
        identity = f"{name}__s{index}"
        rf = ROOT / "Creal" / "input" / f"{identity}.json"
        payload = run(rf)
        if payload is None:
            continue
        proj = payload["projection"]
        text = "\n\n".join(m["text"] for m in proj)
        kept_ids = {e for m in proj for e in (m.get("sourceEntryIds") or [])}

        present, _ = exam.build_exam(covered, len(covered), random.Random(9000 + index))
        for value in present:
            if value in text:
                continue
            src = None
            for i, r in enumerate(covered):
                if value in (r.get("text") or ""):
                    src = (i, r)
                    break
            if src is None:
                origins["NOT FOUND IN SESSION"] += 1
                continue
            i, r = src
            entry_id = f"{sid}-{r.get('position', i)}-{i}"
            kept = entry_id in kept_ids
            record_kind = f'{r["kind"]}/{r["role"]}'
            shapes[kind_of(value)] += 1
            if kept:
                origins[f"kept record ({record_kind}) but value clipped or not selected"] += 1
            else:
                origins[f"record dropped ({record_kind})"] += 1
            detail.append((identity, value, kind_of(value), record_kind, kept))

print(f"real C at window={WINDOW:,}\n")
print("dropped values by shape:")
for k, n in shapes.most_common():
    print(f'  {k:18s} {n:>3d}')
print("\nwhy they are missing:")
for k, n in origins.most_common():
    print(f'  {n:>3d}  {k}')
print("\nsample:")
for identity, value, kind, record_kind, kept in detail[:14]:
    print(f'  {identity:16s} {value[:26]:26s} {kind:16s} {record_kind:18s} kept={kept}')
