// One execution plan for PRs, merge queues, main cache warming and manual runs.
import { appendFileSync, existsSync, readFileSync, readdirSync } from 'node:fs';
import { dirname, relative, resolve } from 'node:path';
import { spawnSync } from 'node:child_process';
import { fileURLToPath, pathToFileURL } from 'node:url';

export const root = fileURLToPath(new URL('../../', import.meta.url));
const json = file => JSON.parse(readFileSync(file, 'utf8'));
const posix = path => path.replaceAll('\\', '/');
const within = (file, directory) => file === directory || file.startsWith(`${directory}/`);
const special = 'tests/remote-interop';
const testOnly = file => /(?:^|\/)(?:tests?|fixtures|__tests__)(?:\/|\.rs$)/.test(file)
  || /(?:\.test\.|\.spec\.|_tests?\.rs$|(?:eslint|jest|vitest|stylelint)\.config\.)/.test(file);

export function catalog() {
  const result = spawnSync('cargo', ['metadata', '--locked', '--offline', '--no-deps', '--format-version', '1'], {
    cwd: root, encoding: 'utf8', maxBuffer: 16 * 1024 * 1024,
  });
  if (result.error) throw result.error;
  if (result.status !== 0) throw new Error(`Cargo metadata failed: ${result.stderr}`);
  const metadata = JSON.parse(result.stdout);
  const members = new Set(metadata.workspace_members);
  const rust = metadata.packages.filter(pkg => members.has(pkg.id)).map(pkg => ({
    name: pkg.name, directory: posix(relative(root, dirname(pkg.manifest_path))),
    dependencies: [...new Set(pkg.dependencies.filter(dep => dep.path).map(dep => dep.name))],
  }));
  const manifests = json(resolve(root, 'package.json')).workspaces.flatMap(pattern => {
    if (!pattern.endsWith('/*')) return [pattern];
    const parent = pattern.slice(0, -2);
    return readdirSync(resolve(root, parent), { withFileTypes: true })
      .filter(entry => entry.isDirectory()).map(entry => `${parent}/${entry.name}`);
  });
  const typescript = manifests.filter(directory => existsSync(resolve(root, directory, 'package.json'))).map(directory => {
    const pkg = json(resolve(root, directory, 'package.json'));
    return { name: pkg.name, directory, dependencies: Object.keys({
      ...pkg.dependencies, ...pkg.devDependencies, ...pkg.peerDependencies, ...pkg.optionalDependencies,
    }) };
  });
  return { rust, typescript };
}

