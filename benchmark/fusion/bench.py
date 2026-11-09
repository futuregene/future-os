#!/usr/bin/env python3
"""Fusion development pilot: local Fusion configuration vs DeepSeek Flash max."""

import argparse
import datetime
import platform
import subprocess
import sys
from pathlib import Path

from datasets import (
    ROOT,
    digest,
    freeze_questions,
    load_cases,
    load_frozen_questions,
    prepare,
    read_json,
    sample_cases,
    write_json,
)
from future_runner import Future, verify_data


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    download = commands.add_parser(
        "prepare", help="download pinned official data; no model calls"
    )
    download.add_argument("--datasets", default="bbeh,omni")
    plan = commands.add_parser(
        "plan", help="freeze question IDs before observing model results"
    )
    plan.add_argument("--datasets", default="bbeh,omni")
    plan.add_argument("--per-dataset", type=int, default=1)
    plan.add_argument("--counts", help="per-dataset counts, e.g. bbeh=460,omni=540")
    plan.add_argument(
        "--exclude",
        default="",
        help="comma-separated unusable case IDs excluded before sampling",
    )
    plan.add_argument("--seed", default="future-fusion-pilot-v1")
    plan.add_argument("--out", type=Path, default=ROOT / "suites/smoke.json")
    check = commands.add_parser(
        "check-suite",
        help="verify committed frozen questions without downloads or model calls",
    )
    check.add_argument("--suite", type=Path, default=ROOT / "suites/fusion-10.json")
    run = commands.add_parser(
        "run", help="execute selected arms or reuse a frozen baseline"
    )
    run.add_argument("--suite", type=Path, default=ROOT / "suites/fusion-10.json")
    run.add_argument(
        "--binary", type=Path, default=ROOT.parent.parent / "target/debug/future"
    )
    run.add_argument("--runtime", type=Path)
    run.add_argument(
        "--source-root",
        type=Path,
        default=ROOT.parent.parent,
        help="source tree used to build the supplied binary; snapshotted for provenance",
    )
    run.add_argument("--out", type=Path, required=True)
    run.add_argument(
        "--timeout",
        type=int,
        default=0,
        help="overall wait seconds; 0 disables this benchmark deadline",
    )
    run.add_argument("--repeats", type=int, default=1)
    run.add_argument(
        "--freeze-out",
        type=Path,
        help="after a completed single baseline, freeze the 14-wrong/6-correct challenge",
    )
    run.add_argument("--arms", choices=["both", "single", "fusion"], default="fusion")
    run.add_argument(
        "--baseline",
        type=Path,
        help="completed single-arm run reused for Fusion comparison",
    )
    run.add_argument("--jobs", type=int, default=1, help="concurrent questions")
    run.add_argument(
        "--http-502-retries",
        type=int,
        default=3,
        help="additional single-model attempts after a confirmed HTTP 502",
    )
    run.add_argument(
        "--http-502-retry-delay",
        type=float,
        default=2,
        help="initial backoff seconds, doubled up to 30 seconds",
    )
    run.add_argument(
        "--resume",
        action="store_true",
        help="continue this run without resubmitting accepted questions",
    )
    freeze = commands.add_parser(
        "freeze", help="freeze 14 wrong + 6 correct from a completed baseline"
    )
    freeze.add_argument("--baseline", type=Path, required=True)
    freeze.add_argument("--seed", default="future-fusion-errors-v1")
    freeze.add_argument("--out", type=Path, required=True)
    freeze.add_argument("--wrong", type=int, default=14)
    freeze.add_argument("--correct", type=int, default=6)
    regrade = commands.add_parser(
        "regrade-baseline",
        help="create a new offline grading revision; preserve original answers and scores",
    )
    regrade.add_argument("--source", type=Path, required=True)
    regrade.add_argument("--out", type=Path, required=True)
    report = commands.add_parser(
        "report", help="offline paired comparison, no model calls"
    )
    report.add_argument("run", type=Path)
    report.add_argument(
        "--public-out",
        type=Path,
        help="write a sanitized JSON/CSV/Markdown export at this prefix",
    )
    audit = commands.add_parser(
        "audit", help="read-only terminal evidence collection; no model calls"
    )
    audit.add_argument("run", type=Path)
    audit.add_argument(
        "--binary", type=Path, default=ROOT.parent.parent / "target/debug/future"
    )
    audit.add_argument("--runtime", type=Path)
    args = parser.parse_args()
    if args.command == "prepare":
        prepare(args.datasets.split(","))
    elif args.command == "plan":
        excluded = set(filter(None, args.exclude.split(",")))
        if excluded and not args.counts:
            parser.error("--exclude requires --counts")
        if args.per_dataset < 1:
            parser.error("--per-dataset must be positive")
        verify_data()
        if args.counts:
            counts = dict(item.split("=") for item in args.counts.split(","))
            if any(int(count) < 1 for count in counts.values()):
                parser.error("--counts values must be positive")
            raw_pools = {name: load_cases(name) for name in counts}
            known = {row["id"] for pool in raw_pools.values() for row in pool}
            if excluded - known:
                parser.error(
                    "Unknown excluded case IDs: " + ", ".join(sorted(excluded - known))
                )
            pools = {
                name: [row for row in pool if row["id"] not in excluded]
                for name, pool in raw_pools.items()
            }
            if any(int(count) > len(pools[name]) for name, count in counts.items()):
                parser.error("--counts exceeds eligible dataset size")
            rows = [
                row
                for name, count in counts.items()
                for row in sorted(
                    pools[name], key=lambda row: digest([args.seed, row["id"]])
                )[: int(count)]
            ]
        else:
            rows = sample_cases(args.datasets.split(","), args.per_dataset, args.seed)
        if args.out.exists() or args.out.with_suffix(".cases.json").exists():
            raise ValueError("Suite already exists; use a new path")
        write_json(
            args.out,
            {
                "schema": 1,
                "purpose": "development pilot; not a headline capability claim",
                "seed": args.seed,
                "excluded_case_ids": sorted(excluded),
                "sampling": "uniform hash ordering within each dataset"
                if args.counts
                else "round robin over dataset strata",
                "sources_lock_sha256": digest(read_json(ROOT / "sources.lock.json")),
                "cases": [
                    {
                        "id": row["id"],
                        "dataset": row["dataset"],
                        "stratum": row["stratum"],
                        "question_sha256": digest(row["question"]),
                        "answer_sha256": digest(row["answer"]),
                    }
                    for row in rows
                ],
            },
        )
        freeze_questions(args.out, read_json(args.out), rows)
        print(f"Frozen {len(rows)} cases: {args.out}")
    elif args.command == "check-suite":
        suite = read_json(args.suite)
        _, cases = load_frozen_questions(args.suite, suite)
        if any(row["scorer"] == "bbeh" for row in cases.values()):
            from scoring import bbeh_module

            bbeh_module()
        print(
            f"Verified {len(cases)} frozen questions and scoring sources: {args.suite}"
        )
    elif args.command == "regrade-baseline":
        from baseline import regrade_baseline

        regrade_baseline(args.source, args.out)
    elif args.command == "freeze":
        from baseline import freeze_challenge

        freeze_challenge(args.baseline, args.out, args.seed, args.wrong, args.correct)
    elif args.command == "run":
        if args.http_502_retries < 0 or not 0 <= args.http_502_retry_delay <= 30:
            parser.error(
                "HTTP 502 retries must be nonnegative; retry delay must be between 0 and 30 seconds"
            )
        if args.freeze_out and (args.arms != "single" or args.repeats != 1):
            parser.error("--freeze-out requires --arms single and --repeats 1")
        if args.jobs < 1:
            parser.error("--jobs must be positive")
        if args.baseline and (args.arms != "fusion" or args.repeats != 1):
            parser.error("--baseline requires --arms fusion and --repeats 1")
        if args.repeats < 1 or args.timeout < 0:
            parser.error("repeats must be positive; timeout must be nonnegative")
        suite = read_json(args.suite)
        if suite["sources_lock_sha256"] != digest(
            read_json(ROOT / "sources.lock.json")
        ):
            raise ValueError("Suite source lock mismatch; create a new suite")
        question_bundle = args.suite.with_suffix(".cases.json")
        if question_bundle.exists():
            bundle, cases = load_frozen_questions(args.suite, suite)
            data_manifest = bundle["upstream_files"]
        else:
            verify_data()
            cases = {
                row["id"]: row
                for name in {row["dataset"] for row in suite["cases"]}
                for row in load_cases(name)
            }
            data_manifest = read_json(ROOT / "data/manifest.json")
        for row in suite["cases"]:
            source = cases[row["id"]]
            if (
                digest(source["question"]) != row["question_sha256"]
                or digest(source["answer"]) != row["answer_sha256"]
            ):
                raise ValueError(f"Suite content mismatch: {row['id']}")
        client = Future(args.binary, args.runtime)
        configuration = client.preflight(single_only=args.arms == "single")
        arms = ["fusion", "single"] if args.arms == "both" else [args.arms]
        reference = None
        if args.baseline:
            from baseline import reference_for

            reference = reference_for(args.baseline, suite, cases)
        if (
            args.arms == "fusion"
            and suite.get("selection", {}).get("kind") == "baseline_outcomes"
            and reference is None
        ):
            raise ValueError("Outcome-selected suite requires its frozen --baseline")
        if args.resume:
            from baseline import validate_resume

            validate_resume(args.out, suite, configuration, args, arms, reference)
        args.out.mkdir(parents=True, exist_ok=args.resume)
        if not args.resume:
            # Even a caller-chosen output path must not expose model responses to
            # a later broad git add. This marker is itself ignored as well.
            (args.out / ".gitignore").write_text("*\n", encoding="utf-8")
            harness = args.out / "harness"
            harness.mkdir(exist_ok=args.resume)
            if question_bundle.exists():
                (args.out / "frozen-questions.json").write_bytes(
                    question_bundle.read_bytes()
                )
            files = list(ROOT.glob("*.py")) + [
                ROOT / "requirements.txt",
                ROOT / "sources.lock.json",
            ]
            for path in files:
                (harness / path.name).write_bytes(path.read_bytes())
            harness_hashes = {path.name: digest(path.read_bytes()) for path in files}
            implementation = args.out / "implementation"
            implementation.mkdir(exist_ok=args.resume)
            repository = args.source_root.resolve()
            implementation_files = [
                "agent/src/agent/run_loop.rs",
                "agent/src/rpc/session_prompt.rs",
                "agent/src/fusion.rs",
                "agent/src/llm/mod.rs",
                "agent/src/types/mod.rs",
                "agent/src/rpc/broadcaster/mod.rs",
                "agent/src/rpc/session.rs",
                "orchestration/fusion/src/config.rs",
                "orchestration/fusion/src/resilience.rs",
                "orchestration/fusion/src/store.rs",
                "orchestration/fusion/src/types.rs",
                "host/src/fusion.rs",
                "host/src/fusion/mapping.rs",
                "host/src/fusion/resilience.rs",
                "host/src/fusion/review.rs",
                "rust-toolchain.toml",
                "benchmark/fusion/third_party/bbeh/evaluate.py",
                "benchmark/fusion/third_party/bbeh/LICENSE",
                "benchmark/fusion/third_party/bbeh/source.json",
            ]
            implementation_hashes = {}
            for name in implementation_files:
                path = repository / name
                destination = implementation / name
                destination.parent.mkdir(parents=True, exist_ok=True)
                destination.write_bytes(path.read_bytes())
                implementation_hashes[name] = digest(path.read_bytes())
            try:
                git = subprocess.check_output(
                    ["git", "rev-parse", "HEAD"], cwd=repository, text=True
                ).strip()
            except (subprocess.SubprocessError, FileNotFoundError):
                git = None
            write_json(
                args.out / "manifest.json",
                {
                    "schema": 1,
                    "started_at_utc": datetime.datetime.now(
                        datetime.timezone.utc
                    ).isoformat(),
                    "git_commit": git,
                    "source_worktree_dirty": bool(
                        subprocess.check_output(
                            ["git", "status", "--porcelain", "-uno"],
                            cwd=repository,
                            text=True,
                        )
                    ),
                    "harness_sha256": harness_hashes,
                    "implementation_sha256": implementation_hashes,
                    "binary_sha256": digest(args.binary.read_bytes()),
                    "python": sys.version,
                    "platform": platform.platform(),
                    "stored_local_fusion_settings": read_json(args.runtime).get(
                        "stored_local_fusion_settings"
                    )
                    if args.runtime
                    else None,
                    "suite": suite,
                    "configuration": configuration,
                    "repeats": args.repeats,
                    "arms": arms,
                    "baseline_reference": reference,
                    "jobs": args.jobs,
                    "http_502_retries": args.http_502_retries,
                    "http_502_retry_delay": args.http_502_retry_delay,
                    "timeout_seconds": args.timeout,
                    "protocol": "daily final answer primary; normal /fusion entry and automatic tool selection; report activation and kernel diagnostics separately; no external tools; no equal-budget claim",
                    "execution_profile": "daily",
                    "primary_endpoint": "daily_final_answer",
                    "question_bundle_sha256": digest(question_bundle.read_bytes())
                    if question_bundle.exists()
                    else None,
                    "data_manifest": data_manifest,
                },
            )
        from baseline import execute

        execute(client, args, suite, cases, configuration, arms)
        if args.freeze_out:
            from baseline import freeze_challenge

            freeze_challenge(args.out, args.freeze_out, "future-fusion-errors-v1")
    elif args.command == "report":
        from reporting import generate

        generate(args.run, args.public_out)
    elif args.command == "audit":
        from audit import collect

        collect(args.run, args.binary, args.runtime)


if __name__ == "__main__":
    main()
