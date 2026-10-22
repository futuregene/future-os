#!/usr/bin/env python3
"""CI-only native update executor fixtures; never touch a real FutureOS home.

Runs the built helper with already-authorized root identity, as pkexec would.
Uses a disposable package on the ephemeral CI runner, and purges it afterward.
"""
import base64
import fcntl
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile

if os.environ.get("GITHUB_ACTIONS") != "true" or sys.platform != "linux":
    raise SystemExit("Privileged update acceptance runs only on Linux CI")

binary = Path(sys.argv[1]).resolve(strict=True)
package = "futureos-update-transaction-fixture"


def run(*args, check=True):
    return subprocess.run(args, check=check, capture_output=True, text=True)


def purge():
    run("sudo", "-n", "dpkg", "--purge", package, check=False)


with tempfile.TemporaryDirectory(prefix="futureos-update-acceptance-") as temporary:
    root = Path(temporary)
    try:
        for case in ("invalid-signature", "cancel", "lock-conflict", "success", "configure-failure"):
            purge()
            directory = root / case
            payload = directory / "package"
            control = payload / "DEBIAN"
            control.mkdir(parents=True)
            (control / "control").write_text(
                f"Package: {package}\nVersion: 0.0.1\nArchitecture: all\n"
                "Maintainer: FutureOS CI <ci@example.invalid>\n"
                "Description: Disposable update transaction acceptance fixture\n"
            )
            marker = payload / "opt" / package / "marker"
            marker.parent.mkdir(parents=True)
            marker.write_text("new-version")
            if case == "configure-failure":
                script = control / "postinst"
                script.write_text("#!/bin/sh\nexit 42\n")
                script.chmod(0o755)
            archive = directory / "update.deb"
            run("dpkg-deb", "--build", "--root-owner-group", str(payload), str(archive))
            run("minisign", "-S", "-W", "-s", os.environ["FUTURE_UPDATE_FIXTURE_KEY"], "-m", str(archive))
            signature_bytes = Path(str(archive) + ".minisig").read_bytes()
            if case == "invalid-signature":
                lines = signature_bytes.decode().splitlines()
                line = lines[1]
                lines[1] = line[:24] + ("B" if line[24] == "A" else "A") + line[25:]
                signature_bytes = ("\n".join(lines) + "\n").encode()
            signature = base64.b64encode(signature_bytes).decode()
            home = directory / "home"
            record = home / "app/updates/update-fixture/result.json"
            record.parent.mkdir(parents=True)
            record.write_text(json.dumps({
                "formatVersion": 1, "operationId": "update-fixture",
                "fromVersion": "old", "targetVersion": "new",
                "stage": "installing", "outcome": None,
                "diagnostic": None, "restartSafe": False,
            }))
            lease = None
            if case == "lock-conflict":
                lock = home / "agent/agent-instance.lock"
                lock.parent.mkdir(parents=True)
                lease = lock.open("w+")
                fcntl.flock(lease, fcntl.LOCK_EX | fcntl.LOCK_NB)
            digest = hashlib.sha256(archive.read_bytes()).hexdigest()
            process = subprocess.Popen(
                ["sudo", "-n", "env", f"PKEXEC_UID={os.getuid()}", str(binary),
                 "--app-update-deb-worker", str(archive), digest, signature, package,
                 "0.0.1", str(record)],
                stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                stderr=subprocess.PIPE, text=True,
            )
            try:
                ready = process.stdout.readline().strip()
                if case == "invalid-signature":
                    output, error = process.communicate(input="", timeout=60)
                    assert process.returncode != 0, "Invalid signature was accepted"
                    assert "signature" in (ready + output + error).lower()
                    assert not (Path("/opt") / package / "marker").exists()
                    print("PASS: root executor independently rejects an invalid signature")
                    continue
                if ready != "READY":
                    raise AssertionError(f"{case}: executor did not prepare: {ready}")
                output, error = process.communicate(
                    input="" if case == "cancel" else "COMMIT\n", timeout=60
                )
                if process.returncode != 0:
                    raise AssertionError(f"{case}: executor failed: {output} {error}")
                result = json.loads(record.read_text())
                installed_marker = Path("/opt") / package / "marker"
                if case == "cancel":
                    assert not installed_marker.exists(), "EOF started installation"
                elif case == "lock-conflict":
                    assert not installed_marker.exists(), "Another Agent's lease was ignored"
                    assert result["outcome"] == "failed-restored"
                    assert result["restartSafe"] is True
                elif case == "success":
                    assert installed_marker.read_text() == "new-version"
                    assert result["outcome"] == "pending-validation"
                    assert result["restartSafe"] is True
                else:
                    assert result["outcome"] == "repair-required"
                    assert result["restartSafe"] is False
                assert record.stat().st_uid == os.getuid(), "Root owns the user's update journal"
                print(f"PASS: prepared DEB update {case}")
            finally:
                if process.poll() is None:
                    process.stdin.close()
                    process.wait(timeout=60)
                if lease:
                    lease.close()
                purge()
    finally:
        purge()
