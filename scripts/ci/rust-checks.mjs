import { spawnSync } from 'node:child_process';
import { catalog, root } from './affected.mjs';

if (process.env.GITHUB_ACTIONS !== 'true') throw new Error('Rust acceptance runs in CI');
const packages = JSON.parse(process.env.CI_PACKAGES || '[]');
const allowed = catalog().rust.filter(pkg => pkg.directory !== 'tests/remote-interop').map(pkg => pkg.name);
if (!Array.isArray(packages) || !packages.length || packages.some(name => !allowed.includes(name))) {
  throw new Error('Invalid Rust package selection');
}
const [phase] = process.argv.slice(2);
const flags = packages.flatMap(name => ['-p', name]);
function run(command, args) {
  console.log(`::group::${command} ${args.join(' ')}`);
  const result = spawnSync(command, args, { cwd: root, stdio: 'inherit' });
  console.log('::endgroup::');
  if (result.error) throw result.error;
  if (result.status !== 0) process.exit(result.status ?? 1);
}
switch (phase) {
  case 'fmt': run('cargo', ['fmt', ...flags, '--check']); break;
  case 'build':
    if (packages.includes('futureos')) {
      run('cargo', ['build', '--locked', ...flags]);
      run('cargo', ['build', '--locked', '-p', 'futureos', '--no-default-features']);
      run('cargo', ['build', '--locked', '-p', 'future-cli', '-p', 'futureos']);
    }
    run('cargo', ['test', '--locked', '--no-run', ...flags]);
    if (packages.includes('futureos')) {
      run('cargo', ['test', '--locked', '--no-run', '-p', 'futureos', '--no-default-features', '--test', 'headless_cli']);
    }
    break;
  case 'clippy':
    run('cargo', ['clippy', '--locked', ...flags, '--all-targets', '--', '-D', 'warnings']);
    for (const name of packages.filter(name => ['future-remote-client', 'future-remote-server'].includes(name))) {
      run('cargo', ['clippy', '--locked', '-p', name, '--features', 'test-support', '--all-targets', '--', '-D', 'warnings']);
    }
    if (packages.includes('futureos')) {
      run('cargo', ['clippy', '--locked', '-p', 'futureos', '--no-default-features', '--all-targets', '--', '-D', 'warnings']);
      run('cargo', ['clippy', '--locked', '-p', 'future-cli', '-p', 'futureos', '--all-targets', '--', '-D', 'warnings']);
    }
    break;
  case 'test':
    if (packages.includes('future-host') && process.platform === 'linux') {
      run('python3', ['scripts/ci/host-tests.py', '--with-broker']);
      const others = packages.filter(name => name !== 'future-host');
      if (others.length) run('cargo', ['test', '--locked', '--no-fail-fast', ...others.flatMap(name => ['-p', name])]);
    } else run('cargo', ['test', '--locked', '--no-fail-fast', ...flags]);
    if (packages.includes('futureos')) {
      run('cargo', ['test', '--locked', '--no-fail-fast', '-p', 'futureos', '--no-default-features', '--test', 'headless_cli']);
    }
    break;
  default: throw new Error(`Unknown Rust phase: ${phase}`);
}
