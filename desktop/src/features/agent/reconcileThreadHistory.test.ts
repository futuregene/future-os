import type { AgentMessage } from "@future-os/thread-projection";
import { describe, expect, it } from "vitest";
import { reconcileThreadHistory } from "./reconcileThreadHistory";

function message(id: string, overrides: Partial<AgentMessage> = {}): AgentMessage {
  return { id, role: "assistant", authorKey: "author.researchCopilot", content: id, createdAt: "2026-01-01T00:00:00Z", ...overrides };
}

describe("history reconciliation", () => {
  it("matches optimistic and canonical exchanges by run while preserving DOM ids", () => {
    const user = message("pending_user", { role: "user", runId: "r1" });
    const assistant = message("local_assistant", { runId: "r1" });
    const current = [user, assistant];
    const result = reconcileThreadHistory(current, [
      { ...user, id: "stored_user" },
      { ...assistant, id: "stored_assistant", content: "persisted" },
    ], current, false, true);
    expect(result.messages.map(item => item.id)).toEqual(["pending_user", "local_assistant"]);
    expect(result.messages[1]?.content).toBe("persisted");
  });

  it("does not overwrite live writes even during a settling refresh", () => {
    const old = message("pending", { runId: "r1", status: "streaming" });
    const live = { ...old, content: "newer streamed text" };
    const next = message("pending_user_next", { role: "user", runId: "r2" });
    const result = reconcileThreadHistory([live, next], [{ ...old, id: "canonical", status: "complete" }], [old], false, true);
    expect(result.messages).toEqual([live, next]);
  });

  it("keeps pages prepended while a tail request was running", () => {
    const oldest = message("old");
    const tail = message("tail");
    const result = reconcileThreadHistory([oldest, tail], [{ ...tail, content: "fresh" }], [tail], true, false);
    expect(result.keptOlder).toBe(true);
    expect(result.messages.map(item => item.content)).toEqual(["old", "fresh"]);
  });

  it("keeps the array and row references for unchanged history", () => {
    const current = [message("same", { attachments: [] })];
    expect(reconcileThreadHistory(current, [{ ...current[0]! }], current, true, false).messages).toBe(current);
  });
});
