#!/usr/bin/env python3
"""Deterministic control for staletable's HostStall guard (F.75).

The host stalls intermittently, so I cannot rely on catching a real one. Instead:
  1. simulate a stalled hashlib load and assert sha256() raises HostStall naming F.75;
  2. assert the guard does NOT fire on a healthy path (a small file hashes fine);
  3. assert main() is wired to the guard, so a stall becomes exit 3 rather than a hang.

Usage: stall-control.py
"""
import importlib.util, os, signal, sys, time

spec = importlib.util.spec_from_file_location('st', '.future/cov100/staletable.py')
st = importlib.util.module_from_spec(spec)
spec.loader.exec_module(st)

failures = []


def check(ok, what, detail=""):
    print(f"  {'ok  ' if ok else 'FAIL'} {what}" + (f"   [{detail}]" if detail else ""))
    if not ok:
        failures.append(what)


print("1. a stalled hasher load becomes HostStall naming the fault")
# Replace the loader with one that blocks - what the wedged syspolicyd does to the
# real import - and shorten the timeout so the control is fast.
st._hashlib = None
st.STALL_TIMEOUT_SECS = 2
real_import = __builtins__.__import__ if hasattr(__builtins__, '__import__') else __import__


def slow_import(name, *a, **kw):
    if name == 'hashlib':
        time.sleep(30)
    return real_import(name, *a, **kw)


import builtins
builtins.__import__ = slow_import
try:
    st.sha256('.future/cov100/pairings.json')
    check(False, "sha256 raises HostStall when the hasher cannot load", "no exception")
except st.HostStall as error:
    check("F.75" in str(error), "sha256 raises HostStall when the hasher cannot load",
          str(error)[:90])
except Exception as error:  # noqa: BLE001
    check(False, "sha256 raises HostStall when the hasher cannot load",
          f"raised {type(error).__name__} instead")
finally:
    builtins.__import__ = real_import

print("2. the guard does NOT fire on a healthy path")
st._hashlib = None
st.STALL_TIMEOUT_SECS = 25
try:
    digest = st.sha256('.future/cov100/pairings.json')
    check(isinstance(digest, str) and len(digest) == 16,
          "a normal file still hashes to 16 hex chars", str(digest))
except st.HostStall as error:
    check(False, "a normal file still hashes to 16 hex chars", str(error)[:90])

print("3. main() is wired to the guard (exit 3, not a hang)")
src = open('.future/cov100/staletable.py', encoding='utf-8').read()
check('_main_guarded' in src and 'HOST STALL (INCIDENTS F.75)' in src,
      "the entry point reports HostStall as a named failure")

print()
if failures:
    print(f"CONTROL FAILED: {failures}")
    sys.exit(1)
print("CONTROL OK: a stall is named, a healthy path is unaffected, main() is guarded")
