import { spawn } from "node:child_process";
import { mkdir as fsMkdir } from "node:fs/promises";
import { createConnection } from "node:net";
import { homedir } from "node:os";
import { join } from "node:path";

import { isRecord } from "../utils/object.js";
import { findBrowser } from "../browser/browser-discovery.js";
import type {
  BrowserSessionFactory,
  BrowserSession,
} from "../browser/backend.js";
import type { BrowserConfig } from "../browser/types.js";
import { DEFAULT_TIMEOUTS } from "../browser/types.js";
import { defaultSessionFactory } from "../browser/default-factory.js";
import { loadBrowserConfig, saveBrowserConfig } from "../browser/browser-state.js";
import { resolveTarget } from "../browser/selector-resolver.js";
import { SNAPSHOT_FUNCTION_SOURCE, type SnapshotResult } from "../browser/scripts/snapshot-script.js";
import { resolveScreenshotPath, writeScreenshot } from "../browser/artifacts/screenshot-writer.js";
import { resolveCdpEndpoint } from "../browser/chromium/chromium-endpoint.js";

const DEFAULT_ENDPOINT = "http://127.0.0.1:9222";
const BROWSER_DIR = join(homedir(), ".future", "agent", "browser");
const DEFAULT_PROFILE_DIR = join(BROWSER_DIR, "profile");

interface LocalToolResult {
  text?: string;
  structuredContent?: Record<string, unknown>;
}

interface BrowserToolEntry {
  description: string;
  args: Record<string, string>;
  example: string;
}

export const BROWSER_TOOL_CATALOG: Record<string, BrowserToolEntry> = {
  browser: {
    description: "Control a local visible Chrome/Edge/Safari browser. Sub-commands: start, status, tabs, open, snapshot, click, type, press, screenshot, console.",
    args: {
      command: '"start" | "status" | "tabs" | "open" | "snapshot" | "click" | "type" | "press" | "scroll" | "screenshot" | "console"',
      // start
      browser: 'string (default: auto, for start: "chrome" | "edge" | "safari")',
      port: "integer (default: 9222)",
      profileDir: "string (default: ~/.future/agent/browser/profile)",
      executablePath: "string (optional)",
      // status
      endpoint: "string (default: saved endpoint or http://127.0.0.1:9222)",
      // tabs
      action: '"list" | "new" | "select" | "close"',
      index: "integer (0-based, for select/close)",
      // open
      url: "string (for open / tabs new)",
      // snapshot
      limit: "integer (default: 80)",
      // click / type
      ref: "string (from snapshot)",
      selector: "string (CSS selector)",
      target: "string (ref or selector)",
      // type
      text: "string (required for type)",
      submit: "boolean (for type)",
      clear: "boolean (default: true, for type)",
      // press
      key: "string (required for press, e.g. Enter, Escape)",
      // screenshot
      fullPage: "boolean",
      path: "string (optional)",
      output: "string (optional alias for path)",
      // scroll
      direction: '"up" | "down" (default: "down")',
      amount: "integer (default: 300, pixels to scroll)",
      // console
      level: '"log" | "info" | "warn" | "error" (optional)',
    },
    example: '{"command": "snapshot"}',
  },
};

export function isBrowserTool(name: string): boolean {
  return name === "browser";
}

export async function callBrowserTool(_name: string, args: Record<string, unknown>): Promise<LocalToolResult> {
  const command = stringArg(args, "command");
  if (!command) throw new Error('browser tool requires "command" argument.');

  switch (command) {
    case "start":
      return browserStart(args);
    case "status":
      return browserStatus(args);
    case "tabs":
      return withSession(args, (ctx) => browserTabs(ctx, args));
    case "open":
      return withSession(args, (ctx) => browserOpen(ctx, args));
    case "snapshot":
      return withSession(args, (ctx) => browserSnapshot(ctx, args));
    case "click":
      return withSession(args, (ctx) => browserClick(ctx, args));
    case "type":
      return withSession(args, (ctx) => browserType(ctx, args));
    case "press":
      return withSession(args, (ctx) => browserPress(ctx, args));
    case "screenshot":
      return withSession(args, (ctx) => browserScreenshot(ctx, args));
    case "scroll":
      return withSession(args, (ctx) => browserScroll(ctx, args));
    case "console":
      return withSession(args, (ctx) => browserConsole(ctx, args));
    default:
      throw new Error(`Unknown browser command: "${command}". Use: start, status, tabs, open, snapshot, click, type, press, scroll, screenshot, console.`);
  }
}

