import { describe, expect, test } from "bun:test";
import { WebSocketTransport } from "../../chromium/cdp-transport.js";

type EventHandler = (event: Event) => void;

class WebSocketWithoutCloseFrame {
  private handlers = new Map<string, Set<EventHandler>>();
  closeCalls = 0;

  addEventListener(type: string, handler: EventHandler): void {
    const handlers = this.handlers.get(type) ?? new Set<EventHandler>();
    handlers.add(handler);
    this.handlers.set(type, handlers);
  }

  send(_message: string): void {}

  close(): void {
    this.closeCalls += 1;
    // Deliberately do not emit a close event, matching the Windows CDP hang.
  }

  asWebSocket(): WebSocket {
    return this as unknown as WebSocket;
  }
}

describe("WebSocketTransport", () => {
  test("close is bounded when Chrome never sends a close frame", async () => {
    const ws = new WebSocketWithoutCloseFrame();
    const transport = new WebSocketTransport(ws.asWebSocket());

    const startedAt = Date.now();
    await transport.close();
    const firstCloseMs = Date.now() - startedAt;

    const secondStartedAt = Date.now();
    await transport.close();
    const secondCloseMs = Date.now() - secondStartedAt;

    expect(ws.closeCalls).toBe(1);
    expect(firstCloseMs).toBeGreaterThanOrEqual(400);
    expect(firstCloseMs).toBeLessThan(1_000);
    expect(secondCloseMs).toBeLessThan(50);
  });
});
