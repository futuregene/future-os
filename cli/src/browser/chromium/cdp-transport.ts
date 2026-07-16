/**
 * Abstract CDP transport interface.
 *
 * Enables FakeTransport for unit testing CdpConnection without
 * a real WebSocket.
 */
export interface CdpTransport {
  /** Send a JSON-encoded CDP message. */
  send(message: string): void;

  /** Close the transport. Implementations may bound the handshake wait. */
  close(): Promise<void>;

  /** Register a message handler. Returns unsubscribe function. */
  onMessage(handler: (message: string) => void): () => void;

  /** Register a close handler. Returns unsubscribe function. */
  onClose(handler: (reason?: unknown) => void): () => void;
}

/**
 * Production WebSocket transport wrapping Bun's built-in WebSocket.
 */
export class WebSocketTransport implements CdpTransport {
  private ws: WebSocket;
  private messageHandlers = new Set<(message: string) => void>();
  private closeHandlers = new Set<(reason?: unknown) => void>();
  private closed = false;
  private closePromise: Promise<void>;
  private resolveClose!: () => void;

  constructor(ws: WebSocket) {
    this.ws = ws;

    this.closePromise = new Promise<void>((resolve) => {
      this.resolveClose = resolve;
    });

    ws.addEventListener("message", (event: MessageEvent) => {
      const data = typeof event.data === "string" ? event.data : "";
      if (!data) return;
      for (const handler of this.messageHandlers) {
        handler(data);
      }
    });

    ws.addEventListener("close", (event: CloseEvent) => {
      this.closed = true;
      const reason = event.reason || `code=${event.code}`;
      for (const handler of this.closeHandlers) {
        handler(reason);
      }
      this.resolveClose();
    });

    ws.addEventListener("error", (event: Event) => {
      const reason = `WebSocket error: ${(event as ErrorEvent).message || "unknown"}`;
      if (!this.closed) {
        this.closed = true;
        for (const handler of this.closeHandlers) {
          handler(reason);
        }
        this.resolveClose();
      }
    });
  }

  send(message: string): void {
    if (this.closed) return;
    this.ws.send(message);
  }

  async close(): Promise<void> {
    if (this.closed) return this.closePromise;
    this.closed = true;
    this.ws.close(1000, "client disconnect");

    // Chrome does not always complete the CDP WebSocket close handshake on
    // Windows. Give a normal close a brief chance, then let the short-lived
    // browser CLI finish instead of hanging indefinitely.
    let timer: ReturnType<typeof setTimeout> | undefined;
    const timeout = new Promise<void>((resolve) => {
      timer = setTimeout(resolve, 500);
    });
    await Promise.race([this.closePromise, timeout]);
    if (timer) clearTimeout(timer);
    // Make subsequent close() calls observe the same bounded completion even
    // when Chrome never sends a close frame.
    this.resolveClose();
  }

  onMessage(handler: (message: string) => void): () => void {
    this.messageHandlers.add(handler);
    return () => { this.messageHandlers.delete(handler); };
  }

  onClose(handler: (reason?: unknown) => void): () => void {
    this.closeHandlers.add(handler);
    return () => { this.closeHandlers.delete(handler); };
  }
}
