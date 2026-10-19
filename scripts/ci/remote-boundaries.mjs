// Static manifest/source boundary audit. Never loads an application or test.
import { existsSync, readFileSync, readdirSync, realpathSync } from 'node:fs';
import { dirname, isAbsolute, relative, resolve, sep } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { spawnSync } from 'node:child_process';

const root = fileURLToPath(new URL('../../', import.meta.url));
const roles = ['crypto', 'protocol', 'client', 'server'];
const allowedRust = { crypto: [], protocol: ['crypto'], client: ['crypto', 'protocol'], server: ['crypto', 'protocol'], interop: roles };
const allowedTS = { crypto: [], protocol: ['crypto', 'projection'], client: ['crypto', 'protocol', 'projection'], projection: [], interop: ['crypto', 'protocol', 'client', 'projection'] };
const npmName = role => role === 'projection' ? '@future-os/thread-projection' : role === 'interop' ? '@future-os/remote-interop' : `@future-os/remote-${role}`;
const directory = role => resolve(root, role === 'interop' ? 'tests/remote-interop' : role === 'projection' ? 'packages/thread-projection' : `packages/remote-${role}`);
const within = (parent, child) => { const part = relative(parent, child); return part === '' || (!isAbsolute(part) && part !== '..' && !part.startsWith(`..${sep}`)); };
const forbidden = /^(?:react(?:-[^/]+)?|@react-native(?:-community)?\/[^/]+|expo(?:-[^/]+)?|jest-expo|@expo\/[^/]+|@tauri-apps\/[^/]+|@futureos\/desktop|@future-os\/(?:desktop|mobile|agent|cli|tui))(?:\/|$)/;

export function checkRemoteSpecifier(role, specifier, { file = resolve(directory(role), 'src/index.ts'), production = true, typeOnly = false } = {}) {
  if (!Object.hasOwn(allowedTS, role)) throw new Error('unknown Remote role');
  if (forbidden.test(specifier)) throw new Error(`${role}: application dependency ${specifier}`);
  if (specifier.startsWith('.')) {
    const target = resolve(dirname(file), specifier);
    if (![role, ...allowedTS[role]].some(allowed => within(directory(allowed), target))) throw new Error(`${role}: relative import escapes role boundary: ${specifier}`);
    if (role === 'protocol' && production && within(directory('projection'), target) && !typeOnly) throw new Error('Protocol may only import projection wire types');
  } else if (specifier.startsWith('@future-os/') || specifier.startsWith('@futureos/')) {
    const name = specifier.split('/').slice(0, 2).join('/');
    if (![role, ...allowedTS[role]].some(allowed => npmName(allowed) === name)) throw new Error(`${role}: invalid workspace dependency ${specifier}`);
    if (role === 'protocol' && production && name === npmName('projection') && !typeOnly) throw new Error('Protocol may only import projection wire types');
  } else if (isAbsolute(specifier) || /^(?:file|https?):/.test(specifier)) throw new Error(`${role}: absolute/URL module import`);
}

function checkRust() {
  const result = spawnSync('cargo', ['metadata', '--locked', '--offline', '--no-deps', '--format-version', '1'], {
    cwd: root, encoding: 'utf8', env: { ...process.env, RUSTC_WRAPPER: '' }, maxBuffer: 16 * 1024 * 1024,
  });
  if (result.error) throw result.error;
  if (result.status !== 0) throw new Error(`Cargo metadata failed: ${result.stderr}`);
  const metadata = JSON.parse(result.stdout);
  const workspace = new Set(metadata.workspace_members);
  const packages = metadata.packages.filter(pkg => workspace.has(pkg.id));
  const names = new Set(packages.map(pkg => pkg.name));
  for (const role of [...roles, 'interop']) {
    const name = `future-remote-${role}`;
    const pkg = packages.find(pkg => pkg.name === name);
    if (!pkg) throw new Error(`missing ${name}`);
    for (const dependency of pkg.dependencies) {
      if (names.has(dependency.name) && !allowedRust[role].some(allowed => dependency.name === `future-remote-${allowed}`)) throw new Error(`${name}: application/cross-role dependency ${dependency.name}`);
      if (dependency.name.startsWith('tauri')) throw new Error(`${name}: GUI dependency`);
      if (['crypto', 'protocol'].includes(role) && dependency.kind !== 'dev' && ['tokio', 'async-nats', 'reqwest'].includes(dependency.name)) throw new Error(`${name}: transport/runtime dependency`);
    }
    const library = pkg.targets.find(target => target.kind.includes('lib'));
    if (!library || library.crate_types.length !== 1 || library.crate_types[0] !== 'lib') throw new Error(`${name}: expected ordinary Rust library`);
    const enabled = new Set(), pending = [...(pkg.features.default || [])];
    while (pending.length) {
      const feature = pending.pop();
      if (enabled.has(feature)) continue;
      enabled.add(feature);
      pending.push(...(pkg.features[feature] || []));
    }
    if ([...enabled].some(feature => feature === 'test-support' || feature.endsWith('/test-support'))) throw new Error(`${name}: fixture transport must not be enabled by default`);
    if (role === 'interop') {
      if (enabled.has('network-tests')) throw new Error('interop network tests must be opt-in');
      for (const target of pkg.targets.filter(target => target.kind.includes('bin') || target.kind.includes('test'))) {
        if (!(target['required-features'] || []).includes('network-tests')) throw new Error('interop executable must require network-tests');
      }
    } else if (pkg.targets.some(target => target.kind.includes('bin'))) throw new Error(`${name}: product entry point outside phase 0-5`);
  }
  return packages.filter(pkg => pkg.name.startsWith('future-remote-')).length;
}

