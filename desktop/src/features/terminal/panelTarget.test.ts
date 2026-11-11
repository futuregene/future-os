import { describe, expect, it } from "vitest";
import { remoteScopeKey, remoteTerminalUnsupported, terminalScope } from "./panelTarget";

describe("terminalScope", () => {
  it("offers the terminal for a workspace conversation", () => {
    // Regression: gating on section === "chat" hid the entry point for every
    // workspace conversation, because selecting one sets section "workspace".
    expect(terminalScope({ section: "workspace", centerMode: "thread", threadId: "t1" }))
      .toEqual({ kind: "local", threadId: "t1", key: "t1" });
  });

  it("offers the terminal for a chat conversation", () => {
    expect(terminalScope({ section: "chat", centerMode: "thread", threadId: "t2" }))
      .toEqual({ kind: "local", threadId: "t2", key: "t2" });
  });

  it("offers nothing outside a conversation view", () => {
    expect(terminalScope({ section: "skill", centerMode: "thread", threadId: "t1" })).toBeNull();
    expect(terminalScope({ section: "remote", centerMode: "thread", threadId: "t1" })).toBeNull();
    expect(terminalScope({ section: "settings", centerMode: "thread", threadId: "t1" })).toBeNull();
    expect(terminalScope({ section: "workspace", centerMode: "new-chat", threadId: "t1" })).toBeNull();
  });

  it("offers nothing without a conversation", () => {
    expect(terminalScope({ section: "workspace", centerMode: "thread", threadId: null })).toBeNull();
    expect(terminalScope({ section: "chat", centerMode: "thread", threadId: undefined })).toBeNull();
  });

  it("offers a remote conversation's own shell, scoped to that host and session", () => {
    // A remote conversation is a "chat" section with no local thread: the shells
    // belong to the host, and the key must not collide with a local thread id.
    expect(
      terminalScope({
        section: "chat",
        centerMode: "thread",
        threadId: null,
        remote: { desktopId: "desk-b", sessionId: "sess-1", terminalSupported: true },
      }),
    ).toEqual({ kind: "remote", desktopId: "desk-b", sessionId: "sess-1", key: remoteScopeKey("desk-b", "sess-1") });
  });

  it("keeps the two machines' tab state apart", () => {
    const key = remoteScopeKey("desk-b", "sess-1");
    expect(key.startsWith("remote:")).toBe(true);
    // A thread id is what the local map is keyed by; a remote scope must be
    // structurally unable to be one.
    expect(key).not.toBe("sess-1");
  });

  it("does not offer a shell on a host that never advertised one", () => {
    expect(
      terminalScope({
        section: "chat",
        centerMode: "thread",
        threadId: null,
        remote: { desktopId: "desk-old", sessionId: "sess-1", terminalSupported: false },
      }),
    ).toBeNull();
  });

  it("does not offer a shell for a draft on a host (nothing exists there yet)", () => {
    expect(
      terminalScope({
        section: "chat",
        centerMode: "thread",
        threadId: null,
        remote: { desktopId: "desk-b", sessionId: "", terminalSupported: true },
      }),
    ).toBeNull();
  });
});

describe("remoteTerminalUnsupported", () => {
  // The refusal `terminalScope` makes has two very different shapes, and only
  // one of them is worth explaining in the header: this machine cannot serve a
  // shell, as opposed to "there is nothing to attach to yet".
  const input = (remote: { desktopId: string; sessionId: string; terminalSupported: boolean } | null) => ({
    section: "chat",
    centerMode: "thread" as const,
    threadId: null,
    remote,
  });

  it("is true for a conversation on a host that serves no terminals", () => {
    expect(remoteTerminalUnsupported(input({ desktopId: "desk-old", sessionId: "sess-1", terminalSupported: false }))).toBe(true);
  });

  it("is false when the host does serve them", () => {
    expect(remoteTerminalUnsupported(input({ desktopId: "desk-b", sessionId: "sess-1", terminalSupported: true }))).toBe(false);
  });

  it("is false for a draft, where no shell exists to be served", () => {
    expect(remoteTerminalUnsupported(input({ desktopId: "desk-old", sessionId: "", terminalSupported: false }))).toBe(false);
  });

  it("is false off a remote conversation", () => {
    expect(remoteTerminalUnsupported({ ...input(null), section: "skill" })).toBe(false);
    expect(remoteTerminalUnsupported({ ...input(null), centerMode: "new-chat" })).toBe(false);
    expect(remoteTerminalUnsupported(input(null))).toBe(false);
  });
});
