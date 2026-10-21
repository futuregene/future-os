import assert from 'node:assert/strict';
import test from 'node:test';
import { readFileSync } from 'node:fs';
import { affected, catalog, manualPlan } from '../ci/affected.mjs';
import { checkResult } from '../ci/ci-result.mjs';

// Exercise the real manifests so newly introduced path dependencies participate.
const projects = catalog();
const plan = (...files) => affected(files, projects);
const rust = result => result.rust.flatMap(group => group.packages);
const ts = result => result.typescript.map(pkg => pkg.name);

test('manual selection keeps exact Cargo/npm projects and existing runner groups', () => {
  const result = manualPlan('future-host, future-tui,future-cli,future-host,@future-os/markdown', projects);
  assert.deepEqual(new Set(rust(result)), new Set(['future-host', 'future-tui', 'future-cli']));
  assert.deepEqual(result.rust.map(row => row.name).sort(), ['host', 'tui-cli']);
  assert.deepEqual(ts(result), ['@future-os/markdown']);
  assert.equal(result.android || result.ios, false);
  assert.ok(!rust(result).includes('futureos'), 'manual selection does not expand to consumers');
});

test('empty manual selection is full and native projects are explicit', () => {
  assert.deepEqual(manualPlan('  ', projects), affected([], projects, true));
  const mobile = manualPlan('@future-os/mobile', projects);
  assert.equal(mobile.android || mobile.ios, false);
  const native = manualPlan('android,ios', projects);
  assert.equal(native.android && native.ios, true);
  assert.deepEqual(rust(native), []);
  assert.deepEqual(ts(native), []);
});

test('invalid manual selections fail rather than silently selecting nothing', () => {
  for (const selection of ['future-hots', 'future-host,', 'future-host,,future-cli', 'future-remote-interop']) {
    assert.throws(() => manualPlan(selection, projects), /Unknown manual CI project/);
  }
});

test('RPC reaches its transitive Rust consumers, including Desktop', () => {
  const result = plan('packages/rpc/src/lib.rs');
  for (const name of ['future-rpc', 'future-agent', 'future-host', 'future-cli', 'futureos']) {
    assert.ok(rust(result).includes(name), name);
  }
  assert.deepEqual(ts(result), []);
});

test('JSON preview reaches Desktop and Mobile and both native builds', () => {
  const result = plan('packages/json-preview/src/index.ts');
  assert.deepEqual(new Set(ts(result)), new Set(['@future-os/json-preview', '@futureos/desktop', '@future-os/mobile']));
  assert.ok(result.android && result.ios);
  assert.deepEqual(rust(result), []);
});

test('mixed-language packages select only the changed implementation language', () => {
  const rustPlan = plan('packages/remote-crypto/src/lib.rs');
  assert.ok(rust(rustPlan).includes('futureos'));
  assert.deepEqual(ts(rustPlan), []);
  const tsPlan = plan('packages/remote-crypto/src/index.ts');
  assert.deepEqual(rust(tsPlan), []);
  assert.ok(ts(tsPlan).includes('@future-os/mobile'));
});

test('shared contract inputs select both languages but interop stays opt-in', () => {
  const result = plan('packages/remote-protocol/command.schema.json');
  assert.ok(rust(result).includes('future-remote-protocol'));
  assert.ok(ts(result).includes('@future-os/remote-protocol'));
  assert.ok(!rust(result).includes('future-remote-interop'));
  assert.ok(!ts(result).includes('@future-os/remote-interop'));
});

test('test-only edits select their owners without rebuilding applications', () => {
  const result = plan('packages/remote-client/src/shared_credentials_tests.rs', 'packages/remote-client/tests/client.test.ts');
  assert.deepEqual(rust(result), ['future-remote-client']);
  assert.deepEqual(ts(result), ['@future-os/remote-client']);
  assert.equal(result.android || result.ios, false);
  assert.deepEqual(rust(plan('agent/src/tests.rs')), ['future-agent']);
});

test('native implementations are scoped to the relevant platform', () => {
  for (const file of ['mobile/android/app/build.gradle', 'mobile/modules/future-share-intent/android/src/main/Test.kt']) {
    const result = plan(file);
    assert.equal(result.android, true);
    assert.equal(result.ios, false);
    assert.deepEqual(ts(result), []);
  }
  for (const file of ['mobile/ios/App.swift', 'mobile/modules/future-share-intent/ios-tests/ShareInboxTests.swift',
    'mobile/modules/future-share-intent/extension/ShareViewController.swift', 'mobile/plugins/withIosShareExtension.js']) {
    const result = plan(file);
    assert.equal(result.ios, true);
    assert.equal(result.android, false);
  }
});

test('Desktop backend does not select its enclosing TS app', () => {
  const result = plan('desktop/src-tauri/src/lib.rs');
  assert.deepEqual(rust(result), ['futureos']);
  assert.deepEqual(ts(result), []);
  assert.equal(result.android || result.ios, false);
});

