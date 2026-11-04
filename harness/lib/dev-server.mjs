/**
 * Dev-server lifecycle for the desktop shot harness.
 *
 * The page under test is the real frontend built by `vite.shot.config.ts`; the
 * harness only replaces the Tauri boundary. If a server is already listening on
 * the port it is reused and left running, which is the normal interactive case.
 */
import { spawn, execFileSync } from "node:child_process";
import { existsSync, readFileSync } from "node:fs";
import { createServer as createHttpServer } from "node:http";
import { createServer, Socket } from "node:net";
import { resolve } from "node:path";

const sleep = ms => new Promise(done => setTimeout(done, ms));

export const REPO_ROOT = resolve(import.meta.dirname, "..", "..");
export const DEFAULT_URL = "http://localhost:5299/shot/";
export const MOBILE_URL = "http://localhost:8099/";
export const ASSETS_PORT = 7392;
export const TERMINAL_PORT = 7391;
/** The mobile workspace, overridable because a worktree may not have node_modules. */
export const MOBILE_ROOT = process.env.FUTURE_MOBILE_ROOT || resolve(REPO_ROOT, "mobile");

/**
 * The main checkout root, for files git does not carry.
 *
 * `desktop/shot/assets/*.png` are generated demo figures and `.gitignore`d, and
 * `node_modules` is installed per checkout — so a fresh worktree has neither.
 * The figures are inputs to the mocks (not branch code), so falling back to the
 * main checkout's copy is correct; without it every mobile/remote run died with
 * "demo assets missing" after a worktree checkout.
 */
export function mainCheckout() {
  try {
    const commonDir = execFileSync("git", ["rev-parse", "--path-format=absolute", "--git-common-dir"], { cwd: REPO_ROOT, encoding: "utf8" }).trim();
    return resolve(commonDir, "..");
  }
  catch {
    return REPO_ROOT;
  }
}

async function reachable(url, timeout = 1200) {
  try {
    const response = await fetch(url, { signal: AbortSignal.timeout(timeout) });
    return response.ok;
  }
  catch {
    return false;
  }
}

/**
 * A port nobody is listening on.
 *
 * Reusing a fixed port is how a run ends up testing **another worktree's code**:
 * `:5299` may be held by a dev server started from a different branch, and the
 * page then shows that branch's UI while the case believes it tests this one.
 * The child-fix session hit exactly this (a run passed/failed on someone else's
 * build). Default = a fresh port per run; `--reuse-server` opts back in.
 */
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

/**
 * Stop a server we started, and the process group it may have spawned.
 *
 * `npm exec` is a wrapper: killing it leaves the real `vite`/`expo` node process
 * running as an orphan, and repeated runs then pile up servers nobody can
 * attribute (measured: three Expo instances and a Vite on the machine at once,
 * after which a mobile run looked up a contended Metro cache and never became
 * ready). Everything is spawned detached so the whole group can be signalled.
 */
function stopServer(child) {
  if (!child || child.exitCode != null)
    return;
  try {
    if (process.platform === "win32")
      child.kill("SIGTERM");
    else
      process.kill(-child.pid, "SIGTERM");
  }
  catch {
    try {
      child.kill("SIGTERM");
    }
    catch {}
  }
}

export async function ensureDevServer({ port = null, reuse = false } = {}) {
  const chosen = reuse ? (port ?? 5299) : (port ?? await freePort());
  const url = `http://localhost:${chosen}/shot/`;
  if (reuse && await reachable(url))
    return { url, reused: true, stop: async () => {} };

  const desktop = resolve(REPO_ROOT, "desktop");
  if (!existsSync(resolve(desktop, "vite.shot.config.ts")))
    throw new Error(`shot harness config missing under ${desktop}`);

  const child = spawn("npm", ["exec", "--", "vite", "--config", "vite.shot.config.ts", "--port", String(chosen)], {
    cwd: desktop,
    stdio: ["ignore", "pipe", "pipe"],
    detached: process.platform !== "win32",
    env: { ...process.env, SHOT_PORT: String(chosen) },
  });
  let output = "";
  child.stdout.on("data", chunk => (output += chunk));
  child.stderr.on("data", chunk => (output += chunk));
  process.once("exit", () => stopServer(child));

  for (let i = 0; i < 150; i += 1) {
    if (await reachable(url))
      return { url, port: chosen, reused: false, stop: async () => stopServer(child) };
    if (child.exitCode != null)
      throw new Error(`shot dev server exited early:\n${output.slice(-2000)}`);
    await sleep(200);
  }
  stopServer(child);
  throw new Error(`shot dev server did not become ready at ${url}\n${output.slice(-2000)}`);
}

