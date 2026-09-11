"""Exercise dev-script host selection with fake tools, stopping before SDK work."""
import os
from pathlib import Path
import subprocess
import tempfile
import unittest

SCRIPT = Path(__file__).with_name("start-mobile-android.sh").resolve()


def executable(path, content):
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text("#!/bin/bash\n" + content)
    path.chmod(0o755)


class AndroidConfigTests(unittest.TestCase):
    def test_host_architecture_selects_matching_image_without_installing(self):
        for host, arch, abi in [("Darwin", "arm64", "arm64-v8a"), ("Darwin", "x86_64", "x86_64"), ("Linux", "x86_64", "x86_64"), ("Linux", "aarch64", "arm64-v8a")]:
            with self.subTest(host=host, arch=arch), tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                sdk = root / ("Library/Android/sdk" if host == "Darwin" else "Android/Sdk")
                executable(root / "bin/uname", f'if [[ "$1" == "-s" ]]; then echo {host}; else echo {arch}; fi\n')
                executable(root / "bin/npm", "exit 0\n")
                executable(sdk / "platform-tools/adb", "exit 99\n")
                executable(sdk / "emulator/emulator", "exit 99\n")
                executable(sdk / "cmdline-tools/latest/bin/sdkmanager", 'printf "%s\\n" "$@"\nexit 23\n')
                env = {**os.environ, "HOME": str(root), "JAVA_HOME": str(root), "PATH": f'{root / "bin"}{os.pathsep}{os.environ["PATH"]}'}
                env.pop("ANDROID_HOME", None)
                env.pop("ANDROID_SDK_ROOT", None)
                result = subprocess.run(["bash", str(SCRIPT)], env=env, text=True, capture_output=True)
                self.assertEqual(result.returncode, 23, result.stdout + result.stderr)
                self.assertIn(f"system-images;android-36.1;google_apis;{abi}", result.stdout)
                self.assertIn(f"ANDROID_HOME={sdk}", result.stdout)


if __name__ == "__main__":
    unittest.main()
