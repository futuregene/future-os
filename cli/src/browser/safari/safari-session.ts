/**
 * SafariSession — BrowserSession implementation via W3C WebDriver protocol.
 *
 * Connects to a running safaridriver. Handles session creation/reuse,
 * window handle ↔ pageId mapping, and all page operations.
 *
 * Capability gaps vs Chromium CDP:
 * - No fullPage screenshot (WebDriver limitation)
 * - WebDriver Element Click instead of Input.dispatchMouseEvent
 * - No network inspection
 * - execute/sync for console hook (less reliable than addScriptToEvaluateOnNewDocument)
 */
import type { BrowserKind, BrowserProtocol } from "../types.js";
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
  BrowserSessionParams,
} from "../backend.js";
import { WebDriverClient, WebDriverErrorResponse } from "./webdriver-client.js";
import {
  ElementNotFoundError,
  UnsupportedCapabilityError,
} from "../errors.js";
import { CONSOLE_HOOK_INVOCATION_SOURCE } from "../scripts/console-hook-script.js";

export class SafariSession implements BrowserSession {
  readonly kind: BrowserKind = "safari";
  readonly protocol: BrowserProtocol = "webdriver";

  private client: WebDriverClient;
  private sessionId: string;

  constructor(params: BrowserSessionParams) {
    if (params.protocol !== "webdriver") {
      throw new Error("SafariSession requires webdriver protocol");
    }
    this.client = new WebDriverClient(params.endpoint);
    this.sessionId = params.sessionId;
  }

  // ── Helpers ────────────────────────────────────────────────────────

  private handleToPageId(handle: string): string {
    return handle;
  }

  /** Resolve a CSS selector to a WebDriver element ID. */
  private async findOne(selector: string): Promise<string> {
    let using = "css selector";
    let value = selector;

    // Playwright-style selector prefixes
    if (selector.startsWith("xpath=")) {
      using = "xpath";
      value = selector.slice(6);
    } else if (selector.startsWith("text=")) {
      // WebDriver doesn't support text selector natively — convert to xpath
      const text = selector.slice(5);
      using = "xpath";
      value = `//*[contains(text(),"${text}")]`;
    }

    try {
      return await this.client.findElement(this.sessionId, using, value);
    } catch (e) {
      if (e instanceof WebDriverErrorResponse && e.wd.error === "no such element") {
        throw new ElementNotFoundError(selector);
      }
      throw e;
    }
  }

  /** Get current active page ID (window handle). */
  private async currentPageId(): Promise<string> {
    const handle = await this.client.getCurrentWindowHandle(this.sessionId);
    return this.handleToPageId(handle);
  }

  // ── Open ──────────────────────────────────────────────────────────

  async open(url: string, _options: OpenPageOptions = {}): Promise<InternalPageInfo> {
    await this.client.navigateTo(this.sessionId, url);

    // Wait for page to load (WebDriver navigateTo waits for page load)
    const handle = await this.client.getCurrentWindowHandle(this.sessionId);
    const pageId = this.handleToPageId(handle);

    // Install console hook
    await this.client.executeScript(this.sessionId, CONSOLE_HOOK_INVOCATION_SOURCE).catch(() => {});

    const title = await this.client.getTitle(this.sessionId);
    const currentUrl = await this.client.getCurrentUrl(this.sessionId);

    return { pageId, title, url: currentUrl };
  }

  // ── Click ─────────────────────────────────────────────────────────

  async click(
    target: ResolvedTarget,
    _options: ClickOptions = {},
  ): Promise<InternalActionResult> {
    const elementId = await this.findOne(target.selector);

    const handle = await this.client.getCurrentWindowHandle(this.sessionId);
    const currentUrl = await this.client.getCurrentUrl(this.sessionId);

    await this.client.clickElement(this.sessionId, elementId);

    // Check if navigation happened (short window — no point waiting 15s)
    const navDeadline = Date.now() + 500;
    let newUrl = currentUrl;
    while (Date.now() < navDeadline) {
      newUrl = await this.client.getCurrentUrl(this.sessionId);
      if (newUrl !== currentUrl) break;
      await sleep(100);
    }

    const title = await this.client.getTitle(this.sessionId);

    return {
      pageId: this.handleToPageId(handle),
      title,
      url: newUrl,
      didNavigate: newUrl !== currentUrl,
    };
  }

  // ── Type ─────────────────────────────────────────────────────────

