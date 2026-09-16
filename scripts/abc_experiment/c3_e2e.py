"""End-to-end check on an isolated agent, sized so prefix caching can be observed.

The first version compacted a ~5K-token session, which is too small for the cache
behaviour to mean anything. This builds a conversation across several turns with a
large file in play, records each turn's cache counters from the agent log, then
compacts and compares the summary request's cache counters against the turns'.

Also reports the algorithm version actually committed, so it is visible whether the
run used C3 or fell back to deterministic C.
"""
import json, os, pathlib, shutil, socket, subprocess, sys, tempfile, time

BINARY = pathlib.Path("/Users/geilige/future-os/target/debug/future")


def free_port():
    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


def main():
    home = pathlib.Path(tempfile.mkdtemp(prefix="c3-e2e-home-"))
    workspace = pathlib.Path(tempfile.mkdtemp(prefix="c3-e2e-work-"))
    port = free_port()
    print(f"isolated HOME={home}\nworkspace={workspace}\nport={port}\n", flush=True)

    real = pathlib.Path.home() / ".future" / "agent"
    (home / ".future" / "agent").mkdir(parents=True, exist_ok=True)
    for name in ("auth.json", "settings.json", "models.json"):
        if (real / name).exists():
            shutil.copy2(real / name, home / ".future" / "agent" / name)

    env = dict(os.environ)
    env["HOME"] = str(home)
    env["FUTURE_AGENT_GRPC_ADDR"] = f"127.0.0.1:{port}"
    env["RUST_LOG"] = "future_agent=info"

    agent = subprocess.Popen([str(BINARY), "agent", "--grpc-addr", f"127.0.0.1:{port}"],
                             env=env, cwd=str(workspace),
                             stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True)
    workspace.joinpath("agent.log")
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

        # Enough text that the conversation is worth caching, and a detail the
        # summary must carry (so the checkpoint is checkable too).
        (workspace / "notes.txt").write_text("The staging token is ZETA-42.\n" * 4000)
        prompts = [
            "Read notes.txt with a shell command and report how many lines it has.",
            "Summarize the first 100 lines of notes.txt in one sentence.",
            "Now report the staging token you found in notes.txt, verbatim.",
        ]
        session_id = None
        for i, prompt in enumerate(prompts):
            run = subprocess.run([str(BINARY), "run", "--mode", "json", prompt],
                                 env=env, cwd=str(workspace), capture_output=True,
                                 text=True, timeout=900)
            print(f"turn {i+1} exit={run.returncode}", flush=True)
            try:
                payload = json.loads(run.stdout)
                session_id = session_id or payload.get("sessionId") or payload["events"][0].get("sessionId")
            except Exception:
                pass

        print(f"\nsession={session_id}", flush=True)
        time.sleep(2)
        print("\nrequesting manual compaction …", flush=True)
        compact = subprocess.run([str(BINARY), "session", "compact", "--session", session_id, "--json"],
                                 env=env, cwd=str(workspace), capture_output=True, text=True, timeout=600)
        print(f"compact exit={compact.returncode}", flush=True)

        db = home / ".future" / "agent" / "agent.db"
        import sqlite3
        found, rows = False, []
        for _ in range(60):
            time.sleep(2)
            con = sqlite3.connect(f"file:{db}?mode=ro", uri=True)
            con.row_factory = sqlite3.Row
            rows = con.execute(
                "SELECT entry_type, content_json FROM entries "
                "WHERE content_json LIKE '%Model handoff summary%' "
                "   OR content_json LIKE '%Deterministic C evidence index%'").fetchall()
            if rows:
                found = True
                break
        print(f"\ncheckpoint entries found: {found} ({len(rows)})")
        for r in rows:
            body = r["content_json"] or ""
            print(f'  {r["entry_type"]:20s} model_summary={"Model handoff summary" in body} '
                  f'evidence={"Deterministic C evidence index" in body} ({len(body)} chars)')
            if "ZETA-42" in body:
                print("    summary/evidence mentions ZETA-42: yes")
        return 0 if found else 1
    finally:
        agent.terminate()
        try:
            agent.wait(timeout=10)
        except subprocess.TimeoutExpired:
            agent.kill()
        log = agent.stdout.read() if agent.stdout else ""
        print("\n--- agent log: cache counters per request ---")
        turns = [l for l in log.splitlines() if "tokens_in=" in l and "cache_read=" in l]
        for line in turns[-8:]:
            clean = line.split(" INFO ")[-1]
            print("  turn:", clean[:160])
        for line in log.splitlines():
            if "charged a" in line or "algorithm_version" in line:
                print("  ", line.split(" INFO ")[-1][:200])
        shutil.rmtree(home, ignore_errors=True)
        shutil.rmtree(workspace, ignore_errors=True)


if __name__ == "__main__":
    sys.exit(main())
