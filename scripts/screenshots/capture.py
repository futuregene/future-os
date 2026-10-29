#!/usr/bin/env python3
"""Screenshot harness for the FutureOS desktop and mobile apps.

Renders the real frontends against fixed demo data, in a real browser, so
product screenshots, feature diagrams and illustrated documents can be produced
on a machine with no display. See docs/guide/screenshots.zh-CN.md (English:
docs/guide/screenshots.md).

Commands
  serve-desktop / serve-mobile     start the harness dev server (foreground)
  capture-desktop / capture-mobile run scenarios, write PNGs to --out
                                   (--baseline FILE annotates movement vs a measure run)
  check-upright-mobile             correlate a capture against an upright copy of itself
  video-desktop / video-mobile     record scenarios as mp4 (needs ffmpeg)
  variants-desktop / variants-mobile  render one scenario with each variant styling
  measure-desktop / measure-mobile record element geometry (offset diagrams)
  compare-desktop / compare-mobile compare two captures, highlighting differences
  terminal OUT.png -- CMD          render a command's real output as a terminal image
  pdf CONTENT.json OUT.pdf         assemble a document from captured PNGs and captions

Options
  --out DIR        output directory (default .screenshots/)
  --port N         override the harness port
  --cdp-port N     CDP port of the headless browser to reuse or to start
                   (default 9222; a port held by another kind of process moves
                   to the next free port rather than driving that browser)

Typical use
  python3 scripts/screenshots/capture.py serve-desktop     # terminal 1
  python3 scripts/screenshots/capture.py capture-desktop   # terminal 2
"""

import argparse
import atexit
import base64
import http.server
import json
import os
import re
import shlex
import shutil
import signal
import socket
import subprocess
import sys
import tempfile
import threading
import time
import urllib.error
import urllib.request
from functools import partial
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent.parent
SCRIPTS = ROOT / "scripts" / "screenshots"
DEFAULT_OUT = ROOT / ".screenshots"


def load_scenarios(scripts: Path = SCRIPTS) -> dict:
    """The scenario table plus the fragments under `scenarios.d/`.

    A fragment maps a platform name to scenario entries and is merged on top of
    `scenarios.json`, so two sessions can add scenarios for different platforms
    at the same time without editing (and overwriting) one shared file. A
    fragment may also override an entry of the same name. Keys starting with an
    underscore are comments, exactly as in `scenarios.json`.
    """
    table = json.loads((scripts / "scenarios.json").read_text(encoding="utf-8"))
    fragments = scripts / "scenarios.d"
    if not fragments.is_dir():
        return table
    for path in sorted(fragments.glob("*.json")):
        extra = json.loads(path.read_text(encoding="utf-8"))
        for platform, scenarios in extra.items():
            if platform.startswith("_"):
                continue
            if platform not in table or not isinstance(table[platform], dict):
                raise SystemExit(f"error: {path.name} names unknown platform {platform!r}")
            if not isinstance(scenarios, dict):
                raise SystemExit(f"error: {path.name}: {platform!r} must map scenario names to scenarios")
            table[platform].setdefault("scenarios", {}).update(scenarios)
    return table


SCENARIOS = load_scenarios()
CHROME_CANDIDATES = (
    "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
    "/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge",
    "/Applications/Chromium.app/Contents/MacOS/Chromium",
    shutil.which("google-chrome"),
    shutil.which("google-chrome-stable"),
    shutil.which("microsoft-edge"),
    shutil.which("chromium"),
    shutil.which("chromium-browser"),
    # Windows installs browsers under Program Files and never puts them on PATH.
    r"C:\Program Files\Google\Chrome\Application\chrome.exe",
    r"C:\Program Files (x86)\Google\Chrome\Application\chrome.exe",
    r"C:\Program Files\Microsoft\Edge\Application\msedge.exe",
    r"C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe",
)


def chrome_path() -> str:
    for candidate in CHROME_CANDIDATES:
        if candidate and Path(candidate).exists():
            return candidate
    raise SystemExit("error: no Chrome/Edge/Chromium found; use --cdp-port to point at a running browser")


def port_free(port: int) -> bool:
    """True when nothing accepts connections on `port` (IPv4 or IPv6 — Vite
    binds `localhost` as ::1 on macOS)."""
    for family, address in ((socket.AF_INET, "127.0.0.1"), (socket.AF_INET6, "::1")):
        try:
            with socket.socket(family, socket.SOCK_STREAM) as probe:
                if probe.connect_ex((address, port)) == 0:
                    return False
        except OSError:
            continue
    return True


def wait_for(url: str, timeout: float = 60.0) -> bool:
    deadline = time.time() + timeout
    while time.time() < deadline:
        try:
            with urllib.request.urlopen(url, timeout=2):
                return True
        except Exception:
            time.sleep(0.5)
    return False


CDP_DEFAULT_PORT = 9222
PROFILE_PREFIX = "future-shot-"
# A profile no live process references is only swept once it has been untouched
# for this long, so a capture that is starting up right now keeps its browser.
ORPHAN_PROFILE_GRACE_SECS = 300


def cdp_version(port: int) -> dict | None:
    """The `/json/version` payload of the CDP endpoint on `port`, or None.

    A port that merely *listens* is not a CDP endpoint: a Chrome started without
    remote debugging — including the default profile, which Chrome no longer
    exposes — answers 404 there. Only a real endpoint says who it is.
    """
    try:
        with urllib.request.urlopen(f"http://127.0.0.1:{port}/json/version", timeout=1.5) as response:
            return json.loads(response.read().decode("utf-8"))
    except urllib.error.HTTPError as error:
        error.close()  # a 404 is still a live socket: it is how the user's own Chrome answers
        return None
    except Exception:
        return None


def is_headless(info: dict) -> bool:
    """True for a headless Chrome — the only browser a capture may drive.

    Reusing a visible browser would navigate and screenshot the user's own
    window, so what the UA says decides this, not whether the port answers.
    """
    return "HeadlessChrome" in info.get("User-Agent", "")


def free_port(start: int) -> int:
    """The first free TCP port at or after `start` (small search window)."""
    for port in range(start, start + 64):
        if port_free(port):
            return port
    raise SystemExit(f"error: no free TCP port in {start}..{start + 63} to run headless Chrome on")


def list_processes() -> list[dict]:
    """Every process as {pid, ppid, command}.

    `ps` has no cross-platform structured output and this script needs only
    those three columns (the same ones the reap below is documented against).
    Windows has no `ps` at all, which is why it has its own listing: returning an
    empty list there looked like "no processes", so the sweep silently kept every
    orphan it was written to remove.
    """
    if os.name == "nt":
        return _windows_processes()
    try:
        listing = subprocess.run(["ps", "-Ao", "pid=,ppid=,command="],
                                 capture_output=True, text=True, check=True).stdout
    except (OSError, subprocess.SubprocessError):
        return []
    processes = []
    for line in listing.splitlines():
        pid, _, rest = line.strip().partition(" ")
        ppid, _, command = rest.strip().partition(" ")
        if pid.isdigit() and ppid.isdigit() and command:
            processes.append({"pid": int(pid), "ppid": int(ppid), "command": command})
    return processes


def _windows_processes() -> list[dict]:
    """`list_processes` on Windows, from the same CIM instances Task Manager reads.

    `tasklist` is the built-in listing and it carries neither field this script
    needs: the parent id (which tells a live run's browser from a dead run's) and
    the command line (where the `future-shot-` profile is named). So the listing
    comes from `Get-CimInstance Win32_Process` as JSON.

    UTF-8 is requested from the PowerShell host explicitly: its default output
    encoding follows the console code page, and a profile path under a non-ASCII
    user name would otherwise arrive mangled — which reads as "not our profile".
    """
    script = (
        "[Console]::OutputEncoding=[System.Text.UTF8Encoding]::new($false);"
        "Get-CimInstance Win32_Process |"
        "Select-Object ProcessId,ParentProcessId,CommandLine |"
        "ConvertTo-Json -Compress"
    )
    for shell in ("powershell", "pwsh"):
        if shutil.which(shell) is None:
            continue
        try:
            listing = subprocess.run(
                [shell, "-NoProfile", "-NonInteractive", "-Command", script],
                capture_output=True, text=True, encoding="utf-8", errors="replace",
                check=True, timeout=120,
            ).stdout
        except (OSError, subprocess.SubprocessError):
            continue
        try:
            rows = json.loads(listing or "[]")
        except ValueError:
            continue
        # `ConvertTo-Json` answers a bare object when the pipeline held one.
        if isinstance(rows, dict):
            rows = [rows]
        if not isinstance(rows, list):
            continue
        processes = []
        for row in rows:
            pid, ppid, command = (row.get("ProcessId"), row.get("ParentProcessId"),
                                  row.get("CommandLine"))
            # `CommandLine` is null for processes this account may not inspect;
            # there is nothing to match a profile against, so it is skipped.
            if isinstance(pid, int) and isinstance(ppid, int) and isinstance(command, str) and command:
                processes.append({"pid": pid, "ppid": ppid, "command": command})
        return processes
    return []


def owned_profile(command: str) -> Path | None:
    """The `future-shot-` profile a command line was started with, if any.

    Only this script creates those (`tempfile.mkdtemp(prefix=PROFILE_PREFIX)`),
    so the prefix — not the browser executable — is what identifies our own
    leftover browser.
    """
    for token in command.split():
        if token.startswith("--user-data-dir="):
            profile = Path(token.split("=", 1)[1])
            if profile.name.startswith(PROFILE_PREFIX):
                return profile
    return None


def parent_is_gone(process: dict, live: set[int]) -> bool:
    """Whether the process that started `process` has exited.

    On POSIX that is what `ppid == 1` says: the orphan was re-parented to init.
    A live run's browser cannot be mistaken for one, because its parent is the
    capture process itself.

    Windows does not re-parent — the parent id keeps naming a process that no
    longer exists — so there it is absence from `live` that means the parent is
    gone. Checking both covers either behaviour, and neither can select a browser
    whose capture is still running.
    """
    return process["ppid"] == 1 or process["ppid"] not in live


def orphan_browsers(processes: list[dict]) -> list[dict]:
    """Browsers a *dead* capture run left behind.

    Our browser is a child of the capture process, so a live run's browser still
    has that process as its parent. When a run is killed outright — SIGKILL, a
    turn timeout, an agent restart — `Browser.close()` never runs, and the
    browser is re-parented to init/launchd (ppid 1) while it keeps running with
    its profile. Both facts must hold, so the user's own browsers and a
    concurrent capture are never selected.
    """
    live = {process["pid"] for process in processes}
    return [process for process in processes
            if parent_is_gone(process, live) and owned_profile(process["command"]) is not None]


