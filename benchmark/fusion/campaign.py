#!/usr/bin/env python3
"""Run comparable single-model cohorts, with network makeups after the first pass."""

import argparse
import csv
import datetime
import re
from collections import Counter, defaultdict
from concurrent.futures import FIRST_COMPLETED, ThreadPoolExecutor, wait
from pathlib import Path

from datasets import ROOT, digest, load_frozen_questions, read_json, write_json
from future_runner import Future
from scoring import bbeh_module

NETWORK_HTTP = {408, 429, 500, 502, 503, 504}
NETWORK_INTERRUPTION = ("[upstream_disconnected]", "[response_timeout]")
CASE_OUTCOMES = {"completed", "deferred", "output_limit", "empty_answer"}


def run_ledger_entry(result):
    run_id = result.get("run_id")
    if (
        result["status"] == "completed"
        or run_id is None
        or result.get("submission_uncertain")
    ):
        return {}
    for entry in result.get("parent_usage_ledger", {}).get("ledger", []):
        if entry.get("runId") == run_id:
            return entry
    return {}


def terminal_run_error(result):
    entry = run_ledger_entry(result)
    if entry.get("status") in {"interrupted", "failed", "error"}:
        return entry.get("error") or ""
    return ""


def network_failure(result):
    # A CLI read failure or an ambiguous submission might still be running.
    if (
        result.get("benchmark_deadline_exceeded")
        and not result.get("failure_evidence_error")
        and run_ledger_entry(result).get("status")
        in {"interrupted", "cancelled", "failed", "error"}
    ):
        return True
    terminal_error = terminal_run_error(result).lower()
    if not terminal_error:
        return False
    error = result.get("upstream_error", "").lower()
    if not error:
        return terminal_error.startswith(NETWORK_INTERRUPTION)
    if any(
        word in error
        for word in ("insufficient_quota", "quota_exceeded", "billing_hard_limit")
    ):
        return False
    return result.get("http_status") in NETWORK_HTTP or any(
        word in error
        for word in (
            *NETWORK_INTERRUPTION,
            "connection reset",
            "connection refused",
            "dns error",
            "error sending request",
        )
    )


def output_limit(result):
    return terminal_run_error(result) == "[OUTPUT_LIMIT] finish_length"


def case_status(result):
    if output_limit(result):
        return "output_limit"
    if network_failure(result):
        return "deferred"
    if (
        run_ledger_entry(result).get("status") == "completed"
        and not result.get("failure_evidence_error")
        and f"run {result['run_id']} ended as empty_answer:" in result.get("error", "")
    ):
        return "empty_answer"
    return result["status"]


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


def read_routes(args, profiles):
    if args.routes is None:
        return {}
    routes = read_json(args.routes)
    if set(routes) != set(profiles):
        raise ValueError("Routes must cover exactly the frozen profile groups")
    for group, lanes in routes.items():
        if not isinstance(lanes, list) or not lanes:
            raise ValueError(f"Missing provider routes for {group}")
        for lane in lanes:
            if (
                set(lane) != {"model", "thinking_level", "jobs"}
                or not isinstance(lane["model"], str)
                or not lane["model"]
                or lane["thinking_level"] != profiles[group]["thinking_level"]
                or type(lane["jobs"]) is not int
                or lane["jobs"] < 1
            ):
                raise ValueError(f"Invalid provider route for {group}")
        if len({lane["model"] for lane in lanes}) != len(lanes):
            raise ValueError(f"Duplicate provider route for {group}")
    return routes


