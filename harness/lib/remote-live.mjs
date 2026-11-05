/**
 * `remote-live` surface: the real relay path, with real processes.
 *
 * The `remote` surface next to this one drives the desktop and mobile UIs
 * against the shot mocks, which prove both clients render the same session — not
 * that a pairing works. This one is the other half: a real self-hosted relay
 * (`future relay init` + `nats-server` + `future relay serve`), a real
 * `future host` serving it, and a real client pairing through the invitation.
 * Nothing is mocked; the only substitution is that all of it runs on one
 * machine, each process with its own `FUTURE_HOME`.
 *
 * Why this is worth its own surface: the relay path is the one no UI test can
 * reach. Its failures (an invitation that cannot be claimed, a bridge that never
 * connects, a device that shows up as connected but stands by forever) are
 * invisible in the desktop mocks, because the mocks answer `remote_peer_list`
 * with a fixture.
 *
 * The driver owns the lifecycle — relay up, host up, pair, assert — because the
 * order is protocol, not preference. A case chooses the scenario: which
 * assertions run, and which fault to inject.
 */
import { spawn, execFileSync } from "node:child_process";
import { mkdirSync, writeFileSync, cpSync, existsSync, readFileSync } from "node:fs";
import { createServer } from "node:net";
import { join, resolve } from "node:path";

const sleep = ms => new Promise(done => setTimeout(done, ms));
const REPO_ROOT = resolve(import.meta.dirname, "..", "..");
const CLI_HOME = resolve(import.meta.dirname, "..", ".data", "cli");

function slug(text) {
  return String(text).replace(/[^\w\u4e00-\u9fa5-]+/g, "-").replace(/^-+|-+$/g, "").slice(0, 40) || "case";
}

function freePort() {
  return new Promise((done, reject) => {
    const server = createServer();
    server.on("error", reject);
    server.listen(0, "127.0.0.1", () => {
      const { port } = server.address();
      server.close(() => done(port));
    });
  });
}

async function waitFor(probe, { timeout = 30_000, interval = 200, what = "condition" } = {}) {
  const deadline = Date.now() + timeout;
  let last = null;
  while (Date.now() < deadline) {
    try {
      last = await probe();
      if (last)
        return last;
    }
    catch (error) {
      last = error;
    }
    await sleep(interval);
  }
  throw new Error(`timed out waiting for ${what}${last ? `: ${String(last?.message ?? last).slice(0, 300)}` : ""}`);
}

function run(bin, args, { home, env = {}, timeout = 60_000 } = {}) {
  const started = Date.now();
  try {
    const stdout = execFileSync(bin, args, {
      encoding: "utf8",
      timeout,
      env: { ...process.env, FUTURE_HOME: home, ...env },
    });
    return { exit: 0, stdout, stderr: "", ms: Date.now() - started, args };
  }
  catch (error) {
    return { exit: error.status ?? 1, stdout: error.stdout ?? "", stderr: error.stderr ?? String(error.message), ms: Date.now() - started, args };
  }
}

/** A home with the CLI's snapshot of models/settings/skills, plus nothing else. */
function seedHome(dir) {
  mkdirSync(dir, { recursive: true });
  if (existsSync(CLI_HOME))
    cpSync(CLI_HOME, dir, { recursive: true });
  return dir;
}

function readJson(text) {
  try {
    return JSON.parse(text);
  }
  catch {
    return null;
  }
}

