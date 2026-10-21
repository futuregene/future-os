import { mkdirSync } from 'node:fs';
import { join } from 'node:path';
import { timedCommand } from './timed-command.mjs';
import { catalog, root } from './affected.mjs';

if (process.env.GITHUB_ACTIONS !== 'true') throw new Error('Rust acceptance runs in CI');
const packages = JSON.parse(process.env.CI_PACKAGES || '[]');
const allowed = catalog().rust.filter(pkg => pkg.directory !== 'tests/remote-interop').map(pkg => pkg.name);
if (!Array.isArray(packages) || !packages.length || packages.some(name => !allowed.includes(name))) {
  throw new Error('Invalid Rust package selection');
}
const [phase] = process.argv.slice(2);
// Create the artifact directory before fixtures append their first row.
try { mkdirSync(join(process.env.RUNNER_TEMP, 'ci-timings'), { recursive: true }); } catch (error) {
  console.log(`::warning::Fixture timing directory unavailable: ${error.message}`);
}
const hostSuite = process.env.CI_HOST_SUITE || '';
if (hostSuite && (!['remote', 'core'].includes(hostSuite) || packages.length !== 1 || packages[0] !== 'future-host')) {
  throw new Error('Host suite requires exactly future-host');
}
const testFilter = hostSuite === 'remote' ? ['--', 'remote'] : hostSuite === 'core' ? ['--', '--skip', 'remote'] : [];
const flags = packages.flatMap(name => ['-p', name]);
async function run(command, args) {
  console.log(`::group::${command} ${args.join(' ')}`);
  const result = await timedCommand(command, args, { cwd: root });
  console.log('::endgroup::');
  if (result.status !== 0) process.exit(result.status ?? 1);
}
switch (phase) {
  case 'fmt': await run('cargo', ['fmt', ...flags, '--check']); break;
  case 'build':
    await run('cargo', ['test', '--locked', '--no-run', '--timings', ...flags]);
    break;
  case 'clippy':
    await run('cargo', ['clippy', '--locked', ...flags, '--all-targets', '--', '-D', 'warnings']);
    for (const name of packages.filter(name => ['future-remote-client', 'future-remote-server'].includes(name))) {
      await run('cargo', ['clippy', '--locked', '-p', name, '--features', 'test-support', '--all-targets', '--', '-D', 'warnings']);
    }
    if (packages.includes('futureos')) {
      await run('cargo', ['clippy', '--locked', '-p', 'future-cli', '-p', 'futureos', '--all-targets', '--', '-D', 'warnings']);
    }
    break;
  case 'test':
    if (packages.includes('future-host') && process.platform === 'linux') {
      await run('python3', ['scripts/ci/host-tests.py', '--with-broker', '--suite', hostSuite || 'all']);
      const others = packages.filter(name => name !== 'future-host');
      if (others.length) await run('cargo', ['test', '--locked', '--no-fail-fast', ...others.flatMap(name => ['-p', name])]);
    } else await run('cargo', ['test', '--locked', '--no-fail-fast', ...flags, ...testFilter]);
    break;
  default: throw new Error(`Unknown Rust phase: ${phase}`);
}
