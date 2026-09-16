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

"""Two questions, measured.

1. Is Codex's search better than ours, or does it simply search more?
2. If every assistant message is already in the projection, what would restricting
   search to tool results save?

For (2) the archive CLI is queried directly: it calls no model, so this is free.
"""
import collections, json, os, pathlib, socket, shutil, sqlite3, subprocess, sys, tempfile, time

WT = pathlib.Path(__file__).resolve().parent.parent.parent
ROOT = ROOT
FROZEN = ROOT / "frozen-sessions"
BINARY = REPO / "target" / "debug" / "future"
REAL_HOME = pathlib.Path.home() / ".future" / "agent"
REAL = {"real-yt", "real-visual", "real-stream"}

# ── 1. efficiency of the two lookup strategies ────────────────────────────────
print("=== is Codex's search better, or just more frequent? ===")
c_open = json.loads((ROOT / "score-c3-open-ours.json").read_text())
x_open = json.loads((ROOT / "score-codex-open-codex.json").read_text())
c_closed = {r["identity"]: r for r in json.loads((ROOT / "score-c3-closed-ours.json").read_text())}
x_closed = {r["identity"]: r for r in json.loads((ROOT / "score-external-closed.json").read_text())
            if r["arm"] == "codex"}


def stats(rows, closed):
    gained = sum(r["hits"] - closed[r["identity"]]["hits"] for r in rows)
    calls = sum(r["calls"] for r in rows)
    bytes_ = sum(r["bytes"] for r in rows)
    probes = len(rows)
    return gained, calls, bytes_, probes


for label, rows, closed in (("C", c_open, c_closed), ("Codex", x_open, x_closed)):
    g, calls, b, p = stats(rows, closed)
    print(f'  {label:7s} gained {g:>3d} fields in {calls:>3d} lookups ({calls/p:5.2f}/probe), '
          f'read {b/1e6:4.2f} MB  ->  {b/max(g,1)/1024:6.1f} KB per field gained, '
          f'{g/max(calls,1):4.2f} fields per lookup')

print("\n=== on the probes where C did not search at all ===")
print(f'{"probe":18s} {"C closed":>9s} {"C open":>8s} {"C calls":>8s} {"Codex open":>11s} {"Codex calls":>12s}')
cx = {r["identity"]: r for r in x_open}
for r in c_open:
    if r["calls"] <= 1 and r["bytes"] == 0:
        i = r["identity"]
        print(f'{i:18s} {c_closed[i]["hits"]:>4d}/{c_closed[i]["of_present"]:<4d} '
              f'{r["hits"]:>4d}/{r["of_present"]:<3d} {r["calls"]:>8d} '
              f'{cx[i]["hits"]:>6d}/{cx[i]["of_present"]:<4d} {cx[i]["calls"]:>12d}')

# ── 2. what a tool-result-only search would save ──────────────────────────────
print("\n=== what would searching only tool results save? ===")


def free_port():
    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


home = pathlib.Path(tempfile.mkdtemp(prefix="search-mix-home-"))
target = home / ".future" / "agent"
target.mkdir(parents=True, exist_ok=True)
for name in ("auth.json", "settings.json", "models.json"):
    if (REAL_HOME / name).exists():
        shutil.copy2(REAL_HOME / name, target / name)
synthetic = ROOT / "synthetic-session-db" / "agent.db"
(target / "agent.db").symlink_to(synthetic if synthetic.exists() else REAL_HOME / "agent.db")
port = free_port()
env = dict(os.environ)
env["HOME"] = str(home)
env["FUTURE_AGENT_GRPC_ADDR"] = f"127.0.0.1:{port}"
agent = subprocess.Popen([str(BINARY), "agent", "--grpc-addr", f"127.0.0.1:{port}"],
                         env=env, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
try:
    for _ in range(90):
        time.sleep(1)
        try:
            with socket.create_connection(("127.0.0.1", port), timeout=0.5):
                break
        except OSError:
            continue

    manifest = json.loads((FROZEN / "manifest.json").read_text())
    # queries that mimic what the exam asks: identifiers and values
    queries = ["MB", "MiB", "call_", "PR #", "commit", "0x", "test", "log", "error"]
    kinds = collections.Counter()
    sizes = collections.Counter()
    sessions = [(name, meta["session"]) for name, meta in manifest.items()]
    for name, session in sessions:
        for q in queries:
            out = subprocess.run([str(BINARY), "session", "history", "search", "--session", session,
                                  "--query", q, "--limit", "5", "--json"],
                                 env=env, capture_output=True, text=True, timeout=180)
            if out.returncode != 0:
                continue
            try:
                payload = json.loads(out.stdout)
            except json.JSONDecodeError:
                continue
            for m in payload.get("matches", []):
                kind = f'{m.get("kind")}/{m.get("role")}'
                kinds[kind] += 1
                sizes[kind] += len(m.get("snippet") or "")

    total_hits = sum(kinds.values())
    total_bytes = sum(sizes.values())
    print(f'  sampled {total_hits} search hits across {len(sessions)} sessions and {len(queries)} queries')
    print(f'  {"kind":24s} {"hits":>6s} {"snippet bytes":>14s} {"share":>7s}')
    for kind, n in kinds.most_common():
        print(f'  {kind:24s} {n:>6d} {sizes[kind]:>14d} {100*sizes[kind]/max(total_bytes,1):>6.1f}%')
    text_bytes = sum(v for k, v in sizes.items() if k.startswith("text/"))
    print(f'\n  assistant+user text share of returned bytes: '
          f'{100*text_bytes/max(total_bytes,1):.1f}%')
finally:
    agent.terminate()
    try:
        agent.wait(timeout=15)
    except subprocess.TimeoutExpired:
        agent.kill()
    shutil.rmtree(home, ignore_errors=True)
