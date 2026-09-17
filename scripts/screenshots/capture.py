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
  video-desktop / video-mobile     record scenarios as mp4 (needs ffmpeg)
  variants-desktop / variants-mobile  render one scenario with each variant styling
  measure-desktop / measure-mobile record element geometry (offset diagrams)
  compare-desktop / compare-mobile compare two captures, highlighting differences
  terminal OUT.png -- CMD          render a command's real output as a terminal image
  pdf CONTENT.json OUT.pdf         assemble a document from captured PNGs and captions

Options
  --out DIR        output directory (default .screenshots/)
  --port N         override the harness port
  --cdp-port N     CDP port of the browser to use or start (default 9222)

Typical use
  python3 scripts/screenshots/capture.py serve-desktop     # terminal 1
  python3 scripts/screenshots/capture.py capture-desktop   # terminal 2
"""

import argparse
import base64
import http.server
import json
import os
import shlex
import shutil
import socket
import subprocess
import sys
import tempfile
import threading
import time
import urllib.request
from functools import partial
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent.parent
SCRIPTS = ROOT / "scripts" / "screenshots"
DEFAULT_OUT = ROOT / ".screenshots"
SCENARIOS = json.loads((SCRIPTS / "scenarios.json").read_text(encoding="utf-8"))
CHROME_CANDIDATES = (
    "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
    "/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge",
    "/Applications/Chromium.app/Contents/MacOS/Chromium",
    shutil.which("google-chrome"),
    shutil.which("google-chrome-stable"),
    shutil.which("microsoft-edge"),
    shutil.which("chromium"),
    shutil.which("chromium-browser"),
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


def cdp_reachable(port: int) -> bool:
    try:
        with urllib.request.urlopen(f"http://127.0.0.1:{port}/json/version", timeout=1.5):
            return True
    except Exception:
        return False


class Browser:
    """A CDP endpoint: either one the caller already runs, or our own headless
    Chrome. Owning the browser is what makes a capture work on a machine with no
    display and no browser-following tooling."""

    def __init__(self, port: int):
        self.port = port
        self.process = None
        self.profile = None

    def __enter__(self):
        if cdp_reachable(self.port):
            print(f"using the browser already listening on CDP port {self.port}")
            return self
        self.profile = tempfile.mkdtemp(prefix="future-shot-")
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
        if not wait_for(f"http://127.0.0.1:{self.port}/json/version", 30):
            raise SystemExit(f"error: headless Chrome did not open CDP port {self.port}")
        print(f"started headless Chrome on CDP port {self.port} (pid {self.process.pid})")
        return self

    def __exit__(self, *exc):
        if self.process:
            self.process.terminate()
            try:
                self.process.wait(timeout=10)
            except subprocess.TimeoutExpired:
                self.process.kill()
        if self.profile:
            shutil.rmtree(self.profile, ignore_errors=True)
        return False


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
    fallback = {"react-native-web": "~0.21.0", "@expo/metro-runtime": "~57.0.15", "react-dom": "19.2.3"}
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


def serve_assets(port: int):
    """Serve the demo figures for the mobile harness (its mock image source)."""
    if not port_free(port):
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
    result = subprocess.run(command, capture_output=True, text=True)
    for line in result.stdout.splitlines():
        print("   " + line.replace("cdp: ", ""))
    if result.returncode != 0:
        print(result.stderr[-600:], file=sys.stderr)
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
    parser.add_argument("--cdp-port", default=9222, help="Chrome DevTools Protocol port (default 9222)")
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
    return args.func(args)


if __name__ == "__main__":
    sys.exit(main())
