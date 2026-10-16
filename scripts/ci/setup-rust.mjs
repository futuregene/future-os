#!/usr/bin/env node
// rust-cache hashes every installed toolchain, including unused runner defaults.
// Keep a job-local rustup home containing only the project's toolchain.
import { spawnSync } from 'node:child_process';
import { appendFileSync } from 'node:fs';
import { homedir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';

if (!process.env.RUNNER_TEMP || !process.env.GITHUB_ENV) {
  throw new Error('This setup script requires a GitHub Actions job');
}
const root = fileURLToPath(new URL('../../', import.meta.url));
const rustupHome = join(process.env.RUNNER_TEMP, 'futureos-rustup');
const cargoHome = process.env.CARGO_HOME || join(homedir(), '.cargo');
const env = { ...process.env, RUSTUP_HOME: rustupHome, CARGO_HOME: cargoHome };
// Let rustup select the version, components and targets from rust-toolchain.toml.
delete env.RUSTUP_TOOLCHAIN;

function rustup(args, capture = false) {
  const result = spawnSync('rustup', args, {
    cwd: root,
    env,
    encoding: 'utf8',
    stdio: capture ? ['inherit', 'pipe', 'inherit'] : 'inherit',
  });
  if (result.error) throw result.error;
  if (result.status !== 0) process.exit(result.status ?? 1);
  return result.stdout?.trim();
}

rustup(['toolchain', 'install', '--profile', 'minimal', '--no-self-update']);
const toolchain = rustup(['show', 'active-toolchain'], true).split(/\s+/)[0];
rustup(['default', toolchain]);
console.log(`CI Rust toolchain: ${toolchain}`);
const settings = {
  RUSTUP_HOME: rustupHome,
  RUSTUP_TOOLCHAIN: toolchain,
  CARGO_HOME: cargoHome,
  CARGO_INCREMENTAL: process.env.CARGO_INCREMENTAL ?? '0',
  CARGO_TERM_COLOR: process.env.CARGO_TERM_COLOR ?? 'always',
};
appendFileSync(process.env.GITHUB_ENV,
  Object.entries(settings).map(([key, value]) => `${key}=${value}\n`).join(''));
