import type { AgentMessage } from "@future-os/thread-projection";
import type { StoredRunEvent } from "../../../integrations/storage/threadStore";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { PREVIEW_STALL_AFTER_MS, resetRunProjection, upsertStreamingPreview } from "./liveRunProjection";

const listRunEventsSince = vi.fn();

vi.mock("../../../integrations/storage/threadStore", () => ({
  listRunEventsSince: (...args: unknown[]) => listRunEventsSince(...args),
}));

vi.mock("../../../lib/futureEvents", () => ({ emitFutureEvent: vi.fn() }));

/** The run's start, so a test can move the clock instead of waiting. */
const T0 = Date.parse("2026-11-03T12:00:00.000Z");

function events(runId: string, list: Array<[string, Record<string, unknown>]>, offset = 0): StoredRunEvent[] {
  return list.map(([eventType, payload], index) => ({
    id: `${runId}_e${offset + index}`,
    runId,
    eventType,
    payload: JSON.stringify(payload),
    sequence: offset + index,
    createdAt: offset + index,
  }));
}

/** Drive a setMessages mock to a final state. */
function collect(initial: AgentMessage[]) {
  let state = initial;
  const setMessages = vi.fn((action: unknown) => {
    state = typeof action === "function" ? (action as (previous: AgentMessage[]) => AgentMessage[])(state) : action as AgentMessage[];
  });
  return { setMessages, state: () => state };
}

function reply(state: AgentMessage[]): AgentMessage | undefined {
  return state.find(message => message.role === "assistant");
}

function streamingBubble(runId: string): AgentMessage {
  return {
    id: `stream_${runId}`,
    role: "assistant",
    authorKey: "author.researchCopilot",
    content: "The",
    status: "streaming",
    createdAt: new Date(T0).toISOString(),
    runStartedAt: T0,
    runId,
  };
}

/** A read that fails the way an unreadable journal fails: nothing to show. */
const readFailure = () => Promise.reject(new Error("get_events_since: event batch commit failed"));

beforeEach(() => {
  listRunEventsSince.mockReset();
  // Only the clock is faked: the projection itself never awaits a timer, and a
  // faked `setTimeout` would change how the failure paths schedule work.
  vi.useFakeTimers({ toFake: ["Date"] });
  vi.setSystemTime(T0);
  vi.spyOn(console, "warn").mockImplementation(() => {});
});

afterEach(() => {
  vi.useRealTimers();
  vi.restoreAllMocks();
});

describe("live preview sync stall", () => {
  // The ledger is module state: every case owns its run id and starts clean, or
  // an earlier case's streak would decide when this one speaks up.
  it("stays silent when a read fails once and the next one succeeds", async () => {
    const runId = "run-sync-blip";
    resetRunProjection(runId);
    listRunEventsSince.mockImplementationOnce(readFailure);
    listRunEventsSince.mockImplementationOnce(async () => events(runId, [["text_chunk", { text: "still here" }]]));
    const { setMessages, state } = collect([streamingBubble(runId)]);

    await upsertStreamingPreview(runId, T0, setMessages);
    vi.setSystemTime(T0 + 1_500);
    await upsertStreamingPreview(runId, T0, setMessages);

    expect(reply(state())?.content).toBe("still here");
    expect(reply(state())?.previewStalled).toBeUndefined();
  });

  it("stays silent while the streak is younger than the threshold", async () => {
    const runId = "run-sync-young";
    resetRunProjection(runId);
    listRunEventsSince.mockImplementation(readFailure);
    const { setMessages, state } = collect([streamingBubble(runId)]);

    await upsertStreamingPreview(runId, T0, setMessages);
    vi.setSystemTime(T0 + PREVIEW_STALL_AFTER_MS - 1);
    await upsertStreamingPreview(runId, T0, setMessages);

    expect(reply(state())?.previewStalled).toBeUndefined();
  });

  it("says sync is paused once the streak passes the threshold, and clears on recovery", async () => {
    const runId = "run-sync-stall";
    resetRunProjection(runId);
    listRunEventsSince.mockImplementation(readFailure);
    const { setMessages, state } = collect([streamingBubble(runId)]);

    await upsertStreamingPreview(runId, T0, setMessages);
    vi.setSystemTime(T0 + PREVIEW_STALL_AFTER_MS);
    await upsertStreamingPreview(runId, T0, setMessages);

    const stall = reply(state())?.previewStalled;
    expect(stall?.stage).toBe("read");
    expect(stall?.attempts).toBe(2);
    expect(stall?.reason).toContain("get_events_since");
    // The note is presentation only: the reply it hangs on is untouched, and
    // still carries the text the reader was looking at.
    expect(reply(state())?.content).toBe("The");

    listRunEventsSince.mockImplementation(async () => events(runId, [["text_chunk", { text: "resumed" }]]));
    vi.setSystemTime(T0 + PREVIEW_STALL_AFTER_MS + 500);
    await upsertStreamingPreview(runId, T0, setMessages);

    expect(reply(state())?.content).toBe("resumed");
    expect(reply(state())?.previewStalled).toBeUndefined();
  });

  it("speaks up when the preview gave up rebuilding this run's log, naming that stage", async () => {
    const runId = "run-sync-gave-up";
    resetRunProjection(runId);
    // A durable body whose prefix this projector never saw: the projection and
    // its one rebuild both refuse it, so every later tick returns without even
    // reading — the shape a reply froze in for minutes with nothing to show for
    // it.
    listRunEventsSince.mockImplementation(async () => events(runId, [[
      "text_chunk",
      { text: "orphan", content_key: "body", content_offset: 64, content_bytes: 6 },
    ]]));
    const { setMessages, state } = collect([]);

    await upsertStreamingPreview(runId, T0, setMessages);
    expect(reply(state())?.previewStalled).toBeUndefined();

    vi.setSystemTime(T0 + PREVIEW_STALL_AFTER_MS);
    await upsertStreamingPreview(runId, T0, setMessages);

    const stall = reply(state())?.previewStalled;
    expect(stall?.stage).toBe("projection");
    expect(stall?.reason.length).toBeGreaterThan(0);
    // The note adds no reads: an abandoned log stays abandoned (one initial
    // read plus its one rebuild, then nothing).
    expect(listRunEventsSince).toHaveBeenCalledTimes(2);
  });

  it("drops an earlier note when a reset re-opens the run's preview", async () => {
    const runId = "run-sync-reset";
    resetRunProjection(runId);
    listRunEventsSince.mockImplementation(readFailure);
    const { setMessages, state } = collect([streamingBubble(runId)]);

    await upsertStreamingPreview(runId, T0, setMessages);
    vi.setSystemTime(T0 + PREVIEW_STALL_AFTER_MS);
    await upsertStreamingPreview(runId, T0, setMessages);
    expect(reply(state())?.previewStalled?.stage).toBe("read");

    // A backend snapshot reset invalidates the streak with the projector: the
    // note must not outlive the state that justified it, or a healed run would
    // keep claiming it is behind.
    resetRunProjection(runId);
    listRunEventsSince.mockImplementation(async () => events(runId, [["text_chunk", { text: "fresh" }]]));
    await upsertStreamingPreview(runId, T0, setMessages);

    expect(reply(state())?.content).toBe("fresh");
    expect(reply(state())?.previewStalled).toBeUndefined();
  });
});
