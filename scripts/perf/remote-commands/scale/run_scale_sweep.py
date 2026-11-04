#!/usr/bin/env python3
"""Drive the chapter-03 probe across the T4 scale points.

`build_scale_data.py` writes the homes; this script measures them. One Agent per
home (own `HOME`, fresh free loopback port, never 50051, never a `kill`), one
probe run per phase, raw samples under `docs/testing/remote-latency/raw/scaling/`.

The probe is the same `host/src/remote_host/latency_probe.rs`
(`measure_real_snapshot`), the command list is the same JSON contract, and the
Agent/environment helpers are imported from `measure-remote-latency.py`. What is
new here is only *which* commands are asked at *which* size: this chapter sweeps
one data dimension at a time (see `build_scale_data.py`'s docstring).

Usage:

    python3 scripts/perf/remote-commands/scale/run_scale_sweep.py --list
    python3 scripts/perf/remote-commands/scale/run_scale_sweep.py                    # every home
    python3 scripts/perf/remote-commands/scale/run_scale_sweep.py --only history
    python3 scripts/perf/remote-commands/scale/run_scale_sweep.py --validate         # 1 iteration, warm only
    python3 scripts/perf/remote-commands/scale/run_scale_sweep.py --grep offset      # subset of labels
"""
import argparse
import importlib.util
import json
import os
import sqlite3
import subprocess
import sys
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[4]
SCALE_ROOT = ROOT / "target" / "remote-latency" / "scale"
HOMES = SCALE_ROOT / "homes"
WORK = SCALE_ROOT / "commands"
MANIFEST = SCALE_ROOT / "manifest.json"
RAW = ROOT / "docs" / "testing" / "remote-latency" / "raw" / "scaling"
LOGS = SCALE_ROOT / "logs"

NEWEST_CURSOR = 9_007_199_254_740_991


