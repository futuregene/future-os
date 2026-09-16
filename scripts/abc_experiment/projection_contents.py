"""What does the projection actually contain: assistant prose only, or also the
assistant's tool-call arguments?

This decides whether a tool-result-only search is safe. If the projection holds every
assistant *text* block but not the tool-call arguments, then restricting search to tool
results would lose the file paths and commands that live in the calls.
"""
import json, pathlib, re, collections

ROOT = pathlib.Path("/Users/geilige/future-os/.future/research/abc-summary-a47313")
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
