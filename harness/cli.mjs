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
import { ensureDevServer, ensureMobileServer, ensureAssetServer, ensureTerminalServer, DEFAULT_URL } from "./lib/dev-server.mjs";
import { runCase } from "./lib/run.mjs";
import { runCliCase } from "./lib/cli-driver.mjs";
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

    const runId = arg("name") ?? stamp();
    const runDir = join(RUNS_DIR, runId);
    mkdirSync(runDir, { recursive: true });
    const viewport = { width: Number(arg("w", 1280)), height: Number(arg("h", 860)), scale: Number(arg("scale", 2)) };
    const results = [];
    const closers = [];

    const record = (result) => {
      results.push(result);
      const checks = result.checks ?? [];
      const passed = checks.filter(check => check.ok).length;
      console.log(`  ${result.status.toUpperCase()}  ${checks.length ? `${passed}/${checks.length} checks` : "no explicit checks"}  ${(result.durationMs / 1000).toFixed(1)}s  → ${result.artifacts.dir}`);
      if (result.fatal)
        console.log(`  ↳ ${String(result.fatal).split("\n")[0]}`);
      writeFileSync(join(runDir, "run.json"), JSON.stringify({
        id: runId,
        startedAt: results[0]?.startedAt,
        surfaces: [...new Set(results.map(item => item.surface ?? "desktop"))],
        cases: results.map(r => ({
          id: r.id, title: r.title, surface: r.surface ?? "desktop", status: r.status, durationMs: r.durationMs,
          checks: (r.checks ?? []).length, fatal: r.fatal, dir: r.artifacts.dir,
        })),
      }, null, 2));
    };

    const bySurface = surface => cases.filter(testCase => (testCase.surface ?? "desktop") === surface);
    try {
      const desktop = bySurface("desktop");
      if (desktop.length) {
        const server = await ensureDevServer({ reuse: flag("reuse-server") });
        const terminal = await ensureTerminalServer();
        closers.push(() => server.stop());
        closers.push(() => terminal.stop());
        const page = await CdpPage.launch({ ...viewport, userDataDir: join(runDir, "chrome-profile") });
        closers.push(() => page.close());
        for (const testCase of desktop) {
          process.stdout.write(`\n▶ ${testCase.id} — ${testCase.title}\n`);
          record(await runCase({
            page,
            testCase: { ...testCase, baseUrl: server.url },
            runDir,
            modelsPath: MODELS_COPY,
            viewport,
            recordVideo: !flag("no-video"),
          }));
        }
      }

      const mobile = bySurface("mobile");
      if (mobile.length) {
        const server = await ensureMobileServer({ reuse: flag("reuse-server") });
        const assets = await ensureAssetServer();
        closers.push(() => server.stop());
        closers.push(() => assets.stop());
        const page = await CdpPage.launch({
          width: 390,
          height: 844,
          scale: 2,
          mobile: true,
          touch: true,
          userDataDir: join(runDir, "mobile-profile"),
        });
        await page.send("Emulation.setLocaleOverride", { locale: "zh-CN" });
        await page.send("Emulation.setUserAgentOverride", {
          userAgent: "Mozilla/5.0 (iPhone; CPU iPhone OS 17_0 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.0 Mobile/15E148 Safari/604.1",
          acceptLanguage: "zh-CN",
        });
        closers.push(() => page.close());
        for (const testCase of mobile) {
          process.stdout.write(`\n▶ ${testCase.id} — ${testCase.title}\n`);
          record(await runCase({
            page,
            testCase: { ...testCase, surface: "mobile", baseUrl: server.url },
            runDir,
            modelsPath: MODELS_COPY,
            viewport: { width: 390, height: 844, scale: 2 },
            recordVideo: !flag("no-video"),
            fixture: null,
          }));
        }
      }

      for (const testCase of bySurface("cli")) {
        process.stdout.write(`\n▶ ${testCase.id} — ${testCase.title}\n`);
        record(await runCliCase({ testCase, runDir }));
      }

      const tui = bySurface("tui");
      if (tui.length) {
        const { runTuiCases } = await import("./lib/tui-driver.mjs");
        for (const result of await runTuiCases({ cases: tui, runDir }))
          record(result);
      }

      const remote = bySurface("remote");
      if (remote.length) {
        const { runRemoteCases } = await import("./lib/remote-driver.mjs");
        const server = await ensureDevServer({ reuse: flag("reuse-server") });
        const mobileServer = await ensureMobileServer({ reuse: flag("reuse-server") });
        const assets = await ensureAssetServer();
        const terminal = await ensureTerminalServer();
        closers.push(() => terminal.stop());
        closers.push(() => server.stop());
        closers.push(() => mobileServer.stop());
        closers.push(() => assets.stop());
        for (const result of await runRemoteCases({ cases: remote, runDir, desktopUrl: server.url, mobileUrl: mobileServer.url }))
          record(result);
      }
    }
    finally {
      for (const close of closers.reverse()) {
        try {
          await close();
        }
        catch {}
      }
      if (!flag("keep-server")) {
        // pages/servers already stopped above
      }
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
