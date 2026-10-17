"""Find tests that mutate a PROCESS-GLOBAL (PATH, other env vars, cwd, signal
handlers) and restore it only on the success path.

Why this matters (found 2026-10-16 by the TAU-B unit at segment 6): a test that sets
PATH to a fixture directory and restores it as its LAST statement leaks that PATH when
any earlier assertion fails or panics - and then every later test that spawns a process
fails with NotFound. Measured in that unit's own instrumented run: ONE such test failing
produced 27 cascading failures, which turned a green suite into a red one and voided the
run's lcov. That is a destructive flake: it does not just fail itself, it fails others.

Heuristic: a function whose body contains `set_var(` (or `remove_var(`) and also an
`assert`/`panic!`/`unwrap`/`expect` BEFORE the LAST `set_var` restoration, and which has
no panic-safe construct (`catch_unwind`, a Drop impl, a guard struct) in its body.
"""
import re
import sys
import glob

GLOBALS = re.compile(r'(?:std::env::|env::)?(?:set_var|remove_var)\(\s*"([A-Za-z_]+)"')
UNSAFE_OK = re.compile(r'catch_unwind|impl Drop for|struct \w*Guard|Guard\s*\{|set_current_dir')
PANICKY = re.compile(r'\bassert(?:_eq|_ne)?!|\bpanic!|\.unwrap\(\)|\.expect\(')


def scan(path):
    src = open(path, errors='replace').read()
    lines = src.split('\n')
    hits = []
    # function boundaries, crudely: from a `fn name(` line to the line before the next
    # `fn ` at a lower-or-equal indent, ending at a line that is exactly `    }`.
    fn_starts = [(i, m.group(1)) for i, l in enumerate(lines)
                 for m in [re.search(r'\bfn\s+(\w+)', l)] if m and 'fn ' in l]
    for idx, (start, name) in enumerate(fn_starts):
        end = fn_starts[idx + 1][0] if idx + 1 < len(fn_starts) else len(lines)
        body = '\n'.join(lines[start:end])
        vars_set = set(GLOBALS.findall(body))
        if not vars_set:
            continue
        if UNSAFE_OK.search(body):
            continue
        # position of the last restore-looking call vs the first panic-capable call
        calls = [(m.start(), m.group(1)) for m in GLOBALS.finditer(body)]
        panics = [m.start() for m in PANICKY.finditer(body)]
        if not panics:
            continue
        last_set = max(c[0] for c in calls)
        first_panic = min(panics)
        if first_panic < last_set:
            hits.append((start + 1, name, sorted(vars_set), len(panics)))
    return hits


paths = sys.argv[1:] or sorted(
    glob.glob('**/*.rs', recursive=True))
total = 0
for p in paths:
    if '/target/' in p or p.startswith('target/'):
        continue
    try:
        hits = scan(p)
    except Exception as e:
        continue
    for line, name, vars_, n in hits:
        total += 1
        print(f'{p}:{line}  fn {name}  mutates {vars_}  ({n} panic-capable calls before the last restore)')
print()
print(f'total candidates: {total}')
