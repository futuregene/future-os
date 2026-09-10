// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { clearThreadMessageSnapshots } from "./threadMessageCache";
import { useThreadMessages } from "./useThreadMessages";

const { page, latest, runs } = vi.hoisted(() => ({
  page: vi.fn(),
  latest: vi.fn(),
  runs: vi.fn(),
}));
vi.mock("../../integrations/storage/threadStore", () => ({
  getSessionEntriesPage: page,
  getLatestRun: latest,
  listRuns: runs,
  getRun: vi.fn(),
}));
vi.mock("./threadRunProjection", () => ({
  applyJournalRunOutcomes: (messages: unknown) => messages,
  applyRunMetadata: (messages: unknown) => messages,
  recoverAbortedTurns: async (messages: unknown) => messages,
  recoverFailedRuns: (messages: unknown) => messages,
  buildStreamingPreview: vi.fn(),
  mergeStreamingPreview: (messages: unknown) => messages,
}));

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

function history(ids: string[], nextOffset: number, hasMore = true) {
  return {
    entries: ids.map(id => ({
      id,
      kind: "user",
      role: "user",
      createdAtMs: Date.parse("2026-01-01T00:00:00Z"),
      blocks: [{ kind: "text", text: id }],
    })),
    nextOffset,
    hasMore,
  };
}

let current: ReturnType<typeof useThreadMessages>;
function Harness({ sessionId = "synthetic-session" }: { sessionId?: string | null }) {
  current = useThreadMessages({
    threadId: "synthetic",
    agentSessionId: sessionId,
  });
  return null;
}

beforeEach(() => {
  vi.clearAllMocks();
  clearThreadMessageSnapshots();
  latest.mockResolvedValue(null);
  runs.mockResolvedValue([]);
});

