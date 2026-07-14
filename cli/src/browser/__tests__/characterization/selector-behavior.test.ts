/**
 * Characterization tests for Playwright behaviors the CDP backend must replicate.
 *
 * Run outside sandbox: cd future-os/cli && bun test src/browser/__tests__/characterization/
 *
 * These lock in the exact Playwright API behaviors before any refactoring.
 */
import { test, expect, beforeAll, afterAll, type TestOptions } from "bun:test";
import { platform } from "node:os";
import { launchTestBrowser, killTestBrowser, type BrowserTestContext } from "../test-browser.js";
import { createTestIsolation } from "../isolation.js";
import { getFixture } from "../fixtures/pages.js";

// ── State ───────────────────────────────────────────────────────────

let ctx: BrowserTestContext | null = null;
let iso: Awaited<ReturnType<typeof createTestIsolation>> | null = null;
let page: import("playwright-core").Page | null = null;

beforeAll(async () => {
  // Detect if we can actually launch Chrome
  try {
    const { spawn } = await import("node:child_process");
    const chromePath = platform() === "darwin"
      ? "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome"
      : "google-chrome";
    const child = spawn(chromePath, ["--version"], { timeout: 5000, stdio: "pipe" });
    const code = await new Promise<number>((r) => { child.on("close", r); child.on("error", () => r(1)); });
    if (code !== 0) {
      console.log("  [char] Browser binary exits non-zero — skipping browser tests");
      return;
    }
  } catch {
    console.log("  [char] Cannot spawn chrome — skipping browser tests");
    return;
  }

  iso = await createTestIsolation();
  try {
    ctx = await launchTestBrowser(iso.tempRoot);
    iso.trackPid(ctx.pid);
    page = await ctx.browser.newPage();
    await page.setViewportSize({ width: 1280, height: 800 });
  } catch (e) {
    console.log(`  [char] Browser launch failed: ${(e as Error).message}`);
  }
}, 20_000);

afterAll(async () => {
  if (page) await page.close().catch(() => {});
  if (ctx) {
    await ctx.browser.close().catch(() => {});
    killTestBrowser(ctx.pid);
  }
  if (iso) await iso.cleanup();
});

/** Get the test page, or throw if browser not available. */
function p(): import("playwright-core").Page {
  if (!page) throw new Error("Browser not available — run outside sandbox");
  return page;
}

/** Mark test: browser required (skip if unavailable) */
function bt(name: string, fn: () => Promise<void>, options?: number | TestOptions): void {
  test(name, async () => {
    if (!page) {
      console.log(`  [skip] ${name}`);
      return;
    }
    await fn();
  }, options);
}

// ═══════════════════════════════════════════════════════════════════
// locator.count()
// ═══════════════════════════════════════════════════════════════════

bt("count: single match returns 1", async () => {
  await p().setContent(getFixture("basic"));
  expect(await p().locator("#btn-submit").count()).toBe(1);
});

bt("count: no match returns 0", async () => {
  await p().setContent(getFixture("basic"));
  expect(await p().locator("#nonexistent").count()).toBe(0);
});

bt("count: multiple matches returns actual count (3)", async () => {
  await p().setContent(getFixture("multiple-matches"));
  expect(await p().locator(".btn-duplicate").count()).toBe(3);
});

bt("count: invalid CSS selector throws with 'selector' in message", async () => {
  await p().setContent(getFixture("basic"));
  try {
    await p().locator("!!invalid!!").count();
    expect.unreachable("Should have thrown");
  } catch (e) {
    expect((e as Error).message).toMatch(/selector|parsing|unexpected/i);
  }
});

bt("count: returns 0 after element removed from DOM", async () => {
  await p().setContent(`<html><body>
    <button id="tmp">Temp</button>
    <script>setTimeout(() => document.getElementById("tmp")?.remove(), 100)</script>
  </body></html>`);
  expect(await p().locator("#tmp").count()).toBe(1);
  await p().waitForTimeout(300);
  expect(await p().locator("#tmp").count()).toBe(0);
});

// ═══════════════════════════════════════════════════════════════════
// locator.click() strictness
// ═══════════════════════════════════════════════════════════════════

bt("click: single match succeeds", async () => {
  await p().setContent(getFixture("basic"));
  await p().locator("#btn-submit").click();
});

bt("click: zero matches throws", async () => {
  await p().setContent(getFixture("basic"));
  try {
    await p().locator("#nonexistent").click();
    expect.unreachable("Should have thrown");
  } catch (e) {
    expect((e as Error).message).toMatch(/locator|element|selector/i);
  }
});

