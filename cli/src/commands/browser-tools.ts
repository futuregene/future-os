import { spawn } from "node:child_process";
import { existsSync } from "node:fs";
import { mkdir, readFile, writeFile } from "node:fs/promises";
import { homedir, platform } from "node:os";
import { basename, dirname, join } from "node:path";

import { isRecord } from "../utils/object.js";

const DEFAULT_ENDPOINT = "http://127.0.0.1:9222";
const BROWSER_DIR = join(homedir(), ".future", "agent", "browser");
const CONFIG_FILE = join(BROWSER_DIR, "config.json");
const DEFAULT_PROFILE_DIR = join(BROWSER_DIR, "profile");
const ARTIFACTS_DIR = join(BROWSER_DIR, "artifacts");

interface BrowserConfig {
  endpoint?: string;
  activeUrl?: string;
  refs?: Record<string, string>;
}

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
  browser_start: {
    description: "Start a visible local Chrome/Edge browser with a remote debugging port.",
    args: {
      port: "integer (default: 9222)",
      profileDir: "string (default: ~/.future/agent/browser/profile)",
      executablePath: "string (optional)",
      url: "string (optional)",
    },
    example: '{"port": 9222, "url": "http://localhost:3000"}',
  },
  browser_status: {
    description: "Check whether a local browser debugging endpoint is reachable.",
    args: { endpoint: "string (default: saved endpoint or http://127.0.0.1:9222)" },
    example: '{}',
  },
  browser_tabs: {
    description: "List, create, select, or close tabs in the connected local browser.",
    args: {
      action: '"list" | "new" | "select" | "close"',
      index: "integer (0-based, for select/close)",
      url: "string (optional, for new)",
    },
    example: '{"action": "list"}',
  },
  browser_open: {
    description: "Open a URL in the active local browser tab.",
    args: { url: "string (required)" },
    example: '{"url": "http://localhost:3000"}',
  },
  browser_snapshot: {
    description: "Return a compact visible DOM snapshot with stable refs for browser_click/browser_type.",
    args: { limit: "integer (default: 80)" },
    example: '{}',
  },
  browser_click: {
    description: "Click an element by ref from browser_snapshot or by explicit selector.",
    args: { ref: "string", selector: "string", target: "string (ref or selector)" },
    example: '{"ref": "b1"}',
  },
  browser_type: {
    description: "Fill or type text into an element by ref from browser_snapshot or by selector.",
    args: {
      ref: "string",
      selector: "string",
      text: "string (required)",
      submit: "boolean",
      clear: "boolean (default: true)",
    },
    example: '{"ref": "i1", "text": "alice@example.com"}',
  },
  browser_press: {
    description: "Press a keyboard key in the active local browser tab.",
    args: { key: "string (required, e.g. Enter, Escape, ArrowDown)" },
    example: '{"key": "Enter"}',
  },
  browser_screenshot: {
    description: "Capture a screenshot of the active local browser tab and save it locally.",
    args: {
      fullPage: "boolean",
      path: "string (optional)",
      output: "string (optional alias for path)",
    },
    example: '{"fullPage": true}',
  },
  browser_console: {
    description: "Read console messages captured after Future browser tooling installed its page hook.",
    args: { level: '"log" | "info" | "warn" | "error" (optional)' },
    example: '{"level": "error"}',
  },
};

export function isBrowserTool(name: string): boolean {
  return Object.hasOwn(BROWSER_TOOL_CATALOG, name);
}

export async function callBrowserTool(name: string, args: Record<string, unknown>): Promise<LocalToolResult> {
  switch (name) {
    case "browser_start":
      return browserStart(args);
    case "browser_status":
      return browserStatus(args);
    case "browser_tabs":
      return withBrowser(args, (ctx) => browserTabs(ctx, args));
    case "browser_open":
      return withBrowser(args, (ctx) => browserOpen(ctx, args));
    case "browser_snapshot":
      return withBrowser(args, (ctx) => browserSnapshot(ctx, args));
    case "browser_click":
      return withBrowser(args, (ctx) => browserClick(ctx, args));
    case "browser_type":
      return withBrowser(args, (ctx) => browserType(ctx, args));
    case "browser_press":
      return withBrowser(args, (ctx) => browserPress(ctx, args));
    case "browser_screenshot":
      return withBrowser(args, (ctx) => browserScreenshot(ctx, args));
    case "browser_console":
      return withBrowser(args, (ctx) => browserConsole(ctx, args));
    default:
      throw new Error(`Unknown browser tool: ${name}`);
  }
}

