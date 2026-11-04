#!/usr/bin/env python3
"""Build the throwaway homes the T4 scaling sweep measures.

Chapter `03-measurements.md` measured each command **once**, at the sizes the
live data happens to have. This script produces the *other* sizes: homes that
are the same real snapshot except for one dimension, so a handler's curve can be
measured instead of guessed.

Two rules decide every number below:

1. **The live databases are read-only, always.** The single snapshot is taken by
   the chapter-03 harness (`measure-remote-latency.py`, SQLite online backup from
   a `mode=ro` source) and every home is a copy of *that* copy. Nothing here
   opens `~/.future/` for writing.
2. **One dimension moves, the rest stay real.** A threads home only rewrites
   `threads`/`workspaces` rows in `app.db`; its `tasks.db` is the real one and no
   `agent.db` is planted (the commands it measures — `list_sessions`,
   `list_workspaces` — never open it). The synthetic sessions and runs live in a
   single `agent.db` because they are *data*, not a dimension: each session is a
   real donor session's rows repeated to a target size, so entry shapes, entry
   bodies and run structure are exactly the ones a real user's conversation has.

What is reused from chapter 03, not reimplemented: the snapshot copy
(`build_home`), the id derivation (`derive`), the free-port/agent-environment
helpers, and the probe itself (`host/src/remote_host/latency_probe.rs`). This
file only writes data and a manifest; `run_scale_sweep.py` drives the probe.

Usage:

    python3 scripts/perf/remote-commands/scale/build_scale_data.py --list
    python3 scripts/perf/remote-commands/scale/build_scale_data.py                 # all homes
    python3 scripts/perf/remote-commands/scale/build_scale_data.py --only history  # one home
    python3 scripts/perf/remote-commands/scale/build_scale_data.py --force         # rebuild over existing
"""
import argparse
import hashlib
import importlib.util
import json
import os
import shutil
import sqlite3
from pathlib import Path

ROOT = Path(__file__).resolve().parents[4]
SCALE_ROOT = ROOT / "target" / "remote-latency" / "scale"
BASE = SCALE_ROOT / "base"
HOMES = SCALE_ROOT / "homes"
RAW = ROOT / "docs" / "testing" / "remote-latency" / "raw" / "scaling"
MANIFEST = SCALE_ROOT / "manifest.json"


