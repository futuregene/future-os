/**
 * Dev-server lifecycle for the desktop shot harness.
 *
 * The page under test is the real frontend built by `vite.shot.config.ts`; the
 * harness only replaces the Tauri boundary. If a server is already listening on
 * the port it is reused and left running, which is the normal interactive case.
 */
import { spawn } from "node:child_process";
import { existsSync } from "node:fs";
import { resolve } from "node:path";

const sleep = ms => new Promise(done => setTimeout(done, ms));

export const REPO_ROOT = resolve(import.meta.dirname, "..", "..");
export const DEFAULT_URL = "http://localhost:5299/shot/";

async function reachable(url, timeout = 1200) {
  try {
    const response = await fetch(url, { signal: AbortSignal.timeout(timeout) });
    return response.ok;
  }
  catch {
    return false;
  }
}

export async function ensureDevServer({ url = DEFAULT_URL, port = 5299 } = {}) {
  if (await reachable(url))
    return { url, reused: true, stop: async () => {} };

  const desktop = resolve(REPO_ROOT, "desktop");
  if (!existsSync(resolve(desktop, "vite.shot.config.ts")))
    throw new Error(`shot harness config missing under ${desktop}`);

  const child = spawn("npm", ["exec", "--", "vite", "--config", "vite.shot.config.ts", "--port", String(port)], {
    cwd: desktop,
    stdio: ["ignore", "pipe", "pipe"],
    env: { ...process.env, SHOT_PORT: String(port) },
  });
  let output = "";
  child.stdout.on("data", chunk => (output += chunk));
  child.stderr.on("data", chunk => (output += chunk));

  for (let i = 0; i < 150; i += 1) {
    if (await reachable(url))
      return { url, reused: false, stop: async () => child.kill("SIGTERM") };
    if (child.exitCode != null)
      throw new Error(`shot dev server exited early:\n${output.slice(-2000)}`);
    await sleep(200);
  }
  child.kill("SIGTERM");
  throw new Error(`shot dev server did not become ready at ${url}\n${output.slice(-2000)}`);
}