async function browserStart(args: Record<string, unknown>): Promise<LocalToolResult> {
  const requestedPort = numberArg(args, "port") ?? 9222;
  const browserArg = stringArg(args, "browser");

  // Safari path — delegate to SafariManager
  if (browserArg === "safari") {
    try {
      const { SafariManager } = await import("../browser/safari/safari-manager.js");
      const mgr = new SafariManager();
      const result = await mgr.start({ port: requestedPort, url: stringArg(args, "url") });

      // Persist connection config
      if (result.connection.protocol === "webdriver") {
        const config = await loadBrowserConfig();
        config.connection = result.connection;
        config.activeUrl = stringArg(args, "url");
        await saveBrowserConfig(config);
      }

      return {
        structuredContent: {
          endpoint: result.connection.endpoint,
          launcher: result.launcher,
          port: result.port,
          status: result.status,
          browserKind: "safari",
        },
      };
    } catch (e) {
      const { BrowserPermissionError } = await import("../browser/errors.js");
      if (e instanceof BrowserPermissionError) {
        return {
          structuredContent: {
            status: "permission_required",
            browserKind: "safari",
            actionRequired: {
              description: "Safari remote automation is not enabled. This is a one-time setup.",
              steps: [
                "Open Terminal and run: safaridriver --enable",
                "You may be prompted for your password or to confirm in System Settings.",
              ],
              command: e.remedyCommand,
            },
          },
        };
      }
      throw e;
    }
  }

  // Chrome/Edge/Chromium path
  const port = await resolveBrowserPort(requestedPort);
  const endpoint = `http://127.0.0.1:${port}`;

  if (await endpointReachable(endpoint)) {
    const config = await loadBrowserConfig();
    const existingEndpoint = config.connection.endpoint;
    config.connection = {
      protocol: "cdp",
      browserKind: "chromium",
      endpoint,
    };
    await saveBrowserConfig(config);
    return {
      structuredContent: {
        endpoint,
        status: "already_running",
        note: existingEndpoint && existingEndpoint !== endpoint
          ? `Browser endpoint was updated (was ${existingEndpoint}). Subsequent commands will use this browser.`
          : "Browser is already running at this endpoint.",
      },
    };
  }

  const executablePath = stringArg(args, "executablePath");
  const launcher = findBrowserLauncher(executablePath);
  if (!launcher) {
    throw new Error("Could not find Chrome or Edge. Pass executablePath to browser with command=start.");
  }

  const profileDir = stringArg(args, "profileDir") ?? (port === requestedPort ? DEFAULT_PROFILE_DIR : join(BROWSER_DIR, `profile-${port}`));
  const url = stringArg(args, "url") ?? "about:blank";
  await fsMkdir(profileDir, { recursive: true });
  await fsMkdir(BROWSER_DIR, { recursive: true });

  const chromeArgs = [
    `--remote-debugging-port=${port}`,
    `--user-data-dir=${profileDir}`,
    "--no-first-run",
    "--no-default-browser-check",
    url,
  ];
  // On Windows, use cmd /c start (ShellExecute) to launch Chrome as a
  // truly independent process.  ShellExecute does NOT inherit handles
  // from the parent chain, so Chrome won't keep the agent's stdout pipe
  // open.  Direct spawn would pass inheritable handles to Chrome via
  // bInheritHandles, causing the agent to wait forever for EOF.
  if (process.platform === "win32") {
    const startArgs = ["/c", "start", "", launcher.command, ...launcher.args, ...chromeArgs];
    const child = spawn("cmd", startArgs, {
      detached: true,
      stdio: "ignore",
    });
    child.unref();
  } else {
    const child = spawn(launcher.command, [...launcher.args, ...chromeArgs], {
      detached: true,
      stdio: "ignore",
    });
    child.unref();
  }

  const deadline = Date.now() + 10_000;
  while (Date.now() < deadline) {
    if (await endpointReachable(endpoint)) {
      const cfg = await loadBrowserConfig();
      cfg.connection = {
        protocol: "cdp",
        browserKind: "chromium",
        endpoint,
      };
      cfg.activeUrl = url;
      await saveBrowserConfig(cfg);
      return {
        structuredContent: {
          endpoint,
          launcher,
          profileDir,
          port,
          requestedPort,
          status: "started",
        },
      };
    }
    await sleep(250);
  }

  const cfg2 = await loadBrowserConfig();
  cfg2.connection = {
    protocol: "cdp",
    browserKind: "chromium",
    endpoint,
  };
  cfg2.activeUrl = url;
  await saveBrowserConfig(cfg2);
  return {
    structuredContent: {
      endpoint,
      launcher,
      profileDir,
      port,
      requestedPort,
      status: "starting",
      note: "Browser was launched, but the debugging endpoint did not answer within 10 seconds.",
    },
  };
}

