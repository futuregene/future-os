#!/usr/bin/env python3
"""Probe actual Linux/Windows package payloads without installing the product.

Only run early help/version paths in isolated profiles. Real upgrade, GUI and
sandbox acceptance still require their separate native acceptance matrix.
"""

import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import tarfile
import tempfile
import zipfile


def require(condition, message):
    if not condition:
        raise RuntimeError(message)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("platform", choices=["linux", "windows"])
    parser.add_argument("--version", required=True)
    parser.add_argument("--packages", type=Path, default=Path("release-assets"))
    parser.add_argument("--output", type=Path, default=Path("target/package-smoke"))
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=True)
    summary = {"platform": args.platform, "version": args.version,
               "status": "running", "archives": [], "probes": []}

    def run(label, command, env=None, timeout=30):
        print(f"{label}: {' '.join(map(str, command))}", flush=True)
        try:
            result = subprocess.run(list(map(str, command)), env=env, stdin=subprocess.DEVNULL,
                                    capture_output=True, text=True, encoding="utf-8",
                                    errors="replace", timeout=timeout)
        except subprocess.TimeoutExpired as error:
            (args.output / f"{label}.log").write_text(str(error), encoding="utf-8")
            raise
        (args.output / f"{label}.log").write_text(
            result.stdout + result.stderr, encoding="utf-8")
        require(result.returncode == 0,
                f"{label} exited {result.returncode}: {result.stdout}\n{result.stderr}")
        return result.stdout

    def archive(pattern):
        found = list(args.packages.glob(pattern))
        require(len(found) == 1, f"Expected exactly one {pattern}, found {found}")
        path = found[0].resolve()
        summary["archives"].append({"name": path.name, "bytes": path.stat().st_size,
                                    "sha256": hashlib.sha256(path.read_bytes()).hexdigest()})
        return path

    def probe(label, binary, flags, expected, home):
        require(binary.is_file(), f"Missing packaged binary: {binary}")
        require(os.access(binary, os.X_OK), f"Packaged binary is not executable: {binary}")
        env = os.environ.copy()
        env.update(HOME=str(home), USERPROFILE=str(home), FUTURE_HOME=str(home / ".future"))
        for key in ("FUTURE_AGENT_GRPC_ADDR", "FUTURE_AGENT_SOCKET"):
            env.pop(key, None)
        output = run(label, [binary, *flags], env=env, timeout=15)
        require(expected in output, f"{label}: missing expected output {expected!r}: {output!r}")
        if flags == ["--version"]:
            require(output.strip() == expected, f"{label}: version mismatch: {output!r}")
        require(not (home / ".future").exists(), f"{label} initialized Agent state")
        summary["probes"].append({"name": label, "binary": binary.name,
                                  "mode": oct(binary.stat().st_mode & 0o777), "exit_code": 0,
                                  "state_created": False})

    try:
        with tempfile.TemporaryDirectory(prefix="futureos package probe ") as folder:
            root = Path(folder)
            home = root / "profile"
            home.mkdir()
            version = "future v" + args.version
            if args.platform == "linux":
                deb = archive("*.deb")
                portable = archive("*-portable.tar.gz")
                cli = archive("*-cli.tar.gz")
                deb_root = root / "deb"
                run("extract-deb", ["dpkg-deb", "--extract", deb, deb_root], timeout=120)
                roots = {"deb": deb_root / "usr/bin"}
                for name, path in [("portable", portable), ("cli", cli)]:
                    destination = root / name
                    with tarfile.open(path, "r:gz") as bundle:
                        bundle.extractall(destination, filter="data")
                    roots[name] = destination
                for name, directory in roots.items():
                    probe(f"{name}-cli-version", directory / "future", ["--version"], version, home)
                    probe(f"{name}-agent-help", directory / "future", ["agent", "--help"],
                          "--grpc-addr", home)
                    # Every Linux CLI distribution promises a static musl binary.
                    dynamic = run(f"{name}-cli-linkage", ["readelf", "-d", directory / "future"])
                    require("NEEDED" not in dynamic, f"{name} CLI is dynamically linked")
                    if name != "cli":
                        # Assert the stable part of the usage line, not the exact flag list:
                        # the flags grow (--no-qr arrived in #1162) and this check exists to
                        # prove the *entrypoint* works, not to freeze its options. Same shape
                        # as the headless probe below. The exact text is asserted by the
                        # crate's own tests (desktop/src-tauri/src/launch.rs).
                        probe(f"{name}-desktop-help", directory / "futureos", ["--help"],
                              "Usage: futureos [--re-pair", home)
                        probe(f"{name}-headless-help", directory / "futureos-headless", ["--help"],
                              "Usage: futureos-headless", home)
                        run(f"{name}-headless-old-glibc", ["bash", "scripts/ci/check-headless-linux.sh",
                            directory / "futureos-headless"], timeout=240)
            else:
                installer = archive("*-setup*.exe")
                portable = archive("*-portable*.zip")
                seven_zip = shutil.which("7z")
                if not seven_zip:
                    candidate = Path(os.environ.get("ProgramFiles", "C:/Program Files")) / "7-Zip/7z.exe"
                    if candidate.is_file():
                        seven_zip = str(candidate)
                require(seven_zip, "7z is required to inspect the actual NSIS installer")
                nsis_root = root / "nsis"
                run("extract-nsis", [seven_zip, "x", "-y", f"-o{nsis_root}", installer], timeout=120)
                portable_root = root / "portable"
                with zipfile.ZipFile(portable) as bundle:
                    bundle.extractall(portable_root)
                for name, directory in [("nsis", nsis_root), ("portable", portable_root)]:
                    for binary_name, flags, expected in [
                        ("future.exe", ["--version"], version),
                        ("futureos.exe", ["--help"], "Usage: futureos [--re-pair"),
                    ]:
                        matches = [p for p in directory.rglob("*")
                                   if p.is_file() and p.name.lower() == binary_name]
                        require(len(matches) == 1, f"{name}: expected one {binary_name}, found {matches}")
                        binary = matches[0]
                        probe(f"{name}-{binary_name}-{flags[0][2:]}", binary, flags, expected, home)
                        if binary_name == "future.exe":
                            probe(f"{name}-agent-help", binary, ["agent", "--help"],
                                  "--reset-windows-sandbox", home)
            summary["status"] = "passed"
    except Exception as error:
        summary["status"] = "failed"
        summary["error"] = str(error)
        raise
    finally:
        (args.output / "summary.json").write_text(json.dumps(summary, indent=2) + "\n", encoding="utf-8")


if __name__ == "__main__":
    main()
