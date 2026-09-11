#!/usr/bin/env python3
"""Exercise the real launcher with fake Agent/npm processes; no user data or GUI."""
import os
from pathlib import Path
import signal
import subprocess
import tempfile
import time
import unittest

LAUNCHER = Path(__file__).with_name('start-desktop-macos.sh')
MOCK = r'''#!/usr/bin/env python3
import os, signal, socket, subprocess, sys, time
from pathlib import Path
root = Path(os.environ['LIFECYCLE_FIXTURE'])
role = Path(sys.argv[0]).name
if role == 'rustc':
    print('host: lifecycle-test')
    sys.exit(0)
if role == 'future':
    s = socket.socket(socket.AF_UNIX)
    try:
        s.connect(os.environ['FUTURE_AGENT_SOCKET'])
    except OSError:
        sys.exit(1)
    sys.exit(0)
def record(event):
    with (root / 'events').open('a') as f:
        f.write(f'{role} {event} {os.getpid()} {os.getpgrp()}\n')
def stop(sig, frame):
    record('stop')
    sys.exit(0)
signal.signal(signal.SIGTERM, stop)
signal.signal(signal.SIGINT, stop)
record('start')
if role == 'gui' and os.environ.get('STUBBORN_GUI'):
    signal.signal(signal.SIGTERM, signal.SIG_IGN)
if role == 'future-agent':
    s = socket.socket(socket.AF_UNIX)
    s.bind(os.environ['FUTURE_AGENT_SOCKET'])
    s.listen()
    s.settimeout(.1)
    while True:
        try:
            c, _ = s.accept()
            c.close()
        except TimeoutError:
            pass
elif role == 'npm':
    child = subprocess.Popen([str(root / 'bin' / 'gui')])
    while True:
        if (root / 'rebuild').exists():
            (root / 'rebuild').unlink()
            child.kill()
            child.wait()
            child = subprocess.Popen([str(root / 'bin' / 'gui')])
            record('rebuilt')
        if (root / 'exit').exists():
            sys.exit(7)
        time.sleep(.05)
else:
    while True:
        time.sleep(.1)
'''


