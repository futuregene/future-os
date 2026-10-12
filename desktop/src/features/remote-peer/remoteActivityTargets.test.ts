// @vitest-environment jsdom
import type { AgentMessage } from "@future-os/thread-projection";
import { describe, expect, it } from "vitest";
import { pendingTargets, withRemoteTargets } from "./remoteActivityTargets";

function activity(id: string, extra: Record<string, unknown> = {}): AgentMessage {
  return {
    id: `m_${id}`,
    role: "assistant",
    authorKey: "author.researchCopilot",
    content: "",
    createdAt: "2026-01-01T00:00:00.000Z",
    status: "complete",
    segments: [{ id: `seg_${id}`, kind: "activity", item: { id, kind: "shell", status: "completed", ...extra } }],
  } as unknown as AgentMessage;
}

/**
 * Which calls need their arguments fetched.
 *
 * The host's history page is byte-budgeted: a shell call arrives with its
 * identity and no target, so the row would say "Ran a command" and nothing else.
 */
describe("pendingTargets", () => {
  it("asks for a call whose target is missing, and not for one that has it", () => {
    const missing = activity("t1", { runId: "r1", toolCallId: "c1" });
    const present = activity("t2", { runId: "r1", toolCallId: "c2", target: "ls -la" });
    expect(pendingTargets([missing, present])).toEqual([{ runId: "r1", toolCallId: "c1" }]);
  });

  it("skips a call it cannot address", () => {
    // No call id, no run: the host has nothing to look the arguments up by, so a
    // request would be refused.
    expect(pendingTargets([activity("t3", { runId: "r1" })])).toEqual([]);
    expect(pendingTargets([activity("t4", { toolCallId: "c4" })])).toEqual([]);
  });

  it("reaches the calls a collapsed summary row stands for", () => {
    const collapsed: AgentMessage = {
      ...activity("t5"),
      segments: [{
        id: "seg_5",
        kind: "activity",
        item: {
          children: [
            { id: "c1", kind: "shell", status: "completed", runId: "r1", toolCallId: "call_1" },
            { id: "c2", kind: "read", status: "completed", runId: "r1", toolCallId: "call_2" },
          ],
          id: "collapsed_1",
          kind: "shell",
          status: "completed",
        },
      }],
    };
    expect(pendingTargets([collapsed])).toEqual([
      { runId: "r1", toolCallId: "call_1" },
      { runId: "r1", toolCallId: "call_2" },
    ]);
  });

  it("reaches a message that carries activity without segments", () => {
    const legacy = {
      ...activity("t6"),
      activityItems: [{ id: "c9", kind: "read", status: "completed", runId: "r1", toolCallId: "call_9" }],
      segments: undefined,
    } as unknown as AgentMessage;
    expect(pendingTargets([legacy])).toEqual([{ runId: "r1", toolCallId: "call_9" }]);
  });
});

/** The fetched target lands where the transcript reads it. */
describe("withRemoteTargets", () => {
  it("fills a call's target, and leaves the messages alone when there is none", () => {
    const message = activity("t1", { runId: "r1", toolCallId: "c1" });
    // With nothing fetched, the very same array comes back: a transcript that
    // re-created its messages on every render would defeat downstream memoisation.
    const messages = [message];
    expect(withRemoteTargets(messages, new Map())).toBe(messages);

    const filled = withRemoteTargets([message], new Map([["c1", "Rscript x.R"]]));
    const item = filled[0]!.segments![0]!;
    expect(item.kind === "activity" && item.item.target).toBe("Rscript x.R");
    // The input is untouched: a re-render must not see the map it produced last
    // time as the transcript.
    expect((message.segments![0] as { item: { target?: string } }).item.target).toBeUndefined();
  });

  it("reports a failed run's reason, which the projector cannot attribute", () => {
    // The shared projector reads a run's outcome from *this* machine's run rows,
    // so a remote failure arrives marked failed with nothing to show.
    const failed = { ...activity("t7", { runId: "r1" }), runError: "the provider dropped", status: "failed" } as AgentMessage;
    const [next] = withRemoteTargets([failed], new Map());
    expect(next!.terminationNotice).toBe("the provider dropped");
  });

  it("keeps a reason the projector did supply", () => {
    const supplied = { ...activity("t8"), terminationNotice: "from the host", runError: "ignored" } as AgentMessage;
    expect(withRemoteTargets([supplied], new Map())[0]!.terminationNotice).toBe("from the host");
  });
});

/**
 * Two rows that stand for the same call ask once.
 *
 * A collapsed summary row carries the calls it stands for, and one of those can
 * also appear as its own segment — the same call, described twice. Asking twice
 * would be two requests for one answer.
 */
it("asks once for a call that appears twice", () => {
  const item = { id: "c1", kind: "shell", status: "completed", runId: "r1", toolCallId: "c1" };
  const twice: AgentMessage = {
    ...activity("t9"),
    segments: [
      { id: "seg_a", kind: "activity", item },
      { id: "seg_b", kind: "activity", item },
    ],
  } as unknown as AgentMessage;
  expect(pendingTargets([twice])).toEqual([{ runId: "r1", toolCallId: "c1" }]);
});

/** A segment that is not an activity is left exactly as it is. */
it("leaves a non-activity segment untouched", () => {
  const message = {
    ...activity("t10"),
    segments: [{ id: "seg_text", kind: "text", text: "hello" }],
  } as unknown as AgentMessage;
  const messages = [message];
  // Nothing fetched and nothing to fill: the very same array comes back.
  expect(withRemoteTargets(messages, new Map([["c1", "ls"]]))).toBe(messages);
});

/** A message with no segments at all is left as it is. */
it("leaves a message with no segments untouched", () => {
  const message = { ...activity("t11"), segments: undefined } as unknown as AgentMessage;
  const found = new Map([["c1", "ls"]]);
  expect(withRemoteTargets([message], found)[0]).toBe(message);
});

/** A collapsed row's children are filled, so an expanded group shows its targets. */
it("fills the calls a collapsed row stands for", () => {
  const collapsed = {
    ...activity("t12"),
    segments: [{
      id: "seg_collapsed",
      kind: "activity",
      item: {
        children: [{ id: "c1", kind: "shell", status: "completed", runId: "r1", toolCallId: "c1" }],
        id: "collapsed",
        kind: "shell",
        status: "completed",
      },
    }],
  } as unknown as AgentMessage;

  const [filled] = withRemoteTargets([collapsed], new Map([["c1", "Rscript x.R"]]));
  const segment = filled!.segments![0]!;
  expect(segment.kind === "activity" && segment.item.children![0]!.target).toBe("Rscript x.R");
});
