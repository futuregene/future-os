import { describe, expect, it } from "vitest";
import { isTransientQueueRefusal } from "./queueDelivery";

describe("queue refusal classification", () => {
  it("reads the host's 'still running' answer as 'not yet'", () => {
    const busy = "session_busy: this session is still running; wait for it to finish or abort it first.";
    // A Tauri rejection can be an Error or a bare string.
    expect(isTransientQueueRefusal(new Error(busy))).toBe(true);
    expect(isTransientQueueRefusal(busy)).toBe(true);
    expect(isTransientQueueRefusal(new Error("send_busy"))).toBe(true);
  });

  it("leaves everything else as a real refusal", () => {
    expect(isTransientQueueRefusal(new Error("prompt_too_large"))).toBe(false);
    expect(isTransientQueueRefusal(new Error("peer_not_connected"))).toBe(false);
    expect(isTransientQueueRefusal(undefined)).toBe(false);
  });
});
