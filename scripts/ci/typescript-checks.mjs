import { timedCommand } from './timed-command.mjs';
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { root } from './affected.mjs';

const [phase] = process.argv.slice(2);
// TS jobs do not need Cargo/rustup: npm's workspace list is the authority here.
const directory = process.env.CI_DIRECTORY;
if (!directory || !['desktop', 'mobile'].includes(directory) && !/^packages\/[a-z0-9-]+$/.test(directory)) {
  throw new Error('Invalid TypeScript directory');
}
const pkg = JSON.parse(readFileSync(resolve(root, directory, 'package.json'), 'utf8'));
// Source-only workspace libraries: they ship TypeScript sources rather than a
// bundle and define no scripts of their own, so CI supplies the whole pipeline
// (tsc typecheck/emit, the shared vitest project, the shared ESLint config).
// A new package of this kind must be registered here; otherwise the `shared`
// branches below fall through to `script(...)` and fail on a missing script,
// which is the intended loud failure rather than a silently skipped project.
const shared = ['markdown', 'thread-projection', 'json-preview', 'peer-icons'].includes(directory.split('/')[1]);
const cwd = resolve(root, directory);
async function run(command, args, workingDirectory = cwd) {
  const result = await timedCommand(command, args, { cwd: workingDirectory });
  if (result.status !== 0) process.exit(result.status ?? 1);
}
async function script(name) {
  if (!pkg.scripts?.[name]) throw new Error(`${pkg.name} must define ${name}`);
  await run('npm', ['run', name]);
}
switch (phase) {
  case 'typecheck':
    if (shared) await run('npx', ['--no-install', 'tsc', '--noEmit', '-p', 'tsconfig.json']);
    else await script('typecheck');
    break;
  case 'build':
    if (directory === 'desktop') await script('build');
    else if (directory === 'mobile') {
      await run('npx', ['--no-install', 'expo', 'export', '--platform', 'android', '--platform', 'ios',
        '--output-dir', resolve(process.env.RUNNER_TEMP, 'mobile-bundle')]);
    } else {
      // Source libraries have no bundler output. Verify declaration/JS emission
      // without writing generated files into their source trees.
      await run('npx', ['--no-install', 'tsc', '-p', 'tsconfig.json', '--noEmit', 'false', '--declaration',
        '--rootDir', root,
        '--outDir', resolve(process.env.RUNNER_TEMP, 'typescript-build', pkg.name.split('/').pop())]);
    }
    break;
  case 'test':
    if (process.env.GITHUB_ACTIONS !== 'true') throw new Error('Unit tests run in CI');
    if (shared) await run('npx', ['--no-install', 'vitest', 'run', '--config', '../packages/markdown/vitest.config.ts',
      `${directory.split('/')[1]}/src/`], resolve(root, 'desktop'));
    else await script('test');
    break;
  case 'lint':
    if (shared) await run('npx', ['--no-install', 'eslint', '--config', '../eslint.config.mjs', 'src']);
    else if (directory === 'desktop') {
      // Desktop's local lint script also runs tsc. CI owns typecheck in the
      // test job, so keep this runner limited to ESLint and Stylelint.
      await run('npx', ['--no-install', 'eslint', 'src/**/*.{ts,tsx}', 'shot/**/*.{ts,tsx}', 'vite.shot.config.ts']);
    }
    else await script('lint');
    if (directory === 'desktop') await script('stylelint');
    break;
  default: throw new Error(`Unknown TypeScript phase: ${phase}`);
}
