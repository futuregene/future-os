// Remote package selection. Source changes propagate to real consumers;
// test-only inputs select their owners without rebuilding application hosts.
import { appendFileSync } from 'node:fs';
import { pathToFileURL } from 'node:url';

const rust = ['crypto', 'protocol', 'client', 'server'];
const typescript = ['crypto', 'protocol', 'client'];
const consumers = { crypto: ['protocol', 'client', 'server'], protocol: ['client', 'server'], client: [], server: [] };

export function remoteAffected(files, force = false) {
  if (!Array.isArray(files) || files.some(file => typeof file !== 'string' || file.includes('\n'))) {
    throw new Error('changed files must be a JSON array of paths');
  }
  const selected = { rust: new Set(), typescript: new Set() };
  let desktopRust = false, mobile = false, interop = false;
  const add = (language, role, propagate) => {
    selected[language].add(role);
    if (!propagate) return;
    for (const consumer of consumers[role]) {
      if (language === 'rust' || consumer !== 'server') selected[language].add(consumer);
    }
    if (language === 'rust') desktopRust = true;
    else mobile = true;
    interop = true;
  };
  const all = language => {
    for (const role of language === 'rust' ? rust : typescript) add(language, role, true);
  };
  if (force) { all('rust'); all('typescript'); }
  for (const file of files) {
    if (file === '.github/workflows/ci.yml' || file.startsWith('scripts/ci/remote-') || file === 'scripts/tests/remote-ci.test.mjs') {
      all('rust'); all('typescript'); continue;
    }
    if (['Cargo.toml', 'Cargo.lock', 'rust-toolchain.toml', 'scripts/ci/setup-rust.mjs'].includes(file) || file.startsWith('.cargo/')) all('rust');
    if (['package.json', 'package-lock.json', '.npmrc'].includes(file)) all('typescript');
    // Protocol's TS model-reference contract imports the projection package.
    if (file.startsWith('packages/thread-projection/') && !file.endsWith('.md') && !/\.(test|spec)\./.test(file)) {
      add('typescript', 'protocol', true);
    }
    if (file.startsWith('tests/remote-interop/') && !file.endsWith('.md')) interop = true;
    const match = /^packages\/remote-(crypto|protocol|client|server)\/(.+)$/.exec(file);
    if (!match || file.endsWith('.md')) continue;
    const [, role, input] = match;
    const hasTS = role !== 'server';
    const testOnly = input.startsWith('tests/') || input.startsWith('fixtures/') || /(?:^|\/)[^/]*tests\.rs$/.test(input) || /\.(test|spec)\./.test(input);
    if (input.endsWith('.rs') || input === 'Cargo.toml') {
      add('rust', role, !testOnly); continue;
    }
    if (/\.(?:ts|tsx|mjs|cjs)$/.test(input) || ['package.json', 'tsconfig.json'].includes(input)) {
      // The vector generator is TS tooling, even though its output is shared.
      if (hasTS) add('typescript', role, !testOnly && input.startsWith('src/'));
      if (input === 'package.json' && hasTS) add('typescript', role, true);
      continue;
    }
    if (input.startsWith('fixtures/') || input === 'tests/vectors.json') {
      add('rust', role, false);
      if (hasTS) add('typescript', role, false);
      if (role === 'client' && ['fixtures/client.json', 'fixtures/isolation.json'].includes(input)) interop = true;
      continue;
    }
    // Schema/codegen and unclassified build inputs fail toward broader scope.
    add('rust', role, true);
    if (hasTS) add('typescript', role, true);
  }
  // Interop composes all four Rust crates, including when only its TS driver changed.
  return {
    rust: rust.filter(role => selected.rust.has(role)).map(role => `future-remote-${role}`),
    typescript: typescript.filter(role => selected.typescript.has(role)).map(role => `@future-os/remote-${role}`),
    interop, desktopRust, mobile,
  };
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  const scope = process.env.REQUESTED_SCOPE || 'all';
  const manual = process.env.GITHUB_EVENT_NAME === 'workflow_dispatch';
  const plan = remoteAffected(JSON.parse(process.env.CHANGED_FILES || '[]'), manual && ['all', 'remote'].includes(scope));
  const outputs = {
    remote_rust: JSON.stringify(plan.rust), remote_ts: JSON.stringify(plan.typescript),
    remote_interop: String(plan.interop), remote_desktop: String(plan.desktopRust), remote_mobile: String(plan.mobile),
    remote: String(plan.rust.length > 0 || plan.typescript.length > 0 || plan.interop),
  };
  if (!process.env.GITHUB_OUTPUT) throw new Error('GITHUB_OUTPUT is required');
  appendFileSync(process.env.GITHUB_OUTPUT, Object.entries(outputs).map(([key, value]) => `${key}=${value}\n`).join(''));
  console.log(JSON.stringify(plan, null, 2));
}
