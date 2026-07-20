/**
 * Safari WebDriver backend contract tests.
 *
 * Run when Safari remote automation is enabled:
 *   safaridriver --enable
 *   cd future-os/cli && bun test src/browser/__tests__/validation/contract-safari.test.ts --timeout 120000
 *
 * These run the SAME contract suite as Chromium CDP, validating
 * SafariSession implements BrowserSession correctly.
 */
import { describe, test, beforeAll, afterAll } from "bun:test";
import { platform } from "node:os";
import { existsSync } from "node:fs";
import { createTestIsolation } from "../isolation.js";
import { SafariManager } from "../../safari/safari-manager.js";
import { SafariSession } from "../../safari/safari-session.js";
import { DEFAULT_TIMEOUTS } from "../../types.js";
import { getContractTests } from "./contract-runner.js";
import { RUN_BROWSER_TESTS } from "../browser-opt-in.js";

// ── Safari availability ─────────────────────────────────────────────

async function canUseSafari(): Promise<boolean> {
  if (!RUN_BROWSER_TESTS) return false; // opt-in gate: keep raw `bun test` fast
  if (platform() !== "darwin") return false;
  if (!existsSync("/usr/bin/safaridriver")) return false;

  // Check if safaridriver is responsive (implies remote automation is enabled)
  try {
    const resp = await fetch("http://127.0.0.1:4444/status", { signal: AbortSignal.timeout(2000) });
    if (resp.ok) return true;
  } catch { /* */ }

  // Try to start temporarily to check
  try {
    const { spawn } = await import("node:child_process");
    const child = spawn("/usr/bin/safaridriver", ["--port", "4445"], {
      detached: true,
      stdio: "ignore",
      timeout: 5000,
    });
    const code = await new Promise<number>((r) => child.on("close", r));
    if (code === 0) return true;
  } catch { /* */ }

  return false;
}

// ── Test suite ──────────────────────────────────────────────────────

describe("Safari WebDriver backend", () => {
  let iso: Awaited<ReturnType<typeof createTestIsolation>> | null = null;
  let safariAvailable = false;

  beforeAll(async () => {
    safariAvailable = await canUseSafari();
    if (!safariAvailable) {
      console.log("  [contract:Safari] Safari remote automation not available — skipping");
      return;
    }
    iso = await createTestIsolation();
  }, 15_000);

  afterAll(async () => {
    if (iso) await iso.cleanup();
  });

  for (const [name, fn] of getContractTests()) {
    test(name, async () => {
      if (!safariAvailable) {
        console.log(`  [skip] ${name}`);
        return;
      }

      // Start safaridriver + create session fresh per test (each test independent)
      const mgr = new SafariManager();
      const startResult = await mgr.start({ port: 4444 });

      if (startResult.connection.protocol !== "webdriver" || !startResult.connection.sessionId) {
        throw new Error("Failed to create Safari WebDriver session");
      }

      const session = new SafariSession({
        protocol: "webdriver",
        browserKind: "safari",
        endpoint: startResult.connection.endpoint,
        sessionId: startResult.connection.sessionId,
        timeouts: DEFAULT_TIMEOUTS,
      });

      try {
        await fn(
          { session, currentTitle: async () => "", currentUrl: async () => "" },
          startResult.connection.endpoint,
        );
      } finally {
        await session.disconnect().catch(() => {});
        // Delete session (fresh start per test)
        await fetch(`http://127.0.0.1:4444/session/${startResult.connection.sessionId}`, {
          method: "DELETE",
        }).catch(() => {});
      }
    }, 20_000);
  }
});
