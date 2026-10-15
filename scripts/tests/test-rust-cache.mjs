import assert from 'node:assert/strict';
import { test } from 'node:test';
import { spawnSync } from 'node:child_process';
import { chmodSync, copyFileSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { checkRustCache } from '../dev/start-desktop-rust-cache.mjs';

const ok = { status: 0, stdout: 'rustc 1.97.0', stderr: '' };
const denied = { status: 2, stderr: 'sccache: error: Operation not permitted (os error 1)' };

test('healthy cache is retained without a direct compiler probe', () => {
  const calls = [];
  const result = checkRustCache({ RUSTC_WRAPPER: 'sccache' }, (...args) => { calls.push(args); return ok; });
  assert.deepEqual(calls, [['sccache', ['rustc', '-vV']]]);
  assert.deepEqual(result, { disable: [], diagnostics: [] });
});

for (const failure of [denied, { status: null, error: new Error('ETIMEDOUT') },
  { status: null, error: new Error('ENOENT') }]) {
  test(`cache failure bypasses only the failed wrapper: ${failure.stderr || failure.error.message}`, () => {
    const calls = [];
    const result = checkRustCache({ RUSTC_WRAPPER: '/cache path/sccache', RUSTC_WORKSPACE_WRAPPER: '/custom/wrapper' }, (cmd, args) => {
      calls.push([cmd, args]);
      return cmd === 'rustc' ? ok : failure;
    });
    assert.deepEqual(result.disable, ['RUSTC_WRAPPER']);
    assert.equal(calls.length, 2);
    assert.match(result.diagnostics.join('\n'), /Bypassing/);
    assert.match(result.diagnostics.join('\n'), /Operation not permitted|ETIMEDOUT|ENOENT/);
  });
}

test('broken compiler does not authorize cache bypass', () => {
  const result = checkRustCache({ RUSTC_WRAPPER: 'sccache' }, () => denied);
  assert.deepEqual(result.disable, []);
  assert.match(result.diagnostics.join('\n'), /Direct rustc also failed/);
});

test('custom wrappers are never probed or disabled', () => {
  assert.deepEqual(checkRustCache({ RUSTC_WRAPPER: 'custom-cache' }, () => assert.fail()), { disable: [], diagnostics: [] });
});

test('Cargo environment aliases and explicit compiler are honored', () => {
  const calls = [];
  const result = checkRustCache({ CARGO_BUILD_RUSTC_WORKSPACE_WRAPPER: 'sccache', RUSTC: '/toolchain/rustc' }, (cmd, args) => {
    calls.push([cmd, args]);
    return cmd === 'sccache' ? denied : ok;
  });
  assert.deepEqual(result.disable, ['RUSTC_WORKSPACE_WRAPPER']);
  assert.deepEqual(calls, [['sccache', ['/toolchain/rustc', '-vV']], ['/toolchain/rustc', ['-vV']]]);
});

test('empty or custom primary wrapper overrides Cargo environment alias', () => {
  for (const wrapper of ['', 'custom']) {
    const result = checkRustCache({ RUSTC_WRAPPER: wrapper, CARGO_BUILD_RUSTC_WRAPPER: 'sccache' }, () => assert.fail());
    assert.deepEqual(result.disable, []);
  }
});

test('both sccache wrappers can be bypassed without modifying caller environment', () => {
  const env = { RUSTC_WRAPPER: 'sccache', RUSTC_WORKSPACE_WRAPPER: 'sccache' };
  const original = { ...env };
  assert.deepEqual(checkRustCache(env, cmd => cmd === 'rustc' ? ok : denied).disable,
    ['RUSTC_WRAPPER', 'RUSTC_WORKSPACE_WRAPPER']);
  assert.deepEqual(env, original);
});

for (const exitCode of [0, 7]) {
  test(`Unix launcher propagates recovery to Cargo and Tauri; child exit ${exitCode}`, { skip: process.platform === 'win32' }, () => {
    const root = mkdtempSync(join(tmpdir(), 'future rust cache '));
    try {
      const scripts = join(root, 'scripts/dev');
      const bin = join(root, 'fake tools');
      mkdirSync(scripts, { recursive: true });
      mkdirSync(bin);
      mkdirSync(join(root, 'desktop/node_modules'), { recursive: true });
      for (const name of ['start-desktop-macos.sh', 'start-desktop-common.sh', 'start-desktop-rust-cache.mjs']) {
        copyFileSync(new URL(`../dev/${name}`, import.meta.url), join(scripts, name));
      }
      const tools = {
        sccache: 'echo "sccache: error: Operation not permitted (os error 1)" >&2\nexit 2',
        rustc: 'echo "host: fixture-triple"',
        cargo: '[[ ${RUSTC_WRAPPER+x} && -z "$RUSTC_WRAPPER" ]] || exit 91\necho CLI_CACHE_BYPASSED',
        npm: '[[ ${RUSTC_WRAPPER+x} && -z "$RUSTC_WRAPPER" ]] || exit 92\necho TAURI_CACHE_BYPASSED\nexit "$CACHE_TEST_EXIT"',
      };
      for (const [name, body] of Object.entries(tools)) {
        const file = join(bin, name);
        writeFileSync(file, `#!/usr/bin/env bash\nset -eu\n${body}\n`);
        chmodSync(file, 0o755);
      }
      const result = spawnSync('bash', [join(scripts, 'start-desktop-macos.sh')], {
        cwd: root,
        env: { ...process.env, PATH: `${bin}:${process.env.PATH}`, RUSTC: join(bin, 'rustc'),
          RUSTC_WRAPPER: join(bin, 'sccache'), RUSTC_WORKSPACE_WRAPPER: '',
          CARGO_TARGET_DIR: join(root, 'target'), DRY_RUN: '0', BUILD_CLI: '1', RUN_CHECKS: '0',
          FUTURE_DESKTOP_AGENT_MODE: 'auto', DESKTOP_DEV_PORT: '5173', CACHE_TEST_EXIT: String(exitCode) },
        encoding: 'utf8', timeout: 20000,
      });
      assert.ifError(result.error);
      assert.equal(result.status, exitCode, `${result.stdout}\n${result.stderr}`);
      assert.match(result.stdout, /TAURI_CACHE_BYPASSED/);
      assert.match(readFileSync(join(root, '.logs/future-cli-dev.build.log'), 'utf8'), /CLI_CACHE_BYPASSED/);
      assert.match(readFileSync(join(root, '.logs/rust-cache-preflight.log'), 'utf8'), /Operation not permitted/);
    } finally {
      rmSync(root, { recursive: true, force: true });
    }
  });
}
