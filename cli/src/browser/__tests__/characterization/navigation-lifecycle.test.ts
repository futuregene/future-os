/**
 * Characterization tests for navigation, tab lifecycle, screenshot, console hook.
 *
 * Run outside sandbox: cd future-os/cli && bun test src/browser/__tests__/characterization/
 */
import { test, expect, beforeAll, afterAll, type TestOptions } from "bun:test";
import { platform } from "node:os";
import { launchTestBrowser, killTestBrowser, type BrowserTestContext } from "../test-browser.js";
import { createTestIsolation } from "../isolation.js";
import { getFixture } from "../fixtures/pages.js";
import { RUN_BROWSER_TESTS, logBrowserSuiteSkipped } from "../browser-opt-in.js";

let ctx: BrowserTestContext | null = null;
let iso: Awaited<ReturnType<typeof createTestIsolation>> | null = null;
let browser: import("playwright-core").Browser | null = null;
let page: import("playwright-core").Page | null = null;

beforeAll(async () => {
  if (!RUN_BROWSER_TESTS) { logBrowserSuiteSkipped("char"); return; }
  try {
    const { spawn } = await import("node:child_process");
    const chromePath = platform() === "darwin"
      ? "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome"
      : "google-chrome";
    const child = spawn(chromePath, ["--version"], { timeout: 5000, stdio: "pipe" });
    const code = await new Promise<number>((r) => { child.on("close", r); child.on("error", () => r(1)); });
    if (code !== 0) return;
  } catch { return; }

  iso = await createTestIsolation();
  try {
    ctx = await launchTestBrowser(iso.tempRoot);
    iso.trackPid(ctx.pid);
    browser = ctx.browser;
    page = await browser.newPage();
    await page.setViewportSize({ width: 1280, height: 800 });
  } catch (e) { console.log(`  [char] Browser unavailable: ${(e as Error).message}`); }
}, 20_000);

afterAll(async () => {
  if (page) await page.close().catch(() => {});
  if (ctx) {
    await ctx.browser.close().catch(() => {});
    killTestBrowser(ctx.pid);
  }
  if (iso) await iso.cleanup();
});

function p(): import("playwright-core").Page {
  if (!page) throw new Error("Browser not available");
  return page;
}

function b(): import("playwright-core").Browser {
  if (!browser) throw new Error("Browser not available");
  return browser;
}

function bt(name: string, fn: () => Promise<void>, options?: number | TestOptions): void {
  test(name, async () => {
    if (!page) { console.log(`  [skip] ${name}`); return; }
    await fn();
  }, options);
}

// ═══════════════════════════════════════════════════════════════════
// Browser lifecycle
// ═══════════════════════════════════════════════════════════════════

bt("browser.contexts() returns at least one context", async () => {
  const contexts = b().contexts();
  expect(contexts.length).toBeGreaterThanOrEqual(1);
});

bt("context.pages() returns pages in order", async () => {
  const contexts = b().contexts();
  const pages = contexts[0]!.pages();
  // At minimum, our initial page is there
  expect(pages.length).toBeGreaterThanOrEqual(1);
  // The last opened page should be accessible
  expect(pages[pages.length - 1]!).toBeTruthy();
});

bt("new tab via context.newPage() works", async () => {
  const context = b().contexts()[0] ?? await b().newContext();
  const pagesBefore = context.pages().length;
  const newPage = await context.newPage();
  await newPage.goto("about:blank", { waitUntil: "domcontentloaded" });
  expect(context.pages().length).toBe(pagesBefore + 1);
  await newPage.close();
});

bt("new tab via page.click() with target=_blank", async () => {
  const context = b().contexts()[0]!;
  const pagesBefore = context.pages().length;

  // Set content with a link that opens in new tab
  await p().setContent(getFixture("tabs"));
  const newPagePromise = context.waitForEvent("page", { timeout: 5000 });
  await p().click("#link-new-tab");
  const newPage = await newPagePromise;

  expect(context.pages().length).toBe(pagesBefore + 1);
  const title = await newPage.title().catch(() => "");
  console.log(`[CHARACTERIZATION] New tab title: "${title}"`);
  await newPage.close();
});

bt("close active tab — Playwright selects another", async () => {
  const context = b().contexts()[0]!;
  const pages = context.pages();
  if (pages.length < 2) {
    // Create a second page for this test
    const p2 = await context.newPage();
    await p2.goto("about:blank", { waitUntil: "domcontentloaded" });
  }
  const allPages = context.pages();
  const initialCount = allPages.length;

  await allPages[allPages.length - 1]!.close();

  // After closing, remaining pages are still accessible
  const remaining = context.pages();
  expect(remaining.length).toBe(initialCount - 1);
  // The last remaining page is now the active one
  expect(remaining[remaining.length - 1]!).toBeTruthy();
});

// ═══════════════════════════════════════════════════════════════════
// Navigation
// ═══════════════════════════════════════════════════════════════════

bt("click link that navigates — page.url() changes", async () => {
  // Set source page with a link
  await p().setContent(`
    <html><body>
      <a id="nav-link" href="data:text/html,<h1>Target</h1>">Go</a>
    </body></html>
  `);
  const beforeUrl = p().url();
  await p().click("#nav-link");
  // Playwright auto-waits for navigation after click
  const afterUrl = p().url();
  console.log(`[CHARACTERIZATION] Nav: ${beforeUrl} → ${afterUrl}`);
  expect(afterUrl).not.toBe(beforeUrl);
});

