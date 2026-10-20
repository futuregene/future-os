#!/usr/bin/env python3
"""Transparent Linux linker timer used by paired Headless observations."""
import json
import os
import subprocess
import sys
import time

started = time.perf_counter()
code = 127
try:
    code = subprocess.call([os.environ['HEADLESS_LINKER_REAL'], *sys.argv[1:]])
finally:
    args = sys.argv[1:]
    output = args[args.index('-o') + 1] if '-o' in args else None
    item = {'phase': os.environ['HEADLESS_MEASURE_PHASE'], 'output': output,
            'startedMonotonic': started, 'seconds': time.perf_counter() - started,
            'exitCode': code}
    descriptor = os.open(os.environ['HEADLESS_LINK_LOG'], os.O_WRONLY | os.O_CREAT | os.O_APPEND, 0o600)
    try:
        os.write(descriptor, (json.dumps(item) + '\n').encode())
    finally:
        os.close(descriptor)
sys.exit(code)
