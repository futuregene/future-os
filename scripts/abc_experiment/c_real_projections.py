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

"""Run the REAL C path over every measured boundary and record its projection.

The earlier C numbers came from a Python approximation of C's evidence selection
(last N tool results, fixed character clipping). C is our own code and needs no
provider, so the driver `agent/examples/abc_c_probe.rs` can call `prepare_evidence`
directly: no model calls, no cost, and the projection is exactly what production
would commit.

Outputs `<root>/Creal/projections/<identity>.json` in the same shape the harness uses,
so the existing scoring and containment analysis can read it unchanged.
"""
import json, pathlib, random, subprocess, sys, time

WT = WORKTREE
sys.path.insert(0, str(WT / "scripts"))
sys.path.insert(0, str(WT / "scripts/abc_experiment"))
from realistic_eval import load_real

ROOT = ROOT
DRIVER = REPO / "target" / "debug" / "examples" / "abc_c_probe"
MODEL = "future/deepseek-flash"
WINDOW = 32_000          # overridden by --window
SYNTH = (0, 3, 7)
REAL_FRACTIONS = (0.4, 0.7, 1.0)


def text_of(projection):
    """Flatten a driver projection the way the harness renders a context."""
    parts = []
    for item in projection:
        parts.append(item["text"])
    return "\n\n".join(parts)


def export(records, path):
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(records, ensure_ascii=False) + "\n")


def main():
    import argparse
    ap = argparse.ArgumentParser()
    ap.add_argument("--window", type=int, default=WINDOW)
    ap.add_argument("--outdir", default="Creal")
    args = ap.parse_args()
    window = args.window
    out_root = ROOT / args.outdir
    cfg = json.loads((ROOT / "real-sessions.json").read_text())
    boundaries = []
    for task in ("export", "analysis", "pipeline"):
        for stage in SYNTH:
            data = json.loads((ROOT / "data" / f"{task}-{stage}.json").read_text())
            boundaries.append((f"{task}__s{stage}", data["archive"] + data["tail"], None))
    for name, sid in cfg["chains"].items():
        records = load_real(sid)
        for index, frac in enumerate(REAL_FRACTIONS):
            covered = records[:max(1, int(len(records) * frac))]
            # The driver needs a journal entry id per record; real sessions carry a
            # journal position instead, which is stable and unique within a session.
            exported = []
            for ordinal, r in enumerate(covered):
                item = dict(r)
                item["id"] = f"{sid}-{r.get('position', ordinal)}-{ordinal}"
                exported.append(item)
            boundaries.append((f"{name}__s{index}", exported, None))

    outdir = out_root / "projections"
    outdir.mkdir(parents=True, exist_ok=True)
    for identity, records, _ in boundaries:
        target = outdir / f"{identity}.json"
        if target.exists():
            print(f"  {identity}: cached", flush=True)
            continue
        records_file = out_root / "input" / f"{identity}.json"
        export(records, records_file)
        began = time.monotonic()
        run = subprocess.run([str(DRIVER), "--records", str(records_file),
                              "--model", MODEL, "--window", str(window)],
                             capture_output=True, text=True, timeout=600)
        if run.returncode != 0:
            print(f"  {identity}: DRIVER FAILED {run.stderr[-300:]}", flush=True)
            continue
        payload = json.loads(run.stdout.strip().splitlines()[-1])
        text = text_of(payload["projection"])
        target.write_text(json.dumps({
            "id": identity, "text": text,
            "source": "real C prepare_evidence (abc_c_probe)",
            "context_tokens": len(text) // 4,
            "estimated_before": payload["estimated_before"],
            "estimated_after": payload["estimated_after"],
            "model_requests": 0,
            "wall_seconds": round(time.monotonic() - began, 3),
            "checkpoint": payload["checkpoint"],
        }, ensure_ascii=False, indent=2) + "\n")
        print(f'  {identity}: {len(text)} chars (~{len(text)//4} tok), '
              f'{len(payload["projection"])} projected messages, '
              f'checkpoint={"yes" if payload["checkpoint"] else "NO"}', flush=True)

    print(f"\nwrote {len(list(outdir.glob('*.json')))} real-C projections to {outdir}")


if __name__ == "__main__":
    main()
