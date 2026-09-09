#!/usr/bin/env python3
"""Offline regression tests: python3 scripts/test-install.py."""
import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest

INSTALLER = Path(__file__).with_name("install.sh").read_text().rsplit('\ncase "$OS" in', 1)[0]
KEY = "linux-x86_64-portable"
URL = "https://example.invalid/releases/1.2.3/FutureOS_linux_x86_64-portable.tar.gz"
SHA = "a" * 64


class InstallManifestTests(unittest.TestCase):
    def run_installer(self, manifest, command):
        with tempfile.TemporaryDirectory() as directory:
            fixture = Path(directory) / "manifest.json"
            fixture.write_text(manifest)
            env = {k: v for k, v in os.environ.items() if not k.startswith("FUTUREOS_")}
            env["MANIFEST_FIXTURE"] = str(fixture)
            return subprocess.run(
                ["bash"], input=INSTALLER + '\nfetch() { cat "$MANIFEST_FIXTURE"; }\n' + command,
                env=env, text=True, capture_output=True,
            )

    def test_manifest_layouts_and_field_order(self):
        for indent in (None, 2):
            for reverse in (False, True):
                asset = {"url": URL, "sha256": SHA}
                if reverse:
                    asset = dict(reversed(list(asset.items())))
                manifest = {"version": "1.2.3", "platforms": {KEY: {"url": "wrong"}},
                            "assets": {KEY: asset}}
                with self.subTest(indent=indent, reverse=reverse):
                    result = self.run_installer(json.dumps(manifest, indent=indent), f'''
resolve_latest
resolve_asset "{KEY}" unused
printf '%s\\n' "$VERSION" "$ASSET_URL" "$ASSET_SHA"
''')
                    self.assertEqual(result.returncode, 0, result.stderr)
                    self.assertEqual(result.stdout.splitlines(), ["1.2.3", URL, SHA])

    def test_inline_assets_with_pretty_printed_version(self):
        # The old version reader succeeds, but its assets-line `next` skips the URL.
        assets = json.dumps({KEY: {"url": URL, "sha256": SHA}})
        manifest = '{\n  "version": "1.2.3",\n  "assets" : ' + assets + '\n}'
        result = self.run_installer(manifest, f'''
resolve_latest
resolve_asset "{KEY}" unused
printf '%s %s' "$ASSET_URL" "$ASSET_SHA"
''')
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout, f"{URL} {SHA}")

    def test_assets_scope_and_missing_key(self):
        manifest = {"version": "1.2.3", "assets": {}, "platforms": {KEY: {"url": URL, "sha256": SHA}}}
        result = self.run_installer(json.dumps(manifest, indent=2), f'''
resolve_latest
resolve_asset "{KEY}" unused
''')
        self.assertNotEqual(result.returncode, 0)
        self.assertIn(f"no release asset '{KEY}'", result.stderr)

    def test_platform_keys_and_legacy_deb_fallback(self):
        for key in ("darwin-aarch64", "darwin-x86_64", "linux-x86_64-deb", "linux-aarch64-deb",
                    "linux-x86_64-portable", "linux-aarch64-portable", "linux-x86_64"):
            with self.subTest(key=key):
                manifest = {"version": "1.2.3", "assets": {key: {"url": URL, "sha256": SHA}}}
                requested = "linux-x86_64-deb" if key == "linux-x86_64" else key
                result = self.run_installer(json.dumps(manifest), f'''
resolve_latest
resolve_asset "{requested}" unused "linux-x86_64"
printf '%s %s' "$ASSET_URL" "$ASSET_SHA"
''')
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertEqual(result.stdout, f"{URL} {SHA}")

    def test_escaped_slashes_and_punctuation_in_other_fields(self):
        manifest = {"version": "1.2.3", "notes": 'ignore {"assets": {}} and , : \\ text',
                    "assets": {KEY: {"sha256": SHA, "url": URL}}}
        result = self.run_installer(json.dumps(manifest).replace("/", r"\/"), f'''
resolve_latest
resolve_asset "{KEY}" unused
printf '%s %s' "$ASSET_URL" "$ASSET_SHA"
''')
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout, f"{URL} {SHA}")

    def test_pinned_version_does_not_fetch_manifest(self):
        result = self.run_installer("", '''
FUTUREOS_VERSION=v1.2.3
fetch() { return 1; }
resolve_latest
resolve_asset linux-x86_64-portable portable.tar.gz
printf '%s\\n' "$VERSION" "$ASSET_URL" "$ASSET_SHA"
''')
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout.splitlines(), ["1.2.3", "https://dl.future-os.cn/releases/1.2.3/portable.tar.gz", ""])


if __name__ == "__main__":
    unittest.main()
