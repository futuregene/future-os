/**
 * Contract validation: Playwright adapter vs Chromium CDP backend.
 *
 * Creates SEPARATE Chrome instances for baseline and CDP tests
 * (Chrome's CDP allows only one browser-level WebSocket connection).
 *
 * Run outside sandbox:
 *   cd future-os/cli && bun test src/browser/__tests__/validation/contract.test.ts --timeout 120000
 */
import { describe, test, beforeAll, afterAll } from "bun:test";
import { platform } from "node:os";
import { existsSync } from "node:fs";
import { createTestIsolation } from "../isolation.js";
import { startTestChrome, type SingleBrowser } from "./contract-harness.js";
import { PlaywrightAdapterSession } from "./playwright-adapter.js";
import { DEFAULT_TIMEOUTS } from "../../types.js";
import { ChromiumSession } from "../../chromium/chromium-session.js";
import { getContractTests } from "./contract-runner.js";
import { RUN_BROWSER_TESTS } from "../browser-opt-in.js";

function chromeExists(): boolean {
  if (!RUN_BROWSER_TESTS) return false; // opt-in gate: keep raw `bun test` fast
  const chromePath = platform() === "darwin"
    ? "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome"
    : "google-chrome";
  return existsSync(chromePath);
}

// ═══════════════════════════════════════════════════════════════════
// Playwright adapter (BASELINE)
// ═══════════════════════════════════════════════════════════════════

describe("Playwright baseline", () => {
  let iso: Awaited<ReturnType<typeof createTestIsolation>> | null = null;
  let chrome: SingleBrowser | null = null;

  beforeAll(async () => {
    if (!chromeExists()) {
      console.log("  [contract:PW] No Chrome — skipping");
      return;
    }
    iso = await createTestIsolation();
    try {
      chrome = await startTestChrome(iso.tempRoot);
      console.log(`  [contract:PW] Chrome ready at ${chrome.endpoint}`);
    } catch (e) {
      console.log(`  [contract:PW] ${(e as Error).message}`);
    }
  }, 30_000);

  afterAll(async () => {
    if (chrome) chrome.cleanup();
    if (iso) await iso.cleanup();
  });

  for (const [name, fn] of getContractTests()) {
    test(name, async () => {
      if (!chrome) { console.log(`  [skip] ${name}`); return; }

      const { chromium } = await import("playwright-core");
      const pwBrowser = await chromium.connectOverCDP(chrome.endpoint, { timeout: 10_000 });
      const session = new PlaywrightAdapterSession(pwBrowser);

      try {
        await fn({ session, currentTitle: async () => "", currentUrl: async () => "" }, chrome.endpoint);
      } finally {
        await session.disconnect().catch(() => {});
        if (pwBrowser.isConnected()) await pwBrowser.close().catch(() => {});
      }
    }, 20_000);
  }
});

// ═══════════════════════════════════════════════════════════════════
// Chromium CDP backend (CANDIDATE)
// ═══════════════════════════════════════════════════════════════════

describe("Chromium CDP backend", () => {
  let iso: Awaited<ReturnType<typeof createTestIsolation>> | null = null;
  let chrome: SingleBrowser | null = null;

  beforeAll(async () => {
    if (!chromeExists()) {
      console.log("  [contract:CDP] No Chrome — skipping");
      return;
    }
    iso = await createTestIsolation();
    try {
      chrome = await startTestChrome(iso.tempRoot);
      console.log(`  [contract:CDP] Chrome ready at ${chrome.endpoint}`);
    } catch (e) {
      console.log(`  [contract:CDP] ${(e as Error).message}`);
    }
  }, 30_000);

  afterAll(async () => {
    if (chrome) chrome.cleanup();
    if (iso) await iso.cleanup();
  });

  for (const [name, fn] of getContractTests()) {
    test(name, async () => {
      if (!chrome) { console.log(`  [skip] ${name}`); return; }

      const session = new ChromiumSession({
        protocol: "cdp",
        browserKind: "chrome",
        endpoint: chrome.endpoint,
        timeouts: DEFAULT_TIMEOUTS,
      });

      try {
        await fn({ session, currentTitle: async () => "", currentUrl: async () => "" }, chrome.endpoint);
      } finally {
        await session.disconnect().catch(() => {});
      }
    }, 20_000);
  }
});
