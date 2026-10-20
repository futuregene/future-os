#!/usr/bin/env python3
"""CI-only paired observation of existing Windows cargo-check commands.

No tests run. Registries are prepared outside timed compilation; each revision
starts with an empty target and shares its target across the three CI commands.
One sequential sample does not control OS page caches, thermal/load drift or
order effects, and does not prove release-build or unit-test performance.
"""

import argparse
from datetime import datetime, timezone
import json
import os
from pathlib import Path
import platform
import shutil
import subprocess
import time


def capture(args, cwd):
    return subprocess.check_output(args, cwd=cwd, text=True).strip()


def main():
    if os.environ.get("GITHUB_ACTIONS") != "true" or platform.system() != "Windows":
        raise SystemExit("This measurement requires a Windows GitHub Actions job")
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--baseline", required=True, type=Path)
    parser.add_argument("--candidate", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    args = parser.parse_args()
    roots = {"baseline": args.baseline.resolve(), "candidate": args.candidate.resolve()}
    output = args.output.resolve()
    output.mkdir(exist_ok=False, parents=True)
    for name in ("rust-toolchain.toml", ".cargo/config.toml"):
        if (roots["baseline"] / name).read_bytes() != (roots["candidate"] / name).read_bytes():
            raise SystemExit(f"Cannot compare different {name}")

    # Explicitly disable compiler wrappers/incremental artifacts for both trees.
    env = {**os.environ, "RUSTC_WRAPPER": "", "RUSTC_WORKSPACE_WRAPPER": "", "CARGO_INCREMENTAL": "0"}
    commands = [
        ("workspace", ["check", "--workspace", "--exclude", "futureos", "--manifest-path", "Cargo.toml"]),
        ("desktop-gui", ["check", "-p", "future-cli", "-p", "futureos"]),
        ("desktop-no-gui-library", ["check", "--no-default-features", "--manifest-path", "desktop/src-tauri/Cargo.toml"]),
    ]
    target_root = Path(os.environ["RUNNER_TEMP"]) / "remote-compilation-targets"
    target_root.mkdir(exist_ok=False)
    report = {
        "version": 1,
        "startedAt": datetime.now(timezone.utc).isoformat(),
        "runnerOS": platform.platform(),
        "cpuCount": os.cpu_count(),
        "rustc": capture(["rustc", "-Vv"], roots["candidate"]),
        "cargo": capture(["cargo", "-V"], roots["candidate"]),
        "toolchain": os.environ.get("RUSTUP_TOOLCHAIN"),
        "rustflags": os.environ.get("RUSTFLAGS"),
        "encodedRustflags": os.environ.get("CARGO_ENCODED_RUSTFLAGS"),
        "order": list(roots),
        "limitations": [
            "One sequential sample; baseline runs first. OS caches, order and runner load are uncontrolled.",
            "Empty separate target per revision; registry preparation is outside compilation. No Actions cache is restored or saved.",
            "The same existing CI cargo-check commands run; workspace membership changes are part of the measured cost.",
            "These are check/metadata compilation observations, not executable linking, release builds or test execution.",
        ],
        "revisions": {},
    }
    failed = False
    try:
        # Warm dependency sources for both variants before either compilation.
        for role, root in roots.items():
            commit = capture(["git", "rev-parse", "HEAD"], root)
            start = time.perf_counter()
            result = subprocess.run(["cargo", "fetch", "--locked", "--target", "x86_64-pc-windows-msvc"], cwd=root, env=env, check=False)
            report["revisions"][role] = {"commit": commit, "dependencyPreparationSeconds": time.perf_counter() - start, "dependencyPreparationExitCode": result.returncode, "checks": []}
            if result.returncode:
                failed = True
                raise RuntimeError(f"{role} dependency preparation failed")
        for role, root in roots.items():
            target = target_root / role
            if target.exists():
                raise RuntimeError(f"Target must start absent: {target}")
            role_env = {**env, "CARGO_TARGET_DIR": str(target)}
            for label, arguments in commands:
                command = ["cargo", *arguments, "--locked", "--offline", "--timings"]
                print(f"::group::{role}: {label}", flush=True)
                start = time.perf_counter()
                result = subprocess.run(command, cwd=root, env=role_env, check=False)
                elapsed = time.perf_counter() - start
                print("::endgroup::", flush=True)
                report["revisions"][role]["checks"].append({"label": label, "command": command, "seconds": elapsed, "exitCode": result.returncode})
                timing_dir = output / role / label
                timing_dir.mkdir(parents=True)
                for timing in (target / "cargo-timings").glob("*.html"):
                    shutil.copy2(timing, timing_dir / timing.name)
                if result.returncode:
                    failed = True
                    raise RuntimeError(f"{role} {label} failed")
    finally:
        report["finishedAt"] = datetime.now(timezone.utc).isoformat()
        (output / "report.json").write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
        summary = ["## Matched Windows compilation observation", "", *report["limitations"], "", "| Revision | Check | Seconds | Exit code |", "|---|---|---:|---:|"]
        for role, revision in report["revisions"].items():
            summary.append(f"| {role} `{revision['commit']}` | dependency preparation (outside compilation) | {revision['dependencyPreparationSeconds']:.2f} | {revision['dependencyPreparationExitCode']} |")
            for check in revision["checks"]:
                summary.append(f"| {role} | {check['label']} | {check['seconds']:.2f} | {check['exitCode']} |")
        text = "\n".join(summary) + "\n"
        (output / "summary.md").write_text(text, encoding="utf-8")
        if os.environ.get("GITHUB_STEP_SUMMARY"):
            with open(os.environ["GITHUB_STEP_SUMMARY"], "a", encoding="utf-8") as stream:
                stream.write(text)
        shutil.rmtree(target_root)
    return 1 if failed else 0


if __name__ == "__main__":
    raise SystemExit(main())
