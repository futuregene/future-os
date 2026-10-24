"""Fragments under `scenarios.d/` merge into the scenario table.

Two sessions auditing different platforms must be able to add scenarios without
editing the one shared `scenarios.json`, so `capture.load_scenarios` merges a
fragment per platform on top of it. This pins the merge rules offline (no
browser): fragments override same-named entries, comments are ignored, and a
fragment that names an unknown platform or is not a mapping fails loudly instead
of silently contributing nothing.
"""
from __future__ import annotations

import importlib.util
import json
import sys
import tempfile
from pathlib import Path

CAPTURE = Path(__file__).resolve().parent.parent / "screenshots" / "capture.py"


def load_capture():
    spec = importlib.util.spec_from_file_location("capture_under_test", CAPTURE)
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module


def write(root: Path, name: str, payload: dict) -> None:
    (root / name).write_text(json.dumps(payload), encoding="utf-8")


def main() -> int:
    capture = load_capture()
    failures: list[str] = []

    with tempfile.TemporaryDirectory() as raw:
        scripts = Path(raw)
        write(scripts, "scenarios.json", {
            "_comment": "the shared table",
            "desktop": {"port": 5299, "scenarios": {"shared": {"steps": []}}},
            "mobile": {"port": 8099, "scenarios": {}},
        })
        merge = capture.load_scenarios(scripts)
        if sorted(merge["desktop"]["scenarios"]) != ["shared"]:
            failures.append("no fragments directory must leave the table untouched")
        if merge["desktop"]["port"] != 5299:
            failures.append("platform-level settings must survive the merge")

        fragments = scripts / "scenarios.d"
        fragments.mkdir()
        write(fragments, "b-desktop.json", {
            "_comment": "desktop audit",
            "desktop": {"extra": {"steps": [{"wait": 10}]}},
        })
        write(fragments, "a-mobile.json", {"mobile": {"only": {"steps": []}}})
        merge = capture.load_scenarios(scripts)
        if sorted(merge["desktop"]["scenarios"]) != ["extra", "shared"]:
            failures.append("a desktop fragment must add to the desktop table only: "
                            + repr(sorted(merge["desktop"]["scenarios"])))
        if sorted(merge["mobile"]["scenarios"]) != ["only"]:
            failures.append("a mobile fragment must add to the mobile table: "
                            + repr(sorted(merge["mobile"]["scenarios"])))

        write(fragments, "c-override.json", {"desktop": {"shared": {"steps": [{"wait": 99}]}}})
        merge = capture.load_scenarios(scripts)
        if merge["desktop"]["scenarios"]["shared"]["steps"] != [{"wait": 99}]:
            failures.append("a fragment must override a scenario of the same name")

        write(fragments, "d-unknown.json", {"tablet": {}})
        try:
            capture.load_scenarios(scripts)
            failures.append("an unknown platform must be an error")
        except SystemExit as error:
            if "unknown platform" not in str(error):
                failures.append(f"the unknown-platform error must name the problem: {error}")

        write(fragments, "d-unknown.json", {"desktop": [1, 2]})
        try:
            capture.load_scenarios(scripts)
            failures.append("a non-mapping platform must be an error")
        except SystemExit as error:
            if "map scenario names" not in str(error):
                failures.append(f"the shape error must say what is wrong: {error}")

    for failure in failures:
        print(f"FAIL: {failure}")
    if failures:
        return 1
    print("ok: scenario fragments merge, override and fail loudly")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
