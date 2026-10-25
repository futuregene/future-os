#!/usr/bin/env python3
"""Run a full-screen program in a pty of a fixed size and capture its bytes.

This is the "real screenshot" half of the TUI harness: the program under test is
the shipped binary, running in raw mode against a real pseudo-terminal, and what
we keep is exactly the byte stream it wrote — SGR colours, absolute cursor
positioning and all. `tui_terminal.py` then replays that stream through a
terminal cell model to produce the picture.

The one subtlety that makes or breaks it: the window size must be set on the
slave *before* the child reads it. `pty.fork()` execs immediately, so sizing it
afterwards races the child's first `TIOCGWINSZ` — and a program that reads 0×0
draws an empty frame (measured: the workbench wrote 0 bytes and looked hung).
So we open the pty ourselves, size it, and only then start the child with
`setsid` + `TIOCSCTTY` so the size is already there when it starts.
"""

from __future__ import annotations

import fcntl
import os
import pty
import select
import signal
import struct
import sys
import termios
import time


def run(
    argv: list[str],
    cols: int,
    rows: int,
    seconds: float,
    *,
    env: dict[str, str] | None = None,
    cwd: str | None = None,
    quiet_after: float = 0.4,
) -> bytes:
    """Capture up to `seconds` of output from `argv`, stopping early when quiet.

    Stops when the child exits, or after `quiet_after` seconds with no output once
    at least one byte has arrived. Without the quiet rule a generator-less UI
    keeps us waiting the full budget for a frame that is already complete.
    """
    master, slave = pty.openpty()
    # Size the terminal the child will inherit, before it can look at it.
    fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", rows, cols, 0, 0))

    child = os.fork()
    if child == 0:  # pragma: no cover - child never returns
        try:
            os.close(master)
            os.setsid()
            fcntl.ioctl(slave, termios.TIOCSCTTY, 0)
            os.dup2(slave, 0)
            os.dup2(slave, 1)
            os.dup2(slave, 2)
            if slave > 2:
                os.close(slave)
            child_env = dict(os.environ)
            child_env["TERM"] = "xterm-256color"
            if env:
                child_env.update(env)
            if cwd:
                os.chdir(cwd)
            os.execvpe(argv[0], argv, child_env)
        except BaseException:
            os._exit(111)

    os.close(slave)
    chunks: list[bytes] = []
    deadline = time.time() + seconds
    last = time.time()
    while time.time() < deadline:
        ready, _, _ = select.select([master], [], [], 0.1)
        if master in ready:
            try:
                data = os.read(master, 1 << 16)
            except OSError:
                break
            if not data:
                break
            chunks.append(data)
            last = time.time()
        elif chunks and time.time() - last > quiet_after:
            break
    if chunks and not ready and time.time() - last <= quiet_after:
        pass
    try:
        os.kill(child, signal.SIGKILL)
    except OSError:
        pass
    try:
        os.waitpid(child, 0)
    except OSError:
        pass
    os.close(master)
    return b"".join(chunks)


def main() -> int:
    cols, rows, seconds, out = int(sys.argv[1]), int(sys.argv[2]), float(sys.argv[3]), sys.argv[4]
    argv = sys.argv[5:]
    if argv and argv[0] == "--":
        argv = argv[1:]
    data = run(argv, cols, rows, seconds)
    with open(out, "wb") as handle:
        handle.write(data)
    print(f"captured {len(data)} bytes -> {out}")
    return 0 if data else 1


if __name__ == "__main__":
    sys.exit(main())