def measure_module():
    """The chapter-03 harness, imported rather than copied.

    Its filename has a hyphen, so it cannot be imported by name; loading it by
    path keeps one implementation of the snapshot copy and the id derivation
    alive for both chapters.
    """
    path = Path(__file__).resolve().parents[1] / "measure-remote-latency.py"
    spec = importlib.util.spec_from_file_location("measure_remote_latency", path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


harness = measure_module()


# ── the snapshot ─────────────────────────────────────────────────────────────

def stage_base(force=False):
    """One real snapshot, shared by every home (also re-derives the anchors)."""
    marker = BASE / "MEASUREMENT_ISOLATED_HOME"
    if force and BASE.exists():
        shutil.rmtree(BASE)
    BASE.mkdir(parents=True, exist_ok=True)
    if not marker.exists():
        print(f"copying the live databases into {BASE}", flush=True)
        copied = harness.build_home(BASE)
        print(json.dumps(copied, indent=2), flush=True)
    ids = harness.derive(
        BASE / ".future/agent/agent.db",
        BASE / ".future/app/app.db",
        BASE / ".future/tasks/tasks.db",
    )
    (SCALE_ROOT / "anchors.json").write_text(
        json.dumps(ids, indent=2, ensure_ascii=False)
    )
    return ids


def new_home(name, force=False):
    home = HOMES / name
    if force and home.exists():
        shutil.rmtree(home)
    home.mkdir(parents=True, exist_ok=True)
    # The probe refuses a home without this marker: it is how the harness says
    # "this is a throwaway copy, measure it".
    (home / "MEASUREMENT_ISOLATED_HOME").touch()
    for name_ in harness.CONFIG_FILES:
        origin = BASE / name_
        if origin.is_file():
            (home / name_).parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(origin, home / name_)
    for name_ in harness.CONFIG_DIRS:
        origin = BASE / name_
        if origin.is_dir():
            shutil.copytree(origin, home / name_, dirs_exist_ok=True)
    return home


def plant(home, relative, force_copy=False):
    """A database for the home: a copy (mutable) or nothing.

    `agent.db` is only planted in homes whose measured commands read it. The
    others let the Agent create a fresh one on start: an empty database cannot
    contaminate a dimension it is not part of, and it keeps a 2.5 GB copy out of
    every home.
    """
    source = BASE / relative
    target = home / relative
    target.parent.mkdir(parents=True, exist_ok=True)
    # A previous run's Agent may have left a WAL next to the database. Copying a
    # clean snapshot over the file while that WAL survives pairs new pages with
    # old frames — measured as `database disk image is malformed`. The snapshot is
    # self-contained, so its sidecars go with it.
    for suffix in ("", "-wal", "-shm"):
        stale = Path(str(target) + suffix)
        if stale.exists():
            stale.unlink()
    shutil.copyfile(source, target)
    for suffix in ("-wal", "-shm"):
        sidecar = Path(str(source) + suffix)
        if sidecar.exists() and force_copy:
            shutil.copyfile(sidecar, Path(str(target) + suffix))
    return target


# ── one dimension at a time ──────────────────────────────────────────────────

def session_row(conn, donor_id, new_id):
    row = conn.execute(
        "SELECT revision,created_at_ms,updated_at_ms,current_metadata_json"
        " FROM sessions WHERE id=?1",
        (donor_id,),
    ).fetchone()
    conn.execute(
        "INSERT INTO sessions(id,revision,created_at_ms,updated_at_ms,current_metadata_json)"
        " VALUES (?,?,?,?,?)",
        (new_id, *row),
    )


def clone_session(db_path, donor_id, new_id, target_entries):
    """A session of `target_entries` entries built from one real donor session.

    The donor's rows are repeated whole cycles: entries (position shifted),
    history_shapes, history_display (ordinal and source_position shifted),
    message_blocks (entry_position shifted), runs and run_snapshots (run_id
    suffixed per cycle so every cycle has its own runs), run_events and
    queued_run_receipts. The donor carries no `content_references` and no
    `stream_*` rows, so there is nothing else to duplicate — and because the
    donor's bodies are inline, a clone's reply bytes are the donor's reply bytes
    per entry, which is what makes the byte column comparable across sizes.

    `history_index_state` is written for the new session, so the first read takes
    the projection the donor already has instead of rebuilding it: the
    measurement must be a page read, not chapter 02's O(entries) rebuild.
    """
    conn = sqlite3.connect(db_path)
    conn.execute("PRAGMA foreign_keys=OFF")
    conn.execute("PRAGMA journal_mode=OFF")
    conn.execute("PRAGMA synchronous=OFF")
    donor_entries, donor_ordinals = conn.execute(
        "SELECT (SELECT count(*) FROM entries WHERE session_id=?1),"
        " (SELECT count(*) FROM history_display WHERE session_id=?1)",
        (donor_id,),
    ).fetchone()

    session_row(conn, donor_id, new_id)
    cycles = max(1, -(-target_entries // donor_entries))
    written = 0
    for cycle in range(cycles):
        base = cycle * donor_entries
        limit = min(donor_entries, target_entries - base)
        if limit <= 0:
            break
        run_suffix = "" if cycle == 0 else f"-c{cycle}"
        conn.execute(
            "INSERT INTO entries(session_id,position,entry_id,entry_type,role,run_id,"
            "timestamp_ms,metadata_json,content_json)"
            " SELECT ?1, ?2 + position,"
            "  CASE WHEN ?4='' THEN entry_id ELSE entry_id || ?4 END,"
            "  entry_type, role,"
            "  CASE WHEN run_id IS NULL OR run_id='' THEN run_id ELSE run_id || ?4 END,"
            "  timestamp_ms, metadata_json, content_json"
            " FROM entries WHERE session_id=?3 AND position < ?5",
            (new_id, base, donor_id, run_suffix, limit),
        )
        written += limit
        conn.execute(
            "INSERT INTO history_shapes(session_id,position,payload)"
            " SELECT ?1, ?2 + position, payload FROM history_shapes"
            " WHERE session_id=?3 AND position < ?4",
            (new_id, base, donor_id, limit),
        )
        conn.execute(
            "INSERT INTO history_display(session_id,ordinal,source_position,is_user,payload)"
            " SELECT ?1, ?2 + ordinal, CASE WHEN source_position IS NULL THEN NULL"
            "   ELSE ?4 + source_position END, is_user, payload"
            " FROM history_display WHERE session_id=?3 AND"
            " (source_position < ?5 OR (source_position IS NULL AND ?5 = ?6))",
            (new_id, cycle * donor_ordinals, donor_id, base, limit, donor_entries),
        )
        conn.execute(
            "INSERT INTO message_blocks(session_id,entry_position,ordinal,kind,text,"
            "tool_call_id,tool_name,arguments_json,is_error,metadata_json)"
            " SELECT ?1, ?2 + entry_position, ordinal, kind, text, tool_call_id,"
            " tool_name, arguments_json, is_error, metadata_json"
            " FROM message_blocks WHERE session_id=?3 AND entry_position < ?4",
            (new_id, base, donor_id, limit),
        )
        conn.execute(
            "INSERT INTO runs(session_id,run_id,status,run_sequence,epoch,started_at_ms,"
            "completed_at_ms,error,cache_write_tokens,input_baseline,cache_read_baseline,"
            "input_tokens,cache_read_tokens,output_tokens,duration_ms)"
            " SELECT ?1, run_id || ?2, status, run_sequence, epoch, started_at_ms,"
            "completed_at_ms, error, cache_write_tokens, input_baseline,"
            "cache_read_baseline, input_tokens, cache_read_tokens, output_tokens,"
            "duration_ms FROM runs WHERE session_id=?3",
            (new_id, run_suffix, donor_id),
        )
        conn.execute(
            "INSERT INTO run_snapshots(session_id,run_id,version,cursor,payload,pricing_payloads)"
            " SELECT ?1, run_id || ?2, version, cursor, payload, pricing_payloads"
            " FROM run_snapshots WHERE session_id=?3",
            (new_id, run_suffix, donor_id),
        )
        conn.execute(
            "INSERT INTO run_events(session_id,run_id,epoch,idx,event_id,payload)"
            " SELECT ?1, run_id || ?2, epoch, idx,"
            "  CASE WHEN event_id IS NULL OR ?2='' THEN event_id ELSE event_id || ?2 END,"
            "  payload FROM run_events WHERE session_id=?3",
            (new_id, run_suffix, donor_id),
        )
        conn.execute(
            "INSERT INTO queued_run_receipts(session_id,run_id,client_request_id,"
            "run_sequence,fingerprint,payload_json,user_entry_json,status)"
            " SELECT ?1, run_id || ?2, client_request_id || ?2, run_sequence,"
            "fingerprint, payload_json, user_entry_json, status"
            " FROM queued_run_receipts WHERE session_id=?3",
            (new_id, run_suffix, donor_id),
        )
        if cycle == 0:
            # Keyed by (session,input_key) / single row per session: copied once.
            conn.execute(
                "INSERT INTO compaction_operations(session_id,input_key,input_digest,"
                "operation_id,state,result_json)"
                " SELECT ?1, input_key, input_digest, operation_id, state, result_json"
                " FROM compaction_operations WHERE session_id=?2",
                (new_id, donor_id),
            )
            conn.execute(
                "INSERT INTO session_formats(session_id,format)"
                " SELECT ?1, format FROM session_formats WHERE session_id=?2",
                (new_id, donor_id),
            )
            conn.execute(
                "INSERT INTO legacy_imports(session_id,status,fingerprint)"
                " SELECT ?1, status, fingerprint FROM legacy_imports WHERE session_id=?2",
                (new_id, donor_id),
            )
    conn.execute(
        "INSERT INTO history_index_state(session_id,projection_version)"
        " SELECT ?1, projection_version FROM history_index_state WHERE session_id=?2",
        (new_id, donor_id),
    )
    conn.commit()
    got = conn.execute(
        "SELECT count(*) FROM entries WHERE session_id=?1", (new_id,)
    ).fetchone()[0]
    conn.close()
    if got != target_entries:
        raise SystemExit(
            f"clone {new_id}: wrote {got} entries, wanted {target_entries}"
        )
    return {"session": new_id, "entries": got, "donor": donor_id}


def add_scaled_run(db_path, donor_session, donor_run, events, run_id):
    """A run of `events` events inside `donor_session`, from a real donor run.

    Events are the dimension here, so the session stays a real one: its entries,
    its thread and its other runs are untouched, and only one new run appears.
    """
    conn = sqlite3.connect(db_path)
    conn.execute("PRAGMA journal_mode=OFF")
    conn.execute("PRAGMA synchronous=OFF")
    donor_events, max_idx = conn.execute(
        "SELECT count(*), COALESCE(MAX(idx)+1,0) FROM run_events"
        " WHERE session_id=?1 AND run_id=?2",
        (donor_session, donor_run),
    ).fetchone()
    conn.execute(
        "INSERT OR IGNORE INTO runs(session_id,run_id,status,started_at_ms,completed_at_ms)"
        " VALUES (?1,?2,'completed',0,1)",
        (donor_session, run_id),
    )
    cycles = max(1, -(-events // max(1, donor_events)))
    written = 0
    for cycle in range(cycles):
        limit = min(donor_events, events - written)
        if limit <= 0:
            break
        shift = cycle * max_idx
        conn.execute(
            "INSERT INTO run_events(session_id,run_id,epoch,idx,event_id,payload)"
            " SELECT session_id, ?1, epoch, idx + ?2,"
            "  CASE WHEN event_id IS NULL OR event_id='' THEN event_id"
            "       ELSE event_id || '-c' || ?3 END, payload"
            " FROM (SELECT * FROM run_events WHERE session_id=?4 AND run_id=?5"
            "       ORDER BY idx LIMIT ?6)",
            (run_id, shift, cycle, donor_session, donor_run, limit),
        )
        written += limit
    conn.commit()
    got, top = conn.execute(
        "SELECT count(*), COALESCE(MAX(idx),0) FROM run_events WHERE session_id=?1 AND run_id=?2",
        (donor_session, run_id),
    ).fetchone()
    # A deep page must carry real payloads. Most of a long run is streaming
    # fragments (`tool_delta` / `thinking_delta`), and a window that falls inside
    # a stretch of them answers with almost no bytes — measuring the cost of
    # drawing an empty page. So the deep cursor is the start of the
    # *durable-event-densest* window within a quarter of the run of the middle:
    # still deep, but a page a client can actually draw.
    types = conn.execute(
        "SELECT idx, json_extract(payload,'$.event_type') FROM run_events"
        " WHERE session_id=?1 AND run_id=?2 ORDER BY idx",
        (donor_session, run_id),
    ).fetchall()
    durable = ("tool_start", "tool_end", "usage", "thinking_start", "thinking_end",
               "assistant_message", "user_message", "run_terminal", "run_started")
    flags = [kind in durable for _, kind in types]
    mid = len(flags) // 2
    span = 201
    window = sum(flags[:span])
    best_score, deep_idx = window, 0
    for start in range(1, max(len(flags) - span, 1)):
        window += flags[start + span - 1] if start + span - 1 < len(flags) else 0
        window -= flags[start - 1]
        if abs(start - mid) > len(flags) // 4:
            continue
        if window > best_score or (window == best_score and
                                   abs(start - mid) < abs(deep_idx - mid)):
            best_score, deep_idx = window, start
    conn.close()
    if got < events:
        raise SystemExit(f"run {run_id}: wrote {got} events, wanted {events}")
    return {"session": donor_session, "run": run_id, "events": got, "topIdx": top,
            "deepIdx": deep_idx, "deepWindowDurable": best_score}


def bind_thread(app_db, workspace_id, session_id, thread_id, title):
    """Point a desktop thread at a session, so commands that resolve a workspace
    through the thread (`list_session_files`, the observer) have a real home."""
    conn = sqlite3.connect(app_db)
    now = 1_793_000_000_000
    conn.execute(
        "INSERT OR REPLACE INTO threads(id,workspace_id,mode,title,status,pinned,readonly,"
        "agent_session_id,created_at,updated_at)"
        " VALUES (?,?,'workspace',?,'active',0,0,?,?,?)",
        (thread_id, workspace_id, title, session_id, now, now),
    )
    conn.commit()
    conn.close()


def set_workspace_path(app_db, workspace_id, path):
    conn = sqlite3.connect(app_db)
    conn.execute("UPDATE workspaces SET path=?2 WHERE id=?1", (workspace_id, str(path)))
    conn.commit()
    conn.close()


def vary_threads(app_db, count):
    """`count` threads in the copy, the rest of the tables left alone."""
    conn = sqlite3.connect(app_db)
    now = 1_793_000_000_000
    keep = [
        row[0]
        for row in conn.execute(
            "SELECT id FROM threads ORDER BY COALESCE(last_message_at,updated_at,created_at)"
            " DESC LIMIT ?1",
            (count,),
        )
    ]
    if len(keep) < count:
        # Duplicate the newest rows, which is the shape of an older conversation
        # the user still scrolls past: same columns, own id, no session binding
        # (`idx_threads_agent_session_unique` is over a non-empty session id).
        rows = conn.execute(
            "SELECT workspace_id,mode,title,status,pinned,readonly,parent_session_id,"
            "asset_root_id,title_seed,last_message_at,last_opened_at,created_at,updated_at"
            " FROM threads ORDER BY COALESCE(last_message_at,updated_at,created_at) DESC"
            " LIMIT ?1",
            (max(count - len(keep), 1),),
        ).fetchall()
        made = 0
        index = 0
        while len(keep) < count:
            row = rows[index % len(rows)]
            new_id = f"thread-scale-{count}-{made:06d}"
            # `list_sessions` skips a thread without a session id
            # (`catalog.rs`, `let Some(sid) = t.agent_session_id`), so a duplicate
            # without one would be invisible to the very command this home
            # measures. The id is shaped like a published session and is unique
            # (`idx_threads_agent_session_unique`); it does not need a row in
            # `agent.db`, because the catalog builds its row from the thread.
            duplicate_session = f"20260901-{count:06d}-{made:032d}"
            conn.execute(
                "INSERT OR REPLACE INTO threads(id,workspace_id,mode,title,status,pinned,"
                "readonly,parent_session_id,asset_root_id,title_seed,last_message_at,"
                "last_opened_at,created_at,updated_at,agent_session_id)"
                " VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)",
                (new_id, *row, duplicate_session),
            )
            # A real conversation that ran has a run row, and `list_sessions`
            # asks for every thread's latest run (`latest_run_infos`). A duplicate
            # without one would be a thread the catalog answers for more cheaply
            # than it does for a real one, which would flatten the very curve this
            # home exists to measure.
            conn.execute(
                "INSERT OR REPLACE INTO runs(id,thread_id,status,started_at,ended_at,"
                "created_at,updated_at) VALUES (?,?,'completed',?,?,?,?)",
                (f"run-scale-{count}-{made:06d}", new_id, now, now, now, now),
            )
            keep.append(new_id)
            made += 1
            index += 1
        installed = made
    else:
        conn.execute(
            "DELETE FROM threads WHERE id NOT IN (%s)" % ",".join("?" * len(keep)), keep
        )
        installed = 0
    conn.commit()
    total = conn.execute("SELECT count(*) FROM threads").fetchone()[0]
    conn.close()
    return {"threads": total, "duplicated": installed}


def copy_run_row(conn, columns, row, task_id, suffix):
    """One `task_runs` row as `task_id`'s, with identity and clock moved.

    `id` is the primary key and `(task_id, due_at)` is unique, so a duplicate
    needs its own id and its own `due_at` when the donor row had one. Every other
    column — status, kind, summary, finish reason — is the donor's, which is what
    keeps a duplicated run the same size on the wire as a real one.
    """
    values = dict(zip(columns, row))
    values["id"] = f"trn_scale_{suffix}"
    values["task_id"] = task_id
    shift = len(suffix)
    for column in ("due_at", "started_at", "finished_at"):
        if values.get(column) is not None:
            values[column] -= shift
    conn.execute(
        f"INSERT OR REPLACE INTO task_runs({','.join(columns)})"
        f" VALUES ({','.join('?' * len(columns))})",
        [values[column] for column in columns],
    )


def vary_task_runs(tasks_db, task_id, count):
    """`count` runs of one task, from the task's own real runs."""
    conn = sqlite3.connect(tasks_db)
    columns = [row[1] for row in conn.execute("PRAGMA table_info(task_runs)").fetchall()]
    keep = conn.execute(
        "SELECT count(*) FROM task_runs WHERE task_id=?1", (task_id,)
    ).fetchone()[0]
    if count < keep:
        conn.execute(
            "DELETE FROM task_runs WHERE rowid IN (SELECT rowid FROM task_runs"
            " WHERE task_id=?1 ORDER BY started_at DESC LIMIT ?2)",
            (task_id, keep - count),
        )
    elif count > keep:
        donor = conn.execute(
            "SELECT * FROM task_runs WHERE task_id=?1 LIMIT 1", (task_id,)
        ).fetchone()
        for index in range(count - keep):
            copy_run_row(conn, columns, donor, task_id, f"{task_id}_{index:07d}")
    conn.commit()
    total = conn.execute(
        "SELECT count(*) FROM task_runs WHERE task_id=?1", (task_id,)
    ).fetchone()[0]
    conn.close()
    return {"task": task_id, "runs": total}


def vary_tasks(tasks_db, count):
    """`count` tasks in the copy, each with its own (real) latest run."""
    conn = sqlite3.connect(tasks_db)
    columns = [row[1] for row in conn.execute("PRAGMA table_info(tasks)").fetchall()]
    run_columns = [row[1] for row in conn.execute("PRAGMA table_info(task_runs)").fetchall()]
    id_column = "id"
    rows = conn.execute(f"SELECT {','.join(columns)} FROM tasks").fetchall()
    present = conn.execute("SELECT count(*) FROM tasks").fetchone()[0]
    if count < present:
        keep = [
            row[0]
            for row in conn.execute(
                "SELECT task_id FROM task_runs GROUP BY task_id"
                " ORDER BY count(*) DESC LIMIT ?1",
                (count,),
            )
        ]
        if len(keep) < count:
            keep += [
                row[0]
                for row in conn.execute(
                    f"SELECT {id_column} FROM tasks WHERE {id_column} NOT IN"
                    f" ({','.join('?' * len(keep))}) LIMIT ?2",
                    (*keep, count - len(keep)),
                )
            ]
        conn.execute(
            f"DELETE FROM tasks WHERE {id_column} NOT IN ({','.join('?' * len(keep))})",
            keep,
        )
        conn.execute(
            "DELETE FROM task_runs WHERE task_id NOT IN (%s)" % ",".join("?" * len(keep)),
            keep,
        )
        made = 0
    else:
        made = count - present
        index = 0
        id_index = columns.index(id_column)
        for step in range(made):
            row = list(rows[index % len(rows)])
            original = row[id_index]
            row[id_index] = f"{original}-scale{step:05d}"
            name_index = columns.index("name") if "name" in columns else None
            if name_index is not None:
                row[name_index] = f"{row[name_index]} #{step}"
            conn.execute(
                f"INSERT OR REPLACE INTO tasks({','.join(columns)})"
                f" VALUES ({','.join('?' * len(columns))})",
                row,
            )
            # A task with no run at all is what `list_tasks` renders as "never
            # ran"; a heavy user's tasks have run, so each copy inherits the
            # donor's latest run (the per-row `latest_run_for_task` the audit
            # counted as N+1).
            donor_run = conn.execute(
                "SELECT * FROM task_runs WHERE task_id=?1 LIMIT 1", (original,)
            ).fetchone()
            if donor_run:
                copy_run_row(
                    conn, run_columns, donor_run, row[id_index], f"{row[id_index]}_{step:07d}"
                )
            index += 1
    conn.commit()
    total = conn.execute("SELECT count(*) FROM tasks").fetchone()[0]
    conn.close()
    return {"tasks": total, "duplicated": made}


def make_files(directory, count, payload=b"x"):
    """`count` regular files — the shape `list_session_files` enumerates."""
    directory.mkdir(parents=True, exist_ok=True)
    body = payload * 64
    for index in range(count):
        (directory / f"entry-{index:05d}.txt").write_bytes(body)
    return {"files": count, "bytes": len(body)}


# ── the homes ────────────────────────────────────────────────────────────────

ENTRY_SIZES = [1000, 25000, 50000]
EVENT_SIZES = [100, 1000, 10000, 25000, 50000]
THREAD_SIZES = [10, 100, 2000, 5000]
TASK_RUN_SIZES = [10, 100, 2000]
TASK_SIZES = [1, 500]
DIR_SIZES = [10, 100, 1000, 5000]


def build_history_home(anchors, force):
    """One agent.db holding every session size and every event size.

    The synthetic sessions are additional sessions; the real ones the chapter-03
    anchors name stay exactly as they are, so the four real points (41 / 236 /
    3145 / 14531) are measured on untouched rows in the same database as the
    generated ones (1000 / 25000 / 50000).
    """
    home = new_home("history", force)
    agent_db = plant(home, ".future/agent/agent.db")
    app_db = plant(home, ".future/app/app.db")
    plant(home, ".future/tasks/tasks.db")

    donor = anchors["largestSession"]["id"]
    scaled = []
    for size in ENTRY_SIZES:
        # The id keeps the timestamp shape a published session has, and the 32
        # hex characters are what keeps it from parsing as a UUID (a UUID would
        # send `session_key` through the native-identity lookup instead of the
        # literal row id).
        digest = hashlib.md5(f"scale-{size}".encode()).hexdigest()
        new_id = f"20261104-{size:06d}-{digest}"
        scaled.append(clone_session(agent_db, donor, new_id, size))

    # A deep cursor per synthetic session: the same fraction of the way in as the
    # donor's (halfway), so the offset point means the same thing at every size.
    for item in scaled:
        item["deepCursor"] = item["entries"] // 2
        item["lastCursor"] = item["entries"] - 1

    # The desktop half of these sessions: a thread in a real workspace, so
    # `list_session_files` and the observer resolve a real directory.
    workspace = anchors["thread"]["workspace"]
    workspace_id = sqlite3.connect(app_db).execute(
        "SELECT id FROM workspaces WHERE path=?1 LIMIT 1", (workspace,)
    ).fetchone()[0]
    for item in scaled:
        bind_thread(
            app_db,
            workspace_id,
            item["session"],
            f"thread-scale-{item['entries']}",
            f"scaled {item['entries']} entries",
        )
        item["thread"] = f"thread-scale-{item['entries']}"

    heavy = anchors["heaviestRun"]
    events = []
    for size in EVENT_SIZES:
        events.append(
            add_scaled_run(
                agent_db,
                heavy["session"],
                heavy["run"],
                size,
                f"run_scale_{size:06d}",
            )
        )
    return home, {
        "agentDb": str(agent_db),
        "appDb": str(app_db),
        "scaledSessions": scaled,
        "scaledRuns": events,
    }


def build_threads_home(anchors, count, force):
    home = new_home(f"threads-{count}", force)
    app_db = plant(home, ".future/app/app.db")
    plant(home, ".future/tasks/tasks.db")
    detail = vary_threads(app_db, count)
    return home, detail


def build_task_runs_home(anchors, count, force):
    home = new_home(f"task-runs-{count}", force)
    plant(home, ".future/app/app.db")
    tasks_db = plant(home, ".future/tasks/tasks.db")
    detail = vary_task_runs(tasks_db, anchors["task"]["busiest"], count)
    return home, detail


def build_tasks_home(anchors, count, force):
    home = new_home(f"tasks-{count}", force)
    plant(home, ".future/app/app.db")
    tasks_db = plant(home, ".future/tasks/tasks.db")
    detail = vary_tasks(tasks_db, count)
    return home, detail


def build_dirs_home(anchors, count, force):
    """A workspace root with `count` files; the thread and session stay real."""
    home = new_home(f"dirs-{count}", force)
    app_db = plant(home, ".future/app/app.db")
    plant(home, ".future/tasks/tasks.db")
    directory = home / "workspace-roots" / f"files-{count}"
    detail = make_files(directory, count)
    workspace_id = sqlite3.connect(app_db).execute(
        "SELECT id FROM workspaces WHERE path=?1 LIMIT 1",
        (anchors["thread"]["workspace"],),
    ).fetchone()[0]
    set_workspace_path(app_db, workspace_id, directory)
    detail.update({"workspace": str(directory), "workspaceId": workspace_id})
    return home, detail


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--only", help="build one home by name instead of all")
    parser.add_argument("--force", action="store_true", help="rebuild over existing homes")
    parser.add_argument("--list", action="store_true", help="print the home names")
    args = parser.parse_args()

    names = (
        ["history"]
        + [f"threads-{n}" for n in THREAD_SIZES]
        + [f"task-runs-{n}" for n in TASK_RUN_SIZES]
        + [f"tasks-{n}" for n in TASK_SIZES]
        + [f"dirs-{n}" for n in DIR_SIZES]
    )
    if args.list:
        print("\n".join(names))
        return

    os.umask(0o077)
    SCALE_ROOT.mkdir(parents=True, exist_ok=True)
    HOMES.mkdir(parents=True, exist_ok=True)
    RAW.mkdir(parents=True, exist_ok=True)
    anchors = stage_base(force=args.force)
    print(json.dumps({k: anchors[k] for k in
                      ("largestSession", "medianSession", "midSession", "smallSession",
                       "heaviestRun", "task", "thread", "threadsTotal", "tasksTotal")},
                     indent=2, ensure_ascii=False), flush=True)

    wanted = [args.only] if args.only else names
    manifest = {"anchorFile": str(SCALE_ROOT / "anchors.json"), "homes": {}}
    for name in wanted:
        print(f"--- {name}", flush=True)
        if name == "history":
            home, detail = build_history_home(anchors, args.force)
        elif name.startswith("threads-"):
            home, detail = build_threads_home(anchors, int(name.split("-")[1]), args.force)
        elif name.startswith("task-runs-"):
            home, detail = build_task_runs_home(anchors, int(name.split("-")[2]), args.force)
        elif name.startswith("tasks-"):
            home, detail = build_tasks_home(anchors, int(name.split("-")[1]), args.force)
        elif name.startswith("dirs-"):
            home, detail = build_dirs_home(anchors, int(name.split("-")[1]), args.force)
        else:
            raise SystemExit(f"unknown home {name}")
        manifest["homes"][name] = {"home": str(home), "detail": detail}
        print(json.dumps(detail, indent=2, ensure_ascii=False)[:2000], flush=True)

    document = json.loads(MANIFEST.read_text()) if MANIFEST.exists() else {"homes": {}}
    document["anchors"] = str(SCALE_ROOT / "anchors.json")
    document.setdefault("homes", {}).update(manifest["homes"])
    MANIFEST.write_text(json.dumps(document, indent=2, ensure_ascii=False))
    print(f"manifest -> {MANIFEST}", flush=True)


if __name__ == "__main__":
    main()
