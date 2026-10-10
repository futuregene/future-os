// Build an offline browser A/B probe. Artifacts are private and gitignored.
// Usage: node scripts/measure/measure-mobile-performance.mjs [baseline-ref]
import { build } from 'esbuild';
import { execFileSync } from 'node:child_process';
import { mkdir, writeFile } from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../..');
const out = path.join(root, 'target', 'mobile-performance');
const baseline = process.argv[2] || '8d96b076';
await mkdir(out, { recursive: true, mode: 0o700 });
const source = `export { SyncEngine } from './mobile/src/remote/syncEngine';
export { emptyTimeline, applyReplayEvents } from './mobile/src/remote/timeline';
export { createStreamingMarkdownParser, parseFutureMarkdown } from './packages/markdown/src/index';`;
for (const name of ['Baseline', 'Current']) {
  await build({
    stdin: { contents: source + (name === 'Current'
      ? `\nexport { decodeJsonBytes } from './mobile/src/remote/cooperativeJson';`
      : `\nexport async function decodeJsonBytes(bytes) { return JSON.parse(new TextDecoder().decode(bytes)); }`), resolveDir: root, loader: 'ts' },
    bundle: true, format: 'iife', globalName: `Mobile${name}`, platform: 'browser',
    outfile: path.join(out, `${name.toLowerCase()}.js`),
    alias: {
      '@future-os/thread-projection': path.join(root, 'packages/thread-projection/src/index.ts'),
      '@future-os/markdown': path.join(root, 'packages/markdown/src/index.ts'),
    },
    plugins: name === 'Baseline' ? [{ name: 'git-baseline', setup(builder) {
      builder.onLoad({ filter: /\.[cm]?tsx?$/ }, args => {
        const relative = path.relative(root, args.path).split(path.sep).join('/');
        if (!/^(mobile|packages)\//.test(relative)) return;
        return { contents: execFileSync('git', ['show', `${baseline}:${relative}`], { cwd: root, encoding: 'utf8', maxBuffer: 10 * 1024 * 1024 }),
          loader: relative.endsWith('.tsx') ? 'tsx' : 'ts', resolveDir: path.dirname(args.path) };
      });
    } }] : [],
  });
}
await build({ entryPoints: [path.join(root, 'scripts/measure/measure-mobile-performance.ts')], bundle: true,
  platform: 'browser', outfile: path.join(out, 'probe.js') });
await writeFile(path.join(out, 'build.json'), JSON.stringify({ baseline,
  current: execFileSync('git', ['rev-parse', 'HEAD'], { cwd: root, encoding: 'utf8' }).trim(),
  includesWorkingTree: true }, null, 2), { mode: 0o600 });
console.log(`Built private browser probe: ${out}`);
