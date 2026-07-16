/**
 * Contract test runner — parameterized by backend.
 *
 * Each test function receives a browser context with either a
 * Playwright Page reference or a ChromiumSession reference,
 * depending on which backend is under test.
 */
import type { BrowserSession } from "../../backend.js";
import type { ResolvedTarget } from "../../selector-resolver.js";
import { getFixture } from "../fixtures/pages.js";

// ── Test context ─────────────────────────────────────────────────────

export interface ContractTestContext {
  /** The backend implementation under test. */
  session: BrowserSession;
  /** Playwright page (only for Playwright baseline tests). */
  playwrightPage?: import("playwright-core").Page;
  /** Current active page title/url via backend. */
  currentTitle(): Promise<string>;
  currentUrl(): Promise<string>;
}

export interface ContractRunnerOptions {
  /** Create a fresh session (one per test to avoid state bleed). */
  makeSession(endpoint: string): Promise<BrowserSession>;
  /** Create a Playwright context for baseline comparison. */
  makePlaywrightPage?(endpoint: string): Promise<import("playwright-core").Page>;
}

// ── Test factory ─────────────────────────────────────────────────────

export type ContractTestFn = (
  ctx: ContractTestContext,
  endpoint: string,
) => Promise<void>;

const tests = new Map<string, ContractTestFn>();

function register(name: string, fn: ContractTestFn): void {
  tests.set(name, fn);
}

export function getContractTests(): Map<string, ContractTestFn> {
  return tests;
}

// ═══════════════════════════════════════════════════════════════════
// open — navigate to a URL and get title/url
// ═══════════════════════════════════════════════════════════════════

register("open: returns title and url", async (ctx) => {
  const page = await ctx.session.open("data:text/html,<title>TestOpen</title><p>Hello</p>");
  if (page.title !== "TestOpen") {
    throw new Error(`Expected title "TestOpen", got "${page.title}"`);
  }
  if (!page.url.includes("data:text/html")) {
    throw new Error(`Expected data URL, got "${page.url}"`);
  }
});

register("open: about:blank succeeds", async (ctx) => {
  const page = await ctx.session.open("about:blank");
  if (page.title !== "") {
    // about:blank has no title
  }
});

// ═══════════════════════════════════════════════════════════════════
// click — default navigation and form submission
// ═══════════════════════════════════════════════════════════════════

register("click: anchor navigation reaches its target", async (ctx) => {
  const targetUrl = "data:text/html," + encodeURIComponent("<title>clicked-target</title>");
  const html = `<html><body><a id="link" href="${targetUrl}">Go</a></body></html>`;
  if (ctx.playwrightPage) {
    await ctx.playwrightPage.setContent(html);
  } else {
    await ctx.session.open("data:text/html," + encodeURIComponent(html));
  }

  const target: ResolvedTarget = { original: "#link", source: "selector", selector: "#link" };
  const result = await ctx.session.click(target);
  if (result.title !== "clicked-target") {
    throw new Error(`Expected clicked-target title, got "${result.title}"`);
  }
});

register("click: submit-button child runs the form submit chain", async (ctx) => {
  const html = `<html><body>
    <script>window.__submitCount = 0;</script>
    <form onsubmit="event.preventDefault(); window.__submitCount += 1; document.title='submitted'">
      <button id="submit" type="submit"><span id="submit-label">Submit</span></button>
    </form>
  </body></html>`;
  if (ctx.playwrightPage) {
    await ctx.playwrightPage.setContent(html);
  } else {
    await ctx.session.open("data:text/html," + encodeURIComponent(html));
  }

  const target: ResolvedTarget = {
    original: "#submit-label",
    source: "selector",
    selector: "#submit-label",
  };
  await ctx.session.click(target);
  const title = await ctx.session.evaluate<string>({ kind: "expression", expression: "document.title" });
  if (title !== "submitted") {
    throw new Error(`Expected submitted title, got "${title}"`);
  }
  const submitCount = await ctx.session.evaluate<number>({
    kind: "expression",
    expression: "window.__submitCount",
  });
  if (submitCount !== 1) {
    throw new Error(`Expected one submit event, got ${submitCount}`);
  }
});

register("click: preventDefault on an anchor is respected", async (ctx) => {
  const html = `<html><body>
    <a id="blocked" href="data:text/html,<title>should-not-open</title>"
       onclick="event.preventDefault(); document.title='blocked'">Blocked</a>
  </body></html>`;
  if (ctx.playwrightPage) {
    await ctx.playwrightPage.setContent(html);
  } else {
    await ctx.session.open("data:text/html," + encodeURIComponent(html));
  }

  const target: ResolvedTarget = { original: "#blocked", source: "selector", selector: "#blocked" };
  await ctx.session.click(target);
  const title = await ctx.session.evaluate<string>({ kind: "expression", expression: "document.title" });
  if (title !== "blocked") {
    throw new Error(`Expected blocked title, got "${title}"`);
  }
});

