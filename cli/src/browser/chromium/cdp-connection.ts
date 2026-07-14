/**
 * CDP connection over WebSocket transport.
 *
 * Responsibilities:
 * - Incrementing request IDs and promise matching
 * - Per-request timeout
 * - Session management (CdpSession handles per-target command dispatch)
 * - Pending request rejection on close
 * - TargetSessionRegistry for targetId ↔ sessionId mapping
 */
import type { CdpTransport } from "./cdp-transport.js";
import { WebSocketTransport } from "./cdp-transport.js";
import { CdpEventRouter, type CdpEventHandler } from "./cdp-event-router.js";
import { TargetSessionRegistry, type AttachedTarget } from "./chromium-target-registry.js";

// ── Types ───────────────────────────────────────────────────────────

export interface CdpError {
  code: number;
  message: string;
  data?: unknown;
}

interface PendingRequest {
  sessionId?: string;
  resolve: (result: unknown) => void;
  reject: (error: CdpError) => void;
  timer: ReturnType<typeof setTimeout>;
}

// ── CdpConnection ───────────────────────────────────────────────────

export class CdpConnection {
  private requestId = 0;
  private pending = new Map<number, PendingRequest>();
  private sessions = new Map<string, CdpSession>();
  private transport: CdpTransport;
  private eventRouter: CdpEventRouter;
  private targetRegistry: TargetSessionRegistry;
  private defaultTimeoutMs: number;
  private _isConnected = true;
  private unsubMessage?: () => void;
  private unsubClose?: () => void;

  private constructor(
    transport: CdpTransport,
    options: { timeoutMs?: number } = {},
  ) {
    this.transport = transport;
    this.eventRouter = new CdpEventRouter();
    this.targetRegistry = new TargetSessionRegistry();
    this.defaultTimeoutMs = options.timeoutMs ?? 10_000;

    this.unsubMessage = transport.onMessage((raw: string) => {
      this.handleMessage(raw);
    });

    this.unsubClose = transport.onClose((_reason?: unknown) => {
      this.handleClose();
    });
  }

  /**
   * Connect to a CDP WebSocket endpoint.
   */
  static async connect(
    webSocketDebuggerUrl: string,
    options?: { timeoutMs?: number },
  ): Promise<CdpConnection> {
    const ws = new WebSocket(webSocketDebuggerUrl);

    await new Promise<void>((resolve, reject) => {
      const timer = setTimeout(() => {
        reject(new Error(`WebSocket connection timeout: ${webSocketDebuggerUrl}`));
      }, options?.timeoutMs ?? 10_000);

      ws.addEventListener("open", () => {
        clearTimeout(timer);
        resolve();
      });

      ws.addEventListener("error", (event: Event) => {
        clearTimeout(timer);
        reject(new Error(`WebSocket connection failed: ${(event as ErrorEvent).message || "unknown"}`));
      });
    });

    const transport = new WebSocketTransport(ws);
    return new CdpConnection(transport, options);
  }

  // ── Send ──────────────────────────────────────────────────────────

  /**
   * Send a CDP command and wait for the response.
   */
  send(
    method: string,
    params?: Record<string, unknown>,
    sessionId?: string,
  ): Promise<unknown> {
    return this.sendWithTimeout(method, params, sessionId, this.defaultTimeoutMs);
  }

  /**
   * Send a CDP command with an explicit timeout.
   */
  sendWithTimeout(
    method: string,
    params: Record<string, unknown> | undefined,
    sessionId: string | undefined,
    timeoutMs: number,
  ): Promise<unknown> {
    if (!this._isConnected) {
      return Promise.reject(new CdpConnectionError("Connection is closed"));
    }

    const id = ++this.requestId;

    const message: Record<string, unknown> = { id, method };
    if (params) message.params = params;
    if (sessionId) message.sessionId = sessionId;

    return new Promise<unknown>((resolve, reject) => {
      const timer = setTimeout(() => {
        this.pending.delete(id);
        reject(new CdpTimeoutError(method, timeoutMs));
      }, timeoutMs);

      this.pending.set(id, { sessionId, resolve, reject, timer });
      this.transport.send(JSON.stringify(message));
    });
  }

  // ── Events ────────────────────────────────────────────────────────

  on(
    sessionId: string | undefined,
    method: string,
    handler: CdpEventHandler,
  ): () => void {
    return this.eventRouter.add(sessionId, method, handler);
  }

  // ── Sessions ──────────────────────────────────────────────────────

  createSession(sessionId: string): CdpSession {
    const session = new CdpSession(sessionId, this);
    this.sessions.set(sessionId, session);
    return session;
  }