  async type(
    target: ResolvedTarget,
    text: string,
    options: TypeOptions = {},
  ): Promise<InternalTypeResult> {
    const elementId = await this.findOne(target.selector);

    const shouldClear = options.clear ?? true;
    if (shouldClear) {
      await this.client.clearElement(this.sessionId, elementId);
    }
    await this.client.sendKeysToElement(this.sessionId, elementId, text);

    if (options.submit) {
      await this.client.sendKeysToElement(this.sessionId, elementId, "\n");
    }

    return {
      pageId: await this.currentPageId(),
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
    // Map common keys to WebDriver sendKeys sequences
    const keyMap: Record<string, string> = {
      "Enter": "",
      "Tab": "",
      "Escape": "",
      "Backspace": "",
      "Delete": "",
      "Space": " ",
      "ArrowUp": "",
      "ArrowDown": "",
      "ArrowLeft": "",
      "ArrowRight": "",
      "Home": "",
      "End": "",
      "PageUp": "",
      "PageDown": "",
    };

    const webdriverKey = keyMap[key] ?? key;

    if (target) {
      const elementId = await this.findOne(target.selector);
      await this.client.sendKeysToElement(this.sessionId, elementId, webdriverKey);
    } else {
      // Send key to the active element
      const elementId = await this.client.findElement(this.sessionId, "css selector", "body");
      await this.client.sendKeysToElement(this.sessionId, elementId, webdriverKey);
    }

    const handle = await this.client.getCurrentWindowHandle(this.sessionId);
    const title = await this.client.getTitle(this.sessionId);
    const url = await this.client.getCurrentUrl(this.sessionId);

    return {
      pageId: this.handleToPageId(handle),
      title,
      url,
      didNavigate: false,
    };
  }

  // ── Tabs ─────────────────────────────────────────────────────────

  async tabs(action: TabsAction): Promise<InternalTabsResult> {
    if (action.action === "list") {
      const handles = await this.client.getWindowHandles(this.sessionId);
      const currentHandle = await this.client.getCurrentWindowHandle(this.sessionId);
      const tabs: InternalTabInfo[] = [];

      for (let i = 0; i < handles.length; i++) {
        const handle = handles[i]!;
        // Switch to each window to get title (expensive but needed)
        await this.client.switchToWindow(this.sessionId, handle).catch(() => {});
        const title = await this.client.getTitle(this.sessionId).catch(() => "");
        const url = await this.client.getCurrentUrl(this.sessionId).catch(() => "");

        tabs.push({
          pageId: this.handleToPageId(handle),
          index: i,
          title,
          url,
          active: handle === currentHandle,
        });
      }

      // Switch back to original
      if (handles.length > 0) {
        await this.client.switchToWindow(this.sessionId, currentHandle).catch(() => {});
      }

      return { kind: "list", tabs };
    }

    if (action.action === "new") {
      const { handle } = await this.client.newWindow(this.sessionId);
      if (action.url) {
        await this.client.navigateTo(this.sessionId, action.url);
      }

      // Install console hook on new page
      await this.client.executeScript(this.sessionId, CONSOLE_HOOK_INVOCATION_SOURCE).catch(() => {});

      const handles = await this.client.getWindowHandles(this.sessionId);
      const index = handles.indexOf(handle);

      const title = await this.client.getTitle(this.sessionId).catch(() => "");
      const url = await this.client.getCurrentUrl(this.sessionId);

      return {
        kind: "new",
        page: { pageId: this.handleToPageId(handle), title, url },
        index,
      };
    }

    const handles = await this.client.getWindowHandles(this.sessionId);
    const { index } = action as { index: number };
    if (index < 0 || index >= handles.length) throw new Error(`Invalid tab index: ${index}`);

    if (action.action === "select") {
      const handle = handles[index]!;
      await this.client.switchToWindow(this.sessionId, handle);

      // Reinstall console hook on newly-focused window
      await this.client.executeScript(this.sessionId, CONSOLE_HOOK_INVOCATION_SOURCE).catch(() => {});

      const title = await this.client.getTitle(this.sessionId).catch(() => "");
      const url = await this.client.getCurrentUrl(this.sessionId);

      return {
        kind: "select",
        page: { pageId: this.handleToPageId(handle), title, url },
      };
    }

    if (action.action === "close") {
      const handle = handles[index]!;
      // Switch to the window first, then close it
      await this.client.switchToWindow(this.sessionId, handle);
      const url = await this.client.getCurrentUrl(this.sessionId).catch(() => "");
      await this.client.closeWindow(this.sessionId);

      // After closing, switch to the last remaining window
      const remaining = await this.client.getWindowHandles(this.sessionId);
      if (remaining.length > 0) {
        await this.client.switchToWindow(this.sessionId, remaining[remaining.length - 1]!);
      }

      return { kind: "close", url, index };
    }

    throw new Error(`Unknown tabs action: ${(action as { action: string }).action}`);
  }

  // ── Evaluate ──────────────────────────────────────────────────────

  async evaluate<T>(request: EvaluateRequest): Promise<T> {
    if (request.kind === "expression") {
      return this.client.executeScript<T>(this.sessionId, `return (${request.expression})`);
    }

    // Function call: wrap as IIFE
    const args = request.arguments ?? [];
    const expr = `return (${request.functionDeclaration}).apply(null, arguments);`;
    return this.client.executeScript<T>(this.sessionId, expr, args);
  }

  // ── Screenshot ────────────────────────────────────────────────────

  async captureScreenshot(options: CaptureScreenshotOptions): Promise<Uint8Array> {
    if (options.fullPage) {
      // WebDriver screenshots are always viewport only. We don't try to
      // stitch — the caller gets a clear error if this is critical.
      throw new UnsupportedCapabilityError(
        "safari",
        "Full-page screenshot",
        "Use viewport screenshot or a Chrome/Edge browser.",
      );
    }

    return this.client.takeScreenshot(this.sessionId);
  }

  // ── Disconnect ────────────────────────────────────────────────────

  async disconnect(): Promise<void> {
    // Don't delete the session — it persists across CLI commands.
    // Just release our local state.
  }
}

function sleep(ms: number): Promise<void> {
  return new Promise(resolve => setTimeout(resolve, ms));
}
