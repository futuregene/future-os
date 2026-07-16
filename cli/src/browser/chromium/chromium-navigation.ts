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
  // Subscribe BEFORE Page.navigate so we don't miss fast-firing
  // DOMContentLoaded events.  If the page loads between the navigate
  // response and the subscription, the event is lost and the caller
  // hangs for the full deadline (15 s).
  let sawDomContentLoaded = false;
  const unsub = session.on("Page.lifecycleEvent", (event: unknown) => {
    const e = event as { loaderId: string; name: string };
    if (e.name === "DOMContentLoaded") sawDomContentLoaded = true;
  });

  try {
    const response = (await session.send("Page.navigate", {
      url,
      frameId: undefined, // main frame
    })) as { frameId?: string; loaderId?: string; errorText?: string };

    if (response.errorText) {
      return { didNavigate: false, errorText: response.errorText };
    }

    if (!response.loaderId) {
      // Same-document navigation
      return { didNavigate: true, sameDocument: true };
    }

    // Race: the event may have already fired, or it may fire soon.
    // Use a short poll instead of the full deadline fallback inside
    // waitForLifecycleEvent.
    const pollEnd = Date.now() + Math.min(deadline.remainingMs(), 5_000);
    while (!sawDomContentLoaded && Date.now() < pollEnd) {
      await sleep(50);
    }

    return { didNavigate: true };
  } finally {
    unsub();
  }
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
        // Also capture DOMContentLoaded for the new loader so the wait()
        // poll below doesn't miss it if it fires before we check.
        if (e.name === "DOMContentLoaded" && e.loaderId === this.newLoaderId) {
          this._sawDomContentLoaded = true;
        }
      }
    });
  }

  private _sawDomContentLoaded = false;

  /**
   * Wait for navigation triggered by a user action (click, press, type).
   *
   * Most actions do NOT cause navigation — a click on a button that runs JS,
   * a Tab press, or typing into a field.  For those we must return quickly so
   * the tool doesn't block for the full 15 s navigation timeout.
   *
   * Strategy: poll for a new loaderId for at most 500 ms.  If one appears the
   * page IS navigating — wait for DOMContentLoaded on the new loader.  If
   * nothing appears within 500 ms the action did not navigate; return
   * immediately so the caller can proceed.
   */
  async wait(
    _session: CdpSession,
    deadline: { remainingMs(): number; expired: boolean },
  ): Promise<NavigationResult> {
    if (this.disposed) return { didNavigate: false };

    // Phase 1 — wait for navigation to *start* (max 500 ms)
    const navStartMs = Math.min(deadline.remainingMs(), 500);
    const navStartAt = Date.now();
    while (Date.now() - navStartAt < navStartMs) {
      if (this.newLoaderId) {
        // Phase 2 — navigation started; wait for it to finish.
        // Don't call waitForLifecycleEvent (same race as explicit nav).
        // Poll the flag set by arm()'s lifecycle listener instead.
        const pollEnd = Date.now() + Math.min(deadline.remainingMs(), 5_000);
        while (!this._sawDomContentLoaded && Date.now() < pollEnd) {
          await sleep(50);
        }
        return { didNavigate: true };
      }
      await sleep(50);
    }

    // No navigation started — action was a non-navigating interaction
    return { didNavigate: false };
  }

  dispose(): void {
    this.disposed = true;
    this.unsub?.();
  }
}

// ── Internal ────────────────────────────────────────────────────────

function sleep(ms: number): Promise<void> {
  return new Promise(resolve => setTimeout(resolve, ms));
}
