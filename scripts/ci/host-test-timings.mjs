import { appendFileSync, readFileSync } from 'node:fs';

const path = process.argv[2];
if (!path) throw new Error('Expected the Host fixture timing JSONL path');
const records = readFileSync(path, 'utf8').trim().split(/\r?\n/).filter(Boolean).map(JSON.parse);
if (!records.length) throw new Error('No Host fixture timings were recorded');
for (const record of records) {
  if (record.schema !== 1 || typeof record.fixture !== 'string'
    || typeof record.test !== 'string' || !record.phases_ms
    || !Number.isFinite(record.wall_ms) || record.wall_ms < 0
    || Object.values(record.phases_ms).some(value => !Number.isFinite(value) || value < 0)) {
    throw new Error('Invalid Host fixture timing record');
  }
}
const phases = ['lock_wait', 'pre_cleanup', 'setup', 'home_setup', 'open_store', 'store_setup', 'body', 'cleanup', 'restore'];
const seconds = value => (value / 1_000).toFixed(2);
const escape = value => String(value).replaceAll('|', '\\|').replace(/[\r\n]/g, ' ');
const lines = [
  '## Host fixture timing',
  '',
  `${records.length} completed fixture scopes across ${new Set(records.map(row => row.pid)).size} process(es).`,
  '',
  'Times are cumulative seconds per fixture kind, not job wall time. Nested Home/Mock guards overlap; concurrent lock waits overlap too. Do not add these rows together.',
  'Body measures the lifetime after fixture setup, including nested fixtures and local teardown, rather than a standalone libtest duration. Tests without these guards are not measured.',
  '',
  `| Fixture | Scopes | ${phases.join(' | ')} |`,
  `|---|---:|${phases.map(() => '---:').join('|')}|`,
];
for (const fixture of [...new Set(records.map(row => row.fixture))].sort()) {
  const rows = records.filter(row => row.fixture === fixture);
  lines.push(`| ${escape(fixture)} | ${rows.length} | ${phases.map(phase =>
    seconds(rows.reduce((sum, row) => sum + (row.phases_ms[phase] ?? 0), 0))).join(' | ')} |`);
}
lines.push('', '### Module / fixture breakdown', '',
  'Module is the first Rust test path component. Each fixture kind is reported separately because nested scopes overlap.', '',
  `| Module | Fixture | Scopes | ${phases.join(' | ')} |`,
  `|---|---|---:|${phases.map(() => '---:').join('|')}|`);
const groups = new Map();
for (const row of records) {
  const module = row.test.split('::')[0];
  const key = JSON.stringify([module, row.fixture]);
  if (!groups.has(key)) groups.set(key, { module, fixture: row.fixture, rows: [] });
  groups.get(key).rows.push(row);
}
for (const { module, fixture, rows } of [...groups.values()].sort((a, b) =>
  a.module.localeCompare(b.module) || a.fixture.localeCompare(b.fixture))) {
  lines.push(`| ${escape(module)} | ${escape(fixture)} | ${rows.length} | ${phases.map(phase =>
    seconds(rows.reduce((sum, row) => sum + (row.phases_ms[phase] ?? 0), 0))).join(' | ')} |`);
}
for (const phase of ['lock_wait', 'store_setup', 'body', 'cleanup', 'restore']) {
  lines.push('', `### Slowest fixture scopes: ${phase}`, '', '| Test | Fixture / label | Seconds |', '|---|---|---:|');
  for (const row of [...records].sort((a, b) =>
    (b.phases_ms[phase] ?? 0) - (a.phases_ms[phase] ?? 0)).slice(0, 15)) {
    if (!(row.phases_ms[phase] > 0)) continue;
    lines.push(`| ${escape(row.test)} | ${escape(row.fixture)} / ${escape(row.label)} | ${seconds(row.phases_ms[phase])} |`);
  }
}
const summary = `${lines.join('\n')}\n`;
console.log(summary);
if (process.env.GITHUB_STEP_SUMMARY) appendFileSync(process.env.GITHUB_STEP_SUMMARY, summary);
