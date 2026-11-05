/**
 * CLI surface: run the real `future` binary against an isolated FutureOS home.
 *
 * Every case gets its own copy of the real `app.db` plus the snapshotted agent
 * settings, so a mutating command (`workspace add/archive/restore`) changes only
 * that case. `FUTURE_HOME` also moves the IPC endpoint into the case, and the
 * agent address is pointed at a closed port: an offline command must never
 * reach the user's running agent. A case that needs a live agent opts in with
 * `"agent": "demo"` and gets the scripted `demo_agent` on a port taken for that
 * case alone — never the address of whatever agent the user is running.
 */
import { execFile, spawn } from "node:child_process";
import { existsSync, mkdirSync, readFileSync, rmSync, writeFileSync, appendFileSync } from "node:fs";
import { createConnection, createServer } from "node:net";
import { join, resolve } from "node:path";
import { promisify } from "node:util";
import { DatabaseSync } from "node:sqlite";
import { createCliHome } from "./data.mjs";
import { resolveLimit, readBaseline, baselineKey } from "./perf.mjs";

const execFileAsync = promisify(execFile);
const REPO_ROOT = resolve(import.meta.dirname, "..", "..");
const ANSI = /\x1b\[[0-9;?]*[A-Za-z]/g;

function demoAgentBin() {
  if (process.env.FUTURE_DEMO_AGENT)
    return process.env.FUTURE_DEMO_AGENT;
  return join(REPO_ROOT, "target", "debug", "examples", "demo_agent");
}

/** A port nothing is listening on, taken by asking the OS for one. */
function freePort() {
  return new Promise((resolvePort, reject) => {
    const server = createServer();
    server.unref();
    server.on("error", reject);
    server.listen(0, "127.0.0.1", () => {
      const { port } = server.address();
      server.close(() => resolvePort(port));
    });
  });
}

async function waitForPort(port, timeoutMs = 10_000) {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    const open = await new Promise((resolveOpen) => {
      const socket = createConnection({ host: "127.0.0.1", port });
      socket.once("connect", () => {
        socket.destroy();
        resolveOpen(true);
      });
      socket.once("error", () => resolveOpen(false));
    });
    if (open)
      return;
    await new Promise(r => setTimeout(r, 100));
  }
  throw new Error(`fixture agent did not listen on 127.0.0.1:${port}`);
}

/**
 * Start the scripted demo agent for a case that needs a live one.
 *
 * `agent: "demo"` in a case file is the whole opt-in: the address handed to the
 * run steps is this process's freshly taken port, never the user's running
 * agent's, and it is torn down with the case. A case without it keeps the
 * closed-port default, so an offline command still cannot reach a real agent by
 * accident.
 */
async function startDemoAgent() {
  const port = await freePort();
  const child = spawn(demoAgentBin(), ["--port", String(port)], { stdio: "ignore" });
  await waitForPort(port);
  return { child, addr: `127.0.0.1:${port}`, port };
}

export function cliBin() {
  if (process.env.FUTURE_BIN)
    return process.env.FUTURE_BIN;
  const local = join(REPO_ROOT, "target", "debug", "future");
  return existsSync(local) ? local : "future";
}

const stripAnsi = text => String(text).replace(ANSI, "");

function getPath(object, path) {
  if (!path)
    return object;
  return String(path).split(".").reduce((value, key) => {
    if (value == null)
      return undefined;
    if (/^\d+$/.test(key))
      return value[Number(key)];
    return value[key];
  }, object);
}