async function browserStatus(args: Record<string, unknown>): Promise<LocalToolResult> {
  const endpoint = await endpointFor(args);
  try {
    const response = await fetch(new URL("/json/version", endpoint));
    if (!response.ok) {
      return { structuredContent: { endpoint, reachable: false, status: response.status } };
    }
    const version = await response.json() as unknown;
    return {
      structuredContent: {
        endpoint,
        reachable: true,
        version,
      },
    };
  } catch (error) {
    return {
      structuredContent: {
        endpoint,
        reachable: false,
        error: error instanceof Error ? error.message : String(error),
      },
    };
  }
}

// ── BrowserSession context ──────────────────────────────────────────

interface SessionContext {
  session: BrowserSession;
  config: BrowserConfig;
}

// ── Session factory (module-level default, overridable per-call for tests) ──

let _sessionFactory: BrowserSessionFactory = defaultSessionFactory;

/**
 * Replace the default session factory. Call once at startup, or from
 * tests to inject a fake. NOT exported to CLI users.
 */
export function __setSessionFactoryForTest(factory: BrowserSessionFactory): void {
  _sessionFactory = factory;
}

// ── Session lifecycle ──────────────────────────────────────────────

async function createSession(
  config: BrowserConfig,
  endpoint: string,
): Promise<BrowserSession> {
  const conn = config.connection;

  if (conn.protocol === "cdp") {
    // Refine browserKind from /json/version
    let browserKind = conn.browserKind;
    if (browserKind === "chromium") {
      try {
        const info = await resolveCdpEndpoint(endpoint);
        browserKind = info.browserKind;
        // Atomically update config
        const fresh = await loadBrowserConfig();
        if (fresh.connection.protocol === "cdp" && fresh.connection.browserKind === "chromium") {
          fresh.connection.browserKind = browserKind;
          await saveBrowserConfig(fresh);
        }
      } catch { /* keep "chromium" */ }
    }

    return _sessionFactory({
      protocol: "cdp",
      browserKind: browserKind as "chrome" | "edge" | "chromium",
      endpoint,
      timeouts: DEFAULT_TIMEOUTS,
      activePageId: config.activePageId,
      initTabOrder: config.tabOrder,
    });
  }

  if (!conn.sessionId) throw new Error("sessionId required for webdriver");
  return _sessionFactory({
    protocol: "webdriver",
    browserKind: "safari",
    endpoint,
    sessionId: conn.sessionId,
    timeouts: DEFAULT_TIMEOUTS,
    activePageId: config.activePageId,
  });
}

async function withSession(
  args: Record<string, unknown>,
  fn: (ctx: SessionContext) => Promise<LocalToolResult>,
): Promise<LocalToolResult> {
  const config = await loadBrowserConfig();

  let endpoint = await ensureBrowser(args);
  let session: BrowserSession;

  try {
    session = await createSession(config, endpoint);
  } catch (error) {
    if (stringArg(args, "endpoint")) throw error;
    // Auto-start and retry
    const fallbackPort = (portFromEndpoint(endpoint) ?? 9222) + 1;
    await browserStart({ ...args, port: fallbackPort });
    endpoint = await waitForSavedEndpoint(`http://127.0.0.1:${fallbackPort}`, 10_000);
    session = await createSession(config, endpoint);
  }

  try {
    return await fn({ session, config });
  } finally {
    // disconnect() has a 2s timeout on the WebSocket close handshake,
    // so it won't hang even if Chrome CDP never replies.
    await session.disconnect().catch(() => {});
  }
}

// ── Browser Tabs ───────────────────────────────────────────────────

