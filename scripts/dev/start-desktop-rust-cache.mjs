#!/usr/bin/env node
// Prints only wrapper variable names to clear; diagnostics go to stderr/a log.
import { spawnSync } from 'node:child_process';
import { appendFileSync, realpathSync } from 'node:fs';
import { basename } from 'node:path';
import { fileURLToPath } from 'node:url';

const wrapperKeys = ['RUSTC_WRAPPER', 'RUSTC_WORKSPACE_WRAPPER'];

export function checkRustCache(env, run) {
  const disable = [];
  const diagnostics = [];
  const compiler = env.RUSTC || env.CARGO_BUILD_RUSTC || 'rustc';
  for (const key of wrapperKeys) {
    // An explicitly empty RUSTC_* overrides the corresponding Cargo config env.
    const wrapper = env[key] ?? env[`CARGO_BUILD_${key}`];
    if (!wrapper || !/^sccache(?:\.exe)?$/i.test(basename(wrapper))) continue;
    const probe = run(wrapper, [compiler, '-vV']);
    if (probe.status === 0 && !probe.error) continue;
    const detail = [probe.stdout, probe.stderr, probe.error?.message].filter(Boolean).join('\n');
    diagnostics.push(`${key}: sccache preflight failed.\n${detail}`);
    const direct = run(compiler, ['-vV']);
    if (direct.status !== 0 || direct.error) {
      diagnostics.push('Direct rustc also failed; keeping the wrapper and original build behavior.');
      diagnostics.push([direct.stdout, direct.stderr, direct.error?.message].filter(Boolean).join('\n'));
      continue;
    }
    disable.push(key);
    diagnostics.push(`Bypassing ${key} for this launch only; direct rustc works. Cache files and shared server are unchanged.`);
  }
  return { disable, diagnostics };
}

// macOS /var is a symlink to /private/var; Node resolves the module URL.
if (process.argv[1] && realpathSync(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const result = checkRustCache(process.env, (command, args) => spawnSync(command, args, {
    env: process.env,
    encoding: 'utf8',
    timeout: 10000,
    maxBuffer: 1024 * 1024,
    windowsHide: true,
    stdio: ['ignore', 'pipe', 'pipe'],
  }));
  if (result.diagnostics.length) {
    const diagnostic = `${result.diagnostics.join('\n')}\n`;
    if (process.argv[2]) appendFileSync(process.argv[2], diagnostic);
    process.stderr.write(diagnostic);
  }
  for (const key of result.disable) console.log(key);
}
