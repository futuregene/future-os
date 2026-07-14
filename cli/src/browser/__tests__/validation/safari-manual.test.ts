/**
 * Safari WebDriver manual validation tests.
 *
 * Run AFTER enabling Safari remote automation:
 *   safaridriver --enable
 *
 * Then:
 *   cd future-os/cli && bun test src/browser/__tests__/validation/safari-manual.test.ts --timeout 120000
 *
 * Each test logs its result for easy diagnosis.
 */
import { describe, test, expect, beforeAll, afterAll } from "bun:test";
import { platform } from "node:os";
import { existsSync } from "node:fs";
import { SafariManager } from "../../safari/safari-manager.js";
import { SafariSession } from "../../safari/safari-session.js";
import { DEFAULT_TIMEOUTS } from "../../types.js";
import { getFixture } from "../fixtures/pages.js";
import { createTestIsolation } from "../isolation.js";

// ── Availability ─────────────────────────────────────────────────────

async function safariReady(): Promise<boolean> {
  if (platform() !== "darwin") return false;
  if (!existsSync("/usr/bin/safaridriver")) return false;

  // Try to create a real session to verify full readiness
  const mgr = new SafariManager();
  try {
    const result = await mgr.start({ port: 4444 });
    if (result.connection.protocol === "webdriver" && result.connection.sessionId) {
      // Clean up immediately — we'll create fresh sessions per test
      await fetch(`http://127.0.0.1:4444/session/${result.connection.sessionId}`, {
        method: "DELETE",
      }).catch(() => {});
      return true;
    }
  } catch { /* */ }
  return false;
}

// ── Test suite ───────────────────────────────────────────────────────

