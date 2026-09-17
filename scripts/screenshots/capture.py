#!/usr/bin/env python3
"""Screenshot harness for the FutureOS desktop and mobile apps.

Renders the real frontends against fixed demo data, in a real browser, so
product screenshots, feature diagrams and illustrated documents can be produced
on a machine with no display. See docs/guide/screenshots.zh-CN.md (English:
docs/guide/screenshots.md).

Commands
  serve-desktop / serve-mobile     start the harness dev server (foreground)
  capture-desktop / capture-mobile run scenarios, write PNGs to --out
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


def run_scenario(platform: str, name: str, spec: dict, out_dir: Path, cd_endpoint: str) -> int:
    config = SCENARIOS[platform]
    viewport = config["viewport"]
    steps = []
    for step in spec["steps"]:
        step = dict(step)
        if "shot" in step:
            target = out_dir / step["shot"]
            target.parent.mkdir(parents=True, exist_ok=True)
            step["shot"] = str(target)
        steps.append(step)
    steps_path = out_dir / f".steps-{platform}-{name}.json"
    steps_path.write_text(json.dumps(steps, ensure_ascii=False), encoding="utf-8")

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
    if not config.get("touch", True):
        command += ["--touch", "false"]
    print(f"── {platform}/{name}")
    result = subprocess.run(command, capture_output=True, text=True)
    for line in result.stdout.splitlines():
        print("   " + line.replace("cdp: ", ""))
    if result.returncode != 0:
        print(result.stderr[-600:], file=sys.stderr)
    steps_path.unlink(missing_ok=True)
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
            if run_scenario(args.platform, name, spec, out_dir, browser.port) != 0:
                failed.append(name)
    if assets:
        assets.shutdown()
    print(f"\n{len(names) - len(failed)}/{len(names)} captured into {out_dir}")
    return 1 if failed else 0


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
        capture_parser.set_defaults(func=capture, platform=platform)

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
