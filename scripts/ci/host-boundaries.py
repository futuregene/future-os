#!/usr/bin/env python3
"""Check the actual combined GUI/CLI resolution, without compiling or testing."""
import argparse
import json
from pathlib import Path
import subprocess

ROOT = Path(__file__).resolve().parents[2]
parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--target', help='Target triple; defaults to the rustc host')
args = parser.parse_args()
host_triple = next(line.split(': ', 1)[1] for line in subprocess.check_output(
    ['rustc', '-vV'], cwd=ROOT, text=True, encoding='utf-8').splitlines() if line.startswith('host: '))
target = args.target or host_triple
metadata = json.loads(subprocess.check_output([
    'cargo', 'metadata', '--offline', '--locked', '--format-version', '1',
    '--filter-platform', target,
], cwd=ROOT, text=True, encoding='utf-8'))
packages = {package['id']: package for package in metadata['packages']}
nodes = {node['id']: node for node in metadata['resolve']['nodes']}

def local_package(name):
    return next(package for package in packages.values()
                if package['name'] == name and package['source'] is None)

host = local_package('future-host')
cli = local_package('future-cli')
gui = local_package('futureos')
if 'gui' not in nodes[gui['id']]['features']:
    raise SystemExit('Desktop GUI must participate in the combined feature resolution')
if 'gui' in host['features']:
    raise SystemExit('future-host must not declare a GUI feature')
if not any('lib' in target['kind'] for target in host['targets']):
    raise SystemExit('future-host must provide an ordinary library')
if any('bin' in target['kind'] for target in host['targets']):
    raise SystemExit('future-host must not provide a product executable')
if any(kind in {'staticlib', 'cdylib'} for target in host['targets'] for kind in target['crate_types']):
    raise SystemExit('future-host must not produce FFI library artifacts')

def reachable(root):
    pending, found = [root], set()
    while pending:
        package_id = pending.pop()
        if package_id in found:
            continue
        found.add(package_id)
        pending.extend(dep['pkg'] for dep in nodes[package_id]['deps'])
    return found

def forbidden(name):
    return (name in {'futureos', 'tauri', 'wry', 'tao', 'muda', 'gtk', 'gtk-sys',
                     'webkit2gtk', 'webkit2gtk-sys', 'gdk', 'gdk-sys'}
            or name.startswith('tauri-'))

declared_refused = sorted({dep['name'] for dep in host['dependencies']
                           if forbidden(dep['name']) or dep['name'] == 'future-cli'})
if declared_refused:
    raise SystemExit('future-host declares forbidden dependencies: ' + ', '.join(declared_refused))

results = {}
for root in [host, cli]:
    closure = reachable(root['id'])
    refused = sorted({packages[item]['name'] for item in closure
                      if forbidden(packages[item]['name'])
                      or (root is host and packages[item]['name'] == 'future-cli')})
    if refused:
        raise SystemExit(root['name'] + ' has forbidden dependencies: ' + ', '.join(refused))
    results[root['name']] = {'packages': len(closure), 'forbidden': refused}
print(json.dumps({'target': target, 'desktopGuiFeatures': nodes[gui['id']]['features'],
                  'checked': results, 'executedTests': 0}, sort_keys=True))
