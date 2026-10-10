#!/usr/bin/env python3
"""Run Android share staging IO tests without an Android SDK or emulator.

Uses JAVA_HOME or PATH. A JDK uses javac; a Java 17+ runtime can use the pinned
Eclipse compiler. Dependencies live only in a temporary directory. This tests
real staging IO, not Android ContentResolver / activity integration.
"""
import hashlib
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import urllib.request

ROOT = Path(__file__).resolve().parents[2]
BASE = "https://repo.maven.apache.org/maven2/"
ARTIFACTS = {
    "junit": ("junit/junit/4.13.2/junit-4.13.2.jar", "8ac9e16d933b6fb43bc7f576336b8f4d7eb5ba12"),
    "hamcrest": ("org/hamcrest/hamcrest-core/1.3/hamcrest-core-1.3.jar", "42a25dc3219429f0e5d060061f71acb49bf010a0"),
    "ecj": ("org/eclipse/jdt/ecj/3.40.0/ecj-3.40.0.jar", "5c26f6a20278196f8038a284d885c3796cd7d422"),
}


def executable(name):
    home = os.environ.get("JAVA_HOME")
    if home:
        candidate = Path(home) / "bin" / (name + (".exe" if os.name == "nt" else ""))
        return str(candidate) if candidate.is_file() else None
    return shutil.which(name)


def download(name, directory):
    path, checksum = ARTIFACTS[name]
    with urllib.request.urlopen(BASE + path, timeout=60) as response:
        data = response.read()
    # Pinned Maven artifact checksums, not a checksum downloaded alongside code.
    if hashlib.sha1(data).hexdigest() != checksum:
        raise RuntimeError("Artifact checksum mismatch: " + name)
    target = directory / (name + ".jar")
    target.write_bytes(data)
    return str(target)


def main():
    java = executable("java")
    if not java:
        raise SystemExit("Java 17+ required: set JAVA_HOME or add java to PATH")
    subprocess.run([java, "-version"], check=True)
    with tempfile.TemporaryDirectory(prefix="future-share-io-") as temporary:
        directory = Path(temporary)
        classpath = os.pathsep.join(download(name, directory) for name in ("junit", "hamcrest"))
        javac = executable("javac")
        compiler = [javac] if javac else [java, "-jar", download("ecj", directory), "-proc:none"]
        module = ROOT / "mobile/modules/future-share-intent/android/src"
        package = "cn/future_os/shareintent"
        sources = [module / "main/java" / package / "ShareFileCopier.java",
                   module / "test/java" / package / "ShareFileCopierTest.java"]
        subprocess.run(compiler + ["-source", "17", "-target", "17", "-cp", classpath,
                                   "-d", str(directory)] + [str(path) for path in sources], check=True)
        subprocess.run([java, "-cp", str(directory) + os.pathsep + classpath,
                        "org.junit.runner.JUnitCore", "cn.future_os.shareintent.ShareFileCopierTest"], check=True)


if __name__ == "__main__":
    main()
