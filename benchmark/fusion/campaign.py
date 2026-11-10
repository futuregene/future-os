#!/usr/bin/env python3
"""Run comparable single-model cohorts, with network makeups after the first pass."""

import argparse
import csv
import datetime
import re
from concurrent.futures import FIRST_COMPLETED, ThreadPoolExecutor, wait
from pathlib import Path

from datasets import ROOT, digest, load_frozen_questions, read_json, write_json
from future_runner import Future
from scoring import bbeh_module

NETWORK_HTTP = {408, 429, 500, 502, 503, 504}


def network_failure(result):
    # Only an accepted run's terminal ledger can authorize a new generation.
    # A CLI read failure or an ambiguous submission might still be running.
    if result["status"] == "completed" or result.get("submission_uncertain"):
        return False
    error = result.get("upstream_error", "").lower()
    if not error or any(
        word in error
        for word in ("insufficient_quota", "quota_exceeded", "billing_hard_limit")
    ):
        return False
    return result.get("http_status") in NETWORK_HTTP or any(
        word in error
        for word in (
            "[upstream_disconnected]",
            "[response_timeout]",
            "connection reset",
            "connection refused",
            "dns error",
            "error sending request",
        )
    )


def attempt_path(out, group, case_id, attempt):
    return out / "groups" / group / case_id.replace(":", "-") / f"attempt-{attempt}"


def read_profiles(path):
    profiles = read_json(path)
    if not isinstance(profiles, dict) or not profiles:
        raise ValueError("Profiles must be a nonempty object keyed by group name")
    for group, profile in profiles.items():
        if not re.fullmatch(r"[a-z0-9][a-z0-9-]*", group):
            raise ValueError(f"Invalid profile name: {group}")
        if (
            not isinstance(profile, dict)
            or set(profile) != {"model", "thinking_level"}
            or not all(isinstance(value, str) and value for value in profile.values())
        ):
            raise ValueError(f"Invalid binding for {group}")
    return profiles


def run_window(items, jobs, worker, record):
    # Bound submissions so a protocol, billing, or storage error can stop new work.
    with ThreadPoolExecutor(max_workers=jobs) as pool:
        items = iter(items)
        pending = {pool.submit(worker, item) for item in take(items, jobs)}
        while pending:
            done, pending = wait(pending, return_when=FIRST_COMPLETED)
            for future in done:
                record(*future.result())
            pending.update(pool.submit(worker, item) for item in take(items, len(done)))


def take(items, count):
    for _ in range(count):
        item = next(items, None)
        if item is None:
            return
        yield item


def prepare(args):
    suite = read_json(args.suite)
    bundle, cases = load_frozen_questions(args.suite, suite)
    if any(case["scorer"] == "bbeh" for case in cases.values()):
        bbeh_module()
    identifiers = [case["id"] for case in suite["cases"]]
    config = {
        "suite": suite,
        "bundle_sha256": digest(bundle),
        "binary_sha256": digest(args.binary.read_bytes()),
        "runtime_home": read_json(args.runtime)["home"],
        "profiles": read_profiles(args.profiles),
        "timeout": args.timeout,
        "makeup_passes": args.makeup_passes,
        "jobs": args.jobs,
        "harness_sha256": {
            name: digest((ROOT / name).read_bytes())
            for name in ("campaign.py", "future_runner.py", "scoring.py", "datasets.py")
        },
    }
    manifest = args.out / "manifest.json"
    if manifest.exists():
        if read_json(manifest)["configuration"] != config:
            raise ValueError(
                "Campaign inputs changed; use the frozen settings to resume"
            )
        if (
            digest(read_json(args.out / "frozen-questions.json"))
            != config["bundle_sha256"]
        ):
            raise ValueError("The saved question bundle changed")
    else:
        write_json(args.out / "frozen-questions.json", bundle)
        write_json(
            manifest,
            {
                "configuration": config,
                "started_at_utc": datetime.datetime.now(
                    datetime.timezone.utc
                ).isoformat(),
            },
        )
    return config["profiles"], identifiers, cases


