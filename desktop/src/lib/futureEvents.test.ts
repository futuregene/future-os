// @vitest-environment jsdom
import { describe, expect, it, vi } from "vitest";
import { emitFutureEvent, onFutureEvent } from "./futureEvents";

describe("futureEvents bus", () => {
  it("delivers typed payloads to subscribers", () => {
    const handler = vi.fn();
    const off = onFutureEvent("toast", handler);
    emitFutureEvent("toast", { message: "hi", tone: "info" });
    expect(handler).toHaveBeenCalledWith({ message: "hi", tone: "info" });
    off();
  });

  it("stops delivering after unsubscribe", () => {
    const handler = vi.fn();
    const off = onFutureEvent("agent_end", handler);
    off();
    emitFutureEvent("agent_end", undefined);
    expect(handler).not.toHaveBeenCalled();
  });

  it("supports void payloads", () => {
    const handler = vi.fn();
    const off = onFutureEvent("file-tree-refresh", handler);
    emitFutureEvent("file-tree-refresh", undefined);
    // CustomEvent normalizes an undefined detail to null.
    expect(handler).toHaveBeenCalledWith(null);
    off();
  });
});
