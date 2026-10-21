import { describe, expect, it } from "vitest";
import { terminalTarget } from "./panelTarget";

describe("terminalTarget", () => {
  it("offers the terminal on the conversation page", () => {
    // The page is one page now: a workspace conversation and a chat are the same
    // answer, because the rail's tab (which list is drawn) has nothing to do with
    // whether a conversation is on screen. The old rule named the two sections
    // separately, and the version before that matched `chat` alone — which hid
    // the entry point for every workspace conversation.
    expect(terminalTarget({ section: "conversations", centerMode: "thread", threadId: "t1" })).toBe("t1");
  });

  it("offers nothing outside a conversation view", () => {
    expect(terminalTarget({ section: "skill", centerMode: "thread", threadId: "t1" })).toBeNull();
    expect(terminalTarget({ section: "remote", centerMode: "thread", threadId: "t1" })).toBeNull();
    expect(terminalTarget({ section: "settings", centerMode: "thread", threadId: "t1" })).toBeNull();
    expect(terminalTarget({ section: "conversations", centerMode: "new-chat", threadId: "t1" })).toBeNull();
  });

  it("offers nothing without a conversation", () => {
    expect(terminalTarget({ section: "conversations", centerMode: "thread", threadId: null })).toBeNull();
    expect(terminalTarget({ section: "conversations", centerMode: "thread", threadId: undefined })).toBeNull();
  });
});
