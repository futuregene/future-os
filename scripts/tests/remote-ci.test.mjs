import assert from 'node:assert/strict';
import test from 'node:test';
import { remoteAffected } from '../ci/remote-affected.mjs';
import { checkRemoteSpecifier } from '../ci/remote-boundaries.mjs';

test('Rust client source selects its Rust consumers, without Mobile or unrelated crates', () => {
  assert.deepEqual(remoteAffected(['packages/remote-client/src/runtime.rs']), {
    rust: ['future-remote-client'], typescript: [], interop: true, desktopRust: true, mobile: false,
  });
});
test('TS protocol propagates through TS client to Mobile, without a GUI Rust build', () => {
  assert.deepEqual(remoteAffected(['packages/remote-protocol/src/codec.ts']), {
    rust: [], typescript: ['@future-os/remote-protocol', '@future-os/remote-client'], interop: true, desktopRust: false, mobile: true,
  });
});
test('Rust crypto reaches protocol, both roles, interop and Desktop', () => {
  assert.deepEqual(remoteAffected(['packages/remote-crypto/src/lib.rs']).rust,
    ['future-remote-crypto', 'future-remote-protocol', 'future-remote-client', 'future-remote-server']);
});
test('shared credential scenarios select both package drivers without host rebuilds', () => {
  assert.deepEqual(remoteAffected(['packages/remote-client/fixtures/credentials.json']), {
    rust: ['future-remote-client'], typescript: ['@future-os/remote-client'], interop: false, desktopRust: false, mobile: false,
  });
});
test('crypto vectors select both owners; TS test helpers stay in TS', () => {
  const vectors = remoteAffected(['packages/remote-crypto/tests/vectors.json']);
  assert.deepEqual(vectors.rust, ['future-remote-crypto']);
  assert.deepEqual(vectors.typescript, ['@future-os/remote-crypto']);
  assert.equal(vectors.desktopRust, false);
  assert.deepEqual(remoteAffected(['packages/remote-crypto/tests/noiseBackend.cjs']).rust, []);
});
test('test-only Rust and TS files do not propagate to apps', () => {
  const plan = remoteAffected(['packages/remote-client/src/shared_credentials_tests.rs', 'packages/remote-client/tests/sharedCredentials.test.ts']);
  assert.deepEqual(plan.rust, ['future-remote-client']);
  assert.deepEqual(plan.typescript, ['@future-os/remote-client']);
  assert.equal(plan.interop, false);
  assert.equal(plan.desktopRust || plan.mobile, false);
});
test('schema and generator are shared contract inputs', () => {
  for (const input of ['command.schema.json', 'scripts/generate.py']) {
    const plan = remoteAffected([`packages/remote-protocol/${input}`]);
    assert.deepEqual(plan.rust, ['future-remote-protocol', 'future-remote-client', 'future-remote-server']);
    assert.deepEqual(plan.typescript, ['@future-os/remote-protocol', '@future-os/remote-client']);
    assert.equal(plan.interop && plan.desktopRust && plan.mobile, true);
  }
});
test('language locks and package configs keep independent scopes', () => {
  assert.equal(remoteAffected(['Cargo.lock']).typescript.length, 0);
  assert.equal(remoteAffected(['package-lock.json']).rust.length, 0);
  assert.deepEqual(remoteAffected(['packages/remote-client/jest.config.cjs']).typescript, ['@future-os/remote-client']);
  assert.equal(remoteAffected(['packages/remote-client/jest.config.cjs']).mobile, false);
});
test('real interop fixture and project edits require interop', () => {
  for (const file of ['client.json', 'isolation.json', 'transfer.json']) {
    assert.deepEqual(remoteAffected([`packages/remote-client/fixtures/${file}`]), {
      rust: [], typescript: [], interop: true, desktopRust: false, mobile: false,
    });
  }
  assert.equal(remoteAffected(['tests/remote-interop/src-ts/client.ts']).interop, true);
});
test('documentation and unrelated application files do not schedule core suites', () => {
  assert.deepEqual(remoteAffected(['docs/architecture/remote-package-development.md', 'packages/remote-client/fixtures/README.md', 'mobile/src/App.tsx']), {
    rust: [], typescript: [], interop: false, desktopRust: false, mobile: false,
  });
});
test('manual full acceptance and planner edits select every package', () => {
  for (const plan of [remoteAffected([], true), remoteAffected(['scripts/ci/remote-checks.mjs'])]) {
    assert.equal(plan.rust.length, 4);
    assert.equal(plan.typescript.length, 3);
    assert.equal(plan.interop && plan.desktopRust && plan.mobile, true);
  }
});
test('invalid path lists fail instead of silently running nothing', () => {
  assert.throws(() => remoteAffected({}));
  assert.throws(() => remoteAffected([42]));
  assert.throws(() => remoteAffected(['path\nwith-newline']));
});

test('Remote source and test configs cannot borrow application modules', () => {
  for (const name of ['react', 'react-native', 'expo-device', 'jest-expo', '@tauri-apps/api', '@futureos/desktop', '@future-os/mobile', '../../../desktop/src/app']) {
    assert.throws(() => checkRemoteSpecifier('client', name));
    assert.throws(() => checkRemoteSpecifier('client', name, { production: false }));
  }
});
test('Protocol can share wire types with projection without importing its runtime', () => {
  assert.doesNotThrow(() => checkRemoteSpecifier('protocol', '@future-os/thread-projection', { typeOnly: true }));
  assert.throws(() => checkRemoteSpecifier('protocol', '@future-os/thread-projection'));
  assert.doesNotThrow(() => checkRemoteSpecifier('client', '@future-os/thread-projection'));
  assert.throws(() => checkRemoteSpecifier('crypto', '@future-os/remote-client'));
  assert.throws(() => checkRemoteSpecifier('projection', '@future-os/remote-protocol'));
});