def reap_orphans() -> tuple[list[int], list[Path]]:
    """Terminate browsers left over by earlier, killed capture runs, then drop
    their temporary profiles. Returns the pids and profiles that were reaped.

    SIGKILL cannot be caught, so this startup sweep — not a handler — is what
    makes an interrupted run leave nothing behind. It runs before the browser
    for this run starts, so a leftover that holds the CDP port is gone by then.
    """
    orphans = orphan_browsers(list_processes())
    reaped_pids = []
    for orphan in orphans:
        try:
            # POSIX delivers SIGTERM, which Chrome handles by exiting cleanly.
            # Windows has no POSIX signals: this calls TerminateProcess, and the
            # browser goes down hard — which is the same outcome this sweep needs,
            # since the run it belonged to is already gone.
            os.kill(orphan["pid"], signal.SIGTERM)
            reaped_pids.append(orphan["pid"])
        except OSError:
            pass
    if orphans:
        time.sleep(1.0)  # let Chrome exit and release its profile directory
    killed_profiles = {owned_profile(orphan["command"]) for orphan in orphans}
    still_running = {owned_profile(process["command"]) for process in list_processes()}
    reaped_profiles = []
    for candidate in Path(tempfile.gettempdir()).glob(f"{PROFILE_PREFIX}*"):
        if candidate in still_running:
            continue
        if candidate not in killed_profiles:
            try:
                age = time.time() - candidate.stat().st_mtime
            except OSError:
                continue
            if age < ORPHAN_PROFILE_GRACE_SECS:
                continue
        shutil.rmtree(candidate, ignore_errors=True)
        reaped_profiles.append(candidate)
    return reaped_pids, reaped_profiles


class Browser:
    """A CDP endpoint: our own headless Chrome, or a headless browser the caller
    already runs. Owning the browser is what makes a capture work on a machine
    with no display and no browser-following tooling — and by the same token,
    never driving one the user is looking at."""

    def __init__(self, port: int):
        self.port = port
        self.process = None
        self.profile = None
        self._previous_sigterm = None
        self._closed = False

    def __enter__(self):
        info = cdp_version(self.port)
        if info is not None and is_headless(info):
            print(f"using the headless browser already listening on CDP port {self.port}")
            return self
        if info is not None:
            agent = info.get("User-Agent", "unknown")[:48]
            print(f"note: CDP port {self.port} answers but is a visible browser, not a headless "
                  f"one ({agent!r}); not driving it")
            self.port = free_port(self.port + 1)
        elif not port_free(self.port):
            print(f"note: CDP port {self.port} is held by something that is not a CDP endpoint")
            self.port = free_port(self.port + 1)
        atexit.register(self.close)
        self._install_sigterm_handler()
        self._start()
        return self

    def _start(self):
        """Start our own headless Chrome, on the next free port if the one this
        run picked is lost to another process between the check and the bind
        (`port_free` can say free and Chrome still lose that race; it then exits
        at once, which `_wait_for_cdp` notices instead of waiting out the 30 s)."""
        for attempt in range(3):
            self._spawn()
            if self._wait_for_cdp(30 if attempt == 0 else 10):
                print(f"started headless Chrome on CDP port {self.port} (pid {self.process.pid})")
                return
            self._stop_browser()
            self.port = free_port(self.port + 1)
        raise SystemExit(f"error: headless Chrome did not open CDP port {self.port}")

    def _spawn(self):
        self.profile = tempfile.mkdtemp(prefix=PROFILE_PREFIX)
        self.process = subprocess.Popen([
            chrome_path(),
            "--headless=new",
            f"--remote-debugging-port={self.port}",
            f"--user-data-dir={self.profile}",
            "--no-first-run",
            "--no-default-browser-check",
            "--disable-gpu",
            "--hide-scrollbars",
            "about:blank",
        ], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)

    def _wait_for_cdp(self, timeout: float) -> bool:
        deadline = time.time() + timeout
        while time.time() < deadline:
            if self.process.poll() is not None:
                return False  # Chrome gave up on the port; do not wait it out
            if wait_for(f"http://127.0.0.1:{self.port}/json/version", timeout=1.0):
                return True
        return False

    def _install_sigterm_handler(self):
        """Take the browser down when this run is killed with SIGTERM.

        POSIX only in practice: Windows cannot deliver SIGTERM to a Python
        process, so a killed run there leaves its browser behind and the startup
        sweep in `reap_orphans` is what removes it.
        """
        if os.name == "nt":
            return
        if threading.current_thread() is not threading.main_thread():
            return
        self._previous_sigterm = signal.getsignal(signal.SIGTERM)
        signal.signal(signal.SIGTERM, self._on_sigterm)

    def _on_sigterm(self, signum, _frame):
        self.close()
        handler = self._previous_sigterm
        signal.signal(signum, handler if handler is not None else signal.SIG_DFL)
        os.kill(os.getpid(), signum)  # re-deliver, so the exit status stays honest

    def __exit__(self, *exc):
        self.close()
        return False

    def close(self):
        """Terminate our browser and drop its profile. Idempotent: the `with`
        body, the SIGTERM handler and atexit may all reach it."""
        if self._closed:
            return
        self._closed = True
        if self._previous_sigterm is not None:
            try:
                signal.signal(signal.SIGTERM, self._previous_sigterm)
            except (OSError, ValueError):
                pass
            self._previous_sigterm = None
        atexit.unregister(self.close)
        self._stop_browser()

    def _stop_browser(self):
        if self.process and self.process.poll() is None:
            self.process.terminate()
            try:
                self.process.wait(timeout=10)
            except subprocess.TimeoutExpired:
                self.process.kill()
        self.process = None
        if self.profile:
            shutil.rmtree(self.profile, ignore_errors=True)
        self.profile = None


# ─── Servers ────────────────────────────────────────────────────────────────


def serve_desktop(args: argparse.Namespace) -> int:
    """Vite (harness config) + the stand-in PTY server for the terminal panel."""
    port = args.port or SCENARIOS["desktop"]["port"]
    if not port_free(port):
        print(f"error: port {port} is already in use; stop that server or pass --port", file=sys.stderr)
        return 1
    ensure_demo_assets()
    # Only the web renderer needs react-dom; check the pair before Vite starts.
    problem = react_pair_problem(ROOT / "desktop")
    if problem:
        print(f"error: the desktop harness cannot render: {problem}.", file=sys.stderr)
        print("  Restore the dependency tree with `npm install` at the repo root.", file=sys.stderr)
        return 1
    terminal_port = SCENARIOS["desktop"]["terminalPort"]
    if not port_free(terminal_port):
        print(f"note: port {terminal_port} is already in use; assuming a terminal server is running")
    else:
        terminal_server = subprocess.Popen(
            ["node", str(ROOT / "desktop" / "shot" / "terminal-server.mjs"), str(terminal_port)],
            cwd=ROOT,
        )
        print(f"terminal server: http://127.0.0.1:{terminal_port} (pid {terminal_server.pid})")

    print(f"desktop harness: http://localhost:{port}/shot/  — Ctrl+C to stop")
    return subprocess.call(
        ["npm", "exec", "--", "vite", "--config", "vite.shot.config.ts", "--port", str(port)],
        cwd=ROOT / "desktop",
        env={**os.environ, "SHOT_PORT": str(port)},
    )


MOBILE_WEB_DEPS = ("react-native-web", "@expo/metro-runtime", "react-dom")
MOBILE_WORKSPACE = json.loads((ROOT / "mobile" / "package.json").read_text(encoding="utf-8"))["name"]


def resolve_package(directory: Path, name: str) -> str | None:
    """Installed version of `name` as Metro/Vite would resolve it from `directory`."""
    for base in (directory, ROOT):
        manifest = base / "node_modules" / name / "package.json"
        if manifest.exists():
            return json.loads(manifest.read_text(encoding="utf-8")).get("version")
    return None


def resolve_package_here(directory: Path, name: str) -> str | None:
    """Installed version of `name` directly under `directory`'s node_modules."""
    manifest = directory / "node_modules" / name / "package.json"
    if not manifest.exists():
        return None
    return json.loads(manifest.read_text(encoding="utf-8")).get("version")


def react_pair_problem(directory: Path) -> str | None:
    """`react` and `react-dom` must be the same version or React refuses to start
    ("Invalid hook call" / a version-mismatch throw) and the page stays blank.
    Returns a message when that invariant is broken, else None.
    """
    react = resolve_package(directory, "react")
    react_dom = resolve_package(directory, "react-dom")
    if react and react_dom and react != react_dom:
        return f"react {react} and react-dom {react_dom} do not match"
    if not react_dom:
        return "react-dom is not installed (the web renderer needs it)"
    return None


def mobile_react_pair_problem() -> str | None:
    """The mobile harness pins `react`/`react-dom` to `mobile/node_modules`
    (see `pinnedReact()` in mobile/metro.config.js), so *that* pair is what the
    web bundle uses; the hoisted root copies do not matter."""
    react = resolve_package_here(ROOT / "mobile", "react")
    react_dom = resolve_package_here(ROOT / "mobile", "react-dom")
    if react is None:
        return "react is not installed in mobile/node_modules"
    if react_dom is None:
        return "react-dom is not installed in mobile/node_modules"
    if react != react_dom:
        return f"react {react} and react-dom {react_dom} do not match"
    return None


def app_react_version() -> str | None:
    """The `react` version the mobile app pins — `react-dom` must match it exactly."""
    manifest = ROOT / "mobile" / "package.json"
    if not manifest.exists():
        return None
    return json.loads(manifest.read_text(encoding="utf-8")).get("dependencies", {}).get("react")


def mobile_web_dep_specs() -> dict[str, str]:
    """Version-pinned specifiers for the harness-only web dependencies.

    Versions come from Expo's own `bundledNativeModules.json` (the table
    `expo install` consults), because guessing them breaks the app: an
    unpinned `react-dom` resolves to a release newer than `react`, and React
    refuses to start when the two versions differ.
    """
    bundled = ROOT / "node_modules" / "expo" / "bundledNativeModules.json"
    pins = json.loads(bundled.read_text(encoding="utf-8")) if bundled.exists() else {}
    fallback = {"react-native-web": "~0.21.4", "@expo/metro-runtime": "~58.0.15", "react-dom": "19.3.0"}
    specs = {name: pins.get(name, fallback[name]) for name in MOBILE_WEB_DEPS}
    # react-dom is not a loose range: it must equal the app's react exactly.
    specs["react-dom"] = app_react_version() or specs["react-dom"]
    return specs


def installed_version(name: str) -> str | None:
    """Version of `name` as the mobile app resolves it (its own node_modules
    first, then the hoisted root)."""
    return resolve_package(ROOT / "mobile", name)


def satisfies(version: str | None, wanted: str) -> bool:
    """Enough of a semver check for this purpose: exact, `~x.y.z`, `^x.y.z`."""
    if version is None:
        return False
    if wanted.startswith("~"):
        return version.split(".")[:2] == wanted[1:].split(".")[:2]
    if wanted.startswith("^"):
        return version.split(".")[0] == wanted[1:].split(".")[0]
    return version == wanted


def mobile_web_install_specs() -> list[str]:
    """Harness-only web dependencies, in install order (react-dom last, so an
    install can never leave the pair mismatched)."""
    specs = mobile_web_dep_specs()
    rest = [f"{name}@{specs[name]}" for name in MOBILE_WEB_DEPS if name != "react-dom"]
    return rest + [f"react-dom@{specs['react-dom']}"]


