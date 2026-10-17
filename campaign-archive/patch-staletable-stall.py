#!/usr/bin/env python3
"""Patch staletable.py so a host stall becomes a NAMED error instead of a hang (F.75).

Measured 2026-10-17: `import hashlib` hung 8 of 8 times on this host (the import loads
`libcrypto`, and macOS's wedged `syspolicyd` stalls that load), so `staletable.py`
- the tool that produces and verifies the freeze stamp - hung too, meaning the endgame
could neither verify its stamp nor report why.

This patch: imports hashlib LAZILY inside sha256(), wraps the import and the read in a
SIGALRM guard, and lets a HostStall propagate to a clear message + exit 3. A hang becomes
a diagnosis, which is the campaign's rule for anything that would otherwise be silent.

Usage: patch-staletable-stall.py [--check]
"""
import re, sys

PATH = '.future/cov100/staletable.py'
CHECK = '--check' in sys.argv

src = open(PATH, encoding='utf-8').read()

if 'class HostStall' in src:
    print("already patched")
    sys.exit(0)

old = '''def sha256(path):
    try:
        with open(path, "rb") as fh:
            return hashlib.sha256(fh.read()).hexdigest()[:16]
    except OSError:
        return None
'''

new = '''class HostStall(Exception):
    """The HOST stalled on a trivial operation (INCIDENTS F.75).

    Measured 2026-10-17: with `/usr/libexec/syspolicyd` wedged at ~1000 % CPU, `import
    hashlib` hung 8 of 8 times (the import loads `libcrypto` and the wedged policy daemon
    stalls that load), and builds stalled outright. This class turns such a stall into a
    named error so the stamp can say "the host could not be checked" instead of hanging
    forever - the campaign's rule for anything that would otherwise fail silently.
    """


# Filled on first successful import; see sha256().
_hashlib = None
STALL_TIMEOUT_SECS = int(os.environ.get("FUTURE_HASH_TIMEOUT", "25"))


def _require_hashlib():
    """Import hashlib under an alarm, so a stalled dylib load becomes HostStall."""
    global _hashlib
    if _hashlib is not None:
        return _hashlib
    import signal as _signal

    def _timeout(signum, frame):  # pragma: no cover - host-fault path
        raise HostStall(
            f"importing hashlib stalled for {STALL_TIMEOUT_SECS}s (it loads libcrypto; a "
            f"wedged syspolicyd makes that load hang - INCIDENTS F.75). The host cannot "
            f"be hashed/verified right now; a machine restart is the usual remedy.")

    previous = _signal.signal(_signal.SIGALRM, _timeout)
    _signal.alarm(STALL_TIMEOUT_SECS)
    try:
        import hashlib as _h
        _hashlib = _h
    finally:
        _signal.alarm(0)
        _signal.signal(_signal.SIGALRM, previous)
    return _hashlib


def sha256(path, timeout=STALL_TIMEOUT_SECS):
    """16-hex digest of a file, or None if it cannot be read.

    Raises HostStall (rather than hanging) if the hasher cannot be loaded in `timeout`
    seconds, and likewise if reading the file itself stalls - both observed on a wedged
    host.
    """
    import signal as _signal
    hasher = _require_hashlib()

    def _timeout(signum, frame):  # pragma: no cover - host-fault path
        raise HostStall(
            f"hashing {path} stalled for {timeout}s - the host is not doing I/O or "
            f"library loads in reasonable time (INCIDENTS F.75).")

    previous = _signal.signal(_signal.SIGALRM, _timeout)
    _signal.alarm(timeout)
    try:
        try:
            with open(path, "rb") as fh:
                return hasher.sha256(fh.read()).hexdigest()[:16]
        except OSError:
            return None
    finally:
        _signal.alarm(0)
        _signal.signal(_signal.SIGALRM, previous)
'''

if old not in src:
    print("ABORT: could not find the sha256() implementation to replace")
    sys.exit(1)

src = src.replace(old, new, 1)

# Report the stall as a named failure at the top level, exit 3 (nothing was measured).
anchor = "def main():"
if anchor not in src:
    print("ABORT: no main() to wrap")
    sys.exit(1)

src = src.replace(
    anchor,
    '''def _main_guarded():
    """Run main(), turning a host stall into a clear message and exit 3."""
    try:
        return main()
    except HostStall as error:
        print(f"HOST STALL (INCIDENTS F.75): {error}", file=sys.stderr)
        print("  This is a MACHINE fault, not a repo one: the stamp cannot be produced or "
              "verified on this host state.", file=sys.stderr)
        return 3


def main():''',
    1)

# and call the guard at the bottom
src = re.sub(r"\n\s*sys\.exit\(main\(\)\)\s*\n", "\n    sys.exit(_main_guarded())\n", src)
src = re.sub(r"\nif __name__ == \"__main__\":\s*\n\s*main\(\)\s*\n",
             "\nif __name__ == \"__main__\":\n    sys.exit(_main_guarded())\n", src)

open(PATH, 'w', encoding='utf-8').write(src)
print(f"patched {PATH}: HostStall + lazy hashlib + guarded main")
