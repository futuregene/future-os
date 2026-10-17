#!/usr/bin/env python3
"""The ledger-row shapes `reconcile.py` ships, loaded dynamically.

`claimed_uncovered()` accepts several row shapes (first cell `path:lines`; a bare
tag first cell then `path:lines`; `path` + space + lines with no colon). Each is a
module-level pattern in `reconcile.py` whose names start with `LEDGER_ROW`, and more
may be added while the campaign runs — this module never restates them, it discovers
them, so a new shape shows up in `rowscan.py`/`claimrows.py` without an edit here.

`BEFORE` is the one historical pattern worth keeping a copy of: it is what the tool
used before todo_7449334225b8, and the "before" column of rowscan's table is the
evidence for what the fix bought.
"""
import importlib.util
import os
import re

HERE = os.path.dirname(os.path.abspath(__file__))

# The pattern shipped BEFORE todo_7449334225b8: it demanded the closing pipe
# immediately after the line numbers, so `| `a.rs:12` | …` matched nothing.
BEFORE_SRC = (
    r"^\s*\|\s*`?([A-Za-z0-9_./-]+\.(?:tsx|ts|rs))`?\s*:\s*([0-9,\-\s]+?)\s*\|`?")
BEFORE = re.compile(BEFORE_SRC)


def load_reconcile():
    spec = importlib.util.spec_from_file_location(
        "cov100_reconcile", os.path.join(HERE, "reconcile.py"))
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


def shipped_patterns(rec):
    """Every `LEDGER_ROW*` module-level pattern the shipped tool exposes."""
    names = [n for n in dir(rec) if n == "LEDGER_ROW" or n.startswith("LEDGER_ROW_")]
    # LEDGER_ROW first (the canonical shape), then the rest alphabetically
    names.sort(key=lambda n: (n != "LEDGER_ROW", n))
    return [(n, getattr(rec, n)) for n in names]


def shipped_match(line, rec):
    """The first shipped pattern that accepts `line`, or None."""
    for _name, pat in shipped_patterns(rec):
        m = pat.match(line)
        if m:
            return m
    return None