def run_window(items, jobs, jobs_per_group, routes, worker, record):
    if jobs_per_group is None and not routes:
        queues = {None: iter(items)}
    else:
        grouped = defaultdict(list)
        for item in items:
            grouped[item[0]].append(item)
        queues = {group: iter(rows) for group, rows in grouped.items()}
    lanes = (
        [
            (f"{group}/{index}", group, route["jobs"])
            for group, rows in routes.items()
            for index, route in enumerate(rows)
        ]
        if routes
        else [
            (group, group, jobs if jobs_per_group is None else jobs_per_group)
            for group in queues
        ]
    )
    # Bound submissions so a protocol, billing, or storage error can stop new work.
    with ThreadPoolExecutor(max_workers=jobs) as pool:
        pending = {}
        for lane, group, slots in lanes:
            queue = queues.get(group, iter(()))
            for _ in range(slots):
                item = next(queue, None)
                if item is not None:
                    pending[pool.submit(worker, item, lane)] = (lane, group)
        while pending:
            done, _ = wait(pending, return_when=FIRST_COMPLETED)
            for future in done:
                record(*future.result())
            for future in done:
                lane, group = pending.pop(future)
                item = next(queues[group], None)
                if item is not None:
                    pending[pool.submit(worker, item, lane)] = (lane, group)


def prepare(args):
    suite = read_json(args.suite)
    bundle, cases = load_frozen_questions(args.suite, suite)
    if any(case["scorer"] == "bbeh" for case in cases.values()):
        bbeh_module()
    identifiers = [case["id"] for case in suite["cases"]]
    profiles = read_profiles(args.profiles)
    routes = read_routes(args, profiles)
    jobs = args.jobs
    if args.jobs_per_group is not None:
        jobs = len(profiles) * args.jobs_per_group
    if routes:
        jobs = sum(lane["jobs"] for lanes in routes.values() for lane in lanes)
    config = {
        "suite": suite,
        "bundle_sha256": digest(bundle),
        "binary_sha256": digest(args.binary.read_bytes()),
        "runtime_home": read_json(args.runtime)["home"],
        "profiles": profiles,
        "timeout": args.timeout,
        "makeup_passes": args.makeup_passes,
        "jobs": jobs,
        "jobs_per_group": args.jobs_per_group,
        "harness_sha256": {
            name: digest((ROOT / name).read_bytes())
            for name in ("campaign.py", "future_runner.py", "scoring.py", "datasets.py")
        },
    }
    manifest = args.out / "manifest.json"
    if manifest.exists():
        original = read_json(manifest)["configuration"]
        # The scheduler can evolve without changing what any model sees or how it is scored.
        if frozen_inputs(original, args.allow_runner_update) != frozen_inputs(
            config, args.allow_runner_update
        ):
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
    runs_path = args.out / "executions.json"
    runs = read_json(runs_path) if runs_path.exists() else []
    runs.append(
        {
            "started_at_utc": datetime.datetime.now(datetime.timezone.utc).isoformat(),
            "jobs": jobs,
            "jobs_per_group": args.jobs_per_group,
            "campaign_sha256": config["harness_sha256"]["campaign.py"],
            "harness_sha256": config["harness_sha256"],
            "allow_runner_update": args.allow_runner_update,
            "timeout": args.timeout,
            "routes": routes,
        }
    )
    write_json(runs_path, runs)
    return profiles, identifiers, cases, jobs, routes


def frozen_inputs(config, allow_runner_update=False):
    inputs = {
        key: value
        for key, value in config.items()
        if key not in {"jobs", "jobs_per_group", "timeout"}
    }
    inputs["harness_sha256"] = {
        name: sha
        for name, sha in config["harness_sha256"].items()
        if name != "campaign.py"
        and not (allow_runner_update and name == "future_runner.py")
    }
    return inputs


