/**
 * Abstract CDP transport interface.
 *
 * Enables FakeTransport for unit testing CdpConnection without
 * a real WebSocket.
 */
export interface CdpTransport {
  /** Send a JSON-encoded CDP message. */
  send(message: string): void;

  /** Close the transport. Returns when fully closed. */
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
    // Don't wait for Chrome CDP's close frame — it may never arrive on
    // Windows (or take long enough to cause a hang).  The OS reclaims
    // the TCP connection when the process exits; the close event handler
    // is still registered for cleanup if the frame does arrive.
    this.resolveClose();
    return this.closePromise;
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
