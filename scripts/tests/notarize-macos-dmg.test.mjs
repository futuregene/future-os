import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import test from 'node:test';
import { notarizeDmg } from '../ci/notarize-macos-dmg.mjs';

const id = 'd4510fe1-995d-4828-bb00-60c8f4693bc8';
const response = (data, status = 0) => ({ stdout: JSON.stringify(data), stderr: '', status });

function fixture(t, results) {
  const directory = mkdtempSync(join(tmpdir(), 'futureos-notary-'));
  t.after(() => rmSync(directory, { recursive: true, force: true }));
  const calls = [];
  let clock = 0;
  const options = {
    run(args, timeout) {
      calls.push({ args, timeout });
      if (args[0] === 'wait') {
        // The ID must already be durable before the first network wait.
        assert.equal(readFileSync(join(directory, 'submission-id.txt'), 'utf8'), `${id}\n`);
        assert.equal(args[1], id);
      }
      assert.ok(timeout > 0 && timeout <= 10 * 60_000);
      assert.ok(results.length, `Unexpected command: ${args}`);
      return results.shift();
    },
    now: () => clock,
    sleep: async ms => { clock += ms; },
  };
  return { directory, calls, options, advance: ms => { clock += ms; } };
}

test('recovers disconnected and malformed responses without submitting again', async t => {
  const f = fixture(t, [
    response({ id }),
    { status: 1, stdout: '', stderr: 'NSURLErrorDomain Code=-1009 No network route' },
    { status: null, stdout: '{', stderr: '', error: new Error('ETIMEDOUT') },
    response({ id, status: 'In Progress' }),
    response({ id, status: 'Accepted' }),
  ]);
  assert.equal(await notarizeDmg('test.dmg', f.directory, f.options), id);
  assert.deepEqual(f.calls.map(c => c.args[0]), ['submit', 'wait', 'wait', 'wait', 'wait']);
  assert.equal(readFileSync(join(f.directory, 'wait-1.stderr.txt'), 'utf8'),
    'NSURLErrorDomain Code=-1009 No network route');
  assert.equal(readFileSync(join(f.directory, 'submit.json'), 'utf8'), JSON.stringify({ id }));
});

test('resumes a valid submission even when submit reports a transport failure', async t => {
  const f = fixture(t, [response({ id }, 1), response({ id, status: 'Accepted' })]);
  assert.equal(await notarizeDmg('test.dmg', f.directory, f.options), id);
  assert.equal(f.calls.length, 2);
});

for (const status of ['Invalid', 'Rejected']) {
  test(`fails immediately on ${status} and retrieves rejection diagnostics`, async t => {
    const f = fixture(t, [response({ id }), response({ id, status }), response({})]);
    await assert.rejects(notarizeDmg('test.dmg', f.directory, f.options), new RegExp(status));
    assert.deepEqual(f.calls.map(c => c.args[0]), ['submit', 'wait', 'log']);
    assert.equal(f.calls[2].args[1], id);
  });
}

test('does not re-upload when submit fails without a usable ID', async t => {
  const f = fixture(t, [{ status: 1, stdout: '', stderr: 'offline' }]);
  await assert.rejects(notarizeDmg('test.dmg', f.directory, f.options), /Not resubmitting/);
  assert.equal(f.calls.length, 1);
  assert.equal(readFileSync(join(f.directory, 'submit.stderr.txt'), 'utf8'), 'offline');
});

test('refuses an Accepted result belonging to a different submission', async t => {
  const f = fixture(t, [response({ id }), response({ id: 'another-id', status: 'Accepted' })]);
  await assert.rejects(notarizeDmg('test.dmg', f.directory, f.options), /different submission ID/);
});

test('bounds the total retry period and preserves the submission for follow-up', async t => {
  const f = fixture(t, [response({ id }), response({ id, status: 'In Progress' }, 1)]);
  f.options.sleep = async () => { f.advance(45 * 60_000); };
  await assert.rejects(notarizeDmg('test.dmg', f.directory, f.options), /Timed out/);
  assert.equal(f.calls.length, 2);
  assert.equal(readFileSync(join(f.directory, 'submission-id.txt'), 'utf8'), `${id}\n`);
});

