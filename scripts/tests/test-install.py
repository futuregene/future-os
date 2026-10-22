#!/usr/bin/env python3
"""Offline regression tests: python3 scripts/tests/test-install.py."""
import json
import hashlib
import os
from pathlib import Path
import subprocess
import tempfile
import unittest

INSTALLER = (Path(__file__).resolve().parents[1] / "release" / "install.sh").read_text(encoding="utf-8").rsplit('\ncase "$OS" in', 1)[0]
KEY = "linux-x86_64-portable"
URL = "https://example.invalid/releases/1.2.3/FutureOS_linux_x86_64-portable.tar.gz"
SHA = "a" * 64
# Platform detection runs before the stubbed download boundary, so pin it here:
# a Git Bash checkout on Windows otherwise reports MINGW64_NT and aborts early.
PLATFORM_STUB = '''uname() {
  case "$1" in
    -s) printf '%s\\n' "${FUTUREOS_TEST_OS:-Linux}" ;;
    -m) printf '%s\\n' "${FUTUREOS_TEST_ARCH:-x86_64}" ;;
    *)  command uname "$@" ;;
  esac
}
'''
# fetch() that serves the fixture only from the URL it is supposed to read.
SINGLE_MANIFEST_FETCH = 'fetch() { cat "$MANIFEST_FIXTURE"; }'
# fetch() that serves the fixture only for the pinned release's own manifest and
# fails on every other URL, so a pinned install must read that manifest.
VERSION_MANIFEST_URL = "https://dl.future-os.cn/releases/1.2.3/latest.json"
VERSION_MANIFEST_FETCH = f'''fetch() {{
  case "$1" in
    "{VERSION_MANIFEST_URL}") cat "$MANIFEST_FIXTURE" ;;
    *) printf 'unexpected fetch: %s\\n' "$1" >&2; return 22 ;;
  esac
}}'''
OFFLINE_FETCH = 'fetch() { return 22; }'


