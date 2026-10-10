import { beforeEach, describe, expect, it, vi } from "vitest";

/**
 * The client's own contract, asserted without a host: which *command* each
 * helper builds, and which id it addresses it with.
 *
 * This is the layer where a mistake is invisible at runtime — a pin sent with a
 * session id where the host wants a thread id is accepted by the transport,
 * answered by some other record, and reported as success. So the ids are the
 * assertion.
 */

const invokeMock = vi.fn();
vi.mock("../../integrations/tauri/invoke", () => ({
  invokeCommand: (...args: unknown[]) => invokeMock(...args),
}));

const {
  abortRemoteRun,
  deleteRemoteConversation,
  fetchRemoteSessions,
  forkRemoteConversation,
  pinRemoteConversation,
  promptRemoteConversation,
  renameRemoteConversation,
} = await import("./remotePeerClient");

beforeEach(() => {
  invokeMock.mockReset();
  invokeMock.mockResolvedValue(undefined);
});

/** The `(command, args)` the helper just handed to the backend. */
function lastCall(): { args: Record<string, unknown>; command: string } {
  const calls = invokeMock.mock.calls;
  const [command, args] = calls[calls.length - 1] as [string, Record<string, unknown>];
  return { args, command };
}

const address = { sessionId: "sess_1", threadId: "thread_1" };

describe("remote session commands", () => {
  it("renames by session id and routes on the session's lane", async () => {
    await renameRemoteConversation("desktop_a", address, "New name");

    const { args, command } = lastCall();
    expect(command).toBe("remote_peer_request");
    expect(args).toEqual({
      desktopId: "desktop_a",
      command: { type: "set_session_name", sessionId: "sess_1", name: "New name" },
      lane: "sess_1",
    });
  });

  /**
   * The heart of the thread/session split: pin carries the *thread* id, and the
   * lane still names the session so the command is routed to that conversation.
   */
  it("pins by thread id, not by session id", async () => {
    await pinRemoteConversation("desktop_a", address, true);

    const { args } = lastCall();
    expect(args.command).toEqual({ type: "set_session_pinned", threadId: "thread_1", pinned: true });
    expect(args.lane).toBe("sess_1");
  });

  it("deletes by thread id", async () => {
    await deleteRemoteConversation("desktop_a", address);

    const { args } = lastCall();
    expect(args.command).toEqual({ type: "delete_session", threadId: "thread_1" });
  });

  /**
   * A host that reported no thread id cannot be pinned: the session id is not a
   * substitute, so the call is refused locally instead of sent and misapplied.
   * Asserted on both actions and on the fact that nothing was sent.
   */
  it("refuses to pin or delete a conversation whose host gave no thread id", async () => {
    const unknown = { sessionId: "sess_1", threadId: null };

    await expect(pinRemoteConversation("desktop_a", unknown, true))
      .rejects
      .toThrow("remote_conversation_without_thread_id");
    await expect(deleteRemoteConversation("desktop_a", unknown))
      .rejects
      .toThrow("remote_conversation_without_thread_id");
    expect(invokeMock).not.toHaveBeenCalled();
  });

  it("stops a run by session id", async () => {
    await abortRemoteRun("desktop_a", "sess_1");

    const { args } = lastCall();
    expect(args).toEqual({
      desktopId: "desktop_a",
      command: { type: "abort", sessionId: "sess_1" },
      lane: "sess_1",
    });
  });
});

describe("promptRemoteConversation", () => {
  it("asks an existing conversation to answer, on its own lane", async () => {
    invokeMock.mockResolvedValue({ sessionId: "sess_1", threadId: "thread_1" });

    const ack = await promptRemoteConversation("desktop_a", "sess_1", "hello");

    const { args } = lastCall();
    expect(args.command).toEqual({ type: "prompt", sessionId: "sess_1", message: "hello" });
    expect(args.lane).toBe("sess_1");
    expect(ack).toEqual({ sessionId: "sess_1", threadId: "thread_1" });
  });

  /**
   * The draft case: an empty session id is the host's own signal to create the
   * conversation, and the ack is the only way to learn the ids it chose. The
   * lane names the "new" lane, which is where the host listens for it.
   */
  it("creates a conversation when asked with no session id", async () => {
    invokeMock.mockResolvedValue({ sessionId: "sess_new", threadId: "thread_new" });

    const ack = await promptRemoteConversation("desktop_a", "", "first message");

    const { args } = lastCall();
    expect(args.command).toEqual({ type: "prompt", sessionId: "", message: "first message" });
    expect(args.lane).toBe("new");
    expect(ack.sessionId).toBe("sess_new");
  });

  /**
   * Without a session id from the host the caller has nothing to open and
   * cannot address the conversation again, so it is an error rather than a
   * prompt that appeared to work.
   */
  it("rejects an ack that names no conversation", async () => {
    invokeMock.mockResolvedValue({ threadId: "thread_new" });
    await expect(promptRemoteConversation("desktop_a", "", "hi"))
      .rejects
      .toThrow("remote_prompt_without_session_id");

    invokeMock.mockResolvedValue({ sessionId: "" });
    await expect(promptRemoteConversation("desktop_a", "", "hi"))
      .rejects
      .toThrow("remote_prompt_without_session_id");
  });

  /** A host that omits the thread id answers with an empty one, not `undefined`. */
  it("tolerates a host that reports no thread id", async () => {
    invokeMock.mockResolvedValue({ sessionId: "sess_new" });
    await expect(promptRemoteConversation("desktop_a", "", "hi"))
      .resolves
      .toEqual({ sessionId: "sess_new", threadId: "" });
  });
});