async function browserTabs(ctx: SessionContext, args: Record<string, unknown>): Promise<LocalToolResult> {
  const action = stringArg(args, "action") ?? "list";
  const session = ctx.session;

  if (action === "list") {
    const result = await session.tabs({ action: "list" });
    const tabs = result.kind === "list" ? result.tabs.map(tab => ({
      index: tab.index,
      title: tab.title,
      url: tab.url,
      active: tab.active,
    })) : [];
    return {
      structuredContent: {
        tabs,
        tabCount: tabs.length,
      },
    };
  }

  if (action === "new") {
    const url = stringArg(args, "url");
    const result = await session.tabs({ action: "new", url });
    if (result.kind !== "new") throw new Error("Unexpected tabs result");
    await saveActivePage(result.page.url, result.page.pageId);
    // Refresh full tab list so the response shape matches "list"
    const fresh = await session.tabs({ action: "list" });
    const tabs = fresh.kind === "list" ? fresh.tabs.map(tab => ({
      index: tab.index,
      title: tab.title,
      url: tab.url,
      active: tab.active,
    })) : [];
    return {
      structuredContent: { tabs, tabCount: tabs.length, created: { index: result.index, url: result.page.url } },
    };
  }

  const index = numberArg(args, "index");
  if (index == null || index < 0) {
    throw new Error(`browser command tabs: action "${action}" requires a valid 0-based index.`);
  }

  if (action === "select") {
    const result = await session.tabs({ action: "select", index });
    if (result.kind !== "select") throw new Error("Unexpected tabs result");
    await saveActivePage(result.page.url, result.page.pageId);
    const fresh = await session.tabs({ action: "list" });
    const tabs = fresh.kind === "list" ? fresh.tabs.map(tab => ({
      index: tab.index,
      title: tab.title,
      url: tab.url,
      active: tab.active,
    })) : [];
    return {
      structuredContent: { tabs, tabCount: tabs.length, selected: { index, url: result.page.url } },
    };
  }

  if (action === "close") {
    const result = await session.tabs({ action: "close", index });
    if (result.kind !== "close") throw new Error("Unexpected tabs result");
    const fresh = await session.tabs({ action: "list" });
    const tabs = fresh.kind === "list" ? fresh.tabs.map(tab => ({
      index: tab.index,
      title: tab.title,
      url: tab.url,
      active: tab.active,
    })) : [];
    return {
      structuredContent: { tabs, tabCount: tabs.length, closed: { index, url: result.url } },
    };
  }

  throw new Error('browser command tabs: action must be "list", "new", "select", or "close".');
}

// ── Browser Open ───────────────────────────────────────────────────

async function browserOpen(ctx: SessionContext, args: Record<string, unknown>): Promise<LocalToolResult> {
  const url = stringArg(args, "url");
  if (!url) throw new Error("browser command open requires url.");
  const page = await ctx.session.open(url);
  await clearRefs();
  await saveActivePage(page.url, page.pageId);
  return { structuredContent: { title: page.title, url: page.url } };
}

// ── Browser Snapshot ───────────────────────────────────────────────

async function browserSnapshot(ctx: SessionContext, args: Record<string, unknown>): Promise<LocalToolResult> {
  const limit = numberArg(args, "limit") ?? 80;
  const snapshot = await ctx.session.evaluate<SnapshotResult>({
    kind: "function",
    functionDeclaration: SNAPSHOT_FUNCTION_SOURCE,
    arguments: [limit],
  });

  const refs: Record<string, string> = {};
  const lines = snapshot.items.map((item) => {
    refs[item.ref] = item.selector;
    const state = [
      item.disabled ? "disabled" : "",
      item.checked != null ? `checked=${item.checked}` : "",
      item.href ? `href=${item.href}` : "",
    ].filter(Boolean).join(" ");
    return `- ${item.role} "${item.name}" [ref=${item.ref}]${state ? ` ${state}` : ""}`;
  });

  await saveRefsAndUrl(refs, snapshot.url);

  return {
    text: [`Page: ${snapshot.title}`, `URL: ${snapshot.url}`, "", ...lines].join("\n"),
    structuredContent: {
      title: snapshot.title,
      url: snapshot.url,
      elements: snapshot.items.map(({ selector: _s, ...item }) => item),
    },
  };
}

// ── Browser Click ──────────────────────────────────────────────────

async function browserClick(ctx: SessionContext, args: Record<string, unknown>): Promise<LocalToolResult> {
  const target = await resolveTargetFromArgs(args, ctx.config);
  const result = await ctx.session.click(target);
  await saveActivePage(result.url, result.pageId);
  return { structuredContent: { clicked: target.original, selector: target.selector, title: result.title, url: result.url } };
}

// ── Browser Type ───────────────────────────────────────────────────