class InstallManifestTests(unittest.TestCase):
    def run_installer(self, manifest, command, fetch=SINGLE_MANIFEST_FETCH, env_extra=None):
        with tempfile.TemporaryDirectory() as directory:
            fixture = Path(directory) / "manifest.json"
            fixture.write_text(manifest)
            env = {k: v for k, v in os.environ.items() if not k.startswith("FUTUREOS_")}
            env["MANIFEST_FIXTURE"] = str(fixture)
            env.update(env_extra or {})
            return subprocess.run(
                ["bash"], input=PLATFORM_STUB + INSTALLER + "\n" + fetch + "\n" + command,
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

    def test_pinned_version_is_verified_against_its_own_manifest(self):
        # A pinned version is exactly the downgrade case: the installer must read
        # releases/<version>/latest.json rather than trust a constructed URL.
        result = self.run_installer(json.dumps({"version": "1.2.3", "assets": {KEY: {"url": URL, "sha256": SHA}}}), '''
FUTUREOS_VERSION=v1.2.3
resolve_latest
resolve_asset linux-x86_64-portable portable.tar.gz
printf '%s\\n' "$VERSION" "$ASSET_URL" "$ASSET_SHA"
''', fetch=VERSION_MANIFEST_FETCH)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout.splitlines(), ["1.2.3", URL, SHA])

    def test_pinned_version_without_manifest_fails_closed(self):
        result = self.run_installer("", '''
FUTUREOS_VERSION=v1.2.3
resolve_latest
resolve_asset linux-x86_64-portable portable.tar.gz
printf 'resolved %s\\n' "$ASSET_URL"
''', fetch=OFFLINE_FETCH)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("FUTUREOS_SKIP_VERIFY", result.stderr)
        self.assertNotIn("resolved", result.stdout)

    def test_pinned_version_installs_unverified_only_when_opted_in(self):
        result = self.run_installer("", '''
FUTUREOS_VERSION=v1.2.3
resolve_latest
resolve_asset linux-x86_64-portable portable.tar.gz
printf '%s\\n' "$ASSET_URL" "$ASSET_SHA"
''', fetch=OFFLINE_FETCH, env_extra={"FUTUREOS_SKIP_VERIFY": "1"})
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout.splitlines(),
                         ["https://dl.future-os.cn/releases/1.2.3/portable.tar.gz", ""])
        self.assertIn("FUTUREOS_SKIP_VERIFY", result.stderr)

    def test_latest_manifest_without_sha256_fails_closed(self):
        # A manifest entry with no SHA-256 is as good as no manifest at all.
        result = self.run_installer(json.dumps({"version": "1.2.3", "assets": {KEY: {"url": URL}}}), '''
resolve_latest
resolve_asset linux-x86_64-portable portable.tar.gz
printf 'resolved %s\\n' "$ASSET_URL"
''')
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("FUTUREOS_SKIP_VERIFY", result.stderr)
        self.assertNotIn("resolved", result.stdout)

    def test_missing_sha256_tool_fails_closed(self):
        result = self.run_installer("", f'''
mkdir -p "$TMP/no-sha-tools"
printf payload > "$TMP/asset"
# Drop the exit trap before emptying PATH, or it cannot find rm afterwards.
trap - EXIT
PATH="$TMP/no-sha-tools"
verify_sha256 "$TMP/asset" {SHA}
printf 'verified\\n'
''')
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("FUTUREOS_SKIP_VERIFY", result.stderr)
        self.assertNotIn("verified", result.stdout)

    def test_missing_sha256_tool_is_skipped_only_when_opted_in(self):
        result = self.run_installer("", f'''
mkdir -p "$TMP/no-sha-tools"
printf payload > "$TMP/asset"
trap - EXIT
PATH="$TMP/no-sha-tools"
verify_sha256 "$TMP/asset" {SHA}
printf 'continued\\n'
''', env_extra={"FUTUREOS_SKIP_VERIFY": "1"})
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("continued", result.stdout)
        self.assertIn("FUTUREOS_SKIP_VERIFY", result.stderr)

    def test_checksum_mismatch_still_aborts(self):
        result = self.run_installer("", f'''
printf payload > "$TMP/asset"
verify_sha256 "$TMP/asset" {SHA}
''')
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("checksum mismatch", result.stderr)

    def test_matching_checksum_is_accepted(self):
        expected = hashlib.sha256(b"payload").hexdigest()
        result = self.run_installer("", f'''
printf payload > "$TMP/asset"
verify_sha256 "$TMP/asset" {expected}
printf 'verified\\n'
''')
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("verified", result.stdout)
        self.assertIn("Checksum verified", result.stdout)

    def test_macos_bundle_replacement_does_not_merge_stale_files(self):
        result = self.run_installer("", r'''
mkdir -p "$TMP/source/FutureOS.app" "$TMP/Applications/FutureOS.app"
printf old > "$TMP/Applications/FutureOS.app/stale"
printf new > "$TMP/source/FutureOS.app/current"
replace_macos_app "$TMP/source/FutureOS.app" "$TMP/Applications/FutureOS.app"
test -f "$TMP/Applications/FutureOS.app/current"
test ! -e "$TMP/Applications/FutureOS.app/stale"
''')
        self.assertEqual(result.returncode, 0, result.stderr)

    def test_macos_bundle_staging_failure_preserves_previous_app(self):
        result = self.run_installer("", r'''
mkdir -p "$TMP/source/FutureOS.app" "$TMP/Applications/FutureOS.app"
printf old > "$TMP/Applications/FutureOS.app/current"
copy_macos_app() { return 1; }
if replace_macos_app "$TMP/source/FutureOS.app" "$TMP/Applications/FutureOS.app"; then
  exit 1
fi
test "$(cat "$TMP/Applications/FutureOS.app/current")" = old
''')
        self.assertEqual(result.returncode, 0, result.stderr)

    def test_macos_bundle_activation_failure_restores_previous_app(self):
        result = self.run_installer("", r'''
mkdir -p "$TMP/source/FutureOS.app" "$TMP/Applications/FutureOS.app"
printf new > "$TMP/source/FutureOS.app/current"
printf old > "$TMP/Applications/FutureOS.app/current"
move_count=0
mv() {
  move_count=$((move_count + 1))
  if [[ "$move_count" -eq 2 ]]; then
    return 1
  fi
  command mv "$@"
}
if replace_macos_app "$TMP/source/FutureOS.app" "$TMP/Applications/FutureOS.app"; then
  exit 1
fi
test "$(cat "$TMP/Applications/FutureOS.app/current")" = old
''')
        self.assertEqual(result.returncode, 0, result.stderr)


if __name__ == "__main__":
    unittest.main()
