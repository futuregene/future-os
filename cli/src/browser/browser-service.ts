/**
 * BrowserService — business logic that sits between the CLI facade
 * and BrowserManager/BrowserSession backends.
 *
 * Handles:
 * - Snapshot generation with refs
 * - Console log reading
 * - Screenshot path resolution and file writing
 * - State persistence (config.json)
 * - Error mapping and output formatting
 */
import { mkdir, writeFile } from "node:fs/promises";
import { basename, dirname, join } from "node:path";
import { homedir } from "node:os";
import type { BrowserConfig, PageId } from "./types.js";
import { loadBrowserConfig, saveBrowserConfig } from "./browser-state.js";
import { reconcilePageOrder, resolveActivePage, insertNewPage, removePage } from "./tab-order.js";
import { resolveTarget, legacySelectorFor } from "./selector-resolver.js";
import type { SnapshotItem } from "./scripts/snapshot-script.js";
import type {
  BrowserSession,
  InternalPageInfo,
  InternalActionResult,
  ResolvedTarget,
} from "./backend.js";
import { isRecord } from "../utils/object.js";

const FUTURE_HOME = process.env["FUTURE_HOME"] ?? join(homedir(), ".future");
const BROWSER_DIR = join(FUTURE_HOME, "agent", "browser");
const ARTIFACTS_DIR = join(BROWSER_DIR, "artifacts");

// ── Public service ──────────────────────────────────────────────────

export class BrowserService {
  private config: BrowserConfig | null = null;

  async getConfig(): Promise<BrowserConfig> {
    if (!this.config) this.config = await loadBrowserConfig();
    return this.config;
  }

  async saveConfig(config: BrowserConfig): Promise<void> {
    this.config = config;
    await saveBrowserConfig(config);
  }

  // ── Refs ──────────────────────────────────────────────────────────

  /** Build a refs map from a snapshot result. */
  buildRefs(items: SnapshotItem[]): Record<string, string> {
    const refs: Record<string, string> = {};
    for (const item of items) {
      refs[item.ref] = item.selector;
    }
    return refs;
  }

  /** Format snapshot as text output. */
  formatSnapshotText(
    title: string,
    url: string,
    items: SnapshotItem[],
  ): string {
    const lines: string[] = [`Page: ${title}`, `URL: ${url}`, ""];
    for (const item of items) {
      const state = [
        item.disabled ? "disabled" : "",
        item.checked != null ? `checked=${item.checked}` : "",
        item.href ? `href=${item.href}` : "",
      ].filter(Boolean).join(" ");
      lines.push(`- ${item.role} "${item.name}" [ref=${item.ref}]${state ? ` ${state}` : ""}`);
    }
    return lines.join("\n");
  }

  /** Format snapshot structured content (without selectors — external output). */
  formatSnapshotStructured(
    title: string,
    url: string,
    items: SnapshotItem[],
  ): Record<string, unknown> {
    return {
      title,
      url,
      elements: items.map(({ selector: _, ...item }) => item),
    };
  }

  /** Resolve target from args using the persisted config. */
  async resolveTargetFromArgs(
    args: Record<string, unknown>,
  ): Promise<ResolvedTarget> {
    const config = await this.getConfig();
    return resolveTarget(
      (typeof args.selector === "string" ? args.selector : undefined) ??
        (typeof args.target === "string" ? args.target : undefined) ??
        (typeof args.ref === "string" ? args.ref : undefined),
      config,
    );
  }

  /** Legacy selectorFor — preserves exact current behavior. */
  async legacySelectorFor(
    args: Record<string, unknown>,
  ): Promise<string> {
    const config = await this.getConfig();
    return legacySelectorFor(args, config);
  }

  // ── Screenshot writer ─────────────────────────────────────────────

  async resolveScreenshotPath(explicitPath?: string): Promise<string> {
    if (explicitPath) return explicitPath;
    const ts = new Date().toISOString().replace(/[:.]/g, "-");
    return join(ARTIFACTS_DIR, `browser-${ts}.png`);
  }

