"""Offline regression tests for the screenshot harness's browser hygiene.

`scripts/screenshots/capture.py` runs one headless Chrome per capture and drops
it in `Browser.close()`. Two defects made that unreliable, and neither needs a
display to reproduce:

1. Only `__exit__` cleaned up, so a run killed with SIGTERM — a turn timeout or
   an agent restart — left its browser running and its `future-shot-` profile
   (130-160 MB) on disk forever. Measured on one machine: four orphans from
   three days of interrupted runs.
2. The CDP port was never checked beyond "something answers `/json/version`",
   so a leftover browser holding `--cdp-port` was silently driven — and when the
   answer came from a *visible* browser (the user's own window, or
   `future-browser`'s persistent profile) the capture would navigate and
   screenshot that window. A listener that is not a CDP endpoint at all made the
   run wait 30 s and die with `headless Chrome did not open CDP port N`.

The classification tests run everywhere; the process tests skip when no
Chrome/Chromium is installed (they start real browsers).
"""
import importlib.util
import http.server
import json
import os
from pathlib import Path
import shutil
import signal
import subprocess
import sys
import tempfile
import threading
import time
import unittest

ROOT = Path(__file__).resolve().parents[2]
SCRIPT = ROOT / "scripts" / "screenshots" / "capture.py"

spec = importlib.util.spec_from_file_location("capture", SCRIPT)
capture = importlib.util.module_from_spec(spec)
spec.loader.exec_module(capture)


class ForeignListener(http.server.BaseHTTPRequestHandler):
    """Stands in for a listener that holds a port without being a CDP endpoint
    (what the user's own Chrome does on 9222 with remote debugging refused)."""

    def do_GET(self):
        self.send_response(404)
        self.end_headers()

    def log_message(self, *args):
        pass

HEADED_UA = ("Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 "
             "(KHTML, like Gecko) Chrome/154.0.0.0 Safari/537.36")
HEADLESS_UA = HEADED_UA.replace("Chrome/154.0.0.0", "HeadlessChrome/155.0.0.0")

BROWSER_COMMAND = ('/Applications/Google Chrome.app/Contents/MacOS/Google Chrome '
                   '--headless=new --remote-debugging-port={port} '
                   '--user-data-dir={profile} about:blank')


def chrome_available() -> bool:
    try:
        capture.chrome_path()
        return True
    except SystemExit:
        return False


HAVE_CHROME = chrome_available()
needs_chrome = unittest.skipUnless(HAVE_CHROME, "no Chrome/Chromium installed")


def ps_row(pid, ppid, profile, port=9222):
    return {"pid": pid, "ppid": ppid,
            "command": BROWSER_COMMAND.format(port=port, profile=profile)}


def process_absent(pid, timeout=10.0):
    """True once `pid` no longer exists (a killed Chrome is reaped by init)."""
    deadline = time.time() + timeout
    while time.time() < deadline:
        try:
            os.kill(pid, 0)
        except ProcessLookupError:
            return True
        except PermissionError:
            return False
        time.sleep(0.1)
    return False


class ProfileIdentificationTests(unittest.TestCase):
    """Only this script's own `future-shot-` profile identifies our browser."""

    def test_our_profile_is_recognised(self):
        row = ps_row(1, 1, "/var/folders/xy/T/future-shot-abc123")
        self.assertEqual(capture.owned_profile(row["command"]),
                         Path("/var/folders/xy/T/future-shot-abc123"))

    def test_users_own_chrome_profile_is_not_ours(self):
        command = ("/Applications/Google Chrome.app/Contents/MacOS/Google Chrome "
                   "--user-data-dir=/Users/someone/Library/Application Support/Google/Chrome")
        self.assertIsNone(capture.owned_profile(command))

    def test_future_browser_persistent_profile_is_not_ours(self):
        command = ("/Applications/Google Chrome.app/Contents/MacOS/Google Chrome "
                   "--remote-debugging-port=9224 "
                   "--user-data-dir=/Users/someone/.future/agent/browser/profile-9224")
        self.assertIsNone(capture.owned_profile(command))