test('language-specific lockfiles expand only the corresponding graph', () => {
  assert.deepEqual(ts(plan('Cargo.lock')), []);
  const result = plan('package-lock.json');
  assert.deepEqual(rust(result), []);
  assert.ok(result.android && result.ios);
});

test('new dependency edges come from the project graph rather than path allowlists', () => {
  const graph = { ...projects, rust: [...projects.rust,
    { name: 'future-extra', directory: 'extra', dependencies: ['future-app-settings'] }] };
  assert.ok(rust(affected(['packages/app-settings/src/lib.rs'], graph)).includes('future-extra'));
});

test('deleted package inputs cannot disappear from the execution plan', () => {
  const result = plan('packages/deleted-library/package.json');
  assert.equal(ts(result).length, projects.typescript.length - 1);
  assert.ok(result.android && result.ios);
  assert.deepEqual(rust(result), []);
});

test('specialty tooling and docs do not schedule regular suites', () => {
  const result = plan('.github/workflows/misc.yml', 'scripts/ci/remote-checks.mjs',
    'scripts/ci/host-boundaries.py', 'tests/remote-interop/src/lib.rs', 'README.md');
  assert.deepEqual(rust(result), []);
  assert.deepEqual(ts(result), []);
  assert.equal(result.android || result.ios, false);
});

test('manual and planner changes select every regular project, excluding interop', () => {
  for (const result of [affected([], projects, true), plan('scripts/ci/affected.mjs'),
    plan('.github/workflows/typescript-ci.yml')]) {
    assert.equal(rust(result).length, projects.rust.length - 1);
    assert.equal(ts(result).length, projects.typescript.length - 1);
    assert.ok(result.android && result.ios);
  }
});

test('invalid changed-file input fails planning', () => {
  for (const files of [{}, [3], ['../escape'], ['a\nb'], ['/absolute']]) {
    assert.throws(() => affected(files, projects));
  }
});

const names = ['rust-linux-test', 'rust-windows-test', 'rust-macos-test', 'rust-check',
  'typescript-test', 'typescript-check', 'android-test', 'ios-test'];
function results(required = false) {
  return {
    affected: { result: 'success', outputs: { rust: required ? '[{}]' : '[]',
      typescript: required ? '[{}]' : '[]', android: String(required), ios: String(required) } },
    ...Object.fromEntries(names.map(name => [name, { result: required ? 'success' : 'skipped' }])),
  };
}

test('gate accepts success and explicitly unneeded skips', () => {
  assert.doesNotThrow(() => checkResult(results()));
  assert.doesNotThrow(() => checkResult(results(true)));
});

test('gate rejects failures, cancellations, missing jobs and required skips', () => {
  for (const name of names) {
    for (const result of ['failure', 'cancelled', 'skipped', undefined]) {
      const needs = results(true);
      needs[name].result = result;
      assert.throws(() => checkResult(needs), new RegExp(name));
    }
    const needs = results();
    needs[name].result = 'failure';
    assert.throws(() => checkResult(needs));
  }
  const needs = results();
  needs.affected.result = 'failure';
  assert.throws(() => checkResult(needs));
});

test('gate rejects malformed planner outputs', () => {
  for (const [key, value] of [['rust', '{}'], ['typescript', 'null'], ['android', ''], ['ios', 'garbage']]) {
    const needs = results();
    needs.affected.outputs[key] = value;
    assert.throws(() => checkResult(needs));
  }
});

test('regular CI contains no specialty commands and has a stable always-run gate', () => {
  const workflow = readFileSync(new URL('../../.github/workflows/ci.yml', import.meta.url), 'utf8');
  for (const command of ['REGENERATE_PROTO', 'remote-checks.mjs', 'host-boundaries.py',
    'check-embedded-agent.py', 'notarize-macos-dmg.test.mjs', 'test-windows-installer-preflight.ps1']) {
    assert.ok(!workflow.includes(command), command);
  }
  assert.match(workflow, /name: CI result/);
  assert.match(workflow, /merge_group:/);
  assert.match(workflow, /ci-result:[\s\S]*if: always\(\)/);
});

test('specialty workflows are manual and expose every migrated suite', () => {
  const workflow = readFileSync(new URL('../../.github/workflows/misc.yml', import.meta.url), 'utf8');
  assert.match(workflow, /workflow_dispatch:/);
  assert.doesNotMatch(workflow, /^  (?:pull_request|push|schedule):/m);
  for (const suite of ['generated', 'remote-interop', 'windows-update', 'macos-notarization',
    'flaky-stress', 'agent-lifecycle', 'remote-compile-measurement', 'headless-measurement', 'full-regression']) {
    assert.ok(workflow.includes(`inputs.suite == '${suite}'`), suite);
  }
});