/** Expo web with the mobile shot mocks (`SHOT_WEB=1`). Slower to boot than Vite. */
export async function ensureMobileServer({ port = null, reuse = false, root = MOBILE_ROOT } = {}) {
  const chosen = reuse ? (port ?? 8099) : (port ?? await freePort());
  const url = `http://localhost:${chosen}/`;
  if (reuse && await reachable(url))
    return { url, reused: true, stop: async () => {} };
  if (!existsSync(resolve(root, "package.json")))
    throw new Error(`mobile workspace not found: ${root}`);
  // Metro watches `node_modules` and dies with ENOENT if it is missing — in a
  // fresh git worktree the npm workspaces root has none. That surfaced as a hang
  // (the page never became ready) rather than as "install the deps", so check
  // first and say what to run.
  const depsRoot = resolve(root, "..");
  if (!existsSync(resolve(depsRoot, "node_modules")) && !existsSync(resolve(root, "node_modules")))
    throw new Error(`mobile surface needs installed dependencies in ${depsRoot}\nrun: npm install  (in ${depsRoot})`);
  // Expo web needs `react-native-web`, which this repo consumes but does not
  // declare: it sits in the main checkout's node_modules and is absent from the
  // lockfile, so a fresh checkout fails inside Metro with an
  // `UnableToResolveError` served as HTTP 500 — a white page, which reads as
  // "the app is blank", the most misleading shape a failure can take. Name it.
  const missing = ["react-native-web"].filter(name =>
    !existsSync(resolve(depsRoot, "node_modules", name)) && !existsSync(resolve(root, "node_modules", name)));
  if (missing.length)
    throw new Error(`mobile web surface needs ${missing.join(", ")} in ${depsRoot}\nnot declared by the repo; install with: npm install --no-save ${missing.join(" ")}\nor point FUTURE_MOBILE_ROOT at a checkout that has it`);

  const child = spawn("npm", ["exec", "--", "expo", "start", "--web", "--port", String(chosen)], {
    cwd: root,
    stdio: ["ignore", "pipe", "pipe"],
    detached: process.platform !== "win32",
    env: { ...process.env, SHOT_WEB: "1" },
  });
  let output = "";
  child.stdout.on("data", chunk => (output += chunk));
  child.stderr.on("data", chunk => (output += chunk));
  process.once("exit", () => stopServer(child));

  // A bounded wait with a short probe. The old loop could sit for 20 minutes:
  // 600 iterations of a 2s probe plus a 200ms sleep, on a Metro that was never
  // going to answer, and reported nothing until the caller's own timeout fired.
  const deadline = Date.now() + 180_000;
  while (Date.now() < deadline) {
    if (await reachable(url, 800))
      return { url, port: chosen, reused: false, stop: async () => stopServer(child) };
    if (child.exitCode != null)
      throw new Error(`mobile dev server exited early:\n${output.slice(-3000)}`);
    await sleep(300);
  }
  stopServer(child);
  throw new Error(`mobile dev server did not become ready at ${url} within 180s\n${output.slice(-3000)}`);
}

/**
 * Demo figures for the mobile mocks.
 *
 * `mobile/shot/mock` hardcodes `http://127.0.0.1:7392/<figure>.png`; without
 * this server every in-message image renders as "reload image" and a vision
 * pass reads that as a product defect. Desktop figures are served by Vite's
 * `/shot/assets/`, so only the mobile/remote surfaces need it.
 */
export async function ensureAssetServer({ port = ASSETS_PORT } = {}) {
  const probe = `http://127.0.0.1:${port}/effect-size.png`;
  if (await reachable(probe))
    return { url: `http://127.0.0.1:${port}`, reused: true, stop: async () => {} };

  const candidates = [
    resolve(REPO_ROOT, "desktop", "shot", "assets"),
    resolve(mainCheckout(), "desktop", "shot", "assets"),
  ];
  const root = candidates.find(dir => existsSync(dir));
  if (!root)
    throw new Error(`demo assets missing (looked in ${candidates.join(", ")})`);

  const server = createHttpServer((req, res) => {
    const name = decodeURIComponent(String(req.url ?? "/").split("?")[0]).replace(/^\/+/, "");
    if (!/^[\w.-]+\.png$/.test(name)) {
      res.writeHead(404);
      res.end("not found");
      return;
    }
    try {
      const body = readFileSync(resolve(root, name));
      res.writeHead(200, { "content-type": "image/png", "cache-control": "no-store" });
      res.end(body);
    }
    catch {
      res.writeHead(404);
      res.end("not found");
    }
  });
  await new Promise((done, reject) => {
    server.once("error", reject);
    server.listen(port, "127.0.0.1", done);
  });
  return { url: `http://127.0.0.1:${port}`, reused: false, stop: () => new Promise(done => server.close(done)) };
}

/** TCP probe for the terminal stand-in (it answers raw PTY bytes, not HTTP). */
function tcpOpen(port, timeout = 300) {
  return new Promise((done) => {
    const client = new Socket();
    client.setTimeout(timeout);
    client.once("connect", () => { client.destroy(); done(true); });
    client.once("timeout", () => { client.destroy(); done(false); });
    client.once("error", () => done(false));
    client.connect(port, "127.0.0.1");
  });
}

/**
 * The embedded terminal's stand-in PTY server.
 *
 * `desktop/shot/mock/backend.ts` reports `http://127.0.0.1:7391` as the terminal
 * endpoint. Without it, opening the terminal panel records three connection
 * failures and the vision pass reads a real product defect; the panel itself
 * falls back to a canned frame, hiding the gap.
 */
export async function ensureTerminalServer({ port = TERMINAL_PORT } = {}) {
  if (await tcpOpen(port))
    return { url: `http://127.0.0.1:${port}`, reused: true, stop: async () => {} };
  const script = resolve(REPO_ROOT, "desktop", "shot", "terminal-server.mjs");
  if (!existsSync(script))
    throw new Error(`terminal server missing: ${script}`);
  const child = spawn("node", [script, String(port)], { stdio: ["ignore", "pipe", "pipe"] });
  for (let i = 0; i < 50; i += 1) {
    if (await tcpOpen(port))
      return { url: `http://127.0.0.1:${port}`, reused: false, stop: async () => child.kill("SIGTERM") };
    if (child.exitCode != null)
      throw new Error("terminal server exited early");
    await sleep(100);
  }
  child.kill("SIGTERM");
  throw new Error(`terminal server did not listen on ${port}`);
}
