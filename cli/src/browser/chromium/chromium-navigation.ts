/**
 * Navigation waiter for Chromium CDP.
 *
 * Explicit navigation (open): Frame.navigate → wait for loaderId.
 * Action-triggered (click/press): capture current loaderId, wait for change.
 */
import type { CdpSession } from "./cdp-connection.js";

export interface NavigationResult {
  didNavigate: boolean;
  newUrl?: string;
  errorText?: string;
  sameDocument?: boolean;
}

// ── Explicit navigation ─────────────────────────────────────────────

export async function waitForExplicitNavigation(
  session: CdpSession,
  url: string,
  deadline: { remainingMs(): number; expired: boolean },
): Promise<NavigationResult> {
  // Navigate (lifecycle events already enabled by activePageSession)
  const response = await session.send("Page.navigate", {
    url,
    frameId: undefined, // main frame
  }) as { frameId?: string; loaderId?: string; errorText?: string };

  if (response.errorText) {
    return { didNavigate: false, errorText: response.errorText };
  }

  if (!response.loaderId) {
    // Same-document navigation
    return { didNavigate: true, sameDocument: true };
  }

  // Wait for DOMContentLoaded for this loader
  await waitForLifecycleEvent(session, "DOMContentLoaded", response.loaderId, deadline);

  return { didNavigate: true };
}

// ── Action-triggered navigation ─────────────────────────────────────

export class ActionNavigationObserver {
  private mainFrameId: string;
  private currentLoaderId: string;
  private newLoaderId: string | null = null;
  private disposed = false;
  private unsub?: () => void;

  constructor(mainFrameId: string, currentLoaderId: string) {
    this.mainFrameId = mainFrameId;
    this.currentLoaderId = currentLoaderId;
  }

  arm(session: CdpSession): void {
    if (this.disposed) return;

    this.unsub = session.on("Page.lifecycleEvent", (event: unknown) => {
      const e = event as {
        frameId: string;
        loaderId: string;
        name: string;
      };
      // Only track main frame navigations — ignore iframes
      if (e.frameId !== this.mainFrameId) return;
      if (e.loaderId && e.loaderId !== this.currentLoaderId) {
        this.newLoaderId = e.loaderId;
      }
    });
  }

  async wait(
    session: CdpSession,
    deadline: { remainingMs(): number; expired: boolean },
  ): Promise<NavigationResult> {
    if (this.disposed) return { didNavigate: false };

    // Wait briefly for a new loader or same-document navigation
    while (!deadline.expired) {
      if (this.newLoaderId) {
        await waitForLifecycleEvent(session, "DOMContentLoaded", this.newLoaderId, deadline);
        return { didNavigate: true };
      }
      await sleep(50);
    }

    // Timeout — no navigation detected
    return { didNavigate: false };
  }

  dispose(): void {
    this.disposed = true;
    this.unsub?.();
  }
}

// ── Internal ────────────────────────────────────────────────────────

async function waitForLifecycleEvent(
  session: CdpSession,
  targetName: string,
  loaderId: string,
  deadline: { remainingMs(): number; expired: boolean },
): Promise<void> {
  return new Promise<void>((resolve) => {
    const unsub = session.on("Page.lifecycleEvent", (event: unknown) => {
      const e = event as { loaderId: string; name: string };
      if (e.loaderId === loaderId && e.name === targetName) {
        unsub();
        resolve();
      }
    });

    // Fallback: resolve on timeout (matches current .catch(() => undefined) behavior)
    setTimeout(() => {
      unsub();
      resolve();
    }, deadline.remainingMs());
  });
}

function sleep(ms: number): Promise<void> {
  return new Promise(resolve => setTimeout(resolve, ms));
}