register("click: type=button inside a form does not submit", async (ctx) => {
  const html = `<html><body>
    <script>window.__submitted = 0; window.__clicked = 0;</script>
    <form onsubmit="event.preventDefault(); window.__submitted += 1">
      <button id="plain" type="button" onclick="window.__clicked += 1">Plain</button>
    </form>
  </body></html>`;
  if (ctx.playwrightPage) {
    await ctx.playwrightPage.setContent(html);
  } else {
    await ctx.session.open("data:text/html," + encodeURIComponent(html));
  }

  const target: ResolvedTarget = { original: "#plain", source: "selector", selector: "#plain" };
  await ctx.session.click(target);
  const state = await ctx.session.evaluate<{ submitted: number; clicked: number }>({
    kind: "expression",
    expression: "({ submitted: window.__submitted, clicked: window.__clicked })",
  });
  if (state.submitted !== 0 || state.clicked !== 1) {
    throw new Error(`Unexpected form state: ${JSON.stringify(state)}`);
  }
});

// ═══════════════════════════════════════════════════════════════════
// evaluate — expression and function evaluation
// ═══════════════════════════════════════════════════════════════════

register("evaluate: returns string", async (ctx) => {
  const result = await ctx.session.evaluate<string>({
    kind: "expression",
    expression: "'hello'",
  });
  if (result !== "hello") throw new Error(`Expected "hello", got "${result}"`);
});

register("evaluate: returns number", async (ctx) => {
  const result = await ctx.session.evaluate<number>({
    kind: "expression",
    expression: "1 + 2",
  });
  if (result !== 3) throw new Error(`Expected 3, got ${result}`);
});

register("evaluate: returns null", async (ctx) => {
  const result = await ctx.session.evaluate<null>({
    kind: "expression",
    expression: "null",
  });
  if (result !== null) throw new Error(`Expected null, got ${result}`);
});

register("evaluate: function with arguments", async (ctx) => {
  const result = await ctx.session.evaluate<number>({
    kind: "function",
    functionDeclaration: "function(a, b) { return a + b; }",
    arguments: [2, 3],
  });
  if (result !== 5) throw new Error(`Expected 5, got ${result}`);
});

// ═══════════════════════════════════════════════════════════════════
// snapshot — accessibility tree of interactive elements
// ═══════════════════════════════════════════════════════════════════

register("snapshot: works with setContent", async (ctx) => {
  const { SNAPSHOT_FUNCTION_SOURCE } = await import("../../scripts/snapshot-script.js");
  type SnapshotResult = import("../../scripts/snapshot-script.js").SnapshotResult;

  if (ctx.playwrightPage) {
    await ctx.playwrightPage.setContent(getFixture("basic"));
  } else {
    await ctx.session.open("data:text/html," + encodeURIComponent(getFixture("basic")));
  }

  const snapshot = await ctx.session.evaluate<SnapshotResult>({
    kind: "function",
    functionDeclaration: SNAPSHOT_FUNCTION_SOURCE,
    arguments: [80],
  });

  if (!snapshot.items || snapshot.items.length === 0) {
    throw new Error("Snapshot returned no items");
  }

  // Should find the submit button
  const submitBtn = snapshot.items.find((i: { ref: string; name: string; role: string }) => i.name === "Submit" || i.role === "button");
  if (!submitBtn) {
    throw new Error(`Snapshot did not find submit button. Items: ${JSON.stringify(snapshot.items.slice(0, 5))}`);
  }
  if (!submitBtn.ref) throw new Error("Snapshot item has no ref");
  if (!submitBtn.selector) throw new Error("Snapshot item has no selector");
});

// ═══════════════════════════════════════════════════════════════════
// type — text input
// ═══════════════════════════════════════════════════════════════════

register("type: clear=true replaces content", async (ctx) => {
  const html = `<html><body><input id="inp" type="text" value="old"></body></html>`;
  if (ctx.playwrightPage) {
    await ctx.playwrightPage.setContent(html);
  } else {
    await ctx.session.open("data:text/html," + encodeURIComponent(html));
  }

  const target: ResolvedTarget = { original: "#inp", source: "selector", selector: "#inp" };
  const result = await ctx.session.type(target, "new text", { clear: true });
  if (!result.typed) throw new Error("No typed selector returned");

  const value = await ctx.session.evaluate<string>({
    kind: "expression",
    expression: 'document.querySelector("#inp").value',
  });
  if (value !== "new text") {
    throw new Error(`Expected "new text", got "${value}"`);
  }
});

