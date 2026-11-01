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
  connectRemotePeer,
  disconnectRemotePeer,
  compactRemoteConversation,
  connectAllRemotePeers,
  continueRemoteRun,
  createRemoteWorkspace,
  deleteRemoteConversation,
  deleteRemoteWorkspace,
  downloadRemoteFile,
  fetchRemoteSessions,
  listRemoteSettingsModels,
  forkRemoteConversation,
  generateRemoteTitle,
  getRemoteConversationSettings,
  getRemoteApprovalSettings,
  installRemoteSkill,
  listRemoteAvailableSkills,
  listRemoteInstalledSkills,
  listRemoteModels,
  getRemotePromptReceipt,
  listRemotePeers,
  pairRemotePeer,
  previewRemoteFile,
  listRemoteSessionFiles,
  listRemoteWorkspaces,
  localizedSkill,
  pinRemoteConversation,
  promptRemoteConversation,
  remoteModelProvider,
  remoteModelReference,
  renameRemoteConversation,
  setRemoteConversationModel,
  setRemoteConversationThinkingLevel,
  setRemotePeerLabel,
  setRemoteWorkspacePinned,
  takeoverRemotePeer,
  uninstallRemoteSkill,
  unpairRemotePeer,
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

  /**
   * The workspace's name, which the merged list uses as a group heading. This
   * parser names every field it keeps, so a field the host sends is dropped
   * here unless it is listed — the heading fell back to the generic label until
   * this was carried (found by asserting the heading text in the harness).
   */
  it("carries the host's workspace name through to the merged row", async () => {
    invokeMock.mockResolvedValue({
      sessions: [
        { sessionId: "sess_1", mode: "workspace", workspaceId: "ws_1", workspaceName: "图表脚本" },
        { sessionId: "sess_2", mode: "workspace", workspaceId: "ws_1" },
      ],
    });

    const catalog = await fetchRemoteSessions("desktop_a");

    expect(catalog.sessions[0]).toMatchObject({ workspaceName: "图表脚本" });
    // Absent rather than an empty heading: a host that predates the field leaves
    // the heading to the caller's fallback.
    expect(catalog.sessions[1]).not.toHaveProperty("workspaceName");
  });

  /**
   * The two fields the merged list draws a group's *place* and a fork's *parent*
   * from. Both are the host's own facts: the place is the key its workspace list
   * is ordered by, and the parent is the session the fork came from.
   */
  it("carries a workspace's own place and a fork's parent", async () => {
    invokeMock.mockResolvedValue({
      sessions: [
        {
          sessionId: "sess_fork",
          mode: "workspace",
          workspaceId: "ws_1",
          workspaceOrderedAt: 1_700_000_000_000,
          parentSessionId: "sess_parent",
        },
        { sessionId: "sess_plain" },
      ],
    });

    const catalog = await fetchRemoteSessions("desktop_a");

    expect(catalog.sessions[0]).toMatchObject({
      parentSessionId: "sess_parent",
      workspaceOrderedAt: 1_700_000_000_000,
    });
    // A host that predates either field says nothing rather than something
    // wrong: no place (not "now") and no parent (not a root by accident).
    expect(catalog.sessions[1]).not.toHaveProperty("parentSessionId");
    expect(catalog.sessions[1]).not.toHaveProperty("workspaceOrderedAt");
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

describe("remote model references", () => {
  /**
   * A catalogue id is provider-local, even when it contains a slash — there are
   * models whose id *is* `openrouter/auto`. Inferring the provider from the id
   * would then read that id as provider `openrouter`, model `auto`, and select a
   * different model than the one the user picked.
   */
  it("prefixes the provider rather than reading one out of the id", () => {
    expect(remoteModelReference({ id: "gpt-5", provider: "openai" })).toBe("openai/gpt-5");
    expect(remoteModelReference({ id: "auto", provider: "openrouter" })).toBe("openrouter/auto");
    expect(remoteModelProvider("openrouter/auto")).toBe("openrouter");

    // An id that itself starts with a provider name keeps its directory: the
    // reference is the prefix plus the whole id.
    expect(remoteModelReference({ id: "openrouter/auto", provider: "future" })).toBe("future/openrouter/auto");
    // And a slash inside with no provider is still just the id.
    expect(remoteModelReference({ id: "vendor/model" })).toBe("vendor/model");
  });

  it("has no provider for a bare id", () => {
    expect(remoteModelProvider("gpt-5")).toBeUndefined();
    // A leading slash is not a provider: there is nothing before it.
    expect(remoteModelProvider("/gpt-5")).toBeUndefined();
  });
});

describe("listRemoteSettingsModels", () => {
  /**
   * The *settings* command, not the picker's. The host strips hidden models out
   * of `list_models`, so a visibility page fed from that could only ever show
   * models that are already on — a hidden one could not be switched back on.
   */
  it("asks the host for the unfiltered catalogue", async () => {
    invokeMock.mockResolvedValue({ models: [{ id: "gpt-5", provider: "openai" }] });

    const models = await listRemoteSettingsModels("desktop_a");

    expect(invokeMock).toHaveBeenCalledWith("remote_peer_request", {
      desktopId: "desktop_a",
      command: { type: "list_settings_models" },
      lane: "list",
    });
    expect(models).toEqual([{ id: "gpt-5", provider: "openai" }]);
  });
});

describe("listRemoteModels", () => {
  it("asks on the catalogue lane and keeps the rows a picker can use", async () => {
    invokeMock.mockResolvedValue({
      models: [
        { id: "gpt-5", label: "GPT-5", provider: "openai", isDefault: true },
        { id: "auto", provider: "openrouter" },
      ],
    });

    const models = await listRemoteModels("desktop_a");

    expect(lastCall().args).toEqual({ desktopId: "desktop_a", command: { type: "list_models" }, lane: "list" });
    expect(models).toEqual([
      { id: "gpt-5", label: "GPT-5", provider: "openai", isDefault: true },
      { id: "auto", provider: "openrouter" },
    ]);
  });

  it("keeps only the thinking levels a host actually named", async () => {
    // The level list comes off the wire as `unknown[]`: a host that mixed in a number
    // or a null must not put a level in the menu that no model can be set to.
    invokeMock.mockResolvedValue({
      models: [{ id: "gpt-5", thinkingLevels: ["low", 3, null, "high"], thinkingLevel: "low" }],
    });

    expect(await listRemoteModels("desktop_a")).toEqual([
      { id: "gpt-5", thinkingLevel: "low", thinkingLevels: ["low", "high"] },
    ]);
  });

  it("drops rows that name no model and tolerates a malformed payload", async () => {
    invokeMock.mockResolvedValue({ models: [null, "nope", { label: "orphan" }, { id: "" }, { id: "ok" }] });
    expect(await listRemoteModels("desktop_a")).toEqual([{ id: "ok" }]);

    invokeMock.mockResolvedValue({});
    expect(await listRemoteModels("desktop_a")).toEqual([]);

    invokeMock.mockResolvedValue(null);
    expect(await listRemoteModels("desktop_a")).toEqual([]);
  });
});

describe("conversation settings", () => {
  it("reads the model and thinking level on the conversation's lane", async () => {
    invokeMock.mockResolvedValue({ model: "openai/gpt-5", thinkingLevel: "medium" });

    expect(await getRemoteConversationSettings("desktop_a", "sess_1")).toEqual({
      model: "openai/gpt-5",
      thinkingLevel: "medium",
    });
    expect(lastCall().args).toEqual({
      desktopId: "desktop_a",
      command: { type: "get_state", sessionId: "sess_1" },
      lane: "sess_1",
    });
  });

  /** A host that omits them is "did not say", not an empty model. */
  it("reads an omitted setting as null", async () => {
    invokeMock.mockResolvedValue({});
    expect(await getRemoteConversationSettings("desktop_a", "sess_1")).toEqual({
      model: null,
      thinkingLevel: null,
    });

    invokeMock.mockResolvedValue({ model: "", thinkingLevel: 7 });
    expect(await getRemoteConversationSettings("desktop_a", "sess_1")).toEqual({
      model: null,
      thinkingLevel: null,
    });
  });
});

describe("setting a remote conversation's model", () => {
  it("sends the reference and its provider, on the conversation's lane", async () => {
    invokeMock.mockResolvedValue(undefined);

    await setRemoteConversationModel("desktop_a", "sess_1", "openrouter/auto");

    expect(lastCall().args).toEqual({
      desktopId: "desktop_a",
      command: {
        type: "set_model",
        sessionId: "sess_1",
        modelId: "openrouter/auto",
        providerId: "openrouter",
      },
      lane: "sess_1",
    });
  });

  /** A bare id has no provider to send, so the field is left out rather than empty. */
  it("omits the provider for a bare id", async () => {
    invokeMock.mockResolvedValue(undefined);

    await setRemoteConversationModel("desktop_a", "sess_1", "gpt-5");

    const { command } = lastCall().args as { command: Record<string, unknown> };
    expect(command.modelId).toBe("gpt-5");
    expect(command).not.toHaveProperty("providerId");
  });

  it("sends a thinking-level change", async () => {
    invokeMock.mockResolvedValue(undefined);

    await setRemoteConversationThinkingLevel("desktop_a", "sess_1", "xhigh");

    expect(lastCall().args).toEqual({
      desktopId: "desktop_a",
      command: { type: "set_thinking_level", sessionId: "sess_1", level: "xhigh" },
      lane: "sess_1",
    });
  });

  it("propagates a refused change", async () => {
    invokeMock.mockRejectedValue(new Error("model_not_available"));
    await expect(setRemoteConversationModel("desktop_a", "sess_1", "openai/gpt-5"))
      .rejects
      .toThrow("model_not_available");
  });
});

describe("continueRemoteRun", () => {
  it("names the session and the failed run, on the conversation's lane", async () => {
    invokeMock.mockResolvedValue(undefined);

    await continueRemoteRun("desktop_a", "sess_1", "run_7");

    expect(lastCall().args).toEqual({
      desktopId: "desktop_a",
      command: { type: "continue_run", sessionId: "sess_1", runId: "run_7" },
      lane: "sess_1",
    });
  });

  /** The host refuses a continuation it cannot justify, and its reason is passed on. */
  it("propagates a refused continuation", async () => {
    invokeMock.mockRejectedValue(new Error("Only a failed run can be continued."));
    await expect(continueRemoteRun("desktop_a", "sess_1", "run_7"))
      .rejects
      .toThrow("Only a failed run can be continued.");
  });
});

describe("localizedSkill", () => {
  const both = {
    id: "both",
    name: "Both",
    description: "English text",
    nameZh: "两者",
    descriptionZh: "中文说明",
  };

  it("picks the language it was asked for", () => {
    expect(localizedSkill(both, "zh")).toEqual({ name: "两者", description: "中文说明" });
    expect(localizedSkill(both, "zh-CN")).toEqual({ name: "两者", description: "中文说明" });
    expect(localizedSkill(both, "en")).toEqual({ name: "Both", description: "English text" });
    expect(localizedSkill(both, "en-US")).toEqual({ name: "Both", description: "English text" });
  });

  /**
   * A catalogue entry with no translation falls back to the language it has,
   * rather than to nothing: the user is deciding whether to install it, and a
   * blank row tells them nothing.
   */
  it("falls back rather than going blank", () => {
    const englishOnly = { id: "e", name: "English only", description: "Only English" };
    expect(localizedSkill(englishOnly, "zh")).toEqual({
      name: "English only",
      description: "Only English",
    });

    const chineseOnly = { id: "c", name: "", description: "", nameZh: "只有中文", descriptionZh: "说明" };
    expect(localizedSkill(chineseOnly, "en")).toEqual({ name: "只有中文", description: "说明" });
  });

  /** A row with neither translation is its id, which is what the host named it. */
  it("uses the id when there is no name at all", () => {
    expect(localizedSkill({ id: "bare", name: "", description: "" }, "en")).toEqual({
      name: "bare",
      description: "",
    });
  });
});

describe("remote skills", () => {
  it("reads both lists on the catalogue lane", async () => {
    invokeMock.mockResolvedValue({
      skills: [
        { id: "alpha", name: "Alpha", description: "installed", version: "1.2.0" },
        { id: "beta", name: "Beta", description: "also installed" },
      ],
    });

    const list = await listRemoteInstalledSkills("desktop_a");

    expect(lastCall().args).toEqual({ desktopId: "desktop_a", command: { type: "list_skills" }, lane: "list" });
    expect(list).toEqual([
      { id: "alpha", name: "Alpha", description: "installed", version: "1.2.0" },
      { id: "beta", name: "Beta", description: "also installed" },
    ]);

    invokeMock.mockResolvedValue({ skills: [{ id: "gamma", name: "Gamma", description: "new", latestVersion: "3.0.0" }] });
    const catalogue = await listRemoteAvailableSkills("desktop_a");
    expect(lastCall().args).toEqual({ desktopId: "desktop_a", command: { type: "list_available_skills" }, lane: "list" });
    expect(catalogue).toEqual([{ id: "gamma", name: "Gamma", description: "new", latestVersion: "3.0.0" }]);
  });

  /** A row with no usable name is dropped, like every other list here. */
  it("drops rows that name no skill and tolerates a malformed payload", async () => {
    invokeMock.mockResolvedValue({ skills: [null, "nope", { name: "orphan" }, { id: "" }, { id: "ok" }] });
    expect(await listRemoteInstalledSkills("desktop_a")).toEqual([
      { id: "ok", name: "ok", description: "" },
    ]);

    invokeMock.mockResolvedValue({});
    expect(await listRemoteInstalledSkills("desktop_a")).toEqual([]);

    invokeMock.mockResolvedValue(null);
    expect(await listRemoteAvailableSkills("desktop_a")).toEqual([]);
  });

  it("installs a named version and removes by id", async () => {
    invokeMock.mockResolvedValue(undefined);
    await installRemoteSkill("desktop_a", "gamma", "3.0.0");
    expect(lastCall().args).toEqual({
      desktopId: "desktop_a",
      command: { type: "install_skill", skillId: "gamma", version: "3.0.0" },
      lane: "list",
    });

    invokeMock.mockResolvedValue({ removed: true });
    await expect(uninstallRemoteSkill("desktop_a", "alpha")).resolves.toBe(true);
    expect(lastCall().args).toEqual({
      desktopId: "desktop_a",
      command: { type: "uninstall_skill", skillId: "alpha" },
      lane: "list",
    });
  });

  /**
   * `removed: false` is not an error: two clients can be looking at the same
   * list, and the second one asking to remove what the first already removed has
   * its answer.
   */
  it("reads a no-op removal as a plain false", async () => {
    invokeMock.mockResolvedValue({ removed: false });
    await expect(uninstallRemoteSkill("desktop_a", "alpha")).resolves.toBe(false);

    // A host that omits the field has not claimed it removed anything.
    invokeMock.mockResolvedValue({});
    await expect(uninstallRemoteSkill("desktop_a", "alpha")).resolves.toBe(false);
  });

  it("propagates a refused install or removal", async () => {
    invokeMock.mockRejectedValue(new Error("skill_not_found"));
    await expect(installRemoteSkill("desktop_a", "ghost", "1")).rejects.toThrow("skill_not_found");
    await expect(uninstallRemoteSkill("desktop_a", "ghost")).rejects.toThrow("skill_not_found");
  });
});

describe("generating a title on a host", () => {
  /** The session is both what is named and what the command routes on. */
  it("asks the host to name that conversation in the given language", async () => {
    invokeMock.mockResolvedValue({ title: "  Nightly build  " });
    await expect(generateRemoteTitle("desktop_a", "sess_1", "zh")).resolves.toBe("Nightly build");

    const { args, command } = lastCall();
    expect(command).toBe("remote_peer_request");
    expect(args).toEqual({
      desktopId: "desktop_a",
      command: { type: "generate_session_title", sessionId: "sess_1", mode: "zh" },
      lane: "sess_1",
    });
  });

  /**
   * A title that came back empty is a failure, not a title: saving it would
   * leave the conversation unnamed, and this app's own generate action treats an
   * empty answer as an error for the same reason.
   */
  it("refuses an empty title rather than returning one", async () => {
    invokeMock.mockResolvedValue({ title: "   " });
    await expect(generateRemoteTitle("desktop_a", "sess_1", "en")).rejects.toThrow("empty_title");

    invokeMock.mockResolvedValue({});
    await expect(generateRemoteTitle("desktop_a", "sess_1", "en")).rejects.toThrow("empty_title");
  });
});

describe("a host's workspaces", () => {
  it("lists them from the host's own snapshot", async () => {
    invokeMock.mockResolvedValue({
      workspaces: [{ id: "ws_1", name: "future-os", path: "/code/future-os", pinned: true }],
    });
    await expect(listRemoteWorkspaces("desktop_a")).resolves.toEqual([
      { id: "ws_1", name: "future-os", path: "/code/future-os", pinned: true },
    ]);
    expect(lastCall().args).toEqual({ desktopId: "desktop_a" });
  });

  /**
   * A workspace with no name falls back to its path — a row has to be labelled
   * with something the user recognises, and the folder is what they picked.
   */
  it("labels an unnamed workspace with its path", async () => {
    invokeMock.mockResolvedValue({ workspaces: [{ id: "ws_1", path: "/code/x" }] });
    await expect(listRemoteWorkspaces("desktop_a")).resolves.toEqual([
      { id: "ws_1", name: "/code/x", path: "/code/x", pinned: false },
    ]);
  });

  /** A snapshot that is not a list is empty, not a crash. */
  it("reads a malformed snapshot as empty", async () => {
    invokeMock.mockResolvedValue({ workspaces: "nonsense" });
    await expect(listRemoteWorkspaces("desktop_a")).resolves.toEqual([]);

    invokeMock.mockResolvedValue(undefined);
    await expect(listRemoteWorkspaces("desktop_a")).resolves.toEqual([]);
  });

  /** Entries without an id cannot be addressed, so they are dropped. */
  it("drops entries that cannot be addressed", async () => {
    invokeMock.mockResolvedValue({
      workspaces: [null, 42, { name: "no id" }, { id: "" }, { id: "ws_1", path: "/x" }],
    });
    await expect(listRemoteWorkspaces("desktop_a")).resolves.toEqual([
      { id: "ws_1", name: "/x", path: "/x", pinned: false },
    ]);
  });

  /**
   * A workspace's place and whether it is a folder the user chose.
   *
   * Both are what the merged list needs to draw a heading for a workspace with
   * nothing in it: the place ranks it, and a `temporary` one is chat scratch
   * whose conversations belong to the chats section rather than under a folder.
   * The place is the host's coalesce (`last_opened_at`, else `updated_at`) — the
   * key that host's own workspace list is ordered by.
   */
  it("carries a workspace's place and kind", async () => {
    invokeMock.mockResolvedValue({
      workspaces: [
        { id: "ws_open", name: "open", path: "/a", kind: "user", lastOpenedAt: 700, updatedAt: 100 },
        { id: "ws_touched", name: "touched", path: "/b", kind: "user", updatedAt: 300 },
        { id: "ws_tmp", name: "scratch", path: "/tmp", kind: "temporary" },
      ],
    });

    const rows = await listRemoteWorkspaces("desktop_a");
    // A workspace opened last is placed by when it was opened, even if it was
    // created before one that has only been touched.
    expect(rows[0]).toMatchObject({ id: "ws_open", kind: "user", orderedAt: 700 });
    expect(rows[1]).toMatchObject({ id: "ws_touched", orderedAt: 300 });
    expect(rows[2]).toMatchObject({ id: "ws_tmp", kind: "temporary" });
    // Nothing to place it by: absent, so it sorts last rather than as "now".
    expect(rows[2]).not.toHaveProperty("orderedAt");
  });

  /**
   * Registering a workspace sends the host's path as typed.
   *
   * The directory has to exist on *that* machine, so this one cannot check it —
   * the host decides, which is why the path is not validated here.
   */
  it("registers a path on the host, trimmed", async () => {
    invokeMock.mockResolvedValue({ workspace: { id: "ws_9", name: "new", path: "/code/new" } });
    await expect(createRemoteWorkspace("desktop_a", "  /code/new  ", "  new  ")).resolves.toEqual({
      id: "ws_9",
      name: "new",
      path: "/code/new",
      pinned: false,
    });

    expect(lastCall().args).toEqual({
      desktopId: "desktop_a",
      command: { type: "create_workspace", path: "/code/new", name: "new" },
      lane: "list",
    });
  });

  /** A host that answers a create with no workspace is reported, not rendered blank. */
  it("refuses a create that came back without a workspace", async () => {
    invokeMock.mockResolvedValue({});
    await expect(createRemoteWorkspace("desktop_a", "/code/new", "")).rejects.toThrow("invalid_workspace_snapshot");
  });

  it("pins and removes by workspace id", async () => {
    await setRemoteWorkspacePinned("desktop_a", "ws_1", true);
    expect(lastCall().args).toEqual({
      desktopId: "desktop_a",
      command: { type: "set_workspace_pinned", workspaceId: "ws_1", pinned: true },
      lane: "list",
    });

    await deleteRemoteWorkspace("desktop_a", "ws_1");
    expect(lastCall().args).toEqual({
      desktopId: "desktop_a",
      command: { type: "delete_workspace", workspaceId: "ws_1" },
      lane: "list",
    });
  });
});

describe("the peer list", () => {
  /**
   * A command the backend does not answer must read as "no hosts", not `null`.
   *
   * The value goes straight into React state, and a screen that then renders it
   * (`peers.map`, `peers.find`) throws during render — which takes the *whole
   * application* down, not just the remote screen. That is not hypothetical: the
   * screenshot harness's mock returns `null` for a command it does not
   * implement, and this call blanked every screen of the desktop app.
   */
  it("reads an unanswered command as no hosts rather than null", async () => {
    invokeMock.mockResolvedValue(null);
    await expect(listRemotePeers()).resolves.toEqual([]);

    invokeMock.mockResolvedValue(undefined);
    await expect(listRemotePeers()).resolves.toEqual([]);
  });

  it("keeps the hosts the backend did report", async () => {
    invokeMock.mockResolvedValue([{ desktopId: "desktop_a" }]);
    await expect(listRemotePeers()).resolves.toEqual([{ desktopId: "desktop_a", stopped: false }]);
  });

  /**
   * `stopped` is normalised rather than passed through, because its *absence* has
   * a meaning the caller acts on: a backend from before the mark existed means
   * "not stopped", and a caller that decides whether to reconnect a machine must
   * be able to trust the field rather than a truthiness test on `undefined`.
   */
  it("reads a host the backend did not mark as not stopped", async () => {
    invokeMock.mockResolvedValue([{ desktopId: "desktop_a", stopped: undefined }]);
    await expect(listRemotePeers()).resolves.toEqual([{ desktopId: "desktop_a", stopped: false }]);
  });

  /**
   * Who may drive the host rides on the same list, and its two arms are read
   * very differently: `standby` is what makes the machine's rows absent, and a
   * word this client does not know is not a state to act on. Keeping an unknown
   * word would let a newer host's vocabulary decide what this window draws.
   */
  it("keeps a control state it knows and drops one it does not", async () => {
    invokeMock.mockResolvedValue([
      { desktopId: "desktop_a", control: "standby", holder: { name: "Phone" } },
      { desktopId: "desktop_b", control: "active" },
      { desktopId: "desktop_c", control: "handover" },
      { desktopId: "desktop_d", holder: "a string, not a holder" },
    ]);
    const peers = await listRemotePeers();

    expect(peers[0]).toEqual({
      desktopId: "desktop_a",
      stopped: false,
      control: "standby",
      holder: { name: "Phone" },
    });
    expect(peers[1]!.control).toBe("active");
    expect(peers[2]).not.toHaveProperty("control");
    expect(peers[3]).not.toHaveProperty("holder");
  });
});

describe("the connect-all pass", () => {
  it("asks the backend to bring every host up", async () => {
    invokeMock.mockResolvedValue([{ desktopId: "desktop_a", stopped: true }]);

    await expect(connectAllRemotePeers())
      .resolves
      .toEqual([{ desktopId: "desktop_a", stopped: true }]);
    expect(lastCall().command).toBe("remote_peer_connect_all");
  });

  /** The caller decides a machine's state from this list, so it is never null. */
  it("reads an unanswered command as no hosts", async () => {
    invokeMock.mockResolvedValue(null);
    await expect(connectAllRemotePeers()).resolves.toEqual([]);
  });
});

/**
 * The pairing lifecycle: commands the user reaches from the machines screen
 * rather than from a conversation.
 *
 * Each is a one-line passthrough, and that is exactly why the assertion is the
 * command *name*: the backend dispatches on it, so a typo here is a button that
 * silently does nothing (the invoke rejects, and the caller's catch shows a
 * message about a command that does not exist). The id each one addresses is the
 * second half: a machine is addressed by its `desktopId`, never by a name or a
 * pair id.
 */
describe("remote machine commands", () => {
  it("pairs from an invitation string", async () => {
    await pairRemotePeer("invitation-blob");

    const { args, command } = lastCall();
    expect(command).toBe("remote_peer_pair");
    expect(args).toEqual({ invitation: "invitation-blob" });
  });

  it("connects and disconnects by desktop id", async () => {
    await connectRemotePeer("desktop_a");
    expect(lastCall()).toEqual({ args: { desktopId: "desktop_a" }, command: "remote_peer_connect" });

    await disconnectRemotePeer("desktop_a");
    expect(lastCall()).toEqual({ args: { desktopId: "desktop_a" }, command: "remote_peer_disconnect" });
  });

  /**
   * Taking a machine back is a pairing-level command, not a conversation one:
   * it names no session, so it rides the `list` lane. The host moves its control
   * pointer and answers; the caller re-reads the machine rather than assuming.
   */
  it("takes a machine over on the pairing's own lane", async () => {
    await takeoverRemotePeer("desktop_a");

    expect(lastCall()).toEqual({
      args: { desktopId: "desktop_a", command: { type: "takeover" }, lane: "list" },
      command: "remote_peer_request",
    });
  });

  it("unpairs by desktop id and reports the host's own answer", async () => {
    // The host answers with a message of its own (why it could not forget the
    // machine, or nothing), so the helper forwards it rather than swallowing it.
    invokeMock.mockResolvedValue("bridge still running");
    await expect(unpairRemotePeer("desktop_a")).resolves.toBe("bridge still running");

    expect(lastCall()).toEqual({ args: { desktopId: "desktop_a" }, command: "remote_peer_unpair" });
  });

  it("renames a machine, sending only the fields the caller set", async () => {
    // A label patch is partial on purpose: sending `icon: undefined` would ask
    // the host to clear it, which is not what "rename it" means.
    await setRemotePeerLabel("desktop_a", { name: "Studio" });
    expect(lastCall()).toEqual({
      args: { desktopId: "desktop_a", name: "Studio" },
      command: "remote_peer_set_label",
    });

    await setRemotePeerLabel("desktop_a", { icon: "laptop" });
    expect(lastCall()).toEqual({
      args: { desktopId: "desktop_a", icon: "laptop" },
      command: "remote_peer_set_label",
    });
  });

  it("fetches a preview into this app's own cache", async () => {
    invokeMock.mockResolvedValue({ name: "shot.png", path: "/cache/shot.png" });
    await expect(previewRemoteFile({
      desktopId: "desktop_a",
      name: "shot.png",
      path: "/home/u/shot.png",
      sessionId: "sess_1",
    })).resolves.toEqual({ name: "shot.png", path: "/cache/shot.png" });

    const { args, command } = lastCall();
    expect(command).toBe("remote_peer_preview_file");
    expect(args).toEqual({
      desktopId: "desktop_a",
      name: "shot.png",
      path: "/home/u/shot.png",
      sessionId: "sess_1",
    });
  });

  /**
   * A receipt is only usable when it names the conversation *and* the run.
   *
   * The caller uses it to decide "the host already accepted this, do not send it
   * again". A receipt without a run id cannot say that — the prompt may have been
   * accepted and the run already finished — so it is reported as no receipt, and
   * a `threadId` the host omitted becomes the empty string rather than undefined,
   * which is what the caller's own record shape expects.
   */
  it("reads a receipt only when it names a session and a run", async () => {
    invokeMock.mockResolvedValue({ sessionId: "sess_1", runId: "run_1" });
    await expect(getRemotePromptReceipt("desktop_a", "cmd_1")).resolves.toEqual({
      sessionId: "sess_1",
      runId: "run_1",
      threadId: "",
    });
    expect(lastCall()).toEqual({
      args: { desktopId: "desktop_a", command: { type: "get_prompt_receipt", promptId: "cmd_1" }, lane: "list" },
      command: "remote_peer_request",
    });
  });
});

describe("remote sandbox compatibility", () => {
  it("preserves the limited capability diagnostic and accepts older hosts", async () => {
    invokeMock.mockResolvedValue({ approvalTier: "auto", sandboxAvailable: true, sandboxCode: "available_write_only" });
    await expect(getRemoteApprovalSettings("desktop_a")).resolves.toEqual({
      approvalTier: "auto",
      sandboxAvailable: true,
      sandboxCode: "available_write_only",
    });
    invokeMock.mockResolvedValue({ approvalTier: "sandbox", sandboxAvailable: true });
    await expect(getRemoteApprovalSettings("desktop_a")).resolves.toEqual({
      approvalTier: "sandbox",
      sandboxAvailable: true,
    });
  });
});
