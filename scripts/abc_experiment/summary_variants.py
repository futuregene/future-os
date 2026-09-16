"""Definitive comparison: every C variant vs Codex, same interface, uncapped."""
import collections, json, pathlib

root = pathlib.Path("/Users/geilige/future-os/.future/research/abc-summary-a47313")
FIELDS = ["project", "first_limit", "latest_limit", "format", "first_code", "old_version",
          "latest_version", "buried", "validation", "blocker", "deployment", "device"]
SUFFIXABLE = ("A", "B", "C", "codex", "opencode", "main", "Cplus", "Clive")


def rows_for(arm):
    out = []
    for p in sorted((root / arm / "results").glob("*retrievalcodex__uncapped*.json")):
        d = json.loads(p.read_text())
        marks = (d.get("grade") or {}).get("marks") or {}
        out.append({"correct": d["grade"]["correct"] if marks else 0, "marks": marks,
                    "status": d["status"], "calls": len(d["call_ids"]), "id": d["id"]})
    return out


print(f'{"arm":28s} {"score":>11s} {"no ans":>7s} {"req/probe":>10s}   summary input')
LABEL = {
    "C": ("C: evidence, no summary", "(none)"),
    "Cplus": ("C + summary (clipped input)", "clipped / protected material"),
    "Clive": ("C + summary (LIVE input)", "prior projection + new records"),
    "codex": ("Codex: user msgs + summary", "whole live history"),
    "A": ("A: originals + summary", "clipped material"),
    "B": ("B: no summary", "(none)"),
    "main": ("M (origin/main)", "clipped material"),
}
for arm in ("codex", "C", "Cplus", "Clive", "A", "B", "main"):
    items = rows_for(arm)
    if not items:
        continue
    total = sum(i["correct"] for i in items)
    blank = sum(1 for i in items if not i["marks"])
    calls = sum(i["calls"] for i in items) / len(items)
    name, source = LABEL.get(arm, (arm, "?"))
    print(f'{name:28s} {total:>4d}/{12*len(items):<6d} {blank:>7d} {calls:>10.1f}   {source}')

print("\nper-field, C variants vs Codex:")
print(f'{"field":18s} {"codex":>7s} {"C":>7s} {"C+clip":>7s} {"C+live":>7s}')
for f in FIELDS:
    cells = []
    for arm in ("codex", "C", "Cplus", "Clive"):
        items = rows_for(arm)
        n = sum(1 for i in items if i["marks"].get(f))
        cells.append(f'{n}/{len(items)}')
    print(f'{f:18s} ' + " ".join(f'{c:>7s}' for c in cells))

print("\nC+live: requests per probe (the summaries cost rounds)")
for item in sorted(rows_for("Clive"), key=lambda r: r["id"]):
    print(f'  {item["id"][:50]:50s} {item["status"]:12s} {item["correct"]:2d}/12 calls={item["calls"]}')

total = 0.0
for p in sorted(root.glob("*calls*.json")):
    try:
        rows = json.loads(p.read_text())
    except Exception:
        continue
    if isinstance(rows, list):
        total += sum(r.get("charged", r["reserved"]) for r in rows if isinstance(r, dict))
print(f'\nrecorded spend CNY {total:.4f}')
