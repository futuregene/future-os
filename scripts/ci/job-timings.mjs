// Collect GitHub's existing step timestamps after the work, without polling or
// adding timers/logging to the tests themselves. Telemetry never masks CI status.
import { appendFileSync, mkdirSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';

const output = join(process.env.RUNNER_TEMP, 'ci-timings');
mkdirSync(output, { recursive: true });
const metadata = {
  schema: 1, run_id: process.env.GITHUB_RUN_ID, attempt: process.env.GITHUB_RUN_ATTEMPT,
  sha: process.env.GITHUB_SHA, platform: process.env.RUNNER_OS,
  runner_image: process.env.ImageOS, runner_image_version: process.env.ImageVersion,
  job: process.env.CI_TIMING_JOB, collected_at: new Date().toISOString(),
};
try {
  if (!metadata.job) throw new Error('Missing CI_TIMING_JOB');
  const jobs = [];
  for (let page = 1; ; page++) {
    const response = await fetch(`${process.env.GITHUB_API_URL}/repos/${process.env.GITHUB_REPOSITORY}/actions/runs/${metadata.run_id}/attempts/${metadata.attempt}/jobs?per_page=100&page=${page}`, {
      headers: { Authorization: `Bearer ${process.env.GH_TOKEN}`, Accept: 'application/vnd.github+json' },
      signal: AbortSignal.timeout(15_000),
    });
    if (!response.ok) throw new Error(`GitHub job timestamps: HTTP ${response.status}`);
    const batch = (await response.json()).jobs;
    jobs.push(...batch);
    if (batch.length < 100) break;
  }
  const matches = jobs.filter(job => job.name === metadata.job || job.name.endsWith(` / ${metadata.job}`));
  if (matches.length !== 1) throw new Error(`Expected one job named ${metadata.job}, found ${matches.length}`);
  const job = matches[0];
  metadata.job_id = job.id;
  metadata.runner_name = job.runner_name;
  metadata.runner_labels = job.labels;
  metadata.started_at = job.started_at;
  metadata.steps = job.steps.map(step => ({
    name: step.name, number: step.number, status: step.status, conclusion: step.conclusion,
    started_at: step.started_at, completed_at: step.completed_at,
    seconds: step.status === 'completed' && step.started_at && step.completed_at
      ? Math.max(0, (Date.parse(step.completed_at) - Date.parse(step.started_at)) / 1000) : null,
  }));
  const escape = value => String(value).replaceAll('|', '\\|').replace(/[\r\n]/g, ' ');
  const summary = ['## Job timings', '', '| Step | Result | Seconds |', '|---|---|---:|',
    ...metadata.steps.filter(step => step.seconds !== null).map(step =>
      `| ${escape(step.name)} | ${step.conclusion} | ${step.seconds.toFixed(0)} |`), '',
    'GitHub step timestamps have second precision. This snapshot excludes later artifact uploads, post-job cache saves and queue time.', '',
  ].join('\n');
  if (process.env.GITHUB_STEP_SUMMARY) appendFileSync(process.env.GITHUB_STEP_SUMMARY, summary);
} catch (error) {
  metadata.collection_error = error.message;
  console.log(`::warning::Timing metadata unavailable: ${error.message}`);
}
writeFileSync(join(output, 'job.json'), `${JSON.stringify(metadata, null, 2)}\n`);
