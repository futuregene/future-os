import os as _os
import pathlib as _pathlib
import subprocess as _subprocess


def _checkout():
    """The checkout this script lives in (…/<checkout>/scripts/abc_experiment/x.py)."""
    return _pathlib.Path(__file__).resolve().parents[2]


def _main_checkout():
    """The main checkout, which owns the shared .future directory.

    `--git-common-dir` resolves to <main>/.git even when running from a worktree, so the
    research directory is found without depending on any absolute path.
    """
    try:
        out = _subprocess.run(
            ["git", "rev-parse", "--path-format=absolute", "--git-common-dir"],
            cwd=_pathlib.Path(__file__).resolve().parent, capture_output=True, text=True,
            timeout=30)
        if out.returncode == 0 and out.stdout.strip():
            return _pathlib.Path(out.stdout.strip()).parent
    except Exception:
        pass
    return _checkout()


def _research():
    override = _os.environ.get("ABC_ROOT")
    if override:
        return _pathlib.Path(override)
    return _main_checkout() / ".future" / "research" / "abc-summary-a47313"


WORKTREE = _checkout()
REPO = _main_checkout()
ROOT = _research()

"""Run the open-book scoring against a fresh isolated agent.

The CLI reaches the Agent over gRPC. The scoring script passed the ambient environment,
so `future session history search` fell back to auto-discovery and landed on the
**production agent**, which predates that command — every lookup returned
"unknown command: search_session_history" and the open-book run measured a broken
interface rather than retrieval.

This starts a fresh agent from the current build on its own port, symlinks the session
database read-only, and runs the scoring inside that environment. The production agent
is never touched.
"""
import json, os, pathlib, shutil, socket, subprocess, sys, tempfile, time

WORKTREE = globals().get("WORKTREE", WORKTREE)
BINARY = REPO / "target" / "debug" / "future"
REAL_HOME = pathlib.Path.home() / ".future" / "agent"
ROOT = ROOT


def free_port():
    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


def main():
    home = pathlib.Path(tempfile.mkdtemp(prefix="c3-open-home-"))
    target = home / ".future" / "agent"
    target.mkdir(parents=True, exist_ok=True)
    for name in ("auth.json", "settings.json", "models.json"):
        if (REAL_HOME / name).exists():
            shutil.copy2(REAL_HOME / name, target / name)
    # A copy that also contains the synthetic fixtures as real sessions, so the archive
    # CLI has something to read on every chain. Without them that arm had nothing on half
    # the exam and its open-book score was a floor, not a measurement.
    synthetic = ROOT / "synthetic-session-db" / "agent.db"
    if synthetic.exists():
        (target / "agent.db").symlink_to(synthetic)
        print(f"using the synthetic-augmented session database", flush=True)
    else:
        (target / "agent.db").symlink_to(REAL_HOME / "agent.db")

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
                print("agent exited early:\n", agent.stdout.read()[-1200:])
                return 1
            try:
                with socket.create_connection(("127.0.0.1", port), timeout=0.5):
                    break
            except OSError:
                continue
        else:
            print("agent never became reachable")
            return 1
        print(f"isolated agent reachable on {port}", flush=True)

        # Prove the interface works before spending anything on scoring. The session is
        # read from the frozen manifest, which is git-ignored: no real session identifier
        # belongs in the repository.
        manifest = json.loads((ROOT / "frozen-sessions" / "manifest.json").read_text())
        sample = next(iter(manifest.values()))["session"]
        probe = subprocess.run(
            [str(BINARY), "session", "history", "search", "--session", sample,
             "--query", "the", "--limit", "3", "--json"],
            env=env, capture_output=True, text=True, timeout=300)
        ok = probe.returncode == 0 and '"matches"' in probe.stdout
        print(f"history interface check: exit={probe.returncode} usable={ok}", flush=True)
        if not ok:
            print(probe.stdout[:300], probe.stderr[:300])
            return 1

        tag = sys.argv[1] if len(sys.argv) > 1 else "c3"
        mode = sys.argv[2] if len(sys.argv) > 2 else "open"
        interface = sys.argv[3] if len(sys.argv) > 3 else "ours"
        # The projection each arm is scored against has to match the arm. Defaulting to
        # C's directory silently scores C's projection with the other arm's interface,
        # which measures an interface difference rather than the strategy.
        projdir = {"c3": "C3proj", "cdet": "Cdet", "codex": "ExternalProj",
                   "opencode": "ExternalProj"}[tag]
        prefix = f"{tag}__" if tag in ("codex", "opencode") else ""
        cmd = [sys.executable, "-u", str(WORKTREE / "scripts/abc_experiment/c3_score.py"),
               "--bridge", str(REPO / "target" / "debug" / "examples" / "abc_probe_bridge"),
               "--binary", str(BINARY), "--mode", mode, "--interface", interface,
               "--projdir", projdir, "--prefix", prefix,
               "--tag", tag, "--retag-suffix", "#pre-lookup-fix"]
        print(f"scoring {tag}: projdir={projdir} prefix={prefix!r} interface={interface}", flush=True)
        return subprocess.run(cmd, env=env, cwd=str(WORKTREE)).returncode
    finally:
        agent.terminate()
        try:
            agent.wait(timeout=15)
        except subprocess.TimeoutExpired:
            agent.kill()
        shutil.rmtree(home, ignore_errors=True)
        print("isolated HOME removed (production agent untouched)")


if __name__ == "__main__":
    sys.exit(main())