  removeSession(sessionId: string): void {
    this.sessions.delete(sessionId);
  }

  // ── Target registry ───────────────────────────────────────────────

  registerTarget(target: AttachedTarget): void {
    this.targetRegistry.add(target);
  }

  detachTargetBySessionId(sessionId: string): AttachedTarget | undefined {
    return this.targetRegistry.detachBySessionId(sessionId);
  }

  detachTargetByTargetId(targetId: string): AttachedTarget | undefined {
    return this.targetRegistry.detachByTargetId(targetId);
  }

  getTargetBySessionId(sessionId: string): AttachedTarget | undefined {
    return this.targetRegistry.getBySessionId(sessionId);
  }

  // ── Pending cleanup ───────────────────────────────────────────────

  /**
   * Reject all pending requests for a specific session.
   * Called when a target is detached or destroyed.
   */
  rejectPendingForSession(sessionId: string, error: CdpError): void {
    for (const [id, pending] of this.pending) {
      if (pending.sessionId === sessionId) {
        clearTimeout(pending.timer);
        this.pending.delete(id);
        pending.reject(error);
      }
    }
  }

  // ── Disconnect ────────────────────────────────────────────────────

  get isConnected(): boolean {
    return this._isConnected;
  }

  async disconnect(): Promise<void> {
    if (!this._isConnected) return;
    this._isConnected = false;

    // Reject all pending requests
    const closeError: CdpError = { code: -1, message: "Connection closed" };
    for (const [_id, pending] of this.pending) {
      clearTimeout(pending.timer);
      pending.reject(closeError);
    }
    this.pending.clear();

    // Clean up event handlers
    this.eventRouter.clear();
    this.unsubMessage?.();
    this.unsubClose?.();

    // Close transport
    await this.transport.close();
  }

  // ── Internal ──────────────────────────────────────────────────────

  private handleMessage(raw: string): void {
    let message: Record<string, unknown>;
    try {
      message = JSON.parse(raw) as Record<string, unknown>;
    } catch {
      return; // Malformed — ignore
    }

    const id = typeof message.id === "number" ? message.id : undefined;
    const sessionId = typeof message.sessionId === "string" ? message.sessionId : undefined;

    // Response to a pending request
    if (id !== undefined && typeof message.method === "undefined") {
      const pending = this.pending.get(id);
      if (!pending) return; // Stale response

      clearTimeout(pending.timer);
      this.pending.delete(id);

      if (message.error) {
        const err = message.error as Record<string, unknown>;
        pending.reject({
          code: typeof err.code === "number" ? err.code : -1,
          message: typeof err.message === "string" ? err.message : "Unknown CDP error",
          data: err.data,
        });
      } else {
        pending.resolve(message.result);
      }
      return;
    }

    // Event
    if (typeof message.method === "string") {
      this.eventRouter.dispatch(
        sessionId,
        message.method as string,
        message.params,
      );
    }
  }

  private handleClose(): void {
    const error: CdpError = { code: -1, message: "Connection closed" };
    for (const [_id, pending] of this.pending) {
      clearTimeout(pending.timer);
      pending.reject(error);
    }
    this.pending.clear();
    this._isConnected = false;
  }
}

// ── CdpSession ──────────────────────────────────────────────────────

export class CdpSession {
  constructor(
    public readonly sessionId: string,
    private connection: CdpConnection,
  ) {}

  send(method: string, params?: Record<string, unknown>): Promise<unknown> {
    return this.connection.send(method, params, this.sessionId);
  }

  sendWithTimeout(
    method: string,
    params: Record<string, unknown> | undefined,
    timeoutMs: number,
  ): Promise<unknown> {
    return this.connection.sendWithTimeout(method, params, this.sessionId, timeoutMs);
  }

  on(method: string, handler: CdpEventHandler): () => void {
    // Normalize: browser-level session (empty string) → undefined
    return this.connection.on(this.sessionId || undefined, method, handler);
  }

  /** Listen for browser-level events (not scoped to this session). */
  onBrowser(method: string, handler: CdpEventHandler): () => void {
    return this.connection.on(undefined, method, handler);
  }
}

// ── Errors ──────────────────────────────────────────────────────────

export class CdpConnectionError extends Error {
  constructor(message: string) {
    super(message);
    this.name = "CdpConnectionError";
  }
}

export class CdpTimeoutError extends Error {
  constructor(
    method: string,
    timeoutMs: number,
  ) {
    super(`CDP command "${method}" timed out after ${timeoutMs}ms`);
    this.name = "CdpTimeoutError";
  }
}
