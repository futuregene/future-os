#!/usr/bin/env python3
"""Replace a FABRICATED id in a note file with the real one, and refuse if the
fabricated id or the real id is not where it should be. Guards the F.53/F.57 error
class: an id written before its todo existed, which looks exactly like a real id.
Usage: fix-note-id.py <note-file> <fake-id> <real-id>
"""
import sys

path, fake, real = sys.argv[1], sys.argv[2], sys.argv[3]
t = open(path, encoding='utf-8').read()

if fake not in t:
    print(f"ABORT: {fake} is not in {path} - nothing to replace")
    sys.exit(1)
if real in t:
    print(f"ABORT: {real} is already in {path} - check before replacing")
    sys.exit(1)

t = t.replace(fake, real)
open(path, 'w', encoding='utf-8').write(t)

n = t.count(real)
print(f"replaced {fake} -> {real} ({n} occurrence(s)); file now {len(t)} chars")
print("tail:", t[-120:].replace('\n', ' | '))
