import type { AgentMessage, MessageSegment } from "@future-os/thread-projection";
import { describe, expect, it } from "vitest";
import { isCompactionDivider, isSupersededCompactionDivider } from "./compactionMarkers";

function dividerSegment(checkpointId?: string): MessageSegment {
  return ({ id: "c1", kind: "compaction", status: "completed", ...(checkpointId ? { checkpointId } : {}) }) as MessageSegment;
}

function textSegment(id: string): MessageSegment {
  return ({ id, kind: "text", text: "…" }) as MessageSegment;
}

function divider(checkpointId?: string): AgentMessage {
  return {
    id: "m-divider",
    role: "assistant",
    content: "",
    segments: [dividerSegment(checkpointId)],
  } as AgentMessage;
}

const LIVE = new Set(["cp-1"]);

describe("isCompactionDivider", () => {
  it("accepts an empty assistant row that is one compaction segment", () => {
    expect(isCompactionDivider(divider("cp-1"))).toBe(true);
    // A divider without a checkpoint id (a manual compaction) is still a divider.
    expect(isCompactionDivider(divider())).toBe(true);
  });

  it("rejects every row that is an actual exchange", () => {
    expect(isCompactionDivider({ id: "u", role: "user", content: "hi" } as AgentMessage)).toBe(false);
    expect(isCompactionDivider({
      id: "a",
      role: "assistant",
      content: "the answer",
      segments: [dividerSegment("cp-1")],
    } as AgentMessage)).toBe(false);
    expect(isCompactionDivider({
      id: "a2",
      role: "assistant",
      content: "",
      segments: [dividerSegment("cp-1"), textSegment("t1")],
    } as AgentMessage)).toBe(false);
    expect(isCompactionDivider({
      id: "a3",
      role: "assistant",
      content: "",
      segments: [],
    } as unknown as AgentMessage)).toBe(false);
  });
});

describe("isSupersededCompactionDivider", () => {
  it("drops the durable divider row the live bubble already draws", () => {
    expect(isSupersededCompactionDivider(divider("cp-1"), LIVE)).toBe(true);
  });

  it("keeps a divider the live bubble does not know about", () => {
    expect(isSupersededCompactionDivider(divider("cp-other"), LIVE)).toBe(false);
    expect(isSupersededCompactionDivider(divider(), LIVE)).toBe(false);
  });

  it("never treats a row with content as a superseded divider", () => {
    // A reply that carries prose AND a live checkpoint is an exchange, not the
    // durable copy of the divider: dropping it would delete the answer from the
    // transcript. The divider test has to run first for this to hold.
    expect(isSupersededCompactionDivider({
      id: "a",
      role: "assistant",
      content: "the answer",
      segments: [dividerSegment("cp-1")],
    } as AgentMessage, LIVE)).toBe(false);
  });

  it("never treats a user row as a superseded divider", () => {
    expect(isSupersededCompactionDivider({
      id: "u",
      role: "user",
      content: "",
      segments: [dividerSegment("cp-1")],
    } as AgentMessage, LIVE)).toBe(false);
  });
});