function sourceFiles(path) {
  return readdirSync(path, { withFileTypes: true }).flatMap(entry => {
    if (['node_modules', '.artifacts', 'target'].includes(entry.name)) return [];
    const file = resolve(path, entry.name);
    return entry.isDirectory() ? sourceFiles(file) : /\.(?:ts|tsx|mjs|cjs|js)$/.test(file) ? [file] : [];
  });
}
async function checkTypeScript() {
  const ts = await import('typescript');
  let count = 0;
  for (const role of Object.keys(allowedTS)) {
    const pkg = JSON.parse(readFileSync(resolve(directory(role), 'package.json'), 'utf8'));
    if (pkg.name !== npmName(role)) throw new Error(`invalid package name for ${role}`);
    for (const dependency of Object.keys({ ...pkg.dependencies, ...pkg.devDependencies, ...pkg.peerDependencies, ...pkg.optionalDependencies })) {
      checkRemoteSpecifier(role, dependency, { production: false });
      if (dependency === pkg.name) throw new Error(`${role}: self dependency`);
    }
    for (const script of Object.values(pkg.scripts || {})) {
      if (/(?:desktop|mobile)[/\\]|jest-expo/.test(script)) throw new Error(`${role}: application test configuration`);
    }
    const configFile = resolve(directory(role), 'tsconfig.json');
    let options = { moduleResolution: ts.ModuleResolutionKind.Bundler, module: ts.ModuleKind.ESNext, resolveJsonModule: true };
    if (existsSync(configFile)) {
      const config = ts.readConfigFile(configFile, ts.sys.readFile);
      if (config.error) throw new Error(`${role}: invalid TypeScript config`);
      for (const parent of config.config.extends ? [].concat(config.config.extends) : []) checkRemoteSpecifier(role, parent, { file: configFile, production: false });
      const parsed = ts.parseJsonConfigFileContent(config.config, ts.sys, directory(role));
      options = parsed.options;
    }
    const resolutionCache = ts.createModuleResolutionCache(directory(role), file => file, options);
    for (const file of sourceFiles(directory(role))) {
      const source = ts.createSourceFile(file, readFileSync(file, 'utf8'), ts.ScriptTarget.Latest, true);
      const production = within(resolve(directory(role), 'src'), file);
      const inspect = (specifier, typeOnly = false) => {
        checkRemoteSpecifier(role, specifier, { file, production, typeOnly });
        const resolved = ts.resolveModuleName(specifier, file, options, ts.sys, resolutionCache).resolvedModule;
        if (resolved && existsSync(resolved.resolvedFileName)) {
          const target = realpathSync(resolved.resolvedFileName);
          if (['desktop', 'mobile', 'agent', 'cli', 'tui', 'channels', 'orchestration'].some(app => within(resolve(root, app), target))) throw new Error(`${role}: module alias resolves into application: ${specifier}`);
          if (within(resolve(root, 'packages'), target) && ![role, ...allowedTS[role]].some(allowed => within(directory(allowed), target))) throw new Error(`${role}: module alias crosses role boundary: ${specifier}`);
          if (role === 'protocol' && production && within(directory('projection'), target) && !typeOnly) throw new Error('Protocol alias may only import projection wire types');
        }
      };
      const visit = node => {
        if ((ts.isImportDeclaration(node) || ts.isExportDeclaration(node)) && node.moduleSpecifier && ts.isStringLiteralLike(node.moduleSpecifier)) {
          const clause = node.importClause;
          const bindings = clause?.namedBindings;
          const typeOnly = node.isTypeOnly || clause?.isTypeOnly || (!clause?.name && bindings && ts.isNamedImports(bindings) && bindings.elements.length > 0 && bindings.elements.every(binding => binding.isTypeOnly)) || (node.exportClause && ts.isNamedExports(node.exportClause) && node.exportClause.elements.length > 0 && node.exportClause.elements.every(binding => binding.isTypeOnly));
          inspect(node.moduleSpecifier.text, Boolean(typeOnly));
        } else if (ts.isImportTypeNode(node) && ts.isLiteralTypeNode(node.argument) && ts.isStringLiteralLike(node.argument.literal)) {
          inspect(node.argument.literal.text, true);
        } else if (ts.isCallExpression(node)) {
          const callee = node.expression;
          const moduleCall = callee.kind === ts.SyntaxKind.ImportKeyword || (ts.isIdentifier(callee) && callee.text === 'require') || (ts.isPropertyAccessExpression(callee) && ts.isIdentifier(callee.expression) && ((callee.expression.text === 'require' && callee.name.text === 'resolve') || (callee.expression.text === 'jest' && ['mock', 'doMock', 'requireActual', 'requireMock'].includes(callee.name.text))));
          if (moduleCall) {
            const argument = node.arguments[0];
            if (!argument || !ts.isStringLiteralLike(argument)) throw new Error(`${file}: dynamic module path cannot be audited`);
            inspect(argument.text);
          }
        }
        if (ts.isStringLiteralLike(node) && forbidden.test(node.text)) throw new Error(`${file}: application module/config literal ${node.text}`);
        ts.forEachChild(node, visit);
      };
      visit(source);
      count++;
    }
  }
  return count;
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  const flags = process.argv.slice(2);
  if (!flags.length || flags.some(flag => !['--rust', '--typescript'].includes(flag))) throw new Error('choose --rust and/or --typescript');
  const report = { rustPackages: flags.includes('--rust') ? checkRust() : 0, typeScriptFiles: flags.includes('--typescript') ? await checkTypeScript() : 0, executedTests: 0 };
  console.log(JSON.stringify(report));
}