export async function runCliCase({ testCase, runDir, bin = cliBin() }) {
  const caseDir = join(runDir, testCase.id);
  const { home, appDb } = createCliHome(join(caseDir, "home"));
  mkdirSync(caseDir, { recursive: true });
  const startedAt = Date.now();
  const baselines = readBaseline(join(import.meta.dirname, "..", "perf", "baseline.json"), { existsSync, readFileSync });
  const timeline = [];
  const checks = [];
  let status = "pass";
  let fatal = null;
  let last = null;
  const logPath = join(caseDir, "timeline.jsonl");
  const note = (entry) => {
    const record = { ts: Date.now(), step: timeline.length, ...entry };
    timeline.push(record);
    appendFileSync(logPath, `${JSON.stringify(record)}\n`);
    return record;
  };

  const queryDb = (sql, file = "app.db") => {
    // Only the two databases a CLI case can write: the desktop store and the
    // scheduled-task store. A path is not accepted, so a case cannot reach out
    // of its own home.
    const target = file === "tasks.db" ? join(home, "tasks", "tasks.db") : appDb;
    if (!existsSync(target))
      throw new Error(`no such database: ${file} (${target})`);
    const db = new DatabaseSync(target, { readOnly: true });
    try {
      return db.prepare(sql).all();
    }
    finally {
      db.close();
    }
  };

  const fail = (message, extra = {}) => {
    status = "fail";
    fatal ??= message;
    note({ action: "fail", ok: false, error: message, ...extra });
  };

  // An agent-backed case (`"agent": "demo"`) gets the scripted gRPC server on a
  // port taken for this case alone; every other case keeps the closed port that
  // makes an accidental connection impossible.
  let fixture = null;
  if (testCase.agent === "demo") {
    fixture = await startDemoAgent();
    note({ action: "agent", ok: true, note: `demo_agent on 127.0.0.1:${fixture.port}` });
    // A crash between here and the kill below still has to take the child with
    // it: an orphaned agent on a fixed port is how the next run talks to a
    // process nobody is watching.
    process.once("exit", () => fixture?.child.kill());
  }
  const agentAddr = fixture?.addr ?? "127.0.0.1:1";

  // `{{home}}` / `{{case}}` name the two paths a case cannot spell for itself:
  // the isolated `FUTURE_HOME` and its parent. A backup has to be written
  // *outside* the home it archives (a directory inside it would be walked into
  // itself), and neither path is knowable when the case file is written.
  const fill = value => typeof value === "string"
    ? value.replaceAll("{{home}}", home).replaceAll("{{case}}", caseDir)
    : Array.isArray(value) ? value.map(fill)
      : value && typeof value === "object"
        ? Object.fromEntries(Object.entries(value).map(([key, item]) => [key, fill(item)]))
        : value;

  for (const [index, rawStep] of testCase.steps.entries()) {
    const step = fill(rawStep);
    const action = Object.keys(step).find(key => !["note", "timeout", "optional"].includes(key)) ?? "note";
    if (step.note)
      note({ action: "note", note: step.note });
    if (action === "note")
      continue;
    const entry = { action, index: index + 1, note: step.note ?? null, target: step[action] };
    const started = Date.now();
    try {
      if (action === "run") {
        const args = Array.isArray(step.run) ? step.run : [step.run];
        const env = {
          ...process.env,
          FUTURE_HOME: home,
          FUTURE_AGENT_GRPC_ADDR: agentAddr,
          NO_COLOR: "1",
          TERM: "dumb",
        };
        try {
          const { stdout, stderr } = await execFileAsync(bin, args, {
            cwd: home,
            env,
            timeout: Number(step.timeout ?? 120_000),
            maxBuffer: 20 * 1024 * 1024,
          });
          last = { args, stdout: stripAnsi(stdout), stderr: stripAnsi(stderr), exit: 0 };
        }
        catch (error) {
          last = {
            args,
            stdout: stripAnsi(error?.stdout ?? ""),
            stderr: stripAnsi(error?.stderr ?? String(error?.message ?? error)),
            exit: typeof error?.code === "number" ? error.code : 1,
          };
        }
        const runIndex = timeline.length;
        writeFileSync(join(caseDir, `run-${String(runIndex).padStart(2, "0")}.stdout.log`), last.stdout);
        writeFileSync(join(caseDir, `run-${String(runIndex).padStart(2, "0")}.stderr.log`), last.stderr);
        entry.args = args;
        entry.exit = last.exit;
        entry.stdout = last.stdout.slice(0, 4000);
        entry.stderr = last.stderr.slice(0, 4000);
        if (step.expectExit !== undefined) {
          const ok = last.exit === step.expectExit;
          checks.push({ kind: "exit", step: index + 1, message: `future ${args.join(" ")} 退出码 ${step.expectExit}`, ok, expected: step.expectExit, actual: last.exit });
          if (!ok)
            throw new Error(`exit code ${last.exit}, expected ${step.expectExit}`);
        }
      }
      else if (action === "expectStdout" || action === "expectStderr") {
        const target = action === "expectStdout" ? last?.stdout : last?.stderr;
        if (target == null)
          throw new Error(`${action} without a preceding run`);
        const wanted = step[action];
        const ok = wanted instanceof Object && wanted.regex
          ? new RegExp(wanted.regex, wanted.flags ?? "").test(target)
          : target.includes(String(wanted));
        checks.push({ kind: action, step: index + 1, message: `${action} ${JSON.stringify(wanted)}`, ok });
        if (!ok)
          throw new Error(`${action} did not match: ${JSON.stringify(wanted)}\n--- actual ---\n${target.slice(0, 2000)}`);
      }
      else if (action === "expectJson") {
        if (!last)
          throw new Error("expectJson without a preceding run");
        let value;
        try {
          value = JSON.parse(last.stdout);
        }
        catch (error) {
          throw new Error(`stdout is not JSON: ${String(error?.message ?? error)}\n${last.stdout.slice(0, 1000)}`);
        }
        const spec = step.expectJson;
        const selected = spec.path ? getPath(value, spec.path) : value;
        let ok = true;
        let actual = selected;
        if (spec.equals !== undefined) {
          ok = JSON.stringify(selected) === JSON.stringify(spec.equals);
          actual = selected;
        }
        else if (spec.lengthMin !== undefined) {
          ok = Array.isArray(selected) && selected.length >= spec.lengthMin;
          actual = Array.isArray(selected) ? selected.length : selected;
        }
        else if (spec.contains !== undefined) {
          ok = JSON.stringify(value).includes(String(spec.contains));
        }
        else if (spec.exists !== undefined) {
          ok = (selected !== undefined) === !!spec.exists;
        }
        checks.push({ kind: "json", step: index + 1, message: spec.message ?? `JSON ${spec.path ?? "<root>"}`, ok, expected: spec, actual: typeof actual === "object" ? JSON.stringify(actual).slice(0, 500) : actual });
        if (!ok)
          throw new Error(`JSON expectation failed: ${JSON.stringify(spec)}\nactual: ${JSON.stringify(actual).slice(0, 1000)}`);
      }
      else if (action === "assertPerf") {
        const spec = step.assertPerf;
        const runs = timeline.filter(item => item.action === "run").map(item => item.ms ?? 0);
        const value = spec.metric === "maxMs"
          ? Math.max(0, ...runs)
          : spec.metric === "lastMs" ? (runs.at(-1) ?? null)
            : spec.metric === "durationMs" ? Date.now() - startedAt : null;
        const key = spec.baselineKey ?? baselineKey(testCase.id, spec);
        const { limit, reason } = resolveLimit(spec, baselines.metrics?.[key] ?? null);
        const ok = value !== null && limit !== null && value <= limit;
        checks.push({ kind: "perf", step: index + 1, message: spec.message ?? `${spec.metric} <= ${Math.round(limit ?? NaN)}`, ok, expected: limit, actual: value, baseline: baselines.metrics?.[key]?.median ?? null, limitReason: reason });
        entry.value = value;
        if (!ok)
          throw new Error(`CLI performance expectation failed: ${spec.metric} = ${value}, expected <= ${Math.round(limit ?? NaN)} (${reason})`);
      }
      else if (action === "db") {
        const spec = typeof step.db === "string" ? { sql: step.db } : step.db;
        const rows = queryDb(spec.sql, spec.file ?? "app.db");
        entry.rows = rows;
        const scalar = rows.length === 1 && Object.keys(rows[0]).length === 1 ? Object.values(rows[0])[0] : rows;
        if (spec.expect !== undefined) {
          const ok = JSON.stringify(scalar) === JSON.stringify(spec.expect);
          checks.push({ kind: "db", step: index + 1, message: spec.message ?? spec.sql, ok, expected: spec.expect, actual: scalar });
          if (!ok)
            throw new Error(`DB expectation failed: ${spec.message ?? spec.sql} (expected ${JSON.stringify(spec.expect)}, got ${JSON.stringify(scalar)})`);
        }
      }
      else if (action === "mkdir") {
        mkdirSync(step.mkdir, { recursive: true });
      }
      else if (action === "remove") {
        // Start a step that measures a fresh artefact (a backup directory, an
        // export) from nothing: re-running a case under the same run name
        // reuses this directory, and `future data backup` refuses a destination
        // that already exists.
        rmSync(step.remove, { recursive: true, force: true });
      }
      else {
        throw new Error(`unknown CLI step: ${action}`);
      }
      entry.ok = true;
      note({ ...entry, ms: Date.now() - started });
    }
    catch (error) {
      entry.ok = false;
      entry.error = String(error?.message ?? error);
      entry.ms = Date.now() - started;
      status = "fail";
      fatal ??= entry.error;
      note(entry);
      if (!step.optional)
        break;
    }
  }

  fixture?.child.kill();

  const result = {
    id: testCase.id,
    title: testCase.title,
    surface: "cli",
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
    bodyText: (timeline.filter(entry => entry.action === "run").at(-1)?.stdout ?? "").slice(0, 12000),
    steps: timeline,
    checks,
    console: { errors: [], warnings: [], all: [] },
    exceptions: [],
    network: { failures: [], count: 0 },
    server: { commands: [], fallbacks: [], errors: [] },
    metrics: [],
    cli: { bin, home, runs: timeline.filter(entry => entry.action === "run").map(entry => ({ args: entry.args, exit: entry.exit, stderr: entry.stderr })) },
    perf: {
      durationMs: Date.now() - startedAt,
      commands: timeline.filter(entry => entry.action === "run").map(entry => ({ args: entry.args, ms: entry.ms })),
      maxMs: Math.max(0, ...timeline.filter(entry => entry.action === "run").map(entry => entry.ms ?? 0)),
    },
    artifacts: {
      dir: caseDir,
      timeline: logPath,
      shots: [],
      home,
    },
  };
  writeFileSync(join(caseDir, "result.json"), JSON.stringify(result, null, 2));
  return result;
}
