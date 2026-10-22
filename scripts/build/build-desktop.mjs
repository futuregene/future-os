#!/usr/bin/env node
// Build the CLI and GUI in one Cargo invocation so their shared dependencies
// use the same feature set. Tauri still owns asset embedding and platform flags.
import { spawnSync } from 'node:child_process';
import { chmodSync, copyFileSync, existsSync, mkdirSync, mkdtempSync, rmSync, symlinkSync } from 'node:fs';
import { createRequire } from 'node:module';
import { tmpdir } from 'node:os';
import { delimiter, dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '../..');
const desktop = join(root, 'desktop');
const require = createRequire(import.meta.url);
const tauri = require.resolve('@tauri-apps/cli/tauri.js');
const args = process.argv.slice(2);
const separator = args.indexOf('--');
const tauriArgs = separator < 0 ? args : args.slice(0, separator);
const cargoArgs = separator < 0 ? [] : args.slice(separator + 1);

function run(command, argv, capture = false, env) {
  const result = spawnSync(command, argv, {
    cwd: desktop,
    stdio: capture ? ['inherit', 'pipe', 'inherit'] : 'inherit',
    encoding: 'utf8',
    env: env ?? process.env,
  });
  if (result.error) throw result.error;
  if (result.status !== 0) process.exit(result.status ?? 1);
  return result.stdout?.trim();
}

function option(argv, long, short) {
  for (let i = 0; i < argv.length; i++) {
    if (argv[i] === long || (short && argv[i] === short)) return argv[i + 1];
    if (argv[i].startsWith(`${long}=`)) return argv[i].slice(long.length + 1);
  }
}

// macOS bundling ends by stripping quarantine attributes with `xattr -r`, which
// only the system binary supports: Homebrew also ships an `xattr` (a Python
// wrapper without -r) and its bin directory usually comes first on PATH, so the
// bundle dies after the .app is already signed. Give that single tool the system
// binary through a shim directory. Prepending /usr/bin itself would also shadow
// git, python3 and friends for Cargo build scripts.
function bundleEnv() {
  const systemXattr = '/usr/bin/xattr';
  if (process.platform !== 'darwin' || !existsSync(systemXattr)) return undefined;
  const shim = mkdtempSync(join(tmpdir(), 'futureos-xattr-'));
  symlinkSync(systemXattr, join(shim, 'xattr'));
  process.on('exit', () => rmSync(shim, { recursive: true, force: true }));
  return { ...process.env, PATH: `${shim}${delimiter}${process.env.PATH ?? ''}` };
}

function stage(source, destination) {
  mkdirSync(dirname(destination), { recursive: true });
  // Replace the inode: overwriting a signed Mach-O in place can leave stale
  // taskgated signing state on macOS. Never mutate Cargo's original output.
  rmSync(destination, { force: true });
  copyFileSync(source, destination);
  chmodSync(destination, 0o755);
}

if (tauriArgs.some(arg => ['--help', '-h', '--version', '-V'].includes(arg))) {
  run(process.execPath, [tauri, 'build', ...tauriArgs]);
  process.exit(0);
}

// Linux releases supply a separately built static musl CLI. A GNU GUI cannot
// share machine code with that target, so don't build an unused GNU CLI too.
const prebuiltCli = process.env.FUTURE_DESKTOP_CLI;
const packages = prebuiltCli
  ? ['-p', 'futureos', '--bin', 'futureos']
  : ['-p', 'future-cli', '-p', 'futureos', '--bin', 'future', '--bin', 'futureos'];
const buildArgs = tauriArgs.includes('--no-bundle') ? tauriArgs : [...tauriArgs, '--no-bundle'];
run(process.execPath, [tauri, 'build', ...buildArgs, '--', ...packages, ...cargoArgs]);

const metadata = JSON.parse(run('cargo', [
  'metadata', '--no-deps', '--format-version', '1', '--manifest-path', 'src-tauri/Cargo.toml',
], true));
const targetDir = option(cargoArgs, '--target-dir')
  ? resolve(desktop, option(cargoArgs, '--target-dir'))
  : metadata.target_directory;
const host = run('rustc', ['-vV'], true).match(/^host: (.+)$/m)?.[1];
if (!host) throw new Error('Cannot determine the Rust host target');
const target = option(tauriArgs, '--target', '-t') || process.env.CARGO_BUILD_TARGET;
const triple = target || host;
const extension = triple.includes('windows') ? '.exe' : '';
const profile = option(cargoArgs, '--profile') || (tauriArgs.includes('--debug') || tauriArgs.includes('-d') ? 'debug' : 'release');
const output = join(targetDir, target || '', profile === 'dev' ? 'debug' : profile);
const cli = prebuiltCli ? resolve(root, prebuiltCli) : join(output, `future${extension}`);
if (target === 'universal-apple-darwin' && !prebuiltCli) {
  mkdirSync(output, { recursive: true });
  run('lipo', ['-create', '-output', cli,
    join(targetDir, 'aarch64-apple-darwin', profile, 'future'),
    join(targetDir, 'x86_64-apple-darwin', profile, 'future'),
  ]);
}
stage(cli, join(desktop, 'src-tauri', 'binaries', `future-${triple}${extension}`));

if (!tauriArgs.includes('--no-bundle')) {
  // All build options except the runner and mismatch override also apply to
  // `tauri bundle`; Cargo-only arguments stay on the compilation invocation.
  const bundleArgs = [];
  for (let i = 0; i < tauriArgs.length; i++) {
    const arg = tauriArgs[i];
    if (arg === '--runner' || arg === '-r') { i++; continue; }
    if (arg.startsWith('--runner=') || arg === '--ignore-version-mismatches') continue;
    bundleArgs.push(arg);
  }
  run(process.execPath, [tauri, 'bundle', ...bundleArgs], false, bundleEnv());
}
