import assert from 'node:assert/strict';
import { test } from 'node:test';
import { spawnSync } from 'node:child_process';
import { chmodSync, copyFileSync, existsSync, mkdirSync, mkdtempSync, readFileSync, readdirSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { repairRustcInfo } from '../dev/start-desktop-rust-cache.mjs';

const failure = { success: false, code: 2, stderr: 'sccache: error: Operation not permitted (os error 1)\n' };
function fixture(run) {
  const root = mkdtempSync(join(tmpdir(), 'future rust cache '));
  try { run(root); } finally { rmSync(root, { recursive: true, force: true }); }
}
function metadata(root, outputs) {
  const file = join(root, '.rustc_info.json');
  writeFileSync(file, JSON.stringify({ rustc_fingerprint: 42, outputs, successes: {} }));
  return file;
}

test('failed sccache probe is archived verbatim; compiled artifacts remain', () => fixture(root => {
  const file = metadata(root, { version: failure });
  const original = readFileSync(file, 'utf8');
  writeFileSync(join(root, 'compiled.rlib'), 'existing compiled artifact');
  const messages = repairRustcInfo([root]);
  assert.equal(existsSync(file), false);
  const backups = readdirSync(root).filter(name => name.endsWith('.bak'));
  assert.equal(backups.length, 1);
  assert.equal(readFileSync(join(root, backups[0]), 'utf8'), original);
  assert.equal(readFileSync(join(root, 'compiled.rlib'), 'utf8'), 'existing compiled artifact');
  assert.match(messages.join('\n'), /Operation not permitted/);
}));

for (const [name, value] of [
  ['healthy probe', { success: true, stderr: '' }],
  ['real compiler failure', { success: false, stderr: 'error: invalid toolchain' }],
  ['warning mentioning sccache', { success: true, stderr: 'sccache: error: old diagnostic' }],
  ['unrecognized entry', null],
]) {
  test(`${name} remains unchanged`, () => fixture(root => {
    const file = metadata(root, { version: value });
    const original = readFileSync(file, 'utf8');
    assert.deepEqual(repairRustcInfo([root]), []);
    assert.equal(readFileSync(file, 'utf8'), original);
  }));
}

test('missing directories, duplicate targets and repeated recovery are safe', () => fixture(root => {
  assert.deepEqual(repairRustcInfo([join(root, 'missing')]), []);
  metadata(root, { version: failure });
  repairRustcInfo([root, root]);
  assert.equal(readdirSync(root).length, 1);
  assert.deepEqual(repairRustcInfo([root]), []);
}));

test('malformed or unknown Cargo metadata is not rewritten', () => fixture(root => {
  const file = join(root, '.rustc_info.json');
  for (const content of ['{invalid', 'null', '{"outputs":[]}']) {
    writeFileSync(file, content);
    repairRustcInfo([root]);
    assert.equal(readFileSync(file, 'utf8'), content);
  }
}));

test('CLI and Desktop target directories are repaired independently', () => fixture(root => {
  const desktop = join(root, 'desktop');
  mkdirSync(desktop);
  metadata(root, { version: failure });
  metadata(desktop, { version: failure });
  repairRustcInfo([root, desktop]);
  assert.equal(existsSync(join(root, '.rustc_info.json')), false);
  assert.equal(existsSync(join(desktop, '.rustc_info.json')), false);
}));

test('real Cargo replays a failed probe; recovery enables the same wrapper without bypass', () => fixture(root => {
  mkdirSync(join(root, 'src'));
  writeFileSync(join(root, 'Cargo.toml'), '[package]\nname="cache-recovery-fixture"\nversion="0.1.0"\nedition="2021"\n');
  writeFileSync(join(root, 'src/lib.rs'), 'pub fn fixture() -> u64 { 42 }\n');
  const source = join(root, 'wrapper.rs');
  // A valid native executable on Windows too; never fake an .exe with text.
  writeFileSync(source, `use std::{env,fs::OpenOptions,io::Write,path::Path,process::{Command,exit}};
fn main() {
 let calls=env::var_os("CACHE_TEST_CALLS").unwrap();
 writeln!(OpenOptions::new().append(true).create(true).open(calls).unwrap(), "called").unwrap();
 if Path::new(&env::var_os("CACHE_TEST_FAILURE").unwrap()).exists() {
  eprintln!("sccache: error: Operation not permitted (os error 1)"); exit(2);
 }
 let mut args=env::args_os().skip(1);
 let compiler=args.next().unwrap();
 exit(Command::new(compiler).args(args).status().unwrap().code().unwrap_or(1));
}
`);
  const wrapper = join(root, process.platform === 'win32' ? 'sccache.exe' : 'sccache');
  const compiled = spawnSync('rustc', [source, '-o', wrapper], { encoding: 'utf8', timeout: 60000 });
  assert.equal(compiled.status, 0, compiled.stderr);
  const env = { ...process.env, RUSTC_WRAPPER: wrapper, RUSTC_WORKSPACE_WRAPPER: '',
    CARGO_TARGET_DIR: join(root, 'target'), CACHE_TEST_CALLS: join(root, 'calls'), CACHE_TEST_FAILURE: join(root, 'fail') };
  delete env.CARGO_CACHE_RUSTC_INFO;
  // A first-ever compiler failure happens before Cargo creates target/. The
  // persistent replay requires an existing target, as in a developer checkout.
  mkdirSync(env.CARGO_TARGET_DIR);
  writeFileSync(env.CACHE_TEST_FAILURE, 'temporary failure');
  const cargo = () => spawnSync('cargo', ['check', '--offline'], { cwd: root, env, encoding: 'utf8', timeout: 60000 });
  const first = cargo();
  assert.notEqual(first.status, 0, first.stderr);
  assert.match(first.stderr, /sccache: error: Operation not permitted/);
  assert.ok(existsSync(join(env.CARGO_TARGET_DIR, '.rustc_info.json')), 'failed probe must have been persisted');
  const calls = readFileSync(env.CACHE_TEST_CALLS, 'utf8');
  rmSync(env.CACHE_TEST_FAILURE);
  const replay = cargo();
  assert.notEqual(replay.status, 0);
  assert.equal(readFileSync(env.CACHE_TEST_CALLS, 'utf8'), calls, 'Cargo should replay the failure without invoking the now healthy wrapper');
  repairRustcInfo([env.CARGO_TARGET_DIR]);
  const recovered = cargo();
  assert.equal(recovered.status, 0, recovered.stderr);
  assert.notEqual(readFileSync(env.CACHE_TEST_CALLS, 'utf8'), calls, 'the same wrapper must be invoked again');
  const info = JSON.parse(readFileSync(join(env.CARGO_TARGET_DIR, '.rustc_info.json'), 'utf8'));
  assert.ok(Object.values(info.outputs).every(output => output.success));
}));

for (const exitCode of [0, 7]) {
  test(`Unix launcher keeps sccache enabled in Cargo/Tauri; child exit ${exitCode}`, { skip: process.platform === 'win32' }, () => fixture(root => {
    const scripts = join(root, 'scripts/dev');
    const bin = join(root, 'fake tools');
    mkdirSync(scripts, { recursive: true });
    mkdirSync(bin);
    mkdirSync(join(root, 'desktop/node_modules'), { recursive: true });
    mkdirSync(join(root, 'target'));
    metadata(join(root, 'target'), { version: failure });
    for (const name of ['start-desktop-macos.sh', 'start-desktop-common.sh', 'start-desktop-rust-cache.mjs']) {
      copyFileSync(new URL(`../dev/${name}`, import.meta.url), join(scripts, name));
    }
    const tools = {
      rustc: 'echo "host: fixture-triple"',
      cargo: '[[ "$RUSTC_WRAPPER" == "$CACHE_TEST_WRAPPER" ]] || exit 91\n[[ ! -f "$CARGO_TARGET_DIR/.rustc_info.json" ]] || exit 93\necho CLI_CACHE_ENABLED',
      npm: '[[ "$RUSTC_WRAPPER" == "$CACHE_TEST_WRAPPER" ]] || exit 92\necho TAURI_CACHE_ENABLED\nexit "$CACHE_TEST_EXIT"',
    };
    for (const [name, body] of Object.entries(tools)) {
      const file = join(bin, name);
      writeFileSync(file, `#!/usr/bin/env bash\nset -eu\n${body}\n`);
      chmodSync(file, 0o755);
    }
    const result = spawnSync('bash', [join(scripts, 'start-desktop-macos.sh')], {
      cwd: root,
      env: { ...process.env, PATH: `${bin}:${process.env.PATH}`, RUSTC_WRAPPER: 'sccache', CACHE_TEST_WRAPPER: 'sccache',
        CARGO_TARGET_DIR: join(root, 'target'), DRY_RUN: '0', BUILD_CLI: '1', RUN_CHECKS: '0',
        FUTURE_DESKTOP_AGENT_MODE: 'auto', DESKTOP_DEV_PORT: '5173', CACHE_TEST_EXIT: String(exitCode) },
      encoding: 'utf8', timeout: 20000,
    });
    assert.ifError(result.error);
    assert.equal(result.status, exitCode, `${result.stdout}\n${result.stderr}`);
    assert.match(result.stdout, /TAURI_CACHE_ENABLED/);
    assert.match(readFileSync(join(root, '.logs/future-cli-dev.build.log'), 'utf8'), /CLI_CACHE_ENABLED/);
    assert.match(readFileSync(join(root, '.logs/rust-cache-preflight.log'), 'utf8'), /Recovered cached sccache probe failure/);
  }));
}
