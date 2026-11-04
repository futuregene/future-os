#!/usr/bin/env python3
"""Turn the raw samples from `measure-remote-latency.py` into one table.

Reads the newest `cold-*.json` / `warm-*.json` pair (or the pair named by
`--tag`) and prints a Markdown table plus the commands that crossed the 100 ms
budget. The table is what `docs/testing/remote-latency/03-measurements.md`
quotes, so a later run can regenerate it instead of retyping numbers.

    python3 scripts/perf/remote-commands/summarize-remote-latency.py
    python3 scripts/perf/remote-commands/summarize-remote-latency.py --tag 20261104-run5
"""
import argparse
import glob
import json
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
RAW = ROOT / "docs" / "testing" / "remote-latency" / "raw"
BUDGET_MS = 100.0


def newest(phase, tag):
    pattern = str(RAW / (f"{phase}-{tag}.json" if tag else f"{phase}-*.json"))
    files = sorted(glob.glob(pattern))
    if not files:
        raise SystemExit(f"no {phase} samples matching {pattern}")
    return json.loads(Path(files[-1]).read_text())


def scale_text(scale):
    parts = []
    for key, value in scale.items():
        if value in (None, "", 0) and key not in ("events", "entries"):
            continue
        parts.append(f"{key}={value}")
    return " ".join(parts) or "—"


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--tag")
    parser.add_argument("--budget", type=float, default=BUDGET_MS)
    args = parser.parse_args()
    cold = newest("cold", args.tag)
    warm = newest("warm", args.tag)
    warm_by_label = {result["label"]: result for result in warm["results"]}

    print("| 命令 | 数据规模 | cold ms | p50 ms | p95 ms | max ms | 回复字节 | 判定 |")
    print("| --- | --- | ---: | ---: | ---: | ---: | ---: | --- |")
    over = []
    unmeasured = []
    for result in cold["results"]:
        warm_result = warm_by_label.get(result["label"], result)
        if result.get("parseError") and warm_result["replies"] == 0:
            unmeasured.append((result["label"], result["parseError"]))
            verdict = "未测出"
        elif warm_result["replies"] < warm_result["iterations"]:
            unmeasured.append((result["label"], (warm_result["cold"] or {}).get("error", "")))
            verdict = "失败"
        elif max(warm_result["p95Ms"], (result["cold"] or {}).get("ms", 0)) > args.budget:
            verdict = "**超预算**"
            over.append(result["label"])
        else:
            verdict = "ok"
        cold_ms = (result["cold"] or {}).get("ms", 0.0)
        print(
            f'| `{result["label"]}` | {scale_text(result["scale"])} | {cold_ms:.1f} | '
            f'{warm_result["p50Ms"]:.1f} | {warm_result["p95Ms"]:.1f} | '
            f'{warm_result["maxMs"]:.1f} | {warm_result["replyBytes"]:,} | {verdict} |'
        )
    print()
    print(f"over budget (> {args.budget:.0f} ms): {', '.join(over) if over else 'none'}")
    for label, reason in unmeasured:
        print(f"not measured: {label} — {reason}")


if __name__ == "__main__":
    main()
