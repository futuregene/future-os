/**
 * PlaywrightSession — wraps existing Playwright browser/page operations
 * behind the BrowserSession interface.
 *
 * This is the existing behavior, re-packaged. Once ChromiumBackend passes
 * all contract tests, this wrapper is deleted.
 */
import type { BrowserKind, BrowserProtocol } from "../types.js";
import type {
  BrowserSession,
  BrowserSessionParams,
  OpenPageOptions,
  ClickOptions,
  TypeOptions,
  PressOptions,
  CaptureScreenshotOptions,
  TabsAction,
  EvaluateRequest,
  ResolvedTarget,
  InternalPageInfo,
  InternalTabInfo,
  InternalActionResult,
  InternalTypeResult,
  InternalTabsResult,
} from "../backend.js";

export class PlaywrightSession implements BrowserSession {
  readonly kind: BrowserKind;
  readonly protocol: BrowserProtocol;

  private browser: import("playwright-core").Browser | null = null;
  private consoleHookInstalled: boolean = false;

  constructor(private params: BrowserSessionParams) {
    this.kind = params.browserKind;
    this.protocol = params.protocol;
  }

  // ── Init ──────────────────────────────────────────────────────────

  private async connect(): Promise<import("playwright-core").Browser> {
    if (this.browser?.isConnected()) return this.browser;

    if (this.params.protocol !== "cdp") {
      throw new Error("PlaywrightSession only supports CDP protocol");
    }

    const { chromium } = await import("playwright-core");
    this.browser = await chromium.connectOverCDP(this.params.endpoint, {
      timeout: 5000,
    });
    return this.browser;
  }

  private async activePage(): Promise<import("playwright-core").Page> {
    const b = await this.connect();
    const pages = b.contexts().flatMap(c => c.pages());
    if (pages.length === 0) {
      const context = b.contexts()[0] ?? await b.newContext();
      return context.newPage();
    }
    return pages[pages.length - 1]!;
  }

  private async ensureConsoleHook(page: import("playwright-core").Page): Promise<void> {
    if (this.consoleHookInstalled) return;
    await page.evaluate(`(${CONSOLE_HOOK_IIFE})()`).catch(() => undefined);
    this.consoleHookInstalled = true;
  }

  // ── Open ──────────────────────────────────────────────────────────

  async open(url: string, options: OpenPageOptions = {}): Promise<InternalPageInfo> {
    const page = await this.activePage();
    const gotoOptions: { waitUntil?: "load" | "domcontentloaded" } = {};
    if (options.waitUntil && options.waitUntil !== "none") {
      gotoOptions.waitUntil = options.waitUntil as "load" | "domcontentloaded";
    }
    await page.goto(url, gotoOptions);
    this.consoleHookInstalled = false;
    await this.ensureConsoleHook(page);

    return {
      pageId: await this.resolvePageId(page),
      title: await page.title().catch(() => ""),
      url: page.url(),
    };
  }

  // ── Click ─────────────────────────────────────────────────────────

  async click(target: ResolvedTarget, options: ClickOptions = {}): Promise<InternalActionResult> {
    const page = await this.activePage();
    const locator = page.locator(target.selector);
    const count = await locator.count();

    if (count === 0) {
      const { ElementNotFoundError } = await import("../errors.js");
      throw new ElementNotFoundError(target.selector);
    }
    if (count > 1) {
      const { StrictModeViolationError } = await import("../errors.js");
      throw new StrictModeViolationError(target.selector, count);
    }

    await locator.click({ timeout: options.timeoutMs });
    await page.waitForLoadState("domcontentloaded", { timeout: 3000 }).catch(() => undefined);

    return {
      pageId: await this.resolvePageId(page),
      title: await page.title().catch(() => ""),
      url: page.url(),
      didNavigate: true, // conservative
    };
  }

  // ── Type ──────────────────────────────────────────────────────────

  async type(target: ResolvedTarget, text: string, options: TypeOptions = {}): Promise<InternalTypeResult> {
    const page = await this.activePage();
    const locator = page.locator(target.selector);
    const count = await locator.count();

    if (count === 0) {
      const { ElementNotFoundError } = await import("../errors.js");
      throw new ElementNotFoundError(target.selector);
    }
    if (count > 1) {
      const { StrictModeViolationError } = await import("../errors.js");
      throw new StrictModeViolationError(target.selector, count);
    }

    const shouldClear = options.clear ?? true;

    if (shouldClear) {
      await locator.fill(text, { timeout: options.timeoutMs });
    } else {
      await locator.type(text, { timeout: options.timeoutMs });
    }

    if (options.submit) {
      await locator.press("Enter", { timeout: options.timeoutMs });
    }

    return {
      pageId: await this.resolvePageId(page),
      typed: target.selector,
      submitted: Boolean(options.submit),
    };
  }

  // ── Press ─────────────────────────────────────────────────────────