export function affected(files, projects, force = false) {
  if (!Array.isArray(files) || files.some(file => typeof file !== 'string' || !file || /[\r\n\0]/.test(file)
    || file.startsWith('/') || file.split('/').includes('..'))) throw new Error('Invalid changed-file list');
  const selected = { rust: new Map(), typescript: new Map() };
  const native = { android: new Set(), ios: new Set() };
  const addNative = (platform, reason) => native[platform].add(reason);
  const add = (language, name, reason, propagate = true, seen = new Set()) => {
    if (seen.has(name)) return;
    seen.add(name);
    const pkg = projects[language].find(item => item.name === name);
    // Interoperability is always an opt-in Misc suite, never a normal CI project.
    if (!pkg || pkg.directory === special) return;
    if (!selected[language].has(name)) selected[language].set(name, new Set());
    selected[language].get(name).add(reason);
    if (!propagate) return;
    if (pkg.directory === 'mobile') {
      for (const platform of Object.keys(native)) addNative(platform, reason);
    }
    for (const consumer of projects[language].filter(item => item.dependencies.includes(name))) {
      add(language, consumer.name, `depends on ${name} (${reason})`, true, seen);
    }
  };
  const all = (language, reason) => projects[language].forEach(pkg => add(language, pkg.name, reason));
  const mobile = reason => add('typescript', '@future-os/mobile', reason);
  if (force) for (const language of Object.keys(selected)) all(language, 'manual full run');
  for (const file of files) {
    if (/\.(?:md|mdx)$/.test(file) || within(file, special)) continue;
    if (['desktop/src-tauri/windows/installer-hooks.nsh',
      'desktop/src-tauri/windows/installer-preflight.ps1'].includes(file)) continue;
    // A workflow helper changes the execution semantics; validate every regular suite.
    if (['.github/workflows/ci.yml', '.github/workflows/rust-ci.yml', 'scripts/ci/affected.mjs',
      'scripts/ci/rust-checks.mjs', 'scripts/ci/typescript-checks.mjs', 'scripts/ci/ci-result.mjs',
      'scripts/tests/ci-plan.test.mjs'].includes(file)) {
      for (const language of Object.keys(selected)) all(language, file);
      continue;
    }
    if (['Cargo.toml', 'Cargo.lock', 'rust-toolchain.toml', 'scripts/ci/setup-rust.mjs',
      'scripts/ci/install-apt-packages.sh'].includes(file) || within(file, '.cargo')) {
      all('rust', file); continue;
    }
    if (['package.json', 'package-lock.json', '.npmrc', 'packages/eslint.config.mjs'].includes(file)) {
      all('typescript', file); continue;
    }
    if (file === 'scripts/version.mjs') {
      mobile(file);
      add('typescript', '@futureos/desktop', file);
      add('rust', 'futureos', file);
      continue;
    }
    if (file === 'scripts/ci/host-tests.py') { add('rust', 'future-host', file, false); continue; }
    if (file === 'scripts/tests/test-mobile-share-io.py' || file === '.github/workflows/mobile-native-io.yml') {
      addNative('android', file); continue;
    }
    if (file === 'scripts/tests/test-mobile-ios-share.py') { addNative('ios', file); continue; }
    // These native trees do not change the other platform or the JS unit suites.
    if (file.startsWith('mobile/')) {
      const platform = /(?:^|\/)(android|ios|ios-tests|extension)(?:\/|$)/.exec(file)?.[1];
      if (platform) { addNative(platform === 'android' ? 'android' : 'ios', file); continue; }
      if (file.startsWith('mobile/plugins/withIos')) { addNative('ios', file); continue; }
    }
    // A directory may contain Rust AND TS. File inputs choose the language;
    // schema/fixtures and unclassified inputs conservatively select both.
    const owners = Object.keys(selected).flatMap(language => projects[language]
      .filter(pkg => within(file, pkg.directory)).map(pkg => ({ language, ...pkg })));
    const deepest = Math.max(0, ...owners.map(pkg => pkg.directory.length));
    const rustInput = file.endsWith('.rs') || file.endsWith('/Cargo.toml');
    const tsInput = /\.(?:ts|tsx|js|jsx|mjs|cjs|css|html)$/.test(file)
      || /\/(?:package|tsconfig[^/]*)\.json$/.test(file);
    // A deleted package no longer appears in head's workspace inventory. Its
    // old paths must still invalidate consumers rather than produce an empty plan.
    if (!owners.length && file.startsWith('packages/')) {
      if (!tsInput) all('rust', `unowned package input: ${file}`);
      if (!rustInput) all('typescript', `unowned package input: ${file}`);
      continue;
    }
    for (const owner of owners.filter(pkg => pkg.directory.length === deepest)) {
      if (rustInput && owner.language !== 'rust') continue;
      if (tsInput && owner.language !== 'typescript') continue;
      add(owner.language, owner.name, file, !testOnly(file));
    }
  }
  // Group small crates to amortize setup/compilation; each group still contains
  // ONLY selected crates. Cargo metadata supplies transitive reverse dependencies.
  const group = pkg => pkg.name === 'futureos' ? 'desktop' : pkg.name === 'future-host' ? 'host'
    : pkg.name === 'future-agent' ? 'agent' : ['future-tui', 'future-cli'].includes(pkg.name) ? 'tui-cli'
      : pkg.name.startsWith('future-remote-') ? 'remote' : 'libraries';
  const rust = [];
  for (const pkg of projects.rust.filter(pkg => selected.rust.has(pkg.name))) {
    const name = group(pkg);
    let row = rust.find(item => item.name === name);
    if (!row) { row = { name, packages: [], desktop: name === 'desktop' }; rust.push(row); }
    row.packages.push(pkg.name);
  }
  const typescript = projects.typescript.filter(pkg => selected.typescript.has(pkg.name))
    .map(pkg => ({ name: pkg.name, directory: pkg.directory }));
  return {
    rust, typescript, android: native.android.size > 0, ios: native.ios.size > 0,
    reasons: {
      rust: Object.fromEntries([...selected.rust].map(([name, reasons]) => [name, [...reasons]])),
      typescript: Object.fromEntries([...selected.typescript].map(([name, reasons]) => [name, [...reasons]])),
      android: [...native.android], ios: [...native.ios],
    },
  };
}

if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  const plan = affected(json(process.argv[2]), catalog(), process.env.FORCE_ALL === 'true');
  console.log(JSON.stringify(plan, null, 2));
  if (process.env.GITHUB_OUTPUT) appendFileSync(process.env.GITHUB_OUTPUT,
    ['rust', 'typescript', 'android', 'ios'].map(key => `${key}=${JSON.stringify(plan[key])}\n`).join(''));
  if (process.env.GITHUB_STEP_SUMMARY) appendFileSync(process.env.GITHUB_STEP_SUMMARY,
    `## Affected CI plan\n\n\`\`\`json\n${JSON.stringify(plan, null, 2)}\n\`\`\`\n`);
}