def execute(args):
    profiles, identifiers, cases, jobs, routes = prepare(args)
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
            states = Counter(map(case_status, latest))
            counts[group] = {
                "binding": bindings.get(group, {}).get("baseline"),
                "scheduled": len(identifiers),
                "first_pass_done": len(rows),
                "attempts": sum(map(len, rows)),
                "answered": states["completed"],
                "correct": sum(row["grade"]["correct"] for row in latest),
                "parse_failures": sum(
                    row["status"] == "completed" and not row["grade"]["parsed"]
                    for row in latest
                ),
                "network_pending": states["deferred"],
                "output_limits": states["output_limit"],
                "empty_answers": states["empty_answer"],
                "deadline_timeouts": sum(
                    bool(row.get("benchmark_deadline_exceeded")) for row in latest
                ),
                "other_errors": states["error"],
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
                "jobs": jobs,
                "jobs_per_group": args.jobs_per_group,
                "routes": routes,
                "timeout": args.timeout,
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
        lane_bindings = {}
        for group, lanes in routes.items():
            for index, route in enumerate(lanes):
                key = f"{group}/{index}"
                client = Future(
                    args.binary,
                    args.runtime,
                    route["model"],
                    route["thinking_level"],
                    full_trace=True,
                )
                lane_bindings[key] = client.preflight(single_only=True)
                clients[key] = client
                print(
                    f"Bound route {key}: {route['model']} {route['thinking_level']} jobs={route['jobs']}",
                    flush=True,
                )
        if lane_bindings:
            route_log = args.out / "route-bindings.json"
            snapshots = read_json(route_log) if route_log.exists() else []
            snapshots.append(
                {
                    "at_utc": datetime.datetime.now(datetime.timezone.utc).isoformat(),
                    "bindings": lane_bindings,
                }
            )
            write_json(route_log, snapshots)

    def worker(item, lane):
        group, identifier, attempt = item
        path = attempt_path(args.out, group, identifier, attempt)
        client = clients[lane or group]
        if (path / "ack.json").exists():
            binding_path = path / "model-binding.json"
            binding = (
                read_json(binding_path) if binding_path.exists() else profiles[group]
            )
            client = Future(
                args.binary,
                args.runtime,
                binding["model"],
                binding["thinking_level"],
                full_trace=True,
            )
        result = client.run_case(
            cases[identifier], "single", path, args.timeout, resume=path.exists()
        )
        return item, result

    def record(item, result):
        group, identifier, attempt = item
        histories[group, identifier].append(result)
        state = case_status(result)
        print(f"{group} {identifier} attempt-{attempt}: {state}", flush=True)
        progress(phase)
        if state not in CASE_OUTCOMES:
            raise ValueError(
                f"Inspect {attempt_path(args.out, group, identifier, attempt)}"
            )

    try:
        for key, history in histories.items():
            if history and case_status(history[-1]) not in CASE_OUTCOMES:
                raise ValueError(f"Unresolved non-network failure: {key}")
        bind_clients()
        phase = "first_pass_and_makeup" if routes else "first_pass"
        progress(phase)
        initial = [
            (group, identifier, len(history) + 1)
            for (group, identifier), history in histories.items()
            if routes
            and history
            and len(history) <= args.makeup_passes
            and network_failure(history[-1])
        ]
        initial.extend(
            (group, identifier, 1)
            for identifier in identifiers
            for group in groups
            if not histories[group, identifier]
        )
        run_window(
            initial,
            jobs,
            args.jobs_per_group,
            routes,
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
                jobs,
                args.jobs_per_group,
                routes,
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
            state = case_status(result)
            writer.writerow(
                {
                    "group": group,
                    "case_id": identifier,
                    "status": result["status"] if state == "deferred" else state,
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
    concurrency = parser.add_mutually_exclusive_group()
    concurrency.add_argument("--jobs", type=int, default=4)
    concurrency.add_argument("--jobs-per-group", type=int)
    concurrency.add_argument("--routes", type=Path)
    parser.add_argument(
        "--allow-runner-update",
        action="store_true",
        help="audit an explicit waiter recovery upgrade when resuming",
    )
    parser.add_argument("--timeout", type=int, default=0)
    parser.add_argument("--makeup-passes", type=int, default=3)
    args = parser.parse_args()
    if (
        args.jobs < 1
        or (args.jobs_per_group is not None and args.jobs_per_group < 1)
        or args.timeout < 0
        or args.makeup_passes < 1
    ):
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
