/**
 * Playwright baseline contract tests.
 *
 * Run WITHIN sandbox (or headless CI):
 *   cd future-os/cli && bun test src/browser/__tests__/validation/playwright-baseline.test.ts --timeout 120000
 *
 * These create PlaywrightAdapterSessions and run the contract suite.
 * The output is the REFERENCE for Chromium validation.
 */
import { test, beforeAll, afterAll } from "bun:test";
import { platform } from "node:os";
import { existsSync } from "node:fs";
import { createTestIsolation } from "../isolation.js";
import { createContractHarness, type ContractTestHarness } from "./contract-harness.js";
import { getContractTests, type ContractTestFn } from "./contract-runner.js";

// ── State ───────────────────────────────────────────────────────────

let harness: ContractTestHarness | null = null;
let iso: Awaited<ReturnType<typeof createTestIsolation>> | null = null;

beforeAll(async () => {
  // Quick can-launch check
  const chromePath = platform() === "darwin"
    ? "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome"
    : "google-chrome";
  if (!existsSync(chromePath)) {
    console.log("  [contract:playwright] No Chrome — skipping");
    return;
  }

  iso = await createTestIsolation();
  try {
    harness = await createContractHarness(iso.tempRoot);
  } catch (e) {
    console.log(`  [contract:playwright] Browser unavailable: ${(e as Error).message}`);
  }
}, 30_000);

afterAll(async () => {
  if (harness) harness.cleanup();
  if (iso) await iso.cleanup();
});

// ── Register tests ──────────────────────────────────────────────────

for (const [name, fn] of getContractTests()) {
  test(`[PW baseline] ${name}`, async () => {
    if (!harness) {
      console.log(`  [skip] ${name}`);
      return;
    }
    const session = await harness.makePlaywrightSession();
    try {
      await fn({ session, currentTitle: async () => "", currentUrl: async () => "" }, harness.endpoint);
    } finally {
      await session.disconnect().catch(() => {});
    }
  }, 15_000);
}
