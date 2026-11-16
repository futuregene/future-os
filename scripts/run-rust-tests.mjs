#!/usr/bin/env node
// Run the workspace test targets through cargo nextest when it is installed,
// and through `cargo test` otherwise.
//
// Why this exists: libtest runs one test *binary* at a time and only parallelises
// within it, so a lane's wall time is set by its single slowest binary no matter
// how many cores the machine has. cargo nextest schedules every test from every
// binary in one pool with one process per test. Measured in this workspace on a
// 32-core Mac, `future-goal` (1301 tests, warm): nextest 8-9 s against
// `cargo test --no-fail-fast` 50 s.
//
// It is a fallback rather than a hard requirement because nextest is a developer
// tool, not a build dependency: `make test-agent` on a fresh clone must work
// before anyone installs it. CI is the opposite — there nextest is pinned and
// installed by scripts/ci/setup-rust.mjs and a missing binary fails the job
// (scripts/ci/rust-checks.mjs calls cargo nextest directly).
import { spawnSync } from 'node:child_process';

// `--no-fail-fast` is nextest's default; `--no-tests=pass` keeps libtest's
// behaviour where a package with no tests is not a failure. Both are also what
// the CI lane passes, so a local run and a CI run agree on semantics.
const NEXTEST_ARGS = ['nextest', 'run', '--no-fail-fast', '--no-tests=pass'];

const args = process.argv.slice(2);
const probe = spawnSync('cargo', ['nextest', '--version'], { stdio: 'ignore' });
const useNextest = probe.status === 0;
if (!useNextest) {
  console.log('cargo-nextest not installed: falling back to `cargo test` (slower).');
  console.log('Install it with `brew install cargo-nextest` or `cargo install cargo-nextest --locked`.');
}
// Arguments are passed through unchanged: nextest accepts the same `-p`/`--test`
// selection as cargo, and anything after `--` keeps its libtest meaning because
// nextest emulates those arguments.
const argv = useNextest ? [...NEXTEST_ARGS, ...args] : ['test', ...args];
console.log(`$ cargo ${argv.join(' ')}`);
const run = spawnSync('cargo', argv, { stdio: 'inherit' });
if (run.error) throw run.error;
process.exit(run.status ?? 1);
