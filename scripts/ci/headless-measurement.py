#!/usr/bin/env python3
"""CI-only paired raw-binary build/link/store-test/size/startup observations.

No bundling, signing, product installation, daemon or external broker. Full
acceptance remains in CI's owner suites and the separate manual/release gates.
"""
import argparse
from datetime import datetime, timezone
import hashlib
import json
import os
from pathlib import Path
import platform
import re
import shutil
import statistics
import subprocess
import tempfile
import time
import tomllib


def capture(command, cwd, env=None):
    return subprocess.check_output(command, cwd=cwd, env=env, text=True).strip()


def digest(path):
    value = hashlib.sha256()
    with path.open('rb') as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b''):
            value.update(block)
    return value.hexdigest()


def main():
    if os.environ.get('GITHUB_ACTIONS') != 'true' or platform.system() != 'Linux':
        raise SystemExit('Run only in a Linux GitHub Actions job, never local unit tests')
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--baseline', type=Path, required=True)
    parser.add_argument('--candidate', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    roots = {'baseline': args.baseline.resolve(), 'candidate': args.candidate.resolve()}
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    for file in ['rust-toolchain.toml', '.cargo/config.toml']:
        if (roots['baseline'] / file).read_bytes() != (roots['candidate'] / file).read_bytes():
            raise SystemExit(f'Cannot compare different {file}')
    if os.environ.get('RUSTFLAGS') or os.environ.get('CARGO_ENCODED_RUSTFLAGS'):
        raise SystemExit('Unrecorded inherited Rust flags would override the shared config')
    target = next(line.split(': ', 1)[1] for line in capture(
        ['rustc', '-Vv'], roots['candidate']).splitlines() if line.startswith('host: '))
    config = tomllib.loads((roots['candidate'] / '.cargo/config.toml').read_text())
    target_config = config.get('target', {}).get(target, {})
    if target_config.get('linker'):
        raise SystemExit('Explicit configured linker requires a separately reviewed timer adapter')
    real_linker = shutil.which('cc')
    if not real_linker:
        raise SystemExit('Native cc linker is required')
    linker = output / 'linker-probe'
    shutil.copy2(Path(__file__).with_name('headless-linker-probe.py'), linker)
    linker.chmod(0o755)
    flags = [*target_config.get('rustflags', []), '-C', f'linker={linker}']
    environment = {**os.environ, 'RUSTC_WRAPPER': '', 'RUSTC_WORKSPACE_WRAPPER': '',
                   'CARGO_INCREMENTAL': '0', 'CARGO_BUILD_JOBS': '2',
                   'CARGO_PROFILE_TEST_DEBUG': '1', 'CARGO_ENCODED_RUSTFLAGS': '\x1f'.join(flags),
                   'HEADLESS_LINKER_REAL': real_linker, 'FUTURE_VERSION': '0.0.2-0+test',
                   'FUTURE_BUILD_CHANNEL': 'dev', 'HEADLESS_MEASURE_PHASE': 'preparation',
                   'HEADLESS_LINK_LOG': str(output / 'preparation-links.jsonl')}
    commands = [
        ('cli-empty-target', ['build', '--release', '-p', 'future-cli', '--bin', 'future']),
        ('cli-same-command-warm', ['build', '--release', '-p', 'future-cli', '--bin', 'future']),
        ('gui-and-cli-after-cli', ['build', '--release', '-p', 'future-cli', '-p', 'futureos']),
        ('gui-and-cli-same-command-warm', ['build', '--release', '-p', 'future-cli', '-p', 'futureos']),
        ('store-test-compile', ['test', '--workspace', '--lib', '--no-run', 'store::']),
        ('store-test-execute', ['test', '--workspace', '--lib', 'store::']),
    ]
    report = {'status': 'running', 'startedAt': datetime.now(timezone.utc).isoformat(),
              'runnerOS': platform.platform(), 'cpuCount': os.cpu_count(), 'target': target,
              'rustc': capture(['rustc', '-Vv'], roots['candidate']),
              'cargo': capture(['cargo', '-V'], roots['candidate']),
              'linker': real_linker, 'linkerVersion': capture([real_linker, '--version'], roots['candidate']),
              'encodedRustFlags': flags, 'runnerImage': os.environ.get('ImageVersion'),
              'toolSha256': digest(Path(__file__)), 'linkerProbeSha256': digest(linker),
              'order': list(roots), 'revisions': {},
              'cachePolicy': 'No Actions target cache; empty independent target per revision; registry prepared before timers; targets shared only between phases of one revision.',
              'limitations': [
                  'One sequential build sample, baseline first. OS page cache, load and order effects are uncontrolled.',
                  'Raw Cargo binaries, not distributable packages; no signing, install or GUI launch.',
                  'GNU native target; these observations do not establish musl or Windows/macOS performance.',
                  'Store tests are a fixed real-persistence sample, not full acceptance or all-core test performance.',
                  'Workspace members/test counts and upstream changes differ and are recorded; do not attribute every delta solely to extraction.',
                  'Link intervals may overlap other compilation. Do not subtract their sum from Cargo wall time.',
                  'Legacy headless build and mapped startup commands differ by architecture; reported separately, not as identical-command build comparison.',
              ]}
    target_root = Path(os.environ['RUNNER_TEMP']) / 'headless-paired-targets'
    if target_root.exists():
        raise SystemExit(f'Target root must start absent: {target_root}')
    target_root.mkdir()
    try:
        # Dependencies and web assets are prepared for BOTH revisions outside
        # all timed Rust commands. No Cargo target cache is restored or copied.
        for role, root in roots.items():
            role_output = output / role
            role_output.mkdir()
            revision = {'commit': capture(['git', 'rev-parse', 'HEAD'], root),
                        'cargoLockSha256': digest(root / 'Cargo.lock'), 'checks': []}
            report['revisions'][role] = revision
            for label, command in [('registry', ['cargo', 'fetch', '--locked']),
                                   ('npm', ['npm', 'ci']),
                                   ('web-assets', ['npm', '--prefix', 'desktop', 'run', 'build'])]:
                with (role_output / f'prepare-{label}.log').open('w') as log:
                    start = time.perf_counter()
                    result = subprocess.run(command, cwd=root, env=environment, stdout=log, stderr=subprocess.STDOUT)
                revision.setdefault('preparation', []).append({'name': label, 'seconds': time.perf_counter() - start,
                                                               'exitCode': result.returncode})
                if result.returncode:
                    raise RuntimeError(f'{role} {label} preparation failed')
        for role, root in roots.items():
            role_output = output / role
            target_dir = target_root / role
            if target_dir.exists():
                raise RuntimeError(f'Target must start absent: {target_dir}')
            role_env = {**environment, 'CARGO_TARGET_DIR': str(target_dir),
                        'HEADLESS_LINK_LOG': str(role_output / 'links.jsonl')}
            revision = report['revisions'][role]
            for label, arguments in commands:
                command = ['cargo', *arguments, '--target', target, '--locked', '--offline', '--timings']
                if label == 'store-test-execute':
                    command += ['--', '--test-threads=1']
                phase_env = {**role_env, 'HEADLESS_MEASURE_PHASE': label}
                with (role_output / f'{label}.log').open('w') as log:
                    start = time.perf_counter()
                    result = subprocess.run(command, cwd=root, env=phase_env, stdout=log, stderr=subprocess.STDOUT)
                entry = {'label': label, 'command': command, 'seconds': time.perf_counter() - start,
                         'exitCode': result.returncode}
                revision['checks'].append(entry)
                timings = role_output / label
                timings.mkdir()
                for html in (target_dir / 'cargo-timings').glob('*.html'):
                    shutil.copy2(html, timings / html.name)
                if result.returncode:
                    raise RuntimeError(f'{role} {label} failed')
                if label == 'store-test-execute':
                    text = (role_output / f'{label}.log').read_text()
                    summaries = re.findall(r'test result: ok\. (\d+) passed; (\d+) failed; (\d+) ignored;', text)
                    entry['testSummaries'] = [{'passed': int(a), 'failed': int(b), 'ignored': int(c)} for a, b, c in summaries]
                    if not summaries or not sum(int(a) for a, _, _ in summaries):
                        raise RuntimeError(f'{role}: zero Store tests is not an execution observation')
            revision['binaries'] = {}
            for binary_name in ['future', 'futureos']:
                binary = target_dir / target / 'release' / binary_name
                revision['binaries'][binary_name] = {'bytes': binary.stat().st_size, 'sha256': digest(binary)}
            cli = target_dir / target / 'release/future'
            dynamic = capture(['readelf', '-d', str(cli)], root)
            revision['cliNeeded'] = re.findall(r'\(NEEDED\).*?\[(.*?)\]', dynamic)
            if any(re.search(r'gtk|webkit|gdk|tauri|wry', name, re.I) for name in revision['cliNeeded']):
                raise RuntimeError(f'{role} CLI links GUI dependencies')
            revision['startup'] = []
            for label, flags in [('cli-help', ['--help']), ('cli-version', ['version', '--json'])]:
                samples = []
                for _ in range(10):
                    with tempfile.TemporaryDirectory(prefix='headless-measure-home-') as profile:
                        probe_env = {**role_env, 'HOME': profile, 'USERPROFILE': profile,
                                     'FUTURE_HOME': str(Path(profile) / '.future')}
                        for key in ['FUTURE_AGENT_GRPC_ADDR', 'FUTURE_AGENT_SOCKET']:
                            probe_env.pop(key, None)
                        start = time.perf_counter()
                        result = subprocess.run([str(cli), *flags], env=probe_env, stdin=subprocess.DEVNULL,
                                                capture_output=True, text=True, timeout=25)
                        samples.append(time.perf_counter() - start)
                        if result.returncode or Path(probe_env['FUTURE_HOME']).exists():
                            raise RuntimeError(f'{role} early startup failed or initialized state: {result.stderr}')
                revision['startup'].append({'label': label, 'args': flags, 'samplesSeconds': samples,
                                             'medianSeconds': statistics.median(samples)})
            # The full old foreground host is a separate binary, while the new
            # CLI owns it. Measure this mapped scenario explicitly, without
            # pretending the extra baseline build has a matching new command.
            if role == 'baseline':
                command = ['cargo', 'build', '--release', '--target', target, '-p', 'futureos',
                           '--no-default-features', '--features', 'headless', '--bin', 'futureos-headless',
                           '--locked', '--offline', '--timings']
                with (role_output / 'legacy-headless-build.log').open('w') as log:
                    start = time.perf_counter()
                    result = subprocess.run(command, cwd=root, env={**role_env, 'HEADLESS_MEASURE_PHASE': 'legacy-headless-build'},
                                            stdout=log, stderr=subprocess.STDOUT)
                revision['legacyHeadlessBuild'] = {'command': command, 'seconds': time.perf_counter() - start,
                                                    'exitCode': result.returncode}
                timings = role_output / 'legacy-headless-build'
                timings.mkdir()
                for html in (target_dir / 'cargo-timings').glob('*.html'):
                    shutil.copy2(html, timings / html.name)
                if result.returncode:
                    raise RuntimeError('Baseline full foreground host build failed')
                host = target_dir / target / 'release/futureos-headless'
                host_args = []
            else:
                host, host_args = cli, ['headless']
            revision['headlessBinary'] = {'bytes': host.stat().st_size, 'sha256': digest(host),
                                          'argsPrefix': host_args}
            samples = []
            for _ in range(10):
                with tempfile.TemporaryDirectory(prefix='headless-measure-start-') as profile:
                    probe_env = {**role_env, 'HOME': profile, 'USERPROFILE': profile,
                                 'FUTURE_HOME': str(Path(profile) / '.future')}
                    for key in ['FUTURE_AGENT_GRPC_ADDR', 'FUTURE_AGENT_SOCKET', 'FUTURE_DESKTOP_AGENT_MODE']:
                        probe_env.pop(key, None)
                    start = time.perf_counter()
                    result = subprocess.run([str(host), *host_args], env=probe_env, stdin=subprocess.DEVNULL,
                                            capture_output=True, text=True, timeout=25)
                    samples.append(time.perf_counter() - start)
                    if result.returncode != 1 or 'interactive terminal' not in result.stdout + result.stderr:
                        raise RuntimeError(f'{role} foreground noninteractive refusal failed')
            revision['startup'].append({'label': 'mapped-first-noninteractive-refusal', 'argsPrefix': host_args,
                                        'samplesSeconds': samples, 'medianSeconds': statistics.median(samples)})
            links = [json.loads(line) for line in (role_output / 'links.jsonl').read_text().splitlines()]
            revision['linkCalls'] = links
            revision['linkSecondsByPhase'] = {label: sum(item['seconds'] for item in links if item['phase'] == label)
                                              for label in sorted({item['phase'] for item in links})}
            # Disk is reused only after all data has been recorded. No compiled
            # artifact is copied into the other revision's empty target.
            shutil.rmtree(target_dir)
        report['status'] = 'passed'
    except Exception as error:
        report.update(status='failed', error=str(error))
        raise
    finally:
        for role, revision in report['revisions'].items():
            links = output / role / 'links.jsonl'
            if links.exists():
                revision['linkCalls'] = [json.loads(line) for line in links.read_text().splitlines()]
        report['finishedAt'] = datetime.now(timezone.utc).isoformat()
        (output / 'report.json').write_text(json.dumps(report, indent=2) + '\n')
        shutil.rmtree(target_root, ignore_errors=True)


if __name__ == '__main__':
    main()