  async press(
    key: string,
    target?: ResolvedTarget,
    _options: PressOptions = {},
  ): Promise<InternalActionResult> {
    const page = await this.activePage();

    if (target) {
      const locator = page.locator(target.selector);
      await locator.press(key);
    } else {
      await page.keyboard.press(key);
    }

    await page.waitForLoadState("domcontentloaded", { timeout: 3000 }).catch(() => undefined);

    return {
      pageId: await this.resolvePageId(page),
      title: await page.title().catch(() => ""),
      url: page.url(),
      didNavigate: true,
    };
  }

  // ── Tabs ──────────────────────────────────────────────────────────

  async tabs(action: TabsAction): Promise<InternalTabsResult> {
    const b = await this.connect();
    const allPages = b.contexts().flatMap(c => c.pages());

    if (action.action === "list") {
      const tabs: InternalTabInfo[] = [];
      for (let i = 0; i < allPages.length; i++) {
        const p = allPages[i]!;
        tabs.push({
          pageId: await this.resolvePageId(p),
          index: i,
          title: await p.title().catch(() => ""),
          url: p.url(),
          active: i === allPages.length - 1,
        });
      }
      return { kind: "list", tabs };
    }

    if (action.action === "new") {
      const context = b.contexts()[0] ?? await b.newContext();
      const p = await context.newPage();
      if (action.url) {
        await p.goto(action.url, { waitUntil: "domcontentloaded" });
      }
      this.consoleHookInstalled = false;
      await this.ensureConsoleHook(p);
      const newPages = b.contexts().flatMap(c => c.pages());
      return {
        kind: "new",
        page: { pageId: await this.resolvePageId(p), title: await p.title().catch(() => ""), url: p.url() },
        index: newPages.indexOf(p),
      };
    }

    const { index } = action;
    if (index < 0 || index >= allPages.length) {
      throw new Error(`browser command tabs: action requires a valid 0-based index.`);
    }

    if (action.action === "select") {
      const p = allPages[index]!;
      await p.bringToFront().catch(() => undefined);
      return {
        kind: "select",
        page: { pageId: await this.resolvePageId(p), title: await p.title().catch(() => ""), url: p.url() },
      };
    }

    if (action.action === "close") {
      const p = allPages[index]!;
      const url = p.url();
      await p.close();
      return { kind: "close", url, index };
    }

    throw new Error(`Unknown tabs action: ${(action as { action: string }).action}`);
  }

  // ── Evaluate ──────────────────────────────────────────────────────

  async evaluate<T>(request: EvaluateRequest): Promise<T> {
    const page = await this.activePage();
    await this.ensureConsoleHook(page);

    if (request.kind === "expression") {
      return page.evaluate(request.expression) as Promise<T>;
    }
    return page.evaluate(request.functionDeclaration, ...(request.arguments ?? [])) as Promise<T>;
  }

  // ── Screenshot ────────────────────────────────────────────────────

  async captureScreenshot(options: CaptureScreenshotOptions): Promise<Uint8Array> {
    const page = await this.activePage();
    const buffer = await page.screenshot({
      fullPage: options.fullPage,
      type: options.format,
      quality: options.quality,
    });
    return new Uint8Array(buffer);
  }

  // ── Disconnect ────────────────────────────────────────────────────

  async disconnect(): Promise<void> {
    if (this.browser) {
      // Don't close the browser — only disconnect our Playwright connection
      this.browser = null;
    }
    this.consoleHookInstalled = false;
  }

  // ── Internal ──────────────────────────────────────────────────────

  private async resolvePageId(page: import("playwright-core").Page): Promise<string> {
    // Playwright doesn't expose CDP targetId directly via the high-level API.
    // Use the page's URL as a fallback identifier (not ideal but sufficient
    // for the Playwright wrapper; ChromiumBackend will use real targetIds).
    const url = page.url();
    return url || `page-${Date.now()}`;
  }
}

// ── Console hook IIFE (inline to avoid circular imports) ─────────────

const CONSOLE_HOOK_IIFE = `(function() {
  var target = globalThis;
  if (target.__futureConsoleHookInstalled) return;
  target.__futureConsoleHookInstalled = true;
  target.__futureConsoleLogs = target.__futureConsoleLogs || [];
  var levels = ['log', 'info', 'warn', 'error'];
  for (var li = 0; li < levels.length; li++) {
    var level = levels[li];
    var original = target.console[level].bind(target.console);
    target.console[level] = function() {
      var parts = [];
      for (var ai = 0; ai < arguments.length; ai++) {
        var arg = arguments[ai];
        try {
          parts.push(typeof arg === 'string' ? arg : JSON.stringify(arg));
        } catch (e) {
          parts.push(String(arg));
        }
      }
      target.__futureConsoleLogs.push({
        level: level,
        text: parts.join(' '),
        time: new Date().toISOString(),
      });
      if (target.__futureConsoleLogs.length > 200) target.__futureConsoleLogs.shift();
      original.apply(this, arguments);
    };
  }
})()`;
