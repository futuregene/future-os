#!/usr/bin/env python3
"""Guard: PR caches and optional compile-only main seeds stay compatible.

PR runs save merge-ref caches for their own subsequent runs. A manually
requested prewarm on main can provide shared seeds for new PRs. Cache identity
inputs and compiled targets must match; save policies differ by workflow.
Prewarm test commands use --no-run, including native lifecycle fixtures.
The guard also prevents automatic prewarm triggers and test execution there.

Parsing is line based so the gate needs no third-party Python YAML package.
Structural changes fail loudly rather than silently disabling the guard.
"""

from __future__ import annotations

import argparse
import re
import sys
from dataclasses import dataclass, field
from pathlib import Path

CACHE_ACTION = "Swatinem/rust-cache@"
MAIN_ONLY = "${{ github.ref == 'refs/heads/main' }}"
CI_WRITES = "${{ github.event_name == 'pull_request' || github.event_name == 'workflow_dispatch' }}"

# Cargo subcommands whose output the cache carries. `fmt` is excluded: it emits
# nothing a later job reuses.
COMPARED_SUBCOMMANDS = {"check", "clippy", "test"}

# The same lifecycle targets are run in PR CI and compiled by manual prewarm.
LIFECYCLE_SCRIPT = "scripts/ci/check-embedded-agent.py"

# Steps this guard itself owns: they read workflow files, are not cache
# content, and are not expected in the prewarm.
GUARD_SCRIPTS = ("check-prewarm-cache.py", "test-prewarm-cache-config.py")

CI_PATH = ".github/workflows/ci.yml"
PREWARM_PATH = ".github/workflows/prewarm.yml"

INPUT_ENTRY = re.compile(r"^([A-Za-z0-9_-]+):(?: +(.*))?$")
STEP_ENTRY = re.compile(r"^([A-Za-z0-9_-]+):(?: +(.*))?$")


class Finding(Exception):
    """A drift that must fail the guard."""


@dataclass
class CacheStep:
    line: int
    job: str
    inputs: dict[str, str]

    @property
    def shared_key(self) -> str | None:
        return self.inputs.get("shared-key")


@dataclass
class Job:
    name: str
    line: int
    lines: list[str] = field(default_factory=list)


def indent_of(line: str) -> int:
    return len(line) - len(line.lstrip(" "))


def normalize(command: str) -> str:
    """Compare compiled targets, treating test and test --no-run alike."""
    return " ".join(sorted(part for part in command.split() if part != "--no-run"))


def split_jobs(lines: list[str]) -> dict[str, Job]:
    jobs: dict[str, Job] = {}
    in_jobs = False
    current: Job | None = None
    for number, line in enumerate(lines, start=1):
        if not in_jobs:
            if line.rstrip() == "jobs:":
                in_jobs = True
            continue
        stripped = line.strip()
        if indent_of(line) == 2 and stripped.endswith(":") and not stripped.startswith("#"):
            current = Job(name=stripped[:-1], line=number)
            jobs[current.name] = current
            continue
        if current is not None:
            current.lines.append(line)
    if not jobs:
        raise Finding("no jobs found — the file's structure changed, so this guard cannot judge it")
    return jobs


def parse_cache_steps(path: str, lines: list[str], jobs: dict[str, Job]) -> list[CacheStep]:
    """Every `Swatinem/rust-cache` step in the file, with its `with:` inputs."""
    job_of_line: dict[int, str] = {}
    for job in jobs.values():
        for offset in range(len(job.lines)):
            job_of_line[job.line + 1 + offset] = job.name

    steps: list[CacheStep] = []
    index = 0
    while index < len(lines):
        stripped = lines[index].strip()
        dash = stripped.startswith("- ")
        if dash:
            stripped = stripped[2:]
        if not (stripped.startswith("uses:") and CACHE_ACTION in stripped):
            index += 1
            continue
        # A step's other keys sit at the same column as `uses:` (or one deeper),
        # while the next step's `-` and the next job sit further left.
        uses_indent = indent_of(lines[index]) + (2 if dash else 0)
        number = index + 1
        inputs: dict[str, str] = {}
        index += 1
        while index < len(lines) and lines[index].strip() and indent_of(lines[index]) >= uses_indent:
            entry = lines[index].strip()
            if entry.startswith("-"):
                break
            if entry.startswith("#"):
                index += 1
                continue
            match = STEP_ENTRY.match(entry)
            if not match:
                raise Finding(f"{path}:{index + 1}: cannot parse step key {entry!r}")
            key, inline = match.group(1), (match.group(2) or "")
            key_indent = indent_of(lines[index])
            index += 1
            if key != "with":
                continue
            if inline:
                raise Finding(f"{path}:{number}: unexpected inline `with:` — cannot parse")
            while index < len(lines) and lines[index].strip() and indent_of(lines[index]) > key_indent:
                inner_line = lines[index].strip()
                if inner_line.startswith("#"):
                    index += 1
                    continue
                inner = INPUT_ENTRY.match(inner_line)
                if not inner:
                    raise Finding(f"{path}:{index + 1}: cannot parse cache input {inner_line!r}")
                name, value = inner.group(1), (inner.group(2) or "")
                inner_indent = indent_of(lines[index])
                index += 1
                if value and value != "|":
                    inputs[name] = value.strip()
                    continue
                block: list[str] = []
                while index < len(lines) and lines[index].strip() and indent_of(lines[index]) > inner_indent:
                    block.append(lines[index].strip())
                    index += 1
                inputs[name] = "\n".join(block)
        job = job_of_line.get(number)
        if job is None:
            raise Finding(f"{path}:{number}: cache step outside any job")
        steps.append(CacheStep(line=number, job=job, inputs=inputs))
    if not steps:
        raise Finding(f"{path}: no `{CACHE_ACTION}` step found — this guard cannot judge cache keys")
    return steps


