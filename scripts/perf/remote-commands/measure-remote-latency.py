#!/usr/bin/env python3
"""Measure every remote read command against a throwaway copy of the real data.

What it does, in order:

1. Copies the three live databases with an **online SQLite backup** (WAL
   included, one consistent snapshot each) into a fresh temporary home. The live
   files are opened `mode=ro` and are never written; no credentials are copied.
2. Derives the ids every command needs from that snapshot with SQL — heaviest
   session by entries, median session, heaviest run by `run_events`, task with
   the most runs, a real prompt receipt id — so the measurement is of real
   shapes, not fixtures.
3. Starts an isolated Agent on a **fresh free loopback port** with its own
   `HOME` (never the user's agent, never port 50051, never a `kill`), then runs
   the probe (`host/src/remote_host/latency_probe.rs`) twice:
   - `cold`: one call per command, so each is a genuine first call in a process;
   - `warm`: warmup + N calls per command on the same Agent.
4. Writes the raw samples under `docs/testing/remote-latency/raw/`.

The probe measures the handler (`business::execute`), not the transport: see the
module comment in `latency_probe.rs`.

Usage (from the repo root):

    python3 scripts/perf/remote-commands/measure-remote-latency.py
    python3 scripts/perf/remote-commands/measure-remote-latency.py --iterations 12
    python3 scripts/perf/remote-commands/measure-remote-latency.py --keep-home  # inspect the copy
"""
import argparse
import json
import os
import shutil
import socket
import sqlite3
import subprocess
import sys
import tempfile
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
RAW = ROOT / "docs" / "testing" / "remote-latency" / "raw"
WORK = ROOT / "target" / "remote-latency"

# (path inside the isolated home, path inside the live home)
DATABASES = [
    (".future/agent/agent.db", ".future/agent/agent.db"),
    (".future/app/app.db", ".future/app/app.db"),
    (".future/tasks/tasks.db", ".future/tasks/tasks.db"),
]

# Configuration the handlers read, never credentials or conversation state. The
# model cache is included on purpose: `get_available_models` answers from it, so
# without it the command would measure a cold provider fetch the real desktop
# never does.
CONFIG_FILES = [
    ".future/agent/settings.json",
    ".future/agent/models.json",
    ".future/agent/.future-models-cache.json",
]

# Directories whose contents are read by a command (`list_skills` renders the
# installed set). Copied, not linked: the probe must not be able to write through
# to the live installation.
CONFIG_DIRS = [
    ".future/agent/skills",
]


def free_port():
    with socket.socket() as sock:
        sock.bind(("127.0.0.1", 0))
        return sock.getsockname()[1]


def wait_port(port, process, timeout=120):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        if process.poll() is not None:
            raise RuntimeError(f"child exited {process.returncode}; see its log")
        try:
            with socket.create_connection(("127.0.0.1", port), timeout=0.2):
                return
        except OSError:
            time.sleep(0.1)
    raise TimeoutError(f"child did not open loopback port {port}")


def copy_database(source, destination):
    """One consistent snapshot via SQLite's online backup API.

    `mode=ro` on the source is what guarantees the measurement cannot disturb the
    live agent: a backup from a read-only handle cannot write the WAL, the schema
    or a checkpoint back into the user's database.
    """
    destination.parent.mkdir(parents=True, exist_ok=True)
    with sqlite3.connect(f"{source.as_uri()}?mode=ro", uri=True) as src, sqlite3.connect(
        destination
    ) as dst:
        src.backup(dst, pages=4096)


def build_home(home: Path):
    live = Path.home()
    copied = []
    for target, source in DATABASES:
        origin = live / source
        if not origin.is_file():
            print(f"skip {source}: not present", flush=True)
            continue
        copy_database(origin, home / target)
        copied.append({"path": target, "bytes": (home / target).stat().st_size})
    for name in CONFIG_FILES:
        origin = live / name
        if origin.is_file():
            (home / name).parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(origin, home / name)
    for name in CONFIG_DIRS:
        origin = live / name
        if origin.is_dir():
            shutil.copytree(origin, home / name, dirs_exist_ok=True)
    # The probe refuses to run without this: it is the harness's declaration that
    # this home is a throwaway copy.
    (home / "MEASUREMENT_ISOLATED_HOME").touch()
    return copied