/** Read `a.b[0].c` out of a parsed value; undefined when the path does not exist. */
function pick(value, path) {
  if (!path)
    return value;
  return path.split(".").reduce((current, key) => {
    if (current === null || current === undefined)
      return undefined;
    const match = /^([^[]+)(?:\[(\d+)\])?$/.exec(key);
    if (!match)
      return undefined;
    const [, name, index] = match;
    const next = current[name];
    return index === undefined ? next : next?.[Number(index)];
  }, value);
}

export async function runRemoteLiveCase({ testCase, runDir, bin }) {
  const caseDir = join(runDir, slug(testCase.id));
  mkdirSync(caseDir, { recursive: true });
  const startedAt = Date.now();
  const timeline = [];
  const checks = [];
  const variables = {};
  const logs = [];
  const started = [];
  let status = "pass";
  let fatal = null;
  let stepIndex = 0;

  const relay = { dir: join(caseDir, "relay"), api: null, nats: null, ws: null, procs: [] };
  const host = { home: seedHome(join(caseDir, "host-home")), proc: null, logPath: join(caseDir, "host.log") };
  const client = { home: seedHome(join(caseDir, "client-home")) };
  const relayLog = join(caseDir, "relay.log");
  const natsLog = join(caseDir, "nats.log");

  const note = (entry) => {
    timeline.push({ ...entry, index: ++stepIndex, ts: Date.now() - startedAt });
  };

  const stop = (proc) => {
    if (!proc || proc.exitCode != null)
      return;
    try {
      if (process.platform === "win32")
        proc.kill("SIGTERM");
      else
        process.kill(-proc.pid, "SIGTERM");
    }
    catch {
      try {
        proc.kill("SIGTERM");
      }
      catch {}
    }
  };

  const spawnLogged = (command, args, { env = {}, logPath, cwd } = {}) => {
    const child = spawn(command, args, {
      cwd,
      stdio: ["ignore", "pipe", "pipe"],
      detached: process.platform !== "win32",
      env: { ...process.env, ...env },
    });
    const out = [];
    const append = chunk => {
      out.push(chunk);
      if (logPath)
        writeFileSync(logPath, Buffer.concat(out).toString("utf8"));
    };
    child.stdout.on("data", append);
    child.stderr.on("data", append);
    started.push(child);
    return child;
  };

  /** Bring the relay up: trust material, a broker, and the relay API. */
  const startRelay = async () => {
    if (relay.api)
      return;
    relay.api = await freePort();
    relay.nats = await freePort();
    relay.ws = await freePort();
    const init = run(bin, [
      "relay", "init",
      "--dir", relay.dir,
      "--listen", `127.0.0.1:${relay.api}`,
      "--public-url", `http://127.0.0.1:${relay.api}`,
      "--nats-url", `nats://127.0.0.1:${relay.nats}`,
      "--nats-ws-url", `ws://127.0.0.1:${relay.ws}`,
    ], { home: host.home });
    if (init.exit !== 0)
      throw new Error(`relay init failed: ${init.stderr || init.stdout}`);
    relay.natsProc = spawnLogged("nats-server", ["-c", join(relay.dir, "nats.conf")], { logPath: natsLog });
    relay.apiProc = spawnLogged(bin, ["relay", "serve", "--config", join(relay.dir, "relay.json")], {
      logPath: relayLog,
      env: { FUTURE_HOME: host.home },
    });
    // The API is what the host calls to register a room; wait for it to answer
    // at all. Any HTTP status means the listener is up.
    await waitFor(async () => {
      try {
        await fetch(`http://127.0.0.1:${relay.api}/`, { signal: AbortSignal.timeout(500) });
        return true;
      }
      catch {
        return false;
      }
    }, { what: `relay API on ${relay.api}`, timeout: 20_000 });
    relay.procs = [relay.apiProc, relay.natsProc];
  };

  const startHost = async () => {
    if (host.proc)
      return;
    host.proc = spawnLogged(bin, ["host", "--relay", `http://127.0.0.1:${relay.api}`], {
      logPath: host.logPath,
      env: { FUTURE_HOME: host.home, FUTURE_RELAY_URL: `http://127.0.0.1:${relay.api}` },
    });
    await waitFor(() => {
      const status = run(bin, ["remote", "status", "--json"], {
        home: host.home,
        env: { FUTURE_RELAY_URL: `http://127.0.0.1:${relay.api}` },
      });
      const parsed = readJson(status.stdout);
      return parsed?.desktopRunning ? parsed : null;
    }, { what: "host to report its remote status", timeout: 60_000 });
  };

  const hostCli = args => run(bin, args, { home: host.home, env: { FUTURE_RELAY_URL: `http://127.0.0.1:${relay.api}` } });
  const clientCli = args => run(bin, args, { home: client.home });

  const actions = {
    async relayStart() {
      await startRelay();
      note({ action: "relayStart", note: `relay api ${relay.api}, nats ${relay.nats}` });
    },
    async hostStart() {
      await startRelay();
      await startHost();
      note({ action: "hostStart", note: "future host --relay" });
    },
    /** Mint an invitation on the host and keep the link. */
    async invite(step) {
      const result = hostCli(["remote", "devices", "add", "--json"]);
      const parsed = readJson(result.stdout);
      const entry = { action: "invite", ok: result.exit === 0 && Boolean(parsed?.link), value: parsed?.pairId ?? null, note: `exit ${result.exit}` };
      if (!parsed?.link)
        throw new Error(`no invitation: ${result.stderr || result.stdout}`.slice(0, 400));
      variables[step.as ?? "link"] = parsed.link;
      note(entry);
    },
    /** Claim the invitation from the client's home: the real pairing handshake. */
    async clientAdd(step) {
      const link = interpolate(step.clientAdd, variables);
      const result = clientCli(["remote", "peers", "add", link, "--json"]);
      const parsed = readJson(result.stdout);
      const entry = { action: "clientAdd", ok: result.exit === 0 && Boolean(parsed), value: parsed?.paired?.pairId ?? null, note: `exit ${result.exit}` };
      variables[step.as ?? "paired"] = parsed;
      note(entry);
      if (!parsed)
        throw new Error(`pairing failed: ${result.stderr || result.stdout}`.slice(0, 400));
    },
    async clientCli(step) {
      const args = (step.clientCli ?? []).map(item => interpolate(item, variables));
      const result = clientCli(args);
      const parsed = readJson(result.stdout);
      variables[step.as ?? "client"] = parsed ?? result.stdout;
      note({ action: "clientCli", note: `future ${args.join(" ")} (exit ${result.exit})`, ok: step.expectExit === undefined || result.exit === step.expectExit });
      if (step.expectExit !== undefined && result.exit !== step.expectExit)
        throw new Error(`future ${args.join(" ")} exited ${result.exit}: ${result.stderr || result.stdout}`.slice(0, 400));
    },
    async hostCli(step) {
      const args = (step.hostCli ?? []).map(item => interpolate(item, variables));
      const result = hostCli(args);
      const parsed = readJson(result.stdout);
      variables[step.as ?? "host"] = parsed ?? result.stdout;
      note({ action: "hostCli", note: `future ${args.join(" ")} (exit ${result.exit})`, ok: step.expectExit === undefined || result.exit === step.expectExit });
      if (step.expectExit !== undefined && result.exit !== step.expectExit)
        throw new Error(`future ${args.join(" ")} exited ${result.exit}: ${result.stderr || result.stdout}`.slice(0, 400));
    },
    /** Poll until an assertion about a CLI's JSON holds — a state that takes time. */
    async waitFor(step) {
      const spec = step.waitFor;
      const source = spec.source === "host" ? hostCli : clientCli;
      const deadline = Date.now() + (spec.timeout ?? 30_000);
      let last = null;
      // The expectation names a path inside the CLI's JSON; resolve it before
      // comparing, exactly as `assertJson` does. Comparing the whole document
      // against a leaf value fails silently until the timeout.
      const matchesExpectation = value => Boolean(value) && matches(pick(value, spec.expect?.path), spec.expect ?? {});
      while (Date.now() < deadline) {
        const result = source(spec.args.map(item => interpolate(item, variables)));
        last = readJson(result.stdout);
        if (matchesExpectation(last))
          break;
        await sleep(spec.interval ?? 400);
      }
      const ok = matchesExpectation(last);
      checks.push({ kind: "waitFor", message: spec.message ?? `wait for ${JSON.stringify(spec.expect)}`, ok, actual: last ? JSON.stringify(last).slice(0, 800) : null });
      note({ action: "waitFor", ok, value: last });
      if (!ok)
        throw new Error(`waitFor failed: ${spec.message ?? JSON.stringify(spec.expect)}`);
      variables[spec.as ?? "waited"] = last;
    },
    assertJson(step) {
      const spec = step.assertJson;
      const actual = pick(variables[spec.from ?? "value"], spec.path);
      const ok = matches(actual, spec);
      checks.push({ kind: "json", message: spec.message ?? `${spec.from ?? "value"}.${spec.path}`, ok, expected: spec.equals ?? spec.contains ?? spec.truthy ?? null, actual });
      note({ action: "assertJson", ok, value: actual });
      if (!ok)
        throw new Error(`expectation failed: ${spec.message ?? spec.path} (expected ${JSON.stringify(spec.equals ?? spec.contains ?? spec.truthy)}, got ${JSON.stringify(actual)})`);
    },
    async fault(step) {
      if (step.fault === "relay-restart") {
        stop(relay.apiProc);
        stop(relay.natsProc);
        await sleep(1200);
        relay.natsProc = spawnLogged("nats-server", ["-c", join(relay.dir, "nats.conf")], { logPath: natsLog });
        relay.apiProc = spawnLogged(bin, ["relay", "serve", "--config", join(relay.dir, "relay.json")], {
          logPath: relayLog,
          env: { FUTURE_HOME: host.home },
        });
        await waitFor(async () => {
          try {
            await fetch(`http://127.0.0.1:${relay.api}/`, { signal: AbortSignal.timeout(500) });
            return true;
          }
          catch {
            return false;
          }
        }, { what: "relay API after restart", timeout: 20_000 });
        note({ action: "fault", note: "relay and broker restarted on the same ports" });
      }
      else if (step.fault === "host-stop") {
        stop(host.proc);
        host.proc = null;
        await sleep(800);
        note({ action: "fault", note: "host process stopped" });
      }
      else if (step.fault === "client-unpair") {
        const peers = readJson(clientCli(["remote", "peers", "list", "--json"]).stdout);
        const id = peers?.peers?.[0]?.desktopId;
        if (id)
          clientCli(["remote", "peers", "remove", id, "--yes"]);
        note({ action: "fault", note: `client unpaired ${id ?? "(none)"}` });
      }
      else {
        throw new Error(`unknown fault: ${step.fault}`);
      }
    },
    async wait(step) {
      await sleep(step.wait);
      note({ action: "wait", note: `${step.wait}ms` });
    },
    note(step) {
      note({ action: "note", note: interpolate(step.note, variables) });
    },
  };

  const interpolate = (value, vars) => typeof value === "string"
    ? value.replace(/\{\{(\w+)\}\}/g, (match, name) => (name in vars ? String(vars[name]) : match))
    : value;

  try {
    for (const step of testCase.steps ?? []) {
      // The action is the first key this driver knows; `as`/`message`/`note` are
      // payload, not verbs.
      const action = Object.keys(step).find(key => key in actions && key !== "note");
      if (action)
        await actions[action](step);
      else if (step.note)
        actions.note(step);
      else
        throw new Error(`unknown remote-live step: ${JSON.stringify(step).slice(0, 120)}`);
    }
  }
  catch (error) {
    status = "fail";
    fatal = String(error?.message ?? error);
  }
  finally {
    // Cleanup is part of the scenario: a stale pairing in the client's book would
    // make the next run look like it paired by itself.
    try {
      const peers = readJson(clientCli(["remote", "peers", "list", "--json"]).stdout);
      for (const peer of peers?.peers ?? [])
        clientCli(["remote", "peers", "remove", peer.desktopId, "--yes"]);
    }
    catch {}
    stop(host.proc);
    for (const proc of relay.procs.slice().reverse())
      stop(proc);
    await sleep(400);
  }

  const durationMs = Date.now() - startedAt;
  const readLog = path => (existsSync(path) ? readFileSync(path, "utf8").slice(-8000) : "");
  // Cut the log where harness teardown begins. Annotating it was not enough: a
  // reader still reported "the device left right after pairing" as a discovery,
  // when that line was this driver's own SIGTERM.
  const trimTeardown = (text) => {
    const at = text.search(/received SIGTERM|clean shutdown request|stopped \(reason=headless_sigterm/);
    return at > 0 ? `${text.slice(0, at)}\n[harness] teardown begins here; the log is cut at this point.` : text;
  };
  const hostLog = trimTeardown(readLog(host.logPath));
  const relayText = readLog(relayLog);
  logs.push(`host.log（截止到收尾前）:\n${hostLog.slice(-4000)}`);
  if (relayText)
    logs.push(`relay.log:\n${relayText.slice(-2000)}`);

  const result = {
    id: testCase.id,
    title: testCase.title,
    surface: "remote-live",
    area: testCase.area ?? "remote",
    priority: testCase.priority ?? null,
    description: testCase.description ?? null,
    preconditions: testCase.preconditions ?? null,
    expectations: testCase.expectations ?? [],
    judge: testCase.judge ?? null,
    status,
    fatal,
    startedAt: new Date(startedAt).toISOString(),
    durationMs,
    steps: timeline,
    checks,
    perf: { durationMs },
    observations: [],
    // Process logs are the transcript for this surface: there is no screen, and a
    // bridge that failed to connect says so here.
    bodyText: logs.join("\n\n"),
    console: { errors: [], warnings: [], all: [] },
    exceptions: [],
    network: { failures: [], count: 0 },
    artifacts: { dir: caseDir, relay: relay.dir },
  };
  writeFileSync(join(caseDir, "result.json"), JSON.stringify(result, null, 2));
  return result;
}

/**
 * Does `actual` satisfy the expectation spec?
 *
 * Deliberately small: `equals` (deep-equal via JSON), `contains` (string or array
 * member), `truthy`, `absent`. A case library built on a large predicate language
 * becomes a test framework; this is a way to say three things about a value the
 * product just printed.
 */
function matches(actual, spec) {
  if ("absent" in spec)
    return actual === undefined || actual === null;
  if ("equals" in spec)
    return JSON.stringify(actual) === JSON.stringify(spec.equals);
  if ("contains" in spec) {
    if (Array.isArray(actual))
      return actual.includes(spec.contains);
    return typeof actual === "string" && actual.includes(spec.contains);
  }
  if ("truthy" in spec)
    return Boolean(actual) === Boolean(spec.truthy);
  return actual !== undefined && actual !== null;
}