async function browserStart(args: Record<string, unknown>): Promise<LocalToolResult> {
  const port = numberArg(args, "port") ?? 9222;
  const endpoint = `http://127.0.0.1:${port}`;

  if (await endpointReachable(endpoint)) {
    await saveConfig({ ...(await loadConfig()), endpoint });
    return { structuredContent: { endpoint, status: "already_running" } };
  }

  const executablePath = stringArg(args, "executablePath") ?? findBrowserExecutable();
  if (!executablePath) {
    throw new Error("Could not find Chrome or Edge. Pass executablePath to browser_start.");
  }

  const profileDir = stringArg(args, "profileDir") ?? DEFAULT_PROFILE_DIR;
  const url = stringArg(args, "url") ?? "about:blank";
  await mkdir(profileDir, { recursive: true });
  await mkdir(BROWSER_DIR, { recursive: true });

  const child = spawn(executablePath, [
    `--remote-debugging-port=${port}`,
    `--user-data-dir=${profileDir}`,
    "--no-first-run",
    "--no-default-browser-check",
    url,
  ], {
    detached: true,
    stdio: "ignore",
  });
  child.unref();

  const deadline = Date.now() + 10_000;
  while (Date.now() < deadline) {
    if (await endpointReachable(endpoint)) {
      await saveConfig({ ...(await loadConfig()), endpoint, activeUrl: url });
      return {
        structuredContent: {
          endpoint,
          executablePath,
          profileDir,
          status: "started",
        },
      };
    }
    await sleep(250);
  }

  await saveConfig({ ...(await loadConfig()), endpoint, activeUrl: url });
  return {
    structuredContent: {
      endpoint,
      executablePath,
      profileDir,
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

interface BrowserContext {
  browser: import("playwright-core").Browser;
  page: import("playwright-core").Page;
  config: BrowserConfig;
}

async function withBrowser(
  args: Record<string, unknown>,
  fn: (ctx: BrowserContext) => Promise<LocalToolResult>,
): Promise<LocalToolResult> {
  const { chromium } = await import("playwright-core");
  const endpoint = await endpointFor(args);

  if (!await endpointReachable(endpoint)) {
    throw new Error(
      `Local browser endpoint is not reachable: ${endpoint}. Run ` +
        `future tools call browser_start --args '{}' first, or start Chrome with --remote-debugging-port=9222.`,
    );
  }

  const browser = await chromium.connectOverCDP(endpoint);
  const config = await loadConfig();
  const page = await activePage(browser, config);
  await installConsoleHook(page);

  const result = await fn({ browser, page, config });
  return result;
}

async function browserTabs(ctx: BrowserContext, args: Record<string, unknown>): Promise<LocalToolResult> {
  const action = stringArg(args, "action") ?? "list";
  const pages = allPages(ctx.browser);

  if (action === "list") {
    return {
      structuredContent: {
        tabs: await Promise.all(pages.map(async (page, index) => ({
          index,
          title: await page.title().catch(() => ""),
          url: page.url(),
          active: page.url() === ctx.config.activeUrl,
        }))),
      },
    };
  }

  if (action === "new") {
    const context = ctx.browser.contexts()[0] ?? await ctx.browser.newContext();
    const page = await context.newPage();
    const url = stringArg(args, "url");
    if (url) await page.goto(url, { waitUntil: "domcontentloaded" });
    await saveConfig({ ...(await loadConfig()), activeUrl: page.url() });
    return { structuredContent: { index: allPages(ctx.browser).indexOf(page), url: page.url(), title: await page.title() } };
  }

  const index = numberArg(args, "index");
  if (index == null || index < 0 || index >= pages.length) {
    throw new Error(`browser_tabs action "${action}" requires a valid 0-based index.`);
  }

  if (action === "select") {
    const page = pages[index]!;
    await page.bringToFront().catch(() => undefined);
    await saveConfig({ ...(await loadConfig()), activeUrl: page.url() });
    return { structuredContent: { selected: index, url: page.url(), title: await page.title().catch(() => "") } };
  }

  if (action === "close") {
    const page = pages[index]!;
    const url = page.url();
    await page.close();
    return { structuredContent: { closed: index, url } };
  }

  throw new Error('browser_tabs action must be "list", "new", "select", or "close".');
}

async function browserOpen(ctx: BrowserContext, args: Record<string, unknown>): Promise<LocalToolResult> {
  const url = stringArg(args, "url");
  if (!url) throw new Error("browser_open requires url.");
  await ctx.page.goto(url, { waitUntil: "domcontentloaded" });
  await installConsoleHook(ctx.page);
  await saveConfig({ ...(await loadConfig()), activeUrl: ctx.page.url(), refs: {} });
  return {
    structuredContent: {
      title: await ctx.page.title().catch(() => ""),
      url: ctx.page.url(),
    },
  };
}

async function browserSnapshot(ctx: BrowserContext, args: Record<string, unknown>): Promise<LocalToolResult> {
  const limit = numberArg(args, "limit") ?? 80;
  const snapshot = await ctx.page.evaluate((maxItems) => {
    type Item = {
      ref: string;
      selector: string;
      role: string;
      name: string;
      tag: string;
      disabled: boolean;
      checked: boolean | null;
      href: string | null;
    };

    const escapeCss = (value: string) => {
      const css = globalThis.CSS as { escape?: (input: string) => string } | undefined;
      return css?.escape ? css.escape(value) : value.replace(/["\\]/g, "\\$&");
    };
    const textOf = (element: Element) => {
      if (element instanceof HTMLInputElement || element instanceof HTMLTextAreaElement) {
        return element.getAttribute("aria-label") ||
          element.getAttribute("placeholder") ||
          element.name ||
          element.value ||
          "";
      }
      if (element instanceof HTMLImageElement) return element.alt || element.title || "";
      return element.getAttribute("aria-label") ||
        element.getAttribute("title") ||
        (element.textContent || "").replace(/\s+/g, " ").trim();
    };
    const roleOf = (element: Element) => {
      const explicit = element.getAttribute("role");
      if (explicit) return explicit;
      const tag = element.tagName.toLowerCase();
      if (tag === "a") return "link";
      if (tag === "button") return "button";
      if (tag === "select") return "combobox";
      if (tag === "textarea") return "textbox";
      if (tag === "summary") return "button";
      if (tag === "input") {
        const type = (element.getAttribute("type") || "text").toLowerCase();
        if (["button", "submit", "reset"].includes(type)) return "button";
        if (type === "checkbox") return "checkbox";
        if (type === "radio") return "radio";
        return "textbox";
      }
      return tag;
    };
    const uniqueSelector = (element: Element) => {
      const id = element.getAttribute("id");
      if (id && document.querySelectorAll(`#${escapeCss(id)}`).length === 1) return `#${escapeCss(id)}`;
      for (const attr of ["data-testid", "data-test", "data-cy", "name", "aria-label"]) {
        const value = element.getAttribute(attr);
        if (!value) continue;
        const selector = `${element.tagName.toLowerCase()}[${attr}="${escapeCss(value)}"]`;
        if (document.querySelectorAll(selector).length === 1) return selector;
      }
      const parts: string[] = [];
      let current: Element | null = element;
      while (current && current !== document.documentElement) {
        const tag = current.tagName.toLowerCase();
        const currentTag = current.tagName;
        const parent: Element | null = current.parentElement;
        if (!parent) break;
        const siblings = Array.from(parent.children).filter((child: Element) => child.tagName === currentTag);
        const index = siblings.indexOf(current) + 1;
        parts.unshift(siblings.length > 1 ? `${tag}:nth-of-type(${index})` : tag);
        const selector = parts.join(" > ");
        if (document.querySelectorAll(selector).length === 1) return selector;
        current = parent;
      }
      return parts.join(" > ");
    };
    const isVisible = (element: Element) => {
      const rect = element.getBoundingClientRect();
      const style = getComputedStyle(element);
      return rect.width > 0 &&
        rect.height > 0 &&
        style.visibility !== "hidden" &&
        style.display !== "none" &&
        Number(style.opacity || "1") > 0;
    };

    const candidates = Array.from(document.querySelectorAll(
      "a[href],button,input,textarea,select,summary,[role],[contenteditable='true'],[tabindex]",
    ));
    const items: Item[] = [];
    let counter = 1;
    for (const element of candidates) {
      if (items.length >= maxItems) break;
      if (!isVisible(element)) continue;
      const tag = element.tagName.toLowerCase();
      const role = roleOf(element);
      const name = textOf(element).slice(0, 120);
      if (!name && !["input", "textarea", "select"].includes(tag)) continue;
      const prefix = role === "button" ? "b" : role === "textbox" ? "i" : role === "link" ? "a" : "e";
      items.push({
        ref: `${prefix}${counter++}`,
        selector: uniqueSelector(element),
        role,
        name,
        tag,
        disabled: Boolean((element as HTMLButtonElement | HTMLInputElement).disabled),
        checked: element instanceof HTMLInputElement && ["checkbox", "radio"].includes(element.type)
          ? element.checked
          : null,
        href: element instanceof HTMLAnchorElement ? element.href : null,
      });
    }
    return {
      title: document.title,
      url: location.href,
      items,
    };
  }, limit);

  const refs: Record<string, string> = {};
  const lines = snapshot.items.map((item) => {
    refs[item.ref] = item.selector;
    const state = [
      item.disabled ? "disabled" : "",
      item.checked == null ? "" : `checked=${item.checked}`,
      item.href ? `href=${item.href}` : "",
    ].filter(Boolean).join(" ");
    return `- ${item.role} "${item.name}" [ref=${item.ref}]${state ? ` ${state}` : ""}`;
  });

  await saveConfig({ ...(await loadConfig()), activeUrl: snapshot.url, refs });

  return {
    text: [`Page: ${snapshot.title}`, `URL: ${snapshot.url}`, "", ...lines].join("\n"),
    structuredContent: {
      title: snapshot.title,
      url: snapshot.url,
      elements: snapshot.items.map(({ selector: _selector, ...item }) => item),
    },
  };
}

async function browserClick(ctx: BrowserContext, args: Record<string, unknown>): Promise<LocalToolResult> {
  const selector = await selectorFor(args);
  const locator = ctx.page.locator(selector);
  const count = await locator.count();
  if (count !== 1) throw new Error(`browser_click target resolved to ${count} elements; run browser_snapshot and use a unique ref.`);
  await locator.click();
  await ctx.page.waitForLoadState("domcontentloaded", { timeout: 3000 }).catch(() => undefined);
  await saveConfig({ ...(await loadConfig()), activeUrl: ctx.page.url() });
  return { structuredContent: { clicked: selector, title: await ctx.page.title().catch(() => ""), url: ctx.page.url() } };
}

async function browserType(ctx: BrowserContext, args: Record<string, unknown>): Promise<LocalToolResult> {
  const text = stringArg(args, "text");
  if (text == null) throw new Error("browser_type requires text.");
  const selector = await selectorFor(args);
  const locator = ctx.page.locator(selector);
  const count = await locator.count();
  if (count !== 1) throw new Error(`browser_type target resolved to ${count} elements; run browser_snapshot and use a unique ref.`);
  if (booleanArg(args, "clear") ?? true) {
    await locator.fill(text);
  } else {
    await locator.type(text);
  }
  if (booleanArg(args, "submit")) await locator.press("Enter");
  return { structuredContent: { typed: selector, submitted: Boolean(booleanArg(args, "submit")) } };
}

async function browserPress(ctx: BrowserContext, args: Record<string, unknown>): Promise<LocalToolResult> {
  const key = stringArg(args, "key");
  if (!key) throw new Error("browser_press requires key.");
  await ctx.page.keyboard.press(key);
  await ctx.page.waitForLoadState("domcontentloaded", { timeout: 3000 }).catch(() => undefined);
  await saveConfig({ ...(await loadConfig()), activeUrl: ctx.page.url() });
  return { structuredContent: { key, title: await ctx.page.title().catch(() => ""), url: ctx.page.url() } };
}

async function browserScreenshot(ctx: BrowserContext, args: Record<string, unknown>): Promise<LocalToolResult> {
  const explicitPath = stringArg(args, "path") ?? stringArg(args, "output");
  const path = explicitPath ?? join(ARTIFACTS_DIR, `browser-${new Date().toISOString().replace(/[:.]/g, "-")}.png`);
  await mkdir(dirname(path), { recursive: true }).catch(async () => mkdir(ARTIFACTS_DIR, { recursive: true }));
  await ctx.page.screenshot({ path, fullPage: Boolean(booleanArg(args, "fullPage")) });
  return {
    structuredContent: {
      path,
      filename: basename(path),
      title: await ctx.page.title().catch(() => ""),
      url: ctx.page.url(),
    },
  };
}

async function browserConsole(ctx: BrowserContext, args: Record<string, unknown>): Promise<LocalToolResult> {
  const level = stringArg(args, "level");
  const logs = await ctx.page.evaluate(() => {
    const value = (globalThis as unknown as { __futureConsoleLogs?: unknown }).__futureConsoleLogs;
    return Array.isArray(value) ? value : [];
  });
  const filtered = logs
    .filter(isRecord)
    .map((entry) => entry as Record<string, unknown>)
    .filter((entry) => !level || entry.level === level);
  return {
    structuredContent: {
      logs: filtered,
      note: filtered.length === 0
        ? "No buffered console messages. The hook captures messages after a Future browser tool has touched the page."
        : undefined,
    },
  };
}

async function activePage(browser: import("playwright-core").Browser, config: BrowserConfig): Promise<import("playwright-core").Page> {
  const pages = allPages(browser);
  const byUrl = config.activeUrl ? pages.find((page) => page.url() === config.activeUrl) : undefined;
  if (byUrl) return byUrl;
  if (pages.length > 0) return pages[pages.length - 1]!;
  const context = browser.contexts()[0] ?? await browser.newContext();
  return context.newPage();
}

function allPages(browser: import("playwright-core").Browser): import("playwright-core").Page[] {
  return browser.contexts().flatMap((context) => context.pages());
}

async function selectorFor(args: Record<string, unknown>): Promise<string> {
  const selector = stringArg(args, "selector");
  if (selector) return selector;

  const target = stringArg(args, "target");
  const ref = stringArg(args, "ref") ?? (target && /^[a-z]\d+$/i.test(target) ? target : undefined);
  if (ref) {
    const config = await loadConfig();
    const resolved = config.refs?.[ref];
    if (!resolved) throw new Error(`Unknown browser ref "${ref}". Run browser_snapshot first.`);
    return resolved;
  }

  if (target) return target;
  throw new Error("Expected ref, selector, or target.");
}

async function installConsoleHook(page: import("playwright-core").Page): Promise<void> {
  await page.evaluate(() => {
    const target = globalThis as unknown as {
      __futureConsoleHookInstalled?: boolean;
      __futureConsoleLogs?: Array<{ level: string; text: string; time: string }>;
      console: Console;
    };
    if (target.__futureConsoleHookInstalled) return;
    target.__futureConsoleHookInstalled = true;
    target.__futureConsoleLogs = target.__futureConsoleLogs ?? [];
    for (const level of ["log", "info", "warn", "error"] as const) {
      const original = target.console[level].bind(target.console);
      target.console[level] = (...values: unknown[]) => {
        target.__futureConsoleLogs!.push({
          level,
          text: values.map((value) => {
            try {
              return typeof value === "string" ? value : JSON.stringify(value);
            } catch {
              return String(value);
            }
          }).join(" "),
          time: new Date().toISOString(),
        });
        if (target.__futureConsoleLogs!.length > 200) target.__futureConsoleLogs!.shift();
        original(...values);
      };
    }
  }).catch(() => undefined);
}

async function endpointFor(args: Record<string, unknown>): Promise<string> {
  return stringArg(args, "endpoint") ?? (await loadConfig()).endpoint ?? DEFAULT_ENDPOINT;
}

async function endpointReachable(endpoint: string): Promise<boolean> {
  try {
    const response = await fetch(new URL("/json/version", endpoint), { signal: AbortSignal.timeout(1000) });
    return response.ok;
  } catch {
    return false;
  }
}

async function loadConfig(): Promise<BrowserConfig> {
  try {
    const raw = await readFile(CONFIG_FILE, "utf8");
    const parsed = JSON.parse(raw) as unknown;
    return isRecord(parsed) ? parsed as BrowserConfig : {};
  } catch {
    return {};
  }
}

async function saveConfig(config: BrowserConfig): Promise<void> {
  await mkdir(BROWSER_DIR, { recursive: true });
  await writeFile(CONFIG_FILE, `${JSON.stringify(config, null, 2)}\n`);
}

function findBrowserExecutable(): string | null {
  if (platform() === "darwin") {
    return [
      "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
      "/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge",
      "/Applications/Chromium.app/Contents/MacOS/Chromium",
    ].find((candidate) => existsSync(candidate)) ?? null;
  }
  if (platform() === "win32") {
    const local = process.env["LOCALAPPDATA"];
    const programFiles = process.env["PROGRAMFILES"];
    const programFilesX86 = process.env["PROGRAMFILES(X86)"];
    return [
      local ? join(local, "Google", "Chrome", "Application", "chrome.exe") : "",
      programFiles ? join(programFiles, "Google", "Chrome", "Application", "chrome.exe") : "",
      programFilesX86 ? join(programFilesX86, "Microsoft", "Edge", "Application", "msedge.exe") : "",
      programFiles ? join(programFiles, "Microsoft", "Edge", "Application", "msedge.exe") : "",
    ].filter(Boolean).find((candidate) => existsSync(candidate)) ?? null;
  }
  return process.env["CHROME_PATH"] ?? "google-chrome";
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
