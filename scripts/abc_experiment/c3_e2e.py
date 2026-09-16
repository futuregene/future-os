"""End-to-end check of the manual compaction path on an isolated agent.

Unit tests prove the code paths; they do not prove that a real run compacts with a
model-written summary. This drives an isolated agent (its own HOME, its own port,
its own working directory), creates a session, asks it to compact, and inspects the
journal for the checkpoint.

Nothing here touches the running production agent.
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

    agent = subprocess.Popen([str(BINARY), "agent", "--grpc-addr", f"127.0.0.1:{port}"],
                             env=env, cwd=str(workspace),
                             stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True)
    try:
        for _ in range(90):
            time.sleep(1)
            if agent.poll() is not None:
                print("agent exited early:\n", agent.stdout.read()[-1500:])
                return 1
            try:
                with socket.create_connection(("127.0.0.1", port), timeout=0.5):
                    break
            except OSError:
                continue
        else:
            print("agent never became reachable")
            return 1
        print("agent reachable", flush=True)

        (workspace / "notes.txt").write_text("The staging token is ZETA-42.\n" * 300)
        prompt = ("Read notes.txt and report the staging token verbatim. Remember it as TOKEN=ZETA-42. "
                  "Also note that deployment is never allowed without approval.")
        run = subprocess.run([str(BINARY), "run", "--mode", "json", prompt],
                             env=env, cwd=str(workspace), capture_output=True, text=True, timeout=900)
        print(f"run exit={run.returncode}", flush=True)
        try:
            payload = json.loads(run.stdout)
            session_id = payload.get("sessionId") or payload["events"][0].get("sessionId")
        except Exception:
            session_id = None
        print(f"session={session_id}", flush=True)
        if not session_id:
            print("could not determine the session id; stdout tail:\n", run.stdout[-800:])
            return 1

        print("\nrequesting manual compaction …", flush=True)
        compact = subprocess.run([str(BINARY), "session", "compact", "--session", session_id, "--json"],
                                 env=env, cwd=str(workspace), capture_output=True, text=True, timeout=600)
        print(f"compact exit={compact.returncode}\nstdout: {compact.stdout.strip()[:400]}\n"
              f"stderr: {compact.stderr.strip()[:400]}", flush=True)

        # wait for the asynchronous worker to commit
        db = home / ".future" / "agent" / "agent.db"
        import sqlite3
        found = False
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
        print(f"\ncheckpoint entries found: {found} ({len(rows) if rows else 0})", flush=True)
        for r in rows or []:
            body = r["content_json"] or ""
            model = "Model handoff summary" in body
            evidence = "Deterministic C evidence index" in body
            print(f'  {r["entry_type"]:26s} model_summary={model} evidence={evidence} ({len(body)} chars)')
            if model:
                idx = body.find("Model handoff summary")
                print("    excerpt:", body[idx:idx + 500].replace("\\n", " ")[:500])
        return 0 if found else 1
    finally:
        agent.terminate()
        try:
            agent.wait(timeout=10)
        except subprocess.TimeoutExpired:
            agent.kill()
        shutil.rmtree(home, ignore_errors=True)
        shutil.rmtree(workspace, ignore_errors=True)
        print("\nisolated HOME and workspace removed")


if __name__ == "__main__":
    sys.exit(main())
