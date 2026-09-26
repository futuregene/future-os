// @vitest-environment jsdom
import type { StoredApprovalRequest, StoredThread, StoredWorkspace } from "../../../integrations/storage/threadStore";
import { act } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { flushAsync, renderHook } from "../../../test/renderHook";
import { useApprovals } from "./useApprovals";
import { useNewConversation } from "./useNewConversation";
import { useWorkspaceDialogs } from "./useWorkspaceDialogs";

const mocks = vi.hoisted(() => ({
  createThread: vi.fn(),
  decideApprovalRequest: vi.fn(),
  deleteWorkspace: vi.fn(),
  handler: null as null | ((event: { payload: { threadId: string } }) => void),
  listApprovalRequests: vi.fn(),
  renameWorkspace: vi.fn(),
  refreshStore: vi.fn(),
  unlisten: vi.fn(),
  updateCachedAgentState: vi.fn(),
  validateImageAttachment: vi.fn(),
}));

vi.mock("../../../integrations/storage/threadStore", () => ({
  createThread: (...args: unknown[]) => mocks.createThread(...args),
  decideApprovalRequest: (...args: unknown[]) => mocks.decideApprovalRequest(...args),
  deleteWorkspace: (...args: unknown[]) => mocks.deleteWorkspace(...args),
  listApprovalRequests: (...args: unknown[]) => mocks.listApprovalRequests(...args),
  renameWorkspace: (...args: unknown[]) => mocks.renameWorkspace(...args),
}));
vi.mock("../../../integrations/agent/agentStateCache", () => ({
  updateCachedAgentState: (...args: unknown[]) => mocks.updateCachedAgentState(...args),
}));
vi.mock("../../../integrations/storage/files", () => ({
  validateImageAttachment: (...args: unknown[]) => mocks.validateImageAttachment(...args),
}));
vi.mock("../../../lib/usePolling", () => ({ usePolling: () => {} }));
vi.mock("@tauri-apps/api/event", () => ({
  listen: (_event: string, handler: (event: { payload: { threadId: string } }) => void) => {
    mocks.handler = handler;
    return Promise.resolve(mocks.unlisten);
  },
}));

function thread(id: string, overrides: Partial<StoredThread> = {}): StoredThread {
  return {
    id,
    agentSessionId: id,
    title: id,
    mode: "chat",
    workspaceId: `w-${id}`,
    status: "active",
    pinned: false,
    readonly: false,
    createdAt: 0,
    updatedAt: 0,
    ...overrides,
  };
}

function workspace(id: string): StoredWorkspace {
  return { cleanupStatus: "active", createdAt: 0, id, kind: "user", name: id, path: `/tmp/${id}`, updatedAt: 0 };
}

function approval(id: string, overrides: Partial<StoredApprovalRequest> = {}): StoredApprovalRequest {
  return {
    id,
    threadId: "t1",
    kind: "shell",
    status: "pending",
    title: id,
    createdAt: 0,
    updatedAt: 0,
    decisionScope: "session",
    decisionSource: "gui",
    reviewer: "user",
    ...overrides,
  };
}

function captureToasts() {
  const toasts: { message: string; tone?: string }[] = [];
  const handler = (event: Event) => toasts.push((event as CustomEvent<{ message: string; tone?: string }>).detail);
  window.addEventListener("futureos:toast", handler);
  return { stop: () => window.removeEventListener("futureos:toast", handler), toasts };
}

beforeEach(() => {
  mocks.createThread.mockReset().mockResolvedValue(thread("new-1"));
  mocks.decideApprovalRequest.mockReset().mockResolvedValue(undefined);
  mocks.deleteWorkspace.mockReset().mockResolvedValue(undefined);
  mocks.handler = null;
  mocks.listApprovalRequests.mockReset().mockResolvedValue([]);
  mocks.renameWorkspace.mockReset().mockResolvedValue(undefined);
  mocks.refreshStore.mockReset().mockResolvedValue(undefined);
  mocks.unlisten.mockReset();
  mocks.updateCachedAgentState.mockReset();
  mocks.validateImageAttachment.mockReset().mockResolvedValue(undefined);
});