def execute(args):
    profiles, identifiers, cases = prepare(args)
    groups = list(profiles)
    histories = {}
    for group in groups:
        for identifier in identifiers:
            history = []
            for attempt in range(1, args.makeup_passes + 2):
                result = (
                    attempt_path(args.out, group, identifier, attempt) / "result.json"
                )
                if not result.exists():
                    break
                history.append(read_json(result))
            histories[group, identifier] = history
    bindings_path = args.out / "bindings.json"
    bindings = read_json(bindings_path) if bindings_path.exists() else {}
    clients = {}

    def progress(phase, error=None):
        counts = {}
        for group in groups:
            rows = [h for (name, _), h in histories.items() if name == group and h]
            latest = [h[-1] for h in rows]
            counts[group] = {
                "binding": bindings.get(group, {}).get("baseline"),
                "scheduled": len(identifiers),
                "first_pass_done": len(rows),
                "attempts": sum(map(len, rows)),
                "answered": sum(row["status"] == "completed" for row in latest),
                "correct": sum(row["grade"]["correct"] for row in latest),
                "parse_failures": sum(
                    row["status"] == "completed" and not row["grade"]["parsed"]
                    for row in latest
                ),
                "network_pending": sum(network_failure(row) for row in latest),
                "other_errors": sum(
                    row["status"] != "completed" and not network_failure(row)
                    for row in latest
                ),
                "first_pass_network_errors": sum(network_failure(h[0]) for h in rows),
            }
        write_json(
            args.out / "progress.json",
            {
                "phase": phase,
                "updated_at_utc": datetime.datetime.now(
                    datetime.timezone.utc
                ).isoformat(),
                "groups": counts,
                "error": error,
            },
        )

    def bind_clients():
        for group, profile in profiles.items():
            if group in bindings and profile != bindings[group]["baseline"]:
                raise ValueError(f"Previously frozen model binding changed for {group}")
            client = Future(
                args.binary,
                args.runtime,
                profile["model"],
                profile["thinking_level"],
                full_trace=True,
            )
            preflight = client.preflight(single_only=True)
            if group not in bindings:
                bindings[group] = preflight
                write_json(bindings_path, bindings)
            clients[group] = client
            print(
                f"Bound {group}: {profile['model']} {profile['thinking_level']}",
                flush=True,
            )

    def worker(item):
        group, identifier, attempt = item
        path = attempt_path(args.out, group, identifier, attempt)
        result = clients[group].run_case(
            cases[identifier], "single", path, args.timeout, resume=path.exists()
        )
        return item, result

    def record(item, result):
        group, identifier, attempt = item
        histories[group, identifier].append(result)
        state = "deferred" if network_failure(result) else result["status"]
        print(f"{group} {identifier} attempt-{attempt}: {state}", flush=True)
        progress(phase)
        if result["status"] != "completed" and not network_failure(result):
            raise ValueError(
                f"Inspect {attempt_path(args.out, group, identifier, attempt)}"
            )

    try:
        for key, history in histories.items():
            if (
                history
                and history[-1]["status"] != "completed"
                and not network_failure(history[-1])
            ):
                raise ValueError(f"Unresolved non-network failure: {key}")
        bind_clients()
        phase = "first_pass"
        progress(phase)
        run_window(
            (
                (group, identifier, 1)
                for identifier in identifiers
                for group in groups
                if not histories[group, identifier]
            ),
            args.jobs,
            worker,
            record,
        )

        for attempt in range(2, args.makeup_passes + 2):
            phase = f"makeup_{attempt - 1}"
            progress(phase)
            run_window(
                (
                    (group, identifier, attempt)
                    for identifier in identifiers
                    for group in groups
                    if len(histories[group, identifier]) == attempt - 1
                    and network_failure(histories[group, identifier][-1])
                ),
                args.jobs,
                worker,
                record,
            )
        write_results(args.out, histories)
        unresolved = any(network_failure(rows[-1]) for rows in histories.values())
        progress("completed_with_network_errors" if unresolved else "completed")
    except Exception as error:
        progress("stopped", str(error))
        raise


def write_results(out, histories):
    temporary = out / "results.csv.tmp"
    with temporary.open("w", encoding="utf-8", newline="") as handle:
        fields = [
            "group",
            "case_id",
            "status",
            "correct",
            "parsed",
            "attempts",
            "result",
        ]
        writer = csv.DictWriter(handle, fieldnames=fields)
        writer.writeheader()
        for (group, identifier), rows in histories.items():
            result = rows[-1]
            writer.writerow(
                {
                    "group": group,
                    "case_id": identifier,
                    "status": result["status"],
                    "correct": result["grade"]["correct"],
                    "parsed": result["grade"]["parsed"],
                    "attempts": len(rows),
                    "result": str(
                        (
                            attempt_path(out, group, identifier, len(rows))
                            / "result.json"
                        ).relative_to(out)
                    ),
                }
            )
    temporary.replace(out / "results.csv")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ("suite", "binary", "runtime", "profiles", "out"):
        parser.add_argument(f"--{name}", required=True, type=Path)
    parser.add_argument("--jobs", type=int, default=4)
    parser.add_argument("--timeout", type=int, default=0)
    parser.add_argument("--makeup-passes", type=int, default=3)
    args = parser.parse_args()
    if args.jobs < 1 or args.timeout < 0 or args.makeup_passes < 1:
        parser.error("jobs and makeup passes must be positive, timeout nonnegative")
    args.out.mkdir(parents=True, exist_ok=True)
    (args.out / ".gitignore").write_text("*\n", encoding="utf-8")
    lock = args.out / "campaign.lock"
    with lock.open("x") as handle:
        handle.write(
            "Campaign active. Remove this file only after confirming the worker stopped.\n"
        )
    try:
        execute(args)
    finally:
        lock.unlink()


if __name__ == "__main__":
    main()