describe("on-demand history", () => {
  it("fills the entire history cache for search and reuses it on the next query", async () => {
    page.mockResolvedValueOnce(history(["u3"], 20));
    const root = createRoot(document.createElement("div"));
    await act(async () => root.render(<Harness />));
    page.mockResolvedValueOnce(history(["u2"], 10));
    page.mockResolvedValueOnce(history(["u1"], 0, false));
    await act(async () => {
      const messages = await current.loadAllHistoryForSearch(new AbortController().signal);
      expect(messages.map(message => message.content)).toEqual(["u1", "u2", "u3"]);
    });
    expect(current.hasOlderHistory).toBe(false);
    const requests = page.mock.calls.length;
    await act(async () => current.loadAllHistoryForSearch(new AbortController().signal));
    expect(page).toHaveBeenCalledTimes(requests);
    act(() => root.unmount());
  });

  it("stops a full-history search on a failed page instead of returning partial results", async () => {
    page.mockResolvedValueOnce(history(["u3"], 20));
    const root = createRoot(document.createElement("div"));
    await act(async () => root.render(<Harness />));
    page.mockRejectedValueOnce(new Error("offline"));
    await act(async () => {
      await expect(current.loadAllHistoryForSearch(new AbortController().signal)).rejects.toThrow("cursor");
    });
    expect(page).toHaveBeenCalledTimes(2);
    expect(current.hasOlderHistory).toBe(true);
    act(() => root.unmount());
  });

  it("does not read additional pages for a cancelled search", async () => {
    page.mockResolvedValueOnce(history(["u3"], 20));
    const root = createRoot(document.createElement("div"));
    await act(async () => root.render(<Harness />));
    const controller = new AbortController();
    controller.abort();
    await expect(current.loadAllHistoryForSearch(controller.signal)).rejects.toMatchObject({ name: "AbortError" });
    expect(page).toHaveBeenCalledTimes(1);
    act(() => root.unmount());
  });

  it("reconciles a persisted first user entry with its optimistic run identity", async () => {
    page.mockResolvedValueOnce(history([], 0, false));
    const root = createRoot(document.createElement("div"));
    await act(async () => root.render(<Harness sessionId={null} />));
    await act(async () => current.setMessages([{
      id: "pending_user",
      runId: "first-run",
      role: "user",
      authorKey: "author.you",
      content: "synthetic prompt",
      createdAt: "2026-01-01T00:00:00Z",
    }]));
    const persisted = history(["canonical-user"], 0, false);
    page.mockResolvedValueOnce({ ...persisted, entries: persisted.entries.map(entry => ({ ...entry, runId: "first-run" })) });
    await act(async () => root.render(<Harness sessionId="bound" />));
    expect(current.messages).toHaveLength(1);
    expect(current.messages[0]?.id).toBe("pending_user");
    expect(current.messages[0]?.runId).toBe("first-run");
    await act(async () => root.unmount());
  });

  it("keeps replacement failures blocked until a successful retry", async () => {
    page.mockResolvedValueOnce(history(["old"], 20));
    const root = createRoot(document.createElement("div"));
    await act(async () => root.render(<Harness />));
    page.mockRejectedValueOnce(new Error("replacement unavailable"));
    await act(async () => root.render(<Harness sessionId="replacement" />));
    expect(current.loadingThread).toBe(true);
    expect(current.messages[0]?.content).toBe("old");
    expect(current.historyError).toBe("replacement unavailable");
    page.mockResolvedValueOnce(history([], 0, false));
    await act(async () => current.reloadMessagesQuiet("synthetic", true));
    expect(current.messages).toEqual([]);
    expect(current.loadingThread).toBe(false);
    expect(current.historyError).toBeNull();
    await act(async () => root.unmount());
  });

  it("binds the first session without clearing messages or blocking the composer", async () => {
    page.mockResolvedValueOnce(history([], 0, false));
    const root = createRoot(document.createElement("div"));
    await act(async () => root.render(<Harness sessionId={null} />));
    const writer = current.setMessages;
    await act(async () => writer([{ id: "pending", runId: "run", role: "assistant", authorKey: "author.researchCopilot", content: "live", status: "streaming", createdAt: "2026-01-01T00:00:00Z" }]));
    let resolve!: (value: ReturnType<typeof history>) => void;
    page.mockImplementationOnce(() => new Promise((done) => {
      resolve = done;
    }));
    await act(async () => root.render(<Harness sessionId="bound" />));
    expect(current.messages[0]?.content).toBe("live");
    expect(current.loadingThread).toBe(false);
    expect(current.loadingIndicator).toBe(false);
    expect(current.setMessages).toBe(writer);
    await act(async () => {
      writer(messages => messages.map(message => ({ ...message, content: "newer" })));
      resolve(history([], 0, false));
    });
    expect(current.messages[0]?.content).toBe("newer");
    await act(async () => root.unmount());
  });

  it("replaces a real session atomically and rejects its old writers", async () => {
    page.mockResolvedValueOnce(history(["old"], 20));
    const root = createRoot(document.createElement("div"));
    await act(async () => root.render(<Harness />));
    const oldWriter = current.setMessages;
    let resolve!: (value: ReturnType<typeof history>) => void;
    page.mockImplementationOnce(() => new Promise((done) => {
      resolve = done;
    }));
    await act(async () => root.render(<Harness sessionId="replacement" />));
    expect(current.messages[0]?.content).toBe("old");
    expect(current.loadingThread).toBe(true);
    expect(current.sessionChanged).toBe(true);
    await act(async () => oldWriter([]));
    expect(current.messages).toHaveLength(1);
    await act(async () => resolve(history(["new"], 0, false)));
    expect(current.messages.map(message => message.content)).toEqual(["new"]);
    expect(current.loadingThread).toBe(false);
    await act(async () => root.unmount());
  });

  it("orders forced refreshes and retains content on background failure", async () => {
    page.mockResolvedValueOnce(history(["initial"], 0, false));
    const root = createRoot(document.createElement("div"));
    await act(async () => root.render(<Harness />));
    let resolve!: (value: ReturnType<typeof history>) => void;
    page.mockImplementationOnce(() => new Promise((done) => {
      resolve = done;
    }));
    let older!: Promise<void>;
    await act(async () => {
      older = current.reloadMessagesQuiet("synthetic", true);
    });
    page.mockResolvedValueOnce(history(["newest"], 0, false));
    await act(async () => {
      await current.reloadMessagesQuiet("synthetic", true);
    });
    await act(async () => {
      resolve(history(["stale"], 0, false));
      await older;
    });
    expect(current.messages[0]?.content).toBe("newest");
    page.mockRejectedValueOnce(new Error("synthetic failure"));
    await act(async () => {
      await current.reloadMessagesQuiet("synthetic");
    });
    expect(current.messages[0]?.content).toBe("newest");
    expect(current.historyError).toBe("synthetic failure");
    expect(current.loadingThread).toBe(false);
    await act(async () => root.unmount());
  });

  it("discards an older-page response after the tail was replaced", async () => {
    page.mockResolvedValueOnce(history(["u3"], 20));
    const root = createRoot(document.createElement("div"));
    await act(async () => {
      root.render(<Harness />);
    });
    let resolve!: (value: ReturnType<typeof history>) => void;
    page.mockImplementationOnce(
      () =>
        new Promise((done) => {
          resolve = done;
        }),
    );
    const beforeCommit = vi.fn();
    let older!: Promise<void>;
    await act(async () => {
      older = current.loadOlderHistory(beforeCommit);
    });
    page.mockResolvedValueOnce(history(["replacement"], 0, false));
    await act(async () => {
      await current.reloadMessagesQuiet("synthetic");
      resolve(history(["obsolete"], 10));
      await older;
    });
    expect(beforeCommit).not.toHaveBeenCalled();
    expect(current.messages.map(message => message.content)).toEqual([
      "replacement",
    ]);
    expect(current.hasOlderHistory).toBe(false);
    await act(async () => {
      root.unmount();
    });
  });

  it("keeps warm pages and their cursor across a remount", async () => {
    page.mockResolvedValueOnce(history(["u3"], 20));
    const root = createRoot(document.createElement("div"));
    await act(async () => {
      root.render(<Harness />);
    });
    page.mockResolvedValueOnce(history(["u2"], 10));
    await act(async () => {
      await current.loadOlderHistory();
    });
    await act(async () => {
      root.unmount();
    });
    page.mockResolvedValueOnce(history(["u3"], 20));
    const remount = createRoot(document.createElement("div"));
    await act(async () => {
      remount.render(<Harness />);
    });
    expect(current.messages.map(message => message.content)).toEqual([
      "u2",
      "u3",
    ]);
    page.mockResolvedValueOnce(history(["u1"], 0, false));
    await act(async () => {
      await current.loadOlderHistory();
    });
    expect(page).toHaveBeenLastCalledWith("synthetic", 10);
    expect(current.messages.map(message => message.content)).toEqual([
      "u1",
      "u2",
      "u3",
    ]);
    await act(async () => {
      remount.unmount();
    });
  });

  it("fetches one initial page, coalesces older requests, and retains older pages on tail refresh", async () => {
    page.mockResolvedValueOnce(history(["u3", "u4"], 20));
    const container = document.createElement("div");
    const root = createRoot(container);
    await act(async () => {
      root.render(<Harness />);
    });
    expect(page).toHaveBeenCalledTimes(1);
    expect(page).toHaveBeenLastCalledWith("synthetic", null);
    expect(current.messages.map(message => message.content)).toEqual([
      "u3",
      "u4",
    ]);
    let resolve!: (value: ReturnType<typeof history>) => void;
    page.mockImplementationOnce(
      () =>
        new Promise((done) => {
          resolve = done;
        }),
    );
    await act(async () => {
      const older = current.loadOlderHistory();
      await current.loadOlderHistory();
      resolve(history(["u1", "u2"], 0, false));
      await older;
    });
    expect(page).toHaveBeenCalledTimes(2);
    expect(page).toHaveBeenLastCalledWith("synthetic", 20);
    expect(current.messages.map(message => message.content)).toEqual([
      "u1",
      "u2",
      "u3",
      "u4",
    ]);
    expect(current.hasOlderHistory).toBe(false);
    page.mockResolvedValueOnce(history(["u3", "u4", "u5"], 20));
    await act(async () => {
      await current.reloadMessagesQuiet("synthetic");
    });
    expect(current.messages.map(message => message.content)).toEqual([
      "u1",
      "u2",
      "u3",
      "u4",
      "u5",
    ]);
    expect(current.hasOlderHistory).toBe(false);
    await act(async () => {
      root.unmount();
    });
  });

  it("retains the cursor and messages after failure and permits retry", async () => {
    page.mockResolvedValueOnce(history(["u3"], 20));
    const root = createRoot(document.createElement("div"));
    await act(async () => {
      root.render(<Harness />);
    });
    page.mockRejectedValueOnce(new Error("synthetic read failure"));
    await act(async () => {
      await current.loadOlderHistory();
    });
    expect(current.historyError).toBe("synthetic read failure");
    expect(current.messages.map(message => message.content)).toEqual(["u3"]);
    expect(current.hasOlderHistory).toBe(true);
    page.mockResolvedValueOnce(history(["u2", "u3"], 0, false));
    await act(async () => {
      await current.loadOlderHistory();
    });
    expect(page).toHaveBeenLastCalledWith("synthetic", 20);
    expect(current.messages.map(message => message.content)).toEqual([
      "u2",
      "u3",
    ]);
    expect(current.historyError).toBeNull();
    await act(async () => {
      root.unmount();
    });
  });
});
