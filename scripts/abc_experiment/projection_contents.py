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

"""What does the projection actually contain: assistant prose only, or also the
assistant's tool-call arguments?

This decides whether a tool-result-only search is safe. If the projection holds every
assistant *text* block but not the tool-call arguments, then restricting search to tool
results would lose the file paths and commands that live in the calls.
"""
import json, pathlib, re, collections

ROOT = ROOT
FROZEN = ROOT / "frozen-sessions"
PROJ = ROOT / "C3proj" / "projections"

manifest = json.loads((FROZEN / "manifest.json").read_text())
manifest["synthetic"] = None  # fixtures handled separately below

print(f'{"projection":18s} {"text blocks":>12s} {"in proj":>9s} '
      f'{"tool-call paths":>16s} {"in proj":>9s}')
for path in sorted(PROJ.glob("*.json")):
    identity = path.stem
    chain = identity.rsplit("__s", 1)[0]
    stage = int(identity.rsplit("__s", 1)[1])
    if chain in manifest and manifest[chain]:
        records = json.loads(pathlib.Path(manifest[chain]["path"]).read_text())["records"]
        records = records[:max(1, int(len(records) * (0.4, 0.7, 1.0)[stage]))]
    else:
        # synthetic fixture: the records live in the driver's input file
        inp = ROOT / "C3proj" / "input" / f"{identity}.json"
        records = json.loads(inp.read_text())
    text_blocks = [r for r in records if r["kind"] == "text"]
    call_paths = [r for r in records if r["kind"] == "tool_call" and r.get("path")]

    proj = json.loads(path.read_text())["text"]
    text_in = sum(1 for r in text_blocks
                  if (r.get("text") or "").strip()[:120] in proj)
    paths_in = sum(1 for r in call_paths if r.get("path") and r["path"] in proj)
    print(f'{identity:18s} {len(text_blocks):>12d} {text_in:>9d} '
          f'{len(call_paths):>16d} {paths_in:>9d}')

print("\nWhat the projection's own rendering includes:")
sample = json.loads((PROJ / "real-yt__s1.json").read_text())["text"]
print(f'  "[Assistant tool call" occurrences in a real projection: '
      f'{sample.count("[Assistant tool call")}')
print(f'  "[Tool result" occurrences: {sample.count("[Tool result")}')
print(f'  "[Assistant]"/"[User]" prose markers: {sample.count("[Assistant]:") + sample.count("[User]:")}')
