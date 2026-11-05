/**
 * `remote-mobile-live`: the phone's own client, connected to a real desktop.
 *
 * Every other mobile case runs the real screens over fixture data, because the
 * Expo web build has no transport. This one has the transport: it bundles
 * `mobile/src/remote/pairing.ts` and `client.ts` — the code a phone actually
 * runs — and drives it against a real self-hosted relay and a real `future host`.
 * Only `expo-device`, `expo-secure-store` and `react-native` are stubbed, and
 * each of those answers with a constant a case can assert.
 *
 * What that buys, which no fixture can: the invitation is really claimed, the
 * NATS WebSocket endpoint really has to be acceptable, the credential really has
 * to be accepted, the catalogue really has to arrive, and a write from the phone
 * really has to land in the desktop's database.
 *
 * The client process is long-lived and speaks one JSON command per line, because
 * a connection, its presence beats and a second client taking over only exist
 * over time.
 */
import { mkdirSync, writeFileSync, existsSync } from "node:fs";
import { join } from "node:path";
import { spawn } from "node:child_process";
import { createRelayFixture, readJson, waitUntil } from "./relay-fixture.mjs";

const sleep = ms => new Promise(done => setTimeout(done, ms));

function slug(text) {
  return String(text).replace(/[^\w\u4e00-\u9fa5-]+/g, "-").replace(/^-+|-+$/g, "").slice(0, 40) || "case";
}

/** One phone: a bundled client process with a line protocol. */
class Phone {
  constructor({ bundle, caseDir, label, logPath }) {
    this.label = label;
    this.caseDir = caseDir;
    this.pending = new Map();
    this.seq = 0;
    this.logPath = logPath ?? join(caseDir, `${label}.log`);
    this.output = "";
    this.proc = spawn(process.execPath, [bundle], {
      cwd: caseDir,
      stdio: ["pipe", "pipe", "pipe"],
      env: { ...process.env, FUTURE_HOME: join(caseDir, `${label}-home`) },
    });
    const onData = (chunk) => {
      this.output += chunk.toString("utf8");
      writeFileSync(this.logPath, this.output);
      for (const line of this.output.split("\n")) {
        if (!line.trim() || !line.startsWith("{"))
          continue;
        const parsed = readJson(line);
        if (!parsed)
          continue;
        if (parsed.id === undefined || parsed.id === null)
          continue;
        const waiting = this.pending.get(parsed.id);
        if (waiting) {
          this.pending.delete(parsed.id);
          waiting(parsed);
        }
      }
      // Keep only the tail: a long run would otherwise reparse everything.
      if (this.output.length > 200_000)
        this.output = this.output.slice(-100_000);
    };
    this.proc.stdout.on("data", onData);
    this.proc.stderr.on("data", onData);
  }

  send(cmd, payload = {}, timeoutMs = 60_000) {
    const id = ++this.seq;
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => {
        this.pending.delete(id);
        reject(new Error(`phone ${this.label}: no answer to ${cmd} in ${timeoutMs}ms\n${this.output.slice(-1500)}`));
      }, timeoutMs);
      this.pending.set(id, (message) => {
        clearTimeout(timer);
        if (message.ok)
          resolve(message.result);
        else
          reject(new Error(`phone ${this.label}: ${cmd} failed: ${message.error}`));
      });
      this.proc.stdin.write(`${JSON.stringify({ id, cmd, ...payload })}\n`);
    });
  }

  async stop() {
    try {
      this.proc.stdin.end();
      this.proc.kill("SIGTERM");
    }
    catch {}
    await sleep(200);
  }
}

