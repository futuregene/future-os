#!/usr/bin/env python3
"""Fusion development pilot: local Fusion configuration vs DeepSeek Flash max."""

import argparse
import datetime
import platform
import subprocess
import sys
from concurrent.futures import ThreadPoolExecutor
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
    download.add_argument("--datasets", default="zebra,bbeh,omni,musr")
    plan = commands.add_parser(
        "plan", help="freeze question IDs before observing model results"
    )
    plan.add_argument("--datasets", default="zebra,bbeh,omni")
    plan.add_argument("--per-dataset", type=int, default=1)
    plan.add_argument("--seed", default="future-fusion-pilot-v1")
    plan.add_argument("--out", type=Path, default=ROOT / "suites/smoke.json")
    check = commands.add_parser(
        "check-suite",
        help="verify committed frozen questions without downloads or model calls",
    )
    check.add_argument("--suite", type=Path, default=ROOT / "suites/dev-12.json")
    run = commands.add_parser("run", help="execute the two real model arms")
    run.add_argument("--suite", type=Path, default=ROOT / "suites/smoke.json")
    run.add_argument(
        "--binary", type=Path, default=ROOT.parent.parent / "target/debug/future"
    )
    run.add_argument("--runtime", type=Path)
    run.add_argument("--out", type=Path, required=True)
    run.add_argument(
        "--timeout",
        type=int,
        default=0,
        help="overall wait seconds; 0 disables this benchmark deadline",
    )
    run.add_argument("--repeats", type=int, default=1)
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
        if args.per_dataset < 1:
            parser.error("--per-dataset must be positive")
        verify_data()
        rows = sample_cases(args.datasets.split(","), args.per_dataset, args.seed)
        write_json(
            args.out,
            {
                "schema": 1,
                "purpose": "development pilot; not a headline capability claim",
                "seed": args.seed,
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
    elif args.command == "run":
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
        configuration = client.preflight()
        args.out.mkdir(parents=True, exist_ok=False)
        # Even a caller-chosen output path must not expose model responses to
        # a later broad git add. This marker is itself ignored as well.
        (args.out / ".gitignore").write_text("*\n", encoding="utf-8")
        harness = args.out / "harness"
        harness.mkdir()
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
        implementation.mkdir()
        repository = ROOT.parent.parent
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
            "host/src/fusion/resilience.rs",
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
                ["git", "rev-parse", "HEAD"], cwd=ROOT, text=True
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
        for repeat in range(args.repeats):
            for planned in suite["cases"]:
                if digest(
                    client.call(["fusion", "settings", "--json"])["configuration"]
                ) != digest(configuration["fusion"]["configuration"]):
                    raise ValueError("Fusion configuration changed during the run")
                case = cases[planned["id"]]
                arms = ["fusion", "single"]
                if int(digest([suite["seed"], repeat, case["id"]])[:8], 16) % 2:
                    arms.reverse()
                print(f"Running {case['id']} repeat {repeat + 1}: {arms}", flush=True)
                with ThreadPoolExecutor(max_workers=2) as pool:
                    futures = [
                        pool.submit(
                            client.run_case,
                            case,
                            arm,
                            args.out
                            / f"r{repeat}-{case['id'].replace(':', '-')}-{arm}",
                            args.timeout,
                        )
                        for arm in arms
                    ]
                    for job in futures:
                        result = job.result()
                        print(
                            f"  {result['arm']}: {result['status']}; correct={result['grade']['correct']}; seconds={result['elapsed_seconds']:.1f}"
                            + (f"; {result['error']}" if result.get("error") else ""),
                            flush=True,
                        )
        from reporting import generate

        generate(args.out)
    elif args.command == "report":
        from reporting import generate

        generate(args.run, args.public_out)
    elif args.command == "audit":
        from audit import collect

        collect(args.run, args.binary, args.runtime)


if __name__ == "__main__":
    main()
