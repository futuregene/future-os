#!/usr/bin/env python3
"""Drive `future tui` in a real pty and assert on the cells it painted.

The TUI is a full-screen terminal program, so the harness reads it the way a
terminal does: the bytes the app writes are replayed through the same cell model
the screenshot renderer uses (`scripts/screenshots/tui_terminal.py`), and an
assertion is a substring of the resulting grid. Screenshots come from the same
grid, so text and pixels cannot disagree.

The agent is `demo_agent` (a scripted gRPC server): no model, no credentials,
deterministic data. That is a deliberate boundary — this surface tests the TUI's
behaviour, not a real model's answer.
"""
from __future__ import annotations

import argparse
import json
import os
import pty
import select
import signal
import socket
import struct
import subprocess
import sys
import termios
import time
import fcntl
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "scripts" / "screenshots"))
from tui_terminal import render, screen_from_stream  # noqa: E402

BASELINES_PATH = ROOT / "harness" / "perf" / "baseline.json"


def perf_limit(case_id: str, spec: dict) -> tuple[float | None, str, float | None]:
    """The same limit resolution the JS drivers use (`harness/lib/perf.mjs`).

    Duplicated here because this driver has no Node dependency, but it must stay
    arithmetically identical: a threshold that means one thing on the desktop and
    another in the TUI is a threshold nobody can reason about.
    """
    try:
        metrics = json.loads(BASELINES_PATH.read_text(encoding="utf-8")).get("metrics", {})
    except Exception:  # noqa: BLE001 - a missing baseline falls back to `max`
        metrics = {}
    parts = [case_id, spec.get("metric", "")]
    if spec.get("label"):
        parts.append(spec["label"])
    if spec.get("stat"):
        parts.append(spec["stat"])
    key = spec.get("baselineKey") or ":".join(parts)
    base = (metrics.get(key) or {}).get("median")
    reasons = []
    limit = None
    if base is not None:
        if spec.get("toleranceAbs"):
            candidate = base + abs(spec["toleranceAbs"])
            reasons.append(f"baseline {base} + {abs(spec['toleranceAbs'])}")
        else:
            tolerance = spec.get("tolerance", 0.25)
            candidate = base * (1 + tolerance)
            reasons.append(f"baseline {base} x (1 + {tolerance})")
        if spec.get("floor") is not None:
            candidate = max(candidate, spec["floor"])
        limit = candidate
    if isinstance(spec.get("max"), (int, float)):
        limit = spec["max"] if limit is None else min(limit, spec["max"])
        reasons.append(f"ceiling {spec['max']}")
    return limit, " &".join(reasons).replace(",", " &") if reasons else "no bound", base


def future_bin() -> str:
    if os.environ.get("FUTURE_BIN"):
        return os.environ["FUTURE_BIN"]
    local = ROOT / "target" / "debug" / "future"
    return str(local) if local.exists() else "future"


def demo_agent_bin() -> str:
    if os.environ.get("FUTURE_DEMO_AGENT"):
        return os.environ["FUTURE_DEMO_AGENT"]
    local = ROOT / "target" / "debug" / "examples" / "demo_agent"
    if not local.exists():
        raise SystemExit(
            f"no demo agent at {local}; build it with "
            "`cargo build -p future-tui --example demo_agent`"
        )
    return str(local)


def free_port() -> int:
    with socket.socket() as sock:
        sock.bind(("127.0.0.1", 0))
        return sock.getsockname()[1]


def wait_port(port: int, timeout: float = 10.0) -> None:
    deadline = time.time() + timeout
    while time.time() < deadline:
        try:
            with socket.create_connection(("127.0.0.1", port), timeout=0.2):
                return
        except OSError:
            time.sleep(0.1)
    raise SystemExit(f"demo_agent did not listen on {port}")


KEYS = {
    "enter": b"\r",
    "tab": b"\t",
    "esc": b"\x1b",
    "escape": b"\x1b",
    "up": b"\x1b[A",
    "down": b"\x1b[B",
    "right": b"\x1b[C",
    "left": b"\x1b[D",
    "shift+right": b"\x1b[1;2C",
    "shift+left": b"\x1b[1;2D",
    "pageup": b"\x1b[5~",
    "pagedown": b"\x1b[6~",
    "home": b"\x1b[H",
    "end": b"\x1b[F",
    "backspace": b"\x7f",
    "ctrl+b": b"\x02",
    "ctrl+d": b"\x04",
    "ctrl+g": b"\x07",
    "ctrl+j": b"\n",
    "ctrl+c": b"\x03",
    "space": b" ",
}