class OrphanSelectionTests(unittest.TestCase):
    """Reaping must hit a dead run's browser and nothing else."""

    def test_reparented_browser_with_our_profile_is_an_orphan(self):
        rows = [ps_row(4242, 1, "/tmp/future-shot-dead")]
        self.assertEqual([row["pid"] for row in capture.orphan_browsers(rows)], [4242])

    def test_live_browsers_are_never_orphans(self):
        """A concurrent capture's browser still has its capture process as parent."""
        rows = [ps_row(4242, 7777, "/tmp/future-shot-live")]
        self.assertEqual(capture.orphan_browsers(rows), [])

    def test_reparented_browser_of_someone_else_is_left_alone(self):
        command = ("/Applications/Google Chrome.app/Contents/MacOS/Google Chrome "
                   "--remote-debugging-port=9224 "
                   "--user-data-dir=/Users/someone/.future/agent/browser/profile-9224")
        rows = [{"pid": 4242, "ppid": 1, "command": command}]
        self.assertEqual(capture.orphan_browsers(rows), [])

    def test_vite_and_terminal_servers_are_not_orphans(self):
        rows = [{"pid": 4242, "ppid": 1,
                 "command": "node /repo/node_modules/.bin/vite --config vite.shot.config.ts"}]
        self.assertEqual(capture.orphan_browsers(rows), [])


class PortDecisionTests(unittest.TestCase):
    """What `Browser` will and will not drive, decided before any browser runs."""

    def setUp(self):
        self.started = []
        real_version = capture.cdp_version

        class RecordingBrowser(capture.Browser):
            def _start(inner):
                inner.started_with = inner.port
                self.started.append(inner.port)

        self.Browser = RecordingBrowser
        self.real_version = real_version
        self.addCleanup(setattr, capture, "cdp_version", real_version)

    def test_headless_endpoint_is_reused(self):
        capture.cdp_version = lambda port: {"User-Agent": HEADLESS_UA}
        browser = self.Browser(9231)
        browser.__enter__()
        self.assertEqual(browser.port, 9231)
        self.assertEqual(self.started, [])

    def test_visible_browser_is_not_driven(self):
        capture.cdp_version = lambda port: {"User-Agent": HEADED_UA}
        browser = self.Browser(9231)
        browser.__enter__()
        self.assertNotEqual(browser.port, 9231, "a visible browser must not be reused")
        self.assertEqual(len(self.started), 1)

    @needs_chrome
    def test_foreign_listener_moves_to_the_next_free_port(self):
        """A listener that is not a CDP endpoint must not stall or be used.

        This is the user's own Chrome on 9222: it holds the port and answers 404
        (Chrome does not expose remote debugging on the default profile), so the
        run must neither drive it nor die waiting for it.
        """
        server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), ForeignListener)
        threading.Thread(target=server.serve_forever, daemon=True).start()
        self.addCleanup(server.server_close)
        self.addCleanup(server.shutdown)
        taken = server.server_address[1]
        self.assertFalse(capture.port_free(taken), "the stand-in must hold the port")
        self.assertIsNone(capture.cdp_version(taken), "it is not a CDP endpoint")

        browser = capture.Browser(taken)
        try:
            browser.__enter__()
            self.assertNotEqual(browser.port, taken)
            self.assertTrue(capture.cdp_version(browser.port), "our browser must answer")
        finally:
            browser.close()