class LifecycleTest(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix='future-lifecycle-')
        self.root = Path(self.temp.name)
        for folder in ('scripts', 'agent', 'cli', 'desktop/node_modules',
                       'target/debug', 'bin', 'home'):
            (self.root / folder).mkdir(parents=True)
        # Redirect only the fixture script's user paths, without changing HOME
        # or starting an actual Agent under another account/data directory.
        script = LAUNCHER.read_text().replace('$HOME', str(self.root / 'home'))
        (self.root / 'scripts/start.sh').write_text(script)
        for name in ('future-agent', 'future', 'npm', 'rustc', 'gui'):
            folder = 'target/debug' if name.startswith('future') else 'bin'
            path = self.root / folder / name
            path.write_text(MOCK)
            path.chmod(0o755)
        self.env = dict(os.environ, LIFECYCLE_FIXTURE=str(self.root),
                        FUTURE_AGENT_SOCKET=str(self.root / 'agent.sock'),
                        FUTURE_AGENT_GRPC_ADDR='auto', BUILD_AGENT='0',
                        BUILD_CLI='0', CLEAN_STALE_APP_TASKS='0', REUSE_AGENT='0',
                        RUN_CHECKS='0', DRY_RUN='0',
                        PATH=str(self.root / 'bin') + os.pathsep + os.environ['PATH'])
        self.proc = None
        self.external = None
        self.output = (self.root / 'output').open('w+')

    def events(self):
        path = self.root / 'events'
        return path.read_text().splitlines() if path.exists() else []

    def until(self, check):
        end = time.monotonic() + 12
        while time.monotonic() < end:
            if check():
                return
            time.sleep(.05)
        self.output.flush()
        self.fail('Timed out:\n' + (self.root / 'output').read_text())

    def start(self):
        self.proc = subprocess.Popen(['/bin/bash', str(self.root / 'scripts/start.sh')],
                                     env=self.env, stdout=self.output,
                                     stderr=subprocess.STDOUT, start_new_session=True)
        self.until(lambda: any(e.startswith('gui start') for e in self.events()))

    def pid(self, role):
        return int(next(e.split()[2] for e in self.events()
                        if e.startswith(role + ' start')))

    def finish(self, expected):
        self.assertEqual(self.proc.wait(timeout=15), expected)
        self.assertFalse((self.root / '.logs/future-agent-test.pid').exists())
        for e in self.events():
            if ' start ' in e and not (self.external and int(e.split()[2]) == self.external.pid):
                pid = int(e.split()[2])
                def gone():
                    try:
                        os.kill(pid, 0)
                        return False
                    except ProcessLookupError:
                        return True
                self.until(gone)

    def test_rebuild_preserves_agent_and_shutdown_order(self):
        self.start()
        agent = self.pid('future-agent')
        self.assertNotEqual(os.getpgid(agent), os.getpgid(self.pid('npm')))
        self.assertNotEqual(os.getpgid(agent), os.getpgid(self.proc.pid))
        for n in range(3):
            (self.root / 'rebuild').touch()
            self.until(lambda: sum(' rebuilt ' in e for e in self.events()) == n + 1)
            os.kill(agent, 0)
            self.assertEqual(subprocess.run([str(self.root / 'target/debug/future')],
                                            env=self.env).returncode, 0)
        self.proc.send_signal(signal.SIGTERM)
        self.finish(143)
        events = self.events()
        self.assertLess(next(i for i,e in enumerate(events) if e.startswith('gui stop')),
                        next(i for i,e in enumerate(events) if e.startswith('future-agent stop')))

    def test_desktop_group_signal_does_not_reach_agent(self):
        self.start()
        os.killpg(self.pid('npm'), signal.SIGINT)
        self.finish(0)
        # Agent receives only the launcher's ordered TERM cleanup, after GUI exit.
        events = self.events()
        self.assertLess(next(i for i,e in enumerate(events) if e.startswith('gui stop')),
                        next(i for i,e in enumerate(events) if e.startswith('future-agent stop')))

    def test_stubborn_desktop_is_force_stopped(self):
        self.env['STUBBORN_GUI'] = '1'
        self.start()
        self.proc.terminate()
        self.finish(143)
        self.assertIn('Force stopping desktop', (self.root / 'output').read_text())

    def test_terminal_interrupt(self):
        self.start()
        os.killpg(self.proc.pid, signal.SIGINT)
        self.finish(130)

    def test_desktop_failure_cleans_descendants(self):
        self.start()
        (self.root / 'exit').touch()
        self.finish(7)

    def test_agent_failure_stops_desktop(self):
        self.start()
        os.kill(self.pid('future-agent'), signal.SIGKILL)
        self.finish(1)
        self.assertIn('future-agent exited unexpectedly', (self.root / 'output').read_text())

    def test_reused_agent_is_not_stopped(self):
        self.external = subprocess.Popen([str(self.root / 'target/debug/future-agent')],
                                         env=self.env, start_new_session=True)
        self.until(lambda: (self.root / 'agent.sock').exists())
        self.env['REUSE_AGENT'] = '1'
        self.start()
        self.proc.terminate()
        self.finish(143)
        self.assertIsNone(self.external.poll())

    def tearDown(self):
        if self.proc and self.proc.poll() is None:
            self.proc.terminate()
            self.proc.wait(timeout=15)
        if self.external:
            self.external.terminate()
            self.external.wait(timeout=5)
        # On assertion failure, reap only mock groups created by this fixture.
        for e in self.events():
            if ' start ' in e:
                try:
                    os.killpg(int(e.split()[3]), signal.SIGKILL)
                except ProcessLookupError:
                    pass
        self.output.close()
        self.temp.cleanup()


if __name__ == '__main__':
    unittest.main()
