// @vitest-environment jsdom
import type { RemoteReferenceMap } from "./remoteSessionReference";
import { describe, expect, it } from "vitest";
import {
  completeRemoteReference,
  expandRemoteReferences,
  filterRemoteReferenceOptions,
  pruneRemoteReferences,
  remoteReferenceOptions,
  remoteReferenceToken,
  sessionReferenceQuery,
} from "./remoteSessionReference";

/**
 * The `#` reference a remote composer writes: a compact token in the draft, a
 * link in the message. These are the four pure steps — what the caret triggers,
 * what the menu lists, what a pick leaves, what the send expands — and they are
 * the phone's rules, because the two clients must write the same message.
 */

const sessions = [
  {
    sessionId: "sess_newest",
    title: "Fix the flaky test",
    mode: "chat" as const,
    lastMessageAt: 300,
  },
  {
    sessionId: "sess_workspace",
    title: "Release notes",
    mode: "workspace" as const,
    workspaceId: "ws_1",
    workspaceName: "future-os",
    lastMessageAt: 200,
  },
  {
    // A chat is a chat by its mode: the host files every chat under a workspace
    // named after itself, and that name must not label the row.
    sessionId: "sess_chat_with_workspace",
    title: "Brainstorm",
    mode: "chat" as const,
    workspaceId: "ws_chat",
    workspaceName: "Brainstorm Workspace",
    lastMessageAt: 100,
  },
  {
    // A host that predates `lastMessageAt` sends none: the start time stands in
    // and the row sorts below the dated ones, never as though it were just used.
    sessionId: "sess_undated",
    title: "Older work",
    mode: "chat" as const,
    createdAt: 50,
  },
];

describe("remoteReferenceOptions", () => {
  it("lists the host's conversations newest activity first", () => {
    expect(remoteReferenceOptions(sessions).map(option => option.sessionId)).toEqual([
      "sess_newest",
      "sess_workspace",
      "sess_chat_with_workspace",
      "sess_undated",
    ]);
  });

  it("drops the conversation being composed in and rows with no session id", () => {
    const rows = [...sessions, { sessionId: "", title: "Never prompted", mode: "chat" as const }];
    expect(remoteReferenceOptions(rows, "sess_newest").map(option => option.sessionId)).toEqual([
      "sess_workspace",
      "sess_chat_with_workspace",
      "sess_undated",
    ]);
  });

  it("names the workspace only for a workspace conversation", () => {
    const byId = new Map(remoteReferenceOptions(sessions).map(option => [option.sessionId, option]));
    expect(byId.get("sess_workspace")?.workspace).toBe("future-os");
    expect(byId.get("sess_chat_with_workspace")?.workspace).toBeNull();
    expect(byId.get("sess_newest")?.workspace).toBeNull();
  });

  it("filters on the title or the workspace, keeping the recency order", () => {
    const options = remoteReferenceOptions(sessions);
    expect(filterRemoteReferenceOptions(options, "flaky").map(option => option.sessionId))
      .toEqual(["sess_newest"]);
    // A workspace match brings its conversations along: the reader may remember
    // the folder and not the title.
    expect(filterRemoteReferenceOptions(options, "future-os").map(option => option.sessionId))
      .toEqual(["sess_workspace"]);
    expect(filterRemoteReferenceOptions(options, "")).toBe(options);
  });

  it("matches case-insensitively", () => {
    const options = remoteReferenceOptions(sessions);
    expect(filterRemoteReferenceOptions(options, "REL")).toHaveLength(1);
  });
});

