import {
  applyStreamEvent,
  emptyTimeline,
  timelineFromEntries,
  timelineFromHistory,
} from "../eventReducer";

describe("history reducer", () => {
  test("skips tool-call-only messages whose content is omitted on the wire", () => {
    const timeline = timelineFromHistory([
      { role: "user", content: "hi" },
      // Assistant tool-call messages serialize without a content field.
      { role: "assistant" },
      { role: "tool", content: null },
      { role: "assistant", content: "done" },
    ]);
    expect(timeline.items).toEqual([
      expect.objectContaining({ kind: "message", role: "user", text: "hi" }),
      expect.objectContaining({ kind: "message", role: "assistant", text: "done" }),
    ]);
  });
});

describe("entry reducer", () => {
  test("projects user/assistant entries and carries attachments", () => {
    const timeline = timelineFromEntries([
      {
        id: "e1",
        role: "user",
        content: "check this",
        meta: {
          attachments: [
            { path: "/tmp/a.png", name: "a.png", kind: "image" },
            { path: "/tmp/b.pdf", name: "b.pdf", kind: "file" },
          ],
        },
      },
      { id: "e2", role: "assistant", content: "looks good" },
      { id: "e3", role: "tool", content: "tool output" },
    ]);
    expect(timeline.items).toEqual([
      expect.objectContaining({
        kind: "message",
        role: "user",
        text: "check this",
        attachments: [
          { path: "/tmp/a.png", name: "a.png", kind: "image" },
          { path: "/tmp/b.pdf", name: "b.pdf", kind: "file" },
        ],
      }),
      expect.objectContaining({ kind: "message", role: "assistant", text: "looks good" }),
    ]);
  });

  test("keeps attachment-only user entries and drops malformed attachments", () => {
    const timeline = timelineFromEntries([
      {
        id: "e1",
        role: "user",
        content: "",
        meta: {
          attachments: [{ path: "/tmp/a.png", name: "a.png" }, { name: "no-path" } as never],
        },
      },
      { id: "e2", role: "user", content: "" },
    ]);
    expect(timeline.items).toHaveLength(1);
    expect(timeline.items[0]).toMatchObject({
      attachments: [{ path: "/tmp/a.png", name: "a.png" }],
    });
  });
});

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
    expect(approval.items.find(item => item.kind === "approval")).toMatchObject({
      kind: "approval",
    });
    expect(ended.streaming).toBe(false);
  });

  test("marks the assistant streaming while running and settles it with a duration on end", () => {
    const started = applyStreamEvent(emptyTimeline(), {
      type: "agent_start",
      data: "{}",
      runId: "run-1",
      idx: 0,
    });
    const placeholder = started.items.find(
      item => item.kind === "message" && item.role === "assistant",
    );
    if (!placeholder || placeholder.kind !== "message")
      throw new Error("streaming assistant placeholder was not created");
    expect(placeholder.streaming).toBe(true);
    expect(typeof placeholder.startedAt).toBe("number");

    const text = applyStreamEvent(started, {
      type: "text_chunk",
      data: JSON.stringify({ text: "done" }),
      runId: "run-1",
      idx: 1,
    });
    const streaming = text.items.find(item => item.kind === "message" && item.role === "assistant");
    expect(streaming && streaming.kind === "message" && streaming.streaming).toBe(true);

    const ended = applyStreamEvent(text, {
      type: "agent_end",
      data: "{}",
      runId: "run-1",
      idx: 2,
    });
    const settled = ended.items.find(item => item.kind === "message" && item.role === "assistant");
    if (!settled || settled.kind !== "message") throw new Error("assistant message missing");
    expect(settled.streaming).toBe(false);
    expect(settled.durationMs).toEqual(expect.any(Number));
  });
});