describe("forkRemoteConversation", () => {
  it("branches at the stored turn and answers with the child's ids", async () => {
    invokeMock.mockResolvedValue({ sessionId: "sess_child", threadId: "thread_child" });

    const child = await forkRemoteConversation("desktop_a", "sess_1", "entry_u1");

    const { args } = lastCall();
    expect(args).toEqual({
      desktopId: "desktop_a",
      command: { type: "fork_session", sessionId: "sess_1", sourceEntryId: "entry_u1" },
      // Routed on the parent's lane: the fork is a command *to* that
      // conversation, and the host answers on the same subject.
      lane: "sess_1",
    });
    expect(child).toEqual({ sessionId: "sess_child", threadId: "thread_child" });
  });

  /**
   * An empty `sessionId` is the host's signal for "a conversation that does not
   * exist yet", and a fork always has a parent. Sending one would quietly start
   * a new conversation instead of branching, so it is never sent.
   */
  it("never sends an empty parent or fork point", async () => {
    invokeMock.mockResolvedValue({ sessionId: "sess_child" });

    await forkRemoteConversation("desktop_a", "sess_1", "entry_u1");
    const { command } = lastCall().args as { command: Record<string, unknown> };
    expect(command.sessionId).toBe("sess_1");
    expect(command.sourceEntryId).toBe("entry_u1");
    expect(command.sourceEntryId).not.toBe("");
  });

  /**
   * Without a session id in the ack the caller cannot open or address the child,
   * so it is an error rather than a fork that appeared to work.
   */
  it("rejects an ack that names no child conversation", async () => {
    invokeMock.mockResolvedValue({ threadId: "thread_child" });
    await expect(forkRemoteConversation("desktop_a", "sess_1", "entry_u1"))
      .rejects
      .toThrow("remote_fork_without_session_id");

    invokeMock.mockResolvedValue({ sessionId: "" });
    await expect(forkRemoteConversation("desktop_a", "sess_1", "entry_u1"))
      .rejects
      .toThrow("remote_fork_without_session_id");
  });

  it("tolerates a host that reports no thread id for the child", async () => {
    invokeMock.mockResolvedValue({ sessionId: "sess_child" });
    await expect(forkRemoteConversation("desktop_a", "sess_1", "entry_u1"))
      .resolves
      .toEqual({ sessionId: "sess_child", threadId: "" });
  });
});

describe("fetchRemoteSessions", () => {
  it("carries the host's thread id through to the merged row", async () => {
    invokeMock.mockResolvedValue({
      desktopId: "desktop_a",
      sessions: [
        { sessionId: "sess_1", threadId: "thread_1", title: "one", lastMessageAt: 5 },
        { sessionId: "sess_2", title: "two" },
      ],
    });

    const catalog = await fetchRemoteSessions("desktop_a");

    expect(catalog.desktopId).toBe("desktop_a");
    expect(catalog.sessions[0]).toMatchObject({ sessionId: "sess_1", threadId: "thread_1" });
    // Absent, not conflated with the session id.
    expect(catalog.sessions[1]).not.toHaveProperty("threadId");
  });

  /** A row with no session id could not be opened, so it is dropped. */
  it("drops rows that name no session and tolerates a malformed payload", async () => {
    invokeMock.mockResolvedValue({
      sessions: [null, "nope", { title: "orphan" }, { sessionId: "" }],
    });

    const catalog = await fetchRemoteSessions("desktop_a");
    expect(catalog.sessions).toEqual([]);
  });

  it("treats a payload with no sessions as an empty catalogue", async () => {
    invokeMock.mockResolvedValue({});
    await expect(fetchRemoteSessions("desktop_a")).resolves.toEqual({ desktopId: "desktop_a", sessions: [] });

    invokeMock.mockResolvedValue(null);
    await expect(fetchRemoteSessions("desktop_a")).resolves.toEqual({ desktopId: "desktop_a", sessions: [] });
  });

  /** Missing/odd fields become the total shape the merge and UI assume. */
  it("normalises the optional fields of a row", async () => {
    invokeMock.mockResolvedValue({
      sessions: [{ sessionId: "sess_1", title: "", mode: "weird", workspaceId: "ws", pinned: "yes", lastMessageAt: "5" }],
    });

    const [row] = (await fetchRemoteSessions("desktop_a")).sessions;
    expect(row).toEqual({
      sessionId: "sess_1",
      // Empty title falls back to the id, so the row is never blank.
      title: "sess_1",
      mode: "chat",
      workspaceId: "ws",
      pinned: false,
      streaming: false,
      lastMessageAt: null,
    });
  });

  it("keeps a workspace session's mode and a host's status string", async () => {
    invokeMock.mockResolvedValue({
      sessions: [{ sessionId: "sess_1", title: "w", mode: "workspace", status: "running", pinned: true, streaming: true, lastMessageAt: 7 }],
    });

    const [row] = (await fetchRemoteSessions("desktop_a")).sessions;
    expect(row).toEqual({
      sessionId: "sess_1",
      title: "w",
      mode: "workspace",
      status: "running",
      pinned: true,
      streaming: true,
      lastMessageAt: 7,
    });
  });
});
