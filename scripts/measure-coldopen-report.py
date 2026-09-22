#!/usr/bin/env python3
"""Summarize one or more cold-open measurement files as a comparison table.

  python3 scripts/measure-coldopen-report.py target/sync-measurements/*.json
"""
import json
from pathlib import Path
import sys


def rows(path: Path):
    return json.loads(path.read_text())


def kb(value: int) -> str:
    return f"{value / 1024:,.0f}"


def main() -> int:
    files = [Path(name) for name in sys.argv[1:]]
    if not files:
        raise SystemExit(__doc__)
    for path in files:
        print(f"\n=== {path.name} ===")
        print(f"{'sample':8}{'mode':13}{'sync ms':>8}{'1st ms':>7}"
              f"{'reqs':>6}{'pages':>7}{'chunks':>7}"
              f"{'hist KB':>9}{'replay KB':>11}{'wire KB':>9}"
              f"{'events':>8}{'items':>7}")
        for record in rows(path):
            if "error" in record:
                print(f"{record['sample']:8}{record['mode']:13}  ERROR {record['error'][:60]}")
                continue
            totals = record["totals"]
            print(
                f"{record['sample']:8}{record['mode']:13}"
                f"{record['syncMs']:>8}{record['firstCommitMs']:>7}"
                f"{totals['requests']:>6}"
                f"{record['history']['requests']:>7}{record['replay']['chunks']:>7}"
                f"{kb(record['history']['wireBytes']):>9}"
                f"{kb(record['replay']['wireBytes']):>11}"
                f"{kb(totals['wireBytes']):>9}"
                f"{record['projectedEvents'] or record['rawEvents']:>8}"
                f"{record['timelineItems']:>7}"
            )
    if len(files) > 1:
        print("\n=== delta (last vs first) ===")
        first, last = rows(files[0]), rows(files[-1])
        print(f"{'sample':8}{'mode':13}{'wire KB':>18}{'reqs':>12}{'sync ms':>16}")
        for a, b in zip(first, last):
            if "error" in a or "error" in b:
                continue
            key = f"{a['sample']:8}{a['mode']:13}"
            print(
                f"{key}"
                f"{kb(b['totals']['wireBytes']) + ' / ' + kb(a['totals']['wireBytes']):>18}"
                f"{str(b['totals']['requests']) + ' / ' + str(a['totals']['requests']):>12}"
                f"{str(b['syncMs']) + ' / ' + str(a['syncMs']):>16}"
            )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
