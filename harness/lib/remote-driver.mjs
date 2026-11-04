/**
 * Remote surface: two clients, one case.
 *
 * Opens the desktop client and the mobile client at the same time and asserts
 * they render the same remote conversation (title, transcript markers, model).
 * Both use the shot mocks' remote datasets; this verifies the two renderings
 * stay consistent, not a live NATS link or relay. A true two-host run needs a
 * relay harness and is documented as the next step.
 */
import { mkdirSync, writeFileSync, appendFileSync } from "node:fs";
import { join } from "node:path";
import { CdpPage } from "./cdp.mjs";
import { muxVideo } from "./video.mjs";

const sleep = ms => new Promise(done => setTimeout(done, ms));
const READY = "!!document.querySelector('[aria-label]') && document.body.innerText.trim().length > 20";

async function launchMobile() {
  const page = await CdpPage.launch({
    width: 390,
    height: 844,
    scale: 2,
    mobile: true,
    touch: true,
  });
  await page.send("Emulation.setLocaleOverride", { locale: "zh-CN" });
  await page.send("Emulation.setUserAgentOverride", {
    userAgent: "Mozilla/5.0 (iPhone; CPU iPhone OS 17_0 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.0 Mobile/15E148 Safari/604.1",
    acceptLanguage: "zh-CN",
  });
  return page;
}

async function runStep(page, step) {
  const action = Object.keys(step).find(key => !["note", "surface", "timeout"].includes(key)) ?? "note";
  if (action === "wait")
    await sleep(Number(step.wait));
  else if (action === "click")
    await page.click(step.click);
  else if (action === "clickText")
    await page.click({ text: step.clickText });
  else if (action === "type")
    await page.type(step.type);
  else if (action === "press")
    await page.press(step.press);
  else if (action === "assertText") {
    const ok = await page.evaluate(`document.body.innerText.includes(${JSON.stringify(step.assertText)})`);
    if (!ok)
      throw new Error(`text not found: ${step.assertText}`);
  }
  else if (action === "assert") {
    const ok = await page.evaluate(step.assert);
    if (!ok)
      throw new Error(`assertion failed: ${step.assert}`);
  }
  else if (action === "shot")
    return { shot: step.shot };
  else if (action !== "note")
    throw new Error(`unknown remote step: ${action}`);
  return {};
}

export async function runRemoteCases({ cases, runDir, desktopUrl, mobileUrl }) {
  const results = [];
  for (const testCase of cases) {
    const caseDir = join(runDir, testCase.id);
    mkdirSync(join(caseDir, "shots"), { recursive: true });
    const startedAt = Date.now();
    const timeline = [];
    const checks = [];
    let status = "pass";
    let fatal = null;
    const desktop = await CdpPage.launch({ width: 1280, height: 860, scale: 2 });
    const mobile = await launchMobile();
    const pages = { desktop, mobile };
    try {
      await desktop.navigate(`${desktopUrl}${testCase.pages?.desktop ?? ""}`, { ready: READY, settle: 1200 });
      await mobile.navigate(`${mobileUrl}${testCase.pages?.mobile ?? ""}`, { ready: READY, settle: 1600 });
      for (const [index, step] of testCase.steps.entries()) {
        const surface = step.surface ?? "desktop";
        const page = pages[surface];
        if (!page)
          throw new Error(`unknown surface ${surface}`);
        const entry = { index: index + 1, surface, action: Object.keys(step).find(key => !["note", "surface", "timeout"].includes(key)), note: step.note ?? null };
        const started = Date.now();
        try {
          const outcome = await runStep(page, step);
          if (outcome.shot) {
            const file = join(caseDir, "shots", `${String(index + 1).padStart(2, "0")}-${surface}-${outcome.shot}.png`);
            await page.screenshot(file);
            entry.shot = file;
          }
          if (step.assertText || step.assert)
            checks.push({ kind: "assert", step: index + 1, message: step.assertText ?? step.assert, ok: true });
          entry.ok = true;
        }
        catch (error) {
          entry.ok = false;
          entry.error = String(error?.message ?? error);
          status = "fail";
          fatal ??= entry.error;
          checks.push({ kind: "assert", step: index + 1, message: step.assertText ?? step.assert ?? entry.action, ok: false });
          try {
            entry.shot = join(caseDir, "shots", `failure-${index + 1}-${surface}.png`);
            await page.screenshot(entry.shot);
          }
          catch {}
        }
        entry.ms = Date.now() - started;
        timeline.push(entry);
        appendFileSync(join(caseDir, "timeline.jsonl"), `${JSON.stringify(entry)}\n`);
        if (!entry.ok && !step.optional)
          break;
      }
    }
    finally {
      await desktop.close();
      await mobile.close();
    }
    const result = {
      id: testCase.id,
      title: testCase.title,
      surface: "remote",
      area: testCase.area ?? null,
      priority: testCase.priority ?? null,
      description: testCase.description ?? null,
      preconditions: testCase.preconditions ?? null,
      expectations: testCase.expectations ?? [],
      judge: testCase.judge ?? null,
      status,
      fatal,
      startedAt: new Date(startedAt).toISOString(),
      durationMs: Date.now() - startedAt,
      steps: timeline,
      checks,
      console: { errors: [], warnings: [], all: [] },
      exceptions: [],
      network: { failures: [], count: 0 },
      server: { commands: [], fallbacks: [], errors: [] },
      metrics: [],
      artifacts: { dir: caseDir, shots: timeline.filter(entry => entry.shot).map(entry => entry.shot), timeline: join(caseDir, "timeline.jsonl") },
    };
    writeFileSync(join(caseDir, "result.json"), JSON.stringify(result, null, 2));
    results.push(result);
  }
  return results;
}
