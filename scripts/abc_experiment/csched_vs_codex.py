"""Apples-to-apples: the same six probes (deepseek, 2 tasks, stages 1/4/8).

Csched and Clive only have deepseek projections, so every arm is filtered to the
probes that exist for all of them.
"""
import collections, json, pathlib

root = pathlib.Path("/Users/geilige/future-os/.future/research/abc-summary-a47313")
FIELDS = ["project", "first_limit", "latest_limit", "format", "first_code", "old_version",
          "latest_version", "buried", "validation", "blocker", "deployment", "device"]
PROBES = {(t, "future/deepseek-flash", s) for t in ("export", "analysis") for s in (0, 3, 7)}


def rows_for(arm):
    out = {}
    for p in sorted((root / arm / "results").glob("*retrievalcodex__uncapped*.json")):
        d = json.loads(p.read_text())
        key = (d["task"], d["model"], d["stage"])
        if key not in PROBES:
            continue
        marks = (d.get("grade") or {}).get("marks") or {}
        out[key] = {"correct": d["grade"]["correct"] if marks else 0, "marks": marks,
                    "status": d["status"], "calls": len(d["call_ids"])}
    return out


LABEL = {
    "codex": "Codex (user msgs + summary)",
    "Csched": "C + sticky summary, Codex trigger",
    "Clive": "C + summary (non-sticky)",
    "Cplus": "C + summary (clipped input)",
    "C": "C (evidence only)",
    "A": "A (originals + summary)",
    "B": "B (no summary)",
    "main": "M (origin/main)",
}
print(f'{"arm":36s} {"score":>8s} {"no ans":>7s} {"req/probe":>10s}')
table = {}
for arm in ("Csched", "codex", "C", "Clive", "Cplus", "A", "B", "main"):
    items = rows_for(arm)
    if len(items) < len(PROBES):
        print(f'{LABEL.get(arm, arm):36s} (only {len(items)}/6 probes available)')
        continue
    total = sum(i["correct"] for i in items.values())
    blank = sum(1 for i in items.values() if not i["marks"])
    calls = sum(i["calls"] for i in items.values()) / len(items)
    table[arm] = items
    print(f'{LABEL.get(arm, arm):36s} {total:>4d}/72 {blank:>7d} {calls:>10.1f}')

print("\nper-field on the same six probes:")
arms = [a for a in ("Csched", "codex", "C") if a in table]
print(f'{"field":18s} ' + " ".join(f'{a:>8s}' for a in arms))
for f in FIELDS:
    cells = []
    for a in arms:
        n = sum(1 for i in table[a].values() if i["marks"].get(f))
        cells.append(f'{n}/6')
    print(f'{f:18s} ' + " ".join(f'{c:>8s}' for c in cells))

print("\nper-probe:")
print(f'{"probe":34s} ' + " ".join(f'{a:>8s}' for a in arms))
for key in sorted(PROBES):
    label = f"{key[0]}/s{key[2]+1}"
    cells = []
    for a in arms:
        i = table[a].get(key)
        cells.append(f'{i["correct"]}/12' if i else "—")
    print(f'{label:34s} ' + " ".join(f'{c:>8s}' for c in cells))

print("\nCsched summary cost (sticky chain, Codex trigger):")
led = json.loads((root / "csched-calls.json").read_text())
summ = [r for r in led if "summary" in r["id"] and not r["id"].endswith("#empty-output")]
print(f'  {len(summ)} summary requests, CNY {sum(r.get("charged", r["reserved"]) for r in summ):.4f}')
for r in summ:
    if r["id"].endswith("__summary"):
        print(f'    {r["id"][:56]:56s} in={str(r.get("input_tokens")):>7s} out={str(r.get("output_tokens")):>6s} '
              f'CNY {r.get("charged", r["reserved"]):.4f}')
