import { describe, expect, it } from "vitest";
import { persistedUserMessageIndex } from "./forkPoint";

describe("persistedUserMessageIndex", () => {
  const entries = [
    { id: "u1", role: "user", runId: "run-1", blocks: [{ kind: "text", text: "same" }] },
    { id: "a1", role: "assistant", runId: "run-1" },
    { id: "u2", role: "user", runId: "run-2", blocks: [{ kind: "text", text: "same" }] },
  ];

  it("resolves a reconciled optimistic bubble by its stable run identity", () => {
    expect(persistedUserMessageIndex(entries, { id: "pending_user_local", runId: "run-2" })).toBe(1);
  });

  it("falls back to the canonical entry-derived UI id for legacy history", () => {
    expect(persistedUserMessageIndex(entries, { id: "m_u1" })).toBe(0);
  });

  it("does not treat a genuinely unpersisted bubble as a fork point", () => {
    expect(persistedUserMessageIndex(entries, { id: "pending_user_new", runId: "run-new" })).toBe(-1);
  });
});
