#!/usr/bin/env python3
"""Run a profiling command under a disposable user home, never the live agent's.

Use FUTURE_PROFILE_HOME to keep a separately configured profile between runs.
No credentials or sessions are copied from the normal home.
"""
import os
from pathlib import Path
import subprocess
import sys
import tempfile


def run(command, home):
    env = os.environ.copy()
    env.update(HOME=str(home), USERPROFILE=str(home), FUTURE_PROFILE_ISOLATED="1")
    # A caller's explicit production endpoint must not override the profile's
    # --grpc-addr. XDG runtime discovery must not reach the live user's socket.
    env.pop("FUTURE_AGENT_SOCKET", None)
    env.pop("FUTURE_AGENT_GRPC_ADDR", None)
    env.pop("XDG_RUNTIME_DIR", None)
    return subprocess.run(command, env=env, check=False).returncode


def main(command):
    if not command:
        raise SystemExit("usage: profile-isolated.py COMMAND [ARG ...]")
    configured = os.environ.get("FUTURE_PROFILE_HOME")
    if configured:
        home = Path(configured)
        if not home.is_absolute() or home.resolve() == Path.home().resolve():
            raise SystemExit("FUTURE_PROFILE_HOME must be an absolute, separate profiling home")
        home.mkdir(parents=True, exist_ok=True)
        return run(command, home)
    with tempfile.TemporaryDirectory(prefix="future-profile-") as home:
        print("Profiling with an empty disposable home (no user credentials or sessions).", flush=True)
        return run(command, home)


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
