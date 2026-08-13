import { detectFinished, effectiveRunStatus } from "../sessionStatus";

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

describe("detectFinished", () => {
  test("flags a run that transitions running → completed", () => {
    const { finished } = detectFinished({ "s-1": "running", "s-2": "completed" }, [
      { sessionId: "s-1", status: "completed" },
      { sessionId: "s-2", status: "completed" },
    ]);
    expect(finished).toEqual(["s-1"]);
  });

  test("does not flag a session that was never observed running", () => {
    const { finished } = detectFinished({}, [{ sessionId: "s-1", status: "failed" }]);
    expect(finished).toEqual([]);
  });

  test("excludes the currently-selected session", () => {
    // L1: a run completing in the conversation the user is viewing is not unread.
    const { finished } = detectFinished(
      { "s-1": "running", "s-2": "running" },
      [
        { sessionId: "s-1", status: "completed" },
        { sessionId: "s-2", status: "completed" },
      ],
      "s-1",
    );
    expect(finished).toEqual(["s-2"]);
  });
});
