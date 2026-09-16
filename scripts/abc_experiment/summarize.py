#!/usr/bin/env python3
"""Summarize every arm of the compaction experiment into a markdown table.

Reads the local results root (default: the git-ignored research directory) and
prints the closed-book table plus the per-arm cost breakdown. A/B/C come from
the frozen SCORED.json; M and the external arms are read from their own result
directories.
"""
import argparse, collections, json, pathlib

ARMS = [
    ("A", "SCORED", "A", "protected originals + model summary + tail"),
    ("B", "SCORED", "B", "originals + tail, summary deleted"),
    ("C", "SCORED", "C", "originals + deterministic evidence + tail"),
    ("M", "dir", "main", "origin/main: summary + tail"),
    ("Codex", "dir", "codex", "all user messages + summary"),
    ("OpenCode", "dir", "opencode", "summary + retained tail"),
]
PROBES = [0, 3, 7]


def load(root):
    rows = collections.defaultdict(dict)
    for item in json.loads((root / "SCORED.json").read_text()):
        if item["retrieval"]:
            continue
        rows[item["arm"]][(item["task"], item["model"], item["stage"])] = item
    for arm in ("main", "codex", "opencode"):
        for path in sorted((root / arm / "results").glob("*.json")):
            data = json.loads(path.read_text())
            key = (data["task"], data["model"], data["stage"])
            rows[arm][key] = {"fact_correct": data["grade"]["correct"],
                              "delivered": data["status"] == "completed",
                              "context_tokens": data["context_tokens"]}
    return rows


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--root", type=pathlib.Path, required=True)
    args = parser.parse_args()
    rows = load(args.root)

    print("| Arm | What survives compaction | stage 1 | stage 4 | stage 8 | total | mean ctx |")
    print("|---|---|---:|---:|---:|---:|---:|")
    for label, source, key, description in ARMS:
        items = rows.get(key, {})
        cells, total, weights = [], 0, []
        for stage in PROBES:
            bucket = [v for (t, m, s), v in items.items() if s == stage]
            if not bucket:
                cells.append("—")
                continue
            got = sum(v["fact_correct"] for v in bucket)
            total += got
            cells.append(f"{got}/{12 * len(bucket)}")
            weights += [v["context_tokens"] for v in bucket]
        grand = sum(12 * len([1 for (t, m, s) in items if s == stage]) for stage in PROBES)
        print(f"| {label} | {description} | {cells[0]} | {cells[1]} | {cells[2]} | "
              f"{total}/{grand} | {sum(weights) // max(len(weights), 1)} |")

    ledger = json.loads((args.root / "calls.json").read_text())
    spend = collections.defaultdict(float)
    calls = collections.Counter()
    for row in ledger:
        arm = ("Codex" if "__codex" in row["id"] else
               "OpenCode" if "__opencode" in row["id"] else
               "M" if "__main" in row["id"] else "A/B/C")
        spend[arm] += row.get("charged", row["reserved"])
        calls[arm] += 1
    print("\n| Group | requests | CNY |")
    print("|---|---:|---:|")
    for arm in ("A/B/C", "M", "Codex", "OpenCode"):
        print(f"| {arm} | {calls[arm]} | {spend[arm]:.4f} |")
    print(f"| **total** | **{len(ledger)}** | **{sum(spend.values()):.4f}** |")


if __name__ == "__main__":
    main()
