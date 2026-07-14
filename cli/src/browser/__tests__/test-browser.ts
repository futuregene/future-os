/**
 * Test helper: launch a real Chrome browser and connect via Playwright.
 * Uses the same discovery logic as the production code.
 *
 * IMPORTANT: Due to sandbox restrictions on listen(), this helper does NOT
 * probe for free ports. It uses a FIXED port (from TEST_BROWSER_PORT env var,
 * default 9222). Only one test file can use this at a time.
 */
import { existsSync } from "node:fs";
import { mkdir } from "node:fs/promises";
import { platform } from "node:os";
import { join } from "node:path";
import { spawn, type ChildProcess } from "node:child_process";

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
    const programFiles = process.env["PROGRAMFILES"];
    const programFilesX86 = process.env["PROGRAMFILES(X86)"];
    const candidates = [
      local ? join(local, "Google", "Chrome", "Application", "chrome.exe") : "",
      programFiles ? join(programFiles, "Google", "Chrome", "Application", "chrome.exe") : "",
      programFilesX86 ? join(programFilesX86, "Microsoft", "Edge", "Application", "msedge.exe") : "",
      programFiles ? join(programFiles, "Microsoft", "Edge", "Application", "msedge.exe") : "",
    ].filter(Boolean);
    return candidates.find(c => existsSync(c)) ?? null;
  }
  const candidates = [
    process.env["CHROME_PATH"],
    "/usr/bin/google-chrome",
    "/usr/bin/chromium-browser",
    "/usr/bin/chromium",
  ].filter(Boolean) as string[];
  return candidates.find(c => existsSync(c)) ?? null;
}

async function waitForEndpoint(endpoint: string, timeoutMs: number): Promise<void> {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    try {
      const resp = await fetch(`${endpoint}/json/version`, { signal: AbortSignal.timeout(1000) });
      if (resp.ok) return;
    } catch {
      // not ready yet
    }
    await new Promise(resolve => setTimeout(resolve, 250));
  }
  throw new Error(`CDP endpoint ${endpoint} not reachable within ${timeoutMs}ms`);
}

export async function launchTestBrowser(tempDir: string): Promise<BrowserTestContext> {
  const chromePath = findChromePath();
  if (!chromePath) {
    throw new Error("No Chrome/Edge/Chromium found. Install Chrome to run tests.");
  }

  const port = parseInt(process.env["TEST_BROWSER_PORT"] ?? "9222", 10);
  const profileDir = join(tempDir, "chrome-profile");
  await mkdir(profileDir, { recursive: true });

  const endpoint = `http://127.0.0.1:${port}`;

  // If an existing browser is already on this port, try to reuse it
  try {
    const resp = await fetch(`${endpoint}/json/version`, { signal: AbortSignal.timeout(500) });
    if (resp.ok) {
      console.log(`  [test-browser] Reusing existing browser at ${endpoint}`);
      const { chromium } = await import("playwright-core");
      const browser = await chromium.connectOverCDP(endpoint, { timeout: 10_000 });
      return { browser, endpoint, profileDir, pid: -1, port };
    }
  } catch {
    // No existing browser — launch new one
  }

  const child: ChildProcess = spawn(
    chromePath,
    [
      `--remote-debugging-port=${port}`,
      `--user-data-dir=${profileDir}`,
      "--no-first-run",
      "--no-default-browser-check",
      "--headless=new",
      "--disable-gpu",
      "--disable-extensions",
      "about:blank",
    ],
    {
      detached: true,
      stdio: "ignore",
    },
  );

  const pid = child.pid!;
  child.unref();

  await waitForEndpoint(endpoint, 15_000);

  const { chromium } = await import("playwright-core");
  const browser = await chromium.connectOverCDP(endpoint, { timeout: 10_000 });

  return { browser, endpoint, profileDir, pid, port };
}

export function killTestBrowser(pid: number): void {
  if (pid <= 0) return; // Was reused, not launched by us
  try {
    if (platform() === "win32") {
      require("node:child_process").execSync(`taskkill /F /T /PID ${pid} 2>nul`, { timeout: 5000 });
    } else {
      process.kill(-pid, "SIGTERM");
      setTimeout(() => {
        try { process.kill(-pid, "SIGKILL"); } catch { /* ignore */ }
      }, 3000);
    }
  } catch {
    // already gone
  }
}
