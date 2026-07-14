/**
 * Shared test helper: launch Chrome for contract validation.
 *
 * Each call to createContractHarness() starts a FRESH Chrome process
 * on a free port. Caller is responsible for cleanup.
 *
 * Chrome only supports ONE browser-level CDP WebSocket connection.
 * We create separate Chrome instances for Playwright baseline and CDP tests.
 */
import { existsSync } from "node:fs";
import { mkdir } from "node:fs/promises";
import { platform } from "node:os";
import { join } from "node:path";
import { spawn, type ChildProcess } from "node:child_process";
import { createServer } from "node:net";

export interface SingleBrowser {
  endpoint: string;
  port: number;
  pid: number;
  profileDir: string;
  /** Kill the Chrome process and clean up the profile. */
  cleanup(): void;
}

function findChrome(): string | null {
  if (platform() === "darwin") {
    for (const app of ["Google Chrome", "Microsoft Edge", "Chromium"]) {
      const p = `/Applications/${app}.app/Contents/MacOS/${app}`;
      if (existsSync(p)) return p;
    }
    return null;
  }
  return null;
}

function findFreePort(): Promise<number> {
  return new Promise((resolve, reject) => {
    const server = createServer();
    server.unref();
    server.on("error", reject);
    server.listen(0, "127.0.0.1", () => {
      const addr = server.address();
      const port = typeof addr === "object" && addr ? addr.port : 0;
      server.close(() => resolve(port));
    });
  });
}

async function waitForEndpoint(endpoint: string, timeoutMs: number): Promise<void> {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    try {
      const resp = await fetch(`${endpoint}/json/version`, { signal: AbortSignal.timeout(1000) });
      if (resp.ok) return;
    } catch { /* */ }
    await new Promise(resolve => setTimeout(resolve, 250));
  }
  throw new Error(`CDP endpoint ${endpoint} not reachable within ${timeoutMs}ms`);
}

/** Start a fresh headless Chrome and return its endpoint info. */
export async function startTestChrome(tempDir: string): Promise<SingleBrowser> {
  const chromePath = findChrome();
  if (!chromePath) throw new Error("No Chrome found. Install Chrome to run contract tests.");

  const port = parseInt(process.env["TEST_BROWSER_PORT"] ?? "0", 10) || await findFreePort();
  const profileDir = join(tempDir, `chrome-profile-${port}`);
  await mkdir(profileDir, { recursive: true });
  const endpoint = `http://127.0.0.1:${port}`;

  const child: ChildProcess = spawn(chromePath, [
    `--remote-debugging-port=${port}`,
    `--user-data-dir=${profileDir}`,
    "--no-first-run",
    "--no-default-browser-check",
    "--headless=new",
    "--disable-gpu",
    "--disable-extensions",
    "--use-mock-keychain",
    "--disable-features=DialMediaRouteProvider",
    "about:blank",
  ], { detached: true, stdio: "ignore" });

  const pid = child.pid!;
  child.unref();
  await waitForEndpoint(endpoint, 15_000);

  return {
    endpoint,
    port,
    pid,
    profileDir,
    cleanup() {
      if (pid > 0) {
        try {
          if (platform() === "win32") {
            require("node:child_process").execSync(`taskkill /F /T /PID ${pid} 2>nul`, { timeout: 5000 });
          } else {
            process.kill(-pid, "SIGTERM");
            setTimeout(() => { try { process.kill(-pid, "SIGKILL"); } catch { /* */ } }, 3000);
          }
        } catch { /* */ }
      }
    },
  };
}
