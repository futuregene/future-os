"""Large-context end-to-end check of C3 on an isolated agent.

The first attempt measured only ~2.5K tokens of context: the agent counted lines
instead of reading the file, so the bulk never entered the conversation. Shell output
is capped at the last 500 KB per call, so this version makes the agent `cat` several
large files in sequence, which accumulates a genuinely large context, then compacts it.

Observations:
  * the context size the agent itself reports (tokens_in) before compaction
  * whether the summary request is served from the prefix cache at that size
  * whether the summary carries a marker planted at the very start of the session,
    which only holds if the whole conversation was in the request
  * the committed algorithm version, so a silent fallback cannot pass

Isolated HOME on its own port; the production agent is never touched.
"""
import json, os, pathlib, shutil, socket, sqlite3, subprocess, sys, tempfile, time

BINARY = pathlib.Path("/Users/geilige/future-os/target/debug/future")
REAL_HOME = pathlib.Path.home() / ".future" / "agent"
MARKER = "ALPHA-MARKER-7741"
FILES = 6


def free_port():
    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


def read_session_info(db):
    try:
        con = sqlite3.connect(f"file:{db}?mode=ro", uri=True)
        con.row_factory = sqlite3.Row
        row = con.execute("SELECT content_json FROM entries WHERE entry_type='session_info' "
                          "ORDER BY position DESC LIMIT 1").fetchone()
        if row and row["content_json"]:
            return json.loads(row["content_json"]).get("session") or {}
    except Exception:
        pass
    return {}


def main():
    home = pathlib.Path(tempfile.mkdtemp(prefix="c3-large-home-"))
    workspace = pathlib.Path(tempfile.mkdtemp(prefix="c3-large-work-"))
    target = home / ".future" / "agent"
    target.mkdir(parents=True, exist_ok=True)
    for name in ("auth.json", "settings.json", "models.json"):
        if (REAL_HOME / name).exists():
            shutil.copy2(REAL_HOME / name, target / name)

    port = free_port()
    env = dict(os.environ)
    env["HOME"] = str(home)
    env["FUTURE_AGENT_GRPC_ADDR"] = f"127.0.0.1:{port}"
    env["RUST_LOG"] = "future_agent=info"

    print(f"isolated HOME={home}\nport={port}\nmarker={MARKER}\n", flush=True)
    agent = subprocess.Popen([str(BINARY), "agent", "--grpc-addr", f"127.0.0.1:{port}"],
                             env=env, cwd=str(workspace),
                             stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True)
    try:
        for _ in range(90):
            time.sleep(1)
            if agent.poll() is not None:
                print("agent exited early")
                return 1
            try:
                with socket.create_connection(("127.0.0.1", port), timeout=0.5):
                    break
            except OSError:
                continue
        print("agent reachable", flush=True)

        # ~600 KB per file: under the 500 KB output cap after framing, so each `cat`
        # puts roughly 125K tokens of content straight into the history.
        for i in range(FILES):
            body = (f"Line 1 of chunk {i} for {MARKER}.\n"
                    + "".join(f"chunk{i} record {j}: channel={j % 7} value={j * 37 % 99991}\n"
                              for j in range(20_000)))
            (workspace / f"chunk{i}.log").write_text(body)
        sizes = sum((workspace / f"chunk{i}.log").stat().st_size for i in range(FILES))
        print(f"wrote {FILES} files, {sizes/1e6:.2f} MB total", flush=True)

        # Accumulate into ONE session: a fresh `future run` starts a new session, which
        # is why the first attempt measured the same 36K four times instead of growing.
        session_id = None
        for i in range(FILES):
            prompt = (f"Run this exact shell command and show its full output: cat chunk{i}.log")
            argv = [str(BINARY), "run", "--mode", "json"]
            if session_id:
                argv += ["--session", session_id]
            argv.append(prompt)
            began = time.monotonic()
            run = subprocess.run(argv, env=env, cwd=str(workspace), capture_output=True,
                                 text=True, timeout=1800)
            print(f"turn {i+1} exit={run.returncode} in {time.monotonic()-began:.0f}s", flush=True)
            try:
                data = json.loads(run.stdout)
                session_id = session_id or data.get("sessionId") or data["events"][0].get("sessionId")
            except Exception:
                pass

        print(f"\nsession={session_id}", flush=True)
        if not session_id:
            return 1
        db = target / "agent.db"
        info = read_session_info(db)
        print(f"context before compaction: tokens_in={info.get('tokens_in')} "
              f"tokens_out={info.get('tokens_out')}", flush=True)

        print("\nrequesting compaction at this size …", flush=True)
        began = time.monotonic()
        compact = subprocess.run([str(BINARY), "session", "compact", "--session", session_id, "--json"],
                                 env=env, cwd=str(workspace), capture_output=True, text=True, timeout=1800)
        print(f"compact exit={compact.returncode} in {time.monotonic()-began:.0f}s "
              f"{compact.stdout.strip()[:160]}", flush=True)

        found = False
        for _ in range(90):
            time.sleep(2)
            con = sqlite3.connect(f"file:{db}?mode=ro", uri=True)
            con.row_factory = sqlite3.Row
            rows = con.execute("SELECT entry_type, content_json FROM entries "
                               "WHERE content_json LIKE '%Model handoff summary%'").fetchall()
            if rows:
                found = True
                break
        print(f"\ncompaction committed: {found}", flush=True)
        for r in rows:
            body = r["content_json"] or ""
            print(f'  {r["entry_type"]}: {len(body)} chars; marker carried = {MARKER in body}')
        return 0 if found else 1
    finally:
        agent.terminate()
        try:
            agent.wait(timeout=15)
        except subprocess.TimeoutExpired:
            agent.kill()
        log = agent.stdout.read() if agent.stdout else ""
        print("\n--- agent accounting ---")
        for line in log.splitlines():
            if "charged a" in line or "algorithm_version" in line:
                print("  ", line.split(" INFO ")[-1][:240])
        turns = [l for l in log.splitlines() if "tokens_in=" in l and "cache_read=" in l]
        for line in turns[-8:]:
            print("   turn:", line.split(" INFO ")[-1][:175])
        shutil.rmtree(home, ignore_errors=True)
        shutil.rmtree(workspace, ignore_errors=True)


if __name__ == "__main__":
    sys.exit(main())