describe("useWorkspaceDialogs", () => {
  const mount = () => renderHook(() => useWorkspaceDialogs({ refreshStore: mocks.refreshStore }));

  it("seeds the rename dialog from the workspace name", () => {
    const harness = mount();
    act(() => harness.current.openRename(workspace("ws-1")));
    expect(harness.current.renameDialog).toMatchObject({ error: null, submitting: false, value: "ws-1" });
    expect(harness.current.renameDialog!.workspace.id).toBe("ws-1");
  });

  it("is a no-op when confirming with nothing open", async () => {
    const harness = mount();
    await act(async () => {
      await harness.current.confirmRename();
      await harness.current.confirmDelete();
    });
    expect(mocks.renameWorkspace).not.toHaveBeenCalled();
    expect(mocks.deleteWorkspace).not.toHaveBeenCalled();
  });

  it("rejects a whitespace-only name and keeps the dialog open", async () => {
    const harness = mount();
    act(() => harness.current.openRename(workspace("ws-1")));
    act(() => harness.current.setRenameDialog(state => (state ? { ...state, value: "   " } : state)));
    await act(async () => {
      await harness.current.confirmRename();
    });
    expect(mocks.renameWorkspace).not.toHaveBeenCalled();
    expect(harness.current.renameDialog?.error).toBe("Name cannot be empty.");
    expect(harness.current.renameDialog?.submitting).toBe(false);
  });

  it("closes without a round-trip when the name is unchanged", async () => {
    const harness = mount();
    act(() => harness.current.openRename(workspace("ws-1")));
    await act(async () => {
      await harness.current.confirmRename();
    });
    expect(mocks.renameWorkspace).not.toHaveBeenCalled();
    expect(harness.current.renameDialog).toBeNull();
  });

  it("trims the new name, renames and refreshes the store", async () => {
    const harness = mount();
    act(() => harness.current.openRename(workspace("ws-1")));
    act(() => harness.current.setRenameDialog(state => (state ? { ...state, value: "  Beta  " } : state)));
    await act(async () => {
      await harness.current.confirmRename();
    });
    expect(mocks.renameWorkspace).toHaveBeenCalledWith({ name: "Beta", workspaceId: "ws-1" });
    expect(mocks.refreshStore).toHaveBeenCalledWith();
    expect(harness.current.renameDialog).toBeNull();
  });

  it("surfaces a rename failure and lets the user retry", async () => {
    mocks.renameWorkspace.mockRejectedValue(new Error("disk full"));
    const harness = mount();
    act(() => harness.current.openRename(workspace("ws-1")));
    act(() => harness.current.setRenameDialog(state => (state ? { ...state, value: "Beta" } : state)));
    await act(async () => {
      await harness.current.confirmRename();
    });
    expect(harness.current.renameDialog?.error).toBe("disk full");
    expect(harness.current.renameDialog?.submitting).toBe(false);
    expect(harness.current.renameDialog?.value).toBe("Beta");
  });

  it("ignores a second confirm while the first is in flight", async () => {
    let release!: () => void;
    mocks.renameWorkspace.mockReturnValue(new Promise<void>((resolve) => {
      release = resolve;
    }));
    const harness = mount();
    act(() => harness.current.openRename(workspace("ws-1")));
    act(() => harness.current.setRenameDialog(state => (state ? { ...state, value: "Beta" } : state)));
    await act(async () => {
      void harness.current.confirmRename();
      await Promise.resolve();
    });
    await act(async () => {
      await harness.current.confirmRename();
    });
    expect(mocks.renameWorkspace).toHaveBeenCalledTimes(1);
    await act(async () => {
      release();
      await Promise.resolve();
    });
  });

  it("deletes the workspace and closes the dialog, or keeps it open with an error", async () => {
    const harness = mount();
    act(() => harness.current.openDelete(workspace("ws-1")));
    await act(async () => {
      await harness.current.confirmDelete();
    });
    expect(mocks.deleteWorkspace).toHaveBeenCalledWith("ws-1");
    expect(mocks.refreshStore).toHaveBeenCalledWith();
    expect(harness.current.deleteDialog).toBeNull();

    mocks.deleteWorkspace.mockRejectedValue(new Error("locked"));
    act(() => harness.current.openDelete(workspace("ws-2")));
    await act(async () => {
      await harness.current.confirmDelete();
    });
    expect(harness.current.deleteDialog?.error).toBe("locked");
    expect(harness.current.deleteDialog?.submitting).toBe(false);
  });

  // The four cases below are the "dialog was dismissed while the write was in
  // flight" arms: each `setState(current => current ? … : current)` updater has
  // a null side that only runs when the user closes the dialog before the
  // updater is applied. Without them an error could reopen a dialog the user
  // already dismissed, or a stale result could resurrect a submitted one.
  it("does not reopen a dismissed dialog when the rename write fails late", async () => {
    let failWrite!: (error: Error) => void;
    mocks.renameWorkspace.mockReturnValue(new Promise<void>((_resolve, reject) => {
      failWrite = reject;
    }));
    const harness = mount();
    act(() => harness.current.openRename(workspace("ws-1")));
    act(() => harness.current.setRenameDialog(state => (state ? { ...state, value: "Beta" } : state)));
    await act(async () => {
      void harness.current.confirmRename();
      await Promise.resolve();
    });

    // The user dismisses the dialog (Escape / backdrop) before the write lands.
    act(() => harness.current.setRenameDialog(null));
    await act(async () => {
      failWrite(new Error("disk full"));
      await Promise.resolve();
      await Promise.resolve();
    });
    expect(harness.current.renameDialog).toBeNull();
    harness.unmount();
  });

  it("does not resurrect a dismissed dialog when a blank name is rejected", async () => {
    const harness = mount();
    act(() => harness.current.openRename(workspace("ws-1")));
    act(() => harness.current.setRenameDialog(state => (state ? { ...state, value: "   " } : state)));

    // The dismissal and the confirmation happen in the same tick, so the
    // confirm still sees the (closed-over) dialog while the state is already
    // null — exactly the race the null side of the updater guards.
    const confirm = harness.current.confirmRename;
    const close = harness.current.setRenameDialog;
    await act(async () => {
      close(null);
      await confirm();
    });
    expect(harness.current.renameDialog).toBeNull();
    expect(mocks.renameWorkspace).not.toHaveBeenCalled();
    harness.unmount();
  });

  it("does not resurrect a dismissed dialog when a valid rename starts", async () => {
    const harness = mount();
    act(() => harness.current.openRename(workspace("ws-1")));
    act(() => harness.current.setRenameDialog(state => (state ? { ...state, value: "Beta" } : state)));

    const confirm = harness.current.confirmRename;
    const close = harness.current.setRenameDialog;
    await act(async () => {
      close(null);
      await confirm();
    });
    // The write still goes through, but no dialog is reopened behind the user.
    expect(mocks.renameWorkspace).toHaveBeenCalledWith({ name: "Beta", workspaceId: "ws-1" });
    expect(harness.current.renameDialog).toBeNull();
    harness.unmount();
  });

  it("does not reopen a dismissed dialog when the delete write fails late", async () => {
    let failWrite!: (error: Error) => void;
    mocks.deleteWorkspace.mockReturnValue(new Promise<void>((_resolve, reject) => {
      failWrite = reject;
    }));
    const harness = mount();
    act(() => harness.current.openDelete(workspace("ws-1")));
    await act(async () => {
      void harness.current.confirmDelete();
      await Promise.resolve();
    });

    act(() => harness.current.setDeleteDialog(null));
    await act(async () => {
      failWrite(new Error("locked"));
      await Promise.resolve();
      await Promise.resolve();
    });
    expect(harness.current.deleteDialog).toBeNull();
    harness.unmount();
  });

  it("does not resurrect a dismissed delete dialog when the delete starts", async () => {
    const harness = mount();
    act(() => harness.current.openDelete(workspace("ws-1")));

    const confirm = harness.current.confirmDelete;
    const close = harness.current.setDeleteDialog;
    await act(async () => {
      close(null);
      await confirm();
    });
    expect(mocks.deleteWorkspace).toHaveBeenCalledWith("ws-1");
    expect(harness.current.deleteDialog).toBeNull();
    harness.unmount();
  });
});

