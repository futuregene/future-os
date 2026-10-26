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


#: What a terminal answers when a TUI interrogates it.
#:
#: A real terminal is not a dumb pipe: programs ask it for its background colour
#: (OSC 11), its device attributes (DA1/DA2), its name (XTVERSION), and whether
#: it supports synchronized output (DECRQM `?2026$p`), then draw a different —
#: often reduced or uncentred — first screen depending on the answers. A pty that
#: says nothing leaves the program waiting or degrading: measured on dsh-tui
#: 0.17.0, silence produced a torn, left-aligned splash in the first frame, while
#: the same app under tmux (which answers) drew a clean centred wordmark.
QUERY_ANSWERS: list[tuple[bytes, bytes]] = [
    # DA1: "VT100 with advanced video option" — what most emulators claim.
    (b"\x1b[c", b"\x1b[?1;2c"),
    (b"\x1b[0c", b"\x1b[?1;2c"),
    # DA2: secondary attributes.
    (b"\x1b[>c", b"\x1b[>0;276;0c"),
    (b"\x1b[>0c", b"\x1b[>0;276;0c"),
    # XTVERSION (`CSI > 0 q`): a DCS reply, as xterm sends.
    (b"\x1b[>0q", b"\x1bP>|xterm(370)\x1b\\"),
    # Background and foreground colour queries, in xterm's rgb: form. These are
    # the renderer's own palette, so an app that adopts the background agrees
    # with the picture it ends up in.
    (b"\x1b]11;?\x07", b"\x1b]11;rgb:1b1b/1d1d/2323\x1b\\"),
    (b"\x1b]11;?\x1b\\", b"\x1b]11;rgb:1b1b/1d1d/2323\x1b\\"),
    (b"\x1b]10;?\x07", b"\x1b]10;rgb:d8d8/dede/e9e9\x1b\\"),
    (b"\x1b]10;?\x1b\\", b"\x1b]10;rgb:d8d8/dede/e9e9\x1b\\"),
    # DECRQM for synchronized output: "supported".
    (b"\x1b[?2026$p", b"\x1b[?2026;1$y"),
    # Primary device status: "no malfunction".
    (b"\x1b[5n", b"\x1b[0n"),
]


def write_all(fd: int, payload: bytes) -> None:
    """Write `payload` completely.

    A pty accepts a bounded number of bytes at a time: `os.write` on the master
    side returns a *short* count once the child's input queue is full, and the
    caller that ignores the return value silently loses the tail. Measured on a
    19-byte scripted line: 12 bytes arrived, so the frame showed a truncated
    draft — a capture bug that looks exactly like a typing bug in the program
    under test.
    """
    view = memoryview(payload)
    while view:
        written = os.write(fd, view)
        view = view[written:]


def answer_queries(since: bytes) -> bytes:
    """Answers for the interrogations in the newest output, if any.

    Only the tail is searched: a query is answered once, and the caller passes the
    chunk it just read (queries are not split across reads in practice, and a
    split one is answered on the next chunk).
    """
    return b"".join(reply for query, reply in QUERY_ANSWERS if query in since)