def mobile_web_deps_unsatisfied() -> list[str]:
    pins = mobile_web_dep_specs()
    return [
        name
        for name in MOBILE_WEB_DEPS
        if not satisfies(installed_version(name), pins[name])
    ]


def serve_mobile(args: argparse.Namespace) -> int:
    """Expo web build with the mock remote context (SHOT_WEB=1).

    react-native-web and friends are only needed for this harness, so they are
    installed on demand instead of being added to mobile/package.json. They go
    **into the mobile workspace** (`-w`), which keeps them out of the desktop
    app's resolution path: the two apps pin different React versions, and a
    react-dom hoisted to the repo root breaks whichever app it does not match.
    `--no-save` leaves package.json and package-lock.json untouched.
    """
    unsatisfied = mobile_web_deps_unsatisfied()
    if unsatisfied:
        # One command for all of them: `--no-save` packages are not in the
        # lockfile, so a second install would re-resolve and drop what the first
        # one added.
        specs = mobile_web_install_specs()
        print(f"installing harness-only web dependencies into {MOBILE_WORKSPACE}: {' '.join(specs)}")
        subprocess.check_call(
            ["npm", "install", "--no-save", "--legacy-peer-deps", "-w", MOBILE_WORKSPACE, *specs],
            cwd=ROOT,
        )
        print(f"resolved: react {resolve_package(ROOT / 'mobile', 'react')}, "
              f"react-dom {resolve_package(ROOT / 'mobile', 'react-dom')}, "
              f"react-native-web {resolve_package(ROOT / 'mobile', 'react-native-web')}")
    # The web bundle uses mobile's own react/react-dom (pinned in
    # metro.config.js); a mismatch blanks the page instead of erroring clearly,
    # so check the pair before starting Expo.
    problem = mobile_react_pair_problem()
    if problem:
        print(f"error: the mobile harness cannot render: {problem}.", file=sys.stderr)
        print("  Fix the pair in the mobile workspace and re-run:", file=sys.stderr)
        print(f"  npm install --no-save -w {MOBILE_WORKSPACE} react@{app_react_version()} react-dom@{app_react_version()}", file=sys.stderr)
        return 1
    port = args.port or SCENARIOS["mobile"]["port"]
    if not port_free(port):
        print(f"error: port {port} is already in use; stop that server or pass --port", file=sys.stderr)
        return 1
    print(f"mobile harness: http://localhost:{port}/  — Ctrl+C to stop")
    return subprocess.call(
        ["npm", "exec", "--", "expo", "start", "--web", "--port", str(port)],
        cwd=ROOT / "mobile",
        env={**os.environ, "SHOT_WEB": "1"},
    )


def ensure_demo_assets() -> None:
    """Generate the demo figures the harness serves into conversations.

    They live under `desktop/shot/assets/`, which is gitignored: they are
    generated output, not fixtures. Best-effort — without them the harness still
    captures every screen, it just shows the image fallback chip in a reply.
    """
    assets = ROOT / "desktop" / "shot" / "assets"
    if (assets / "effect-size.png").exists() and (assets / "forest-plot.png").exists():
        return
    print("generating the demo figures (scripts/screenshots/gen-demo-assets.py)")
    result = subprocess.run(
        [sys.executable, str(SCRIPTS / "gen-demo-assets.py"), "--out", str(assets)],
        capture_output=True, text=True,
    )
    if result.returncode != 0:
        print("warning: could not generate the demo figures; replies will show an image placeholder.", file=sys.stderr)
        print(f"  {(result.stdout + result.stderr).strip().splitlines()[-1] if (result.stdout + result.stderr).strip() else ''}", file=sys.stderr)
        print("  install matplotlib to fix: pip install matplotlib", file=sys.stderr)


def serves_demo_assets(port: int) -> bool:
    """True when `port` already answers with the demo figures."""
    try:
        with urllib.request.urlopen(f"http://127.0.0.1:{port}/effect-size.png", timeout=2) as response:
            return response.status == 200 and response.headers.get_content_type() == "image/png"
    except (urllib.error.URLError, OSError):
        return False


def serve_assets(port: int):
    """Serve the demo figures for the mobile harness (its mock image source)."""
    if not port_free(port):
        # Adopting a live port is only safe when it is a previous run's asset
        # server. Anything else on that number answers the image request with
        # something else (the desktop harness's terminal server, for instance),
        # and then every demo picture renders as a broken image while the
        # capture still reports success.
        if not serves_demo_assets(port):
            raise SystemExit(
                f"port {port} is held by something that is not the demo-asset server; "
                f"give the mobile harness a free assetsPort in scripts/screenshots/scenarios.json"
            )
        print(f"note: port {port} is already serving assets; reusing it")
        return None
    handler = partial(http.server.SimpleHTTPRequestHandler, directory=str(ROOT / "desktop" / "shot" / "assets"))
    server = http.server.ThreadingHTTPServer(("127.0.0.1", port), handler)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    print(f"demo assets: http://127.0.0.1:{port}/")
    return server


# ─── Capture ────────────────────────────────────────────────────────────────


def encode_video(frames_dir: Path, output: Path, max_fps: float = 30.0) -> int:
    """Encode recorded screencast frames into an mp4 with real pacing.

    Two things make the result sharper than a naive encode:

    * Chrome's screencast emits frames at the **CSS viewport size**, ignoring the
      emulated device scale factor (still captures do honour it, so they come out
      at 2x). The frames are therefore upscaled here with Lanczos so the mp4 has
      the same pixel dimensions as the stills and stays smooth on a HiDPI display
      — interpolated, not extra optical detail.
    * A fixed `-crf` below the x264 default keeps the JPEG-sourced detail instead
      of quantising it away a second time.

    The concat list carries each frame's real delay: Chrome only emits a frame
    when the page changes, so a fixed `-framerate` would compress idle stretches
    and make the video feel sped up.
    """
    timing_file = frames_dir / "timing.json"
    if not timing_file.exists():
        print("error: no frames were recorded", file=sys.stderr)
        return 1
    timing = json.loads(timing_file.read_text(encoding="utf-8"))
    frames = timing.get("frames", [])
    if not frames:
        print("error: no frames were recorded", file=sys.stderr)
        return 1

    # A short minimum duration keeps each frame visible (a 1-frame-per-ms burst
    # would otherwise flash by); the cap stops one long pause becoming a still.
    minimum = 1.0 / max_fps
    lines = ["ffconcat version 1.0"]
    for index, frame in enumerate(frames):
        following = frames[index + 1]["at"] if index + 1 < len(frames) else timing["durationMs"]
        duration = max(minimum, (following - frame["at"]) / 1000.0)
        lines.append(f"file {shlex.quote(str(frame['file']))}")
        lines.append(f"duration {duration:.3f}")
    # The concat demuxer ignores the last duration unless its file repeats.
    lines.append(f"file {shlex.quote(str(frames[-1]['file']))}")
    (frames_dir / "frames.txt").write_text("\n".join(lines) + "\n", encoding="utf-8")

    # Match the stills' pixel dimensions: the frames arrive at CSS size, the
    # stills at CSS x device-scale-factor.
    scale = max(1, int(round(timing.get("scale", 1))))
    filters = [f"scale=iw*{scale}:ih*{scale}:flags=lanczos",
               # Even dimensions are required by yuv420p.
               "scale=trunc(iw/2)*2:trunc(ih/2)*2"]

    output.parent.mkdir(parents=True, exist_ok=True)
    result = subprocess.run([
        # -nostdin matters: without it ffmpeg blocks reading standard input when
        # it is not a terminal, and the encode never finishes.
        ffmpeg_path(), "-nostdin", "-y", "-loglevel", "error",
        "-f", "concat", "-safe", "0", "-i", str(frames_dir / "frames.txt"),
        "-vf", ",".join(filters),
        "-fps_mode", "vfr", "-pix_fmt", "yuv420p",
        "-c:v", "libx264", "-preset", "medium", "-crf", "18",
        "-movflags", "+faststart",
        str(output),
    ], capture_output=True, text=True, stdin=subprocess.DEVNULL)
    if result.returncode != 0:
        print(result.stderr[-800:], file=sys.stderr)
        return 1
    print(f"   video {output}  ({len(frames)} frames, {timing['durationMs'] / 1000:.1f}s)")
    return 0


def video(args: argparse.Namespace) -> int:
    """Record scenarios as mp4 videos (see docs/guide/screenshots.md)."""
    config = SCENARIOS[args.platform]
    out_dir = Path(args.out)
    out_dir.mkdir(parents=True, exist_ok=True)
    if port_free(config["port"]):
        print(f"error: nothing is listening on port {config['port']}.", file=sys.stderr)
        print(f"  start it first:  python3 scripts/screenshots/capture.py serve-{args.platform}", file=sys.stderr)
        return 1
    if not ffmpeg_path():
        print("error: ffmpeg is required to record video; install it first", file=sys.stderr)
        return 1
    ensure_demo_assets()
    assets = serve_assets(config["assetsPort"]) if config.get("assetsPort") else None
    names = args.scenario or list(config["scenarios"])
    failed = []
    with Browser(int(args.cdp_port)) as browser:
        for name in names:
            spec = config["scenarios"].get(name)
            if spec is None:
                print(f"error: unknown scenario {name!r} for {args.platform}", file=sys.stderr)
                print(f"  available: {', '.join(config['scenarios'])}", file=sys.stderr)
                return 2
            if run_scenario(args.platform, name, spec, out_dir, browser.port, record=True) != 0:
                failed.append(name)
    if assets:
        assets.shutdown()
    print(f"\n{len(names) - len(failed)}/{len(names)} recorded into {out_dir}")
    return 1 if failed else 0


def ffmpeg_path() -> str | None:
    return shutil.which("ffmpeg")


def run_scenario(platform: str, name: str, spec: dict, out_dir: Path, cd_endpoint: str,
                 record: bool = False, inject: Path | None = None,
                 measure: Path | None = None, baseline: Path | None = None,
                 shot_suffix: str = "") -> int:
    config = SCENARIOS[platform]
    viewport = config["viewport"]
    steps = []
    for step in spec["steps"]:
        step = dict(step)
        if "shot" in step:
            target = out_dir / f"{Path(step['shot']).stem}{shot_suffix}{Path(step['shot']).suffix}"
            target.parent.mkdir(parents=True, exist_ok=True)
            step["shot"] = str(target)
        steps.append(step)
    steps_path = out_dir / f".steps-{platform}-{name}.json"
    steps_path.write_text(json.dumps(steps, ensure_ascii=False), encoding="utf-8")

    frames_dir = None
    if record:
        frames_dir = out_dir / f".frames-{platform}-{name}"
        shutil.rmtree(frames_dir, ignore_errors=True)
        frames_dir.mkdir(parents=True, exist_ok=True)

    url = spec.get("url", config["url"])
    command = [
        "node", str(SCRIPTS / "cdp.mjs"),
        "--port", str(cd_endpoint),
        "--w", str(spec.get("width", viewport["w"])),
        "--h", str(spec.get("height", viewport["h"])),
        "--mobile", "true" if viewport.get("mobile", False) else "false",
        "--url", url,
        "--settle", str(spec.get("settle", config.get("settle", 500))),
        "--ready", spec.get("ready", config["ready"]),
        "--ready-timeout", str(config.get("readyTimeout", 30000)),
        "--steps", str(steps_path),
    ]
    if frames_dir is not None:
        command += ["--frames", str(frames_dir)]
    if inject is not None:
        command += ["--inject", str(inject)]
    if measure is not None:
        command += ["--measure", str(measure)]
    if baseline is not None:
        command += ["--baseline", str(baseline)]
    if not config.get("touch", True):
        command += ["--touch", "false"]
    print(f"── {platform}/{name}{shot_suffix}")
    # `text=True` with no encoding decodes with the locale's, which on a Windows
    # console is a legacy code page: the step runner prints whatever the page said
    # — a Chinese title, an error message from the app — and one byte it cannot
    # decode takes the whole capture down (`stdout` is then `None`) rather than
    # reporting the scenario that failed. The runner speaks UTF-8; decode that.
    result = subprocess.run(command, capture_output=True, text=True, encoding="utf-8", errors="replace")
    for line in (result.stdout or "").splitlines():
        print("   " + line.replace("cdp: ", ""))
    if result.returncode != 0:
        print((result.stderr or "")[-600:], file=sys.stderr)
    steps_path.unlink(missing_ok=True)

    if frames_dir is not None:
        video = out_dir / f"{spec.get('video', f'{platform}-{name}.mp4')}"
        if encode_video(frames_dir, video) != 0:
            return 1
        shutil.rmtree(frames_dir, ignore_errors=True)
    return result.returncode