def job_commands(job: Job) -> set[str]:
    """The cargo/script commands of a job that decide what its entry holds."""
    commands: set[str] = set()
    for line in job.lines:
        text = line.strip()
        if text.startswith("run:"):
            # `run: cargo test ...` on one line; a `run: |` block's commands
            # arrive as bare lines further down.
            text = text[4:].strip()
        if "${{" in text:
            continue
        if text.startswith("packages:"):
            # A rust-tests matrix entry: the group's `cargo test` invocation.
            commands.add(normalize(f"cargo test {text.split(':', 1)[1].strip()}"))
            continue
        if text.startswith("cargo "):
            parts = text.split()
            if len(parts) < 2 or parts[1] not in COMPARED_SUBCOMMANDS:
                continue
            if "--test " in text:
                # A narrowed single-target run: a subset of the full-suite
                # command in the same job, which the prewarm runs.
                continue
            commands.add(normalize(text))
            continue
        if text.startswith(("python ", "python3 ")):
            if any(script in text for script in GUARD_SCRIPTS):
                continue
            commands.add(normalize(text))
    return commands


def commands_by_key_job(steps: list[CacheStep], jobs: dict[str, Job]) -> dict[str, dict[str, set[str]]]:
    """Commands run per shared-key, split by the job that runs them."""
    by_key: dict[str, dict[str, set[str]]] = {}
    for step in steps:
        if step.shared_key is None:
            continue
        by_key.setdefault(step.shared_key, {})[step.job] = job_commands(jobs[step.job])
    return by_key


def compare_commands(
    path: str,
    by_key: dict[str, dict[str, set[str]]],
    findings: list[str],
) -> dict[str, set[str]]:
    """Per key: every writer runs the same commands; return the union."""
    union: dict[str, set[str]] = {}
    for shared_key, per_job in sorted(by_key.items()):
        union[shared_key] = set().union(*per_job.values())
        if len(per_job) > 1:
            first_job = sorted(per_job)[0]
            for job, commands in sorted(per_job.items())[1:]:
                if commands != per_job[first_job]:
                    findings.append(
                        f"key {shared_key!r}: jobs {first_job!r} and {job!r} in {path} run different commands, so one "
                        f"of them writes an entry the other's readers cannot fully reuse: "
                        f"{sorted(per_job[first_job] ^ commands)}"
                    )
    return union


def describe(inputs: dict[str, str], keys: set[str]) -> str:
    return ", ".join(f"{key}={inputs.get(key, '<unset>')!r}" for key in sorted(keys))