def ro(path: Path):
    """A read-only connection to the snapshot.

    Read-only WAL needs the `-wal`/`-shm` files writable, which they are here:
    the *snapshot* is disposable, only the live databases are off limits. So this
    is a plain connection, used for the derivations only.
    """
    return sqlite3.connect(path)


SANE_SESSION = "session_id glob '20[0-9][0-9][0-9][0-9][0-9][0-9]-*'"


def derive(agent_db, app_db, tasks_db):
    """The ids the command list needs, each picked for a reason."""
    agent = ro(agent_db)
    app = ro(app_db)
    tasks = ro(tasks_db)
    ids = {}

    sessions = agent.execute(
        f"SELECT session_id, count(*) n FROM entries WHERE {SANE_SESSION} "
        "GROUP BY session_id ORDER BY n DESC"
    ).fetchall()
    ids["sessionsWithEntries"] = len(sessions)
    ids["entriesTotal"] = agent.execute("SELECT count(*) FROM entries").fetchone()[0]
    largest_id, largest_entries = sessions[0]
    median_id, median_entries = sessions[len(sessions) // 2]
    ids["largestSession"] = {"id": largest_id, "entries": largest_entries}
    ids["medianSession"] = {"id": median_id, "entries": median_entries}
    # A second, larger tier: two points only say "different", three show the
    # shape (flat? linear in the offset? in the session?).
    mid_id, mid_entries = sessions[min(2, len(sessions) - 1)]
    ids["midSession"] = {"id": mid_id, "entries": mid_entries}
    small = min(sessions, key=lambda row: abs(row[1] - 40))
    ids["smallSession"] = {"id": small[0], "entries": small[1]}

    # A deep page is a scrolled-up read: a cursor in the middle of the session's
    # positions, far from both ends.
    deep_cursor = agent.execute(
        "SELECT position FROM entries WHERE session_id=? ORDER BY position LIMIT 1 OFFSET ?",
        (largest_id, largest_entries // 2),
    ).fetchone()[0]
    ids["largestSession"]["deepCursor"] = deep_cursor
    ids["largestSession"]["lastCursor"] = agent.execute(
        "SELECT max(position) FROM entries WHERE session_id=?", (largest_id,)
    ).fetchone()[0]

    heaviest = agent.execute(
        f"SELECT session_id, run_id, count(*) n FROM run_events WHERE {SANE_SESSION} "
        "GROUP BY session_id, run_id ORDER BY n DESC LIMIT 1"
    ).fetchone()
    ids["heaviestRun"] = {"session": heaviest[0], "run": heaviest[1], "events": heaviest[2]}
    ids["runEventsTotal"] = agent.execute("SELECT count(*) FROM run_events").fetchone()[0]
    ids["runsTotal"] = agent.execute("SELECT count(*) FROM runs").fetchone()[0]

    # The other end of the event curve: a small run, so "a page costs the same on
    # both" is a measurement instead of an assumption.
    small_run = agent.execute(
        f"SELECT session_id, run_id, count(*) n FROM run_events WHERE {SANE_SESSION} "
        "AND run_id<>'' AND session_id IN (SELECT session_id FROM entries) "
        "GROUP BY session_id, run_id HAVING n BETWEEN 20 AND 80 ORDER BY n DESC LIMIT 1"
    ).fetchone()
    ids["smallRun"] = (
        {"session": small_run[0], "run": small_run[1], "events": small_run[2]}
        if small_run
        else None
    )

    # `get_event_content_range` needs an event whose body the Agent did not
    # inline (> 16 KiB, see `agent/src/session/content/transport.rs`). A long
    # `user_message` is the shape that reliably crosses it, so the run of the
    # biggest one is the run worth asking about. Restricted to sessions that have
    # published entries: a session with run events but no entries answers
    # "session not found" to every read, which is a broken fixture rather than a
    # measurement.
    big_message = agent.execute(
        f"SELECT session_id, run_id, idx, length(json_extract(payload,'$.data')) n, "
        "json_extract(payload,'$.event_type') FROM run_events "
        "WHERE json_extract(payload,'$.event_type') IN "
        "('user_message','compaction_committed','tool_end') AND run_id<>'' AND "
        f"{SANE_SESSION} AND session_id IN (SELECT session_id FROM entries) "
        "ORDER BY n DESC LIMIT 1"
    ).fetchone()
    ids["bigBody"] = (
        {
            "session": big_message[0],
            "run": big_message[1],
            "idx": big_message[2],
            "bodyBytes": big_message[3],
            "eventType": big_message[4],
            # The Agent inlines a body up to this size and hands out a
            # `content_refs` reference only above it
            # (`agent/src/session/content/transport.rs`). Recorded next to the
            # biggest body so a report can say whether the command was
            # measurable at all on this data.
            "inlineThresholdBytes": 16384,
        }
        if big_message
        else None
    )

    # `get_tool_call_args` answers one tool call by identity, and the interesting
    # one is the heaviest: a `write`/`edit` whose arguments a lean page dropped.
    # `data` is a JSON *string* in `run_events`, so the tool id is one
    # `json_extract` deeper than it looks.
    tool = agent.execute(
        "SELECT json_extract(json_extract(payload,'$.data'),'$.tool_id'),"
        " json_extract(json_extract(payload,'$.data'),'$.tool_name'),"
        " length(json_extract(json_extract(payload,'$.data'),'$.tool_args')) FROM run_events "
        "WHERE session_id=? AND run_id=? AND json_extract(payload,'$.event_type')='tool_start'"
        " AND json_extract(json_extract(payload,'$.data'),'$.tool_id') IS NOT NULL"
        " ORDER BY 3 DESC LIMIT 1",
        (heaviest[0], heaviest[1]),
    ).fetchone()
    if tool:
        ids["heaviestRun"]["toolCall"] = {"id": tool[0], "name": tool[1], "argsBytes": tool[2]}

    # A session whose thread the desktop knows: `list_session_files` and
    # `get_state` root at the thread's workspace.
    thread = app.execute(
        "SELECT t.id, t.agent_session_id, w.path, (SELECT count(*) FROM runs r WHERE r.thread_id=t.id) n"
        " FROM threads t JOIN workspaces w ON w.id=t.workspace_id"
        " WHERE t.agent_session_id=? LIMIT 1",
        (largest_id,),
    ).fetchone()
    ids["threadsTotal"] = app.execute("SELECT count(*) FROM threads").fetchone()[0]
    if not thread:
        thread = app.execute(
            "SELECT t.id, t.agent_session_id, w.path, (SELECT count(*) FROM runs r WHERE r.thread_id=t.id) n"
            " FROM threads t JOIN workspaces w ON w.id=t.workspace_id"
            " WHERE t.agent_session_id IS NOT NULL AND t.agent_session_id<>''"
            " ORDER BY n DESC LIMIT 1"
        ).fetchone()
    ids["thread"] = {
        "id": thread[0],
        "session": thread[1],
        "workspace": thread[2],
        "runs": thread[3],
    }

    prompt = app.execute(
        "SELECT trigger_message_id FROM runs WHERE trigger_message_id IS NOT NULL"
        " AND trigger_message_id<>'' ORDER BY created_at DESC LIMIT 1"
    ).fetchone()
    ids["promptId"] = prompt[0] if prompt else ""

    busiest_task = tasks.execute(
        "SELECT task_id, count(*) n FROM task_runs GROUP BY task_id ORDER BY n DESC LIMIT 1"
    ).fetchone()
    revised_task = tasks.execute(
        "SELECT task_id, count(*) n FROM task_prompt_revisions GROUP BY task_id ORDER BY n DESC LIMIT 1"
    ).fetchone()
    ids["tasksTotal"] = tasks.execute("SELECT count(*) FROM tasks").fetchone()[0]
    ids["taskRunsTotal"] = tasks.execute("SELECT count(*) FROM task_runs").fetchone()[0]
    ids["task"] = {
        "busiest": busiest_task[0] if busiest_task else "",
        "busiestRuns": busiest_task[1] if busiest_task else 0,
        "mostRevisions": revised_task[0] if revised_task else "",
        "revisions": revised_task[1] if revised_task else 0,
    }

    ids["contentObjects"] = agent.execute("SELECT count(*) FROM content_objects").fetchone()[0]
    ids["databaseId"] = agent.execute(
        "SELECT value FROM storage_meta WHERE key='database_id'"
    ).fetchone()[0]
    for connection in (agent, app, tasks):
        connection.close()
    return ids


def scale(**fields):
    return fields


def specs(ids, phase):
    """The command list. `iterations`/`warmup` depend on the phase."""
    warm = phase == "warm"
    largest = ids["largestSession"]
    median = ids["medianSession"]
    heavy = ids["heaviestRun"]
    thread = ids["thread"]
    task = ids["task"]

    def spec(label, command, data_scale, spacing_ms=0):
        return {
            "label": label,
            "scale": data_scale,
            "iterations": 8 if warm else 1,
            "warmup": 2 if warm else 0,
            # Both catalog pulls are served from a one-second snapshot cache, so
            # a tight warm loop would only time the cache. 1100 ms is just past
            # the window: every warm sample is a real store read.
            "spacingMs": spacing_ms,
            "command": command,
        }

    catalog = [
        spec(
            "list_sessions",
            {"type": "list_sessions"},
            scale(threads=ids["threadsTotal"], sessionsWithEntries=ids["sessionsWithEntries"]),
            spacing_ms=1100,
        ),
        spec(
            "list_workspaces",
            {"type": "list_workspaces"},
            scale(threads=ids["threadsTotal"]),
            spacing_ms=1100,
        ),
    ]
    history = [
        spec(
            "get_state:largest",
            {"type": "get_state", "sessionId": largest["id"]},
            scale(sessionId=largest["id"], entries=largest["entries"]),
        ),
        spec(
            "get_state:median",
            {"type": "get_state", "sessionId": median["id"]},
            scale(sessionId=median["id"], entries=median["entries"]),
        ),
        spec(
            "get_state:small",
            {"type": "get_state", "sessionId": ids["smallSession"]["id"]},
            scale(sessionId=ids["smallSession"]["id"], entries=ids["smallSession"]["entries"]),
        ),
        # `get_messages` fetches the whole conversation and paginates in memory,
        # so the same page on four sessions of different sizes is the curve that
        # decides whether it is O(1) or O(history).
        spec(
            "get_messages:first_page:largest",
            {"type": "get_messages", "sessionId": largest["id"], "offset": 0, "limit": 40},
            scale(sessionId=largest["id"], entries=largest["entries"], offset=0),
        ),
        spec(
            "get_messages:deep_page:largest",
            {"type": "get_messages", "sessionId": largest["id"],
             "offset": largest["entries"] // 2, "limit": 40},
            scale(sessionId=largest["id"], entries=largest["entries"],
                  offset=largest["entries"] // 2),
        ),
        spec(
            "get_messages:first_page:mid",
            {"type": "get_messages", "sessionId": ids["midSession"]["id"], "offset": 0,
             "limit": 40},
            scale(sessionId=ids["midSession"]["id"], entries=ids["midSession"]["entries"],
                  offset=0),
        ),
        spec(
            "get_messages:first_page:median",
            {"type": "get_messages", "sessionId": median["id"], "offset": 0, "limit": 40},
            scale(sessionId=median["id"], entries=median["entries"], offset=0),
        ),
        spec(
            "get_messages:first_page:small",
            {"type": "get_messages", "sessionId": ids["smallSession"]["id"], "offset": 0,
             "limit": 40},
            scale(sessionId=ids["smallSession"]["id"], entries=ids["smallSession"]["entries"],
                  offset=0),
        ),
        spec(
            "get_session_entries:first_paint:largest",
            {"type": "get_session_entries", "sessionId": largest["id"],
             "before": 9007199254740991, "limit": 40, "chunkedRead": True},
            scale(sessionId=largest["id"], entries=largest["entries"], cursor="newest"),
        ),
        spec(
            "get_session_entries:deep_page:largest",
            {"type": "get_session_entries", "sessionId": largest["id"],
             "before": largest["deepCursor"], "limit": 40, "chunkedRead": True},
            scale(sessionId=largest["id"], entries=largest["entries"],
                  cursor=largest["deepCursor"]),
        ),
        spec(
            "get_session_entries:first_paint:median",
            {"type": "get_session_entries", "sessionId": median["id"],
             "before": 9007199254740991, "limit": 40, "chunkedRead": True},
            scale(sessionId=median["id"], entries=median["entries"], cursor="newest"),
        ),
        spec(
            "get_session_entries:offset_page:largest",
            {"type": "get_session_entries", "sessionId": largest["id"],
             "offset": largest["entries"] // 2, "limit": 40, "chunkedRead": True},
            scale(sessionId=largest["id"], entries=largest["entries"],
                  offset=largest["entries"] // 2),
        ),
        spec(
            "get_session_entries:offset_page:mid",
            {"type": "get_session_entries", "sessionId": ids["midSession"]["id"],
             "offset": ids["midSession"]["entries"] // 2, "limit": 40, "chunkedRead": True},
            scale(sessionId=ids["midSession"]["id"], entries=ids["midSession"]["entries"],
                  offset=ids["midSession"]["entries"] // 2),
        ),
        spec(
            "get_session_entries:offset_page:median",
            {"type": "get_session_entries", "sessionId": median["id"],
             "offset": median["entries"] // 2, "limit": 40, "chunkedRead": True},
            scale(sessionId=median["id"], entries=median["entries"],
                  offset=median["entries"] // 2),
        ),
        spec(
            "get_session_entries:deep_page:median",
            {"type": "get_session_entries", "sessionId": median["id"],
             "before": median["entries"] // 2, "limit": 40, "chunkedRead": True},
            scale(sessionId=median["id"], entries=median["entries"],
                  cursor=median["entries"] // 2),
        ),
        spec(
            "get_events_since:snapshot:heaviest",
            {"type": "get_events_since", "sessionId": heavy["session"], "runId": heavy["run"],
             "sinceIdx": -1, "limit": 200, "chunkedRead": True, "preferSnapshot": True},
            scale(sessionId=heavy["session"], runId=heavy["run"], events=heavy["events"],
                  path="snapshot"),
        ),
        spec(
            "get_events_since:first_page:heaviest",
            {"type": "get_events_since", "sessionId": heavy["session"], "runId": heavy["run"],
             "sinceIdx": -1, "limit": 200, "chunkedRead": True, "preferSnapshot": False},
            scale(sessionId=heavy["session"], runId=heavy["run"], events=heavy["events"],
                  path="page"),
        ),
        spec(
            "get_tool_call_args:heaviest",
            {"type": "get_tool_call_args", "sessionId": heavy["session"], "runId": heavy["run"],
             "toolCallId": heavy.get("toolCall", {}).get("id", "")},
            scale(sessionId=heavy["session"], runId=heavy["run"],
                  toolCallId=heavy.get("toolCall", {}).get("id", ""),
                  argsBytes=heavy.get("toolCall", {}).get("argsBytes", 0)),
        ),
        # The request is lifted from a real page by the probe: only a reply
        # carries `databaseId`/`cursor`/`field` for a body the Agent did not
        # inline. The run is the one with the biggest `user_message`, because
        # that is the body that crosses the inline limit.
        spec(
            "get_event_content_range:biggest_body",
            {"type": "get_event_content_range",
             "sessionId": (ids["bigBody"] or heavy)["session"],
             "runId": (ids["bigBody"] or heavy)["run"]},
            scale(
                sessionId=(ids["bigBody"] or heavy)["session"],
                runId=(ids["bigBody"] or heavy)["run"],
                bodyBytes=(ids["bigBody"] or {}).get("bodyBytes", 0),
                inlineThresholdBytes=(ids["bigBody"] or {}).get("inlineThresholdBytes", 0),
                derived=True,
            ),
        ),
        spec(
            "get_events_since:seek:biggest_body_run",
            {"type": "get_events_since",
             "sessionId": (ids["bigBody"] or heavy)["session"],
             "runId": (ids["bigBody"] or heavy)["run"],
             "sinceIdx": max((ids["bigBody"] or {}).get("idx", 0) - 2, 0),
             "limit": 200, "chunkedRead": True, "preferSnapshot": False},
            scale(sessionId=(ids["bigBody"] or heavy)["session"],
                  runId=(ids["bigBody"] or heavy)["run"],
                  sinceIdx=max((ids["bigBody"] or {}).get("idx", 0) - 2, 0),
                  bodyBytes=(ids["bigBody"] or {}).get("bodyBytes", 0)),
        ),
        spec(
            "get_fusion:heaviest",
            {"type": "get_fusion", "sessionId": heavy["session"], "runId": heavy["run"],
             "toolCallId": heavy.get("toolCall", {}).get("id", ""), "offset": 0},
            scale(sessionId=heavy["session"], runId=heavy["run"],
                  toolCallId=heavy.get("toolCall", {}).get("id", "")),
        ),
    ]
    transfers = [
        spec(
            "list_session_files:workspace_root",
            {"type": "list_session_files", "sessionId": thread["session"], "filePath": ""},
            scale(sessionId=thread["session"], workspace=thread["workspace"]),
        ),
    ]
    prompt = [
        spec(
            "get_queue:largest",
            {"type": "get_queue", "sessionId": largest["id"]},
            scale(sessionId=largest["id"]),
        ),
        spec(
            "get_prompt_receipt:newest",
            {"type": "get_prompt_receipt", "promptId": ids["promptId"]},
            scale(promptId=ids["promptId"]),
        ),
    ]
    settings = [
        spec("get_fusion_settings", {"type": "get_fusion_settings"}, scale()),
        spec("get_desktop_settings", {"type": "get_desktop_settings"}, scale()),
        spec("get_settings", {"type": "get_settings"}, scale()),
        spec("list_settings_models", {"type": "list_settings_models"}, scale()),
        spec("list_models", {"type": "list_models"}, scale()),
        spec("get_available_models", {"type": "get_available_models"}, scale()),
        spec("list_skills", {"type": "list_skills"}, scale()),
        spec("list_available_skills", {"type": "list_available_skills"}, scale()),
        spec("skill_reco_today", {"type": "skill_reco_today"}, scale()),
    ]
    providers = [spec("list_providers", {"type": "list_providers"}, scale())]
    tasks = [
        spec("list_tasks", {"type": "list_tasks"}, scale(tasks=ids["tasksTotal"])),
        spec(
            "get_task:busiest",
            {"type": "get_task", "taskId": task["busiest"]},
            scale(taskId=task["busiest"], runs=task["busiestRuns"]),
        ),
        spec(
            "list_task_runs:busiest",
            {"type": "list_task_runs", "taskId": task["busiest"], "limit": 20},
            scale(taskId=task["busiest"], runs=task["busiestRuns"]),
        ),
        spec(
            "list_task_revisions:most_revisions",
            {"type": "list_task_revisions", "taskId": task["mostRevisions"]},
            scale(taskId=task["mostRevisions"], revisions=task["revisions"]),
        ),
        spec("list_task_deps:busiest", {"type": "list_task_deps", "taskId": task["busiest"]},
             scale(taskId=task["busiest"])),
    ]
    ordered = catalog + history + transfers + prompt + settings + providers + tasks
    small_run = ids.get("smallRun")
    if small_run:
        ordered.append(
            spec(
                "get_events_since:first_page:small_run",
                {"type": "get_events_since", "sessionId": small_run["session"],
                 "runId": small_run["run"], "sinceIdx": -1, "limit": 200,
                 "chunkedRead": True, "preferSnapshot": False},
                scale(sessionId=small_run["session"], runId=small_run["run"],
                      events=small_run["events"]),
            )
        )
    for item in ordered:
        if item["label"] == "get_event_content_range:biggest_body":
            item["deriveContentRequest"] = True
    return ordered


def cargo(args):
    subprocess.run(["cargo", *args], cwd=ROOT, check=True, stdout=subprocess.DEVNULL)


def locate_test_binary(profile):
    """Ask Cargo where the lib test binary is, rather than globbing `deps/`."""
    result = subprocess.run(
        ["cargo", "test", "-p", "future-host", *profile, "--no-run", "--message-format=json"],
        cwd=ROOT,
        check=True,
        capture_output=True,
        text=True,
    )
    for line in result.stdout.splitlines():
        try:
            message = json.loads(line)
        except json.JSONDecodeError:
            continue
        if message.get("reason") == "compiler-artifact" and message.get("executable"):
            # A lib test target's kind is `["lib"]` (the harness is added by
            # `--test`); the name is what tells it apart from a binary target.
            if message["target"]["name"] == "future_host":
                return Path(message["executable"])
    raise RuntimeError("cargo did not report the future-host test binary")


def probe_env(home, port, extra):
    env = {key: value for key, value in os.environ.items() if not key.startswith("FUTURE_")}
    env.update(
        HOME=str(home),
        USERPROFILE=str(home),
        FUTURE_AGENT_GRPC_ADDR=f"http://127.0.0.1:{port}",
        **extra,
    )
    return env


def machine():
    info = {
        "os": os.uname().sysname + " " + os.uname().release,
        "arch": os.uname().machine,
        "python": sys.version.split()[0],
    }
    for key, command in (
        ("cpu", ["sysctl", "-n", "machdep.cpu.brand_string"]),
        ("cpus", ["sysctl", "-n", "hw.ncpu"]),
        ("memoryBytes", ["sysctl", "-n", "hw.memsize"]),
        ("osVersion", ["sw_vers", "-productVersion"]),
    ):
        try:
            info[key] = subprocess.check_output(command, text=True).strip()
        except (OSError, subprocess.CalledProcessError):
            pass
    return info


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--iterations", type=int, default=8, help="warm iterations per command")
    parser.add_argument("--phase", choices=["cold", "warm", "both"], default="both")
    parser.add_argument("--home", help="reuse a prepared isolated home instead of copying")
    parser.add_argument("--keep-home", action="store_true", help="keep the copy and log it")
    parser.add_argument("--test-binary", help="prebuilt future-host test binary")
    parser.add_argument("--agent-binary", default=str(ROOT / "target" / "release" / "future-agent"))
    parser.add_argument(
        "--profile",
        default="--release",
        choices=["--release", "--debug"],
        help="cargo profile for the probe binary (default: release)",
    )
    parser.add_argument("--tag", default=time.strftime("%Y%m%d-%H%M%S"), help="raw file stem")
    args = parser.parse_args()

    if not args.agent_binary or not Path(args.agent_binary).is_file():
        raise SystemExit(f"missing Agent binary {args.agent_binary}; build it first")
    profile = [] if args.profile == "--debug" else ["--release"]
    test_binary = Path(args.test_binary) if args.test_binary else locate_test_binary(profile)

    WORK.mkdir(parents=True, exist_ok=True)
    RAW.mkdir(parents=True, exist_ok=True)
    temporary = None
    if args.home:
        home = Path(args.home).resolve()
        home.mkdir(parents=True, exist_ok=True)
    else:
        temporary = tempfile.TemporaryDirectory(prefix="remote-latency-", dir=WORK)
        home = Path(temporary.name)

    os.umask(0o077)
    if not (home / "MEASUREMENT_ISOLATED_HOME").exists():
        print("copying the live databases into a throwaway home", flush=True)
        copied = build_home(home)
        print(json.dumps(copied, indent=2), flush=True)

    ids = derive(
        home / ".future/agent/agent.db",
        home / ".future/app/app.db",
        home / ".future/tasks/tasks.db",
    )
    document = {
        "tag": args.tag,
        "machine": machine(),
        "profile": args.profile,
        "sourceBytes": {
            target: (home / target).stat().st_size for target, _ in DATABASES
        },
        "derived": ids,
    }
    (RAW / f"snapshot-{args.tag}.json").write_text(json.dumps(document, indent=2, ensure_ascii=False))

    port = free_port()
    phases = ["cold", "warm"] if args.phase == "both" else [args.phase]
    agent_log = (WORK / f"agent-{args.tag}.log").open("w")
    agent = subprocess.Popen(
        [args.agent_binary, "--grpc-addr", f"127.0.0.1:{port}"],
        env=probe_env(home, port, {}),
        cwd=str(home),
        stdout=agent_log,
        stderr=subprocess.STDOUT,
    )
    try:
        wait_port(port, agent)
        for phase in phases:
            list_path = WORK / f"commands-{phase}-{args.tag}.json"
            out_path = RAW / f"{phase}-{args.tag}.json"
            commands = specs(ids, phase)
            if phase == "warm":
                for command in commands:
                    if command["label"].startswith(("list_sessions", "list_workspaces")):
                        continue
                    command["iterations"] = args.iterations
            list_path.write_text(json.dumps(commands, indent=2, ensure_ascii=False))
            env = probe_env(
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
            print(f"--- {phase} phase ({len(commands)} commands) -> {out_path}", flush=True)
            log_path = WORK / f"probe-{phase}-{args.tag}.log"
            with log_path.open("w") as log:
                subprocess.run(
                    [
                        str(test_binary),
                        "remote_host::latency_probe::measure_real_snapshot",
                        "--exact",
                        "--ignored",
                        "--nocapture",
                        "--test-threads=1",
                    ],
                    env=env,
                    cwd=ROOT,
                    check=True,
                    stdout=log,
                    stderr=subprocess.STDOUT,
                )
            print(log_path.read_text(), flush=True)
    finally:
        agent_log.close()
        if agent.poll() is None:
            agent.terminate()
            try:
                agent.wait(timeout=20)
            except subprocess.TimeoutExpired:
                agent.kill()
                agent.wait()
        if temporary is not None:
            if args.keep_home:
                print(f"kept {home}", flush=True)
                temporary._finalizer.detach()  # noqa: SLF001 - deliberate retention
            else:
                temporary.cleanup()
    print(f"raw samples in {RAW}", flush=True)


if __name__ == "__main__":
    main()
