import { describe, expect, test } from "bun:test";
import type { CdpSession } from "../../chromium/cdp-connection.js";
import {
  ActionNavigationObserver,
  waitForExplicitNavigation,
} from "../../chromium/chromium-navigation.js";

type Handler = (event: unknown) => void;

class FakeSession {
  private handlers = new Set<Handler>();

  constructor(
    private onNavigate?: (session: FakeSession) => void,
  ) {}

  async send(method: string): Promise<unknown> {
    if (method !== "Page.navigate") return {};
    this.onNavigate?.(this);
    return { frameId: "main", loaderId: "loader-new" };
  }

  on(method: string, handler: Handler): () => void {
    if (method === "Page.lifecycleEvent") this.handlers.add(handler);
    return () => this.handlers.delete(handler);
  }

  emit(event: unknown): void {
    for (const handler of this.handlers) handler(event);
  }

  asCdpSession(): CdpSession {
    return this as unknown as CdpSession;
  }
}

function deadline(ms = 500): { remainingMs(): number; expired: boolean } {
  const end = Date.now() + ms;
  return {
    remainingMs: () => Math.max(end - Date.now(), 0),
    get expired() { return Date.now() >= end; },
  };
}

describe("Chromium navigation waiting", () => {
  test("explicit navigation catches DOMContentLoaded fired before navigate returns", async () => {
    const session = new FakeSession((current) => {
      current.emit({
        frameId: "main",
        loaderId: "loader-new",
        name: "DOMContentLoaded",
      });
    });

    const result = await waitForExplicitNavigation(
      session.asCdpSession(),
      "https://example.test/",
      deadline(),
    );

    expect(result.didNavigate).toBe(true);
  });

  test("action observer remembers navigation events fired before wait starts", async () => {
    const session = new FakeSession();
    const observer = new ActionNavigationObserver("main", "loader-old");
    observer.arm(session.asCdpSession());

    session.emit({ frameId: "main", loaderId: "loader-new", name: "init" });
    session.emit({
      frameId: "main",
      loaderId: "loader-new",
      name: "DOMContentLoaded",
    });

    const result = await observer.wait(session.asCdpSession(), deadline());
    observer.dispose();

    expect(result.didNavigate).toBe(true);
  });

  test("action observer ignores iframe lifecycle events", async () => {
    const session = new FakeSession();
    const observer = new ActionNavigationObserver("main", "loader-old");
    observer.arm(session.asCdpSession());
    session.emit({ frameId: "iframe", loaderId: "loader-new", name: "DOMContentLoaded" });

    const result = await observer.wait(session.asCdpSession(), deadline(75));
    observer.dispose();

    expect(result.didNavigate).toBe(false);
  });
});
