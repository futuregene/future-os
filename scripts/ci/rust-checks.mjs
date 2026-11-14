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
// Test execution goes through cargo nextest, which runs each test in its own
// process: `--no-fail-fast` stays explicit because that is the guarantee the
// previous `cargo test --no-fail-fast` made, and `--no-tests=pass` keeps the
// libtest behaviour where a filter that matches nothing is not a failure.
//
// No executable doctest is lost by leaving `cargo test` behind: every doc code
// fence in this workspace is tagged `text`, `ignore` or `json`, and rustdoc
// compiles none of those (`cargo test --doc` reports zero tests), so nextest not
// running doctests drops nothing.
//
// `testFilter` is passed through unchanged: nextest emulates the libtest
// arguments after `--` (`remote` as a filter, `--skip remote` as the inverse).
//
// nextest adds no per-test deadline either: measured on 0.9.152, a test that
// sleeps 150 s is reported as `SLOW` at 60 s and 120 s and then passes, so a
// loaded runner cannot fail a slow-but-passing test where `cargo test` would not.
const NEXTEST = ['nextest', 'run', '--locked', '--no-tests=pass'];
// `--no-run` rejects `--no-fail-fast` (there is nothing to fail), so the flag that
// preserves the old `cargo test --no-fail-fast` guarantee is added for runs only.
const NEXTEST_TEST = [...NEXTEST, '--no-fail-fast'];
// nextest lists a test binary before running it, and `desktop/src-tauri`'s
// `gui_main_thread_drive` target is a `harness = false` driver that gives its
// tests the process's main thread (AppKit). It speaks no listing protocol, so
// nextest exits while enumerating it. Exclude only that binary here and run its
// target through `cargo test` below, unchanged.
const MAIN_THREAD_HARNESS = 'gui_main_thread_drive';
const nextestSelection = ['-E', `not binary(=${MAIN_THREAD_HARNESS})`];
async function run(command, args) {
  console.log(`::group::${command} ${args.join(' ')}`);
  const result = await timedCommand(command, args, { cwd: root });
  console.log('::endgroup::');
  if (result.status !== 0) process.exit(result.status ?? 1);
}
switch (phase) {
  case 'fmt': await run('cargo', ['fmt', ...flags, '--check']); break;
  case 'build':
    // `--timings` is forwarded to the cargo build nextest drives, so the lane
    // still uploads target/cargo-timings/cargo-timing.html.
    await run('cargo', [...NEXTEST, '--no-run', '--timings', ...nextestSelection, ...flags]);
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
      if (others.length) await run('cargo', [...NEXTEST_TEST, ...nextestSelection, ...others.flatMap(name => ['-p', name]), ...testFilter]);
    } else await run('cargo', [...NEXTEST_TEST, ...nextestSelection, ...flags, ...testFilter]);
    if (packages.includes('futureos'))
      await run('cargo', ['test', '--locked', '--no-fail-fast', '-p', 'futureos', '--test', MAIN_THREAD_HARNESS]);
    break;
  default: throw new Error(`Unknown Rust phase: ${phase}`);
}
