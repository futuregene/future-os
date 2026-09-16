"""Does the summary carry anything the rest of the projection lacks?

The ablation shows C-with-summary and C-without-summary score identically. This tests the
mechanism: for every exam value the deterministic projection does not contain, is it
present in the summary?

If the answer is "no", the summary is a lossy third copy of material that is already
retained verbatim, and can add nothing on a question set that asks about exact values.
"""
import json, pathlib, random, sys, collections

WT = pathlib.Path("/Users/geilige/future-os/.worktrees/session-history-a47313")
sys.path.insert(0, str(WT / "scripts"))
sys.path.insert(0, str(WT / "scripts/abc_experiment"))
import realistic_exam as exam
import abc_cache_shapes as shapes

ROOT = pathlib.Path("/Users/geilige/future-os/.future/research/abc-summary-a47313")
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