def collect_by_key(steps: list[CacheStep], path: str, findings: list[str]) -> dict[str, CacheStep]:
    by_key: dict[str, CacheStep] = {}
    for step in steps:
        if step.shared_key is None:
            findings.append(
                f"{path}:{step.line}: cache step has no `shared-key`; an explicit key is what lets "
                f"{PREWARM_PATH} write the same entry independently of job names"
            )
            continue
        previous = by_key.get(step.shared_key)
        if previous is not None and previous.inputs != step.inputs:
            findings.append(
                f"{path}: cache steps for key {step.shared_key!r} (lines {previous.line}, {step.line}) disagree "
                f"on their inputs; one key holds one configuration"
            )
        by_key[step.shared_key] = step
    return by_key


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--root", default=None, help="repository root (default: two levels above this script)")
    args = parser.parse_args()
    root = Path(args.root).resolve() if args.root else Path(__file__).resolve().parents[2]

    findings: list[str] = []
    keys: list[str] = []
    try:
        parsed: dict[str, tuple[list[CacheStep], dict[str, Job]]] = {}
        for relative in (CI_PATH, PREWARM_PATH):
            path = root / relative
            if not path.is_file():
                raise Finding(f"{relative} is missing")
            lines = path.read_text(encoding="utf-8").splitlines()
            jobs = split_jobs(lines)
            parsed[relative] = (parse_cache_steps(relative, lines, jobs), jobs)

        ci_steps, ci_jobs = parsed[CI_PATH]
        prewarm_steps, prewarm_jobs = parsed[PREWARM_PATH]

        for path, steps, policy in (
            (CI_PATH, ci_steps, CI_WRITES),
            (PREWARM_PATH, prewarm_steps, MAIN_ONLY),
        ):
            for step in steps:
                for family, dimension in (
                    ("rust-tests-workspace", "filter"),
                    ("rust-tests-desktop", "flavor"),
                ):
                    if step.shared_key and step.shared_key.startswith(family):
                        partition = family + "-${{ matrix." + dimension + " }}"
                        if step.shared_key != partition:
                            findings.append(
                                f"{path}:{step.line}: {family} requires matrix cache partition {partition!r}"
                            )
                if "key" in step.inputs:
                    findings.append(
                        f"{path}:{step.line}: key is ignored when shared-key is set; put the partition in shared-key"
                    )
                if step.inputs.get("save-if") != policy:
                    findings.append(
                        f"{path}:{step.line}: `save-if` is {step.inputs.get('save-if', '<unset>')!r}; "
                        f"expected {policy!r} for this workflow's cache scope"
                    )

        prewarm_lines = (root / PREWARM_PATH).read_text(encoding="utf-8").splitlines()
        on_index = next((i for i, line in enumerate(prewarm_lines) if line == "on:"), None)
        triggers = []
        if on_index is not None:
            for line in prewarm_lines[on_index + 1:]:
                if line.strip() and not line.startswith((" ", "#")):
                    break
                if indent_of(line) == 2 and line.strip().endswith(":"):
                    triggers.append(line.strip())
        if triggers != ["workflow_dispatch:"]:
            findings.append(f"{PREWARM_PATH}: prewarm must be manual-only (workflow_dispatch)")
        for job in prewarm_jobs.values():
            for line in job.lines:
                command = line.strip().removeprefix("run:").strip()
                if command.startswith("cargo test ") and "--no-run" not in command.split():
                    findings.append(f"{PREWARM_PATH}: test compilation requires --no-run: {command}")
                if command.startswith(("python ", "python3 ")) and LIFECYCLE_SCRIPT in command:
                    if "--no-run" not in command.split():
                        findings.append(f"{PREWARM_PATH}: lifecycle compilation requires --no-run")

        ci_by_key = collect_by_key(ci_steps, CI_PATH, findings)
        prewarm_by_key = collect_by_key(prewarm_steps, PREWARM_PATH, findings)

        for shared_key in sorted(set(ci_by_key) & set(prewarm_by_key)):
            ci_step, prewarm_step = ci_by_key[shared_key], prewarm_by_key[shared_key]
            names = (set(ci_step.inputs) | set(prewarm_step.inputs)) - {"save-if"}
            differing = {
                name
                for name in names
                if ci_step.inputs.get(name, "<unset>") != prewarm_step.inputs.get(name, "<unset>")
            }
            if differing:
                findings.append(
                    f"key {shared_key!r}: cache inputs differ — {describe(ci_step.inputs, differing)} in {CI_PATH} "
                    f"vs {describe(prewarm_step.inputs, differing)} in {PREWARM_PATH}"
                )

        for shared_key in sorted(set(ci_by_key) - set(prewarm_by_key)):
            findings.append(
                f"key {shared_key!r} is read by {CI_PATH} but written by no prewarm job; add a writer to "
                f"{PREWARM_PATH} (or drop the key)"
            )
        for shared_key in sorted(set(prewarm_by_key) - set(ci_by_key)):
            findings.append(
                f"key {shared_key!r} is written by {PREWARM_PATH} but read by no {CI_PATH} job; it would only "
                f"consume the repository cache quota"
            )

        ci_commands = compare_commands(CI_PATH, commands_by_key_job(ci_steps, ci_jobs), findings)
        prewarm_commands = compare_commands(
            PREWARM_PATH, commands_by_key_job(prewarm_steps, prewarm_jobs), findings
        )
        for shared_key in sorted(set(ci_commands) & set(prewarm_commands)):
            missing = ci_commands[shared_key] - prewarm_commands[shared_key]
            extra = prewarm_commands[shared_key] - ci_commands[shared_key]
            if missing:
                findings.append(
                    f"key {shared_key!r}: {CI_PATH} runs commands the prewarm does not, so the entry cannot be "
                    f"reused for them: {sorted(missing)}"
                )
            if extra:
                findings.append(
                    f"key {shared_key!r}: {PREWARM_PATH} runs commands no {CI_PATH} job with that key runs, so it "
                    f"caches artifacts nothing reads: {sorted(extra)}"
                )

        keys = sorted(ci_by_key)
    except Finding as finding:
        findings.append(str(finding))

    if findings:
        print("prewarm cache guard: prewarm.yml and ci.yml have drifted apart", file=sys.stderr)
        for finding in findings:
            print(f"  - {finding}", file=sys.stderr)
        return 1

    print(
        f"prewarm cache guard: {len(keys)} cache keys in sync ({', '.join(keys)}); "
        f"PRs save their own caches; manual main seeds compile tests without running them"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
