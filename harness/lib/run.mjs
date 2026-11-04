/**
 * Case execution: one DB copy, one fixture server, one page, one video.
 *
 * A case is data (`cases/*.json`): a name, the human-readable preconditions,
 * the ordered actions a tester would take, and the expectations. This module
 * turns that into an evidence bundle — screenshots after every step, a
 * screencast, console/network/performance channels, the fixture server's
 * command log, and DB queries — so a later analysis step can reason about what
 * happened without re-running anything.
 */
import { mkdirSync, writeFileSync, appendFileSync, rmSync, cpSync } from "node:fs";
import { join, resolve } from "node:path";
import { CdpPage } from "./cdp.mjs";
import { copyDatabase, BASE_DB } from "./data.mjs";
import { createFixtureServer } from "./fixture-server.mjs";
import { muxVideo } from "./video.mjs";

const sleep = ms => new Promise(done => setTimeout(done, ms));

export const READY = "!!document.querySelector('[aria-label]') && document.body.innerText.trim().length > 20";

function slug(text) {
  return String(text).replace(/[^\w\u4e00-\u9fa5-]+/g, "-").replace(/^-+|-+$/g, "").slice(0, 40) || "shot";
}

function stepAction(step) {
  return Object.keys(step).find(key => !["note", "capture", "optional", "label", "timeout"].includes(key)) ?? "note";
}

/** `{{name}}` from a previous `db` step, so a case can act on real row titles. */
function substitute(value, variables) {
  if (typeof value === "string")
    return value.replace(/\{\{(\w+)\}\}/g, (match, name) => (name in variables ? String(variables[name]) : match));
  if (Array.isArray(value))
    return value.map(item => substitute(item, variables));
  if (value && typeof value === "object")
    return Object.fromEntries(Object.entries(value).map(([key, item]) => [key, substitute(item, variables)]));
  return value;
}

async function waitFor(page, spec, timeout) {
  const deadline = Date.now() + timeout;
  while (Date.now() < deadline) {
    let ok = false;
    try {
      if (spec.expr)
        ok = !!(await page.evaluate(spec.expr));
      else if (spec.selector)
        ok = !!(await page.evaluate(`!!document.querySelector(${JSON.stringify(spec.selector)})`));
      else if (spec.text)
        ok = !!(await page.evaluate(`document.body.innerText.includes(${JSON.stringify(spec.text)})`));
      else if (spec.absent)
        ok = !(await page.evaluate(`document.body.innerText.includes(${JSON.stringify(spec.absent)})`));
    }
    catch {
      ok = false;
    }
    if (ok)
      return true;
    await sleep(150);
  }
  return false;
}

async function queryFixture(fixture, sql) {
  const response = await fetch(`${fixture.url}/query`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ sql }),
  });
  const payload = await response.json();
  if (payload.error)
    throw new Error(payload.error);
  return payload.rows;
}

/**
 * Run one case. Returns the result record; artifacts are written beside it.
 */