export async function runRemoteMobileCase({ testCase, runDir, bin }) {
  const caseDir = join(runDir, slug(testCase.id));
  mkdirSync(caseDir, { recursive: true });
  const bundle = join(import.meta.dirname, "..", "mobile-client", ".artifacts", "mobile-client.cjs");
  if (!existsSync(bundle)) {
    // The bundle is gitignored, so a fresh checkout has the sources and no build.
    // Building it here keeps a first run working instead of failing with an
    // instruction the caller has to go and follow.
    const { execFileSync } = await import("node:child_process");
    execFileSync(process.execPath, [join(import.meta.dirname, "..", "mobile-client", "build.mjs")], { stdio: "pipe" });
  }
  if (!existsSync(bundle))
    throw new Error(`mobile client bundle missing (${bundle}); run: node harness/mobile-client/build.mjs`);

  const startedAt = Date.now();
  const timeline = [];
  const checks = [];
  const variables = {};
  const notes = [];
  const phones = [];
  const fixture = createRelayFixture({ bin, caseDir });
  let status = "pass";
  let fatal = null;
  let index = 0;

  const note = (entry) => {
    timeline.push({ ...entry, index: ++index, ts: Date.now() - startedAt });
  };
  const phone = label => phones.find(item => item.label === label) ?? phones[0];

  const actions = {
    async hostStart() {
      await fixture.start();
      note({ action: "hostStart", note: `relay ${fixture.relay.api} / nats-ws ${fixture.relay.ws}` });
    },
    /** Mint an invitation on the host, exactly as the desktop's Remote page does. */
    async invite(step) {
      // A host that has just come up can still be finishing its bridge; asking for
      // an invitation in that window answers "remote start ended in Reconnecting".
      // Retry with a deadline rather than failing a case on the host's boot race.
      const deadline = Date.now() + 30_000;
      let parsed = null;
      let lastError = "";
      while (Date.now() < deadline) {
        const result = fixture.hostCli(["remote", "devices", "add", "--json"]);
        parsed = readJson(result.stdout);
        if (parsed?.link)
          break;
        lastError = (result.stderr || result.stdout || "").trim();
        await sleep(1000);
      }
      if (!parsed?.link)
        throw new Error(`no invitation: ${lastError}`.slice(0, 400));
      variables[step.as ?? "link"] = parsed.link;
      note({ action: "invite", ok: true, value: parsed.pairId, note: `pairId ${parsed.pairId}` });
    },
    /** Start a phone process; `label` allows a second one for takeover cases. */
    async phone(step) {
      const label = step.label ?? "phone";
      const instance = new Phone({ bundle, caseDir, label });
      phones.push(instance);
      note({ action: "phone", note: `started ${label}` });
    },
    /** Claim the invitation with the phone's own pairing code. */
    async claim(step) {
      const link = interpolate(step.claim, variables);
      const result = await phone(step.label).send("claim", { invitation: link });
      variables[step.as ?? "claimed"] = result;
      checks.push({ kind: "claim", message: "手机认领邀请", ok: Boolean(result?.pairId), actual: result?.pairId ?? null });
      note({ action: "claim", ok: Boolean(result?.pairId), value: result?.pairId });
    },
    async connect(step) {
      await phone(step.label).send("connect", {});
      note({ action: "connect" });
    },
    /** Wait for the catalogue — the first thing that proves a real connection. */
    async ready(step) {
      const timeoutMs = step.timeout ?? 45_000;
      const result = await phone(step.label).send("ready", { timeoutMs: timeoutMs - 5_000 }, timeoutMs);
      variables[step.as ?? "ready"] = result;
      checks.push({ kind: "ready", message: "手机收到会话目录", ok: result.sessions > 0, actual: `${result.sessions} 个会话 / ${result.workspaces} 个工作区` });
      note({ action: "ready", value: result });
    },
    async snapshot(step) {
      const result = await phone(step.label).send("snapshot", {});
      variables[step.as ?? "snapshot"] = result;
      note({ action: "snapshot", value: { sessions: result.sessions.length, presence: result.presence, firstRow: result.sessions[0] ?? null } });
    },
    async visible(step) {
      await phone(step.label).send("visible", { sessionId: interpolate(step.visible, variables) });
      note({ action: "visible" });
    },
    /** A command through the phone's real request channel. */
    async request(step) {
      const spec = step.request;
      const result = await phone(step.label).send("request", {
        command: substitute(spec.command, variables),
        sessionId: spec.sessionId ? interpolate(spec.sessionId, variables) : undefined,
      });
      variables[spec.as ?? "response"] = result;
      checks.push({ kind: "request", message: spec.message ?? `手机发出 ${spec.command?.type}`, ok: result.ok !== false, actual: result.ok });
      note({ action: "request", ok: result.ok !== false, note: spec.command?.type });
    },
    /** What the host's own book says now — the other half of every write. */
    async hostDevices(step) {
      // The book is written when the bridge finishes its handshake, which is a
      // little after the phone is already usable; poll rather than race it.
      const deadline = Date.now() + (step.wait ?? 0);
      let parsed = readJson(fixture.hostCli(["remote", "devices", "--json"]).stdout);
      while (Date.now() < deadline && !(parsed?.devices?.length)) {
        await sleep(400);
        parsed = readJson(fixture.hostCli(["remote", "devices", "--json"]).stdout);
      }
      variables[step.as ?? "devices"] = parsed;
      note({ action: "hostDevices", value: parsed?.devices?.length ?? null });
    },
    async hostStatus(step) {
      const parsed = readJson(fixture.hostCli(["remote", "status", "--json"]).stdout);
      variables[step.as ?? "hostStatus"] = parsed;
      note({ action: "hostStatus", value: { state: parsed?.state ?? null, active: parsed?.active ?? null, devices: (parsed?.devices ?? []).map(d => ({ pair: (d.pairId ?? "").slice(0,6), control: d.control, connected: d.connected, present: d.present })) } });
    },
    /** A row in the host's `app.db`: the phone's write, seen from the desktop side. */
    async hostDb(step) {
      const spec = step.hostDb;
      const result = await import("./remote-live-db.mjs").then(module => module.queryHostDb({
        home: fixture.host.home,
        sql: interpolate(spec.sql, variables),
      }));
      variables[spec.as ?? "rows"] = result;
      note({ action: "hostDb", value: Array.isArray(result) ? result.length : result });
      if (spec.expect !== undefined) {
        const ok = JSON.stringify(result) === JSON.stringify(spec.expect);
        checks.push({ kind: "db", message: spec.message ?? spec.sql, ok, expected: spec.expect, actual: result });
        if (!ok)
          throw new Error(`host db expectation failed: ${spec.message ?? spec.sql} (expected ${JSON.stringify(spec.expect)}, got ${JSON.stringify(result)})`);
      }
    },
    /**
     * Poll a phone's snapshot until a path reaches a value.
     *
     * Presence is repeated on the host's heartbeat, so a control change reaches
     * the other side on the next tick rather than with the response. Polling is
     * the honest way to assert that: it says "within N seconds this became true"
     * instead of hiding the wait in a fixed sleep that fails on a slow machine.
     */
    async until(step) {
      const spec = step.until;
      const deadline = Date.now() + (spec.timeout ?? 25_000);
      let actual;
      let last = null;
      while (Date.now() < deadline) {
        last = await phone(spec.label).send("snapshot", {});
        actual = pick(last, spec.path);
        if (matches(actual, spec))
          break;
        await sleep(1000);
      }
      const ok = matches(actual, spec);
      checks.push({ kind: "until", message: spec.message ?? `${spec.path} → ${JSON.stringify(spec.equals)}`, ok, expected: spec.equals ?? null, actual });
      note({ action: "until", ok, value: actual });
      if (!ok)
        throw new Error(`timed out: ${spec.message ?? spec.path} (got ${JSON.stringify(actual)})`);
      variables[spec.as ?? "until"] = last;
    },
    assertJson(step) {
      const spec = step.assertJson;
      // The expected value may itself be a variable (`{{claimedA.pairId}}`): a
      // case comparing two live values — "the holder is the device that claimed
      // first" — has no literal to write down.
      const expected = spec.equals !== undefined ? substitute(spec.equals, variables) : spec.equals;
      const specForMatch = { ...spec, ...(spec.equals !== undefined ? { equals: expected } : {}) };
      const actual = pick(variables[spec.from ?? "snapshot"], spec.path);
      const ok = matches(actual, specForMatch);
      checks.push({ kind: "json", message: spec.message ?? `${spec.from}.${spec.path}`, ok, expected: expected ?? spec.expect ?? null, actual });
      note({ action: "assertJson", ok, value: actual });
      if (!ok) {
        const from = variables[spec.from ?? "snapshot"];
        throw new Error(`expectation failed: ${spec.message ?? spec.path} (got ${JSON.stringify(actual)}; from=${typeof from} keys=${from && typeof from === "object" ? Object.keys(from).join(",") : "-"})`);
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

  // `{{snapshot.sessions[0].sessionId}}` resolves through the variable's own
  // shape, not just a flat name: a case has to name a row it only discovered at
  // run time, and the real database is what decides which row that is.
  const interpolate = (value, vars) => typeof value === "string"
    ? value.replace(/\{\{([\w.\[\]]+)\}\}/g, (match, path) => {
        const resolved = pick(vars, path);
        return resolved === undefined ? match : String(resolved);
      })
    : value;
  const substitute = (value, vars) => {
    if (typeof value === "string")
      return interpolate(value, vars);
    if (Array.isArray(value))
      return value.map(item => substitute(item, vars));
    if (value && typeof value === "object")
      return Object.fromEntries(Object.entries(value).map(([key, item]) => [key, substitute(item, vars)]));
    return value;
  };

  try {
    for (const step of testCase.steps ?? []) {
      const action = Object.keys(step).find(key => key in actions && key !== "note");
      if (action) {
        // `{"snapshot": {"as": "after"}}` and `{"snapshot": true, "as": ...}`
        // both read as "store it under this name"; only one shape was handled.
        const payload = step[action];
        if (payload && typeof payload === "object") {
          // `{"snapshot": {"as": "after", "label": "b"}}` and
          // `{"snapshot": true, "as": ..., "label": ...}` both mean the same
          // thing; only the flat shape was read, so a `label` inside the payload
          // silently addressed the first phone and made the second look identical.
          if (payload.as && !step.as)
            step.as = payload.as;
          if (payload.label && !step.label)
            step.label = payload.label;
        }
        await actions[action](step);
      }
      else if (step.note)
        actions.note(step);
      else
        throw new Error(`unknown remote-mobile-live step: ${JSON.stringify(step).slice(0, 120)}`);
    }
  }
  catch (error) {
    status = "fail";
    fatal = String(error?.message ?? error);
  }
  finally {
    for (const instance of phones.reverse())
      await instance.stop();
    await fixture.stop();
  }

  const logs = fixture.logs();
  const phoneLogs = phones.map(instance => `${instance.label}.log:\n${(instance.output ?? "").slice(-3000)}`).join("\n\n");
  notes.push(`relay ${fixture.relay.api}，nats-ws ${fixture.relay.ws}`);

  const result = {
    id: testCase.id,
    title: testCase.title,
    surface: "remote-mobile-live",
    area: testCase.area ?? "remote",
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
    perf: { durationMs: Date.now() - startedAt },
    observations: [],
    // No screen on this surface; what the model reads is the two processes'
    // transcripts and the numbers a case asserted on.
    bodyText: [
      `以下是手机客户端与被控主机的进程日志。harness 收尾时会停止进程，日志已被截断在收尾之前。`,
      notes.join("\n"),
      phoneLogs,
      `host.log:\n${logs.host.slice(-4000)}`,
    ].join("\n\n"),
    console: { errors: [], warnings: [], all: [] },
    exceptions: [],
    network: { failures: [], count: 0 },
    artifacts: { dir: caseDir, relay: fixture.relay.dir },
  };
  writeFileSync(join(caseDir, "result.json"), JSON.stringify(result, null, 2));
  return result;
}

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

function matches(actual, spec) {
  if ("absent" in spec)
    return actual === undefined || actual === null;
  if ("equals" in spec)
    return JSON.stringify(actual) === JSON.stringify(spec.equals);
  if ("gte" in spec)
    return typeof actual === "number" && actual >= spec.gte;
  if ("contains" in spec) {
    if (Array.isArray(actual))
      return actual.includes(spec.contains);
    return typeof actual === "string" && actual.includes(spec.contains);
  }
  if ("truthy" in spec)
    return Boolean(actual) === Boolean(spec.truthy);
  return actual !== undefined && actual !== null;
}