bt("click: multiple matches → STRICT MODE VIOLATION", async () => {
  await p().setContent(getFixture("multiple-matches"));
  try {
    await p().locator(".btn-duplicate").click();
    expect.unreachable("Should have thrown");
  } catch (e) {
    expect((e as Error).message).toMatch(/strict|resolve.*\d+.*elements/i);
  }
});

bt("click: .first() on multiple matches succeeds", async () => {
  await p().setContent(getFixture("multiple-matches"));
  await p().locator(".btn-duplicate").first().click();
  expect(await p().locator(".btn-duplicate").first().textContent()).toBe("First");
});

// ═══════════════════════════════════════════════════════════════════
// locator auto-wait
// ═══════════════════════════════════════════════════════════════════

bt("auto-wait: click waits for element to appear (2s delay)", async () => {
  await p().setContent(getFixture("delayed-element"));
  await p().locator("#btn-late").click({ timeout: 5000 });
  expect(await p().locator("#btn-late").textContent()).toBe("Clicked!");
});

bt("auto-wait: click times out if element never appears (500ms timeout)", async () => {
  await p().setContent(getFixture("delayed-element"));
  try {
    await p().locator("#btn-late").click({ timeout: 500 });
    expect.unreachable("Should have thrown");
  } catch (e) {
    expect((e as Error).message).toMatch(/timeout/i);
  }
});

bt("auto-wait: click waits for display:none → block", async () => {
  await p().setContent(`<html><body>
    <button id="h" style="display:none">H</button>
    <script>setTimeout(() => {document.getElementById("h").style.display="block"}, 500)</script>
  </body></html>`);
  await p().locator("#h").click({ timeout: 3000 });
});

bt("auto-wait: click waits for non-zero size", async () => {
  await p().setContent(`<html><body>
    <button id="z" style="width:0;height:0;overflow:hidden">Z</button>
    <script>setTimeout(() => {
      const b=document.getElementById("z"); b.style.width="100px"; b.style.height="40px";
    }, 500)</script>
  </body></html>`);
  await p().locator("#z").click({ timeout: 3000 });
});

// ═══════════════════════════════════════════════════════════════════
// locator.fill()
// ═══════════════════════════════════════════════════════════════════

bt("fill: empty input replaces value", async () => {
  await p().setContent(getFixture("form"));
  await p().locator("#input-text").fill("hello");
  expect(await p().locator("#input-text").inputValue()).toBe("hello");
});

bt("fill: pre-filled input replaces existing value", async () => {
  await p().setContent(getFixture("form"));
  await p().locator("#input-prefilled").fill("new value");
  expect(await p().locator("#input-prefilled").inputValue()).toBe("new value");
});

bt("fill: textarea works", async () => {
  await p().setContent(getFixture("form"));
  await p().locator("#textarea").fill("textarea content");
  expect(await p().locator("#textarea").inputValue()).toBe("textarea content");
});

bt("fill: contenteditable works", async () => {
  await p().setContent(getFixture("form"));
  await p().locator("#contenteditable").fill("editable content");
  expect(await p().locator("#contenteditable").textContent()).toBe("editable content");
});

bt("fill: empty string clears input", async () => {
  await p().setContent(getFixture("form"));
  await p().locator("#input-prefilled").fill("");
  expect(await p().locator("#input-prefilled").inputValue()).toBe("");
});

bt("fill: zero matches throws", async () => {
  await p().setContent(getFixture("form"));
  try {
    await p().locator("#nonexistent").fill("test");
    expect.unreachable("Should have thrown");
  } catch (e) {
    expect((e as Error).message).toMatch(/locator|element|selector/i);
  }
});

bt("fill: multiple matches → strict mode violation", async () => {
  await p().setContent(`<html><body><input class="d"><input class="d"></body></html>`);
  try {
    await p().locator(".d").fill("test");
    expect.unreachable("Should have thrown");
  } catch (e) {
    expect((e as Error).message).toMatch(/strict/i);
  }
});

// ═══════════════════════════════════════════════════════════════════
// locator.type()
// ═══════════════════════════════════════════════════════════════════

bt("type: appends to existing value (does NOT clear)", async () => {
  await p().setContent(getFixture("form"));
  await p().locator("#input-text").fill("prefix-");
  await p().locator("#input-text").type("suffix");
  expect(await p().locator("#input-text").inputValue()).toBe("prefix-suffix");
});

bt("type: on empty input sets value", async () => {
  await p().setContent(getFixture("form"));
  await p().locator("#input-text").type("hello world");
  expect(await p().locator("#input-text").inputValue()).toBe("hello world");
});

bt("type: zero matches throws", async () => {
  await p().setContent(getFixture("form"));
  try {
    await p().locator("#nonexistent").type("test");
    expect.unreachable("Should have thrown");
  } catch (e) {
    expect((e as Error).message).toMatch(/locator|element|selector/i);
  }
});

