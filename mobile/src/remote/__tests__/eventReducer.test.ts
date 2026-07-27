import { applyStreamEvent, emptyTimeline } from "../eventReducer";

describe("stream event reducer", () => {
  test("deduplicates and appends text chunks by run", () => {
    const first = applyStreamEvent(emptyTimeline(), {
      type: "text_chunk",
      data: JSON.stringify({ text: "hello" }),
      runId: "run-1",
      idx: 1,
    });
    const duplicate = applyStreamEvent(first, {
      type: "text_chunk",
      data: JSON.stringify({ text: "hello" }),
      runId: "run-1",
      idx: 1,
    });
    const second = applyStreamEvent(duplicate, {
      type: "text_chunk",
      data: JSON.stringify({ text: " world" }),
      runId: "run-1",
      idx: 2,
    });
    expect(second.items).toHaveLength(1);
    expect(second.items[0]).toMatchObject({ kind: "message", text: "hello world" });
  });

  test("tracks streaming and approval state", () => {
    const started = applyStreamEvent(emptyTimeline(), {
      type: "agent_start",
      data: "{}",
      runId: "run-1",
      idx: 0,
    });
    const approval = applyStreamEvent(started, {
      type: "approval_request",
      data: JSON.stringify({ approval_request_id: "approval-1", title: "Write file" }),
      runId: "run-1",
      idx: 1,
    });
    const ended = applyStreamEvent(approval, {
      type: "agent_end",
      data: "{}",
      runId: "run-1",
      idx: 2,
    });
    expect(started.streaming).toBe(true);
    expect(approval.items[0]).toMatchObject({ kind: "approval" });
    expect(ended.streaming).toBe(false);
  });
});
