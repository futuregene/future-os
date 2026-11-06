/**
 * TUI surface wrapper: the real `future tui` in a pty, read through a terminal
 * cell model. The Python driver owns the pty and the renderer; this file only
 * marshals cases and folds the result into the run bundle.
 */
import { spawn } from "node:child_process";
import { existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { join, resolve } from "node:path";

const REPO_ROOT = resolve(import.meta.dirname, "..", "..");
const DRIVER = join(import.meta.dirname, "..", "tui", "driver.py");

function binPath() {
  if (process.env.FUTURE_BIN)
    return process.env.FUTURE_BIN;
  const local = join(REPO_ROOT, "target", "debug", "future");
  return existsSync(local) ? local : "future";
}

function agentPath() {
  if (process.env.FUTURE_DEMO_AGENT)
    return process.env.FUTURE_DEMO_AGENT;
  const local = join(REPO_ROOT, "target", "debug", "examples", "demo_agent");
  return local;
}

function runDriver(args, env) {
  return new Promise((resolvePromise, reject) => {
    const child = spawn("python3", [DRIVER, ...args], {
      cwd: REPO_ROOT,
      env,
      stdio: ["ignore", "pipe", "pipe"],
    });
    let stdout = "";
    let stderr = "";
    child.stdout.on("data", chunk => (stdout += chunk));
    child.stderr.on("data", chunk => (stderr += chunk));
    child.on("error", reject);
    child.on("close", (code) => {
      if (code === 0) {
        resolvePromise({ stdout, stderr });
        return;
      }
      reject(new Error(`TUI driver exited ${code}\n${stdout}\n${stderr}`.trim()));
    });
  });
}

export async function runTuiCases({ cases, runDir }) {
  const results = [];
  for (const testCase of cases) {
    const caseDir = join(runDir, testCase.id);
    mkdirSync(caseDir, { recursive: true });
    const stepsPath = join(caseDir, "steps.json");
    writeFileSync(stepsPath, JSON.stringify(testCase, null, 2));
    process.stdout.write(`\n▶ ${testCase.id} — ${testCase.title}\n`);
    const env = {
      ...process.env,
      FUTURE_BIN: binPath(),
      FUTURE_DEMO_AGENT: agentPath(),
    };
    try {
      await runDriver([
        "--steps", stepsPath,
        "--out", caseDir,
        "--id", testCase.id,
        "--cols", String(testCase.cols ?? 130),
        "--rows", String(testCase.rows ?? 40),
      ], env);
      const result = JSON.parse(readFileSync(join(caseDir, "result.json"), "utf8"));
      // The TUI refuses to attach when the agent and the client were stamped with
      // different versions, and every case then fails on its own unrelated
      // assertion ("text not found: …"). Name the cause on the case itself so one
      // root cause does not read as ten defects.
      if (/Incompatible Future Agent|local startup is forbidden/i.test(result.bodyText ?? "")) {
        result.fatal = "TUI 拒绝连接 demo agent：二进制版本不一致（见 preflight 提示），" +
          "重建 future 与 demo_agent 时使用同一个 FUTURE_VERSION";
        result.status = "fail";
      }
      results.push(result);
    }
    catch (error) {
      const result = {
        id: testCase.id,
        title: testCase.title,
        surface: "tui",
        area: testCase.area ?? null,
        priority: testCase.priority ?? null,
        description: testCase.description ?? null,
        preconditions: testCase.preconditions ?? null,
        expectations: testCase.expectations ?? [],
        status: "error",
        fatal: String(error?.message ?? error).slice(0, 2000),
        startedAt: new Date().toISOString(),
        durationMs: 0,
        steps: [],
        checks: [],
        console: { errors: [], warnings: [], all: [] },
        exceptions: [],
        network: { failures: [], count: 0 },
        server: { commands: [], fallbacks: [], errors: [] },
        metrics: [],
        artifacts: { dir: caseDir, shots: [], timeline: join(caseDir, "timeline.jsonl") },
      };
      writeFileSync(join(caseDir, "result.json"), JSON.stringify(result, null, 2));
      results.push(result);
    }
  }
  return results;
}