def measure_module():
    path = Path(__file__).resolve().parents[1] / "measure-remote-latency.py"
    spec = importlib.util.spec_from_file_location("measure_remote_latency", path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


harness = measure_module()


def ro(path):
    return sqlite3.connect(f"file:{path}?mode=ro", uri=True)


def tool_call_for(db, session):
    """The heaviest tool call of a session, as `get_tool_call_args` needs it.

    Entry ownership matters: the command resolves a block by
    `(session, entry_position)` and then requires the entry's `run_id`, so the
    pair has to come from the same row — a run id guessed from the run list
    answers `not found`, which would look like a fast read.
    """
    row = ro(db).execute(
        "SELECT e.run_id,b.tool_call_id,length(b.arguments_json)"
        " FROM message_blocks b JOIN entries e"
        "   ON e.session_id=b.session_id AND e.position=b.entry_position"
        " WHERE b.session_id=?1 AND b.kind='tool_call' AND b.tool_call_id IS NOT NULL"
        "   AND b.tool_call_id<>'' AND e.run_id IS NOT NULL AND e.run_id<>''"
        " ORDER BY length(b.arguments_json) DESC LIMIT 1",
        (session,),
    ).fetchone()
    if not row:
        return None
    return {"runId": row[0], "toolCallId": row[1], "argsBytes": row[2]}


def thread_for(app_db, session):
    row = ro(app_db).execute(
        "SELECT id FROM threads WHERE agent_session_id=?1 AND status<>'deleted' LIMIT 1",
        (session,),
    ).fetchone()
    return row[0] if row else None


def spec(label, command, scale, iterations, warmup=2, spacing=0):
    return {
        "label": label,
        "scale": scale,
        "iterations": iterations,
        "warmup": warmup,
        "spacingMs": spacing,
        "command": command,
    }


def history_specs(anchors, detail, iterations):
    """`entries` and `run_events` dimension: every size in one home."""
    agent_db = detail["agentDb"]
    app_db = detail["appDb"]
    points = [
        ("small", anchors["smallSession"]["id"], anchors["smallSession"]["entries"]),
        ("median", anchors["medianSession"]["id"], anchors["medianSession"]["entries"]),
        ("mid", anchors["midSession"]["id"], anchors["midSession"]["entries"]),
        ("largest", anchors["largestSession"]["id"], anchors["largestSession"]["entries"]),
    ]
    for item in detail["scaledSessions"]:
        points.append((f"scaled{item['entries']}", item["session"], item["entries"]))
    points.sort(key=lambda point: point[2])

    specs, notes = [], []
    for name, session, entries in points:
        depth = entries // 2
        specs.append(
            spec(
                f"get_state:{name}",
                {"type": "get_state", "sessionId": session},
                {"sessionId": session, "entries": entries},
                iterations,
            )
        )
        specs.append(
            spec(
                f"get_messages:first:{name}",
                {"type": "get_messages", "sessionId": session, "offset": 0, "limit": 40},
                {"sessionId": session, "entries": entries, "offset": 0},
                iterations,
            )
        )
        specs.append(
            spec(
                f"get_messages:deep:{name}",
                {"type": "get_messages", "sessionId": session, "offset": depth, "limit": 40},
                {"sessionId": session, "entries": entries, "offset": depth},
                iterations,
            )
        )
        specs.append(
            spec(
                f"get_session_entries:before:{name}",
                {"type": "get_session_entries", "sessionId": session,
                 "before": NEWEST_CURSOR, "limit": 40, "chunkedRead": True},
                {"sessionId": session, "entries": entries, "path": "cursor"},
                iterations,
            )
        )
        specs.append(
            spec(
                f"get_session_entries:offset:{name}",
                {"type": "get_session_entries", "sessionId": session,
                 "offset": depth, "limit": 40, "chunkedRead": True},
                {"sessionId": session, "entries": entries, "path": "offset", "offset": depth},
                iterations,
            )
        )
        tool = tool_call_for(agent_db, session)
        if tool:
            specs.append(
                spec(
                    f"get_tool_call_args:{name}",
                    {"type": "get_tool_call_args", "sessionId": session,
                     "runId": tool["runId"], "toolCallId": tool["toolCallId"]},
                    {"sessionId": session, "entries": entries, **tool},
                    iterations,
                )
            )
        else:
            notes.append(f"get_tool_call_args:{name}: no run-bound tool call in this session")
        if thread_for(app_db, session):
            specs.append(
                spec(
                    f"list_session_files:{name}",
                    {"type": "list_session_files", "sessionId": session, "filePath": ""},
                    {"sessionId": session, "entries": entries},
                    iterations,
                )
            )
        else:
            notes.append(f"list_session_files:{name}: no desktop thread bound to this session")

    heavy = anchors["heaviestRun"]
    for item in detail["scaledRuns"]:
        events, run = item["events"], item["run"]
        deep = item.get("deepIdx", events // 2)
        specs.append(
            spec(
                f"get_events_since:first:{events}",
                {"type": "get_events_since", "sessionId": item["session"], "runId": run,
                 "sinceIdx": -1, "limit": 200, "chunkedRead": True, "preferSnapshot": False},
                {"events": events, "path": "first_page"},
                iterations,
            )
        )
        specs.append(
            spec(
                f"get_events_since:deep:{events}",
                {"type": "get_events_since", "sessionId": item["session"], "runId": run,
                 "sinceIdx": deep, "limit": 200, "chunkedRead": True,
                 "preferSnapshot": False},
                {"events": events, "path": "deep_page", "sinceIdx": deep},
                iterations,
            )
        )
        specs.append(
            spec(
                f"get_events_since:snapshot:{events}",
                {"type": "get_events_since", "sessionId": item["session"], "runId": run,
                 "sinceIdx": -1, "limit": 200, "chunkedRead": True, "preferSnapshot": True},
                {"events": events, "path": "snapshot"},
                iterations,
            )
        )

    # The real-value control for the `threads` homes (which plant no agent.db):
    # the same two catalog commands on this home's real app.db + real agent.db.
    specs.append(
        spec(
            "list_sessions:real",
            {"type": "list_sessions"},
            {"threads": anchors["threadsTotal"], "control": "real agent.db"},
            8,
            spacing=1100,
        )
    )
    specs.append(
        spec(
            "list_workspaces:real",
            {"type": "list_workspaces"},
            {"threads": anchors["threadsTotal"], "control": "real agent.db"},
            8,
            spacing=1100,
        )
    )
    return specs, notes


def threads_specs(anchors, detail, iterations):
    return [
        spec("list_sessions", {"type": "list_sessions"},
             {"threads": detail["threads"]}, 8, spacing=1100),
        spec("list_workspaces", {"type": "list_workspaces"},
             {"threads": detail["threads"]}, 8, spacing=1100),
    ], []


def task_runs_specs(anchors, detail, iterations):
    task = anchors["task"]["busiest"]
    return [
        spec("list_tasks", {"type": "list_tasks"},
             {"tasks": anchors["tasksTotal"], "runsOfBusiest": detail["runs"]}, iterations),
        spec("get_task", {"type": "get_task", "taskId": task},
             {"taskId": task, "runs": detail["runs"]}, iterations),
        spec("list_task_runs", {"type": "list_task_runs", "taskId": task, "limit": 20},
             {"taskId": task, "runs": detail["runs"]}, iterations),
    ], []


def tasks_specs(anchors, detail, iterations):
    return [
        spec("list_tasks", {"type": "list_tasks"},
             {"tasks": detail["tasks"]}, iterations),
    ], []


def dirs_specs(anchors, detail, iterations):
    session = anchors["thread"]["session"]
    return [
        spec("list_session_files", {"type": "list_session_files", "sessionId": session,
                                    "filePath": ""},
             {"files": detail["files"], "workspace": detail["workspace"]}, iterations),
    ], []


BUILDERS = {
    "history": history_specs,
    "threads": threads_specs,
    "task-runs": task_runs_specs,
    "tasks": tasks_specs,
    "dirs": dirs_specs,
}


def specs_for(name, anchors, detail, iterations):
    for prefix, builder in BUILDERS.items():
        if name.startswith(prefix):
            return builder(anchors, detail, iterations)
    raise SystemExit(f"no spec builder for home {name}")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--only", help="one home by name")
    parser.add_argument("--tag", default=time.strftime("%Y%m%d-%H%M%S"))
    parser.add_argument("--iterations", type=int, default=10)
    parser.add_argument("--phase", choices=["cold", "warm", "both"], default="both")
    parser.add_argument("--grep", help="only labels containing this substring")
    parser.add_argument("--validate", action="store_true",
                        help="1 iteration, warm phase only: is this home measurable at all?")
    parser.add_argument("--list", action="store_true")
    parser.add_argument("--agent-binary", default=str(ROOT / "target" / "release" / "future-agent"))
    parser.add_argument("--test-binary", help="prebuilt future-host test binary")
    parser.add_argument("--profile", default="--release", choices=["--release", "--debug"])
    args = parser.parse_args()

    if args.list:
        print("\n".join(sorted(json.loads(MANIFEST.read_text())["homes"])))
        return
    if not MANIFEST.exists():
        raise SystemExit("run build_scale_data.py first")
    manifest = json.loads(MANIFEST.read_text())
    anchors = json.loads(Path(manifest["anchors"]).read_text())

    iterations = 1 if args.validate else args.iterations
    phases = ["warm"] if args.validate else (
        ["cold", "warm"] if args.phase == "both" else [args.phase]
    )
    profile = [] if args.profile == "--debug" else ["--release"]
    test_binary = Path(args.test_binary) if args.test_binary else harness.locate_test_binary(profile)
    agent_binary = Path(args.agent_binary)
    if not agent_binary.is_file():
        raise SystemExit(f"missing Agent binary {agent_binary}; build it first")

    WORK.mkdir(parents=True, exist_ok=True)
    RAW.mkdir(parents=True, exist_ok=True)
    LOGS.mkdir(parents=True, exist_ok=True)
    names = [args.only] if args.only else sorted(manifest["homes"])
    summary = []
    for name in names:
        entry = manifest["homes"][name]
        home = Path(entry["home"])
        specs, notes = specs_for(name, anchors, entry["detail"], iterations)
        if args.grep:
            specs = [item for item in specs if args.grep in item["label"]]
        if not specs:
            print(f"--- {name}: no spec matched", flush=True)
            continue
        port = harness.free_port()
        agent_log = (LOGS / f"agent-{name}-{args.tag}.log").open("w")
        agent = subprocess.Popen(
            [str(agent_binary), "--grpc-addr", f"127.0.0.1:{port}"],
            env=harness.probe_env(home, port, {}),
            cwd=str(home),
            stdout=agent_log,
            stderr=subprocess.STDOUT,
        )
        written = []
        try:
            harness.wait_port(port, agent)
            for phase in phases:
                list_path = WORK / f"{name}-{phase}-{args.tag}.json"
                out_path = RAW / f"{name}-{phase}-{args.tag}.json"
                listing = json.loads(json.dumps(specs))
                for item in listing:
                    # The cold phase is one call per command in a fresh process;
                    # the warm phase is warmup + N on the same Agent, which is the
                    # column the curve is fitted from.
                    item["iterations"] = (8 if phase == "warm" else 1) if item[
                        "label"
                    ].startswith(("list_sessions", "list_workspaces")) else (
                        iterations if phase == "warm" else 1
                    )
                    item["warmup"] = 0 if phase == "cold" else 2
                list_path.write_text(json.dumps(listing, indent=2, ensure_ascii=False))
                env = harness.probe_env(
                    home,
                    port,
                    {
                        "REMOTE_LATENCY_PHASE": phase,
                        "REMOTE_LATENCY_COMMANDS": str(list_path),
                        "REMOTE_LATENCY_OUT": str(out_path),
                        "REMOTE_LATENCY_PROFILE": args.profile,
                        "REMOTE_LATENCY_BUILT_AT": time.strftime("%Y-%m-%dT%H:%M:%S"),
                        "REMOTE_LATENCY_VERBOSE": "1",
                    },
                )
                log_path = LOGS / f"probe-{name}-{phase}-{args.tag}.log"
                print(f"--- {name} {phase}: {len(listing)} specs -> {out_path.name}", flush=True)
                started = time.monotonic()
                with log_path.open("w") as log:
                    subprocess.run(
                        [str(test_binary), "remote_host::latency_probe::measure_real_snapshot",
                         "--exact", "--ignored", "--nocapture", "--test-threads=1"],
                        env=env, cwd=ROOT, check=True, stdout=log, stderr=subprocess.STDOUT,
                    )
                print(f"    done in {time.monotonic() - started:.1f}s", flush=True)
                written.append(str(out_path))
        finally:
            agent_log.close()
            if agent.poll() is None:
                agent.terminate()
                try:
                    agent.wait(timeout=20)
                except subprocess.TimeoutExpired:
                    agent.kill()
                    agent.wait()
        summary.append({"home": name, "specs": len(specs), "raw": written, "notes": notes})

    report = SCALE_ROOT / f"runs-{args.tag}.json"
    report.write_text(json.dumps(
        {"tag": args.tag, "iterations": iterations, "phases": phases, "homes": summary},
        indent=2, ensure_ascii=False))
    notes = [note for item in summary for note in item["notes"]]
    if notes:
        print("\nnotes:", flush=True)
        for note in notes:
            print(f"  - {note}", flush=True)
    print(f"raw -> {RAW}\nrun record -> {report}", flush=True)


if __name__ == "__main__":
    main()
