"""Closed-book vs search-enabled, with the search tool held constant.

A/B/C retrieval results come from the original driver (same fixtures, same
archive CLI, same 5-call / 32 KB budget). Codex/OpenCode/M retrieval results were
produced by this harness with the identical tool and budget, so the search engine,
archive, models and questionnaire are all controlled — only the projection varies.

Codex's own history tools are excluded by auth gating (see
abc_external_provenance.json), so its search arm uses our CLI and is labelled a
hybrid rather than a measurement of Codex's built-in retrieval.
"""
import collections, json, pathlib

root = pathlib.Path("/Users/geilige/future-os/.future/research/abc-summary-a47313")
rows = collections.defaultdict(lambda: {"closed": [], "search": []})

for item in json.loads((root / "SCORED.json").read_text()):
    rows[item["arm"]]["search" if item["retrieval"] else "closed"].append(
        {"correct": item["fact_correct"], "delivered": item["delivered"]})

for arm in ("main", "codex", "opencode"):
    for path in sorted((root / arm / "results").glob("*.json")):
        data = json.loads(path.read_text())
        if arm == "opencode" and "v2" not in data["id"]:
            continue
        rows[arm]["search" if data.get("retrieval") else "closed"].append(
            {"correct": data["grade"]["correct"], "delivered": data["status"] == "completed"})


def cell(items):
    if not items:
        return "—", None, None
    got = sum(i["correct"] for i in items)
    full = 12 * len(items)
    delivered = [i for i in items if i["delivered"]]
    delivered_got = sum(i["correct"] for i in delivered)
    text = f'{got}/{full} · {len(delivered)}/{len(items)} delivered'
    return text, (got, full), (delivered_got, 12 * len(delivered))


print(f'{"arm":10s} {"closed-book":>34s} {"with search (same CLI)":>34s}   delta')
summary = {}
for arm in ("A", "B", "C", "main", "codex", "opencode"):
    closed, closed_n, closed_d = cell(rows[arm]["closed"])
    search, search_n, search_d = cell(rows[arm]["search"])
    delta = f'{search_n[0] - closed_n[0]:+d}/{closed_n[1]}' if closed_n and search_n else ""
    summary[arm] = {"closed": closed_n, "closed_delivered": closed_d,
                    "search": search_n, "search_delivered": search_d}
    print(f'{arm:10s} {closed:>34s} {search:>34s}   {delta}')

print()
print("delivered-only accuracy (undelivered probes excluded):")
for arm, s in summary.items():
    if not s["search_delivered"]:
        continue
    c, cn = s["closed_delivered"]
    q, qn = s["search_delivered"]
    print(f'  {arm:9s} closed {c}/{cn}   with search {q}/{qn}')

ledger = json.loads((root / "calls.json").read_text())
spend = sum(r.get("charged", r["reserved"]) for r in ledger)
print(f'\nledger: {len(ledger)} requests, CNY {spend:.4f}')
