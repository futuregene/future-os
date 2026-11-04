#!/usr/bin/env node
/**
 * UI test harness — end to end.
 *
 *   node harness/cli.mjs prepare
 *   node harness/cli.mjs run --case smoke-open-list
 *   node harness/cli.mjs analyze --run harness/runs/<id>
 *   node harness/cli.mjs report  --run harness/runs/<id>
 *   node harness/cli.mjs issues  --run harness/runs/<id>          (dry run)
 *   node harness/cli.mjs list
 *
 * See harness/README.md for what is real and what is substituted.
 */
import { mkdirSync, writeFileSync, readdirSync, readFileSync, existsSync, rmSync } from "node:fs";
import { join, resolve } from "node:path";
import { loadCases, listCases, CASES_DIR } from "./lib/cases.mjs";
import { prepare, BASE_DB, MODELS_COPY, DATA_DIR } from "./lib/data.mjs";
import { ensureDevServer, DEFAULT_URL } from "./lib/dev-server.mjs";
import { runCase } from "./lib/run.mjs";
import { CdpPage } from "./lib/cdp.mjs";
import { analyzeRun } from "./lib/analyze.mjs";
import { writeReport } from "./lib/report.mjs";
import { draftIssues } from "./lib/issues.mjs";

const RUNS_DIR = resolve(import.meta.dirname, "runs");

function arg(name, fallback = null) {
  const index = process.argv.indexOf(`--${name}`);
  return index >= 0 ? (process.argv[index + 1] ?? true) : fallback;
}
/** Repeated flags: `--case a --case b`. */
function args(name) {
  const values = [];
  for (let i = 0; i < process.argv.length; i += 1) {
    if (process.argv[i] === `--${name}` && process.argv[i + 1] && !process.argv[i + 1].startsWith("--"))
      values.push(process.argv[i + 1]);
  }
  return values;
}
function flag(name) {
  return process.argv.includes(`--${name}`);
}

function stamp() {
  const d = new Date();
  const pad = n => String(n).padStart(2, "0");
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}T${pad(d.getHours())}-${pad(d.getMinutes())}-${pad(d.getSeconds())}`;
}

async function main() {
  const command = process.argv[2] ?? "help";

  if (command === "help" || command === "--help") {
    console.log(`usage: node harness/cli.mjs <command>

  prepare                  snapshot the real app.db into harness/.data/base.db
  list [--tag T]           list cases
  run [--case ID|--tag T] [--no-video] [--keep-server] [--analyze none|heuristic|full]
  analyze --run DIR        (re)analyze a finished run
  report --run DIR         write report.md
  issues --run DIR [--create --yes]
  serve                    start the shot dev server and wait
`);
    return;
  }

  if (command === "prepare") {
    const result = await prepare({ force: flag("force") });
    console.log(JSON.stringify(result.meta, null, 2));
    console.log(`${result.reused ? "reused" : "wrote"} ${result.path}`);
    return;
  }

  if (command === "list") {
    for (const testCase of listCases({ tag: arg("tag"), ids: args("case") }))
      console.log(`${testCase.priority ?? "P9"}  ${testCase.id.padEnd(34)} ${testCase.steps} steps  ${testCase.title}`);
    return;
  }

  if (command === "serve") {
    const server = await ensureDevServer();
    console.log(`serving ${server.url} (${server.reused ? "reused" : "started"})`);
    await new Promise(() => {});
    return;
  }

  if (command === "run") {
    await prepare();
    if (!existsSync(BASE_DB))
      throw new Error("no base database; run `node harness/cli.mjs prepare` first");
    const ids = args("case");
    const cases = loadCases({ ids, tag: arg("tag") });
    if (!cases.length)
      throw new Error(`no cases matched (library: ${CASES_DIR})`);

    const server = await ensureDevServer();
    const runId = arg("name") ?? stamp();
    const runDir = join(RUNS_DIR, runId);
    mkdirSync(runDir, { recursive: true });
    const viewport = { width: Number(arg("w", 1280)), height: Number(arg("h", 860)), scale: Number(arg("scale", 2)) };

    const page = await CdpPage.launch({ ...viewport, userDataDir: join(runDir, "chrome-profile") });
    const results = [];
    try {
      for (const testCase of cases) {
        process.stdout.write(`\n▶ ${testCase.id} — ${testCase.title}\n`);
        const result = await runCase({
          page,
          testCase: { ...testCase, baseUrl: `${server.url}` },
          runDir,
          modelsPath: MODELS_COPY,
          viewport,
          recordVideo: !flag("no-video"),
        });
        results.push(result);
        const checks = result.checks;
        const passed = checks.filter(check => check.ok).length;
        console.log(`  ${result.status.toUpperCase()}  ${checks.length ? `${passed}/${checks.length} checks` : "no explicit checks"}  ${(result.durationMs / 1000).toFixed(1)}s  → ${result.artifacts.dir}`);
        if (result.fatal)
          console.log(`  ↳ ${result.fatal.split("\n")[0]}`);
        writeFileSync(join(runDir, "run.json"), JSON.stringify({
          id: runId,
          startedAt: results[0]?.startedAt,
          url: DEFAULT_URL,
          cases: results.map(r => ({
            id: r.id, title: r.title, status: r.status, durationMs: r.durationMs,
            checks: r.checks.length, fatal: r.fatal, dir: r.artifacts.dir,
          })),
        }, null, 2));
      }
    }
    finally {
      await page.close();
      if (!flag("keep-server"))
        await server.stop();
    }

    const analyze = arg("analyze", "heuristic");
    if (analyze !== "none") {
      const analysis = await analyzeRun(runDir, { mode: analyze });
      const report = writeReport(runDir);
      console.log(`\n${analysis.totals.findings} findings (high ${analysis.totals.high}, medium ${analysis.totals.medium}). Report: ${report}`);
    }

    const failed = results.filter(result => result.status !== "pass").length;
    console.log(`\n${results.length - failed}/${results.length} cases passed. Artifacts: ${runDir}`);
    process.exit(failed ? 1 : 0);
    return;
  }

  if (command === "analyze") {
    const runDir = resolve(arg("run") ?? "");
    await analyzeRun(runDir, { mode: arg("mode", "full"), model: arg("model"), only: args("case") });
    return;
  }

  if (command === "report") {
    const runDir = resolve(arg("run") ?? "");
    const path = writeReport(runDir);
    console.log(`wrote ${path}`);
    return;
  }

  if (command === "issues") {
    const runDir = resolve(arg("run") ?? "");
    const result = await draftIssues(runDir, { create: flag("create") && flag("yes"), repo: arg("repo") });
    console.log(result.message);
    return;
  }

  if (command === "clean") {
    const keep = new Set([].concat(args("keep")));
    for (const name of readdirSync(RUNS_DIR)) {
      if (keep.has(name))
        continue;
      rmSync(join(RUNS_DIR, name), { recursive: true, force: true });
      console.log(`removed harness/runs/${name}`);
    }
    return;
  }

  console.error(`unknown command: ${command}`);
  process.exitCode = 1;
}

main().catch((error) => {
  console.error(error?.stack ?? error);
  process.exitCode = 1;
});
