import { effectiveRunStatus } from "../sessionStatus";

describe("effectiveRunStatus", () => {
  test("local running/queued status wins", () => {
    expect(effectiveRunStatus("running", false)).toBe("running");
    expect(effectiveRunStatus("queued", false)).toBe("queued");
    expect(effectiveRunStatus("running", true)).toBe("running");
  });

  test("agent streaming with no local run row reads as running", () => {
    // A prompt started by the TUI/CLI/another machine has no local run status.
    expect(effectiveRunStatus(undefined, true)).toBe("running");
    expect(effectiveRunStatus("", true)).toBe("running");
  });

  test("settled statuses pass through", () => {
    expect(effectiveRunStatus("completed", false)).toBe("completed");
    expect(effectiveRunStatus("failed", false)).toBe("failed");
    expect(effectiveRunStatus("waiting_approval", false)).toBe("waiting_approval");
    expect(effectiveRunStatus("cancelled", false)).toBe("cancelled");
  });

  test("idle session has no status", () => {
    expect(effectiveRunStatus(undefined, false)).toBeUndefined();
  });
});
