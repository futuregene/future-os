/**
 * ChromiumSession — BrowserSession implementation for Chrome/Edge/Chromium via CDP.
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
  BrowserSessionParams,
  Deadline,
} from "../backend.js";
import { createDeadline } from "../backend.js";
import { CdpConnection, CdpSession } from "./cdp-connection.js";
import { resolveCdpEndpoint } from "./chromium-endpoint.js";
import { ChromiumPageManager } from "./chromium-page.js";
import { installConsoleHook, withTemporaryPreload } from "./chromium-console-hook.js";
import {
  waitForExplicitNavigation,
  ActionNavigationObserver,
} from "./chromium-navigation.js";
import { captureScreenshot } from "./chromium-screenshot.js";
import {
  ElementNotFoundError,
  ElementNotInteractableError,
} from "../errors.js";

// ── Element check script ──────────────────────────────────────────

const ELEMENT_CHECK_SCRIPT = `function(selector) {
  var element = document.querySelector(selector);
  if (!element) return { exists: false };
  var rect = element.getBoundingClientRect();
  var style = getComputedStyle(element);
  var visible = rect.width > 0 &&
    rect.height > 0 &&
    style.visibility !== 'hidden' &&
    style.display !== 'none' &&
    Number(style.opacity || '1') > 0;
  var disabled = !!(element.disabled);
  return {
    exists: true,
    connected: element.isConnected,
    visible: visible,
    disabled: disabled,
    box: { x: rect.x, y: rect.y, width: rect.width, height: rect.height },
    obscured: false,
  };
}`;

const SCROLL_INTO_VIEW_SCRIPT = `function(selector) {
  var element = document.querySelector(selector);
  if (element) {
    element.scrollIntoView({ block: 'center', inline: 'center', behavior: 'instant' });
    var rect = element.getBoundingClientRect();
    return { x: rect.x, y: rect.y, width: rect.width, height: rect.height };
  }
  return null;
}`;

// ── Page session ───────────────────────────────────────────────────

interface PageSession {
  session: CdpSession;
  pageId: string;
  mainFrameId: string;
}

// ── ChromiumSession ────────────────────────────────────────────────

export class ChromiumSession implements BrowserSession {
  readonly kind: BrowserKind;
  readonly protocol: BrowserProtocol;

  private connection: CdpConnection | null = null;
  private browserSess: CdpSession | null = null;
  private pageMgr: ChromiumPageManager | null = null;
  private activePs: PageSession | null = null;
  private timeouts: { action: number; navigation: number };
  private initTabOrder?: string[];
  private initActivePageId?: string;

  constructor(private params: BrowserSessionParams) {
    this.kind = params.browserKind;
    this.protocol = params.protocol;
    this.timeouts = {
      action: params.timeouts.actionTimeoutMs,
      navigation: params.timeouts.navigationTimeoutMs,
    };
    if (params.protocol === "cdp") {
      this.initTabOrder = params.initTabOrder;
      this.initActivePageId = params.activePageId;
    }
  }

  // ── Init ──────────────────────────────────────────────────────────

  private async init(): Promise<{
    connection: CdpConnection;
    browserSess: CdpSession;
    pageMgr: ChromiumPageManager;
  }> {
    if (this.connection?.isConnected) {
      return {
        connection: this.connection,
        browserSess: this.browserSess!,
        pageMgr: this.pageMgr!,
      };
    }

    if (this.params.protocol !== "cdp") {
      throw new Error("ChromiumSession requires CDP protocol");
    }

    const endpointInfo = await resolveCdpEndpoint(this.params.endpoint);

    this.connection = await CdpConnection.connect(endpointInfo.webSocketDebuggerUrl, {
      timeoutMs: 10_000,
    });

    this.browserSess = new CdpSession("", this.connection);

    this.pageMgr = new ChromiumPageManager(this.browserSess, this.connection);
    await this.pageMgr.initialize(this.initTabOrder, this.initActivePageId);

    return {
      connection: this.connection,
      browserSess: this.browserSess,
      pageMgr: this.pageMgr,
    };
  }

  private async activePageSession(): Promise<PageSession> {
    const { connection, browserSess, pageMgr } = await this.init();

    let page = pageMgr.getActivePage();
    if (!page) {
      const created = await pageMgr.createPage("about:blank");
      page = created.page;
    }

    let session: CdpSession;
    if (page.sessionId) {
      session = new CdpSession(page.sessionId, connection);
    } else {
      const attachResult = await browserSess.send("Target.attachToTarget", {
        targetId: page.targetId,
        flatten: true,
      }) as { sessionId: string };
      page.sessionId = attachResult.sessionId;
      session = new CdpSession(attachResult.sessionId, connection);
    }

    await session.send("Page.enable");
    await session.send("Runtime.enable");
    await session.send("Page.setLifecycleEventsEnabled", { enabled: true });

    connection.registerTarget({
      targetId: page.targetId,
      sessionId: session.sessionId,
      type: "page",
    });

    const mainFrame = await getMainFrameState(session);

    this.activePs = {
      session,
      pageId: page.targetId,
      mainFrameId: mainFrame.frameId,
    };

    return this.activePs;
  }

  private disposePageSession(): void {
    this.activePs = null;
  }

  // ── Evaluate helpers ──────────────────────────────────────────────

  private async evaluateExpression<T>(
    session: CdpSession,
    expression: string,
  ): Promise<T> {
    const result = await session.send("Runtime.evaluate", {
      expression,
      returnByValue: true,
    }) as { result: { value: T } };
    return result.result?.value as T;
  }

  /** Evaluate a function by wrapping it in an IIFE (avoids executionContextId requirement). */
  private async evaluateFunction<T>(
    _ps: PageSession,
    functionDeclaration: string,
    args: unknown[],
  ): Promise<T> {
    const argsJson = args.map(a => JSON.stringify(a)).join(",");
    const expression = `((${functionDeclaration})(${argsJson}))`;

    const result = await _ps.session.send("Runtime.evaluate", {
      expression,
      returnByValue: true,
      awaitPromise: true,
    }) as { result: { value: T } };

    return result.result?.value as T;
  }

  // ── Open ──────────────────────────────────────────────────────────

  async open(url: string, _options: OpenPageOptions = {}): Promise<InternalPageInfo> {
    const ps = await this.activePageSession();
    try {
      await installConsoleHook(ps.session);

      const deadline = createDeadline(this.timeouts.navigation);

      const result = await withTemporaryPreload(ps.session, async () => {
        return waitForExplicitNavigation(ps.session, url, deadline);
      });

      if (result.errorText) {
        throw new Error(`Navigation failed: ${result.errorText}`);
      }

      const title = await this.evaluateExpression<string>(ps.session, "document.title").catch(() => "");
      const finalUrl = await this.evaluateExpression<string>(ps.session, "location.href").catch(() => "");

      const { pageMgr } = await this.init();
      const page = pageMgr.getPage(ps.pageId);
      if (page) {
        page.url = finalUrl || url;
        page.title = title || "";
      }

      return { pageId: ps.pageId, title: title || "", url: finalUrl || url };
    } finally {
      this.disposePageSession();
    }
  }

  // ── Click ─────────────────────────────────────────────────────────

  async click(
    target: ResolvedTarget,
    options: ClickOptions = {},
  ): Promise<InternalActionResult> {
    const ps = await this.activePageSession();
    try {
      await installConsoleHook(ps.session);

      const timeoutMs = options.timeoutMs ?? this.timeouts.action;

      await this.waitForActionable(ps.session, target.selector, createDeadline(timeoutMs));

      const box = await this.evaluateExpression<{ x: number; y: number; width: number; height: number } | null>(
        ps.session, `(${SCROLL_INTO_VIEW_SCRIPT})(${JSON.stringify(target.selector)})`,
      );

      const center = box
        ? { x: Math.round(box.x + box.width / 2), y: Math.round(box.y + box.height / 2) }
        : { x: 0, y: 0 };

      const navDeadline = createDeadline(this.timeouts.navigation);
      const navObserver = new ActionNavigationObserver(ps.mainFrameId, await this.getLoaderId(ps.session));
      navObserver.arm(ps.session);

      // Read element metadata BEFORE dispatching mouse events — once
      // the page navigates, the element is gone.
      const meta = await this.evaluateExpression<{ href: string | null; inForm: boolean }>(
        ps.session,
        `(() => {
          const el = document.querySelector(${JSON.stringify(target.selector)});
          return {
            href: el?.href || el?.closest?.('a')?.href || null,
            inForm: Boolean(el?.closest?.('form')),
          };
        })()`,
      ).catch(() => ({ href: null as string | null, inForm: false }));

      await withTemporaryPreload(ps.session, async () => {
        await ps.session.send("Input.dispatchMouseEvent", { type: "mouseMoved", x: center.x, y: center.y });
        await ps.session.send("Input.dispatchMouseEvent", {
          type: "mousePressed", x: center.x, y: center.y, button: "left", clickCount: 1,
        });
        await ps.session.send("Input.dispatchMouseEvent", {
          type: "mouseReleased", x: center.x, y: center.y, button: "left", clickCount: 1,
        });
      });

      // CDP Input.dispatchMouseEvent produces untrusted synthetic events
      // that Chrome blocks from triggering default navigation / form
      // submission.  Work around this at the protocol level:
      //
      //   <a href="...">  →  Page.navigate(href)
      //   <button> in form →  form.submit() via JS
      //   everything else  →  mouse events may trigger JS handlers
      let didNavigate = false;
      if (meta.href) {
        const baseUrl = await this.evaluateExpression<string>(ps.session, "location.href").catch(() => "");
        const resolved = baseUrl ? new URL(meta.href, baseUrl).href : meta.href;
        await ps.session.send("Page.navigate", { url: resolved });
        didNavigate = true;
      } else if (meta.inForm) {
        // form.submit() bypasses onsubmit handlers, breaking sites
        // like Baidu that validate/fix-up the form on submit.
        // requestSubmit(button) fires the submit event chain and
        // respects preventDefault() from on-page JS.
        await this.evaluateExpression(
          ps.session,
          `(() => {
            const el = document.querySelector(${JSON.stringify(target.selector)});
            const form = el?.closest?.('form');
            if (form) {
              if (typeof form.requestSubmit === 'function') {
                form.requestSubmit(el);
              } else {
                form.submit();
              }
            }
          })()`,
        );
        // Wait for navigation after submit.
        const navResult = await navObserver.wait(ps.session, navDeadline).catch(() => ({ didNavigate: false }));
        didNavigate = navResult.didNavigate;
      } else {
        // Non-link, non-form element — mouse events may trigger JS handlers.
        const navResult = await navObserver.wait(ps.session, navDeadline).catch(() => ({ didNavigate: false }));
        didNavigate = navResult.didNavigate;
      }
      navObserver.dispose();

      const title = await this.evaluateExpression<string>(ps.session, "document.title").catch(() => "");
      const url = await this.evaluateExpression<string>(ps.session, "location.href").catch(() => "");

      return { pageId: ps.pageId, title, url, didNavigate };
    } finally {
      this.disposePageSession();
    }
  }

  // ── Type ──────────────────────────────────────────────────────────

  async type(
    target: ResolvedTarget,
    text: string,
    options: TypeOptions = {},
  ): Promise<InternalTypeResult> {
    const ps = await this.activePageSession();
    try {
      await installConsoleHook(ps.session);

      const timeoutMs = options.timeoutMs ?? this.timeouts.action;
      const shouldClear = options.clear ?? true;

      await this.waitForActionable(ps.session, target.selector, createDeadline(timeoutMs));

      if (shouldClear) {
        await this.focusAndClear(ps.session, target.selector);
      } else {
        await this.evaluateExpression(
          ps.session, `document.querySelector(${JSON.stringify(target.selector)})?.focus()`,
        );
      }
      await ps.session.send("Input.insertText", { text });

      if (options.submit) {
        await this.dispatchEnter(ps.session);
      }

      return { pageId: ps.pageId, typed: target.selector, submitted: Boolean(options.submit) };
    } finally {
      this.disposePageSession();
    }
  }

  // ── Press ─────────────────────────────────────────────────────────

  async press(
    key: string,
    target?: ResolvedTarget,
    _options: PressOptions = {},
  ): Promise<InternalActionResult> {
    const ps = await this.activePageSession();
    try {
      await installConsoleHook(ps.session);

      const navDeadline = createDeadline(this.timeouts.navigation);
      const navObserver = new ActionNavigationObserver(ps.mainFrameId, await this.getLoaderId(ps.session));
      navObserver.arm(ps.session);

      if (target) {
        await this.waitForActionable(ps.session, target.selector, createDeadline(this.timeouts.action));
        await this.evaluateExpression(
          ps.session, `document.querySelector(${JSON.stringify(target.selector)})?.focus()`,
        );
      }

      const { parseKey } = await import("../input/keyboard.js");
      const keys = parseKey(key);
      for (const k of keys) {
        await withTemporaryPreload(ps.session, async () => {
          // For Enter key, use rawKeyDown + char to trigger form submission
          if (k.key === "Enter" && k.type === "keyDown") {
            await ps.session.send("Input.dispatchKeyEvent", {
              type: "rawKeyDown", key: k.key, code: k.code,
              windowsVirtualKeyCode: k.windowsVirtualKeyCode,
              nativeVirtualKeyCode: k.nativeVirtualKeyCode || undefined,
              modifiers: k.modifiers,
            });
            await ps.session.send("Input.dispatchKeyEvent", {
              type: "char", key: k.key, code: k.code, text: "\r",
              windowsVirtualKeyCode: k.windowsVirtualKeyCode,
              nativeVirtualKeyCode: k.nativeVirtualKeyCode || undefined,
              modifiers: k.modifiers,
            });
          } else {
            await ps.session.send("Input.dispatchKeyEvent", {
              type: k.type, key: k.key, code: k.code,
              text: k.text || undefined,
              windowsVirtualKeyCode: k.windowsVirtualKeyCode,
              nativeVirtualKeyCode: k.nativeVirtualKeyCode || undefined,
              modifiers: k.modifiers,
            });
          }
        });
      }

      const navResult = await navObserver.wait(ps.session, navDeadline).catch(() => ({ didNavigate: false }));
      navObserver.dispose();

      const title = await this.evaluateExpression<string>(ps.session, "document.title").catch(() => "");
      const url = await this.evaluateExpression<string>(ps.session, "location.href").catch(() => "");

      return { pageId: ps.pageId, title, url, didNavigate: navResult.didNavigate };
    } finally {
      this.disposePageSession();
    }
  }

  // ── Tabs ──────────────────────────────────────────────────────────

  async tabs(action: TabsAction): Promise<InternalTabsResult> {
    const { pageMgr } = await this.init();

    if (action.action === "list") {
      const pages = pageMgr.getPages();
      return {
        kind: "list",
        tabs: pages.map((p, i) => ({
          pageId: p.targetId, index: i, title: p.title, url: p.url,
          active: p.targetId === pageMgr.getActivePageId(),
        })),
      };
    }

    if (action.action === "new") {
      const { targetId } = await pageMgr.createPage(action.url ?? "about:blank");
      const page = pageMgr.getPage(targetId)!;
      const pages = pageMgr.getPages();
      return {
        kind: "new",
        page: { pageId: targetId, title: page.title, url: page.url },
        index: pages.findIndex(p => p.targetId === targetId),
      };
    }

    const pages = pageMgr.getPages();
    if (typeof (action as { index: number }).index !== "number") {
      throw new Error("browser command tabs: action requires a valid index.");
    }
    const index = (action as { index: number }).index;

    if (action.action === "select") {
      if (index < 0 || index >= pages.length) throw new Error(`Invalid tab index: ${index}`);
      const page = pages[index]!;
      await pageMgr.activatePage(page.targetId);
      return { kind: "select", page: { pageId: page.targetId, title: page.title, url: page.url } };
    }

    if (action.action === "close") {
      if (index < 0 || index >= pages.length) throw new Error(`Invalid tab index: ${index}`);
      const page = pages[index]!;
      const url = page.url;
      await pageMgr.closePage(page.targetId);
      return { kind: "close", url, index };
    }

    throw new Error(`Unknown tabs action: ${(action as { action: string }).action}`);
  }

  // ── Evaluate ──────────────────────────────────────────────────────

  async evaluate<T>(request: EvaluateRequest): Promise<T> {
    const ps = await this.activePageSession();
    try {
      if (request.kind === "expression") {
        return this.evaluateExpression<T>(ps.session, request.expression);
      }
      return this.evaluateFunction<T>(ps, request.functionDeclaration, request.arguments ?? []);
    } finally {
      this.disposePageSession();
    }
  }

  // ── Screenshot ────────────────────────────────────────────────────

  async captureScreenshot(options: CaptureScreenshotOptions): Promise<Uint8Array> {
    const ps = await this.activePageSession();
    try {
      return captureScreenshot(ps.session, options);
    } finally {
      this.disposePageSession();
    }
  }

  // ── Disconnect ────────────────────────────────────────────────────

  async disconnect(): Promise<void> {
    this.disposePageSession();
    if (this.connection) {
      await this.connection.disconnect().catch(() => {});
      this.connection = null;
      this.browserSess = null;
      this.pageMgr = null;
    }
  }

  // ── Internal helpers ──────────────────────────────────────────────

  private async waitForActionable(
    session: CdpSession,
    selector: string,
    deadline: Deadline,
  ): Promise<void> {
    while (!deadline.expired) {
      const result = await session.send("Runtime.evaluate", {
        expression: `(${ELEMENT_CHECK_SCRIPT})(${JSON.stringify(selector)})`,
        returnByValue: true,
      }) as { result: { value: { exists: boolean; connected: boolean; visible: boolean; disabled: boolean } } };

      const check = result.result?.value;
      if (!check?.exists) { await sleep(100); continue; }
      if (!check.connected || !check.visible) { await sleep(100); continue; }
      if (check.disabled) throw new ElementNotInteractableError(selector, "element is disabled");
      return;
    }
    throw new ElementNotFoundError(selector);
  }

  private async focusAndClear(session: CdpSession, selector: string): Promise<void> {
    await session.send("Runtime.evaluate", {
      expression: `(function(){var el=document.querySelector(${JSON.stringify(selector)});if(el){el.focus();el.select()}})()`,
    });
  }

  /** Send Enter as rawKeyDown + char(\r) to trigger form submission. */
  private async dispatchEnter(session: CdpSession): Promise<void> {
    await session.send("Input.dispatchKeyEvent", {
      type: "rawKeyDown", key: "Enter", code: "Enter",
      windowsVirtualKeyCode: 13, nativeVirtualKeyCode: 36,
    });
    await session.send("Input.dispatchKeyEvent", {
      type: "char", key: "Enter", code: "Enter", text: "\r",
      windowsVirtualKeyCode: 13, nativeVirtualKeyCode: 36,
    });
    await session.send("Input.dispatchKeyEvent", {
      type: "keyUp", key: "Enter", code: "Enter",
      windowsVirtualKeyCode: 13, nativeVirtualKeyCode: 36,
    });
  }

  private async getLoaderId(session: CdpSession): Promise<string> {
    const result = await session.send("Page.getFrameTree") as {
      frameTree: { frame: { loaderId: string } };
    };
    return result.frameTree.frame.loaderId;
  }
}

// ── Module-level helpers ────────────────────────────────────────────

async function getMainFrameState(
  session: CdpSession,
): Promise<{ frameId: string; loaderId: string }> {
  const result = await session.send("Page.getFrameTree") as {
    frameTree: { frame: { id: string; loaderId: string } };
  };
  return {
    frameId: result.frameTree.frame.id,
    loaderId: result.frameTree.frame.loaderId,
  };
}

function sleep(ms: number): Promise<void> {
  return new Promise(resolve => setTimeout(resolve, ms));
}