describe("sessionReferenceQuery", () => {
  it("reads a `#` word at the caret", () => {
    expect(sessionReferenceQuery("#", 1)).toEqual({ start: 0, end: 1, query: "" });
    expect(sessionReferenceQuery("#flaky", 6)).toEqual({ start: 0, end: 6, query: "flaky" });
    expect(sessionReferenceQuery("see #fix", 8)).toEqual({ start: 4, end: 8, query: "fix" });
  });

  it("ignores a `#` that does not start a word", () => {
    expect(sessionReferenceQuery("issue#12", 8)).toBeNull();
    // A markdown heading stays text.
    expect(sessionReferenceQuery("## Plan", 7)).toBeNull();
    expect(sessionReferenceQuery("see docs/#anchor", 16)).toBeNull();
    expect(sessionReferenceQuery("no hash here", 12)).toBeNull();
  });

  it("reads the word at the caret, not the one at the end of the draft", () => {
    // The caret sits right after the `#`; the rest of the line is the word being
    // replaced, so a pick does not eat it.
    expect(sessionReferenceQuery("#fi and then some text", 3)).toEqual({ start: 0, end: 3, query: "fi" });
    // Nothing being typed at this caret.
    expect(sessionReferenceQuery("see #fix and more", 13)).toBeNull();
  });

  it("opens nothing while a range is selected", () => {
    expect(sessionReferenceQuery("#fix", null)).toBeNull();
  });
});

describe("completeRemoteReference", () => {
  const target = { sessionId: "sess_newest", title: "Fix the flaky test" };

  it("replaces the typed word with the compact token and names the caret", () => {
    const query = sessionReferenceQuery("see #fix", 8)!;
    const next = completeRemoteReference("see #fix", query, target);
    expect(next.text).toBe("see #Fix the flaky test ");
    expect(next.caret).toBe(next.text.length);
    expect(next.token).toBe("#Fix the flaky test");
  });

  it("keeps a space that already follows the word", () => {
    const query = sessionReferenceQuery("see #fix rest", 8)!;
    expect(completeRemoteReference("see #fix rest", query, target).text)
      .toBe("see #Fix the flaky test rest");
  });

  it("makes a repeated title distinct so the two expand to their own ids", () => {
    const taken: RemoteReferenceMap = { "#Fix the flaky test": target };
    const other = { sessionId: "sess_other_x9f2c", title: "Fix the flaky test" };
    expect(remoteReferenceToken(other, taken)).toBe("#Fix the flaky test·x9f2c");
    // Re-picking the same conversation keeps its own token.
    expect(remoteReferenceToken(target, taken)).toBe("#Fix the flaky test");
  });

  it("falls back to the id's tail for a blank title", () => {
    expect(remoteReferenceToken({ sessionId: "sess_blank_7f3a1", title: "  " })).toBe("#7f3a1");
  });
});

describe("expandRemoteReferences", () => {
  const refs: RemoteReferenceMap = {
    "#Fix the flaky test": { sessionId: "sess_newest", title: "Fix the flaky test" },
    "#Fix the flaky test·x9f2c": { sessionId: "sess_other_x9f2c", title: "Fix the flaky test" },
  };

  it("writes the link the agent resolves, longest token first", () => {
    expect(expandRemoteReferences(`#Fix the flaky test·x9f2c and #Fix the flaky test`, refs))
      .toBe("[Fix the flaky test](futureos://session/sess_other_x9f2c) and [Fix the flaky test](futureos://session/sess_newest)");
  });

  it("expands every occurrence and leaves the rest alone", () => {
    expect(expandRemoteReferences("#Fix the flaky test twice: #Fix the flaky test", refs))
      .toBe("[Fix the flaky test](futureos://session/sess_newest) twice: [Fix the flaky test](futureos://session/sess_newest)");
    expect(expandRemoteReferences("no tokens here", refs)).toBe("no tokens here");
    expect(expandRemoteReferences("stale #Gone", refs)).toBe("stale #Gone");
  });
});

describe("pruneRemoteReferences", () => {
  const refs: RemoteReferenceMap = {
    "#A": { sessionId: "one", title: "A" },
    "#B": { sessionId: "two", title: "B" },
  };

  it("drops the references whose token the user edited away", () => {
    expect(pruneRemoteReferences(refs, "only #A here")).toEqual({ "#A": refs["#A"] });
    expect(pruneRemoteReferences(refs, "")).toEqual({});
  });

  it("returns the same object when nothing changed, so it cannot loop", () => {
    expect(pruneRemoteReferences(refs, "#A and #B")).toBe(refs);
  });
});