  async writeScreenshot(bytes: Uint8Array, path: string): Promise<void> {
    // Try writing — if parent missing, create artifacts dir and retry
    try {
      await mkdir(dirname(path), { recursive: true });
      await writeFile(path, bytes);
    } catch {
      await mkdir(ARTIFACTS_DIR, { recursive: true });
      // If the path wasn't in artifacts dir, move it there
      const fallbackPath = join(ARTIFACTS_DIR, basename(path));
      await writeFile(fallbackPath, bytes);
    }
  }

  // ── Page order ────────────────────────────────────────────────────

  async reconcilePageOrder(currentPageIds: PageId[]): Promise<PageId[]> {
    const config = await this.getConfig();
    return reconcilePageOrder(config.tabOrder, currentPageIds);
  }

  async resolveActivePage(orderedPages: PageId[]): Promise<PageId | undefined> {
    const config = await this.getConfig();
    return resolveActivePage(orderedPages, config.activePageId);
  }

  async trackNewPage(pageId: PageId): Promise<void> {
    const config = await this.getConfig();
    config.activePageId = pageId;
    config.tabOrder = insertNewPage(config.tabOrder ?? [], pageId);
    await this.saveConfig(config);
  }

  async trackClosedPage(pageId: PageId): Promise<void> {
    const config = await this.getConfig();
    config.tabOrder = removePage(config.tabOrder ?? [], pageId);
    if (config.activePageId === pageId) {
      config.activePageId = config.tabOrder[config.tabOrder.length - 1];
    }
    await this.saveConfig(config);
  }

  async setActivePage(pageId: PageId): Promise<void> {
    const config = await this.getConfig();
    config.activePageId = pageId;
    config.activeUrl = undefined; // deprecate in favor of pageId
    await this.saveConfig(config);
  }

  async clearRefs(): Promise<void> {
    const config = await this.getConfig();
    config.refs = {};
    config.refsPageId = undefined;
    config.refsUrl = undefined;
    await this.saveConfig(config);
  }

  async saveRefs(refs: Record<string, string>, pageId: PageId, url: string): Promise<void> {
    const config = await this.getConfig();
    config.refs = refs;
    config.refsPageId = pageId;
    config.refsUrl = url;
    await this.saveConfig(config);
  }

  async setActiveUrl(url: string): Promise<void> {
    const config = await this.getConfig();
    config.activeUrl = url;
    await this.saveConfig(config);
  }

  // ── Console log reader ────────────────────────────────────────────

  async readConsoleLogs(
    session: BrowserSession,
    level?: string,
  ): Promise<{ logs: Array<{ level: string; text: string; time: string }>; note?: string }> {
    const raw = await session.evaluate<unknown>({
      kind: "expression",
      expression: "(globalThis.__futureConsoleLogs) || []",
    });

    const logs = Array.isArray(raw)
      ? raw.filter(isRecord).map(e => e as Record<string, unknown>)
      : [];

    const filtered = logs
      .filter(e => !level || e.level === level)
      .map(e => ({
        level: String(e.level ?? ""),
        text: String(e.text ?? ""),
        time: String(e.time ?? ""),
      }));

    return {
      logs: filtered,
      note: filtered.length === 0
        ? "No buffered console messages. The hook captures messages after a Future browser tool has touched the page."
        : undefined,
    };
  }

  // ── External result formatting ────────────────────────────────────

  toExternalPageInfo(page: InternalPageInfo): { title: string; url: string } {
    return { title: page.title, url: page.url };
  }

  toExternalActionResult(result: InternalActionResult): {
    clicked?: string;
    title: string;
    url: string;
  } {
    return { title: result.title, url: result.url };
  }

  toExternalScreenshotResult(
    path: string,
    filename: string,
    title: string,
    url: string,
  ): { path: string; filename: string; title: string; url: string } {
    return { path, filename, title, url };
  }
}
