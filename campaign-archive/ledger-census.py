#!/usr/bin/env python3
"""Is every module record REGISTERED in the waiver ledger?

WHY (measured 2026-10-17): the app-data slice finished its coverage, classified every line, passed its
tripwire - and was still not satisfying metric A, because `module-tauri-appdata` appeared ZERO times in
`docs/testing/waiver-ledger.md`. Metric A is "DA:0 = 0 OR registered in the waiver ledger", so an
unregistered module is a module that has not met the objective no matter how good its numbers are, and
**no per-line check can see it**: every line is mentioned, every category assigned, the gate green.

This census compares the module records that exist against the ledger's per-module table, so the same
gap cannot be discovered one module at a time at freeze.
"""
import os
import re
import sys

LEDGER = "docs/testing/waiver-ledger.md"
DOCS = "docs/testing"


def _rows_from(path):
    """Module names registered in the per-module table of `path`."""
    out = set()
    in_table = False
    for line in open(path, encoding="utf-8", errors="replace"):
        if line.startswith("## Per module"):
            in_table = True
            continue
        if in_table and line.startswith("## "):
            break
        if in_table:
            m = re.match(r"^\|\s*((?:module|desktop)-[a-z0-9-]+)\s*\|", line)
            if m:
                out.add(m.group(1))
    return out


def ledger_rows():
    """Module names registered in the per-module table."""
    return _rows_from(LEDGER)


def records():
    """Module records on disk -> the module name the ledger would use.

    TWO families, not one: the Rust/JS module records are `module-*.md`, and the desktop frontend's are
    `desktop-*.md` (desktop-agent, desktop-packages, ...). Globbing only `module-*` would have reported
    "every module record is registered" while five real records were never compared - the exact
    cannot-fail shape this campaign keeps finding in its own tools.
    """
    out = {}
    for f in sorted(os.listdir(DOCS)):
        if not f.endswith(".md"):
            continue
        if not (f.startswith("module-") or f.startswith("desktop-")):
            continue
        out[f[: -len(".md")]] = f
    return out


def self_test():
    """The control: the census must FAIL on a ledger that omits a registered record.

    Without this, a regex or glob bug reports 'every module record is registered' - which is exactly
    what happened twice while writing it (a `module-*`-only file glob, then a `module-*`-only row
    regex), and both times the output looked like success.
    """
    import tempfile
    ok = True

    # (1) a ledger missing module-cli-mcp must be reported
    with open(LEDGER, encoding="utf-8", errors="replace") as fh:
        body = fh.read()
    holed = "\n".join(l for l in body.splitlines() if not l.startswith("| module-cli-mcp |"))
    with tempfile.NamedTemporaryFile("w", suffix=".md", delete=False) as tf:
        tf.write(holed)
        tmp = tf.name
    reg = _rows_from(tmp)
    missing = [k for k in records() if k not in reg]
    hit = "module-cli-mcp" in missing
    print(f"  {'ok  ' if hit else 'FAIL'} a ledger without the module-cli-mcp row is reported "
          f"({len(missing)} missing)")
    ok = ok and hit

    # (2) the desktop-* family must be compared, not silently skipped
    has_desktop = any(k.startswith("desktop-") for k in records())
    print(f"  {'ok  ' if has_desktop else 'FAIL'} desktop-* records are in the comparison set")
    ok = ok and has_desktop

    # (3) the real ledger must be clean
    real = _rows_from(LEDGER)
    clean = not [k for k in records() if k not in real]
    print(f"  {'ok  ' if clean else 'FAIL'} the real ledger registers every record")
    ok = ok and clean

    os.unlink(tmp)
    return 0 if ok else 1


def main():
    if "--self-test" in sys.argv:
        return self_test()
    reg = ledger_rows()
    recs = records()
    missing = {k: v for k, v in recs.items() if k not in reg}
    print(f"ledger registers {len(reg)} module(s); {len(recs)} module record(s) exist on disk")
    print()
    print("REGISTERED:")
    for k in sorted(reg):
        mark = "" if k in recs else "   (no record file on disk -- stale ledger row?)"
        print(f"  ok  {k}{mark}")
    if missing:
        print()
        print("NOT REGISTERED (metric A cannot be claimed for these, whatever their numbers):")
        for k, f in sorted(missing.items()):
            print(f"  !!  {k}   record: {DOCS}/{f}")
        print()
        print("For each: either add a per-module row (the ledger's own convention is a")
        print("'(see its own §waiver ledger)' pointer plus categories), or state in the ledger")
        print("why the record is not a module of this campaign (a round-1 record, a review, etc.).")
        return 1
    print()
    print("every module record is registered")
    return 0


if __name__ == "__main__":
    sys.exit(main())
