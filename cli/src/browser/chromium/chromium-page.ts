/**
 * Chromium page management — target discovery, tab CRUD, active page tracking.
 *
 * Uses Target.setDiscoverTargets + Target.targetCreated (NOT Target.setAutoAttach).
 * Only attaches to type="page" targets.
 */
import type { CdpConnection, CdpSession } from "./cdp-connection.js";
import { reconcilePageOrder, insertNewPage, removePage } from "../tab-order.js";

export interface ChromiumPage {
  targetId: string;
  sessionId: string;
  type: string;
  url: string;
  title: string;
}

export class ChromiumPageManager {
  private pages = new Map<string, ChromiumPage>();
  private browserSession: CdpSession;
  private connection: CdpConnection;
  private activePageId: string | undefined;
  private tabOrder: string[] = [];

  constructor(browserSession: CdpSession, connection: CdpConnection) {
    this.browserSession = browserSession;
    this.connection = connection;
  }

  async initialize(existingTabOrder?: string[], activePageId?: string): Promise<void> {
    await this.browserSession.send("Target.setDiscoverTargets", {
      discover: true,
      filter: [{ type: "page" }],
    });

    this.browserSession.on("Target.targetCreated", (event: unknown) => {
      const info = (event as { targetInfo: { targetId: string; type: string; url: string; title: string } }).targetInfo;
      if (!info || info.type !== "page") return;
      this.trackTarget(info.targetId, info.type, info.url, info.title);
    });

    this.browserSession.on("Target.targetInfoChanged", (event: unknown) => {
      const info = (event as { targetInfo: { targetId: string; url: string; title: string } }).targetInfo;
      if (!info) return;
      const existing = this.pages.get(info.targetId);
      if (existing) { existing.url = info.url; existing.title = info.title; }
    });

    // Unified cleanup on Target.targetDestroyed
    this.browserSession.on("Target.targetDestroyed", (event: unknown) => {
      const { targetId } = event as { targetId: string };
      const attached = this.connection.detachTargetByTargetId(targetId);
      if (attached) {
        this.connection.rejectPendingForSession(attached.sessionId, {
          code: -1, message: `Target ${targetId} destroyed`,
        });
      }
      this.pages.delete(targetId);
      this.tabOrder = removePage(this.tabOrder, targetId);
      if (this.activePageId === targetId) {
        this.activePageId = this.tabOrder[this.tabOrder.length - 1];
      }
    });

    // Get existing targets
    const targets = await this.browserSession.send("Target.getTargets", {
      filter: [{ type: "page" }],
    }) as { targetInfos: Array<{ targetId: string; type: string; url: string; title: string }> };

    for (const info of targets.targetInfos) {
      if (info.type === "page") {
        await this.attachToTarget(info.targetId, info.type, info.url, info.title);
      }
    }

    const pageIds = Array.from(this.pages.keys());
    this.tabOrder = reconcilePageOrder(existingTabOrder, pageIds);

    // Restore active page from config, or default to last
    if (activePageId && this.pages.has(activePageId)) {
      this.activePageId = activePageId;
    }
  }

  private async attachToTarget(
    targetId: string,
    type: string,
    url: string,
    title: string,
  ): Promise<void> {
    const result = await this.browserSession.send("Target.attachToTarget", {
      targetId,
      flatten: true,
    }) as { sessionId: string };

    this.pages.set(targetId, {
      targetId,
      sessionId: result.sessionId,
      type,
      url,
      title,
    });
  }

  // ── Tab management ────────────────────────────────────────────────

  async createPage(url: string = "about:blank"): Promise<{ targetId: string; page: ChromiumPage }> {
    const result = await this.browserSession.send("Target.createTarget", { url }) as { targetId: string };
    const targetId = result.targetId;

    // Wait for target discovery
    const deadline = Date.now() + 5000;
    let page: ChromiumPage | undefined;
    while (Date.now() < deadline) {
      page = this.pages.get(targetId);
      if (page) break;
      await sleep(50);
    }
    if (!page) throw new Error(`Target ${targetId} not discovered within timeout`);

    // Attach if not yet attached
    if (!page.sessionId) {
      await this.attachToTarget(targetId, page.type, page.url, page.title);
      page = this.pages.get(targetId)!;
    }

    this.tabOrder = insertNewPage(this.tabOrder, targetId);
    return { targetId, page };
  }

  async closePage(targetId: string): Promise<void> {
    await this.browserSession.send("Target.closeTarget", { targetId });
    // Cleanup is handled by Target.targetDestroyed event handler above
  }

  async activatePage(targetId: string): Promise<void> {
    await this.browserSession.send("Target.activateTarget", { targetId });
    this.activePageId = targetId;
  }

  // ── Queries ───────────────────────────────────────────────────────

  getPages(): ChromiumPage[] {
    return this.tabOrder
      .map(id => this.pages.get(id))
      .filter((p): p is ChromiumPage => p !== undefined);
  }

  getPage(targetId: string): ChromiumPage | undefined {
    return this.pages.get(targetId);
  }

  getActivePage(): ChromiumPage | undefined {
    if (this.activePageId) {
      const page = this.pages.get(this.activePageId);
      if (page) return page;
    }
    const ordered = this.getPages();
    return ordered[ordered.length - 1];
  }

  getActivePageId(): string | undefined {
    return this.getActivePage()?.targetId;
  }

  getTabOrder(): string[] {
    return [...this.tabOrder];
  }

  setActivePageId(pageId: string): void {
    if (this.pages.has(pageId)) this.activePageId = pageId;
  }

  // ── Internal ──────────────────────────────────────────────────────

  private trackTarget(targetId: string, type: string, url: string, title: string): void {
    if (!this.pages.has(targetId)) {
      this.pages.set(targetId, { targetId, sessionId: "", type, url, title });
    }
  }
}

function sleep(ms: number): Promise<void> {
  return new Promise(resolve => setTimeout(resolve, ms));
}