def capture(args: argparse.Namespace) -> int:
    config = SCENARIOS[args.platform]
    out_dir = Path(args.out)
    out_dir.mkdir(parents=True, exist_ok=True)

    if port_free(config["port"]):
        print(f"error: nothing is listening on port {config['port']}.", file=sys.stderr)
        print(f"  start it first:  python3 scripts/screenshots/capture.py serve-{args.platform}", file=sys.stderr)
        return 1

    ensure_demo_assets()
    assets = serve_assets(config["assetsPort"]) if config.get("assetsPort") else None
    names = args.scenario or list(config["scenarios"])
    failed = []
    with Browser(int(args.cdp_port)) as browser:
        for name in names:
            spec = config["scenarios"].get(name)
            if spec is None:
                print(f"error: unknown scenario {name!r} for {args.platform}", file=sys.stderr)
                print(f"  available: {', '.join(config['scenarios'])}", file=sys.stderr)
                return 2
            baseline = Path(args.baseline).expanduser() if args.baseline else None
            if run_scenario(args.platform, name, spec, out_dir, browser.port,
                            baseline=baseline) != 0:
                failed.append(name)
    if assets:
        assets.shutdown()
    print(f"\n{len(names) - len(failed)}/{len(names)} captured into {out_dir}")
    return 1 if failed else 0


# ─── Variants, measurement and comparison ───────────────────────────────────


# Fonts for composed sheets. Pillow's built-in bitmap font is ~11px and
# unreadable next to a 1200px panel, and captions are usually Chinese, so CJK
# faces come first: a Latin-only face silently renders every CJK glyph as a tofu
# box. (index selects a face inside a .ttc collection.)
FONT_CANDIDATES = (
    ("/System/Library/Fonts/PingFang.ttc", 0),
    ("/System/Library/Fonts/Hiragino Sans GB.ttc", 0),
    ("/Library/Fonts/Arial Unicode.ttf", 0),
    ("/System/Library/Fonts/STHeiti Light.ttc", 1),
    ("/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc", 0),
    ("/usr/share/fonts/truetype/noto/NotoSansCJK-Regular.ttc", 0),
    ("/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf", 0),
    ("C:/Windows/Fonts/msyh.ttc", 0),
    ("C:/Windows/Fonts/simhei.ttf", 0),
    ("C:/Windows/Fonts/arial.ttf", 0),
    ("/System/Library/Fonts/Helvetica.ttc", 0),
)


def load_font(size: int):
    """A readable font at `size`, or Pillow's default when none is installed."""
    from PIL import ImageFont

    for candidate, index in FONT_CANDIDATES:
        if not Path(candidate).exists():
            continue
        try:
            return ImageFont.truetype(candidate, size, index=index)
        except OSError:
            continue
    return ImageFont.load_default()