async function browserType(ctx: SessionContext, args: Record<string, unknown>): Promise<LocalToolResult> {
  const text = stringArg(args, "text");
  if (text == null) throw new Error("browser command type requires text.");
  const target = await resolveTargetFromArgs(args, ctx.config);
  const clear = booleanArg(args, "clear") ?? true;
  const submit = booleanArg(args, "submit") ?? false;
  const result = await ctx.session.type(target, text, { clear, submit });
  return { structuredContent: { typed: target.original, selector: target.selector, submitted: result.submitted } };
}

// ── Browser Press ──────────────────────────────────────────────────

async function browserPress(ctx: SessionContext, args: Record<string, unknown>): Promise<LocalToolResult> {
  const key = stringArg(args, "key");
  if (!key) throw new Error("browser command press requires key.");
  const target = await resolveTargetFromArgsOptional(args, ctx.config);
  const result = await ctx.session.press(key, target);
  await saveActivePage(result.url, result.pageId);
  return { structuredContent: { key, title: result.title, url: result.url } };
}

// ── Browser Screenshot ─────────────────────────────────────────────

async function browserScreenshot(ctx: SessionContext, args: Record<string, unknown>): Promise<LocalToolResult> {
  const explicitPath = stringArg(args, "path") ?? stringArg(args, "output");
  const path = resolveScreenshotPath(explicitPath);
  const bytes = await ctx.session.captureScreenshot({
    fullPage: Boolean(booleanArg(args, "fullPage")),
    format: "png",
  });
  const { path: finalPath, filename } = await writeScreenshot(bytes, path);
  const title = await ctx.session.evaluate<string>({ kind: "expression", expression: "document.title" }).catch(() => "");
  const url = await ctx.session.evaluate<string>({ kind: "expression", expression: "location.href" }).catch(() => "");
  return { structuredContent: { path: finalPath, filename, title, url } };
}

// ── Browser Scroll ────────────────────────────────────────────────

async function browserScroll(ctx: SessionContext, args: Record<string, unknown>): Promise<LocalToolResult> {
  const direction = stringArg(args, "direction") ?? "down";
  const amount = numberArg(args, "amount") ?? 300;
  const target = stringArg(args, "ref") ?? stringArg(args, "selector");

  const px = direction === "down" || direction === "up" ? 0 : amount;
  const py = direction === "down" ? amount : direction === "up" ? -amount : 0;

  if (target) {
    // Scroll a specific element
    const resolved = await resolveTargetFromArgsOptional(args, ctx.config);
    await ctx.session.evaluate({
      kind: "function",
      functionDeclaration: `function(sel, x, y) { var el = document.querySelector(sel); if (el) el.scrollBy({ left: x, top: y, behavior: 'smooth' }); }`,
      arguments: [resolved?.selector ?? target, px, py],
    });
  } else {
    // Scroll the page
    await ctx.session.evaluate({
      kind: "function",
      functionDeclaration: `function(x, y) { window.scrollBy({ left: x, top: y, behavior: 'smooth' }); }`,
      arguments: [px, py],
    });
  }

  return { structuredContent: { scrolled: { direction, amount, target: target ?? "page" } } };
}

// ── Browser Console ────────────────────────────────────────────────

async function browserConsole(ctx: SessionContext, args: Record<string, unknown>): Promise<LocalToolResult> {
  const level = stringArg(args, "level");
  const raw = await ctx.session.evaluate<unknown>({
    kind: "expression",
    expression: "(globalThis.__futureConsoleLogs) || []",
  });
  const logs = Array.isArray(raw) ? raw.filter(isRecord).map(e => e as Record<string, unknown>) : [];
  const filtered = logs.filter(e => !level || e.level === level);
  return {
    structuredContent: {
      logs: filtered,
      note: filtered.length === 0
        ? "No buffered console messages. The hook captures messages after a Future browser tool has touched the page."
        : undefined,
    },
  };
}

// ── Helpers ─────────────────────────────────────────────────────────

async function resolveTargetFromArgs(
  args: Record<string, unknown>,
  config: BrowserConfig,
): Promise<{ original: string; source: "ref" | "selector"; selector: string; ref?: string }> {
  const input = stringArg(args, "selector")
    ?? stringArg(args, "target")
    ?? stringArg(args, "ref");
  if (!input) throw new Error("Expected ref, selector, or target.");
  return resolveTarget(input, config);
}

