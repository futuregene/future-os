"""Is C's loss a projection problem or a model problem?

Rebuilds the exact exam items (same seeds) and checks, without any model call,
whether each item is present in C's projection. That separates two very different
failures:

  not in projection  -> the retention rule dropped it; a projection fix could help
  in projection      -> the model failed to confirm a value it was shown; a prompt or
                        exam-format effect, not a retention limit
"""
import json, pathlib, random, sys

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent.parent))
sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
import realistic_exam as exam
from realistic_eval import load_real, real_to_messages, sanitize_messages
import abc_cache_shapes as shapes

ROOT = pathlib.Path("/Users/geilige/future-os/.future/research/abc-summary-a47313")
SYNTH_STAGES = (0, 3, 7)
REAL_FRACTIONS = (0.4, 0.7, 1.0)
PROTECTED_CHARS = 64_000 * 4


def clip(text, head, tail):
    if len(text) <= head + tail:
        return text
    return text[:head] + f"\n[... {len(text) - head - tail} characters omitted ...]\n" + text[-tail:]


def render_c(covered):
    """The same projection the closed-book run scored."""
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


cfg = json.loads((ROOT / "real-sessions.json").read_text())
chains = {}
for name, sid in cfg["chains"].items():
    records = load_real(sid)
    chains[name] = [records[:max(1, int(len(records) * f))] for f in REAL_FRACTIONS]

print("For each exam item: is it present in C's projection, and in the raw archive?\n")
print(f'{"chain":13s} {"stage":>5s} {"items":>6s} {"in proj":>8s} {"in archive":>11s} '
      f'{"proj-absent":>12s}')
totals = {"items": 0, "in_proj": 0, "in_archive": 0, "absent": 0}
examples = []
for name, stages in chains.items():
    for stage_index, covered in enumerate(stages):
        projection = render_c(covered)
        archive = shapes._plain(covered)
        present, decoys = exam.build_exam(covered, len(covered), random.Random(9000 + stage_index))
        in_proj = sum(1 for v in present if v in projection)
        in_arch = sum(1 for v in present if v in archive)
        absent = [v for v in present if v not in projection]
        totals["items"] += len(present)
        totals["in_proj"] += in_proj
        totals["in_archive"] += in_arch
        totals["absent"] += len(absent)
        print(f'{name:13s} {stage_index+1:>5d} {len(present):>6d} {in_proj:>8d} '
              f'{in_arch:>11d} {len(absent):>12d}')
        for v in absent[:3]:
            examples.append((name, stage_index + 1, v, v in archive))

print("\n=== the values C's projection does not contain ===")
for name, stage, value, in_archive in examples:
    where = "in the raw archive" if in_archive else "NOT IN THE ARCHIVE EITHER"
    print(f'  {name} s{stage}: {value!r:46s} {where}')

print(f'\nsummary: {totals["items"]} exam items')
print(f'  present in C projection : {totals["in_proj"]:>4d} ({100*totals["in_proj"]/totals["items"]:.0f}%)')
print(f'  present in raw archive  : {totals["in_archive"]:>4d} ({100*totals["in_archive"]/totals["items"]:.0f}%)')
print(f'  absent from projection  : {totals["absent"]:>4d}')