export async function runCase({ page, testCase, runDir, modelsPath, viewport, recordVideo = true }) {
  const caseDir = join(runDir, slug(testCase.id));
  mkdirSync(caseDir, { recursive: true });
  const shotsDir = join(caseDir, "shots");
  const framesDir = join(caseDir, "video");
  mkdirSync(shotsDir, { recursive: true });

  const dbPath = copyDatabase(join(caseDir, "app.db"));
  const serverLogPath = join(caseDir, "server.jsonl");
  const fixture = await createFixtureServer({
    dbPath: dbPath || BASE_DB,
    modelsPath,
    logPath: serverLogPath,
  });

  const startedAt = Date.now();
  const timelinePath = join(caseDir, "timeline.jsonl");
  const timeline = [];
  const checks = [];
  const observations = [];
  const metrics = [];
  const dbValues = new Map();
  const variables = {};
  let status = "pass";
  let fatal = null;
  let stepsEndedAt = null;

  page.consoleMessages = [];
  page.exceptions = [];
  page.networkFailures = [];
  page.networkRequests = new Map();
  page.onEvent = event => {
    observations.push({ ...event, step: timeline.length });
    if (event.channel === "console" && event.type === "error")
      appendFileSync(join(caseDir, "console-errors.log"), `${JSON.stringify(event)}\n`);
  };

  const note = (entry) => {
    const record = { ts: Date.now(), step: timeline.length, ...entry };
    timeline.push(record);
    appendFileSync(timelinePath, `${JSON.stringify(record)}\n`);
    return record;
  };

  const capture = async (name) => {
    const file = join(shotsDir, `${String(timeline.length).padStart(2, "0")}-${slug(name)}.png`);
    await page.screenshot(file);
    return file;
  };

  const params = new URLSearchParams({
    fixture: fixture.url,
    lang: testCase.lang ?? "zh",
    ...(testCase.theme ? { theme: testCase.theme } : {}),
  });
  const url = `${testCase.baseUrl ?? "http://localhost:5299/shot/"}?${params.toString()}`;

  // The folder picker in the workspace form is a native dialog, mocked to a
  // canned path. Give each case a directory that exists, so "create workspace"
  // picks something this run owns rather than a demo path.
  const pickDir = join(caseDir, "workspace-pick");
  mkdirSync(pickDir, { recursive: true });
  const pickFile = join(pickDir, "attachment.txt");
  writeFileSync(pickFile, "harness attachment\n");
  const injection = await page.send("Page.addScriptToEvaluateOnNewDocument", {
    source: `window.__harnessPickPaths = ${JSON.stringify({ directory: pickDir, file: pickFile })};`,
  });
  // Each case starts from the same UI state: the app remembers its last
  // section/sidebar width in localStorage, so a previous case's navigation
  // would otherwise decide which rail is on screen for this one.
  await page.send("Storage.clearDataForOrigin", {
    origin: new URL(url).origin,
    storageTypes: "local_storage,session_storage",
  }).catch(() => {});

  try {
    if (recordVideo)
      await page.startScreencast(framesDir);
    await page.navigate(url, { ready: READY, settle: 700 });

    const baseline = await page.metrics();
    metrics.push({ at: "baseline", ts: Date.now(), ...baseline, process: page.processStats() });

    for (const [index, rawStep] of testCase.steps.entries()) {
      if (rawStep.note)
        note({ action: "note", note: rawStep.note });
      const step = substitute(rawStep, variables);
      const action = stepAction(step);
      if (action === "note")
        continue;
      const started = Date.now();
      const entry = { action, index: index + 1, note: step.note ?? null, target: step[action] ?? null };
      try {
        if (action === "wait" || action === "sleep") {
          await sleep(Number(step[action]));
        }
        else if (action === "waitFor") {
          const ok = await waitFor(page, step[action], Number(step.timeout ?? 10_000));
          if (!ok)
            throw new Error(`waitFor timed out: ${JSON.stringify(step[action])}`);
        }
        else if (action === "click") {
          entry.spot = await page.click(step.click, { label: step.label ?? step.note });
        }
        else if (action === "clickText") {
          entry.spot = await page.click({ text: step.clickText }, { label: step.clickText });
        }
        else if (action === "hover") {
          entry.spot = await page.hover(step.hover);
        }
        else if (action === "type") {
          if (step.into)
            await page.click(step.into, { label: `type into ${step.into}` });
          await page.type(step.type);
        }
        else if (action === "press") {
          await page.press(step.press);
        }
        else if (action === "scroll") {
          await page.scroll(step.scroll[0] ?? 0, step.scroll[1] ?? 0);
        }
        else if (action === "eval") {
          entry.value = await page.evaluate(step.eval);
        }
        else if (action === "assert") {
          const spec = typeof step.assert === "string" ? { expr: step.assert } : step.assert;
          const value = await page.evaluate(spec.expr);
          const ok = Boolean(value);
          checks.push({
            kind: "assert",
            step: index + 1,
            message: spec.message ?? spec.expr,
            ok,
            value: value === undefined ? null : value,
          });
          if (!ok)
            throw new Error(`assertion failed: ${spec.message ?? spec.expr}`);
        }
        else if (action === "expectText") {
          const text = step.expectText;
          const ok = await page.evaluate(`document.body.innerText.includes(${JSON.stringify(text)})`);
          checks.push({ kind: "expectText", step: index + 1, message: `屏幕上出现「${text}」`, ok });
          if (!ok)
            throw new Error(`expected text not found: ${text}`);
        }
        else if (action === "expectAbsent") {
          const text = step.expectAbsent;
          const ok = !(await page.evaluate(`document.body.innerText.includes(${JSON.stringify(text)})`));
          checks.push({ kind: "expectAbsent", step: index + 1, message: `屏幕上不出现「${text}」`, ok });
          if (!ok)
            throw new Error(`text should be absent: ${text}`);
        }
        else if (action === "db") {
          const spec = typeof step.db === "string" ? { sql: step.db } : step.db;
          const rows = await queryFixture(fixture, spec.sql);
          entry.rows = rows;
          if (spec.as && rows.length)
            variables[spec.as] = Object.values(rows[0])[0];
          await page.evaluate(`window.__harnessLastDb = ${JSON.stringify(rows)}`);
          const scalar = rows.length === 1 && Object.keys(rows[0]).length === 1 ? Object.values(rows[0])[0] : rows;
          const dbKey = `db:${spec.sql}`;
          const previousValue = dbValues.get(dbKey);
          if (typeof scalar === "number" || /^\d+$/.test(String(scalar)))
            dbValues.set(dbKey, Number(scalar));
          if (spec.delta !== undefined) {
            const actual = previousValue === undefined ? null : Number(scalar) - previousValue;
            const ok = actual === spec.delta;
            checks.push({ kind: "dbDelta", step: index + 1, message: spec.message ?? spec.sql, ok, expected: spec.delta, actual });
            if (!ok)
              throw new Error(`DB delta failed: ${spec.message ?? spec.sql} (expected ${spec.delta}, got ${actual})`);
          }
          else if (spec.expect !== undefined) {
            const ok = JSON.stringify(scalar) === JSON.stringify(spec.expect) || String(scalar) === String(spec.expect);
            checks.push({
              kind: "db",
              step: index + 1,
              message: spec.message ?? spec.sql,
              ok,
              expected: spec.expect,
              actual: scalar,
            });
            if (!ok)
              throw new Error(`DB expectation failed: ${spec.message ?? spec.sql} (expected ${JSON.stringify(spec.expect)}, got ${JSON.stringify(scalar)})`);
          }
        }
        else if (action === "reload") {
          await page.navigate(url, { ready: READY, settle: 700 });
        }
        else if (action === "server") {
          const spec = typeof step.server === "string" ? { command: step.server } : step.server;
          const matches = fixture.log.filter((row) => {
            if (row.command !== spec.command)
              return false;
            if (!spec.args)
              return true;
            return Object.entries(spec.args).every(([key, value]) => {
              const args = row.args ?? {};
              return JSON.stringify(args[key]) === JSON.stringify(value);
            });
          });
          entry.matches = matches.length;
          if (spec.expect !== undefined) {
            const ok = matches.length === spec.expect;
            checks.push({
              kind: "server",
              step: index + 1,
              message: spec.message ?? `IPC ${spec.command} 调用 ${spec.expect} 次`,
              ok,
              expected: spec.expect,
              actual: matches.length,
            });
            if (!ok)
              throw new Error(`IPC expectation failed: ${spec.command} expected ${spec.expect}, got ${matches.length}`);
          }
        }
        else if (action === "shot") {
          entry.shot = await capture(step.shot);
        }
        else {
          throw new Error(`unknown step action: ${action}`);
        }
        entry.ok = true;
      }
      catch (error) {
        entry.ok = false;
        entry.error = String(error?.message ?? error);
        status = "fail";
        fatal ??= entry.error;
        try {
          entry.failureShot = join(shotsDir, `failure-${index + 1}.png`);
          await page.screenshot(entry.failureShot);
        }
        catch {}
        note({ ...entry, ms: Date.now() - started });
        if (!step.optional && testCase.continueOnFailure !== true)
          break;
      }
      entry.ms = Date.now() - started;
      entry.metrics = await page.metrics().catch(() => null);
      entry.process = page.processStats();
      if (entry.metrics)
        metrics.push({ at: `step-${index + 1}`, ts: Date.now(), ...entry.metrics, process: entry.process });

      // Evidence after every action, not only where a case asked for it: the
      // interesting failure is often one step before the assertion.
      if (!entry.shot && step.capture !== false && action !== "wait" && action !== "sleep")
        entry.shot = await capture(`${index + 1}-${action}`);

      note(entry);
    }

    const finalMetrics = await page.metrics();
    metrics.push({ at: "final", ts: Date.now(), ...finalMetrics, process: page.processStats() });
    if (status === "pass")
      await capture("final");
    stepsEndedAt = Date.now();
  }
  catch (error) {
    status = "error";
    fatal = String(error?.message ?? error);
    try {
      await page.screenshot(join(shotsDir, "error.png"));
    }
    catch {}
  }
  finally {
    page.onEvent = null;
    stepsEndedAt ??= Date.now();
    const frames = await page.stopScreencast().catch(() => []);
    // Let the last frame land before the page leaves; otherwise the filmstrip's
    // tail is the teardown (a black frame), which a vision model can only read
    // as "the product went blank".
    await sleep(300);
    if (injection?.identifier)
      await page.send("Page.removeScriptToEvaluateOnNewDocument", { identifier: injection.identifier }).catch(() => {});
    // Leave the page before the fixture server goes away: a poll tick that
    // lands after `server.close()` records ERR_CONNECTION_REFUSED and looks
    // like a product defect in the report.
    await page.send("Page.navigate", { url: "about:blank" }).catch(() => {});
    if (recordVideo && frames.length) {
      const video = muxVideo(framesDir, frames, join(caseDir, "video.mp4"));
      if (!video.ok)
        note({ action: "video", ok: false, error: video.reason });
    }
    if (!recordVideo)
      rmSync(framesDir, { recursive: true, force: true });
    await fixture.close();
  }

  const durationMs = Date.now() - startedAt;
  const serverLog = fixture.log;
  const result = {
    id: testCase.id,
    title: testCase.title,
    area: testCase.area ?? null,
    priority: testCase.priority ?? null,
    description: testCase.description ?? null,
    preconditions: testCase.preconditions ?? null,
    expectations: testCase.expectations ?? [],
    status,
    fatal,
    startedAt: new Date(startedAt).toISOString(),
    stepsEndedAt: stepsEndedAt ? new Date(stepsEndedAt).toISOString() : null,
    durationMs,
    steps: timeline,
    checks,
    metrics,
    observations,
    console: {
      errors: page.consoleMessages.filter(m => m.type === "error"),
      warnings: page.consoleMessages.filter(m => m.type === "warning"),
      all: page.consoleMessages,
    },
    exceptions: page.exceptions,
    network: {
      failures: page.networkFailures,
      count: page.networkRequests.size,
    },
    server: {
      commands: serverLog,
      fallbacks: serverLog.filter(entry => !entry.handled),
      errors: serverLog.filter(entry => entry.error),
    },
    artifacts: {
      dir: caseDir,
      timeline: timelinePath,
      serverLog: serverLogPath,
      video: join(caseDir, "video.mp4"),
      shots: timeline.filter(entry => entry.shot).map(entry => entry.shot),
    },
  };
  writeFileSync(join(caseDir, "result.json"), JSON.stringify(result, null, 2));
  return result;
}

/** Copy a case's artifacts into a neighbouring folder for sharing. */
export function bundleCase(sourceDir, targetDir) {
  mkdirSync(targetDir, { recursive: true });
  cpSync(sourceDir, targetDir, { recursive: true });
  return resolve(targetDir);
}
