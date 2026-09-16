"""Final comparison for the C3 combination validation, against the pre-registered
criteria.

C3 = sticky summary + overflow trigger + verbatim assistant originals.
"""
import collections, json, pathlib

root = pathlib.Path("/Users/geilige/future-os/.future/research/abc-summary-a47313")
FIELDS = ["project", "first_limit", "latest_limit", "format", "first_code", "old_version",
          "latest_version", "buried", "validation", "blocker", "deployment", "device"]
ALL_PROBES = {(t, m, s) for t in ("export", "analysis")
              for m in ("future/deepseek-flash", "future/glm-5.3-flash") for s in (0, 3, 7)}


def rows_for(arm, probes=ALL_PROBES):
    out = {}
    for p in sorted((root / arm / "results").glob("*retrievalcodex__uncapped*.json")):
        d = json.loads(p.read_text())
        key = (d["task"], d["model"], d["stage"])
        if key not in probes:
            continue
        marks = (d.get("grade") or {}).get("marks") or {}
        out[key] = {"correct": d["grade"]["correct"] if marks else 0, "marks": marks,
                    "status": d["status"], "calls": len(d["call_ids"])}
    return out


print("=== full sample: 12 probes (2 chains x 2 models x 3 stages) ===")
print(f'{"arm":34s} {"score":>10s} {"lost":>6s} {"req/probe":>10s}')
table = {}
for arm, label in (("C3", "C3 (combined)"), ("codex", "Codex"), ("Csched", "C+sticky"),
                   ("C", "C (evidence only)"), ("B", "B (no summary)")):
    items = rows_for(arm)
    if len(items) < len(ALL_PROBES):
        print(f'{label:34s} (only {len(items)}/12 probes)')
        continue
    table[arm] = items
    total = sum(i["correct"] for i in items.values())
    lost = sum(1 for i in items.values() if not i["marks"])
    calls = sum(i["calls"] for i in items.values()) / len(items)
    print(f'{label:34s} {total:>5d}/144 {lost:>6d} {calls:>10.1f}')

if "C3" in table and "codex" in table:
    print("\nper-field, C3 vs Codex (12 probes each):")
    print(f'{"field":18s} {"C3":>8s} {"codex":>8s} {"C":>8s}')
    for f in FIELDS:
        cells = []
        for arm in ("C3", "codex", "C"):
            if arm not in table:
                continue
            n = sum(1 for i in table[arm].values() if i["marks"].get(f))
            cells.append(f'{n}/12')
        print(f'{f:18s} ' + " ".join(f'{c:>8s}' for c in cells))

print("\n=== pre-registered criteria ===")
if "C3" in table:
    c3_total = sum(i["correct"] for i in table["C3"].values())
    c3_lost = sum(1 for i in table["C3"].values() if not i["marks"])
    buried = sum(1 for i in table["C3"].values() if i["marks"].get("buried"))
    codex_total = sum(i["correct"] for i in table.get("codex", {}).values()) or None
    print(f'P1 accuracy vs Codex   : C3 {c3_total}/144' +
          (f' vs Codex {codex_total}/144 -> ' +
           ("PASS" if codex_total is None or c3_total >= codex_total else "FAIL") if codex_total else ""))
    print(f'P2 no lost probes      : {c3_lost} lost -> ' + ("PASS" if c3_lost == 0 else "FAIL"))
    print(f'P2 buried (mid-record)  : {buried}/12 -> ' + ("PASS" if buried >= 10 else "FAIL"))
    if "C" in table:
        regressions = [f for f in FIELDS
                       if sum(1 for i in table["C3"].values() if i["marks"].get(f))
                       < sum(1 for i in table["C"].values() if i["marks"].get(f))]
        print(f'P4 no field regression  : {regressions or "none"} -> ' + ("PASS" if not regressions else "FAIL"))

res = json.loads((root / "c3-calls.json").read_text()) if (root / "c3-calls.json").exists() else []
print(f'\nC3 ledger: {len(res)} requests, CNY '
      f'{sum(r.get("charged", r["reserved"]) for r in res):.4f}')
total = 0.0
for p in sorted(root.glob("*calls*.json")):
    try:
        r = json.loads(p.read_text())
    except Exception:
        continue
    if isinstance(r, list):
        total += sum(x.get("charged", x["reserved"]) for x in r if isinstance(x, dict))
print(f'TOTAL: CNY {total:.4f} of 140')
