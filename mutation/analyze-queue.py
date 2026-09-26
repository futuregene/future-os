"""Per-mutant attribution table for a cargo-mutants sample.

Reads cargo-mutants' own outcomes.json (the authoritative name<->log mapping;
log file names carry a `_NNN` dedup counter assigned in RUN order, so the
mapping must come from the JSON, not from the file name) and prints, for every
mutant, the tests that actually failed in that mutant's log plus the panic
message that failed them.

Usage:  python mutation/analyze-queue.py [mutation/out-<run>/mutants.out]
        (default: mutation/out-queue/mutants.out)
"""
import json
import pathlib
import re
import sys

OUT = pathlib.Path(sys.argv[1]) if len(sys.argv) > 1 else pathlib.Path("mutation/out-queue/mutants.out")

FAILED_RE = re.compile(r"^test (\S+) \.\.\. FAILED", re.M)
# panic payload lines look like `thread 'x' panicked at ...:\n<message>`
PANIC_RE = re.compile(r"panicked at [^\n]*\n((?:.*\n){0,4}?)note: run with", re.M)
ASSERT_RE = re.compile(r"^(assertion.*|left.*|right.*|.*assert!.*)$", re.M)


def failure_lines(log_text: str) -> list[str]:
    """Test names that failed, in order, deduplicated."""
    seen: list[str] = []
    for m in FAILED_RE.finditer(log_text):
        if m.group(1) not in seen:
            seen.append(m.group(1))
    return seen


def panic_snippets(log_text: str, limit: int = 2) -> list[str]:
    out: list[str] = []
    for m in re.finditer(r"panicked at ([^\n]*)\n", log_text):
        # grab the 3 lines after the panic header
        start = m.end()
        chunk = log_text[start:start + 400]
        head = " | ".join(x.strip() for x in chunk.splitlines()[:3] if x.strip())
        out.append(f"{m.group(1)} => {head}")
        if len(out) >= limit:
            break
    return out


def compile_errors(log_text: str) -> list[str]:
    return sorted(set(re.findall(r"error\[(E\d+)\]: ([^\n]{0,80})", log_text)))[:3]


def main() -> int:
    doc = json.loads((OUT / "outcomes.json").read_text(encoding="utf-8"))
    rows = []
    for entry in doc["outcomes"]:
        scen = entry["scenario"]
        if "Mutant" not in scen:
            continue
        name = scen["Mutant"]["name"]
        log_rel = entry["log_path"]
        log = (OUT / log_rel).read_text(encoding="utf-8", errors="replace")
        rows.append((name, entry["summary"], log_rel, log))

    order = {"CaughtMutant": 0, "MissedMutant": 1, "Unviable": 2, "Timeout": 3}
    rows.sort(key=lambda r: (order.get(r[1], 9), r[0]))

    for name, summary, log_rel, log in rows:
        print("=" * 100)
        print(f"{summary:14s} {name}")
        print(f"  log: {log_rel}  ({len(log)} bytes)")
        if summary == "Unviable":
            for code, msg in compile_errors(log):
                print(f"  COMPILE ERROR {code}: {msg}")
            continue
        fails = failure_lines(log)
        print(f"  failing tests ({len(fails)}): " + ", ".join(fails) if fails else "  failing tests: NONE")
        for p in panic_snippets(log):
            print(f"    panic: {p}")

    print("=" * 100)
    counts: dict[str, int] = {}
    for _, s, _, _ in rows:
        counts[s] = counts.get(s, 0) + 1
    print("counts:", counts)
    print("outcomes.json totals:", {k: doc[k] for k in
          ("total_mutants", "caught", "missed", "timeout", "unviable")})
    return 0


if __name__ == "__main__":
    sys.exit(main())
