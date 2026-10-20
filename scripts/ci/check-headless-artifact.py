#!/usr/bin/env python3
"""Inspect and run an actual packaged unified CLI in an isolated profile."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import re
import struct
import subprocess
import tempfile


def pe_imports(path):
    """Read normal and delayed DLL imports without requiring a developer shell."""
    data = path.read_bytes()
    if data[:2] != b'MZ':
        raise RuntimeError('Not a PE executable')
    pe = struct.unpack_from('<I', data, 0x3c)[0]
    if data[pe:pe + 4] != b'PE\0\0':
        raise RuntimeError('Invalid PE signature')
    count = struct.unpack_from('<H', data, pe + 6)[0]
    optional_size = struct.unpack_from('<H', data, pe + 20)[0]
    optional = pe + 24
    magic = struct.unpack_from('<H', data, optional)[0]
    if magic not in (0x10b, 0x20b):
        raise RuntimeError('Unsupported PE optional header')
    directories = optional + (112 if magic == 0x20b else 96)
    sections = []
    for index in range(count):
        entry = optional + optional_size + index * 40
        size, rva, raw_size, raw = struct.unpack_from('<IIII', data, entry + 8)
        sections.append((rva, max(size, raw_size), raw))

    def offset(rva):
        for start, size, raw in sections:
            if start <= rva < start + size:
                return raw + rva - start
        raise RuntimeError(f'Unmapped PE RVA {rva:#x}')

    names = set()
    for directory, width, name_index in [(1, 20, 3), (13, 32, 1)]:
        rva, size = struct.unpack_from('<II', data, directories + directory * 8)
        if not rva:
            continue
        start = offset(rva)
        for entry in range(start, start + size, width):
            values = struct.unpack_from('<' + 'I' * (width // 4), data, entry)
            if not any(values):
                break
            if directory == 13 and not values[0] & 1:
                raise RuntimeError('Unsupported legacy VA-based delayed PE import')
            name = offset(values[name_index])
            end = data.index(b'\0', name)
            names.add(data[name:end].decode('ascii'))
    return sorted(names)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('binary', type=Path)
    parser.add_argument('--output', required=True, type=Path)
    parser.add_argument('--require-static', action='store_true')
    args = parser.parse_args()
    binary = args.binary.resolve(strict=True)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    report = {'binary': str(binary), 'sha256': hashlib.sha256(binary.read_bytes()).hexdigest(),
              'bytes': binary.stat().st_size, 'platform': platform.platform(),
              'status': 'running', 'probes': []}
    try:
        system = platform.system()
        if system == 'Linux':
            output = subprocess.check_output(['readelf', '-d', binary], text=True)
            dependencies = re.findall(r'\(NEEDED\).*?\[(.*?)\]', output)
            if args.require_static and dependencies:
                raise RuntimeError(f'CLI must be static: {dependencies}')
        elif system == 'Darwin':
            output = subprocess.check_output(['otool', '-L', binary], text=True)
            dependencies = [line.strip().split(' (', 1)[0] for line in output.splitlines()[1:]]
        elif system == 'Windows':
            dependencies = pe_imports(binary)
        else:
            raise RuntimeError(f'Unsupported artifact platform: {system}')
        report['linkedDependencies'] = dependencies
        refused = [name for name in dependencies if re.search(
            r'webkit|webview|gtk|gdk|appkit|qt[56]|libtauri|libwry', name, re.I)]
        if refused:
            raise RuntimeError(f'GUI dependencies in CLI artifact: {refused}')
        with tempfile.TemporaryDirectory(prefix='future headless artifact ') as folder:
            home = Path(folder)
            state = home / '.future'
            env = os.environ.copy()
            env.update(HOME=str(home), USERPROFILE=str(home), FUTURE_HOME=str(state))
            for key in ('FUTURE_AGENT_GRPC_ADDR', 'FUTURE_AGENT_SOCKET', 'FUTURE_DESKTOP_AGENT_MODE'):
                env.pop(key, None)
            for flags, expected, code in [(['headless', '--help'], 'Usage: future headless', 0),
                                          (['headless'], 'interactive terminal', 1)]:
                result = subprocess.run([str(binary), *flags], stdin=subprocess.DEVNULL,
                                        capture_output=True, text=True, encoding='utf-8',
                                        errors='replace', env=env, timeout=25)
                output = result.stdout + result.stderr
                report['probes'].append({'args': flags, 'exitCode': result.returncode,
                                         'stdout': result.stdout, 'stderr': result.stderr})
                if result.returncode != code or expected not in output:
                    raise RuntimeError(f'Unexpected {flags} result: {result.returncode}: {output}')
                if len(flags) == 2 and state.exists():
                    raise RuntimeError('Help initialized Future state')
            if result.stdout:
                raise RuntimeError('First noninteractive startup printed authorization output')
            for relative in ('remote_pairing.json', 'app/app.db'):
                if (state / relative).exists():
                    raise RuntimeError(f'First noninteractive startup created {relative}')
            auth = state / 'agent/auth.json'
            if auth.exists() and re.search(r'"key"\s*:', auth.read_text(encoding='utf-8')):
                raise RuntimeError('First noninteractive startup persisted credentials')
        report['status'] = 'passed'
    except Exception as error:
        report.update(status='failed', error=str(error))
        raise
    finally:
        args.output.write_text(json.dumps(report, indent=2) + '\n', encoding='utf-8')


if __name__ == '__main__':
    main()
