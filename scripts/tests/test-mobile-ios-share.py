#!/usr/bin/env python3
"""Run the actual Foundation-only iOS inbox code with macOS Swift."""
from pathlib import Path
import subprocess
import tempfile

root = Path(__file__).resolve().parents[2]
module = root / "mobile/modules/future-share-intent"
with tempfile.TemporaryDirectory(prefix="future-ios-share-") as temporary:
    binary = Path(temporary) / "share-inbox-tests"
    subprocess.run([
        "swiftc", "-warnings-as-errors",
        str(module / "ios/ShareInbox.swift"),
        str(module / "ios-tests/ShareInboxTests.swift"),
        "-o", str(binary),
    ], check=True)
    subprocess.run([str(binary)], check=True)