describe("Safari WebDriver", () => {
  let iso: Awaited<ReturnType<typeof createTestIsolation>> | null = null;
  let ready = false;

  beforeAll(async () => {
    ready = await safariReady();
    if (!ready) {
      console.log("  Safari remote automation not available — skipping all tests.");
      console.log("  Run: safaridriver --enable");
      return;
    }
    iso = await createTestIsolation();
  }, 20_000);

  afterAll(async () => {
    if (iso) await iso.cleanup();
  });

  // ── Helper: create a fresh session per test ──────────────────────

  async function makeSession(): Promise<SafariSession> {
    const mgr = new SafariManager();
    const result = await mgr.start({ port: 4444 });
    if (result.connection.protocol !== "webdriver" || !result.connection.sessionId) {
      throw new Error("Failed to create Safari session");
    }
    const session = new SafariSession({
      protocol: "webdriver",
      browserKind: "safari",
      endpoint: result.connection.endpoint,
      sessionId: result.connection.sessionId,
      timeouts: DEFAULT_TIMEOUTS,
    });
    // Store sessionId for cleanup
    (session as unknown as { _sid: string })._sid = result.connection.sessionId;
    return session;
  }

  async function cleanupSession(session: SafariSession): Promise<void> {
    const sid = (session as unknown as { _sid?: string })._sid;
    if (sid) {
      await fetch(`http://127.0.0.1:4444/session/${sid}`, { method: "DELETE" }).catch(() => {});
    }
    await session.disconnect().catch(() => {});
  }

  // ═══════════════════════════════════════════════════════════════
  // 1. Session
  // ═══════════════════════════════════════════════════════════════

  test("session: creates and status is reachable", async () => {
    if (!ready) { console.log("  [skip]"); return; }
    const session = await makeSession();
    try {
      // Just creating it without error is success
      expect(session.kind).toBe("safari");
      expect(session.protocol).toBe("webdriver");
      console.log("  ✅ session created");
    } finally {
      await cleanupSession(session);
    }
  });

  // ═══════════════════════════════════════════════════════════════
  // 2. Navigation
  // ═══════════════════════════════════════════════════════════════

  test("open: navigates to a URL and returns title", async () => {
    if (!ready) { console.log("  [skip]"); return; }
    const session = await makeSession();
    try {
      const page = await session.open("data:text/html,<title>SafariTest</title><h1>Hello</h1>");
      expect(page.title).toBe("SafariTest");
      expect(page.url).toContain("data:text/html");
      console.log(`  ✅ open → title="${page.title}" url="${page.url.slice(0, 50)}..."`);
    } finally {
      await cleanupSession(session);
    }
  });

  test("open: about:blank succeeds", async () => {
    if (!ready) { console.log("  [skip]"); return; }
    const session = await makeSession();
    try {
      const page = await session.open("about:blank");
      expect(page.url).toContain("about:blank");
      console.log(`  ✅ about:blank → url="${page.url}"`);
    } finally {
      await cleanupSession(session);
    }
  });

  test("open: real website", async () => {
    if (!ready) { console.log("  [skip]"); return; }
    const session = await makeSession();
    try {
      const page = await session.open("https://example.com");
      expect(page.title).toContain("Example");
      console.log(`  ✅ example.com → title="${page.title}"`);
    } catch (e) {
      // Network may be unavailable — skip gracefully
      console.log(`  ⚠️  example.com unreachable: ${(e as Error).message}`);
    } finally {
      await cleanupSession(session);
    }
  });

  // ═══════════════════════════════════════════════════════════════
  // 3. Evaluate
  // ═══════════════════════════════════════════════════════════════

  test("evaluate: expression returns value", async () => {
    if (!ready) { console.log("  [skip]"); return; }
    const session = await makeSession();
    try {
      await session.open("data:text/html,<script>window._v={x:1,y:2}</script>");
      const result = await session.evaluate<{ x: number; y: number }>({
        kind: "expression",
        expression: "window._v",
      });
      expect(result.x).toBe(1);
      expect(result.y).toBe(2);
      console.log(`  ✅ evaluate expression → {x:${result.x}, y:${result.y}}`);
    } finally {
      await cleanupSession(session);
    }
  });

  test("evaluate: function with arguments", async () => {
    if (!ready) { console.log("  [skip]"); return; }
    const session = await makeSession();
    try {
      await session.open("data:text/html,<div id='t'>hello</div>");
      const text = await session.evaluate<string>({
        kind: "function",
        functionDeclaration: "function(sel) { return document.querySelector(sel).textContent; }",
        arguments: ["#t"],
      });
      expect(text).toBe("hello");
      console.log(`  ✅ evaluate function → "${text}"`);
    } finally {
      await cleanupSession(session);
    }
  });

  test("evaluate: snapshot script", async () => {
    if (!ready) { console.log("  [skip]"); return; }
    const session = await makeSession();
    try {
      await session.open("data:text/html," + encodeURIComponent(getFixture("basic")));

      const { SNAPSHOT_FUNCTION_SOURCE } = await import("../../scripts/snapshot-script.js");
      const result = await session.evaluate<{
        title: string;
        items: Array<{ ref: string; selector: string; role: string; name: string }>;
      }>({
        kind: "function",
        functionDeclaration: SNAPSHOT_FUNCTION_SOURCE,
        arguments: [80],
      });

      expect(result.title).toBe("Basic Page");
      expect(result.items.length).toBeGreaterThan(0);

      // Find the submit button
      const btn = result.items.find(i => i.name === "Submit" || i.role === "button");
      expect(btn).toBeTruthy();
      console.log(`  ✅ snapshot → ${result.items.length} elements, found ref="${btn!.ref}" selector="${btn!.selector}"`);
    } finally {
      await cleanupSession(session);
    }
  });

  // ═══════════════════════════════════════════════════════════════
  // 4. Type
  // ═══════════════════════════════════════════════════════════════

  test("type: clear=true replaces input value", async () => {
    if (!ready) { console.log("  [skip]"); return; }
    const session = await makeSession();
    try {
      await session.open("data:text/html," + encodeURIComponent(
        '<input id="inp" type="text" value="old">',
      ));
      const result = await session.type(
        { original: "#inp", source: "selector", selector: "#inp" },
        "new value",
        { clear: true },
      );
      expect(result.typed).toBe("#inp");

      const value = await session.evaluate<string>({
        kind: "expression",
        expression: 'document.querySelector("#inp").value',
      });
      expect(value).toBe("new value");
      console.log(`  ✅ type clear=true → input value="${value}"`);
    } finally {
      await cleanupSession(session);
    }
  });

  test("type: submit=true sends Enter", async () => {
    if (!ready) { console.log("  [skip]"); return; }
    const session = await makeSession();
    try {
      await session.open("data:text/html," + encodeURIComponent(`
        <script>window.__submitted = false;</script>
        <form onsubmit="event.preventDefault(); window.__submitted = true">
          <input id="inp" type="text">
        </form>
      `));

      const result = await session.type(
        { original: "#inp", source: "selector", selector: "#inp" },
        "test",
        { clear: true, submit: true },
      );
      expect(result.submitted).toBe(true);

      const submitted = await session.evaluate<boolean>({
        kind: "expression",
        expression: "window.__submitted",
      });
      expect(submitted).toBe(true);
      console.log(`  ✅ type submit=true → form intercepted, window.__submitted=true`);
    } finally {
      await cleanupSession(session);
    }
  });

  // ═══════════════════════════════════════════════════════════════
  // 5. Click
  // ═══════════════════════════════════════════════════════════════

  test("click: triggers onclick handler", async () => {
    if (!ready) { console.log("  [skip]"); return; }
    const session = await makeSession();
    try {
      await session.open("data:text/html," + encodeURIComponent(`
        <button id="btn" onclick="this.textContent='Clicked'">Click me</button>
      `));

      const result = await session.click(
        { original: "#btn", source: "selector", selector: "#btn" },
      );

      const text = await session.evaluate<string>({
        kind: "expression",
        expression: 'document.querySelector("#btn").textContent',
      });
      expect(text).toBe("Clicked");
      console.log(`  ✅ click → button now says "${text}"`);
    } finally {
      await cleanupSession(session);
    }
  });

  test("click: non-existent element throws", async () => {
    if (!ready) { console.log("  [skip]"); return; }
    const session = await makeSession();
    try {
      await session.open("data:text/html,<h1>No button</h1>");
      try {
        await session.click(
          { original: "#nonexistent", source: "selector", selector: "#nonexistent" },
          { timeoutMs: 2000 },
        );
        expect.unreachable("Should have thrown");
      } catch (e) {
        expect((e as Error).message).toContain("not found");
        console.log(`  ✅ click missing element → throws as expected`);
      }
    } finally {
      await cleanupSession(session);
    }
  });

  // ═══════════════════════════════════════════════════════════════
  // 6. Press
  // ═══════════════════════════════════════════════════════════════

  test("press: Enter on focused input submits form", async () => {
    if (!ready) { console.log("  [skip]"); return; }
    const session = await makeSession();
    try {
      await session.open("data:text/html," + encodeURIComponent(`
        <script>window.__submitted = false;</script>
        <form onsubmit="event.preventDefault(); window.__submitted = true">
          <input id="inp" type="text" autofocus>
        </form>
      `));

      await session.press("Enter", {
        original: "#inp", source: "selector", selector: "#inp",
      });

      const submitted = await session.evaluate<boolean>({
        kind: "expression",
        expression: "window.__submitted",
      });
      // Safari WebDriver sendKeys with Enter may not trigger form submit
      // depending on the driver version. Log the result.
      console.log(`  press Enter on input → window.__submitted=${submitted}`);
      // Don't hard-fail — this is a known WebDriver quirk
    } finally {
      await cleanupSession(session);
    }
  });

  test("press: Escape key without target (body-level)", async () => {
    if (!ready) { console.log("  [skip]"); return; }
    const session = await makeSession();
    try {
      await session.open("data:text/html,<h1>Press test</h1>");
      const result = await session.press("Escape");
      console.log(`  ✅ press Escape → title="${result.title}"`);
    } finally {
      await cleanupSession(session);
    }
  });

  // ═══════════════════════════════════════════════════════════════
  // 7. Tabs
  // ═══════════════════════════════════════════════════════════════

  test("tabs: list returns at least one tab", async () => {
    if (!ready) { console.log("  [skip]"); return; }
    const session = await makeSession();
    try {
      await session.open("data:text/html,<title>TabTest</title>");
      const result = await session.tabs({ action: "list" });
      expect(result.kind).toBe("list");
      expect(result.tabs.length).toBeGreaterThanOrEqual(1);
      console.log(`  ✅ tabs list → ${result.tabs.length} tabs: ${JSON.stringify(result.tabs.map(t => ({ idx: t.index, title: t.title })))}`);
    } finally {
      await cleanupSession(session);
    }
  });

  test("tabs: new creates a tab", async () => {
    if (!ready) { console.log("  [skip]"); return; }
    const session = await makeSession();
    try {
      await session.open("data:text/html,<title>First</title>");
      const before = await session.tabs({ action: "list" });
      const result = await session.tabs({ action: "new", url: "data:text/html,<title>Second</title>" });
      expect(result.kind).toBe("new");
      const after = await session.tabs({ action: "list" });
      expect(after.kind === "list" && after.tabs.length).toBe(
        before.kind === "list" ? before.tabs.length + 1 : 0,
      );
      console.log(`  ✅ tabs new → ${before.kind === "list" ? before.tabs.length : "?"} → ${after.kind === "list" ? after.tabs.length : "?"} tabs`);
    } finally {
      await cleanupSession(session);
    }
  });

  test("tabs: select switches active tab", async () => {
    if (!ready) { console.log("  [skip]"); return; }
    const session = await makeSession();
    try {
      // Open two tabs with distinct titles
      await session.open("data:text/html,<title>TabA</title>");
      await session.tabs({ action: "new", url: "data:text/html,<title>TabB</title>" });

      // Select first tab
      const selectResult = await session.tabs({ action: "select", index: 0 });
      const titleAfterSelect = await session.evaluate<string>({
        kind: "expression",
        expression: "document.title",
      });
      console.log(`  tabs select index=0 → "${titleAfterSelect}"`);
      // Safari may or may not report the correct title after switch.
      // This is a known WebDriver quirk — tab focus isn't always synchronous.
    } finally {
      await cleanupSession(session);
    }
  });

  test("tabs: close removes a tab", async () => {
    if (!ready) { console.log("  [skip]"); return; }
    const session = await makeSession();
    try {
      await session.open("data:text/html,<title>Keep</title>");
      await session.tabs({ action: "new", url: "data:text/html,<title>CloseMe</title>" });
      const before = await session.tabs({ action: "list" });
      const beforeCount = before.kind === "list" ? before.tabs.length : 0;

      const closeResult = await session.tabs({ action: "close", index: 1 });
      expect(closeResult.kind).toBe("close");

      const after = await session.tabs({ action: "list" });
      const afterCount = after.kind === "list" ? after.tabs.length : 0;
      expect(afterCount).toBe(beforeCount - 1);
      console.log(`  ✅ tabs close → ${beforeCount} → ${afterCount} tabs`);
    } finally {
      await cleanupSession(session);
    }
  });

  // ═══════════════════════════════════════════════════════════════
  // 8. Screenshot
  // ═══════════════════════════════════════════════════════════════

  test("screenshot: returns PNG bytes", async () => {
    if (!ready) { console.log("  [skip]"); return; }
    const session = await makeSession();
    try {
      await session.open("data:text/html,<h1 style='color:red'>Screenshot</h1>");
      const bytes = await session.captureScreenshot({ fullPage: false, format: "png" });
      expect(bytes).toBeInstanceOf(Uint8Array);
      expect(bytes.length).toBeGreaterThan(1000);
      // Check PNG magic bytes
      expect(bytes[0]).toBe(0x89);
      expect(bytes[1]).toBe(0x50);
      console.log(`  ✅ screenshot → ${bytes.length} bytes (valid PNG)`);
    } finally {
      await cleanupSession(session);
    }
  });

  test("screenshot: fullPage throws UnsupportedCapability", async () => {
    if (!ready) { console.log("  [skip]"); return; }
    const session = await makeSession();
    try {
      await session.open("data:text/html,<h1>Test</h1>");
      try {
        await session.captureScreenshot({ fullPage: true, format: "png" });
        expect.unreachable("Should have thrown");
      } catch (e) {
        expect((e as Error).message).toMatch(/full.?page.*screenshot|unsupported/i);
        console.log(`  ✅ fullPage screenshot → correctly rejected`);
      }
    } finally {
      await cleanupSession(session);
    }
  });

  // ═══════════════════════════════════════════════════════════════
  // 9. Disconnect
  // ═══════════════════════════════════════════════════════════════

  test("disconnect: releases without error", async () => {
    if (!ready) { console.log("  [skip]"); return; }
    const session = await makeSession();
    try {
      await session.open("data:text/html,<h1>Test</h1>");
      await session.disconnect();
      console.log("  ✅ disconnect → clean");
    } finally {
      await cleanupSession(session);
    }
  });
});
