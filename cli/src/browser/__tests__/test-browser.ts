/**
 * Test helper: launch a real Chrome browser and connect via Playwright.
 * Uses a free port (or TEST_BROWSER_PORT env var).
 */
import { existsSync } from "node:fs";
import { mkdir } from "node:fs/promises";
import { platform } from "node:os";
import { join } from "node:path";
import { spawn, type ChildProcess } from "node:child_process";
import { createServer } from "node:net";

export interface BrowserTestContext {
  browser: import("playwright-core").Browser;
  endpoint: string;
  profileDir: string;
  pid: number;
  port: number;
}

function findChromePath(): string | null {
  if (platform() === "darwin") {
    const apps = [
      ["Google Chrome", "Google Chrome"],
      ["Microsoft Edge", "Microsoft Edge"],
      ["Chromium", "Chromium"],
    ];
    for (const [appName, executableName] of apps) {
      const executable = `/Applications/${appName}.app/Contents/MacOS/${executableName}`;
      if (existsSync(executable)) return executable;
    }
    return null;
  }
  if (platform() === "win32") {
    const local = process.env["LOCALAPPDATA"];
    const prog = process.env["PROGRAMFILES"];
    const progX86 = process.env["PROGRAMFILES(X86)"];
    return [
      local ? join(local, "Google", "Chrome", "Application", "chrome.exe") : "",
      prog ? join(prog, "Google", "Chrome", "Application", "chrome.exe") : "",
      progX86 ? join(progX86, "Microsoft", "Edge", "Application", "msedge.exe") : "",
      prog ? join(prog, "Microsoft", "Edge", "Application", "msedge.exe") : "",
    ].filter(Boolean).find(c => existsSync(c)) ?? null;
  }
  return [
    process.env["CHROME_PATH"],
    "/usr/bin/google-chrome",
    "/usr/bin/chromium-browser",
    "/usr/bin/chromium",
  ].filter(Boolean).find(c => existsSync(c as string)) ?? null;
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

export async function launchTestBrowser(tempDir: string): Promise<BrowserTestContext> {
  const chromePath = findChromePath();
  if (!chromePath) throw new Error("No Chrome/Edge/Chromium found.");

  const envPort = process.env["TEST_BROWSER_PORT"];
  const port = envPort ? parseInt(envPort, 10) : await findFreePort();
  const profileDir = join(tempDir, "chrome-profile");
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

  const { chromium } = await import("playwright-core");
  const browser = await chromium.connectOverCDP(endpoint, { timeout: 10_000 });
  return { browser, endpoint, profileDir, pid, port };
}

export function killTestBrowser(pid: number): void {
  if (pid <= 0) return;
  try {
    if (platform() === "win32") {
      require("node:child_process").execSync(`taskkill /F /T /PID ${pid} 2>nul`, { timeout: 5000 });
    } else {
      process.kill(-pid, "SIGTERM");
      setTimeout(() => { try { process.kill(-pid, "SIGKILL"); } catch { /* */ } }, 3000);
    }
  } catch { /* */ }
}
