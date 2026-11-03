import type { ContentPage } from "@future-os/thread-projection";
import { eventPayload } from "@future-os/thread-projection";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { fetchRunReplay } from "./remoteRunReplay";

/**
 * Reading a run's log, and finishing the bodies a lean host only referenced.
 *
 * The page loop itself is the shared package's; what this client adds is the
 * transport and one piece of finishing work — a feed that trims its bodies
 * (`lean_events_v1`) leaves a `content_refs` map where the projector expects
 * text, and a client that handed that straight on would rebuild the run with
 * empty messages. The hydration has to happen *here*, before the caller applies
 * the replay, because the caller has no content lane of its own.
 */

const request = vi.fn<(desktopId: string, command: Record<string, unknown>, lane: string) => Promise<unknown>>();
vi.mock("./remotePeerClient", async importOriginal => ({
  ...await importOriginal<typeof import("./remotePeerClient")>(),
  requestRemotePeer: (desktopId: string, command: Record<string, unknown>, lane: string) =>
    request(desktopId, command, lane),
}));

/** A reading of the body, in the canonical page shape the host serves. */
function contentPage(text: string, encoding: "json" | "text" = "text"): ContentPage {
  const bytes = new TextEncoder().encode(text).byteLength;
  return {
    databaseId: "db",
    encoding,
    etag: "immutable",
    hasMore: false,
    nextOffset: bytes,
    restoreGeneration: "generation",
    text,
    totalBytes: bytes,
  };
}

/** One run event as `get_events_since` serves it. */
function wireEvent(type: string, data: unknown, idx: number) {
  return { data: JSON.stringify(data), idx, runId: "run_1", type };
}

function replayPage(events: Array<Record<string, unknown>>) {
  return {
    events,
    watermark: events.length === 0 ? -1 : Number(events[events.length - 1]!.idx),
  };
}

beforeEach(() => {
  request.mockReset();
});

describe("fetchRunReplay", () => {
  it("hydrates a body the feed only referenced, before handing the replay on", async () => {
    const reference = {
      cursor: 2,
      databaseId: "db",
      epoch: 1,
      field: "/data/text",
      restoreGeneration: "generation",
    };
    request.mockImplementation(async (_desktopId, command) => {
      if (command.type === "get_event_content_range") {
        // The reference is passed through untouched: it is the host's own
        // coordinate, and the client adds nothing but the run it belongs to.
        expect(command.runId).toBe("run_1");
        expect(command.contentRequest).toEqual(expect.objectContaining({
          cursor: 2,
          databaseId: "db",
          field: "/data/text",
          restoreGeneration: "generation",
        }));
        return contentPage("the answer, in full");
      }
      return replayPage([
        wireEvent("user_message", { text: "hello" }, 0),
        wireEvent("user_message", { content_refs: { text: reference }, text: "" }, 1),
      ]);
    });

    const replay = await fetchRunReplay({
      desktopId: "desktop_a",
      runId: "run_1",
      sessionId: "sess_1",
      sinceIdx: -1,
    });

    // The referenced event now carries the decoded body to the projector...
    expect(eventPayload(replay.events[1]!, replay.events[1]!.data)).toEqual({ text: "the answer, in full" });
    // ...and the sibling that carried its body inline was not even asked about.
    expect(eventPayload(replay.events[0]!, replay.events[0]!.data)).toEqual({ text: "hello" });
    const contentReads = request.mock.calls
      .filter(([, command]) => command.type === "get_event_content_range");
    expect(contentReads).toHaveLength(1);
    // The wire string is kept as it arrived: the decoded body lives in the
    // process, not in a re-serialized payload a journal could quote back.
    expect(replay.events[1]!.data).toContain("\"content_refs\"");
  });

  it("asks for nothing when a page has no oversized or referenced body", async () => {
    request.mockResolvedValue(replayPage([
      wireEvent("agent_start", { started_at_ms: 1_700_000_000_000 }, 0),
      wireEvent("text_chunk", { text: "short" }, 1),
    ]));

    const replay = await fetchRunReplay({
      desktopId: "desktop_a",
      runId: "run_1",
      sessionId: "sess_1",
      sinceIdx: 0,
    });

    expect(request.mock.calls.filter(([, command]) => command.type !== "get_events_since")).toEqual([]);
    expect(replay.events.map(event => event.type)).toEqual(["agent_start", "text_chunk"]);
  });

  /**
   * A body too large for the feed is referenced rather than trimmed away, and it
   * is *not* always one of the kinds that needs materializing: a tool result
   * stays lazy, so the event is handed on with its reference intact instead of
   * being pulled through the content lane on every page.
   */
  it("leaves a lazy body referenced", async () => {
    const reference = {
      cursor: 3,
      databaseId: "db",
      epoch: 1,
      field: "/data/output",
      restoreGeneration: "generation",
    };
    request.mockResolvedValue(replayPage([
      wireEvent("tool_end", { content_refs: { output: reference }, name: "ls" }, 0),
    ]));

    const replay = await fetchRunReplay({
      desktopId: "desktop_a",
      runId: "run_1",
      sessionId: "sess_1",
      sinceIdx: 0,
    });

    expect(request.mock.calls.filter(([, command]) => command.type === "get_event_content_range")).toEqual([]);
    expect(eventPayload(replay.events[0]!, replay.events[0]!.data)).toMatchObject({
      content_refs: { output: expect.objectContaining({ field: "/data/output" }) },
      name: "ls",
    });
  });
});
