import { pathToFileURL } from 'node:url';
import { resolve } from 'node:path';

// The aggregation gate.
//
// ci.yml fans out into reusable workflows, so GitHub reports one job per row,
// named "<caller job name> / <matrix row name>" (callers in ci.yml, rows in
// rust-ci.yml / typescript-ci.yml), plus one standalone job for each native
// suite. This gate decides one verdict per planned caller group.
//
// The verdict comes from the group's OWN jobs and, for every row, from its NEWEST
// COMPLETED ATTEMPT — not from `needs.<job>.result`. That is the bug this gate exists
// for: after a rerun (`re-run failed jobs`) or a close/reopen of the pull request
// GitHub leaves the attempt it replaced behind as `abandoned`, and `needs` then
// reports `abandoned` for a caller whose rows in fact all passed in the attempt
// the run ended with. Judging each row's attempt history makes a superseded
// attempt (`abandoned`, or `cancelled` when a newer attempt replaced it) harmless,
// while
//
//   * `failure` / `timed_out` are never exempted — they stay red whenever they are
//     the newest completed attempt;
//   * cancelling a whole CI run still fails the gate, because the cancelled row is
//     the newest attempt and a group with no completed attempt at all is red;
//   * only the rows a group actually ran are judged, so a rerun that re-created a
//     subset of the matrix cannot fail on the rows it never ran.
//
// Scoping to this run's own jobs (rather than every check run on the commit) also
// keeps an unrelated run of the same commit — a workflow_dispatch full run, say —
// from deciding this run's verdict.

// Caller job names from ci.yml; `needs` names the planner output that schedules
// the group. Renaming a caller there breaks the guard test in
// scripts/tests/ci-plan.test.mjs instead of silently turning this gate red.
export const GROUPS = [
  { key: 'rust-linux-test', prefix: 'Rust Linux test', need: 'rust' },
  { key: 'rust-windows-test', prefix: 'Rust Windows test', need: 'rust' },
  { key: 'rust-macos-test', prefix: 'Rust macOS test', need: 'rust' },
  { key: 'rust-check', prefix: 'Rust check', need: 'rust' },
  { key: 'typescript-test', prefix: 'TypeScript test', need: 'typescript' },
  { key: 'typescript-check', prefix: 'TypeScript check', need: 'typescript' },
  { key: 'android-test', prefix: 'Android test', need: 'android' },
  { key: 'ios-test', prefix: 'iOS test', need: 'ios' },
];

const INVALID = 'Invalid affected CI outputs';

function plannedProjects(value) {
  let parsed;
  try {
    parsed = JSON.parse(value);
  } catch {
    throw new Error(INVALID);
  }
  if (!Array.isArray(parsed)) throw new Error(INVALID);
  return parsed;
}

// The groups this plan requires, using the planner's own notion of "selected": a
// language with no selected project runs none of its groups.
export function requiredGroups(outputs) {
  if (!outputs || typeof outputs !== 'object') throw new Error(INVALID);
  const selected = {
    rust: plannedProjects(outputs.rust).length > 0,
    typescript: plannedProjects(outputs.typescript).length > 0,
  };
  const { android, ios } = outputs;
  if (!['true', 'false'].includes(android) || !['true', 'false'].includes(ios)) throw new Error(INVALID);
  return GROUPS.filter(group => selected[group.need] ?? outputs[group.need] === 'true');
}

// A job that never finished (still queued/running) carries no verdict.
function finished(job) {
  return job && typeof job.name === 'string' && job.conclusion != null;
}

function isNewer(candidate, current) {
  const attempt = Number(candidate.attempt ?? 0) - Number(current.attempt ?? 0);
  if (attempt !== 0) return attempt > 0;
  return Number(candidate.id ?? 0) > Number(current.id ?? 0);
}

