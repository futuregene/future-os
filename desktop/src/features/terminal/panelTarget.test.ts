import { describe, expect, it } from "vitest";
import { terminalTarget } from "./panelTarget";

describe("terminalTarget", () => {
  it("offers the terminal for a workspace conversation", () => {
    // Regression: gating on section === "chat" hid the entry point for every
    // workspace conversation, because selecting one sets section "workspace".
    expect(terminalTarget({ section: "workspace", centerMode: "thread", threadId: "t1" })).toBe("t1");
  });

  it("offers the terminal for a chat conversation", () => {
    expect(terminalTarget({ section: "chat", centerMode: "thread", threadId: "t2" })).toBe("t2");
  });

  it("offers nothing outside a conversation view", () => {
    expect(terminalTarget({ section: "skill", centerMode: "thread", threadId: "t1" })).toBeNull();
    expect(terminalTarget({ section: "remote", centerMode: "thread", threadId: "t1" })).toBeNull();
    expect(terminalTarget({ section: "settings", centerMode: "thread", threadId: "t1" })).toBeNull();
    expect(terminalTarget({ section: "workspace", centerMode: "new-chat", threadId: "t1" })).toBeNull();
  });

  it("offers nothing without a conversation", () => {
    expect(terminalTarget({ section: "workspace", centerMode: "thread", threadId: null })).toBeNull();
    expect(terminalTarget({ section: "chat", centerMode: "thread", threadId: undefined })).toBeNull();
  });
});