// ═══════════════════════════════════════════════════════════════════
// page.evaluate()
// ═══════════════════════════════════════════════════════════════════

bt("evaluate: returns string", async () => {
  await p().setContent(getFixture("basic"));
  expect(await p().evaluate(() => document.title)).toBe("Basic Page");
});

bt("evaluate: returns object", async () => {
  await p().setContent(getFixture("basic"));
  const r = await p().evaluate(() => ({ title: document.title }));
  expect(r.title).toBe("Basic Page");
});

bt("evaluate: returns null", async () => {
  await p().setContent(getFixture("basic"));
  expect(await p().evaluate(() => null)).toBeNull();
});

bt("evaluate: returns undefined as null", async () => {
  await p().setContent(getFixture("basic"));
  expect(await p().evaluate(() => undefined)).toBeNull();
});

bt("evaluate: throws on page-side error", async () => {
  await p().setContent(getFixture("basic"));
  try {
    await p().evaluate(() => { throw new Error("boom"); });
    expect.unreachable("Should have thrown");
  } catch (e) {
    expect((e as Error).message).toContain("boom");
  }
});

bt("evaluate: passes arguments", async () => {
  await p().setContent(getFixture("basic"));
  const r = await p().evaluate(
    (sel: string) => document.querySelector(sel)?.textContent,
    "#greeting",
  );
  expect(r).toBe("Hello, world!");
});

// ═══════════════════════════════════════════════════════════════════
// page.goto() and page.waitForLoadState()
// ═══════════════════════════════════════════════════════════════════

bt("goto: about:blank succeeds with domcontentloaded", async () => {
  const resp = await p().goto("about:blank", { waitUntil: "domcontentloaded" });
  expect(resp).not.toBeNull();
});

bt("waitForLoadState: already-loaded page returns instantly", async () => {
  await p().goto("about:blank", { waitUntil: "load" });
  const start = Date.now();
  await p().waitForLoadState("domcontentloaded");
  expect(Date.now() - start).toBeLessThan(1000);
});

bt("waitForLoadState: networkidle timeout=1 throws", async () => {
  await p().setContent(getFixture("basic"));
  try {
    await p().waitForLoadState("networkidle", { timeout: 1 });
    expect.unreachable("Should have thrown");
  } catch (e) {
    expect((e as Error).message).toMatch(/timeout/i);
  }
});

bt("waitForLoadState: SPA click with .catch(() => undefined)", async () => {
  // The pattern from browser-tools.ts:
  //   await page.waitForLoadState("domcontentloaded", { timeout: 3000 }).catch(() => undefined)
  await p().setContent(getFixture("spa"));
  await p().click("#btn-spa-update");
  const result = await p().waitForLoadState("domcontentloaded", { timeout: 3000 })
    .then(() => "loaded")
    .catch(() => "timeout-swallowed");
  console.log(`[CHARACTERIZATION] SPA click → waitForLoadState("domcontentloaded", 3s) = ${result}`);
});

// ═══════════════════════════════════════════════════════════════════
// Default timeouts
// ═══════════════════════════════════════════════════════════════════

bt("setDefaultTimeout affects locator.click()", async () => {
  await p().setContent(getFixture("delayed-element"));
  p().setDefaultTimeout(1000);
  try {
    await p().locator("#btn-late").click(); // 1s timeout, appears at 2s
    expect.unreachable("Should have thrown");
  } catch (e) {
    expect((e as Error).message).toMatch(/timeout/i);
  }
  p().setDefaultTimeout(5000);
});

// ═══════════════════════════════════════════════════════════════════
// Shadow DOM
// ═══════════════════════════════════════════════════════════════════

bt("Shadow DOM: native querySelector cannot find shadow elements", async () => {
  await p().setContent(getFixture("shadow-dom"));
  const found = await p().evaluate(() => document.querySelector("#shadow-btn"));
  expect(found).toBeNull();
});

bt("Shadow DOM: Playwright locator CSS selector behavior", async () => {
  await p().setContent(getFixture("shadow-dom"));
  const count = await p().locator("#shadow-btn").count();
  console.log(`[CHARACTERIZATION] Shadow #shadow-btn locator.count() = ${count}`);
});

bt("Shadow DOM: querySelectorAll snapshot candidates", async () => {
  await p().setContent(getFixture("shadow-dom"));
  const result = await p().evaluate(() => {
    return Array.from(document.querySelectorAll("button, a, input"))
      .map(el => ({ tag: el.tagName, id: el.id }));
  });
  console.log(`[CHARACTERIZATION] querySelectorAll candidates: ${JSON.stringify(result)}`);
});
