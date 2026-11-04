/**
 * Dev-server lifecycle for the desktop shot harness.
 *
 * The page under test is the real frontend built by `vite.shot.config.ts`; the
 * harness only replaces the Tauri boundary. If a server is already listening on
 * the port it is reused and left running, which is the normal interactive case.
 */
import { spawn } from "node:child_process";
import { existsSync } from "node:fs";
import { createServer } from "node:net";
import { resolve } from "node:path";

const sleep = ms => new Promise(done => setTimeout(done, ms));

export const REPO_ROOT = resolve(import.meta.dirname, "..", "..");
export const DEFAULT_URL = "http://localhost:5299/shot/";
export const MOBILE_URL = "http://localhost:8099/";
/** The mobile workspace, overridable because a worktree may not have node_modules. */
export const MOBILE_ROOT = process.env.FUTURE_MOBILE_ROOT || resolve(REPO_ROOT, "mobile");

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
    env: { ...process.env, SHOT_PORT: String(chosen) },
  });
  let output = "";
  child.stdout.on("data", chunk => (output += chunk));
  child.stderr.on("data", chunk => (output += chunk));

  for (let i = 0; i < 150; i += 1) {
    if (await reachable(url))
      return { url, port: chosen, reused: false, stop: async () => child.kill("SIGTERM") };
    if (child.exitCode != null)
      throw new Error(`shot dev server exited early:\n${output.slice(-2000)}`);
    await sleep(200);
  }
  child.kill("SIGTERM");
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

  const child = spawn("npm", ["exec", "--", "expo", "start", "--web", "--port", String(chosen)], {
    cwd: root,
    stdio: ["ignore", "pipe", "pipe"],
    env: { ...process.env, SHOT_WEB: "1" },
  });
  let output = "";
  child.stdout.on("data", chunk => (output += chunk));
  child.stderr.on("data", chunk => (output += chunk));

  for (let i = 0; i < 600; i += 1) {
    if (await reachable(url, 2000))
      return { url, port: chosen, reused: false, stop: async () => child.kill("SIGTERM") };
    if (child.exitCode != null)
      throw new Error(`mobile dev server exited early:\n${output.slice(-3000)}`);
    await sleep(200);
  }
  child.kill("SIGTERM");
  throw new Error(`mobile dev server did not become ready at ${url}\n${output.slice(-3000)}`);
}
