/**
 * A self-hosted relay plus a headless host, as processes, for one case.
 *
 * Shared by the two `remote-live` drivers: one drives the Rust client (the
 * desktop's path), the other the phone's own TypeScript client. Both need the
 * same four processes in the same order — trust material, broker, relay API,
 * host — and both need to stop them again, so the lifecycle lives here once.
 *
 * Each process gets its own `FUTURE_HOME`, which is what makes it a *different
 * installation* as far as pairing is concerned: the host's device identity, the
 * client's book and the relay's rooms are separate files.
 */
import { spawn, execFileSync } from "node:child_process";
import { mkdirSync, writeFileSync, cpSync, existsSync, readFileSync } from "node:fs";
import { createServer } from "node:net";
import { join, resolve } from "node:path";
import { copyDatabase } from "./data.mjs";

const sleep = ms => new Promise(done => setTimeout(done, ms));
const REPO_ROOT = resolve(import.meta.dirname, "..", "..");
const CLI_HOME = resolve(import.meta.dirname, "..", ".data", "cli");

export function freePort() {
  return new Promise((done, reject) => {
    const server = createServer();
    server.on("error", reject);
    server.listen(0, "127.0.0.1", () => {
      const { port } = server.address();
      server.close(() => done(port));
    });
  });
}

export async function waitUntil(probe, { timeout = 30_000, interval = 200, what = "condition" } = {}) {
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

export function seedHome(dir) {
  mkdirSync(dir, { recursive: true });
  if (existsSync(CLI_HOME))
    cpSync(CLI_HOME, dir, { recursive: true });
  return dir;
}

/**
 * Give a home a real conversation database.
 *
 * The CLI snapshot carries models, settings and skills but no `app.db`, so a host
 * seeded from it alone serves an empty catalogue — which is exactly what the
 * first phone run met: `presence.catalogVersion.sessions` said 1 while
 * `list_sessions` returned nothing, because there was nothing to list. The host
 * is the desktop here, and its catalogue has to be the real one.
 */
export function seedDatabase(home) {
  const target = join(home, "app", "app.db");
  mkdirSync(join(home, "app"), { recursive: true });
  copyDatabase(target);
  return target;
}

export function readJson(text) {
  try {
    return JSON.parse(text);
  }
  catch {
    return null;
  }
}

export function cli(bin, args, { home, env = {}, timeout = 60_000 } = {}) {
  const started = Date.now();
  try {
    const stdout = execFileSync(bin, args, { encoding: "utf8", timeout, env: { ...process.env, FUTURE_HOME: home, ...env } });
    return { exit: 0, stdout, stderr: "", ms: Date.now() - started, args };
  }
  catch (error) {
    return { exit: error.status ?? 1, stdout: error.stdout ?? "", stderr: error.stderr ?? String(error.message), ms: Date.now() - started, args };
  }
}

export function stopGroup(proc) {
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
}

export function spawnLogged(command, args, { env = {}, logPath, cwd } = {}) {
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
  return child;
}

/**
 * One relay + one host, started on demand.
 *
 * `caseDir` is where every artefact lands: the relay's trust material, the three
 * logs, and the host's own `FUTURE_HOME` (which holds its `app.db`, so a case can
 * assert on what a remote write actually changed).
 */
export function createRelayFixture({ bin, caseDir }) {
  const relay = { dir: join(caseDir, "relay"), api: null, nats: null, ws: null, apiProc: null, natsProc: null };
  const host = { home: seedHome(join(caseDir, "host-home")), proc: null, logPath: join(caseDir, "host.log") };
  seedDatabase(host.home);
  const relayLog = join(caseDir, "relay.log");
  const natsLog = join(caseDir, "nats.log");
  const procs = [];

  const hostEnv = () => ({ FUTURE_HOME: host.home, FUTURE_RELAY_URL: `http://127.0.0.1:${relay.api}` });

  const startRelay = async () => {
    if (relay.api)
      return;
    relay.api = await freePort();
    relay.nats = await freePort();
    relay.ws = await freePort();
    const init = cli(bin, [
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
    procs.push(relay.apiProc, relay.natsProc);
    await waitUntil(async () => {
      try {
        await fetch(`http://127.0.0.1:${relay.api}/`, { signal: AbortSignal.timeout(500) });
        return true;
      }
      catch {
        return false;
      }
    }, { what: `relay API on ${relay.api}`, timeout: 20_000 });
  };

  const startHost = async () => {
    if (host.proc)
      return;
    host.proc = spawnLogged(bin, ["host", "--relay", `http://127.0.0.1:${relay.api}`], {
      logPath: host.logPath,
      env: hostEnv(),
    });
    procs.push(host.proc);
    await waitUntil(() => {
      const status = readJson(cli(bin, ["remote", "status", "--json"], { home: host.home, env: hostEnv() }).stdout);
      return status?.desktopRunning ? status : null;
    }, { what: "host to report its remote status", timeout: 60_000 });
  };

  return {
    relay,
    host,
    hostEnv,
    hostCli: args => cli(bin, args, { home: host.home, env: hostEnv() }),
    async start({ host: withHost = true } = {}) {
      await startRelay();
      if (withHost)
        await startHost();
    },
    /** Restart relay + broker on the same ports, as an operator would. */
    async restartRelay() {
      stopGroup(relay.apiProc);
      stopGroup(relay.natsProc);
      await sleep(1200);
      relay.natsProc = spawnLogged("nats-server", ["-c", join(relay.dir, "nats.conf")], { logPath: natsLog });
      relay.apiProc = spawnLogged(bin, ["relay", "serve", "--config", join(relay.dir, "relay.json")], {
        logPath: relayLog,
        env: { FUTURE_HOME: host.home },
      });
      procs.push(relay.apiProc, relay.natsProc);
      await waitUntil(async () => {
        try {
          await fetch(`http://127.0.0.1:${relay.api}/`, { signal: AbortSignal.timeout(500) });
          return true;
        }
        catch {
          return false;
        }
      }, { what: "relay API after restart", timeout: 20_000 });
    },
    stopHost() {
      stopGroup(host.proc);
      host.proc = null;
    },
    logs() {
      const read = path => (existsSync(path) ? readFileSync(path, "utf8") : "");
      const trimTeardown = (text) => {
        const at = text.search(/received SIGTERM|clean shutdown request|stopped \(reason=headless_sigterm/);
        return at > 0 ? `${text.slice(0, at)}\n[harness] teardown begins here; the log is cut at this point.` : text;
      };
      return {
        host: trimTeardown(read(host.logPath)),
        relay: read(relayLog),
      };
    },
    async stop() {
      stopGroup(host.proc);
      for (const proc of procs.slice().reverse())
        stopGroup(proc);
      await sleep(400);
    },
  };
}

export { REPO_ROOT, CLI_HOME };