class SweepTests(unittest.TestCase):
    """Which leftover profile directories a reap is allowed to remove."""

    def setUp(self):
        self.dirs = []

    def tearDown(self):
        for directory in self.dirs:
            shutil.rmtree(directory, ignore_errors=True)

    def make_profile(self, age_secs):
        directory = Path(tempfile.mkdtemp(prefix=capture.PROFILE_PREFIX))
        self.dirs.append(directory)
        if age_secs:
            stamp = time.time() - age_secs
            os.utime(directory, (stamp, stamp))
        return directory

    def test_stale_unreferenced_profile_is_swept(self):
        stale = self.make_profile(age_secs=capture.ORPHAN_PROFILE_GRACE_SECS + 60)
        capture.reap_orphans()
        self.assertFalse(stale.exists())

    def test_fresh_unreferenced_profile_is_kept(self):
        """A capture starting up right now must keep its profile."""
        fresh = self.make_profile(age_secs=0)
        capture.reap_orphans()
        self.assertTrue(fresh.exists())

    def test_profile_of_a_living_process_is_kept(self):
        fresh = self.make_profile(age_secs=capture.ORPHAN_PROFILE_GRACE_SECS + 60)
        holder = subprocess.Popen([sys.executable, "-c", "import time; time.sleep(30)",
                                   f"--user-data-dir={fresh}"],
                                  stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        try:
            self.assertIn(fresh, {capture.owned_profile(row["command"])
                                  for row in capture.list_processes()})
            capture.reap_orphans()
            self.assertTrue(fresh.exists(), "the running process still uses it")
        finally:
            holder.kill()
            holder.wait()


@needs_chrome
class ReapTests(unittest.TestCase):
    """The end-to-end sweep: kill the dead run's browser, keep the live one."""

    def orphan_browser(self):
        """Start a browser whose parent exits immediately (so it is re-parented
        to init exactly as a killed run leaves it), then wait for `ps` to show
        it as ppid 1."""
        profile = Path(tempfile.mkdtemp(prefix=capture.PROFILE_PREFIX))
        port = capture.free_port(9260)
        launcher = subprocess.Popen([
            sys.executable, "-c",
            "import subprocess, sys\n"
            "subprocess.Popen([sys.argv[1], '--headless=new',"
            " f'--remote-debugging-port={sys.argv[2]}',"
            " f'--user-data-dir={sys.argv[3]}', '--no-first-run',"
            " '--no-default-browser-check', '--disable-gpu', 'about:blank'],"
            " stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)\n",
            capture.chrome_path(), str(port), str(profile),
        ], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        launcher.wait()

        deadline = time.time() + 20
        while time.time() < deadline:
            orphans = [row for row in capture.orphan_browsers(capture.list_processes())
                       if capture.owned_profile(row["command"]) == profile]
            if orphans:
                return profile, orphans[0]["pid"]
            time.sleep(0.2)
        self.fail("the launched browser never appeared as an orphan")

    def test_orphaned_browser_and_profile_are_reaped(self):
        profile, pid = self.orphan_browser()
        try:
            reaped_pids, _ = capture.reap_orphans()
            self.assertIn(pid, reaped_pids)
            self.assertTrue(process_absent(pid), "the orphan must be terminated")
            self.assertFalse(profile.exists(), "its profile must be removed")
        finally:
            shutil.rmtree(profile, ignore_errors=True)

    def test_a_live_browser_survives_a_reap(self):
        browser = capture.Browser(capture.free_port(9300))
        browser.__enter__()
        try:
            pid = browser.process.pid
            profile = browser.profile
            reaped_pids, _ = capture.reap_orphans()
            self.assertNotIn(pid, reaped_pids)
            self.assertIsNone(browser.process.poll(), "our browser must keep running")
            self.assertTrue(os.path.isdir(profile), "its profile must be kept")
        finally:
            browser.close()


@needs_chrome
class SignalCleanupTests(unittest.TestCase):
    """A SIGTERM (turn timeout, agent restart) must still take the browser down."""

    def test_sigterm_removes_browser_and_profile(self):
        report_dir = Path(tempfile.mkdtemp(prefix="future-shot-test-"))
        self.addCleanup(shutil.rmtree, report_dir, True)
        report = report_dir / "browser.json"
        port = capture.free_port(9350)
        child = subprocess.Popen([
            sys.executable, "-c",
            "import json, sys, time\n"
            f"sys.path.insert(0, {str(SCRIPT.parent)!r})\n"
            "import capture\n"
            f"with capture.Browser({port}) as browser:\n"
            f"    open({str(report)!r}, 'w').write(json.dumps("
            "{'pid': browser.process.pid, 'profile': browser.profile}))\n"
            "    time.sleep(60)\n",
        ], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        try:
            browser = None
            deadline = time.time() + 30
            while browser is None and time.time() < deadline:
                if report.exists():
                    browser = json.loads(report.read_text())
                elif child.poll() is not None:
                    self.fail("the child exited before it started a browser")
                time.sleep(0.2)
            self.assertIsNotNone(browser, "the child never reported its browser")

            os.kill(child.pid, signal.SIGTERM)
            self.assertEqual(child.wait(timeout=30), -signal.SIGTERM)
            self.assertTrue(process_absent(browser["pid"]), "SIGTERM must kill the browser")
            self.assertFalse(os.path.exists(browser["profile"]), "SIGTERM must drop the profile")
        finally:
            if child.poll() is None:
                child.kill()
                child.wait()


if __name__ == "__main__":
    unittest.main(verbosity=2)