describe("useNewConversation", () => {
  function mount() {
    const syncSelection = vi.fn();
    const setSection = vi.fn();
    const setCenterMode = vi.fn();
    const harness = renderHook(() => useNewConversation({
      refreshStore: mocks.refreshStore,
      syncSelection,
      setSection,
      setCenterMode,
    }));
    return { harness, setCenterMode, setSection, syncSelection };
  }

  it("creates the thread, primes the selection and stages the first prompt", async () => {
    const { harness, setCenterMode, setSection, syncSelection } = mount();
    await act(async () => {
      await harness.current.startNewConversation({
        content: "  Fix   the   flaky test  ",
        mode: "chat",
        modelId: "m1",
        thinkingLevel: "high",
      });
    });

    expect(mocks.createThread).toHaveBeenCalledWith({
      mode: "chat",
      title: "Fix the flaky test",
      workspaceId: undefined,
      workspaceName: undefined,
      workspacePath: undefined,
      modelId: "m1",
      thinkingLevel: "high",
    });
    expect(syncSelection).toHaveBeenCalledWith("m1", "high");
    expect(mocks.updateCachedAgentState).toHaveBeenCalledWith("new-1", { model: "m1", thinkingLevel: "high" });
    expect(mocks.refreshStore).toHaveBeenCalledWith("new-1");
    expect(setSection).toHaveBeenCalledWith("chat");
    expect(setCenterMode).toHaveBeenCalledWith("thread");
    expect(harness.current.pendingPrompt).toMatchObject({ content: "  Fix   the   flaky test  ", targetThreadId: "new-1" });
  });

  it("falls back to the New Chat label for empty content and routes a workspace thread", async () => {
    mocks.createThread.mockResolvedValue(thread("ws-thread", { mode: "workspace" }));
    const { harness, setSection } = mount();
    await act(async () => {
      await harness.current.startNewConversation({
        content: "   \n ",
        mode: "workspace",
        modelId: "m1",
        thinkingLevel: "off",
        workspace: { id: "ws-1", label: "Alpha", path: "/tmp/alpha" },
      });
    });
    expect(mocks.createThread).toHaveBeenCalledWith(expect.objectContaining({
      title: "New Chat",
      workspaceId: "ws-1",
      workspaceName: "Alpha",
      workspacePath: "/tmp/alpha",
    }));
    expect(setSection).toHaveBeenCalledWith("workspace");
  });

  it("truncates an over-long title by characters, not code units", async () => {
    const { harness } = mount();
    await act(async () => {
      await harness.current.startNewConversation({
        content: " 中文".repeat(40),
        mode: "chat",
        modelId: "m1",
        thinkingLevel: "off",
      });
    });
    const title = mocks.createThread.mock.calls[0]![0].title as string;
    expect(title.endsWith("...")).toBe(true);
    expect(Array.from(title.replace("...", ""))).toHaveLength(28);
  });

  it("re-validates image attachments before creating the thread", async () => {
    const { harness } = mount();
    await act(async () => {
      await harness.current.startNewConversation({
        attachments: [{ kind: "image", name: "shot.png", path: "/tmp/shot.png" } as never],
        content: "look",
        mode: "chat",
        modelId: "m1",
        thinkingLevel: "off",
      });
    });
    expect(mocks.validateImageAttachment).toHaveBeenCalledWith("/tmp/shot.png");
    expect(mocks.createThread).toHaveBeenCalledTimes(1);
  });

  it("rejects an unreadable image with a toast and never creates the thread", async () => {
    mocks.validateImageAttachment.mockRejectedValue(new Error("gone"));
    const toasts = captureToasts();
    const { harness } = mount();
    await act(async () => {
      await harness.current.startNewConversation({
        attachments: [{ kind: "image", name: "shot.png", path: "/tmp/shot.png" } as never],
        content: "look",
        mode: "chat",
        modelId: "m1",
        thinkingLevel: "off",
      }).catch(() => {});
    });

    expect(mocks.createThread).not.toHaveBeenCalled();
    expect(toasts.toasts).toHaveLength(1);
    expect(toasts.toasts[0]!.message).toContain("shot.png");
    expect(toasts.toasts[0]!.tone).toBe("error");
    toasts.stop();
  });

  it("skips validation for non-image attachments", async () => {
    const { harness } = mount();
    await act(async () => {
      await harness.current.startNewConversation({
        attachments: [{ kind: "file", name: "notes.txt", path: "/tmp/notes.txt" } as never],
        content: "see file",
        mode: "chat",
        modelId: "m1",
        thinkingLevel: "off",
      });
    });
    expect(mocks.validateImageAttachment).not.toHaveBeenCalled();
    expect(mocks.createThread).toHaveBeenCalledTimes(1);
  });

  it("toasts and rethrows when thread creation fails, leaving no pending prompt", async () => {
    mocks.createThread.mockRejectedValue(new Error("db offline"));
    const toasts = captureToasts();
    const { harness } = mount();
    await act(async () => {
      await harness.current.startNewConversation({
        content: "hello",
        mode: "chat",
        modelId: "m1",
        thinkingLevel: "off",
      }).catch(() => {});
    });
    expect(toasts.toasts[0]!.message).toContain("db offline");
    expect(harness.current.pendingPrompt).toBeNull();
    toasts.stop();
  });

  it("consumes only the prompt it was asked to drop", async () => {
    const { harness } = mount();
    await act(async () => {
      await harness.current.startNewConversation({
        content: "hello",
        mode: "chat",
        modelId: "m1",
        thinkingLevel: "off",
      });
    });
    const id = harness.current.pendingPrompt!.id;
    act(() => harness.current.consumePendingPrompt("someone-else"));
    expect(harness.current.pendingPrompt?.id).toBe(id);
    act(() => harness.current.consumePendingPrompt(id));
    expect(harness.current.pendingPrompt).toBeNull();
  });
});