bt("click button that does NOT navigate — url stays same", async () => {
  await p().setContent(getFixture("spa"));
  const beforeUrl = p().url();
  await p().click("#btn-spa-update");
  const afterUrl = p().url();
  expect(afterUrl).toBe(beforeUrl);
});

bt("page.title() works after navigation", async () => {
  await p().setContent(getFixture("basic"));
  expect(await p().title()).toBe("Basic Page");
  await p().goto("data:text/html,<title>New Title</title>", { waitUntil: "domcontentloaded" });
  expect(await p().title()).toBe("New Title");
});

bt("page.title() returns empty string for untitled page", async () => {
  await p().goto("data:text/html,<h1>No Title</h1>", { waitUntil: "domcontentloaded" });
  const title = await p().title();
  // Playwright returns empty string for pages without <title>
  expect(title).toBe("");
});

// ═══════════════════════════════════════════════════════════════════
// Screenshot
// ═══════════════════════════════════════════════════════════════════

bt("screenshot with explicit path writes PNG file", async () => {
  const { mkdtempSync, existsSync, statSync, unlinkSync, rmdirSync } = await import("node:fs");
  const { join } = await import("node:path");
  const { tmpdir } = await import("node:os");

  await p().setContent(getFixture("basic"));
  const dir = mkdtempSync(join(tmpdir(), "future-screenshot-"));
  const filePath = join(dir, "test.png");

  await p().screenshot({ path: filePath });

  expect(existsSync(filePath)).toBe(true);
  const st = statSync(filePath);
  expect(st.size).toBeGreaterThan(100); // valid PNG > 100 bytes

  unlinkSync(filePath);
  rmdirSync(dir);
});

bt("screenshot with fullPage option", async () => {
  const { mkdtempSync, existsSync, unlinkSync, rmdirSync } = await import("node:fs");
  const { join } = await import("node:path");
  const { tmpdir } = await import("node:os");

  await p().setContent(getFixture("scroll"));
  const dir = mkdtempSync(join(tmpdir(), "future-screenshot-"));
  const filePath = join(dir, "full.png");

  await p().screenshot({ path: filePath, fullPage: true });

  expect(existsSync(filePath)).toBe(true);
  unlinkSync(filePath);
  rmdirSync(dir);
});

bt("screenshot fails if parent directory does not exist", async () => {
  await p().setContent(getFixture("basic"));
  try {
    await p().screenshot({ path: "/nonexistent-dir-should-not-exist/test.png" });
    expect.unreachable("Should have thrown");
  } catch (e) {
    expect((e as Error).message).toMatch(/ENOENT|no such file|directory/i);
  }
});

// ═══════════════════════════════════════════════════════════════════
// Console hook behavior (page.evaluate-based)
// ═══════════════════════════════════════════════════════════════════

bt("console hook: install via page.evaluate intercepts console.log", async () => {
  const { getFixture } = await import("../fixtures/pages.js");
  await p().setContent(getFixture("console"));

  // Install hook (same pattern as current browser-tools.ts)
  await p().evaluate(() => {
    const target = globalThis as unknown as {
      __futureConsoleHookInstalled?: boolean;
      __futureConsoleLogs?: Array<{ level: string; text: string }>;
      console: Console;
    };
    if (target.__futureConsoleHookInstalled) return;
    target.__futureConsoleHookInstalled = true;
    target.__futureConsoleLogs = [];
    for (const level of ["log", "info", "warn", "error"] as const) {
      const orig = target.console[level].bind(target.console);
      target.console[level] = (...args: unknown[]) => {
        target.__futureConsoleLogs!.push({
          level,
          text: args.map(a => typeof a === "string" ? a : JSON.stringify(a)).join(" "),
        });
        orig(...args);
      };
    }
  });

  // Trigger a console.log
  await p().click("#btn-log");

  // Read the buffered logs
  const logs = await p().evaluate(() => {
    const v = (globalThis as unknown as { __futureConsoleLogs?: unknown }).__futureConsoleLogs;
    return Array.isArray(v) ? v : [];
  });

  console.log(`[CHARACTERIZATION] Console hook captured ${logs.length} logs`);
  expect(logs.length).toBeGreaterThanOrEqual(1); // "page-loaded" + "button-clicked"
});

bt("console hook: survives page navigation (without re-install)", async () => {
  // Install hook
  await p().evaluate(() => {
    const target = globalThis as unknown as {
      __futureConsoleHookInstalled?: boolean;
      __futureConsoleLogs?: unknown[];
      console: Console;
    };
    if (target.__futureConsoleHookInstalled) return;
    target.__futureConsoleHookInstalled = true;
    target.__futureConsoleLogs = [];
    for (const level of ["log", "info", "warn", "error"] as const) {
      const orig = target.console[level].bind(target.console);
      target.console[level] = (...args: unknown[]) => {
        target.__futureConsoleLogs!.push({ level, text: args.join(" ") });
        orig(...args);
      };
    }
  });

  // Navigate away (hook script lost)
  await p().goto("data:text/html,<script>console.log('after-nav')</script>", { waitUntil: "domcontentloaded" });

  // Check logs — the old hook is GONE (navigation resets JS context)
  const logs = await p().evaluate(() => {
    const v = (globalThis as unknown as { __futureConsoleLogs?: unknown }).__futureConsoleLogs;
    return Array.isArray(v) ? v : null;
  });

  console.log(`[CHARACTERIZATION] After nav, hook: ${logs === null ? "GONE" : `present, ${logs.length} logs`}`);
  // This is the KEY finding: Playwright's page.evaluate hook is LOST on navigation.
  // This is why browser-tools.ts re-installs the hook after every goto.
  expect(logs).toBeNull();
});
