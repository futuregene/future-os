/**
 * Minimal BrowserSession adapter using Playwright directly.
 *
 * Used as the reference implementation for contract validation.
 * This is NOT the PlaywrightBackend from the production codebase —
 * it's a thin adapter that maps BrowserSession methods to Playwright APIs
 * so the contract runner can exercise both backends through the same interface.
 */
import type { BrowserKind, BrowserProtocol } from "../../types.js";
import type {
  BrowserSession,
  OpenPageOptions,
  ClickOptions,
  TypeOptions,
  PressOptions,
  CaptureScreenshotOptions,
  TabsAction,
  EvaluateRequest,
  ResolvedTarget,
  InternalPageInfo,
  InternalActionResult,
  InternalTypeResult,
  InternalTabsResult,
  InternalTabInfo,
} from "../../backend.js";

export class PlaywrightAdapterSession implements BrowserSession {
  readonly kind: BrowserKind;
  readonly protocol: BrowserProtocol = "cdp";

  private browser: import("playwright-core").Browser;
  private page: import("playwright-core").Page | null = null;

  constructor(browser: import("playwright-core").Browser, kind: BrowserKind = "chrome") {
    this.browser = browser;
    this.kind = kind;
  }

  private async getPage(): Promise<import("playwright-core").Page> {
    if (this.page) return this.page;
    const pages = this.browser.contexts().flatMap(c => c.pages());
    if (pages.length > 0) {
      this.page = pages[pages.length - 1]!;
    } else {
      const ctx = this.browser.contexts()[0] ?? await this.browser.newContext();
      this.page = await ctx.newPage();
    }
    return this.page;
  }

  private pageId(url: string): string {
    return url || "page";
  }

  async open(url: string, _options: OpenPageOptions = {}): Promise<InternalPageInfo> {
    const page = await this.getPage();
    await page.goto(url, { waitUntil: "domcontentloaded" });
    return {
      pageId: this.pageId(page.url()),
      title: await page.title().catch(() => ""),
      url: page.url(),
    };
  }

  async click(target: ResolvedTarget, options: ClickOptions = {}): Promise<InternalActionResult> {
    const page = await this.getPage();
    const locator = page.locator(target.selector);
    const count = await locator.count();
    if (count === 0) throw new Error(`Element not found: ${target.selector}`);
    if (count > 1) throw new Error(`Strict mode violation: ${target.selector} resolved to ${count} elements`);

    await locator.click({ timeout: options.timeoutMs });
    await page.waitForLoadState("domcontentloaded", { timeout: 3000 }).catch(() => undefined);

    return {
      pageId: this.pageId(page.url()),
      title: await page.title().catch(() => ""),
      url: page.url(),
      didNavigate: true,
    };
  }

  async type(target: ResolvedTarget, text: string, options: TypeOptions = {}): Promise<InternalTypeResult> {
    const page = await this.getPage();
    const locator = page.locator(target.selector);
    const count = await locator.count();
    if (count === 0) throw new Error(`Element not found: ${target.selector}`);
    if (count > 1) throw new Error(`Strict mode violation: ${target.selector} resolved to ${count} elements`);

    if (options.clear ?? true) {
      await locator.fill(text);
    } else {
      await locator.type(text);
    }
    if (options.submit) await locator.press("Enter");

    return { pageId: this.pageId(page.url()), typed: target.selector, submitted: Boolean(options.submit) };
  }

  async press(key: string, target?: ResolvedTarget, _options: PressOptions = {}): Promise<InternalActionResult> {
    const page = await this.getPage();
    if (target) {
      await page.locator(target.selector).press(key);
    } else {
      await page.keyboard.press(key);
    }
    await page.waitForLoadState("domcontentloaded", { timeout: 3000 }).catch(() => undefined);

    return {
      pageId: this.pageId(page.url()),
      title: await page.title().catch(() => ""),
      url: page.url(),
      didNavigate: true,
    };
  }

  async tabs(action: TabsAction): Promise<InternalTabsResult> {
    const allPages = this.browser.contexts().flatMap(c => c.pages());

    if (action.action === "list") {
      const tabs: InternalTabInfo[] = [];
      for (let i = 0; i < allPages.length; i++) {
        const p = allPages[i]!;
        tabs.push({
          pageId: this.pageId(p.url()),
          index: i,
          title: await p.title().catch(() => ""),
          url: p.url(),
          active: i === allPages.length - 1,
        });
      }
      return { kind: "list", tabs };
    }

    if (action.action === "new") {
      const ctx = this.browser.contexts()[0] ?? await this.browser.newContext();
      const p = await ctx.newPage();
      if (action.url) await p.goto(action.url, { waitUntil: "domcontentloaded" });
      const newPages = this.browser.contexts().flatMap(c => c.pages());
      return {
        kind: "new",
        page: { pageId: this.pageId(p.url()), title: await p.title().catch(() => ""), url: p.url() },
        index: newPages.indexOf(p),
      };
    }

    const { index } = action;
    if (index < 0 || index >= allPages.length) throw new Error(`Invalid tab index: ${index}`);

    if (action.action === "select") {
      const p = allPages[index]!;
      await p.bringToFront().catch(() => undefined);
      return {
        kind: "select",
        page: { pageId: this.pageId(p.url()), title: await p.title().catch(() => ""), url: p.url() },
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

  async evaluate<T>(request: EvaluateRequest): Promise<T> {
    const page = await this.getPage();
    if (request.kind === "expression") {
      return page.evaluate(request.expression) as Promise<T>;
    }
    return page.evaluate(request.functionDeclaration, ...(request.arguments ?? [])) as Promise<T>;
  }

  async captureScreenshot(options: CaptureScreenshotOptions): Promise<Uint8Array> {
    const page = await this.getPage();
    const buf = await page.screenshot({ fullPage: options.fullPage, type: options.format, quality: options.quality });
    return new Uint8Array(buf);
  }

  async disconnect(): Promise<void> {
    this.page = null;
  }
}
