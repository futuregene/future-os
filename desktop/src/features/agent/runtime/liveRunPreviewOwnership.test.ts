import type { AgentMessage } from "@future-os/thread-projection";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { ensureLiveReply, resetRunProjection, upsertStreamingPreview } from "./liveRunProjection";

const listRunEventsSince = vi.fn();

vi.mock("../../../integrations/storage/threadStore", () => ({
  listRunEventsSince: (...args: unknown[]) => listRunEventsSince(...args),
}));

vi.mock("../../../lib/futureEvents", () => ({ emitFutureEvent: vi.fn() }));

/** Drive a setMessages mock to a final state. */
function collect(initial: AgentMessage[]) {
  let state = initial;
  const setMessages = vi.fn((action: unknown) => {
    state = typeof action === "function" ? (action as (previous: AgentMessage[]) => AgentMessage[])(state) : action as AgentMessage[];
  });
  return { setMessages, state: () => state };
}

function streamingBubble(runId: string, content = "half a repl"): AgentMessage {
  return {
    id: `stream_${runId}`,
    role: "assistant",
    authorKey: "author.researchCopilot",
    content,
    status: "streaming",
    createdAt: new Date(1_000).toISOString(),
    runStartedAt: 1_000,
    runId,
  };
}

const userMessage: AgentMessage = {
  id: "u1",
  role: "user",
  authorKey: "author.you",
  content: "go",
  status: "complete",
  createdAt: new Date(500).toISOString(),
};

beforeEach(() => {
  listRunEventsSince.mockReset();
  vi.spyOn(console, "warn").mockImplementation(() => {});
});

describe("ensureLiveReply", () => {
  // A run that has produced nothing yet must not adopt the reply another run is
  // already streaming. Doing so moved the running reply onto the new run id, and
  // the real run's next projection then found no bubble of its own: the answer
  // flickered, and a tool row whose result had already arrived read
  // "interrupted" until the turn ended — the "already interrupted, but it keeps
  // answering" report.
  it("leaves a reply another run is still streaming alone", () => {
    const current = [userMessage, streamingBubble("run_live")];

    const next = ensureLiveReply(current, "run_empty", 2_000);

    expect(next).toBe(current);
    expect(next.find(message => message.role === "assistant")?.runId).toBe("run_live");
    expect(next.find(message => message.role === "assistant")?.status).toBe("streaming");
  });

  it("still adopts a settled trailing reply that has no owner yet", () => {
    const settled: AgentMessage = {
      ...streamingBubble("run_live"),
      status: "complete",
      runId: undefined,
    };
    const current = [userMessage, settled];

    const next = ensureLiveReply(current, "run_empty", 2_000);
    const reply = next.find(message => message.role === "assistant");

    expect(reply?.status).toBe("streaming");
    expect(reply?.runId).toBe("run_empty");
  });
});

describe("a run the Agent does not know", () => {
  // The Agent answers `run ... is not known by this session` for a run it never
  // executed, and for one already retired. That answer never changes, so the
  // preview must stop asking instead of re-reading on every push — the retries
  // are what held a "sync paused" note on screen over a run that will never
  // exist (a queued prompt the turn boundary folded into the run ahead).
  it("stops reading and raises no sync note", async () => {
    const runId = "run-folded-away";
    resetRunProjection(runId);
    listRunEventsSince.mockImplementation(() =>
      Promise.reject(new Error("run `run-folded-away` is not known by this session")));
    const { setMessages, state } = collect([userMessage, streamingBubble(runId)]);

    await upsertStreamingPreview(runId, 1_000, setMessages);
    await upsertStreamingPreview(runId, 1_000, setMessages);
    await upsertStreamingPreview(runId, 1_000, setMessages);

    expect(listRunEventsSince).toHaveBeenCalledTimes(1);
    expect(state().find(message => message.role === "assistant")?.previewStalled).toBeUndefined();
  });

  it("keeps retrying a read that failed for a recoverable reason", async () => {
    const runId = "run-transient";
    resetRunProjection(runId);
    listRunEventsSince.mockImplementation(() =>
      Promise.reject(new Error("get_events_since: database is locked")));
    const { setMessages } = collect([userMessage, streamingBubble(runId)]);

    await upsertStreamingPreview(runId, 1_000, setMessages);
    await upsertStreamingPreview(runId, 1_000, setMessages);

    expect(listRunEventsSince).toHaveBeenCalledTimes(2);
  });
});
