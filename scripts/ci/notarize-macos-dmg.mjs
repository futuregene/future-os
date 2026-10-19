#!/usr/bin/env node
// Submit once, then recover interrupted status requests using the saved ID.
import { spawnSync } from 'node:child_process';
import { appendFileSync, mkdirSync, writeFileSync } from 'node:fs';
import { join, resolve } from 'node:path';
import { setTimeout } from 'node:timers/promises';
import { pathToFileURL } from 'node:url';

function runNotarytool(args, timeout) {
  const result = spawnSync('xcrun', ['notarytool', ...args,
    '--key', process.env.APPLE_API_KEY_PATH,
    '--key-id', process.env.APPLE_API_KEY_ID,
    '--issuer', process.env.APPLE_API_ISSUER,
    '--output-format', 'json'], {
    encoding: 'utf8', timeout, killSignal: 'SIGKILL',
  });
  if (result.error) console.error(result.error.message);
  return result;
}

export async function notarizeDmg(dmg, directory, {
  run = runNotarytool, now = () => performance.now(), sleep = ms => setTimeout(ms),
} = {}) {
  mkdirSync(directory, { recursive: true });
  function request(args, name, timeout) {
    const result = run(args, timeout);
    writeFileSync(join(directory, `${name}.json`), result.stdout || '');
    writeFileSync(join(directory, `${name}.stderr.txt`), result.stderr || '');
    if (result.stdout) console.log(result.stdout);
    if (result.stderr) console.error(result.stderr);
    let data;
    try { data = JSON.parse(result.stdout); } catch { data = {}; }
    if (!data || typeof data !== 'object' || Array.isArray(data)) data = {};
    return { ...result, data };
  }

  // Do not use submit --wait: a disconnected wait can lose the JSON response
  // containing the ID even though Apple has already received the upload.
  const submitted = request(['submit', dmg], 'submit', 10 * 60_000);
  const id = submitted.data.id;
  if (typeof id !== 'string' || !/^[0-9a-f]{8}-(?:[0-9a-f]{4}-){3}[0-9a-f]{12}$/i.test(id)) {
    throw new Error('No submission ID returned. Not resubmitting an upload with an unknown outcome; see submit diagnostics.');
  }
  writeFileSync(join(directory, 'submission-id.txt'), `${id}\n`);
  console.log(`DMG notarization submission: ${id}`);

  // A valid ID is enough to resume, even if submit returned a transport error.
  // Bound the complete wait and each process so a network stall cannot consume
  // the Actions step timeout and prevent the test-build fallback from running.
  const deadline = now() + 45 * 60_000;
  let attempt = 0;
  while (now() < deadline) {
    const remaining = deadline - now();
    const seconds = Math.max(1, Math.floor(Math.min(180_000, remaining) / 1000));
    const result = request(['wait', id, '--timeout', `${seconds}s`],
      `wait-${++attempt}`, Math.min(remaining, (seconds + 15) * 1000));
    if (result.data.id && result.data.id !== id) {
      throw new Error('Apple returned a different submission ID; refusing to accept its status.');
    }
    const status = result.data.status;
    if (status === 'Accepted') return id;
    if (status === 'Invalid' || status === 'Rejected') {
      request(['log', id, join(directory, 'rejection-log.json')], 'rejection', 60_000);
      throw new Error(`DMG notarization ${status}: ${id}`);
    }
    console.warn(`Notarization wait ${attempt} interrupted or still processing (exit=${result.status}, status=${status || 'unknown'}); continuing submission ${id}.`);
    await sleep(Math.max(0, Math.min(10_000, deadline - now())));
  }
  throw new Error(`Timed out waiting for DMG notarization; submission ${id} is saved for follow-up.`);
}

if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  try {
    for (const key of ['APPLE_API_KEY_PATH', 'APPLE_API_KEY_ID', 'APPLE_API_ISSUER']) {
      if (!process.env[key]) throw new Error(`${key} is required`);
    }
    const [dmg, directory] = process.argv.slice(2);
    if (!dmg || !directory) throw new Error('Usage: notarize-macos-dmg.mjs DMG DIAGNOSTICS_DIRECTORY');
    const id = await notarizeDmg(dmg, directory);
    if (process.env.GITHUB_STEP_SUMMARY) {
      appendFileSync(process.env.GITHUB_STEP_SUMMARY, `DMG notarization accepted: \`${id}\`\n`);
    }
  } catch (error) {
    console.error(error.message);
    process.exitCode = 1;
  }
}
