#!/usr/bin/env python3
"""Cross-check the channel provider matrix against the code that defines it.

    python3 scripts/docs/check-channel-matrix.py

The table in `docs/guide/channels-providers.md` is the page a reader scans to
decide what a channel can do, and every value in it is declared by the provider's
`ChannelDefinition`. Nothing keeps the two in step by itself: the page is written
by hand and the code changes as providers land — which is exactly how the table
came to describe three implemented channels as `planned`.

This compares the capability columns, the message limit (value *and* unit) and the
maturity of every row against the provider source, and exits non-zero on a
mismatch. Run it after touching a provider's definition, or the page.
"""

from __future__ import annotations

import pathlib
import re
import sys

PROVIDERS = pathlib.Path("channels/src/providers")
MATRIX = pathlib.Path("docs/guide/channels-providers.md")
REGISTRY = pathlib.Path("channels/src/providers/registry.rs")

# `ProviderEntry { definition: &crate::providers::<module>::DEFINITION, … }`
REGISTERED = re.compile(r"&crate::providers::(\w+)::DEFINITION")

# | Channel | `id` | maturity | inbound | edit | threads | typing | reactions | media | gate | max | requires |
ROW = re.compile(r"^\|\s*([^|]+?)\s*\|\s*`([a-z]+)`\s*\|\s*(\w+)\s*\|\s*([^|]+?)\s*\|(.+)\|\s*$")

# Capability fields in the column order the table uses.
COLUMNS = ("edit", "threads", "typing", "reactions", "media_in", "mention_gate")
UNIT_NAMES = {"Chars": "chars", "Utf16": "UTF-16 units", "Bytes": "bytes"}


def read_definition(provider: pathlib.Path) -> dict | None:
    """The capability flags, limit, unit and maturity a provider declares."""
    text = provider.read_text()
    block = re.search(r"capabilities: Capabilities \{(.*?)\}", text, re.S)
    if block is None:
        return None
    body = block.group(1)

    def flag(name: str) -> bool | None:
        found = re.search(rf"{name}: (\w+)", body)
        return None if found is None else found.group(1) == "true"

    limit = re.search(r"max_text_len: ([\d_]+)", text)
    unit = re.search(r"length_unit: LengthUnit::(\w+)", text)
    maturity = re.search(r"maturity: Maturity::(\w+)", text)
    return {
        "flags": [flag(name) for name in COLUMNS],
        "limit": limit.group(1).replace("_", "") if limit else "?",
        "unit": UNIT_NAMES.get(unit.group(1), "?") if unit else "?",
        "maturity": maturity.group(1).lower() if maturity else "?",
    }


def check_every_provider_has_a_row(rows: set[str]) -> int:
    """Every registered channel must appear in the table.

    The row loop in `main` iterates the *table* and skips a provider it has no
    row for, so on its own it cannot notice a channel that was added without
    one — the reader's page would simply be missing a channel. This is the
    other direction, and it is the mistake that is easy to make: the code, the
    registry and `future channel list` are all consistent while the table a
    person scans is not.

    Only the framework registry is consulted; the self-bridged pair (Feishu,
    DingTalk) is deliberately not in this table.
    """
    problems = 0
    for module in sorted(set(REGISTERED.findall(REGISTRY.read_text()))):
        if module not in rows:
            problems += 1
            print(f"MISSING {module}: registered but has no row in the matrix")
    return problems


def main() -> int:
    problems = 0
    checked = 0
    documented: set[str] = set()
    for line in MATRIX.read_text().splitlines():
        match = ROW.match(line)
        if match is None or match.group(1).strip() == "Channel":
            continue
        channel_id, maturity = match.group(2), match.group(3)
        cells = [cell.strip() for cell in match.group(5).split("|")]
        if len(cells) < 7:
            continue
        documented.add(channel_id)
        provider = PROVIDERS / f"{channel_id}.rs"
        if not provider.exists():
            continue
        declared = read_definition(provider)
        if declared is None:
            continue
        checked += 1

        actual = ["yes" if flag else "no" for flag in declared["flags"]]
        if cells[:6] != actual:
            problems += 1
            print(f"MISMATCH {channel_id} capabilities: page={cells[:6]} code={actual}")

        expected_limit = f"{declared['limit']} {declared['unit']}"
        if cells[6] != expected_limit:
            problems += 1
            print(f"MISMATCH {channel_id} limit: page='{cells[6]}' code='{expected_limit}'")

        if maturity != declared["maturity"]:
            problems += 1
            print(f"MISMATCH {channel_id} maturity: page={maturity} code={declared['maturity']}")

    missing_rows = check_every_provider_has_a_row(documented)
    problems += missing_rows

    if problems:
        print(f"{problems} mismatch(es) across {checked} provider row(s)")
        return 1
    print(
        f"provider matrix matches the code ({checked} rows, every registered "
        f"provider has one)"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
