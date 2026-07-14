/**
 * Execution context tracker — per-CdpSession management of
 * Runtime.executionContextCreated / Destroyed / Cleared events.
 *
 * One tracker per CDP session. Owns its event listener subscriptions.
 * dispose() removes all listeners.
 */
import type { CdpSession } from "./cdp-connection.js";

export interface ExecutionContextInfo {
  contextId: number;
  frameId: string;
  isDefault: boolean;
  name: string;
}

export class ExecutionContextTracker {
  private contexts = new Map<number, ExecutionContextInfo>();
  private unsubs: Array<() => void> = [];

  constructor(session: CdpSession) {
    this.unsubs.push(
      session.on("Runtime.executionContextCreated", (params: unknown) => {
        const event = params as {
          context: { id: number; auxData?: { frameId?: string; isDefault?: boolean }; name: string };
        };
        if (event.context) {
          this.contexts.set(event.context.id, {
            contextId: event.context.id,
            frameId: event.context.auxData?.frameId ?? "",
            isDefault: event.context.auxData?.isDefault ?? true,
            name: event.context.name,
          });
        }
      }),
    );

    this.unsubs.push(
      session.on("Runtime.executionContextDestroyed", (params: unknown) => {
        const event = params as { executionContextId: number };
        this.contexts.delete(event.executionContextId);
      }),
    );

    this.unsubs.push(
      session.on("Runtime.executionContextsCleared", () => {
        this.contexts.clear();
      }),
    );
  }

  /**
   * Get a default-world execution context.
   *
   * Prefer one matching the given frameId. Falls back to any default-world
   * context because Page.getFrameTree frameId may differ from
   * Runtime.executionContextCreated auxData.frameId — a known CDP quirk.
   */
  async getMainWorldContextId(
    frameId: string,
    deadline: { remainingMs(): number; expired: boolean },
  ): Promise<number> {
    while (!deadline.expired) {
      // Exact frameId match
      for (const ctx of this.contexts.values()) {
        if (ctx.frameId === frameId && ctx.isDefault) {
          return ctx.contextId;
        }
      }
      // Fallback: any default-world context
      for (const ctx of this.contexts.values()) {
        if (ctx.isDefault) {
          return ctx.contextId;
        }
      }
      await sleep(50);
    }
    throw new Error(`No execution context found within timeout`);
  }

  /** Remove all event listeners. */
  dispose(): void {
    for (const unsub of this.unsubs) {
      unsub();
    }
    this.unsubs = [];
    this.contexts.clear();
  }
}

function sleep(ms: number): Promise<void> {
  return new Promise(resolve => setTimeout(resolve, ms));
}
