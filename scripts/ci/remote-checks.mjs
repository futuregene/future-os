// CI execution only. Compile/test times are separate observations, not a claim
// of cold-build performance or a substitute for matched baseline measurements.
import { appendFileSync } from 'node:fs';
import { spawnSync } from 'node:child_process';
import { performance } from 'node:perf_hooks';

if (!process.env.GITHUB_ACTIONS) throw new Error('Remote acceptance runs in CI');
const rust = JSON.parse(process.env.REMOTE_RUST || '[]');
const ts = JSON.parse(process.env.REMOTE_TS || '[]');
const interop = process.env.REMOTE_INTEROP === 'true';
const allowedRust = ['crypto', 'protocol', 'client', 'server'].map(role => `future-remote-${role}`);
const allowedTS = ['crypto', 'protocol', 'client'].map(role => `@future-os/remote-${role}`);
if (!Array.isArray(rust) || !Array.isArray(ts) || rust.some(name => !allowedRust.includes(name)) || ts.some(name => !allowedTS.includes(name))) throw new Error('invalid package selection');
function run(label, command, args) {
  console.log(`::group::${label}`);
  const start = performance.now();
  const result = spawnSync(command, args, { stdio: 'inherit', shell: false });
  const seconds = ((performance.now() - start) / 1000).toFixed(2);
  console.log('::endgroup::');
  if (process.env.GITHUB_STEP_SUMMARY) appendFileSync(process.env.GITHUB_STEP_SUMMARY, `| ${label} | ${seconds} | ${result.status === 0 ? 'PASS' : 'FAIL'} |\n`);
  if (result.error) throw result.error;
  if (result.status !== 0) process.exit(result.status || 1);
}
if (process.env.GITHUB_STEP_SUMMARY) appendFileSync(process.env.GITHUB_STEP_SUMMARY, '\nRemote package checks (one Linux runner, shared target/install). Test invocation includes Cargo freshness checks; times are not a matched performance comparison.\n\n| Check | Seconds | Result |\n|---|---:|---|\n');
run('affected graph regressions', 'node', ['--test', 'scripts/tests/remote-ci.test.mjs']);
const boundaryFlags = [...(rust.length || interop ? ['--rust'] : []), ...(ts.length || interop ? ['--typescript'] : [])];
if (boundaryFlags.length) run('Remote package boundaries', 'node', ['scripts/ci/remote-boundaries.mjs', ...boundaryFlags]);
if (rust.includes('future-remote-protocol') || ts.includes('@future-os/remote-protocol') || interop) run('protocol generated files', 'python3', ['packages/remote-protocol/scripts/generate.py', '--check']);
for (const name of rust) {
  run(`${name} fmt`, 'cargo', ['fmt', '-p', name, '--check']);
  run(`${name} clippy`, 'cargo', ['clippy', '--locked', '-p', name, '--all-targets', '--', '-D', 'warnings']);
  if (['future-remote-client', 'future-remote-server'].includes(name)) run(`${name} test-support clippy`, 'cargo', ['clippy', '--locked', '-p', name, '--features', 'test-support', '--all-targets', '--', '-D', 'warnings']);
  run(`${name} test compilation`, 'cargo', ['test', '--locked', '-p', name, '--no-run']);
  run(`${name} tests`, 'cargo', ['test', '--locked', '-p', name, '--', '--nocapture']);
}
for (const name of ts) {
  run(`${name} typecheck`, 'npm', ['run', 'typecheck', '--workspace', name]);
  run(`${name} lint`, 'npm', ['run', 'lint', '--workspace', name]);
  run(`${name} tests`, 'npm', ['test', '--workspace', name]);
}
if (interop) {
  run('broker version', 'nats-server', ['--version']);
  run('interop fmt', 'cargo', ['fmt', '-p', 'future-remote-interop', '--check']);
  run('interop clippy', 'cargo', ['clippy', '--locked', '-p', 'future-remote-interop', '--features', 'network-tests', '--all-targets', '--', '-D', 'warnings']);
  run('Rust interop compilation', 'cargo', ['test', '--locked', '-p', 'future-remote-interop', '--features', 'network-tests', '--lib', '--test', 'rust_client_server', '--no-run']);
  run('Rust interop tests', 'cargo', ['test', '--locked', '-p', 'future-remote-interop', '--features', 'network-tests', '--lib', '--test', 'rust_client_server', '--', '--nocapture']);
  run('TS interop host compilation', 'cargo', ['build', '--locked', '-p', 'future-remote-interop', '--features', 'network-tests', '--bin', 'remote-interop-host']);
  for (const script of ['typecheck', 'lint', 'test']) run(`TS interop ${script}`, 'npm', ['run', script, '--workspace', '@future-os/remote-interop']);
}