def run(
    argv: list[str],
    cols: int,
    rows: int,
    seconds: float,
    *,
    env: dict[str, str] | None = None,
    cwd: str | None = None,
    quiet_after: float = 0.4,
    steps: list[tuple[float, object]] | None = None,
) -> bytes:
    """Capture up to `seconds` of output from `argv`, stopping early when quiet.

    Stops when the child exits, or after `quiet_after` seconds with no output once
    at least one byte has arrived. Without the quiet rule a generator-less UI
    keeps us waiting the full budget for a frame that is already complete.

    `steps` drives the surface the way a reader would: each entry is
    `(delay_seconds, action)`, and the actions run in order, `delay` after the
    previous one (the first, after the child starts). An action is either raw
    bytes to write to the pty, or a callable taking the text captured so far and
    returning bytes to write (or None to do nothing). The callable form is what
    lets a click be computed from the frame that is actually on screen — a
    hard-coded column would silently point at the wrong tab as soon as the
    layout moves.
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
    pending = list(steps or [])
    next_step = time.time() + pending[0][0] if pending else None
    while time.time() < deadline:
        now = time.time()
        if next_step is not None and now >= next_step:
            delay, action = pending.pop(0)
            payload = action(b"".join(chunks).decode("utf-8", "replace")) if callable(action) else action
            if payload:
                try:
                    write_all(master, payload)
                except OSError:
                    break
            last = time.time()
            next_step = time.time() + pending[0][0] if pending else None
            continue
        wait = 0.1 if next_step is None else max(0.0, min(0.1, next_step - now))
        ready, _, _ = select.select([master], [], [], wait)
        if master in ready:
            try:
                data = os.read(master, 1 << 16)
            except OSError:
                break
            if not data:
                break
            chunks.append(data)
            replies = answer_queries(data)
            if replies:
                try:
                    write_all(master, replies)
                except OSError:
                    break
            # A step may have come due while we were blocked; keep it exact by
            # re-checking against `next_step` rather than resetting the clock.
            last = time.time()
        elif next_step is None and chunks and time.time() - last > quiet_after:
            break
    try:
        os.kill(child, signal.SIGKILL)
    except OSError:
        pass
    try:
        os.waitpid(child, 0)
    except OSError:
        pass
    os.close(master)
    return trim_incomplete_tail(b"".join(chunks))


def trim_incomplete_tail(data: bytes) -> bytes:
    """Drop a character that was cut in half when the child was killed.

    The capture ends by killing the process, which can land in the middle of a
    frame — and in the middle of a multi-byte character. Decoding that tail with
    `errors="replace"` turns the stump into U+FFFD, which the renderer then
    happily draws as a cell, so a screenshot can show a `�` that the app never
    emitted (measured: the wordmark's bottom row ended in one).

    Only the incomplete *character* is dropped, not the partial frame: the
    newest written cells are still the newest content, and the cells it did not
    reach hold the previous frame, which for a repaint is the same picture.
    """
    # Walk back over continuation bytes (a UTF-8 sequence is at most 4 bytes) to
    # find the lead byte of the sequence that ends the stream.
    end = len(data) - 1
    start = end
    while start >= 0 and (data[start] & 0xC0) == 0x80 and end - start < 3:
        start -= 1
    if start < 0:
        return data
    lead = data[start]
    if lead & 0x80 == 0:
        return data  # ASCII: the stream ended on a character boundary
    if lead & 0xE0 == 0xC0:
        need = 2
    elif lead & 0xF0 == 0xE0:
        need = 3
    elif lead & 0xF8 == 0xF0:
        need = 4
    else:
        return data  # not a valid lead byte: the app emitted it, keep it visible
    if any(byte & 0xC0 != 0x80 for byte in data[start + 1 :]):
        return data  # corrupt continuation, not a plain truncation
    have = len(data) - start
    return data[:start] if have < need else data


def _self_test() -> int:
    """The tail trimmer, against the case that shipped a `\ufffd` into a frame."""
    problems = []

    def check(name, got, want):
        if got != want:
            problems.append(f"{name}: got {got!r}, want {want!r}")

    check("plain ascii untouched", trim_incomplete_tail(b"hello"), b"hello")
    check("complete cjk untouched", trim_incomplete_tail("中文".encode()), "中文".encode())
    check("complete block untouched", trim_incomplete_tail("██".encode()), "██".encode())
    # The captured tail that produced the stray U+FFFD: a `█` cut after 2 bytes.
    check("cut block trimmed", trim_incomplete_tail(b"ok" + "█".encode()[:2]), b"ok")
    check("cut hanzi trimmed", trim_incomplete_tail(b"ok" + "音".encode()[:1]), b"ok")
    check("cut emoji trimmed", trim_incomplete_tail(b"ok" + "🎯".encode()[:3]), b"ok")
    # A byte that is invalid on its own is the app's output, not a truncation.
    check("stray lead kept", trim_incomplete_tail(b"\xffok"), b"\xffok")

    if problems:
        for problem in problems:
            print(f"FAIL {problem}")
        return 1
    print("tui_capture self-test: ok")
    return 0


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
    if sys.argv[1:] == ["--self-test"]:
        raise SystemExit(_self_test())
    sys.exit(main())
