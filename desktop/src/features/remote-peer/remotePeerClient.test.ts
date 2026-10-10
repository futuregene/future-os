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
  compactRemoteConversation,
  deleteRemoteConversation,
  downloadRemoteFile,
  fetchRemoteSessions,
  forkRemoteConversation,
  listRemoteSessionFiles,
  pinRemoteConversation,
  promptRemoteConversation,
  renameRemoteConversation,
  uploadRemoteFile,
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

  /**
   * Attachments travel as `[{uploadId}]`, which is the shape the host parses:
   * `UploadReference` has exactly that one field. A bare string array would be
   * silently dropped by the host, and the file would never be attached.
   */
  it("sends attachments as the references the host parses", async () => {
    invokeMock.mockResolvedValue({ sessionId: "sess_1", threadId: "thread_1" });

    await promptRemoteConversation("desktop_a", "sess_1", "look at these", [
      "upload_1",
      "upload_2",
    ]);

    expect(lastCall().args.command).toEqual({
      type: "prompt",
      sessionId: "sess_1",
      message: "look at these",
      attachments: [{ uploadId: "upload_1" }, { uploadId: "upload_2" }],
    });
  });

  /** No attachments means no field at all, rather than an empty list. */
  it("omits the attachments field when there are none", async () => {
    invokeMock.mockResolvedValue({ sessionId: "sess_1", threadId: "thread_1" });

    await promptRemoteConversation("desktop_a", "sess_1", "just words");

    expect(lastCall().args.command).not.toHaveProperty("attachments");
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

describe("compactRemoteConversation", () => {
  it("asks the host to compact, on that conversation's lane", async () => {
    invokeMock.mockResolvedValue({ accepted: true, operationId: "op_1" });

    await compactRemoteConversation("desktop_a", "sess_1");

    expect(lastCall().args).toEqual({
      desktopId: "desktop_a",
      command: { type: "compact_context", sessionId: "sess_1" },
      lane: "sess_1",
    });
  });

  /**
   * An ack is an *acknowledgement*, not a result. One without an `operationId`
   * is not a real acceptance — the host uses that id to correlate the events
   * that follow — so resolving on it would report success for a request the
   * host never took.
   */
  it("rejects an acknowledgement the host did not really take", async () => {
    for (const ack of [
      { accepted: false, operationId: "op_1" },
      { accepted: true },
      { accepted: true, operationId: "" },
      { operationId: "op_1" },
      {},
      null,
    ]) {
      invokeMock.mockResolvedValue(ack);
      await expect(compactRemoteConversation("desktop_a", "sess_1"))
        .rejects
        .toThrow("remote_compaction_not_accepted");
    }
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

describe("listRemoteSessionFiles", () => {
  it("asks the host for a session's root, and for a directory by its own path", async () => {
    invokeMock.mockResolvedValue({ rootPath: "/root", path: "/root", entries: [] });

    await listRemoteSessionFiles("desktop_a", "sess_1");
    // An absent path is sent as absent, not as "": the backend turns it into the
    // empty path the host reads as the session root, and sending "" from here
    // would make that translation invisible.
    expect(lastCall().args).toEqual({ desktopId: "desktop_a", sessionId: "sess_1" });

    await listRemoteSessionFiles("desktop_a", "sess_1", "/root/nested");
    expect(lastCall().args).toEqual({
      desktopId: "desktop_a",
      sessionId: "sess_1",
      path: "/root/nested",
    });
  });

  it("keeps the entries a user could actually open", async () => {
    invokeMock.mockResolvedValue({
      rootPath: "/root",
      path: "/root",
      entries: [
        { name: "a.txt", path: "/root/a.txt", isDir: false, size: 12 },
        { name: "src", path: "/root/src", isDir: true, size: 0 },
      ],
    });

    const result = await listRemoteSessionFiles("desktop_a", "sess_1");
    expect(result).toEqual({
      rootPath: "/root",
      path: "/root",
      entries: [
        { name: "a.txt", path: "/root/a.txt", isDir: false, size: 12 },
        { name: "src", path: "/root/src", isDir: true, size: 0 },
      ],
    });
  });

  it("drops rows that could not be opened and tolerates a malformed payload", async () => {
    invokeMock.mockResolvedValue({
      entries: [
        null,
        "nope",
        { name: "no-path.txt" },
        { path: "/root/no-name.txt" },
        { name: "", path: "/root/empty.txt" },
        { name: "ok.txt", path: "/root/ok.txt", size: "12" },
      ],
    });

    const result = await listRemoteSessionFiles("desktop_a", "sess_1");
    // The one usable row survives, with the unusable `size` defaulted.
    expect(result.entries).toEqual([
      { name: "ok.txt", path: "/root/ok.txt", isDir: false, size: 0 },
    ]);
    expect(result.rootPath).toBe("");
    expect(result.path).toBe("");

    invokeMock.mockResolvedValue(null);
    await expect(listRemoteSessionFiles("desktop_a", "sess_1")).resolves.toEqual({
      rootPath: "",
      path: "",
      entries: [],
    });
  });
});

describe("downloadRemoteFile", () => {
  it("addresses the file by the host's path and reports the host's name", async () => {
    invokeMock.mockResolvedValue("report-preview.png");

    const saved = await downloadRemoteFile({
      desktopId: "desktop_a",
      sessionId: "sess_1",
      path: "/root/report.txt",
      name: "report.txt",
      destination: "/local/saved.png",
    });

    expect(lastCall().command).toBe("remote_peer_download_file");
    expect(lastCall().args).toEqual({
      desktopId: "desktop_a",
      sessionId: "sess_1",
      path: "/root/report.txt",
      name: "report.txt",
      destination: "/local/saved.png",
    });
    // The name to show is the one the host chose, not the one requested.
    expect(saved).toBe("report-preview.png");
  });

  it("passes a variant only when one was asked for", async () => {
    invokeMock.mockResolvedValue("n");

    await downloadRemoteFile({
      desktopId: "desktop_a",
      sessionId: "sess_1",
      path: "/p",
      name: "n",
      destination: "/d",
    });
    expect(lastCall().args).not.toHaveProperty("variant");

    await downloadRemoteFile({
      desktopId: "desktop_a",
      sessionId: "sess_1",
      path: "/p",
      name: "n",
      destination: "/d",
      variant: "preview",
    });
    expect(lastCall().args).toHaveProperty("variant", "preview");
  });

  it("propagates a refused download", async () => {
    invokeMock.mockRejectedValue(new Error("remote_download_hash_mismatch"));
    await expect(downloadRemoteFile({
      desktopId: "desktop_a",
      sessionId: "sess_1",
      path: "/p",
      name: "n",
      destination: "/d",
    })).rejects.toThrow("remote_download_hash_mismatch");
  });
});

describe("uploadRemoteFile", () => {
  it("sends the path, and the name only when there is one", async () => {
    invokeMock.mockResolvedValue({ uploadId: "upload_1", name: "n.txt", contentHash: "h" });

    await uploadRemoteFile({ desktopId: "desktop_a", path: "/local/n.txt" });
    // Absent rather than empty: the host takes a *missing* name as "use the
    // file's own", and an empty string as a name it then has to reject.
    expect(lastCall().args).toEqual({ desktopId: "desktop_a", path: "/local/n.txt" });
    expect(lastCall().args).not.toHaveProperty("name");

    await uploadRemoteFile({ desktopId: "desktop_a", path: "/local/DSC.JPG", name: "Holiday.jpg" });
    expect(lastCall().args).toEqual({
      desktopId: "desktop_a",
      path: "/local/DSC.JPG",
      name: "Holiday.jpg",
    });
  });

  it("propagates a refused upload", async () => {
    invokeMock.mockRejectedValue(new Error("Original file exceeds the 10 MiB limit"));
    await expect(uploadRemoteFile({ desktopId: "desktop_a", path: "/local/big.bin" }))
      .rejects
      .toThrow("10 MiB");
  });
});
