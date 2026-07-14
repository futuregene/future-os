/**
 * Shared test helper: launch Chrome and create sessions for contract validation.
 *
 * Creates both a Playwright connection (for baseline) and a ChromiumSession
 * (for candidate), using the same Chrome process.
 */
import { existsSync } from "node:fs";
import { mkdir } from "node:fs/promises";
import { platform } from "node:os";
import { join } from "node:path";
import { spawn, type ChildProcess } from "node:child_process";
import { createServer } from "node:net";

import type { BrowserSession } from "../../backend.js";
import { DEFAULT_TIMEOUTS } from "../../types.js";
import { ChromiumSession } from "../../chromium/chromium-session.js";
import { PlaywrightAdapterSession } from "./playwright-adapter.js";

// ── Types ────────────────────────────────────────────────────────────

export interface ContractTestHarness {
  endpoint: string;
  port: number;
  pid: number;
  profileDir: string;
  /** Create a Playwright adapter (baseline). */
  makePlaywrightSession(): Promise<BrowserSession>;
  /** Create a ChromiumSession (candidate). */
  makeChromiumSession(): Promise<BrowserSession>;
  /** Kill browser process. */
  cleanup(): void;
}

// ── Chrome discovery ─────────────────────────────────────────────────

function findChrome(): string | null {
  if (platform() === "darwin") {
    const apps = ["Google Chrome", "Microsoft Edge", "Chromium"];
    for (const app of apps) {
      const path = `/Applications/${app}.app/Contents/MacOS/${app}`;
      if (existsSync(path)) return path;
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

// ── Harness ──────────────────────────────────────────────────────────

export async function createContractHarness(tempDir: string): Promise<ContractTestHarness> {
  const chromePath = findChrome();
  if (!chromePath) throw new Error("No Chrome found. Install Chrome to run contract tests.");

  const port = parseInt(process.env["TEST_BROWSER_PORT"] ?? "0", 10) || await findFreePort();
  const profileDir = join(tempDir, "contract-chrome-profile");
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

  // Shared Playwright browser (reused across tests via adapters)
  let pwBrowser: import("playwright-core").Browser | null = null;
  const getPwBrowser = async (): Promise<import("playwright-core").Browser> => {
    if (pwBrowser?.isConnected()) return pwBrowser;
    const { chromium } = await import("playwright-core");
    pwBrowser = await chromium.connectOverCDP(endpoint, { timeout: 10_000 });
    return pwBrowser;
  };

  return {
    endpoint,
    port,
    pid,
    profileDir,
    async makePlaywrightSession(): Promise<BrowserSession> {
      return new PlaywrightAdapterSession(await getPwBrowser());
    },
    async makeChromiumSession(): Promise<BrowserSession> {
      // Each session is independent (fresh CDP connection)
      return new ChromiumSession({
        protocol: "cdp",
        browserKind: "chrome",
        endpoint,
        timeouts: DEFAULT_TIMEOUTS,
      });
    },
    cleanup(): void {
      if (pwBrowser) {
        pwBrowser.close().catch(() => {});
        pwBrowser = null;
      }
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