describe("useApprovals", () => {
  const mount = (activeThreadId: string | null) => renderHook(() => useApprovals(activeThreadId));

  it("returns nothing and skips the fetch when no thread is active", async () => {
    const harness = mount(null);
    await flushAsync();
    expect(mocks.listApprovalRequests).not.toHaveBeenCalled();
    expect(harness.current.activeApproval).toBeNull();
  });

  it("exposes the oldest pending approval, ignoring decided ones", async () => {
    mocks.listApprovalRequests.mockResolvedValue([
      approval("newer", { createdAt: 30 }),
      approval("done", { createdAt: 5, status: "approved" }),
      approval("oldest", { createdAt: 10 }),
    ]);
    const harness = mount("t1");
    await flushAsync();
    expect(mocks.listApprovalRequests).toHaveBeenCalledWith("t1");
    expect(harness.current.activeApproval?.id).toBe("oldest");
  });

  it("reloads only for the active thread's push", async () => {
    const harness = mount("t1");
    await flushAsync();
    expect(mocks.listApprovalRequests).toHaveBeenCalledTimes(1);

    act(() => mocks.handler!({ payload: { threadId: "other" } }));
    await flushAsync();
    expect(mocks.listApprovalRequests).toHaveBeenCalledTimes(1);

    act(() => mocks.handler!({ payload: { threadId: "t1" } }));
    await flushAsync();
    expect(mocks.listApprovalRequests).toHaveBeenCalledTimes(2);
    expect(harness.current.activeApproval).toBeNull();
  });

  it("records the decision with a localized note and refetches", async () => {
    mocks.listApprovalRequests.mockResolvedValue([approval("a")]);
    const harness = mount("t1");
    await flushAsync();
    await act(async () => {
      await harness.current.decideApproval(approval("a"), "approved");
    });
    expect(mocks.decideApprovalRequest).toHaveBeenCalledWith({
      approvalRequestId: "a",
      decisionNote: "Approved in GUI.",
      status: "approved",
    });
    expect(mocks.listApprovalRequests).toHaveBeenCalledTimes(2);

    await act(async () => {
      await harness.current.decideApproval(approval("a"), "rejected");
    });
    expect(mocks.decideApprovalRequest).toHaveBeenLastCalledWith(expect.objectContaining({
      decisionNote: "Rejected in GUI.",
      status: "rejected",
    }));
  });

  it("keeps the previous queue visible when a reload fails", async () => {
    mocks.listApprovalRequests.mockResolvedValueOnce([approval("keep")]);
    const harness = mount("t1");
    await flushAsync();
    expect(harness.current.activeApproval?.id).toBe("keep");

    mocks.listApprovalRequests.mockRejectedValueOnce(new Error("backend down"));
    act(() => harness.current.reloadApprovals());
    await flushAsync();
    expect(harness.current.activeApproval?.id).toBe("keep");
  });

  it("unsubscribes from the push on unmount", async () => {
    const harness = mount("t1");
    await flushAsync();
    harness.unmount();
    await flushAsync();
    expect(mocks.unlisten).toHaveBeenCalled();
  });
});
