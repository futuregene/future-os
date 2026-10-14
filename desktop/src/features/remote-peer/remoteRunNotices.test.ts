// @vitest-environment jsdom
import type { AgentMessage } from "@future-os/thread-projection";
import { describe, expect, it } from "vitest";
import { withRemoteRunNotices } from "./remoteRunNotices";

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

describe("withRemoteRunNotices", () => {
  it("reports a failed run's reason, which the projector cannot attribute", () => {
    // The shared projector reads a run's outcome from *this* machine's run rows,
    // so a remote failure arrives marked failed with nothing to show — a reply
    // that offers Retry and says nothing about the cause.
    const failed = { ...activity("t7", { runId: "r1" }), runError: "the provider dropped", status: "failed" } as AgentMessage;
    const [next] = withRemoteRunNotices([failed]);
    expect(next!.terminationNotice).toBe("the provider dropped");
    // The input is untouched: a re-render must not see the row it produced last
    // time as the transcript.
    expect(failed.terminationNotice).toBeUndefined();
  });

  it("keeps a reason the projector did supply", () => {
    const supplied = { ...activity("t8"), terminationNotice: "from the host", runError: "ignored" } as AgentMessage;
    expect(withRemoteRunNotices([supplied])[0]!.terminationNotice).toBe("from the host");
  });

  it("leaves a reply that ended well exactly as it was", () => {
    // Identity, not equality: a transcript that re-created its messages on every
    // render would defeat downstream memoisation on every streaming push.
    const settled = activity("t1", { runId: "r1", toolCallId: "c1" });
    const messages = [settled];
    expect(withRemoteRunNotices(messages)).toBe(messages);
  });

  it("ignores an empty or blank reason", () => {
    // boundary: a host that reports a failure without one — the run is still
    // marked failed, so there is nothing to say and no notice to invent.
    for (const runError of ["", "   ", undefined]) {
      const failed = { ...activity("t2"), runError, status: "failed" } as AgentMessage;
      expect(withRemoteRunNotices([failed])[0]!.terminationNotice).toBeUndefined();
    }
  });
});