def compose_sheet(panels: list, output: Path, columns: int = 0, title: str = "",
                  panel_width: int = 1280) -> int:
    """Lay images out in a labelled grid.

    Panels are downscaled to `panel_width` so a multi-panel sheet stays a
    viewable size and the labels stay legible next to them. Used by `variants`
    and `compare`, and useful on its own for putting a set of captures into one
    image. Requires Pillow.
    """
    try:
        from PIL import Image, ImageDraw
    except ImportError:
        print("error: composing a sheet needs Pillow (pip install pillow)", file=sys.stderr)
        return 1

    columns = columns or len(panels) or 1
    columns = max(1, min(columns, len(panels) or 1))
    rows = (len(panels) + columns - 1) // columns

    loaded = []
    for panel in panels:
        image_path = Path(panel["image"])
        if not image_path.exists():
            print(f"error: missing panel image {image_path}", file=sys.stderr)
            return 1
        image = Image.open(image_path).convert("RGB")
        if image.width > panel_width:
            image = image.resize(
                (panel_width, round(image.height * panel_width / image.width)), Image.LANCZOS)
        loaded.append((image, panel.get("label", "")))

    cell_w = max(image.width for image, _ in loaded)
    cell_h = max(image.height for image, _ in loaded)
    pad = 20
    label_size = max(16, cell_w // 60)
    font = load_font(label_size)
    title_font = load_font(int(label_size * 1.5))
    label_h = label_size + 18
    title_h = int(label_size * 2.2) if title else 0
    width = pad + columns * (cell_w + pad)
    height = pad + title_h + rows * (cell_h + label_h + pad)
    sheet = Image.new("RGB", (width, height), (246, 248, 251))
    draw = ImageDraw.Draw(sheet)

    if title:
        draw.text((pad, pad + 6), title, fill=(28, 31, 36), font=title_font)

    for index, (image, label) in enumerate(loaded):
        column = index % columns
        row = index // columns
        x = pad + column * (cell_w + pad) + (cell_w - image.width) // 2
        y = pad + title_h + row * (cell_h + label_h + pad)
        sheet.paste(image, (x, y))
        draw.rectangle([x, y, x + image.width, y + image.height], outline=(210, 216, 226))
        if label:
            draw.text((pad + column * (cell_w + pad), y + image.height + 8),
                      label, fill=(28, 31, 36), font=font)

    output.parent.mkdir(parents=True, exist_ok=True)
    sheet.save(output)
    return 0


def _variant_shots(spec: dict, out_dir: Path, index: int) -> list:
    suffix = f"-v{index + 1}"
    return [
        out_dir / f"{Path(step['shot']).stem}{suffix}{Path(step['shot']).suffix}"
        for step in spec["steps"] if "shot" in step
    ]


def _substitute_variant(spec: dict, inject: str) -> dict:
    """Copy the scenario with `{variantInject}` replaced by this variant's file.

    Lets one scenario serve every variant: the inject lands at the right step
    (after any interaction that re-renders the target) instead of at boot.
    """
    resolved = json.loads(json.dumps(spec))
    for step in resolved.get("steps", []):
        if step.get("inject") == "{variantInject}":
            step["inject"] = inject
    return resolved


def variants(args: argparse.Namespace) -> int:
    """Render one scenario several times with different injected styling.

    For "show me a few icon/colour options so I can pick": each variant is the
    same screen with one `variants[].inject` JS file applied, and the results are
    composed into a single labelled sheet.

    Purely visual — this renders options *over* the real UI, it does not change
    the product. Whatever is chosen still has to be implemented in the app.
    """
    config = SCENARIOS[args.platform]
    out_dir = Path(args.out)
    out_dir.mkdir(parents=True, exist_ok=True)
    if port_free(config["port"]):
        print(f"error: nothing is listening on port {config['port']}.", file=sys.stderr)
        print(f"  start it first:  python3 scripts/screenshots/capture.py serve-{args.platform}", file=sys.stderr)
        return 1

    spec = config["scenarios"].get(args.scenario)
    if spec is None:
        print(f"error: unknown scenario {args.scenario!r} for {args.platform}", file=sys.stderr)
        print(f"  available: {', '.join(config['scenarios'])}", file=sys.stderr)
        return 2
    variants_spec = spec.get("variants")
    if not variants_spec:
        print(f'error: scenario {args.scenario!r} declares no "variants"', file=sys.stderr)
        return 2

    ensure_demo_assets()
    assets = serve_assets(config["assetsPort"]) if config.get("assetsPort") else None
    panels = []
    with Browser(int(args.cdp_port)) as browser:
        for index, variant in enumerate(variants_spec):
            inject = variant.get("inject", "")
            if inject:
                inject_path = (SCRIPTS / inject).resolve()
                if not inject_path.exists():
                    print(f"error: variant inject file not found: {inject_path}", file=sys.stderr)
                    return 2
            else:
                inject_path = None
            # The scenario carries an `{variantInject}` placeholder so each
            # variant can inject its own styling at the right point in the steps.
            resolved = _substitute_variant(spec, inject)
            if run_scenario(args.platform, args.scenario, resolved, out_dir, browser.port,
                            inject=inject_path, shot_suffix=f"-v{index + 1}") != 0:
                return 1
            shots = _variant_shots(resolved, out_dir, index)
            if not shots:
                print(f"error: scenario {args.scenario!r} captures no shot", file=sys.stderr)
                return 2
            panels.append({"image": shots[-1], "label": variant.get("label", f"variant {index + 1}")})
    if assets:
        assets.shutdown()

    output = out_dir / (args.output or f"{args.platform}-{args.scenario}-variants.png")
    if compose_sheet(panels, output, columns=args.columns,
                     title=spec.get("variantsTitle", "")) != 0:
        return 1
    print(f"\nwrote {output} ({len(panels)} variants)")
    return 0


def measure(args: argparse.Namespace) -> int:
    """Record element geometry for an offset diagram.

    Writes the measurement JSON that a scenario's `offsets` step draws, and that
    `--baseline` compares a later capture against (see the offset section of the
    guide). Run it once per version to get a comparison baseline.
    """
    config = SCENARIOS[args.platform]
    out_dir = Path(args.out)
    out_dir.mkdir(parents=True, exist_ok=True)
    if port_free(config["port"]):
        print(f"error: nothing is listening on port {config['port']}.", file=sys.stderr)
        print(f"  start it first:  python3 scripts/screenshots/capture.py serve-{args.platform}", file=sys.stderr)
        return 1
    spec = config["scenarios"].get(args.scenario)
    if spec is None:
        print(f"error: unknown scenario {args.scenario!r} for {args.platform}", file=sys.stderr)
        print(f"  available: {', '.join(config['scenarios'])}", file=sys.stderr)
        return 2
    ensure_demo_assets()
    assets = serve_assets(config["assetsPort"]) if config.get("assetsPort") else None
    measure_path = Path(args.measure).expanduser() if args.measure \
        else out_dir / f"{args.platform}-{args.scenario}-measure.json"
    with Browser(int(args.cdp_port)) as browser:
        code = run_scenario(args.platform, args.scenario, spec, out_dir, browser.port,
                            measure=measure_path)
    if assets:
        assets.shutdown()
    if code == 0:
        print(f"\nmeasurements: {measure_path}")
    return code


def compare(args: argparse.Namespace) -> int:
    """Compare two captures of the same screen and highlight what differs.

    Two uses: one screen rendered with two style options, or the same screen from
    two versions (capture each revision separately — see the guide). Outputs a
    side-by-side image with the changed regions boxed on both sides, plus a
    difference panel showing where they diverge.
    """
    try:
        from PIL import Image, ImageChops, ImageDraw
    except ImportError:
        print("error: comparing images needs Pillow (pip install pillow)", file=sys.stderr)
        return 1

    left_path = Path(args.left)
    right_path = Path(args.right)
    for image_path in (left_path, right_path):
        if not image_path.exists():
            print(f"error: no such image {image_path}", file=sys.stderr)
            return 1

    left = Image.open(left_path).convert("RGB")
    right = Image.open(right_path).convert("RGB")
    if left.size != right.size:
        # Align a same-content pair captured at different device scales.
        right = right.resize(left.size, Image.LANCZOS)

    diff = ImageChops.difference(left, right).convert("L")
    # A threshold keeps anti-aliasing noise from boxing the entire screen.
    mask = diff.point(lambda value: 255 if value >= args.threshold else 0)
    regions = changed_regions(mask, min_area=args.min_area)
    print(f"   {len(regions)} changed region(s) above threshold {args.threshold}")

    left_boxes = left.copy()
    left_draw = ImageDraw.Draw(left_boxes)
    right_boxes = right.copy()
    right_draw = ImageDraw.Draw(right_boxes)
    for index, (x, y, box_w, box_h) in enumerate(regions, start=1):
        for canvas, pen in ((left_draw, (226, 104, 95)), (right_draw, (43, 108, 255))):
            canvas.rectangle([x - 2, y - 2, x + box_w + 2, y + box_h + 2], outline=pen, width=3)
        right_draw.text((x, max(0, y - 14)), str(index), fill=(43, 108, 255))

    # Unchanged content faded, changes in red: a glance shows where they diverge.
    heat = Image.new("RGB", left.size, (246, 248, 251))
    heat.paste(left.point(lambda value: 255 - (255 - value) // 4), (0, 0))
    heat.paste(Image.new("RGB", left.size, (226, 60, 50)), (0, 0), mask)

    panels = [
        {"image": _save_sidecar(left_boxes, left_path, "left-boxes"), "label": args.label_left},
        {"image": _save_sidecar(right_boxes, right_path, "right-boxes"), "label": args.label_right},
        {"image": _save_sidecar(heat, right_path, "diff"), "label": f"差异区域（{len(regions)} 处）"},
    ]
    output = Path(args.output).expanduser()
    if compose_sheet(panels, output, columns=args.columns) != 0:
        return 1
    print(f"wrote {output}")
    return 0


def _save_sidecar(image, source: Path, suffix: str) -> Path:
    target = source.with_name(f".compare-{suffix}{source.suffix}")
    image.save(target)
    return target


# ─── Is the transcript the right way up? ────────────────────────────────────
#
# react-native-web renders an `inverted` FlatList by flipping the scroll view and
# flipping every cell back — two flips that cancel out, which is what puts the
# newest row at the bottom and the text the right way up. When the list also has
# a `refreshControl`, `ScrollView.render` clones it with the scroll view's own
# style (`{ style: props.style }`), which carries the inversion, so the refresh
# control's wrapper adds a *third* flip: the whole transcript renders upside down
# (glyphs mirrored, rows in reverse). Native does not do this, so a phone capture
# would disagree with the device.
#
# `cdp.mjs` neutralises that extra flip in the page for mobile captures. This is
# how the result is judged instead of eyeballed: `upright-probe.js` puts the
# transcript's own text beside an unflipped clone of itself in one screenshot,
# and the two tiles are correlated straight and vertically mirrored. The clone
# hangs in `document.body`, so it is upright by construction — no device, no
# second browser, no OCR and no human in the loop.

UPRIGHT_IN_PLACE = (255, 0, 255)
UPRIGHT_CONTROL = (0, 255, 255)


def colour_bbox(image, colour, tolerance: int = 40):
    """Bounding box of the pixels within `tolerance` of `colour`, else None.

    Distance is measured in the worst channel rather than as luminance: the
    probe's outlines are pure magenta and pure cyan, which are far apart in one
    channel and close in another.
    """
    from PIL import Image, ImageChops

    difference = ImageChops.difference(image, Image.new("RGB", image.size, colour))
    red, green, blue = difference.split()
    worst = ImageChops.lighter(ImageChops.lighter(red, green), blue)
    return worst.point(lambda value: 255 if value <= tolerance else 0).getbbox()


def _tile_rows(tile, height: int = 80) -> list:
    """A grayscale tile as rows of samples, at a fixed height.

    Comparing at a fixed height makes the two tiles comparable whatever their
    pixel size, and a little downsampling does not touch the signal: a mirrored
    tile is still a mirrored tile.
    """
    from PIL import Image

    width, tall = tile.size
    resized = tile.convert("L").resize((max(2, round(width * height / tall)), height), Image.LANCZOS)
    pixels = resized.load()
    return [[float(pixels[x, y]) for x in range(resized.width)] for y in range(height)]


def _normalized_correlation(rows_a, rows_b) -> float:
    """Pearson correlation of two sample grids of the same shape."""
    flat_a = [value for row in rows_a for value in row]
    flat_b = [value for row in rows_b for value in row]
    count = len(flat_a)
    mean_a = sum(flat_a) / count
    mean_b = sum(flat_b) / count
    covariance = variance_a = variance_b = 0.0
    for value_a, value_b in zip(flat_a, flat_b):
        offset_a = value_a - mean_a
        offset_b = value_b - mean_b
        covariance += offset_a * offset_b
        variance_a += offset_a * offset_a
        variance_b += offset_b * offset_b
    if variance_a <= 0 or variance_b <= 0:
        return 0.0
    return covariance / (variance_a ** 0.5 * variance_b ** 0.5)


def _best_correlation(rows_a, rows_b, shifts: int = 4) -> float:
    """The best correlation over a small alignment window.

    The two tiles are cropped from their own outlines, so a padding or rounding
    difference of a pixel or two must not read as "different text". The window is
    inset by `shifts` on every side so every candidate compares the same area.
    """
    height = min(len(rows_a), len(rows_b)) - 2 * shifts
    width = min(len(rows_a[0]), len(rows_b[0])) - 2 * shifts
    if height <= 1 or width <= 1:
        return 0.0
    best = -1.0
    for offset_y in range(-shifts, shifts + 1):
        for offset_x in range(-shifts, shifts + 1):
            window_a = [row[shifts + offset_x:shifts + offset_x + width]
                        for row in rows_a[shifts + offset_y:shifts + offset_y + height]]
            window_b = [row[shifts:shifts + width] for row in rows_b[shifts:shifts + height]]
            best = max(best, _normalized_correlation(window_a, window_b))
    return best


def report_upright(image_path: Path, viewport: dict, in_place_colour=UPRIGHT_IN_PLACE,
                   control_colour=UPRIGHT_CONTROL) -> int:
    """Print how the capture's transcript correlates with an upright copy of it."""
    try:
        from PIL import Image
    except ImportError:
        print("error: judging a capture needs Pillow (pip install pillow)", file=sys.stderr)
        return 1

    image = Image.open(image_path).convert("RGB")
    scale = image.width / viewport["w"] if viewport.get("w") else 2
    inset = max(2, round(4 * scale))
    boxes = {}
    for name, colour in (("in-place", in_place_colour), ("control", control_colour)):
        box = colour_bbox(image, colour)
        if box is None:
            print(f"error: no {name} tile in {image_path} — did the probe inject?", file=sys.stderr)
            return 1
        boxes[name] = (box[0] + inset, box[1] + inset, box[2] - inset, box[3] - inset)
    for name, box in boxes.items():
        left, top, right, bottom = box
        print(f"   {name} tile: {(right - left) / scale:.0f}x{(bottom - top) / scale:.0f} css px "
              f"at {left / scale:.0f},{top / scale:.0f}")

    in_place = _tile_rows(image.crop(boxes["in-place"]))
    control = _tile_rows(image.crop(boxes["control"]))
    mirrored_control = _tile_rows(image.crop(boxes["control"]).transpose(Image.FLIP_TOP_BOTTOM))
    upright = _best_correlation(in_place, control)
    mirrored = _best_correlation(in_place, mirrored_control)
    print(f"   upright  correlation {upright:.3f}   (the transcript against an unflipped copy of itself)")
    print(f"   mirrored correlation {mirrored:.3f}   (the same comparison, control flipped top-to-bottom)")
    if abs(upright - mirrored) < 0.05:
        print(f"   verdict: inconclusive — the two scores are within 0.05 of each other, so this "
              f"capture's text does not say which way up it is")
        return 2
    if upright > mirrored:
        print("   verdict: upright — the capture reads the right way up")
        return 0
    print("   verdict: MIRRORED — the transcript is rendered upside down "
          "(an extra vertical flip, most likely the inverted list's refresh-control wrapper)")
    return 1


def check_upright(args: argparse.Namespace) -> int:
    """Capture a scenario with the upright probe and judge the result by correlation."""
    config = SCENARIOS[args.platform]
    if port_free(config["port"]):
        print(f"error: nothing is listening on port {config['port']}.", file=sys.stderr)
        print(f"  start it first:  python3 scripts/screenshots/capture.py serve-{args.platform}", file=sys.stderr)
        return 1
    spec = config["scenarios"].get(args.scenario)
    if spec is None:
        print(f"error: unknown scenario {args.scenario!r} for {args.platform}", file=sys.stderr)
        print(f"  available: {', '.join(config['scenarios'])}", file=sys.stderr)
        return 2
    out_dir = Path(args.out)
    out_dir.mkdir(parents=True, exist_ok=True)
    shot = out_dir / f"upright-{args.scenario}.png"
    probe = {
        **spec,
        "steps": [dict(step) for step in spec["steps"] if "shot" not in step]
        + [{"inject": "upright-probe.js"}, {"wait": 300}, {"shot": shot.name}],
    }
    print(f"── {args.platform}/{args.scenario} (upright probe)")
    with Browser(int(args.cdp_port)) as browser:
        if run_scenario(args.platform, args.scenario, probe, out_dir, browser.port) != 0:
            return 1
    print(f"   image {shot}")
    return report_upright(shot, config["viewport"])


def changed_regions(mask, min_area: int = 24, pad: int = 4) -> list:
    """Group a difference mask into bounding boxes of changed content.

    A coarse row/column projection rather than full connected-component
    labelling: screens change in rectangular bands (a moved row, a resized
    button), and this keeps the dependency list to Pillow.
    """
    width, height = mask.size
    pixels = mask.load()

    bands = []
    for y in range(height):
        if any(pixels[x, y] for x in range(0, width, 2)):
            if bands and bands[-1][1] == y - 1:
                bands[-1][1] = y
            else:
                bands.append([y, y])

    regions = []
    for top, bottom in bands:
        runs = []
        for x in range(width):
            if any(pixels[x, y] for y in range(top, bottom + 1, 2)):
                if runs and runs[-1][1] == x - 1:
                    runs[-1][1] = x
                else:
                    runs.append([x, x])
        for left, right in runs:
            box_w = right - left + 1
            box_h = bottom - top + 1
            if box_w * box_h < min_area:
                continue
            regions.append((
                max(0, left - pad),
                max(0, top - pad),
                min(width - left, box_w + pad * 2),
                min(height - top, box_h + pad * 2),
            ))
    return regions


# ─── Terminal captures ──────────────────────────────────────────────────────


def _tui_binaries(args: argparse.Namespace) -> tuple[Path, Path | None]:
    """The programs a TUI capture needs, with the build hint when missing.

    Reaching an existing agent (`--addr`) needs only the TUI itself; the scripted
    agent is required only when the capture has to start one.
    """
    profile = "release" if args.release else "debug"
    tui = Path(args.bin) if args.bin else ROOT / "target" / profile / "future"
    agent: Path | None = None if args.addr else (
        Path(args.agent_bin) if args.agent_bin else ROOT / "target" / profile / "examples" / "demo_agent"
    )
    missing = [path for path in (tui, agent) if path is not None and not path.exists()]
    if missing:
        print(
            "error: missing\n  " + "\n  ".join(str(p) for p in missing) + "\nbuild them first:\n"
            "  cargo build -p future-cli --bin future\n"
            "  cargo build -p future-tui --example demo_agent",
            file=sys.stderr,
        )
        raise SystemExit(2)
    return tui, agent


def _wait_for_addr(process: subprocess.Popen, timeout: float) -> str:
    """Read the demo agent's `listening on HOST:PORT` line, then stop reading.

    The agent prints one line and then serves; anything that goes wrong before
    that line (a busy port) arrives as a nonzero exit, which must not turn into
    a hang waiting for output that will never come.
    """
    deadline = time.time() + timeout
    while time.time() < deadline:
        line = process.stdout.readline()
        if not line:
            if process.poll() is not None:
                raise SystemExit(f"demo agent exited before listening (code {process.returncode})")
            continue
        marker = "listening on "
        if marker in line:
            return line.split(marker, 1)[1].strip()
    raise SystemExit("demo agent did not report its address in time")


# The scenarios `capture.py tui --scenario` and `tui/scripts/shot.sh` accept.
#
# Each maps to what a reader would do: `chat` is the surface as it opens; `help`
# presses `?`; `palette` presses `/` on an empty draft (which is the only way
# the palette opens); the panel scenarios click a sidebar tab, because the
# workbench switches panels by click and has no key for it.
#
# The click is resolved from the frame that is on screen when the step runs, not
# from a fixed column: the tab labels move with the sidebar's width, the theme
# and the language, and a hard-coded column that lands one cell off selects the
# neighbouring panel — a wrong picture that still looks plausible.
#
# A scenario listed in `TUI_SCENARIO_AGENT_ENV` also changes what the demo agent
# pretends to have: `scrolled` asks for a longer conversation
# (`FUTURE_DEMO_LONG`), because the scripted tutorial is a six-row exchange and a
# transcript shorter than the band has nothing to scroll — `↑` on it asks for
# older rows instead of moving (see `Workbench::scroll`); `queue` asks for queued
# runs (`FUTURE_DEMO_QUEUE`), which `get_state` otherwise does not report.
TUI_SCENARIO_KEYS: dict[str, str] = {
    "chat": "",
    "help": "?",
    "palette": "/",
    # A palette entry that names a rail panel: `/` opens the palette, the word
    # filters it down to that one row, Enter runs it. These two used to answer
    # `Unknown command /usage`; what the frame must show now is the panel and a
    # hint row with no notice on it.
    "command-usage": "/usage\r",
    "command-skills": "/skills\r",
    # The same palette, typed into but not run: the filtered list is the frame.
    "palette-filter": "/theme",
    # `/theme` opens the six-palette picker; one `down` moves off the palette in
    # force (the first entry) onto `light`, and Enter applies it. The picker
    # persists what it applies, so this shot runs last (see the record's §7).
    "light": "/theme\r\x1b[B\r",
    # `/new` detaches from the session without creating one: the launchpad.
    "launchpad": "/new\r",
    # The three pickers share one overlay (`run.rs` intercepts these three verbs
    # before `live::command` sees them). Each frame is the picker itself: the
    # reasoning slider and provider groups, the six palettes, the session rows.
    # Typed through the palette rather than as a bare command so the frame is the
    # route a reader takes.
    "picker-model": "/model\r",
    "picker-theme": "/theme\r",
    "picker-sessions": "/sessions\r",
    # `#` in the draft opens the session-mention menu: the list that completes
    # `#title` into a reference, one row per conversation, its workspace in the
    # column on the right (and this is the frame the alignment bug showed in).
    "mention-sessions": "#",
    # `ctrl+b` on a terminal too narrow for the rail: the panel takes the body.
    "full-panel": "\x02",
    # A click that hands the keys back to the conversation, then three wheel-ups:
    # the frame is the chat scrolled up, with the `↑ 更早` badge. Needs the long
    # conversation (see `TUI_SCENARIO_AGENT_ENV`).
    "scrolled": "\x1b[<0;20;20M\x1b[<0;20;20m" + "\x1b[<64;20;20M" * 3,
    # `ctrl+g` opens the repository browser on the session's workspace. The
    # panel reads the real repository the workbench runs in, so this frame is a
    # checkout's own log — the same one `git log` would print — and the capture
    # needs `FUTURE_DEMO_CWD` for the scripted session to have one.
    "git": "\x07",
    # The rail's own search: `ctrl+d` gives the rail the keys, `/` opens its
    # search line, and the word narrows both panels to the conversations that
    # answer it. `workbench` matches one of the demo agent's three sessions.
    "rail-search": "\x04/workbench",
    # A reply that is still arriving: the demo agent answers this prompt with a
    # scripted turn (`STREAM_DELTAS`), one delta every 120 ms, and never ends it
    # — so the frame shows the activity row and a transcript that stops
    # mid-sentence, which is what "streaming" looks like and what a finished
    # reply does not. Needs a capture long enough to reach the deltas
    # (`--seconds 6` or more). The frame is reproducible run to run (measured:
    # byte-identical PNGs) because the deltas are spaced by the demo agent's own
    # clock and a frame is painted per batch; a machine much slower than this one
    # may land one delta later, which is a different picture by one line.
    "streaming": "解释一下这个 off-by-one，要具体一点\r",
    # The same live turn, with two prompts queued behind it (`FUTURE_DEMO_QUEUE`):
    # the composer's queue band, the transcript that does not contain them, and
    # the activity row still spinning — the frame a reader sees when they send
    # something mid-reply.
    "queue": "解释一下这个 off-by-one，要具体一点\r",
}
# Scenarios that change what the demo agent pretends to have, and the environment
# that makes it. `scrolled` needs the longer conversation (`FUTURE_DEMO_LONG`) —
# a transcript shorter than the band has nothing to scroll — and `queue` needs
# the queued runs `get_state` otherwise does not report. See `TUI_SCENARIO_KEYS`.
TUI_SCENARIO_AGENT_ENV: dict[str, dict[str, str]] = {
    "scrolled": {"FUTURE_DEMO_LONG": "1"},
    "queue": {"FUTURE_DEMO_QUEUE": "1"},
}

# The one scenario whose keystrokes are *prose* rather than a verb: `streaming`
# submits a prompt, so the words are the reader's and have to be in the language
# the frame is pinned to — a Chinese question in an English frame (or the other
# way round) is the mismatch this payload exists to avoid. Everything else in
# `TUI_SCENARIO_KEYS` is a verb, a key or a query word, and reads the same in
# both languages.
TUI_SCENARIO_KEYS_EN: dict[str, str] = {
    "streaming": "Explain this off-by-one in detail, and where else it hides\r",
}


# Each entry lists the label in every language the surface ships: the capture
# pins one language (`--lang`), and a tab the frame spells differently is a
# click that lands on nothing.
TUI_SCENARIO_TABS: dict[str, tuple[str, ...]] = {
    "todo": ("待办", "To-do"),
    "usage": ("用量", "Usage"),
    "skills": ("技能", "Skills"),
    "sessions": ("会话", "Sessions"),
}
# The divider drag, in columns to the right. A drag is the one pointer gesture a
# keystroke string cannot express: it needs the seam's own column, so the step
# reads it from the frame on screen (see `_tui_drag_divider`).
TUI_SCENARIO_DRAGS: dict[str, int] = {"drag": 6}


def _painted_cells(row: list) -> list[tuple[int, str]]:
    """`(cell index, character)` for each painted character, left to right.

    A wide (CJK) character owns two cells: its own, and a `""` continuation the
    renderer keeps so that the following cell starts at the right column. Using
    a string index instead of the cell index is what made a click land on the
    tab *before* its target: `待办 任务 会话 用量` is 11 characters but 19
    columns, so every target after the first CJK label was clicked short.
    """
    painted = []
    for index, cell in enumerate(row):
        if cell.ch:
            painted.append((index, cell.ch))
    return painted


def _find_word_column(cells: list, word: str) -> tuple[int, int] | None:
    """Locate `word` in a painted grid: `(row, first cell index)`."""
    for row_index, row in enumerate(cells):
        painted = _painted_cells(row)
        for start in range(len(painted)):
            if "".join(ch for _, ch in painted[start : start + len(word)]) == word:
                return row_index, painted[start][0]
    return None


def _tui_click_tab(words: tuple[str, ...], cols: int, rows: int, settle: float):
    """A step that clicks the sidebar tab labelled with any of `words`.

    The surface ships more than one language, so the label is looked up in each
    in turn rather than assumed: the frame on screen says which one is in use.
    """

    def action(text: str) -> bytes | None:
        import tui_terminal

        cells, _ = tui_terminal.screen_from_stream(text, cols, rows)
        found = None
        for word in words:
            found = _find_word_column(cells, word)
            if found is not None:
                break
        if found is None:
            raise SystemExit(
                f"error: the frame has no {words!r} tab to click at {cols}x{rows}. On a "
                "terminal too narrow for the sidebar the tabs are unreachable; try a "
                "wider --cols."
            )
        row_index, column = found
        # SGR 1006, 1-based: press then release at the same cell.
        press = f"\x1b[<0;{column + 1};{row_index + 1}M".encode()
        release = f"\x1b[<0;{column + 1};{row_index + 1}m".encode()
        return press + release

    return (settle, action)


def _tui_drag_divider(dx: int, cols: int, rows: int, settle: float):
    """A step that grabs the sidebar divider and drags it `dx` columns right.

    The seam is read from the frame that is on screen rather than assumed: the
    rail's width moves with the terminal, and a hard-coded column would grab
    whatever happens to be there.
    """

    def action(text: str) -> bytes | None:
        import tui_terminal

        cells, _ = tui_terminal.screen_from_stream(text, cols, rows)
        # The divider is the `│` column painted on the most rows of the body; the
        # input box and the panel rows have their own verticals, and counting
        # rows is what tells the full-height one apart.
        tally: dict[int, int] = {}
        for row in cells[: max(2, len(cells) - 4)]:
            for index, ch in _painted_cells(row):
                if ch == "│":
                    tally[index] = tally.get(index, 0) + 1
        if not tally:
            raise SystemExit(
                "error: no divider in the frame — the terminal is too narrow for the "
                f"rail ({cols}x{rows}); the drag needs at least 88 columns."
            )
        seam = max(tally, key=lambda index: tally[index])
        y = max(1, len(cells) // 3)
        # SGR 1006, 1-based: press on the seam, move with the button held, release.
        bytes_out = f"\x1b[<0;{seam + 1};{y + 1}M"
        bytes_out += "".join(
            f"\x1b[<32;{seam + 1 + step};{y + 1}M" for step in range(1, dx + 1)
        )
        bytes_out += f"\x1b[<0;{seam + 1 + dx};{y + 1}m"
        return bytes_out.encode()

    return (settle, action)


def _tui_steps(scenario: str, cols: int, rows: int, settle: float = 2.5, lang: str = "zh"):
    """The input script for a scenario, or a clear error for an unknown one.

    `lang` only reaches the scenarios that submit prose (see
    `TUI_SCENARIO_KEYS_EN`); every other keystroke is a verb or a key, and the
    same bytes work in both languages.
    """
    if scenario in TUI_SCENARIO_KEYS:
        keys = TUI_SCENARIO_KEYS[scenario]
        if lang.startswith("en"):
            keys = TUI_SCENARIO_KEYS_EN.get(scenario, keys)
        return [(settle, keys.encode())] if keys else []
    if scenario in TUI_SCENARIO_TABS:
        return [_tui_click_tab(TUI_SCENARIO_TABS[scenario], cols, rows, settle)]
    if scenario in TUI_SCENARIO_DRAGS:
        return [_tui_drag_divider(TUI_SCENARIO_DRAGS[scenario], cols, rows, settle)]
    known = ", ".join(
        sorted(list(TUI_SCENARIO_KEYS) + list(TUI_SCENARIO_TABS) + list(TUI_SCENARIO_DRAGS))
    )
    raise SystemExit(f"error: unknown tui scenario {scenario!r}. Known: {known}")


def tui(args: argparse.Namespace) -> int:
    """Capture the terminal UI as a real screenshot.

    Real, and reproducible, in that order: the program under test is the shipped
    `future` binary talking to the demo agent, running in raw mode against a pty
    of a fixed size, and what gets rasterized is the byte stream it wrote there.
    Only the *data* is scripted (the agent answers with a fixed session), so the
    same command produces the same picture on any machine — which a live model
    cannot. See `tui_terminal.py` for what "terminal cell model" does and does
    not reproduce.
    """
    import shutil as _shutil
    import tempfile

    import tui_capture
    import tui_terminal

    tui_bin, agent_bin = _tui_binaries(args)
    output = Path(args.output)
    if not output.is_absolute():
        output = Path(args.out) / output
    work = Path(tempfile.mkdtemp(prefix="future-tui-shot-"))
    home = work / "home"
    home.mkdir()
    agent = None
    steps = _tui_steps(args.scenario, args.cols, args.rows, lang=args.lang)
    try:
        if args.addr:
            # Attaching to an agent that is already running (the "real session"
            # path): the caller's FUTURE_HOME is authoritative, so the child
            # inherits the environment instead of getting a scratch home.
            addr = args.addr
            env = None
        else:
            # One flag pins the whole frame: the chrome's language (`--lang`,
            # written into the TUI's settings below) and the scripted data's
            # (`FUTURE_DEMO_LANG`, which the demo agent reads) have to agree, or
            # the picture is English around a Chinese session list.
            agent_env = {**os.environ, "FUTURE_DEMO_LANG": args.lang}
            # …and the scenario's own scripted state, on top of it.
            agent_env.update(TUI_SCENARIO_AGENT_ENV.get(args.scenario, {}))
            agent = subprocess.Popen(
                [str(agent_bin), "--port", "0"],
                stdout=subprocess.PIPE,
                stderr=subprocess.STDOUT,
                text=True,
                bufsize=1,
                env=agent_env,
            )
            addr = _wait_for_addr(agent, 20)
            env = {"FUTURE_HOME": str(home), "HOME": str(home)}
            # Pin the interface language: it otherwise follows the machine's
            # locale, and a capture whose chrome depends on who ran it is not a
            # baseline. Same file the TUI itself writes, so the flag goes
            # through the real settings path rather than a special case.
            #
            # The rail is pinned for the same reason. A first start hides it
            # (the shipped default), while the workbench figures in
            # `docs/tui/assets/` are of a reader who has it open — so whether a
            # frame carries the rail stays the scenario's choice instead of
            # following a product default that can change under it.
            #
            # Its width is usually left to the surface (`SIDEBAR_DEFAULT`), and
            # `--sidebar-width` is for the frames where the default is the
            # problem: the rail reserves a third of its own width for the
            # workspace name, so at 36 columns an English session title has
            # about nineteen cells before the ellipsis and most of them end in
            # `…`, while the same list in Chinese fits. Pinning it keeps the
            # layout a decision of the capture instead of the reader's terminal.
            pinned = {"language": args.lang, "sidebarVisible": True}
            if args.sidebar_width is not None:
                pinned["sidebarWidth"] = args.sidebar_width
            (home / "tui").mkdir(parents=True, exist_ok=True)
            (home / "tui" / "settings.json").write_text(
                json.dumps(pinned),
                encoding="utf-8",
            )
        argv = [str(tui_bin), "tui", "--offline", "--grpc-addr", addr]
        if args.session:
            argv += ["--session", args.session]
        # The app must be sized before it draws, and it must not inherit the
        # caller's terminal: `tui_capture.run` opens its own pty and does both.
        stream = tui_capture.run(argv, args.cols, args.rows, args.seconds, env=env, steps=steps)
        text = stream.decode("utf-8", "replace")
        if not text.strip():
            raise SystemExit(
                "error: the TUI wrote nothing. The usual causes: the binary was not "
                "built with the agent's version, or the terminal size reached it after "
                "it drew (see tui_capture.run)."
            )
        if args.keep_stream:
            Path(args.keep_stream).write_bytes(stream)
        frames = text.count("\x1b[1;1H")
        if frames == 0:
            # The app wrote *something* but never a frame — an unusable binary
            # (a version skew), a refused connection, a terminal it could not
            # size. Rendering that as a picture produces a plausible-looking PNG
            # of an error message and a green exit, which is worse than nothing.
            # Measured: a stale `target/debug/future` wrote its "Incompatible
            # Future Agent" line and the capture quietly saved it as a frame.
            message = re.sub(r"\x1b\[[0-9;?]*[A-Za-z]", "", text).strip()
            shown = "\n  ".join(line for line in message.splitlines() if line.strip())[:1200]
            raise SystemExit(
                "error: the TUI wrote no frame, only:\n  "
                + (shown or "(nothing readable)")
                + "\nrebuild both sides from one tree (cargo build -p future-cli --bin future, "
                "cargo build -p future-tui --example demo_agent), and check that the "
                "agent's version matches the client's."
            )
        size = tui_terminal.render(text, output, args.cols, args.rows, work, scale=args.scale)
        print(f"{output}: {size[0]}x{size[1]} css px from {len(stream)} bytes in {frames} frame(s)")
        return 0
    finally:
        if agent is not None:
            agent.terminate()
            try:
                agent.wait(timeout=5)
            except subprocess.TimeoutExpired:
                agent.kill()
        _shutil.rmtree(work, ignore_errors=True)


def _self_test() -> int:
    """Check the pieces a screenshot depends on, without a browser or an agent."""
    problems = []

    def check(name, got, want):
        if got != want:
            problems.append(f"{name}: got {got!r}, want {want!r}")

    # The tab click resolves a word to a *cell* column. Counting characters
    # instead put the click on the tab before the target once any CJK label
    # preceded it, which is what shipped first and produced a "usage" frame
    # showing the sessions panel.
    import tui_terminal

    tabs = "  待办   任务   会话   用量   技能"
    stream = "\x1b[1;1H" + tabs
    cells, _ = tui_terminal.screen_from_stream(stream, 40, 2)
    location = _find_word_column(cells, "用量")
    check("tab row", location[0] if location else None, 0)
    # Hand-counted from the layout: two spaces, then three double-width labels
    # and three-space gaps put `用` (a wide char) at cells 23-24, while the same
    # word sits at string index 17. The difference is what the first version got
    # wrong.
    check("usage cell", location[1] if location else None, 23)
    check("chars would say 17", tabs.index("用量"), 17)

    # And the bytes a scenario step actually writes: SGR 1006 press + release at
    # the cell a reader sees, 1-based.
    step = _tui_click_tab("用量", 40, 2, 0.0)
    sent = step[1](stream)
    check("click bytes", sent, b"\x1b[<0;24;1M\x1b[<0;24;1m")
    check("no phantom find", _find_word_column(cells, "不存在"), None)
    # A word starting mid-slot is still found; a partial match is not a match.
    check("chat tab", _find_word_column(cells, "待办")[1], 2)

    # One flag pins the whole frame. `--lang en` has to reach the *keystrokes* as
    # well as the chrome and the scripted data: `streaming` submits a prompt, and
    # an English frame that asks its question in Chinese is the mismatch the
    # language pin exists to avoid. Every other scenario is a verb or a key, so
    # the bytes must stay the same in both languages.
    #
    # The second check is the one with teeth: comparing the English prompt to the
    # English table alone would still pass if `--lang` stopped reaching this
    # function, because the table is what the expected value comes from.
    en_prompt = _tui_steps("streaming", 40, 2, settle=0.0, lang="en")[0][1]
    zh_prompt = _tui_steps("streaming", 40, 2, settle=0.0)[0][1]
    check("en prompt comes from the en table", en_prompt, TUI_SCENARIO_KEYS_EN["streaming"].encode())
    check("en prompt is not the zh one", en_prompt == zh_prompt, False)
    check(
        "verbs are language-free",
        _tui_steps("help", 40, 2, settle=0.0, lang="en")[0][1],
        TUI_SCENARIO_KEYS["help"].encode(),
    )

    if problems:
        for problem in problems:
            print(f"FAIL {problem}")
        return 1
    print("capture.py self-test: ok")
    return 0


def terminal(args: argparse.Namespace) -> int:
    """Render a shell command's output inside a terminal-styled window."""
    transcript = subprocess.run(args.command, capture_output=True, text=True, cwd=ROOT)
    lines = [f"$ {' '.join(args.command)}"]
    lines += (transcript.stdout + transcript.stderr).rstrip("\n").split("\n")

    prompts = {"--headless": "err", "removed": "err"}
    rows = []
    for line in lines:
        klass = ""
        if line.startswith("$ "):
            klass = "prompt"
        else:
            for needle, css in prompts.items():
                if needle in line:
                    klass = css
                    break
        rows.append({"text": line, "cls": klass})

    template = (SCRIPTS / "terminal.html").read_text(encoding="utf-8")
    page = Path(args.out) / f".terminal-{Path(args.output).stem}.html"
    page.parent.mkdir(parents=True, exist_ok=True)
    page.write_text(template.replace("__LINES__", json.dumps(rows, ensure_ascii=False)), encoding="utf-8")

    steps = [{"wait": 500}, {"shot": str(Path(args.out) / Path(args.output).name)}]
    steps_path = page.with_suffix(".steps.json")
    steps_path.write_text(json.dumps(steps), encoding="utf-8")
    with Browser(int(args.cdp_port)) as browser:
        result = subprocess.run([
            "node", str(SCRIPTS / "cdp.mjs"),
            "--port", str(browser.port),
            "--match", "file://",
            "--w", str(args.width), "--h", str(args.height),
            "--mobile", "false", "--touch", "false",
            "--url", page.as_uri(),
            "--settle", "1200",
            "--steps", str(steps_path),
        ], capture_output=True, text=True)
    print(result.stdout.strip())
    steps_path.unlink(missing_ok=True)
    page.unlink(missing_ok=True)
    return result.returncode


# ─── Document assembly ──────────────────────────────────────────────────────


def embed_image(path: Path, width: int) -> str:
    """Inline an image. Downscales with Pillow when it is available."""
    try:
        import io

        from PIL import Image

        image = Image.open(path).convert("RGB")
        if image.width > width:
            image = image.resize((width, round(image.height * width / image.width)), Image.LANCZOS)
        if image.width < 700:
            image = image.resize((image.width * 2, image.height * 2), Image.LANCZOS)
        buffer = io.BytesIO()
        image.save(buffer, format="JPEG", quality=88, optimize=True)
        payload = base64.b64encode(buffer.getvalue()).decode()
        return f"data:image/jpeg;base64,{payload}"
    except ImportError:
        payload = base64.b64encode(path.read_bytes()).decode()
        return f"data:image/png;base64,{payload}"


def build_pdf(args: argparse.Namespace) -> int:
    content = json.loads(Path(args.content).read_text(encoding="utf-8"))
    shots = Path(args.out).resolve()
    style = (SCRIPTS / "document.css").read_text(encoding="utf-8")

    def figure(spec: dict) -> str:
        source = shots / spec["file"]
        if not source.exists():
            print(f"warning: missing screenshot {source}", file=sys.stderr)
        klass = spec.get("class", "shot")
        return (
            f'<figure class="{klass}"><img src="{embed_image(source, spec.get("width", 1500))}"'
            f' alt="{spec["caption"]}"/><figcaption>{spec["caption"]}</figcaption></figure>'
        )

    def item(index: int, spec: dict) -> str:
        figures = spec.get("figures", [])
        if len(figures) == 2:
            body_figures = '<div class="pair">' + "".join(figure(entry) for entry in figures) + "</div>"
        else:
            body_figures = "".join(figure(entry) for entry in figures)
        return (
            f'<section class="item"><h3><span class="num">{index}</span>{spec["title"]}</h3>'
            f'<div class="prose">{spec["body"]}</div>{body_figures}</section>'
        )

    parts = [f"<!doctype html>\n<html lang=\"zh-CN\">\n<head><meta charset=\"utf-8\"/>"
             f"<title>{content['title']}</title><style>{style}</style></head><body>"]
    cover = content["cover"]
    parts.append(
        f'<div class="cover"><h1>{cover["title"]}</h1><div class="sub">{cover["subtitle"]}</div>'
        f'<div class="meta">{cover["meta"]}</div></div>'
    )
    if content.get("intro"):
        parts.append(f'<div class="lead">{content["intro"]}</div>')
    if content.get("toc"):
        parts.append("<h2>目录</h2><ul class=\"toc\">"
                     + "".join(f"<li>{entry}</li>" for entry in content["toc"]) + "</ul>")

    number = 0
    for section in content["sections"]:
        parts.append(f'<h2>{section["heading"]}</h2>')
        for spec in section["items"]:
            number += 1
            parts.append(item(number, spec))
    parts.append(f'<footer>{content.get("footer", "")}</footer></body></html>')

    page = shots / ".document.html"
    page.write_text("".join(parts), encoding="utf-8")
    output = Path(args.output).expanduser()
    output.parent.mkdir(parents=True, exist_ok=True)
    result = subprocess.run([
        chrome_path(), "--headless", "--disable-gpu", "--no-pdf-header-footer",
        f"--print-to-pdf={output}", page.as_uri(),
    ], capture_output=True, text=True)
    page.unlink(missing_ok=True)
    if not output.exists():
        print(result.stdout[-1500:], result.stderr[-1500:], file=sys.stderr)
        return 1
    print(f"wrote {output} ({output.stat().st_size // 1024} KB)")
    return 0


# ─── Entry point ────────────────────────────────────────────────────────────


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--out", default=str(DEFAULT_OUT), help="output directory (default .screenshots/)")
    parser.add_argument("--cdp-port", default=CDP_DEFAULT_PORT,
                        help=f"CDP port of the headless browser to reuse or to start (default {CDP_DEFAULT_PORT})")
    sub = parser.add_subparsers(dest="command", required=True)

    for platform in ("desktop", "mobile"):
        serve = sub.add_parser(f"serve-{platform}", help=f"start the {platform} harness server")
        serve.add_argument("--port", type=int, default=None)
        serve.set_defaults(func=serve_desktop if platform == "desktop" else serve_mobile)

        capture_parser = sub.add_parser(f"capture-{platform}", help=f"capture {platform} screenshots")
        capture_parser.add_argument("scenario", nargs="*", help="scenario name(s); default all")
        capture_parser.add_argument("--baseline", default=None,
                                    help="a measure-*.json to annotate movement since then")
        capture_parser.set_defaults(func=capture, platform=platform)

        video_parser = sub.add_parser(f"video-{platform}", help=f"record {platform} scenarios as mp4")
        video_parser.add_argument("scenario", nargs="*", help="scenario name(s); default all")
        video_parser.set_defaults(func=video, platform=platform)

        variants_parser = sub.add_parser(
            f"variants-{platform}",
            help=f"render one {platform} scenario with each declared variant styling")
        variants_parser.add_argument("scenario")
        variants_parser.add_argument("--output", default=None, help="sheet filename (inside --out)")
        variants_parser.add_argument("--columns", type=int, default=0, help="sheet columns")
        variants_parser.set_defaults(func=variants, platform=platform)

        measure_parser = sub.add_parser(
            f"measure-{platform}",
            help=f"record {platform} element geometry for an offset diagram")
        measure_parser.add_argument("scenario")
        measure_parser.add_argument("--measure", default=None, help="where to write the JSON")
        measure_parser.set_defaults(func=measure, platform=platform)

        compare_parser = sub.add_parser(
            f"compare-{platform}",
            help=f"compare two {platform} captures and highlight what differs")
        compare_parser.add_argument("left")
        compare_parser.add_argument("right")
        compare_parser.add_argument("--output", required=True)
        compare_parser.add_argument("--label-left", default="A")
        compare_parser.add_argument("--label-right", default="B")
        compare_parser.add_argument("--threshold", type=int, default=24,
                                    help="per-pixel difference that counts as a change (0-255)")
        compare_parser.add_argument("--min-area", type=int, default=24,
                                    help="ignore changed regions smaller than this")
        compare_parser.add_argument("--columns", type=int, default=3)
        compare_parser.set_defaults(func=compare, platform=platform)

    upright_parser = sub.add_parser(
        "check-upright-mobile",
        help="judge a mobile capture's orientation against an unflipped copy of its own text")
    upright_parser.add_argument("scenario")
    upright_parser.set_defaults(func=check_upright, platform="mobile")

    tui_parser = sub.add_parser("tui", help="capture the terminal UI as a real screenshot")
    tui_parser.add_argument("output", nargs="?", default="tui.png",
                            help="PNG to write (relative paths land in --out; default tui.png)")
    tui_parser.add_argument("--cols", type=int, default=130, help="pty width in columns (default 130)")
    tui_parser.add_argument("--rows", type=int, default=40, help="pty height in rows (default 40)")
    tui_parser.add_argument("--scale", type=float, default=2.0, help="device pixel ratio (default 2)")
    tui_parser.add_argument("--seconds", type=float, default=6.0,
                            help="how long to let the app paint before the capture closes (default 6)")
    tui_parser.add_argument("--session", default=None, help="open a specific session id")
    tui_parser.add_argument("--addr", default=None,
                            help="attach to an already-running agent instead of starting demo_agent")
    tui_parser.add_argument("--scenario", default="chat",
                            choices=sorted(
                                list(TUI_SCENARIO_KEYS)
                                + list(TUI_SCENARIO_TABS)
                                + list(TUI_SCENARIO_DRAGS)
                            ),
                            help="what the surface is showing (default chat)")
    tui_parser.add_argument("--bin", default=None, help="path to the future binary")
    tui_parser.add_argument("--agent-bin", default=None, help="path to the demo_agent example binary")
    tui_parser.add_argument(
        "--lang",
        default="zh",
        choices=("system", "zh", "en"),
        help="interface language the capture pins, chrome and scripted data alike "
        "(default zh, the documented screenshots)",
    )
    tui_parser.add_argument(
        "--sidebar-width",
        type=int,
        default=None,
        help="columns the sidebar is pinned to (default: the surface's own default)",
    )
    tui_parser.add_argument("--release", action="store_true", help="look in target/release instead of target/debug")
    tui_parser.add_argument("--keep-stream", default=None,
                            help="also write the raw pty bytes here (for debugging a blank capture)")
    tui_parser.set_defaults(func=tui)

    terminal_parser = sub.add_parser("terminal", help="render a command's output as a terminal image", add_help=False)
    terminal_parser.add_argument("output")
    terminal_parser.add_argument("--width", type=int, default=960)
    terminal_parser.add_argument("--height", type=int, default=420)
    terminal_parser.add_argument("command", nargs=argparse.REMAINDER)
    terminal_parser.set_defaults(func=terminal)

    pdf_parser = sub.add_parser("pdf", help="assemble a document from captured screenshots")
    pdf_parser.add_argument("content")
    pdf_parser.add_argument("output")
    pdf_parser.set_defaults(func=build_pdf)

    args = parser.parse_args()
    if args.command == "terminal":
        if args.command and args.command[0:1] == ["--"]:
            args.command = args.command[1:]
        if not args.command:
            print("error: terminal needs a command after --", file=sys.stderr)
            return 2
    Path(args.out).mkdir(parents=True, exist_ok=True)
    reaped_pids, reaped_profiles = reap_orphans()
    if reaped_pids or reaped_profiles:
        print(f"reaped {len(reaped_pids)} leftover headless browser(s) and "
              f"{len(reaped_profiles)} profile dir(s) from earlier runs")
    return args.func(args)


if __name__ == "__main__":
    if sys.argv[1:] == ["--self-test"]:
        # Importable from anywhere: the TUI helpers live beside this file.
        sys.path.insert(0, str(Path(__file__).resolve().parent))
        sys.exit(_self_test())
    sys.exit(main())
