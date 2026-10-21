import { pathToFileURL } from 'node:url';
import { resolve } from 'node:path';

export function checkResult(needs) {
  if (needs.affected?.result !== 'success') throw new Error('Affected CI planning failed');
  const { rust, typescript, android, ios } = needs.affected.outputs;
  const rustProjects = JSON.parse(rust), tsProjects = JSON.parse(typescript);
  if (!Array.isArray(rustProjects) || !Array.isArray(tsProjects)
    || !['true', 'false'].includes(android) || !['true', 'false'].includes(ios)) {
    throw new Error('Invalid affected CI outputs');
  }
  const rustRequired = rustProjects.length > 0;
  const tsRequired = tsProjects.length > 0;
  const required = {
    'rust-linux-test': rustRequired, 'rust-windows-test': rustRequired, 'rust-macos-test': rustRequired,
    'rust-check': rustRequired, 'typescript-test': tsRequired, 'typescript-check': tsRequired,
    'android-test': android === 'true', 'ios-test': ios === 'true',
  };
  for (const [name, expected] of Object.entries(required)) {
    const result = needs[name]?.result;
    if (result !== 'success' && !(result === 'skipped' && !expected)) {
      throw new Error(`${name}: ${result}; required=${expected}`);
    }
  }
  console.log('All planned CI checks passed.');
}

if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  checkResult(JSON.parse(process.env.CI_NEEDS));
}