// name -> newest completed attempt, plus the attempts it superseded (diagnostics).
function newestAttempts(jobs) {
  const newest = new Map();
  const superseded = new Map();
  for (const job of Array.isArray(jobs) ? jobs : []) {
    if (!finished(job)) continue;
    const current = newest.get(job.name);
    if (!current) {
      newest.set(job.name, job);
      continue;
    }
    if (isNewer(job, current)) {
      superseded.set(job.name, [...(superseded.get(job.name) ?? []), current]);
      newest.set(job.name, job);
    } else {
      superseded.set(job.name, [...(superseded.get(job.name) ?? []), job]);
    }
  }
  return { newest, superseded };
}

// A native suite is its own job; a reusable workflow contributes "<caller> / <row>".
const belongs = (job, prefix) => job.name === prefix || job.name.startsWith(`${prefix} / `);

export function checkResult({ needs, jobs } = {}) {
  if (needs?.affected?.result !== 'success') throw new Error('Affected CI planning failed');
  const groups = requiredGroups(needs.affected.outputs);
  const runJobs = (Array.isArray(jobs) ? jobs : []).filter(job => finished(job));
  const problems = [];
  for (const group of groups) {
    const { newest, superseded } = newestAttempts(runJobs.filter(job => belongs(job, group.prefix)));
    if (!newest.size) {
      // Only reachable when GitHub did not list a single row of this group in the
      // run, which a rerun can do; the caller job is then the remaining evidence.
      // `abandoned`/`cancelled`/`skipped` never pass here, because a caller only
      // concludes `success` after its rows did.
      const caller = needs?.[group.key]?.result;
      if (caller !== 'success') problems.push(`${group.prefix}: ${caller ?? 'missing'}`);
      continue;
    }
    for (const [name, job] of newest) {
      if (job.conclusion !== 'success') problems.push(`${name}: ${job.conclusion}`);
    }
    for (const [name, ignored] of superseded) {
      for (const attempt of ignored) {
        console.log(`Superseded attempt, not judged: ${name}: ${attempt.conclusion}`
          + ` (attempt ${attempt.attempt}); newest is ${newest.get(name)?.conclusion}`);
      }
    }
  }
  if (problems.length) throw new Error(`CI result failed: ${problems.join(', ')}`);
  console.log(`All planned CI checks passed (${groups.length} groups).`);
}

const API = (process.env.GITHUB_API_URL ?? 'https://api.github.com').replace(/\/+$/, '');

async function get(path) {
  const token = process.env.GITHUB_TOKEN;
  if (!token) throw new Error("GITHUB_TOKEN is required to read this run's jobs");
  for (let attempt = 1; ; attempt += 1) {
    const response = await fetch(`${API}${path}`, {
      headers: {
        authorization: `Bearer ${token}`,
        accept: 'application/vnd.github+json',
        'x-github-api-version': '2022-11-28',
        'user-agent': 'future-os-ci-result',
      },
    });
    if (response.ok) return response.json();
    const detail = `${response.status} ${response.statusText}: ${(await response.text()).slice(0, 300)}`;
    const transient = response.status >= 500 || response.status === 429;
    if (!transient || attempt >= 3) throw new Error(`${path} failed: ${detail}`);
    await new Promise(done => setTimeout(done, 1000 * attempt));
  }
}

// Every attempt of every job in this run (`filter=all`), one entry per attempt.
async function runJobs() {
  const repository = process.env.GITHUB_REPOSITORY;
  const runId = process.env.GITHUB_RUN_ID;
  if (!repository || !runId) throw new Error('GITHUB_REPOSITORY and GITHUB_RUN_ID are required');
  const jobs = [];
  for (let page = 1; ; page += 1) {
    const body = await get(`/repos/${repository}/actions/runs/${runId}/jobs?filter=all&per_page=100&page=${page}`);
    const batch = body?.jobs ?? [];
    jobs.push(...batch);
    if (!batch.length || jobs.length >= (body?.total_count ?? 0)) break;
  }
  return jobs.map(job => ({ id: job.id, name: job.name, conclusion: job.conclusion, attempt: job.run_attempt }));
}

if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  checkResult({ needs: JSON.parse(process.env.CI_NEEDS), jobs: await runJobs() });
}