class Session:
    def __init__(self, bin_path: str, agent: str, port: int, cols: int, rows: int, home: Path):
        self.cols, self.rows = cols, rows
        self.buffer = bytearray()
        self.home = home
        env = {
            **os.environ,
            "FUTURE_HOME": str(home),
            "TERM": "xterm-256color",
            "NO_COLOR": "1",
        }
        self.master, slave = pty.openpty()
        fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", rows, cols, 0, 0))
        self.proc = subprocess.Popen(
            [bin_path, "tui", "--offline", "--grpc-addr", f"127.0.0.1:{port}"],
            stdin=slave,
            stdout=slave,
            stderr=slave,
            env=env,
            cwd=str(ROOT),
            preexec_fn=os.setsid,
        )
        os.close(slave)
        self._pump(0.4)

    def _pump(self, timeout: float = 0.25) -> None:
        deadline = time.time() + timeout
        while time.time() < deadline:
            ready, _, _ = select.select([self.master], [], [], 0.05)
            if not ready:
                continue
            try:
                chunk = os.read(self.master, 65536)
            except OSError:
                return
            if not chunk:
                return
            self.buffer.extend(chunk)

    def drain(self, seconds: float) -> None:
        self._pump(seconds)

    def text(self) -> str:
        stream = self.buffer.decode("utf-8", "replace")
        cells, _ = screen_from_stream(stream, self.cols, self.rows)
        return "\n".join("".join(cell.ch for cell in row).rstrip() for row in cells).rstrip()

    def send(self, data: bytes) -> None:
        os.write(self.master, data)
        self._pump(0.35)

    def shot(self, out: Path) -> str:
        stream = self.buffer.decode("utf-8", "replace")
        work = out.parent / "_render"
        work.mkdir(parents=True, exist_ok=True)
        render(stream, out, self.cols, self.rows, work, scale=2.0)
        return str(out)

    def close(self) -> None:
        try:
            os.killpg(os.getpgid(self.proc.pid), signal.SIGTERM)
        except OSError:
            pass
        try:
            self.proc.wait(timeout=3)
        except subprocess.TimeoutExpired:
            try:
                os.killpg(os.getpgid(self.proc.pid), signal.SIGKILL)
            except OSError:
                pass
        try:
            os.close(self.master)
        except OSError:
            pass


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--steps", required=True)
    parser.add_argument("--out", required=True)
    parser.add_argument("--cols", type=int, default=130)
    parser.add_argument("--rows", type=int, default=40)
    parser.add_argument("--id", required=True)
    args = parser.parse_args()

    spec = json.loads(Path(args.steps).read_text(encoding="utf-8"))
    out_dir = Path(args.out)
    out_dir.mkdir(parents=True, exist_ok=True)
    home = out_dir / "home"
    home.mkdir(parents=True, exist_ok=True)

    port = free_port()
    agent = subprocess.Popen(
        [demo_agent_bin(), "--port", str(port)],
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
        env=os.environ,
    )
    session = None
    timeline = []
    checks = []
    status = "pass"
    fatal = None
    started = time.time()
    try:
        wait_port(port)
        session = Session(future_bin(), str(agent), port, args.cols, args.rows, home)
        session.drain(1.8)
        startup_ms = round((time.time() - started) * 1000)
        before = session.text()
        for index, step in enumerate(spec["steps"], start=1):
            action = next((key for key in step if key not in ("note", "optional", "timeout")), "note")
            entry = {"index": index, "action": action, "note": step.get("note"), "ok": True}
            t0 = time.time()
            try:
                if action == "wait":
                    session.drain(float(step["wait"]) / 1000)
                elif action == "press":
                    key = str(step["press"]).lower()
                    if key in KEYS:
                        session.send(KEYS[key])
                    elif len(key) == 1:
                        session.send(key.encode())
                    else:
                        raise RuntimeError(f"unknown key {step['press']!r}")
                elif action == "keys":
                    session.send(str(step["keys"]).encode())
                elif action == "type":
                    session.send(str(step["type"]).encode())
                elif action in ("assertText", "expectText"):
                    wanted = step[action]
                    text = session.text()
                    ok = wanted in text
                    checks.append({"kind": action, "step": index, "message": f"屏幕包含「{wanted}」", "ok": ok})
                    if not ok:
                        raise RuntimeError(f"text not found: {wanted}")
                    entry["text"] = text[-1500:]
                elif action in ("expectAbsent", "assertAbsent"):
                    wanted = step[action]
                    text = session.text()
                    ok = wanted not in text
                    checks.append({"kind": action, "step": index, "message": f"屏幕不包含「{wanted}」", "ok": ok})
                    if not ok:
                        raise RuntimeError(f"text should be absent: {wanted}")
                elif action == "measure":
                    measure_spec = step["measure"]
                    inner = next((key for key in measure_spec if key != "label"), None)
                    if inner not in ("press", "type", "keys"):
                        raise RuntimeError(f"measure supports press/type/keys, not {inner}")
                    before = session.text()
                    t0 = time.time()
                    if inner == "press":
                        key = str(measure_spec["press"]).lower()
                        session.send(KEYS.get(key, key.encode() if len(key) == 1 else b""))
                    elif inner == "type":
                        session.send(str(measure_spec["type"]).encode())
                    else:
                        session.send(str(measure_spec["keys"]).encode())
                    deadline = t0 + 3
                    while time.time() < deadline and session.text() == before:
                        session.drain(0.05)
                    entry["label"] = measure_spec.get("label")
                    entry["perfMs"] = round((time.time() - t0) * 1000)
                elif action == "assertPerf":
                    perf_spec = step["assertPerf"]
                    metric = perf_spec.get("metric")
                    if metric == "startupMs":
                        value = startup_ms if "startup_ms" in locals() else None
                    elif metric == "durationMs":
                        value = round((time.time() - started) * 1000)
                    elif metric == "measure":
                        values = [e["perfMs"] for e in timeline if e.get("perfMs") is not None and (not perf_spec.get("label") or e.get("label") == perf_spec.get("label"))]
                        value = (round(sum(values) / len(values)) if perf_spec.get("stat") == "avg" else max(values)) if values else None
                    else:
                        value = None
                    limit, reason, baseline_median = perf_limit(spec["id"], perf_spec)
                    ok = value is not None and limit is not None and value <= limit
                    checks.append({"kind": "perf", "step": index, "message": perf_spec.get("message") or f"{metric} <= {round(limit) if limit is not None else '?'}", "ok": ok, "expected": limit, "actual": value, "baseline": baseline_median, "limitReason": reason})
                    entry["value"] = value
                    if not ok:
                        raise RuntimeError(f"TUI performance expectation failed: {metric} = {value}, expected <= {limit} ({reason})")
                elif action == "shot":
                    entry["shot"] = session.shot(out_dir / "shots" / f"{index:02d}-{step['shot']}.png")
                else:
                    raise RuntimeError(f"unknown TUI step: {action}")
            except Exception as error:  # noqa: BLE001 - report, do not crash the run
                entry["ok"] = False
                entry["error"] = str(error)
                status = "fail"
                fatal = fatal or str(error)
                try:
                    entry["shot"] = session.shot(out_dir / "shots" / f"failure-{index}.png")
                except Exception:
                    pass
                entry["text"] = session.text()[-2000:]
                timeline.append(entry)
                if not step.get("optional"):
                    break
            entry["ms"] = round((time.time() - t0) * 1000)
            if "text" not in entry and action not in ("wait", "shot"):
                entry["text"] = session.text()[-1500:]
            timeline.append(entry)
        if status == "pass":
            session.shot(out_dir / "shots" / "final.png")
    finally:
        if session:
            session.close()
        agent.terminate()
        try:
            agent.wait(timeout=3)
        except subprocess.TimeoutExpired:
            agent.kill()

    result = {
        "id": spec["id"],
        "title": spec["title"],
        "surface": "tui",
        "area": spec.get("area"),
        "priority": spec.get("priority"),
        "description": spec.get("description"),
        "preconditions": spec.get("preconditions"),
        "expectations": spec.get("expectations", []),
        "judge": spec.get("judge"),
        "status": status,
        "fatal": fatal,
        "startedAt": time.strftime("%Y-%m-%dT%H:%M:%S", time.localtime(started)),
        "durationMs": round((time.time() - started) * 1000),
        "bodyText": (next((e.get("text") for e in reversed(timeline) if e.get("text")), "") or "")[:12000],
        "steps": timeline,
        "checks": checks,
        "console": {"errors": [], "warnings": [], "all": []},
        "exceptions": [],
        "network": {"failures": [], "count": 0},
        "server": {"commands": [], "fallbacks": [], "errors": []},
        "metrics": [],
        "perf": {
            "startupMs": startup_ms if "startup_ms" in locals() else None,
            "measures": [{"label": e.get("label"), "ms": e["perfMs"]} for e in timeline if e.get("perfMs") is not None],
        },
        "artifacts": {"dir": str(out_dir), "shots": [e["shot"] for e in timeline if e.get("shot")], "timeline": str(out_dir / "timeline.jsonl")},
    }
    (out_dir / "result.json").write_text(json.dumps(result, ensure_ascii=False, indent=2), encoding="utf-8")
    with (out_dir / "timeline.jsonl").open("w", encoding="utf-8") as handle:
        for entry in timeline:
            handle.write(json.dumps(entry, ensure_ascii=False) + "\n")
    return 0


if __name__ == "__main__":
    sys.exit(main())