test('test builds can stage signed artifacts after notarization and verification fail', t => {
  const f = fixture(t, []);
  const workflow = readFileSync(new URL('../../.github/workflows/build-macos-signed.yml', import.meta.url), 'utf8');
  function step(name) {
    const block = workflow.split(`      - name: ${name}\n`)[1]?.split('\n      - name: ')[0];
    assert.ok(block, `Missing workflow step: ${name}`);
    return block;
  }
  function script(name) {
    return step(name).split('        run: |\n')[1].split('\n')
      .map(line => line.startsWith('          ') ? line.slice(10) : line).join('\n');
  }
  // The real workflows must keep failure tolerance conditional on test mode.
  assert.match(step('Notarize and staple DMG'), /continue-on-error: \$\{\{ !inputs.strict_verification \}\}/);
  assert.match(step('Verify signed artifacts'), /continue-on-error: \$\{\{ !inputs.strict_verification \}\}/);
  const release = readFileSync(new URL('../../.github/workflows/release.yml', import.meta.url), 'utf8');
  const build = readFileSync(new URL('../../.github/workflows/build.yml', import.meta.url), 'utf8');
  assert.match(release, /strict_verification: true/);
  assert.match(build, /strict_verification: false/);

  for (const directory of ['bin', 'target/release/bundle/dmg', 'target/release/bundle/macos/FutureOS.app']) {
    mkdirSync(join(f.directory, directory), { recursive: true });
  }
  for (const [file, contents] of [
    ['target/release/bundle/dmg/test.dmg', 'signed-dmg'],
    ['target/release/bundle/macos/FutureOS.app.tar.gz', 'signed-updater'],
    ['target/release/bundle/macos/FutureOS.app.tar.gz.sig', 'updater-signature'],
    ['bin/codesign', '#!/bin/sh\nexit "${FAIL_CODESIGN:-0}"\n'],
    ['bin/hdiutil', '#!/bin/sh\nexit 0\n'],
    ['bin/node', '#!/bin/sh\necho offline >&2\nexit 1\n'],
  ]) {
    writeFileSync(join(f.directory, file), contents, { mode: 0o755 });
  }
  const envFile = join(f.directory, 'github-env');
  /** @type {NodeJS.ProcessEnv} */
  const env = { ...process.env, PATH: `${join(f.directory, 'bin')}:${process.env.PATH}`,
    GITHUB_ENV: envFile, RUNNER_TEMP: f.directory };
  function execute(name, replacements = {}) {
    let source = script(name);
    for (const [expression, value] of Object.entries(replacements)) source = source.replaceAll(expression, value);
    return spawnSync('bash', ['-c', source], { cwd: f.directory, env, encoding: 'utf8' });
  }
  function loadEnv() {
    for (const line of readFileSync(envFile, 'utf8').trim().split('\n')) {
      const separator = line.indexOf('=');
      env[line.slice(0, separator)] = line.slice(separator + 1);
    }
  }
  const prepared = execute('Prepare signed DMG');
  assert.equal(prepared.status, 0, prepared.stderr);
  loadEnv();
  assert.equal(env.SIGNED_DMG, 'target/release/bundle/dmg/test.dmg');
  assert.equal(execute('Notarize and staple DMG').status, 1);
  env.FAIL_CODESIGN = '1';
  assert.equal(execute('Verify signed artifacts').status, 1);
  loadEnv();
  const staged = execute('Stage artifacts', {
    '${{ inputs.signed }}': 'true',
    '${{ steps.v.outputs.artifact_version }}': '0.0.2-nightly.1',
    '${{ matrix.asset_arch }}': 'x64',
  });
  assert.equal(staged.status, 0, staged.stderr);
  const base = join(f.directory, 'release-assets/FutureOS_0.0.2-nightly.1_x64');
  assert.equal(readFileSync(`${base}.dmg`, 'utf8'), 'signed-dmg');
  assert.equal(readFileSync(`${base}.app.tar.gz`, 'utf8'), 'signed-updater');
  assert.equal(readFileSync(`${base}.app.tar.gz.sig`, 'utf8'), 'updater-signature');
});