async function resolveTargetFromArgsOptional(
  args: Record<string, unknown>,
  config: BrowserConfig,
): Promise<{ original: string; source: "ref" | "selector"; selector: string; ref?: string } | undefined> {
  const input = stringArg(args, "selector")
    ?? stringArg(args, "target")
    ?? stringArg(args, "ref");
  if (!input) return undefined;
  // If it's a ref but optional, just use as selector (for press on page body)
  try {
    return resolveTarget(input, config);
  } catch {
    return undefined;
  }
}

async function saveActivePage(url: string, pageId?: string): Promise<void> {
  const config = await loadBrowserConfig();
  config.activeUrl = url;
  if (pageId) config.activePageId = pageId;
  await saveBrowserConfig(config);
}

async function clearRefs(): Promise<void> {
  const config = await loadBrowserConfig();
  config.refs = {};
  await saveBrowserConfig(config);
}

async function saveRefsAndUrl(refs: Record<string, string>, url: string): Promise<void> {
  const config = await loadBrowserConfig();
  config.refs = refs;
  config.activeUrl = url;
  await saveBrowserConfig(config);
}

async function endpointFor(args: Record<string, unknown>): Promise<string> {
  const config = await loadBrowserConfig();
  return stringArg(args, "endpoint") ?? config.connection.endpoint ?? DEFAULT_ENDPOINT;
}

function portFromEndpoint(endpoint: string): number | null {
  try {
    const port = Number(new URL(endpoint).port);
    return Number.isFinite(port) ? port : null;
  } catch {
    return null;
  }
}

async function ensureBrowser(args: Record<string, unknown>): Promise<string> {
  const explicitEndpoint = stringArg(args, "endpoint");
  const endpoint = await endpointFor(args);
  if (await endpointReachable(endpoint)) return endpoint;

  if (explicitEndpoint) {
    throw new Error(
      `Local browser endpoint is not reachable: ${explicitEndpoint}. Check the browser was started with a reachable remote debugging endpoint.`,
    );
  }

  await browserStart(args);
  return waitForSavedEndpoint(DEFAULT_ENDPOINT, 10_000);
}

async function waitForSavedEndpoint(fallbackEndpoint: string, timeoutMs: number): Promise<string> {
  const config = await loadBrowserConfig();
  const startedEndpoint = config.connection.endpoint ?? fallbackEndpoint;
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    if (await endpointReachable(startedEndpoint)) return startedEndpoint;
    await sleep(250);
  }

  throw new Error(`Local browser endpoint is not reachable after auto-start: ${startedEndpoint}.`);
}

async function endpointReachable(endpoint: string): Promise<boolean> {
  try {
    const response = await fetch(new URL("/json/version", endpoint), { signal: AbortSignal.timeout(1000) });
    return response.ok;
  } catch {
    return false;
  }
}

async function resolveBrowserPort(requestedPort: number): Promise<number> {
  const endpoint = `http://127.0.0.1:${requestedPort}`;
  if (await endpointReachable(endpoint)) return requestedPort;
  if (!await portHasListener(requestedPort)) return requestedPort;

  for (let port = requestedPort + 1; port < requestedPort + 50; port += 1) {
    if (!await portHasListener(port)) return port;
  }

  throw new Error(`No available browser debugging port found near ${requestedPort}.`);
}

function portHasListener(port: number): Promise<boolean> {
  return new Promise((resolve) => {
    const socket = createConnection({ host: "127.0.0.1", port });
    socket.setTimeout(500);
    socket.once("connect", () => {
      socket.destroy();
      resolve(true);
    });
    socket.once("timeout", () => {
      socket.destroy();
      resolve(false);
    });
    socket.once("error", () => resolve(false));
  });
}

function findBrowserLauncher(executablePath?: string): { command: string; args: string[] } | null {
  const discovered = findBrowser(executablePath);
  if (!discovered) return null;
  return { command: discovered.executablePath, args: [] };
}

function stringArg(args: Record<string, unknown>, key: string): string | undefined {
  const value = args[key];
  return typeof value === "string" && value.length > 0 ? value : undefined;
}

function numberArg(args: Record<string, unknown>, key: string): number | undefined {
  const value = args[key];
  return typeof value === "number" && Number.isFinite(value) ? value : undefined;
}

function booleanArg(args: Record<string, unknown>, key: string): boolean | undefined {
  const value = args[key];
  return typeof value === "boolean" ? value : undefined;
}

function sleep(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms));
}