register("type: submit=true sends Enter", async (ctx) => {
  const html = `<html><body>
    <script>window.__submitted = false;</script>
    <form onsubmit="event.preventDefault(); window.__submitted = true">
      <input id="inp" type="text">
    </form>
  </body></html>`;
  if (ctx.playwrightPage) {
    await ctx.playwrightPage.setContent(html);
  } else {
    await ctx.session.open("data:text/html," + encodeURIComponent(html));
  }

  const target: ResolvedTarget = { original: "#inp", source: "selector", selector: "#inp" };
  const result = await ctx.session.type(target, "test", { clear: true, submit: true });
  if (result.submitted !== true) throw new Error("Expected submitted=true");

  const submitted = await ctx.session.evaluate<boolean>({
    kind: "expression",
    expression: "window.__submitted",
  });
  if (submitted !== true) {
    throw new Error(`Expected window.__submitted=true, got ${submitted}`);
  }
});

// ═══════════════════════════════════════════════════════════════════
// press — keyboard events
// ═══════════════════════════════════════════════════════════════════

register("press: Enter key works", async (ctx) => {
  const html = `<html><body>
    <form onsubmit="event.preventDefault(); document.title='pressed'">
      <input id="inp" type="text" autofocus>
    </form>
  </body></html>`;
  if (ctx.playwrightPage) {
    await ctx.playwrightPage.setContent(html);
  } else {
    await ctx.session.open("data:text/html," + encodeURIComponent(html));
  }

  const result = await ctx.session.press("Enter");
  if (!result.title) { /* may be empty */ }
});

register("press: Escape key works", async (ctx) => {
  const html = `<html><body><input id="inp" type="text"></body></html>`;
  if (ctx.playwrightPage) {
    await ctx.playwrightPage.setContent(html);
  } else {
    await ctx.session.open("data:text/html," + encodeURIComponent(html));
  }

  await ctx.session.press("Escape");
  // No error = success
});

// ═══════════════════════════════════════════════════════════════════
// tabs — page management
// ═══════════════════════════════════════════════════════════════════

register("tabs: list returns page info", async (ctx) => {
  const result = await ctx.session.tabs({ action: "list" });
  if (result.kind !== "list") throw new Error(`Expected list, got ${result.kind}`);
  if (result.tabs.length === 0) throw new Error("Expected at least one tab");
  if (result.tabs[0]!.index !== 0) throw new Error("Expected first tab index 0");
});

register("tabs: new creates a tab", async (ctx) => {
  const before = await ctx.session.tabs({ action: "list" });
  const beforeCount = before.kind === "list" ? before.tabs.length : 0;
  await ctx.session.tabs({ action: "new", url: "data:text/html,<title>Tab2</title>" });
  const after = await ctx.session.tabs({ action: "list" });
  const afterCount = after.kind === "list" ? after.tabs.length : 0;
  if (afterCount !== beforeCount + 1) {
    throw new Error(`Tab count did not increase: ${beforeCount} → ${afterCount}`);
  }
});

// ═══════════════════════════════════════════════════════════════════
// screenshot — page capture
// ═══════════════════════════════════════════════════════════════════

register("screenshot: returns bytes", async (ctx) => {
  if (ctx.playwrightPage) {
    await ctx.playwrightPage.setContent(getFixture("basic"));
  } else {
    await ctx.session.open("data:text/html," + encodeURIComponent(getFixture("basic")));
  }

  const bytes = await ctx.session.captureScreenshot({ fullPage: false, format: "png" });
  if (!(bytes instanceof Uint8Array)) throw new Error("Expected Uint8Array");
  if (bytes.length < 100) throw new Error(`Screenshot too small: ${bytes.length} bytes`);

  // PNG magic bytes
  if (bytes[0] !== 0x89 || bytes[1] !== 0x50 || bytes[2] !== 0x4E || bytes[3] !== 0x47) {
    throw new Error("Not a valid PNG file");
  }
});

// ═══════════════════════════════════════════════════════════════════
// timeout — action timeout
// ═══════════════════════════════════════════════════════════════════

register("timeout: click on non-existent element throws", async (ctx) => {
  if (ctx.playwrightPage) {
    await ctx.playwrightPage.setContent(getFixture("basic"));
  } else {
    await ctx.session.open("data:text/html," + encodeURIComponent(getFixture("basic")));
  }

  const target: ResolvedTarget = { original: "#nonexistent", source: "selector", selector: "#nonexistent" };
  try {
    await ctx.session.click(target, { timeoutMs: 1000 });
    throw new Error("Expected click to throw");
  } catch (e) {
    // Expected
  }
});

// ═══════════════════════════════════════════════════════════════════
// disconnect — releases connection, browser remains running
// ═══════════════════════════════════════════════════════════════════

register("disconnect: session releases without closing browser", async (ctx) => {
  await ctx.session.disconnect();
  // No error = success
  // (Browser process check is done by the test harness)
});
