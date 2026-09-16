"""Run the open-book six-chain comparison against a fresh isolated agent.

`ours` uses `future session history`, which the CLI sends to the Agent over RPC. The
production agent currently running predates that command, so the archive interface
would fail for reasons unrelated to the experiment. This starts a fresh agent from
the current build on its own port, pointed at a copy of the session database, and
points the CLI at it — the production agent is never touched.
"""
import json, os, pathlib, shutil, socket, subprocess, sys, tempfile, time

WORKTREE = pathlib.Path("/Users/geilige/future-os/.worktrees/session-history-a47313")
BINARY = pathlib.Path("/Users/geilige/future-os/target/debug/future")
REAL_HOME = pathlib.Path.home() / ".future" / "agent"


def free_port():
    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


def main():
    home = pathlib.Path(tempfile.mkdtemp(prefix="sixopen-home-"))
    target = home / ".future" / "agent"
    target.mkdir(parents=True, exist_ok=True)
    for name in ("auth.json", "settings.json", "models.json"):
        if (REAL_HOME / name).exists():
            shutil.copy2(REAL_HOME / name, target / name)

    # Copy the session database so the archive interface can read the real sessions
    # without the isolated agent ever writing to the live one.
    # Symlink rather than copy: the database is ~3 GB, and the archive interface only
    # reads. The isolated agent must not be allowed to write it, so the session used
    # for probing is opened read-only by the CLI.
    src_db = REAL_HOME / "agent.db"
    if src_db.exists():
        (target / "agent.db").symlink_to(src_db)
        print(f"linked the session database ({src_db.stat().st_size / 1e9:.2f} GB)", flush=True)

    port = free_port()
    env = dict(os.environ)
    env["HOME"] = str(home)
    env["FUTURE_AGENT_GRPC_ADDR"] = f"127.0.0.1:{port}"

    agent = subprocess.Popen([str(BINARY), "agent", "--grpc-addr", f"127.0.0.1:{port}"],
                             env=env, cwd=str(WORKTREE),
                             stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True)
    try:
        for _ in range(120):
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
        print(f"isolated agent reachable on {port}\n", flush=True)

        cmd = [sys.executable, "-u", str(WORKTREE / "scripts/abc_experiment/six_chain_open.py"),
               "--root", "/Users/geilige/future-os/.future/research/abc-summary-a47313",
               "--bridge", "/Users/geilige/future-os/target/debug/examples/abc_probe_bridge",
               "--binary", str(BINARY), "--budget", "200"] + sys.argv[1:]
        result = subprocess.run(cmd, env=env, cwd=str(WORKTREE))
        return result.returncode
    finally:
        agent.terminate()
        try:
            agent.wait(timeout=15)
        except subprocess.TimeoutExpired:
            agent.kill()
        shutil.rmtree(home, ignore_errors=True)
        print("\nisolated HOME removed (production agent untouched)")


if __name__ == "__main__":
    sys.exit(main())
