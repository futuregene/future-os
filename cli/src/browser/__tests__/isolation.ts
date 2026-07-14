/**
 * Test isolation harness for browser characterization tests.
 *
 * Creates a temporary directory structure:
 *   - $HOME → tempRoot/home
 *   - FUTURE_HOME → tempRoot/future
 *   - Browser config → tempRoot/home/.future/agent/browser/
 *
 * On teardown, kills tracked browser processes and removes temp dir.
 */
import { mkdir, rm } from "node:fs/promises";
import { platform, tmpdir } from "node:os";
import { join } from "node:path";

export interface TestIsolation {
  tempRoot: string;
  homeDir: string;
  futureHomeDir: string;
  configDir: string;
  pids: Set<number>;

  trackPid(pid: number): void;
  cleanup(): Promise<void>;
}

let originalEnv: Record<string, string | undefined> = {};

export async function createTestIsolation(): Promise<TestIsolation> {
  const tempRoot = join(
    tmpdir(),
    `future-browser-test-${Date.now()}-${Math.random().toString(36).slice(2, 8)}`,
  );
  const homeDir = join(tempRoot, "home");
  const futureHomeDir = join(tempRoot, "future");
  const configDir = join(homeDir, ".future", "agent", "browser");

  await mkdir(configDir, { recursive: true });
  await mkdir(futureHomeDir, { recursive: true });

  // Save and override env vars
  originalEnv = {
    HOME: process.env["HOME"],
    FUTURE_HOME: process.env["FUTURE_HOME"],
  };
  process.env["HOME"] = homeDir;
  process.env["FUTURE_HOME"] = futureHomeDir;

  if (platform() === "win32") {
    originalEnv["USERPROFILE"] = process.env["USERPROFILE"];
    originalEnv["LOCALAPPDATA"] = process.env["LOCALAPPDATA"];
    originalEnv["APPDATA"] = process.env["APPDATA"];
    process.env["USERPROFILE"] = homeDir;
    process.env["LOCALAPPDATA"] = join(homeDir, "AppData", "Local");
    process.env["APPDATA"] = join(homeDir, "AppData", "Roaming");
  }

  const pids = new Set<number>();

  return {
    tempRoot,
    homeDir,
    futureHomeDir,
    configDir,
    pids,
    trackPid(pid: number) {
      pids.add(pid);
    },
    async cleanup() {
      // Restore original env vars
      for (const [key, val] of Object.entries(originalEnv)) {
        if (val === undefined) {
          delete process.env[key];
        } else {
          process.env[key] = val;
        }
      }
      originalEnv = {};

      // Kill tracked browser processes
      for (const pid of pids) {
        try {
          if (platform() === "win32") {
            const { execSync } = await import("node:child_process");
            execSync(`taskkill /F /T /PID ${pid} 2>nul`, { timeout: 5000 });
          } else {
            process.kill(-pid, "SIGTERM");
            await new Promise(resolve => setTimeout(resolve, 3000));
            process.kill(-pid, "SIGKILL");
          }
        } catch {
          // Process already gone
        }
      }

      // Remove temp directory
      await rm(tempRoot, { recursive: true, force: true }).catch(() => {});
    },
  };
}
