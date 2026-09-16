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

"""Does the summary carry semantic content the deterministic parts lack?

The value-recall exam showed the summary is a strict subset for exact strings. That is
expected, and it does not settle the question the summary is meant to answer: objectives,
decisions, reasons, and corrections. Those are free-form, not identifier-shaped, so they
need their own material to test against.

The synthetic fixtures carry labelled semantic ground truth (`gold`, `assistant_gold`),
so this checks containment for that content with no model calls: where does each semantic
fact live in a C projection — protected originals, the summary, or the evidence index?
"""
import json, pathlib, sys, collections

ROOT = ROOT
PROJ = ROOT / "C3proj" / "projections"
DET = ROOT / "Cdet" / "projections"

SYNTH = [("pipeline", 0), ("pipeline", 3), ("pipeline", 7),
         ("export", 0), ("export", 3), ("export", 7),
         ("analysis", 0), ("analysis", 3), ("analysis", 7)]


def parts(identity):
    """(originals, evidence, summary) for one projection."""
    payload = json.loads((PROJ / f"{identity}.json").read_text())
    slot = (payload.get("checkpoint") or {}).get("summary") or []
    slot_text = slot[0].get("text", "") if slot else ""
    mark = "Model handoff summary"
    if mark in slot_text:
        summary = slot_text[slot_text.find(mark):]
        evidence = slot_text[:slot_text.find(mark)]
    else:
        summary, evidence = "", slot_text
    full = payload["text"]
    originals = full[:full.find(slot_text)] if slot_text and slot_text in full else full
    return originals, evidence, summary


print("Where each labelled semantic fact lives in a C projection\n")
rows = []
for task, stage in SYNTH:
    fixture = json.loads((ROOT / "data" / f"{task}-{stage}.json").read_text())
    identity = f"{task}__s{stage}"
    originals, evidence, summary = parts(identity)
    for key in ("gold", "assistant_gold"):
        entry = fixture.get(key)
        if not entry:
            continue
        # The gold is a mapping of field -> value; the value is what a projection must
        # be able to reproduce, not the serialized key/value pair.
        items = entry.items() if isinstance(entry, dict) else enumerate(entry)
        for field, value in items:
            text = str(value)
            rows.append({
                "identity": identity, "key": f"{key}.{field}", "fact": text[:60],
                "in_originals": text in originals,
                "in_summary": text in summary,
                "in_evidence": text in evidence,
            })

total = len(rows)
print(f'{"fact (truncated)":62s} {"originals":>10s} {"summary":>8s} {"evidence":>9s}')
for r in rows[:24]:
    print(f'{r["fact"]:62s} {str(r["in_originals"]):>10s} {str(r["in_summary"]):>8s} '
          f'{str(r["in_evidence"]):>9s}')

print(f'\nlabelled semantic facts: {total}')
for field in ("in_originals", "in_summary", "in_evidence"):
    n = sum(1 for r in rows if r[field])
    print(f'  in {field:14s} {n:>3d}/{total}  ({100*n/max(total,1):.0f}%)')
added = [r for r in rows if r["in_summary"] and not r["in_originals"]]
print(f'\nfacts the summary adds that the originals lack: {len(added)}')
for r in added[:8]:
    print(f'  {r["identity"]:14s} {r["fact"][:56]!r}')

out = ROOT / "semantic-containment.json"
out.write_text(json.dumps(rows, ensure_ascii=False, indent=2) + "\n")
print(f"\nwrote {out}")
