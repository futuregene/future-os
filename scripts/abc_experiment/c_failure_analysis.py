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

ROOT = pathlib.Path("/Users/geilige/future-os/.future/research/abc-summary-a47313")
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

cfg = json.loads((ROOT / "real-sessions.json").read_text())
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
